//! Resolve quit targets from the full current process set, never a displayed ranking.
use super::{
    process_control::{request_application_quit, ApplicationQuitStatus},
    running_identity,
};
use crate::{CoreError, CoreResult};
use mangodisk_platform::system_resources::memory::{MemorySampler, MemorySource, ProcessMemory};

pub fn request(application_id: &str) -> CoreResult<ApplicationQuitStatus> {
    if application_id.len() != 64 || !application_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CoreError::invalid_input("invalid application identity"));
    }
    let started = std::time::Instant::now();
    let processes = MemorySampler::default()
        .sample(true)?
        .processes
        .unwrap_or_default();
    let status = resolve_and_request(
        application_id,
        &processes,
        std::process::id(),
        request_application_quit,
    )?;
    log::info!(
        "application_quit_requested status={status:?} inspected={} elapsed_ms={}",
        processes.len(),
        started.elapsed().as_millis()
    );
    Ok(status)
}

fn resolve_and_request(
    id: &str,
    processes: &[ProcessMemory],
    current_pid: u32,
    request: impl FnOnce(&std::path::Path) -> CoreResult<ApplicationQuitStatus>,
) -> CoreResult<ApplicationQuitStatus> {
    match resolve_target(id, processes, current_pid) {
        QuitTarget::Available(path) => request(&path),
        QuitTarget::Unavailable => Ok(ApplicationQuitStatus::Unavailable),
        QuitTarget::Unsupported(_) => Ok(ApplicationQuitStatus::Unsupported),
    }
}

pub(super) enum QuitTarget {
    Available(std::path::PathBuf),
    Unavailable,
    Unsupported(Option<std::path::PathBuf>),
}

pub(super) fn resolve_target(
    id: &str,
    processes: &[ProcessMemory],
    current_pid: u32,
) -> QuitTarget {
    let own_path = processes
        .iter()
        .find(|process| process.pid == current_pid)
        .and_then(|process| process.executable.as_deref())
        .map(running_identity::application_path);
    for process in processes {
        let path = process
            .executable
            .as_deref()
            .map(running_identity::application_path);
        if running_identity::id(path.as_deref(), process.pid) != id {
            continue;
        }
        if let Some(path) = path
            .as_ref()
            .filter(|path| running_identity::can_quit(path, own_path.as_deref()))
        {
            return QuitTarget::Available(path.clone());
        }
        log::info!("application_quit_target_rejected process_id={} stage=validate_identity reason=not_quittable_or_current_application path={} own_path={} outcome=unchanged", process.pid,
            mangodisk_platform::diagnostics::text(&path.as_deref().unwrap_or_else(|| std::path::Path::new("")).display()),
            mangodisk_platform::diagnostics::text(&own_path.as_deref().unwrap_or_else(|| std::path::Path::new("")).display()));
        return QuitTarget::Unsupported(path);
    }
    QuitTarget::Unavailable
}

#[cfg(test)]
mod tests {
    use super::*;
    fn processes() -> Vec<ProcessMemory> {
        (1..=40)
            .map(|pid| ProcessMemory {
                pid,
                name: format!("App{pid}"),
                executable: Some(format!("/App{pid}.app/Contents/MacOS/App").into()),
                used_bytes: Some(if pid == 40 { 0 } else { 1000 - u64::from(pid) }),
                is_application: false,
            })
            .collect()
    }
    #[test]
    fn quit_ignores_rank_and_zero_memory_but_rejects_stale_and_own_identities() {
        let processes = processes();
        let id = running_identity::id(Some(std::path::Path::new("/App40.app")), 40);
        assert_eq!(
            resolve_and_request(&id, &processes, 1, |path| {
                assert_eq!(path, std::path::Path::new("/App40.app"));
                Ok(ApplicationQuitStatus::Requested)
            })
            .unwrap(),
            ApplicationQuitStatus::Requested
        );
        assert_eq!(
            resolve_and_request(&id, &processes[..39], 1, |_| panic!("stale target")).unwrap(),
            ApplicationQuitStatus::Unavailable
        );
        assert_eq!(
            resolve_and_request(&id, &processes, 40, |_| panic!("own target")).unwrap(),
            ApplicationQuitStatus::Unsupported
        );
        assert!(request("/arbitrary/path").is_err());
    }
}
