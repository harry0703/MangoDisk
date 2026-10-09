mod parser;

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    time::Instant,
};

use crate::{
    FastAnalysisFile, FastAnalysisRecord, FastAnalysisScanError, FastAnalysisSummary,
    FileSpaceUsage, LargeFileCandidateScanError, LargeFileCandidateSummary, Platform, ScanPurpose,
};

use super::native_io::{file_id, OwnedHandle, VolumePaths};
use super::WindowsPlatform;
use parser::enumerate_layout;

const MAX_DEFERRED_PATHS: usize = 200_000;
const MAX_DIRECTORY_CHAIN_LENGTH: usize = 4_096;
const DISABLE_FILE_LAYOUT_ENV: &str = "MANGODISK_DISABLE_WINDOWS_FILE_LAYOUT";

#[derive(Clone)]
struct DirectoryNode {
    parent_id: u64,
    name: OsString,
    boundary: DirectoryBoundary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectoryBoundary {
    None,
    Reparse,
    RemotePlaceholder,
    Internal,
}

struct CandidateRecord {
    names: Vec<FileNameLink>,
}

struct FileNameLink {
    parent_id: u64,
    name: OsString,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DirectoryTotals {
    logical_bytes: u64,
    allocated_bytes: u64,
    file_count: u64,
    direct_file_count: u64,
    skipped_count: u64,
}

impl DirectoryTotals {
    /// Selects the progress unit promised by the caller-facing scan purpose.
    fn progress_bytes(self, purpose: ScanPurpose) -> u64 {
        match purpose {
            ScanPurpose::DuplicateFiles => self.logical_bytes,
            _ => self.allocated_bytes,
        }
    }

    fn checked_add_assign(&mut self, other: Self) -> Result<(), LayoutScanError> {
        self.logical_bytes = self
            .logical_bytes
            .checked_add(other.logical_bytes)
            .ok_or_else(|| LayoutScanError::Platform("directory_bytes_overflow".to_string()))?;
        self.allocated_bytes = self
            .allocated_bytes
            .checked_add(other.allocated_bytes)
            .ok_or_else(|| {
                LayoutScanError::Platform("directory_allocation_overflow".to_string())
            })?;
        self.file_count = self
            .file_count
            .checked_add(other.file_count)
            .ok_or_else(|| {
                LayoutScanError::Platform("directory_file_count_overflow".to_string())
            })?;
        self.skipped_count = self
            .skipped_count
            .checked_add(other.skipped_count)
            .ok_or_else(|| {
                LayoutScanError::Platform("directory_skipped_count_overflow".to_string())
            })?;
        Ok(())
    }

    fn checked_add_file(&mut self, usage: FileSpaceUsage) -> Result<(), LayoutScanError> {
        self.logical_bytes = self
            .logical_bytes
            .checked_add(usage.logical_bytes)
            .ok_or_else(|| LayoutScanError::Platform("directory_bytes_overflow".to_string()))?;
        self.allocated_bytes = self
            .allocated_bytes
            .checked_add(usage.allocated_bytes)
            .ok_or_else(|| {
                LayoutScanError::Platform("directory_allocation_overflow".to_string())
            })?;
        self.direct_file_count += u64::from(usage.allocated_bytes > 0);
        self.file_count = self.file_count.checked_add(1).ok_or_else(|| {
            LayoutScanError::Platform("directory_file_count_overflow".to_string())
        })?;
        Ok(())
    }

    fn checked_add_skipped(&mut self) -> Result<(), LayoutScanError> {
        self.skipped_count = self.skipped_count.checked_add(1).ok_or_else(|| {
            LayoutScanError::Platform("directory_skipped_count_overflow".to_string())
        })?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayoutCollectionMode {
    CandidatesOnly,
    FullAnalysis,
    AnalysisWithFiles,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CandidateSizeMetric {
    Logical,
    Allocated,
}

impl CandidateSizeMetric {
    fn bytes(self, usage: FileSpaceUsage) -> u64 {
        match self {
            Self::Logical => usage.logical_bytes,
            Self::Allocated => usage.allocated_bytes,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Logical => "logical",
            Self::Allocated => "allocated",
        }
    }
}

#[derive(Default)]
struct LayoutCollection {
    directories: HashMap<u64, DirectoryNode>,
    direct_totals: HashMap<u64, DirectoryTotals>,
    candidates: Vec<CandidateRecord>,
    candidate_path_count: usize,
    // Allocation, parent identity, name, and logical length. NTFS enumeration
    // already supplies these facts; defer only a bounded subset until paths resolve.
    analysis_files: BinaryHeap<Reverse<(u64, u64, OsString, u64)>>,
    page_count: u64,
    entry_count: u64,
    returned_bytes: u64,
    remote_file_count: u64,
    remote_directory_count: u64,
}

#[derive(Debug)]
enum LayoutScanError {
    Cancelled,
    Platform(String),
    Consumer(String),
}

/// Uses NTFS file-layout enumeration only for a complete volume root when a
/// read-only volume handle is available. Ordinary users, subdirectories,
/// non-NTFS volumes, and unsupported systems return `None` so the caller can
/// fall back to Win32 traversal without UAC or broader process privileges.
pub(super) fn find_candidates(
    platform: &WindowsPlatform,
    root: &Path,
    minimum_bytes: u64,
    excluded_roots: &[PathBuf],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    consumer: &mut dyn FnMut(PathBuf) -> Result<(), String>,
) -> Result<Option<LargeFileCandidateSummary>, LargeFileCandidateScanError> {
    // This switch compares native indexing with Win32 traversal on the same
    // machine and data. Default behavior is unchanged; an explicit fallback
    // lets performance baselines measure the previous path without fixtures.
    if env::var_os(DISABLE_FILE_LAYOUT_ENV).is_some() {
        return Ok(None);
    }
    let Some(volume) = VolumePaths::from_scan_root(root) else {
        return Ok(None);
    };
    let started = Instant::now();
    let result = collect_and_emit(
        platform,
        root,
        &volume,
        minimum_bytes,
        excluded_roots,
        is_cancelled,
        consumer,
    );
    match result {
        Ok(summary) => {
            log::info!(
                "windows_file_layout_scan_finished pages={} entries={} candidates={} returned_bytes={} elapsed_ms={}",
                summary.page_count,
                summary.entry_count,
                summary.candidate_summary.candidate_count,
                summary.returned_bytes,
                started.elapsed().as_millis()
            );
            Ok(Some(summary.candidate_summary))
        }
        Err(LayoutScanError::Cancelled) => Err(LargeFileCandidateScanError::Cancelled),
        Err(LayoutScanError::Consumer(error)) => Err(LargeFileCandidateScanError::Consumer(error)),
        Err(LayoutScanError::Platform(error)) => {
            // Native enumeration validates every candidate before invoking
            // the consumer, so fallback cannot mix partial MFT and complete
            // Win32 results. The fixed reason excludes paths, and the full
            // error is retained only as a digest to protect file-name privacy.
            log::info!(
                "windows_file_layout_scan_fallback reason={} error={} elapsed_ms={}",
                platform_error_kind(&error),
                crate::diagnostics::text(&error),
                started.elapsed().as_millis()
            );
            Ok(None)
        }
    }
}

/// Builds directory aggregates and large-file candidates from one NTFS layout.
/// Only a volume root has the complete file-ID namespace. Subdirectories keep
/// using generic traversal instead of filtering a whole-volume result by text.
pub(super) struct AnalysisScanRequest<'a> {
    pub(super) name_exclusions: &'a crate::NameExclusions,
    pub(super) excluded_roots: &'a [PathBuf],
    pub(super) root: &'a Path,
    pub(super) purpose: ScanPurpose,
    pub(super) large_file_minimum_bytes: u64,
    pub(super) is_cancelled: &'a (dyn Fn() -> bool + Sync),
    pub(super) should_prune_directory: fn(&Path) -> bool,
    pub(super) report_progress: &'a mut dyn FnMut(&Path, u64, u64),
}

pub(super) fn analyze_records(
    platform: &WindowsPlatform,
    request: AnalysisScanRequest<'_>,
    consumer: &mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
) -> Result<Option<FastAnalysisSummary>, FastAnalysisScanError> {
    // Volume-wide layout reads cannot prune subtrees before enumeration.
    // Let the caller use its directory walker when a narrowed scope is requested.
    if !request.name_exclusions.is_empty() {
        log::info!(
            "windows_file_layout_analysis_fallback root={} reason=nameExclusionsRequireDirectoryTraversal outcome=directoryTraversal",
            crate::diagnostics::text(&request.root.display())
        );
        return Ok(None);
    }
    if !request.excluded_roots.is_empty() || env::var_os(DISABLE_FILE_LAYOUT_ENV).is_some() {
        return Ok(None);
    }
    let Some(volume) = VolumePaths::from_scan_root(request.root) else {
        return Ok(None);
    };
    let candidate_size_metric = candidate_size_metric(request.purpose);
    let started = Instant::now();
    let result = collect_analysis(AnalysisCollectionRequest {
        platform,
        root: request.root,
        purpose: request.purpose,
        volume: &volume,
        large_file_minimum_bytes: request.large_file_minimum_bytes,
        is_cancelled: request.is_cancelled,
        should_prune_directory: request.should_prune_directory,
        report_progress: request.report_progress,
        consumer,
    });
    match result {
        Ok(summary) => {
            log::info!(
                "windows_file_layout_analysis_finished candidate_size_metric={} pages={} entries={} directories={} candidates={} returned_bytes={} consumer_ms={} elapsed_ms={}",
                candidate_size_metric.code(),
                summary.page_count,
                summary.entry_count,
                summary.directory_count,
                summary.candidate_count,
                summary.returned_bytes,
                summary.consumer_elapsed_ms,
                started.elapsed().as_millis()
            );
            Ok(Some(summary))
        }
        Err(LayoutScanError::Cancelled) => Err(FastAnalysisScanError::Cancelled),
        Err(LayoutScanError::Consumer(error)) => Err(FastAnalysisScanError::Consumer(error)),
        Err(LayoutScanError::Platform(error)) => {
            log::info!(
                "windows_file_layout_analysis_fallback reason={} error={} elapsed_ms={}",
                platform_error_kind(&error),
                crate::diagnostics::text(&error),
                started.elapsed().as_millis()
            );
            Err(FastAnalysisScanError::Platform(error))
        }
    }
}

fn platform_error_kind(error: &str) -> &str {
    error.split_once(':').map_or(error, |(kind, _)| kind)
}

fn candidate_purpose(purpose: ScanPurpose) -> ScanPurpose {
    match purpose {
        ScanPurpose::DuplicateFiles => ScanPurpose::DuplicateFiles,
        _ => ScanPurpose::LargeFiles,
    }
}

fn candidate_size_metric(purpose: ScanPurpose) -> CandidateSizeMetric {
    match purpose {
        ScanPurpose::DuplicateFiles => CandidateSizeMetric::Logical,
        _ => CandidateSizeMetric::Allocated,
    }
}

struct CompletedLayoutScan {
    candidate_summary: LargeFileCandidateSummary,
    page_count: u64,
    entry_count: u64,
    returned_bytes: u64,
}

struct PreparedAnalysis {
    collection: LayoutCollection,
    path_cache: HashMap<u64, Option<PathBuf>>,
    totals: HashMap<u64, DirectoryTotals>,
    completion_order: Vec<u64>,
    root_totals: DirectoryTotals,
    candidate_count: u64,
}

struct AnalysisCollectionRequest<'a> {
    platform: &'a WindowsPlatform,
    root: &'a Path,
    purpose: ScanPurpose,
    volume: &'a VolumePaths,
    large_file_minimum_bytes: u64,
    is_cancelled: &'a (dyn Fn() -> bool + Sync),
    should_prune_directory: fn(&Path) -> bool,
    report_progress: &'a mut dyn FnMut(&Path, u64, u64),
    consumer: &'a mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
}

fn collect_analysis(
    request: AnalysisCollectionRequest<'_>,
) -> Result<FastAnalysisSummary, LayoutScanError> {
    let AnalysisCollectionRequest {
        platform,
        root,
        purpose,
        volume,
        large_file_minimum_bytes,
        is_cancelled,
        should_prune_directory,
        report_progress,
        consumer,
    } = request;
    let volume_handle = OwnedHandle::open_volume(&volume.device)
        .map_err(|error| LayoutScanError::Platform(format!("open_volume:{error}")))?;
    let root_id = file_id(&volume.root)
        .map_err(|error| LayoutScanError::Platform(format!("volume_root_id:{error}")))?;
    let collection = enumerate_layout(
        volume_handle.raw(),
        large_file_minimum_bytes,
        if purpose == ScanPurpose::Analysis {
            LayoutCollectionMode::AnalysisWithFiles
        } else {
            LayoutCollectionMode::FullAnalysis
        },
        candidate_size_metric(purpose),
        is_cancelled,
    )?;
    let mut prepared = prepare_analysis(
        platform,
        root,
        purpose,
        root_id,
        collection,
        is_cancelled,
        should_prune_directory,
    )?;

    // NTFS layout enumeration currently validates the complete graph before exposing records.
    // Publish its trusted total before persistence begins so adapters do not remain at zero while
    // the validated records are written. macOS can report smaller live batches during traversal.
    report_progress(
        root,
        prepared.root_totals.file_count,
        prepared.root_totals.progress_bytes(purpose),
    );

    // Validate all native structures, parent chains, skip boundaries, and
    // aggregate values before the first consumer call. Later failures can only
    // be cancellation or consumer errors, making rollback decisions reliable.
    let consumer_started = Instant::now();
    for directory_id in &prepared.completion_order {
        if is_cancelled() {
            return Err(LayoutScanError::Cancelled);
        }
        let path = prepared
            .path_cache
            .get(directory_id)
            .and_then(Option::as_ref)
            .expect("a prevalidated directory must have a stable path")
            .clone();
        let totals = *prepared
            .totals
            .get(directory_id)
            .expect("a prevalidated directory must have completed totals");
        consumer(FastAnalysisRecord::Directory {
            path,
            retained_file_limit: 0,
            logical_bytes: totals.logical_bytes,
            allocated_bytes: totals.allocated_bytes,
            file_count: totals.file_count,
            direct_file_count: totals.direct_file_count,
            skipped_count: totals.skipped_count,
        })
        .map_err(LayoutScanError::Consumer)?;
    }
    if purpose == ScanPurpose::Analysis {
        for Reverse((allocated_bytes, parent_id, name, logical_bytes)) in
            prepared.collection.analysis_files.drain()
        {
            if is_cancelled() {
                return Err(LayoutScanError::Cancelled);
            }
            let Some(parent) = prepared.path_cache.get(&parent_id).and_then(Option::as_ref) else {
                continue;
            };
            let path = parent.join(name);
            if platform
                .should_skip(&path, root, ScanPurpose::LargeFiles)
                .is_none()
            {
                consumer(FastAnalysisRecord::AnalysisFile(FastAnalysisFile {
                    parent_file_count: 0,
                    path,
                    allocated_bytes,
                    logical_bytes,
                    modified_at_ms: None,
                }))
                .map_err(LayoutScanError::Consumer)?;
            }
        }
    }
    for candidate in prepared.collection.candidates.drain(..) {
        if is_cancelled() {
            return Err(LayoutScanError::Cancelled);
        }
        for name in candidate.names {
            let Some(parent) = prepared
                .path_cache
                .get(&name.parent_id)
                .and_then(Option::as_ref)
            else {
                continue;
            };
            let path = parent.join(name.name);
            if platform
                .should_skip(&path, root, candidate_purpose(purpose))
                .is_some()
            {
                continue;
            }
            consumer(FastAnalysisRecord::LargeFileCandidate(path))
                .map_err(LayoutScanError::Consumer)?;
        }
    }
    let consumer_elapsed_ms =
        u64::try_from(consumer_started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(FastAnalysisSummary {
        root_logical_bytes: prepared.root_totals.logical_bytes,
        root_allocated_bytes: prepared.root_totals.allocated_bytes,
        root_file_count: prepared.root_totals.file_count,
        root_skipped_count: prepared.root_totals.skipped_count,
        page_count: prepared.collection.page_count,
        entry_count: prepared.collection.entry_count,
        directory_count: prepared.completion_order.len() as u64,
        candidate_count: prepared.candidate_count,
        returned_bytes: prepared.collection.returned_bytes,
        consumer_elapsed_ms,
        strategy: "windows_file_layout_allocated_aggregate_v2",
    })
}

fn prepare_analysis(
    platform: &WindowsPlatform,
    root: &Path,
    purpose: ScanPurpose,
    root_id: u64,
    mut collection: LayoutCollection,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    should_prune_directory: fn(&Path) -> bool,
) -> Result<PreparedAnalysis, LayoutScanError> {
    let mut path_cache = HashMap::new();
    path_cache.insert(root_id, Some(root.to_path_buf()));
    for directory_id in collection.directories.keys() {
        resolve_directory_path(
            *directory_id,
            root_id,
            root,
            &collection.directories,
            &mut path_cache,
            is_cancelled,
        )?;
    }

    // should_skip depends only on the normalized path and scan purpose.
    // Reparse directories and descendants have None in the path cache, while
    // descendants of protected system directories hit the same root rule.
    let mut eligible = HashSet::with_capacity(collection.directories.len());
    for directory_id in collection.directories.keys() {
        if is_cancelled() {
            return Err(LayoutScanError::Cancelled);
        }
        let Some(path) = path_cache.get(directory_id).and_then(Option::as_ref) else {
            continue;
        };
        if (path == root || !should_prune_directory(path))
            && platform.should_skip(path, root, purpose).is_none()
        {
            eligible.insert(*directory_id);
        }
    }

    for parent_id in collection.direct_totals.keys() {
        if *parent_id != root_id && !collection.directories.contains_key(parent_id) {
            return Err(LayoutScanError::Platform(
                "file_parent_directory_missing".to_string(),
            ));
        }
    }

    let mut totals = HashMap::with_capacity(eligible.len().saturating_add(1));
    totals.insert(
        root_id,
        collection
            .direct_totals
            .remove(&root_id)
            .unwrap_or_default(),
    );
    for directory_id in &eligible {
        totals.insert(
            *directory_id,
            collection
                .direct_totals
                .remove(directory_id)
                .unwrap_or_default(),
        );
    }
    // Direct totals from skipped subtrees do not enter the result. Release
    // them after extracting visible nodes so consumer output does not retain
    // two aggregate maps at once.
    collection.direct_totals.clear();

    // Generic traversal records one skip at the nearest visible parent of a
    // protected or reparse directory, then abandons the subtree. Counting only
    // the visible-to-hidden edge avoids warning once per hidden descendant.
    for (node_id, node) in &collection.directories {
        if is_cancelled() {
            return Err(LayoutScanError::Cancelled);
        }
        if eligible.contains(node_id) {
            continue;
        }
        if node.boundary != DirectoryBoundary::Internal
            && (node.parent_id == root_id || eligible.contains(&node.parent_id))
        {
            totals
                .get_mut(&node.parent_id)
                .expect("a visible parent must have an aggregate slot")
                .checked_add_skipped()?;
        }
    }

    let mut remaining_children = HashMap::with_capacity(eligible.len().saturating_add(1));
    remaining_children.insert(root_id, 0usize);
    for directory_id in &eligible {
        remaining_children.insert(*directory_id, 0);
    }
    for directory_id in &eligible {
        let parent_id = collection.directories[directory_id].parent_id;
        let Some(children) = remaining_children.get_mut(&parent_id) else {
            return Err(LayoutScanError::Platform(
                "eligible_directory_parent_missing".to_string(),
            ));
        };
        *children = children.checked_add(1).ok_or_else(|| {
            LayoutScanError::Platform("directory_child_count_overflow".to_string())
        })?;
    }

    let mut ready = eligible
        .iter()
        .filter_map(|directory_id| (remaining_children[directory_id] == 0).then_some(*directory_id))
        .collect::<VecDeque<_>>();
    let mut completion_order = Vec::with_capacity(eligible.len().saturating_add(1));
    while let Some(directory_id) = ready.pop_front() {
        if is_cancelled() {
            return Err(LayoutScanError::Cancelled);
        }
        let parent_id = collection.directories[&directory_id].parent_id;
        let child_totals = totals[&directory_id];
        totals
            .get_mut(&parent_id)
            .expect("a prevalidated parent must have an aggregate slot")
            .checked_add_assign(child_totals)?;
        completion_order.push(directory_id);
        let children = remaining_children
            .get_mut(&parent_id)
            .expect("a prevalidated parent must have a child count");
        *children = children
            .checked_sub(1)
            .expect("post-order aggregation must complete each child once");
        if parent_id != root_id && *children == 0 {
            ready.push_back(parent_id);
        }
    }
    if completion_order.len() != eligible.len() || remaining_children[&root_id] != 0 {
        return Err(LayoutScanError::Platform(
            "directory_aggregation_cycle".to_string(),
        ));
    }
    completion_order.push(root_id);

    // Validate candidate parents before the first consumer call. Directory
    // aggregation follows Analysis boundaries, while the shared large-file
    // result must apply stricter LargeFiles boundaries. A reused in-memory
    // result does not repeat platform filtering, so protected paths must never
    // enter it.
    let mut candidate_count = 0u64;
    for candidate in &collection.candidates {
        for name in &candidate.names {
            let Some(parent) = path_cache.get(&name.parent_id).and_then(Option::as_ref) else {
                continue;
            };
            let path = parent.join(&name.name);
            if platform
                .should_skip(&path, root, ScanPurpose::LargeFiles)
                .is_some()
            {
                continue;
            }
            candidate_count = candidate_count
                .checked_add(1)
                .ok_or_else(|| LayoutScanError::Platform("candidate_count_overflow".to_string()))?;
        }
    }

    let root_totals = totals[&root_id];
    Ok(PreparedAnalysis {
        collection,
        path_cache,
        totals,
        completion_order,
        root_totals,
        candidate_count,
    })
}

fn collect_and_emit(
    platform: &WindowsPlatform,
    root: &Path,
    volume: &VolumePaths,
    minimum_bytes: u64,
    excluded_roots: &[PathBuf],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    consumer: &mut dyn FnMut(PathBuf) -> Result<(), String>,
) -> Result<CompletedLayoutScan, LayoutScanError> {
    let volume_handle = OwnedHandle::open_volume(&volume.device)
        .map_err(|error| LayoutScanError::Platform(format!("open_volume:{error}")))?;
    let volume_root_id = file_id(&volume.root)
        .map_err(|error| LayoutScanError::Platform(format!("volume_root_id:{error}")))?;
    let mut collection = enumerate_layout(
        volume_handle.raw(),
        minimum_bytes,
        LayoutCollectionMode::CandidatesOnly,
        CandidateSizeMetric::Allocated,
        is_cancelled,
    )?;

    let mut path_cache = HashMap::new();
    path_cache.insert(volume_root_id, Some(root.to_path_buf()));
    let peak_in_flight_candidates = collection.candidate_path_count;
    let mut skipped_count = 0u64;

    // Platform errors must occur before the first consumer call; otherwise
    // automatic fallback could mix partial native and complete Win32 results.
    // The first pass validates parent chains and boundaries while caching
    // parent paths. The second constructs paths without retaining another
    // PathBuf list.
    for candidate in &collection.candidates {
        if is_cancelled() {
            return Err(LayoutScanError::Cancelled);
        }
        for name in &candidate.names {
            let Some(parent) = resolve_directory_path(
                name.parent_id,
                volume_root_id,
                root,
                &collection.directories,
                &mut path_cache,
                is_cancelled,
            )?
            else {
                skipped_count = skipped_count.saturating_add(1);
                continue;
            };
            let path = parent.join(&name.name);
            if excluded_roots
                .iter()
                .any(|root| platform.path_is_same_or_child(&path, root))
            {
                continue;
            }
            if platform
                .should_skip(&path, root, ScanPurpose::LargeFiles)
                .is_some()
            {
                skipped_count = skipped_count.saturating_add(1);
            }
        }
    }

    let mut candidate_count = 0u64;
    let mut consumer_nanos = 0u64;
    for candidate in collection.candidates.drain(..) {
        if is_cancelled() {
            return Err(LayoutScanError::Cancelled);
        }
        for name in candidate.names {
            let Some(parent) = resolve_directory_path(
                name.parent_id,
                volume_root_id,
                root,
                &collection.directories,
                &mut path_cache,
                is_cancelled,
            )?
            else {
                continue;
            };
            let path = parent.join(name.name);
            if excluded_roots
                .iter()
                .any(|root| platform.path_is_same_or_child(&path, root))
            {
                continue;
            }
            if platform
                .should_skip(&path, root, ScanPurpose::LargeFiles)
                .is_some()
            {
                continue;
            }
            let consumer_started = Instant::now();
            consumer(path).map_err(LayoutScanError::Consumer)?;
            consumer_nanos = consumer_nanos.saturating_add(
                u64::try_from(consumer_started.elapsed().as_nanos()).unwrap_or(u64::MAX),
            );
            candidate_count = candidate_count.saturating_add(1);
        }
    }

    Ok(CompletedLayoutScan {
        candidate_summary: LargeFileCandidateSummary {
            native_directory_reads: 0,
            candidate_count,
            skipped_count,
            consumer_elapsed_ms: consumer_nanos / 1_000_000,
            producer_backpressure_ms: consumer_nanos / 1_000_000,
            peak_in_flight_candidates,
            strategy: "windows_file_layout_stream",
        },
        page_count: collection.page_count,
        entry_count: collection.entry_count,
        returned_bytes: collection.returned_bytes,
    })
}

fn resolve_directory_path(
    directory_id: u64,
    volume_root_id: u64,
    scan_root: &Path,
    directories: &HashMap<u64, DirectoryNode>,
    cache: &mut HashMap<u64, Option<PathBuf>>,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Option<PathBuf>, LayoutScanError> {
    if is_cancelled() {
        return Err(LayoutScanError::Cancelled);
    }
    if let Some(path) = cache.get(&directory_id) {
        return Ok(path.clone());
    }
    let mut current = directory_id;
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let base = loop {
        if is_cancelled() {
            return Err(LayoutScanError::Cancelled);
        }
        if let Some(path) = cache.get(&current) {
            break path.clone();
        }
        if current == volume_root_id {
            break Some(scan_root.to_path_buf());
        }
        if !seen.insert(current) {
            return Err(LayoutScanError::Platform(
                "directory_parent_cycle".to_string(),
            ));
        }
        if chain.len() >= MAX_DIRECTORY_CHAIN_LENGTH {
            return Err(LayoutScanError::Platform(
                "directory_parent_chain_limit_exceeded".to_string(),
            ));
        }
        let Some(node) = directories.get(&current) else {
            log::debug!(
                "windows_file_layout_parent_missing record={current} root_record={volume_root_id}"
            );
            return Err(LayoutScanError::Platform(
                "directory_parent_missing".to_string(),
            ));
        };
        if node.boundary != DirectoryBoundary::None {
            // Win32 fallback does not enter reparse directories or expose
            // reserved NTFS namespaces such as `$Extend`. Layout enumeration
            // still returns descendants, so propagating None through the
            // parent chain keeps internal objects out of ordinary analysis.
            cache.insert(current, None);
            break None;
        }
        chain.push(current);
        current = node.parent_id;
    };

    let mut resolved = base;
    for id in chain.into_iter().rev() {
        resolved = resolved.map(|path| path.join(&directories[&id].name));
        cache.insert(id, resolved.clone());
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_aggregates_visible_directories_and_skips_whole_boundaries_once() {
        let root_id = 5;
        let collection = LayoutCollection {
            directories: HashMap::from([
                (
                    10,
                    DirectoryNode {
                        parent_id: root_id,
                        name: OsString::from("Users"),
                        boundary: DirectoryBoundary::None,
                    },
                ),
                (
                    11,
                    DirectoryNode {
                        parent_id: 10,
                        name: OsString::from("sample-user"),
                        boundary: DirectoryBoundary::None,
                    },
                ),
                (
                    20,
                    DirectoryNode {
                        parent_id: root_id,
                        name: OsString::from("Windows"),
                        boundary: DirectoryBoundary::None,
                    },
                ),
                (
                    21,
                    DirectoryNode {
                        parent_id: 20,
                        name: OsString::from("System32"),
                        boundary: DirectoryBoundary::None,
                    },
                ),
                (
                    30,
                    DirectoryNode {
                        parent_id: root_id,
                        name: OsString::from("junction"),
                        boundary: DirectoryBoundary::Reparse,
                    },
                ),
                (
                    31,
                    DirectoryNode {
                        parent_id: 30,
                        name: OsString::from("outside"),
                        boundary: DirectoryBoundary::None,
                    },
                ),
                (
                    40,
                    DirectoryNode {
                        parent_id: root_id,
                        name: OsString::from("$Extend"),
                        boundary: DirectoryBoundary::Internal,
                    },
                ),
                (
                    41,
                    DirectoryNode {
                        parent_id: 40,
                        name: OsString::from("$TxfLog"),
                        boundary: DirectoryBoundary::None,
                    },
                ),
            ]),
            direct_totals: HashMap::from([
                (
                    root_id,
                    DirectoryTotals {
                        logical_bytes: 100,
                        allocated_bytes: 100,
                        file_count: 1,
                        direct_file_count: 1,
                        skipped_count: 0,
                    },
                ),
                (
                    10,
                    DirectoryTotals {
                        logical_bytes: 10,
                        allocated_bytes: 10,
                        file_count: 1,
                        direct_file_count: 1,
                        skipped_count: 0,
                    },
                ),
                (
                    11,
                    DirectoryTotals {
                        logical_bytes: 5,
                        allocated_bytes: 5,
                        file_count: 1,
                        direct_file_count: 1,
                        skipped_count: 1,
                    },
                ),
                (
                    20,
                    DirectoryTotals {
                        logical_bytes: 1_000,
                        allocated_bytes: 1_000,
                        file_count: 1,
                        direct_file_count: 1,
                        skipped_count: 0,
                    },
                ),
                (
                    21,
                    DirectoryTotals {
                        logical_bytes: 2_000,
                        allocated_bytes: 2_000,
                        file_count: 1,
                        direct_file_count: 1,
                        skipped_count: 0,
                    },
                ),
                (
                    31,
                    DirectoryTotals {
                        logical_bytes: 4_000,
                        allocated_bytes: 4_000,
                        file_count: 1,
                        direct_file_count: 1,
                        skipped_count: 0,
                    },
                ),
                (
                    41,
                    DirectoryTotals {
                        logical_bytes: 8_000,
                        allocated_bytes: 8_000,
                        file_count: 1,
                        direct_file_count: 1,
                        skipped_count: 0,
                    },
                ),
            ]),
            ..LayoutCollection::default()
        };

        let prepared = prepare_analysis(
            &WindowsPlatform,
            Path::new(r"C:\"),
            ScanPurpose::Analysis,
            root_id,
            collection,
            &|| false,
            |_| false,
        )
        .expect("directory graph should aggregate");

        assert_eq!(
            prepared.root_totals,
            DirectoryTotals {
                logical_bytes: 1_115,
                allocated_bytes: 1_115,
                file_count: 4,
                direct_file_count: 1,
                // Reparse file under sample-user, System32, and junction.
                skipped_count: 3,
            }
        );
        assert_eq!(
            prepared.totals[&10],
            DirectoryTotals {
                logical_bytes: 15,
                allocated_bytes: 15,
                file_count: 2,
                direct_file_count: 1,
                skipped_count: 1,
            }
        );
        assert_eq!(prepared.completion_order.last(), Some(&root_id));
        assert_eq!(
            prepared.totals[&20],
            DirectoryTotals {
                logical_bytes: 1_000,
                allocated_bytes: 1_000,
                file_count: 1,
                direct_file_count: 1,
                skipped_count: 1,
            }
        );
        assert!(!prepared.totals.contains_key(&21));
        assert!(!prepared.totals.contains_key(&30));
        assert!(!prepared.totals.contains_key(&40));
    }

    #[test]
    fn analysis_rejects_direct_file_with_unknown_parent() {
        let mut collection = LayoutCollection::default();
        collection.direct_totals.insert(
            999,
            DirectoryTotals {
                logical_bytes: 1,
                allocated_bytes: 1,
                file_count: 1,
                direct_file_count: 1,
                skipped_count: 0,
            },
        );

        let result = prepare_analysis(
            &WindowsPlatform,
            Path::new(r"C:\"),
            ScanPurpose::Analysis,
            5,
            collection,
            &|| false,
            |_| false,
        );

        assert!(matches!(
            result,
            Err(LayoutScanError::Platform(error))
                if error == "file_parent_directory_missing"
        ));
    }

    #[test]
    fn duplicate_progress_uses_logical_bytes_without_changing_disk_usage() {
        let totals = DirectoryTotals {
            logical_bytes: 64 * 1024 * 1024,
            allocated_bytes: 4_096,
            file_count: 1,
            direct_file_count: 1,
            skipped_count: 0,
        };

        assert_eq!(
            totals.progress_bytes(ScanPurpose::DuplicateFiles),
            totals.logical_bytes
        );
        assert_eq!(
            totals.progress_bytes(ScanPurpose::Analysis),
            totals.allocated_bytes
        );
        assert_eq!(
            totals.progress_bytes(ScanPurpose::LargeFiles),
            totals.allocated_bytes
        );
    }

    #[test]
    fn analysis_candidate_index_uses_stricter_large_file_boundary() {
        let root_id = 5;
        let collection = LayoutCollection {
            directories: HashMap::from([
                (
                    10,
                    DirectoryNode {
                        parent_id: root_id,
                        name: OsString::from("Program Files"),
                        boundary: DirectoryBoundary::None,
                    },
                ),
                (
                    20,
                    DirectoryNode {
                        parent_id: root_id,
                        name: OsString::from("Users"),
                        boundary: DirectoryBoundary::None,
                    },
                ),
            ]),
            candidates: vec![
                CandidateRecord {
                    names: vec![FileNameLink {
                        parent_id: 10,
                        name: OsString::from("protected.bin"),
                    }],
                },
                CandidateRecord {
                    names: vec![FileNameLink {
                        parent_id: 20,
                        name: OsString::from("visible.bin"),
                    }],
                },
            ],
            ..LayoutCollection::default()
        };

        let prepared = prepare_analysis(
            &WindowsPlatform,
            Path::new(r"C:\"),
            ScanPurpose::Analysis,
            root_id,
            collection,
            &|| false,
            |_| false,
        )
        .expect("candidate boundaries should pass prevalidation");

        assert_eq!(prepared.candidate_count, 1);
        assert!(
            prepared.totals.contains_key(&10),
            "analysis should still count application directories"
        );
        assert!(prepared.totals.contains_key(&20));
    }

    #[test]
    fn directory_paths_resolve_out_of_order_and_stop_at_reparse_points() {
        let root_id = 5;
        let mut directories = HashMap::new();
        directories.insert(
            11,
            DirectoryNode {
                parent_id: 10,
                name: OsString::from("sample-user-Δ"),
                boundary: DirectoryBoundary::None,
            },
        );
        directories.insert(
            10,
            DirectoryNode {
                parent_id: root_id,
                name: OsString::from("Users"),
                boundary: DirectoryBoundary::None,
            },
        );
        directories.insert(
            20,
            DirectoryNode {
                parent_id: root_id,
                name: OsString::from("junction"),
                boundary: DirectoryBoundary::Reparse,
            },
        );
        let mut cache = HashMap::from([(root_id, Some(PathBuf::from(r"C:\")))]);

        let nested = resolve_directory_path(
            11,
            root_id,
            Path::new(r"C:\"),
            &directories,
            &mut cache,
            &|| false,
        )
        .expect("parent record order should not affect resolution");
        let linked = resolve_directory_path(
            20,
            root_id,
            Path::new(r"C:\"),
            &directories,
            &mut cache,
            &|| false,
        )
        .expect("reparse directory should be skipped safely");

        assert_eq!(
            nested.as_deref(),
            Some(Path::new(r"C:\Users\sample-user-Δ"))
        );
        assert_eq!(linked, None);
    }

    #[test]
    fn directory_parent_cycles_fail_closed() {
        let directories = HashMap::from([
            (
                10,
                DirectoryNode {
                    parent_id: 11,
                    name: OsString::from("a"),
                    boundary: DirectoryBoundary::None,
                },
            ),
            (
                11,
                DirectoryNode {
                    parent_id: 10,
                    name: OsString::from("b"),
                    boundary: DirectoryBoundary::None,
                },
            ),
        ]);
        let mut cache = HashMap::from([(5, Some(PathBuf::from(r"C:\")))]);

        let result =
            resolve_directory_path(10, 5, Path::new(r"C:\"), &directories, &mut cache, &|| {
                false
            });

        assert!(matches!(
            result,
            Err(LayoutScanError::Platform(error)) if error == "directory_parent_cycle"
        ));
    }

    #[test]
    fn directory_path_resolution_honours_cancellation() {
        let directories = HashMap::from([(
            10,
            DirectoryNode {
                parent_id: 5,
                name: OsString::from("Users"),
                boundary: DirectoryBoundary::None,
            },
        )]);
        let mut cache = HashMap::from([(5, Some(PathBuf::from(r"C:\")))]);

        let result =
            resolve_directory_path(10, 5, Path::new(r"C:\"), &directories, &mut cache, &|| true);

        assert!(matches!(result, Err(LayoutScanError::Cancelled)));
    }

    #[test]
    fn directory_parent_chain_has_a_fixed_limit() {
        let root_id = 5;
        let first_id = 10_000u64;
        let mut directories = HashMap::new();
        for index in 0..=MAX_DIRECTORY_CHAIN_LENGTH {
            let id = first_id + index as u64;
            let parent_id = if index == MAX_DIRECTORY_CHAIN_LENGTH {
                root_id
            } else {
                id + 1
            };
            directories.insert(
                id,
                DirectoryNode {
                    parent_id,
                    name: OsString::from("nested"),
                    boundary: DirectoryBoundary::None,
                },
            );
        }
        let mut cache = HashMap::from([(root_id, Some(PathBuf::from(r"C:\")))]);

        let result = resolve_directory_path(
            first_id,
            root_id,
            Path::new(r"C:\"),
            &directories,
            &mut cache,
            &|| false,
        );

        assert!(matches!(
            result,
            Err(LayoutScanError::Platform(error))
                if error == "directory_parent_chain_limit_exceeded"
        ));
    }
}
