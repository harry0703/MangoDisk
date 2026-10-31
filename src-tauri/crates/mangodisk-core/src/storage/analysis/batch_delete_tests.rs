use super::*;
use crate::filesystem::metadata::display_path;
use std::{fs, path::Path};

#[cfg(windows)]
#[path = "batch_delete_windows_test_token.rs"]
mod windows_test_token;

fn analyze(root: &Path) -> AnalysisResult {
    AnalysisService::analyze_with_progress(Some(root.to_string_lossy().into_owned()), true, |_| {})
        .unwrap()
}

#[test]
fn batch_accepts_the_complete_visible_selection_limit() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    for index in 0..super::super::ANALYSIS_VISIBLE_ENTRY_LIMIT {
        fs::write(root.join(format!("item-{index:03}.bin")), [1; 64]).unwrap();
    }
    let initial = analyze(&root);
    let selected = initial
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    let batch = AnalysisService::delete_entries_permanently(initial.scan_id, selected).unwrap();
    assert_eq!(
        batch.removed_paths.len(),
        super::super::ANALYSIS_VISIBLE_ENTRY_LIMIT
    );
    assert!(batch.failed.is_empty());
    assert!(batch.not_attempted_paths.is_empty());
    assert!(!batch.requires_rescan);
    assert_eq!(batch.released_bytes, initial.total_bytes);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    let updated = batch
        .updated_results
        .iter()
        .find(|result| result.scan_id == initial.scan_id)
        .unwrap();
    assert_eq!(updated.total_bytes, 0);
    assert!(updated.entries.is_empty());
    cache::clear_all().unwrap();
}

#[test]
fn batch_preserves_path_and_name_exclusions_and_continues_with_valid_items() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    for by_name in [false, true] {
        cache::clear_all().unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        let guarded = root.join("guarded");
        let retained = guarded.join("keep");
        fs::create_dir_all(&retained).unwrap();
        fs::write(retained.join("private.bin"), b"retained fixture").unwrap();
        fs::write(guarded.join("visible.bin"), b"visible fixture").unwrap();
        let valid = root.join("valid.bin");
        fs::write(&valid, b"deletable fixture").unwrap();
        let exclusions = ScanExclusionOptions {
            paths: if by_name {
                Vec::new()
            } else {
                vec![retained.to_string_lossy().into_owned()]
            },
            names: if by_name {
                vec![mangodisk_platform::ScanNameExclusion {
                    name: "keep".into(),
                    kind: mangodisk_platform::ExcludedNameKind::Folder,
                }]
            } else {
                Vec::new()
            },
        };
        let initial = AnalysisService::analyze_with_exclusions_progress(
            Some(root.to_string_lossy().into_owned()),
            true,
            exclusions,
            |_| {},
        )
        .unwrap();
        let selected = [&guarded, &valid].map(|path| display_path(path));
        let batch = AnalysisService::delete_entries_permanently(initial.scan_id, selected.to_vec())
            .unwrap();
        assert_eq!(batch.removed_paths, [selected[1].clone()]);
        assert_eq!(batch.failed.len(), 1);
        assert_eq!(batch.failed[0].path, selected[0]);
        assert!(!batch.requires_rescan);
        assert!(batch.not_attempted_paths.is_empty());
        assert_eq!(
            fs::read(retained.join("private.bin")).unwrap(),
            b"retained fixture"
        );
        assert!(guarded.join("visible.bin").exists());
        assert!(!valid.exists());
    }
    cache::clear_all().unwrap();
}

#[test]
fn batch_rejects_expired_authority_without_mutating_a_valid_path() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let file = root.join("retain.bin");
    fs::write(&file, b"retained fixture").unwrap();
    let previous = analyze(&root);
    let current = analyze(&root);
    assert_ne!(previous.scan_id, current.scan_id);
    let result = AnalysisService::delete_entries_permanently(
        previous.scan_id,
        vec![previous.entries[0].path.clone()],
    );
    assert_eq!(
        result.unwrap_err().code(),
        crate::CoreErrorCode::InvalidInput
    );
    assert_eq!(fs::read(file).unwrap(), b"retained fixture");
    cache::clear_all().unwrap();
}

#[test]
fn batch_deletes_unicode_and_long_paths_without_touching_siblings() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let directory = root.join("\u{6d4b}\u{8bd5} folder 'quoted' \u{65e5}\u{672c}\u{8a9e}");
    let mut nested = directory.clone();
    for index in 0..12 {
        nested.push(format!("long-component-{index:02}-abcdefgh"));
    }
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("\u{5185}\u{5bb9}.bin"), b"nested fixture").unwrap();
    let file = root.join("\u{e9} \u{7a7a}\u{683c} emoji \u{1f96d}.bin");
    fs::write(&file, b"unicode fixture").unwrap();
    let keep = root.join("keep.bin");
    fs::write(&keep, b"retained fixture").unwrap();
    let initial = analyze(&root);
    let selected = [&directory, &file].map(|path| display_path(path));
    let batch =
        AnalysisService::delete_entries_permanently(initial.scan_id, selected.to_vec()).unwrap();
    assert_eq!(batch.removed_paths, selected);
    assert!(batch.failed.is_empty());
    assert!(!batch.requires_rescan);
    assert_eq!(fs::read(keep).unwrap(), b"retained fixture");
    assert!(!directory.exists());
    assert!(!file.exists());
    cache::clear_all().unwrap();
}

#[cfg(unix)]
#[test]
fn unix_batch_rejects_replacements_with_links_and_preserves_external_contents() {
    use std::os::unix::fs::symlink;

    let _operation_lock = crate::shared::operation::test_operation_lock();
    for directory in [false, true] {
        cache::clear_all().unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let parent = fs::canonicalize(fixture.path()).unwrap();
        let root = parent.join("root");
        let external = parent.join("external");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&external).unwrap();
        fs::write(external.join("retained.bin"), b"external fixture").unwrap();
        let replaced = root.join("replaced");
        if directory {
            fs::create_dir(&replaced).unwrap();
            fs::write(replaced.join("original.bin"), b"original fixture").unwrap();
        } else {
            fs::write(&replaced, b"original fixture").unwrap();
        }
        let valid = root.join("valid.bin");
        fs::write(&valid, b"deletable fixture").unwrap();
        let initial = analyze(&root);
        if directory {
            fs::remove_dir_all(&replaced).unwrap();
            symlink("../external", &replaced).unwrap();
        } else {
            fs::remove_file(&replaced).unwrap();
            symlink("../external/retained.bin", &replaced).unwrap();
        }
        let selected = [&replaced, &valid].map(|path| display_path(path));
        let batch = AnalysisService::delete_entries_permanently(initial.scan_id, selected.to_vec())
            .unwrap();
        assert_eq!(batch.removed_paths, [selected[1].clone()]);
        assert_eq!(batch.failed.len(), 1);
        assert_eq!(batch.failed[0].path, selected[0]);
        assert!(!batch.requires_rescan);
        assert!(batch.not_attempted_paths.is_empty());
        assert!(fs::symlink_metadata(&replaced).unwrap().is_symlink());
        assert_eq!(
            fs::read(external.join("retained.bin")).unwrap(),
            b"external fixture"
        );
        assert!(!valid.exists());
    }
    cache::clear_all().unwrap();
}

#[cfg(unix)]
#[test]
fn unix_batch_deletes_an_open_read_only_file_from_a_writable_parent() {
    use std::{io::Read, os::unix::fs::PermissionsExt};

    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let selected = root.join("open-readonly.bin");
    fs::write(&selected, b"open fixture").unwrap();
    fs::set_permissions(&selected, fs::Permissions::from_mode(0o444)).unwrap();
    let mut open_file = fs::File::open(&selected).unwrap();
    let retained = root.join("retained.bin");
    fs::write(&retained, b"retained fixture").unwrap();
    let initial = analyze(&root);
    let selected = display_path(&selected);
    let batch =
        AnalysisService::delete_entries_permanently(initial.scan_id, vec![selected.clone()])
            .unwrap();
    assert_eq!(
        batch.removed_paths.as_slice(),
        std::slice::from_ref(&selected)
    );
    assert!(batch.failed.is_empty());
    assert!(!batch.requires_rescan);
    assert!(!Path::new(&selected).exists());
    let mut contents = Vec::new();
    open_file.read_to_end(&mut contents).unwrap();
    assert_eq!(contents, b"open fixture");
    assert_eq!(fs::read(&retained).unwrap(), b"retained fixture");
    let updated = batch
        .updated_results
        .iter()
        .find(|result| result.scan_id == initial.scan_id)
        .unwrap();
    assert!(updated.entries.iter().all(|entry| entry.path != selected));
    assert_eq!(updated.total_bytes, analyze(&root).total_bytes);
    cache::clear_all().unwrap();
}

#[cfg(windows)]
#[test]
fn windows_batch_retains_locked_items_continues_and_retries_after_unlock() {
    use std::os::windows::fs::OpenOptionsExt;
    let _operation_lock = crate::shared::operation::test_operation_lock();
    for locked_index in 0..3 {
        cache::clear_all().unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        let paths = ["first.bin", "middle.bin", "last.bin"].map(|name| root.join(name));
        for path in &paths {
            fs::write(path, b"batch fixture").unwrap();
        }
        let initial = analyze(&root);
        let selected = paths.each_ref().map(|path| display_path(path));
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&paths[locked_index])
            .unwrap();
        let batch = AnalysisService::delete_entries_permanently(initial.scan_id, selected.to_vec())
            .unwrap();
        drop(locked);
        assert_eq!(batch.failed.len(), 1);
        assert_eq!(batch.failed[0].path, selected[locked_index]);
        assert_eq!(
            batch.failed[0].reason,
            Some(crate::shared::CoreErrorReason::ResourceBusy)
        );
        assert_eq!(batch.removed_paths.len(), 2);
        assert!(!batch.requires_rescan);
        assert!(batch.not_attempted_paths.is_empty());
        assert_eq!(fs::read(&paths[locked_index]).unwrap(), b"batch fixture");
        let updated = batch
            .updated_results
            .iter()
            .find(|result| result.scan_id == initial.scan_id)
            .unwrap();
        assert_eq!(updated.entries.len(), 1);
        assert_eq!(updated.entries[0].path, selected[locked_index]);
        let retried = AnalysisService::delete_entries_permanently(
            initial.scan_id,
            vec![selected[locked_index].clone()],
        )
        .unwrap();
        assert_eq!(retried.removed_paths, [selected[locked_index].clone()]);
        assert!(retried.failed.is_empty());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }
    cache::clear_all().unwrap();
}

#[cfg(windows)]
#[test]
fn windows_batch_deletes_read_only_descendants_when_explicitly_confirmed() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let directory = root.join("partial");
    fs::create_dir(&directory).unwrap();
    for name in ["first.bin", "second.bin"] {
        fs::write(directory.join(name), [1; 4096]).unwrap();
    }
    let other_selected = root.join("other-selected.bin");
    fs::write(&other_selected, [2; 4096]).unwrap();
    let initial = analyze(&root);
    let children: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    let original_permissions = fs::metadata(&children[1]).unwrap().permissions();
    let mut read_only = original_permissions.clone();
    read_only.set_readonly(true);
    fs::set_permissions(&children[1], read_only).unwrap();
    let selected = [&directory, &other_selected].map(|path| display_path(path));
    let batch = AnalysisService::delete_entries_permanently(initial.scan_id, selected.to_vec());
    if children[1].exists() {
        fs::set_permissions(&children[1], original_permissions).unwrap();
    }
    let batch = batch.unwrap();
    assert_eq!(batch.removed_paths, selected);
    assert!(batch.failed.is_empty());
    assert!(!batch.requires_rescan);
    assert!(batch.not_attempted_paths.is_empty());
    assert!(!directory.exists());
    assert!(!other_selected.exists());
    assert!(fs::read_dir(&root).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".mangodisk-delete-")));
    cache::clear_all().unwrap();
}

#[cfg(windows)]
#[test]
fn windows_batch_stops_after_a_descendant_acl_causes_partial_deletion() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let directory = root.join("partial");
    for name in ["one", "two"] {
        fs::create_dir_all(directory.join(name)).unwrap();
        fs::write(directory.join(name).join("file.bin"), [1; 4096]).unwrap();
    }
    let untouched = root.join("untouched.bin");
    fs::write(&untouched, [2; 4096]).unwrap();
    let initial = analyze(&root);
    let children: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    let denied_file = children[1].join("file.bin");
    // Windows can authorize unlink through DELETE on the file or DELETE_CHILD
    // on its parent. Deny both while allowing directory enumeration.
    for (path, permission) in [
        (&denied_file, "*S-1-1-0:(D)"),
        (&children[1], "*S-1-1-0:(DC)"),
    ] {
        let output = std::process::Command::new("icacls.exe")
            .arg(path)
            .args(["/deny", permission])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "apply the isolated deletion denial"
        );
    }
    let selected = [&directory, &untouched].map(|path| display_path(path));
    let batch = {
        // Elevated SSH and CI runners may bypass ACLs through backup/restore
        // privileges. Restrict this thread so the fixture always exercises denial.
        let _token = windows_test_token::RestrictedToken::enter();
        AnalysisService::delete_entries_permanently(initial.scan_id, selected.to_vec())
    };
    for path in [&denied_file, &children[1]] {
        if path.exists() {
            let output = std::process::Command::new("icacls.exe")
                .arg(path)
                .args(["/remove:d", "*S-1-1-0"])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "restore the fixture ACL before assertions"
            );
        }
    }
    let batch = batch.unwrap();
    assert!(
        !children[0].exists(),
        "exercise an actual irreversible unlink"
    );
    assert!(
        denied_file.exists(),
        "restore the failed descendant: {batch:?}"
    );
    assert!(batch.requires_rescan);
    assert_eq!(batch.failed.len(), 1);
    assert_eq!(batch.failed[0].path, selected[0]);
    assert!(batch.removed_paths.is_empty());
    assert_eq!(batch.not_attempted_paths, [selected[1].clone()]);
    assert!(untouched.exists());
    assert!(batch.updated_results.is_empty());
    assert!(AnalysisService::resolve_open_target(initial.scan_id, selected[0].clone()).is_err());
    assert!(fs::read_dir(&root).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".mangodisk-delete-")));
    cache::clear_all().unwrap();
}

#[cfg(windows)]
#[test]
fn windows_batch_rejects_a_directory_replaced_with_a_junction() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let sandbox = fs::canonicalize(fixture.path()).unwrap();
    let root = sandbox.join("analysis-root");
    let external = sandbox.join("external");
    let replaced = root.join("replaced");
    fs::create_dir_all(&replaced).unwrap();
    fs::create_dir(&external).unwrap();
    fs::write(replaced.join("old.bin"), b"old fixture").unwrap();
    fs::write(external.join("retain.bin"), b"retained fixture").unwrap();
    let valid = root.join("valid.bin");
    fs::write(&valid, b"deletable fixture").unwrap();
    let initial = analyze(&root);
    fs::remove_dir_all(&replaced).unwrap();
    let output = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(display_path(&replaced))
        .arg(display_path(&external))
        .output()
        .unwrap();
    assert!(output.status.success(), "create a disposable NTFS junction");
    let selected = [&replaced, &valid].map(|path| display_path(path));
    let batch =
        AnalysisService::delete_entries_permanently(initial.scan_id, selected.to_vec()).unwrap();
    assert_eq!(batch.failed.len(), 1);
    assert_eq!(batch.failed[0].path, selected[0]);
    assert_eq!(batch.removed_paths, [selected[1].clone()]);
    assert!(!batch.requires_rescan);
    assert!(batch.not_attempted_paths.is_empty());
    assert_eq!(
        fs::read(external.join("retain.bin")).unwrap(),
        b"retained fixture"
    );
    assert!(replaced.exists());
    fs::remove_dir(&replaced).unwrap();
    cache::clear_all().unwrap();
}
