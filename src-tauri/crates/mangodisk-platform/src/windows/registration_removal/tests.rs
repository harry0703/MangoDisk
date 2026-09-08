use super::*;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    name: String,
    registry_path: String,
    root: PathBuf,
    key: RegKey,
}
impl Fixture {
    fn new(live_inventory: bool) -> Self {
        let name = format!(
            "MangoDiskRecordRemovalFixture-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let base = if live_inventory {
            UNINSTALL_PATH
        } else {
            r"Software\MangoDisk\Tests\RecordRemoval"
        };
        let registry_path = format!(r"{base}\{name}");
        let (key, disposition) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey_with_flags(&registry_path, KEY_READ | KEY_WRITE | KEY_WOW64_64KEY)
            .unwrap();
        assert_eq!(disposition, REG_CREATED_NEW_KEY);
        let root = std::env::temp_dir().join(&name);
        fs::create_dir(&root).unwrap();
        key.set_value("DisplayName", &name).unwrap();
        key.set_value(
            "InstallLocation",
            &root.join("missing").to_string_lossy().as_ref(),
        )
        .unwrap();
        key.set_value(
            "DisplayIcon",
            &root.join("missing/app.exe").to_string_lossy().as_ref(),
        )
        .unwrap();
        key.set_value(
            "UninstallString",
            &format!("\"{}\"", root.join("missing/uninstall.exe").display()),
        )
        .unwrap();
        Self {
            name,
            registry_path,
            root,
            key,
        }
    }
    fn id(&self) -> String {
        record_id(&self.name, ApplicationInstallScope::CurrentUser)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // This exact uniquely-created test leaf is the only registry subtree owned by the fixture.
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.registry_path);
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn record_snapshot_accepts_existing_files_missing_metadata_and_installer_flags() {
    let fixture = Fixture::new(false);
    fs::create_dir(fixture.root.join("missing")).unwrap();
    fixture
        .key
        .set_value(
            "QuietUninstallString",
            &format!("\"{}\"", std::env::current_exe().unwrap().display()),
        )
        .unwrap();
    fixture.key.set_value("WindowsInstaller", &1u32).unwrap();
    fixture.key.set_value("SystemComponent", &1u32).unwrap();
    for value in ["InstallLocation", "DisplayIcon", "DisplayName"] {
        fixture.key.delete_value(value).unwrap();
    }
    let (child, _) = fixture.key.create_subkey(r"settings\nested").unwrap();
    child.set_value("data", &"registered metadata").unwrap();
    drop(child);
    let transaction = Transaction::new().unwrap();
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(&fixture.registry_path)
        .unwrap();
    let (_, children) = snapshot_tree(key, KEY_WOW64_64KEY, &transaction).unwrap();
    assert_eq!(children, ["settings", r"settings\nested"]);
}

#[test]
fn registry_link_markers_are_never_followed_by_removal_preflight() {
    let fixture = Fixture::new(false);
    let (child, _) = fixture.key.create_subkey("plain").unwrap();
    child
        .set_raw_value(
            "SymbolicLinkValue",
            &winreg::RegValue {
                vtype: REG_LINK,
                bytes: vec![0, 0],
            },
        )
        .unwrap();
    let transaction = Transaction::new().unwrap();
    assert!(open_plain(&fixture.key, "plain", KEY_WOW64_64KEY, &transaction).is_err());
}

#[test]
#[ignore = "creates and removes uniquely named current-user uninstall fixture entries"]
fn live_record_removal_preserves_files_rejects_stale_snapshots_and_verifies_deletion() {
    let mut fixture = Fixture::new(true);
    let other = Fixture::new(true);
    let user_data = fixture.root.join("user-data.txt");
    fs::write(&user_data, b"preserved").unwrap();
    // Explicit removal must not depend on whether the application can still be uninstalled,
    // whether its directory exists, or which installer originally wrote the registration.
    fs::create_dir(fixture.root.join("missing")).unwrap();
    fixture
        .key
        .set_value(
            "QuietUninstallString",
            &format!("\"{}\"", std::env::current_exe().unwrap().display()),
        )
        .unwrap();
    fixture.key.set_value("WindowsInstaller", &1u32).unwrap();
    fixture.key.set_value("SystemComponent", &1u32).unwrap();
    fixture.key.delete_value("DisplayIcon").unwrap();
    let (child, _) = fixture.key.create_subkey(r"settings\nested").unwrap();
    child
        .set_value("payload", &"preserved until confirmed")
        .unwrap();
    drop(child);
    remove(&fixture.id(), true).unwrap();
    let transaction = Transaction::new().unwrap();
    let targets = locate(&fixture.id(), &transaction).unwrap();
    let digest = fingerprint(&targets);
    drop(targets);
    drop(transaction);
    // The elevated helper must reject HKCU even when presented with an otherwise valid digest.
    assert!(execute(&fixture.id(), &digest, true).is_err());
    let child = fixture
        .key
        .open_subkey_with_flags(r"settings\nested", KEY_WRITE)
        .unwrap();
    child
        .set_value("new field", &"changed after preview")
        .unwrap();
    drop(child);
    assert!(execute(&fixture.id(), &digest, false).is_err());
    assert!(RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(&fixture.registry_path)
        .is_ok());
    // Close the fixture's ordinary handle: native deletion is finalized after the last handle.
    let id = fixture.id();
    drop(std::mem::replace(
        &mut fixture.key,
        RegKey::predef(HKEY_CURRENT_USER),
    ));
    remove(&id, false).unwrap();
    assert!(verify_absent(&id).is_ok());
    // A second explicit removal is an idempotent success, including with a now-stale UI row.
    remove(&id, false).unwrap();
    assert_eq!(fs::read(&user_data).unwrap(), b"preserved");
    assert!(RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(&other.registry_path)
        .is_ok());
}

#[test]
fn postflight_and_lost_helper_results_preserve_mutation_uncertainty() {
    for stage in [Stage::Verify, Stage::Elevate] {
        let error = Failure { stage, code: 31 }.platform("application-000000000000000000000000");
        assert_eq!(
            error.mutation_state(),
            crate::PlatformMutationState::MayHaveChanged
        );
    }
    let cancelled = Failure {
        stage: Stage::Elevate,
        code: 1223,
    }
    .platform("application-000000000000000000000000");
    assert_eq!(
        cancelled.mutation_state(),
        crate::PlatformMutationState::NotAttempted
    );
}

#[test]
fn snapshot_and_scope_failures_have_distinct_private_path_free_reasons() {
    assert_eq!(Failure::changed(Stage::Snapshot).reason(), "record_changed");
    assert_eq!(
        Failure::changed(Stage::Scope).reason(),
        "helper_scope_mismatch"
    );
    assert_eq!(
        Failure::changed(Stage::Verify).reason(),
        "record_still_present"
    );
    assert_eq!(
        Failure {
            stage: Stage::Inspect,
            code: UNSUPPORTED
        }
        .reason(),
        "registry_link"
    );
}
