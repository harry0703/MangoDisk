use std::{
    mem::{size_of, zeroed},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
};

use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE},
    System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    },
};

use crate::PlatformCancellation;

/// Name-only writer checks need neither per-process access nor ownership lookup.
/// Capture every call so a browser started after preflight is still detected.
pub(super) fn capture(cancellation: &PlatformCancellation) -> Result<Vec<String>, String> {
    ensure_not_cancelled(cancellation)?;
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(native_error("create", unsafe { GetLastError() }));
    }
    // The valid snapshot handle is owned through all success, error, and cancel paths.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    enumerate(snapshot.as_raw_handle(), cancellation)
}

fn enumerate(snapshot: HANDLE, cancellation: &PlatformCancellation) -> Result<Vec<String>, String> {
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    ensure_not_cancelled(cancellation)?;
    if unsafe { Process32FirstW(snapshot, &mut entry) } == 0 {
        // A live system always contains processes. An empty first enumeration
        // must fall back rather than authorize cleanup with an empty writer set.
        return Err(native_error("first", unsafe { GetLastError() }));
    }
    let mut names = Vec::new();
    loop {
        ensure_not_cancelled(cancellation)?;
        if entry.th32ProcessID != 0 {
            let length = entry
                .szExeFile
                .iter()
                .position(|unit| *unit == 0)
                .ok_or_else(|| "windows_process_snapshot_invalid_name".to_string())?;
            if length == 0 {
                return Err("windows_process_snapshot_empty_name".into());
            }
            names.push(String::from_utf16_lossy(&entry.szExeFile[..length]));
        }
        if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
            let code = unsafe { GetLastError() };
            // Only the documented end-of-list status makes this a complete
            // snapshot; partial enumeration cannot authorize a destructive action.
            if code != ERROR_NO_MORE_FILES {
                return Err(native_error("next", code));
            }
            ensure_not_cancelled(cancellation)?;
            return Ok(names);
        }
    }
}

fn ensure_not_cancelled(cancellation: &PlatformCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("windows_process_snapshot_cancelled".into())
    } else {
        Ok(())
    }
}

fn native_error(stage: &str, code: u32) -> String {
    format!("windows_process_snapshot_native_failed stage={stage} code={code}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        process::{Child, Command, Stdio},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    #[test]
    fn invalid_snapshot_does_not_return_a_partial_writer_set() {
        let error =
            enumerate(INVALID_HANDLE_VALUE, &PlatformCancellation::new(|| false)).unwrap_err();
        assert!(error.contains("stage=first"), "{error}");
        assert!(error.contains("code=6"), "{error}");
    }

    #[test]
    fn cancellation_during_enumeration_stops_the_snapshot() {
        let calls = Arc::new(AtomicUsize::new(0));
        let cancellation =
            PlatformCancellation::new(move || calls.fetch_add(1, Ordering::SeqCst) >= 2);
        assert!(capture(&cancellation).unwrap_err().contains("cancelled"));
    }

    struct FixtureProcess(Child);
    impl Drop for FixtureProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    #[ignore = "child entry point for the isolated process-name test"]
    fn process_name_fixture_waits() {
        let Some(ready) = std::env::var_os("MANGODISK_PROCESS_NAMES_READY") else {
            return;
        };
        fs::write(ready, b"ready").unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    #[test]
    fn fresh_snapshots_detect_started_and_stopped_unicode_processes() {
        let directory = tempfile::tempdir().unwrap();
        let name = format!(
            "mangodisk-\u{8FDB}\u{7A0B}-{}.exe",
            directory.path().file_name().unwrap().to_string_lossy()
        );
        let executable = directory.path().join(&name);
        let cancellation = PlatformCancellation::new(|| false);
        assert!(!capture(&cancellation).unwrap().contains(&name));
        fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        let ready = directory.path().join("ready");
        let mut child = FixtureProcess(
            Command::new(&executable)
                .args([
                    "--ignored",
                    "--exact",
                    "windows::inventory::process_names::tests::process_name_fixture_waits",
                ])
                .env("MANGODISK_PROCESS_NAMES_READY", &ready)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !ready.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "fixture must initialize"
            );
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "fixture must remain running"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(capture(&cancellation).unwrap().contains(&name));
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert!(!capture(&cancellation).unwrap().contains(&name));
    }
}
