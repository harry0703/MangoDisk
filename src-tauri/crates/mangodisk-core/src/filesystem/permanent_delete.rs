use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
};

use mangodisk_platform::{current_platform, FileSpaceUsage, Platform, ScanPurpose};

use crate::{
    filesystem::{
        metadata::{diagnostic_path, modified_ms},
        PermanentDeleteCandidate,
    },
    shared::CoreErrorReason,
    storage::analysis::{AnalysisDeleteResult, AnalysisEntryCandidate},
};

pub(crate) struct AnalysisDeleteOutcome {
    pub(crate) target: PathBuf,
    pub(crate) removed_usage: FileSpaceUsage,
    pub(crate) result: AnalysisDeleteResult,
}

static NEXT_DELETE_STAGING_ID: AtomicU64 = AtomicU64::new(1);
// Small directories remain serial because thread startup can cost more than
// their filesystem work. Larger batches use conservative platform caps chosen
// from repeated release benchmarks; higher concurrency caused unstable delete
// latency, especially on macOS and Windows systems with active file indexing.
const PARALLEL_DELETE_ENTRY_THRESHOLD: usize = 512;
const PARALLEL_DELETE_BATCH_SIZE: usize = 8_192;
#[cfg(target_os = "macos")]
const MAX_PARALLEL_DELETE_WORKERS: usize = 2;
#[cfg(windows)]
const MAX_PARALLEL_DELETE_WORKERS: usize = 4;
#[cfg(target_os = "linux")]
const MAX_PARALLEL_DELETE_WORKERS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PhysicalPathIdentity {
    volume: u64,
    index: u64,
}

/// Binds domain validation to one physical filesystem object.
///
/// Callers capture this value before their final ownership and snapshot checks.
/// The deletion boundary then moves the current path into private staging and
/// compares its physical identity with this capture before removing anything.
/// A same-path replacement therefore fails closed instead of deleting the new
/// object that appeared after validation.
pub(crate) struct PreparedPermanentDelete {
    path: PathBuf,
    metadata: fs::Metadata,
    identity: PhysicalPathIdentity,
    // Keep the identity handle open through the staging rename. This prevents
    // an unlinked object identity from being reused before the post-rename
    // comparison. Windows opens it with delete sharing so the handle does not
    // block MangoDisk's own rename operation.
    _identity_handle: fs::File,
}

/// Counts regular files that the permanent-delete boundary actually removed.
///
/// Complete-root cleanup cannot treat an earlier scan summary as the actual
/// result: tools may still write before atomic staging. Accumulating the live
/// removal traversal keeps progress and history truthful. The internal mutation
/// marker also records removals, such as links and directories, that intentionally
/// do not contribute to regular-file accounting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PermanentDeleteOutcome {
    released_bytes: u64,
    affected_item_count: u64,
    had_irreversible_mutation: bool,
}

impl PermanentDeleteOutcome {
    pub(crate) fn released_bytes(self) -> u64 {
        self.released_bytes
    }

    pub(crate) fn affected_item_count(self) -> u64 {
        self.affected_item_count
    }

    fn add(&mut self, other: Self) {
        self.released_bytes = self.released_bytes.saturating_add(other.released_bytes);
        self.affected_item_count = self
            .affected_item_count
            .saturating_add(other.affected_item_count);
        self.had_irreversible_mutation |= other.had_irreversible_mutation;
    }
}

impl PreparedPermanentDelete {
    pub(crate) fn metadata(&self) -> &fs::Metadata {
        &self.metadata
    }

    /// Returns the physical identity captured by the permanent-delete boundary.
    /// Scan-authorized cleanup uses this value to reject a same-path replacement
    /// without opening the target a second time immediately before deletion.
    pub(crate) fn identity(&self) -> PhysicalPathIdentity {
        self.identity
    }
}

pub(crate) fn physical_path_identity_snapshot(
    path: &Path,
) -> Result<PhysicalPathIdentity, PermanentDeleteError> {
    physical_path_identity(path)
}

#[derive(Debug)]
pub(crate) struct PermanentDeleteError {
    message: String,
    reason: Option<CoreErrorReason>,
    released_bytes: u64,
    affected_item_count: u64,
    partial: bool,
    remaining_restored: bool,
}

/// Captures a stable target before domain-specific validation begins.
///
/// The path is checked on both sides of the capture. Later replacement is
/// detected after the atomic staging rename, while an already present link or
/// reparse point is rejected before any mutation occurs.
pub(crate) fn prepare_path_for_permanent_delete(
    path: &Path,
) -> Result<PreparedPermanentDelete, PermanentDeleteError> {
    current_platform()
        .validate_path_no_links(path)
        .map_err(|error| error.to_string())?;

    #[cfg(unix)]
    let (metadata, identity, identity_handle) = {
        // Reject special files before opening them. A blocking read-only open
        // on a FIFO waits indefinitely when no writer is connected.
        let initial_metadata = fs::symlink_metadata(path).map_err(|error| {
            permanent_delete_io_error("failed to read deletion target metadata", error)
        })?;
        validate_permanent_delete_target_metadata(&initial_metadata)?;
        let initial_identity = metadata_identity(&initial_metadata);
        let identity_handle = open_identity_handle(path)?;
        let metadata = identity_handle.metadata().map_err(|error| {
            permanent_delete_io_error("failed to read deletion target metadata", error)
        })?;
        validate_permanent_delete_target_metadata(&metadata)?;
        let identity = metadata_identity(&metadata);
        if identity != initial_identity {
            return Err("the item changed while its physical identity was captured"
                .to_string()
                .into());
        }
        current_platform()
            .validate_path_no_links(path)
            .map_err(|error| error.to_string())?;
        let verified = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        if current_platform().is_link_like(&verified) || metadata_identity(&verified) != identity {
            return Err("the item changed while its physical identity was captured"
                .to_string()
                .into());
        }
        (metadata, identity, identity_handle)
    };

    #[cfg(windows)]
    let (metadata, identity, identity_handle) = {
        let identity_handle = open_identity_handle(path)?;
        let metadata = identity_handle.metadata().map_err(|error| {
            permanent_delete_io_error("failed to read deletion target metadata", error)
        })?;
        validate_permanent_delete_target_metadata(&metadata)?;
        let identity = handle_identity(&identity_handle)?;
        current_platform()
            .validate_path_no_links(path)
            .map_err(|error| error.to_string())?;
        (metadata, identity, identity_handle)
    };

    Ok(PreparedPermanentDelete {
        path: path.to_path_buf(),
        metadata,
        identity,
        _identity_handle: identity_handle,
    })
}

fn validate_permanent_delete_target_metadata(
    metadata: &fs::Metadata,
) -> Result<(), PermanentDeleteError> {
    if current_platform().is_link_like(metadata) {
        return Err(
            "MangoDisk cannot permanently delete a link or reparse point"
                .to_string()
                .into(),
        );
    }
    if metadata.is_file() || metadata.is_dir() {
        return Ok(());
    }
    Err(
        "only regular files and directories can be permanently deleted"
            .to_string()
            .into(),
    )
}

impl PermanentDeleteError {
    fn before_mutation(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            reason: None,
            released_bytes: 0,
            affected_item_count: 0,
            partial: false,
            remaining_restored: false,
        }
    }

    fn before_mutation_with_reason(message: impl Into<String>, reason: CoreErrorReason) -> Self {
        Self {
            message: message.into(),
            reason: Some(reason),
            released_bytes: 0,
            affected_item_count: 0,
            partial: false,
            remaining_restored: false,
        }
    }

    fn after_mutation(
        message: impl Into<String>,
        reason: Option<CoreErrorReason>,
        released_bytes: u64,
        affected_item_count: u64,
    ) -> Self {
        Self {
            message: message.into(),
            reason,
            released_bytes,
            affected_item_count,
            partial: true,
            remaining_restored: false,
        }
    }

    pub(crate) fn released_bytes(&self) -> u64 {
        self.released_bytes
    }

    pub(crate) fn is_partial(&self) -> bool {
        self.partial
    }

    pub(crate) fn affected_item_count(&self) -> u64 {
        self.affected_item_count
    }

    pub(crate) fn remaining_was_restored(&self) -> bool {
        self.remaining_restored
    }

    pub(crate) fn reason(&self) -> Option<CoreErrorReason> {
        self.reason
    }
}

impl fmt::Display for PermanentDeleteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for PermanentDeleteError {}

impl From<String> for PermanentDeleteError {
    fn from(message: String) -> Self {
        Self::before_mutation(message)
    }
}

fn permanent_delete_io_error(context: &str, error: std::io::Error) -> PermanentDeleteError {
    let message = format!("{context}: {error}");
    match permanent_delete_io_reason(&error) {
        Some(reason) => PermanentDeleteError::before_mutation_with_reason(message, reason),
        None => PermanentDeleteError::before_mutation(message),
    }
}

fn permanent_delete_io_reason(error: &std::io::Error) -> Option<CoreErrorReason> {
    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(32 | 33)) {
        return Some(CoreErrorReason::ResourceBusy);
    }

    // Windows may report an open directory as access denied rather than a
    // sharing violation. Keep that ambiguity explicit so the UI recommends
    // both closing users of the item and checking permissions.
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        return Some(CoreErrorReason::AccessDeniedOrBusy);
    }
    None
}

/// Validates and permanently deletes one analysis entry without updating a domain cache.
/// The analysis service owns cache synchronization because filesystem helpers
/// must not depend on storage-domain state.
pub(crate) fn delete_analysis_candidate_permanently(
    candidate: AnalysisEntryCandidate,
) -> Result<AnalysisDeleteOutcome, PermanentDeleteError> {
    let requested_root = PathBuf::from(&candidate.root);
    let root = current_platform()
        .canonicalize_no_links(&requested_root)
        .map_err(|error| format!("failed to access the current analysis directory: {error}"))?;
    let requested_target = PathBuf::from(&candidate.path);
    current_platform()
        .validate_path_no_links(&requested_target)
        .map_err(|error| error.to_string())?;
    let prepared = prepare_path_for_permanent_delete(&requested_target)?;
    let requested_metadata = prepared.metadata();
    // Canonicalization follows symbolic links. Reject links and reparse
    // points before resolving the path so the target cannot be moved instead
    // of the link itself.
    if current_platform().is_link_like(requested_metadata) {
        return Err("MangoDisk cannot process a link or reparse point"
            .to_string()
            .into());
    }
    let target = current_platform()
        .canonicalize_no_links(&requested_target)
        .map_err(|error| format!("failed to access the requested item: {error}"))?;

    // The UI displays only direct children of the current directory. Enforce
    // the same boundary in Core so a modified request cannot target the scan
    // root or an arbitrary system path.
    if target.parent() != Some(root.as_path()) {
        return Err(
            "only direct children of the current analysis directory can be processed"
                .to_string()
                .into(),
        );
    }

    let metadata = prepared.metadata();
    let target_type_matches = if candidate.is_directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    };
    if !target_type_matches {
        return Err("the item type changed after scanning".to_string().into());
    }
    // The explicit confirmation authorizes deletion of the current regular
    // file or directory at this direct-child path. Scan metadata is retained
    // only for result accounting: rejecting normal changes between analysis
    // and confirmation made a user-requested delete appear broken. Physical
    // identity remains pinned from this preparation through the staging rename,
    // so a concurrent replacement during execution still fails closed.
    if current_platform()
        .should_skip(
            &target,
            &current_platform().system_volume_path(),
            ScanPurpose::Analysis,
        )
        .is_some()
    {
        return Err(
            "MangoDisk cannot process a protected system or application item"
                .to_string()
                .into(),
        );
    }

    let verified_target = current_platform()
        .canonicalize_no_links(&requested_target)
        .map_err(|error| error.to_string())?;
    if verified_target != target {
        return Err("the item changed during safety validation"
            .to_string()
            .into());
    }
    delete_path_permanently(
        prepared,
        candidate.expected_logical_bytes,
        candidate.expected_file_count,
    )?;
    let removed_usage = FileSpaceUsage {
        logical_bytes: candidate.expected_logical_bytes,
        allocated_bytes: candidate.expected_allocated_bytes,
    };
    Ok(AnalysisDeleteOutcome {
        target,
        removed_usage,
        result: AnalysisDeleteResult {
            removed_path: candidate.path,
            released_bytes: candidate.expected_allocated_bytes,
            removed_file_count: candidate.expected_file_count,
        },
    })
}

/// Deletes a preflighted regular file or directory without following a top-level link.
///
/// Domain services must complete their own ownership, snapshot, protected-path, and running
/// process checks before calling this irreversible boundary.
pub(crate) fn delete_path_permanently(
    target: PreparedPermanentDelete,
    expected_bytes: u64,
    expected_item_count: u64,
) -> Result<(), PermanentDeleteError> {
    if target.metadata.is_file() {
        delete_via_staging(
            target,
            expected_bytes,
            expected_item_count,
            StagedRemoval::File,
        )
        .map(|_| ())
    } else if target.metadata.is_dir() {
        delete_via_staging(
            target,
            expected_bytes,
            expected_item_count,
            StagedRemoval::DirectoryTree,
        )
        .map(|_| ())
    } else {
        Err(
            "only regular files and directories can be permanently deleted"
                .to_string()
                .into(),
        )
    }
}

/// Atomically stages a directory, then removes the tree in one cancellable pass.
///
/// Per-file cleanup checks cancellation between candidates; the whole-root fast
/// path must preserve that contract. It removes per-file identity and staging
/// transactions without sacrificing cancellation, link safety, or accounting.
pub(crate) fn delete_directory_tree_permanently_with_cancellation(
    target: PreparedPermanentDelete,
    expected_bytes: u64,
    expected_item_count: u64,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<PermanentDeleteOutcome, PermanentDeleteError> {
    if !target.metadata.is_dir() {
        return Err(
            "only a regular directory can be removed as a directory tree"
                .to_string()
                .into(),
        );
    }
    delete_via_staging(
        target,
        expected_bytes,
        expected_item_count,
        StagedRemoval::CancellableDirectoryTree(is_cancelled),
    )
}

/// Deletes files from a staged directory tree while retaining directories that
/// were already empty before cleanup.
///
/// The staged traversal applies the same pruning rule as generic cleanup:
/// directories are removed only after at least one child was removed and no
/// child remains. Any retained directory skeleton is atomically restored with
/// its original identity and metadata after the destructive pass.
pub(crate) fn delete_directory_contents_permanently_with_cancellation(
    target: PreparedPermanentDelete,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<PermanentDeleteOutcome, PermanentDeleteError> {
    delete_directory_contents_permanently_with_cancellation_mode(target, is_cancelled, true)
}

#[cfg(test)]
pub(crate) fn delete_directory_contents_permanently_with_cancellation_serial(
    target: PreparedPermanentDelete,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<PermanentDeleteOutcome, PermanentDeleteError> {
    delete_directory_contents_permanently_with_cancellation_mode(target, is_cancelled, false)
}

fn delete_directory_contents_permanently_with_cancellation_mode(
    target: PreparedPermanentDelete,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    allow_parallel: bool,
) -> Result<PermanentDeleteOutcome, PermanentDeleteError> {
    if !target.metadata.is_dir() {
        return Err(
            "only a regular directory can be cleaned as directory contents"
                .to_string()
                .into(),
        );
    }
    delete_via_staging(
        target,
        0,
        0,
        StagedRemoval::CancellableDirectoryContents {
            is_cancelled,
            allow_parallel,
        },
    )
}

/// Removes a directory only when it is still the captured object and remains
/// empty. Declarative cleanup uses this boundary after removing individually
/// authorized children, so concurrent new files are preserved rather than
/// being swept up by a recursive delete.
pub(crate) fn delete_empty_directory_permanently(
    target: PreparedPermanentDelete,
) -> Result<(), PermanentDeleteError> {
    if !target.metadata.is_dir() {
        return Err("only a regular directory can be removed as empty"
            .to_string()
            .into());
    }
    delete_via_staging(target, 0, 0, StagedRemoval::EmptyDirectory).map(|_| ())
}

#[derive(Clone, Copy)]
enum StagedRemoval<'a> {
    File,
    EmptyDirectory,
    DirectoryTree,
    CancellableDirectoryTree(&'a (dyn Fn() -> bool + Sync)),
    CancellableDirectoryContents {
        is_cancelled: &'a (dyn Fn() -> bool + Sync),
        allow_parallel: bool,
    },
}

struct StagedRemovalSuccess {
    outcome: PermanentDeleteOutcome,
    restore_remainder: bool,
}

struct StagedRemovalFailure {
    error: std::io::Error,
    verified_outcome: Option<PermanentDeleteOutcome>,
}

/// Moves a directory to a private same-volume staging location before recursively deleting it.
///
/// Recursive deletion is not atomic on either supported platform. The rename first removes the
/// original path atomically, while a failed recursive deletion can restore whatever remains. The
/// structured error records bytes that were already released so callers do not report a false
/// zero-byte outcome.
fn delete_via_staging(
    target: PreparedPermanentDelete,
    expected_bytes: u64,
    expected_item_count: u64,
    removal: StagedRemoval<'_>,
) -> Result<PermanentDeleteOutcome, PermanentDeleteError> {
    let path = target.path.as_path();
    let parent = path
        .parent()
        .ok_or_else(|| PermanentDeleteError::before_mutation("the directory has no parent"))?;
    let staging_root = create_staging_directory(parent)?;
    let staged_target = staging_root.join("target");
    if let Err(error) = fs::rename(path, &staged_target) {
        let _ = fs::remove_dir(&staging_root);
        return Err(permanent_delete_io_error(
            "failed to prepare the item for permanent deletion",
            error,
        ));
    }

    let staged_identity_matches =
        physical_path_identity(&staged_target).is_ok_and(|identity| identity == target.identity);
    if !staged_identity_matches {
        return rollback_staged_target(
            path,
            &staging_root,
            &staged_target,
            "the item was replaced before permanent deletion",
            Some(CoreErrorReason::ItemChanged),
            PermanentDeleteOutcome::default(),
        )
        .map(|_| PermanentDeleteOutcome::default());
    }

    let expected_outcome = PermanentDeleteOutcome {
        released_bytes: expected_bytes,
        affected_item_count: expected_item_count,
        had_irreversible_mutation: false,
    };
    let removal_result = match removal {
        StagedRemoval::File => fs::remove_file(&staged_target)
            .map(|_| StagedRemovalSuccess {
                outcome: expected_outcome,
                restore_remainder: false,
            })
            .map_err(|error| StagedRemovalFailure {
                error,
                verified_outcome: None,
            }),
        StagedRemoval::EmptyDirectory => fs::remove_dir(&staged_target)
            .map(|_| StagedRemovalSuccess {
                outcome: PermanentDeleteOutcome::default(),
                restore_remainder: false,
            })
            .map_err(|error| StagedRemovalFailure {
                error,
                verified_outcome: None,
            }),
        StagedRemoval::DirectoryTree => fs::remove_dir_all(&staged_target)
            .map(|_| StagedRemovalSuccess {
                outcome: expected_outcome,
                restore_remainder: false,
            })
            .map_err(|error| StagedRemovalFailure {
                error,
                verified_outcome: None,
            }),
        StagedRemoval::CancellableDirectoryTree(is_cancelled) => {
            remove_directory_tree_cancellable(&staged_target, is_cancelled).map(|outcome| {
                StagedRemovalSuccess {
                    outcome,
                    restore_remainder: false,
                }
            })
        }
        StagedRemoval::CancellableDirectoryContents {
            is_cancelled,
            allow_parallel,
        } => remove_directory_contents_cancellable(&staged_target, is_cancelled, allow_parallel)
            .map(|(outcome, restore_remainder)| StagedRemovalSuccess {
                outcome,
                restore_remainder,
            }),
    };
    match removal_result {
        Ok(success) => {
            if success.restore_remainder {
                if !physical_path_identity(&staged_target)
                    .is_ok_and(|identity| identity == target.identity)
                {
                    return Err(PermanentDeleteError::after_mutation(
                        "the retained directory identity changed and could not be restored",
                        Some(CoreErrorReason::ItemChanged),
                        success.outcome.released_bytes,
                        success.outcome.affected_item_count,
                    ));
                }
                if let Err(error) = fs::rename(&staged_target, path) {
                    log::error!(
                        "permanent_delete_remainder_restore_failed target={} staging={} error={}",
                        diagnostic_path(path),
                        diagnostic_path(&staged_target),
                        mangodisk_platform::diagnostics::text(&error)
                    );
                    return Err(PermanentDeleteError::after_mutation(
                        "the retained directory skeleton could not be restored automatically",
                        permanent_delete_io_reason(&error),
                        success.outcome.released_bytes,
                        success.outcome.affected_item_count,
                    ));
                }
            }
            if let Err(error) = fs::remove_dir(&staging_root) {
                log::warn!(
                    "permanent_delete_staging_cleanup_failed staging={} error={}",
                    diagnostic_path(&staging_root),
                    mangodisk_platform::diagnostics::text(&error)
                );
            }
            Ok(success.outcome)
        }
        Err(delete_failure) => {
            if !staged_target.exists() {
                let _ = fs::remove_dir(&staging_root);
                return Ok(delete_failure.verified_outcome.unwrap_or(expected_outcome));
            }
            if !physical_path_identity(&staged_target)
                .is_ok_and(|identity| identity == target.identity)
            {
                log::error!(
                    "permanent_delete_staging_identity_changed staging={} error={}",
                    diagnostic_path(&staged_target),
                    mangodisk_platform::diagnostics::text(&delete_failure.error)
                );
                return Err(PermanentDeleteError::after_mutation(
                    "the staged item changed and could not be restored automatically",
                    Some(CoreErrorReason::ItemChanged),
                    0,
                    0,
                ));
            }
            // The cancellable traversal supplies verified live totals, avoiding
            // another full scan after cancellation. Legacy standard removals do
            // not collect per-entry totals and still infer them from the remainder.
            let verified_outcome = delete_failure.verified_outcome.or_else(|| {
                measure_remaining(&staged_target)
                    .ok()
                    .map(|remaining| PermanentDeleteOutcome {
                        released_bytes: expected_bytes.saturating_sub(remaining.bytes),
                        affected_item_count: expected_item_count
                            .saturating_sub(remaining.item_count),
                        had_irreversible_mutation: false,
                    })
            });
            rollback_staged_target(
                path,
                &staging_root,
                &staged_target,
                &format!(
                    "failed to permanently delete the item: {}",
                    delete_failure.error
                ),
                permanent_delete_io_reason(&delete_failure.error),
                verified_outcome.unwrap_or_default(),
            )
            .map(|_| PermanentDeleteOutcome::default())
        }
    }
}

/// Removes a staged tree while observing cancellation between directory entries.
///
/// `remove_dir_all` has no cancellation hook, which is unacceptable for the
/// large small-file trees most likely to be cancelled. The traversal uses
/// `symlink_metadata` and the platform link-like policy. Unix symbolic-link
/// entries are unlinked without following their targets. Other link-like
/// entries, including Windows reparse points, stop deletion and roll back the
/// remainder.
fn remove_directory_tree_cancellable(
    root: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<PermanentDeleteOutcome, StagedRemovalFailure> {
    let mut outcome = PermanentDeleteOutcome::default();
    remove_directory_tree_entry(root, is_cancelled, &mut outcome).map_err(|error| {
        StagedRemovalFailure {
            error,
            verified_outcome: Some(outcome),
        }
    })?;
    Ok(outcome)
}

fn remove_directory_tree_entry(
    path: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    outcome: &mut PermanentDeleteOutcome,
) -> Result<(), std::io::Error> {
    if is_cancelled() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "directory tree deletion cancelled",
        ));
    }
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(unix)]
    if metadata.file_type().is_symlink() {
        fs::remove_file(path)?;
        outcome.had_irreversible_mutation = true;
        return Ok(());
    }
    if current_platform().is_link_like(&metadata) {
        return Err(std::io::Error::other(
            "a link or reparse point appeared during directory tree deletion",
        ));
    }
    if metadata.is_file() {
        let bytes = metadata.len();
        fs::remove_file(path)?;
        outcome.had_irreversible_mutation = true;
        outcome.released_bytes = outcome.released_bytes.saturating_add(bytes);
        outcome.affected_item_count = outcome.affected_item_count.saturating_add(1);
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(std::io::Error::other(
            "an unsupported filesystem entry appeared during directory tree deletion",
        ));
    }

    for entry in fs::read_dir(path)? {
        if is_cancelled() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "directory tree deletion cancelled",
            ));
        }
        remove_directory_tree_entry(&entry?.path(), is_cancelled, outcome)?;
    }
    if is_cancelled() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "directory tree deletion cancelled",
        ));
    }
    fs::remove_dir(path)?;
    outcome.had_irreversible_mutation = true;
    Ok(())
}

fn remove_directory_contents_cancellable(
    root: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    allow_parallel: bool,
) -> Result<(PermanentDeleteOutcome, bool), StagedRemovalFailure> {
    let mut outcome = PermanentDeleteOutcome::default();
    let root_removed =
        remove_directory_contents_entry(root, is_cancelled, &mut outcome, allow_parallel).map_err(
            |error| StagedRemovalFailure {
                error,
                verified_outcome: Some(outcome),
            },
        )?;
    Ok((outcome, !root_removed))
}

fn remove_directory_contents_entry(
    path: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    outcome: &mut PermanentDeleteOutcome,
    allow_parallel: bool,
) -> Result<bool, std::io::Error> {
    if is_cancelled() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "directory contents deletion cancelled",
        ));
    }
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(unix)]
    if metadata.file_type().is_symlink() {
        fs::remove_file(path)?;
        outcome.had_irreversible_mutation = true;
        return Ok(true);
    }
    if current_platform().is_link_like(&metadata) {
        return Err(std::io::Error::other(
            "a link or reparse point appeared during directory contents deletion",
        ));
    }
    if metadata.is_file() {
        let bytes = metadata.len();
        fs::remove_file(path)?;
        outcome.had_irreversible_mutation = true;
        outcome.released_bytes = outcome.released_bytes.saturating_add(bytes);
        outcome.affected_item_count = outcome.affected_item_count.saturating_add(1);
        return Ok(true);
    }
    if !metadata.is_dir() {
        return Err(std::io::Error::other(
            "an unsupported filesystem entry appeared during directory contents deletion",
        ));
    }

    let mut entries = fs::read_dir(path)?;
    let mut had_entry = false;
    let mut all_removed = true;
    loop {
        let batch = entries
            .by_ref()
            .take(PARALLEL_DELETE_BATCH_SIZE)
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?;
        if batch.is_empty() {
            break;
        }
        had_entry = true;
        if allow_parallel && batch.len() >= PARALLEL_DELETE_ENTRY_THRESHOLD {
            let result = remove_directory_entries_parallel(&batch, is_cancelled);
            outcome.add(result.outcome);
            all_removed &= result.all_removed;
            if let Some(error) = result.error {
                return Err(error);
            }
        } else {
            for child in batch {
                if is_cancelled() {
                    return Err(directory_contents_cancelled_error());
                }
                if !remove_directory_contents_entry(&child, is_cancelled, outcome, allow_parallel)?
                {
                    all_removed = false;
                }
            }
        }
    }
    if had_entry && all_removed {
        fs::remove_dir(path)?;
        outcome.had_irreversible_mutation = true;
        return Ok(true);
    }
    Ok(false)
}

struct ParallelDirectoryRemovalResult {
    outcome: PermanentDeleteOutcome,
    all_removed: bool,
    error: Option<std::io::Error>,
}

fn remove_directory_entries_parallel(
    entries: &[PathBuf],
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> ParallelDirectoryRemovalResult {
    let worker_count = thread::available_parallelism()
        .map_or(1, usize::from)
        .min(MAX_PARALLEL_DELETE_WORKERS)
        .min(entries.len());
    if worker_count <= 1 {
        return remove_directory_entries_serial(entries, is_cancelled, true);
    }

    let stopped = AtomicBool::new(false);
    let worker_results = thread::scope(|scope| {
        (0..worker_count)
            .map(|worker_index| {
                let stopped = &stopped;
                scope.spawn(move || {
                    let mut result = ParallelDirectoryRemovalResult {
                        outcome: PermanentDeleteOutcome::default(),
                        all_removed: true,
                        error: None,
                    };
                    for path in entries.iter().skip(worker_index).step_by(worker_count) {
                        if stopped.load(Ordering::Relaxed) {
                            break;
                        }
                        if is_cancelled() {
                            stopped.store(true, Ordering::Relaxed);
                            result.error = Some(directory_contents_cancelled_error());
                            break;
                        }
                        match remove_directory_contents_entry(
                            path,
                            is_cancelled,
                            &mut result.outcome,
                            false,
                        ) {
                            Ok(removed) => result.all_removed &= removed,
                            Err(error) => {
                                stopped.store(true, Ordering::Relaxed);
                                result.error = Some(error);
                                break;
                            }
                        }
                    }
                    result
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|worker| worker.join())
            .collect::<Vec<_>>()
    });

    let mut combined = ParallelDirectoryRemovalResult {
        outcome: PermanentDeleteOutcome::default(),
        all_removed: true,
        error: None,
    };
    for worker in worker_results {
        match worker {
            Ok(result) => {
                combined.outcome.add(result.outcome);
                combined.all_removed &= result.all_removed;
                if combined.error.is_none() {
                    combined.error = result.error;
                }
            }
            Err(_) if combined.error.is_none() => {
                combined.error = Some(std::io::Error::other(
                    "a parallel directory deletion worker stopped unexpectedly",
                ));
            }
            Err(_) => {}
        }
    }
    combined
}

fn remove_directory_entries_serial(
    entries: &[PathBuf],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    allow_parallel: bool,
) -> ParallelDirectoryRemovalResult {
    let mut result = ParallelDirectoryRemovalResult {
        outcome: PermanentDeleteOutcome::default(),
        all_removed: true,
        error: None,
    };
    for path in entries {
        if is_cancelled() {
            result.error = Some(directory_contents_cancelled_error());
            break;
        }
        match remove_directory_contents_entry(
            path,
            is_cancelled,
            &mut result.outcome,
            allow_parallel,
        ) {
            Ok(removed) => result.all_removed &= removed,
            Err(error) => {
                result.error = Some(error);
                break;
            }
        }
    }
    result
}

fn directory_contents_cancelled_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::Interrupted,
        "directory contents deletion cancelled",
    )
}

fn rollback_staged_target(
    original_path: &Path,
    staging_root: &Path,
    staged_target: &Path,
    reason: &str,
    failure_reason: Option<CoreErrorReason>,
    outcome: PermanentDeleteOutcome,
) -> Result<(), PermanentDeleteError> {
    let PermanentDeleteOutcome {
        released_bytes,
        affected_item_count,
        had_irreversible_mutation,
    } = outcome;
    match fs::rename(staged_target, original_path) {
        Ok(()) => {
            let _ = fs::remove_dir(staging_root);
            let partially_deleted =
                released_bytes > 0 || affected_item_count > 0 || had_irreversible_mutation;
            let message = if partially_deleted {
                format!(
                    "the item was partially deleted; remaining contents were restored: {reason}"
                )
            } else {
                format!("the item was not deleted and was restored: {reason}")
            };
            Err(if partially_deleted {
                let mut error = PermanentDeleteError::after_mutation(
                    message,
                    failure_reason,
                    released_bytes,
                    affected_item_count,
                );
                error.remaining_restored = true;
                error
            } else {
                let mut error = PermanentDeleteError::before_mutation(message);
                error.reason = failure_reason;
                error
            })
        }
        Err(rollback_error) => {
            log::error!(
                "permanent_delete_rollback_failed target={} staging={} reason={} rollback_error={}",
                diagnostic_path(original_path),
                diagnostic_path(staged_target),
                mangodisk_platform::diagnostics::text(&reason),
                mangodisk_platform::diagnostics::text(&rollback_error)
            );
            Err(PermanentDeleteError::after_mutation(
                format!(
                    "the item could not be restored automatically after permanent deletion stopped: {reason}"
                ),
                failure_reason,
                released_bytes,
                affected_item_count,
            ))
        }
    }
}

fn create_staging_directory(parent: &Path) -> Result<PathBuf, PermanentDeleteError> {
    for _ in 0..32 {
        let id = NEXT_DELETE_STAGING_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".mangodisk-delete-{}-{id}", std::process::id()));
        #[cfg(unix)]
        let create_result = {
            use std::os::unix::fs::DirBuilderExt;

            // Administrator authorization may keep this directory present
            // while a user responds to the system prompt. Create it private
            // atomically so another local account cannot alter entries inside.
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            builder.create(&path)
        };
        #[cfg(windows)]
        let create_result = fs::create_dir(&path);
        match create_result {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(permanent_delete_io_error(
                    "failed to create a private deletion staging directory",
                    error,
                ));
            }
        }
    }
    Err(PermanentDeleteError::before_mutation(
        "failed to reserve a unique deletion staging directory",
    ))
}

#[derive(Clone, Copy)]
struct RemainingMeasurement {
    bytes: u64,
    item_count: u64,
}

fn measure_remaining(path: &Path) -> Result<RemainingMeasurement, std::io::Error> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_file() {
        return Ok(RemainingMeasurement {
            bytes: metadata.len(),
            item_count: 1,
        });
    }
    if !metadata.is_dir() || current_platform().is_link_like(&metadata) {
        return Ok(RemainingMeasurement {
            bytes: metadata.len(),
            item_count: 1,
        });
    }
    let mut measurement = RemainingMeasurement {
        bytes: 0,
        item_count: 0,
    };
    for entry in fs::read_dir(path)? {
        let child = measure_remaining(&entry?.path())?;
        measurement.bytes = measurement.bytes.saturating_add(child.bytes);
        measurement.item_count = measurement.item_count.saturating_add(child.item_count);
    }
    Ok(measurement)
}

#[cfg(unix)]
fn metadata_identity(metadata: &fs::Metadata) -> PhysicalPathIdentity {
    use std::os::unix::fs::MetadataExt;

    PhysicalPathIdentity {
        volume: metadata.dev(),
        index: metadata.ino(),
    }
}

#[cfg(unix)]
fn open_identity_handle(path: &Path) -> Result<fs::File, PermanentDeleteError> {
    use std::os::unix::fs::OpenOptionsExt;

    // O_NONBLOCK prevents a concurrently substituted FIFO from waiting for a
    // writer. O_NOFOLLOW closes the final-component symlink race, while the
    // surrounding path and physical identity checks retain the full boundary.
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| permanent_delete_io_error("failed to open the deletion target", error))
}

#[cfg(unix)]
fn physical_path_identity(path: &Path) -> Result<PhysicalPathIdentity, PermanentDeleteError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if current_platform().is_link_like(&metadata) {
        return Err("the staged item became a link or reparse point"
            .to_string()
            .into());
    }
    Ok(metadata_identity(&metadata))
}

#[cfg(windows)]
fn open_identity_handle(path: &Path) -> Result<fs::File, PermanentDeleteError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| permanent_delete_io_error("failed to open the deletion target", error))
}

#[cfg(windows)]
fn handle_identity(file: &fs::File) -> Result<PhysicalPathIdentity, PermanentDeleteError> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
    // SAFETY: `file` owns a valid handle for the duration of the call. A
    // nonzero result guarantees that Windows initialized every output field.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, information.as_mut_ptr()) };
    if succeeded == 0 {
        return Err(
            "failed to read the physical identity of the deletion target"
                .to_string()
                .into(),
        );
    }
    // SAFETY: API success was checked immediately above.
    let information = unsafe { information.assume_init() };
    Ok(PhysicalPathIdentity {
        volume: u64::from(information.dwVolumeSerialNumber),
        index: (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    })
}

#[cfg(windows)]
fn physical_path_identity(path: &Path) -> Result<PhysicalPathIdentity, PermanentDeleteError> {
    let handle = open_identity_handle(path)?;
    let metadata = handle.metadata().map_err(|error| {
        permanent_delete_io_error("failed to read staged target metadata", error)
    })?;
    if current_platform().is_link_like(&metadata) {
        return Err("the staged item became a link or reparse point"
            .to_string()
            .into());
    }
    handle_identity(&handle)
}

/// Validates and permanently deletes one file result without coordinating the
/// batch. Batch logging, operation locking, and cache updates belong to the
/// large-file service that produced the candidate.
pub(crate) fn delete_file_candidate_permanently(
    candidate: &PermanentDeleteCandidate,
) -> Result<(PathBuf, FileSpaceUsage), PermanentDeleteError> {
    let requested_target = PathBuf::from(&candidate.path);
    let prepared = prepare_path_for_permanent_delete(&requested_target)?;
    validate_permanent_delete_candidate(prepared.metadata(), candidate)?;
    let target = current_platform()
        .canonicalize_no_links(&requested_target)
        .map_err(|error| format!("failed to access the requested file: {error}"))?;
    if current_platform()
        .should_skip(
            &target,
            &current_platform().system_volume_path(),
            ScanPurpose::LargeFiles,
        )
        .is_some()
    {
        return Err(
            "MangoDisk cannot process a protected system or application file"
                .to_string()
                .into(),
        );
    }
    let verified_target = current_platform()
        .canonicalize_no_links(&requested_target)
        .map_err(|error| error.to_string())?;
    if verified_target != target {
        return Err("the file changed during safety validation"
            .to_string()
            .into());
    }
    validate_permanent_delete_candidate(prepared.metadata(), candidate)?;
    let usage = current_platform().file_space_usage(&requested_target, prepared.metadata());
    delete_path_permanently(prepared, candidate.expected_bytes, 1)?;
    Ok((target, usage))
}

fn validate_permanent_delete_candidate(
    metadata: &fs::Metadata,
    candidate: &PermanentDeleteCandidate,
) -> Result<(), String> {
    if !metadata.is_file() || current_platform().is_link_like(metadata) {
        return Err("only regular files can be permanently deleted".to_string());
    }
    if candidate.expected_modified_at_ms.is_none()
        || metadata.len() != candidate.expected_bytes
        || modified_ms(metadata) != candidate.expected_modified_at_ms
    {
        return Err("the file changed after scanning".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod permanent_delete_tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    static NEXT_DELETE_SANDBOX_ID: AtomicU64 = AtomicU64::new(1);

    struct DeleteSandbox(PathBuf);

    impl DeleteSandbox {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let id = NEXT_DELETE_SANDBOX_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mangodisk-permanent-delete-{}-{unique}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create the permanent-delete fixture");
            Self(path)
        }
    }

    impl Drop for DeleteSandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn create_sparse_file(path: &Path, logical_bytes: u64) -> fs::File {
        let file = fs::File::create(path).expect("create the sparse deletion fixture");
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
            assert_ne!(marked_sparse, 0, "mark the deletion fixture as sparse");
        }
        file.set_len(logical_bytes)
            .expect("extend the sparse deletion fixture");
        file
    }

    #[cfg(unix)]
    struct UnixSocketFixture {
        path: PathBuf,
        _listener: std::os::unix::net::UnixListener,
    }

    #[cfg(unix)]
    impl UnixSocketFixture {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            // Unix-domain socket paths are short and bounded. `/tmp` is a
            // platform-approved system alias on macOS and a normal root on
            // other Unix systems.
            let path = PathBuf::from("/tmp").join(format!(
                "mangodisk-permanent-delete-socket-{}-{unique}.sock",
                std::process::id()
            ));
            let listener = std::os::unix::net::UnixListener::bind(&path)
                .expect("the Unix socket fixture should be bound");
            Self {
                path,
                _listener: listener,
            }
        }
    }

    #[cfg(unix)]
    impl Drop for UnixSocketFixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[cfg(unix)]
    fn create_fifo(path: &Path) {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};

        let native_path = CString::new(path.as_os_str().as_bytes())
            .expect("the FIFO fixture path should not contain a null byte");
        // SAFETY: `native_path` is a valid, null-terminated pathname and each
        // caller confines the fixture to an isolated test sandbox.
        let result = unsafe { libc::mkfifo(native_path.as_ptr(), 0o600) };
        assert_eq!(
            result,
            0,
            "the FIFO fixture should be created: {}",
            std::io::Error::last_os_error()
        );
    }

    #[cfg(unix)]
    fn assert_special_file_is_rejected_promptly(path: &Path) {
        use std::{sync::mpsc, time::Duration};

        let target = path.to_path_buf();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = prepare_path_for_permanent_delete(&target)
                .map(|_| ())
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });

        let result = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("special-file validation must not block");
        worker
            .join()
            .expect("the special-file validation worker should finish");
        let error = result.expect_err("special files must not enter permanent deletion");
        assert!(
            error.contains("only regular files and directories"),
            "the rejection should identify the unsupported file type"
        );
    }

    #[test]
    fn permanent_delete_removes_a_matching_regular_file() {
        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("large.bin");
        fs::write(&path, b"large-file-fixture").expect("write the deletion fixture");
        let metadata = fs::metadata(&path).expect("read the deletion fixture");
        let candidate = PermanentDeleteCandidate {
            path: path.to_string_lossy().into_owned(),
            expected_bytes: metadata.len(),
            expected_modified_at_ms: modified_ms(&metadata),
        };

        let expected_usage = current_platform().file_space_usage(&path, &metadata);
        let (_, usage) = delete_file_candidate_permanently(&candidate)
            .expect("a matching regular file should be permanently deleted");

        assert_eq!(usage, expected_usage);
        assert!(!path.exists());
    }

    #[test]
    fn permanent_delete_reports_allocated_space_for_a_sparse_file() {
        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("sparse.bin");
        let file = create_sparse_file(&path, 64 * 1024 * 1024);
        let metadata = file.metadata().expect("read the sparse deletion metadata");
        let expected_usage = current_platform().file_space_usage(&path, &metadata);
        assert!(expected_usage.allocated_bytes < expected_usage.logical_bytes);
        drop(file);
        let candidate = PermanentDeleteCandidate {
            path: path.to_string_lossy().into_owned(),
            expected_bytes: metadata.len(),
            expected_modified_at_ms: modified_ms(&metadata),
        };

        let (_, usage) = delete_file_candidate_permanently(&candidate)
            .expect("the sparse fixture should be permanently deleted");

        assert_eq!(usage, expected_usage);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn permanent_delete_rejects_a_fifo_without_blocking_or_mutation() {
        use std::os::unix::fs::FileTypeExt;

        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("stale.pipe");
        create_fifo(&path);

        assert_special_file_is_rejected_promptly(&path);

        let metadata = fs::symlink_metadata(&path).expect("the FIFO fixture should remain");
        assert!(metadata.file_type().is_fifo());
    }

    #[cfg(unix)]
    #[test]
    fn unix_identity_handle_opens_a_fifo_without_blocking() {
        use std::{os::unix::fs::FileTypeExt, sync::mpsc, time::Duration};

        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("raced-in.pipe");
        create_fifo(&path);

        let target = path.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = open_identity_handle(&target)
                .map_err(|error| error.to_string())
                .and_then(|handle| handle.metadata().map_err(|error| error.to_string()))
                .map(|metadata| metadata.file_type().is_fifo());
            let _ = sender.send(result);
        });

        let result = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("the Unix identity handle must open a FIFO without blocking");
        worker
            .join()
            .expect("the Unix identity-handle worker should finish");
        assert_eq!(
            result,
            Ok(true),
            "the identity handle should expose the raced-in FIFO for rejection"
        );
    }

    #[cfg(unix)]
    #[test]
    fn permanent_delete_rejects_a_unix_socket_without_blocking_or_mutation() {
        use std::os::unix::fs::FileTypeExt;

        let socket = UnixSocketFixture::new();
        assert_special_file_is_rejected_promptly(&socket.path);

        let metadata =
            fs::symlink_metadata(&socket.path).expect("the Unix socket fixture should remain");
        assert!(metadata.file_type().is_socket());
    }

    #[test]
    fn permanent_delete_preserves_a_file_that_changed_after_scanning() {
        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("changed.bin");
        fs::write(&path, b"original").expect("write the original deletion fixture");
        let metadata = fs::metadata(&path).expect("read the original deletion fixture");
        let candidate = PermanentDeleteCandidate {
            path: path.to_string_lossy().into_owned(),
            expected_bytes: metadata.len(),
            expected_modified_at_ms: modified_ms(&metadata),
        };
        fs::write(&path, b"changed-content").expect("change the deletion fixture");

        let error = delete_file_candidate_permanently(&candidate)
            .expect_err("a changed file must not be permanently deleted");

        assert!(error.to_string().contains("changed after scanning"));
        assert!(path.exists());
    }

    #[test]
    fn permanent_delete_path_removes_a_preflighted_directory() {
        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("directory");
        fs::create_dir_all(&path).expect("create the directory deletion fixture");
        fs::write(path.join("payload.bin"), b"payload")
            .expect("write the directory deletion fixture");

        let prepared = prepare_path_for_permanent_delete(&path)
            .expect("the directory target should be prepared");
        delete_path_permanently(prepared, b"payload".len() as u64, 1)
            .expect("a preflighted directory should be permanently deleted");

        assert!(!path.exists());
        assert!(
            fs::read_dir(&sandbox.0)
                .expect("read the deletion sandbox")
                .all(|entry| !entry
                    .expect("read the staging entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".mangodisk-delete-")),
            "successful deletion must not leave a staging directory"
        );
    }

    #[test]
    fn cancellable_directory_delete_reports_live_removed_content() {
        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("cancellable-directory");
        fs::create_dir_all(path.join("nested")).expect("create the cancellable fixture");
        fs::write(path.join("first.bin"), b"first").expect("write the first fixture");
        fs::write(path.join("nested/second.bin"), b"second").expect("write the second fixture");
        let prepared = prepare_path_for_permanent_delete(&path)
            .expect("the cancellable directory should be prepared");

        let outcome =
            delete_directory_tree_permanently_with_cancellation(prepared, 1, 1, &|| false)
                .expect("the cancellable directory should be removed");

        assert_eq!(outcome.released_bytes(), 11);
        assert_eq!(outcome.affected_item_count(), 2);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn cancellable_directory_delete_unlinks_a_descendant_symlink_without_following_it() {
        use std::os::unix::fs::symlink;

        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("cancellable-directory-with-link");
        let external = sandbox.0.join("external");
        let protected_file = external.join("must-remain.bin");
        fs::create_dir_all(&path).expect("create the cancellable fixture");
        fs::create_dir_all(&external).expect("create the external fixture");
        fs::write(&protected_file, b"protected").expect("write the protected fixture");
        symlink(&external, path.join("external-link")).expect("create the descendant symlink");
        let prepared = prepare_path_for_permanent_delete(&path)
            .expect("the directory containing the symlink should be prepared");

        let outcome =
            delete_directory_tree_permanently_with_cancellation(prepared, 0, 0, &|| false)
                .expect("the approved tree should be removed without following its symlink");

        assert_eq!(outcome.released_bytes(), 0);
        assert_eq!(outcome.affected_item_count(), 0);
        assert!(
            !path.exists(),
            "the approved directory tree must be removed"
        );
        assert!(
            protected_file.exists(),
            "the external symlink target must remain untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_after_descendant_symlink_unlink_reports_partial_restored_deletion() {
        use std::os::unix::fs::symlink;

        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("cancelled-directory-with-link");
        let external_link = path.join("external-link");
        let external = sandbox.0.join("external");
        let protected_file = external.join("must-remain.bin");
        fs::create_dir_all(&path).expect("create the cancellation fixture");
        fs::create_dir_all(&external).expect("create the external fixture");
        fs::write(&protected_file, b"protected").expect("write the protected fixture");
        symlink(&external, &external_link).expect("create the descendant symlink");
        let prepared = prepare_path_for_permanent_delete(&path)
            .expect("the directory containing the symlink should be prepared");
        let checks = AtomicU64::new(0);

        let error = delete_directory_tree_permanently_with_cancellation(prepared, 0, 0, &|| {
            checks.fetch_add(1, Ordering::Relaxed) >= 3
        })
        .expect_err("cancellation should stop deletion after the symlink is unlinked");

        assert!(error.is_partial());
        assert!(error.remaining_was_restored());
        assert_eq!(error.released_bytes(), 0);
        assert_eq!(error.affected_item_count(), 0);
        assert_eq!(checks.load(Ordering::Relaxed), 4);
        assert!(path.exists(), "the remaining directory must be restored");
        assert!(
            fs::symlink_metadata(&external_link).is_err(),
            "the unlinked descendant symlink must not be restored"
        );
        assert!(
            protected_file.exists(),
            "the external symlink target must remain untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_contents_delete_unlinks_a_descendant_symlink_without_following_it() {
        use std::os::unix::fs::symlink;

        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("directory-contents-with-link");
        let retained_empty = path.join("retained-empty");
        let external_link = path.join("external-link");
        let external = sandbox.0.join("external");
        let protected_file = external.join("must-remain.bin");
        fs::create_dir_all(&retained_empty).expect("create the retained empty directory");
        fs::create_dir_all(&external).expect("create the external fixture");
        fs::write(&protected_file, b"protected").expect("write the protected fixture");
        symlink(&external, &external_link).expect("create the descendant symlink");
        let prepared = prepare_path_for_permanent_delete(&path)
            .expect("the directory containing the symlink should be prepared");

        let outcome = delete_directory_contents_permanently_with_cancellation(prepared, &|| false)
            .expect("the approved contents should be removed without following their symlink");

        assert_eq!(outcome.released_bytes(), 0);
        assert_eq!(outcome.affected_item_count(), 0);
        assert!(
            path.exists(),
            "the retained directory skeleton must be restored"
        );
        assert!(
            retained_empty.exists(),
            "the pre-existing empty directory must remain"
        );
        assert!(
            fs::symlink_metadata(&external_link).is_err(),
            "the descendant symlink entry must be removed"
        );
        assert!(
            protected_file.exists(),
            "the external symlink target must remain untouched"
        );
    }

    #[test]
    fn cancellable_directory_delete_restores_remaining_content() {
        use std::sync::atomic::{AtomicU64, Ordering};

        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("cancelled-directory");
        fs::create_dir_all(&path).expect("create the cancellation fixture");
        for index in 0..32 {
            fs::write(path.join(format!("{index:02}.cache")), b"cache")
                .expect("write the cancellation fixture");
        }
        let prepared = prepare_path_for_permanent_delete(&path)
            .expect("the cancelled directory should be prepared");
        let checks = AtomicU64::new(0);

        let error =
            delete_directory_tree_permanently_with_cancellation(prepared, 32 * 5, 32, &|| {
                checks.fetch_add(1, Ordering::Relaxed) >= 5
            })
            .expect_err("cancellation should stop the staged directory traversal");

        assert!(error.is_partial());
        assert!(error.released_bytes() > 0);
        assert!(error.released_bytes() < 32 * 5);
        assert!(error.affected_item_count() > 0);
        assert!(error.affected_item_count() < 32);
        assert!(path.exists(), "remaining content must be restored");
        assert_eq!(
            fs::read_dir(&path)
                .expect("read the restored directory")
                .count() as u64
                + error.affected_item_count(),
            32
        );
    }

    /// Measures return latency after cancellation during a large small-file delete.
    ///
    /// The diagnostic creates a large temporary dataset and is ignored by
    /// default. Its output contains only counts and timings, never fixture paths.
    #[test]
    #[ignore = "filesystem cancellation latency benchmark"]
    fn benchmark_cancellable_directory_delete_latency() {
        use std::{
            sync::{
                atomic::{AtomicBool, Ordering},
                Arc,
            },
            thread,
            time::{Duration, Instant},
        };

        let file_count = std::env::var("MANGODISK_CLEANUP_BENCHMARK_FILE_COUNT")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|count| *count > 0)
            .unwrap_or(20_000);
        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("cancellation-latency");
        fs::create_dir_all(&path).expect("create the cancellation benchmark root");
        for index in 0..file_count {
            fs::write(path.join(format!("{index:08}.cache")), b"cache")
                .expect("write the cancellation benchmark file");
        }
        let prepared = prepare_path_for_permanent_delete(&path)
            .expect("prepare the cancellation benchmark directory");
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation_signal = Arc::clone(&cancelled);
        let signal_delay = Duration::from_millis(50);
        let signal_thread = thread::spawn(move || {
            thread::sleep(signal_delay);
            cancellation_signal.store(true, Ordering::Release);
        });

        let started = Instant::now();
        let error = delete_directory_tree_permanently_with_cancellation(
            prepared,
            file_count * 5,
            file_count,
            &|| cancelled.load(Ordering::Acquire),
        )
        .expect_err("the asynchronous cancellation signal should stop deletion");
        let total_elapsed = started.elapsed();
        signal_thread
            .join()
            .expect("join the cancellation signal thread");
        let response_elapsed = total_elapsed.saturating_sub(signal_delay);

        assert!(
            path.exists(),
            "remaining files must be restored after cancellation"
        );
        assert!(error.affected_item_count() < file_count);
        assert!(
            response_elapsed < Duration::from_millis(500),
            "cancellation should be observed within 500 ms"
        );
        println!(
            "cleanup_whole_root_cancellation file_count={file_count} total_ms={:.2} response_ms={:.2} affected_item_count={}",
            total_elapsed.as_secs_f64() * 1_000.0,
            response_elapsed.as_secs_f64() * 1_000.0,
            error.affected_item_count()
        );
    }

    #[test]
    fn confirmed_analysis_directory_deletes_when_scan_metadata_is_stale() {
        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("confirmed-analysis-directory");
        fs::create_dir_all(&path).expect("create the confirmed analysis fixture");
        fs::write(path.join("payload.bin"), b"payload changed after analysis")
            .expect("write the changed analysis fixture");
        fs::write(path.join("new-after-analysis.bin"), b"new")
            .expect("write the new analysis fixture");
        let candidate = AnalysisEntryCandidate {
            root: sandbox.0.to_string_lossy().into_owned(),
            path: path.to_string_lossy().into_owned(),
            expected_logical_bytes: b"payload".len() as u64,
            expected_allocated_bytes: 4_096,
            expected_file_count: 1,
            is_directory: true,
        };

        let outcome = delete_analysis_candidate_permanently(candidate)
            .expect("explicit confirmation should authorize the current directory contents");

        // The result updates the displayed scan snapshot, so it keeps the
        // original aggregate rather than claiming an unmeasured live value.
        assert_eq!(outcome.result.released_bytes, 4_096);
        assert_eq!(outcome.removed_usage.logical_bytes, b"payload".len() as u64);
        assert_eq!(outcome.removed_usage.allocated_bytes, 4_096);
        assert_eq!(outcome.result.removed_file_count, 1);
        assert!(!path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_delete_errors_preserve_actionable_failure_reasons() {
        assert_eq!(
            permanent_delete_io_reason(&std::io::Error::from_raw_os_error(32)),
            Some(CoreErrorReason::ResourceBusy)
        );
        assert_eq!(
            permanent_delete_io_reason(&std::io::Error::from_raw_os_error(5)),
            Some(CoreErrorReason::AccessDeniedOrBusy)
        );
    }

    #[test]
    fn permanent_delete_preserves_a_same_path_replacement() {
        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("directory");
        let moved_original = sandbox.0.join("moved-original");
        fs::create_dir_all(&path).expect("create the original deletion target");
        fs::write(path.join("original.bin"), b"original")
            .expect("write the original deletion target");

        let prepared = prepare_path_for_permanent_delete(&path)
            .expect("the original target should be prepared");
        fs::rename(&path, &moved_original).expect("move the original target aside");
        fs::create_dir_all(&path).expect("create the same-path replacement");
        fs::write(path.join("replacement.bin"), b"replacement")
            .expect("write the same-path replacement");

        let error = delete_path_permanently(prepared, b"original".len() as u64, 1)
            .expect_err("a same-path replacement must not be deleted");

        assert!(error.to_string().contains("replaced"));
        assert!(
            path.join("replacement.bin").exists(),
            "the same-path replacement must be restored"
        );
        assert!(
            moved_original.join("original.bin").exists(),
            "the original object must remain untouched after being moved"
        );
    }

    #[test]
    fn empty_directory_delete_preserves_a_concurrent_new_file() {
        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("directory");
        fs::create_dir_all(&path).expect("create the empty directory fixture");
        let prepared = prepare_path_for_permanent_delete(&path)
            .expect("the empty directory should be prepared");
        fs::write(path.join("concurrent.bin"), b"concurrent")
            .expect("write a file after the directory was prepared");

        let error = delete_empty_directory_permanently(prepared)
            .expect_err("a directory that became non-empty must not be deleted");

        assert!(error.to_string().contains("restored"));
        assert!(
            path.join("concurrent.bin").exists(),
            "a concurrently created file must remain visible"
        );
    }

    #[test]
    fn rollback_reports_deleted_zero_byte_items_as_partial() {
        let sandbox = DeleteSandbox::new();
        let staging_root = sandbox.0.join("staging");
        let staged_target = staging_root.join("target");
        let original_path = sandbox.0.join("restored");
        fs::create_dir_all(&staged_target).expect("create the rollback fixture");

        let error = rollback_staged_target(
            &original_path,
            &staging_root,
            &staged_target,
            "simulated zero-byte partial deletion",
            None,
            PermanentDeleteOutcome {
                released_bytes: 0,
                affected_item_count: 1,
                had_irreversible_mutation: false,
            },
        )
        .expect_err("a partial rollback should retain its structured outcome");

        assert!(error.is_partial());
        assert_eq!(error.released_bytes(), 0);
        assert_eq!(error.affected_item_count(), 1);
        assert!(original_path.exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn permanent_delete_restores_remaining_directory_after_recursive_failure() {
        use std::os::unix::fs::PermissionsExt;

        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("directory");
        let blocked = path.join("blocked");
        let payload = blocked.join("payload.bin");
        fs::create_dir_all(&blocked).expect("create the rollback fixture");
        fs::write(&payload, b"payload").expect("write the rollback fixture");
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000))
            .expect("block traversal of the rollback fixture");

        let prepared = prepare_path_for_permanent_delete(&path)
            .expect("the rollback target should be prepared");
        let error = delete_path_permanently(prepared, b"payload".len() as u64, 1)
            .expect_err("an unreadable descendant should fail recursive deletion");

        assert!(
            error.to_string().contains("restored"),
            "the failure must explain that remaining contents were restored"
        );
        assert!(
            path.exists(),
            "the remaining directory must return to its path"
        );
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700))
            .expect("restore fixture permissions");
        assert!(
            payload.exists(),
            "the blocked payload must remain available"
        );
    }
}

#[cfg(all(test, windows))]
mod windows_permanent_delete_tests {
    use std::{
        fs::OpenOptions,
        os::windows::fs::OpenOptionsExt,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct JunctionSandbox {
        root: PathBuf,
        junction: PathBuf,
    }

    impl Drop for JunctionSandbox {
        fn drop(&mut self) {
            let junction_removed =
                !self.junction.exists() || fs::remove_dir(&self.junction).is_ok();
            if junction_removed {
                let _ = fs::remove_dir_all(&self.root);
            }
        }
    }

    #[test]
    fn permanent_delete_rejects_file_reached_through_junction() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sandbox = std::env::temp_dir().join(format!(
            "MangoDisk-Junction-Permanent-Delete-{}-{unique}",
            std::process::id()
        ));
        let real_directory = sandbox.join("real");
        let junction = sandbox.join("junction");
        let _sandbox_cleanup = JunctionSandbox {
            root: sandbox.clone(),
            junction: junction.clone(),
        };
        let protected_file = real_directory.join("must-remain.bin");
        fs::create_dir_all(&real_directory).expect("the junction fixture directory should exist");
        fs::write(&protected_file, b"MangoDisk junction safety fixture")
            .expect("the junction fixture should be written");
        let output = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&real_directory)
            .output()
            .expect("mklink should create the test junction");
        assert!(
            output.status.success(),
            "failed to create test junction: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let linked_file = junction.join("must-remain.bin");
        let metadata =
            fs::metadata(&protected_file).expect("the fixture metadata should be readable");
        let candidate = PermanentDeleteCandidate {
            path: linked_file.to_string_lossy().into_owned(),
            expected_bytes: metadata.len(),
            expected_modified_at_ms: modified_ms(&metadata),
        };
        let error = delete_file_candidate_permanently(&candidate)
            .expect_err("a file reached through a junction must be rejected");

        assert!(error.to_string().contains("link or reparse point"));
        assert!(
            protected_file.exists(),
            "the junction target must remain untouched"
        );
    }

    #[test]
    fn permanent_delete_does_not_hide_a_directory_with_a_locked_child() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sandbox = std::env::temp_dir().join(format!(
            "MangoDisk-Locked-Permanent-Delete-{}-{unique}",
            std::process::id()
        ));
        let directory = sandbox.join("directory");
        let payload = directory.join("payload.bin");
        fs::create_dir_all(&directory).expect("create the locked deletion fixture");
        fs::write(&payload, b"locked payload").expect("write the locked deletion fixture");
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&payload)
            .expect("lock the deletion fixture without delete sharing");

        let prepared = prepare_path_for_permanent_delete(&directory)
            .expect("the locked target should be prepared");
        let error = delete_path_permanently(prepared, b"locked payload".len() as u64, 1)
            .expect_err("a locked descendant must stop permanent deletion");

        assert!(
            directory.exists(),
            "a failed delete must leave or restore the visible directory"
        );
        assert!(payload.exists(), "the locked file must remain available");
        drop(locked);
        fs::remove_dir_all(sandbox).expect("remove the locked deletion fixture");
        assert!(
            !error.is_partial() || error.released_bytes() == 0,
            "the untouched locked fixture must not report released bytes"
        );
    }

    #[test]
    fn cancellable_directory_delete_restores_a_locked_child() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sandbox = std::env::temp_dir().join(format!(
            "MangoDisk-Locked-Cancellable-Delete-{}-{unique}",
            std::process::id()
        ));
        let directory = sandbox.join("directory");
        let payload = directory.join("payload.bin");
        fs::create_dir_all(&directory).expect("create the locked cancellable fixture");
        fs::write(&payload, b"locked payload").expect("write the locked cancellable fixture");
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&payload)
            .expect("lock the cancellable fixture without delete sharing");
        let prepared = prepare_path_for_permanent_delete(&directory)
            .expect("prepare the locked cancellable directory");

        let error = delete_directory_tree_permanently_with_cancellation(
            prepared,
            b"locked payload".len() as u64,
            1,
            &|| false,
        )
        .expect_err("a locked descendant must stop cancellable directory deletion");

        assert!(directory.exists(), "the directory must be restored");
        assert!(payload.exists(), "the locked payload must remain available");
        assert_eq!(error.released_bytes(), 0);
        assert_eq!(error.affected_item_count(), 0);
        drop(locked);
        fs::remove_dir_all(sandbox).expect("remove the locked cancellable fixture");
    }

    #[test]
    fn cancellable_directory_delete_rejects_a_new_junction() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sandbox = std::env::temp_dir().join(format!(
            "MangoDisk-Cancellable-Junction-{}-{unique}",
            std::process::id()
        ));
        let directory = sandbox.join("directory");
        let protected = sandbox.join("protected");
        let junction = directory.join("late-junction");
        let protected_file = protected.join("must-remain.bin");
        let _sandbox_cleanup = JunctionSandbox {
            root: sandbox.clone(),
            junction: junction.clone(),
        };
        fs::create_dir_all(&directory).expect("create the cancellable junction root");
        fs::create_dir_all(&protected).expect("create the protected junction target");
        fs::write(&protected_file, b"protected").expect("write the protected junction fixture");
        let prepared = prepare_path_for_permanent_delete(&directory)
            .expect("prepare the directory before the junction appears");
        let output = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&protected)
            .output()
            .expect("mklink should create the late junction");
        assert!(output.status.success(), "create the late junction fixture");

        let error = delete_directory_tree_permanently_with_cancellation(prepared, 0, 0, &|| false)
            .expect_err("a new junction must stop cancellable directory deletion");

        assert!(
            error.to_string().contains("link or reparse point"),
            "the failure must retain the safety reason"
        );
        assert!(directory.exists(), "the cleanup root must be restored");
        assert!(
            protected_file.exists(),
            "the junction target must never be traversed or deleted"
        );
    }
}
