use crate::filesystem::ScanExclusionOptions;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::sync_channel,
        Arc,
    },
    thread,
    time::Instant,
};

use mangodisk_platform::{
    current_platform, FastAnalysisQuery, FastAnalysisRecord, FastAnalysisScanError,
    FastAnalysisSummary, FileSpaceUsage, Platform, PlatformCancellation, ScanDeviceClass,
    ScanPurpose, VolumeInfo,
};

use super::cache_validation::{
    capture_duplicate_cache_roots, finish_duplicate_cache_validation, log_cache_error,
    start_duplicate_cache_validation, CacheValidationPhase, PendingDuplicateCacheValidation,
};
use super::candidates::{
    collect_native_candidate, normalize_roots, pruned_directory_ancestor, remove_physical_aliases,
    validate_current_path, validate_open_file, CandidateEnumeration, CandidateEnumerationRequest,
    DuplicateCandidatePolicy, FileCandidate, FileIdentity, NativeCandidateRequest,
};
use super::directory_aggregation::{aggregate_exact_directories, verify_live_directory};
use super::hash_cache::{
    self, DuplicateHashCacheFile, DuplicateHashCacheRoot, DuplicateHashCacheWriteDiagnostics,
};
use super::session::{
    clear_result_session, publish_result_session, resolve_open_target, result_page,
    synchronize_result_session, validate_permanent_delete_candidates,
    ValidatedDuplicateDeleteCandidate, DUPLICATE_RESULT_PAGE_SIZE,
};
use super::stream::DuplicateGroupStream;
use crate::filesystem::metadata::{
    diagnostic_path, display_path, modified_ms, native_path_string, now_ms,
};
use crate::filesystem::{
    permanent_delete::{
        delete_file_candidate_permanently, delete_path_permanently,
        prepare_path_for_permanent_delete,
    },
    PermanentDeleteBatchResult, PermanentDeleteCandidate, PermanentDeleteFailure,
};
use crate::history::{file_cleanup_record, FileCleanupHistoryCategory, HistoryService};
use crate::shared::operation::{CoordinatedOperationKind, OperationGuard};
use crate::storage::{index::cache, StorageScanExclusions};
use crate::{
    shared::{CoreResult, ProgressSink, TraversalProgress, TraversalStage},
    storage::duplicates::{
        DuplicateEntryDeletePolicy, DuplicateFileEntry, DuplicateFilesResult, DuplicateGroup,
        DuplicateGroupBatch, DuplicateGroupKind, DuplicateGroupPage, DuplicateScanLocation,
        DuplicateScanLocationMode,
    },
};

const FULL_HASH_BUFFER_BYTES: usize = 1024 * 1024;
const DUPLICATE_HASH_WORKER_LIMIT: usize = 4;
const HASH_RESULT_QUEUE_PER_WORKER: usize = 2;
const HASH_FAILURE_SAMPLE_LIMIT: usize = 3;
const PROGRESS_INTERVAL_MS: u64 = 120;
/// Sampling only rejects clearly different files with the same size; it never establishes an
/// exact duplicate. Modeling the plan as an internal value lets benchmarks and production share
/// identical offsets and read behavior. Recorded cross-platform benchmark evidence determines the
/// production default.
#[derive(Clone, Copy, Debug)]
enum SamplePlan {
    #[cfg(test)]
    Head4KiB,
    #[cfg(test)]
    HeadTail8KiB,
    HeadMiddleTail16KiB,
    #[cfg(test)]
    HeadMiddleTail256KiB,
}

impl SamplePlan {
    const fn name(self) -> &'static str {
        match self {
            #[cfg(test)]
            Self::Head4KiB => "head-4k",
            #[cfg(test)]
            Self::HeadTail8KiB => "head-tail-8k",
            Self::HeadMiddleTail16KiB => "head-middle-tail-16k",
            #[cfg(test)]
            Self::HeadMiddleTail256KiB => "head-middle-tail-256k",
        }
    }

    const fn segment_bytes(self) -> usize {
        match self {
            #[cfg(test)]
            Self::Head4KiB => 4 * 1024,
            #[cfg(test)]
            Self::HeadTail8KiB => 8 * 1024,
            Self::HeadMiddleTail16KiB => 16 * 1024,
            #[cfg(test)]
            Self::HeadMiddleTail256KiB => 256 * 1024,
        }
    }

    fn offsets(self, file_bytes: u64, sample_bytes: u64) -> [Option<u64>; 3] {
        let tail = file_bytes.saturating_sub(sample_bytes);
        match self {
            #[cfg(test)]
            Self::Head4KiB => [Some(0), None, None],
            #[cfg(test)]
            Self::HeadTail8KiB => [Some(0), Some(tail), None],
            Self::HeadMiddleTail16KiB => [Some(0), Some(tail / 2), Some(tail)],
            #[cfg(test)]
            Self::HeadMiddleTail256KiB => [Some(0), Some(tail / 2), Some(tail)],
        }
    }
}

const PRODUCTION_SAMPLE_PLAN: SamplePlan = SamplePlan::HeadMiddleTail16KiB;

#[derive(Clone, Copy)]
struct DuplicateExecutionOptions {
    sample_plan: SamplePlan,
    worker_override: Option<usize>,
}

#[derive(Clone, Copy)]
struct DuplicateHashCachePolicy {
    minimum_bytes: u64,
    sample_plan: SamplePlan,
    exclusion_fingerprint: [u8; 32],
}

#[derive(Clone, Copy)]
enum HashStage {
    Sample(SamplePlan),
    Full,
}

impl HashStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Sample(_) => "sample",
            Self::Full => "full",
        }
    }
}

struct HashOutcome {
    candidate_index: usize,
    result: Result<blake3::Hash, String>,
    bytes_read: u64,
}

#[derive(Default)]
struct HashStageDiagnostics {
    elapsed_ms: u64,
    worker_count: usize,
    peak_in_flight: usize,
    queue_capacity: usize,
}

struct HashWorkerConfig {
    worker_count: usize,
    identity_worker_count: usize,
    device_classes: String,
}

struct HashPipelineResult {
    full_groups: HashMap<(u64, blake3::Hash), Vec<usize>>,
    fully_sparse_group_hashes: HashSet<blake3::Hash>,
    sample_hashes: Vec<Option<blake3::Hash>>,
    full_hashes: Vec<Option<blake3::Hash>>,
    skipped_count: u64,
    sample_failures: HashFailureDiagnostics,
    full_failures: HashFailureDiagnostics,
    diagnostics: HashPipelineDiagnostics,
}

#[derive(Clone, Copy)]
struct HashPipelinePlan {
    sample_plan: SamplePlan,
    worker_count: usize,
}

#[derive(Default)]
struct FullTaskGroupState {
    remaining_tasks: usize,
    hash_groups: HashMap<(u64, blake3::Hash), Vec<usize>>,
}

#[derive(Default)]
struct HashPipelineDiagnostics {
    sample_hash_ms: u64,
    full_hash_ms: u64,
    sample_hash_bytes: u64,
    full_hash_bytes: u64,
    sample_hash_candidate_count: u64,
    full_hash_candidate_count: u64,
    sample_hash_cache_hit_count: u64,
    full_hash_cache_hit_count: u64,
    cache_candidate_match_count: u64,
    sample_hash_worker_count: u64,
    sample_hash_peak_in_flight: u64,
    full_hash_worker_count: u64,
    full_hash_peak_in_flight: u64,
    hash_result_queue_capacity: u64,
    fully_sparse_candidate_count: u64,
    fully_sparse_group_count: u64,
    fully_sparse_logical_bytes_skipped: u64,
    allocated_range_query_fallback_count: u64,
}

#[derive(Default)]
struct HashFailureDiagnostics {
    count: u64,
    samples: Vec<String>,
}

impl HashFailureDiagnostics {
    fn record(&mut self, path: &Path, error: &str) {
        self.count = self.count.saturating_add(1);
        if self.samples.len() >= HASH_FAILURE_SAMPLE_LIMIT {
            return;
        }
        let diagnostic = mangodisk_platform::diagnostics::text(&error);
        self.samples
            .push(format!("path={} error={diagnostic}", diagnostic_path(path)));
    }

    fn write_log(&self, operation_id: u64, stage: HashStage) {
        if self.count == 0 {
            return;
        }
        // Permission and sharing failures can affect thousands of files. Per-file warnings would
        // create a log storm and slow the scan. Stage summaries keep totals and a few readable
        // path/error samples so support can reproduce a representative failure.
        log::warn!(
            "duplicate_hash_stage_failures operation_id={} stage={} failure_count={} samples={:?}",
            operation_id,
            stage.as_str(),
            self.count,
            self.samples
        );
    }
}

struct DuplicateProgress {
    operation_id: u64,
    callback: Box<dyn Fn(TraversalProgress) + Send + Sync>,
    started_at_ms: u64,
    last_emit_ms: AtomicU64,
    items_scanned: AtomicU64,
    bytes_scanned: AtomicU64,
    completed_steps: AtomicU64,
    total_steps: AtomicU64,
}

#[derive(Debug)]
pub(crate) struct DuplicateScanDiagnostics {
    pub(crate) candidate_strategy: &'static str,
    pub(crate) enumeration_and_size_group_ms: u64,
    pub(crate) group_and_identity_ms: u64,
    pub(crate) sample_hash_ms: u64,
    pub(crate) full_hash_ms: u64,
    pub(crate) allocation_measurement_ms: u64,
    pub(crate) allocation_measurement_fallback_count: u64,
    pub(crate) result_sort_ms: u64,
    pub(crate) directory_aggregation_ms: u64,
    pub(crate) directory_aggregation_candidate_count: u64,
    pub(crate) aggregated_directory_group_count: u64,
    pub(crate) aggregated_file_entry_count: u64,
    pub(crate) sample_plan: &'static str,
    pub(crate) size_group_candidate_count: u64,
    pub(crate) physical_alias_filtered_count: u64,
    pub(crate) identity_unavailable_count: u64,
    pub(crate) identity_worker_count: u64,
    pub(crate) identity_peak_in_flight: u64,
    pub(crate) identity_hint_count: u64,
    pub(crate) identity_hint_verified_count: u64,
    pub(crate) identity_hint_fallback_directory_count: u64,
    pub(crate) sample_hash_candidate_count: u64,
    pub(crate) sample_hash_bytes: u64,
    pub(crate) full_hash_candidate_count: u64,
    pub(crate) full_hash_bytes: u64,
    pub(crate) sample_hash_worker_count: u64,
    pub(crate) sample_hash_peak_in_flight: u64,
    pub(crate) full_hash_worker_count: u64,
    pub(crate) full_hash_peak_in_flight: u64,
    pub(crate) hash_result_queue_capacity: u64,
    pub(crate) fully_sparse_candidate_count: u64,
    pub(crate) fully_sparse_group_count: u64,
    pub(crate) fully_sparse_logical_bytes_skipped: u64,
    pub(crate) allocated_range_query_fallback_count: u64,
    pub(crate) cache_snapshot_found: u64,
    pub(crate) cache_candidate_match_count: u64,
    pub(crate) sample_hash_cache_hit_count: u64,
    pub(crate) full_hash_cache_hit_count: u64,
    pub(crate) cache_load_ms: u64,
    pub(crate) cache_validation_ms: u64,
    pub(crate) cache_fallback_count: u64,
    pub(crate) cache_write_entry_count: u64,
    pub(crate) cache_write_ms: u64,
    pub(crate) streamed_group_batch_count: u64,
    pub(crate) streamed_group_count: u64,
    pub(crate) first_streamed_group_ms: Option<u64>,
}

impl Default for DuplicateScanDiagnostics {
    fn default() -> Self {
        Self {
            candidate_strategy: "generic_recursive_read_dir",
            enumeration_and_size_group_ms: 0,
            group_and_identity_ms: 0,
            sample_hash_ms: 0,
            full_hash_ms: 0,
            allocation_measurement_ms: 0,
            allocation_measurement_fallback_count: 0,
            result_sort_ms: 0,
            directory_aggregation_ms: 0,
            directory_aggregation_candidate_count: 0,
            aggregated_directory_group_count: 0,
            aggregated_file_entry_count: 0,
            sample_plan: PRODUCTION_SAMPLE_PLAN.name(),
            size_group_candidate_count: 0,
            physical_alias_filtered_count: 0,
            identity_unavailable_count: 0,
            identity_worker_count: 0,
            identity_peak_in_flight: 0,
            identity_hint_count: 0,
            identity_hint_verified_count: 0,
            identity_hint_fallback_directory_count: 0,
            sample_hash_candidate_count: 0,
            sample_hash_bytes: 0,
            full_hash_candidate_count: 0,
            full_hash_bytes: 0,
            sample_hash_worker_count: 0,
            sample_hash_peak_in_flight: 0,
            full_hash_worker_count: 0,
            full_hash_peak_in_flight: 0,
            hash_result_queue_capacity: 0,
            fully_sparse_candidate_count: 0,
            fully_sparse_group_count: 0,
            fully_sparse_logical_bytes_skipped: 0,
            allocated_range_query_fallback_count: 0,
            cache_snapshot_found: 0,
            cache_candidate_match_count: 0,
            sample_hash_cache_hit_count: 0,
            full_hash_cache_hit_count: 0,
            cache_load_ms: 0,
            cache_validation_ms: 0,
            cache_fallback_count: 0,
            cache_write_entry_count: 0,
            cache_write_ms: 0,
            streamed_group_batch_count: 0,
            streamed_group_count: 0,
            first_streamed_group_ms: None,
        }
    }
}

impl DuplicateProgress {
    fn new(
        operation_id: u64,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> Self {
        Self {
            operation_id,
            callback: Box::new(callback),
            started_at_ms: now_ms(),
            last_emit_ms: AtomicU64::new(0),
            items_scanned: AtomicU64::new(0),
            bytes_scanned: AtomicU64::new(0),
            completed_steps: AtomicU64::new(0),
            total_steps: AtomicU64::new(0),
        }
    }

    fn visit(&self, stage: TraversalStage, path: &Path, bytes: u64) {
        self.items_scanned.fetch_add(1, Ordering::Relaxed);
        self.bytes_scanned.fetch_add(bytes, Ordering::Relaxed);
        self.emit(stage, path, false, 0, 0);
    }

    fn observe_batch(&self, path: &Path, file_count: u64, bytes: u64) {
        self.items_scanned.fetch_add(file_count, Ordering::Relaxed);
        self.bytes_scanned.fetch_add(bytes, Ordering::Relaxed);
        self.emit(TraversalStage::Analyzing, path, false, 0, 0);
    }

    fn scan_observations(&self) -> (u64, u64) {
        (
            self.items_scanned.load(Ordering::Relaxed),
            self.bytes_scanned.load(Ordering::Relaxed),
        )
    }

    fn restore_scan_observations_for_retry(&self, observations: (u64, u64)) {
        // A multi-root scan may already have completed earlier roots. Restore that stable prefix
        // instead of resetting the whole operation when only the current native root must retry.
        self.items_scanned.store(observations.0, Ordering::Relaxed);
        self.bytes_scanned.store(observations.1, Ordering::Relaxed);
        self.last_emit_ms.store(0, Ordering::Relaxed);
    }

    /// Publishes the complete enumeration counters before another stage changes their meaning.
    ///
    /// Small scans can finish inside the normal progress throttle interval. Without this forced
    /// boundary event, the UI and benchmark evidence would jump from zero directly to hashing and
    /// never expose the logical amount of candidate data that was discovered.
    fn finish_enumeration(&self, path: &Path) {
        self.emit(TraversalStage::Analyzing, path, true, 0, 0);
    }

    /// Starts or extends content verification without losing earlier hash work.
    ///
    /// Enumeration bytes are logical candidate sizes, while hashing bytes are bytes actually read
    /// from files. The first hash stage replaces enumeration counters; later stages add their work
    /// to the same totals so the UI never drops already completed sample reads when full hashing
    /// begins.
    fn extend_hash_stage(&self, path: &Path, additional_steps: u64) {
        let previous_total = self.total_steps.load(Ordering::Relaxed);
        if previous_total == 0 {
            self.bytes_scanned.store(0, Ordering::Relaxed);
            self.completed_steps.store(0, Ordering::Relaxed);
        }
        self.total_steps.store(
            previous_total.saturating_add(additional_steps),
            Ordering::Relaxed,
        );
        self.emit(TraversalStage::HashingFiles, path, true, 0, 0);
    }

    fn complete_hash_step(&self, path: &Path, bytes_read: u64) {
        self.bytes_scanned.fetch_add(bytes_read, Ordering::Relaxed);
        let completed = self.completed_steps.fetch_add(1, Ordering::Relaxed) + 1;
        let total = self.total_steps.load(Ordering::Relaxed);
        self.emit(TraversalStage::HashingFiles, path, completed >= total, 0, 0);
    }

    fn emit(
        &self,
        stage: TraversalStage,
        path: &Path,
        force: bool,
        found_items: u64,
        found_bytes: u64,
    ) {
        let current_ms = now_ms();
        if force {
            self.last_emit_ms.store(current_ms, Ordering::Release);
        } else if self
            .last_emit_ms
            .try_update(Ordering::AcqRel, Ordering::Acquire, |previous_ms| {
                (current_ms.saturating_sub(previous_ms) >= PROGRESS_INTERVAL_MS)
                    .then_some(current_ms)
            })
            .is_err()
        {
            // Multiple hash workers can reach the throttle concurrently. A load followed by a
            // store lets all of them pass and emit duplicate progress events; the CAS grants the
            // current time window to exactly one worker.
            return;
        }
        (self.callback)(TraversalProgress {
            operation_id: self.operation_id,
            current_stage: stage,
            current_path: display_path(path),
            items_scanned: self.items_scanned.load(Ordering::Relaxed),
            bytes_scanned: self.bytes_scanned.load(Ordering::Relaxed),
            completed_steps: self.completed_steps.load(Ordering::Relaxed),
            total_steps: self.total_steps.load(Ordering::Relaxed),
            found_items,
            found_bytes,
            elapsed_ms: current_ms.saturating_sub(self.started_at_ms),
        });
    }
}

struct NativeCandidateCollector<'a> {
    root_ordinal: usize,
    minimum_bytes: u64,
    progress: &'a DuplicateProgress,
    size_groups: &'a mut HashMap<u64, Vec<FileCandidate>>,
    skipped_count: &'a mut u64,
    scanned_file_count: &'a mut u64,
    operation: &'a OperationGuard,
    policy: DuplicateCandidatePolicy,
    exclusions: &'a StorageScanExclusions,
}

impl NativeCandidateCollector<'_> {
    fn enumerate(
        &mut self,
        root: &Path,
    ) -> Result<Option<FastAnalysisSummary>, FastAnalysisScanError> {
        // Keep native output isolated until the platform certifies a complete scan. A failure may
        // arrive after partial records on macOS, and merging those records before generic fallback
        // would duplicate candidates and make exact duplicate totals nondeterministic.
        let mut native_groups = HashMap::<u64, Vec<FileCandidate>>::new();
        let mut native_skipped_count = 0_u64;
        let mut pruned_roots = HashSet::<PathBuf>::new();
        let summary = current_platform().fast_analysis_records(
            FastAnalysisQuery {
                retained_file_limit: 0,
                name_exclusions: self.exclusions.names(),
                excluded_roots: self.exclusions.roots(),
                root,
                purpose: ScanPurpose::DuplicateFiles,
                large_file_minimum_bytes: self.minimum_bytes,
                should_prune_directory: DuplicateCandidatePolicy::should_prune_directory,
            },
            &|| self.operation.cancelled().load(Ordering::Relaxed),
            &mut |path, file_count, bytes| self.progress.observe_batch(path, file_count, bytes),
            &mut |record| match record {
                FastAnalysisRecord::Directory { .. }
                | FastAnalysisRecord::HardLinkedFile { .. }
                | FastAnalysisRecord::AnalysisFile(_) => Ok(()),
                FastAnalysisRecord::LargeFileCandidate(path) => {
                    if self.exclusions.matches(&path) {
                        return Ok(());
                    }
                    if let Some(pruned_root) = pruned_directory_ancestor(root, &path) {
                        pruned_roots.insert(pruned_root);
                        return Ok(());
                    }
                    collect_native_candidate(NativeCandidateRequest {
                        root_ordinal: self.root_ordinal,
                        path,
                        scan_root: root,
                        minimum_bytes: self.minimum_bytes,
                        size_groups: &mut native_groups,
                        skipped_count: &mut native_skipped_count,
                        operation: self.operation,
                        policy: self.policy,
                    })
                }
            },
        )?;
        let Some(summary) = summary else {
            return Ok(None);
        };
        for (bytes, mut candidates) in native_groups {
            self.size_groups
                .entry(bytes)
                .or_default()
                .append(&mut candidates);
        }
        *self.scanned_file_count = self
            .scanned_file_count
            .saturating_add(summary.root_file_count);
        *self.skipped_count = self
            .skipped_count
            .saturating_add(summary.root_skipped_count)
            .saturating_add(native_skipped_count)
            .saturating_add(u64::try_from(pruned_roots.len()).unwrap_or(u64::MAX));
        Ok(Some(summary))
    }
}

pub struct DuplicateFileService;

impl DuplicateFileService {
    pub fn find_with_progress(
        roots: Vec<String>,
        minimum_bytes: u64,
        callback: impl ProgressSink,
    ) -> CoreResult<DuplicateFilesResult> {
        Self::find_with_diagnostics(roots, minimum_bytes, move |progress| {
            callback.report(progress);
        })
        .map(|(result, _)| result)
    }

    /// Product entry point used by Tauri. Starting a scan invalidates the old paginated session.
    /// On success, only the first page is copied into the WebView; the complete bounded result
    /// remains in Rust and later pages are loaded on demand.
    pub fn find_paged_with_progress(
        roots: Vec<String>,
        minimum_bytes: u64,
        progress_callback: impl ProgressSink,
        group_callback: impl Fn(DuplicateGroupBatch) + Send + Sync + 'static,
    ) -> CoreResult<DuplicateFilesResult> {
        Self::find_paged_with_protected_roots(
            roots,
            Vec::new(),
            Vec::new(),
            minimum_bytes,
            progress_callback,
            group_callback,
        )
    }

    /// Scans typed locations while retaining protected descendants even when a selected parent
    /// already covers their traversal. Protection changes deletion authority, not scan coverage.
    pub fn find_paged_with_locations(
        locations: Vec<DuplicateScanLocation>,
        minimum_bytes: u64,
        progress_callback: impl ProgressSink,
        group_callback: impl Fn(DuplicateGroupBatch) + Send + Sync + 'static,
    ) -> CoreResult<DuplicateFilesResult> {
        let roots = locations
            .iter()
            .map(|location| location.path.clone())
            .collect::<Vec<_>>();
        let protected_roots = locations
            .into_iter()
            .filter(|location| location.mode == DuplicateScanLocationMode::Protected)
            .map(|location| location.path)
            .collect::<Vec<_>>();
        Self::find_paged_with_protected_roots(
            roots,
            protected_roots,
            Vec::new(),
            minimum_bytes,
            progress_callback,
            group_callback,
        )
    }

    pub fn find_paged_with_locations_and_exclusions(
        locations: Vec<DuplicateScanLocation>,
        excluded_paths: impl Into<ScanExclusionOptions>,
        minimum_bytes: u64,
        progress_callback: impl ProgressSink,
        group_callback: impl Fn(DuplicateGroupBatch) + Send + Sync + 'static,
    ) -> CoreResult<DuplicateFilesResult> {
        let excluded_paths = excluded_paths.into();
        let roots = locations
            .iter()
            .map(|location| location.path.clone())
            .collect::<Vec<_>>();
        let protected_roots = locations
            .into_iter()
            .filter(|location| location.mode == DuplicateScanLocationMode::Protected)
            .map(|location| location.path)
            .collect::<Vec<_>>();
        Self::find_paged_with_protected_roots(
            roots,
            protected_roots,
            excluded_paths,
            minimum_bytes,
            progress_callback,
            group_callback,
        )
    }

    fn find_paged_with_protected_roots(
        roots: Vec<String>,
        protected_roots: Vec<String>,
        excluded_paths: impl Into<ScanExclusionOptions>,
        minimum_bytes: u64,
        progress_callback: impl ProgressSink,
        group_callback: impl Fn(DuplicateGroupBatch) + Send + Sync + 'static,
    ) -> CoreResult<DuplicateFilesResult> {
        let excluded_paths = excluded_paths.into();
        clear_result_session()?;
        let (result, _) = Self::find_with_protection_stream_diagnostics(
            roots,
            protected_roots,
            excluded_paths,
            minimum_bytes,
            move |progress| progress_callback.report(progress),
            group_callback,
        )?;
        Ok(publish_result_session(result)?)
    }

    pub fn page(scan_id: u64, offset: u64, limit: u64) -> Result<DuplicateGroupPage, String> {
        result_page(scan_id, offset, limit)
    }

    pub fn resolve_open_target(scan_id: u64, selected_path: String) -> CoreResult<String> {
        Ok(resolve_open_target(scan_id, &selected_path)?)
    }

    /// Permanently deletes files only after binding every candidate to the current duplicate scan.
    ///
    /// Selection happens in the WebView, so Core independently verifies membership and rejects a
    /// request that would remove every file from an affected group.
    pub fn delete_files_permanently(
        scan_id: u64,
        candidates: Vec<PermanentDeleteCandidate>,
    ) -> CoreResult<PermanentDeleteBatchResult> {
        let candidate_count = candidates.len();
        let candidates = validate_permanent_delete_candidates(scan_id, candidates).inspect_err(
            |error| {
                log::warn!(
                    "duplicate_delete_validation_failed scan_id={} candidate_count={} reason={} error={}",
                    scan_id,
                    candidate_count,
                    duplicate_delete_validation_reason(error),
                    mangodisk_platform::diagnostics::text(&error)
                );
            },
        )?;
        let operation = OperationGuard::start(CoordinatedOperationKind::PermanentDelete)?;
        let started = Instant::now();
        let started_at_ms = now_ms();
        let requested_count = candidates.len();
        let expected_bytes = candidates
            .iter()
            .map(|candidate| candidate.candidate.expected_bytes)
            .sum();
        let selected_paths = candidates
            .iter()
            .map(|candidate| candidate.candidate.path.clone())
            .collect::<Vec<_>>();
        let path_sample = candidates
            .iter()
            .take(3)
            .map(|candidate| diagnostic_path(Path::new(&candidate.candidate.path)))
            .collect::<Vec<_>>();
        let mut result = PermanentDeleteBatchResult {
            removed_paths: Vec::new(),
            failed: Vec::new(),
            released_bytes: 0,
        };

        for validated in candidates {
            let candidate = validated.candidate.clone();
            let deletion = match validated.kind {
                DuplicateGroupKind::File => delete_file_candidate_permanently(&candidate)
                    .map(|(target, usage)| (target, usage, 1, false))
                    .map_err(|error| error.to_string()),
                DuplicateGroupKind::Directory => {
                    delete_duplicate_directory_candidate(&validated, &operation)
                }
            };
            match deletion {
                Ok((target, usage, file_count, is_directory)) => {
                    result.released_bytes =
                        result.released_bytes.saturating_add(usage.allocated_bytes);
                    result.removed_paths.push(candidate.path);
                    cache::remove_entry(&target, usage, file_count, is_directory);
                    hash_cache::invalidate_containing(&target);
                }
                Err(error) => result.failed.push(PermanentDeleteFailure {
                    path: candidate.path,
                    message: error,
                }),
            }
        }
        #[cfg(windows)]
        crate::filesystem::permanent_delete_diagnostics::log_failed_volume_samples(
            "duplicates",
            operation.id(),
            scan_id,
            &result.failed,
        );
        if !result.removed_paths.is_empty() {
            synchronize_result_session(scan_id, result.removed_paths.clone())?;
        }
        let history_record = file_cleanup_record(
            format!("duplicate-file-cleanup-{}-{}", operation.id(), now_ms()),
            FileCleanupHistoryCategory::DuplicateFiles,
            started_at_ms,
            now_ms(),
            selected_paths,
            expected_bytes,
            &result,
        );
        if let Err(error) = HistoryService::append(history_record) {
            log::warn!(
                "duplicate_file_history_save_failed operation_id={} error={}",
                operation.id(),
                mangodisk_platform::diagnostics::text(&error)
            );
        }
        log::info!(
            "duplicate_permanent_delete_finished operation_id={} scan_id={} requested_count={} path_sample={:?} selected_logical_bytes={} removed_count={} failed_count={} released_allocated_bytes={} elapsed_ms={}",
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

    pub(crate) fn find_with_diagnostics(
        roots: Vec<String>,
        minimum_bytes: u64,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<(DuplicateFilesResult, DuplicateScanDiagnostics)> {
        Self::find_with_stream_diagnostics(roots, minimum_bytes, callback, |_| {})
    }

    pub(crate) fn find_with_stream_diagnostics(
        roots: Vec<String>,
        minimum_bytes: u64,
        progress_callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
        group_callback: impl Fn(DuplicateGroupBatch) + Send + Sync + 'static,
    ) -> CoreResult<(DuplicateFilesResult, DuplicateScanDiagnostics)> {
        Self::find_with_protection_stream_diagnostics(
            roots,
            Vec::new(),
            Vec::new(),
            minimum_bytes,
            progress_callback,
            group_callback,
        )
    }

    fn find_with_protection_stream_diagnostics(
        roots: Vec<String>,
        protected_roots: Vec<String>,
        excluded_paths: impl Into<ScanExclusionOptions>,
        minimum_bytes: u64,
        progress_callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
        group_callback: impl Fn(DuplicateGroupBatch) + Send + Sync + 'static,
    ) -> CoreResult<(DuplicateFilesResult, DuplicateScanDiagnostics)> {
        let excluded_paths = excluded_paths.into();
        Self::find_with_sample_plan_stream_diagnostics(
            roots,
            protected_roots,
            excluded_paths,
            minimum_bytes,
            PRODUCTION_SAMPLE_PLAN,
            progress_callback,
            group_callback,
        )
    }

    fn find_with_sample_plan_stream_diagnostics(
        roots: Vec<String>,
        protected_roots: Vec<String>,
        excluded_paths: impl Into<ScanExclusionOptions>,
        minimum_bytes: u64,
        sample_plan: SamplePlan,
        progress_callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
        group_callback: impl Fn(DuplicateGroupBatch) + Send + Sync + 'static,
    ) -> CoreResult<(DuplicateFilesResult, DuplicateScanDiagnostics)> {
        let excluded_paths = excluded_paths.into();
        Self::find_with_options_stream_diagnostics(
            roots,
            protected_roots,
            excluded_paths,
            minimum_bytes,
            DuplicateExecutionOptions {
                sample_plan,
                worker_override: None,
            },
            progress_callback,
            group_callback,
        )
    }

    #[cfg(test)]
    fn find_with_options_diagnostics(
        roots: Vec<String>,
        minimum_bytes: u64,
        sample_plan: SamplePlan,
        worker_override: Option<usize>,
        callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
    ) -> CoreResult<(DuplicateFilesResult, DuplicateScanDiagnostics)> {
        Self::find_with_options_stream_diagnostics(
            roots,
            Vec::new(),
            Vec::new(),
            minimum_bytes,
            DuplicateExecutionOptions {
                sample_plan,
                worker_override,
            },
            callback,
            |_| {},
        )
    }

    fn find_with_options_stream_diagnostics(
        roots: Vec<String>,
        protected_roots: Vec<String>,
        excluded_paths: impl Into<ScanExclusionOptions>,
        minimum_bytes: u64,
        options: DuplicateExecutionOptions,
        progress_callback: impl Fn(TraversalProgress) + Send + Sync + 'static,
        group_callback: impl Fn(DuplicateGroupBatch) + Send + Sync + 'static,
    ) -> CoreResult<(DuplicateFilesResult, DuplicateScanDiagnostics)> {
        let excluded_paths = excluded_paths.into();
        let DuplicateExecutionOptions {
            sample_plan,
            worker_override,
        } = options;
        let operation = OperationGuard::start(CoordinatedOperationKind::DuplicateFiles)?;
        let started = Instant::now();
        let mut diagnostics = DuplicateScanDiagnostics {
            sample_plan: sample_plan.name(),
            ..DuplicateScanDiagnostics::default()
        };
        let roots = normalize_roots(roots)?;
        if roots.is_empty() {
            return Err(crate::shared::CoreError::invalid_input(
                "at least one duplicate-file scan root is required",
            ));
        }
        let protected_roots = normalize_protected_roots(protected_roots, &roots)?;
        let exclusions = roots
            .iter()
            .map(|root| {
                let mut exclusions = StorageScanExclusions::resolve_options(root, &excluded_paths)?;
                exclusions.delegate_selected_descendants(root, &roots);
                Ok(exclusions)
            })
            .collect::<CoreResult<Vec<_>>>()?;
        let exclusion_fingerprint = exclusions[0].configuration_fingerprint();
        // `worker_override` is limited to unit tests and explicit scheduler diagnostics. Those
        // paths must observe actual worker counts and read volume; a second-run memory-cache
        // hit with zero workers would hide the behavior under test. Product and release benchmark
        // callers pass `None`, so they retain the safe cache.
        let cache_enabled = worker_override.is_none();
        let worker_config = duplicate_hash_worker_config(&roots, worker_override);
        log::info!(
            "duplicate_hash_scheduler_configured operation_id={} hash_workers={} identity_workers={} device_classes={}",
            operation.id(),
            worker_config.worker_count,
            worker_config.identity_worker_count,
            worker_config.device_classes
        );

        let progress = DuplicateProgress::new(operation.id(), progress_callback);
        let mut group_stream = DuplicateGroupStream::new(operation.id(), group_callback);
        progress.emit(TraversalStage::Analyzing, &roots[0], true, 0, 0);
        // Capture the new cursor before enumeration. Digests from this real scan can become the
        // next cache generation only if history remains clean from this point through hashing.
        // Failure to capture a cursor does not affect the current real result.
        let fresh_cache_roots = cache_enabled
            .then(|| capture_duplicate_cache_roots(&roots, &operation))
            .flatten();
        // Snapshot lookup depends only on normalized roots and options, so complete it before
        // enumeration. History validation can then overlap enumeration. An untrusted validation
        // outcome still prevents cached facts from entering the hash pipeline.
        let snapshot = if cache_enabled {
            match hash_cache::find_snapshot(
                &roots,
                minimum_bytes,
                sample_plan.name(),
                exclusion_fingerprint,
            ) {
                Ok((snapshot, elapsed_ms)) => {
                    diagnostics.cache_load_ms = elapsed_ms;
                    snapshot
                }
                Err(error) => {
                    log_cache_error(operation.id(), "load", &error);
                    None
                }
            }
        } else {
            None
        };
        if snapshot.is_some() {
            diagnostics.cache_snapshot_found = 1;
        }
        let mut pending_validation = if let Some(snapshot) = &snapshot {
            PendingDuplicateCacheValidation::start(
                snapshot.roots.clone(),
                &operation,
                CacheValidationPhase::ExistingSnapshotStart,
            )
        } else if current_platform().filesystem_change_monitor_is_continuous() {
            // FSEvents needs a continuous monitor before enumeration to cover the whole scan
            // window. The Windows USN monitor is a one-shot history query; a fresh token at the
            // end already establishes the upper bound, so querying it here only delays first scan.
            fresh_cache_roots.clone().and_then(|roots| {
                PendingDuplicateCacheValidation::start(
                    roots,
                    &operation,
                    CacheValidationPhase::FreshSnapshotStart,
                )
            })
        } else {
            None
        };
        let mut size_groups = HashMap::<u64, Vec<FileCandidate>>::new();
        let mut skipped_count = 0_u64;
        let mut scanned_file_count = 0_u64;
        let enumeration_started = Instant::now();
        for (root_ordinal, root) in roots.iter().enumerate() {
            let exclusions = &exclusions[root_ordinal];
            log::info!(
                "duplicate_scan_root_started operation_id={} platform={} root={} root_index={} root_count={} requested_path_exclusions={} active_path_exclusions={} unavailable_exclusions={} out_of_scope_exclusions={}",
                operation.id(),
                current_platform().os_name(),
                diagnostic_path(root),
                root_ordinal,
                roots.len(),
                exclusions.requested_count(),
                exclusions.active_count(),
                exclusions.unavailable_count(),
                exclusions.out_of_scope_count()
            );
            if exclusions.matches(root) {
                log::info!(
                    "duplicate_scan_root_excluded operation_id={} root={} outcome=pruned",
                    operation.id(),
                    diagnostic_path(root)
                );
                continue;
            }
            let policy = DuplicateCandidatePolicy::for_scan_root(root);
            let progress_before_root = progress.scan_observations();
            match (NativeCandidateCollector {
                root_ordinal,
                minimum_bytes: minimum_bytes.max(1),
                progress: &progress,
                size_groups: &mut size_groups,
                skipped_count: &mut skipped_count,
                scanned_file_count: &mut scanned_file_count,
                operation: &operation,
                policy,
                exclusions,
            })
            .enumerate(root)
            {
                Ok(Some(summary)) => {
                    diagnostics.candidate_strategy = summary.strategy;
                    log::info!(
                        "duplicate_candidate_enumeration_native operation_id={} platform={} root={} strategy={} files={} candidates={} directories={} pages={}",
                        operation.id(),
                        current_platform().os_name(),
                        diagnostic_path(root),
                        summary.strategy,
                        summary.root_file_count,
                        summary.candidate_count,
                        summary.directory_count,
                        summary.page_count
                    );
                    continue;
                }
                Ok(None) => {
                    log::info!(
                        "duplicate_candidate_enumeration_fallback operation_id={} platform={} root={} reason=unsupported",
                        operation.id(),
                        current_platform().os_name(),
                        diagnostic_path(root)
                    );
                }
                Err(FastAnalysisScanError::Cancelled) => {
                    return Err(crate::shared::CoreError::operation_cancelled());
                }
                Err(FastAnalysisScanError::Busy) => {
                    operation.defer();
                    return Err(crate::shared::CoreError::scan_resources_releasing());
                }
                Err(FastAnalysisScanError::Platform(error)) => {
                    log::warn!(
                        "duplicate_candidate_enumeration_fallback operation_id={} platform={} root={} reason=native_failed error={}",
                        operation.id(),
                        current_platform().os_name(),
                        diagnostic_path(root),
                        mangodisk_platform::diagnostics::text(&error)
                    );
                }
                Err(FastAnalysisScanError::Consumer(error)) => {
                    if operation.cancelled().load(Ordering::Relaxed) {
                        return Err(crate::shared::CoreError::operation_cancelled());
                    }
                    log::warn!(
                        "duplicate_candidate_enumeration_fallback operation_id={} platform={} root={} reason=consumer_failed error={}",
                        operation.id(),
                        current_platform().os_name(),
                        diagnostic_path(root),
                        mangodisk_platform::diagnostics::text(&error)
                    );
                }
            }
            // Native streams may report progress before failing. The candidate collector keeps
            // partial records isolated; restore the completed-root prefix before recursively
            // retrying only this root.
            progress.restore_scan_observations_for_retry(progress_before_root);
            let visit = |stage, path: &Path, bytes| progress.visit(stage, path, bytes);
            CandidateEnumeration::new(CandidateEnumerationRequest {
                root_ordinal,
                minimum_bytes: minimum_bytes.max(1),
                visit: &visit,
                size_groups: &mut size_groups,
                skipped_count: &mut skipped_count,
                scanned_file_count: &mut scanned_file_count,
                operation: &operation,
                policy,
                exclusions,
            })
            .scan(root, root)?;
        }
        progress.finish_enumeration(&roots[roots.len() - 1]);
        diagnostics.enumeration_and_size_group_ms =
            enumeration_started.elapsed().as_millis() as u64;
        diagnostics.size_group_candidate_count = size_groups
            .values()
            .filter(|items| items.len() > 1)
            .map(|items| items.len() as u64)
            .sum();

        let identity_filter_started = Instant::now();
        let mut sorted_size_groups = size_groups
            .into_iter()
            .filter(|(_, items)| items.len() > 1)
            .collect::<Vec<_>>();
        sorted_size_groups.sort_by_key(|(bytes, _)| *bytes);
        let size_candidates = sorted_size_groups
            .into_iter()
            .flat_map(|(_, mut candidates)| {
                candidates.sort_by(|left, right| left.path.cmp(&right.path));
                candidates
            })
            .collect::<Vec<_>>();
        // Windows metadata lacks stable identity. Resolve candidate parents through native batch
        // hints first, then use one bounded per-file fallback pool rather than creating threads for
        // thousands of tiny size groups. Unix candidates already carry metadata-based identity and
        // stay on the allocation-free serial path.
        let identity_cancellation_flag = operation.cancellation_flag();
        let identity_cancellation =
            PlatformCancellation::new(move || identity_cancellation_flag.load(Ordering::Relaxed));
        let filtered = remove_physical_aliases(
            size_candidates,
            worker_config.identity_worker_count,
            &identity_cancellation,
            |path| {
                operation
                    .ensure_not_cancelled()
                    .map_err(|error| error.to_string())?;
                progress.emit(TraversalStage::ValidatingFiles, path, false, 0, 0);
                Ok(())
            },
        )?;
        diagnostics.physical_alias_filtered_count =
            u64::try_from(filtered.alias_count).unwrap_or(u64::MAX);
        diagnostics.identity_unavailable_count =
            u64::try_from(filtered.unavailable_count).unwrap_or(u64::MAX);
        diagnostics.identity_worker_count =
            u64::try_from(filtered.worker_count).unwrap_or(u64::MAX);
        diagnostics.identity_peak_in_flight =
            u64::try_from(filtered.peak_in_flight).unwrap_or(u64::MAX);
        diagnostics.identity_hint_count = u64::try_from(filtered.hint_count).unwrap_or(u64::MAX);
        diagnostics.identity_hint_verified_count =
            u64::try_from(filtered.verified_hint_count).unwrap_or(u64::MAX);
        diagnostics.identity_hint_fallback_directory_count =
            u64::try_from(filtered.hint_fallback_directory_count).unwrap_or(u64::MAX);
        if filtered.hint_fallback_directory_count > 0 {
            let failure_samples = filtered
                .hint_failure_samples
                .iter()
                .map(|sample| format!("{:?}:{}", sample.code, sample.diagnostic_detail))
                .collect::<Vec<_>>();
            log::warn!(
                "duplicate_identity_hint_fallback operation_id={} fallback_directories={} failure_samples={:?}",
                operation.id(),
                filtered.hint_fallback_directory_count,
                failure_samples
            );
        }
        skipped_count = skipped_count
            .saturating_add(u64::try_from(filtered.unavailable_count).unwrap_or(u64::MAX));
        let remaining_size_counts = filtered.candidates.iter().fold(
            HashMap::<u64, usize>::new(),
            |mut counts, candidate| {
                *counts.entry(candidate.bytes).or_default() += 1;
                counts
            },
        );
        let candidates = filtered
            .candidates
            .into_iter()
            .filter(|candidate| {
                remaining_size_counts
                    .get(&candidate.bytes)
                    .is_some_and(|count| *count > 1)
            })
            .collect::<Vec<_>>();
        diagnostics.group_and_identity_ms = elapsed_ms(identity_filter_started);

        let mut existing_validation = if snapshot.is_some() {
            let validation_started = Instant::now();
            let validation = match pending_validation.take() {
                Some(pending) => pending.finish(&operation)?,
                None => None,
            };
            // This metric is foreground time spent waiting for background validation, not the
            // watcher's wall-clock lifetime. Its background work already overlapped enumeration
            // and must not be counted again in stage duration.
            diagnostics.cache_validation_ms = diagnostics
                .cache_validation_ms
                .saturating_add(elapsed_ms(validation_started));
            validation
        } else {
            None
        };

        // A cached outcome remains speculative until final history verification and cannot enter
        // the final DTO. If anything changed during the scan, discard the entire pipeline and run
        // the current candidates fresh once.
        let validated_cache = snapshot
            .as_ref()
            .filter(|_| existing_validation.is_some())
            .map(|snapshot| snapshot.files.as_ref());
        // Directory aggregation owns final result shape. Streaming raw file groups here would
        // briefly show groups that disappear once their exact parent directories are folded.
        let mut pipeline = {
            let mut stream_group = |_, _| {};
            execute_hash_pipeline(
                &candidates,
                HashPipelinePlan {
                    sample_plan,
                    worker_count: worker_config.worker_count,
                },
                validated_cache,
                &operation,
                &progress,
                &mut stream_group,
            )?
        };
        let used_cache_outcomes = pipeline.diagnostics.sample_hash_cache_hit_count > 0
            || pipeline.diagnostics.full_hash_cache_hit_count > 0;
        let existing_cache_still_valid = if let Some(validation) = existing_validation.take() {
            let validation_started = Instant::now();
            let valid = finish_duplicate_cache_validation(
                &validation,
                &operation,
                CacheValidationPhase::ExistingSnapshotEnd,
            )?;
            diagnostics.cache_validation_ms = diagnostics
                .cache_validation_ms
                .saturating_add(elapsed_ms(validation_started));
            valid
        } else {
            false
        };
        let accepted_existing_cache = existing_cache_still_valid && snapshot.is_some();
        if used_cache_outcomes && !accepted_existing_cache {
            diagnostics.cache_fallback_count = 1;
            log::info!(
                "duplicate_hash_cache_fallback operation_id={} reason=validation_failed",
                operation.id()
            );
            let mut stream_group = |_, _| {};
            pipeline = execute_hash_pipeline(
                &candidates,
                HashPipelinePlan {
                    sample_plan,
                    worker_count: worker_config.worker_count,
                },
                None,
                &operation,
                &progress,
                &mut stream_group,
            )?;
        }

        apply_pipeline_diagnostics(&mut diagnostics, &pipeline.diagnostics);
        pipeline
            .sample_failures
            .write_log(operation.id(), HashStage::Sample(sample_plan));
        pipeline
            .full_failures
            .write_log(operation.id(), HashStage::Full);
        skipped_count = skipped_count.saturating_add(pipeline.skipped_count);

        let cache_complete_hit = accepted_existing_cache
            && pipeline.diagnostics.sample_hash_cache_hit_count
                == pipeline.diagnostics.sample_hash_candidate_count
            && pipeline.diagnostics.full_hash_cache_hit_count
                == pipeline.diagnostics.full_hash_candidate_count;
        let cache_roots_to_publish = if cache_complete_hit {
            None
        } else if snapshot.is_none() {
            // The continuous macOS monitor started before enumeration. The one-shot Windows USN
            // monitor obtains its upper bound from the fresh token here. Both cover the complete
            // scan window, and only a clean final verification can publish this run's digests.
            let validation_started = Instant::now();
            let validation = match pending_validation.take() {
                Some(pending) => pending.finish(&operation)?,
                None => fresh_cache_roots.as_ref().map_or(Ok(None), |roots| {
                    start_duplicate_cache_validation(
                        roots,
                        &operation,
                        CacheValidationPhase::FreshSnapshotEnd,
                    )
                })?,
            };
            let roots = match validation {
                Some(validation)
                    if finish_duplicate_cache_validation(
                        &validation,
                        &operation,
                        CacheValidationPhase::FreshSnapshotEnd,
                    )? =>
                {
                    Some(validation.roots)
                }
                Some(_) | None => None,
            };
            diagnostics.cache_validation_ms = diagnostics
                .cache_validation_ms
                .saturating_add(elapsed_ms(validation_started));
            roots
        } else if let Some(roots) = fresh_cache_roots {
            // Advance to this run's cursor when the old snapshot is invalid or only partially
            // useful. Reusing the old cursor would not authorize stale hashes, but it would reach
            // a Journal history gap sooner on high-churn volumes.
            let validation_started = Instant::now();
            let validation = start_duplicate_cache_validation(
                &roots,
                &operation,
                CacheValidationPhase::FreshSnapshotEnd,
            )?;
            diagnostics.cache_validation_ms = diagnostics
                .cache_validation_ms
                .saturating_add(elapsed_ms(validation_started));
            validation.map(|validation| validation.roots)
        } else {
            None
        };
        if !cache_complete_hit {
            if let Some(cache_roots) = cache_roots_to_publish {
                publish_duplicate_hash_cache(
                    &cache_roots,
                    DuplicateHashCachePolicy {
                        minimum_bytes,
                        sample_plan,
                        exclusion_fingerprint,
                    },
                    &candidates,
                    &pipeline,
                    &operation,
                    &mut diagnostics,
                )?;
            }
        }

        let excluded_directory_hashes = pipeline
            .fully_sparse_group_hashes
            .iter()
            .map(|hash| hash.to_hex().to_string())
            .collect::<HashSet<_>>();
        let full_groups = pipeline.full_groups;
        let mut groups = Vec::new();
        let allocation_measurement_started = Instant::now();
        for (key, candidate_indices) in full_groups.into_iter().filter(|(_, items)| items.len() > 1)
        {
            if let Some(group) = build_duplicate_group(
                &candidates,
                key,
                candidate_indices,
                &mut diagnostics.allocation_measurement_fallback_count,
            ) {
                groups.push(group);
            }
        }
        diagnostics.allocation_measurement_ms = elapsed_ms(allocation_measurement_started);
        let directory_aggregation_started = Instant::now();
        let aggregation =
            aggregate_exact_directories(&roots, groups, &excluded_directory_hashes, &operation)?;
        diagnostics.directory_aggregation_ms = elapsed_ms(directory_aggregation_started);
        diagnostics.directory_aggregation_candidate_count =
            aggregation.diagnostics.candidate_directory_count;
        diagnostics.aggregated_directory_group_count =
            aggregation.diagnostics.aggregated_directory_group_count;
        diagnostics.aggregated_file_entry_count =
            aggregation.diagnostics.aggregated_file_entry_count;
        let mut groups = aggregation.groups;
        let (protected_group_count, protected_entry_count) =
            apply_protection_policy(&mut groups, &protected_roots);
        normalize_group_paths_for_output(&mut groups);
        let sort_started = Instant::now();
        groups.sort_by(|left, right| {
            right
                .reclaimable_bytes
                .cmp(&left.reclaimable_bytes)
                .then_with(|| left.hash.cmp(&right.hash))
        });
        let result_sort_elapsed = sort_started.elapsed();
        diagnostics.result_sort_ms = result_sort_elapsed.as_millis() as u64;
        let duplicate_file_count = groups
            .iter()
            .map(|group| {
                u64::try_from(group.entries.len())
                    .unwrap_or(u64::MAX)
                    .saturating_mul(group.file_count_per_entry)
            })
            .sum();
        let total_duplicate_bytes = groups
            .iter()
            .map(DuplicateGroup::total_allocated_bytes)
            .fold(0_u64, u64::saturating_add);
        let reclaimable_bytes = groups
            .iter()
            .map(|group| group.reclaimable_bytes)
            .fold(0_u64, u64::saturating_add);
        let total_group_count = groups.len() as u64;
        let returned_group_count = groups.len() as u64;
        // Exact directory aggregation and cache authorization are complete. Publish only stable
        // final groups, while the authoritative session still retains pagination outside WebView.
        group_stream.push(
            groups
                .iter()
                .take(DUPLICATE_RESULT_PAGE_SIZE)
                .cloned()
                .collect(),
        );
        group_stream.finish();
        let (streamed_batch_count, streamed_group_count, first_streamed_group_ms) =
            group_stream.metrics();
        diagnostics.streamed_group_batch_count = streamed_batch_count;
        diagnostics.streamed_group_count = streamed_group_count;
        diagnostics.first_streamed_group_ms = first_streamed_group_ms;
        let final_progress_stage =
            if diagnostics.sample_hash_worker_count > 0 || diagnostics.full_hash_worker_count > 0 {
                TraversalStage::HashingFiles
            } else {
                TraversalStage::Analyzing
            };
        progress.emit(
            final_progress_stage,
            &roots[0],
            true,
            duplicate_file_count,
            reclaimable_bytes,
        );
        let root_sample = roots
            .iter()
            .take(3)
            .map(|path| diagnostic_path(path))
            .collect::<Vec<_>>();
        log::info!(
            "duplicate_scan_finished operation_id={} root_count={} protected_root_count={} protected_group_count={} protected_entry_count={} root_sample={:?} candidate_strategy={} scanned_files={} duplicate_groups={} returned_groups={} duplicate_files={} reclaimable_allocated_bytes={} skipped_count={} enumeration_ms={} group_identity_ms={} identity_hints={} identity_hints_verified={} identity_hint_fallback_directories={} identity_workers={} identity_peak_in_flight={} sample_hash_ms={} full_hash_ms={} allocation_measurement_ms={} allocation_measurement_fallbacks={} result_sort_ms={} sample_plan={} size_candidates={} aliases_filtered={} identity_unavailable={} sample_candidates={} sample_read_bytes={} sample_workers={} sample_peak_in_flight={} full_candidates={} full_read_bytes={} full_workers={} full_peak_in_flight={} hash_queue_capacity={} fully_sparse_candidates={} fully_sparse_groups={} fully_sparse_logical_bytes_skipped={} allocated_range_query_fallbacks={} cache_snapshot_found={} cache_candidate_matches={} sample_cache_hits={} full_cache_hits={} cache_load_ms={} cache_validation_ms={} cache_fallbacks={} cache_write_entries={} cache_write_ms={} directory_candidates={} directory_groups={} aggregated_file_entries={} directory_aggregation_ms={} stream_batches={} streamed_groups={} first_stream_group_ms={:?} elapsed_ms={}",
            operation.id(),
            roots.len(),
            protected_roots.len(),
            protected_group_count,
            protected_entry_count,
            root_sample,
            diagnostics.candidate_strategy,
            scanned_file_count,
            total_group_count,
            returned_group_count,
            duplicate_file_count,
            reclaimable_bytes,
            skipped_count,
            diagnostics.enumeration_and_size_group_ms,
            diagnostics.group_and_identity_ms,
            diagnostics.identity_hint_count,
            diagnostics.identity_hint_verified_count,
            diagnostics.identity_hint_fallback_directory_count,
            diagnostics.identity_worker_count,
            diagnostics.identity_peak_in_flight,
            diagnostics.sample_hash_ms,
            diagnostics.full_hash_ms,
            diagnostics.allocation_measurement_ms,
            diagnostics.allocation_measurement_fallback_count,
            diagnostics.result_sort_ms,
            diagnostics.sample_plan,
            diagnostics.size_group_candidate_count,
            diagnostics.physical_alias_filtered_count,
            diagnostics.identity_unavailable_count,
            diagnostics.sample_hash_candidate_count,
            diagnostics.sample_hash_bytes,
            diagnostics.sample_hash_worker_count,
            diagnostics.sample_hash_peak_in_flight,
            diagnostics.full_hash_candidate_count,
            diagnostics.full_hash_bytes,
            diagnostics.full_hash_worker_count,
            diagnostics.full_hash_peak_in_flight,
            diagnostics.hash_result_queue_capacity,
            diagnostics.fully_sparse_candidate_count,
            diagnostics.fully_sparse_group_count,
            diagnostics.fully_sparse_logical_bytes_skipped,
            diagnostics.allocated_range_query_fallback_count,
            diagnostics.cache_snapshot_found,
            diagnostics.cache_candidate_match_count,
            diagnostics.sample_hash_cache_hit_count,
            diagnostics.full_hash_cache_hit_count,
            diagnostics.cache_load_ms,
            diagnostics.cache_validation_ms,
            diagnostics.cache_fallback_count,
            diagnostics.cache_write_entry_count,
            diagnostics.cache_write_ms,
            diagnostics.directory_aggregation_candidate_count,
            diagnostics.aggregated_directory_group_count,
            diagnostics.aggregated_file_entry_count,
            diagnostics.directory_aggregation_ms,
            diagnostics.streamed_group_batch_count,
            diagnostics.streamed_group_count,
            diagnostics.first_streamed_group_ms,
            started.elapsed().as_millis()
        );
        operation.complete();
        Ok((
            DuplicateFilesResult {
                scan_id: operation.id(),
                roots: roots.iter().map(|path| display_path(path)).collect(),
                protected_roots: protected_roots
                    .iter()
                    .map(|path| display_path(path))
                    .collect(),
                scanned_at_ms: now_ms(),
                scanned_file_count,
                skipped_count,
                duplicate_file_count,
                total_duplicate_bytes,
                reclaimable_bytes,
                total_group_count,
                returned_group_count,
                truncated: returned_group_count < total_group_count,
                groups,
            },
            diagnostics,
        ))
    }

    pub fn cancel() {
        OperationGuard::cancel(CoordinatedOperationKind::DuplicateFiles);
    }
}

fn delete_duplicate_directory_candidate(
    validated: &ValidatedDuplicateDeleteCandidate,
    operation: &OperationGuard,
) -> Result<(PathBuf, FileSpaceUsage, u64, bool), String> {
    let candidate = &validated.candidate;
    let requested_target = PathBuf::from(&candidate.path);
    let prepared =
        prepare_path_for_permanent_delete(&requested_target).map_err(|error| error.to_string())?;
    current_platform()
        .validate_path_no_links(&requested_target)
        .map_err(|error| error.to_string())?;
    let metadata = prepared.metadata();
    if !metadata.is_dir() || current_platform().is_link_like(metadata) {
        return Err("only regular directories can be permanently deleted".to_string());
    }
    if candidate.expected_modified_at_ms.is_none()
        || modified_ms(metadata) != candidate.expected_modified_at_ms
    {
        return Err("the directory changed after scanning".to_string());
    }

    let target = current_platform()
        .canonicalize_no_links(&requested_target)
        .map_err(|error| format!("failed to access the requested directory: {error}"))?;
    let root = current_platform()
        .canonicalize_no_links(Path::new(&validated.scan_root))
        .map_err(|error| format!("failed to access the duplicate scan root: {error}"))?;
    if validated.protected_roots.iter().any(|protected_root| {
        let protected_root = Path::new(protected_root);
        current_platform().path_is_same_or_child(&target, protected_root)
            || current_platform().path_is_same_or_child(protected_root, &target)
    }) {
        return Err("a protected duplicate item cannot be deleted".to_string());
    }
    if current_platform().paths_equal(&target, &root)
        || !current_platform().path_is_same_or_child(&target, &root)
    {
        return Err("the directory is outside the current duplicate scan root".to_string());
    }
    if current_platform()
        .should_skip(&target, &root, ScanPurpose::DuplicateFiles)
        .is_some()
    {
        return Err("MangoDisk cannot process a protected duplicate directory".to_string());
    }
    let usage = verify_live_directory(
        &target,
        &validated.expected_hash,
        candidate.expected_bytes,
        validated.expected_file_count,
        operation,
    )?;
    let verified_target = current_platform()
        .canonicalize_no_links(&requested_target)
        .map_err(|error| error.to_string())?;
    if !current_platform().paths_equal(&verified_target, &target) {
        return Err("the directory changed during safety validation".to_string());
    }

    match delete_path_permanently(
        prepared,
        candidate.expected_bytes,
        validated.expected_file_count,
    ) {
        Ok(()) => Ok((target, usage, validated.expected_file_count, true)),
        Err(error) if error.is_partial() && !requested_target.exists() => {
            // The same-volume staging move already removed the selected path. Keep the result UI
            // synchronized even when best-effort cleanup of the private staging tree was partial.
            log::warn!(
                "duplicate_directory_staging_cleanup_partial operation_id={} released_logical_bytes={} error={}",
                operation.id(),
                error.released_bytes(),
                mangodisk_platform::diagnostics::text(&error)
            );
            Ok((
                target,
                FileSpaceUsage::logical_only(error.released_bytes()),
                error.affected_item_count(),
                true,
            ))
        }
        Err(error) => Err(error.to_string()),
    }
}

fn duplicate_delete_validation_reason(error: &str) -> &'static str {
    match error {
        "at least one duplicate file must be selected" => "empty_selection",
        "the duplicate-file result session is unavailable" => "session_unavailable",
        "the duplicate-file result session expired; scan again" => "session_expired",
        "a duplicate file was selected more than once" => "duplicate_selection",
        "at least one file must remain in every duplicate group" => "group_exhausted",
        "a duplicate file no longer matches the scan result" => "candidate_changed",
        "a duplicate item is outside the current scan roots" => "outside_scan_roots",
        "a selected file is not part of the current duplicate scan" => "unknown_candidate",
        "a protected duplicate item cannot be deleted" => "protected_root",
        _ => "unknown",
    }
}

fn normalize_protected_roots(
    protected_roots: Vec<String>,
    scan_roots: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let mut canonical = Vec::<PathBuf>::new();
    for value in protected_roots
        .into_iter()
        .filter(|value| !value.trim().is_empty())
    {
        let path = current_platform()
            .canonicalize_no_links(Path::new(&value))
            .map_err(|error| format!("the protected duplicate-file root is unsafe: {error}"))?;
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!("failed to access a protected duplicate-file root: {error}")
        })?;
        if !metadata.is_dir() || current_platform().is_link_like(&metadata) {
            return Err(
                "a protected duplicate-file root must be a regular directory or volume".to_string(),
            );
        }
        if !scan_roots
            .iter()
            .any(|root| current_platform().path_is_same_or_child(&path, root))
        {
            return Err(
                "a protected duplicate-file root must be inside the current scan scope".to_string(),
            );
        }
        if !canonical
            .iter()
            .any(|existing| current_platform().paths_equal(existing, &path))
        {
            canonical.push(path);
        }
    }
    canonical.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    let mut normalized = Vec::<PathBuf>::new();
    for path in canonical {
        if !normalized
            .iter()
            .any(|root| current_platform().path_is_same_or_child(&path, root))
        {
            normalized.push(path);
        }
    }
    Ok(normalized)
}

/// Applies protection after exact directory aggregation so a directory candidate cannot hide a
/// protected descendant. A protected path intersecting an aggregated directory protects that
/// entire directory entry and prevents an ancestor deletion from bypassing the policy.
fn apply_protection_policy(
    groups: &mut [DuplicateGroup],
    protected_roots: &[PathBuf],
) -> (u64, u64) {
    let mut protected_group_count = 0_u64;
    let mut protected_entry_count = 0_u64;
    for group in groups {
        let mut group_has_protected_entry = false;
        for entry in &mut group.entries {
            let entry_path = Path::new(&entry.path);
            let protected = protected_roots.iter().any(|root| {
                current_platform().path_is_same_or_child(entry_path, root)
                    || (group.kind == DuplicateGroupKind::Directory
                        && current_platform().path_is_same_or_child(root, entry_path))
            });
            entry.delete_policy = if protected {
                group_has_protected_entry = true;
                protected_entry_count = protected_entry_count.saturating_add(1);
                DuplicateEntryDeletePolicy::Protected
            } else {
                DuplicateEntryDeletePolicy::Cleanable
            };
        }
        if group_has_protected_entry {
            protected_group_count = protected_group_count.saturating_add(1);
        }
        group.refresh_reclaimable_bytes();
    }
    (protected_group_count, protected_entry_count)
}

fn build_duplicate_group(
    candidates: &[FileCandidate],
    (bytes_per_file, hash): (u64, blake3::Hash),
    mut candidate_indices: Vec<usize>,
    allocation_fallback_count: &mut u64,
) -> Option<DuplicateGroup> {
    if candidate_indices.len() < 2 {
        return None;
    }
    candidate_indices.sort_by(|left, right| candidates[*left].path.cmp(&candidates[*right].path));
    let hash = hash.to_hex().to_string();
    let entries = candidate_indices
        .into_iter()
        .map(|index| {
            let candidate = &candidates[index];
            let measured_allocated_bytes = fs::symlink_metadata(&candidate.path)
                .ok()
                .filter(|metadata| metadata.is_file() && metadata.len() == candidate.bytes)
                .map(|metadata| {
                    current_platform()
                        .file_space_usage(&candidate.path, &metadata)
                        .allocated_bytes
                });
            if measured_allocated_bytes.is_none() {
                *allocation_fallback_count = allocation_fallback_count.saturating_add(1);
            }
            let allocated_bytes = measured_allocated_bytes.unwrap_or(candidate.bytes);
            DuplicateFileEntry {
                name: candidate
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                parent_path: native_path_string(candidate.path.parent().unwrap_or(&candidate.path)),
                path: native_path_string(&candidate.path),
                bytes: candidate.bytes,
                allocated_bytes,
                modified_at_ms: candidate.modified_at_ms,
                delete_policy: DuplicateEntryDeletePolicy::Cleanable,
            }
        })
        .collect();
    let mut group = DuplicateGroup {
        id: hash.chars().take(16).collect(),
        hash,
        kind: DuplicateGroupKind::File,
        bytes_per_file,
        file_count_per_entry: 1,
        reclaimable_bytes: 0,
        entries,
    };
    group.refresh_reclaimable_bytes();
    Some(group)
}

fn execute_hash_pipeline(
    candidates: &[FileCandidate],
    plan: HashPipelinePlan,
    cache: Option<&HashMap<PathBuf, DuplicateHashCacheFile>>,
    operation: &OperationGuard,
    progress: &DuplicateProgress,
    on_group_complete: &mut impl FnMut((u64, blake3::Hash), Vec<usize>),
) -> Result<HashPipelineResult, String> {
    let mut query_allocated_content = |file: &File, logical_bytes: u64| {
        current_platform()
            .file_has_allocated_content(file, logical_bytes)
            // The fast path treats every native error identically: it records a fallback and
            // performs ordinary content hashing. Keeping the native diagnostic out of this
            // private callback avoids making a safety optimization depend on error text.
            .map_err(|_| ())
    };
    execute_hash_pipeline_with_allocated_content_query(
        candidates,
        plan,
        cache,
        operation,
        progress,
        on_group_complete,
        &mut query_allocated_content,
    )
}

fn execute_hash_pipeline_with_allocated_content_query(
    candidates: &[FileCandidate],
    plan: HashPipelinePlan,
    cache: Option<&HashMap<PathBuf, DuplicateHashCacheFile>>,
    operation: &OperationGuard,
    progress: &DuplicateProgress,
    on_group_complete: &mut impl FnMut((u64, blake3::Hash), Vec<usize>),
    query_allocated_content: &mut impl FnMut(&File, u64) -> Result<Option<bool>, ()>,
) -> Result<HashPipelineResult, String> {
    let HashPipelinePlan {
        sample_plan,
        worker_count,
    } = plan;
    let mut diagnostics = HashPipelineDiagnostics {
        sample_hash_candidate_count: u64::try_from(candidates.len()).unwrap_or(u64::MAX),
        ..HashPipelineDiagnostics::default()
    };
    let matching_cache = candidates
        .iter()
        .map(|candidate| {
            cache
                .and_then(|files| files.get(&candidate.path))
                .filter(|cached| duplicate_cache_file_matches(candidate, cached))
        })
        .collect::<Vec<_>>();
    diagnostics.cache_candidate_match_count = u64::try_from(
        matching_cache
            .iter()
            .filter(|cached| cached.is_some())
            .count(),
    )
    .unwrap_or(u64::MAX);

    // The digest array feeds the next in-memory cache snapshot while groups feed the next filter
    // stage.
    // Both cached and fresh results populate them together, avoiding another pass over all
    // candidates. Low-threshold scans may contain hundreds of thousands of candidates, so an
    // additional full pass would directly increase first-scan cost.
    let mut sample_hashes = vec![None; candidates.len()];
    let mut sample_groups = HashMap::<(u64, blake3::Hash), Vec<usize>>::new();
    let mut sample_task_indices = Vec::new();
    for (index, cached) in matching_cache.iter().enumerate() {
        if let Some(cached) = cached {
            let hash = blake3::Hash::from_bytes(cached.sample_hash);
            sample_hashes[index] = Some(hash);
            sample_groups
                .entry((candidates[index].bytes, hash))
                .or_default()
                .push(index);
            diagnostics.sample_hash_cache_hit_count =
                diagnostics.sample_hash_cache_hit_count.saturating_add(1);
        } else {
            sample_task_indices.push(index);
        }
    }

    let mut sample_failures = HashFailureDiagnostics::default();
    let sample_stage = run_hash_stage(
        candidates,
        &sample_task_indices,
        HashStage::Sample(sample_plan),
        operation,
        progress,
        worker_count,
        |outcome| {
            diagnostics.sample_hash_bytes = diagnostics
                .sample_hash_bytes
                .saturating_add(outcome.bytes_read);
            match outcome.result {
                Ok(hash) => {
                    sample_hashes[outcome.candidate_index] = Some(hash);
                    sample_groups
                        .entry((candidates[outcome.candidate_index].bytes, hash))
                        .or_default()
                        .push(outcome.candidate_index);
                }
                Err(error) => {
                    sample_failures.record(&candidates[outcome.candidate_index].path, &error);
                }
            }
        },
    )?;
    diagnostics.sample_hash_ms = sample_stage.elapsed_ms;
    diagnostics.sample_hash_worker_count =
        u64::try_from(sample_stage.worker_count).unwrap_or(u64::MAX);
    diagnostics.sample_hash_peak_in_flight =
        u64::try_from(sample_stage.peak_in_flight).unwrap_or(u64::MAX);
    diagnostics.hash_result_queue_capacity =
        u64::try_from(sample_stage.queue_capacity).unwrap_or(u64::MAX);

    let mut full_task_groups = sample_groups
        .into_values()
        .filter(|items| items.len() > 1)
        .collect::<Vec<_>>();
    for group in &mut full_task_groups {
        group.sort_unstable();
    }
    // Prioritize sample groups with more potential reclaimable space so large real duplicate
    // groups reach the UI sooner. All tasks still share one bounded worker pool; ordering changes
    // scheduling priority only, not the stable final result.
    full_task_groups.sort_by(|left, right| {
        let left_bytes = candidates[left[0]]
            .bytes
            .saturating_mul(left.len().saturating_sub(1) as u64);
        let right_bytes = candidates[right[0]]
            .bytes
            .saturating_mul(right.len().saturating_sub(1) as u64);
        right_bytes
            .cmp(&left_bytes)
            .then_with(|| left[0].cmp(&right[0]))
    });
    diagnostics.full_hash_candidate_count = full_task_groups
        .iter()
        .map(|group| u64::try_from(group.len()).unwrap_or(u64::MAX))
        .fold(0_u64, u64::saturating_add);

    // Retain a lightweight candidate-to-sample-group mapping. Full-hash workers remain globally
    // parallel, but a group can finalize its local full-hash buckets and stream them as soon as its
    // remaining count reaches zero, without waiting for unrelated large files.
    let mut full_hashes = vec![None; candidates.len()];
    let mut full_groups = HashMap::<(u64, blake3::Hash), Vec<usize>>::new();
    let mut fully_sparse_group_hashes = HashSet::<blake3::Hash>::new();
    let mut full_task_indices = Vec::new();
    let mut candidate_group_indices = vec![usize::MAX; candidates.len()];
    let mut full_task_group_states = Vec::with_capacity(full_task_groups.len());
    for (group_index, candidate_indices) in full_task_groups.into_iter().enumerate() {
        let mut state = FullTaskGroupState::default();
        let has_cached_full_hash = candidate_indices.iter().any(|index| {
            matching_cache[*index]
                .and_then(|cached| cached.full_hash)
                .is_some()
        });
        let fully_sparse = if has_cached_full_hash {
            false
        } else {
            certify_fully_sparse_group(
                candidates,
                &candidate_indices,
                operation,
                &mut diagnostics.allocated_range_query_fallback_count,
                query_allocated_content,
            )?
        };
        let sparse_group_hash =
            fully_sparse.then(|| fully_sparse_group_hash(candidates[candidate_indices[0]].bytes));
        if fully_sparse {
            fully_sparse_group_hashes
                .insert(sparse_group_hash.expect("a certified sparse group always has a digest"));
            let candidate_count = u64::try_from(candidate_indices.len()).unwrap_or(u64::MAX);
            diagnostics.fully_sparse_candidate_count = diagnostics
                .fully_sparse_candidate_count
                .saturating_add(candidate_count);
            diagnostics.fully_sparse_group_count =
                diagnostics.fully_sparse_group_count.saturating_add(1);
            diagnostics.fully_sparse_logical_bytes_skipped = diagnostics
                .fully_sparse_logical_bytes_skipped
                .saturating_add(
                    candidates[candidate_indices[0]]
                        .bytes
                        .saturating_mul(candidate_count),
                );
        }
        for index in candidate_indices {
            candidate_group_indices[index] = group_index;
            if let Some(hash) = sparse_group_hash {
                // This layout-derived digest is valid only while every member is certified as a
                // complete hole. It groups the current scan but intentionally remains absent from
                // `full_hashes`, so a later mixed sparse/dense group can never compare it with a
                // real content digest loaded from cache.
                state
                    .hash_groups
                    .entry((candidates[index].bytes, hash))
                    .or_default()
                    .push(index);
                continue;
            }
            let cached_full_hash = matching_cache[index].and_then(|cached| cached.full_hash);
            if let Some(cached_full_hash) = cached_full_hash {
                let hash = blake3::Hash::from_bytes(cached_full_hash);
                full_hashes[index] = Some(hash);
                state
                    .hash_groups
                    .entry((candidates[index].bytes, hash))
                    .or_default()
                    .push(index);
                diagnostics.full_hash_cache_hit_count =
                    diagnostics.full_hash_cache_hit_count.saturating_add(1);
            } else {
                state.remaining_tasks = state.remaining_tasks.saturating_add(1);
                full_task_indices.push(index);
            }
        }
        full_task_group_states.push(state);
    }
    for state in &mut full_task_group_states {
        if state.remaining_tasks == 0 {
            complete_full_task_group(state, &mut full_groups, on_group_complete);
        }
    }

    let mut full_failures = HashFailureDiagnostics::default();
    let full_stage = run_hash_stage(
        candidates,
        &full_task_indices,
        HashStage::Full,
        operation,
        progress,
        worker_count,
        |outcome| {
            let group_index = candidate_group_indices[outcome.candidate_index];
            let state = &mut full_task_group_states[group_index];
            diagnostics.full_hash_bytes = diagnostics
                .full_hash_bytes
                .saturating_add(outcome.bytes_read);
            match outcome.result {
                Ok(hash) => {
                    full_hashes[outcome.candidate_index] = Some(hash);
                    state
                        .hash_groups
                        .entry((candidates[outcome.candidate_index].bytes, hash))
                        .or_default()
                        .push(outcome.candidate_index);
                }
                Err(error) => {
                    full_failures.record(&candidates[outcome.candidate_index].path, &error);
                }
            }
            state.remaining_tasks = state.remaining_tasks.saturating_sub(1);
            if state.remaining_tasks == 0 {
                complete_full_task_group(state, &mut full_groups, on_group_complete);
            }
        },
    )?;
    diagnostics.full_hash_ms = full_stage.elapsed_ms;
    diagnostics.full_hash_worker_count = u64::try_from(full_stage.worker_count).unwrap_or(u64::MAX);
    diagnostics.full_hash_peak_in_flight =
        u64::try_from(full_stage.peak_in_flight).unwrap_or(u64::MAX);
    diagnostics.hash_result_queue_capacity = diagnostics
        .hash_result_queue_capacity
        .max(u64::try_from(full_stage.queue_capacity).unwrap_or(u64::MAX));

    Ok(HashPipelineResult {
        full_groups,
        fully_sparse_group_hashes,
        sample_hashes,
        full_hashes,
        skipped_count: sample_failures.count.saturating_add(full_failures.count),
        sample_failures,
        full_failures,
        diagnostics,
    })
}

/// Certifies an entire sample group before bypassing content reads.
///
/// A single allocated member, unsupported filesystem, query error, or validation race returns the
/// group to ordinary full hashing. This whole-group rule preserves equality with dense all-zero
/// files and avoids using physical allocation as a proxy for content in mixed layouts.
fn certify_fully_sparse_group(
    candidates: &[FileCandidate],
    candidate_indices: &[usize],
    operation: &OperationGuard,
    fallback_count: &mut u64,
    query_allocated_content: &mut impl FnMut(&File, u64) -> Result<Option<bool>, ()>,
) -> Result<bool, String> {
    for &index in candidate_indices {
        operation
            .ensure_not_cancelled()
            .map_err(|error| error.to_string())?;
        let candidate = &candidates[index];
        let file = match File::open(&candidate.path) {
            Ok(file) => file,
            Err(_) => return Ok(false),
        };
        if validate_open_file(candidate, &file, true).is_err() {
            return Ok(false);
        }
        let has_allocated_content = match query_allocated_content(&file, candidate.bytes) {
            Ok(Some(value)) => value,
            Ok(None) | Err(_) => {
                *fallback_count = fallback_count.saturating_add(1);
                return Ok(false);
            }
        };
        if has_allocated_content {
            return Ok(false);
        }
        if validate_open_file(candidate, &file, false).is_err()
            || validate_current_path(candidate).is_err()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn fully_sparse_group_hash(logical_bytes: u64) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new_derive_key("mangodisk.duplicates.fully-sparse-group.v1");
    hasher.update(&logical_bytes.to_le_bytes());
    hasher.finalize()
}

fn complete_full_task_group(
    state: &mut FullTaskGroupState,
    full_groups: &mut HashMap<(u64, blake3::Hash), Vec<usize>>,
    on_group_complete: &mut impl FnMut((u64, blake3::Hash), Vec<usize>),
) {
    // Identical full content necessarily has the same size and sample hash, so groups never need
    // merging across sample buckets. Local results still enter the global map so final ordering
    // and the digest use one authoritative collection.
    for (key, mut indices) in std::mem::take(&mut state.hash_groups) {
        // Worker completion order is nondeterministic. Sorting by candidate ordinal stabilizes
        // local order so cached and fresh paths produce the same internal result and streaming
        // batches remain reproducible.
        indices.sort_unstable();
        full_groups
            .entry(key)
            .or_default()
            .extend(indices.iter().copied());
        if indices.len() > 1 {
            on_group_complete(key, indices);
        }
    }
}

fn duplicate_cache_file_matches(
    candidate: &FileCandidate,
    cached: &DuplicateHashCacheFile,
) -> bool {
    candidate.root_ordinal == cached.root_ordinal
        && candidate.path == cached.path
        && candidate.bytes == cached.bytes
        && candidate.modified_at == cached.modified_at
        && candidate
            .identity
            .is_some_and(|identity| encode_file_identity(identity) == cached.identity)
}

fn encode_file_identity(identity: FileIdentity) -> [u8; 16] {
    let mut encoded = [0_u8; 16];
    encoded[..8].copy_from_slice(&identity.volume.to_be_bytes());
    encoded[8..].copy_from_slice(&identity.index.to_be_bytes());
    encoded
}

fn publish_duplicate_hash_cache(
    roots: &[DuplicateHashCacheRoot],
    policy: DuplicateHashCachePolicy,
    candidates: &[FileCandidate],
    pipeline: &HashPipelineResult,
    operation: &OperationGuard,
    diagnostics: &mut DuplicateScanDiagnostics,
) -> Result<(), String> {
    let files = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            let identity = candidate.identity?;
            let sample_hash = pipeline.sample_hashes[index]?;
            Some(DuplicateHashCacheFile {
                root_ordinal: candidate.root_ordinal,
                path: candidate.path.clone(),
                bytes: candidate.bytes,
                modified_at: candidate.modified_at,
                identity: encode_file_identity(identity),
                sample_hash: *sample_hash.as_bytes(),
                full_hash: pipeline.full_hashes[index].map(|hash| *hash.as_bytes()),
            })
        })
        .collect::<Vec<_>>();
    let write: Result<DuplicateHashCacheWriteDiagnostics, String> = hash_cache::store_snapshot(
        roots,
        policy.minimum_bytes,
        policy.sample_plan.name(),
        policy.exclusion_fingerprint,
        files,
        || operation.cancelled().load(Ordering::Relaxed),
    );
    match write {
        Ok(write) => {
            diagnostics.cache_write_entry_count = write.entry_count;
            diagnostics.cache_write_ms = write.elapsed_ms;
            log::info!(
                "duplicate_hash_cache_published operation_id={} root_count={} entry_count={} elapsed_ms={}",
                operation.id(),
                roots.len(),
                write.entry_count,
                write.elapsed_ms
            );
            Ok(())
        }
        Err(error) => {
            operation
                .ensure_not_cancelled()
                .map_err(|error| error.to_string())?;
            log_cache_error(operation.id(), "write", &error);
            Ok(())
        }
    }
}

fn apply_pipeline_diagnostics(
    diagnostics: &mut DuplicateScanDiagnostics,
    pipeline: &HashPipelineDiagnostics,
) {
    diagnostics.sample_hash_ms = pipeline.sample_hash_ms;
    diagnostics.full_hash_ms = pipeline.full_hash_ms;
    diagnostics.sample_hash_bytes = pipeline.sample_hash_bytes;
    diagnostics.full_hash_bytes = pipeline.full_hash_bytes;
    diagnostics.sample_hash_candidate_count = pipeline.sample_hash_candidate_count;
    diagnostics.full_hash_candidate_count = pipeline.full_hash_candidate_count;
    diagnostics.cache_candidate_match_count = pipeline.cache_candidate_match_count;
    diagnostics.sample_hash_cache_hit_count = pipeline.sample_hash_cache_hit_count;
    diagnostics.full_hash_cache_hit_count = pipeline.full_hash_cache_hit_count;
    diagnostics.sample_hash_worker_count = pipeline.sample_hash_worker_count;
    diagnostics.sample_hash_peak_in_flight = pipeline.sample_hash_peak_in_flight;
    diagnostics.full_hash_worker_count = pipeline.full_hash_worker_count;
    diagnostics.full_hash_peak_in_flight = pipeline.full_hash_peak_in_flight;
    diagnostics.hash_result_queue_capacity = pipeline.hash_result_queue_capacity;
    diagnostics.fully_sparse_candidate_count = pipeline.fully_sparse_candidate_count;
    diagnostics.fully_sparse_group_count = pipeline.fully_sparse_group_count;
    diagnostics.fully_sparse_logical_bytes_skipped = pipeline.fully_sparse_logical_bytes_skipped;
    diagnostics.allocated_range_query_fallback_count =
        pipeline.allocated_range_query_fallback_count;
}

fn run_hash_stage(
    candidates: &[FileCandidate],
    task_indices: &[usize],
    stage: HashStage,
    operation: &OperationGuard,
    progress: &DuplicateProgress,
    configured_worker_count: usize,
    mut consume: impl FnMut(HashOutcome),
) -> Result<HashStageDiagnostics, String> {
    if task_indices.is_empty() {
        return Ok(HashStageDiagnostics::default());
    }
    progress.extend_hash_stage(
        &candidates[task_indices[0]].path,
        u64::try_from(task_indices.len()).unwrap_or(u64::MAX),
    );
    let worker_count = configured_worker_count.max(1).min(task_indices.len());
    let queue_capacity = worker_count.saturating_mul(HASH_RESULT_QUEUE_PER_WORKER);
    let started = Instant::now();
    let peak_in_flight = AtomicUsize::new(0);
    let active_tasks = AtomicUsize::new(0);
    thread::scope(|scope| -> Result<(), String> {
        let next_task = Arc::new(AtomicUsize::new(0));
        // The channel carries only fixed-size indexes, digests, and counters; it copies neither
        // paths nor file content. The receiver aggregates immediately instead of retaining a
        // second outcome per candidate. Capacity scales with worker count and applies backpressure
        // directly to hash threads when the consumer slows down.
        let (sender, receiver) = sync_channel(queue_capacity);
        let workers = (0..worker_count)
            .map(|_| {
                let sender = sender.clone();
                let next_task = Arc::clone(&next_task);
                let active_tasks = &active_tasks;
                let peak_in_flight = &peak_in_flight;
                scope.spawn(move || {
                    let mut buffer = Vec::<u8>::new();
                    loop {
                        if operation.cancelled().load(Ordering::Relaxed) {
                            break;
                        }
                        let task_position = next_task.fetch_add(1, Ordering::Relaxed);
                        let Some(&candidate_index) = task_indices.get(task_position) else {
                            break;
                        };
                        let candidate = &candidates[candidate_index];
                        progress.emit(TraversalStage::HashingFiles, &candidate.path, false, 0, 0);
                        let active = active_tasks.fetch_add(1, Ordering::AcqRel) + 1;
                        peak_in_flight.fetch_max(active, Ordering::AcqRel);
                        let mut bytes_read = 0_u64;
                        let result = match stage {
                            HashStage::Sample(plan) => sample_hash(
                                candidate,
                                plan,
                                operation,
                                &mut buffer,
                                &mut bytes_read,
                            ),
                            HashStage::Full => {
                                full_hash(candidate, operation, &mut buffer, &mut bytes_read)
                            }
                        };
                        active_tasks.fetch_sub(1, Ordering::AcqRel);
                        if sender
                            .send(HashOutcome {
                                candidate_index,
                                result,
                                bytes_read,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        drop(sender);

        let mut completed_count = 0_usize;
        while let Ok(outcome) = receiver.recv() {
            completed_count = completed_count.saturating_add(1);
            progress.complete_hash_step(
                &candidates[outcome.candidate_index].path,
                outcome.bytes_read,
            );
            consume(outcome);
        }
        for worker in workers {
            worker
                .join()
                .map_err(|_| "a duplicate-file hash worker exited unexpectedly".to_string())?;
        }
        operation
            .ensure_not_cancelled()
            .map_err(|error| error.to_string())?;
        if completed_count != task_indices.len() {
            return Err(format!(
                "duplicate-file hashing was incomplete: expected {}, completed {}",
                task_indices.len(),
                completed_count
            ));
        }
        Ok(())
    })?;
    Ok(HashStageDiagnostics {
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        worker_count,
        peak_in_flight: peak_in_flight.load(Ordering::Acquire),
        queue_capacity,
    })
}

fn duplicate_hash_worker_config(
    roots: &[PathBuf],
    worker_override: Option<usize>,
) -> HashWorkerConfig {
    let available_workers = thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
        .max(1);
    if let Some(worker_count) = worker_override {
        return HashWorkerConfig {
            worker_count: worker_count
                .clamp(1, DUPLICATE_HASH_WORKER_LIMIT)
                .min(available_workers),
            identity_worker_count: worker_count
                .clamp(1, DUPLICATE_HASH_WORKER_LIMIT)
                .min(available_workers),
            device_classes: "benchmark_override".to_string(),
        };
    }

    let volumes = current_platform().volumes().unwrap_or_else(|error| {
        log::warn!(
            "duplicate_volume_inventory_failed error={}",
            mangodisk_platform::diagnostics::text(&error)
        );
        Vec::new()
    });
    duplicate_hash_worker_config_from_volumes(roots, &volumes, available_workers)
}

fn duplicate_hash_worker_config_from_volumes(
    roots: &[PathBuf],
    volumes: &[VolumeInfo],
    available_workers: usize,
) -> HashWorkerConfig {
    let mut device_classes = Vec::<ScanDeviceClass>::new();
    let mut device_limit = DUPLICATE_HASH_WORKER_LIMIT;
    let mut identity_device_limit = DUPLICATE_HASH_WORKER_LIMIT;
    for root in roots {
        let scheduling = volumes
            .iter()
            .filter(|volume| {
                current_platform().path_is_same_or_child(root, Path::new(&volume.mount_point))
            })
            .max_by_key(|volume| Path::new(&volume.mount_point).components().count())
            .map(|volume| volume.scan_concurrency);
        let (class, hash_worker_limit, identity_worker_limit) = scheduling.map_or(
            (ScanDeviceClass::Unknown, 1, 1),
            |scheduling| match scheduling.class {
                ScanDeviceClass::SolidState => (
                    scheduling.class,
                    scheduling.worker_limit.min(DUPLICATE_HASH_WORKER_LIMIT),
                    scheduling.worker_limit.min(DUPLICATE_HASH_WORKER_LIMIT),
                ),
                // Duplicate hashing interleaves reads from multiple large files. Even when cleanup
                // allows two independent roots on rotational media, this stage must use one worker
                // so random seeks do not erase all throughput.
                ScanDeviceClass::Rotational => (
                    scheduling.class,
                    1,
                    scheduling.worker_limit.min(DUPLICATE_HASH_WORKER_LIMIT),
                ),
                ScanDeviceClass::Removable
                | ScanDeviceClass::Network
                | ScanDeviceClass::Unknown => (scheduling.class, 1, 1),
            },
        );
        device_limit = device_limit.min(hash_worker_limit);
        identity_device_limit = identity_device_limit.min(identity_worker_limit);
        if !device_classes.contains(&class) {
            device_classes.push(class);
        }
    }
    if device_classes.is_empty() {
        device_classes.push(ScanDeviceClass::Unknown);
        device_limit = 1;
        identity_device_limit = 1;
    }
    device_classes.sort_by_key(|class| class.as_str());
    HashWorkerConfig {
        worker_count: available_workers
            .max(1)
            .min(device_limit)
            .min(DUPLICATE_HASH_WORKER_LIMIT),
        identity_worker_count: available_workers
            .max(1)
            .min(identity_device_limit)
            .min(DUPLICATE_HASH_WORKER_LIMIT),
        device_classes: device_classes
            .into_iter()
            .map(ScanDeviceClass::as_str)
            .collect::<Vec<_>>()
            .join(","),
    }
}

fn sample_hash(
    candidate: &FileCandidate,
    plan: SamplePlan,
    operation: &OperationGuard,
    buffer: &mut Vec<u8>,
    bytes_read: &mut u64,
) -> Result<blake3::Hash, String> {
    let mut file = File::open(&candidate.path).map_err(|error| error.to_string())?;
    validate_open_file(candidate, &file, true)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&candidate.bytes.to_le_bytes());
    let sample_size = u64::try_from(plan.segment_bytes())
        .unwrap_or(u64::MAX)
        .min(candidate.bytes);
    buffer.resize(
        usize::try_from(sample_size).unwrap_or(plan.segment_bytes()),
        0,
    );
    let mut previous = None;
    for offset in plan
        .offsets(candidate.bytes, sample_size)
        .into_iter()
        .flatten()
    {
        operation
            .ensure_not_cancelled()
            .map_err(|error| error.to_string())?;
        if previous == Some(offset) {
            continue;
        }
        previous = Some(offset);
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        let read = read_up_to(&mut file, buffer, bytes_read).map_err(|error| error.to_string())?;
        hasher.update(&offset.to_le_bytes());
        hasher.update(&buffer[..read]);
    }
    // Sampling filters candidates; it does not establish duplicates. At the end, verify that the
    // same open object retained its length and modification time. If the path was replaced during
    // sampling, full hashing reopens it and strictly verifies physical identity. Avoid reopening
    // every path here because low-threshold Windows scans can otherwise create thousands of handles.
    validate_open_file(candidate, &file, false)?;
    Ok(hasher.finalize())
}

fn full_hash(
    candidate: &FileCandidate,
    operation: &OperationGuard,
    buffer: &mut Vec<u8>,
    bytes_read: &mut u64,
) -> Result<blake3::Hash, String> {
    // Full hashing uses the current file opened by this scan. Across scans, path, size,
    // modification time, and file identity alone cannot prove that content stayed unchanged.
    // Reusing an old hash on removable media with coarse timestamps could classify an equal-size
    // rewrite as a duplicate and create unacceptable cleanup risk.
    let mut file = File::open(&candidate.path).map_err(|error| error.to_string())?;
    validate_open_file(candidate, &file, true)?;
    let mut hasher = blake3::Hasher::new();
    buffer.resize(FULL_HASH_BUFFER_BYTES, 0);
    loop {
        operation
            .ensure_not_cancelled()
            .map_err(|error| error.to_string())?;
        let read = file.read(buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        *bytes_read = bytes_read.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        hasher.update(&buffer[..read]);
    }
    validate_open_file(candidate, &file, false)?;
    validate_current_path(candidate)?;
    Ok(hasher.finalize())
}

fn read_up_to(
    file: &mut impl Read,
    buffer: &mut [u8],
    bytes_read: &mut u64,
) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buffer.len() {
        let read = file.read(&mut buffer[total..])?;
        if read == 0 {
            break;
        }
        total += read;
        *bytes_read = bytes_read.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
    Ok(total)
}

fn normalize_group_paths_for_output(groups: &mut [DuplicateGroup]) {
    for entry in groups.iter_mut().flat_map(|group| &mut group.entries) {
        entry.path = display_path(Path::new(&entry.path));
        entry.parent_path = display_path(Path::new(&entry.parent_path));
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
