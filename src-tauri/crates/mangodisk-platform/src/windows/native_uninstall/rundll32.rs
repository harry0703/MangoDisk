use super::{
    expand_environment_path, path_identity, system_directory_path, RegisteredCommandRejection,
};
use crate::ApplicationUninstallDiagnostic as Reason;
use std::{
    fs,
    os::windows::fs::MetadataExt,
    path::{Component, Path, PathBuf, Prefix},
};
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

/// Keep the registered DLL call separate from ordinary EXEs. The scanner never loads a DLL
/// to inspect its exports: even loading it would execute vendor code before user confirmation.
pub(super) struct RegisteredDllCommand {
    pub(super) executable: PathBuf,
    pub(super) arguments: String,
    pub(super) host_kind: &'static str,
    pub(super) entry_point_kind: &'static str,
    pub(super) parameters_present: bool,
    pub(super) nvidia_installer: bool,
}

pub(super) fn validate(
    executable: PathBuf,
    arguments: &str,
) -> Result<RegisteredDllCommand, RegisteredCommandRejection> {
    let system = system_directory_path().map_err(|error| RegisteredCommandRejection {
        reason: Reason::ExecutableProbeFailed,
        native_code: error.native_code().map(|code| code as i32),
        detail: "system_directory_query",
        target_kind: "exe",
    })?;
    let host_kind = trusted_host_kind(&executable, &system).ok_or_else(|| {
        rejection(
            Reason::UnsupportedCommandHost,
            "untrusted_rundll32_host",
            "exe",
        )
    })?;
    probe_file(&executable, "host_metadata", "exe")?;
    let (library, suffix, entry) = parse_dll_arguments(arguments)
        .map_err(|detail| rejection(Reason::InvalidCommand, detail, "dll"))?;
    let library = expand_environment_path(library);
    if library.contains('%') {
        return Err(rejection(
            Reason::UnresolvedEnvironment,
            "dll_environment",
            "dll",
        ));
    }
    let library_path = Path::new(&library);
    // Absolute local DLLs make the loaded target explicit. Never use PATH, the working
    // directory, UNC shares, device paths, parent traversal, or an alternate data stream.
    if !local_dll_path(library_path) {
        return Err(rejection(
            Reason::InvalidExecutable,
            "invalid_dll_path",
            "dll",
        ));
    }
    probe_file(library_path, "dll_metadata", "dll")?;
    // Only the DLL path is quoted/expanded. Export spelling, argument whitespace, and
    // vendor package identifiers remain untouched and are never interpreted by cmd.exe.
    Ok(RegisteredDllCommand {
        executable,
        arguments: format!("\"{library}\",{suffix}"),
        host_kind,
        entry_point_kind: if entry == "UninstallPackage" {
            "uninstall_package"
        } else {
            "named_export"
        },
        parameters_present: !suffix[entry.len()..].trim().is_empty(),
        nvidia_installer: uses_nvidia_result_codes(library_path, entry),
    })
}

/// NVIDIA Installer 2 documents vendor-specific results. This identifies the calling
/// convention only; it grants no execution permission and does not replace path validation.
fn uses_nvidia_result_codes(library: &Path, entry: &str) -> bool {
    library
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("NVI2.DLL"))
        && entry == "UninstallPackage"
}

/// Derive trusted locations from the native Windows API, not an overridable environment
/// variable. Preserve System32 versus SysWOW64: the DLL and its host must have matching bitness.
fn trusted_host_kind(executable: &Path, system: &Path) -> Option<&'static str> {
    if path_identity::equal(executable, &system.join("rundll32.exe")) {
        return Some("system32");
    }
    let wow = system.parent()?.join("SysWOW64").join("rundll32.exe");
    path_identity::equal(executable, &wow).then_some("syswow64")
}

fn local_dll_path(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)))
        && matches!(components.next(), Some(Component::RootDir))
        && components.all(|component| matches!(component, Component::Normal(value) if !value.to_string_lossy().contains([':', '"'])))
        && path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
}

fn probe_file(
    path: &Path,
    stage: &'static str,
    kind: &'static str,
) -> Result<(), RegisteredCommandRejection> {
    let metadata = fs::symlink_metadata(path).map_err(|error| RegisteredCommandRejection {
        reason: match error.kind() {
            std::io::ErrorKind::NotFound => Reason::ExecutableMissing,
            std::io::ErrorKind::PermissionDenied => Reason::ExecutableAccessDenied,
            _ => Reason::ExecutableProbeFailed,
        },
        native_code: error.raw_os_error(),
        detail: stage,
        target_kind: kind,
    })?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(rejection(
            Reason::InvalidExecutable,
            "target_not_regular_file",
            kind,
        ));
    }
    Ok(())
}

fn rejection(
    reason: Reason,
    detail: &'static str,
    target_kind: &'static str,
) -> RegisteredCommandRejection {
    RegisteredCommandRejection {
        reason,
        native_code: None,
        detail,
        target_kind,
    }
}

/// A quoted library can contain commas; an unquoted one ends at the first comma.
/// Return the untouched suffix so DLL export names and package arguments keep their case.
fn parse_dll_arguments(arguments: &str) -> Result<(&str, &str, &str), &'static str> {
    if arguments.contains(['\0', '\r', '\n']) {
        return Err("dll_control_character");
    }
    let arguments = arguments.trim_start();
    let (library, suffix) = if let Some(quoted) = arguments.strip_prefix('"') {
        let closing = quoted.find('"').ok_or("dll_unclosed_quote")?;
        let suffix = quoted[closing + 1..]
            .strip_prefix(',')
            .ok_or("dll_missing_entry_separator")?;
        (&quoted[..closing], suffix)
    } else {
        arguments
            .split_once(',')
            .ok_or("dll_missing_entry_separator")?
    };
    if library.is_empty() || library.contains('"') {
        return Err("dll_invalid_library_token");
    }
    let end = suffix.find(char::is_whitespace).unwrap_or(suffix.len());
    let entry = &suffix[..end];
    if !entry
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        || !entry
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'@' | b'?' | b'$'))
    {
        return Err("dll_invalid_entry_point");
    }
    Ok((library, suffix, entry))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "compiles x64/x86 fixture DLLs and invokes system Rundll32 against disposable HKCU records"]
    fn registered_rundll32_fixture_executes_and_verifies_removal() {
        use super::super::*;
        let system = system_directory_path().unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let key_name = format!("MangoDiskRundll32Fixture-{}-{nonce}", std::process::id());
        let directory = std::env::temp_dir().join(format!("{key_name} spaces & punctuation"));
        fs::create_dir(&directory).unwrap();
        let key_path = format!(r"{UNINSTALL_PATH}\{key_name}");
        struct Cleanup(PathBuf, String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = RegKey::predef(HKEY_CURRENT_USER)
                    .delete_subkey_with_flags(&self.1, KEY_WOW64_64KEY);
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone(), key_path.clone());
        let source = directory.join("fixture.rs");
        fs::write(&source, include_str!("fixtures/rundll32_fixture.rs")).unwrap();
        for (target, host) in [
            ("x86_64-pc-windows-msvc", system.join("rundll32.exe")),
            (
                "i686-pc-windows-msvc",
                system.parent().unwrap().join("SysWOW64/rundll32.exe"),
            ),
        ] {
            let library = directory.join("NVI2.DLL");
            let compile = Command::new("rustc")
                .args(["--edition=2021", "--crate-type=cdylib", "--target", target])
                .arg(&source)
                .arg("-o")
                .arg(&library)
                .output()
                .unwrap();
            assert!(
                compile.status.success(),
                "fixture compilation failed: {}",
                String::from_utf8_lossy(&compile.stderr)
            );
            for outcome in [
                "success",
                "error",
                "keep",
                "cancel",
                "cancel_removed",
                "reboot",
            ] {
                let command = format!(
                    r#""{}" "{}",UninstallPackage {key_name} {outcome}"#,
                    host.display(),
                    library.display()
                );
                let (entry, _) = RegKey::predef(HKEY_CURRENT_USER)
                    .create_subkey_with_flags(&key_path, winreg::enums::KEY_WRITE | KEY_WOW64_64KEY)
                    .unwrap();
                entry.set_value("DisplayName", &key_name).unwrap();
                entry.set_value("UninstallString", &command).unwrap();
                drop(entry);
                let (command_kind, command_digest) =
                    registered_uninstall_command_evidence_with_diagnostic(
                        &command,
                        &key_name,
                        ApplicationInstallScope::CurrentUser,
                    )
                    .unwrap();
                let registration = ApplicationUninstallRegistration::WindowsRegistered {
                    key_name: key_name.clone(),
                    scope: ApplicationInstallScope::CurrentUser,
                    registry_view: WindowsRegistryView::Registry64,
                    command_kind,
                    command_digest,
                    estimated_bytes: 0,
                };
                let expected = match outcome {
                    "success" => Ok(ApplicationUninstallExecutionOutcome::Completed),
                    "error" => Err(ApplicationUninstallPlatformError::NativeFailureAfterRemoval(7)),
                    "cancel" => Err(ApplicationUninstallPlatformError::UserCancelled),
                    "cancel_removed" => Err(
                        ApplicationUninstallPlatformError::NativeFailureAfterRemoval(0xE0E0_0001),
                    ),
                    "reboot" => Ok(ApplicationUninstallExecutionOutcome::RestartRequired),
                    _ => Err(ApplicationUninstallPlatformError::RegistrationChanged),
                };
                assert_eq!(
                    execute_registration(&registration),
                    expected,
                    "{target} {outcome}"
                );
            }
        }
    }

    #[test]
    fn nvidia_result_convention_does_not_apply_to_other_dll_exports() {
        assert!(uses_nvidia_result_codes(
            Path::new(r"C:\Vendor\nvi2.dll"),
            "UninstallPackage"
        ));
        assert!(!uses_nvidia_result_codes(
            Path::new(r"C:\Vendor\other.dll"),
            "UninstallPackage"
        ));
        assert!(!uses_nvidia_result_codes(
            Path::new(r"C:\Vendor\NVI2.DLL"),
            "Remove"
        ));
    }

    #[test]
    fn rundll32_parser_preserves_nvidia_export_and_package_arguments() {
        for library in [
            r"C:\Program Files\NVIDIA Corporation\Installer2\InstallerCore\NVI2.DLL",
            r"C:\Vendor, Inc\uninstall.dll",
        ] {
            let arguments =
                format!("\"{library}\",UninstallPackage Display.PhysX  /keep \"a & b\"");
            let (parsed, suffix, entry) = parse_dll_arguments(&arguments).unwrap();
            assert_eq!(parsed, library);
            assert_eq!(suffix, "UninstallPackage Display.PhysX  /keep \"a & b\"");
            assert_eq!(entry, "UninstallPackage");
        }
        assert_eq!(
            parse_dll_arguments(r"C:\Vendor Suite\uninstall.dll,Remove /keep").unwrap(),
            (r"C:\Vendor Suite\uninstall.dll", "Remove /keep", "Remove")
        );
    }

    #[test]
    fn rundll32_rejects_ambiguous_or_script_style_dll_calls() {
        for arguments in [
            "",
            "javascript:RunHTMLApplication",
            "a.dll",
            "a.dll,",
            "a.dll, Remove",
            "a.dll,Remove,Other",
            "a.dll,Remove\n/arg",
            "\"a.dll\" /arg,Remove",
            "\"a.dll,Remove",
        ] {
            assert!(parse_dll_arguments(arguments).is_err(), "{arguments:?}");
        }
        for path in [
            r"uninstall.dll",
            r"C:uninstall.dll",
            r"\\server\share\uninstall.dll",
            r"\\?\C:\uninstall.dll",
            r"C:\a\..\uninstall.dll",
            r"C:\app:payload.dll",
            r"C:\app.exe",
        ] {
            assert!(!local_dll_path(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn rundll32_host_selection_preserves_system_bitness_and_rejects_copies() {
        let system = Path::new(r"C:\Windows\System32");
        assert_eq!(
            trusted_host_kind(Path::new(r"c:\WINDOWS\system32\RUNDLL32.EXE"), system),
            Some("system32")
        );
        assert_eq!(
            trusted_host_kind(Path::new(r"C:\Windows\SysWOW64\rundll32.exe"), system),
            Some("syswow64")
        );
        for path in [
            r"C:\tools\rundll32.exe",
            r"C:\Windows\System32-other\rundll32.exe",
            r"C:\Windows\System32\..\rundll32.exe",
        ] {
            assert_eq!(trusted_host_kind(Path::new(path), system), None);
        }
    }

    #[test]
    fn rundll32_evidence_requires_a_real_dll_and_tracks_package_arguments() {
        let root =
            std::env::temp_dir().join(format!("mangodisk-dll-evidence-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let library = root.join("fixture.dll");
        fs::write(&library, b"read-only parser fixture, never executed").unwrap();
        let host = system_directory_path().unwrap().join("rundll32.exe");
        let command = format!(
            "\"{}\" \"{}\",UninstallPackage Display.PhysX",
            host.display(),
            library.display()
        );
        let evidence = |command: &str| {
            super::super::registered_uninstall_command_evidence_with_diagnostic(
                command,
                "fixture",
                crate::ApplicationInstallScope::Machine,
            )
        };
        let (kind, digest) = evidence(&command).unwrap();
        assert_eq!(kind, crate::WindowsRegisteredUninstallKind::Rundll32);
        assert_ne!(
            digest,
            evidence(&command.replace("Display.PhysX", "Display.Driver"))
                .unwrap()
                .1
        );
        fs::remove_file(&library).unwrap();
        let error = evidence(&command).unwrap_err();
        assert_eq!(error.reason, Reason::ExecutableMissing);
        assert_eq!(error.detail, "dll_metadata");
        fs::create_dir(&library).unwrap();
        assert_eq!(
            evidence(&command).unwrap_err().detail,
            "target_not_regular_file"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
