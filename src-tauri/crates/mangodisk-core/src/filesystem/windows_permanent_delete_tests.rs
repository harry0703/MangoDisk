use std::{
    fs::OpenOptions,
    os::windows::fs::OpenOptionsExt,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

struct JunctionSandbox {
    root: PathBuf,
    junction: PathBuf,
}

impl Drop for JunctionSandbox {
    fn drop(&mut self) {
        let junction_removed = !self.junction.exists() || fs::remove_dir(&self.junction).is_ok();
        if junction_removed {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
fn permanent_delete_rejects_file_reached_through_junction() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sandbox = std::env::temp_dir().join(format!(
        "MangoDisk-Junction-Permanent-Delete-{}-{unique}",
        std::process::id()
    ));
    let real_directory = sandbox.join("real");
    let junction = sandbox.join("junction");
    let _sandbox_cleanup = JunctionSandbox {
        root: sandbox.clone(),
        junction: junction.clone(),
    };
    let protected_file = real_directory.join("must-remain.bin");
    fs::create_dir_all(&real_directory).expect("the junction fixture directory should exist");
    fs::write(&protected_file, b"MangoDisk junction safety fixture")
        .expect("the junction fixture should be written");
    let output = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&real_directory)
        .output()
        .expect("mklink should create the test junction");
    assert!(
        output.status.success(),
        "failed to create test junction: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let linked_file = junction.join("must-remain.bin");
    let metadata = fs::metadata(&protected_file).expect("the fixture metadata should be readable");
    let candidate = PermanentDeleteCandidate {
        path: linked_file.to_string_lossy().into_owned(),
        expected_bytes: metadata.len(),
        expected_modified_at_ms: modified_ms(&metadata),
    };
    let error = delete_file_candidate_permanently(&candidate)
        .expect_err("a file reached through a junction must be rejected");

    assert!(error.to_string().contains("link or reparse point"));
    assert!(
        protected_file.exists(),
        "the junction target must remain untouched"
    );
}

#[test]
fn permanent_delete_does_not_hide_a_directory_with_a_locked_child() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sandbox = std::env::temp_dir().join(format!(
        "MangoDisk-Locked-Permanent-Delete-{}-{unique}",
        std::process::id()
    ));
    let directory = sandbox.join("directory");
    let payload = directory.join("payload.bin");
    fs::create_dir_all(&directory).expect("create the locked deletion fixture");
    fs::write(&payload, b"locked payload").expect("write the locked deletion fixture");
    let locked = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&payload)
        .expect("lock the deletion fixture without delete sharing");

    let prepared = prepare_path_for_permanent_delete(&directory)
        .expect("the locked target should be prepared");
    let error = delete_path_permanently(prepared, b"locked payload".len() as u64, 1)
        .expect_err("a locked descendant must stop permanent deletion");

    assert!(
        directory.exists(),
        "a failed delete must leave the visible directory in place"
    );
    assert!(payload.exists(), "the locked file must remain available");
    drop(locked);
    fs::remove_dir_all(sandbox).expect("remove the locked deletion fixture");
    assert!(
        !error.is_partial() || error.released_bytes() == 0,
        "the untouched locked fixture must not report released bytes"
    );
}

#[test]
fn cancellable_directory_delete_retains_a_locked_child() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sandbox = std::env::temp_dir().join(format!(
        "MangoDisk-Locked-Cancellable-Delete-{}-{unique}",
        std::process::id()
    ));
    let directory = sandbox.join("directory");
    let payload = directory.join("payload.bin");
    fs::create_dir_all(&directory).expect("create the locked cancellable fixture");
    fs::write(&payload, b"locked payload").expect("write the locked cancellable fixture");
    let locked = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&payload)
        .expect("lock the cancellable fixture without delete sharing");
    let prepared = prepare_path_for_permanent_delete(&directory)
        .expect("prepare the locked cancellable directory");

    let error = delete_directory_tree_permanently_with_cancellation(
        prepared,
        b"locked payload".len() as u64,
        1,
        &|| false,
    )
    .expect_err("a locked descendant must stop cancellable directory deletion");

    assert!(directory.exists(), "the directory must remain in place");
    assert!(payload.exists(), "the locked payload must remain available");
    assert_eq!(error.released_bytes(), 0);
    assert_eq!(error.affected_item_count(), 0);
    drop(locked);
    fs::remove_dir_all(sandbox).expect("remove the locked cancellable fixture");
}

#[test]
fn cancellable_directory_delete_rejects_a_new_junction() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sandbox = std::env::temp_dir().join(format!(
        "MangoDisk-Cancellable-Junction-{}-{unique}",
        std::process::id()
    ));
    let directory = sandbox.join("directory");
    let protected = sandbox.join("protected");
    let junction = directory.join("late-junction");
    let protected_file = protected.join("must-remain.bin");
    let _sandbox_cleanup = JunctionSandbox {
        root: sandbox.clone(),
        junction: junction.clone(),
    };
    fs::create_dir_all(&directory).expect("create the cancellable junction root");
    fs::create_dir_all(&protected).expect("create the protected junction target");
    fs::write(&protected_file, b"protected").expect("write the protected junction fixture");
    let prepared = prepare_path_for_permanent_delete(&directory)
        .expect("prepare the directory before the junction appears");
    let output = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&protected)
        .output()
        .expect("mklink should create the late junction");
    assert!(output.status.success(), "create the late junction fixture");

    let error = delete_directory_tree_permanently_with_cancellation(prepared, 0, 0, &|| false)
        .expect_err("a new junction must stop cancellable directory deletion");

    assert!(
        error.to_string().contains("link or reparse point"),
        "the failure must retain the safety reason"
    );
    assert!(directory.exists(), "the cleanup root must remain in place");
    assert!(
        protected_file.exists(),
        "the junction target must never be traversed or deleted"
    );
}
