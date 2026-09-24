use std::{path::Path, time::Instant};

use crate::{
    filesystem::{
        metadata::{diagnostic_path, now_ms},
        permanent_delete::delete_file_candidate_permanently,
        PermanentDeleteBatchResult, PermanentDeleteFailure,
    },
    history::{file_cleanup_record, FileCleanupHistoryCategory, HistoryService},
    shared::{
        operation::{CoordinatedOperationKind, OperationGuard},
        CoreResult, TraversalProgress,
    },
    storage::index::cache,
    storage::large_files::{LargeFileScanMode, LargeFilesResult},
    storage::traversal::{LargeFileScanDiagnostics, StorageTraversal},
    ProgressSink,
};

use super::session::{
    filter_result, publish_result_session, resolve_delete_candidates, resolve_open_target,
    synchronize_removed_paths,
};

pub struct LargeFileService;

impl LargeFileService {
    pub fn find_with_progress(
        roots: Vec<String>,
        minimum_bytes: u64,
        scan_mode: LargeFileScanMode,
        excluded_paths: Vec<String>,
        callback: impl ProgressSink,
    ) -> CoreResult<LargeFilesResult> {
        let result = StorageTraversal::find_large_files_with_progress(
            roots,
            minimum_bytes,
            scan_mode,
            excluded_paths,
            move |progress| callback.report(progress),
        )?;
        Ok(publish_result_session(result)?)
    }

    pub fn filter(scan_id: u64, minimum_bytes: u64) -> CoreResult<LargeFilesResult> {
        let started = Instant::now();
        let result = filter_result(scan_id, minimum_bytes)?;
        let result = publish_result_session(result)?;
        log::info!(
            "large_file_filter_finished source_scan_id={} scan_id={} mode={} minimum_bytes={} total_count={} returned_count={} elapsed_ms={}",
            scan_id,
            result.scan_id,
            result.scan_mode.as_str(),
            result.minimum_bytes,
            result.total_count,
            result.returned_count,
            started.elapsed().as_millis()
        );
        Ok(result)
    }

    pub(crate) fn find_with_diagnostics(
        roots: Vec<String>,
        minimum_bytes: u64,
        scan_mode: LargeFileScanMode,
        excluded_paths: Vec<String>,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<(LargeFilesResult, LargeFileScanDiagnostics)> {
        StorageTraversal::find_large_files_with_diagnostics(
            roots,
            minimum_bytes,
            scan_mode,
            excluded_paths,
            callback,
        )
    }

    pub fn cancel() {
        StorageTraversal::cancel_large_files();
    }

    pub fn resolve_open_target(scan_id: u64, selected_path: String) -> CoreResult<String> {
        Ok(resolve_open_target(scan_id, &selected_path)?)
    }

    pub fn delete_files_permanently(
        scan_id: u64,
        selected_paths: Vec<String>,
    ) -> CoreResult<PermanentDeleteBatchResult> {
        let selection = resolve_delete_candidates(scan_id, selected_paths)?;
        let expected_bytes = selection.expected_allocated_bytes;
        let candidates = selection.candidates;
        let operation = OperationGuard::start(CoordinatedOperationKind::PermanentDelete)?;
        let started = Instant::now();
        let started_at_ms = now_ms();
        let requested_count = candidates.len();
        let selected_paths = candidates
            .iter()
            .map(|candidate| candidate.path.clone())
            .collect::<Vec<_>>();
        let path_sample = candidates
            .iter()
            .take(3)
            .map(|candidate| diagnostic_path(Path::new(&candidate.path)))
            .collect::<Vec<_>>();
        let mut result = PermanentDeleteBatchResult {
            removed_paths: Vec::new(),
            failed: Vec::new(),
            released_bytes: 0,
        };
        for candidate in candidates {
            match delete_file_candidate_permanently(&candidate) {
                Ok((target, usage)) => {
                    result.released_bytes =
                        result.released_bytes.saturating_add(usage.allocated_bytes);
                    result.removed_paths.push(candidate.path);
                    cache::remove_entry(&target, usage, 1, false);
                }
                Err(error) => result.failed.push(PermanentDeleteFailure {
                    path: candidate.path,
                    message: error.to_string(),
                }),
            }
        }
        synchronize_removed_paths(scan_id, &result.removed_paths)?;
        let history_record = file_cleanup_record(
            format!("large-file-cleanup-{}-{}", operation.id(), now_ms()),
            FileCleanupHistoryCategory::LargeFiles,
            started_at_ms,
            now_ms(),
            selected_paths,
            expected_bytes,
            &result,
        );
        if let Err(error) = HistoryService::append(history_record) {
            log::warn!(
                "large_file_history_save_failed operation_id={} error={}",
                operation.id(),
                mangodisk_platform::diagnostics::text(&error)
            );
        }
        log::info!(
            "permanent_delete_batch_finished operation_id={} scan_id={} requested_count={} path_sample={:?} selected_allocated_bytes={} removed_count={} failed_count={} released_allocated_bytes={} elapsed_ms={}",
            operation.id(),
            scan_id,
            requested_count,
            path_sample,
            expected_bytes,
            result.removed_paths.len(),
            result.failed.len(),
            result.released_bytes,
            started.elapsed().as_millis()
        );
        operation.complete();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write, path::PathBuf};

    use mangodisk_platform::{current_platform, Platform};

    use super::*;
    use crate::storage::large_files::LARGE_FILE_CANDIDATE_FLOOR_BYTES;

    struct LargeFileFixture {
        root: PathBuf,
    }

    impl LargeFileFixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "mangodisk-large-file-service-{}-{}",
                std::process::id(),
                now_ms()
            ));
            fs::create_dir_all(&root).expect("the large-file service fixture should be created");
            Self { root }
        }

        fn file(&self) -> PathBuf {
            self.root.join("candidate.bin")
        }

        fn write_dense_candidate(&self, path: &Path) {
            fs::write(
                path,
                vec![3_u8; (LARGE_FILE_CANDIDATE_FLOOR_BYTES + 1024) as usize],
            )
            .expect("the dense large-file candidate should be written");
        }
    }

    #[test]
    fn overlapping_scopes_do_not_repeat_candidate_discovery() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let fixture = LargeFileFixture::new();
        let children = [
            fixture.root.join("downloads"),
            fixture.root.join("documents"),
        ];
        fixture.write_dense_candidate(&fixture.root.join("parent.bin"));
        for child in &children {
            fs::create_dir_all(child).unwrap();
            fixture.write_dense_candidate(&child.join("child.bin"));
        }
        let roots = std::iter::once(&fixture.root)
            .chain(children.iter())
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        let (result, diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
            roots,
            1,
            LargeFileScanMode::Complete,
            vec![],
            |_| {},
        )
        .unwrap();
        assert_eq!(result.total_count, 3);
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        {
            assert_eq!(diagnostics.fast_path, "used");
            assert_eq!(
                diagnostics.native_directory_reads, 3,
                "each physical directory should be read once"
            );
            assert_eq!(
                diagnostics.candidate_count, 3,
                "native discovery must visit each candidate once, before Core result filtering"
            );
        }
    }

    #[test]
    #[ignore = "creates an isolated overlap benchmark dataset and prints timing evidence"]
    fn overlapping_scopes_scan_benchmark() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let fixture = LargeFileFixture::new();
        let children = [
            fixture.root.join("downloads"),
            fixture.root.join("documents"),
        ];
        fixture.write_dense_candidate(&fixture.root.join("parent.bin"));
        for child in &children {
            for directory in 0..64 {
                let path = child.join(format!("dir-{directory}"));
                fs::create_dir_all(&path).unwrap();
                for file in 0..64 {
                    fs::write(path.join(format!("small-{file}.bin")), [1_u8; 64]).unwrap();
                }
            }
            fixture.write_dense_candidate(&child.join("child.bin"));
        }
        for run in 0..7 {
            for multiple in [false, true] {
                let mut roots = vec![fixture.root.to_string_lossy().into_owned()];
                if multiple {
                    roots.extend(
                        children
                            .iter()
                            .map(|path| path.to_string_lossy().into_owned()),
                    );
                }
                let observed = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let captured = std::sync::Arc::clone(&observed);
                let start = std::time::Instant::now();
                let (result, diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
                    roots,
                    1,
                    LargeFileScanMode::Complete,
                    vec![],
                    move |event| {
                        captured
                            .fetch_max(event.items_scanned, std::sync::atomic::Ordering::Relaxed);
                    },
                )
                .unwrap();
                assert_eq!(result.total_count, 3);
                println!("overlap_benchmark run={run} multiple={multiple} elapsed_us={} candidates={} observed_items={} directory_reads={} strategy={}",
                    start.elapsed().as_micros(), diagnostics.candidate_count,
                    observed.load(std::sync::atomic::Ordering::Relaxed), diagnostics.native_directory_reads, diagnostics.candidate_strategy);
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "reads a dedicated three-file fixture already indexed by Spotlight"]
    fn indexed_overlapping_scopes_benchmark() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let root = PathBuf::from(std::env::var("MANGODISK_INDEXED_OVERLAP_FIXTURE").unwrap());
        for run in 0..7 {
            for multiple in [false, true] {
                let mut roots = vec![root.to_string_lossy().into_owned()];
                if multiple {
                    roots.extend(
                        [root.join("downloads"), root.join("documents")]
                            .iter()
                            .map(|path| path.to_string_lossy().into_owned()),
                    );
                }
                let start = std::time::Instant::now();
                let (result, diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
                    roots,
                    1,
                    LargeFileScanMode::Quick,
                    vec![],
                    |_| {},
                )
                .unwrap();
                assert_eq!(
                    result.total_count, 3,
                    "wait until the dedicated fixture is indexed"
                );
                assert_eq!(
                    diagnostics.candidate_count, 3,
                    "query overlapping index scopes only once"
                );
                println!("indexed_overlap_benchmark run={run} multiple={multiple} elapsed_us={} candidates={} directory_reads={}",
                    start.elapsed().as_micros(), diagnostics.candidate_count, diagnostics.native_directory_reads);
            }
        }
        let roots = [root.clone(), root.join("downloads"), root.join("documents")]
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let result = LargeFileService::find_with_progress(
            roots.clone(),
            1,
            LargeFileScanMode::Quick,
            vec![root.join("documents").to_string_lossy().into_owned()],
            |_| {},
        )
        .unwrap();
        assert_eq!(
            result.total_count, 2,
            "a saved exclusion also applies to an explicitly selected child"
        );
        assert_eq!(result.roots.len(), 3);
        assert_eq!(
            resolve_delete_candidates(
                result.scan_id,
                result
                    .entries
                    .iter()
                    .map(|entry| entry.path.clone())
                    .collect()
            )
            .unwrap()
            .candidates
            .len(),
            2
        );
        let filtered = LargeFileService::filter(result.scan_id, u64::MAX).unwrap();
        assert!(filtered.entries.is_empty());
        assert_eq!(
            LargeFileService::filter(filtered.scan_id, 1)
                .unwrap()
                .total_count,
            2
        );
        let cancelled = LargeFileService::find_with_progress(
            roots,
            1,
            LargeFileScanMode::Quick,
            vec![],
            |event: TraversalProgress| {
                if event.completed_steps == 1 {
                    LargeFileService::cancel();
                }
            },
        )
        .unwrap_err();
        assert_eq!(cancelled.code(), crate::CoreErrorCode::OperationCancelled);
    }

    #[test]
    fn multiple_roots_share_one_session_and_preserve_filter_and_delete_boundaries() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let fixture = LargeFileFixture::new();
        let first = fixture.root.join("first");
        let second = fixture.root.join("second");
        let nested = first.join("nested");
        let excluded = second.join("excluded");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&excluded).unwrap();
        fixture.write_dense_candidate(&nested.join("one.bin"));
        fixture.write_dense_candidate(&second.join("two.bin"));
        fixture.write_dense_candidate(&excluded.join("hidden.bin"));
        let roots = vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
            nested.to_string_lossy().into_owned(),
            first.to_string_lossy().into_owned(),
        ];
        let exclusions = vec![excluded.to_string_lossy().into_owned()];
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = std::sync::Arc::clone(&events);
        let result = LargeFileService::find_with_progress(
            roots.clone(),
            1,
            LargeFileScanMode::Complete,
            exclusions.clone(),
            move |event| {
                captured.lock().unwrap().push(event);
            },
        )
        .expect("multiple roots should publish one result session");
        assert_eq!(result.roots.len(), 3);
        assert_eq!(result.total_count, 2);
        assert!(result
            .entries
            .iter()
            .all(|entry| entry.name != "hidden.bin"));
        assert_eq!(
            result.total_bytes,
            result.entries.iter().map(|entry| entry.bytes).sum::<u64>()
        );
        assert!(result
            .entries
            .windows(2)
            .all(|entries| entries[0].bytes >= entries[1].bytes));
        let paths = result
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        let selection = resolve_delete_candidates(result.scan_id, paths.clone()).unwrap();
        assert_eq!(selection.candidates.len(), 2);
        assert!(resolve_delete_candidates(
            result.scan_id,
            vec![excluded.join("hidden.bin").to_string_lossy().into_owned()]
        )
        .is_err());
        for path in paths {
            assert!(LargeFileService::resolve_open_target(result.scan_id, path).is_ok());
        }
        let filtered = LargeFileService::filter(result.scan_id, u64::MAX).unwrap();
        assert!(filtered.entries.is_empty());
        assert_eq!(filtered.roots, result.roots);
        let restored = LargeFileService::filter(filtered.scan_id, 1).unwrap();
        assert_eq!(restored.total_count, 2);
        let events = events.lock().unwrap();
        let last = events.last().unwrap();
        assert_eq!(last.completed_steps, 3);
        assert_eq!(last.total_steps, 3);
        assert!(last.items_scanned >= 2);
        assert!(events
            .iter()
            .all(|event| event.operation_id == last.operation_id));
        drop(events);
        let cancelled = LargeFileService::find_with_progress(
            roots,
            1,
            LargeFileScanMode::Complete,
            exclusions,
            |event: TraversalProgress| {
                if event.completed_steps == 1 {
                    LargeFileService::cancel();
                }
            },
        )
        .expect_err("cancelling after the first root must not publish partial results");
        assert_eq!(cancelled.code(), crate::CoreErrorCode::OperationCancelled);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires an isolated parent directory with a real nested mounted volume"]
    fn real_nested_volume_keeps_both_scopes_and_authorizes_both_results() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let parent = PathBuf::from(
            std::env::var("MANGODISK_LARGE_FILE_MOUNT_FIXTURE")
                .expect("supply the isolated mount fixture"),
        );
        let mounted = parent.join("mounted");
        assert!(!current_platform().is_same_filesystem(
            &fs::metadata(&parent).unwrap(),
            &fs::metadata(&mounted).unwrap()
        ));
        let result = LargeFileService::find_with_progress(
            vec![
                parent.to_string_lossy().into_owned(),
                mounted.to_string_lossy().into_owned(),
            ],
            1,
            LargeFileScanMode::Complete,
            vec![],
            |_| {},
        )
        .unwrap();
        assert_eq!(result.roots.len(), 2);
        assert_eq!(
            result.total_count, 2,
            "both host and mounted-volume files must be included exactly once"
        );
        let mut names = result
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, vec!["host.bin", "mounted.bin"]);
        let paths = result
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        assert_eq!(
            resolve_delete_candidates(result.scan_id, paths)
                .unwrap()
                .candidates
                .len(),
            2
        );
        let filtered = LargeFileService::filter(result.scan_id, u64::MAX).unwrap();
        assert!(filtered.entries.is_empty());
        let restored = LargeFileService::filter(filtered.scan_id, 1).unwrap();
        assert_eq!(restored.total_count, 2);
        assert_eq!(restored.roots, result.roots);
    }

    #[test]
    fn empty_scopes_never_expand_to_the_system_volume() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let error = LargeFileService::find_with_progress(
            vec![],
            1,
            LargeFileScanMode::Complete,
            vec![],
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), crate::CoreErrorCode::InvalidInput);
    }

    #[test]
    fn complete_scan_omits_candidates_below_a_user_exclusion() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().expect("the large-file cache should be clear before the service test");
        let fixture = LargeFileFixture::new();
        let excluded = fixture.root.join("excluded");
        fs::create_dir_all(&excluded).expect("the excluded directory should be created");
        let included_file = fixture.root.join("included.bin");
        let excluded_file = excluded.join("excluded.bin");
        fixture.write_dense_candidate(&included_file);
        fixture.write_dense_candidate(&excluded_file);

        let result = LargeFileService::find_with_progress(
            vec![fixture.root.to_string_lossy().into_owned()],
            LARGE_FILE_CANDIDATE_FLOOR_BYTES,
            LargeFileScanMode::Complete,
            vec![excluded.to_string_lossy().into_owned()],
            |_| {},
        )
        .expect("the complete scan should apply the user exclusion");

        assert_eq!(result.entries.len(), 1);
        let included_file =
            fs::canonicalize(&included_file).expect("the included candidate should resolve");
        let excluded_file =
            fs::canonicalize(&excluded_file).expect("the excluded candidate should resolve");
        assert!(current_platform().paths_equal(Path::new(&result.entries[0].path), &included_file));
        assert!(result
            .entries
            .iter()
            .all(|entry| !current_platform().paths_equal(Path::new(&entry.path), &excluded_file)));
    }

    impl Drop for LargeFileFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn changed_large_file_is_preserved_until_a_fresh_snapshot_authorizes_delete() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().expect("the large-file cache should be clear before the service test");
        HistoryService::clear().expect("the test history should be clear before the service test");
        let fixture = LargeFileFixture::new();
        let path = fixture.file();
        let initial_bytes = LARGE_FILE_CANDIDATE_FLOOR_BYTES.saturating_add(1024 * 1024);
        fs::write(&path, vec![3_u8; initial_bytes as usize])
            .expect("the dense large-file candidate should be written");

        let initial = LargeFileService::find_with_progress(
            vec![fixture.root.to_string_lossy().into_owned()],
            1,
            LargeFileScanMode::Complete,
            vec![],
            |_| {},
        )
        .expect("the large-file service should scan the isolated fixture");
        assert_eq!(initial.entries.len(), 1);
        let filtered = LargeFileService::filter(initial.scan_id, initial_bytes + 1)
            .expect("the active scan should support an in-memory threshold filter");
        assert!(filtered.entries.is_empty());
        let restored = LargeFileService::filter(filtered.scan_id, LARGE_FILE_CANDIDATE_FLOOR_BYTES)
            .expect("lowering the threshold should restore the retained candidate");
        assert_eq!(restored.entries.len(), 1);
        assert_eq!(restored.scan_mode, LargeFileScanMode::Complete);
        let selected_path = initial.entries[0].path.clone();
        assert_eq!(
            LargeFileService::resolve_open_target(initial.scan_id, selected_path.clone())
                .expect("the published large file should resolve"),
            selected_path
        );

        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("the large-file candidate should reopen")
            .write_all(&[9])
            .expect("the large-file candidate should change after the scan");
        let stale_delete = LargeFileService::delete_files_permanently(
            initial.scan_id,
            vec![selected_path.clone()],
        )
        .expect("a live preflight failure should remain a typed batch result");
        assert!(stale_delete.removed_paths.is_empty());
        assert_eq!(stale_delete.failed.len(), 1);
        assert!(
            path.exists(),
            "failed preflight must preserve the changed file"
        );

        let refreshed = LargeFileService::find_with_progress(
            vec![fixture.root.to_string_lossy().into_owned()],
            1,
            LargeFileScanMode::Complete,
            vec![],
            |_| {},
        )
        .expect("the changed large-file fixture should rescan successfully");
        let deleted = LargeFileService::delete_files_permanently(
            refreshed.scan_id,
            vec![selected_path.clone()],
        )
        .expect("a candidate matching the fresh snapshot should be deleted");

        assert_eq!(deleted.removed_paths, vec![selected_path.clone()]);
        assert!(deleted.failed.is_empty());
        assert!(!path.exists());
        assert!(
            LargeFileService::resolve_open_target(refreshed.scan_id, selected_path).is_err(),
            "a deleted file must disappear from the authoritative result session"
        );
        let history = HistoryService::list().expect("large-file cleanup history should load");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].affected_item_count, 1);
        assert_eq!(history[1].failed_item_count, 1);

        HistoryService::clear().expect("the test history should be clear after the service test");
        cache::clear_all().expect("the large-file cache should be clear after the service test");
    }
}
