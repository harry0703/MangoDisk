use super::*;
use crate::storage::analysis::AnalysisScanMode;
use crate::{
    filesystem::metadata::display_path,
    storage::{
        analysis::{
            directory_loader::analysis_remainder_entries, index_projection::analysis_result,
            AnalysisRemainderParent, AnalysisRemainderRequest,
        },
        large_files::{
            index_projection::large_file_entries_from_snapshot, LARGE_FILE_CANDIDATE_FLOOR_BYTES,
        },
    },
};
use std::collections::HashSet;
use std::fs;
fn store_test_analysis_root(root: &Path, scanned_at_ms: u64) {
    let aggregate = DirectoryAggregate {
        scanned_at_ms,
        ..DirectoryAggregate::default()
    };
    store_memory_only(
        root,
        aggregate,
        HashMap::from([(root.to_path_buf(), aggregate)]),
        HashMap::new(),
        SnapshotPublication::new(
            ScanPurpose::Analysis,
            true,
            None,
            scanned_at_ms,
            mutation_revision().expect("test cache revision should load"),
        ),
    )
    .expect("test analysis result should store");
}

#[test]
fn remainder_entries_include_unindexed_small_files_and_reconcile_allocated_bytes() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    clear_all().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let allocated = |path: &Path| {
        current_platform()
            .file_space_usage(path, &fs::symlink_metadata(path).unwrap())
            .allocated_bytes
    };
    let visible = root.join("visible.bin");
    fs::write(&visible, vec![3; 32_768]).unwrap();
    let mut remainder_bytes = 0;
    for index in 0..230 {
        let path = root.join(format!("small-{index:03}.bin"));
        fs::write(&path, vec![index as u8; 1_024]).unwrap();
        remainder_bytes += allocated(&path);
    }
    let nested = root.join("nested");
    fs::create_dir(&nested).unwrap();
    let child = nested.join("child.bin");
    fs::write(&child, vec![1; 1_024]).unwrap();
    let child_bytes = allocated(&child);
    remainder_bytes += child_bytes;
    let aggregate = DirectoryAggregate {
        bytes: remainder_bytes + allocated(&visible),
        scanned_at_ms: 12,
        ..Default::default()
    };
    store_memory_only(
        &root,
        aggregate,
        HashMap::from([
            (root.clone(), aggregate),
            (
                nested.clone(),
                DirectoryAggregate {
                    bytes: child_bytes,
                    scanned_at_ms: 12,
                    ..Default::default()
                },
            ),
        ]),
        HashMap::new(),
        SnapshotPublication::new(
            ScanPurpose::Analysis,
            true,
            None,
            12,
            mutation_revision().unwrap(),
        ),
    )
    .unwrap();
    let parent = AnalysisRemainderParent {
        scan_mode: AnalysisScanMode::Standard,
        path: display_path(&root),
        bytes: aggregate.bytes,
        exclusions: Default::default(),
    };
    let mut request = AnalysisRemainderRequest {
        schema_version: AnalysisRemainderRequest::SCHEMA_VERSION,
        snapshot_id: None,
        scan_id: 1,
        parent_path: parent.path.clone(),
        visible_paths: vec![display_path(&visible)],
        expected_bytes: remainder_bytes,
        offset: 0,
    };
    let (total_bytes, entries) = analysis_remainder_entries(&root, &parent, &request).unwrap();
    assert_eq!(entries.len(), 231);
    assert_eq!(total_bytes, remainder_bytes);
    assert_eq!(
        entries.iter().map(|entry| entry.bytes).sum::<u64>(),
        remainder_bytes
    );
    assert_eq!(
        entries
            .iter()
            .map(|entry| &entry.path)
            .collect::<HashSet<_>>()
            .len(),
        231
    );
    assert!(entries
        .iter()
        .any(|entry| entry.name == "nested" && entry.is_directory));
    assert!(entries.iter().all(|entry| entry.name != "visible.bin"));
    fs::write(root.join("new.bin"), vec![7; 1_024]).unwrap();
    request.offset = 0;
    let (changed_bytes, changed_entries) =
        analysis_remainder_entries(&root, &parent, &request).unwrap();
    assert_eq!(changed_entries.len(), 232);
    assert!(changed_bytes > remainder_bytes);
}

#[test]
fn missing_change_token_is_stale() {
    let validation =
        validate_change_token(Path::new("/missing-change-token"), None, None, &|| false)
            .expect("validation should succeed");
    assert!(matches!(validation, ChangeValidation::Stale));
}

#[test]
fn large_file_entries_are_derived_from_an_index_snapshot() {
    let root = PathBuf::from("/memory-large-files");
    let file = root.join("large.bin");
    let mut files = HashMap::from([(
        file,
        IndexedFile {
            shared_identity: None,
            bytes: LARGE_FILE_CANDIDATE_FLOOR_BYTES,
            logical_bytes: LARGE_FILE_CANDIDATE_FLOOR_BYTES,
            modified_at_ms: Some(5),
        },
    )]);

    files.insert(
        root.join("chart-candidate.bin"),
        IndexedFile {
            shared_identity: None,
            bytes: 4096,
            logical_bytes: 4096,
            modified_at_ms: None,
        },
    );
    let entries = large_file_entries_from_snapshot(&root, &files);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].bytes, LARGE_FILE_CANDIDATE_FLOOR_BYTES);
}

#[test]
fn uncached_file_removal_subtracts_allocated_and_logical_sizes_independently() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    clear_all().expect("cache should clear");
    let root = PathBuf::from("/memory-analysis-removal");
    let file = root.join("small.bin");
    let aggregate = DirectoryAggregate {
        bytes: 4_096,
        logical_bytes: 1,
        file_count: 1,
        scanned_at_ms: 10,
        ..DirectoryAggregate::default()
    };
    store_memory_only(
        &root,
        aggregate,
        HashMap::from([(root.clone(), aggregate)]),
        HashMap::new(),
        SnapshotPublication::new(
            ScanPurpose::Analysis,
            true,
            None,
            10,
            mutation_revision().expect("test cache revision should load"),
        ),
    )
    .expect("analysis result should store");

    remove_entry(
        &file,
        FileSpaceUsage {
            logical_bytes: 1,
            allocated_bytes: 4_096,
        },
        1,
        false,
    );

    let cache = cache().lock().expect("cache should remain available");
    let updated = cache
        .directories
        .get(&root)
        .expect("the containing aggregate should remain cached");
    assert_eq!(updated.bytes, 0);
    assert_eq!(updated.logical_bytes, 0);
    assert_eq!(updated.file_count, 0);
    drop(cache);
    clear_all().expect("cache should clear");
}

#[test]
fn clearing_memory_removes_the_only_scan_result() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    clear_all().expect("cache should clear");
    let root = PathBuf::from("/memory-only-result");
    let aggregate = DirectoryAggregate {
        scanned_at_ms: 9,
        ..DirectoryAggregate::default()
    };
    store_memory_only(
        &root,
        aggregate,
        HashMap::from([(root.clone(), aggregate)]),
        HashMap::new(),
        SnapshotPublication::new(
            ScanPurpose::Analysis,
            true,
            None,
            9,
            mutation_revision().expect("test cache revision should load"),
        ),
    )
    .expect("memory result should store");
    clear_all().expect("cache should clear");
    assert!(analysis_result(&root)
        .expect("cache lookup should succeed")
        .is_none());
}

#[test]
fn older_concurrent_snapshot_cannot_replace_a_newer_generation() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    clear_all().expect("cache should clear");
    let root = PathBuf::from("/memory-generation-order");
    let newer = DirectoryAggregate {
        bytes: 2,
        logical_bytes: 2,
        file_count: 1,
        scanned_at_ms: 2,
        ..DirectoryAggregate::default()
    };
    let revision = mutation_revision().expect("cache revision should load");
    assert!(store_memory_only(
        &root,
        newer,
        HashMap::from([(root.clone(), newer)]),
        HashMap::new(),
        SnapshotPublication::new(ScanPurpose::Analysis, true, None, 20, revision),
    )
    .expect("newer snapshot should publish"));

    let older = DirectoryAggregate {
        bytes: 1,
        logical_bytes: 1,
        file_count: 1,
        scanned_at_ms: 1,
        ..DirectoryAggregate::default()
    };
    assert!(!store_memory_only(
        &root,
        older,
        HashMap::from([(root.clone(), older)]),
        HashMap::new(),
        SnapshotPublication::new(ScanPurpose::Analysis, true, None, 10, revision),
    )
    .expect("older snapshot should be skipped safely"));
    assert_eq!(
        cache()
            .lock()
            .expect("cache should remain available")
            .directories
            .get(&root)
            .expect("newer root should remain")
            .scanned_at_ms,
        2
    );
    clear_all().expect("cache should clear");
}

#[test]
fn older_ancestor_snapshot_cannot_erase_a_newer_descendant() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    clear_all().expect("cache should clear");
    let root = PathBuf::from("/memory-overlapping-generation");
    let descendant = root.join("nested");
    let newer = DirectoryAggregate {
        bytes: 2,
        logical_bytes: 2,
        file_count: 1,
        scanned_at_ms: 2,
        ..DirectoryAggregate::default()
    };
    let revision = mutation_revision().expect("cache revision should load");
    assert!(store_memory_only(
        &descendant,
        newer,
        HashMap::from([(descendant.clone(), newer)]),
        HashMap::new(),
        SnapshotPublication::new(ScanPurpose::Analysis, true, None, 20, revision),
    )
    .expect("newer descendant should publish"));

    let older = DirectoryAggregate {
        bytes: 1,
        logical_bytes: 1,
        file_count: 1,
        scanned_at_ms: 1,
        ..DirectoryAggregate::default()
    };
    assert!(!store_memory_only(
        &root,
        older,
        HashMap::from([(root.clone(), older)]),
        HashMap::new(),
        SnapshotPublication::new(ScanPurpose::Analysis, true, None, 10, revision),
    )
    .expect("older ancestor should be skipped safely"));
    let cache = cache().lock().expect("cache should remain available");
    assert!(cache.directories.contains_key(&descendant));
    assert!(!cache.directories.contains_key(&root));
    drop(cache);
    clear_all().expect("cache should clear");
}

#[test]
fn newer_ancestor_replaces_cached_descendant_without_an_explicit_refresh() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    clear_all().expect("cache should clear");
    let root = PathBuf::from("/memory-overlapping-replacement");
    let descendant = root.join("nested");
    let unrelated = PathBuf::from("/memory-overlapping-unrelated");
    let stale_file = descendant.join("stale.bin");
    store_test_analysis_root(&unrelated, 5);
    let old = DirectoryAggregate {
        bytes: 1,
        logical_bytes: 1,
        file_count: 1,
        scanned_at_ms: 1,
        ..DirectoryAggregate::default()
    };
    let revision = mutation_revision().expect("cache revision should load");
    assert!(store_memory_only(
        &descendant,
        old,
        HashMap::from([(descendant.clone(), old)]),
        HashMap::from([(
            stale_file.clone(),
            IndexedFile {
                shared_identity: None,
                bytes: 1,
                logical_bytes: 1,
                modified_at_ms: None,
            },
        )]),
        SnapshotPublication::new(ScanPurpose::Analysis, true, None, 10, revision),
    )
    .expect("descendant snapshot should publish"));

    let replacement = DirectoryAggregate {
        bytes: 2,
        logical_bytes: 2,
        file_count: 1,
        scanned_at_ms: 2,
        ..DirectoryAggregate::default()
    };
    assert!(store_memory_only(
        &root,
        replacement,
        HashMap::from([
            (root.clone(), replacement),
            (descendant.clone(), replacement),
        ]),
        HashMap::new(),
        SnapshotPublication::new(ScanPurpose::Analysis, false, None, 20, revision),
    )
    .expect("newer ancestor snapshot should publish"));

    let cache = cache().lock().expect("cache should remain available");
    assert_eq!(cache.scan_roots.len(), 2);
    assert!(cache.scan_roots.contains_key(&root));
    assert!(cache.scan_roots.contains_key(&unrelated));
    assert!(!cache.scan_roots.contains_key(&descendant));
    assert!(!cache.files.contains_key(&stale_file));
    assert_eq!(
        cache
            .directories
            .get(&descendant)
            .expect("replacement descendant should remain")
            .scanned_at_ms,
        2
    );
    drop(cache);
    clear_all().expect("cache should clear");
}

#[test]
fn concurrent_mutation_prevents_stale_snapshot_publication() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    clear_all().expect("cache should clear");
    let root = PathBuf::from("/memory-concurrent-mutation");
    let aggregate = DirectoryAggregate {
        bytes: 1,
        logical_bytes: 1,
        file_count: 1,
        scanned_at_ms: 1,
        ..DirectoryAggregate::default()
    };
    let revision = mutation_revision().expect("cache revision should load");
    remove_entry(
        &root.join("removed.bin"),
        FileSpaceUsage {
            logical_bytes: 1,
            allocated_bytes: 1,
        },
        1,
        false,
    );

    assert!(!store_memory_only(
        &root,
        aggregate,
        HashMap::from([(root.clone(), aggregate)]),
        HashMap::new(),
        SnapshotPublication::new(ScanPurpose::Analysis, true, None, 1, revision),
    )
    .expect("stale publication should be skipped safely"));
    assert!(!cache()
        .lock()
        .expect("cache should remain available")
        .directories
        .contains_key(&root));
    clear_all().expect("cache should clear");
}

#[test]
fn storing_a_third_root_evicts_only_the_least_recent_root() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    clear_all().expect("cache should clear");
    let root_a = PathBuf::from("/memory-lru-a");
    let root_b = PathBuf::from("/memory-lru-b");
    let root_c = PathBuf::from("/memory-lru-c");

    store_test_analysis_root(&root_a, 1);
    store_test_analysis_root(&root_b, 2);
    store_test_analysis_root(&root_c, 3);

    let cache = cache().lock().expect("cache should be readable");
    assert!(!cache.scan_roots.contains_key(&root_a));
    assert!(!cache.directories.contains_key(&root_a));
    assert!(cache.scan_roots.contains_key(&root_b));
    assert!(cache.directories.contains_key(&root_b));
    assert!(cache.scan_roots.contains_key(&root_c));
    assert!(cache.directories.contains_key(&root_c));
    assert_eq!(
        cache.root_recency,
        VecDeque::from([root_b.clone(), root_c.clone()])
    );
    drop(cache);
    clear_all().expect("cache should clear");
}

#[test]
fn marking_a_root_recent_updates_its_eviction_recency() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    clear_all().expect("cache should clear");
    let root_a = PathBuf::from("/memory-lru-reused-a");
    let root_b = PathBuf::from("/memory-lru-reused-b");
    let root_c = PathBuf::from("/memory-lru-reused-c");

    store_test_analysis_root(&root_a, 1);
    store_test_analysis_root(&root_b, 2);
    mark_root_recent(&root_a).expect("the cached root should move to the recent end");
    store_test_analysis_root(&root_c, 3);

    let cache = cache().lock().expect("cache should be readable");
    assert!(cache.scan_roots.contains_key(&root_a));
    assert!(!cache.scan_roots.contains_key(&root_b));
    assert!(cache.scan_roots.contains_key(&root_c));
    assert_eq!(
        cache.root_recency,
        VecDeque::from([root_a.clone(), root_c.clone()])
    );
    drop(cache);
    clear_all().expect("cache should clear");
}

#[test]
fn directory_removal_reconciles_omissions_in_ancestors() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    clear_all().unwrap();
    let root = PathBuf::from("/coverage-removal");
    let child = root.join("child");
    let aggregate = DirectoryAggregate {
        bytes: 4096,
        logical_bytes: 4096,
        file_count: 1,
        skipped_count: 2,
        omissions: mangodisk_platform::ScanOmissionCounts {
            permission_denied: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    store_memory_only(
        &root,
        aggregate,
        HashMap::from([(root.clone(), aggregate), (child.clone(), aggregate)]),
        HashMap::new(),
        SnapshotPublication::new(
            ScanPurpose::Analysis,
            true,
            None,
            10,
            mutation_revision().unwrap(),
        ),
    )
    .unwrap();
    remove_entry(
        &child,
        FileSpaceUsage {
            logical_bytes: 4096,
            allocated_bytes: 4096,
        },
        1,
        true,
    );
    let snapshot = cache().lock().unwrap().directories[&root];
    assert_eq!(snapshot.skipped_count, 0);
    assert_eq!(snapshot.omissions.total(), 0);
    clear_all().unwrap();
}
