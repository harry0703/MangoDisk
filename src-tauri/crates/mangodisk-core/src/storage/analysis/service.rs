use crate::filesystem::ScanExclusionOptions;
use std::time::Instant;

use crate::{
    filesystem::{
        metadata::diagnostic_path, permanent_delete::delete_analysis_candidate_permanently,
    },
    shared::{
        operation::{CoordinatedOperationKind, OperationGuard},
        CoreResult, TraversalProgress,
    },
    storage::traversal::{AnalysisScanDiagnostics, StorageTraversal},
    storage::{
        analysis::{
            AnalysisDeleteResult, AnalysisRemainderPage, AnalysisRemainderRequest, AnalysisResult,
            AnalysisScanMode,
        },
        index::cache,
    },
    ProgressSink,
};

use super::session::{
    invalidate_changed_path, publish_result_session, resolve_entry_candidate, resolve_open_path,
    resolve_remainder_parent, synchronize_removed_path,
};

pub struct AnalysisService;

impl AnalysisService {
    pub fn list_remainder(request: AnalysisRemainderRequest) -> CoreResult<AnalysisRemainderPage> {
        if request.schema_version != AnalysisRemainderRequest::SCHEMA_VERSION {
            return Err(crate::shared::CoreError::invalid_input(
                "unsupported analysis remainder schema",
            ));
        }
        if request.visible_paths.len() > super::models::ANALYSIS_VISIBLE_ENTRY_LIMIT {
            return Err(crate::shared::CoreError::invalid_input(
                "too many visible analysis entries",
            ));
        }
        let parent = resolve_remainder_parent(request.scan_id, &request.parent_path)?;
        super::remainder::list(&parent, &request)
    }

    pub fn release_remainder(snapshot_id: u64) -> CoreResult<()> {
        super::remainder::release(snapshot_id)
    }

    pub fn analyze_with_progress(
        path: Option<String>,
        refresh: bool,
        callback: impl ProgressSink,
    ) -> CoreResult<AnalysisResult> {
        let result =
            StorageTraversal::analyze_path_with_progress(path, refresh, move |progress| {
                callback.report(progress);
            })?;
        Ok(publish_result_session(result)?)
    }

    pub fn analyze_with_exclusions_progress(
        path: Option<String>,
        refresh: bool,
        excluded_paths: impl Into<ScanExclusionOptions>,
        callback: impl ProgressSink,
    ) -> CoreResult<AnalysisResult> {
        Self::analyze_with_mode_progress(
            path,
            refresh,
            excluded_paths,
            AnalysisScanMode::Standard,
            callback,
        )
    }

    pub fn analyze_with_mode_progress(
        path: Option<String>,
        refresh: bool,
        excluded_paths: impl Into<ScanExclusionOptions>,
        scan_mode: AnalysisScanMode,
        callback: impl ProgressSink,
    ) -> CoreResult<AnalysisResult> {
        let excluded_paths = excluded_paths.into();
        let snapshot = StorageTraversal::analyze_path_with_mode_snapshot(
            path,
            refresh,
            excluded_paths,
            scan_mode,
            move |progress| callback.report(progress),
        )?;
        Ok(super::session::publish_result_with_exclusions(
            snapshot.result,
            snapshot.exclusions,
        )?)
    }

    pub(crate) fn analyze_with_diagnostics(
        path: Option<String>,
        refresh: bool,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<(AnalysisResult, AnalysisScanDiagnostics)> {
        StorageTraversal::analyze_path_with_diagnostics(path, refresh, callback)
    }

    pub fn cancel() {
        StorageTraversal::cancel_analysis();
    }

    /// Resolves an external-open request against the authoritative scan snapshot.
    ///
    /// The platform adapter owns launching the system handler. Core only proves
    /// that the requested path was published to the current UI by a real scan.
    pub fn resolve_open_target(scan_id: u64, selected_path: String) -> CoreResult<String> {
        Ok(resolve_open_path(scan_id, &selected_path)?)
    }

    pub fn delete_entry_permanently(
        scan_id: u64,
        selected_path: String,
    ) -> CoreResult<AnalysisDeleteResult> {
        let candidate = resolve_entry_candidate(scan_id, &selected_path)?;
        let operation = OperationGuard::start(CoordinatedOperationKind::PermanentDelete)?;
        let started = Instant::now();
        let is_directory = candidate.is_directory;
        let scan_mode = candidate.scan_mode;
        let has_shared_allocation = candidate.has_shared_allocation;
        let selected_path = std::path::PathBuf::from(&candidate.path);
        log::info!(
            "analysis_permanent_delete_started operation_id={} scan_id={} path={} scan_mode={} entry_kind={}",
            operation.id(),
            scan_id,
            diagnostic_path(std::path::Path::new(&candidate.path)),
            scan_mode.as_str(),
            if is_directory { "directory" } else { "file" }
        );
        let mut outcome = match delete_analysis_candidate_permanently(candidate) {
            Ok(outcome) => outcome,
            Err(error) => {
                if error.is_partial() {
                    if let Err(session_error) = super::session::invalidate_all() {
                        log::error!("analysis_partial_delete_session_invalidation_failed operation_id={} path={} error={}",
                            operation.id(), diagnostic_path(&selected_path), mangodisk_platform::diagnostics::text(&session_error));
                    }
                    // A partially changed directory no longer matches any derived index snapshot.
                    // Clearing the rebuildable cache prevents stale sizes from surviving the
                    // irreversible boundary.
                    let cache_started = Instant::now();
                    let cache_result = cache::clear_all();
                    log::info!(
                        "analysis_delete_stage_finished operation_id={} stage=invalidate_cache outcome={} elapsed_ms={}",
                        operation.id(), if cache_result.is_ok() { "completed" } else { "failed" },
                        cache_started.elapsed().as_millis()
                    );
                    if let Err(cache_error) = cache_result {
                        log::error!(
                            "analysis_partial_delete_cache_clear_failed operation_id={} scan_id={} error={}",
                            operation.id(),
                            scan_id,
                            mangodisk_platform::diagnostics::text(&cache_error)
                        );
                    }
                }
                log::warn!(
                    "analysis_permanent_delete_failed operation_id={} scan_id={} partial={} released_logical_bytes={:?} removed_files={:?} remaining_restored={} elapsed_ms={} error={}",
                    operation.id(),
                    scan_id,
                    error.is_partial(),
                    error.observed_or_estimated_bytes(),
                    error.observed_or_estimated_files(),
                    error.remaining_was_restored(),
                    started.elapsed().as_millis(),
                    mangodisk_platform::diagnostics::text(&error)
                );
                let mut core_error = crate::shared::CoreError::operation_failed(error.to_string());
                if let Some(reason) = error.reason() {
                    core_error = core_error.with_reason(reason);
                }
                if error.is_partial() {
                    core_error = core_error.with_possible_side_effects();
                    if !error.remaining_was_restored() {
                        core_error = core_error
                            .with_reason(crate::shared::CoreErrorReason::DeleteRecoveryFailed);
                    } else if error.reason()
                        != Some(crate::shared::CoreErrorReason::DirectoryNotEmpty)
                    {
                        core_error = core_error
                            .with_reason(crate::shared::CoreErrorReason::DeleteIncomplete);
                    }
                }
                return Err(core_error);
            }
        };
        let cache_started = Instant::now();
        // Staging frees the original name before recursive deletion completes.
        // A concurrently recreated entry belongs to another operation and must
        // remain visible; uncertainty also requires a fresh authoritative scan.
        outcome.result.requires_rescan = original_path_requires_rescan(&outcome.target);
        if outcome.result.requires_rescan {
            let invalidation = if has_shared_allocation {
                super::session::invalidate_all()
            } else {
                invalidate_changed_path(&outcome.target)
            };
            invalidation.map_err(|error| {
                crate::shared::CoreError::operation_failed(error).with_possible_side_effects()
            })?;
            cache::clear_all().map_err(|error| {
                crate::shared::CoreError::operation_failed(error).with_possible_side_effects()
            })?;
            log::info!(
                "analysis_delete_rescan_required operation_id={} path={} shared_allocation={} reason=original_path_recreated action=rescan",
                operation.id(),
                diagnostic_path(&outcome.target),
                has_shared_allocation
            );
        } else {
            let synchronized =
                synchronize_removed_path(scan_id, &outcome.target)
                    .map_err(|error| {
                    if let Err(invalidation_error) = super::session::invalidate_all() {
                        log::error!("analysis_delete_session_invalidation_failed operation_id={} path={} error={}",
                            operation.id(), diagnostic_path(&outcome.target), mangodisk_platform::diagnostics::text(&invalidation_error));
                    }
                    if let Err(cache_error) = cache::clear_all() {
                        log::error!("analysis_delete_cache_clear_failed operation_id={} path={} error={}",
                            operation.id(), diagnostic_path(&outcome.target), mangodisk_platform::diagnostics::text(&cache_error));
                    }
                    crate::shared::CoreError::operation_failed(error).with_possible_side_effects()
                })?;
            cache::remove_entry_with_allocation_credits(
                &outcome.target,
                outcome.removed_usage,
                outcome.result.removed_file_count,
                is_directory,
                scan_mode,
                Some(&synchronized.credits),
            );
            outcome.result.updated_results = synchronized.updated_results;
            outcome.result.invalidated_scan_ids = synchronized.invalidated_scan_ids;
            for result in &mut outcome.result.updated_results {
                match cache::analysis_result(std::path::Path::new(&result.root)) {
                    Ok(Some(rebuilt)) => *result = super::session::replace_reconciled_result(rebuilt, result.scan_id)
                        .map_err(|error| {
                            crate::shared::CoreError::operation_failed(error)
                                .with_possible_side_effects()
                        })?,
                    Ok(None) => {},
                    Err(error) => log::warn!(
                        "analysis_delete_projection_rebuild_failed operation_id={} scan_id={} root={} error={} action=retain_reconciled_snapshot",
                        operation.id(), result.scan_id, diagnostic_path(std::path::Path::new(&result.root)), mangodisk_platform::diagnostics::text(&error)
                    ),
                }
            }
            log::info!(
                "analysis_delete_allocation_reconciled operation_id={} path={} updated_sessions={} invalidated_sessions={} promoted_aliases={} action=update_snapshot",
                operation.id(), diagnostic_path(&outcome.target), outcome.result.updated_results.len(),
                outcome.result.invalidated_scan_ids.len(), synchronized.credits.len()
            );
        }
        log::info!(
            "analysis_delete_stage_finished operation_id={} stage=synchronize_cache outcome=completed elapsed_ms={}",
            operation.id(), cache_started.elapsed().as_millis()
        );
        log::info!(
            "analysis_permanent_delete_finished operation_id={} scan_id={} path={} entry_kind={} snapshot_logical_bytes={} snapshot_displayed_bytes={} scan_mode={} snapshot_file_count={} count_source=scan_snapshot elapsed_ms={}",
            operation.id(),
            scan_id,
            diagnostic_path(&outcome.target),
            if is_directory { "directory" } else { "file" },
            outcome.removed_usage.logical_bytes,
            outcome.result.released_bytes,
            scan_mode.as_str(),
            outcome.result.removed_file_count,
            started.elapsed().as_millis()
        );
        operation.complete();
        Ok(outcome.result)
    }
}

fn original_path_requires_rescan(path: &std::path::Path) -> bool {
    !matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}

#[cfg(test)]
#[path = "delete_validation_tests.rs"]
mod delete_validation_tests;

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use super::*;

    #[test]
    #[cfg(windows)]
    fn scan_modes_keep_navigation_remainder_and_delete_in_one_metric() {
        use mangodisk_platform::Platform;
        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().unwrap();
        let fixture = AnalysisFixture::new();
        let nested = fixture.root.join("nested");
        fs::create_dir(&nested).unwrap();
        let candidate = nested.join("candidate.bin");
        fs::write(&candidate, vec![0_u8; 1024 * 1024]).unwrap();
        #[cfg(windows)]
        {
            let output = std::process::Command::new("compact.exe")
                .args(["/C", "/F", "/EXE:XPRESS4K"])
                .arg(&candidate)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "WOF fixture compression must succeed"
            );
        }
        let logical = fs::metadata(&candidate).unwrap().len();
        let native = mangodisk_platform::current_platform()
            .file_space_usage(&candidate, &fs::metadata(&candidate).unwrap())
            .allocated_bytes;
        assert!(
            native < logical,
            "fixture must distinguish allocation from logical length"
        );
        for index in 0..520 {
            fs::write(fixture.root.join(format!("small-{index:03}")), b"x").unwrap();
        }
        let analyze = |root: &PathBuf, refresh, mode| {
            AnalysisService::analyze_with_mode_progress(
                Some(root.to_string_lossy().into_owned()),
                refresh,
                ScanExclusionOptions::default(),
                mode,
                |_| {},
            )
            .unwrap()
        };
        let fast = analyze(&fixture.root, true, AnalysisScanMode::Fast);
        assert_eq!(fast.scan_mode, AnalysisScanMode::Fast);
        assert_eq!(fast.total_bytes, logical + 520);
        let details = AnalysisService::list_remainder(AnalysisRemainderRequest {
            schema_version: AnalysisRemainderRequest::SCHEMA_VERSION,
            scan_id: fast.scan_id,
            parent_path: fast.root.clone(),
            visible_paths: fast
                .entries
                .iter()
                .map(|entry| entry.path.clone())
                .collect(),
            expected_bytes: 21,
            offset: 0,
            snapshot_id: None,
        })
        .unwrap();
        assert_eq!(details.total_bytes, 21);
        assert!(details.entries.iter().all(|entry| entry.bytes == 1));
        AnalysisService::release_remainder(details.snapshot_id).unwrap();
        let child = analyze(&nested, false, AnalysisScanMode::Fast);
        assert_eq!(child.total_bytes, logical);
        assert_eq!(child.scan_mode, AnalysisScanMode::Fast);
        let standard = analyze(&nested, false, AnalysisScanMode::Standard);
        assert_eq!(
            standard.total_bytes, native,
            "switching a descendant must not reuse a logical ancestor"
        );
        assert_eq!(standard.scan_mode, AnalysisScanMode::Standard);
        let child = analyze(&nested, false, AnalysisScanMode::Fast);
        assert_eq!(
            child.total_bytes, logical,
            "switching back must not reuse native allocation"
        );
        let _standard_parent = analyze(&fixture.root, true, AnalysisScanMode::Standard);
        let removed =
            AnalysisService::delete_entry_permanently(child.scan_id, child.entries[0].path.clone())
                .unwrap();
        assert_eq!(
            removed.released_bytes, logical,
            "delete reconciliation uses the displayed metric"
        );
        let empty = analyze(&nested, true, AnalysisScanMode::Fast);
        assert_eq!(empty.total_bytes, 0);
        assert!(!candidate.exists());
        cache::clear_all().unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn unix_analysis_uses_allocation_even_when_fast_is_requested() {
        use mangodisk_platform::Platform;
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let fixture = AnalysisFixture::new();
        let file = fixture.root.join("sparse.bin");
        fs::write(&file, [1_u8; 4096]).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&file)
            .unwrap()
            .set_len(16 * 1024 * 1024)
            .unwrap();
        let metadata = fs::symlink_metadata(&file).unwrap();
        let allocated = mangodisk_platform::current_platform()
            .file_space_usage(&file, &metadata)
            .allocated_bytes;
        assert!(allocated < metadata.len());
        let result = AnalysisService::analyze_with_mode_progress(
            Some(fixture.root.to_string_lossy().into_owned()),
            true,
            ScanExclusionOptions::default(),
            AnalysisScanMode::Fast,
            |_| {},
        )
        .unwrap();
        assert_eq!(result.scan_mode, AnalysisScanMode::Standard);
        assert_eq!(result.total_bytes, allocated);
        assert_eq!(result.entries[0].bytes, allocated);
    }

    struct AnalysisFixture {
        root: PathBuf,
    }

    impl AnalysisFixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "mangodisk-analysis-service-{}-{}",
                std::process::id(),
                crate::filesystem::metadata::now_ms()
            ));
            fs::create_dir_all(&root).expect("the analysis service fixture should be created");
            Self { root }
        }

        fn file(&self) -> PathBuf {
            self.root.join("candidate.bin")
        }
    }

    impl Drop for AnalysisFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(unix)]
    #[test]
    fn deleting_unrelated_entries_preserves_sessions_with_shared_allocation() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        for evict_index in [false, true] {
            cache::clear_all().unwrap();
            let fixture = AnalysisFixture::new();
            fs::write(fixture.root.join("owner.bin"), vec![7; 8192]).unwrap();
            fs::hard_link(
                fixture.root.join("owner.bin"),
                fixture.root.join("alias.bin"),
            )
            .unwrap();
            fs::write(fixture.root.join("ordinary.bin"), vec![1; 4096]).unwrap();
            fs::create_dir(fixture.root.join("ordinary-folder")).unwrap();
            fs::write(fixture.root.join("ordinary-folder/empty.bin"), []).unwrap();
            fs::write(fixture.root.join("ordinary-folder/data.bin"), vec![2; 4096]).unwrap();
            let result = AnalysisService::analyze_with_progress(
                Some(fixture.root.to_string_lossy().into_owned()),
                true,
                |_| {},
            )
            .unwrap();
            if evict_index {
                cache::clear_all().unwrap();
            }
            for name in ["ordinary.bin", "ordinary-folder"] {
                let entry = result
                    .entries
                    .iter()
                    .find(|entry| entry.name == name)
                    .unwrap();
                let deleted =
                    AnalysisService::delete_entry_permanently(result.scan_id, entry.path.clone())
                        .unwrap();
                assert!(
                    !deleted.requires_rescan,
                    "unrelated deletion must not rescan: {name}"
                );
                assert!(!std::path::Path::new(&entry.path).exists());
                assert!(resolve_entry_candidate(result.scan_id, &entry.path).is_err());
                let owner = result
                    .entries
                    .iter()
                    .find(|entry| entry.name == "owner.bin")
                    .unwrap();
                assert!(resolve_entry_candidate(result.scan_id, &owner.path).is_ok());
            }
            if !evict_index {
                let root = fs::canonicalize(&fixture.root).unwrap();
                let cached = cache::analysis_result(&root).unwrap().unwrap();
                assert_eq!(cached.entries.len(), 2);
                assert_eq!(
                    cached.total_bytes,
                    result
                        .entries
                        .iter()
                        .filter(|entry| entry.name == "owner.bin" || entry.name == "alias.bin")
                        .map(|entry| entry.bytes)
                        .sum::<u64>()
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn repeated_hard_link_deletion_reassigns_allocation_without_scanning() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        for evict_index in [false, true] {
            cache::clear_all().unwrap();
            let fixture = AnalysisFixture::new();
            let owner = fixture.root.join("a.bin");
            fs::write(&owner, vec![7; 8192]).unwrap();
            for name in ["b.bin", "c.bin"] {
                fs::hard_link(&owner, fixture.root.join(name)).unwrap();
            }
            let result = AnalysisService::analyze_with_progress(
                Some(fixture.root.to_string_lossy().into_owned()),
                true,
                |_| {},
            )
            .unwrap();
            if evict_index {
                cache::clear_all().unwrap();
            }
            for (index, name) in ["a.bin", "b.bin", "c.bin"].into_iter().enumerate() {
                let deleted = AnalysisService::delete_entry_permanently(
                    result.scan_id,
                    std::path::Path::new(&result.root)
                        .join(name)
                        .to_string_lossy()
                        .into_owned(),
                )
                .unwrap();
                assert!(!deleted.requires_rescan);
                let updated = deleted
                    .updated_results
                    .iter()
                    .find(|updated| updated.scan_id == result.scan_id)
                    .unwrap();
                let expected_bytes = if index == 2 { 0 } else { result.total_bytes };
                assert_eq!(updated.total_bytes, expected_bytes);
                assert_eq!(updated.entries.len(), 2 - index);
                assert_eq!(
                    updated.entries.iter().map(|entry| entry.bytes).sum::<u64>(),
                    expected_bytes
                );
                assert_eq!(updated.total_entry_count, usize::from(index != 2));
                if !evict_index {
                    let cached = cache::analysis_result(&fs::canonicalize(&fixture.root).unwrap())
                        .unwrap()
                        .unwrap();
                    assert_eq!(cached.total_bytes, expected_bytes);
                    assert_eq!(cached.entries.len(), 2 - index);
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn deleting_zero_charge_alias_keeps_its_owner_and_snapshot() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().unwrap();
        let fixture = AnalysisFixture::new();
        fs::write(fixture.root.join("a.bin"), vec![7; 8192]).unwrap();
        fs::hard_link(fixture.root.join("a.bin"), fixture.root.join("b.bin")).unwrap();
        let result = AnalysisService::analyze_with_progress(
            Some(fixture.root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )
        .unwrap();
        let alias = result
            .entries
            .iter()
            .find(|entry| entry.name == "b.bin")
            .unwrap();
        assert_eq!(alias.bytes, 0);
        let deleted =
            AnalysisService::delete_entry_permanently(result.scan_id, alias.path.clone()).unwrap();
        assert!(!deleted.requires_rescan);
        assert_eq!(deleted.updated_results[0].total_bytes, result.total_bytes);
        assert_eq!(deleted.updated_results[0].entries.len(), 1);
        assert_eq!(deleted.updated_results[0].entries[0].name, "a.bin");
    }

    #[cfg(unix)]
    #[test]
    fn deleting_linked_folder_accounts_for_inside_and_outside_survivors() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        for outside_survives in [false, true] {
            for evict_index in [false, true] {
                cache::clear_all().unwrap();
                let fixture = AnalysisFixture::new();
                let folder = fixture.root.join("a-folder");
                fs::create_dir(&folder).unwrap();
                fs::write(folder.join("owner.bin"), vec![7; 8192]).unwrap();
                fs::hard_link(folder.join("owner.bin"), folder.join("alias.bin")).unwrap();
                fs::write(fixture.root.join("ordinary.bin"), vec![1; 4096]).unwrap();
                if outside_survives {
                    fs::create_dir(fixture.root.join("b-folder")).unwrap();
                    fs::hard_link(
                        folder.join("owner.bin"),
                        fixture.root.join("b-folder/alias.bin"),
                    )
                    .unwrap();
                }
                let result = AnalysisService::analyze_with_progress(
                    Some(fixture.root.to_string_lossy().into_owned()),
                    true,
                    |_| {},
                )
                .unwrap();
                let folder_entry = result
                    .entries
                    .iter()
                    .find(|entry| entry.name == "a-folder")
                    .unwrap();
                if evict_index {
                    cache::clear_all().unwrap();
                }
                let deleted = AnalysisService::delete_entry_permanently(
                    result.scan_id,
                    folder_entry.path.clone(),
                )
                .unwrap();
                assert!(!deleted.requires_rescan);
                let updated = deleted
                    .updated_results
                    .iter()
                    .find(|updated| updated.scan_id == result.scan_id)
                    .unwrap();
                let expected = if outside_survives {
                    result.total_bytes
                } else {
                    result.total_bytes - folder_entry.bytes
                };
                assert_eq!(updated.total_bytes, expected);
                assert_eq!(
                    updated.entries.iter().map(|entry| entry.bytes).sum::<u64>(),
                    expected
                );
                assert!(!folder.exists());
                if outside_survives {
                    let survivor = updated
                        .entries
                        .iter()
                        .find(|entry| entry.name == "b-folder")
                        .unwrap();
                    assert_eq!(survivor.bytes, folder_entry.bytes);
                    if !evict_index {
                        assert_eq!(
                            updated
                                .directory_hierarchy
                                .iter()
                                .find(|node| node.name == "b-folder")
                                .unwrap()
                                .files[0]
                                .bytes,
                            folder_entry.bytes
                        );
                    }
                }
                let fresh =
                    AnalysisService::analyze_with_progress(Some(result.root.clone()), true, |_| {})
                        .unwrap();
                assert_eq!(fresh.total_bytes, updated.total_bytes);
                assert_eq!(fresh.total_entry_count, updated.total_entry_count);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn promoted_alias_folder_enters_bounded_rows_after_index_eviction() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().unwrap();
        let fixture = AnalysisFixture::new();
        let owners = fixture.root.join("a-owners");
        let aliases = fixture.root.join("b-aliases");
        fs::create_dir(&owners).unwrap();
        fs::create_dir(&aliases).unwrap();
        for index in 0..5 {
            let owner = owners.join(format!("{index}.bin"));
            fs::write(&owner, vec![7; 16_384]).unwrap();
            fs::hard_link(&owner, aliases.join(format!("{index}.bin"))).unwrap();
        }
        for index in 0..600 {
            fs::write(fixture.root.join(format!("ordinary-{index}.bin")), [1]).unwrap();
        }
        let result = AnalysisService::analyze_with_progress(
            Some(fixture.root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )
        .unwrap();
        assert!(result.truncated);
        assert!(!result.entries.iter().any(|entry| entry.name == "b-aliases"));
        let owner = result
            .entries
            .iter()
            .find(|entry| entry.name == "a-owners")
            .unwrap();
        cache::clear_all().unwrap();
        let deleted =
            AnalysisService::delete_entry_permanently(result.scan_id, owner.path.clone()).unwrap();
        assert!(!deleted.requires_rescan);
        let updated = &deleted.updated_results[0];
        assert_eq!(updated.total_bytes, result.total_bytes);
        assert_eq!(updated.total_entry_count, result.total_entry_count);
        assert_eq!(
            updated.entries.len(),
            super::super::ANALYSIS_VISIBLE_ENTRY_LIMIT
        );
        let promoted = &updated.entries[0];
        assert_eq!(promoted.name, "b-aliases");
        assert_eq!(promoted.bytes, owner.bytes);
        assert_eq!(promoted.file_count, 5);
        let last = AnalysisService::delete_entry_permanently(result.scan_id, promoted.path.clone())
            .unwrap();
        assert!(!last.requires_rescan);
        assert_eq!(
            last.updated_results[0].total_bytes,
            result.total_bytes - owner.bytes
        );
    }

    #[cfg(unix)]
    #[test]
    fn replaced_alias_does_not_receive_another_files_allocation() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().unwrap();
        let fixture = AnalysisFixture::new();
        fs::write(fixture.root.join("a.bin"), vec![7; 8192]).unwrap();
        for name in ["b.bin", "c.bin"] {
            fs::hard_link(fixture.root.join("a.bin"), fixture.root.join(name)).unwrap();
        }
        let result = AnalysisService::analyze_with_progress(
            Some(fixture.root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )
        .unwrap();
        fs::remove_file(fixture.root.join("b.bin")).unwrap();
        fs::write(fixture.root.join("b.bin"), vec![1; 16_384]).unwrap();
        cache::clear_all().unwrap();
        let owner = result
            .entries
            .iter()
            .find(|entry| entry.name == "a.bin")
            .unwrap();
        let deleted =
            AnalysisService::delete_entry_permanently(result.scan_id, owner.path.clone()).unwrap();
        assert!(!deleted.requires_rescan);
        let updated = &deleted.updated_results[0];
        assert_eq!(
            updated
                .entries
                .iter()
                .find(|entry| entry.name == "c.bin")
                .unwrap()
                .bytes,
            result.total_bytes
        );
        assert_eq!(
            updated
                .entries
                .iter()
                .find(|entry| entry.name == "b.bin")
                .unwrap()
                .bytes,
            0
        );
        assert_eq!(fs::read(fixture.root.join("b.bin")).unwrap().len(), 16_384);
    }

    #[cfg(unix)]
    #[test]
    fn deleting_from_cached_child_reconciles_shared_sibling_sessions() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        for refresh in [false, true] {
            cache::clear_all().unwrap();
            let fixture = AnalysisFixture::new();
            let a = fixture.root.join("a");
            let b = fixture.root.join("b");
            fs::create_dir(&a).unwrap();
            fs::create_dir(&b).unwrap();
            fs::write(a.join("owner.bin"), vec![7; 8192]).unwrap();
            fs::hard_link(a.join("owner.bin"), b.join("alias.bin")).unwrap();
            let parent = AnalysisService::analyze_with_progress(
                Some(fixture.root.to_string_lossy().into_owned()),
                true,
                |_| {},
            )
            .unwrap();
            let sibling = AnalysisService::analyze_with_progress(
                Some(b.to_string_lossy().into_owned()),
                false,
                |_| {},
            )
            .unwrap();
            let owner = AnalysisService::analyze_with_progress(
                Some(a.to_string_lossy().into_owned()),
                refresh,
                |_| {},
            )
            .unwrap();
            assert_eq!(sibling.total_bytes, 0);
            cache::clear_all().unwrap();
            let deleted = AnalysisService::delete_entry_permanently(
                owner.scan_id,
                owner.entries[0].path.clone(),
            )
            .unwrap();
            assert!(!deleted.requires_rescan);
            assert!(deleted.invalidated_scan_ids.contains(&parent.scan_id));
            let updated_sibling = deleted
                .updated_results
                .iter()
                .find(|result| result.scan_id == sibling.scan_id)
                .unwrap();
            assert_eq!(updated_sibling.total_bytes, parent.total_bytes);
            let remaining =
                resolve_entry_candidate(sibling.scan_id, &sibling.entries[0].path).unwrap();
            assert_eq!(remaining.expected_displayed_bytes, parent.total_bytes);
            let last = AnalysisService::delete_entry_permanently(
                sibling.scan_id,
                sibling.entries[0].path.clone(),
            )
            .unwrap();
            assert!(!last.requires_rescan);
            assert_eq!(
                last.updated_results
                    .iter()
                    .find(|result| result.scan_id == sibling.scan_id)
                    .unwrap()
                    .total_bytes,
                0
            );
        }
    }

    #[test]
    fn remainder_survives_ancestor_refresh_and_changed_totals() {
        use mangodisk_platform::{current_platform, Platform};
        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().unwrap();
        let fixture = AnalysisFixture::new();
        let root = fixture.root.join("analyzed");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let visible = nested.join("visible.bin");
        let other = nested.join("other.bin");
        fs::write(&visible, vec![1; 8192]).unwrap();
        fs::write(&other, vec![2; 4096]).unwrap();
        let expected_bytes = current_platform()
            .file_space_usage(&other, &fs::symlink_metadata(&other).unwrap())
            .allocated_bytes;
        let initial = AnalysisService::analyze_with_progress(
            Some(root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )
        .unwrap();
        let request = || AnalysisRemainderRequest {
            schema_version: AnalysisRemainderRequest::SCHEMA_VERSION,
            snapshot_id: None,
            scan_id: initial.scan_id,
            parent_path: fs::canonicalize(&nested)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            visible_paths: vec![fs::canonicalize(&visible)
                .unwrap()
                .to_string_lossy()
                .into_owned()],
            expected_bytes,
            offset: 0,
        };
        assert_eq!(
            AnalysisService::list_remainder(request())
                .unwrap()
                .total_count,
            1
        );
        // A later ancestor scan replaces index timestamps without changing this directory.
        std::thread::sleep(std::time::Duration::from_millis(2));
        AnalysisService::analyze_with_progress(
            Some(fixture.root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )
        .unwrap();
        let page = AnalysisService::list_remainder(request()).unwrap();
        assert_eq!(page.total_bytes, expected_bytes);
        assert_eq!(page.entries[0].name, "other.bin");
        fs::write(nested.join("added.bin"), vec![3; 4096]).unwrap();
        let changed = AnalysisService::list_remainder(request()).unwrap();
        assert_eq!(changed.total_count, 2);
        assert!(changed.total_bytes > expected_bytes);
        AnalysisService::analyze_with_progress(
            Some(fixture.root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )
        .unwrap();
        assert_eq!(
            AnalysisService::list_remainder(request())
                .unwrap()
                .total_count,
            2
        );
        let mut outside = request();
        outside.parent_path = std::env::temp_dir().to_string_lossy().into_owned();
        assert!(AnalysisService::list_remainder(outside).is_err());
        let mut unsupported = request();
        unsupported.schema_version += 1;
        assert!(AnalysisService::list_remainder(unsupported).is_err());
    }

    #[test]
    fn remainder_service_reconciles_real_scan_pages_and_recovers_after_rescan() {
        use mangodisk_platform::{current_platform, Platform};
        use std::collections::HashSet;

        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().unwrap();
        let fixture = AnalysisFixture::new();
        let root = fs::canonicalize(&fixture.root).unwrap();
        let allocated = |path: &std::path::Path| {
            current_platform()
                .file_space_usage(path, &fs::symlink_metadata(path).unwrap())
                .allocated_bytes
        };
        let visible = root.join("visible.bin");
        fs::write(&visible, vec![5; 131_072]).unwrap();
        let mut expected_bytes = 0;
        for index in 0..230 {
            let path = root.join(format!("small-{index:03}.bin"));
            fs::write(&path, vec![index as u8; 4_096]).unwrap();
            expected_bytes += allocated(&path);
        }
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        let nested_file = nested.join("data.bin");
        fs::write(&nested_file, vec![3; 8_192]).unwrap();
        expected_bytes += allocated(&nested_file);
        let initial = AnalysisService::analyze_with_progress(
            Some(root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )
        .unwrap();
        assert_eq!(initial.total_bytes, expected_bytes + allocated(&visible));
        let request = |scan_id, expected_bytes, offset| AnalysisRemainderRequest {
            schema_version: AnalysisRemainderRequest::SCHEMA_VERSION,
            snapshot_id: None,
            scan_id,
            parent_path: initial.root.clone(),
            visible_paths: vec![visible.to_string_lossy().into_owned()],
            expected_bytes,
            offset,
        };
        let first =
            AnalysisService::list_remainder(request(initial.scan_id, expected_bytes, 0)).unwrap();
        assert_eq!(first.total_bytes, expected_bytes);
        assert_eq!(first.total_count, 231);
        assert_eq!(first.entries.len(), 200);
        let mut next = request(initial.scan_id, expected_bytes, first.next_offset.unwrap());
        next.snapshot_id = Some(first.snapshot_id);
        let second = AnalysisService::list_remainder(next).unwrap();
        assert_eq!(second.entries.len(), 31);
        assert_eq!(second.next_offset, None);
        let entries = first
            .entries
            .into_iter()
            .chain(second.entries)
            .collect::<Vec<_>>();
        assert_eq!(
            entries.iter().map(|entry| entry.bytes).sum::<u64>(),
            expected_bytes
        );
        assert_eq!(
            entries
                .iter()
                .map(|entry| &entry.path)
                .collect::<HashSet<_>>()
                .len(),
            231
        );
        let directory = entries.iter().find(|entry| entry.is_directory).unwrap();
        let navigated =
            AnalysisService::analyze_with_progress(Some(directory.path.clone()), false, |_| {})
                .unwrap();
        assert_eq!(navigated.total_bytes, allocated(&nested_file));

        let added = root.join("added.bin");
        fs::write(&added, vec![7; 4_096]).unwrap();
        let mut latest_snapshot_id = first.snapshot_id;
        for _ in 0..2 {
            let changed =
                AnalysisService::list_remainder(request(initial.scan_id, expected_bytes, 0))
                    .unwrap();
            latest_snapshot_id = changed.snapshot_id;
            assert_eq!(changed.total_count, 232);
            assert_eq!(changed.total_bytes, expected_bytes + allocated(&added));
        }
        fs::remove_file(&added).unwrap();
        let mut next = request(initial.scan_id, expected_bytes, 232);
        next.snapshot_id = Some(latest_snapshot_id);
        let exhausted = AnalysisService::list_remainder(next).unwrap();
        assert!(exhausted.entries.is_empty());
        assert_eq!(exhausted.next_offset, None);
        fs::write(&added, vec![7; 4_096]).unwrap();
        let refreshed =
            AnalysisService::analyze_with_progress(Some(initial.root.clone()), true, |_| {})
                .unwrap();
        let recovered = AnalysisService::list_remainder(request(
            refreshed.scan_id,
            expected_bytes + allocated(&added),
            0,
        ))
        .unwrap();
        assert_eq!(recovered.total_bytes, expected_bytes + allocated(&added));
        assert_eq!(recovered.total_count, 232);
    }

    #[test]
    fn remainder_keeps_session_exclusions_after_scan_cache_eviction() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().unwrap();
        let fixture = AnalysisFixture::new();
        let root = fs::canonicalize(&fixture.root).unwrap();
        fs::write(root.join("listed.bin"), vec![1; 4096]).unwrap();
        let excluded = root.join("excluded");
        fs::create_dir(&excluded).unwrap();
        fs::write(excluded.join("data.bin"), vec![2; 4096]).unwrap();
        fs::write(root.join("ignored.bin"), vec![3; 4096]).unwrap();
        let result = AnalysisService::analyze_with_exclusions_progress(
            Some(root.to_string_lossy().into_owned()),
            true,
            ScanExclusionOptions {
                paths: vec![excluded.to_string_lossy().into_owned()],
                names: vec![mangodisk_platform::ScanNameExclusion {
                    name: "ignored.bin".into(),
                    kind: mangodisk_platform::ExcludedNameKind::File,
                }],
            },
            |_| {},
        )
        .unwrap();
        cache::clear_all().unwrap();
        let page = AnalysisService::list_remainder(AnalysisRemainderRequest {
            schema_version: AnalysisRemainderRequest::SCHEMA_VERSION,
            snapshot_id: None,
            scan_id: result.scan_id,
            parent_path: result.root,
            visible_paths: Vec::new(),
            expected_bytes: result.total_bytes,
            offset: 0,
        })
        .unwrap();
        assert_eq!(page.total_count, 1);
        assert_eq!(page.entries[0].name, "listed.bin");
        fs::create_dir(root.join("uncached-folder")).unwrap();
        let unavailable = AnalysisService::list_remainder(AnalysisRemainderRequest {
            schema_version: AnalysisRemainderRequest::SCHEMA_VERSION,
            snapshot_id: None,
            scan_id: result.scan_id,
            parent_path: root.to_string_lossy().into_owned(),
            visible_paths: Vec::new(),
            expected_bytes: result.total_bytes,
            offset: 0,
        });
        assert!(unavailable
            .unwrap_err()
            .to_string()
            .contains("index is unavailable"));
    }

    #[test]
    fn remainder_pagination_keeps_each_entry_once_when_directory_changes() {
        use std::collections::HashSet;
        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().unwrap();
        let fixture = AnalysisFixture::new();
        for index in 0..230 {
            fs::write(
                fixture.root.join(format!("small-{index:03}.bin")),
                vec![1; 4096],
            )
            .unwrap();
        }
        let result = AnalysisService::analyze_with_progress(
            Some(fixture.root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )
        .unwrap();
        let request = |offset| AnalysisRemainderRequest {
            schema_version: AnalysisRemainderRequest::SCHEMA_VERSION,
            snapshot_id: None,
            scan_id: result.scan_id,
            parent_path: result.root.clone(),
            visible_paths: Vec::new(),
            expected_bytes: result.total_bytes,
            offset,
        };
        let first = AnalysisService::list_remainder(request(0)).unwrap();
        fs::write(fixture.root.join("new-largest.bin"), vec![2; 8192]).unwrap();
        let mut next = request(first.next_offset.unwrap());
        next.snapshot_id = Some(first.snapshot_id);
        let second = AnalysisService::list_remainder(next).unwrap();
        let paths = first
            .entries
            .iter()
            .chain(&second.entries)
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            paths.iter().copied().collect::<HashSet<_>>().len(),
            paths.len(),
            "a file inserted before the cursor must not repeat a loaded entry"
        );
        assert_eq!(
            second.total_count, first.total_count,
            "one open details list must paginate a consistent set of entries"
        );
        fs::remove_file(fixture.root.join("small-000.bin")).unwrap();
        fs::write(fixture.root.join("small-001.bin"), vec![3; 16_384]).unwrap();
        let mut next = request(first.next_offset.unwrap());
        next.snapshot_id = Some(first.snapshot_id);
        let changed = AnalysisService::list_remainder(next).unwrap();
        assert_eq!(
            changed
                .entries
                .iter()
                .map(|entry| &entry.path)
                .collect::<Vec<_>>(),
            second
                .entries
                .iter()
                .map(|entry| &entry.path)
                .collect::<Vec<_>>()
        );
        assert_eq!(changed.total_bytes, first.total_bytes);
        let mut wrong_scope = request(0);
        wrong_scope.snapshot_id = Some(first.snapshot_id);
        wrong_scope.visible_paths.push(
            fixture
                .root
                .join("small-002.bin")
                .to_string_lossy()
                .into_owned(),
        );
        assert!(AnalysisService::list_remainder(wrong_scope).is_err());
        AnalysisService::release_remainder(first.snapshot_id).unwrap();
        AnalysisService::release_remainder(first.snapshot_id).unwrap();
        let reopened = AnalysisService::list_remainder(request(0)).unwrap();
        assert_ne!(reopened.snapshot_id, first.snapshot_id);
        assert!(reopened.total_bytes > first.total_bytes);
        assert!(reopened
            .entries
            .iter()
            .any(|entry| entry.name == "new-largest.bin"));
        for _ in 0..2 {
            AnalysisService::list_remainder(request(0)).unwrap();
        }
        let mut evicted = request(200);
        evicted.snapshot_id = Some(reopened.snapshot_id);
        let restarted = AnalysisService::list_remainder(evicted).unwrap();
        assert_ne!(restarted.snapshot_id, reopened.snapshot_id);
        assert_eq!(restarted.next_offset, Some(200));
        assert_eq!(restarted.entries[0].name, "small-001.bin");
    }

    #[test]
    fn recreated_original_path_requires_rescan_including_dangling_links() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("selected");
        assert!(!super::original_path_requires_rescan(&target));
        fs::create_dir(&target).unwrap();
        assert!(super::original_path_requires_rescan(&target));
        fs::remove_dir(&target).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path().join("absent"), &target).unwrap();
            assert!(super::original_path_requires_rescan(&target));
        }
    }

    #[test]
    fn analysis_service_deletes_the_current_direct_child_and_synchronizes_its_session() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().expect("the analysis cache should be clear before the service test");
        let fixture = AnalysisFixture::new();
        let path = fixture.file();
        fs::write(&path, vec![1_u8; 16 * 1024]).expect("the analysis candidate should be written");
        let progress_events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&progress_events);

        let initial = AnalysisService::analyze_with_progress(
            Some(fixture.root.to_string_lossy().into_owned()),
            true,
            move |progress| {
                captured_events
                    .lock()
                    .expect("the analysis progress fixture should remain available")
                    .push(progress)
            },
        )
        .expect("the analysis service should scan the isolated fixture");
        let selected_path = initial
            .entries
            .iter()
            .find(|entry| entry.name == "candidate.bin")
            .expect("the analysis result should contain the fixture file")
            .path
            .clone();
        assert_eq!(
            AnalysisService::resolve_open_target(initial.scan_id, selected_path.clone())
                .expect("the published analysis entry should resolve"),
            selected_path
        );
        assert!(
            !progress_events
                .lock()
                .expect("the analysis progress fixture should remain readable")
                .is_empty(),
            "the service adapter must forward traversal progress"
        );

        assert!(
            AnalysisService::delete_entry_permanently(
                initial.scan_id,
                fixture
                    .root
                    .join("fabricated.bin")
                    .to_string_lossy()
                    .into_owned(),
            )
            .is_err(),
            "the service must reject a path that was not published by the scan"
        );
        // Analysis deletion intentionally authorizes the current regular direct child even when
        // it changed after measurement. The permanent-delete boundary pins its physical identity
        // during execution; stale scan sizes are accounting facts rather than preflight gates.
        fs::write(&path, vec![2_u8; 32 * 1024])
            .expect("the analysis candidate should change after the scan");
        let deleted =
            AnalysisService::delete_entry_permanently(initial.scan_id, selected_path.clone())
                .expect("the current direct child should be deleted safely");

        assert_eq!(deleted.removed_path, selected_path);
        assert_eq!(deleted.removed_file_count, 1);
        assert!(!path.exists());
        assert!(
            AnalysisService::resolve_open_target(initial.scan_id, deleted.removed_path).is_err(),
            "a deleted entry must disappear from the authoritative result session"
        );
        cache::clear_all().expect("the analysis cache should be clear after the service test");
    }
    #[test]
    fn hierarchy_matches_independent_directory_results_and_stops_at_six_levels() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().unwrap();
        let fixture = AnalysisFixture::new();
        let deep = fixture.root.join("A/B/C/D/E/F/G");
        fs::create_dir_all(&deep).unwrap();
        fs::write(fixture.root.join("A/direct.bin"), vec![1_u8; 16 * 1024]).unwrap();
        fs::write(deep.join("nested.bin"), vec![2_u8; 32 * 1024]).unwrap();
        let result = AnalysisService::analyze_with_progress(
            Some(fixture.root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )
        .unwrap();
        let a = &result.directory_hierarchy[0];
        assert_eq!(a.name, "A");
        assert_eq!(a.bytes, result.entries[0].bytes);
        assert_eq!(a.file_count, 2);
        let b = &a.children[0];
        let c = &b.children[0];
        let d = &c.children[0];
        let e = &d.children[0];
        let f = &e.children[0];
        assert_eq!(f.name, "F");
        assert!(
            f.children.is_empty(),
            "the seventh directory level must not be transported"
        );
        assert!(
            a.bytes > b.bytes,
            "direct files remain part of the parent total"
        );
        let independent =
            AnalysisService::analyze_with_progress(Some(b.path.clone()), false, |_| {}).unwrap();
        assert_eq!(b.bytes, independent.total_bytes);
        assert_eq!(
            b.file_count,
            independent
                .entries
                .iter()
                .map(|entry| entry.file_count)
                .sum::<u64>()
        );
        cache::clear_all().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unchanged_directory_failure_retains_authoritative_sessions() {
        use std::os::unix::fs::PermissionsExt;
        let _operation_lock = crate::shared::operation::test_operation_lock();
        cache::clear_all().unwrap();
        let fixture = AnalysisFixture::new();
        let directory = fixture.root.join("directory");
        let locked = directory.join("locked");
        fs::create_dir_all(&locked).unwrap();
        fs::write(locked.join("retained"), b"keep").unwrap();
        let initial = AnalysisService::analyze_with_progress(
            Some(fixture.root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )
        .unwrap();
        let selected = initial
            .entries
            .iter()
            .find(|entry| entry.name == "directory")
            .unwrap()
            .path
            .clone();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o0)).unwrap();
        let result = AnalysisService::delete_entry_permanently(initial.scan_id, selected.clone());
        // Restore fixture access before assertions so a failed assertion cannot
        // leave an unreadable temporary tree behind.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        let error = result.expect_err("unreadable contents must stop deletion");
        assert_eq!(
            error.mutation_state(),
            mangodisk_platform::PlatformMutationState::NotAttempted
        );
        assert_eq!(
            error.reason(),
            Some(crate::shared::CoreErrorReason::AccessDeniedOrBusy)
        );
        assert!(locked.join("retained").exists());
        assert!(AnalysisService::resolve_open_target(initial.scan_id, selected).is_ok());
        cache::clear_all().unwrap();
    }
}
