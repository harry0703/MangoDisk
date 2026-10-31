//! Regression coverage for direct deletion without sibling creation or content access.
use super::*;

#[test]
fn selected_file_allows_normal_edits_while_duplicate_snapshot_rejects_them() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("selected.bin");
    fs::write(&path, b"original").unwrap();
    let metadata = fs::metadata(&path).unwrap();
    let candidate = PermanentDeleteCandidate {
        path: path.to_string_lossy().into_owned(),
        expected_bytes: metadata.len(),
        expected_modified_at_ms: modified_ms(&metadata),
    };
    fs::write(&path, b"changed content").unwrap();
    let error = delete_file_candidate_permanently(&candidate).unwrap_err();
    assert_eq!(error.reason(), Some(CoreErrorReason::ItemChanged));
    let (_, usage) = delete_selected_file_permanently(&path).unwrap();
    assert_eq!(usage.logical_bytes, 15);
    assert!(!path.exists());
}

#[test]
fn file_selection_rejects_a_directory_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("directory");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("keep.bin"), b"keep").unwrap();
    let error = delete_selected_file_permanently(&path).unwrap_err();
    assert!(!error.is_partial());
    assert_eq!(fs::read(path.join("keep.bin")).unwrap(), b"keep");
}

#[test]
fn missing_file_retains_native_not_found_evidence() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("missing");
    let failure = delete_selected_file_permanently(&path)
        .unwrap_err()
        .into_failure(path.to_string_lossy().into_owned());
    assert_eq!(
        failure.details.reason,
        super::super::PermanentDeleteFailureReason::NotFound
    );
    assert!(failure.details.native_code.is_some());
    assert!(!failure.details.partial);
}

#[cfg(unix)]
#[test]
fn file_deletion_does_not_require_content_read_permission() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("unreadable.bin");
    fs::write(&path, b"payload").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    delete_selected_file_permanently(&path).unwrap();
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn file_deletion_does_not_require_parent_listing_permission() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("no-listing");
    fs::create_dir(&parent).unwrap();
    let path = parent.join("payload.bin");
    fs::write(&path, b"payload").unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o300)).unwrap();
    let result = delete_selected_file_permanently(&path);
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    result.unwrap();
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn unwritable_parent_reports_removal_failure_without_moving_the_file() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("unwritable");
    fs::create_dir(&parent).unwrap();
    let path = parent.join("keep.bin");
    fs::write(&path, b"keep").unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();
    let result = delete_selected_file_permanently(&path);
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let failure = result.unwrap_err();
    assert_eq!(failure.stage(), "remove");
    assert!(failure.native_code().is_some());
    assert!(!failure.is_partial());
    assert_eq!(fs::read(path).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn replacement_symlink_is_rejected_without_deleting_external_content() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("selected");
    let outside = root.path().join("keep");
    fs::write(&path, b"selected").unwrap();
    fs::write(&outside, b"keep").unwrap();
    let prepared = prepare_path_for_permanent_delete(&path).unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert!(delete_path_permanently(prepared, 8, 1).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"keep");
    assert!(fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[cfg(windows)]
#[test]
fn deletion_succeeds_when_parent_disallows_creating_subdirectories() {
    use std::process::Command;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("payload.bin");
    fs::write(&path, b"payload").unwrap();
    let deny = Command::new("icacls")
        .arg(root.path())
        .args(["/deny", "*S-1-1-0:(AD)"])
        .output()
        .unwrap();
    assert!(deny.status.success(), "set the disposable parent ACL");
    let mkdir = fs::create_dir(root.path().join("forbidden"));
    let result = delete_selected_file_permanently(&path);
    let restore = Command::new("icacls")
        .arg(root.path())
        .args(["/remove:d", "*S-1-1-0"])
        .output()
        .unwrap();
    assert!(
        restore.status.success(),
        "restore the disposable parent ACL"
    );
    assert_eq!(
        mkdir.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    result.unwrap();
    assert!(!path.exists());
}
