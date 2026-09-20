use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalFileIdentity {
    pub volume: u64,
    pub index: u64,
}

pub type DirectoryEntryIdentities = HashMap<OsString, PhysicalFileIdentity>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPurpose {
    Cleanup,
    Analysis,
    LargeFiles,
    DuplicateFiles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    LinkOrMount,
    SystemCritical,
    PermissionDenied,
}

/// A filesystem change boundary captured before a full scan. Volume identity
/// identifies the mounted object, while history identity identifies the event
/// log generation. They remain separate because rebuilding a Windows USN
/// journal invalidates the cursor without changing the volume. macOS uses zero
/// because its current implementation has no independent history identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilesystemChangeToken {
    pub volume_id: [u8; 16],
    pub history_id: u64,
    pub cursor: u64,
}

/// Event history answers only whether a snapshot remains trustworthy. Lost
/// history stays distinct from ordinary changes for diagnostics; both require a
/// full-scan fallback in core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemChangeStatus {
    Pending,
    Clean,
    Changed,
    HistoryUnavailable,
}

/// A change impact plan identifies directories that require remeasurement. It
/// neither mutates caches nor authorizes deletion. Platform path identity
/// deduplication and ancestor compression let core reuse its existing traversal
/// without interpreting USN reasons or file identifiers.
#[derive(Debug, Clone)]
pub struct FilesystemChangeImpactPlan {
    pub dirty_directories: Vec<PathBuf>,
    pub next_token: FilesystemChangeToken,
    pub summary: FilesystemChangeImpactSummary,
}

impl FilesystemChangeImpactPlan {
    pub fn has_changes(&self) -> bool {
        !self.dirty_directories.is_empty()
    }
}

/// The summary never stores file names, paths, or file identifiers. Categorized
/// reason counts diagnose workloads without leaking Windows constants into
/// core. One record may contribute to multiple reason categories.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilesystemChangeImpactSummary {
    pub start_cursor: u64,
    pub end_cursor: u64,
    pub page_count: u64,
    pub record_count: u64,
    pub data_change_records: u64,
    pub create_delete_records: u64,
    pub rename_records: u64,
    pub metadata_change_records: u64,
    pub other_records: u64,
    pub directory_records: u64,
    pub parent_cache_peak: usize,
    pub dirty_directory_count: u64,
    pub returned_bytes: u64,
    pub elapsed_ms: u64,
    pub strategy: &'static str,
}

/// A trait returns `None` when the capability is unsupported. Once Windows
/// recognizes a volume but cannot prove the complete impact scope, it returns a
/// stable reason. Callers must fall back to a full scan for every reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemChangeImpactUnavailable {
    UnsupportedRoot,
    UnsupportedFilesystem,
    PermissionDenied,
    HistoryUnavailable,
    ParentUnavailable,
    ResourceLimit,
    InvalidJournal,
}

impl FilesystemChangeImpactUnavailable {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedRoot => "unsupported_root",
            Self::UnsupportedFilesystem => "unsupported_filesystem",
            Self::PermissionDenied => "permission_denied",
            Self::HistoryUnavailable => "history_unavailable",
            Self::ParentUnavailable => "parent_unavailable",
            Self::ResourceLimit => "resource_limit",
            Self::InvalidJournal => "invalid_journal",
        }
    }
}

#[derive(Debug, Clone)]
pub enum FilesystemChangeImpactOutcome {
    Complete(FilesystemChangeImpactPlan),
    Unavailable(FilesystemChangeImpactUnavailable),
}

/// Cancellation remains distinct from unavailability so callers never restart a
/// full scan after user cancellation. Platform failures retain a diagnostic
/// summary but cannot publish a partial plan.
#[derive(Debug)]
pub enum FilesystemChangeImpactError {
    Cancelled,
    Platform(String),
}

/// Core holds only a clonable state handle and never touches watcher threads,
/// callbacks, or raw pointers. The backend must stop and release resources when
/// the last handle is dropped.
#[derive(Clone)]
pub struct FilesystemChangeMonitor {
    backend: Arc<dyn FilesystemChangeMonitorBackend>,
}

impl FilesystemChangeMonitor {
    #[cfg(not(target_os = "linux"))]
    pub(crate) fn new(backend: Arc<dyn FilesystemChangeMonitorBackend>) -> Self {
        Self { backend }
    }

    pub fn status(&self) -> FilesystemChangeStatus {
        self.backend.status()
    }

    /// macOS FSEvents keeps listening after history catch-up, while the current
    /// Windows USN monitor describes only the fixed window captured at creation.
    /// Cache publication needs this capability fact instead of guessing from an
    /// operating-system name.
    pub fn continuously_tracks_changes(&self) -> bool {
        self.backend.continuously_tracks_changes()
    }
}

pub(crate) trait FilesystemChangeMonitorBackend: Send + Sync {
    fn status(&self) -> FilesystemChangeStatus;
    fn continuously_tracks_changes(&self) -> bool;
}

/// A platform fast path reports production statistics only. Candidate paths are
/// streamed to core through the consumer and never retained in the summary.
#[derive(Debug, Clone)]
pub struct LargeFileCandidateSummary {
    pub candidate_count: u64,
    pub skipped_count: u64,
    pub consumer_elapsed_ms: u64,
    pub producer_backpressure_ms: u64,
    pub peak_in_flight_candidates: usize,
    pub strategy: &'static str,
}

/// Platform, consumer, and cancellation failures remain distinct. Platform
/// failures may fall back to generic traversal, consumer failures require writer
/// rollback before replay, and cancellation must never restart automatically.
#[derive(Debug)]
pub enum LargeFileCandidateScanError {
    Cancelled,
    Platform(String),
    Consumer(String),
}

/// Platform-native project marker discovery only reports aggregate diagnostics.
/// Every emitted path is still an untrusted candidate and must be validated by
/// the cleanup engine before it can become a project root.
#[derive(Debug, Clone)]
pub struct ProjectMarkerCandidateSummary {
    pub candidate_count: u64,
    pub file_count: u64,
    pub directory_count: u64,
    pub strategy: &'static str,
}

/// One completed directory batch from native project-marker discovery.
///
/// The scanner requests names and object types only, so it can report exact
/// inspected-object counts without paying the metadata cost required for byte
/// totals. Core publishes these batches as scan work; cleanup candidate sizes
/// remain authoritative only after the later artifact measurement stage.
#[derive(Debug)]
pub struct ProjectMarkerCandidateProgress {
    pub current_directory: PathBuf,
    pub file_count: u64,
    pub directory_count: u64,
}

/// Immutable scope and matching policy for one native project-marker scan.
///
/// Grouping these values keeps platform implementations aligned when project discovery gains a
/// new pruning or depth constraint. Cancellation and candidate consumption remain callbacks
/// because they describe operation state rather than the query itself.
pub struct ProjectMarkerCandidateQuery<'a> {
    pub root: &'a Path,
    pub file_names: &'a [String],
    pub file_suffixes: &'a [String],
    pub pruned_directory_names: &'a [String],
    pub maximum_depth: usize,
}

/// Project marker discovery keeps cancellation, platform availability, and
/// consumer validation failures separate so callers can make safe fallback
/// decisions without accidentally restarting a cancelled scan.
#[derive(Debug)]
pub enum ProjectMarkerCandidateScanError {
    Cancelled,
    Unavailable(String),
    Platform(String),
    Consumer(String),
}

/// Immutable scope and pruning policy for one native analysis scan.
///
/// Core owns the product-specific pruning predicate. Platform implementations apply it before
/// descending into a directory and continue to own filesystem facts such as mount and system
/// protection boundaries. Keeping the query typed prevents the analysis and duplicate adapters
/// from silently drifting as native traversal evolves.
pub struct FastAnalysisQuery<'a> {
    pub root: &'a Path,
    pub purpose: ScanPurpose,
    pub large_file_minimum_bytes: u64,
    pub should_prune_directory: fn(&Path) -> bool,
}

/// Distinguishes a file's content length from the storage charged to its volume.
///
/// Logical bytes remain the stable identity and preflight fact. Allocated bytes
/// drive disk-usage reporting so sparse and compressed files do not inflate the
/// space that users can actually attribute to a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSpaceUsage {
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
}

impl FileSpaceUsage {
    pub fn logical_only(logical_bytes: u64) -> Self {
        Self {
            logical_bytes,
            allocated_bytes: logical_bytes,
        }
    }
}

/// Native analysis emits only records needed to build the existing core index.
/// Directory aggregates include all visible descendants. Large-file paths are
/// untrusted candidates whose live metadata is revalidated by core. A single
/// record consumer preserves ordering and failure atomicity without exposing
/// core storage details to the platform crate.
#[derive(Debug)]
pub enum FastAnalysisRecord {
    Directory {
        path: PathBuf,
        logical_bytes: u64,
        allocated_bytes: u64,
        file_count: u64,
        skipped_count: u64,
    },
    LargeFileCandidate(PathBuf),
}

/// The summary stores no user paths. It contains only root aggregates and
/// production diagnostics; directory and candidate records are streamed once.
#[derive(Debug, Clone)]
pub struct FastAnalysisSummary {
    pub root_logical_bytes: u64,
    pub root_allocated_bytes: u64,
    pub root_file_count: u64,
    pub root_skipped_count: u64,
    pub page_count: u64,
    pub entry_count: u64,
    pub directory_count: u64,
    pub candidate_count: u64,
    pub returned_bytes: u64,
    pub consumer_elapsed_ms: u64,
    pub strategy: &'static str,
}

/// Native aggregation failures may safely fall back to generic traversal.
/// Consumer failures require staging rollback. Cancellation ends the operation
/// without starting a slower full scan.
#[derive(Debug)]
pub enum FastAnalysisScanError {
    Cancelled,
    /// A previous cancelled native scan still owns readers blocked inside the operating system.
    /// Callers should retry later instead of starting another worker pool or falling back to a
    /// second traversal that can hit the same unavailable provider.
    Busy,
    Platform(String),
    Consumer(String),
}
