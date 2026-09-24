use std::{
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "macos")]
use std::time::Duration;

use super::*;

struct DirectoryCleanup(PathBuf);

#[cfg(windows)]
#[test]
#[ignore = "creates an isolated fixture under an explicitly supplied shared directory"]
fn real_redirected_share_supports_storage_scans() {
    use crate::storage::{
        analysis::AnalysisService,
        duplicates::{DuplicateFileService, DuplicateScanLocation, DuplicateScanLocationMode},
        large_files::LargeFileService,
    };
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let parent = std::env::var("MANGODISK_TEST_SHARED_SCAN_PARENT")
        .expect("supply a redirected shared directory for isolated test files");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let entry = PathBuf::from(parent).join(format!("MangoDisk-Shared-Scan-{unique}"));
    fs::create_dir(&entry).expect("create the isolated shared fixture");
    let _cleanup = DirectoryCleanup(entry.clone());
    let data = vec![0x5a_u8; LARGE_FILE_CANDIDATE_FLOOR_BYTES as usize + 4096];
    fs::write(entry.join("first.bin"), &data).unwrap();
    fs::write(entry.join("second.bin"), &data).unwrap();
    fs::create_dir(entry.join("nested")).unwrap();
    fs::write(entry.join("nested/unique.txt"), b"different content").unwrap();
    let excluded = entry.join("excluded-small-files");
    fs::create_dir(&excluded).expect("create the high-file-count excluded directory");
    for index in 0..2_000 {
        fs::write(excluded.join(format!("small-{index}.tmp")), [index as u8])
            .expect("write an excluded small-file fixture");
    }
    let canonical = current_platform()
        .resolve_directory_entry(&entry)
        .expect("resolve the redirected shared directory");
    let root = current_platform().display_path(&canonical);
    let excluded = current_platform()
        .resolve_directory_entry(&excluded)
        .map(|path| current_platform().display_path(&path))
        .expect("resolve the excluded high-file-count directory");
    let baseline_started = Instant::now();
    let unfiltered = DuplicateFileService::find_paged_with_locations_and_exclusions(
        vec![DuplicateScanLocation {
            path: root.clone(),
            mode: DuplicateScanLocationMode::Cleanable,
        }],
        Vec::new(),
        1,
        |_| {},
        |_| {},
    )
    .expect("scan the complete high-file-count fixture");
    let baseline_ms = baseline_started.elapsed().as_millis();
    assert_eq!(unfiltered.scanned_file_count, 2_003);
    let started = Instant::now();
    let analysis = AnalysisService::analyze_with_exclusions_progress(
        Some(root.clone()),
        true,
        vec![excluded.clone()],
        |_| {},
    )
    .unwrap();
    assert_eq!(
        analysis
            .entries
            .iter()
            .map(|entry| entry.file_count)
            .sum::<u64>(),
        3
    );
    assert!(analysis
        .entries
        .iter()
        .all(|entry| entry.name != "excluded-small-files"));
    let large = LargeFileService::find_with_progress(
        vec![root.clone()],
        1,
        LargeFileScanMode::Complete,
        vec![excluded.clone()],
        |_| {},
    )
    .unwrap();
    assert_eq!(large.entries.len(), 2);
    let filtered_duplicate_started = Instant::now();
    let duplicates = DuplicateFileService::find_paged_with_locations_and_exclusions(
        vec![DuplicateScanLocation {
            path: root,
            mode: DuplicateScanLocationMode::Cleanable,
        }],
        vec![excluded],
        1,
        |_| {},
        |_| {},
    )
    .unwrap();
    let filtered_duplicate_ms = filtered_duplicate_started.elapsed().as_millis();
    assert_eq!(duplicates.scanned_file_count, 3);
    assert_eq!(duplicates.groups.len(), 1);
    assert_eq!(duplicates.groups[0].entries.len(), 2);
    assert_eq!(duplicates.groups[0].bytes_per_file, data.len() as u64);
    println!(
        "shared_scan_exclusions_verified included_files=3 excluded_files=2000 large_files=2 duplicate_groups=1 unfiltered_duplicate_ms={baseline_ms} filtered_duplicate_ms={filtered_duplicate_ms} filtered_workflow_ms={}",
        started.elapsed().as_millis()
    );
}

impl Drop for DirectoryCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn saved_parent_exclusion_prunes_explicit_large_and_duplicate_scan_roots() {
    use crate::storage::duplicates::{
        DuplicateFileService, DuplicateScanLocation, DuplicateScanLocationMode,
    };

    let _operation_lock = crate::shared::operation::test_operation_lock();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let parent = std::env::temp_dir().join(format!(
        "mangodisk-explicit-root-exclusion-{}-{unique}",
        std::process::id()
    ));
    let root = parent.join("selected");
    fs::create_dir_all(&root).expect("create the selected scan root");
    let _cleanup = DirectoryCleanup(parent.clone());
    let contents = vec![0x42_u8; LARGE_FILE_CANDIDATE_FLOOR_BYTES as usize + 1];
    fs::write(root.join("first.bin"), &contents).expect("create the first scan candidate");
    fs::write(root.join("second.bin"), &contents).expect("create the second scan candidate");
    let root_value = current_platform().display_path(&root);
    let parent_value = current_platform().display_path(&parent);

    let (large, diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
        vec![root_value.clone()],
        1,
        LargeFileScanMode::Complete,
        vec![parent_value.clone()],
        |_| {},
    )
    .expect("exclude the selected large-file root");
    assert_eq!(large.total_count, 0);
    assert_eq!(diagnostics.candidate_strategy, "excluded_root");

    let duplicates = DuplicateFileService::find_paged_with_locations_and_exclusions(
        vec![DuplicateScanLocation {
            path: root_value,
            mode: DuplicateScanLocationMode::Cleanable,
        }],
        vec![parent_value],
        1,
        |_| {},
        |_| {},
    )
    .expect("exclude the selected duplicate-file root");
    assert_eq!(duplicates.scanned_file_count, 0);
    assert!(duplicates.groups.is_empty());
}

#[test]
fn traversal_cancellation_preserves_the_typed_error_code() {
    let error = traversal_core_error(OPERATION_CANCELLED_ERROR.to_string());

    assert_eq!(
        error.code(),
        crate::shared::CoreErrorCode::OperationCancelled
    );
}

#[test]
fn native_worker_shutdown_preserves_the_retryable_busy_code() {
    // Other tests may hold the process-wide operation slot in parallel.
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let operation = OperationGuard::start(CoordinatedOperationKind::Analysis)
        .expect("the isolated analysis operation should start");
    let error = analysis_stream_core_error(&operation, AnalysisStreamError::ResourcesReleasing);

    assert_eq!(error.code(), crate::shared::CoreErrorCode::OperationBusy);
    assert_eq!(
        error.reason(),
        Some(crate::shared::CoreErrorReason::ScanResourcesReleasing)
    );
}

#[test]
fn native_large_file_candidate_below_physical_threshold_is_not_skipped() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "MangoDisk-Large-Candidate-{}-{unique}",
        std::process::id()
    ));
    let _sandbox_cleanup = DirectoryCleanup(root.clone());
    fs::create_dir_all(&root).expect("create the large-file candidate fixture");
    let path = root.join("candidate.bin");
    fs::write(&path, [1_u8, 2, 3, 4]).expect("write the large-file candidate fixture");
    let metadata = fs::metadata(&path).expect("read the large-file candidate metadata");
    let allocated = current_platform()
        .file_space_usage(&path, &metadata)
        .allocated_bytes;
    let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
    let cancelled = AtomicBool::new(false);
    let exclusions = StorageScanExclusions::resolve(&root, &[]).expect("prepare empty exclusions");
    let mut validation = LargeFileStreamValidation::new(
        &root,
        allocated.saturating_add(1),
        now_ms(),
        &progress,
        &cancelled,
        true,
        &exclusions,
    )
    .expect("prepare native large-file validation");
    let mut sink = IndexRecordSink::memory(None);

    validation
        .consume(path, &mut sink)
        .expect("filter the ineligible native candidate");

    assert_eq!(validation.valid_count, 0);
    assert_eq!(validation.aggregate.skipped_count, 0);
}

#[test]
fn duplicate_native_large_file_candidate_is_idempotent() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "MangoDisk-Duplicate-Large-Candidate-{}-{unique}",
        std::process::id()
    ));
    let _sandbox_cleanup = DirectoryCleanup(root.clone());
    fs::create_dir_all(&root).expect("create the duplicate candidate fixture");
    let path = root.join("candidate.bin");
    fs::write(&path, [1_u8, 2, 3, 4]).expect("write the duplicate candidate fixture");
    let metadata = fs::metadata(&path).expect("read the duplicate candidate metadata");
    let usage = current_platform().file_space_usage(&path, &metadata);
    let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
    let cancelled = AtomicBool::new(false);
    let exclusions = StorageScanExclusions::resolve(&root, &[]).expect("prepare empty exclusions");
    let mut validation = LargeFileStreamValidation::new(
        &root,
        0,
        now_ms(),
        &progress,
        &cancelled,
        true,
        &exclusions,
    )
    .expect("prepare native large-file validation");
    let mut sink = IndexRecordSink::memory(None);

    validation
        .consume(path.clone(), &mut sink)
        .expect("accept the first native candidate");
    validation
        .consume(path, &mut sink)
        .expect("ignore a duplicate native candidate");

    assert_eq!(validation.valid_count, 1);
    assert_eq!(validation.aggregate.bytes, usage.allocated_bytes);
    assert_eq!(validation.aggregate.logical_bytes, usage.logical_bytes);
    assert_eq!(validation.aggregate.file_count, 1);
    assert_eq!(validation.aggregate.skipped_count, 0);
    assert_eq!(
        sink.finish()
            .expect("finish the deduplicated candidate index")
            .files
            .len(),
        1
    );
}

#[cfg(target_os = "macos")]
#[test]
fn analysis_filesystem_boundary_keeps_firmlinks_and_rejects_mounts() {
    let platform = current_platform();
    let root = fs::symlink_metadata("/").expect("the system volume metadata should be readable");
    let users =
        fs::symlink_metadata("/Users").expect("the user-directory metadata should be readable");
    let device_mount =
        fs::symlink_metadata("/dev").expect("the device mount metadata should be readable");

    assert!(platform.is_same_filesystem(&root, &users));
    assert!(!platform.is_same_filesystem(&root, &device_mount));
}

#[test]
fn isolated_analysis_scans_only_requested_root() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().expect("the memory cache should be cleared before the test");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "MangoDisk-Analysis-{}-{unique}",
        std::process::id()
    ));
    let _sandbox_cleanup = DirectoryCleanup(root.clone());
    let fixture = root.join("fingerprint-test").join("sample.bin");
    fs::create_dir_all(
        fixture
            .parent()
            .expect("the fixture file should have a parent directory"),
    )
    .expect("the analysis fixture directory should be created");
    fs::write(&fixture, [1_u8, 2, 3, 4, 5, 6])
        .expect("the analysis fixture file should be written");
    let expected_allocated = current_platform()
        .file_space_usage(
            &fixture,
            &fs::metadata(&fixture).expect("the analysis fixture metadata should be readable"),
        )
        .allocated_bytes;

    let result = StorageTraversal::analyze_path_with_progress(
        Some(root.to_string_lossy().into_owned()),
        true,
        |_| {},
    )
    .expect("analysis of the isolated directory should succeed");

    assert_eq!(result.total_bytes, expected_allocated);
    assert_eq!(result.skipped_count, 0);
    assert_eq!(result.entries.len(), 1);
    assert_eq!(result.entries[0].name, "fingerprint-test");
    assert_eq!(
        cache::memory_entry_counts().expect("memory-cache counts should be readable"),
        (1, 2, 0),
        "a completed analysis should publish one authoritative in-memory directory snapshot"
    );
}

#[test]
fn analysis_exclusions_prune_results_and_invalidate_the_previous_snapshot() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().expect("clear the analysis cache before exclusion validation");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "MangoDisk-Analysis-Exclusions-{}-{unique}",
        std::process::id()
    ));
    let _sandbox_cleanup = DirectoryCleanup(root.clone());
    let included = root.join("included");
    let excluded = root.join("excluded");
    fs::create_dir_all(&included).expect("create the included analysis directory");
    fs::create_dir_all(&excluded).expect("create the excluded analysis directory");
    fs::write(included.join("kept.bin"), vec![1_u8; 4096])
        .expect("write the included analysis fixture");
    fs::write(excluded.join("omitted.bin"), vec![2_u8; 8192])
        .expect("write the excluded analysis fixture");
    let root_value = current_platform().display_path(&root);
    let canonical_root = current_platform()
        .canonicalize_no_links(&root)
        .expect("canonicalize the analysis exclusion root");
    let canonical_excluded = current_platform()
        .canonicalize_no_links(&excluded)
        .expect("canonicalize the excluded analysis directory");
    let validated_exclusions = StorageScanExclusions::resolve(
        &canonical_root,
        &[current_platform().display_path(&excluded)],
    )
    .expect("resolve the analysis exclusion fixture");
    assert!(
        validated_exclusions.matches(&canonical_excluded),
        "resolved exclusions do not cover the fixture: {:?}",
        validated_exclusions.roots()
    );

    let complete = StorageTraversal::analyze_path_with_exclusions_progress(
        Some(root_value.clone()),
        true,
        Vec::new(),
        |_| {},
    )
    .expect("scan the complete fixture");
    assert!(complete
        .entries
        .iter()
        .any(|entry| entry.name == "excluded"));

    let (filtered, diagnostics) = StorageTraversal::analyze_path_with_exclusions_diagnostics(
        Some(root_value),
        false,
        vec![current_platform().display_path(&excluded)],
        |_| {},
    )
    .expect("rescan after the exclusion configuration changes");
    assert_ne!(
        diagnostics.fast_path, "cache",
        "a changed exclusion configuration must not reuse the previous snapshot"
    );
    assert!(filtered
        .entries
        .iter()
        .any(|entry| entry.name == "included"));
    assert!(
        filtered
            .entries
            .iter()
            .all(|entry| entry.name != "excluded"),
        "excluded analysis entry survived: {:?}",
        filtered
            .entries
            .iter()
            .map(|entry| (&entry.name, &entry.path, entry.bytes))
            .collect::<Vec<_>>()
    );
    assert!(filtered.total_bytes < complete.total_bytes);

    let cached = cache::analysis_result(&canonical_root)
        .expect("read the exclusion-aware analysis snapshot")
        .expect("the completed analysis snapshot should be cached");
    assert!(cached.entries.iter().all(|entry| entry.name != "excluded"));
}

#[test]
fn analysis_rejects_a_root_covered_by_an_exclusion() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().expect("create an isolated analysis fixture");
    let root = fixture.path().join("excluded-root");
    fs::create_dir(&root).expect("create the excluded analysis root");
    fs::write(root.join("hidden.bin"), vec![1_u8; 4096]).expect("create excluded content");
    let path = current_platform().display_path(&root);

    for excluded in [
        path.clone(),
        current_platform().display_path(fixture.path()),
    ] {
        let error = StorageTraversal::analyze_path_with_exclusions_progress(
            Some(path.clone()),
            true,
            vec![excluded],
            |_| {},
        )
        .expect_err("an excluded analysis root should not reach filesystem traversal");

        assert_eq!(error.code(), crate::CoreErrorCode::InvalidInput);
        assert_eq!(
            error.reason(),
            Some(crate::CoreErrorReason::AnalysisRootExcluded)
        );
    }
}

#[test]
fn fast_analysis_contract_validates_record_counts_before_publish() {
    let root = Path::new("/fixture");
    let progress = Arc::new(ProgressTracker::new(1, |_| {}, 0));
    let cancelled = AtomicBool::new(false);
    let exclusions =
        StorageScanExclusions::resolve(root, &[]).expect("prepare empty analysis exclusions");
    let mut validation =
        FastAnalysisStreamValidation::new(root, 100, &progress, &cancelled, &exclusions);
    let mut sink = IndexRecordSink::memory(None);
    validation
        .consume(
            FastAnalysisRecord::Directory {
                path: root.join("child"),
                logical_bytes: 5,
                allocated_bytes: 5,
                file_count: 1,
                skipped_count: 0,
            },
            &mut sink,
        )
        .expect("the child-directory record should be written");
    validation
        .consume(
            FastAnalysisRecord::Directory {
                path: root.to_path_buf(),
                logical_bytes: 5,
                allocated_bytes: 5,
                file_count: 1,
                skipped_count: 0,
            },
            &mut sink,
        )
        .expect("the root-directory record should be written");
    validation
        .consume(
            FastAnalysisRecord::LargeFileCandidate(root.join("missing.bin")),
            &mut sink,
        )
        .expect("a candidate disappearing after enumeration should not invalidate directories");
    let mut summary = FastAnalysisSummary {
        root_logical_bytes: 5,
        root_allocated_bytes: 5,
        root_file_count: 1,
        root_skipped_count: 0,
        page_count: 1,
        entry_count: 3,
        directory_count: 1,
        candidate_count: 1,
        returned_bytes: 128,
        consumer_elapsed_ms: 0,
        strategy: "test",
    };

    assert!(
        validation.complete(&summary).is_err(),
        "a snapshot must not publish when directory records disagree with the summary"
    );
    summary.directory_count = 2;
    assert_eq!(
        validation
            .complete(&summary)
            .expect("a complete record stream should satisfy contract validation")
            .bytes,
        5
    );
}

#[test]
fn fast_analysis_progress_accumulates_batches_without_repeating_final_totals() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&events);
    let progress = Arc::new(ProgressTracker::new(
        1,
        move |event| {
            captured
                .lock()
                .expect("the progress event lock should remain valid")
                .push(event)
        },
        0,
    ));
    let root = Path::new("/fixture");
    let mut validation = FastAnalysisProgressValidation::new(root, &progress);

    validation.observe(&root.join("first"), 2, 30);
    validation.observe(&root.join("second"), 3, 70);
    validation
        .complete(DirectoryAggregate {
            bytes: 100,
            file_count: 5,
            ..DirectoryAggregate::default()
        })
        .expect("matching progress batches should satisfy the final aggregate");
    progress.finish(TraversalStage::Analyzing, root);

    let events = events
        .lock()
        .expect("the progress event lock should remain valid");
    let final_event = events.last().expect("final progress should be published");
    assert_eq!(final_event.items_scanned, 5);
    assert_eq!(final_event.bytes_scanned, 100);
}

#[test]
fn memory_index_rejects_duplicate_stream_records() {
    let path = PathBuf::from("/fixture");
    let aggregate = DirectoryAggregate {
        bytes: 1,
        file_count: 1,
        ..DirectoryAggregate::default()
    };
    let mut sink = IndexRecordSink::memory(None);

    sink.push_directory(path.clone(), aggregate)
        .expect("the first directory record should be written");
    assert!(
        sink.push_directory(path, aggregate).is_err(),
        "an in-memory retry must not hide duplicate platform records by overwriting them"
    );
}

#[cfg(target_os = "macos")]
fn analyze_until_total_bytes(path: Option<String>, expected: u64) -> AnalysisResult {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let result = StorageTraversal::analyze_path_with_progress(path.clone(), false, |_| {})
            .expect("analysis should succeed after an FSEvents change");
        if result.total_bytes == expected {
            return result;
        }
        assert!(
                Instant::now() < deadline,
                "FSEvents did not invalidate the cache before the deadline: actual={} expected={expected}",
                result.total_bytes
            );
        // FSEvents delivery is asynchronous by design. Polling the real cache entry verifies
        // eventual monitor behavior without imposing the full timeout on fast machines.
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires an explicit real FSEvents cache-invalidation diagnostic"]
fn macos_file_create_modify_and_delete_invalidate_analysis_snapshot() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().expect("the memory cache should be cleared before the test");
    let root = std::env::temp_dir().join(format!(
        "mangodisk-fsevents-cache-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let _sandbox_cleanup = DirectoryCleanup(root.clone());
    fs::create_dir_all(&root).expect("the FSEvents cache fixture should be created");
    fs::write(root.join("stable.bin"), [1_u8; 6])
        .expect("the initial fixture file should be written");
    let path = Some(root.to_string_lossy().into_owned());

    let initial = StorageTraversal::analyze_path_with_progress(path.clone(), true, |_| {})
        .expect("the initial analysis should succeed");
    assert_eq!(initial.total_bytes, 6);

    let changed = root.join("changed.bin");
    let file = fs::File::create(&changed).expect("the new fixture file should be created");
    file.set_len(4)
        .expect("the new fixture file size should be set");
    file.sync_all()
        .expect("the new fixture file should be synchronized");
    let after_create = analyze_until_total_bytes(path.clone(), 10);
    assert_eq!(after_create.total_bytes, 10);

    let file = fs::OpenOptions::new()
        .write(true)
        .open(&changed)
        .expect("the fixture file should open for modification");
    file.set_len(8)
        .expect("the fixture file size should be modified");
    file.sync_all()
        .expect("the modified fixture file should be synchronized");
    let after_modify = analyze_until_total_bytes(path.clone(), 14);
    assert_eq!(after_modify.total_bytes, 14);

    fs::remove_file(changed).expect("the fixture file should be removed");
    let after_delete = analyze_until_total_bytes(path, 6);
    assert_eq!(after_delete.total_bytes, 6);
}

#[test]
fn large_file_session_supports_switching_from_high_threshold_to_candidate_floor() {
    let root = std::env::temp_dir().join(format!(
        "mangodisk-large-file-cache-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let files = [("60-mb.bin", 60), ("120-mb.bin", 120), ("600-mb.bin", 600)]
        .into_iter()
        .map(|(name, mebibytes)| {
            let bytes = mebibytes * 1024 * 1024;
            (
                root.join(name),
                IndexedFile {
                    bytes,
                    logical_bytes: bytes,
                    modified_at_ms: None,
                },
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let aggregate = DirectoryAggregate {
        bytes: files.values().map(|file| file.bytes).sum(),
        logical_bytes: files.values().map(|file| file.logical_bytes).sum(),
        file_count: files.len() as u64,
        scanned_at_ms: now_ms(),
        ..DirectoryAggregate::default()
    };
    let retained_entries = cache::large_file_entries_from_snapshot(&root, &files);
    let result = LargeFilesResult::from_retained_entries(
        vec![current_platform().display_path(&root)],
        aggregate.scanned_at_ms,
        LargeFileScanMode::Complete,
        500 * 1024 * 1024,
        0,
        retained_entries,
    );
    let high_threshold = result.filtered(500 * 1024 * 1024);
    assert_eq!(high_threshold.entries.len(), 1);

    let low_threshold = result.filtered(LARGE_FILE_CANDIDATE_FLOOR_BYTES);
    assert_eq!(low_threshold.entries.len(), 3);
}

/// Validates platform fast scanning and recursive fallback against a real directory selected
/// through the development environment variable. The test is ignored by default so CI cannot
/// traverse a large disk accidentally. Local runs must set `MANGODISK_ANALYSIS_ROOT` explicitly.
#[test]
#[ignore = "requires an explicit real scan root in MANGODISK_ANALYSIS_ROOT"]
fn real_large_file_scan_completes_fast_path_or_recursive_fallback() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::var(ANALYSIS_ROOT_ENV)
        .expect("MANGODISK_ANALYSIS_ROOT must be set before a real large-file scan");
    let (result, diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
        vec![root],
        LARGE_FILE_CANDIDATE_FLOOR_BYTES,
        LargeFileScanMode::Complete,
        vec![],
        |_| {},
    )
    .expect("the real large-file scan should succeed");

    println!(
            "real_large_file_scan results={} bytes={} skipped={} fast_path={} strategy={} candidates={} peak_in_flight={} discovery_ms={} validation_ms={}",
            result.total_count,
            result.total_bytes,
            result.skipped_count,
            diagnostics.fast_path,
            diagnostics.candidate_strategy,
            diagnostics.candidate_count,
            diagnostics.candidate_peak_in_flight,
            diagnostics.candidate_discovery_ms,
            diagnostics.validation_or_traversal_ms
        );
    assert!(result
        .entries
        .iter()
        .all(|entry| entry.bytes >= LARGE_FILE_CANDIDATE_FLOOR_BYTES));
    if diagnostics.fast_path == "used" {
        assert!(
            diagnostics.candidate_count >= result.total_count,
            "platform candidates must cover every valid result"
        );
    }
}

/// Exercises the explicit indexed mode against a real root. This diagnostic intentionally does
/// not compare it with complete mode because files created before Spotlight catches up are an
/// expected product distinction, not a correctness failure.
#[test]
#[ignore = "requires an explicit real scan root in MANGODISK_ANALYSIS_ROOT"]
fn real_quick_large_file_scan_uses_platform_index() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::var(ANALYSIS_ROOT_ENV)
        .expect("MANGODISK_ANALYSIS_ROOT must be set before a real quick large-file scan");
    let (result, diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
        vec![root],
        LARGE_FILE_CANDIDATE_FLOOR_BYTES,
        LargeFileScanMode::Quick,
        vec![],
        |_| {},
    )
    .expect("the platform index should serve the real quick scan");

    println!(
        "real_quick_large_file_scan results={} bytes={} skipped={} strategy={} candidates={} peak_in_flight={} discovery_ms={}",
        result.total_count,
        result.total_bytes,
        result.skipped_count,
        diagnostics.candidate_strategy,
        diagnostics.candidate_count,
        diagnostics.candidate_peak_in_flight,
        diagnostics.candidate_discovery_ms
    );
    assert_eq!(result.scan_mode, LargeFileScanMode::Quick);
    assert_eq!(diagnostics.fast_path, "used");
    assert!(!diagnostics.candidate_strategy.is_empty());
    if diagnostics.candidate_count > 0 {
        assert!(diagnostics.candidate_peak_in_flight > 0);
    }
}

/// Measures the complete in-memory analysis representation for a real directory tree.
#[test]
#[ignore = "requires an explicit real scan root in MANGODISK_ANALYSIS_ROOT"]
fn real_analysis_materializes_complete_memory_index() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::var(ANALYSIS_ROOT_ENV)
        .expect("MANGODISK_ANALYSIS_ROOT must be set before real in-memory analysis");
    let canonical_root = current_platform()
        .canonicalize_no_links(Path::new(&root))
        .expect("the real scan root should be safely accessible");
    let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
    let cancelled = AtomicBool::new(false);
    let started = Instant::now();
    let mut sink = IndexRecordSink::memory(None);
    let exclusions = StorageScanExclusions::resolve(&canonical_root, &[])
        .expect("the empty exclusion set should resolve");
    let (aggregate, summary) = stream_fast_analysis_once(
        &canonical_root,
        now_ms(),
        &progress,
        &cancelled,
        &exclusions,
        &mut sink,
    )
    .expect("the real in-memory analysis fast path should not fail")
    .expect("the platform must support native analysis for this diagnostic");
    let elapsed_ms = started.elapsed().as_millis();
    let CompletedIndexSink {
        directories, files, ..
    } = sink
        .finish()
        .expect("the in-memory analysis sink should finish");

    assert_eq!(directories.len() as u64, summary.directory_count);
    assert!(
        files.len() as u64 <= summary.candidate_count,
        "live allocation validation may discard logical-size candidates"
    );
    assert_eq!(aggregate.bytes, summary.root_allocated_bytes);
    println!(
        "real_analysis_memory strategy={} directories={} candidates={} bytes={} elapsed_ms={}",
        summary.strategy,
        directories.len(),
        files.len(),
        aggregate.bytes,
        elapsed_ms
    );

    // Keep both maps live through the final print so an external process monitor observes the
    // actual steady-state footprint instead of a value after Rust has already released the tree.
    std::hint::black_box((&directories, &files));
}

/// Complete large-file scans own their candidate snapshot and leave disk-analysis navigation
/// intact. This keeps scan ownership explicit without retaining millions of unrelated directory
/// aggregates in the large-file session.
#[test]
#[ignore = "requires a real MANGODISK_ANALYSIS_ROOT volume with change history"]
fn complete_large_file_scan_preserves_real_analysis_snapshot() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::var(ANALYSIS_ROOT_ENV)
        .expect("MANGODISK_ANALYSIS_ROOT must be set before shared-snapshot validation");
    let canonical_root = current_platform()
        .canonicalize_no_links(Path::new(&root))
        .expect("the validation root should resolve");
    let (_, analysis_diagnostics) =
        StorageTraversal::analyze_path_with_diagnostics(Some(root.clone()), true, |_| {})
            .expect("the real analysis should succeed");
    let (large_files, large_diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
        vec![root],
        LARGE_FILE_CANDIDATE_FLOOR_BYTES,
        LargeFileScanMode::Complete,
        vec![],
        |_| {},
    )
    .expect("the large-file query should succeed");

    assert_eq!(
        large_diagnostics.fast_path, "used",
        "a complete large-file request must perform its own candidate scan"
    );
    assert!(large_files
        .entries
        .iter()
        .all(|entry| entry.bytes >= LARGE_FILE_CANDIDATE_FLOOR_BYTES));
    assert!(
        cache::analysis_result(&canonical_root)
            .expect("the analysis cache should remain readable")
            .is_some(),
        "a large-file scan must not evict the independent disk-analysis snapshot"
    );
    println!(
            "independent_large_file_snapshot analysis_fast_path={} strategy={} large_files={} bytes={} result_build_ms={}",
            analysis_diagnostics.fast_path,
            analysis_diagnostics.strategy,
            large_files.total_count,
            large_files.total_bytes,
            large_diagnostics.result_build_ms
        );
}

#[cfg(target_os = "macos")]
#[test]
fn explicit_nested_filesystem_root_is_not_covered_by_its_parent() {
    let parent = Path::new("/");
    let mounted = Path::new("/dev");
    assert!(!current_platform().is_same_filesystem(
        &fs::metadata(parent).unwrap(),
        &fs::metadata(mounted).unwrap()
    ));
    let roots = normalize_large_file_roots(vec![
        parent.display().to_string(),
        mounted.display().to_string(),
    ])
    .unwrap();
    assert_eq!(
        roots.len(),
        2,
        "a mounted filesystem must remain an explicit scan root"
    );
}
