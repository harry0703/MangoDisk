//! Explicit termination of a freshly resolved application, never arbitrary PIDs.
use super::{
    quit::{self, QuitTarget},
    running_identity,
};
use crate::{CoreError, CoreResult};
use mangodisk_platform::system_resources::memory::{MemorySampler, MemorySource, ProcessMemory};
use serde::Serialize;
use std::{path::Path, time::Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationForceQuitStatus {
    Completed,
    StillRunning,
    Unavailable,
    Blocked,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationForceQuitResult {
    pub schema_version: u32,
    pub operation_id: String,
    pub status: ApplicationForceQuitStatus,
    pub matched_process_count: u64,
    pub requested_process_count: u64,
    /// Unique native names, not a PID count or proof that every child belongs to this app.
    pub remaining_processes: Vec<String>,
}

pub fn execute(application_id: &str) -> CoreResult<ApplicationForceQuitResult> {
    if application_id.len() != 64 || !application_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CoreError::invalid_input("invalid application identity"));
    }
    let operation_id = uuid::Uuid::new_v4().to_string();
    let started = Instant::now();
    let processes = MemorySampler::default()
        .sample(true).inspect_err(|error| {
            log::warn!("application_force_quit_failed operation_id={operation_id} application_id={application_id} stage=resolve_processes error={} elapsed_ms={}",
                mangodisk_platform::diagnostics::text(error), started.elapsed().as_millis());
        })?
        .processes
        .unwrap_or_default();
    execute_resolved(
        application_id,
        &operation_id,
        &processes,
        std::process::id(),
        started,
        mangodisk_platform::application_force_quit::execute,
    )
}

fn execute_resolved(
    id: &str,
    operation_id: &str,
    processes: &[ProcessMemory],
    current_pid: u32,
    started: Instant,
    action: impl FnOnce(
        &Path,
    ) -> mangodisk_platform::PlatformResult<
        mangodisk_platform::ApplicationProcessCloseResult,
    >,
) -> CoreResult<ApplicationForceQuitResult> {
    let mut result = ApplicationForceQuitResult {
        schema_version: 1,
        operation_id: operation_id.into(),
        status: ApplicationForceQuitStatus::Unavailable,
        matched_process_count: 0,
        requested_process_count: 0,
        remaining_processes: Vec::new(),
    };
    let own_path = processes
        .iter()
        .find(|process| process.pid == current_pid)
        .and_then(|process| process.executable.as_deref())
        .map(running_identity::application_path);
    let target = match quit::resolve_target(id, processes, current_pid) {
        QuitTarget::Available(path)
            if running_identity::can_force_quit(&path, own_path.as_deref()) =>
        {
            Some(path)
        }
        QuitTarget::Available(path) => {
            log::info!("application_force_quit_blocked operation_id={operation_id} path={} reason=protected_target_or_unknown_own_identity", mangodisk_platform::diagnostics::text(&path.display()));
            result.status = ApplicationForceQuitStatus::Blocked;
            None
        }
        QuitTarget::Unsupported(path) => {
            log::info!("application_force_quit_blocked operation_id={operation_id} path={} reason=current_application_or_unquittable_target", mangodisk_platform::diagnostics::text(&path.as_deref().unwrap_or_else(|| Path::new("")).display()));
            result.status = ApplicationForceQuitStatus::Blocked;
            None
        }
        QuitTarget::Unavailable => None,
    };
    if let Some(path) = target.as_deref() {
        log::info!("application_force_quit_started operation_id={operation_id} path={} inspected_process_count={}",
            mangodisk_platform::diagnostics::text(&path.display()), processes.len());
        let native = action(path).inspect_err(|error| {
            log::warn!("application_force_quit_failed operation_id={operation_id} path={} stage=platform_close code={:?} error={} elapsed_ms={}",
                mangodisk_platform::diagnostics::text(&path.display()), error.code(), mangodisk_platform::diagnostics::text(error), started.elapsed().as_millis());
        })?;
        result.matched_process_count = native.matched_process_count;
        result.requested_process_count = native.requested_process_count;
        result.remaining_processes = native.remaining_processes;
        result.status = if !result.remaining_processes.is_empty() {
            ApplicationForceQuitStatus::StillRunning
        } else if result.matched_process_count == 0 {
            ApplicationForceQuitStatus::Unavailable
        } else {
            ApplicationForceQuitStatus::Completed
        };
    }
    log::info!("application_force_quit_finished operation_id={operation_id} application_id={id} status={:?} path={} matched_process_count={} requested_process_count={} remaining_process_names={} elapsed_ms={}",
        result.status, mangodisk_platform::diagnostics::text(&target.as_deref().map(|path|path.display().to_string()).unwrap_or_default()),
        result.matched_process_count, result.requested_process_count,
        mangodisk_platform::diagnostics::text(&result.remaining_processes.join(",")), started.elapsed().as_millis());
    Ok(result)
}

#[cfg(all(test, any(windows, target_os = "macos")))]
#[path = "force_quit_tests.rs"]
mod tests;
