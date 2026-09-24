use super::super::candidates::{
    load_file_identity, modified_ms, normalize_roots, remove_physical_aliases, validate_open_file,
    FileCandidate, FileIdentity, FileIdentitySource,
};
use super::super::hash_cache;
use super::*;
use crate::shared::operation::OPERATION_CANCELLED_ERROR;
use crate::storage::duplicates::session::DUPLICATE_RESULT_PAGE_SIZE;
use mangodisk_platform::PlatformCancellation;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Instant, UNIX_EPOCH},
};

const SAMPLE_BENCHMARK_RUNS: usize = 5;
const SAMPLE_BENCHMARK_PLANS: [SamplePlan; 4] = [
    SamplePlan::Head4KiB,
    SamplePlan::HeadTail8KiB,
    SamplePlan::HeadMiddleTail16KiB,
    SamplePlan::HeadMiddleTail256KiB,
];

fn never_cancelled() -> PlatformCancellation {
    PlatformCancellation::new(|| false)
}

#[test]
fn delete_validation_failures_have_stable_diagnostic_reasons() {
    assert_eq!(
        duplicate_delete_validation_reason("the duplicate-file result session expired; scan again"),
        "session_expired"
    );
    assert_eq!(
        duplicate_delete_validation_reason("a duplicate item is outside the current scan roots"),
        "outside_scan_roots"
    );
    assert_eq!(
        duplicate_delete_validation_reason("a protected duplicate item cannot be deleted"),
        "protected_root"
    );
    assert_eq!(
        duplicate_delete_validation_reason("an unexpected validation failure"),
        "unknown"
    );
}

#[test]
fn protected_scan_location_is_visible_but_cannot_be_deleted() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-protected-duplicate-root-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let work = root.join("work");
    let chat = root.join("chat");
    fs::create_dir_all(&work).expect("create protected work fixture");
    fs::create_dir_all(&chat).expect("create cleanable chat fixture");
    fs::write(work.join("report.bin"), b"protected duplicate content")
        .expect("write protected fixture");
    fs::write(chat.join("report-copy.bin"), b"protected duplicate content")
        .expect("write cleanable fixture");

    let result = DuplicateFileService::find_paged_with_locations(
        vec![
            DuplicateScanLocation {
                path: display_path(&work),
                mode: DuplicateScanLocationMode::Protected,
            },
            DuplicateScanLocation {
                path: display_path(&chat),
                mode: DuplicateScanLocationMode::Cleanable,
            },
        ],
        1,
        |_| {},
        |_| {},
    )
    .expect("scan protected and cleanable duplicate roots");

    assert_eq!(result.protected_roots.len(), 1);
    assert_eq!(result.groups.len(), 1);
    let group = &result.groups[0];
    let protected = group
        .entries
        .iter()
        .find(|entry| entry.delete_policy == DuplicateEntryDeletePolicy::Protected)
        .expect("the work copy should be protected")
        .clone();
    let cleanable = group
        .entries
        .iter()
        .find(|entry| entry.delete_policy == DuplicateEntryDeletePolicy::Cleanable)
        .expect("the chat copy should remain cleanable")
        .clone();
    assert_eq!(group.reclaimable_bytes, cleanable.allocated_bytes);

    let protected_error = DuplicateFileService::delete_files_permanently(
        result.scan_id,
        vec![PermanentDeleteCandidate {
            path: protected.path.clone(),
            expected_bytes: protected.bytes,
            expected_modified_at_ms: protected.modified_at_ms,
        }],
    )
    .expect_err("Core must reject a protected duplicate even if the WebView submits it");
    assert!(protected_error.to_string().contains("protected duplicate"));
    assert!(Path::new(&protected.path).exists());

    let deletion = DuplicateFileService::delete_files_permanently(
        result.scan_id,
        vec![PermanentDeleteCandidate {
            path: cleanable.path.clone(),
            expected_bytes: cleanable.bytes,
            expected_modified_at_ms: cleanable.modified_at_ms,
        }],
    )
    .expect("the cleanable duplicate should remain deletable");
    assert_eq!(deletion.removed_paths, vec![cleanable.path.clone()]);
    assert!(!Path::new(&cleanable.path).exists());
    assert!(Path::new(&protected.path).exists());

    clear_result_session().expect("clear protected duplicate session");
    fs::remove_dir_all(root).expect("remove protected duplicate fixture");
}

#[test]
fn shared_exclusions_prune_duplicate_candidates_before_hashing() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    hash_cache::clear().expect("clear the duplicate hash cache before exclusion validation");
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-exclusions-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let included = root.join("included");
    let excluded = root.join("excluded");
    fs::create_dir_all(&included).expect("create the included duplicate fixture");
    fs::create_dir_all(&excluded).expect("create the excluded duplicate fixture");
    let included_files = [included.join("first.bin"), included.join("second.bin")];
    fs::write(&included_files[0], b"included duplicate content")
        .expect("write the first included duplicate");
    fs::write(&included_files[1], b"included duplicate content")
        .expect("write the second included duplicate");
    fs::write(excluded.join("first.bin"), b"excluded duplicate content")
        .expect("write the first excluded duplicate");
    fs::write(excluded.join("second.bin"), b"excluded duplicate content")
        .expect("write the second excluded duplicate");
    let canonical_included_files = included_files.map(|path| {
        current_platform()
            .canonicalize_no_links(&path)
            .expect("canonicalize an included duplicate fixture")
    });
    let result = DuplicateFileService::find_paged_with_locations_and_exclusions(
        vec![DuplicateScanLocation {
            path: display_path(&root),
            mode: DuplicateScanLocationMode::Cleanable,
        }],
        vec![display_path(&excluded)],
        1,
        |_| {},
        |_| {},
    )
    .expect("scan the fixture with a shared exclusion");

    assert_eq!(result.scanned_file_count, 2);
    assert_eq!(result.groups.len(), 1);
    assert_eq!(result.groups[0].entries.len(), 2);
    assert!(
        result.groups[0]
            .entries
            .iter()
            .all(|entry| canonical_included_files.iter().any(|expected| {
                current_platform().paths_equal(Path::new(&entry.path), expected)
            })),
        "excluded duplicate entry survived: {:?}",
        result.groups[0]
            .entries
            .iter()
            .map(|entry| &entry.path)
            .collect::<Vec<_>>()
    );

    clear_result_session().expect("clear the exclusion result session");
    fs::remove_dir_all(root).expect("remove the duplicate exclusion fixture");
}

#[test]
fn protected_descendant_protects_its_aggregated_directory_entry() {
    let root = std::env::temp_dir().join("mangodisk-protected-directory-policy");
    let protected_root = root.join("work").join("important");
    let work_directory = root.join("work");
    let chat_directory = root.join("chat");
    let mut groups = vec![DuplicateGroup {
        id: "directory-group".to_owned(),
        hash: "directory-proof".to_owned(),
        kind: DuplicateGroupKind::Directory,
        bytes_per_file: 64,
        file_count_per_entry: 2,
        reclaimable_bytes: 0,
        entries: vec![
            DuplicateFileEntry {
                name: "work".to_owned(),
                path: display_path(&work_directory),
                parent_path: display_path(&root),
                bytes: 64,
                allocated_bytes: 96,
                modified_at_ms: None,
                delete_policy: DuplicateEntryDeletePolicy::Cleanable,
            },
            DuplicateFileEntry {
                name: "chat".to_owned(),
                path: display_path(&chat_directory),
                parent_path: display_path(&root),
                bytes: 64,
                allocated_bytes: 80,
                modified_at_ms: None,
                delete_policy: DuplicateEntryDeletePolicy::Cleanable,
            },
        ],
    }];

    let counts = apply_protection_policy(&mut groups, &[protected_root]);

    assert_eq!(counts, (1, 1));
    assert_eq!(
        groups[0].entries[0].delete_policy,
        DuplicateEntryDeletePolicy::Protected
    );
    assert_eq!(
        groups[0].entries[1].delete_policy,
        DuplicateEntryDeletePolicy::Cleanable
    );
    assert_eq!(groups[0].reclaimable_bytes, 80);
}

fn result_signature(result: &DuplicateFilesResult) -> Vec<(String, u64, u64, Vec<String>)> {
    result
        .groups
        .iter()
        .map(|group| {
            (
                group.hash.clone(),
                group.bytes_per_file,
                group.reclaimable_bytes,
                group
                    .entries
                    .iter()
                    .map(|entry| entry.path.clone())
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn multiple_roots_match_renamed_copies_without_counting_overlaps_twice() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-multiple-roots-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let work = root.join("work");
    let chat = root.join("chat");
    let nested = work.join("nested");
    fs::create_dir_all(&nested).expect("create work fixture");
    fs::create_dir_all(&chat).expect("create chat fixture");
    fs::write(work.join("proposal.docx"), vec![1_u8; 4096]).expect("write original");
    fs::write(chat.join("renamed.docx"), vec![1_u8; 4096]).expect("write renamed copy");
    fs::write(nested.join("version.docx"), vec![2_u8; 4096]).expect("write work version");
    fs::write(chat.join("version.docx"), vec![3_u8; 4096]).expect("write different chat version");

    let result = DuplicateFileService::find_with_progress(
        vec![
            display_path(&chat),
            display_path(&work),
            display_path(&nested),
        ],
        1,
        |_| {},
    )
    .expect("scan multiple selected roots");
    assert_eq!(result.roots.len(), 2);
    assert_eq!(result.scanned_file_count, 4);
    assert_eq!(result.groups.len(), 1);
    assert_eq!(result.groups[0].entries.len(), 2);
    assert!(result.groups[0]
        .entries
        .iter()
        .all(|entry| entry.name != "version.docx"));
    fs::remove_dir_all(root).expect("remove multi-root fixture");
}

#[test]
fn scan_root_order_is_independent_of_user_insertion_order() {
    let sandbox = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-root-order-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let alpha = sandbox.join("alpha");
    let beta = sandbox.join("beta");
    let nested = beta.join("nested");
    fs::create_dir_all(&alpha).expect("the first scan root should be created");
    fs::create_dir_all(&nested).expect("the nested scan root should be created");

    let forward = normalize_roots(vec![
        display_path(&nested),
        display_path(&beta),
        display_path(&alpha),
    ])
    .expect("forward-ordered scan roots should normalize");
    let reversed = normalize_roots(vec![
        display_path(&alpha),
        display_path(&beta),
        display_path(&nested),
    ])
    .expect("reverse-ordered scan roots should normalize");

    assert_eq!(
        forward, reversed,
        "reordering the same root set must preserve cache identity"
    );
    assert_eq!(
        forward.len(),
        2,
        "the parent root must consistently subsume the duplicate nested root"
    );
    fs::remove_dir_all(sandbox).expect("the scan-root ordering fixture should be removed");
}

fn benchmark_sample_plans(case_name: &str, root: &Path) {
    let mut expected_signature = None;
    for plan in SAMPLE_BENCHMARK_PLANS {
        let mut elapsed_samples = Vec::with_capacity(SAMPLE_BENCHMARK_RUNS);
        for run in 1..=SAMPLE_BENCHMARK_RUNS {
            let started = Instant::now();
            let (result, diagnostics) = DuplicateFileService::find_with_options_diagnostics(
                vec![display_path(root)],
                1,
                plan,
                Some(1),
                |_| {},
            )
            .expect("the sample-plan scan should succeed");
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let signature = result_signature(&result);
            if let Some(expected) = &expected_signature {
                assert_eq!(
                    &signature, expected,
                    "sample plans must not change the final full-hash result"
                );
            } else {
                expected_signature = Some(signature);
            }
            elapsed_samples.push(elapsed_ms);
            println!(
                    "duplicate_sample_plan case={} plan={} run={} total_ms={} size_candidates={} aliases_filtered={} sample_candidates={} sample_bytes={} full_candidates={} full_bytes={} groups={} reclaimable_bytes={}",
                    case_name,
                    diagnostics.sample_plan,
                    run,
                    elapsed_ms,
                    diagnostics.size_group_candidate_count,
                    diagnostics.physical_alias_filtered_count,
                    diagnostics.sample_hash_candidate_count,
                    diagnostics.sample_hash_bytes,
                    diagnostics.full_hash_candidate_count,
                    diagnostics.full_hash_bytes,
                    result.total_group_count,
                    result.reclaimable_bytes
                );
        }
        elapsed_samples.sort_unstable();
        println!(
                "duplicate_sample_plan_summary case={} plan={} runs={} median_ms={} min_ms={} max_ms={}",
                case_name,
                plan.name(),
                SAMPLE_BENCHMARK_RUNS,
                elapsed_samples[SAMPLE_BENCHMARK_RUNS / 2],
                elapsed_samples[0],
                elapsed_samples[SAMPLE_BENCHMARK_RUNS - 1]
            );
    }
}

fn benchmark_worker_counts(case_name: &str, root: &Path) {
    let mut expected_signature = None;
    for worker_count in [1, 2, 4] {
        let mut elapsed_samples = Vec::with_capacity(SAMPLE_BENCHMARK_RUNS);
        let mut elapsed_microsecond_samples = Vec::with_capacity(SAMPLE_BENCHMARK_RUNS);
        for run in 1..=SAMPLE_BENCHMARK_RUNS {
            let started = Instant::now();
            let (result, diagnostics) = DuplicateFileService::find_with_options_diagnostics(
                vec![display_path(root)],
                1,
                PRODUCTION_SAMPLE_PLAN,
                Some(worker_count),
                |_| {},
            )
            .expect("the worker-count scan should succeed");
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
            let signature = result_signature(&result);
            if let Some(expected) = &expected_signature {
                assert_eq!(
                    &signature, expected,
                    "worker counts must not change the full-hash result"
                );
            } else {
                expected_signature = Some(signature);
            }
            elapsed_samples.push(elapsed_ms);
            elapsed_microsecond_samples.push(elapsed_us);
            println!(
                    "duplicate_worker_count case={} workers={} run={} total_ms={} total_us={} group_identity_ms={} identity_workers={} identity_peak={} sample_workers={} sample_peak={} full_workers={} full_peak={} queue_capacity={} full_bytes={} fully_sparse_candidates={} fully_sparse_groups={} fully_sparse_logical_bytes_skipped={} allocated_range_query_fallbacks={} groups={} reclaimable_bytes={}",
                    case_name,
                    worker_count,
                    run,
                    elapsed_ms,
                    elapsed_us,
                    diagnostics.group_and_identity_ms,
                    diagnostics.identity_worker_count,
                    diagnostics.identity_peak_in_flight,
                    diagnostics.sample_hash_worker_count,
                    diagnostics.sample_hash_peak_in_flight,
                    diagnostics.full_hash_worker_count,
                    diagnostics.full_hash_peak_in_flight,
                    diagnostics.hash_result_queue_capacity,
                    diagnostics.full_hash_bytes,
                    diagnostics.fully_sparse_candidate_count,
                    diagnostics.fully_sparse_group_count,
                    diagnostics.fully_sparse_logical_bytes_skipped,
                    diagnostics.allocated_range_query_fallback_count,
                    result.total_group_count,
                    result.reclaimable_bytes
                );
        }
        elapsed_samples.sort_unstable();
        elapsed_microsecond_samples.sort_unstable();
        println!(
                "duplicate_worker_count_summary case={} workers={} runs={} median_ms={} min_ms={} max_ms={} median_us={} min_us={} max_us={}",
                case_name,
                worker_count,
                SAMPLE_BENCHMARK_RUNS,
                elapsed_samples[SAMPLE_BENCHMARK_RUNS / 2],
                elapsed_samples[0],
                elapsed_samples[SAMPLE_BENCHMARK_RUNS - 1],
                elapsed_microsecond_samples[SAMPLE_BENCHMARK_RUNS / 2],
                elapsed_microsecond_samples[0],
                elapsed_microsecond_samples[SAMPLE_BENCHMARK_RUNS - 1]
            );
    }
}

fn write_sparse_marker_file(path: &Path, bytes: u64, offset: u64, marker: [u8; 8]) {
    use std::io::Write;

    let mut file = File::create(path).expect("the sampling fixture should be created");
    file.set_len(bytes)
        .expect("the sampling fixture length should be set");
    file.seek(SeekFrom::Start(offset))
        .expect("the sampling fixture should seek to the marker");
    file.write_all(&marker)
        .expect("the sampling marker should be written");
}

fn create_sparse_file(path: &Path, logical_bytes: u64) {
    let file = File::create(path).expect("the sparse duplicate fixture should be created");
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::HANDLE,
            System::{Ioctl::FSCTL_SET_SPARSE, IO::DeviceIoControl},
        };

        let mut returned = 0_u32;
        let marked_sparse = unsafe {
            DeviceIoControl(
                file.as_raw_handle() as HANDLE,
                FSCTL_SET_SPARSE,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(marked_sparse, 0, "mark the duplicate fixture as sparse");
    }
    file.set_len(logical_bytes)
        .expect("the sparse duplicate fixture length should be set");
}

fn candidate_for_path(path: &Path) -> FileCandidate {
    let metadata = fs::symlink_metadata(path).expect("candidate metadata should be readable");
    let identity = load_file_identity(path, metadata.len());
    FileCandidate {
        root_ordinal: 0,
        path: path.to_path_buf(),
        bytes: metadata.len(),
        modified_at: metadata.modified().ok(),
        modified_at_ms: modified_ms(&metadata),
        identity,
        identity_source: identity.map(|_| FileIdentitySource::FileHandle),
    }
}

fn assert_sparse_query_falls_back(
    case_name: &str,
    mut query_allocated_content: impl FnMut(&File, u64) -> Result<Option<bool>, ()>,
) {
    const LOGICAL_BYTES: u64 = 1024 * 1024;

    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-sparse-query-{case_name}-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the sparse query fixture should be created");
    let first = root.join("first.bin");
    let second = root.join("second.bin");
    create_sparse_file(&first, LOGICAL_BYTES);
    create_sparse_file(&second, LOGICAL_BYTES);
    let candidates = vec![candidate_for_path(&first), candidate_for_path(&second)];
    let operation = OperationGuard::start(CoordinatedOperationKind::DuplicateFiles)
        .expect("the duplicate-file test operation should start");
    let progress = DuplicateProgress::new(operation.id(), |_| {});
    let mut ignore_group = |_, _| {};

    let pipeline = execute_hash_pipeline_with_allocated_content_query(
        &candidates,
        HashPipelinePlan {
            sample_plan: PRODUCTION_SAMPLE_PLAN,
            worker_count: 2,
        },
        None,
        &operation,
        &progress,
        &mut ignore_group,
        &mut query_allocated_content,
    )
    .expect("an unavailable sparse query must fall back to content hashing");
    operation.complete();

    assert_eq!(pipeline.diagnostics.allocated_range_query_fallback_count, 1);
    assert_eq!(pipeline.diagnostics.fully_sparse_group_count, 0);
    assert_eq!(pipeline.diagnostics.fully_sparse_candidate_count, 0);
    assert_eq!(pipeline.diagnostics.full_hash_candidate_count, 2);
    assert_eq!(pipeline.diagnostics.full_hash_bytes, 2 * LOGICAL_BYTES);
    assert_eq!(pipeline.full_groups.len(), 1);
    assert_eq!(pipeline.skipped_count, 0);
    fs::remove_dir_all(root).expect("the sparse query fixture should be removed");
}

#[test]
fn staged_hashing_only_reports_identical_content() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-test-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the duplicate-file fixture should be created");
    let matching = vec![b'a'; 1024 * 1024];
    let different = vec![b'b'; matching.len()];
    fs::write(root.join("first.bin"), &matching)
        .expect("the first matching file should be written");
    fs::write(root.join("second.bin"), &matching)
        .expect("the second matching file should be written");
    fs::write(root.join("same-size.bin"), &different)
        .expect("the same-size unique file should be written");
    fs::create_dir_all(root.join(".dependency-cache"))
        .expect("the hidden dependency directory should be created");
    fs::write(root.join(".dependency-cache/dependency.bin"), &matching)
        .expect("the hidden dependency fixture should be written");
    fs::create_dir_all(root.join(".python-cache"))
        .expect("the hidden Python cache directory should be created");
    fs::write(root.join(".python-cache/module.cpython-314.pyc"), &matching)
        .expect("the hidden Python cache fixture should be written");

    let progress_events = Arc::new(Mutex::new(Vec::<TraversalProgress>::new()));
    let captured_events = Arc::clone(&progress_events);
    let result =
        DuplicateFileService::find_with_progress(vec![display_path(&root)], 1, move |event| {
            captured_events
                .lock()
                .expect("capture staged hashing progress")
                .push(event);
        })
        .expect("the duplicate-file scan should succeed");

    assert_eq!(result.groups.len(), 1);
    assert_eq!(result.groups[0].entries.len(), 2);
    assert_eq!(result.reclaimable_bytes, matching.len() as u64);
    assert!(result.groups[0]
        .entries
        .iter()
        .all(|entry| entry.name != "same-size.bin"));
    assert!(result.groups[0]
        .entries
        .iter()
        .all(|entry| entry.name != "dependency.bin"));
    assert!(result.groups[0]
        .entries
        .iter()
        .all(|entry| entry.name != "module.cpython-314.pyc"));
    let progress_events = progress_events
        .lock()
        .expect("read staged hashing progress");
    let completed = progress_events
        .last()
        .expect("the completed scan must publish final progress");
    assert!(matches!(
        completed.current_stage,
        TraversalStage::HashingFiles
    ));
    // Three candidates each contribute three 16 KiB sample reads, then the two matching files
    // contribute one complete 1 MiB read. Progress reports all real verification I/O rather than
    // discarding the sample stage when full hashing starts.
    assert_eq!(completed.bytes_scanned, 2 * 1024 * 1024 + 3 * 3 * 16 * 1024);
    assert_eq!(completed.completed_steps, 5);
    assert_eq!(completed.total_steps, 5);
    fs::remove_dir_all(root).expect("the duplicate-file fixture should be removed");
}

#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "allocated content query not yet implemented on Linux"
)]
fn sparse_duplicates_report_physical_reclaimable_space() {
    const LOGICAL_BYTES: u64 = 8 * 1024 * 1024;

    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-sparse-duplicate-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the sparse duplicate root should be created");
    create_sparse_file(&root.join("first.bin"), LOGICAL_BYTES);
    create_sparse_file(&root.join("second.bin"), LOGICAL_BYTES);

    let (result, diagnostics) =
        DuplicateFileService::find_with_diagnostics(vec![display_path(&root)], 1, |_| {})
            .expect("the sparse duplicate scan should succeed");
    let group = result
        .groups
        .first()
        .expect("the sparse files should form one duplicate group");

    assert_eq!(group.bytes_per_file, LOGICAL_BYTES);
    assert!(
        group
            .entries
            .iter()
            .all(|entry| entry.allocated_bytes < entry.bytes),
        "sparse holes must not inflate physical disk usage"
    );
    assert_eq!(result.total_duplicate_bytes, group.total_allocated_bytes());
    assert_eq!(result.reclaimable_bytes, group.maximum_reclaimable_bytes());
    assert_eq!(diagnostics.full_hash_bytes, 0);
    assert_eq!(diagnostics.fully_sparse_candidate_count, 2);
    assert_eq!(diagnostics.fully_sparse_group_count, 1);
    assert_eq!(
        diagnostics.fully_sparse_logical_bytes_skipped,
        2 * LOGICAL_BYTES
    );
    fs::remove_dir_all(root).expect("the sparse duplicate fixture should be removed");
}

#[test]
fn unsupported_allocated_content_query_falls_back_to_full_hashing() {
    assert_sparse_query_falls_back("unsupported", |_, _| Ok(None));
}

#[test]
fn failed_allocated_content_query_falls_back_to_full_hashing() {
    assert_sparse_query_falls_back("error", |_, _| Err(()));
}

#[test]
fn sparse_candidate_mutation_after_query_is_not_certified() {
    const LOGICAL_BYTES: u64 = 1024 * 1024;

    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-sparse-query-race-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the sparse race fixture should be created");
    let first = root.join("first.bin");
    let second = root.join("second.bin");
    create_sparse_file(&first, LOGICAL_BYTES);
    create_sparse_file(&second, LOGICAL_BYTES);
    let candidates = vec![candidate_for_path(&first), candidate_for_path(&second)];
    let operation = OperationGuard::start(CoordinatedOperationKind::DuplicateFiles)
        .expect("the duplicate-file test operation should start");
    let progress = DuplicateProgress::new(operation.id(), |_| {});
    let mut ignore_group = |_, _| {};
    let mut query_count = 0_u64;
    let mut mutate_after_query = |_: &File, _: u64| {
        query_count = query_count.saturating_add(1);
        if query_count == 1 {
            // Simulate a file changing after the native query but before the second identity and
            // metadata validation. A shorter dense payload makes the race deterministic on both
            // macOS and Windows without relying on timestamp resolution.
            fs::write(&first, vec![1_u8; (LOGICAL_BYTES / 2) as usize])
                .expect("the sparse candidate should be mutated");
        }
        Ok(Some(false))
    };

    let pipeline = execute_hash_pipeline_with_allocated_content_query(
        &candidates,
        HashPipelinePlan {
            sample_plan: PRODUCTION_SAMPLE_PLAN,
            worker_count: 2,
        },
        None,
        &operation,
        &progress,
        &mut ignore_group,
        &mut mutate_after_query,
    )
    .expect("a changed sparse candidate should fail closed without aborting the scan");
    operation.complete();

    assert_eq!(query_count, 1);
    assert_eq!(pipeline.diagnostics.allocated_range_query_fallback_count, 0);
    assert_eq!(pipeline.diagnostics.fully_sparse_group_count, 0);
    assert_eq!(pipeline.diagnostics.fully_sparse_candidate_count, 0);
    assert_eq!(pipeline.diagnostics.full_hash_candidate_count, 2);
    assert_eq!(pipeline.full_failures.count, 1);
    assert!(
        pipeline
            .full_groups
            .values()
            .all(|indices| indices.len() == 1),
        "a candidate changed after the sparse query must not form a duplicate group"
    );
    assert_eq!(pipeline.skipped_count, 1);
    fs::remove_dir_all(root).expect("the sparse race fixture should be removed");
}

#[test]
fn mixed_sparse_and_dense_zero_files_use_real_content_hashes() {
    const LOGICAL_BYTES: u64 = 8 * 1024 * 1024;

    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-mixed-sparse-duplicate-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("create the mixed sparse fixture");
    create_sparse_file(&root.join("sparse-first.bin"), LOGICAL_BYTES);
    create_sparse_file(&root.join("sparse-second.bin"), LOGICAL_BYTES);
    fs::write(
        root.join("dense-zero.bin"),
        vec![0_u8; usize::try_from(LOGICAL_BYTES).expect("fixture length fits usize")],
    )
    .expect("write the dense zero fixture");

    let (result, diagnostics) =
        DuplicateFileService::find_with_diagnostics(vec![display_path(&root)], 1, |_| {})
            .expect("the mixed sparse scan should succeed");

    assert_eq!(result.total_group_count, 1);
    assert_eq!(result.duplicate_file_count, 3);
    assert_eq!(diagnostics.fully_sparse_group_count, 0);
    assert_eq!(diagnostics.fully_sparse_candidate_count, 0);
    assert_eq!(diagnostics.full_hash_bytes, 3 * LOGICAL_BYTES);
    fs::remove_dir_all(root).expect("remove the mixed sparse fixture");
}

#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "allocated content query not yet implemented on Linux"
)]
fn fully_sparse_groups_are_not_promoted_to_unverifiable_directories() {
    const LOGICAL_BYTES: u64 = 8 * 1024 * 1024;

    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-sparse-directory-boundary-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let first = root.join("first-copy");
    let second = root.join("second-copy");
    fs::create_dir_all(&first).expect("create first sparse directory");
    fs::create_dir_all(&second).expect("create second sparse directory");
    create_sparse_file(&first.join("payload.bin"), LOGICAL_BYTES);
    create_sparse_file(&second.join("payload.bin"), LOGICAL_BYTES);

    let (result, diagnostics) =
        DuplicateFileService::find_with_diagnostics(vec![display_path(&root)], 1, |_| {})
            .expect("scan sparse directory copies");

    assert_eq!(result.total_group_count, 1);
    assert_eq!(result.groups[0].kind, DuplicateGroupKind::File);
    assert_eq!(result.duplicate_file_count, 2);
    assert_eq!(diagnostics.fully_sparse_group_count, 1);
    assert_eq!(diagnostics.aggregated_directory_group_count, 0);
    fs::remove_dir_all(root).expect("remove sparse directory fixture");
}

#[test]
fn exact_duplicate_directories_replace_nested_file_groups() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-directory-service-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let first = root.join("first-copy");
    let second = root.join("second-copy");
    fs::create_dir_all(first.join("nested"))
        .expect("the first duplicate directory should be created");
    fs::create_dir_all(second.join("nested"))
        .expect("the second duplicate directory should be created");
    for directory in [&first, &second] {
        fs::write(directory.join("manifest.json"), br#"{"name":"fixture"}"#)
            .expect("the duplicate manifest should be written");
        fs::write(directory.join("nested/payload.bin"), vec![7_u8; 4096])
            .expect("the duplicate payload should be written");
    }

    let result = DuplicateFileService::find_paged_with_progress(
        vec![display_path(&root)],
        1,
        |_| {},
        |_| {},
    )
    .expect("the directory aggregation scan should succeed");

    assert_eq!(result.total_group_count, 1);
    assert_eq!(result.groups[0].kind, DuplicateGroupKind::Directory);
    assert_eq!(result.groups[0].entries.len(), 2);
    assert_eq!(result.groups[0].file_count_per_entry, 2);
    assert_eq!(result.groups[0].bytes_per_file, 4114);
    assert_eq!(
        result.groups[0].reclaimable_bytes,
        result.groups[0].entries[0].allocated_bytes
    );
    let selected = &result.groups[0].entries[0];
    let selected_path = PathBuf::from(&selected.path);
    let retained_path = PathBuf::from(&result.groups[0].entries[1].path);
    assert!(
        selected_path.starts_with(&result.roots[0]),
        "selected path {:?} should belong to scan roots {:?}",
        selected.path,
        result.roots
    );
    let deletion = DuplicateFileService::delete_files_permanently(
        result.scan_id,
        vec![PermanentDeleteCandidate {
            path: selected.path.clone(),
            expected_bytes: selected.bytes,
            expected_modified_at_ms: selected.modified_at_ms,
        }],
    )
    .expect("an aggregated directory from the current scan root should remain deletable");
    assert_eq!(deletion.removed_paths.len(), 1);
    assert!(!selected_path.exists());
    assert!(retained_path.exists());
    clear_result_session().expect("the duplicate directory result session should be cleared");
    fs::remove_dir_all(root).expect("the directory aggregation fixture should be removed");
}

#[test]
fn streamed_full_hash_groups_match_the_final_result() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-stream-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the streaming duplicate fixture should be created");
    for (name, content) in [
        ("alpha-a.bin", vec![1_u8; 1024]),
        ("alpha-b.bin", vec![1_u8; 1024]),
        ("beta-a.bin", vec![2_u8; 2048]),
        ("beta-b.bin", vec![2_u8; 2048]),
        ("same-size-unique.bin", vec![3_u8; 2048]),
    ] {
        fs::write(root.join(name), content)
            .expect("the streaming duplicate fixture should be written");
    }
    let batches = Arc::new(Mutex::new(Vec::<DuplicateGroupBatch>::new()));
    let batches_for_callback = Arc::clone(&batches);
    let (result, diagnostics) = DuplicateFileService::find_with_stream_diagnostics(
        vec![display_path(&root)],
        1,
        |_| {},
        move |batch| {
            batches_for_callback
                .lock()
                .expect("the streaming batch lock should not be poisoned")
                .push(batch);
        },
    )
    .expect("the streaming duplicate scan should succeed");
    let batches = batches
        .lock()
        .expect("streaming batches should remain readable");
    let mut streamed = batches
        .iter()
        .flat_map(|batch| batch.groups.clone())
        .collect::<Vec<_>>();
    streamed.sort_by(|left, right| {
        right
            .reclaimable_bytes
            .cmp(&left.reclaimable_bytes)
            .then_with(|| left.hash.cmp(&right.hash))
    });

    assert_eq!(result.groups.len(), 2);
    let final_hashes = result
        .groups
        .iter()
        .map(|group| group.hash.as_str())
        .collect::<HashSet<_>>();
    assert!(
            !streamed.is_empty()
                && streamed
                    .iter()
                    .all(|group| final_hashes.contains(group.hash.as_str())),
            "throttling may defer groups to the final response, but events must contain only final groups"
        );
    assert_eq!(
        diagnostics.streamed_group_count,
        u64::try_from(streamed.len()).unwrap_or(u64::MAX)
    );
    assert!(streamed.iter().all(|group| group.entries.len() == 2));
    assert!(streamed.iter().all(|group| {
        group
            .entries
            .iter()
            .all(|entry| entry.name != "same-size-unique.bin")
    }));
    fs::remove_dir_all(root).expect("the streaming duplicate fixture should be removed");
}

#[test]
fn paginated_sessions_are_scan_scoped_and_recomputed_after_removal() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-page-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the duplicate pagination fixture should be created");
    for index in 0..45_u8 {
        let content = vec![index.saturating_add(1); 1024 + usize::from(index)];
        fs::write(root.join(format!("group-{index:02}-a.bin")), &content)
            .expect("the original pagination fixture should be written");
        fs::write(root.join(format!("group-{index:02}-b.bin")), &content)
            .expect("the duplicate pagination fixture should be written");
    }

    let result = DuplicateFileService::find_paged_with_progress(
        vec![display_path(&root)],
        1,
        |_| {},
        |_| {},
    )
    .expect("the paginated duplicate scan should succeed");
    assert_eq!(result.total_group_count, 45);
    assert_eq!(result.returned_group_count, 45);
    assert_eq!(result.groups.len(), DUPLICATE_RESULT_PAGE_SIZE);
    assert!(
        DuplicateFileService::page(result.scan_id, 0, 0).is_err(),
        "a zero page size must be rejected"
    );
    assert!(
        DuplicateFileService::page(result.scan_id.saturating_add(1), 0, 1).is_err(),
        "a session identifier from another scan must be rejected"
    );
    assert!(
        DuplicateFileService::page(result.scan_id, result.returned_group_count + 1, 1).is_err(),
        "an offset beyond the result total must be rejected"
    );
    let capped_page = DuplicateFileService::page(result.scan_id, 0, u64::MAX)
        .expect("an oversized page limit should be bounded safely");
    assert_eq!(capped_page.groups.len(), 45);

    let last_page = DuplicateFileService::page(
        result.scan_id,
        DUPLICATE_RESULT_PAGE_SIZE as u64,
        DUPLICATE_RESULT_PAGE_SIZE as u64,
    )
    .expect("the final page should be readable");
    assert_eq!(last_page.groups.len(), 5);
    assert_eq!(last_page.next_offset, None);

    let removed = result.groups[0].entries[0].clone();
    let deletion = DuplicateFileService::delete_files_permanently(
        result.scan_id,
        vec![crate::filesystem::PermanentDeleteCandidate {
            path: removed.path,
            expected_bytes: removed.bytes,
            expected_modified_at_ms: removed.modified_at_ms,
        }],
    )
    .expect("deletion should update the paginated session in the same Core operation");
    assert_eq!(deletion.removed_paths.len(), 1);
    let updated = DuplicateFileService::page(result.scan_id, 0, u64::MAX)
        .expect("the synchronized first page should remain readable");
    assert_eq!(updated.total_count, 44);
    assert_eq!(updated.groups.len(), 44);
    let updated_last_page = DuplicateFileService::page(
        result.scan_id,
        DUPLICATE_RESULT_PAGE_SIZE as u64,
        DUPLICATE_RESULT_PAGE_SIZE as u64,
    )
    .expect("the final page should be readable after the update");
    assert_eq!(updated_last_page.groups.len(), 4);

    clear_result_session().expect("the pagination test session should be cleared");
    assert!(
        DuplicateFileService::page(result.scan_id, 0, 1).is_err(),
        "an invalidated session must not remain readable across scans"
    );
    fs::remove_dir_all(root).expect("the duplicate pagination fixture should be removed");
}

#[test]
fn multiple_workers_emit_progress_once_per_throttle_window() {
    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_count_for_progress = Arc::clone(&callback_count);
    let progress = DuplicateProgress::new(1, move |_| {
        callback_count_for_progress.fetch_add(1, Ordering::Relaxed);
    });
    const WORKERS: usize = 8;
    let barrier = Arc::new(std::sync::Barrier::new(WORKERS + 1));
    thread::scope(|scope| {
        for _ in 0..WORKERS {
            let barrier = Arc::clone(&barrier);
            let progress = &progress;
            scope.spawn(move || {
                barrier.wait();
                progress.emit(
                    TraversalStage::Analyzing,
                    Path::new("/benchmark/progress"),
                    false,
                    0,
                    0,
                );
            });
        }
        barrier.wait();
    });
    assert_eq!(
        callback_count.load(Ordering::Relaxed),
        1,
        "exactly one worker may emit progress in a throttle window"
    );
}

#[test]
fn hash_progress_replaces_logical_bytes_then_accumulates_actual_reads() {
    let events = Arc::new(Mutex::new(Vec::<TraversalProgress>::new()));
    let captured = Arc::clone(&events);
    let progress = DuplicateProgress::new(1, move |event| {
        captured
            .lock()
            .expect("capture duplicate progress")
            .push(event);
    });
    let path = Path::new("/benchmark/progress.bin");

    progress.visit(TraversalStage::Analyzing, path, 8 * 1024 * 1024);
    progress.extend_hash_stage(path, 2);
    progress.complete_hash_step(path, 4 * 1024);
    progress.complete_hash_step(path, 8 * 1024);
    progress.extend_hash_stage(path, 1);
    progress.complete_hash_step(path, 16 * 1024);

    let events = events.lock().expect("read duplicate progress");
    let hash_start = events
        .iter()
        .find(|event| matches!(event.current_stage, TraversalStage::HashingFiles))
        .expect("hashing must publish a stage boundary");
    assert_eq!(hash_start.items_scanned, 1);
    assert_eq!(hash_start.bytes_scanned, 0);
    assert_eq!(hash_start.completed_steps, 0);
    assert_eq!(hash_start.total_steps, 2);

    let completed = events.last().expect("hashing must publish completion");
    assert!(matches!(
        completed.current_stage,
        TraversalStage::HashingFiles
    ));
    assert_eq!(completed.bytes_scanned, 28 * 1024);
    assert_eq!(completed.completed_steps, 3);
    assert_eq!(completed.total_steps, 3);
}

#[test]
fn io_before_a_read_failure_remains_in_diagnostics() {
    struct PartialReader {
        first_read: bool,
    }

    impl Read for PartialReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.first_read {
                return Err(std::io::Error::other("expected read failure"));
            }
            self.first_read = true;
            buffer[..4].copy_from_slice(&[1, 2, 3, 4]);
            Ok(4)
        }
    }

    let mut reader = PartialReader { first_read: false };
    let mut buffer = [0_u8; 8];
    let mut bytes_read = 0_u64;
    let error = read_up_to(&mut reader, &mut buffer, &mut bytes_read)
        .expect_err("the second read should return the diagnostic error");
    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert_eq!(bytes_read, 4);
}

#[test]
fn every_cached_file_fact_must_match_exactly() {
    let identity = FileIdentity {
        volume: u64::MAX,
        index: 42,
    };
    let candidate = FileCandidate {
        root_ordinal: 1,
        path: PathBuf::from("/cache/root/file.bin"),
        bytes: 1024,
        modified_at: UNIX_EPOCH.checked_add(std::time::Duration::new(123, 100_000)),
        modified_at_ms: Some(123_000),
        identity: Some(identity),
        identity_source: Some(FileIdentitySource::FileHandle),
    };
    let cached = DuplicateHashCacheFile {
        root_ordinal: candidate.root_ordinal,
        path: candidate.path.clone(),
        bytes: candidate.bytes,
        modified_at: candidate.modified_at,
        identity: encode_file_identity(identity),
        sample_hash: [3; 32],
        full_hash: Some([5; 32]),
    };
    assert!(duplicate_cache_file_matches(&candidate, &cached));

    let mut changed = cached.clone();
    changed.root_ordinal = 0;
    assert!(!duplicate_cache_file_matches(&candidate, &changed));
    changed = cached.clone();
    changed.path.push("replacement");
    assert!(!duplicate_cache_file_matches(&candidate, &changed));
    changed = cached.clone();
    changed.bytes += 1;
    assert!(!duplicate_cache_file_matches(&candidate, &changed));
    changed = cached.clone();
    changed.modified_at = UNIX_EPOCH.checked_add(std::time::Duration::new(123, 200_000));
    assert!(!duplicate_cache_file_matches(&candidate, &changed));
    changed = cached;
    changed.identity[15] ^= 1;
    assert!(!duplicate_cache_file_matches(&candidate, &changed));
}

#[test]
fn verified_hash_cache_eliminates_reads_in_both_stages_without_changing_results() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-pipeline-cache-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the hash-cache fixture should be created");
    let content = vec![9_u8; 1024 * 1024];
    for name in ["first.bin", "second.bin", "third.bin"] {
        fs::write(root.join(name), &content).expect("the hash-cache fixture should be written");
    }
    let candidates = ["first.bin", "second.bin", "third.bin"]
        .into_iter()
        .map(|name| {
            let path = root.join(name);
            let metadata =
                fs::symlink_metadata(&path).expect("cached candidate metadata should be read");
            let identity = load_file_identity(&path, metadata.len());
            FileCandidate {
                root_ordinal: 0,
                path: path.clone(),
                bytes: metadata.len(),
                modified_at: metadata.modified().ok(),
                modified_at_ms: modified_ms(&metadata),
                identity,
                identity_source: identity.map(|_| FileIdentitySource::FileHandle),
            }
        })
        .collect::<Vec<_>>();
    let operation = OperationGuard::start(CoordinatedOperationKind::DuplicateFiles)
        .expect("the cache test operation should start");
    let progress = DuplicateProgress::new(operation.id(), |_| {});
    let mut ignore_group = |_, _| {};
    let fresh = execute_hash_pipeline(
        &candidates,
        HashPipelinePlan {
            sample_plan: PRODUCTION_SAMPLE_PLAN,
            worker_count: 2,
        },
        None,
        &operation,
        &progress,
        &mut ignore_group,
    )
    .expect("the fresh hash pipeline should succeed");
    assert!(fresh.diagnostics.sample_hash_bytes > 0);
    assert!(fresh.diagnostics.full_hash_bytes > 0);

    let cache = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let identity = candidate
                .identity
                .expect("a cached candidate must have physical identity");
            (
                candidate.path.clone(),
                DuplicateHashCacheFile {
                    root_ordinal: candidate.root_ordinal,
                    path: candidate.path.clone(),
                    bytes: candidate.bytes,
                    modified_at: candidate.modified_at,
                    identity: encode_file_identity(identity),
                    sample_hash: *fresh.sample_hashes[index]
                        .expect("fresh sampling should succeed")
                        .as_bytes(),
                    full_hash: fresh.full_hashes[index].map(|hash| *hash.as_bytes()),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let cached = execute_hash_pipeline(
        &candidates,
        HashPipelinePlan {
            sample_plan: PRODUCTION_SAMPLE_PLAN,
            worker_count: 2,
        },
        Some(&cache),
        &operation,
        &progress,
        &mut ignore_group,
    )
    .expect("the cached hash pipeline should succeed");
    assert_eq!(cached.full_groups, fresh.full_groups);
    assert_eq!(cached.diagnostics.sample_hash_cache_hit_count, 3);
    assert_eq!(cached.diagnostics.full_hash_cache_hit_count, 3);
    assert_eq!(cached.diagnostics.sample_hash_bytes, 0);
    assert_eq!(cached.diagnostics.full_hash_bytes, 0);
    assert_eq!(cached.skipped_count, 0);

    operation.complete();
    fs::remove_dir_all(root).expect("the hash-cache fixture should be removed");
}

/// This fixture must use real FSEvents or USN Journal history instead of a fake monitor.
/// Continuous change history must invalidate a cached digest even when a file restores its
/// original size and modification time. The test is ignored by default and should run in an
/// isolated process during cross-platform validation to avoid shared cache and operation state.
#[test]
#[ignore = "requires real macOS FSEvents or Windows NTFS USN Journal history"]
fn real_file_changes_make_the_memory_duplicate_hash_cache_fail_closed() {
    use std::time::Duration;

    const FILE_BYTES: usize = 1024 * 1024;
    const EVENT_TIMEOUT: Duration = Duration::from_secs(5);
    const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(25);

    let _operation_lock = crate::shared::operation::test_operation_lock();
    hash_cache::clear().expect("the memory hash cache should clear before the fixture");
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-history-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the real change-history fixture should be created");
    let original_content = vec![7_u8; FILE_BYTES];
    for name in ["first.bin", "second.bin", "third.bin"] {
        fs::write(root.join(name), &original_content)
            .expect("the duplicate-file fixture should be written");
    }
    let roots = vec![display_path(&root)];
    let scan = || {
        DuplicateFileService::find_with_diagnostics(roots.clone(), 1, |_| {})
            .expect("the real change-history scan should succeed")
    };
    let assert_fresh_read = |diagnostics: &DuplicateScanDiagnostics, stage: &str| {
        assert_eq!(
            diagnostics.sample_hash_cache_hit_count, 0,
            "{stage} must not reuse a stale sample digest"
        );
        assert_eq!(
            diagnostics.full_hash_cache_hit_count, 0,
            "{stage} must not reuse a stale full digest"
        );
        assert!(
            diagnostics.sample_hash_bytes > 0,
            "{stage} must reread real sample content"
        );
    };
    let mutate_and_wait = |mutate: &mut dyn FnMut()| {
        let previous = current_platform()
            .capture_filesystem_change_token(&root)
            .expect("the token before mutation should be captured")
            .expect("the test volume must support continuous change history");
        mutate();
        let deadline = Instant::now() + EVENT_TIMEOUT;
        loop {
            let current = current_platform()
                .capture_filesystem_change_token(&root)
                .expect("the token after mutation should be captured")
                .expect("the test volume must continue to support change history");
            if current != previous {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the platform did not publish a filesystem change cursor before the deadline"
            );
            thread::sleep(EVENT_POLL_INTERVAL);
        }
    };

    let (fresh_result, fresh_diagnostics) = scan();
    assert_eq!(fresh_result.duplicate_file_count, 3);
    assert_eq!(fresh_diagnostics.cache_snapshot_found, 0);
    assert!(fresh_diagnostics.sample_hash_bytes > 0);
    assert!(fresh_diagnostics.full_hash_bytes > 0);

    let (cached_result, cached_diagnostics) = scan();
    assert_eq!(
        result_signature(&cached_result),
        result_signature(&fresh_result)
    );
    assert_eq!(cached_diagnostics.cache_snapshot_found, 1);
    assert_eq!(cached_diagnostics.sample_hash_cache_hit_count, 3);
    assert_eq!(cached_diagnostics.full_hash_cache_hit_count, 3);
    assert_eq!(cached_diagnostics.sample_hash_bytes, 0);
    assert_eq!(cached_diagnostics.full_hash_bytes, 0);

    // Restoring the original size and modification time makes the per-file facts look equal
    // again. Only real change history can prevent a stale full digest from being reauthorized.
    let changed_path = root.join("third.bin");
    let original_modified = fs::metadata(&changed_path)
        .and_then(|metadata| metadata.modified())
        .expect("the original modification time should be available");
    mutate_and_wait(&mut || {
        fs::write(&changed_path, vec![8_u8; FILE_BYTES])
            .expect("the file should be rewritten with equal-size content");
        File::options()
            .write(true)
            .open(&changed_path)
            .and_then(|file| {
                file.set_times(std::fs::FileTimes::new().set_modified(original_modified))
            })
            .expect("the original modification time should be restored");
    });
    assert_eq!(
        fs::metadata(&changed_path)
            .and_then(|metadata| metadata.modified())
            .expect("the restored modification time should be verified"),
        original_modified
    );
    let (modified_result, modified_diagnostics) = scan();
    assert_eq!(modified_result.duplicate_file_count, 2);
    assert_eq!(modified_result.reclaimable_bytes, FILE_BYTES as u64);
    assert_fresh_read(
        &modified_diagnostics,
        "an equal-size rewrite with restored modification time",
    );

    let created_path = root.join("created.bin");
    mutate_and_wait(&mut || {
        fs::write(&created_path, vec![9_u8; FILE_BYTES])
            .expect("an equal-size unique file should be created");
    });
    let (_, created_diagnostics) = scan();
    assert_fresh_read(&created_diagnostics, "file creation");

    let renamed_path = root.join("renamed-first.bin");
    mutate_and_wait(&mut || {
        fs::rename(root.join("first.bin"), &renamed_path)
            .expect("the duplicate file should be renamed");
    });
    let (_, renamed_diagnostics) = scan();
    assert_fresh_read(&renamed_diagnostics, "file rename");

    mutate_and_wait(&mut || {
        fs::remove_file(&created_path).expect("the newly created file should be removed");
    });
    let (deleted_result, deleted_diagnostics) = scan();
    assert_eq!(deleted_result.duplicate_file_count, 2);
    assert_fresh_read(&deleted_diagnostics, "file deletion");

    let (recached_result, recached_diagnostics) = scan();
    assert_eq!(
        result_signature(&recached_result),
        result_signature(&deleted_result)
    );
    assert_eq!(recached_diagnostics.sample_hash_bytes, 0);
    assert_eq!(recached_diagnostics.full_hash_bytes, 0);
    assert!(recached_diagnostics.sample_hash_cache_hit_count > 0);
    assert!(recached_diagnostics.full_hash_cache_hit_count > 0);

    hash_cache::invalidate_containing(&root);
    fs::remove_dir_all(root).expect("the real change-history fixture should be removed");
}

#[test]
fn unreadable_file_identity_fails_closed() {
    let candidates = ["first.bin", "second.bin"]
        .into_iter()
        .map(|name| FileCandidate {
            root_ordinal: 0,
            path: PathBuf::from("/missing").join(name),
            bytes: 1024,
            modified_at: None,
            modified_at_ms: None,
            identity: None,
            identity_source: None,
        })
        .collect();
    let filtered = remove_physical_aliases(candidates, 4, &never_cancelled(), |_| Ok(()))
        .expect("filter aliases");
    assert!(filtered.candidates.is_empty());
    assert_eq!(filtered.alias_count, 0);
    assert_eq!(filtered.unavailable_count, 2);
    #[cfg(windows)]
    {
        assert_eq!(filtered.hint_fallback_directory_count, 1);
        assert_eq!(filtered.hint_failure_samples.len(), 1);
        assert!(!filtered.hint_failure_samples[0]
            .diagnostic_detail
            .is_empty());
        assert!(filtered.hint_failure_samples[0]
            .diagnostic_detail
            .contains("directory"));
    }
}

#[test]
fn physical_identity_validation_propagates_cancellation() {
    let candidates = vec![FileCandidate {
        root_ordinal: 0,
        path: PathBuf::from("/missing/candidate.bin"),
        bytes: 1024,
        modified_at: None,
        modified_at_ms: None,
        identity: None,
        identity_source: None,
    }];
    let result = remove_physical_aliases(candidates, 4, &never_cancelled(), |_| {
        Err("operation cancelled".to_string())
    });
    assert!(matches!(result, Err(error) if error == "operation cancelled"));
}

#[test]
fn physical_identity_validation_observes_preloaded_and_fallback_candidates() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let observed = AtomicUsize::new(0);
    let candidates = vec![
        FileCandidate {
            root_ordinal: 0,
            path: PathBuf::from("preloaded.bin"),
            bytes: 8,
            modified_at: None,
            modified_at_ms: None,
            identity: Some(FileIdentity {
                volume: 1,
                index: 1,
            }),
            identity_source: Some(FileIdentitySource::FileHandle),
        },
        FileCandidate {
            root_ordinal: 0,
            path: PathBuf::from("fallback.bin"),
            bytes: 8,
            modified_at: None,
            modified_at_ms: None,
            identity: None,
            identity_source: None,
        },
    ];

    remove_physical_aliases(candidates, 4, &never_cancelled(), |_| {
        observed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    })
    .expect("filter aliases");

    assert_eq!(observed.load(Ordering::Relaxed), 2);
}

#[test]
fn colliding_directory_hints_are_verified_before_alias_filtering() {
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-hint-collision-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("create the identity hint collision fixture");
    let candidates = ["first.bin", "second.bin"]
        .into_iter()
        .map(|name| {
            let path = root.join(name);
            fs::write(&path, b"independent files")
                .expect("write an identity hint collision candidate");
            let metadata = fs::symlink_metadata(&path)
                .expect("read identity hint collision candidate metadata");
            FileCandidate {
                root_ordinal: 0,
                path,
                bytes: metadata.len(),
                modified_at: metadata.modified().ok(),
                modified_at_ms: modified_ms(&metadata),
                identity: Some(FileIdentity {
                    volume: 7,
                    index: 11,
                }),
                identity_source: Some(FileIdentitySource::DirectoryHint),
            }
        })
        .collect::<Vec<_>>();

    let filtered = remove_physical_aliases(candidates, 4, &never_cancelled(), |_| Ok(()))
        .expect("verify colliding identity hints");

    assert_eq!(filtered.candidates.len(), 2);
    assert_eq!(filtered.alias_count, 0);
    assert_eq!(filtered.verified_hint_count, 2);
    assert!(filtered
        .candidates
        .iter()
        .all(|candidate| candidate.identity_source == Some(FileIdentitySource::FileHandle)));
    fs::remove_dir_all(root).expect("remove the identity hint collision fixture");
}

#[test]
fn directory_identity_hint_loading_honors_platform_cancellation() {
    let candidates = vec![FileCandidate {
        root_ordinal: 0,
        path: PathBuf::from("cancelled-parent/candidate.bin"),
        bytes: 8,
        modified_at: None,
        modified_at_ms: None,
        identity: None,
        identity_source: None,
    }];
    let cancellation = PlatformCancellation::new(|| true);

    let result = remove_physical_aliases(candidates, 4, &cancellation, |_| Ok(()));

    assert!(matches!(result, Err(error) if error == OPERATION_CANCELLED_ERROR));
}

#[test]
fn hash_failure_logging_keeps_bounded_readable_samples() {
    let mut failures = HashFailureDiagnostics::default();
    for index in 0..5 {
        failures.record(
            Path::new(&format!("/private/user/failure-{index}.bin")),
            &format!("failure-{index}"),
        );
    }
    assert_eq!(failures.count, 5);
    assert_eq!(failures.samples.len(), HASH_FAILURE_SAMPLE_LIMIT);
    assert!(failures
        .samples
        .iter()
        .all(|sample| sample.contains("/private/user") && sample.contains("error=")));
}

#[test]
fn internal_validation_preserves_sub_millisecond_modification_precision() {
    use std::time::Duration;

    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-mtime-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the modification-time fixture should be created");
    let path = root.join("candidate.bin");
    fs::write(&path, b"mtime precision")
        .expect("the modification-time fixture file should be written");
    let metadata =
        fs::symlink_metadata(&path).expect("the fixture file metadata should be available");
    let identity = load_file_identity(&path, metadata.len());
    let candidate = FileCandidate {
        root_ordinal: 0,
        path: path.clone(),
        bytes: metadata.len(),
        modified_at: metadata
            .modified()
            .ok()
            // Windows `SystemTime` uses 100 ns ticks. Both platforms represent 100 µs, while
            // it remains below the public DTO's 1 ms precision boundary.
            .map(|value| value + Duration::from_micros(100)),
        modified_at_ms: modified_ms(&metadata),
        identity,
        identity_source: identity.map(|_| FileIdentitySource::FileHandle),
    };
    let file = File::open(&path).expect("the modification-time fixture should be opened");
    validate_open_file(&candidate, &file, true)
        .expect_err("a sub-millisecond modification-time mismatch must fail closed");
    fs::remove_dir_all(root).expect("the modification-time fixture should be removed");
}

#[test]
fn sample_ranges_are_deduplicated_and_bounded_for_small_files() {
    assert_eq!(
        SamplePlan::Head4KiB.offsets(1024, 1024),
        [Some(0), None, None]
    );
    assert_eq!(
        SamplePlan::HeadTail8KiB.offsets(1024, 1024),
        [Some(0), Some(0), None]
    );
    assert_eq!(
        SamplePlan::HeadMiddleTail16KiB.offsets(32 * 1024, 16 * 1024),
        [Some(0), Some(8 * 1024), Some(16 * 1024)]
    );
    assert_eq!(
        SamplePlan::HeadMiddleTail256KiB.offsets(2 * 1024 * 1024, 256 * 1024),
        [Some(0), Some(896 * 1024), Some(1792 * 1024)]
    );
}

#[test]
fn full_hashing_rejects_equal_size_files_replaced_after_enumeration() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-replacement-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the file-replacement fixture should be created");
    let path = root.join("candidate.bin");
    fs::write(&path, vec![1_u8; 1024 * 1024])
        .expect("the original candidate file should be written");
    let metadata =
        fs::symlink_metadata(&path).expect("the original candidate metadata should be read");
    let identity = load_file_identity(&path, metadata.len());
    let candidate = FileCandidate {
        root_ordinal: 0,
        path: path.clone(),
        bytes: metadata.len(),
        modified_at: metadata.modified().ok(),
        modified_at_ms: modified_ms(&metadata),
        identity,
        identity_source: identity.map(|_| FileIdentitySource::FileHandle),
    };

    // Keep the original object allocated so a fast replacement cannot reuse its identity.
    fs::rename(&path, root.join("retained-original.bin"))
        .expect("the original candidate file should be retained");
    fs::write(&path, vec![2_u8; 1024 * 1024])
        .expect("the equal-size replacement file should be written");
    let operation = OperationGuard::start(CoordinatedOperationKind::DuplicateFiles)
        .expect("the duplicate-file test operation should start");
    let error = full_hash(&candidate, &operation, &mut Vec::new(), &mut 0)
        .expect_err("a path replaced after enumeration must fail closed");
    assert!(error.contains("changed") || error.contains("different object"));
    operation.complete();
    fs::remove_dir_all(root).expect("the file-replacement fixture should be removed");
}

#[test]
fn single_and_multiple_workers_return_the_same_stable_result() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-workers-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the worker-consistency fixture should be created");
    for index in 0..4 {
        fs::write(
            root.join(format!("duplicate-{index}.bin")),
            vec![7_u8; 2 * 1024 * 1024],
        )
        .expect("the real duplicate file should be written");
        fs::write(
            root.join(format!("unique-{index}.bin")),
            vec![
                u8::try_from(index).expect("the fixture index should fit in u8") + 20;
                2 * 1024 * 1024
            ],
        )
        .expect("the equal-size unique file should be written");
    }

    let (serial, serial_diagnostics) = DuplicateFileService::find_with_options_diagnostics(
        vec![display_path(&root)],
        1,
        PRODUCTION_SAMPLE_PLAN,
        Some(1),
        |_| {},
    )
    .expect("the single-worker scan should succeed");
    let (parallel, parallel_diagnostics) = DuplicateFileService::find_with_options_diagnostics(
        vec![display_path(&root)],
        1,
        PRODUCTION_SAMPLE_PLAN,
        Some(4),
        |_| {},
    )
    .expect("the multi-worker scan should succeed");

    assert_eq!(result_signature(&serial), result_signature(&parallel));
    assert_eq!(serial.reclaimable_bytes, parallel.reclaimable_bytes);
    assert_eq!(serial_diagnostics.sample_hash_worker_count, 1);
    let expected_parallel_workers = thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
        .min(4);
    assert_eq!(
        parallel_diagnostics.sample_hash_worker_count,
        expected_parallel_workers as u64
    );
    assert_eq!(
        parallel_diagnostics.full_hash_worker_count,
        expected_parallel_workers as u64
    );
    assert!(parallel_diagnostics.sample_hash_peak_in_flight <= expected_parallel_workers as u64);
    assert!(parallel_diagnostics.full_hash_peak_in_flight <= expected_parallel_workers as u64);
    fs::remove_dir_all(root).expect("the worker-consistency fixture should be removed");
}

#[test]
fn duplicate_hash_scheduling_uses_one_worker_for_non_solid_state_media() {
    let root = PathBuf::from("/benchmark/root");
    let volume = |scan_concurrency| VolumeInfo {
        name: "benchmark".to_string(),
        mount_point: "/benchmark".to_string(),
        total_bytes: 1,
        available_bytes: 1,
        used_bytes: 0,
        scan_concurrency,
    };
    let solid_state = duplicate_hash_worker_config_from_volumes(
        std::slice::from_ref(&root),
        &[volume(mangodisk_platform::ScanConcurrency::solid_state())],
        8,
    );
    assert_eq!(solid_state.worker_count, 4);
    assert_eq!(solid_state.identity_worker_count, 4);
    assert_eq!(solid_state.device_classes, "solid_state");

    for scheduling in [
        mangodisk_platform::ScanConcurrency::rotational(),
        mangodisk_platform::ScanConcurrency::conservative(ScanDeviceClass::Removable),
        mangodisk_platform::ScanConcurrency::conservative(ScanDeviceClass::Network),
        mangodisk_platform::ScanConcurrency::conservative(ScanDeviceClass::Unknown),
    ] {
        let conservative = duplicate_hash_worker_config_from_volumes(
            std::slice::from_ref(&root),
            &[volume(scheduling)],
            8,
        );
        assert_eq!(conservative.worker_count, 1);
        let expected_identity_workers = if scheduling.class == ScanDeviceClass::Rotational {
            2
        } else {
            1
        };
        assert_eq!(
            conservative.identity_worker_count,
            expected_identity_workers
        );
    }
}

#[cfg(windows)]
#[test]
fn duplicate_hash_scheduling_matches_verbatim_windows_roots() {
    let root = PathBuf::from(r"\\?\C:\benchmark\fixture");
    let volume = VolumeInfo {
        name: "benchmark".to_string(),
        mount_point: r"C:\".to_string(),
        total_bytes: 1,
        available_bytes: 1,
        used_bytes: 0,
        scan_concurrency: mangodisk_platform::ScanConcurrency::solid_state(),
    };
    let scheduling = duplicate_hash_worker_config_from_volumes(&[root], &[volume], 8);

    assert_eq!(scheduling.worker_count, 4);
    assert_eq!(scheduling.identity_worker_count, 4);
    assert_eq!(scheduling.device_classes, "solid_state");
}

#[test]
fn hard_links_do_not_inflate_duplicate_counts_or_reclaimable_space() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-hardlink-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the hard-link fixture should be created");
    let original = root.join("original.bin");
    let alias = root.join("alias.bin");
    let copy = root.join("copy.bin");
    let content = vec![9_u8; 1024 * 1024];
    fs::write(&original, &content).expect("the hard-link source file should be written");
    fs::hard_link(&original, &alias).expect("the test filesystem should support hard links");
    fs::write(&copy, &content).expect("the independent content copy should be written");

    let (result, diagnostics) =
        DuplicateFileService::find_with_diagnostics(vec![display_path(&root)], 1, |_| {})
            .expect("the hard-link fixture scan should succeed");

    assert_eq!(result.total_group_count, 1);
    assert_eq!(result.duplicate_file_count, 2);
    assert_eq!(result.reclaimable_bytes, content.len() as u64);
    assert_eq!(diagnostics.physical_alias_filtered_count, 1);
    #[cfg(windows)]
    assert_eq!(diagnostics.identity_hint_verified_count, 2);
    #[cfg(unix)]
    assert_eq!(diagnostics.identity_hint_verified_count, 0);
    fs::remove_dir_all(root).expect("the hard-link fixture should be removed");
}

/// Full hashing checks cancellation after each 1 MiB read. This capacity diagnostic remains
/// ignored to avoid creating a 2 GiB sparse file during regular tests. Cross-platform
/// validation should run five samples and record the slowest observed latency.
#[test]
#[ignore = "requires an explicit large-file cancellation latency diagnostic"]
fn full_hash_cancellation_latency_is_below_250_ms() {
    use std::{sync::mpsc::channel, time::Duration};

    const FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
    const RUNS: usize = 5;
    const CANCEL_DELAY_MS: u64 = 10;
    const MAX_CANCEL_LATENCY_MS: u64 = 250;

    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-cancel-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the cancellation-latency fixture should be created");
    let path = root.join("large-sparse.bin");
    let file = File::create(&path).expect("the cancellation-latency sparse file should be created");
    file.set_len(FILE_BYTES)
        .expect("the cancellation-latency sparse file length should be set");
    drop(file);
    let metadata =
        fs::symlink_metadata(&path).expect("the cancellation-latency file metadata should be read");
    let identity = load_file_identity(&path, metadata.len());
    let candidate = FileCandidate {
        root_ordinal: 0,
        path: path.clone(),
        bytes: metadata.len(),
        modified_at: metadata.modified().ok(),
        modified_at_ms: modified_ms(&metadata),
        identity,
        identity_source: identity.map(|_| FileIdentitySource::FileHandle),
    };
    let mut latency_samples = Vec::with_capacity(RUNS);

    for run in 1..=RUNS {
        let (ready_sender, ready_receiver) = channel();
        let hash_candidate = candidate.clone();
        let worker = thread::spawn(move || {
            let operation = OperationGuard::start(CoordinatedOperationKind::DuplicateFiles)
                .expect("the cancellation-latency test operation should start");
            ready_sender
                .send(())
                .expect("the hash task start should be reported");
            full_hash(&hash_candidate, &operation, &mut Vec::new(), &mut 0)
        });
        ready_receiver.recv().expect("the hash task should start");
        thread::sleep(Duration::from_millis(CANCEL_DELAY_MS));
        let cancelled_at = Instant::now();
        DuplicateFileService::cancel();
        let error = worker
            .join()
            .expect("the cancellation-latency worker should not panic")
            .expect_err("large-file hashing should respond to cancellation");
        let latency_ms = u64::try_from(cancelled_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        assert_eq!(error, OPERATION_CANCELLED_ERROR);
        latency_samples.push(latency_ms);
        println!("duplicate_cancel_latency run={run} latency_ms={latency_ms}");
    }
    latency_samples.sort_unstable();
    let observed_p95_ms = latency_samples[RUNS - 1];
    println!("duplicate_cancel_latency_summary runs={RUNS} observed_p95_ms={observed_p95_ms}");
    assert!(
        observed_p95_ms < MAX_CANCEL_LATENCY_MS,
        "the slowest observed cancellation latency of {observed_p95_ms} ms exceeds the limit"
    );
    fs::remove_dir_all(root).expect("the cancellation-latency fixture should be removed");
}

/// This diagnostic compares four sample plans through the same production scan pipeline. It is
/// ignored by default to avoid reading a large fixture during regular tests.
/// `MANGODISK_DUPLICATE_BENCHMARK_ROOT` must point to the fixed dataset's core directory. Output
/// contains only plan, count, byte, and duration metrics and never persists absolute paths.
#[test]
#[ignore = "requires an explicit MANGODISK_DUPLICATE_BENCHMARK_ROOT"]
fn real_duplicate_sample_plans_match_and_report_read_volume() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::var("MANGODISK_DUPLICATE_BENCHMARK_ROOT")
        .expect("MANGODISK_DUPLICATE_BENCHMARK_ROOT must be set before the sample benchmark");
    benchmark_sample_plans("fixed-v1", Path::new(&root));
}

/// This diagnostic compares the complete duplicate pipeline with one, two, and four workers on a
/// caller-owned fixture. The worker override also disables the memory hash cache, so every run
/// performs the same identity and content reads. Output contains only counts and durations.
#[test]
#[ignore = "requires an explicit MANGODISK_DUPLICATE_BENCHMARK_ROOT"]
fn real_duplicate_worker_counts_preserve_results() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::var("MANGODISK_DUPLICATE_BENCHMARK_ROOT")
        .expect("MANGODISK_DUPLICATE_BENCHMARK_ROOT must be set before the worker benchmark");
    benchmark_worker_counts("external", Path::new(&root));
}

/// The dedicated collision fixture places differences at the head, middle, tail, and outside
/// every sampled range. Sparse files only reduce fixture construction cost; hashing still reads
/// all logical bytes so the diagnostic compares smaller samples with more full hashes against
/// larger samples with more random reads.
#[test]
#[ignore = "requires an explicit duplicate-mix performance diagnostic"]
fn dedicated_collision_dataset_compares_sample_rejection_power() {
    const COLLISION_FILE_BYTES: u64 = 2 * 1024 * 1024;
    const DUPLICATE_FILE_BYTES: u64 = 8 * 1024 * 1024;
    const FILES_PER_CASE: u8 = 24;
    const UNSAMPLED_FILES: u8 = 8;

    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::temp_dir().join(format!(
        "mangodisk-duplicate-mix-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir_all(&root).expect("the duplicate-mix fixture should be created");
    let middle_offset = SamplePlan::HeadMiddleTail16KiB.offsets(COLLISION_FILE_BYTES, 16 * 1024)[1]
        .expect("the three-range sample plan must include a middle offset");

    for index in 0..FILES_PER_CASE {
        write_sparse_marker_file(
            &root.join(format!("head-{index:02}.bin")),
            COLLISION_FILE_BYTES,
            0,
            [1, index, 0, 0, 0, 0, 0, 0],
        );
        write_sparse_marker_file(
            &root.join(format!("middle-{index:02}.bin")),
            COLLISION_FILE_BYTES,
            middle_offset,
            [2, index, 0, 0, 0, 0, 0, 0],
        );
        write_sparse_marker_file(
            &root.join(format!("tail-{index:02}.bin")),
            COLLISION_FILE_BYTES,
            COLLISION_FILE_BYTES - 8,
            [3, index, 0, 0, 0, 0, 0, 0],
        );
    }
    for index in 0..UNSAMPLED_FILES {
        write_sparse_marker_file(
            &root.join(format!("unsampled-{index:02}.bin")),
            COLLISION_FILE_BYTES,
            384 * 1024,
            [4, index, 0, 0, 0, 0, 0, 0],
        );
    }
    for index in 0..4 {
        let file = File::create(root.join(format!("duplicate-{index}.bin")))
            .expect("the real duplicate file should be created");
        file.set_len(DUPLICATE_FILE_BYTES)
            .expect("the real duplicate file length should be set");
    }

    benchmark_sample_plans("duplicate-mix", &root);
    benchmark_worker_counts("duplicate-mix", &root);
    fs::remove_dir_all(root).expect("the duplicate-mix fixture should be removed");
}
