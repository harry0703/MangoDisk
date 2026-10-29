//! A fixed-current-executable UAC handoff, completed before single-instance initialization.
use std::{
    ffi::OsString,
    io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE},
        Threading::{
            CreateEventW, OpenEventW, OpenProcess, QueryFullProcessImageNameW, SetEvent,
            WaitForMultipleObjects, WaitForSingleObject, EVENT_MODIFY_STATE,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        },
    },
    UI::{
        Shell::{
            ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
            SHELLEXECUTEINFOW,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

use crate::{PlatformError, PlatformResult};

const RESTART_FLAG: &str = "--mangodisk-admin-restart-v1";
const HANDOFF_TIMEOUT_MS: u32 = 30_000;
const SYNCHRONIZE: u32 = 0x0010_0000;

struct Handle(HANDLE);
impl Handle {
    fn new(value: HANDLE, stage: &str) -> PlatformResult<Self> {
        if value.is_null() {
            Err(native_error(stage))
        } else {
            Ok(Self(value))
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn native_error(stage: &str) -> PlatformError {
    PlatformError::operation_failed(format!(
        "administrator restart stage={stage} error={}",
        io::Error::last_os_error()
    ))
}
fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
fn event_name(pid: u32, nonce: &str, phase: &str) -> Vec<u16> {
    wide(format!(
        "Local\\MangoDisk.AdminRestart.v1.{pid}.{nonce}.{phase}"
    ))
}
fn current_executable() -> PlatformResult<PathBuf> {
    std::env::current_exe().map_err(|error| {
        PlatformError::operation_failed(format!(
            "administrator restart executable lookup failed: {error}"
        ))
    })
}

/// Returns false only when the user cancels UAC. The caller must exit only after true.
/// Neither a caller-controlled executable nor inherited command-line arguments are elevated.
pub fn restart_current_application_elevated() -> PlatformResult<bool> {
    restart_with_analysis(None)
}

pub fn restart_current_application_elevated_for_analysis(root: &Path) -> PlatformResult<bool> {
    restart_with_analysis(Some(root))
}

fn analysis_arguments(root: &Path) -> PlatformResult<String> {
    let value = root
        .to_str()
        .ok_or_else(|| PlatformError::invalid_path("invalid analysis root"))?;
    if !root.is_absolute() || value.chars().any(char::is_control) || value.contains('"') {
        return Err(PlatformError::invalid_path("invalid analysis root"));
    }
    // Windows argv parsing consumes backslashes before a closing quote. Preserve a volume root.
    let trailing = value
        .chars()
        .rev()
        .take_while(|value| *value == '\\')
        .count();
    Ok(format!(
        " --analyze-directory \"{value}{}\"",
        "\\".repeat(trailing)
    ))
}

fn restart_with_analysis(root: Option<&Path>) -> PlatformResult<bool> {
    if super::windows_process_is_elevated()? {
        return Err(PlatformError::operation_failed(
            "the application is already elevated",
        ));
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|error| {
        PlatformError::operation_failed(format!("administrator restart nonce failed: {error}"))
    })?;
    let nonce: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let pid = std::process::id();
    let ready = Handle::new(
        unsafe { CreateEventW(ptr::null(), 1, 0, event_name(pid, &nonce, "ready").as_ptr()) },
        "create_ready",
    )?;
    let proceed = Handle::new(
        unsafe {
            CreateEventW(
                ptr::null(),
                1,
                0,
                event_name(pid, &nonce, "proceed").as_ptr(),
            )
        },
        "create_proceed",
    )?;
    let executable = wide(current_executable()?);
    let analysis = root
        .map(analysis_arguments)
        .transpose()?
        .unwrap_or_default();
    let arguments = wide(format!("{RESTART_FLAG} {pid} {nonce}{analysis}"));
    let verb = wide("runas");
    let com = unsafe {
        CoInitializeEx(
            ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    if com < 0 {
        return Err(PlatformError::operation_failed(format!(
            "administrator restart COM initialization failed hresult={com}"
        )));
    }
    let mut execution = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        lpVerb: verb.as_ptr(),
        lpFile: executable.as_ptr(),
        lpParameters: arguments.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..unsafe { std::mem::zeroed() }
    };
    let launched = unsafe { ShellExecuteExW(&mut execution) } != 0;
    let error = unsafe { GetLastError() };
    unsafe {
        CoUninitialize();
    }
    if !launched {
        log::info!(
            "administrator_restart_launch status={} native_code={error}",
            if error == 1223 { "cancelled" } else { "failed" }
        );
        if error == 1223 {
            return Ok(false);
        }
        return Err(PlatformError::operation_failed(format!(
            "administrator restart launch failed: {}",
            io::Error::from_raw_os_error(error as i32)
        )));
    }
    let child = Handle::new(execution.hProcess, "child_process")?;
    commit_ready_child(ready.0, child.0, proceed.0, HANDOFF_TIMEOUT_MS)?;
    Ok(true)
}

fn commit_ready_child(
    ready: HANDLE,
    child: HANDLE,
    proceed: HANDLE,
    timeout_ms: u32,
) -> PlatformResult<()> {
    let handles = [ready, child];
    let wait = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, timeout_ms) };
    if wait != WAIT_OBJECT_0 {
        return Err(PlatformError::operation_failed(format!(
            "administrator restart child did not become ready wait_code={wait}"
        )));
    }
    // Readiness wins when both handles are signaled; an exited child cannot take over.
    let child_wait = unsafe { WaitForSingleObject(child, 0) };
    if child_wait != WAIT_TIMEOUT {
        return Err(PlatformError::operation_failed(format!(
            "administrator restart child exited before commit wait_code={child_wait}"
        )));
    }
    // A separate commit event prevents a late child from restarting after a failed handoff.
    if unsafe { SetEvent(proceed) } == 0 {
        return Err(native_error("commit"));
    }
    Ok(())
}

fn parse_handoff(arguments: &[OsString]) -> PlatformResult<(u32, String)> {
    let invalid = || PlatformError::operation_failed("invalid administrator restart arguments");
    if !matches!(arguments.len(), 4 | 6) || arguments[1] != RESTART_FLAG {
        return Err(invalid());
    }
    if arguments.len() == 6 {
        if arguments[4] != "--analyze-directory" {
            return Err(invalid());
        }
        analysis_arguments(Path::new(&arguments[5]))?;
    }
    let pid = arguments[2]
        .to_str()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|pid| *pid != 0)
        .ok_or_else(invalid)?;
    let nonce = arguments[3]
        .to_str()
        .filter(|value| value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(invalid)?;
    Ok((pid, nonce.to_owned()))
}

/// Wait for the old process to release its single-instance lock before starting Tauri.
/// Invalid or unacknowledged handoffs fail closed and never create another application window.
pub fn prepare_elevated_restart_startup(
    arguments: impl IntoIterator<Item = OsString>,
) -> Option<PlatformResult<()>> {
    let arguments: Vec<_> = arguments.into_iter().collect();
    if !arguments.iter().any(|argument| argument == RESTART_FLAG) {
        return None;
    }
    Some(complete_handoff(&arguments))
}
fn complete_handoff(arguments: &[OsString]) -> PlatformResult<()> {
    let (pid, nonce) = parse_handoff(arguments)?;
    if pid == std::process::id() || !super::windows_process_is_elevated()? {
        return Err(PlatformError::operation_failed(
            "administrator restart requires an elevated child and a separate parent",
        ));
    }
    let parent = Handle::new(
        unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        },
        "open_parent",
    )?;
    let mut path = vec![0u16; 32_768];
    let mut length = path.len() as u32;
    if unsafe { QueryFullProcessImageNameW(parent.0, 0, path.as_mut_ptr(), &mut length) } == 0 {
        return Err(native_error("parent_executable"));
    }
    let expected = wide(current_executable()?);
    if path[..length as usize] != expected[..expected.len() - 1] {
        return Err(PlatformError::operation_failed(
            "administrator restart parent executable changed",
        ));
    }
    let ready = Handle::new(
        unsafe {
            OpenEventW(
                EVENT_MODIFY_STATE,
                0,
                event_name(pid, &nonce, "ready").as_ptr(),
            )
        },
        "open_ready",
    )?;
    let proceed = Handle::new(
        unsafe { OpenEventW(SYNCHRONIZE, 0, event_name(pid, &nonce, "proceed").as_ptr()) },
        "open_proceed",
    )?;
    if unsafe { SetEvent(ready.0) } == 0 {
        return Err(native_error("signal_ready"));
    }
    for (stage, handle) in [("commit", proceed.0), ("parent_exit", parent.0)] {
        let wait = unsafe { WaitForSingleObject(handle, HANDOFF_TIMEOUT_MS) };
        if wait != WAIT_OBJECT_0 {
            return Err(PlatformError::operation_failed(format!(
                "administrator restart stage={stage} wait_code={wait}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_restart_preserves_quoted_roots_and_rejects_mixed_arguments() {
        assert_eq!(
            analysis_arguments(Path::new(r"C:\")).unwrap(),
            " --analyze-directory \"C:\\\\\""
        );
        assert_eq!(
            analysis_arguments(Path::new(r"C:\Folder & \u{4e2d}")).unwrap(),
            " --analyze-directory \"C:\\Folder & \\u{4e2d}\""
        );
        assert!(analysis_arguments(Path::new("relative")).is_err());
        assert!(analysis_arguments(Path::new("C:\\bad\nfolder")).is_err());
        let args = vec![
            "app".into(),
            RESTART_FLAG.into(),
            "123".into(),
            "0123456789abcdef0123456789abcdef".into(),
            "--analyze-directory".into(),
            r"C:\".into(),
        ];
        assert_eq!(parse_handoff(&args).unwrap().0, 123);
        let mut mixed = args;
        mixed.push("--background".into());
        assert!(parse_handoff(&mixed).is_err());
    }

    fn test_event(signaled: bool) -> Handle {
        Handle::new(
            unsafe { CreateEventW(ptr::null(), 1, i32::from(signaled), ptr::null()) },
            "test_event",
        )
        .unwrap()
    }

    #[test]
    fn parent_commits_only_after_child_readiness_and_never_after_timeout_or_exit() {
        let ready = test_event(false);
        let proceed = test_event(false);
        let running_process = Handle::new(
            unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, std::process::id()) },
            "test_running_process",
        )
        .unwrap();
        assert!(commit_ready_child(ready.0, running_process.0, proceed.0, 0).is_err());
        assert_ne!(unsafe { WaitForSingleObject(proceed.0, 0) }, WAIT_OBJECT_0);

        // A signaled waitable object has the same wait semantics as an exited process.
        let exited = test_event(true);
        assert!(commit_ready_child(ready.0, exited.0, proceed.0, 0).is_err());
        assert_ne!(unsafe { WaitForSingleObject(proceed.0, 0) }, WAIT_OBJECT_0);

        assert_ne!(unsafe { SetEvent(ready.0) }, 0);
        assert!(commit_ready_child(ready.0, exited.0, proceed.0, 0).is_err());
        assert_ne!(unsafe { WaitForSingleObject(proceed.0, 0) }, WAIT_OBJECT_0);
        commit_ready_child(ready.0, running_process.0, proceed.0, 0).unwrap();
        assert_eq!(unsafe { WaitForSingleObject(proceed.0, 0) }, WAIT_OBJECT_0);
    }

    #[test]
    fn restart_handoff_accepts_only_versioned_bounded_arguments() {
        let args = |tail: &[&str]| {
            std::iter::once(OsString::from("MangoDisk.exe"))
                .chain(tail.iter().map(OsString::from))
                .collect::<Vec<_>>()
        };
        let nonce = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            parse_handoff(&args(&[RESTART_FLAG, "123", nonce])).unwrap(),
            (123, nonce.to_owned())
        );
        for tail in [
            vec![RESTART_FLAG],
            vec![RESTART_FLAG, "0", nonce],
            vec![RESTART_FLAG, "-1", nonce],
            vec![RESTART_FLAG, "123", "../bad"],
            vec![RESTART_FLAG, "123", nonce, "extra"],
        ] {
            assert!(parse_handoff(&args(&tail)).is_err());
        }
        assert!(prepare_elevated_restart_startup(args(&[])).is_none());
        assert!(
            prepare_elevated_restart_startup(args(&["--other", RESTART_FLAG]))
                .unwrap()
                .is_err()
        );
    }
}
