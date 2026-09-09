//! Compiled only by the explicit native integration test, never linked into MangoDisk.
use std::{
    ffi::{c_char, c_void, CStr},
    process::Command,
};

/// Rundll32's documented callback ABI returns void; terminate this disposable host with
/// the requested test status so postflight can exercise both successful and partial removal.
#[no_mangle]
pub unsafe extern "system" fn UninstallPackage(
    _window: *mut c_void,
    _instance: *mut c_void,
    arguments: *mut c_char,
    _show: i32,
) {
    let arguments = CStr::from_ptr(arguments).to_str().unwrap();
    let mut tokens = arguments.split_whitespace();
    let key = tokens.next().unwrap();
    let outcome = tokens.next().unwrap();
    if !key.starts_with("MangoDiskRundll32Fixture-")
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || tokens.next().is_some()
    {
        std::process::exit(99);
    }
    if !matches!(outcome, "keep" | "cancel") {
        let registration =
            format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{key}");
        let status = Command::new(std::env::var("SystemRoot").unwrap() + r"\System32\reg.exe")
            .args(["delete", &registration, "/f", "/reg:64"])
            .status()
            .unwrap();
        if !status.success() {
            std::process::exit(98);
        }
    }
    std::process::exit(match outcome {
        "error" => 7,
        "cancel" | "cancel_removed" => 0xE0E0_0001u32 as i32,
        "reboot" => 1,
        _ => 0,
    });
}
