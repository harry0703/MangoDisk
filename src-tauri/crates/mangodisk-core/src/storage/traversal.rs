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
    modified_ms, now_ms,
};
use crate::shared::operation::{
    CoordinatedOperationKind, OperationGuard, OPERATION_CANCELLED_ERROR,
};
use crate::shared::progress::ProgressTracker;
use crate::shared::{CoreError, CoreResult, TraversalProgress, TraversalStage};
use crate::storage::analysis::AnalysisResult;
use crate::storage::index::cache::{
    self, CacheReuseDecision, DirectoryAggregate, IndexedFile, LARGE_FILE_INDEX_FLOOR_BYTES,
};
use crate::storage::large_files::LargeFilesResult;
use mangodisk_platform::{
    current_platform, FastAnalysisQuery, FastAnalysisRecord, FastAnalysisScanError,
    FastAnalysisSummary, FilesystemChangeToken, LargeFileCandidateScanError,
    LargeFileCandidateSummary, Platform, ScanPurpose,
};

mod index_sink;

use index_sink::{CompletedIndexSink, IndexRecordSink};
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
    pub(crate) cache_validation_ms: u64,
    pub(crate) candidate_discovery_ms: u64,
    pub(crate) validation_or_traversal_ms: u64,
    pub(crate) candidate_count: u64,
    pub(crate) candidate_backpressure_ms: u64,
    pub(crate) candidate_peak_in_flight: usize,
    pub(crate) candidate_strategy: &'static str,
    pub(crate) cache_write_ms: u64,
    pub(crate) result_build_ms: u64,
    pub(crate) fast_path: &'static str,
    pub(crate) fallback_reason: Option<&'static str>,
}

struct AnalysisTraversal<'a> {
    scan_root: &'a Path,
    root_metadata: fs::Metadata,
    purpose: ScanPurpose,
    progress: &'a Arc<ProgressTracker>,
    scanned_at_ms: u64,
    sink: &'a mut IndexRecordSink,
    cancelled: &'a AtomicBool,
}

struct LargeFileStreamValidation<'a> {
    root: &'a Path,
    root_metadata: fs::Metadata,
    minimum_bytes: u64,
    progress: &'a Arc<ProgressTracker>,
    cancelled: &'a AtomicBool,
    aggregate: DirectoryAggregate,
    valid_count: usize,
}

struct FastAnalysisStreamValidation<'a> {
    root: &'a Path,
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

struct CompletedLargeFileFallback {
    aggregate: DirectoryAggregate,
    completed_sink: CompletedIndexSink,
    snapshot_purpose: ScanPurpose,
    native_summary: Option<FastAnalysisSummary>,
}

pub(crate) struct StorageTraversal;

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
        let operation = OperationGuard::start(CoordinatedOperationKind::Analysis)?;
        let started = Instant::now();
        let mut diagnostics = AnalysisScanDiagnostics::default();
        let root = resolve_analysis_root(path)?;
        let root = current_platform()
            .canonicalize_no_links(&root)
            .map_err(|error| error.to_string())?;
        if !root.is_dir() {
            return Err(CoreError::invalid_input(
                "the analysis root must be a directory",
            ));
        }
        let progress = Arc::new(ProgressTracker::new(operation.id(), callback, 0));
        let cache_validation_started = Instant::now();
        let cache_decision = if refresh {
            CacheReuseDecision::Miss
        } else {
            cache::reuse_decision(&root, ScanPurpose::Analysis, &|| {
                operation.cancelled().load(Ordering::Relaxed)
            })
            .map_err(traversal_core_error)?
        };
        diagnostics.cache_validation_ms = cache_validation_started.elapsed().as_millis() as u64;
        match cache_decision {
            CacheReuseDecision::Reusable => {
                if let Some(result) = cache::analysis_result(&root)? {
                    diagnostics.fast_path = "cache";
                    operation.complete();
                    return Ok((result, diagnostics));
                }
            }
            CacheReuseDecision::Miss => {}
        }
        progress.emit(TraversalStage::Analyzing, &root);
        let scanned_at_ms = now_ms();
        let cache_mutation_revision = cache::mutation_revision()?;
        let change_token = capture_filesystem_change_token(&root);
        let traversal_started = Instant::now();
        let fast_scan = stream_fast_analysis(
            &root,
            scanned_at_ms,
            change_token,
            &progress,
            operation.cancelled(),
        );
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
            FastAnalysisOutcome::Unsupported => {
                diagnostics.fast_path = "fallback";
                diagnostics.fallback_reason = Some("unsupported");
                traverse_memory_only(
                    &root,
                    ScanPurpose::Analysis,
                    scanned_at_ms,
                    change_token,
                    &progress,
                    operation.cancelled(),
                )
                .map_err(traversal_core_error)?
            }
            FastAnalysisOutcome::PlatformFailed { error } => {
                diagnostics.fast_path = "fallback";
                diagnostics.fallback_reason = Some("fastPathFailed");
                log::warn!(
                    "analysis_scan_fallback operation_id={} platform={} root={} reason=fast_scan_failed error_digest={}",
                    operation.id(),
                    current_platform().os_name(),
                    diagnostic_path(&root),
                    blake3::hash(error.as_bytes()).to_hex()
                );
                traverse_memory_only(
                    &root,
                    ScanPurpose::Analysis,
                    scanned_at_ms,
                    change_token,
                    &progress,
                    operation.cancelled(),
                )
                .map_err(traversal_core_error)?
            }
        };
        diagnostics.traversal_ms = traversal_started.elapsed().as_millis() as u64;
        // A scan may finish before the progress-throttling window. Force the final counters here so
        // adapters receive complete state and benchmarks do not record a fast scan as zero files
        // and zero bytes.
        progress.finish(TraversalStage::Analyzing, &root);
        let result_build_started = Instant::now();
        let result = cache::analysis_result_from_snapshot(
            &root,
            root_aggregate,
            &completed_sink.directories,
            &completed_sink.files,
        )?;
        diagnostics.result_build_ms = result_build_started.elapsed().as_millis() as u64;
        let cache_write_started = Instant::now();
        publish_completed_index(
            &root,
            root_aggregate,
            completed_sink,
            ScanPurpose::Analysis,
            refresh,
            operation.id(),
            cache_mutation_revision,
        )?;
        diagnostics.cache_write_ms = cache_write_started.elapsed().as_millis() as u64;
        log::info!(
            "analysis_scan_finished operation_id={} root={} total_bytes={} entry_count={} skipped_count={} elapsed_ms={}",
            operation.id(),
            diagnostic_path(&root),
            result.total_bytes,
            result.entries.len(),
            result.skipped_count,
            started.elapsed().as_millis()
        );
        operation.complete();
        Ok((result, diagnostics))
    }

    pub fn find_large_files_with_progress(
        path: Option<String>,
        minimum_bytes: u64,
        refresh: bool,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<LargeFilesResult> {
        Self::find_large_files_with_diagnostics(path, minimum_bytes, refresh, callback)
            .map(|(result, _)| result)
    }

    pub(crate) fn find_large_files_with_diagnostics(
        path: Option<String>,
        minimum_bytes: u64,
        refresh: bool,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<(LargeFilesResult, LargeFileScanDiagnostics)> {
        let operation = OperationGuard::start(CoordinatedOperationKind::LargeFiles)?;
        let mut diagnostics = LargeFileScanDiagnostics::default();
        let root = path
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| current_platform().system_volume_path());
        let root = current_platform()
            .canonicalize_no_links(&root)
            .map_err(|error| error.to_string())?;
        if !root.is_dir() {
            return Err(CoreError::invalid_input(
                "the scan root must be a directory or volume",
            ));
        }

        let result_minimum_bytes = minimum_bytes.max(LARGE_FILE_INDEX_FLOOR_BYTES);
        let cache_validation_started = Instant::now();
        let progress = Arc::new(ProgressTracker::new(operation.id(), callback, 0));
        let cache_decision = if refresh {
            CacheReuseDecision::Miss
        } else {
            cache::reuse_decision(&root, ScanPurpose::LargeFiles, &|| {
                operation.cancelled().load(Ordering::Relaxed)
            })
            .map_err(traversal_core_error)?
        };
        let mut cache_reused = matches!(&cache_decision, CacheReuseDecision::Reusable);
        diagnostics.cache_validation_ms = cache_validation_started.elapsed().as_millis() as u64;
        log::info!(
            "large_file_query_started operation_id={} root={} requested_minimum_bytes={} result_minimum_bytes={} index_floor_bytes={} refresh={} cache_reused={}",
            operation.id(),
            diagnostic_path(&root),
            minimum_bytes,
            result_minimum_bytes,
            LARGE_FILE_INDEX_FLOOR_BYTES,
            refresh,
            cache_reused
        );
        if cache_reused {
            let result_build_started = Instant::now();
            match cache::large_files_result(&root, result_minimum_bytes, true) {
                Ok(result) => {
                    diagnostics.fast_path = "cache";
                    diagnostics.result_build_ms = result_build_started.elapsed().as_millis() as u64;
                    operation.complete();
                    return Ok((result, diagnostics));
                }
                Err(error) => {
                    // Corruption or user-triggered index removal can race snapshot validation and
                    // result loading. Persistence is only an optimization, so this race falls back
                    // to a complete scan instead of becoming a user-visible failure.
                    log::warn!(
                        "large_file_cache_read_fallback operation_id={} error_digest={}",
                        operation.id(),
                        blake3::hash(error.as_bytes()).to_hex()
                    );
                    cache_reused = false;
                }
            }
        }
        if !cache_reused {
            progress.emit(TraversalStage::Analyzing, &root);
            let scanned_at_ms = now_ms();
            let cache_mutation_revision = cache::mutation_revision()?;
            let change_token = capture_filesystem_change_token(&root);
            let candidate_started = Instant::now();
            let fast_scan = stream_fast_large_files(
                &root,
                LARGE_FILE_INDEX_FLOOR_BYTES,
                scanned_at_ms,
                change_token,
                &progress,
                operation.cancelled(),
            );
            diagnostics.candidate_discovery_ms = candidate_started.elapsed().as_millis() as u64;
            let (root_aggregate, completed_sink, snapshot_purpose) = match fast_scan
                .map_err(traversal_core_error)?
            {
                FastLargeFileScanOutcome::Completed(scan) => {
                    diagnostics.fast_path = "used";
                    diagnostics.validation_or_traversal_ms = scan.summary.consumer_elapsed_ms;
                    diagnostics.candidate_count = scan.summary.candidate_count;
                    diagnostics.candidate_backpressure_ms = scan.summary.producer_backpressure_ms;
                    diagnostics.candidate_peak_in_flight = scan.summary.peak_in_flight_candidates;
                    diagnostics.candidate_strategy = scan.summary.strategy;
                    log::info!(
                        "large_file_fast_scan_finished operation_id={} platform={} root={} strategy={} candidate_count={} valid_count={} skipped_count={} producer_backpressure_ms={} peak_in_flight_candidates={} elapsed_ms={}",
                        operation.id(),
                        current_platform().os_name(),
                        diagnostic_path(&root),
                        scan.summary.strategy,
                        scan.summary.candidate_count,
                        scan.valid_count,
                        scan.aggregate.skipped_count,
                        scan.summary.producer_backpressure_ms,
                        scan.summary.peak_in_flight_candidates,
                        diagnostics.candidate_discovery_ms
                    );
                    (scan.aggregate, scan.completed_sink, ScanPurpose::LargeFiles)
                }
                FastLargeFileScanOutcome::Unsupported => {
                    diagnostics.fallback_reason = Some("unsupported");
                    log::info!(
                        "large_file_scan_fallback operation_id={} platform={} root={} reason=unsupported",
                        operation.id(),
                        current_platform().os_name(),
                        diagnostic_path(&root)
                    );
                    let fallback_started = Instant::now();
                    let result = scan_large_files_after_candidate_failure(
                        &root,
                        scanned_at_ms,
                        change_token,
                        &progress,
                        operation.cancelled(),
                    )
                    .map_err(|error| analysis_stream_core_error(&operation, error))?;
                    diagnostics.validation_or_traversal_ms =
                        fallback_started.elapsed().as_millis() as u64;
                    apply_large_file_fallback_diagnostics(&mut diagnostics, &result);
                    (
                        result.aggregate,
                        result.completed_sink,
                        result.snapshot_purpose,
                    )
                }
                FastLargeFileScanOutcome::PlatformFailed { error } => {
                    diagnostics.fallback_reason = Some("fastPathFailed");
                    // Spotlight can be unavailable for a directory even when macOS supports fast
                    // metadata enumeration. Reuse the native full-analysis stream before falling
                    // back to `read_dir`; it preserves exact large-file semantics and publishes an
                    // Analysis snapshot that Disk Space Analysis can reuse later.
                    let error_digest = blake3::hash(error.as_bytes()).to_hex().to_string();
                    log::warn!(
                        "large_file_scan_fallback operation_id={} platform={} root={} reason=fast_scan_failed error_digest={}",
                        operation.id(),
                        current_platform().os_name(),
                        diagnostic_path(&root),
                        error_digest
                    );
                    let fallback_started = Instant::now();
                    let result = scan_large_files_after_candidate_failure(
                        &root,
                        scanned_at_ms,
                        change_token,
                        &progress,
                        operation.cancelled(),
                    )
                    .map_err(|error| analysis_stream_core_error(&operation, error))?;
                    diagnostics.validation_or_traversal_ms =
                        fallback_started.elapsed().as_millis() as u64;
                    apply_large_file_fallback_diagnostics(&mut diagnostics, &result);
                    (
                        result.aggregate,
                        result.completed_sink,
                        result.snapshot_purpose,
                    )
                }
            };
            progress.finish(TraversalStage::Analyzing, &root);
            let result_build_started = Instant::now();
            let result = cache::large_files_result_from_snapshot(
                &root,
                root_aggregate,
                &completed_sink.files,
                result_minimum_bytes,
            );
            diagnostics.result_build_ms = result_build_started.elapsed().as_millis() as u64;
            let cache_write_started = Instant::now();
            publish_completed_index(
                &root,
                root_aggregate,
                completed_sink,
                snapshot_purpose,
                refresh,
                operation.id(),
                cache_mutation_revision,
            )?;
            diagnostics.cache_write_ms = cache_write_started.elapsed().as_millis() as u64;
            operation.complete();
            return Ok((result, diagnostics));
        }
        Err(CoreError::operation_failed(
            "the large-file scan did not produce a result",
        ))
    }
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

fn measure_analysis_directory(
    path: &Path,
    traversal: &mut AnalysisTraversal<'_>,
) -> Result<DirectoryAggregate, String> {
    if traversal.cancelled.load(Ordering::Relaxed) {
        return Err(OPERATION_CANCELLED_ERROR.to_string());
    }
    traversal
        .progress
        .visit_directory(traversal_stage(traversal.purpose), path);
    let mut aggregate = DirectoryAggregate {
        scanned_at_ms: traversal.scanned_at_ms,
        ..DirectoryAggregate::default()
    };
    let Ok(entries) = fs::read_dir(path) else {
        aggregate.skipped_count = 1;
        traversal
            .sink
            .push_directory(path.to_path_buf(), aggregate)?;
        return Ok(aggregate);
    };
    let mut fingerprint_entries = Vec::new();

    for entry in entries {
        if traversal.cancelled.load(Ordering::Relaxed) {
            return Err(OPERATION_CANCELLED_ERROR.to_string());
        }
        let Ok(entry) = entry else {
            aggregate.skipped_count += 1;
            continue;
        };
        let child_path = entry.path();
        if current_platform()
            .should_skip(&child_path, traversal.scan_root, traversal.purpose)
            .is_some()
        {
            aggregate.skipped_count += 1;
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&child_path) else {
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
            let child = measure_analysis_directory(&child_path, traversal)?;
            aggregate.bytes += child.bytes;
            aggregate.logical_bytes += child.logical_bytes;
            aggregate.file_count += child.file_count;
            aggregate.skipped_count += child.skipped_count;
            if let Some(entry) =
                metadata_fingerprint_entry(&child_path, &metadata, child.fingerprint)
            {
                fingerprint_entries.push(entry);
            } else {
                aggregate.skipped_count += 1;
            }
        } else if metadata.is_file() {
            let usage = current_platform().file_space_usage(&child_path, &metadata);
            traversal.progress.visit_file(
                traversal_stage(traversal.purpose),
                &child_path,
                usage.allocated_bytes,
            );
            aggregate.bytes += usage.allocated_bytes;
            aggregate.logical_bytes += usage.logical_bytes;
            aggregate.file_count += 1;
            if let Some(entry) = metadata_fingerprint_entry(&child_path, &metadata, None) {
                fingerprint_entries.push(entry);
            } else {
                aggregate.skipped_count += 1;
            }
            // Analysis aggregates include application and system files, but their large-file rows
            // serve `LargeFiles` queries after restart. Apply the stricter large-file boundary
            // before persistence instead of relying only on in-memory result filtering.
            if usage.allocated_bytes >= LARGE_FILE_INDEX_FLOOR_BYTES
                && current_platform()
                    .should_skip(&child_path, traversal.scan_root, ScanPurpose::LargeFiles)
                    .is_none()
            {
                traversal.sink.push_large_file(
                    child_path,
                    IndexedFile {
                        bytes: usage.allocated_bytes,
                        logical_bytes: usage.logical_bytes,
                        modified_at_ms: modified_ms(&metadata),
                    },
                )?;
            }
        } else {
            aggregate.skipped_count += 1;
        }
    }
    if aggregate.skipped_count == 0 {
        aggregate.fingerprint = Some(finalize_metadata_fingerprint(fingerprint_entries));
    }
    traversal
        .sink
        .push_directory(path.to_path_buf(), aggregate)?;
    Ok(aggregate)
}

impl<'a> FastAnalysisStreamValidation<'a> {
    fn new(
        root: &'a Path,
        scanned_at_ms: u64,
        progress: &'a Arc<ProgressTracker>,
        cancelled: &'a AtomicBool,
    ) -> Self {
        Self {
            root,
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
            FastAnalysisRecord::Directory {
                path,
                logical_bytes,
                allocated_bytes,
                file_count,
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
                let aggregate = DirectoryAggregate {
                    bytes: allocated_bytes,
                    logical_bytes,
                    file_count,
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
                if !path.starts_with(self.root)
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
                if usage.allocated_bytes < LARGE_FILE_INDEX_FLOOR_BYTES {
                    return Ok(());
                }
                sink.push_large_file(
                    path,
                    IndexedFile {
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
        })
    }

    fn consume(&mut self, path: PathBuf, sink: &mut IndexRecordSink) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(OPERATION_CANCELLED_ERROR.to_string());
        }
        // A platform index narrows the candidate set but is not a safety boundary. Files may move,
        // be replaced, or shrink after indexing, so every candidate still needs live validation of
        // scope, protection policy, link type, and size.
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
                bytes: usage.allocated_bytes,
                logical_bytes: usage.logical_bytes,
                modified_at_ms: modified_ms(&metadata),
            },
        ) {
            return Ok(());
        }
        self.progress.visit_file(
            traversal_stage(ScanPurpose::LargeFiles),
            &path,
            usage.allocated_bytes,
        );
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
    sink: &mut IndexRecordSink,
) -> Result<
    Option<(DirectoryAggregate, usize, LargeFileCandidateSummary)>,
    LargeFileCandidateScanError,
> {
    let mut validation =
        LargeFileStreamValidation::new(root, minimum_bytes, scanned_at_ms, progress, cancelled)
            .map_err(LargeFileCandidateScanError::Consumer)?;
    let summary = current_platform().fast_large_file_candidates(
        root,
        minimum_bytes,
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

fn traverse_once(
    root: &Path,
    purpose: ScanPurpose,
    scanned_at_ms: u64,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
    sink: &mut IndexRecordSink,
) -> Result<DirectoryAggregate, String> {
    traverse_subtree(
        root,
        root,
        purpose,
        scanned_at_ms,
        progress,
        cancelled,
        sink,
    )
}

fn traverse_subtree(
    scan_root: &Path,
    measurement_root: &Path,
    purpose: ScanPurpose,
    scanned_at_ms: u64,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
    sink: &mut IndexRecordSink,
) -> Result<DirectoryAggregate, String> {
    let root_metadata = fs::symlink_metadata(scan_root)
        .map_err(|error| format!("failed to read scan-root metadata: {error}"))?;
    if measurement_root != scan_root {
        let measurement_metadata = fs::symlink_metadata(measurement_root)
            .map_err(|error| format!("failed to read subtree metadata: {error}"))?;
        if !current_platform().is_same_filesystem(&root_metadata, &measurement_metadata) {
            let aggregate = DirectoryAggregate {
                skipped_count: 1,
                scanned_at_ms,
                ..DirectoryAggregate::default()
            };
            sink.push_directory(measurement_root.to_path_buf(), aggregate)?;
            return Ok(aggregate);
        }
    }
    let mut traversal = AnalysisTraversal {
        scan_root,
        root_metadata,
        purpose,
        progress,
        scanned_at_ms,
        sink,
        cancelled,
    };
    measure_analysis_directory(measurement_root, &mut traversal)
}

fn stream_fast_analysis_once(
    root: &Path,
    scanned_at_ms: u64,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
    sink: &mut IndexRecordSink,
) -> Result<Option<(DirectoryAggregate, FastAnalysisSummary)>, FastAnalysisScanError> {
    let mut validation =
        FastAnalysisStreamValidation::new(root, scanned_at_ms, progress, cancelled);
    let mut progress_validation = FastAnalysisProgressValidation::new(root, progress);
    let summary = current_platform().fast_analysis_records(
        FastAnalysisQuery {
            root,
            purpose: ScanPurpose::Analysis,
            large_file_minimum_bytes: LARGE_FILE_INDEX_FLOOR_BYTES,
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
) -> Result<FastAnalysisOutcome, AnalysisStreamError> {
    let mut sink = IndexRecordSink::memory(change_token);
    let attempt = stream_fast_analysis_once(root, scanned_at_ms, progress, cancelled, &mut sink);
    match attempt {
        Ok(Some((aggregate, summary))) => Ok(FastAnalysisOutcome::Completed(Box::new(
            CompletedFastAnalysis {
                aggregate,
                completed_sink: sink.finish()?,
                summary,
            },
        ))),
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
) -> Result<FastLargeFileScanOutcome, String> {
    let mut sink = IndexRecordSink::memory(change_token);
    let attempt = stream_indexed_large_files_once(
        root,
        minimum_bytes,
        scanned_at_ms,
        progress,
        cancelled,
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

fn scan_large_files_after_candidate_failure(
    root: &Path,
    scanned_at_ms: u64,
    change_token: Option<FilesystemChangeToken>,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
) -> Result<CompletedLargeFileFallback, AnalysisStreamError> {
    // A failed candidate provider may already have emitted progress. Reset before starting a
    // complete native stream so the final counters continue to describe one filesystem pass.
    progress.reset_scan_observations_for_retry();
    match stream_fast_analysis(root, scanned_at_ms, change_token, progress, cancelled)? {
        FastAnalysisOutcome::Completed(scan) => Ok(CompletedLargeFileFallback {
            aggregate: scan.aggregate,
            completed_sink: scan.completed_sink,
            snapshot_purpose: ScanPurpose::Analysis,
            native_summary: Some(scan.summary),
        }),
        FastAnalysisOutcome::Unsupported => {
            let (aggregate, completed_sink) = traverse_memory_only(
                root,
                ScanPurpose::LargeFiles,
                scanned_at_ms,
                change_token,
                progress,
                cancelled,
            )?;
            Ok(CompletedLargeFileFallback {
                aggregate,
                completed_sink,
                snapshot_purpose: ScanPurpose::LargeFiles,
                native_summary: None,
            })
        }
        FastAnalysisOutcome::PlatformFailed { error } => {
            log::warn!(
                "large_file_native_analysis_fallback platform={} root={} error_digest={}",
                current_platform().os_name(),
                diagnostic_path(root),
                blake3::hash(error.as_bytes()).to_hex()
            );
            let (aggregate, completed_sink) = traverse_memory_only(
                root,
                ScanPurpose::LargeFiles,
                scanned_at_ms,
                change_token,
                progress,
                cancelled,
            )?;
            Ok(CompletedLargeFileFallback {
                aggregate,
                completed_sink,
                snapshot_purpose: ScanPurpose::LargeFiles,
                native_summary: None,
            })
        }
    }
}

fn apply_large_file_fallback_diagnostics(
    diagnostics: &mut LargeFileScanDiagnostics,
    fallback: &CompletedLargeFileFallback,
) {
    if let Some(summary) = &fallback.native_summary {
        diagnostics.fast_path = "nativeAnalysisFallback";
        diagnostics.candidate_strategy = summary.strategy;
        diagnostics.candidate_count = summary.candidate_count;
        log::info!(
            "large_file_native_analysis_finished strategy={} pages={} entries={} directories={} candidates={} consumer_ms={}",
            summary.strategy,
            summary.page_count,
            summary.entry_count,
            summary.directory_count,
            summary.candidate_count,
            summary.consumer_elapsed_ms
        );
    } else {
        diagnostics.fast_path = "fallback";
    }
}

fn traverse_memory_only(
    root: &Path,
    purpose: ScanPurpose,
    scanned_at_ms: u64,
    change_token: Option<FilesystemChangeToken>,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
) -> Result<(DirectoryAggregate, CompletedIndexSink), String> {
    progress.reset_scan_observations_for_retry();
    let mut sink = IndexRecordSink::memory(change_token);
    let aggregate = traverse_once(root, purpose, scanned_at_ms, progress, cancelled, &mut sink)?;
    let completed = sink.finish()?;
    Ok((aggregate, completed))
}

fn publish_completed_index(
    root: &Path,
    root_aggregate: DirectoryAggregate,
    completed: CompletedIndexSink,
    purpose: ScanPurpose,
    refresh: bool,
    publish_generation: u64,
    expected_mutation_revision: u64,
) -> Result<(), String> {
    let _published = cache::store_memory_only(
        root,
        root_aggregate,
        completed.directories,
        completed.files,
        cache::SnapshotPublication::new(
            purpose,
            refresh,
            completed.change_token,
            publish_generation,
            expected_mutation_revision,
        ),
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
                "filesystem_change_token_capture_failed platform={} error_digest={}",
                current_platform().os_name(),
                blake3::hash(error.as_bytes()).to_hex()
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
