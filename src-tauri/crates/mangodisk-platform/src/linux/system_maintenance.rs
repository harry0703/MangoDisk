use std::{path::PathBuf, time::Duration};

use crate::{
    run_controlled_command, ControlledCommandLimits, ControlledEnvironmentPolicy,
    ControlledExecutable, PlatformCancellation, PlatformError, PlatformErrorCode, PlatformResult,
    PlatformSystemMaintenanceCompletion, PlatformSystemMaintenanceDiagnosticCode,
    PlatformSystemMaintenanceExecution, PlatformSystemMaintenancePhase,
    PlatformSystemMaintenanceProgress, PlatformSystemMaintenanceProgressSink,
    PlatformSystemMaintenanceState, PlatformSystemMaintenanceStatus,
};

const FONT_CACHE_TASK: &str = "linux.maintenance.font-cache";
const PACKAGE_INTEGRITY_TASK: &str = "linux.maintenance.package-integrity";

const SUPPORTED_TASKS: &[&str] = &[FONT_CACHE_TASK, PACKAGE_INTEGRITY_TASK];

const MAINTENANCE_LIMITS: ControlledCommandLimits = ControlledCommandLimits {
    timeout: Duration::from_secs(300),
    stdout_bytes: 256 * 1024,
    stderr_bytes: 64 * 1024,
};

/// Resolves the first executable on PATH for a named tool.
fn resolve_on_path(name: &str) -> Option<PathBuf> {
    let path_value = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path_value) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn capture_tool(name: &str) -> Option<ControlledExecutable> {
    let path = resolve_on_path(name)?;
    ControlledExecutable::capture(&path).ok()
}

pub(crate) fn scan(
    task_ids: &[&str],
    cancellation: &PlatformCancellation,
) -> PlatformResult<Vec<PlatformSystemMaintenanceState>> {
    validate_ids(task_ids)?;
    let mut states = Vec::with_capacity(task_ids.len());
    for task_id in task_ids {
        if cancellation.is_cancelled() {
            return Err(PlatformError::operation_failed(
                "system maintenance scan was cancelled",
            ));
        }
        states.push(match *task_id {
            FONT_CACHE_TASK => availability_state(
                FONT_CACHE_TASK,
                resolve_on_path("fc-cache").is_some(),
                false,
            ),
            PACKAGE_INTEGRITY_TASK => availability_state(
                PACKAGE_INTEGRITY_TASK,
                resolve_on_path("pacman").is_some() || resolve_on_path("dpkg").is_some(),
                false,
            ),
            _ => unreachable!("validated maintenance identifier"),
        });
    }
    Ok(states)
}

pub(crate) fn execute(
    task_id: &str,
    cancellation: &PlatformCancellation,
    _authorization_prompt: Option<&str>,
    progress: &PlatformSystemMaintenanceProgressSink,
) -> PlatformResult<PlatformSystemMaintenanceExecution> {
    validate_ids(&[task_id])?;
    match task_id {
        FONT_CACHE_TASK => {
            progress(PlatformSystemMaintenanceProgress::phase(
                PlatformSystemMaintenancePhase::RefreshingShellCaches,
            ));
            let executable = capture_tool("fc-cache").ok_or_else(|| {
                PlatformError::new(
                    PlatformErrorCode::Unsupported,
                    "fc-cache is not installed on this system",
                )
            })?;
            let output = run_controlled_command(
                "linux-maintenance-font-cache",
                &executable,
                &["-f"],
                ControlledEnvironmentPolicy::Inherit,
                MAINTENANCE_LIMITS,
                &|| cancellation.is_cancelled(),
            )
            .map_err(|error| {
                PlatformError::operation_failed(format!(
                    "font cache refresh failed reason={}",
                    error.as_str()
                ))
            })?;
            let verified = output.status.success();
            Ok(execution(
                task_id,
                true,
                verified,
                false,
                false,
                PlatformSystemMaintenanceCompletion::Completed,
            ))
        }
        PACKAGE_INTEGRITY_TASK => {
            progress(PlatformSystemMaintenanceProgress::phase(
                PlatformSystemMaintenancePhase::CheckingSystemFiles,
            ));
            let (tool, arguments): (&str, &[&str]) = if resolve_on_path("pacman").is_some() {
                ("pacman", &["-Qk"])
            } else {
                ("dpkg", &["--verify"])
            };
            let executable = capture_tool(tool).ok_or_else(|| {
                PlatformError::new(
                    PlatformErrorCode::Unsupported,
                    "no supported package manager is installed on this system",
                )
            })?;
            progress(PlatformSystemMaintenanceProgress::phase(
                PlatformSystemMaintenancePhase::Verifying,
            ));
            let output = run_controlled_command(
                "linux-maintenance-package-integrity",
                &executable,
                arguments,
                ControlledEnvironmentPolicy::Inherit,
                MAINTENANCE_LIMITS,
                &|| cancellation.is_cancelled(),
            )
            .map_err(|error| {
                PlatformError::operation_failed(format!(
                    "package integrity check failed reason={}",
                    error.as_str()
                ))
            })?;
            let verified = output.status.success();
            Ok(execution(
                task_id,
                false,
                verified,
                false,
                false,
                PlatformSystemMaintenanceCompletion::Completed,
            ))
        }
        _ => unreachable!("validated maintenance identifier"),
    }
}

fn validate_ids(task_ids: &[&str]) -> PlatformResult<()> {
    if task_ids.is_empty()
        || task_ids
            .iter()
            .any(|task_id| !SUPPORTED_TASKS.contains(task_id))
    {
        return Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "system maintenance task identifier is unsupported",
        ));
    }
    Ok(())
}

fn availability_state(
    task_id: &str,
    is_available: bool,
    requires_elevation: bool,
) -> PlatformSystemMaintenanceState {
    if is_available {
        state(
            task_id,
            PlatformSystemMaintenanceStatus::Available,
            requires_elevation,
            None,
        )
    } else {
        state(
            task_id,
            PlatformSystemMaintenanceStatus::Unavailable,
            requires_elevation,
            Some(PlatformSystemMaintenanceDiagnosticCode::ToolUnavailable),
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn state(
    task_id: &str,
    status: PlatformSystemMaintenanceStatus,
    requires_elevation: bool,
    diagnostic: Option<PlatformSystemMaintenanceDiagnosticCode>,
) -> PlatformSystemMaintenanceState {
    PlatformSystemMaintenanceState {
        task_id: task_id.to_string(),
        status,
        requires_elevation,
        diagnostic,
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::fn_params_excessive_bools)]
fn execution(
    task_id: &str,
    changed: bool,
    verified: bool,
    requires_restart: bool,
    started: bool,
    completion: PlatformSystemMaintenanceCompletion,
) -> PlatformSystemMaintenanceExecution {
    PlatformSystemMaintenanceExecution {
        task_id: task_id.to_string(),
        changed,
        verified,
        requires_restart,
        completion: if started {
            PlatformSystemMaintenanceCompletion::Started
        } else {
            completion
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cancellation() -> PlatformCancellation {
        PlatformCancellation::new(|| false)
    }

    #[test]
    fn rejects_unknown_task_identifiers() {
        assert!(scan(&["garbage"], &cancellation()).is_err());
        assert!(execute("garbage", &cancellation(), None, &|_| {}).is_err());
    }

    #[test]
    fn rejects_empty_task_identifiers() {
        assert!(scan(&[], &cancellation()).is_err());
    }

    #[test]
    fn accepts_every_supported_task_identifier() {
        let states = scan(SUPPORTED_TASKS, &cancellation()).unwrap();
        assert_eq!(states.len(), SUPPORTED_TASKS.len());
        for state in states {
            assert!(!state.requires_elevation);
        }
    }

    #[test]
    fn resolves_an_executable_from_path() {
        // `sh` is present on every POSIX system that runs these tests.
        assert!(resolve_on_path("sh").is_some());
    }
}
