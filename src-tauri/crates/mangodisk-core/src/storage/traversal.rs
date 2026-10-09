use crate::filesystem::ScanExclusionOptions;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

use crate::filesystem::metadata::{
    diagnostic_path, finalize_metadata_fingerprint, is_link_like, metadata_fingerprint_entry,
    modified_ms, native_path_string, now_ms, scan_entry_metadata, MetadataFingerprintEntry,
};
use crate::shared::operation::{
    CoordinatedOperationKind, OperationGuard, OPERATION_CANCELLED_ERROR,
};
use crate::shared::progress::ProgressTracker;
use crate::shared::{CoreError, CoreErrorReason, CoreResult, TraversalProgress, TraversalStage};
use crate::storage::analysis::{AnalysisResult, AnalysisScanMode};
use crate::storage::index::cache::{self, CacheReuseDecision, DirectoryAggregate, IndexedFile};
use crate::storage::large_files::{
    LargeFileScanMode, LargeFilesResult, LARGE_FILE_CANDIDATE_FLOOR_BYTES,
};
use crate::storage::StorageScanExclusions;
use mangodisk_platform::{
    current_platform, FastAnalysisFile, FastAnalysisQuery, FastAnalysisRecord,
    FastAnalysisScanError, FastAnalysisSummary, FilesystemChangeToken, LargeFileCandidateScanError,
    LargeFileCandidateSummary, Platform, ScanPurpose,
};

mod index_sink;
mod indexed_scopes;
#[cfg(any(windows, test))]
mod parallel_analysis;

use index_sink::{
    AnalysisCandidates, CompletedIndexSink, IndexRecordSink, ANALYSIS_FILES_PER_DIRECTORY,
};
#[cfg(any(debug_assertions, test))]
const ANALYSIS_ROOT_ENV: &str = "MANGODISK_ANALYSIS_ROOT";
#[derive(Debug, Default)]
pub(crate) struct AnalysisScanDiagnostics {
    pub(crate) cache_validation_ms: u64,
    pub(crate) traversal_ms: u64,
    pub(crate) fast_path: &'static str,
    pub(crate) strategy: &'static str,
    pub(crate) layout_page_count: u64,
    pub(crate) layout_entry_count: u64,
    pub(crate) directory_count: u64,
    pub(crate) candidate_count: u64,
    pub(crate) consumer_ms: u64,
    pub(crate) fallback_reason: Option<&'static str>,
    pub(crate) cache_write_ms: u64,
    pub(crate) result_build_ms: u64,
}

#[derive(Debug, Default)]
pub(crate) struct LargeFileScanDiagnostics {
    pub(crate) native_directory_reads: u64,
    pub(crate) candidate_discovery_ms: u64,
    pub(crate) validation_or_traversal_ms: u64,
    pub(crate) candidate_count: u64,
    pub(crate) candidate_backpressure_ms: u64,
    pub(crate) candidate_peak_in_flight: usize,
    pub(crate) candidate_strategy: &'static str,
    pub(crate) result_build_ms: u64,
    pub(crate) fast_path: &'static str,
    pub(crate) fallback_reason: Option<&'static str>,
}

impl LargeFileScanDiagnostics {
    fn accumulate(&mut self, root: &Self) {
        self.native_directory_reads += root.native_directory_reads;
        self.candidate_discovery_ms += root.candidate_discovery_ms;
        self.validation_or_traversal_ms += root.validation_or_traversal_ms;
        self.candidate_count += root.candidate_count;
        self.candidate_backpressure_ms += root.candidate_backpressure_ms;
        self.candidate_peak_in_flight = self
            .candidate_peak_in_flight
            .max(root.candidate_peak_in_flight);
        self.result_build_ms += root.result_build_ms;
        self.fast_path = if self.fast_path.is_empty() || self.fast_path == root.fast_path {
            root.fast_path
        } else {
            "mixed"
        };
        self.candidate_strategy = if self.candidate_strategy.is_empty()
            || self.candidate_strategy == root.candidate_strategy
        {
            root.candidate_strategy
        } else {
            "mixed"
        };
        self.fallback_reason = self.fallback_reason.or(root.fallback_reason);
    }
}

#[derive(Clone, Copy)]
enum TraversalKind {
    Analysis(AnalysisScanMode),
    LargeFiles,
}

impl TraversalKind {
    fn purpose(self) -> ScanPurpose {
        match self {
            Self::Analysis(_) => ScanPurpose::Analysis,
            Self::LargeFiles => ScanPurpose::LargeFiles,
        }
    }

    fn scan_mode(self) -> AnalysisScanMode {
        match self {
            Self::Analysis(mode) => mode,
            Self::LargeFiles => AnalysisScanMode::Standard,
        }
    }
}

struct AnalysisTraversal<'a> {
    scan_root: &'a Path,
    root_metadata: fs::Metadata,
    purpose: ScanPurpose,
    scan_mode: AnalysisScanMode,
    progress: &'a Arc<ProgressTracker>,
    scanned_at_ms: u64,
    sink: &'a mut IndexRecordSink,
    cancelled: &'a AtomicBool,
    scan_exclusions: Option<&'a StorageScanExclusions>,
}

struct LargeFileStreamValidation<'a> {
    root: &'a Path,
    root_metadata: fs::Metadata,
    minimum_bytes: u64,
    progress: &'a Arc<ProgressTracker>,
    cancelled: &'a AtomicBool,
    aggregate: DirectoryAggregate,
    valid_count: usize,
    report_candidate_progress: bool,
    exclusions: &'a StorageScanExclusions,
}

struct FastAnalysisStreamValidation<'a> {
    root: &'a Path,
    exclusions: &'a StorageScanExclusions,
    scanned_at_ms: u64,
    progress: &'a Arc<ProgressTracker>,
    cancelled: &'a AtomicBool,
    root_aggregate: Option<DirectoryAggregate>,
    directory_count: u64,
    candidate_count: u64,
}

struct FastAnalysisProgressValidation<'a> {
    root: &'a Path,
    progress: &'a Arc<ProgressTracker>,
    reported_file_count: u64,
    reported_bytes: u64,
}

struct CompletedFastAnalysis {
    aggregate: DirectoryAggregate,
    completed_sink: CompletedIndexSink,
    summary: FastAnalysisSummary,
}

enum FastAnalysisOutcome {
    Completed(Box<CompletedFastAnalysis>),
    Unsupported,
    LogicalTraversal,
    PlatformFailed { error: String },
}

struct CompletedFastLargeFileScan {
    aggregate: DirectoryAggregate,
    completed_sink: CompletedIndexSink,
    summary: LargeFileCandidateSummary,
    valid_count: usize,
}

enum FastLargeFileScanOutcome {
    Completed(Box<CompletedFastLargeFileScan>),
    Unsupported,
    PlatformFailed { error: String },
}

struct IndexPublicationOptions<'a> {
    purpose: ScanPurpose,
    refresh: bool,
    generation: u64,
    expected_mutation_revision: u64,
    configuration_fingerprint: [u8; 32],
    excluded_roots: &'a [PathBuf],
    excluded_names: &'a mangodisk_platform::NameExclusions,
}

pub(crate) struct StorageTraversal;

pub(crate) struct AnalysisTraversalSnapshot {
    pub(crate) result: AnalysisResult,
    pub(crate) diagnostics: AnalysisScanDiagnostics,
    pub(crate) exclusions: ScanExclusionOptions,
}

impl StorageTraversal {
    pub fn cancel_analysis() {
        OperationGuard::cancel(CoordinatedOperationKind::Analysis);
    }

    pub fn cancel_large_files() {
        OperationGuard::cancel(CoordinatedOperationKind::LargeFiles);
    }

    pub fn analyze_path_with_progress(
        path: Option<String>,
        refresh: bool,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<AnalysisResult> {
        Self::analyze_path_with_diagnostics(path, refresh, callback).map(|(result, _)| result)
    }

    pub(crate) fn analyze_path_with_diagnostics(
        path: Option<String>,
        refresh: bool,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<(AnalysisResult, AnalysisScanDiagnostics)> {
        Self::analyze_path_with_exclusions_diagnostics(path, refresh, Vec::new(), callback)
    }

    pub(crate) fn analyze_path_with_exclusions_diagnostics(
        path: Option<String>,
        refresh: bool,
        excluded_paths: impl Into<ScanExclusionOptions>,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<(AnalysisResult, AnalysisScanDiagnostics)> {
        Self::analyze_path_with_exclusions_snapshot(path, refresh, excluded_paths, callback)
            .map(|snapshot| (snapshot.result, snapshot.diagnostics))
    }

    pub(crate) fn analyze_path_with_exclusions_snapshot(
        path: Option<String>,
        refresh: bool,
        excluded_paths: impl Into<ScanExclusionOptions>,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<AnalysisTraversalSnapshot> {
        Self::analyze_path_with_mode_snapshot(
            path,
            refresh,
            excluded_paths,
            AnalysisScanMode::Standard,
            callback,
        )
    }

    pub(crate) fn analyze_path_with_mode_snapshot(
        path: Option<String>,
        refresh: bool,
        excluded_paths: impl Into<ScanExclusionOptions>,
        scan_mode: AnalysisScanMode,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<AnalysisTraversalSnapshot> {
        #[cfg(not(windows))]
        let requested_scan_mode = scan_mode;
        #[cfg(not(windows))]
        let scan_mode = AnalysisScanMode::Standard;
        let excluded_paths = excluded_paths.into();
        let operation = OperationGuard::start(CoordinatedOperationKind::Analysis)?;
        let started = Instant::now();
        let mut diagnostics = AnalysisScanDiagnostics::default();
        let root = resolve_analysis_root(path)?;
        let root = current_platform()
            .canonicalize_no_links(&root)
            .map_err(CoreError::from)?;
        if !root.is_dir() {
            return Err(CoreError::invalid_input(
                "the analysis root must be a directory",
            ));
        }
        #[cfg(not(windows))]
        if requested_scan_mode != scan_mode {
            log::info!("analysis_scan_mode_normalized operation_id={} root={} requested_mode={} applied_mode=standard reason=windows_only", operation.id(), diagnostic_path(&root), requested_scan_mode.as_str());
        }
        let exclusions = StorageScanExclusions::resolve_options(&root, &excluded_paths)?;
        // Deletion must use the same resolved paths as traversal, including system aliases.
        // Capture them now so later filesystem changes cannot rewrite the scan's safety policy.
        let resolved_options = ScanExclusionOptions {
            paths: exclusions
                .roots()
                .iter()
                .map(|path| native_path_string(path))
                .collect(),
            names: excluded_paths.names,
        };
        if exclusions.names().matches_path(&root) {
            log::info!("analysis_scan_root_excluded operation_id={} root={} reason=nameExclusion outcome=blocked", operation.id(), diagnostic_path(&root));
            return Err(
                CoreError::invalid_input("the analysis root is excluded by name")
                    .with_reason(crate::CoreErrorReason::AnalysisRootExcluded),
            );
        }
        if let Some(excluded_root) = exclusions
            .roots()
            .iter()
            .find(|excluded| current_platform().path_is_same_or_child(&root, excluded))
        {
            log::info!(
                "analysis_scan_root_excluded operation_id={} root={} excluded_root={} outcome=blocked",
                operation.id(),
                diagnostic_path(&root),
                diagnostic_path(excluded_root)
            );
            return Err(CoreError::invalid_input(
                "the analysis root is excluded by scan preferences",
            )
            .with_reason(crate::CoreErrorReason::AnalysisRootExcluded));
        }
        log::info!(
            "analysis_scan_started operation_id={} platform={} root={} scan_mode={} refresh={} requested_path_exclusions={} active_path_exclusions={} unavailable_exclusions={} out_of_scope_exclusions={}",
            operation.id(),
            current_platform().os_name(),
            diagnostic_path(&root),
            scan_mode.as_str(),
            refresh,
            exclusions.requested_count(),
            exclusions.active_count(),
            exclusions.unavailable_count(),
            exclusions.out_of_scope_count()
        );
        let progress = Arc::new(ProgressTracker::new(operation.id(), callback, 0));
        let cache_validation_started = Instant::now();
        let cache_decision = if refresh {
            CacheReuseDecision::Miss
        } else {
            cache::reuse_analysis_mode_decision(
                &root,
                scan_mode,
                exclusions.configuration_fingerprint(),
                &|| operation.cancelled().load(Ordering::Relaxed),
            )
            .map_err(traversal_core_error)?
        };
        diagnostics.cache_validation_ms = cache_validation_started.elapsed().as_millis() as u64;
        match cache_decision {
            CacheReuseDecision::Reusable => {
                if let Some(result) = cache::analysis_result(&root)? {
                    diagnostics.fast_path = "cache";
                    log::info!("analysis_scan_cache_reused operation_id={} root={} scan_mode={} total_bytes={} entry_count={} elapsed_ms={}", operation.id(), diagnostic_path(&root), scan_mode.as_str(), result.total_bytes, result.entries.len(), started.elapsed().as_millis());
                    operation.complete();
                    return Ok(AnalysisTraversalSnapshot {
                        result,
                        diagnostics,
                        exclusions: resolved_options,
                    });
                }
            }
            CacheReuseDecision::Miss => {}
        }
        progress.emit(TraversalStage::Analyzing, &root);
        let scanned_at_ms = now_ms();
        let cache_mutation_revision = cache::mutation_revision()?;
        let change_token = capture_filesystem_change_token(&root);
        let traversal_started = Instant::now();
        // Native streams return allocated bytes. Logical scans use metadata traversal
        // to avoid native allocation queries and preserve one consistent byte metric.
        let fast_scan = if scan_mode == AnalysisScanMode::Fast {
            Ok(FastAnalysisOutcome::LogicalTraversal)
        } else {
            stream_fast_analysis(
                &root,
                scanned_at_ms,
                change_token,
                &progress,
                operation.cancelled(),
                &exclusions,
            )
        };
        let (root_aggregate, completed_sink) = match fast_scan
            .map_err(|error| analysis_stream_core_error(&operation, error))?
        {
            FastAnalysisOutcome::Completed(scan) => {
                diagnostics.fast_path = "used";
                diagnostics.strategy = scan.summary.strategy;
                diagnostics.layout_page_count = scan.summary.page_count;
                diagnostics.layout_entry_count = scan.summary.entry_count;
                diagnostics.directory_count = scan.summary.directory_count;
                diagnostics.candidate_count = scan.summary.candidate_count;
                diagnostics.consumer_ms = scan.summary.consumer_elapsed_ms;
                log::info!(
                    "analysis_fast_scan_finished operation_id={} platform={} root={} strategy={} pages={} entries={} directories={} candidates={} logical_bytes={} allocated_bytes={} consumer_ms={} elapsed_ms={}",
                    operation.id(),
                    current_platform().os_name(),
                    diagnostic_path(&root),
                    scan.summary.strategy,
                    scan.summary.page_count,
                    scan.summary.entry_count,
                    scan.summary.directory_count,
                    scan.summary.candidate_count,
                    scan.summary.root_logical_bytes,
                    scan.summary.root_allocated_bytes,
                    scan.summary.consumer_elapsed_ms,
                    traversal_started.elapsed().as_millis()
                );
                (scan.aggregate, scan.completed_sink)
            }
            FastAnalysisOutcome::Unsupported | FastAnalysisOutcome::LogicalTraversal => {
                diagnostics.fast_path = if scan_mode == AnalysisScanMode::Fast {
                    "logical_traversal"
                } else {
                    "fallback"
                };
                diagnostics.fallback_reason =
                    (scan_mode == AnalysisScanMode::Standard).then_some("unsupported");
                traverse_memory_only(
                    &root,
                    TraversalKind::Analysis(scan_mode),
                    scanned_at_ms,
                    change_token,
                    &progress,
                    operation.cancelled(),
                    Some(&exclusions),
                )
                .map_err(traversal_core_error)?
            }
            FastAnalysisOutcome::PlatformFailed { error } => {
                diagnostics.fast_path = "fallback";
                diagnostics.fallback_reason = Some("fastPathFailed");
                log::warn!(
                    "analysis_scan_fallback operation_id={} platform={} root={} reason=fast_scan_failed error={}",
                    operation.id(),
                    current_platform().os_name(),
                    diagnostic_path(&root),
                    mangodisk_platform::diagnostics::text(&error)
                );
                traverse_memory_only(
                    &root,
                    TraversalKind::Analysis(scan_mode),
                    scanned_at_ms,
                    change_token,
                    &progress,
                    operation.cancelled(),
                    Some(&exclusions),
                )
                .map_err(traversal_core_error)?
            }
        };
        diagnostics.traversal_ms = traversal_started.elapsed().as_millis() as u64;
        // A scan may finish before the progress-throttling window. Force the final counters here so
        // adapters receive complete state and benchmarks do not record a fast scan as zero files
        // and zero bytes.
        progress.finish(TraversalStage::Analyzing, &root);
        operation.ensure_not_cancelled()?;
        let result_build_started = Instant::now();
        #[cfg(windows)]
        let result_workers = parallel_analysis::worker_count(&root);
        #[cfg(not(windows))]
        let result_workers = 1;
        let result = cache::analysis_result_from_snapshot(
            &root,
            root_aggregate,
            &completed_sink.directories,
            &completed_sink.files,
            exclusions.roots(),
            exclusions.names(),
            result_workers,
        )?;
        diagnostics.result_build_ms = result_build_started.elapsed().as_millis() as u64;
        // Result assembly can outlast traversal for a large direct-file directory.
        // A cancellation during that stage must not publish a completed snapshot.
        operation.ensure_not_cancelled()?;
        let cache_write_started = Instant::now();
        publish_completed_index(
            &root,
            root_aggregate,
            completed_sink,
            IndexPublicationOptions {
                excluded_names: exclusions.names(),
                purpose: ScanPurpose::Analysis,
                refresh,
                generation: operation.id(),
                expected_mutation_revision: cache_mutation_revision,
                configuration_fingerprint: exclusions.configuration_fingerprint(),
                excluded_roots: exclusions.roots(),
            },
        )?;
        diagnostics.cache_write_ms = cache_write_started.elapsed().as_millis() as u64;
        log::info!(
            "analysis_scan_finished operation_id={} root={} scan_mode={} size_metric={} total_bytes={} files={} entry_count={} skipped_count={} active_path_exclusions={} traversal_ms={} result_build_ms={} cache_write_ms={} elapsed_ms={}",
            operation.id(),
            diagnostic_path(&root),
            scan_mode.as_str(),
            if scan_mode == AnalysisScanMode::Fast { "logical" } else { "allocated" },
            result.total_bytes,
            root_aggregate.file_count,
            result.entries.len(),
            result.skipped_count,
            exclusions.active_count(),
            diagnostics.traversal_ms,
            diagnostics.result_build_ms,
            diagnostics.cache_write_ms,
            started.elapsed().as_millis()
        );
        operation.complete();
        Ok(AnalysisTraversalSnapshot {
            result,
            diagnostics,
            exclusions: resolved_options,
        })
    }

    pub fn find_large_files_with_progress(
        roots: Vec<String>,
        minimum_bytes: u64,
        scan_mode: LargeFileScanMode,
        excluded_paths: impl Into<ScanExclusionOptions>,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<LargeFilesResult> {
        let excluded_paths = excluded_paths.into();
        Self::find_large_files_with_diagnostics(
            roots,
            minimum_bytes,
            scan_mode,
            excluded_paths,
            callback,
        )
        .map(|(result, _)| result)
    }

    pub(crate) fn find_large_files_with_diagnostics(
        roots: Vec<String>,
        minimum_bytes: u64,
        scan_mode: LargeFileScanMode,
        excluded_paths: impl Into<ScanExclusionOptions>,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<(LargeFilesResult, LargeFileScanDiagnostics)> {
        let excluded_paths = excluded_paths.into();
        let operation = OperationGuard::start(CoordinatedOperationKind::LargeFiles)?;
        let started = Instant::now();
        let roots = normalize_large_file_roots(roots)?;
        if scan_mode == LargeFileScanMode::Quick
            && roots.len() > 1
            && !current_platform().fast_large_file_candidates_are_complete()
        {
            let result =
                indexed_scopes::scan(&roots, minimum_bytes, &excluded_paths, &operation, callback)?;
            operation.complete();
            return Ok(result);
        }
        let root_count = roots.len() as u64;
        let callback = Arc::new(callback);
        let scanned_at_ms = now_ms();
        let mut combined = LargeFileScanDiagnostics::default();
        let mut retained_entries = Vec::new();
        let mut skipped_count = 0_u64;
        let mut scanned_items = 0_u64;
        let mut scanned_bytes = 0_u64;
        // One coordinated operation owns every root. Root-local trackers let a failed
        // native attempt reset its observations without erasing completed roots.
        for (root_index, root) in roots.iter().enumerate() {
            if operation.cancelled().load(Ordering::Relaxed) {
                return Err(CoreError::operation_cancelled());
            }
            let callback = Arc::clone(&callback);
            let progress = Arc::new(ProgressTracker::new(
                operation.id(),
                move |mut event: TraversalProgress| {
                    event.items_scanned = event.items_scanned.saturating_add(scanned_items);
                    event.bytes_scanned = event.bytes_scanned.saturating_add(scanned_bytes);
                    event.completed_steps += root_index as u64;
                    event.total_steps = root_count;
                    event.elapsed_ms = started.elapsed().as_millis() as u64;
                    callback(event);
                },
                1,
            ));
            let root_log = diagnostic_path(root);
            let mut diagnostics = LargeFileScanDiagnostics::default();
            let mut exclusions = StorageScanExclusions::resolve_options(root, &excluded_paths)?;
            let delegated_roots = exclusions.delegate_selected_descendants(root, &roots);

            let result_minimum_bytes = minimum_bytes.max(LARGE_FILE_CANDIDATE_FLOOR_BYTES);
            log::info!(
                "large_file_scan_started delegated_roots={delegated_roots} root_index={root_index} root_count={root_count} operation_id={} platform={} root={} mode={} requested_minimum_bytes={} result_minimum_bytes={} candidate_floor_bytes={} requested_path_exclusions={} active_path_exclusions={} unavailable_exclusions={} out_of_scope_exclusions={}",
                operation.id(),
                current_platform().os_name(),
                diagnostic_path(root),
                scan_mode.as_str(),
                minimum_bytes,
                result_minimum_bytes,
                LARGE_FILE_CANDIDATE_FLOOR_BYTES,
                exclusions.requested_count(),
                exclusions.active_count(),
                exclusions.unavailable_count(),
                exclusions.out_of_scope_count()
            );

            if exclusions.matches(root) {
                log::info!(
                    "large_file_scan_root_excluded operation_id={} root={} outcome=pruned",
                    operation.id(),
                    diagnostic_path(root)
                );
                diagnostics.candidate_strategy = "excluded_root";
                diagnostics.fast_path = "excluded_root";
                combined.accumulate(&diagnostics);
                progress.complete_step(TraversalStage::Analyzing, root, 0);
                continue;
            }

            progress.emit(TraversalStage::Analyzing, root);
            let candidate_started = Instant::now();
            let use_authoritative_candidate_source =
                current_platform().fast_large_file_candidates_are_complete();
            let scan =
                if scan_mode == LargeFileScanMode::Quick || use_authoritative_candidate_source {
                    stream_fast_large_files(
                        root,
                        LARGE_FILE_CANDIDATE_FLOOR_BYTES,
                        scanned_at_ms,
                        None,
                        &progress,
                        operation.cancelled(),
                        &exclusions,
                    )
                    .map_err(traversal_core_error)?
                } else {
                    stream_complete_large_files(
                        root,
                        LARGE_FILE_CANDIDATE_FLOOR_BYTES,
                        scanned_at_ms,
                        &progress,
                        operation.cancelled(),
                        &exclusions,
                    )
                    .map_err(|error| analysis_stream_core_error(&operation, error))?
                };
            diagnostics.candidate_discovery_ms = candidate_started.elapsed().as_millis() as u64;

            let (root_aggregate, completed_sink) = match scan {
                FastLargeFileScanOutcome::Completed(scan) => {
                    diagnostics.native_directory_reads = scan.summary.native_directory_reads;
                    diagnostics.fast_path = "used";
                    diagnostics.validation_or_traversal_ms = scan.summary.consumer_elapsed_ms;
                    diagnostics.candidate_count = scan.summary.candidate_count;
                    diagnostics.candidate_backpressure_ms = scan.summary.producer_backpressure_ms;
                    diagnostics.candidate_peak_in_flight = scan.summary.peak_in_flight_candidates;
                    diagnostics.candidate_strategy = scan.summary.strategy;
                    log::info!(
                        "large_file_candidate_scan_finished native_directory_reads={} root={root_log} operation_id={} platform={} mode={} strategy={} candidate_count={} valid_count={} skipped_count={} producer_backpressure_ms={} peak_in_flight_candidates={} elapsed_ms={}",
                        diagnostics.native_directory_reads,
                        operation.id(),
                        current_platform().os_name(),
                        scan_mode.as_str(),
                        scan.summary.strategy,
                        scan.summary.candidate_count,
                        scan.valid_count,
                        scan.aggregate.skipped_count,
                        scan.summary.producer_backpressure_ms,
                        scan.summary.peak_in_flight_candidates,
                        diagnostics.candidate_discovery_ms
                    );
                    (scan.aggregate, scan.completed_sink)
                }
                FastLargeFileScanOutcome::Unsupported
                | FastLargeFileScanOutcome::PlatformFailed { .. }
                    if scan_mode == LargeFileScanMode::Quick =>
                {
                    let reason = match &scan {
                        FastLargeFileScanOutcome::Unsupported => "unsupported",
                        FastLargeFileScanOutcome::PlatformFailed { .. } => "provider_failed",
                        FastLargeFileScanOutcome::Completed(_) => unreachable!(),
                    };
                    if let FastLargeFileScanOutcome::PlatformFailed { error } = &scan {
                        log::warn!(
                            "large_file_quick_scan_unavailable root={root_log} operation_id={} platform={} reason={} error={}",
                            operation.id(),
                            current_platform().os_name(),
                            reason,
                            mangodisk_platform::diagnostics::text(&error)
                        );
                    } else {
                        log::info!(
                            "large_file_quick_scan_unavailable root={root_log} operation_id={} platform={} reason={}",
                            operation.id(),
                            current_platform().os_name(),
                            reason
                        );
                    }
                    return Err(
                        CoreError::operation_failed("quick large-file scan unavailable")
                            .with_reason(CoreErrorReason::QuickScanUnavailable),
                    );
                }
                FastLargeFileScanOutcome::Unsupported
                | FastLargeFileScanOutcome::PlatformFailed { .. } => {
                    let reason = match &scan {
                        FastLargeFileScanOutcome::Unsupported => "native_unsupported",
                        FastLargeFileScanOutcome::PlatformFailed { .. } => "native_failed",
                        FastLargeFileScanOutcome::Completed(_) => unreachable!(),
                    };
                    diagnostics.fallback_reason = Some(reason);
                    if let FastLargeFileScanOutcome::PlatformFailed { error } = &scan {
                        log::warn!(
                            "large_file_complete_scan_fallback root={root_log} operation_id={} platform={} reason={} error={}",
                            operation.id(),
                            current_platform().os_name(),
                            reason,
                            mangodisk_platform::diagnostics::text(&error)
                        );
                    }
                    progress.reset_scan_observations_for_retry();
                    let fallback_started = Instant::now();
                    let fallback = traverse_memory_only(
                        root,
                        TraversalKind::LargeFiles,
                        scanned_at_ms,
                        None,
                        &progress,
                        operation.cancelled(),
                        Some(&exclusions),
                    )
                    .map_err(traversal_core_error)?;
                    diagnostics.validation_or_traversal_ms =
                        fallback_started.elapsed().as_millis() as u64;
                    diagnostics.fast_path = "genericTraversal";
                    diagnostics.candidate_strategy = "generic_read_dir_candidates";
                    fallback
                }
            };

            let result_build_started = Instant::now();
            retained_entries.extend(cache::large_file_entries_from_snapshot(
                root,
                &completed_sink.files,
            ));
            skipped_count = skipped_count.saturating_add(root_aggregate.skipped_count);
            diagnostics.result_build_ms = result_build_started.elapsed().as_millis() as u64;
            combined.accumulate(&diagnostics);
            let (items, bytes) = progress.scan_observation_counts();
            scanned_items = scanned_items.saturating_add(items);
            scanned_bytes = scanned_bytes.saturating_add(bytes);
            progress.complete_step(TraversalStage::Analyzing, root, 0);
            progress.finish(TraversalStage::Analyzing, root);
        }
        if operation.cancelled().load(Ordering::Relaxed) {
            return Err(CoreError::operation_cancelled());
        }
        let result_build_started = Instant::now();
        let result = LargeFilesResult::from_retained_entries(
            roots
                .iter()
                .map(|root| current_platform().display_path(root))
                .collect(),
            scanned_at_ms,
            scan_mode,
            minimum_bytes,
            skipped_count,
            retained_entries,
        );
        combined.result_build_ms += result_build_started.elapsed().as_millis() as u64;
        log::info!(
            "large_file_scan_finished operation_id={} platform={} root_count={} mode={} strategy={} total_count={} returned_count={} skipped_count={} elapsed_ms={}",
            operation.id(), current_platform().os_name(), root_count, scan_mode.as_str(),
            combined.candidate_strategy, result.total_count, result.returned_count,
            result.skipped_count, started.elapsed().as_millis()
        );
        operation.complete();
        Ok((result, combined))
    }
}

/// Preserve explicit roots across mount and exclusion boundaries. Descendant scopes are
/// delegated during traversal, so lexical ancestry alone never removes a selected volume.
fn normalize_large_file_roots(roots: Vec<String>) -> CoreResult<Vec<PathBuf>> {
    if roots.is_empty() {
        return Err(CoreError::invalid_input(
            "at least one large-file scan root is required",
        ));
    }
    let mut normalized = Vec::<PathBuf>::new();
    for value in roots {
        let requested = Path::new(&value);
        if value.trim().is_empty() || !requested.is_absolute() {
            return Err(CoreError::invalid_input(
                "large-file scan roots must be absolute directories",
            ));
        }
        let root = current_platform()
            .canonicalize_no_links(requested)
            .map_err(|error| {
                CoreError::invalid_input(format!(
                    "invalid large-file scan root path={} error={}",
                    diagnostic_path(requested),
                    mangodisk_platform::diagnostics::text(&error)
                ))
            })?;
        if !root.is_dir() {
            return Err(CoreError::invalid_input(format!(
                "large-file scan root is not a directory: {}",
                diagnostic_path(&root)
            )));
        }
        if normalized
            .iter()
            .any(|selected| current_platform().paths_equal(&root, selected))
        {
            continue;
        }
        normalized.push(root);
    }
    normalized.sort();
    Ok(normalized)
}

/// Preserves cancellation from the historical string-based fallback traversal. Native fast-path
/// failures use `AnalysisStreamError` below and never rely on diagnostic text for control flow.
fn traversal_core_error(error: String) -> CoreError {
    if error == OPERATION_CANCELLED_ERROR {
        CoreError::operation_cancelled()
    } else {
        CoreError::operation_failed(error)
    }
}

fn analysis_stream_core_error(operation: &OperationGuard, error: AnalysisStreamError) -> CoreError {
    match error {
        AnalysisStreamError::Cancelled => CoreError::operation_cancelled(),
        AnalysisStreamError::ResourcesReleasing => {
            operation.defer();
            CoreError::scan_resources_releasing()
        }
        AnalysisStreamError::Failed(diagnostic) => CoreError::operation_failed(diagnostic),
    }
}

fn resolve_analysis_root(path: Option<String>) -> Result<PathBuf, String> {
    let requested = path
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| current_platform().system_volume_path());

    #[cfg(debug_assertions)]
    {
        let system_root = current_platform().system_volume_path();
        if requested == system_root {
            if let Ok(value) = std::env::var(ANALYSIS_ROOT_ENV) {
                let value = value.trim();
                if !value.is_empty() {
                    let development_root = PathBuf::from(value);
                    // Accept only absolute paths so different shell working directories cannot
                    // redirect the scan. Reject invalid configuration instead of silently scanning
                    // the entire volume when a developer expects a constrained root.
                    if !development_root.is_absolute() {
                        return Err(format!(
                            "development environment variable {ANALYSIS_ROOT_ENV} must contain an absolute path"
                        ));
                    }
                    log::info!(
                        "development_analysis_root_applied requested_root={} effective_root={}",
                        diagnostic_path(&requested),
                        diagnostic_path(&development_root)
                    );
                    return Ok(development_root);
                }
            }
        }
    }

    Ok(requested)
}

const ANALYSIS_FILE_BATCH_THRESHOLD: usize = 512;

struct AnalysisFileEntry {
    path: PathBuf,
    metadata: fs::Metadata,
}

struct AnalysisDirectoryRead {
    aggregate: DirectoryAggregate,
    fingerprint_entries: Vec<MetadataFingerprintEntry>,
    children: Vec<(PathBuf, fs::Metadata)>,
    files: Vec<AnalysisFileEntry>,
    analysis_files: AnalysisCandidates,
}

fn measure_analysis_directory(
    path: &Path,
    traversal: &mut AnalysisTraversal<'_>,
) -> Result<DirectoryAggregate, String> {
    let mut read = read_analysis_directory(path, traversal)?;
    for (child_path, metadata) in std::mem::take(&mut read.children) {
        let child = measure_analysis_directory(&child_path, traversal)?;
        add_analysis_child(&mut read, &child_path, &metadata, child, traversal.purpose);
    }
    finish_analysis_directory(
        path,
        read,
        traversal.purpose,
        traversal.scan_root,
        traversal.sink,
    )
}

fn add_analysis_child(
    read: &mut AnalysisDirectoryRead,
    path: &Path,
    metadata: &fs::Metadata,
    child: DirectoryAggregate,
    purpose: ScanPurpose,
) {
    read.aggregate.bytes += child.bytes;
    read.aggregate.logical_bytes += child.logical_bytes;
    read.aggregate.file_count += child.file_count;
    read.aggregate.skipped_count += child.skipped_count;
    if purpose != ScanPurpose::LargeFiles {
        if let Some(entry) = metadata_fingerprint_entry(path, metadata, child.fingerprint) {
            read.fingerprint_entries.push(entry);
        } else {
            read.aggregate.skipped_count += 1;
        }
    }
}

fn finish_analysis_directory(
    path: &Path,
    mut read: AnalysisDirectoryRead,
    purpose: ScanPurpose,
    scan_root: &Path,
    sink: &mut IndexRecordSink,
) -> Result<DirectoryAggregate, String> {
    if !read.files.is_empty() {
        return Err("analysis directory has unfinished file batches".into());
    }
    if purpose != ScanPurpose::LargeFiles && read.aggregate.skipped_count == 0 {
        read.aggregate.fingerprint = Some(finalize_metadata_fingerprint(read.fingerprint_entries));
    }
    for mut file in read.analysis_files.into_files() {
        file.parent_file_count = read.aggregate.direct_file_count;
        sink.push_analysis_file(file);
    }
    if purpose != ScanPurpose::LargeFiles || path == scan_root {
        sink.push_directory(path.to_path_buf(), read.aggregate)?;
    }
    Ok(read.aggregate)
}

fn read_analysis_directory(
    path: &Path,
    traversal: &mut AnalysisTraversal<'_>,
) -> Result<AnalysisDirectoryRead, String> {
    read_analysis_directory_with_file_queries(path, traversal, false)
}

fn read_analysis_directory_with_file_queries(
    path: &Path,
    traversal: &mut AnalysisTraversal<'_>,
    defer_large_directories: bool,
) -> Result<AnalysisDirectoryRead, String> {
    if traversal.cancelled.load(Ordering::Relaxed) {
        return Err(OPERATION_CANCELLED_ERROR.to_string());
    }
    traversal
        .progress
        .visit_directory(traversal_stage(traversal.purpose), path);
    let mut aggregate = DirectoryAggregate {
        scanned_at_ms: traversal.scanned_at_ms,
        scan_mode: traversal.scan_mode,
        ..DirectoryAggregate::default()
    };
    // Directory work can wait behind other entries or workers. Revalidate at execution time
    // so earlier enumeration attributes never authorize following a replacement junction.
    let live = fs::symlink_metadata(path).ok();
    if !live.as_ref().is_some_and(|metadata| {
        metadata.is_dir()
            && !is_link_like(metadata)
            && current_platform().is_same_filesystem(&traversal.root_metadata, metadata)
    }) {
        aggregate.skipped_count = 1;
        return Ok(AnalysisDirectoryRead {
            aggregate,
            fingerprint_entries: Vec::new(),
            children: Vec::new(),
            files: Vec::new(),
            analysis_files: AnalysisCandidates::new(0),
        });
    }
    let Ok(entries) = fs::read_dir(path) else {
        aggregate.skipped_count = 1;
        return Ok(AnalysisDirectoryRead {
            aggregate,
            fingerprint_entries: Vec::new(),
            children: Vec::new(),
            files: Vec::new(),
            analysis_files: AnalysisCandidates::new(0),
        });
    };
    let mut fingerprint_entries = Vec::new();
    let mut children = Vec::new();
    let mut files = Vec::new();
    aggregate.retained_file_limit = ANALYSIS_FILES_PER_DIRECTORY;
    let mut analysis_files = AnalysisCandidates::new(ANALYSIS_FILES_PER_DIRECTORY);

    for entry in entries {
        if traversal.cancelled.load(Ordering::Relaxed) {
            return Err(OPERATION_CANCELLED_ERROR.to_string());
        }
        let Ok(entry) = entry else {
            aggregate.skipped_count += 1;
            continue;
        };
        let child_path = entry.path();
        if traversal
            .scan_exclusions
            .is_some_and(|exclusions| exclusions.matches(&child_path))
        {
            continue;
        }
        if current_platform()
            .should_skip(&child_path, traversal.scan_root, traversal.purpose)
            .is_some()
        {
            aggregate.skipped_count += 1;
            continue;
        }
        let Ok(metadata) = scan_entry_metadata(&entry, &child_path) else {
            aggregate.skipped_count += 1;
            continue;
        };
        if is_link_like(&metadata) {
            aggregate.skipped_count += 1;
            continue;
        }
        if !current_platform().is_same_filesystem(&traversal.root_metadata, &metadata) {
            // Mounted volumes can appear below the selected root (for example a Parallels SMB
            // share under /Volumes). Descending would count the guest files in addition to the
            // host-side virtual disk image and could report more data than the selected disk owns.
            log::info!(
                "storage_mount_boundary_skipped platform={} root={} path={}",
                current_platform().os_name(),
                diagnostic_path(traversal.scan_root),
                diagnostic_path(&child_path)
            );
            aggregate.skipped_count += 1;
            continue;
        }
        if metadata.is_dir() {
            children.push((child_path, metadata));
        } else if metadata.is_file() {
            let file = AnalysisFileEntry {
                path: child_path,
                metadata,
            };
            if defer_large_directories {
                files.push(file);
            } else {
                measure_analysis_file(
                    file,
                    traversal,
                    &mut aggregate,
                    &mut fingerprint_entries,
                    &mut analysis_files,
                )?;
            }
        } else {
            aggregate.skipped_count += 1;
        }
    }
    // Small directories stay one task. Large direct-file sets can share the same bounded
    // worker pool as directory work instead of serializing behind a single directory reader.
    if defer_large_directories && files.len() < ANALYSIS_FILE_BATCH_THRESHOLD {
        for file in std::mem::take(&mut files) {
            measure_analysis_file(
                file,
                traversal,
                &mut aggregate,
                &mut fingerprint_entries,
                &mut analysis_files,
            )?;
        }
    }
    Ok(AnalysisDirectoryRead {
        aggregate,
        fingerprint_entries,
        children,
        files,
        analysis_files,
    })
}

fn measure_analysis_file(
    file: AnalysisFileEntry,
    traversal: &mut AnalysisTraversal<'_>,
    aggregate: &mut DirectoryAggregate,
    fingerprint_entries: &mut Vec<MetadataFingerprintEntry>,
    analysis_files: &mut AnalysisCandidates,
) -> Result<(), String> {
    let AnalysisFileEntry {
        path: child_path,
        metadata,
    } = file;
    let usage = traversal.scan_mode.file_usage(&child_path, &metadata);
    if traversal.purpose == ScanPurpose::Analysis {
        if let Some(identity) = current_platform().hard_link_identity(&metadata) {
            traversal.sink.push_hard_link(
                child_path.clone(),
                identity,
                IndexedFile {
                    shared_identity: None,
                    bytes: usage.allocated_bytes,
                    logical_bytes: usage.logical_bytes,
                    modified_at_ms: modified_ms(&metadata),
                },
            );
        }
    }
    traversal.progress.visit_file(
        traversal_stage(traversal.purpose),
        &child_path,
        usage.allocated_bytes,
    );
    aggregate.bytes += usage.allocated_bytes;
    aggregate.logical_bytes += usage.logical_bytes;
    aggregate.file_count += 1;
    aggregate.direct_file_count += u64::from(usage.allocated_bytes > 0);
    if traversal.purpose != ScanPurpose::LargeFiles {
        if let Some(entry) = metadata_fingerprint_entry(&child_path, &metadata, None) {
            fingerprint_entries.push(entry);
        } else {
            aggregate.skipped_count += 1;
        }
    }
    if traversal.purpose == ScanPurpose::Analysis
        && usage.allocated_bytes > 0
        && current_platform().hard_link_identity(&metadata).is_none()
        && analysis_files.would_retain(usage.allocated_bytes, &child_path)
        && current_platform()
            .should_skip(&child_path, traversal.scan_root, ScanPurpose::Analysis)
            .is_none()
    {
        analysis_files.push(FastAnalysisFile {
            parent_file_count: 0,
            path: child_path.clone(),
            allocated_bytes: usage.allocated_bytes,
            logical_bytes: usage.logical_bytes,
            modified_at_ms: modified_ms(&metadata),
        });
    }
    // Analysis includes system-owned allocation in directory totals, while cached file
    // rows still use the stricter large-file safety boundary. This keeps later analysis
    // navigation and destructive cache updates from exposing protected file targets.
    if usage.allocated_bytes >= LARGE_FILE_CANDIDATE_FLOOR_BYTES
        && current_platform()
            .should_skip(&child_path, traversal.scan_root, ScanPurpose::LargeFiles)
            .is_none()
    {
        traversal.sink.push_large_file(
            child_path,
            IndexedFile {
                shared_identity: None,
                bytes: usage.allocated_bytes,
                logical_bytes: usage.logical_bytes,
                modified_at_ms: modified_ms(&metadata),
            },
        )?;
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn read_analysis_file_batch(
    directory: &Path,
    files: Vec<AnalysisFileEntry>,
    traversal: &mut AnalysisTraversal<'_>,
) -> Result<AnalysisDirectoryRead, String> {
    let mut read = AnalysisDirectoryRead {
        aggregate: DirectoryAggregate {
            scanned_at_ms: traversal.scanned_at_ms,
            scan_mode: traversal.scan_mode,
            ..DirectoryAggregate::default()
        },
        fingerprint_entries: Vec::new(),
        children: Vec::new(),
        files: Vec::new(),
        analysis_files: AnalysisCandidates::new(ANALYSIS_FILES_PER_DIRECTORY),
    };
    let live_directory = fs::symlink_metadata(directory).ok();
    if !live_directory.as_ref().is_some_and(|metadata| {
        metadata.is_dir()
            && !is_link_like(metadata)
            && current_platform().is_same_filesystem(&traversal.root_metadata, metadata)
    }) {
        read.aggregate.skipped_count = 1;
        return Ok(read);
    }
    for file in files {
        if traversal.cancelled.load(Ordering::Relaxed) {
            return Err(OPERATION_CANCELLED_ERROR.to_string());
        }
        // File work also waits in the queue. Keep a live no-follow check here, even though it
        // costs an extra query, rather than following a file replaced by a link while queued.
        let Ok(metadata) = fs::symlink_metadata(&file.path) else {
            read.aggregate.skipped_count += 1;
            continue;
        };
        if !metadata.is_file() || is_link_like(&metadata) {
            read.aggregate.skipped_count += 1;
            continue;
        }
        measure_analysis_file(
            AnalysisFileEntry {
                path: file.path,
                metadata,
            },
            traversal,
            &mut read.aggregate,
            &mut read.fingerprint_entries,
            &mut read.analysis_files,
        )?;
    }
    Ok(read)
}

impl<'a> FastAnalysisStreamValidation<'a> {
    fn new(
        root: &'a Path,
        scanned_at_ms: u64,
        progress: &'a Arc<ProgressTracker>,
        cancelled: &'a AtomicBool,
        exclusions: &'a StorageScanExclusions,
    ) -> Self {
        Self {
            root,
            exclusions,
            scanned_at_ms,
            progress,
            cancelled,
            root_aggregate: None,
            directory_count: 0,
            candidate_count: 0,
        }
    }

    fn consume(
        &mut self,
        record: FastAnalysisRecord,
        sink: &mut IndexRecordSink,
    ) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(OPERATION_CANCELLED_ERROR.to_string());
        }
        match record {
            FastAnalysisRecord::AnalysisFile(file) => {
                if !file.path.starts_with(self.root) || file.path == self.root {
                    return Err("analysis file record is outside the scan root".to_string());
                }
                if !self.exclusions.matches(&file.path)
                    && current_platform()
                        .should_skip(&file.path, self.root, ScanPurpose::Analysis)
                        .is_none()
                {
                    sink.push_analysis_file(file);
                }
                Ok(())
            }
            FastAnalysisRecord::HardLinkedFile {
                path,
                identity,
                logical_bytes,
                allocated_bytes,
                modified_at_ms,
            } => {
                if !path.starts_with(self.root) || path == self.root {
                    return Err("hard-link record is outside the scan root".to_string());
                }
                if !self.exclusions.matches(&path) {
                    sink.push_hard_link(
                        path,
                        identity,
                        IndexedFile {
                            shared_identity: None,
                            bytes: allocated_bytes,
                            logical_bytes,
                            modified_at_ms,
                        },
                    );
                }
                Ok(())
            }
            FastAnalysisRecord::Directory {
                retained_file_limit,
                path,
                logical_bytes,
                allocated_bytes,
                file_count,
                direct_file_count,
                skipped_count,
            } => {
                self.directory_count = self.directory_count.checked_add(1).ok_or_else(|| {
                    "the platform analysis directory count overflowed".to_string()
                })?;
                // Platform implementations must already constrain records to the scan root. Core
                // verifies the boundary again so a future adapter defect cannot persist an
                // out-of-volume directory. This is the final fail-closed contract boundary, not a
                // product rule based on string matching.
                if !path.starts_with(self.root)
                    || (path != self.root
                        && current_platform()
                            .should_skip(&path, self.root, ScanPurpose::Analysis)
                            .is_some())
                {
                    return Err(
                        "the platform analysis returned a directory outside the scan root"
                            .to_string(),
                    );
                }
                // Native enumeration owns early pruning, but Core still enforces the shared
                // exclusion boundary before publishing records. This also suppresses a harmless
                // zero-byte placeholder some platform enumerators may emit for a pruned subtree.
                if self.exclusions.matches(&path) {
                    return Ok(());
                }
                let aggregate = DirectoryAggregate {
                    retained_file_limit,
                    scan_mode: AnalysisScanMode::Standard,
                    bytes: allocated_bytes,
                    logical_bytes,
                    file_count,
                    direct_file_count,
                    skipped_count,
                    scanned_at_ms: self.scanned_at_ms,
                    // Native fast paths validate snapshots with a platform change token rather
                    // than the generic traversal's directory metadata fingerprint. Without a
                    // token, Core already refuses cross-request reuse.
                    fingerprint: None,
                };
                if path == self.root && self.root_aggregate.replace(aggregate).is_some() {
                    return Err(
                        "the platform analysis returned the root directory more than once"
                            .to_string(),
                    );
                }
                self.progress
                    .visit_directory(TraversalStage::Analyzing, &path);
                sink.push_directory(path, aggregate)
            }
            FastAnalysisRecord::LargeFileCandidate(path) => {
                // Count raw records emitted by the platform rather than entries that survive live
                // metadata validation. A file disappearing after enumeration must not look like a
                // platform summary protocol violation.
                self.candidate_count = self.candidate_count.checked_add(1).ok_or_else(|| {
                    "the platform analysis candidate count overflowed".to_string()
                })?;
                // Files can change between layout enumeration and consumption. Validate candidates
                // against live metadata; an invalid candidate is omitted from the large-file index
                // without invalidating the directory aggregates completed by the same scan.
                if self.exclusions.matches(&path)
                    || !path.starts_with(self.root)
                    || current_platform()
                        .should_skip(&path, self.root, ScanPurpose::LargeFiles)
                        .is_some()
                {
                    return Ok(());
                }
                let Ok(metadata) = fs::symlink_metadata(&path) else {
                    return Ok(());
                };
                if !metadata.is_file() || is_link_like(&metadata) {
                    return Ok(());
                }
                let usage = current_platform().file_space_usage(&path, &metadata);
                if usage.allocated_bytes < LARGE_FILE_CANDIDATE_FLOOR_BYTES {
                    return Ok(());
                }
                sink.push_large_file(
                    path,
                    IndexedFile {
                        shared_identity: None,
                        bytes: usage.allocated_bytes,
                        logical_bytes: usage.logical_bytes,
                        modified_at_ms: modified_ms(&metadata),
                    },
                )
            }
        }
    }

    fn complete(&self, summary: &FastAnalysisSummary) -> Result<DirectoryAggregate, String> {
        let aggregate = self
            .root_aggregate
            .ok_or_else(|| "the platform analysis did not return a root aggregate".to_string())?;
        if aggregate.bytes != summary.root_allocated_bytes
            || aggregate.logical_bytes != summary.root_logical_bytes
            || aggregate.file_count != summary.root_file_count
            || aggregate.skipped_count != summary.root_skipped_count
        {
            return Err(
                "the platform analysis root aggregate does not match its summary".to_string(),
            );
        }
        if self.directory_count != summary.directory_count
            || self.candidate_count != summary.candidate_count
        {
            return Err(
                "the platform analysis record count does not match its summary".to_string(),
            );
        }
        Ok(aggregate)
    }
}

impl<'a> FastAnalysisProgressValidation<'a> {
    fn new(root: &'a Path, progress: &'a Arc<ProgressTracker>) -> Self {
        Self {
            root,
            progress,
            reported_file_count: 0,
            reported_bytes: 0,
        }
    }

    fn observe(&mut self, path: &Path, file_count: u64, bytes: u64) {
        self.reported_file_count = self.reported_file_count.saturating_add(file_count);
        self.reported_bytes = self.reported_bytes.saturating_add(bytes);
        self.progress
            .observe_files(TraversalStage::Analyzing, path, file_count, bytes);
    }

    fn complete(&mut self, aggregate: DirectoryAggregate) -> Result<(), String> {
        if self.reported_file_count > aggregate.file_count || self.reported_bytes > aggregate.bytes
        {
            return Err("the platform analysis progress exceeds the validated total".to_string());
        }
        let remaining_file_count = aggregate.file_count - self.reported_file_count;
        let remaining_bytes = aggregate.bytes - self.reported_bytes;
        if remaining_file_count > 0 || remaining_bytes > 0 {
            // Platforms that cannot expose trustworthy batches during enumeration still publish
            // the exact remainder after their final aggregate has passed validation.
            self.progress.observe_files(
                TraversalStage::Analyzing,
                self.root,
                remaining_file_count,
                remaining_bytes,
            );
            self.reported_file_count = aggregate.file_count;
            self.reported_bytes = aggregate.bytes;
        }
        Ok(())
    }
}

impl<'a> LargeFileStreamValidation<'a> {
    fn new(
        root: &'a Path,
        minimum_bytes: u64,
        scanned_at_ms: u64,
        progress: &'a Arc<ProgressTracker>,
        cancelled: &'a AtomicBool,
        report_candidate_progress: bool,
        exclusions: &'a StorageScanExclusions,
    ) -> Result<Self, String> {
        let root_metadata = fs::symlink_metadata(root)
            .map_err(|error| format!("failed to read scan-root metadata: {error}"))?;
        Ok(Self {
            root,
            root_metadata,
            minimum_bytes,
            progress,
            cancelled,
            aggregate: DirectoryAggregate {
                scanned_at_ms,
                ..DirectoryAggregate::default()
            },
            valid_count: 0,
            report_candidate_progress,
            exclusions,
        })
    }

    fn consume(&mut self, path: PathBuf, sink: &mut IndexRecordSink) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(OPERATION_CANCELLED_ERROR.to_string());
        }
        // A platform index narrows the candidate set but is not a safety boundary. Files may move,
        // be replaced, or shrink after indexing, so every candidate still needs live validation of
        // scope, protection policy, link type, and size.
        if self.exclusions.matches(&path) {
            return Ok(());
        }
        if !path.starts_with(self.root)
            || current_platform()
                .should_skip(&path, self.root, ScanPurpose::LargeFiles)
                .is_some()
        {
            self.aggregate.skipped_count += 1;
            return Ok(());
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            self.aggregate.skipped_count += 1;
            return Ok(());
        };
        if !current_platform().is_same_filesystem(&self.root_metadata, &metadata) {
            self.aggregate.skipped_count += 1;
            return Ok(());
        }
        if !metadata.is_file() || is_link_like(&metadata) {
            self.aggregate.skipped_count += 1;
            return Ok(());
        }
        let usage = current_platform().file_space_usage(&path, &metadata);
        if usage.allocated_bytes < self.minimum_bytes {
            // A native index may nominate a file by logical size before Core applies the current
            // physical-space threshold. Rejecting that valid but ineligible candidate is a normal
            // filter result, not an unreadable entry, so it must not inflate the user-visible
            // skipped count.
            return Ok(());
        }
        if !sink.insert_large_file_candidate(
            path.clone(),
            IndexedFile {
                shared_identity: None,
                bytes: usage.allocated_bytes,
                logical_bytes: usage.logical_bytes,
                modified_at_ms: modified_ms(&metadata),
            },
        ) {
            return Ok(());
        }
        if self.report_candidate_progress {
            self.progress.visit_file(
                traversal_stage(ScanPurpose::LargeFiles),
                &path,
                usage.allocated_bytes,
            );
        }
        self.aggregate.bytes = self.aggregate.bytes.saturating_add(usage.allocated_bytes);
        self.aggregate.logical_bytes = self
            .aggregate
            .logical_bytes
            .saturating_add(usage.logical_bytes);
        self.aggregate.file_count += 1;
        self.valid_count += 1;
        Ok(())
    }
}

fn stream_indexed_large_files_once(
    root: &Path,
    minimum_bytes: u64,
    scanned_at_ms: u64,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
    exclusions: &StorageScanExclusions,
    sink: &mut IndexRecordSink,
) -> Result<
    Option<(DirectoryAggregate, usize, LargeFileCandidateSummary)>,
    LargeFileCandidateScanError,
> {
    let mut validation = LargeFileStreamValidation::new(
        root,
        minimum_bytes,
        scanned_at_ms,
        progress,
        cancelled,
        true,
        exclusions,
    )
    .map_err(LargeFileCandidateScanError::Consumer)?;
    let summary = current_platform().fast_large_file_candidates(
        root,
        minimum_bytes,
        exclusions.roots(),
        &|| cancelled.load(Ordering::Relaxed),
        &mut |path| validation.consume(path, sink),
    )?;
    let Some(summary) = summary else {
        return Ok(None);
    };
    validation.aggregate.skipped_count = validation
        .aggregate
        .skipped_count
        .saturating_add(summary.skipped_count);
    sink.push_directory(root.to_path_buf(), validation.aggregate)
        .map_err(LargeFileCandidateScanError::Consumer)?;
    Ok(Some((
        validation.aggregate,
        validation.valid_count,
        summary,
    )))
}

/// Runs authoritative native enumeration while retaining only size-qualified file candidates.
///
/// The disk-analysis reader also emits directory aggregates. Ignoring those records here keeps
/// the complete large-file snapshot proportional to matching files instead of every directory on
/// the volume, while sharing the platform's tested traversal and protection policy.
fn stream_complete_large_files(
    root: &Path,
    minimum_bytes: u64,
    scanned_at_ms: u64,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
    exclusions: &StorageScanExclusions,
) -> Result<FastLargeFileScanOutcome, AnalysisStreamError> {
    let mut sink = IndexRecordSink::memory(None);
    let mut validation = LargeFileStreamValidation::new(
        root,
        minimum_bytes,
        scanned_at_ms,
        progress,
        cancelled,
        false,
        exclusions,
    )?;
    let summary = current_platform().fast_analysis_records(
        FastAnalysisQuery {
            retained_file_limit: 0,
            name_exclusions: exclusions.names(),
            excluded_roots: exclusions.roots(),
            root,
            purpose: ScanPurpose::LargeFiles,
            large_file_minimum_bytes: minimum_bytes,
            // Prune selected descendants and saved exclusions before native directory reads.
            // Candidate validation remains a second boundary for every source.
            should_prune_directory: |_| false,
        },
        &|| cancelled.load(Ordering::Relaxed),
        &mut |path, file_count, bytes| {
            progress.observe_files(TraversalStage::Analyzing, path, file_count, bytes);
        },
        &mut |record| match record {
            FastAnalysisRecord::LargeFileCandidate(path) => validation.consume(path, &mut sink),
            FastAnalysisRecord::Directory { .. }
            | FastAnalysisRecord::HardLinkedFile { .. }
            | FastAnalysisRecord::AnalysisFile(_) => Ok(()),
        },
    );
    match summary {
        Ok(Some(summary)) => {
            validation.aggregate.skipped_count = validation
                .aggregate
                .skipped_count
                .saturating_add(summary.root_skipped_count);
            sink.push_directory(root.to_path_buf(), validation.aggregate)?;
            Ok(FastLargeFileScanOutcome::Completed(Box::new(
                CompletedFastLargeFileScan {
                    aggregate: validation.aggregate,
                    completed_sink: sink.finish()?,
                    summary: LargeFileCandidateSummary {
                        native_directory_reads: summary.directory_count,
                        candidate_count: summary.candidate_count,
                        skipped_count: summary.root_skipped_count,
                        consumer_elapsed_ms: summary.consumer_elapsed_ms,
                        producer_backpressure_ms: 0,
                        peak_in_flight_candidates: 0,
                        strategy: summary.strategy,
                    },
                    valid_count: validation.valid_count,
                },
            )))
        }
        Ok(None) => Ok(FastLargeFileScanOutcome::Unsupported),
        Err(FastAnalysisScanError::Cancelled) => Err(AnalysisStreamError::Cancelled),
        Err(FastAnalysisScanError::Busy) => Err(AnalysisStreamError::ResourcesReleasing),
        Err(FastAnalysisScanError::Platform(error)) => {
            Ok(FastLargeFileScanOutcome::PlatformFailed { error })
        }
        Err(FastAnalysisScanError::Consumer(error)) => {
            if cancelled.load(Ordering::Relaxed) {
                Err(AnalysisStreamError::Cancelled)
            } else {
                Err(AnalysisStreamError::Failed(format!(
                    "failed to consume the complete large-file stream: {error}"
                )))
            }
        }
    }
}

fn traverse_once(
    root: &Path,
    kind: TraversalKind,
    scanned_at_ms: u64,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
    sink: &mut IndexRecordSink,
    scan_exclusions: Option<&StorageScanExclusions>,
) -> Result<DirectoryAggregate, String> {
    let purpose = kind.purpose();
    let scan_mode = kind.scan_mode();
    let root_metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("failed to read scan-root metadata: {error}"))?;
    let mut traversal = AnalysisTraversal {
        scan_root: root,
        root_metadata,
        purpose,
        scan_mode,
        progress,
        scanned_at_ms,
        sink,
        cancelled,
        scan_exclusions,
    };
    #[cfg(windows)]
    if purpose == ScanPurpose::Analysis {
        let workers = parallel_analysis::worker_count(root);
        if workers > 1 {
            if let Some(aggregate) = parallel_analysis::measure(root, &mut traversal, workers)? {
                return Ok(aggregate);
            }
        }
    }
    measure_analysis_directory(root, &mut traversal)
}

fn stream_fast_analysis_once(
    root: &Path,
    scanned_at_ms: u64,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
    exclusions: &StorageScanExclusions,
    sink: &mut IndexRecordSink,
) -> Result<Option<(DirectoryAggregate, FastAnalysisSummary)>, FastAnalysisScanError> {
    let mut validation =
        FastAnalysisStreamValidation::new(root, scanned_at_ms, progress, cancelled, exclusions);
    let mut progress_validation = FastAnalysisProgressValidation::new(root, progress);
    let summary = current_platform().fast_analysis_records(
        FastAnalysisQuery {
            retained_file_limit: ANALYSIS_FILES_PER_DIRECTORY,
            name_exclusions: exclusions.names(),
            excluded_roots: exclusions.roots(),
            root,
            purpose: ScanPurpose::Analysis,
            large_file_minimum_bytes: LARGE_FILE_CANDIDATE_FLOOR_BYTES,
            should_prune_directory: |_| false,
        },
        &|| cancelled.load(Ordering::Relaxed),
        &mut |path, file_count, bytes| progress_validation.observe(path, file_count, bytes),
        &mut |record| validation.consume(record, sink),
    )?;
    let Some(summary) = summary else {
        return Ok(None);
    };
    let aggregate = validation
        .complete(&summary)
        .map_err(FastAnalysisScanError::Consumer)?;
    progress_validation
        .complete(aggregate)
        .map_err(FastAnalysisScanError::Consumer)?;
    Ok(Some((aggregate, summary)))
}

#[derive(Debug)]
enum AnalysisStreamError {
    Cancelled,
    ResourcesReleasing,
    Failed(String),
}

impl From<String> for AnalysisStreamError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

fn stream_fast_analysis(
    root: &Path,
    scanned_at_ms: u64,
    change_token: Option<FilesystemChangeToken>,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
    exclusions: &StorageScanExclusions,
) -> Result<FastAnalysisOutcome, AnalysisStreamError> {
    let mut sink = IndexRecordSink::memory(change_token);
    let attempt = stream_fast_analysis_once(
        root,
        scanned_at_ms,
        progress,
        cancelled,
        exclusions,
        &mut sink,
    );
    match attempt {
        Ok(Some((_, mut summary))) => {
            let completed_sink = sink.finish_analysis()?;
            let aggregate = *completed_sink.directories.get(root).ok_or_else(|| {
                AnalysisStreamError::Failed("analysis root is missing".to_string())
            })?;
            summary.root_allocated_bytes = aggregate.bytes;
            progress.replace_scan_observations(aggregate.file_count, aggregate.bytes);
            Ok(FastAnalysisOutcome::Completed(Box::new(
                CompletedFastAnalysis {
                    aggregate,
                    completed_sink,
                    summary,
                },
            )))
        }
        Ok(None) => Ok(FastAnalysisOutcome::Unsupported),
        Err(FastAnalysisScanError::Cancelled) => Err(AnalysisStreamError::Cancelled),
        Err(FastAnalysisScanError::Busy) => Err(AnalysisStreamError::ResourcesReleasing),
        Err(FastAnalysisScanError::Platform(error)) => {
            Ok(FastAnalysisOutcome::PlatformFailed { error })
        }
        Err(FastAnalysisScanError::Consumer(error)) => {
            if cancelled.load(Ordering::Relaxed) {
                Err(AnalysisStreamError::Cancelled)
            } else {
                Err(AnalysisStreamError::Failed(format!(
                    "failed to consume the analysis stream: {error}"
                )))
            }
        }
    }
}

fn stream_fast_large_files(
    root: &Path,
    minimum_bytes: u64,
    scanned_at_ms: u64,
    change_token: Option<FilesystemChangeToken>,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
    exclusions: &StorageScanExclusions,
) -> Result<FastLargeFileScanOutcome, String> {
    let mut sink = IndexRecordSink::memory(change_token);
    let attempt = stream_indexed_large_files_once(
        root,
        minimum_bytes,
        scanned_at_ms,
        progress,
        cancelled,
        exclusions,
        &mut sink,
    );
    match attempt {
        Ok(Some((aggregate, valid_count, summary))) => Ok(FastLargeFileScanOutcome::Completed(
            Box::new(CompletedFastLargeFileScan {
                aggregate,
                completed_sink: sink.finish()?,
                summary,
                valid_count,
            }),
        )),
        Ok(None) => Ok(FastLargeFileScanOutcome::Unsupported),
        Err(LargeFileCandidateScanError::Cancelled) => Err(OPERATION_CANCELLED_ERROR.to_string()),
        Err(LargeFileCandidateScanError::Platform(error)) => {
            Ok(FastLargeFileScanOutcome::PlatformFailed { error })
        }
        Err(LargeFileCandidateScanError::Consumer(error)) => {
            if cancelled.load(Ordering::Relaxed) {
                Err(OPERATION_CANCELLED_ERROR.to_string())
            } else {
                Err(format!("failed to consume the candidate stream: {error}"))
            }
        }
    }
}

fn traverse_memory_only(
    root: &Path,
    kind: TraversalKind,
    scanned_at_ms: u64,
    change_token: Option<FilesystemChangeToken>,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
    scan_exclusions: Option<&StorageScanExclusions>,
) -> Result<(DirectoryAggregate, CompletedIndexSink), String> {
    let purpose = kind.purpose();
    progress.reset_scan_observations_for_retry();
    let mut sink = IndexRecordSink::memory(change_token);
    let aggregate = traverse_once(
        root,
        kind,
        scanned_at_ms,
        progress,
        cancelled,
        &mut sink,
        scan_exclusions,
    )?;
    let completed = if purpose == ScanPurpose::Analysis {
        sink.finish_analysis()?
    } else {
        sink.finish()?
    };
    let aggregate = completed
        .directories
        .get(root)
        .copied()
        .unwrap_or(aggregate);
    if purpose == ScanPurpose::Analysis {
        progress.replace_scan_observations(aggregate.file_count, aggregate.bytes);
    }
    Ok((aggregate, completed))
}

fn publish_completed_index(
    root: &Path,
    root_aggregate: DirectoryAggregate,
    completed: CompletedIndexSink,
    options: IndexPublicationOptions<'_>,
) -> Result<(), String> {
    let _published = cache::store_memory_only(
        root,
        root_aggregate,
        completed.directories,
        completed.files,
        cache::SnapshotPublication::new(
            options.purpose,
            options.refresh,
            completed.change_token,
            options.generation,
            options.expected_mutation_revision,
        )
        .with_configuration_fingerprint(options.configuration_fingerprint)
        .with_excluded_roots(options.excluded_roots)
        .with_excluded_names(options.excluded_names),
    )?;
    Ok(())
}

fn capture_filesystem_change_token(root: &Path) -> Option<FilesystemChangeToken> {
    match current_platform().capture_filesystem_change_token(root) {
        Ok(token) => token,
        Err(error) => {
            // A change cursor only accelerates a later scan. Capture failure must not invalidate the
            // current real scan, but the resulting snapshot cannot prove cross-change reuse, so
            // later requests perform a complete scan under the fail-closed policy.
            log::warn!(
                "filesystem_change_token_capture_failed platform={} error={}",
                current_platform().os_name(),
                mangodisk_platform::diagnostics::text(&error)
            );
            None
        }
    }
}

fn traversal_stage(purpose: ScanPurpose) -> TraversalStage {
    match purpose {
        ScanPurpose::Cleanup
        | ScanPurpose::Analysis
        | ScanPurpose::LargeFiles
        | ScanPurpose::DuplicateFiles => TraversalStage::Analyzing,
    }
}

#[cfg(test)]
#[path = "traversal_tests.rs"]
mod tests;
