use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    ffi::OsString,
    mem::{size_of, size_of_val, zeroed},
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
    ptr::null_mut,
    thread,
    time::{Duration, Instant},
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM},
    Security::{EqualSid, GetTokenInformation, TokenSessionId, TokenUser, TOKEN_QUERY, TOKEN_USER},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        Threading::{
            GetCurrentProcess, GetCurrentProcessId, IsProcessCritical, OpenProcess,
            OpenProcessToken, QueryFullProcessImageNameW, TerminateProcess,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        },
    },
    UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE},
};

use crate::{
    ApplicationProcessCloseMode, ApplicationProcessCloseResult, ApplicationProcessTarget,
    PlatformCancellation, PlatformError, PlatformResult, RunningProcessIdentity,
};

use super::path_identity;

const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const FORCE_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const CLOSE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const CLOSE_QUIESCENCE_INTERVAL: Duration = Duration::from_millis(500);
const MAX_EXECUTABLE_PATH_UNITS: usize = 32_768;

#[derive(Debug, Clone)]
struct ProcessInstance {
    pid: u32,
    executable_name: String,
    executable_path: Option<PathBuf>,
    executable_path_failure: Option<ProcessCloseRequestFailure>,
    owner_scope: ProcessOwnerScope,
    owner_failure: Option<ProcessCloseRequestFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessOwnerScope {
    CurrentSession,
    SameUserOtherSession,
    ForeignUser,
    Unknown,
}

#[derive(Debug, Clone)]
struct ProcessIdentity {
    executable_path: Option<PathBuf>,
    executable_path_failure: Option<ProcessCloseRequestFailure>,
    owner_scope: ProcessOwnerScope,
    owner_failure: Option<ProcessCloseRequestFailure>,
}

/// TOKEN_USER contains a pointer into this aligned allocation. The allocation
/// must stay alive while the SID is compared.
struct ProcessOwner {
    user: Vec<usize>,
    session: u32,
}

impl ProcessOwner {
    fn read(process: HANDLE) -> Result<Self, ProcessCloseRequestFailure> {
        let mut token = null_mut();
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(ProcessCloseRequestFailure::new(
                "open_process_token",
                Some(unsafe { GetLastError() }),
            ));
        }
        let token = OwnedHandle(token);
        let mut required = 0;
        unsafe {
            GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut required);
        }
        if !(size_of::<TOKEN_USER>() as u32..=65_536).contains(&required) {
            return Err(ProcessCloseRequestFailure::new(
                "query_token_user_size",
                Some(unsafe { GetLastError() }),
            ));
        }
        let mut user = vec![0_usize; (required as usize).div_ceil(size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                user.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(ProcessCloseRequestFailure::new(
                "query_token_user",
                Some(unsafe { GetLastError() }),
            ));
        }
        let mut session = 0_u32;
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenSessionId,
                (&mut session as *mut u32).cast(),
                size_of_val(&session) as u32,
                &mut required,
            )
        } == 0
        {
            return Err(ProcessCloseRequestFailure::new(
                "query_token_session",
                Some(unsafe { GetLastError() }),
            ));
        }
        Ok(Self { user, session })
    }

    fn scope_of(&self, other: &Self) -> ProcessOwnerScope {
        let same_owner = unsafe {
            EqualSid(
                (*(self.user.as_ptr().cast::<TOKEN_USER>())).User.Sid,
                (*(other.user.as_ptr().cast::<TOKEN_USER>())).User.Sid,
            ) != 0
        };
        if !same_owner {
            ProcessOwnerScope::ForeignUser
        } else if self.session == other.session {
            ProcessOwnerScope::CurrentSession
        } else {
            ProcessOwnerScope::SameUserOtherSession
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ProcessCloseRequestOutcome {
    Requested,
    NotRequested {
        stage: &'static str,
        os_error_code: Option<u32>,
    },
}

impl ProcessCloseRequestOutcome {
    const fn was_requested(self) -> bool {
        matches!(self, Self::Requested)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessCloseRequestFailure {
    stage: &'static str,
    os_error_code: Option<u32>,
}

impl ProcessCloseRequestFailure {
    const fn new(stage: &'static str, os_error_code: Option<u32>) -> Self {
        Self {
            stage,
            os_error_code,
        }
    }

    fn stable_reason(self) -> String {
        match self.os_error_code {
            Some(code) => format!("{}:{code}", self.stage),
            None => self.stage.to_string(),
        }
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

pub(super) fn close(
    target: &ApplicationProcessTarget,
    mode: ApplicationProcessCloseMode,
) -> PlatformResult<ApplicationProcessCloseResult> {
    close_many(std::slice::from_ref(target), mode)
        .into_iter()
        .next()
        .expect("a single process-close target must produce one result")
}

/// Uses one ToolHelp snapshot per polling interval and one deadline for the
/// complete selection. This avoids serial five-second waits and thousands of
/// redundant image-path queries when many applications show save prompts.
pub(super) fn close_many(
    targets: &[ApplicationProcessTarget],
    mode: ApplicationProcessCloseMode,
) -> Vec<PlatformResult<ApplicationProcessCloseResult>> {
    if targets.is_empty() {
        return Vec::new();
    }
    let validation_errors = targets
        .iter()
        .map(|target| validate_target(target).err())
        .collect::<Vec<_>>();
    if validation_errors.iter().all(Option::is_some) {
        return validation_errors
            .into_iter()
            .map(|error| Err(error.expect("every target was invalid")))
            .collect();
    }

    let initial_snapshot = match process_snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => return snapshot_error_results(&validation_errors, error),
    };
    let mut matched = targets
        .iter()
        .zip(&validation_errors)
        .map(|(target, error)| {
            if error.is_none() {
                matching_processes_in_snapshot(target, &initial_snapshot)
            } else {
                Vec::new()
            }
        })
        .collect::<Vec<_>>();

    let mut requests = HashMap::new();
    let mut attempted_processes = HashSet::new();
    for (target, processes) in targets.iter().zip(&matched) {
        request_process_close(
            target,
            processes,
            mode,
            &mut requests,
            &mut attempted_processes,
        );
    }

    let timeout = match mode {
        ApplicationProcessCloseMode::Graceful => GRACEFUL_CLOSE_TIMEOUT,
        ApplicationProcessCloseMode::Force => FORCE_CLOSE_TIMEOUT,
    };
    let deadline = Instant::now() + timeout;
    let mut empty_since = None;
    let final_snapshot = loop {
        let snapshot = match process_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => return snapshot_error_results(&validation_errors, error),
        };
        let remaining = targets
            .iter()
            .zip(&validation_errors)
            .map(|(target, error)| {
                if error.is_none() {
                    matching_processes_in_snapshot(target, &snapshot)
                } else {
                    Vec::new()
                }
            })
            .collect::<Vec<_>>();
        let has_remaining = remaining.iter().any(|processes| !processes.is_empty());
        let now = Instant::now();
        if !has_remaining {
            // Edge background mode and Startup Boost can replace the browser
            // process family shortly after the visible window exits. Require a
            // bounded quiet interval before reporting success so cleanup does
            // not race a delayed helper restart.
            let since = empty_since.get_or_insert(now);
            if now.duration_since(*since) >= CLOSE_QUIESCENCE_INTERVAL || now >= deadline {
                break snapshot;
            }
            thread::sleep(CLOSE_POLL_INTERVAL);
            continue;
        }
        empty_since = None;
        if now >= deadline {
            break snapshot;
        }
        if mode == ApplicationProcessCloseMode::Force {
            // Chromium browsers may replace helper processes after their main
            // process exits. Continue terminating newly observed instances
            // until the process family is quiescent or the bounded deadline is
            // reached; each PID is still attempted at most once.
            for ((target, observed), processes) in targets.iter().zip(&mut matched).zip(&remaining)
            {
                for process in processes {
                    if !observed
                        .iter()
                        .any(|candidate| candidate.pid == process.pid)
                    {
                        observed.push(process.clone());
                    }
                }
                request_process_close(
                    target,
                    processes,
                    mode,
                    &mut requests,
                    &mut attempted_processes,
                );
            }
        }
        thread::sleep(CLOSE_POLL_INTERVAL);
    };

    let remaining_processes = targets
        .iter()
        .zip(&validation_errors)
        .filter(|(_, error)| error.is_none())
        .flat_map(|(target, _)| matching_processes_in_snapshot(target, &final_snapshot))
        .map(|process| (process.pid, process))
        .collect::<BTreeMap<_, _>>();
    log_remaining_process_diagnostics(mode, &requests, &attempted_processes, &remaining_processes);
    let remaining_process_names = remaining_processes
        .values()
        .map(|process| process.executable_name.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(",");
    let remaining_current_session_count = remaining_processes
        .values()
        .filter(|process| process.owner_scope == ProcessOwnerScope::CurrentSession)
        .count();
    let remaining_same_user_other_session_count = remaining_processes
        .values()
        .filter(|process| process.owner_scope == ProcessOwnerScope::SameUserOtherSession)
        .count();
    let remaining_unknown_owner_count = remaining_processes
        .values()
        .filter(|process| process.owner_scope == ProcessOwnerScope::Unknown)
        .count();
    log::info!(
        "windows_application_close_finished mode={} observed_process_count={} attempted_process_count={} requested_process_count={} remaining_process_count={} remaining_current_session_count={} remaining_same_user_other_session_count={} remaining_unknown_owner_count={} remaining_process_names={}",
        close_mode_code(mode),
        requests.len(),
        attempted_processes.len(),
        requests
            .values()
            .filter(|outcome| outcome.was_requested())
            .count(),
        remaining_processes.len(),
        remaining_current_session_count,
        remaining_same_user_other_session_count,
        remaining_unknown_owner_count,
        crate::diagnostics::text(&remaining_process_names)
    );

    targets
        .iter()
        .zip(validation_errors)
        .zip(matched)
        .map(|((target, validation_error), matched)| {
            if let Some(error) = validation_error {
                return Err(error);
            }
            let remaining = matching_processes_in_snapshot(target, &final_snapshot);
            Ok(ApplicationProcessCloseResult {
                matched_process_count: matched.len() as u64,
                requested_process_count: matched
                    .iter()
                    .filter(|process| {
                        requests
                            .get(&process.pid)
                            .copied()
                            .is_some_and(ProcessCloseRequestOutcome::was_requested)
                    })
                    .count() as u64,
                remaining_processes: unique_executable_names(&remaining),
            })
        })
        .collect()
}

fn request_process_close(
    target: &ApplicationProcessTarget,
    processes: &[ProcessInstance],
    mode: ApplicationProcessCloseMode,
    requests: &mut HashMap<u32, ProcessCloseRequestOutcome>,
    attempted_processes: &mut HashSet<u32>,
) {
    for process in processes {
        if attempted_processes.contains(&process.pid) {
            continue;
        }
        // Unknown ownership or a missing image path is sufficient to keep a
        // destructive operation blocked, but it must never authorize closing
        // a process outside the current user session or installation.
        let Some(failure) = process_authorization_failure(target, process) else {
            attempted_processes.insert(process.pid);
            let outcome = match mode {
                ApplicationProcessCloseMode::Graceful => request_graceful_close(process),
                ApplicationProcessCloseMode::Force => request_force_close(process),
            };
            if mode == ApplicationProcessCloseMode::Force {
                log::info!(
                    "windows_process_force_close pid={} path={} outcome={outcome:?}",
                    process.pid,
                    crate::diagnostics::text(
                        &process
                            .executable_path
                            .as_deref()
                            .unwrap_or_else(|| Path::new(""))
                            .display()
                    )
                );
            }
            requests.insert(process.pid, outcome);
            continue;
        };
        requests
            .entry(process.pid)
            .or_insert(ProcessCloseRequestOutcome::NotRequested {
                stage: failure.stage,
                os_error_code: failure.os_error_code,
            });
    }
}

fn log_remaining_process_diagnostics(
    mode: ApplicationProcessCloseMode,
    requests: &HashMap<u32, ProcessCloseRequestOutcome>,
    attempted_processes: &HashSet<u32>,
    remaining_processes: &BTreeMap<u32, ProcessInstance>,
) {
    if remaining_processes.is_empty() {
        return;
    }
    let mut reasons = BTreeMap::<String, u64>::new();
    let mut native_errors = BTreeMap::<u32, String>::new();
    for pid in remaining_processes.keys() {
        let key = match requests.get(pid) {
            Some(ProcessCloseRequestOutcome::NotRequested {
                stage,
                os_error_code,
            }) => {
                if let Some(code) = os_error_code {
                    native_errors.entry(*code).or_insert_with(|| {
                        std::io::Error::from_raw_os_error(*code as i32).to_string()
                    });
                }
                ProcessCloseRequestFailure::new(stage, *os_error_code).stable_reason()
            }
            Some(ProcessCloseRequestOutcome::Requested) => {
                "request_accepted_process_remained".to_string()
            }
            None => "not_attempted_before_deadline".to_string(),
        };
        *reasons.entry(key).or_default() += 1;
    }
    let native_error_details = native_errors
        .into_iter()
        .map(|(code, message)| format!("{code}={message}"))
        .collect::<Vec<_>>()
        .join(";");
    let requested_count = requests
        .values()
        .filter(|outcome| outcome.was_requested())
        .count();
    let mode_code = close_mode_code(mode);
    if mode == ApplicationProcessCloseMode::Force {
        log::warn!(
            "windows_application_close_incomplete mode={} attempted_process_count={} requested_process_count={} remaining_process_count={} remaining_reasons={:?} native_error_details={}",
            mode_code,
            attempted_processes.len(),
            requested_count,
            remaining_processes.len(),
            reasons,
            crate::diagnostics::text(&native_error_details)
        );
    } else {
        log::info!(
            "windows_application_close_pending mode={} attempted_process_count={} requested_process_count={} remaining_process_count={} remaining_reasons={:?} native_error_details={}",
            mode_code,
            attempted_processes.len(),
            requested_count,
            remaining_processes.len(),
            reasons,
            crate::diagnostics::text(&native_error_details)
        );
    }
}

const fn close_mode_code(mode: ApplicationProcessCloseMode) -> &'static str {
    match mode {
        ApplicationProcessCloseMode::Graceful => "graceful",
        ApplicationProcessCloseMode::Force => "force",
    }
}

fn validate_target(target: &ApplicationProcessTarget) -> PlatformResult<()> {
    if target.executable_names.is_empty() && target.executable_paths.is_empty() {
        return Err(PlatformError::operation_failed(
            "application process target contains no identity",
        ));
    }
    Ok(())
}

fn request_graceful_close(process: &ProcessInstance) -> ProcessCloseRequestOutcome {
    let _process_handle = match open_verified_process(process, 0) {
        Ok(handle) => handle,
        Err(failure) => {
            return ProcessCloseRequestOutcome::NotRequested {
                stage: failure.stage,
                os_error_code: failure.os_error_code,
            };
        }
    };
    let mut context = WindowCloseContext {
        pid: process.pid,
        posted: false,
    };
    unsafe {
        EnumWindows(
            Some(post_close_to_process_window),
            (&mut context as *mut WindowCloseContext) as LPARAM,
        );
    }
    if context.posted {
        ProcessCloseRequestOutcome::Requested
    } else {
        ProcessCloseRequestOutcome::NotRequested {
            stage: "top_level_window_unavailable",
            os_error_code: None,
        }
    }
}

struct WindowCloseContext {
    pid: u32,
    posted: bool,
}

unsafe extern "system" fn post_close_to_process_window(window: HWND, context: LPARAM) -> i32 {
    let context = unsafe { &mut *(context as *mut WindowCloseContext) };
    let mut window_pid = 0_u32;
    unsafe {
        GetWindowThreadProcessId(window, &mut window_pid);
    }
    if window_pid == context.pid && unsafe { PostMessageW(window, WM_CLOSE, 0, 0) } != 0 {
        context.posted = true;
    }
    1
}

fn request_force_close(process: &ProcessInstance) -> ProcessCloseRequestOutcome {
    let handle = match open_verified_process(process, PROCESS_TERMINATE) {
        Ok(handle) => handle,
        Err(failure) => {
            return ProcessCloseRequestOutcome::NotRequested {
                stage: failure.stage,
                os_error_code: failure.os_error_code,
            };
        }
    };
    let mut critical = 0;
    if unsafe { IsProcessCritical(handle.0, &mut critical) } == 0 {
        return ProcessCloseRequestOutcome::NotRequested {
            stage: "query_process_critical",
            os_error_code: Some(unsafe { GetLastError() }),
        };
    }
    if critical != 0 {
        return ProcessCloseRequestOutcome::NotRequested {
            stage: "critical_process",
            os_error_code: None,
        };
    }
    if unsafe { TerminateProcess(handle.0, 1) } != 0 {
        ProcessCloseRequestOutcome::Requested
    } else {
        ProcessCloseRequestOutcome::NotRequested {
            stage: "terminate_process",
            os_error_code: Some(unsafe { GetLastError() }),
        }
    }
}

#[cfg(test)]
fn matching_processes(target: &ApplicationProcessTarget) -> PlatformResult<Vec<ProcessInstance>> {
    process_snapshot().map(|snapshot| matching_processes_in_snapshot(target, &snapshot))
}

fn matching_processes_in_snapshot(
    target: &ApplicationProcessTarget,
    snapshot: &[ProcessInstance],
) -> Vec<ProcessInstance> {
    let names = target
        .executable_names
        .iter()
        .flat_map(|name| normalized_name_aliases(name))
        .collect::<HashSet<_>>();
    let paths = target
        .executable_paths
        .iter()
        .map(|path| normalize_path(path))
        .collect::<HashSet<_>>();
    let path_names = target
        .executable_paths
        .iter()
        .filter_map(|path| path.file_name())
        .flat_map(|name| normalized_name_aliases(&name.to_string_lossy()))
        .collect::<HashSet<_>>();
    let current_pid = unsafe { GetCurrentProcessId() };
    snapshot
        .iter()
        .filter(|process| {
            process.pid != current_pid
                && process.owner_scope != ProcessOwnerScope::ForeignUser
                && if paths.is_empty() {
                    normalized_name_aliases(&process.executable_name)
                        .iter()
                        .any(|name| names.contains(name))
                } else {
                    process.executable_path.as_deref().map_or_else(
                        || {
                            normalized_name_aliases(&process.executable_name)
                                .iter()
                                .any(|name| path_names.contains(name))
                        },
                        |path| paths.contains(&normalize_path(path)),
                    )
                }
        })
        .cloned()
        .collect()
}

fn process_authorization_failure(
    target: &ApplicationProcessTarget,
    process: &ProcessInstance,
) -> Option<ProcessCloseRequestFailure> {
    match process.owner_scope {
        ProcessOwnerScope::CurrentSession => {}
        ProcessOwnerScope::SameUserOtherSession => {
            return Some(ProcessCloseRequestFailure::new(
                "same_user_other_session",
                None,
            ));
        }
        ProcessOwnerScope::ForeignUser => {
            return Some(ProcessCloseRequestFailure::new("foreign_user", None));
        }
        ProcessOwnerScope::Unknown => {
            return Some(process.owner_failure.unwrap_or_else(|| {
                ProcessCloseRequestFailure::new("process_owner_unavailable", None)
            }));
        }
    }
    if target.executable_paths.is_empty() {
        return None;
    }
    let Some(process_path) = process.executable_path.as_deref() else {
        return Some(process.executable_path_failure.unwrap_or_else(|| {
            ProcessCloseRequestFailure::new("process_image_unavailable", None)
        }));
    };
    if target
        .executable_paths
        .iter()
        .any(|target_path| normalize_path(target_path) == normalize_path(process_path))
    {
        None
    } else {
        Some(ProcessCloseRequestFailure::new(
            "target_path_mismatch",
            None,
        ))
    }
}

fn snapshot_error_results(
    validation_errors: &[Option<PlatformError>],
    snapshot_error: PlatformError,
) -> Vec<PlatformResult<ApplicationProcessCloseResult>> {
    validation_errors
        .iter()
        .map(|validation_error| {
            Err(validation_error
                .clone()
                .unwrap_or_else(|| snapshot_error.clone()))
        })
        .collect()
}

/// Keeps a handle to the original process object while posting `WM_CLOSE` or
/// terminating it. Windows cannot reuse that PID while this handle remains
/// alive, and the queried image path must still match the captured identity.
fn open_verified_process(
    process: &ProcessInstance,
    additional_access: u32,
) -> Result<OwnedHandle, ProcessCloseRequestFailure> {
    if process.pid == 0 || process.pid == unsafe { GetCurrentProcessId() } {
        return Err(ProcessCloseRequestFailure::new("invalid_process", None));
    }
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | additional_access,
            0,
            process.pid,
        )
    };
    if handle.is_null() {
        return Err(ProcessCloseRequestFailure::new(
            "open_process",
            Some(unsafe { GetLastError() }),
        ));
    }
    let handle = OwnedHandle(handle);
    let current_owner = ProcessOwner::read(unsafe { GetCurrentProcess() })?;
    let opened_owner = ProcessOwner::read(handle.0)?;
    match current_owner.scope_of(&opened_owner) {
        ProcessOwnerScope::CurrentSession => {}
        ProcessOwnerScope::SameUserOtherSession => {
            return Err(ProcessCloseRequestFailure::new(
                "process_session_changed",
                None,
            ));
        }
        ProcessOwnerScope::ForeignUser => {
            return Err(ProcessCloseRequestFailure::new(
                "process_owner_changed",
                None,
            ));
        }
        ProcessOwnerScope::Unknown => {
            return Err(ProcessCloseRequestFailure::new(
                "process_owner_unavailable",
                None,
            ));
        }
    }
    let current_path = query_executable_path_result(handle.0)
        .map_err(|code| ProcessCloseRequestFailure::new("query_process_image", Some(code)))?;
    let identity_matches = process
        .executable_path
        .as_deref()
        .map(|path| normalize_path(path) == normalize_path(&current_path))
        .unwrap_or_else(|| {
            current_path.file_name().is_some_and(|name| {
                normalize_name(&name.to_string_lossy()) == normalize_name(&process.executable_name)
            })
        });
    if identity_matches {
        Ok(handle)
    } else {
        Err(ProcessCloseRequestFailure::new(
            "process_identity_changed",
            None,
        ))
    }
}

fn process_snapshot() -> PlatformResult<Vec<ProcessInstance>> {
    process_snapshot_with_cancellation(None)
}

fn process_snapshot_with_cancellation(
    cancellation: Option<&PlatformCancellation>,
) -> PlatformResult<Vec<ProcessInstance>> {
    if cancellation.is_some_and(PlatformCancellation::is_cancelled) {
        return Err(PlatformError::operation_failed(
            "windows process snapshot capture was cancelled",
        ));
    }
    let current_owner = ProcessOwner::read(unsafe { GetCurrentProcess() }).map_err(|failure| {
        PlatformError::operation_failed(format!(
            "windows current process ownership is unavailable: {}",
            failure.stable_reason()
        ))
    })?;
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(PlatformError::operation_failed(
            "windows process control snapshot creation failed",
        ));
    }
    let snapshot = OwnedHandle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    if unsafe { Process32FirstW(snapshot.0, &mut entry) } == 0 {
        return Err(PlatformError::operation_failed(
            "windows process control snapshot enumeration failed",
        ));
    }

    let mut processes = Vec::new();
    loop {
        if cancellation.is_some_and(PlatformCancellation::is_cancelled) {
            return Err(PlatformError::operation_failed(
                "windows process snapshot capture was cancelled",
            ));
        }
        let executable_name = wide_c_string(&entry.szExeFile);
        if entry.th32ProcessID != 0 && !executable_name.is_empty() {
            let identity = process_identity(entry.th32ProcessID, &current_owner);
            processes.push(ProcessInstance {
                pid: entry.th32ProcessID,
                executable_path: identity.executable_path,
                executable_path_failure: identity.executable_path_failure,
                executable_name,
                owner_scope: identity.owner_scope,
                owner_failure: identity.owner_failure,
            });
        }
        if unsafe { Process32NextW(snapshot.0, &mut entry) } == 0 {
            break;
        }
    }
    Ok(processes)
}

fn process_identity(pid: u32, current_owner: &ProcessOwner) -> ProcessIdentity {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        let failure = ProcessCloseRequestFailure::new(
            "open_process_identity",
            Some(unsafe { GetLastError() }),
        );
        return ProcessIdentity {
            executable_path: None,
            executable_path_failure: Some(failure),
            owner_scope: ProcessOwnerScope::Unknown,
            owner_failure: Some(failure),
        };
    }
    let handle = OwnedHandle(handle);
    let (executable_path, executable_path_failure) = match query_executable_path_result(handle.0) {
        Ok(path) => (Some(path), None),
        Err(code) => (
            None,
            Some(ProcessCloseRequestFailure::new(
                "query_process_image",
                Some(code),
            )),
        ),
    };
    let (owner_scope, owner_failure) = match ProcessOwner::read(handle.0) {
        Ok(owner) => (current_owner.scope_of(&owner), None),
        Err(failure) => (ProcessOwnerScope::Unknown, Some(failure)),
    };
    ProcessIdentity {
        executable_path,
        executable_path_failure,
        owner_scope,
        owner_failure,
    }
}

pub(super) fn running_process_identities(
    cancellation: &PlatformCancellation,
) -> PlatformResult<Vec<RunningProcessIdentity>> {
    process_snapshot_with_cancellation(Some(cancellation)).map(|processes| {
        processes
            .into_iter()
            .filter(|process| process.owner_scope != ProcessOwnerScope::ForeignUser)
            .map(|process| RunningProcessIdentity {
                executable_name: process.executable_name,
                executable_path: process.executable_path,
            })
            .collect()
    })
}

#[cfg(test)]
fn executable_path(pid: u32) -> Option<PathBuf> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let handle = OwnedHandle(handle);
    query_executable_path(handle.0)
}

#[cfg(test)]
fn query_executable_path(handle: HANDLE) -> Option<PathBuf> {
    query_executable_path_result(handle).ok()
}

fn query_executable_path_result(handle: HANDLE) -> Result<PathBuf, u32> {
    let mut buffer = vec![0_u16; MAX_EXECUTABLE_PATH_UNITS];
    let mut length = buffer.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
        return Err(unsafe { GetLastError() });
    }
    buffer.truncate(length as usize);
    Ok(PathBuf::from(OsString::from_wide(&buffer)))
}

fn wide_c_string(units: &[u16]) -> String {
    let length = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    OsString::from_wide(&units[..length])
        .to_string_lossy()
        .into_owned()
}

fn unique_executable_names(processes: &[ProcessInstance]) -> Vec<String> {
    let mut names = processes
        .iter()
        .map(|process| process.executable_name.clone())
        .collect::<Vec<_>>();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    names
}

fn normalize_name(name: &str) -> String {
    Path::new(name.trim())
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}

fn normalized_name_aliases(name: &str) -> Vec<String> {
    let normalized = normalize_name(name);
    let mut aliases = vec![normalized.clone()];
    if let Some(stem) = normalized.strip_suffix(".exe") {
        aliases.push(stem.to_string());
    }
    aliases
}

fn normalize_path(path: &Path) -> String {
    path_identity::comparison_key(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env, fs,
        process::{Command, Stdio},
    };

    #[test]
    fn path_normalization_is_case_and_separator_insensitive() {
        assert_eq!(
            normalize_path(Path::new("C:/Program Files/Example/App.exe")),
            normalize_path(Path::new(r"c:\Program Files\Example\APP.EXE"))
        );
    }

    #[test]
    fn executable_extension_is_an_optional_process_alias() {
        let target = normalized_name_aliases("MangoDiskCloseUninstallFixture");
        let process = normalized_name_aliases("MangoDiskCloseUninstallFixture.exe");
        assert!(process.iter().any(|name| target.contains(name)));
    }

    #[test]
    fn exact_paths_disable_same_name_fallback() {
        let snapshot = vec![ProcessInstance {
            pid: 42,
            executable_name: "SharedHelper.exe".to_string(),
            executable_path: Some(PathBuf::from(r"C:\Other\SharedHelper.exe")),
            executable_path_failure: None,
            owner_scope: ProcessOwnerScope::CurrentSession,
            owner_failure: None,
        }];
        let target = ApplicationProcessTarget {
            executable_names: vec!["SharedHelper.exe".to_string()],
            executable_paths: vec![PathBuf::from(r"C:\Expected\SharedHelper.exe")],
        };

        assert!(matching_processes_in_snapshot(&target, &snapshot).is_empty());
    }

    #[test]
    fn unresolved_path_blocks_without_authorizing_same_name_process() {
        let process = ProcessInstance {
            pid: 42,
            executable_name: "SharedHelper.exe".to_string(),
            executable_path: None,
            executable_path_failure: Some(ProcessCloseRequestFailure::new(
                "query_process_image",
                Some(5),
            )),
            owner_scope: ProcessOwnerScope::CurrentSession,
            owner_failure: None,
        };
        let target = ApplicationProcessTarget {
            executable_names: Vec::new(),
            executable_paths: vec![PathBuf::from(r"C:\Expected\SharedHelper.exe")],
        };

        assert_eq!(
            matching_processes_in_snapshot(&target, std::slice::from_ref(&process)).len(),
            1
        );
        assert_eq!(
            process_authorization_failure(&target, &process),
            Some(ProcessCloseRequestFailure::new(
                "query_process_image",
                Some(5)
            ))
        );
        assert_eq!(
            process_authorization_failure(
                &ApplicationProcessTarget {
                    executable_names: vec!["SharedHelper.exe".to_string()],
                    executable_paths: Vec::new(),
                },
                &process
            ),
            None
        );
    }

    #[test]
    fn foreign_user_process_does_not_block_current_user_cleanup() {
        let process = ProcessInstance {
            pid: 42,
            executable_name: "msedge.exe".to_string(),
            executable_path: Some(PathBuf::from(
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            )),
            executable_path_failure: None,
            owner_scope: ProcessOwnerScope::ForeignUser,
            owner_failure: None,
        };
        let target = ApplicationProcessTarget {
            executable_names: vec!["msedge.exe".to_string()],
            executable_paths: Vec::new(),
        };

        assert!(matching_processes_in_snapshot(&target, &[process]).is_empty());
    }

    #[test]
    fn same_user_other_session_blocks_without_authorizing_termination() {
        let process = ProcessInstance {
            pid: 42,
            executable_name: "msedge.exe".to_string(),
            executable_path: Some(PathBuf::from(
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            )),
            executable_path_failure: None,
            owner_scope: ProcessOwnerScope::SameUserOtherSession,
            owner_failure: None,
        };
        let target = ApplicationProcessTarget {
            executable_names: vec!["msedge.exe".to_string()],
            executable_paths: Vec::new(),
        };

        assert_eq!(
            matching_processes_in_snapshot(&target, std::slice::from_ref(&process)).len(),
            1
        );
        assert_eq!(
            process_authorization_failure(&target, &process),
            Some(ProcessCloseRequestFailure::new(
                "same_user_other_session",
                None
            ))
        );
    }

    #[test]
    fn unknown_owner_blocks_cleanup_without_authorizing_termination() {
        let process = ProcessInstance {
            pid: 42,
            executable_name: "msedge.exe".to_string(),
            executable_path: None,
            executable_path_failure: Some(ProcessCloseRequestFailure::new(
                "open_process_identity",
                Some(5),
            )),
            owner_scope: ProcessOwnerScope::Unknown,
            owner_failure: Some(ProcessCloseRequestFailure::new(
                "open_process_identity",
                Some(5),
            )),
        };
        let target = ApplicationProcessTarget {
            executable_names: vec!["msedge.exe".to_string()],
            executable_paths: Vec::new(),
        };

        assert_eq!(
            matching_processes_in_snapshot(&target, std::slice::from_ref(&process)).len(),
            1
        );
        assert_eq!(
            process_authorization_failure(&target, &process),
            Some(ProcessCloseRequestFailure::new(
                "open_process_identity",
                Some(5)
            ))
        );
    }

    #[test]
    #[ignore = "launches and closes a real native window"]
    fn closes_spawned_window_gracefully() {
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-STA",
                "-Command",
                "Add-Type -AssemblyName PresentationFramework; $window = New-Object System.Windows.Window; $window.Title = 'MangoDisk Process Close Test'; $window.Width = 420; $window.Height = 140; [void]$window.ShowDialog()",
            ])
            .spawn()
            .expect("the test window should start");
        let process_path = wait_for_spawned_process(&mut child);
        // Give WPF time to create its top-level HWND after the process itself
        // becomes visible in the ToolHelp snapshot.
        thread::sleep(Duration::from_millis(500));
        let result = close(
            &ApplicationProcessTarget {
                executable_names: Vec::new(),
                executable_paths: vec![process_path],
            },
            ApplicationProcessCloseMode::Graceful,
        );

        // Reap the isolated process even when an assertion below fails. A
        // successful WM_CLOSE usually makes `kill` a harmless no-op.
        let _ = child.kill();
        let _ = child.wait();

        let result = result.expect("the close request should succeed");
        assert_eq!(result.matched_process_count, 1);
        assert_eq!(result.requested_process_count, 1);
        assert!(result.remaining_processes.is_empty());
    }

    #[test]
    #[ignore = "launches and force-terminates a real child process"]
    fn force_closes_spawned_command() {
        let mut child = Command::new("ping.exe")
            .args(["-t", "127.0.0.1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the test process should start");
        let process_path = wait_for_spawned_process(&mut child);
        let result = close(
            &ApplicationProcessTarget {
                executable_names: Vec::new(),
                executable_paths: vec![process_path],
            },
            ApplicationProcessCloseMode::Force,
        );

        let _ = child.kill();
        let _ = child.wait();

        let result = result.expect("the force-close request should succeed");
        assert_eq!(result.matched_process_count, 1);
        assert_eq!(result.requested_process_count, 1);
        assert!(result.remaining_processes.is_empty());
    }

    #[test]
    #[ignore = "launches and force-terminates an installed Microsoft Edge process family"]
    fn force_closes_spawned_edge_process_family() {
        let edge_path = [
            env::var_os("ProgramFiles(x86)"),
            env::var_os("ProgramFiles"),
        ]
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .map(|root| root.join(r"Microsoft\Edge\Application\msedge.exe"))
        .find(|path| path.is_file())
        .expect("Microsoft Edge must be installed for this native integration test");
        let profile = env::temp_dir().join(format!("mangodisk-edge-close-{}", unsafe {
            GetCurrentProcessId()
        }));
        fs::create_dir_all(&profile).expect("the isolated Edge profile should be created");
        let profile_argument = format!("--user-data-dir={}", profile.display());
        let mut child = Command::new(&edge_path)
            .args([profile_argument.as_str(), "--no-first-run", "about:blank"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Microsoft Edge should start");
        let target = ApplicationProcessTarget {
            executable_names: Vec::new(),
            executable_paths: vec![edge_path],
        };
        wait_for_matching_processes(&target, 2);
        // Cleanup can submit several rules backed by the same Edge process
        // family. Exercise the shared-PID path so one target cannot suppress a
        // valid request or make another target report a false remainder.
        let results = close_many(
            &[target.clone(), target],
            ApplicationProcessCloseMode::Force,
        );

        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_dir_all(&profile);

        assert_eq!(results.len(), 2);
        for result in results {
            let result = result.expect("the Edge force-close request should complete");
            assert!(result.matched_process_count >= 2);
            assert!(result.requested_process_count >= 1);
            assert!(result.remaining_processes.is_empty());
        }
    }

    fn wait_for_spawned_process(child: &mut std::process::Child) -> PathBuf {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(path) = executable_path(child.id()) {
                if matching_processes(&ApplicationProcessTarget {
                    executable_names: Vec::new(),
                    executable_paths: vec![path.clone()],
                })
                .is_ok_and(|processes| processes.iter().any(|process| process.pid == child.id()))
                {
                    return path;
                }
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the test process did not become discoverable");
            }
            thread::sleep(CLOSE_POLL_INTERVAL);
        }
    }

    fn wait_for_matching_processes(target: &ApplicationProcessTarget, minimum_count: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if matching_processes(target).is_ok_and(|processes| processes.len() >= minimum_count) {
                return;
            }
            if Instant::now() >= deadline {
                panic!("the expected process family did not become discoverable");
            }
            thread::sleep(CLOSE_POLL_INTERVAL);
        }
    }
}
