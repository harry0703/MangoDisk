use super::{execute, valid_id, Failure, Result, Stage};
use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::OsStrExt,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0},
    System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE},
    UI::{
        Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
        WindowsAndMessaging::SW_HIDE,
    },
};

const FLAG: &str = "--mangodisk-application-record-helper-v1";

/// The privileged boundary accepts only a redacted application ID and a full registry snapshot
/// digest. It discovers machine uninstall keys itself and revalidates the selected registry tree.
/// Exit codes carry a finite stage and native error number, never command text or private paths.
pub fn run_application_record_helper_mode(
    arguments: impl IntoIterator<Item = OsString>,
) -> Option<i32> {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    if arguments.get(1).and_then(|value| value.to_str()) != Some(FLAG) {
        return None;
    }
    let result = parse_arguments(&arguments).and_then(|(id, digest)| execute(id, digest, true));
    Some(match result {
        Ok(()) => 0,
        Err(failure) => ((failure.stage as u32) << 16 | (failure.code as u32 & 0xffff)) as i32,
    })
}

fn parse_arguments(arguments: &[OsString]) -> Result<(&str, &str)> {
    let invalid = || Failure {
        stage: Stage::Arguments,
        code: 87,
    };
    if arguments.len() != 4 {
        return Err(invalid());
    }
    let id = arguments[2].to_str().ok_or_else(invalid)?;
    let digest = arguments[3].to_str().ok_or_else(invalid)?;
    if !valid_id(id) || digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    Ok((id, digest))
}

pub(super) fn elevate(id: &str, digest: &str) -> Result<()> {
    let executable =
        std::env::current_exe().map_err(|error| Failure::new(Stage::Elevate, error))?;
    let executable = wide(executable.as_os_str());
    let verb = wide(OsStr::new("runas"));
    // Both arguments are fixed-width hexadecimal tokens validated in the ordinary process.
    let arguments = wide(OsStr::new(&format!("{FLAG} {id} {digest}")));
    let mut execution = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: verb.as_ptr(),
        lpFile: executable.as_ptr(),
        lpParameters: arguments.as_ptr(),
        nShow: SW_HIDE,
        // SAFETY: zero initializes optional pointers and reserved fields for ShellExecuteExW.
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: all three terminated UTF-16 buffers remain live for the synchronous launch call.
    if unsafe { ShellExecuteExW(&mut execution) } == 0 {
        return Err(Failure {
            stage: Stage::Elevate,
            code: unsafe { GetLastError() } as i32,
        });
    }
    if execution.hProcess.is_null() {
        return Err(Failure {
            stage: Stage::Elevate,
            code: 6,
        });
    }
    // SAFETY: ShellExecuteExW returned this owned process handle; close it after both reads.
    let wait = unsafe { WaitForSingleObject(execution.hProcess, INFINITE) };
    let mut code = 0;
    let read = unsafe { GetExitCodeProcess(execution.hProcess, &mut code) };
    unsafe { CloseHandle(execution.hProcess) };
    if wait != WAIT_OBJECT_0 || read == 0 {
        return Err(Failure {
            stage: Stage::Elevate,
            code: 31,
        });
    }
    decode_exit(code)
}

fn decode_exit(code: u32) -> Result<()> {
    if code == 0 {
        return Ok(());
    }
    let stage = match code >> 16 {
        1 => Stage::Arguments,
        2 => Stage::Locate,
        3 => Stage::Inspect,
        4 => Stage::Delete,
        5 => Stage::Commit,
        6 => Stage::Verify,
        8 => Stage::Snapshot,
        9 => Stage::Scope,
        _ => Stage::Elevate,
    };
    Err(Failure {
        stage,
        code: (code & 0xffff) as i32,
    })
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_refuses_paths_commands_and_incomplete_digests() {
        for id in [
            r"HKLM\Software\Example",
            "application-../entry",
            "application-00000000000000000000000z",
        ] {
            let args = [
                "MangoDisk".into(),
                FLAG.into(),
                id.into(),
                "a".repeat(64).into(),
            ];
            assert!(parse_arguments(&args).is_err());
        }
        let args = [
            "MangoDisk".into(),
            FLAG.into(),
            format!("application-{}", "a".repeat(24)).into(),
            "a".repeat(63).into(),
        ];
        assert!(parse_arguments(&args).is_err());
        assert_eq!(
            run_application_record_helper_mode([OsString::from("MangoDisk")]),
            None
        );
    }

    #[test]
    fn helper_errors_preserve_native_code_and_stage() {
        let error = decode_exit((Stage::Delete as u32) << 16 | 5).err().unwrap();
        assert!(matches!(error.stage, Stage::Delete));
        assert_eq!(error.code, 5);
        assert!(decode_exit(0).is_ok());
    }
}
