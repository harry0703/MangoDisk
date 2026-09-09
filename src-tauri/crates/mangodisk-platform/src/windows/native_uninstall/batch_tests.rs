use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn registered_batch_elevation_matches_the_registration_scope() {
    assert_eq!(
        registered_host_launch_mode(ApplicationInstallScope::Machine),
        ShellLaunchMode::RequestElevation
    );
    assert_eq!(
        registered_host_launch_mode(ApplicationInstallScope::CurrentUser),
        ShellLaunchMode::Default
    );
}

struct BatchFixture {
    directory: PathBuf,
    key_name: String,
}

impl BatchFixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let key_name = format!("MangoDiskBatchFixture-{}-{nonce}", std::process::id());
        let directory = env::temp_dir().join(format!("{key_name} spaces & punctuation"));
        fs::create_dir(&directory).expect("fixture directory should be created");
        Self {
            directory,
            key_name,
        }
    }
}

impl Drop for BatchFixture {
    fn drop(&mut self) {
        // Only this test's uniquely named registration and directory are removed.
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_with_flags(
            format!(r"{UNINSTALL_PATH}\{}", self.key_name),
            KEY_WOW64_64KEY,
        );
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn registered_batch_evidence_tracks_kind_arguments_and_precise_rejections() {
    let fixture = BatchFixture::new();
    for extension in ["bat", "cmd"] {
        let script = fixture.directory.join(format!("uninstall.{extension}"));
        fs::write(&script, b"@exit /b 0\r\n").unwrap();
        let command = format!(r#""{}" /remove "vendor argument""#, script.display());
        let (kind, digest) = registered_uninstall_command_evidence_with_diagnostic(
            &command,
            &fixture.key_name,
            ApplicationInstallScope::Machine,
        )
        .expect("registered batch files should be accepted without execution");
        assert_eq!(kind, WindowsRegisteredUninstallKind::BatchScript);
        let (_, changed_digest) = registered_uninstall_command_evidence_with_diagnostic(
            &format!("{command} /changed"),
            &fixture.key_name,
            ApplicationInstallScope::Machine,
        )
        .unwrap();
        assert_ne!(
            digest, changed_digest,
            "argument changes must invalidate reviewed evidence"
        );
        fs::remove_file(&script).unwrap();
        let missing = registered_uninstall_command_evidence_with_diagnostic(
            &command,
            &fixture.key_name,
            ApplicationInstallScope::Machine,
        )
        .unwrap_err();
        assert_eq!(
            missing.reason,
            crate::ApplicationUninstallDiagnostic::ExecutableMissing
        );
        assert_eq!(missing.detail, "metadata");
        assert_eq!(missing.target_kind, extension);
        fs::create_dir(&script).unwrap();
        let directory = registered_uninstall_command_evidence_with_diagnostic(
            &command,
            &fixture.key_name,
            ApplicationInstallScope::Machine,
        )
        .unwrap_err();
        assert_eq!(directory.detail, "target_not_regular_file");
        assert_eq!(directory.target_kind, extension);
    }
    let unsupported = registered_uninstall_command_evidence_with_diagnostic(
        r#""C:\Fixture\uninstall.vbs""#,
        &fixture.key_name,
        ApplicationInstallScope::Machine,
    )
    .unwrap_err();
    assert_eq!(unsupported.detail, "unsupported_target_extension");
    assert_eq!(unsupported.target_kind, "vbs");
}

#[test]
#[ignore = "launches disposable batch uninstallers and removes only their own HKCU registrations"]
fn registered_batch_fixture_executes_and_verifies_native_removal() {
    // The same path and parameter hazards occur in vendor registrations. Exercise
    // the real Shell boundary instead of replacing it with a process mock.
    for (extension, exit_code, removes_record) in
        [("bat", 0, true), ("cmd", 7, true), ("bat", 0, false)]
    {
        let fixture = BatchFixture::new();
        let script = fixture.directory.join(format!("uninstall.{extension}"));
        let key_path = format!(r"{UNINSTALL_PATH}\{}", fixture.key_name);
        let removal = if removes_record {
            format!(r#""%SystemRoot%\System32\reg.exe" delete "HKCU\{key_path}" /f /reg:64 >nul"#)
        } else {
            "rem Leave the fixture registered to verify that exit zero is insufficient.".to_string()
        };
        let contents = format!(
            "@echo off\r\n> \"%~dp0args.txt\" echo %1\r\n{removal}\r\nexit /b {exit_code}\r\n"
        );
        fs::write(&script, contents).unwrap();
        let command = format!(
            r#""{}" "value with spaces & punctuation""#,
            script.display()
        );
        let (entry, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey_with_flags(&key_path, winreg::enums::KEY_WRITE | KEY_WOW64_64KEY)
            .unwrap();
        entry.set_value("DisplayName", &fixture.key_name).unwrap();
        entry.set_value("UninstallString", &command).unwrap();
        drop(entry);
        let (command_kind, command_digest) = registered_uninstall_command_evidence_with_diagnostic(
            &command,
            &fixture.key_name,
            ApplicationInstallScope::CurrentUser,
        )
        .unwrap();
        let registration = ApplicationUninstallRegistration::WindowsRegistered {
            key_name: fixture.key_name.clone(),
            scope: ApplicationInstallScope::CurrentUser,
            registry_view: WindowsRegistryView::Registry64,
            command_kind,
            command_digest,
            estimated_bytes: 0,
        };
        let result = execute_registration(&registration);
        if !removes_record {
            assert_eq!(
                result,
                Err(ApplicationUninstallPlatformError::RegistrationChanged)
            );
        } else if exit_code == 0 {
            assert_eq!(result, Ok(ApplicationUninstallExecutionOutcome::Completed));
        } else {
            assert_eq!(
                result,
                Err(ApplicationUninstallPlatformError::NativeFailureAfterRemoval(exit_code))
            );
        }
        assert_eq!(
            registration_state(&registration),
            Ok(if removes_record {
                ApplicationUninstallRegistrationState::Absent
            } else {
                ApplicationUninstallRegistrationState::Installed
            })
        );
        assert_eq!(
            fs::read_to_string(fixture.directory.join("args.txt"))
                .unwrap()
                .trim(),
            r#""value with spaces & punctuation""#
        );
    }
}
