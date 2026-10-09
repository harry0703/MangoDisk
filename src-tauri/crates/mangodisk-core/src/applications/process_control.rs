use std::{collections::HashSet, path::PathBuf, time::Instant};

use mangodisk_platform::{
    current_platform, ApplicationProcessCloseMode, ApplicationProcessTarget, Platform,
};
use serde::{Deserialize, Serialize};

use crate::shared::{CoreError, CoreResult};

const MAX_CLOSE_TARGETS: usize = 64;
const MAX_TARGET_ID_BYTES: usize = 128;
const MAX_PROCESS_IDENTITIES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationCloseMode {
    Graceful,
    Force,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationCloseTargetStatus {
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationCloseTargetResult {
    pub target_id: String,
    pub status: ApplicationCloseTargetStatus,
    pub matched_process_count: u64,
    pub requested_process_count: u64,
    pub remaining_processes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationCloseBatchResult {
    pub mode: ApplicationCloseMode,
    pub matched_process_count: u64,
    pub requested_process_count: u64,
    pub remaining_process_count: u64,
    pub failed_target_count: u64,
    pub targets: Vec<ApplicationCloseTargetResult>,
    pub elapsed_ms: u64,
}

/// A normal application quit request is distinct from verified process termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationQuitStatus {
    Requested,
    Unavailable,
    Unsupported,
}

pub(crate) fn request_application_quit(
    path: &std::path::Path,
) -> CoreResult<ApplicationQuitStatus> {
    use mangodisk_platform::application_quit::{self, ApplicationQuitOutcome};
    Ok(match application_quit::request(path)? {
        ApplicationQuitOutcome::Requested => ApplicationQuitStatus::Requested,
        ApplicationQuitOutcome::Unavailable => ApplicationQuitStatus::Unavailable,
        ApplicationQuitOutcome::Unsupported => ApplicationQuitStatus::Unsupported,
    })
}

/// Core-only close target resolved from trusted rule or application-catalog
/// evidence. Adapters pass stable product identifiers and never construct the
/// process names or executable paths held here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedApplicationCloseTarget {
    pub(crate) target_id: String,
    pub(crate) executable_names: Vec<String>,
    pub(crate) executable_paths: Vec<PathBuf>,
}

/// Brave and standalone Brave Origin share an executable filename on Windows.
/// A name-only close request cannot distinguish the selected product.
pub(crate) fn requires_exact_close_identity(names: &[String]) -> bool {
    names
        .iter()
        .any(|name| name.eq_ignore_ascii_case("brave.exe"))
}

pub(crate) fn close_resolved_applications(
    targets: Vec<ResolvedApplicationCloseTarget>,
    mode: ApplicationCloseMode,
) -> CoreResult<ApplicationCloseBatchResult> {
    validate_targets(&targets)?;
    let started = Instant::now();
    let platform_targets = targets
        .iter()
        .map(|target| ApplicationProcessTarget {
            executable_names: target.executable_names.clone(),
            executable_paths: target.executable_paths.clone(),
        })
        .collect::<Vec<_>>();
    let mut platform_results = current_platform()
        .close_application_processes_many(
            &platform_targets,
            match mode {
                ApplicationCloseMode::Graceful => ApplicationProcessCloseMode::Graceful,
                ApplicationCloseMode::Force => ApplicationProcessCloseMode::Force,
            },
        )
        .into_iter();
    // Report only the selected application's verified image state. Cleanup and
    // privacy preflight separately guard against every potential same-name writer.
    // Treating those other writers as failed closes would offer an ineffective
    // force retry for an application that has already stopped.
    let mut results = Vec::with_capacity(targets.len());

    for (index, target) in targets.into_iter().enumerate() {
        let platform_result = platform_results.next();
        match platform_result {
            Some(Ok(platform_result)) => {
                let status = close_target_status(mode, &platform_result.remaining_processes);
                let log_message = format!(
                    "application_close_target_finished target_id={} mode={} status={} matched_process_count={} requested_process_count={} remaining_process_count={}",
                    target.target_id,
                    mode.stable_code(),
                    status.stable_code(),
                    platform_result.matched_process_count,
                    platform_result.requested_process_count,
                    platform_result.remaining_processes.len()
                );
                if status == ApplicationCloseTargetStatus::Failed {
                    log::warn!("{log_message}");
                } else {
                    log::info!("{log_message}");
                }
                results.push(ApplicationCloseTargetResult {
                    target_id: target.target_id,
                    status,
                    matched_process_count: platform_result.matched_process_count,
                    requested_process_count: platform_result.requested_process_count,
                    remaining_processes: platform_result.remaining_processes,
                });
            }
            Some(Err(error)) => {
                let diagnostic = mangodisk_platform::diagnostics::text(&error);
                log::warn!(
                    "application_close_target_failed target_id={} mode={} error_code={:?} error_detail={}",
                    target.target_id,
                    mode.stable_code(),
                    error.code(),
                    diagnostic
                );
                results.push(ApplicationCloseTargetResult {
                    target_id: target.target_id,
                    status: ApplicationCloseTargetStatus::Failed,
                    matched_process_count: 0,
                    requested_process_count: 0,
                    remaining_processes: Vec::new(),
                });
            }
            None => {
                log::error!(
                    "application_close_target_result_missing target_id={} mode={} target_index={}",
                    target.target_id,
                    mode.stable_code(),
                    index
                );
                results.push(ApplicationCloseTargetResult {
                    target_id: target.target_id,
                    status: ApplicationCloseTargetStatus::Failed,
                    matched_process_count: 0,
                    requested_process_count: 0,
                    remaining_processes: Vec::new(),
                });
            }
        }
    }

    let result = ApplicationCloseBatchResult {
        mode,
        matched_process_count: results
            .iter()
            .map(|target| target.matched_process_count)
            .sum(),
        requested_process_count: results
            .iter()
            .map(|target| target.requested_process_count)
            .sum(),
        remaining_process_count: results
            .iter()
            .map(|target| target.remaining_processes.len() as u64)
            .sum(),
        failed_target_count: results
            .iter()
            .filter(|target| target.status == ApplicationCloseTargetStatus::Failed)
            .count() as u64,
        targets: results,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    log::info!(
        "application_close_batch_finished mode={} target_count={} matched_process_count={} requested_process_count={} remaining_process_count={} failed_target_count={} elapsed_ms={}",
        mode.stable_code(),
        result.targets.len(),
        result.matched_process_count,
        result.requested_process_count,
        result.remaining_process_count,
        result.failed_target_count,
        result.elapsed_ms
    );
    Ok(result)
}

fn close_target_status(
    mode: ApplicationCloseMode,
    remaining_processes: &[String],
) -> ApplicationCloseTargetStatus {
    if mode == ApplicationCloseMode::Force && !remaining_processes.is_empty() {
        ApplicationCloseTargetStatus::Failed
    } else {
        ApplicationCloseTargetStatus::Completed
    }
}

impl ApplicationCloseMode {
    const fn stable_code(self) -> &'static str {
        match self {
            Self::Graceful => "graceful",
            Self::Force => "force",
        }
    }
}

impl ApplicationCloseTargetStatus {
    const fn stable_code(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

fn validate_targets(targets: &[ResolvedApplicationCloseTarget]) -> CoreResult<()> {
    if targets.is_empty() || targets.len() > MAX_CLOSE_TARGETS {
        return Err(CoreError::invalid_input(
            "the application close target count is invalid",
        ));
    }
    let mut target_ids = HashSet::with_capacity(targets.len());
    for target in targets {
        if target.target_id.trim().is_empty()
            || target.target_id.len() > MAX_TARGET_ID_BYTES
            || !target_ids.insert(target.target_id.as_str())
        {
            return Err(CoreError::invalid_input(
                "the application close target identity is invalid",
            ));
        }
        if requires_exact_close_identity(&target.executable_names)
            && target.executable_paths.is_empty()
        {
            log::warn!(
                "application_close_identity_unavailable target_id={}",
                target.target_id
            );
            return Err(CoreError::operation_failed(
                "the shared browser executable requires a verified application path",
            ));
        }
        let identity_count = target.executable_names.len() + target.executable_paths.len();
        if identity_count == 0 || identity_count > MAX_PROCESS_IDENTITIES {
            return Err(CoreError::invalid_input(
                "the application close process identity count is invalid",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_brave_name_requires_verified_executable_paths() {
        for name in ["brave.exe", "BRAVE.EXE"] {
            let target = ResolvedApplicationCloseTarget {
                target_id: "brave-origin".into(),
                executable_names: vec![name.into()],
                executable_paths: Vec::new(),
            };
            assert!(
                validate_targets(&[target]).is_err(),
                "a name shared with regular Brave must not authorize closing both products"
            );
        }
    }

    #[test]
    fn duplicate_target_ids_are_rejected() {
        let targets = vec![
            ResolvedApplicationCloseTarget {
                target_id: "application-one".to_string(),
                executable_names: vec!["one".to_string()],
                executable_paths: Vec::new(),
            },
            ResolvedApplicationCloseTarget {
                target_id: "application-one".to_string(),
                executable_names: vec!["two".to_string()],
                executable_paths: Vec::new(),
            },
        ];
        assert!(validate_targets(&targets).is_err());
    }

    #[test]
    fn empty_process_identity_is_rejected() {
        let targets = vec![ResolvedApplicationCloseTarget {
            target_id: "application-one".to_string(),
            executable_names: Vec::new(),
            executable_paths: Vec::new(),
        }];
        assert!(validate_targets(&targets).is_err());
    }

    #[test]
    fn force_close_with_remaining_processes_is_failed() {
        assert_eq!(
            close_target_status(ApplicationCloseMode::Force, &["msedge.exe".to_string()]),
            ApplicationCloseTargetStatus::Failed
        );
        assert_eq!(
            close_target_status(ApplicationCloseMode::Force, &[]),
            ApplicationCloseTargetStatus::Completed
        );
    }

    #[test]
    fn graceful_close_with_remaining_processes_can_be_retried() {
        assert_eq!(
            close_target_status(ApplicationCloseMode::Graceful, &["msedge.exe".to_string()]),
            ApplicationCloseTargetStatus::Completed
        );
    }
}
