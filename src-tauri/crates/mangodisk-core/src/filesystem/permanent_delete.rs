use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    time::Instant,
};

use mangodisk_platform::{current_platform, FileSpaceUsage, Platform, ScanPurpose};

use crate::{
    filesystem::{
        metadata::{diagnostic_path, modified_ms},
        PermanentDeleteCandidate,
    },
    shared::CoreErrorReason,
};

#[path = "directory_removal.rs"]
mod directory_removal;
use directory_removal::{
    remove_directory_contents_cancellable, remove_directory_tree_cancellable,
    remove_directory_tree_entry,
};

#[cfg(test)]
#[path = "permanent_delete_validation_tests.rs"]
mod validation_tests;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PhysicalPathIdentity {
    volume: u64,
    index: u64,
}

/// Holds the live metadata used by domain policy and deletion accounting.
/// Ordinary deletion does not require reading file contents or creating a sibling directory.
pub(crate) struct PreparedPermanentDelete {
    path: PathBuf,
    metadata: fs::Metadata,
    identity: PhysicalPathIdentity,
}

/// Counts regular files that the permanent-delete boundary actually removed.
///
/// Complete-root cleanup cannot treat an earlier scan summary as the actual
/// result: tools may still write before deletion. Accumulating the live
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

    /// Returns the identity used by domain-specific ownership checks.
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
    counts_unknown: bool,
    native_code: Option<i32>,
    stage: &'static str,
    failure_reason: super::models::PermanentDeleteFailureReason,
}

/// Inspects a regular target without requesting content-read or rename access.
/// Domain callers retain ownership, protected-path and application-state policy.
pub(crate) fn prepare_path_for_permanent_delete(
    path: &Path,
) -> Result<PreparedPermanentDelete, PermanentDeleteError> {
    reject_system_managed_file(path)?;
    current_platform()
        .validate_path_no_links(path)
        .map_err(|error| PermanentDeleteError::from(error).at_stage("prepare_target"))?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        permanent_delete_io_error("failed to inspect deletion target", error)
            .at_stage("prepare_target")
    })?;
    validate_permanent_delete_target_metadata(&metadata)?;
    #[cfg(unix)]
    let identity = metadata_identity(&metadata);
    #[cfg(windows)]
    let identity = physical_path_identity(path)?;
    Ok(PreparedPermanentDelete {
        path: path.to_path_buf(),
        metadata,
        identity,
    })
}

pub(crate) fn reject_system_managed_file(path: &Path) -> Result<(), PermanentDeleteError> {
    if current_platform().is_system_managed_file(path) {
        return Err(PermanentDeleteError::before_mutation_with_reason(
            "Windows manages this file; change the corresponding system setting instead of deleting it",
            CoreErrorReason::SystemManagedFile,
        ).at_stage("protect_system_file"));
    }
    Ok(())
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
    pub(crate) fn before_mutation(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            reason: None,
            released_bytes: 0,
            affected_item_count: 0,
            partial: false,
            counts_unknown: false,
            native_code: None,
            stage: "validate",
            failure_reason: Default::default(),
        }
    }

    pub(crate) fn before_mutation_with_reason(
        message: impl Into<String>,
        reason: CoreErrorReason,
    ) -> Self {
        Self {
            message: message.into(),
            reason: Some(reason),
            released_bytes: 0,
            affected_item_count: 0,
            partial: false,
            counts_unknown: false,
            native_code: None,
            stage: "validate",
            failure_reason: Default::default(),
        }
    }

    pub(crate) fn with_failure_reason(
        mut self,
        reason: super::models::PermanentDeleteFailureReason,
    ) -> Self {
        self.failure_reason = reason;
        self
    }

    pub(crate) fn into_failure(self, path: String) -> super::models::PermanentDeleteFailure {
        use super::models::{
            PermanentDeleteFailure, PermanentDeleteFailureDetails,
            PermanentDeleteFailureReason as Reason, PermanentDeleteFailureStage as Stage,
        };
        let reason = match self.reason {
            Some(CoreErrorReason::SystemManagedFile) => Reason::Protected,
            Some(CoreErrorReason::ResourceBusy) => Reason::ResourceBusy,
            Some(CoreErrorReason::AccessDeniedOrBusy) => Reason::AccessDenied,
            Some(CoreErrorReason::ItemChanged) => Reason::ItemChanged,
            _ => self.failure_reason,
        };
        let stage = match self.stage {
            "prepare_target" => Stage::PrepareTarget,
            "create_staging" => Stage::CreateStaging,
            "stage_rename" => Stage::MoveToStaging,
            "remove" => Stage::Remove,
            _ => Stage::Validate,
        };
        PermanentDeleteFailure {
            path,
            details: PermanentDeleteFailureDetails {
                message: self.message,
                reason,
                stage,
                native_code: self.native_code,
                partial: self.partial,
                remaining_restored: false,
            },
        }
    }

    pub(crate) fn is_not_found(&self) -> bool {
        !self.partial
            && self.failure_reason == super::models::PermanentDeleteFailureReason::NotFound
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

    pub(crate) fn observed_or_estimated_files(&self) -> Option<u64> {
        (!self.counts_unknown).then_some(self.affected_item_count)
    }

    pub(crate) fn observed_or_estimated_bytes(&self) -> Option<u64> {
        (!self.counts_unknown).then_some(self.released_bytes)
    }

    pub(crate) fn reason(&self) -> Option<CoreErrorReason> {
        self.reason
    }

    pub(crate) fn native_code(&self) -> Option<i32> {
        self.native_code
    }
    pub(crate) fn stage(&self) -> &'static str {
        self.stage
    }
    fn at_stage(mut self, stage: &'static str) -> Self {
        self.stage = stage;
        self
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

impl From<mangodisk_platform::PlatformError> for PermanentDeleteError {
    fn from(error: mangodisk_platform::PlatformError) -> Self {
        use mangodisk_platform::PlatformErrorCode;
        let mut failure = if let Some(kind) = error.io_kind() {
            let native = error.native_code().map_or_else(
                || std::io::Error::from(kind),
                std::io::Error::from_raw_os_error,
            );
            permanent_delete_io_error("platform path validation failed", native)
        } else {
            match error.code() {
                PlatformErrorCode::AccessDenied => Self::before_mutation_with_reason(
                    error.to_string(),
                    CoreErrorReason::AccessDeniedOrBusy,
                ),
                PlatformErrorCode::ItemChanged => Self::before_mutation_with_reason(
                    error.to_string(),
                    CoreErrorReason::ItemChanged,
                ),
                _ => Self::before_mutation(error.to_string()),
            }
        };
        // Preserve the operation context as well as typed native evidence.
        failure.message = error.to_string();
        failure
    }
}

pub(crate) fn permanent_delete_io_error(
    context: &str,
    error: std::io::Error,
) -> PermanentDeleteError {
    let message = format!("{context}: {error}");
    let mut failure = match permanent_delete_io_reason(&error) {
        Some(reason) => PermanentDeleteError::before_mutation_with_reason(message, reason),
        None => PermanentDeleteError::before_mutation(message),
    };
    failure.native_code = error.raw_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        failure.failure_reason = super::models::PermanentDeleteFailureReason::NotFound;
    }
    failure
}

fn permanent_delete_io_reason(error: &std::io::Error) -> Option<CoreErrorReason> {
    if error.kind() == std::io::ErrorKind::DirectoryNotEmpty {
        return Some(CoreErrorReason::DirectoryNotEmpty);
    }
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

/// Removes a prepared directory using its confirmed scan totals for accounting.
/// Small trees collect live progress; large native removals use scan estimates.
/// Native failures report unknown progress instead of scanning the remainder.
pub(crate) fn delete_prepared_directory_tree_from_snapshot(
    target: PreparedPermanentDelete,
    expected_bytes: u64,
    expected_item_count: u64,
) -> Result<(), PermanentDeleteError> {
    delete_directly(
        target,
        expected_bytes,
        expected_item_count,
        RemovalMode::SnapshotDirectoryTree,
    )
    .map(|_| ())
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
        delete_directly(
            target,
            expected_bytes,
            expected_item_count,
            RemovalMode::File,
        )
        .map(|_| ())
    } else if target.metadata.is_dir() {
        delete_directly(
            target,
            expected_bytes,
            expected_item_count,
            RemovalMode::DirectoryTree,
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

/// Removes a directory in one cancellable pass at its original path.
///
/// Per-file cleanup checks cancellation between candidates; the whole-root fast
/// path must preserve cancellation, link safety, and live progress accounting.
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
    delete_directly(
        target,
        expected_bytes,
        expected_item_count,
        RemovalMode::CancellableDirectoryTree(is_cancelled),
    )
}

/// Applies name exclusions during deletion; surviving entries remain in place.
pub(crate) fn delete_directory_tree_with_name_exclusions(
    target: PreparedPermanentDelete,
    expected_bytes: u64,
    expected_item_count: u64,
    names: &mangodisk_platform::NameExclusions,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<PermanentDeleteOutcome, PermanentDeleteError> {
    if names.matches_path(&target.path) {
        return Err(PermanentDeleteError::before_mutation(
            "the directory is excluded by name",
        ));
    }
    delete_directly(
        target,
        expected_bytes,
        expected_item_count,
        RemovalMode::NameProtectedDirectoryTree {
            names,
            is_cancelled,
        },
    )
}

/// Deletes matching contents in place, retaining directories that were already empty.
/// Cancellation and failures preserve the surviving tree at its original location.
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
    delete_directly(
        target,
        0,
        0,
        RemovalMode::CancellableDirectoryContents {
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
    delete_directly(target, 0, 0, RemovalMode::EmptyDirectory).map(|_| ())
}

#[derive(Clone, Copy)]
enum RemovalMode<'a> {
    NameProtectedDirectoryTree {
        names: &'a mangodisk_platform::NameExclusions,
        is_cancelled: &'a (dyn Fn() -> bool + Sync),
    },
    File,
    EmptyDirectory,
    DirectoryTree,
    SnapshotDirectoryTree,
    CancellableDirectoryTree(&'a (dyn Fn() -> bool + Sync)),
    CancellableDirectoryContents {
        is_cancelled: &'a (dyn Fn() -> bool + Sync),
        allow_parallel: bool,
    },
}

struct RemovalFailure {
    error: std::io::Error,
    verified_outcome: Option<PermanentDeleteOutcome>,
}

/// Deletes at the selected path. Failures leave surviving entries in place;
/// no sibling creation, rename, rollback or failure-accounting traversal is required.
fn delete_directly(
    target: PreparedPermanentDelete,
    expected_bytes: u64,
    expected_item_count: u64,
    removal: RemovalMode<'_>,
) -> Result<PermanentDeleteOutcome, PermanentDeleteError> {
    let started = Instant::now();
    let path = target.path.as_path();
    // One final boundary check protects against ordinary replacements and links.
    // Concurrent mutation after this check is accepted; deletion is not a transaction.
    current_platform()
        .validate_path_no_links(path)
        .map_err(PermanentDeleteError::from)?;
    let identity = physical_path_identity(path)?;
    if identity != target.identity {
        return Err(PermanentDeleteError::before_mutation_with_reason(
            "the item was replaced before permanent deletion",
            CoreErrorReason::ItemChanged,
        ));
    }
    let expected = PermanentDeleteOutcome {
        released_bytes: expected_bytes,
        affected_item_count: expected_item_count,
        had_irreversible_mutation: true,
    };
    let unobserved = |error| RemovalFailure {
        error,
        verified_outcome: None,
    };
    let result = match removal {
        RemovalMode::File => {
            fs::remove_file(path)
                .map(|()| expected)
                .map_err(|error| RemovalFailure {
                    error,
                    verified_outcome: Some(PermanentDeleteOutcome::default()),
                })
        }
        RemovalMode::EmptyDirectory => {
            deletion_entry_io(path, "remove_directory", fs::remove_dir(path))
                .map(|()| PermanentDeleteOutcome {
                    had_irreversible_mutation: true,
                    ..Default::default()
                })
                .map_err(|error| RemovalFailure {
                    error,
                    verified_outcome: Some(PermanentDeleteOutcome::default()),
                })
        }
        RemovalMode::DirectoryTree => fs::remove_dir_all(path)
            .map(|()| expected)
            .map_err(unobserved),
        RemovalMode::SnapshotDirectoryTree if use_native_snapshot_removal(expected_item_count) => {
            fs::remove_dir_all(path)
                .map(|()| expected)
                .map_err(unobserved)
        }
        RemovalMode::SnapshotDirectoryTree => remove_directory_tree_cancellable(path, &|| false),
        RemovalMode::CancellableDirectoryTree(cancelled) => {
            remove_directory_tree_cancellable(path, cancelled)
        }
        RemovalMode::CancellableDirectoryContents {
            is_cancelled,
            allow_parallel,
        } => remove_directory_contents_cancellable(path, is_cancelled, allow_parallel),
        RemovalMode::NameProtectedDirectoryTree {
            names,
            is_cancelled,
        } => {
            let mut outcome = PermanentDeleteOutcome::default();
            remove_directory_tree_entry(path, is_cancelled, &mut outcome, Some((names, path)))
                .map(|()| outcome)
                .map_err(|error| RemovalFailure {
                    error,
                    verified_outcome: Some(outcome),
                })
        }
    };
    match result {
        Ok(outcome) => {
            if target.metadata.is_dir() {
                log::info!("permanent_delete_finished path={} stage=remove mode=direct outcome=completed removed_files={} removed_logical_bytes={} count_source={} elapsed_ms={}",
                    diagnostic_path(path), outcome.affected_item_count, outcome.released_bytes,
                    if matches!(removal, RemovalMode::DirectoryTree) || matches!(removal, RemovalMode::SnapshotDirectoryTree) && use_native_snapshot_removal(expected_item_count) { "scan_estimate" } else { "observed" },
                    started.elapsed().as_millis());
            }
            Ok(outcome)
        }
        Err(failure) => {
            let known = failure.verified_outcome;
            let outcome = known.unwrap_or_default();
            let mut error = permanent_delete_io_error(
                "permanent deletion stopped; remaining entries stay at their original paths",
                failure.error,
            )
            .at_stage("remove");
            error.partial = known.is_none() || outcome.had_irreversible_mutation;
            error.counts_unknown = known.is_none();
            error.released_bytes = outcome.released_bytes;
            error.affected_item_count = outcome.affected_item_count;
            log::log!(if target.metadata.is_dir() { log::Level::Warn } else { log::Level::Debug }, "permanent_delete_finished path={} stage=remove mode=direct outcome=failed partial={} removed_files={:?} removed_logical_bytes={:?} remaining_location=original native_code={:?} reason={:?} elapsed_ms={} error={}",
                diagnostic_path(path), error.is_partial(), error.observed_or_estimated_files(), error.observed_or_estimated_bytes(),
                error.native_code(), error.reason(), started.elapsed().as_millis(), mangodisk_platform::diagnostics::text(&error));
            Err(error)
        }
    }
}

fn use_native_snapshot_removal(expected_item_count: u64) -> bool {
    // Unix metadata reads add a measurable cost to large observed walks. Keep
    // their native removal throughput; small trees retain exact failure counts.
    // Windows can reuse enumeration metadata without another file query.
    cfg!(unix) && expected_item_count > 256
}

/// Logs only the operation that failed, preserving its native error unchanged.
fn deletion_entry_io<T>(
    path: &Path,
    stage: &str,
    result: std::io::Result<T>,
) -> std::io::Result<T> {
    result.inspect_err(|error| {
        log::warn!(
            "permanent_delete_entry_failed path={} path_scope=operation_target stage={stage} error_kind={:?} native_code={:?} error={}",
            diagnostic_path(path), error.kind(), error.raw_os_error(),
            mangodisk_platform::diagnostics::text(error)
        );
    })
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
fn physical_path_identity(path: &Path) -> Result<PhysicalPathIdentity, PermanentDeleteError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| permanent_delete_io_error("failed to inspect deletion target", error))?;
    if current_platform().is_link_like(&metadata) {
        return Err("the deletion target is a link or reparse point"
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
struct WindowsIdentityInformation {
    identity: PhysicalPathIdentity,
    attributes: u32,
}

#[cfg(windows)]
fn read_windows_identity_information(
    file: &fs::File,
) -> Result<WindowsIdentityInformation, PermanentDeleteError> {
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
        return Err(permanent_delete_io_error(
            "failed to read the physical identity of the deletion target",
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: API success was checked immediately above.
    let information = unsafe { information.assume_init() };
    Ok(WindowsIdentityInformation {
        identity: PhysicalPathIdentity {
            volume: u64::from(information.dwVolumeSerialNumber),
            index: (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow),
        },
        attributes: information.dwFileAttributes,
    })
}

#[cfg(windows)]
fn physical_path_identity(path: &Path) -> Result<PhysicalPathIdentity, PermanentDeleteError> {
    let handle = open_identity_handle(path)?;
    let information = read_windows_identity_information(&handle)?;
    if mangodisk_platform::windows_file_attributes_are_link_like(information.attributes) {
        return Err("the deletion target is a link or reparse point"
            .to_string()
            .into());
    }
    Ok(information.identity)
}

/// Removes a duplicate file only while its size and modification time match the scan.
/// The owning domain coordinates batch logging, operation locking, and cache updates.
pub(crate) fn delete_file_candidate_permanently(
    candidate: &PermanentDeleteCandidate,
) -> Result<(PathBuf, FileSpaceUsage), PermanentDeleteError> {
    let prepared = prepare_path_for_permanent_delete(Path::new(&candidate.path))?;
    validate_permanent_delete_candidate(prepared.metadata(), candidate)?;
    delete_prepared_file(prepared)
}

/// Explicit file selection authorizes the current regular file, including normal edits.
pub(crate) fn delete_selected_file_permanently(
    path: &Path,
) -> Result<(PathBuf, FileSpaceUsage), PermanentDeleteError> {
    delete_prepared_file(prepare_path_for_permanent_delete(path)?)
}

fn delete_prepared_file(
    prepared: PreparedPermanentDelete,
) -> Result<(PathBuf, FileSpaceUsage), PermanentDeleteError> {
    if !prepared.metadata.is_file() {
        return Err(PermanentDeleteError::before_mutation(
            "only regular files can be permanently deleted",
        )
        .with_failure_reason(super::models::PermanentDeleteFailureReason::Protected));
    }
    let target = current_platform()
        .canonicalize_no_links(&prepared.path)
        .map_err(PermanentDeleteError::from)?;
    if current_platform()
        .should_skip(
            &target,
            &current_platform().system_volume_path(),
            ScanPurpose::LargeFiles,
        )
        .is_some()
    {
        return Err(PermanentDeleteError::before_mutation(
            "MangoDisk cannot process a protected system or application file",
        )
        .with_failure_reason(super::models::PermanentDeleteFailureReason::Protected));
    }
    let usage = current_platform().file_space_usage(&prepared.path, prepared.metadata());
    let live_bytes = prepared.metadata().len();
    delete_path_permanently(prepared, live_bytes, 1)?;
    Ok((target, usage))
}

fn validate_permanent_delete_candidate(
    metadata: &fs::Metadata,
    candidate: &PermanentDeleteCandidate,
) -> Result<(), PermanentDeleteError> {
    if !metadata.is_file() || current_platform().is_link_like(metadata) {
        return Err(PermanentDeleteError::before_mutation(
            "only regular files can be permanently deleted",
        )
        .with_failure_reason(super::models::PermanentDeleteFailureReason::Protected));
    }
    if candidate.expected_modified_at_ms.is_none()
        || metadata.len() != candidate.expected_bytes
        || modified_ms(metadata) != candidate.expected_modified_at_ms
    {
        return Err(PermanentDeleteError::before_mutation_with_reason(
            "the file changed after scanning",
            CoreErrorReason::ItemChanged,
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "permanent_delete_tests.rs"]
mod permanent_delete_tests;

#[cfg(test)]
#[path = "direct_delete_tests.rs"]
mod direct_delete_tests;

#[cfg(all(test, windows))]
#[path = "windows_permanent_delete_tests.rs"]
mod windows_permanent_delete_tests;
