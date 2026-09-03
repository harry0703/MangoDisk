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
        analysis::AnalysisService, duplicates::DuplicateFileService, large_files::LargeFileService,
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
    let data = vec![0x5a_u8; LARGE_FILE_INDEX_FLOOR_BYTES as usize + 4096];
    fs::write(entry.join("first.bin"), &data).unwrap();
    fs::write(entry.join("second.bin"), &data).unwrap();
    fs::create_dir(entry.join("nested")).unwrap();
    fs::write(entry.join("nested/unique.txt"), b"different content").unwrap();
    let canonical = current_platform()
        .resolve_directory_entry(&entry)
        .expect("resolve the redirected shared directory");
    let root = current_platform().display_path(&canonical);
    let started = Instant::now();
    let analysis =
        AnalysisService::analyze_with_progress(Some(root.clone()), true, |_| {}).unwrap();
    assert_eq!(
        analysis
            .entries
            .iter()
            .map(|entry| entry.file_count)
            .sum::<u64>(),
        3
    );
    let large = LargeFileService::find_with_progress(Some(root.clone()), 1, true, |_| {}).unwrap();
    assert_eq!(large.entries.len(), 2);
    let duplicates = DuplicateFileService::find_with_progress(vec![root], 1, |_| {}).unwrap();
    assert_eq!(duplicates.groups.len(), 1);
    assert_eq!(duplicates.groups[0].entries.len(), 2);
    assert_eq!(duplicates.groups[0].bytes_per_file, data.len() as u64);
    println!(
        "shared_scan_verified files=3 large_files=2 duplicate_groups=1 elapsed_ms={}",
        started.elapsed().as_millis()
    );
}

impl Drop for DirectoryCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
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
    let mut validation = LargeFileStreamValidation::new(
        &root,
        allocated.saturating_add(1),
        now_ms(),
        &progress,
        &cancelled,
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
    let mut validation = LargeFileStreamValidation::new(&root, 0, now_ms(), &progress, &cancelled)
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
fn fast_analysis_contract_validates_record_counts_before_publish() {
    let root = Path::new("/fixture");
    let progress = Arc::new(ProgressTracker::new(1, |_| {}, 0));
    let cancelled = AtomicBool::new(false);
    let mut validation = FastAnalysisStreamValidation::new(root, 100, &progress, &cancelled);
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
fn large_file_cache_supports_switching_from_high_threshold_to_index_floor() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
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
    cache::store_memory_only(
        &root,
        aggregate,
        std::collections::HashMap::from([(root.clone(), aggregate)]),
        files,
        cache::SnapshotPublication::new(
            ScanPurpose::LargeFiles,
            true,
            None,
            1,
            cache::mutation_revision().expect("the cache revision should load"),
        ),
    )
    .expect("the large-file index should be stored in the cache");

    let high_threshold = cache::large_files_result(&root, 500 * 1024 * 1024, false)
        .expect("the high-threshold result should be read from the cache");
    assert_eq!(high_threshold.entries.len(), 1);

    let low_threshold = cache::large_files_result(&root, LARGE_FILE_INDEX_FLOOR_BYTES, true)
        .expect("the cache should remain reusable after lowering the threshold");
    assert_eq!(low_threshold.entries.len(), 3);
    assert!(low_threshold.cache_reused);

    cache::remove_entry(
        &root,
        mangodisk_platform::FileSpaceUsage {
            logical_bytes: aggregate.logical_bytes,
            allocated_bytes: aggregate.bytes,
        },
        aggregate.file_count,
        true,
    );
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
    let canonical_root = current_platform()
        .canonicalize_no_links(Path::new(&root))
        .expect("the real scan root should be safely accessible");

    let (result, diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
        Some(root),
        LARGE_FILE_INDEX_FLOOR_BYTES,
        true,
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
        .all(|entry| entry.bytes >= LARGE_FILE_INDEX_FLOOR_BYTES));
    if diagnostics.fast_path == "used" {
        assert!(
            diagnostics.candidate_count >= result.total_count,
            "platform candidates must cover every valid result"
        );
        if diagnostics.candidate_count > 0 {
            assert!(
                diagnostics.candidate_peak_in_flight > 0,
                "a non-empty candidate stream must record its in-flight peak"
            );
        }
    }
    cache::remove_entry(
        &canonical_root,
        mangodisk_platform::FileSpaceUsage::logical_only(0),
        0,
        true,
    );
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
    let (aggregate, summary) =
        stream_fast_analysis_once(&canonical_root, now_ms(), &progress, &cancelled, &mut sink)
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

/// Analysis stores complete directory aggregates and files at or above 50 MB, so a later
/// large-file query should derive its result from that immutable in-process snapshot. Explicit
/// refresh and destructive-operation preflight remain the boundaries for observing newer state.
#[test]
#[ignore = "requires a real MANGODISK_ANALYSIS_ROOT volume with change history"]
fn large_file_query_reuses_real_analysis_snapshot() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::var(ANALYSIS_ROOT_ENV)
        .expect("MANGODISK_ANALYSIS_ROOT must be set before shared-snapshot validation");
    let (_, analysis_diagnostics) =
        StorageTraversal::analyze_path_with_diagnostics(Some(root.clone()), true, |_| {})
            .expect("the real analysis should succeed");
    let (large_files, large_diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
        Some(root),
        LARGE_FILE_INDEX_FLOOR_BYTES,
        false,
        |_| {},
    )
    .expect("the large-file query should succeed");

    assert_eq!(
        large_diagnostics.fast_path, "cache",
        "a large-file query on an unchanged volume must reuse the analysis snapshot"
    );
    assert!(large_files
        .entries
        .iter()
        .all(|entry| entry.bytes >= LARGE_FILE_INDEX_FLOOR_BYTES));
    println!(
            "shared_memory_snapshot analysis_fast_path={} strategy={} large_files={} bytes={} cache_validation_ms={} result_build_ms={}",
            analysis_diagnostics.fast_path,
            analysis_diagnostics.strategy,
            large_files.total_count,
            large_files.total_bytes,
            large_diagnostics.cache_validation_ms,
            large_diagnostics.result_build_ms
        );
}
