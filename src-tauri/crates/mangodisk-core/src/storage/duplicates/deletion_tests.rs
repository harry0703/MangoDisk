use super::*;

#[test]
fn deleting_a_real_copy_does_not_skip_the_unloaded_duplicate_group() {
    let _guard = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    for group in 0..41 {
        let (copies, bytes) = if group == 0 {
            (3, 24 * 1024)
        } else {
            (2, 32 * 1024)
        };
        let content = vec![group as u8; bytes];
        for copy in 0..copies {
            fs::write(
                root.path().join(format!("group-{group}-copy-{copy}.bin")),
                &content,
            )
            .unwrap();
        }
    }
    let scan = DuplicateFileService::find_paged_with_progress(
        vec![display_path(root.path())],
        1,
        |_| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(scan.groups.len(), 40);
    assert_eq!(scan.total_group_count, 41);
    let selected = &scan.groups[0].entries[0];
    assert_eq!(scan.groups[0].entries.len(), 3);
    let previously_unloaded = DuplicateFileService::page(scan.scan_id, 40, 40).unwrap();
    let deleted = DuplicateFileService::delete_files_permanently(
        scan.scan_id,
        vec![PermanentDeleteCandidate {
            path: selected.path.clone(),
            expected_bytes: selected.bytes,
            expected_modified_at_ms: selected.modified_at_ms,
        }],
    )
    .unwrap();
    let next = DuplicateFileService::page(scan.scan_id, 40, 40).unwrap();
    assert_eq!(deleted.removed_paths, std::slice::from_ref(&selected.path));
    assert!(deleted.failed.is_empty());
    assert!(!Path::new(&selected.path).exists());
    assert_eq!(next.groups.len(), 1);
    assert_eq!(next.groups[0].id, previously_unloaded.groups[0].id);
    for retained in &scan.groups[0].entries[1..] {
        assert_eq!(fs::read(&retained.path).unwrap(), vec![0; 24 * 1024]);
    }
    clear_result_session().unwrap();
}

#[test]
fn missing_duplicate_is_reconciled_without_counting_a_deletion_or_released_bytes() {
    let _guard = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    for name in ["a.bin", "b.bin", "c.bin"] {
        fs::write(root.path().join(name), b"same payload").unwrap();
    }
    let result = DuplicateFileService::find_paged_with_progress(
        vec![display_path(root.path())],
        1,
        |_| {},
        |_| {},
    )
    .unwrap();
    let group = &result.groups[0];
    let selected = &group.entries[0];
    let retained = &group.entries[1];
    fs::remove_file(&selected.path).unwrap();
    let deleted = DuplicateFileService::delete_files_permanently(
        result.scan_id,
        vec![PermanentDeleteCandidate {
            path: selected.path.clone(),
            expected_bytes: selected.bytes,
            expected_modified_at_ms: selected.modified_at_ms,
        }],
    )
    .unwrap();
    assert!(deleted.removed_paths.is_empty());
    assert!(deleted.failed.is_empty());
    assert_eq!(
        deleted.already_absent_paths,
        std::slice::from_ref(&selected.path)
    );
    assert_eq!(deleted.released_bytes, 0);
    assert!(Path::new(&retained.path).exists());
    assert!(resolve_open_target(result.scan_id, &selected.path).is_err());
    clear_result_session().unwrap();
}

#[test]
fn duplicate_directory_deletion_uses_scan_proof_without_rehashing_nested_content() {
    let _guard = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    for name in ["left", "right"] {
        let directory = root.path().join(name);
        fs::create_dir_all(directory.join("nested")).unwrap();
        fs::write(directory.join("nested/content.bin"), [1; 4096]).unwrap();
    }
    let result = DuplicateFileService::find_paged_with_progress(
        vec![display_path(root.path())],
        1,
        |_| {},
        |_| {},
    )
    .unwrap();
    let group = &result.groups[0];
    assert_eq!(group.kind, DuplicateGroupKind::Directory);
    let selected = &group.entries[0];
    let retained = &group.entries[1];
    let content = Path::new(&selected.path).join("nested/content.bin");
    // Editing a descendant does not change the selected directory's own metadata.
    fs::write(&content, [2; 4096]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&content, fs::Permissions::from_mode(0o000)).unwrap();
    }
    let deleted = DuplicateFileService::delete_files_permanently(
        result.scan_id,
        vec![PermanentDeleteCandidate {
            path: selected.path.clone(),
            expected_bytes: selected.bytes,
            expected_modified_at_ms: selected.modified_at_ms,
        }],
    )
    .unwrap();
    assert!(deleted.failed.is_empty(), "{deleted:?}");
    assert_eq!(deleted.removed_paths, std::slice::from_ref(&selected.path));
    assert!(!Path::new(&selected.path).exists());
    assert_eq!(
        fs::read(Path::new(&retained.path).join("nested/content.bin")).unwrap(),
        vec![1; 4096]
    );
    clear_result_session().unwrap();
}

#[test]
fn changed_duplicate_file_is_skipped_while_other_selected_files_are_deleted() {
    let _guard = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    for name in ["a.bin", "b.bin", "c.bin"] {
        fs::write(root.path().join(name), b"same payload").unwrap();
    }
    let result = DuplicateFileService::find_paged_with_progress(
        vec![display_path(root.path())],
        1,
        |_| {},
        |_| {},
    )
    .unwrap();
    let entries = &result.groups[0].entries;
    let candidates = entries[..2]
        .iter()
        .map(|entry| PermanentDeleteCandidate {
            path: entry.path.clone(),
            expected_bytes: entry.bytes,
            expected_modified_at_ms: entry.modified_at_ms,
        })
        .collect();
    fs::write(&entries[0].path, b"different length payload").unwrap();
    let deleted =
        DuplicateFileService::delete_files_permanently(result.scan_id, candidates).unwrap();
    assert_eq!(deleted.failed.len(), 1);
    assert_eq!(
        deleted.failed[0].details.reason,
        crate::filesystem::PermanentDeleteFailureReason::ItemChanged
    );
    assert_eq!(deleted.removed_paths, [entries[1].path.clone()]);
    assert!(Path::new(&entries[0].path).exists());
    assert!(Path::new(&entries[2].path).exists());
    clear_result_session().unwrap();
}
