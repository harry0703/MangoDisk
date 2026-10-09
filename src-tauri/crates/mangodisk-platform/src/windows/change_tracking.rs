use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    io,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr,
    sync::Arc,
    time::Instant,
};

use windows_sys::Win32::{
    Foundation::ERROR_ACCESS_DENIED,
    Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY,
    System::Ioctl::{
        FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL, READ_USN_JOURNAL_DATA_V1,
        USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V2, USN_REASON_BASIC_INFO_CHANGE, USN_REASON_CLOSE,
        USN_REASON_COMPRESSION_CHANGE, USN_REASON_DATA_EXTEND, USN_REASON_DATA_OVERWRITE,
        USN_REASON_DATA_TRUNCATION, USN_REASON_DESIRED_STORAGE_CLASS_CHANGE, USN_REASON_EA_CHANGE,
        USN_REASON_ENCRYPTION_CHANGE, USN_REASON_FILE_CREATE, USN_REASON_FILE_DELETE,
        USN_REASON_HARD_LINK_CHANGE, USN_REASON_INDEXABLE_CHANGE, USN_REASON_INTEGRITY_CHANGE,
        USN_REASON_NAMED_DATA_EXTEND, USN_REASON_NAMED_DATA_OVERWRITE,
        USN_REASON_NAMED_DATA_TRUNCATION, USN_REASON_OBJECT_ID_CHANGE, USN_REASON_RENAME_NEW_NAME,
        USN_REASON_RENAME_OLD_NAME, USN_REASON_REPARSE_POINT_CHANGE, USN_REASON_SECURITY_CHANGE,
        USN_REASON_STREAM_CHANGE, USN_REASON_TRANSACTED_CHANGE,
    },
};

use crate::{
    FilesystemChangeImpactError, FilesystemChangeImpactOutcome, FilesystemChangeImpactPlan,
    FilesystemChangeImpactSummary, FilesystemChangeImpactUnavailable, FilesystemChangeMonitor,
    FilesystemChangeMonitorBackend, FilesystemChangeStatus, FilesystemChangeToken, Platform,
    ScanPurpose,
};

use super::{
    native_io::{
        device_io_control, file_id, is_ntfs, read_copy, stable_volume_id, AlignedBuffer,
        OwnedHandle, RawLayoutValue, VolumePaths,
    },
    path_identity, WindowsPlatform,
};

const OUTPUT_BUFFER_BYTES: usize = 1024 * 1024;
const USN_RECORD_V2_FIXED_BYTES: usize = 60;
const SUPPORTED_USN_MAJOR_VERSION: u16 = 2;
// CLOSE is only the commit boundary for aggregated reasons and does not affect analysis results
// by itself. Read every other reason, including security, attribute, and reparse-point changes,
// because conservative invalidation is safer than missing an event that affects a path or size.
const RELEVANT_REASONS: u32 = !USN_REASON_CLOSE;
const MAX_PAGES: usize = 16_384;
const MAX_RECORDS: u64 = 10_000_000;
const MAX_PARENT_CACHE_ENTRIES: usize = 100_000;
const MAX_DIRTY_DIRECTORIES: usize = 100_000;
const MAX_DIRTY_PATH_UTF16_UNITS: usize = 8 * 1024 * 1024;
const NTFS_FILE_REFERENCE_MASK: u64 = (1_u64 << 48) - 1;
const NTFS_FIRST_USER_FILE_NUMBER: u64 = 24;
const DATA_CHANGE_REASONS: u32 = USN_REASON_DATA_OVERWRITE
    | USN_REASON_DATA_EXTEND
    | USN_REASON_DATA_TRUNCATION
    | USN_REASON_NAMED_DATA_OVERWRITE
    | USN_REASON_NAMED_DATA_EXTEND
    | USN_REASON_NAMED_DATA_TRUNCATION
    | USN_REASON_STREAM_CHANGE;
const CREATE_DELETE_REASONS: u32 = USN_REASON_FILE_CREATE | USN_REASON_FILE_DELETE;
const RENAME_REASONS: u32 = USN_REASON_RENAME_OLD_NAME | USN_REASON_RENAME_NEW_NAME;
const METADATA_CHANGE_REASONS: u32 = USN_REASON_BASIC_INFO_CHANGE
    | USN_REASON_SECURITY_CHANGE
    | USN_REASON_EA_CHANGE
    | USN_REASON_HARD_LINK_CHANGE
    | USN_REASON_COMPRESSION_CHANGE
    | USN_REASON_ENCRYPTION_CHANGE
    | USN_REASON_OBJECT_ID_CHANGE
    | USN_REASON_REPARSE_POINT_CHANGE
    | USN_REASON_TRANSACTED_CHANGE
    | USN_REASON_INTEGRITY_CHANGE
    | USN_REASON_DESIRED_STORAGE_CLASS_CHANGE
    | USN_REASON_INDEXABLE_CHANGE;
const KNOWN_RELEVANT_REASONS: u32 =
    DATA_CHANGE_REASONS | CREATE_DELETE_REASONS | RENAME_REASONS | METADATA_CHANGE_REASONS;

#[derive(Clone, Copy)]
struct JournalState {
    history_id: u64,
    lowest_valid_usn: u64,
    next_usn: u64,
}

#[derive(Clone, Copy)]
struct UsnEvent<'page> {
    file_id: u64,
    parent_id: u64,
    usn: u64,
    reason: u32,
    attributes: u32,
    name_bytes: &'page [u8],
}

impl UsnEvent<'_> {
    fn decoded_name(self) -> OsString {
        let units = self
            .name_bytes
            .as_chunks::<{ size_of::<u16>() }>()
            .0
            .iter()
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
            .collect::<Vec<_>>();
        OsString::from_wide(&units)
    }
}

#[derive(Default)]
struct DirtyDirectorySet {
    paths: HashMap<Vec<u16>, PathBuf>,
    utf16_units: usize,
}

impl DirtyDirectorySet {
    fn insert(&mut self, path: PathBuf) -> Result<(), FilesystemChangeImpactUnavailable> {
        let identity = windows_path_identity(&path);
        if self.paths.contains_key(&identity) {
            return Ok(());
        }
        if self.paths.len() >= MAX_DIRTY_DIRECTORIES {
            return Err(FilesystemChangeImpactUnavailable::ResourceLimit);
        }
        let units = path.as_os_str().encode_wide().count();
        self.utf16_units = self
            .utf16_units
            .checked_add(units)
            .filter(|total| *total <= MAX_DIRTY_PATH_UTF16_UNITS)
            .ok_or(FilesystemChangeImpactUnavailable::ResourceLimit)?;
        self.paths.insert(identity, path);
        Ok(())
    }

    fn finish(self) -> Vec<PathBuf> {
        let mut paths = self.paths.into_iter().collect::<Vec<_>>();
        paths.sort_by(|(left, _), (right, _)| {
            identity_depth(left)
                .cmp(&identity_depth(right))
                .then_with(|| left.cmp(right))
        });

        // Sorting by depth places ancestors first, after which only the current path's ancestors
        // need HashSet lookups. Comparing only the lexicographic predecessor is incorrect because
        // an unrelated sibling can sort between an ancestor and its descendant. This keeps the
        // work proportional to path depth instead of degrading to O(n²) at 100,000 directories.
        let mut compressed = Vec::<(Vec<u16>, PathBuf)>::with_capacity(paths.len());
        let mut retained = HashSet::<Vec<u16>>::with_capacity(paths.len());
        for (identity, path) in paths {
            if identity_has_ancestor(&identity, &retained) {
                continue;
            }
            retained.insert(identity.clone());
            compressed.push((identity, path));
        }
        compressed.sort_by(|(left, _), (right, _)| left.cmp(right));
        compressed.into_iter().map(|(_, path)| path).collect()
    }
}

#[derive(Debug)]
enum ImpactPlanFailure {
    Cancelled,
    Unavailable(FilesystemChangeImpactUnavailable),
}

struct TerminalMonitor {
    status: FilesystemChangeStatus,
}

impl FilesystemChangeMonitorBackend for TerminalMonitor {
    fn status(&self) -> FilesystemChangeStatus {
        self.status
    }

    fn continuously_tracks_changes(&self) -> bool {
        false
    }
}

unsafe impl RawLayoutValue for USN_JOURNAL_DATA_V0 {}
unsafe impl RawLayoutValue for i64 {}
unsafe impl RawLayoutValue for u16 {}
unsafe impl RawLayoutValue for u32 {}
unsafe impl RawLayoutValue for u64 {}

pub(super) fn capture_token(root: &Path) -> Result<Option<FilesystemChangeToken>, String> {
    let started = Instant::now();
    let Some(volume) = VolumePaths::from_path(root) else {
        return Ok(None);
    };
    if !is_ntfs(&volume.root).map_err(|error| format!("failed to read filesystem type: {error}"))? {
        return Ok(None);
    }
    let volume_id = stable_volume_id(&volume.root)
        .map_err(|error| format!("failed to read stable volume identity: {error}"))?;
    let handle = match OwnedHandle::open_volume(&volume.device) {
        Ok(handle) => handle,
        Err(error) => {
            log_unavailable("open_volume", &error);
            return Ok(None);
        }
    };
    let journal = match query_journal(&handle) {
        Ok(journal) => journal,
        Err(error) => {
            log_unavailable("query_journal", &error);
            return Ok(None);
        }
    };
    log::info!(
        "filesystem_change_token_captured platform=windows elapsed_ms={}",
        started.elapsed().as_millis()
    );
    Ok(Some(FilesystemChangeToken {
        volume_id,
        history_id: journal.history_id,
        cursor: journal.next_usn,
    }))
}

pub(super) fn start_monitor(
    root: &Path,
    token: &FilesystemChangeToken,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Option<FilesystemChangeMonitor>, String> {
    if is_cancelled() {
        return Err("scan cancelled".to_string());
    }
    let started = Instant::now();
    let status = match validate_history(root, token, is_cancelled) {
        Ok(status) => status,
        // Cancellation is an intentional user action, not damaged USN history. Recheck the
        // shared sticky flag after the read loop returns so cancellation is not collapsed into
        // HistoryUnavailable with a misleading journal failure log.
        Err(_) if is_cancelled() => return Err("scan cancelled".to_string()),
        Err(error) => {
            log::warn!(
                "windows_change_validation_failed error={}",
                crate::diagnostics::text(&error)
            );
            FilesystemChangeStatus::HistoryUnavailable
        }
    };
    log::info!(
        "filesystem_change_validation_finished platform=windows status={} elapsed_ms={}",
        status_name(status),
        started.elapsed().as_millis()
    );
    Ok(Some(FilesystemChangeMonitor::new(Arc::new(
        TerminalMonitor { status },
    ))))
}

pub(super) fn impact_plan(
    platform: &WindowsPlatform,
    root: &Path,
    token: &FilesystemChangeToken,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<FilesystemChangeImpactOutcome, FilesystemChangeImpactError> {
    let started = Instant::now();
    let outcome = match build_impact_plan(platform, root, token, is_cancelled) {
        Ok(outcome) => outcome,
        Err(ImpactPlanFailure::Cancelled) => return Err(FilesystemChangeImpactError::Cancelled),
        Err(ImpactPlanFailure::Unavailable(reason)) => {
            log::info!(
                "filesystem_change_impact_unavailable platform=windows reason={}",
                reason.as_str()
            );
            FilesystemChangeImpactOutcome::Unavailable(reason)
        }
    };
    match &outcome {
        FilesystemChangeImpactOutcome::Complete(plan) => log::info!(
            "filesystem_change_impact_finished platform=windows status=complete pages={} records={} dirty_directories={} elapsed_ms={}",
            plan.summary.page_count,
            plan.summary.record_count,
            plan.summary.dirty_directory_count,
            started.elapsed().as_millis()
        ),
        FilesystemChangeImpactOutcome::Unavailable(_) => {}
    }
    Ok(outcome)
}

fn build_impact_plan(
    platform: &WindowsPlatform,
    root: &Path,
    token: &FilesystemChangeToken,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<FilesystemChangeImpactOutcome, ImpactPlanFailure> {
    check_impact_cancelled(is_cancelled)?;
    let Some(volume) = VolumePaths::from_scan_root(root) else {
        return Ok(FilesystemChangeImpactOutcome::Unavailable(
            FilesystemChangeImpactUnavailable::UnsupportedRoot,
        ));
    };
    let is_supported = is_ntfs(&volume.root).map_err(|error| {
        unavailable_io(
            &error,
            FilesystemChangeImpactUnavailable::UnsupportedFilesystem,
        )
    })?;
    if !is_supported {
        return Ok(FilesystemChangeImpactOutcome::Unavailable(
            FilesystemChangeImpactUnavailable::UnsupportedFilesystem,
        ));
    }
    let volume_id = stable_volume_id(&volume.root).map_err(|error| {
        unavailable_io(
            &error,
            FilesystemChangeImpactUnavailable::HistoryUnavailable,
        )
    })?;
    if volume_id != token.volume_id {
        return Ok(FilesystemChangeImpactOutcome::Unavailable(
            FilesystemChangeImpactUnavailable::HistoryUnavailable,
        ));
    }
    let handle = OwnedHandle::open_volume(&volume.device).map_err(|error| {
        unavailable_io(
            &error,
            FilesystemChangeImpactUnavailable::HistoryUnavailable,
        )
    })?;
    let journal = query_journal(&handle).map_err(|error| {
        unavailable_io(
            &error,
            FilesystemChangeImpactUnavailable::HistoryUnavailable,
        )
    })?;
    if !journal_contains_token(journal, token) {
        return Ok(FilesystemChangeImpactOutcome::Unavailable(
            FilesystemChangeImpactUnavailable::HistoryUnavailable,
        ));
    }
    let root_id = file_id(&volume.root).map_err(|error| {
        unavailable_io(&error, FilesystemChangeImpactUnavailable::ParentUnavailable)
    })?;

    let started = Instant::now();
    let mut summary = FilesystemChangeImpactSummary {
        start_cursor: token.cursor,
        end_cursor: journal.next_usn,
        strategy: "windows-usn-v2-impact",
        ..FilesystemChangeImpactSummary::default()
    };
    let mut dirty = DirtyDirectorySet::default();
    let mut dirty_parent_ids = HashSet::<u64>::new();
    let mut parent_paths = HashMap::<u64, (PathBuf, bool)>::new();
    let mut cursor = token.cursor;
    let mut buffer = AlignedBuffer::new(OUTPUT_BUFFER_BYTES);

    for _ in 0..MAX_PAGES {
        check_impact_cancelled(is_cancelled)?;
        if cursor >= journal.next_usn {
            return complete_impact_plan(token, journal, dirty, summary, started);
        }
        let input = journal_read_input(cursor, token.history_id)?;
        let returned = device_io_control(
            handle.raw(),
            FSCTL_READ_USN_JOURNAL,
            ptr::from_ref(&input).cast(),
            size_of::<READ_USN_JOURNAL_DATA_V1>(),
            buffer.as_mut_ptr(),
            buffer.capacity_bytes(),
        )
        .map_err(|code| {
            if code == ERROR_ACCESS_DENIED {
                ImpactPlanFailure::Unavailable(FilesystemChangeImpactUnavailable::PermissionDenied)
            } else {
                ImpactPlanFailure::Unavailable(
                    FilesystemChangeImpactUnavailable::HistoryUnavailable,
                )
            }
        })?;
        summary.page_count =
            summary
                .page_count
                .checked_add(1)
                .ok_or(ImpactPlanFailure::Unavailable(
                    FilesystemChangeImpactUnavailable::ResourceLimit,
                ))?;
        summary.returned_bytes = summary.returned_bytes.checked_add(returned as u64).ok_or(
            ImpactPlanFailure::Unavailable(FilesystemChangeImpactUnavailable::ResourceLimit),
        )?;
        let bytes = buffer
            .as_bytes(returned)
            .ok_or(ImpactPlanFailure::Unavailable(
                FilesystemChangeImpactUnavailable::InvalidJournal,
            ))?;
        let (next_cursor, events) = parse_page(bytes).map_err(|_| {
            ImpactPlanFailure::Unavailable(FilesystemChangeImpactUnavailable::InvalidJournal)
        })?;
        let page_has_records =
            validate_page_progress(cursor, next_cursor, &events).map_err(|_| {
                ImpactPlanFailure::Unavailable(FilesystemChangeImpactUnavailable::InvalidJournal)
            })?;
        if !page_has_records {
            return complete_impact_plan(token, journal, dirty, summary, started);
        }
        cursor = next_cursor;

        for event in events {
            check_impact_cancelled(is_cancelled)?;
            if event.usn >= journal.next_usn {
                continue;
            }
            if event.reason & RELEVANT_REASONS == 0 {
                continue;
            }
            observe_impact_reason(&mut summary, &event)?;
            if is_ntfs_internal_reference(event.file_id)
                || (is_ntfs_internal_reference(event.parent_id)
                    && !same_file_reference(event.parent_id, root_id))
            {
                continue;
            }

            let parent_is_root = same_file_reference(event.parent_id, root_id);
            if !parent_is_root && !parent_paths.contains_key(&event.parent_id) {
                if parent_paths.len() >= MAX_PARENT_CACHE_ENTRIES {
                    return Ok(FilesystemChangeImpactOutcome::Unavailable(
                        FilesystemChangeImpactUnavailable::ResourceLimit,
                    ));
                }
                check_impact_cancelled(is_cancelled)?;
                let parent = resolve_parent(&handle, &volume.root, event.parent_id)?;
                let parent_allowed = platform
                    .should_skip(&parent, &volume.root, ScanPurpose::Analysis)
                    .is_none();
                parent_paths.insert(event.parent_id, (parent, parent_allowed));
                summary.parent_cache_peak = summary.parent_cache_peak.max(parent_paths.len());
            }
            let (parent, parent_allowed) = if parent_is_root {
                (&volume.root, true)
            } else {
                let (parent, parent_allowed) = parent_paths
                    .get(&event.parent_id)
                    .expect("a successfully resolved parent must be present in the current cache");
                (parent, *parent_allowed)
            };
            if relevant_parent(
                platform,
                &volume.root,
                parent,
                parent_is_root,
                parent_allowed,
                event,
            ) && dirty_parent_ids.insert(event.parent_id)
            {
                dirty
                    .insert(parent.clone())
                    .map_err(ImpactPlanFailure::Unavailable)?;
            }
        }
    }
    Ok(FilesystemChangeImpactOutcome::Unavailable(
        FilesystemChangeImpactUnavailable::ResourceLimit,
    ))
}

fn journal_read_input(
    cursor: u64,
    history_id: u64,
) -> Result<READ_USN_JOURNAL_DATA_V1, ImpactPlanFailure> {
    Ok(READ_USN_JOURNAL_DATA_V1 {
        StartUsn: i64::try_from(cursor).map_err(|_| {
            ImpactPlanFailure::Unavailable(FilesystemChangeImpactUnavailable::InvalidJournal)
        })?,
        ReasonMask: RELEVANT_REASONS,
        ReturnOnlyOnClose: 0,
        Timeout: 0,
        BytesToWaitFor: 0,
        UsnJournalID: history_id,
        MinMajorVersion: SUPPORTED_USN_MAJOR_VERSION,
        MaxMajorVersion: SUPPORTED_USN_MAJOR_VERSION,
    })
}

fn observe_impact_reason(
    summary: &mut FilesystemChangeImpactSummary,
    event: &UsnEvent,
) -> Result<(), ImpactPlanFailure> {
    let reason = event.reason & RELEVANT_REASONS;
    increment_counter(&mut summary.record_count)?;
    if summary.record_count > MAX_RECORDS {
        return Err(ImpactPlanFailure::Unavailable(
            FilesystemChangeImpactUnavailable::ResourceLimit,
        ));
    }
    if reason & DATA_CHANGE_REASONS != 0 {
        increment_counter(&mut summary.data_change_records)?;
    }
    if reason & CREATE_DELETE_REASONS != 0 {
        increment_counter(&mut summary.create_delete_records)?;
    }
    if reason & RENAME_REASONS != 0 {
        increment_counter(&mut summary.rename_records)?;
    }
    if reason & METADATA_CHANGE_REASONS != 0 {
        increment_counter(&mut summary.metadata_change_records)?;
    }
    if reason & !KNOWN_RELEVANT_REASONS != 0 {
        increment_counter(&mut summary.other_records)?;
    }
    if event.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        increment_counter(&mut summary.directory_records)?;
    }
    Ok(())
}

fn increment_counter(value: &mut u64) -> Result<(), ImpactPlanFailure> {
    *value = value.checked_add(1).ok_or(ImpactPlanFailure::Unavailable(
        FilesystemChangeImpactUnavailable::ResourceLimit,
    ))?;
    Ok(())
}

fn resolve_parent(
    volume_handle: &OwnedHandle,
    root: &Path,
    parent_id: u64,
) -> Result<PathBuf, ImpactPlanFailure> {
    let parent_handle = OwnedHandle::open_file_by_id(volume_handle, parent_id).map_err(|_| {
        // Deleting an entire tree can make child parent IDs impossible to open. USN does not keep
        // a reliable old path, so do not guess the ancestor. Report unavailability and let the
        // caller run a full scan instead of retaining stale size data.
        ImpactPlanFailure::Unavailable(FilesystemChangeImpactUnavailable::ParentUnavailable)
    })?;
    let parent = parent_handle.final_dos_path().map_err(|_| {
        ImpactPlanFailure::Unavailable(FilesystemChangeImpactUnavailable::ParentUnavailable)
    })?;
    if !impact_path_is_same_or_child(&parent, root) {
        return Err(ImpactPlanFailure::Unavailable(
            FilesystemChangeImpactUnavailable::ParentUnavailable,
        ));
    }
    Ok(parent)
}

fn relevant_parent(
    platform: &WindowsPlatform,
    root: &Path,
    parent: &Path,
    parent_is_root: bool,
    parent_allowed: bool,
    event: UsnEvent<'_>,
) -> bool {
    if !parent_allowed {
        return false;
    }
    // Analysis exclusions inspect only the first two components below the root. Once a non-root
    // parent is accepted, a child cannot change those components, so avoid allocating an OsString
    // for every USN record. Decode only direct root children whose names can themselves introduce
    // a protected boundary such as `$Recycle.Bin`.
    if !parent_is_root {
        return true;
    }
    platform
        .should_skip(
            &parent.join(event.decoded_name()),
            root,
            ScanPurpose::Analysis,
        )
        .is_none()
}

fn complete_impact_plan(
    token: &FilesystemChangeToken,
    journal: JournalState,
    dirty: DirtyDirectorySet,
    mut summary: FilesystemChangeImpactSummary,
    started: Instant,
) -> Result<FilesystemChangeImpactOutcome, ImpactPlanFailure> {
    let dirty_directories = dirty.finish();
    summary.dirty_directory_count = u64::try_from(dirty_directories.len()).map_err(|_| {
        ImpactPlanFailure::Unavailable(FilesystemChangeImpactUnavailable::ResourceLimit)
    })?;
    summary.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(FilesystemChangeImpactOutcome::Complete(
        FilesystemChangeImpactPlan {
            dirty_directories,
            next_token: FilesystemChangeToken {
                volume_id: token.volume_id,
                history_id: journal.history_id,
                cursor: journal.next_usn,
            },
            summary,
        },
    ))
}

fn check_impact_cancelled(
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), ImpactPlanFailure> {
    if is_cancelled() {
        Err(ImpactPlanFailure::Cancelled)
    } else {
        Ok(())
    }
}

fn unavailable_io(
    error: &io::Error,
    fallback: FilesystemChangeImpactUnavailable,
) -> ImpactPlanFailure {
    let reason = if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
        FilesystemChangeImpactUnavailable::PermissionDenied
    } else {
        fallback
    };
    ImpactPlanFailure::Unavailable(reason)
}

fn same_file_reference(left: u64, right: u64) -> bool {
    left & NTFS_FILE_REFERENCE_MASK == right & NTFS_FILE_REFERENCE_MASK
}

fn is_ntfs_internal_reference(file_id: u64) -> bool {
    file_id & NTFS_FILE_REFERENCE_MASK < NTFS_FIRST_USER_FILE_NUMBER
}

fn windows_path_identity(path: &Path) -> Vec<u16> {
    let mut identity = path
        .as_os_str()
        .encode_wide()
        .map(|unit| match unit {
            0x2f => 0x5c,
            value if (u16::from(b'A')..=u16::from(b'Z')).contains(&value) => {
                value + u16::from(b'a' - b'A')
            }
            value => value,
        })
        .collect::<Vec<_>>();
    let verbatim = [
        u16::from(b'\\'),
        u16::from(b'\\'),
        u16::from(b'?'),
        u16::from(b'\\'),
    ];
    let verbatim_unc = [
        u16::from(b'\\'),
        u16::from(b'\\'),
        u16::from(b'?'),
        u16::from(b'\\'),
        u16::from(b'u'),
        u16::from(b'n'),
        u16::from(b'c'),
        u16::from(b'\\'),
    ];
    if identity.starts_with(&verbatim_unc) {
        identity.splice(0..verbatim_unc.len(), [u16::from(b'\\'), u16::from(b'\\')]);
    } else if identity.starts_with(&verbatim) {
        identity.drain(0..verbatim.len());
    }
    while identity.last() == Some(&u16::from(b'\\')) {
        identity.pop();
    }
    identity
}

fn identity_depth(path: &[u16]) -> usize {
    path.iter()
        .filter(|value| **value == u16::from(b'\\'))
        .count()
}

fn identity_has_ancestor(path: &[u16], candidates: &HashSet<Vec<u16>>) -> bool {
    let mut ancestor = path;
    while let Some(separator) = ancestor
        .iter()
        .rposition(|value| *value == u16::from(b'\\'))
    {
        ancestor = &ancestor[..separator];
        if candidates.contains(ancestor) {
            return true;
        }
    }
    false
}

fn impact_path_is_same_or_child(path: &Path, root: &Path) -> bool {
    let path = windows_path_identity(path);
    let root = windows_path_identity(root);
    path == root
        || path
            .strip_prefix(root.as_slice())
            .is_some_and(|suffix| suffix.starts_with(&[u16::from(b'\\')]))
}

fn validate_history(
    root: &Path,
    token: &FilesystemChangeToken,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<FilesystemChangeStatus, String> {
    let volume = VolumePaths::from_path(root)
        .ok_or_else(|| "scan root is not on a supported local drive-letter volume".to_string())?;
    if !is_ntfs(&volume.root)
        .map_err(|error| format!("failed to validate filesystem type: {error}"))?
    {
        return Ok(FilesystemChangeStatus::HistoryUnavailable);
    }
    let current_volume_id = stable_volume_id(&volume.root)
        .map_err(|error| format!("failed to validate stable volume identity: {error}"))?;
    if current_volume_id != token.volume_id {
        return Ok(FilesystemChangeStatus::HistoryUnavailable);
    }
    let handle = OwnedHandle::open_volume(&volume.device)
        .map_err(|error| format!("failed to open USN volume handle: {error}"))?;
    let journal =
        query_journal(&handle).map_err(|error| format!("failed to query USN journal: {error}"))?;
    if !journal_contains_token(journal, token) {
        return Ok(FilesystemChangeStatus::HistoryUnavailable);
    }
    if token.cursor == journal.next_usn {
        return Ok(FilesystemChangeStatus::Clean);
    }

    let root_id =
        file_id(root).map_err(|error| format!("failed to read scan root identity: {error}"))?;
    let volume_root_id = file_id(&volume.root)
        .map_err(|error| format!("failed to read volume root identity: {error}"))?;
    if root_id == volume_root_id {
        return Ok(FilesystemChangeStatus::Changed);
    }

    read_events(
        &handle,
        root,
        root_id,
        token,
        journal.next_usn,
        is_cancelled,
    )
}

fn journal_contains_token(journal: JournalState, token: &FilesystemChangeToken) -> bool {
    journal.history_id == token.history_id
        && token.cursor >= journal.lowest_valid_usn
        && token.cursor <= journal.next_usn
}

fn read_events(
    handle: &OwnedHandle,
    root: &Path,
    root_id: u64,
    token: &FilesystemChangeToken,
    validation_upper_usn: u64,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<FilesystemChangeStatus, String> {
    let mut cursor = token.cursor;
    let mut buffer = AlignedBuffer::new(OUTPUT_BUFFER_BYTES);
    let mut parent_relevance = HashMap::<u64, bool>::new();
    let mut record_count = 0_u64;

    for _ in 0..MAX_PAGES {
        if is_cancelled() {
            return Err("scan cancelled".to_string());
        }
        if cursor >= validation_upper_usn {
            return Ok(FilesystemChangeStatus::Clean);
        }
        let input = READ_USN_JOURNAL_DATA_V1 {
            StartUsn: i64::try_from(cursor)
                .map_err(|_| "USN cursor is outside the Windows API range".to_string())?,
            ReasonMask: RELEVANT_REASONS,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 0,
            UsnJournalID: token.history_id,
            MinMajorVersion: SUPPORTED_USN_MAJOR_VERSION,
            MaxMajorVersion: SUPPORTED_USN_MAJOR_VERSION,
        };
        let returned = device_io_control(
            handle.raw(),
            FSCTL_READ_USN_JOURNAL,
            ptr::from_ref(&input).cast(),
            size_of::<READ_USN_JOURNAL_DATA_V1>(),
            buffer.as_mut_ptr(),
            buffer.capacity_bytes(),
        )
        .map_err(|code| format!("failed to read USN history with error code {code}"))?;
        let bytes = buffer
            .as_bytes(returned)
            .ok_or_else(|| "USN API returned more bytes than the output buffer".to_string())?;
        let (next_cursor, events) = parse_page(bytes)?;
        let page_has_records = validate_page_progress(cursor, next_cursor, &events)?;
        if !page_has_records {
            // Microsoft explicitly permits a cursor equal to StartUsn when BytesToWaitFor is zero
            // and no record matches ReasonMask. CLOSE-only records are excluded, so this can occur
            // even when the journal's NextUsn advanced. It means no cache-relevant record exists
            // before the validation boundary, not that pagination is corrupt.
            return Ok(FilesystemChangeStatus::Clean);
        }
        cursor = next_cursor;

        for event in events {
            if event.usn >= validation_upper_usn {
                continue;
            }
            record_count = record_count
                .checked_add(1)
                .ok_or_else(|| "USN record count overflowed".to_string())?;
            if record_count > MAX_RECORDS {
                return Err("USN record count exceeded the safety limit".to_string());
            }
            if event.file_id == root_id || event.parent_id == root_id {
                return Ok(FilesystemChangeStatus::Changed);
            }
            let relevant = if let Some(relevant) = parent_relevance.get(&event.parent_id) {
                *relevant
            } else {
                if parent_relevance.len() >= MAX_PARENT_CACHE_ENTRIES {
                    return Ok(FilesystemChangeStatus::HistoryUnavailable);
                }
                let parent = match OwnedHandle::open_file_by_id(handle, event.parent_id) {
                    Ok(parent) => parent,
                    // A deleted parent can no longer be proven inside or outside the root.
                    // Invalidate conservatively so deleting a directory tree cannot reuse an
                    // analysis result that still contains its former files.
                    Err(_) => return Ok(FilesystemChangeStatus::HistoryUnavailable),
                };
                let parent_path = match parent.final_dos_path() {
                    Ok(path) => path,
                    Err(_) => return Ok(FilesystemChangeStatus::HistoryUnavailable),
                };
                let relevant = path_identity::is_same_or_child(&parent_path, root);
                parent_relevance.insert(event.parent_id, relevant);
                relevant
            };
            if relevant {
                return Ok(FilesystemChangeStatus::Changed);
            }
        }
    }
    Err("USN page count exceeded the safety limit".to_string())
}

/// USN records and page cursors both come from FFI. Records returned by the kernel must fall
/// within the page's half-open interval and increase strictly. Trusting only an advancing next
/// cursor could incorrectly publish Clean for a corrupt page. Windows defines an equal cursor on
/// an empty page as normal completion, which must remain distinct from regression or stagnation.
fn validate_page_progress(
    start_cursor: u64,
    next_cursor: u64,
    events: &[UsnEvent],
) -> Result<bool, String> {
    if next_cursor < start_cursor {
        return Err("USN page cursor regressed".to_string());
    }
    if next_cursor == start_cursor {
        return if events.is_empty() {
            Ok(false)
        } else {
            Err("USN page contained records without advancing the cursor".to_string())
        };
    }
    let mut previous_usn = None;
    for event in events {
        if event.usn < start_cursor || event.usn >= next_cursor {
            return Err("USN record cursor is outside the current page range".to_string());
        }
        if previous_usn.is_some_and(|previous| event.usn <= previous) {
            return Err("USN record cursors are not strictly increasing".to_string());
        }
        previous_usn = Some(event.usn);
    }
    Ok(true)
}

fn query_journal(handle: &OwnedHandle) -> io::Result<JournalState> {
    // Newer systems can return V1/V2 extension tails. Parse only the V0 prefix shared by every
    // supported response version.
    let mut buffer = AlignedBuffer::new(size_of::<USN_JOURNAL_DATA_V2>());
    let returned = device_io_control(
        handle.raw(),
        FSCTL_QUERY_USN_JOURNAL,
        ptr::null(),
        0,
        buffer.as_mut_ptr(),
        buffer.capacity_bytes(),
    )
    .map_err(|code| io::Error::from_raw_os_error(code as i32))?;
    let bytes = buffer.as_bytes(returned).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "USN query length exceeded the buffer",
        )
    })?;
    let data = read_copy::<USN_JOURNAL_DATA_V0>(bytes, 0).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "USN query result was truncated",
        )
    })?;
    let lowest_valid_usn = u64::try_from(data.LowestValidUsn).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "USN lowest valid cursor was negative",
        )
    })?;
    let next_usn = u64::try_from(data.NextUsn)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "USN next cursor was negative"))?;
    if lowest_valid_usn > next_usn {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "USN readable range was reversed",
        ));
    }
    Ok(JournalState {
        history_id: data.UsnJournalID,
        lowest_valid_usn,
        next_usn,
    })
}

fn parse_page(bytes: &[u8]) -> Result<(u64, Vec<UsnEvent<'_>>), String> {
    let next_cursor = read_copy::<i64>(bytes, 0)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| "USN page is missing a valid next cursor".to_string())?;
    let mut offset = size_of::<i64>();
    let mut events = Vec::new();
    while offset < bytes.len() {
        let record_length = read_copy::<u32>(bytes, offset)
            .ok_or_else(|| "USN record is missing its length".to_string())?
            as usize;
        if record_length < USN_RECORD_V2_FIXED_BYTES || !record_length.is_multiple_of(8) {
            return Err("USN record length or alignment is invalid".to_string());
        }
        let end = offset
            .checked_add(record_length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| "USN record extends beyond its page".to_string())?;
        let major = read_copy::<u16>(bytes, offset + 4)
            .ok_or_else(|| "USN record is missing its major version".to_string())?;
        let minor = read_copy::<u16>(bytes, offset + 6)
            .ok_or_else(|| "USN record is missing its minor version".to_string())?;
        if major != SUPPORTED_USN_MAJOR_VERSION || minor != 0 {
            return Err(format!("unsupported USN record version: {major}.{minor}"));
        }
        let file_id = read_copy::<u64>(bytes, offset + 8)
            .ok_or_else(|| "USN record is missing its file ID".to_string())?;
        let parent_id = read_copy::<u64>(bytes, offset + 16)
            .ok_or_else(|| "USN record is missing its parent file ID".to_string())?;
        let usn = read_copy::<i64>(bytes, offset + 24)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| "USN record cursor is invalid".to_string())?;
        let reason = read_copy::<u32>(bytes, offset + 40)
            .ok_or_else(|| "USN reason is missing".to_string())?;
        let attributes = read_copy::<u32>(bytes, offset + 52)
            .ok_or_else(|| "USN file attributes are missing".to_string())?;
        let name_length = read_copy::<u16>(bytes, offset + 56)
            .ok_or_else(|| "USN file name length is missing".to_string())?
            as usize;
        let name_offset = read_copy::<u16>(bytes, offset + 58)
            .ok_or_else(|| "USN file name offset is missing".to_string())?
            as usize;
        if !name_length.is_multiple_of(size_of::<u16>())
            || name_offset < USN_RECORD_V2_FIXED_BYTES
            || !name_offset.is_multiple_of(size_of::<u16>())
            || name_offset
                .checked_add(name_length)
                .filter(|name_end| *name_end <= record_length)
                .is_none()
        {
            return Err("USN file name range is invalid".to_string());
        }
        let name_start = offset + name_offset;
        let name_end = name_start + name_length;
        let name_bytes = &bytes[name_start..name_end];
        if !valid_file_name_bytes(name_bytes) {
            return Err("USN file name is not a safe single path component".to_string());
        }
        events.push(UsnEvent {
            file_id,
            parent_id,
            usn,
            reason,
            attributes,
            name_bytes,
        });
        offset = end;
    }
    Ok((next_cursor, events))
}

fn valid_file_name_bytes(name: &[u8]) -> bool {
    if name.is_empty() || !name.len().is_multiple_of(size_of::<u16>()) {
        return false;
    }
    let first = u16::from_le_bytes([name[0], name[1]]);
    if (name.len() == 2 && first == u16::from(b'.'))
        || (name.len() == 4
            && first == u16::from(b'.')
            && u16::from_le_bytes([name[2], name[3]]) == u16::from(b'.'))
    {
        return false;
    }
    name.as_chunks::<{ size_of::<u16>() }>()
        .0
        .iter()
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
        .all(|unit| !matches!(unit, 0 | 0x2f | 0x3a | 0x5c))
}

fn log_unavailable(stage: &str, error: &io::Error) {
    log::info!(
        "windows_change_journal_unavailable stage={stage} error_code={}",
        error.raw_os_error().unwrap_or_default()
    );
}

fn status_name(status: FilesystemChangeStatus) -> &'static str {
    match status {
        FilesystemChangeStatus::Pending => "pending",
        FilesystemChangeStatus::Clean => "clean",
        FilesystemChangeStatus::Changed => "changed",
        FilesystemChangeStatus::HistoryUnavailable => "history_unavailable",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Mutex, MutexGuard, OnceLock},
        time::Duration,
    };

    use super::*;

    /// Real USN tests share one journal and deliberately delete fixture parents. Running them in
    /// parallel can make an unrelated test's deleted parent impossible to resolve, which correctly
    /// produces the production fail-closed status but makes the test result nondeterministic.
    fn real_usn_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn v2_record_with(
        file_id: u64,
        parent_id: u64,
        usn: i64,
        reason: u32,
        attributes: u32,
        name: &[u16],
    ) -> Vec<u8> {
        let record_length = USN_RECORD_V2_FIXED_BYTES + size_of_val(name);
        let aligned_length = record_length.next_multiple_of(8);
        let mut bytes = vec![0_u8; aligned_length];
        bytes[0..4].copy_from_slice(&(aligned_length as u32).to_le_bytes());
        bytes[4..6].copy_from_slice(&SUPPORTED_USN_MAJOR_VERSION.to_le_bytes());
        bytes[8..16].copy_from_slice(&file_id.to_le_bytes());
        bytes[16..24].copy_from_slice(&parent_id.to_le_bytes());
        bytes[24..32].copy_from_slice(&usn.to_le_bytes());
        bytes[40..44].copy_from_slice(&reason.to_le_bytes());
        bytes[52..56].copy_from_slice(&attributes.to_le_bytes());
        bytes[56..58].copy_from_slice(&((name.len() * 2) as u16).to_le_bytes());
        bytes[58..60].copy_from_slice(&(USN_RECORD_V2_FIXED_BYTES as u16).to_le_bytes());
        for (index, value) in name.iter().enumerate() {
            let start = USN_RECORD_V2_FIXED_BYTES + index * 2;
            bytes[start..start + 2].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    fn v2_record(file_id: u64, parent_id: u64, usn: i64, name: &[u16]) -> Vec<u8> {
        v2_record_with(file_id, parent_id, usn, USN_REASON_FILE_CREATE, 0, name)
    }

    #[test]
    fn usn_v2_page_parses_records_and_cursor() {
        let mut page = 100_i64.to_le_bytes().to_vec();
        page.extend(v2_record(7, 3, 90, &[b'a' as u16]));
        let (cursor, events) = parse_page(&page).expect("a valid V2 page should parse");
        assert_eq!(cursor, 100);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].file_id, 7);
        assert_eq!(events[0].parent_id, 3);
        assert_eq!(events[0].usn, 90);
        assert_eq!(events[0].reason, USN_REASON_FILE_CREATE);
        assert_eq!(events[0].attributes, 0);
        assert_eq!(events[0].decoded_name(), OsString::from("a"));
        assert!(validate_page_progress(80, cursor, &events)
            .expect("a valid page cursor should pass validation"));
    }

    #[test]
    fn usn_page_rejects_unknown_version_and_truncation() {
        let mut unknown = 100_i64.to_le_bytes().to_vec();
        let mut record = v2_record(7, 3, 90, &[b'a' as u16]);
        record[4..6].copy_from_slice(&3_u16.to_le_bytes());
        unknown.extend(record);
        assert!(parse_page(&unknown).is_err());

        let mut truncated = 100_i64.to_le_bytes().to_vec();
        let mut record = v2_record(7, 3, 90, &[b'a' as u16]);
        record.truncate(record.len() - 1);
        truncated.extend(record);
        assert!(parse_page(&truncated).is_err());
    }

    #[test]
    fn usn_page_rejects_invalid_name_range_and_negative_cursor() {
        let mut page = 100_i64.to_le_bytes().to_vec();
        let mut record = v2_record(7, 3, 90, &[b'a' as u16]);
        record[58..60].copy_from_slice(&59_u16.to_le_bytes());
        page.extend(record);
        assert!(parse_page(&page).is_err());
        assert!(parse_page(&(-1_i64).to_le_bytes()).is_err());
    }

    #[test]
    fn journal_identity_and_readable_range_are_required() {
        let journal = JournalState {
            history_id: 7,
            lowest_valid_usn: 10,
            next_usn: 20,
        };
        let token = |history_id, cursor| FilesystemChangeToken {
            volume_id: [0; 16],
            history_id,
            cursor,
        };
        assert!(journal_contains_token(journal, &token(7, 10)));
        assert!(journal_contains_token(journal, &token(7, 20)));
        assert!(!journal_contains_token(journal, &token(6, 15)));
        assert!(!journal_contains_token(journal, &token(7, 9)));
        assert!(!journal_contains_token(journal, &token(7, 21)));
    }

    #[test]
    fn empty_usn_page_can_finish_without_advancing_cursor() {
        assert!(
            !validate_page_progress(100, 100, &[])
                .expect("no matching records should be a valid terminal state"),
            "an empty page should mean the current range has no matching records"
        );
        assert!(validate_page_progress(
            100,
            100,
            &[UsnEvent {
                file_id: 1,
                parent_id: 2,
                usn: 100,
                reason: USN_REASON_FILE_CREATE,
                attributes: 0,
                name_bytes: &[],
            }]
        )
        .is_err());
    }

    #[test]
    fn usn_page_rejects_out_of_range_and_non_monotonic_records() {
        let event = |usn| UsnEvent {
            file_id: usn,
            parent_id: 2,
            usn,
            reason: USN_REASON_FILE_CREATE,
            attributes: 0,
            name_bytes: &[],
        };
        assert!(validate_page_progress(100, 110, &[event(99)]).is_err());
        assert!(validate_page_progress(100, 110, &[event(110)]).is_err());
        assert!(validate_page_progress(100, 110, &[event(105), event(105)]).is_err());
        assert!(validate_page_progress(100, 110, &[event(106), event(105)]).is_err());
        assert!(validate_page_progress(100, 99, &[]).is_err());
    }

    #[test]
    fn usn_v2_page_preserves_reason_attributes_and_windows_name() {
        let name = "cache-Δ-🧪.bin".encode_utf16().collect::<Vec<_>>();
        let reason = USN_REASON_DATA_EXTEND | USN_REASON_RENAME_NEW_NAME;
        let mut page = 100_i64.to_le_bytes().to_vec();
        page.extend(v2_record_with(7, 3, 90, reason, 0x20, &name));

        let (_, events) =
            parse_page(&page).expect("a valid Windows file name should parse losslessly");
        assert_eq!(events[0].reason, reason);
        assert_eq!(events[0].attributes, 0x20);
        assert_eq!(events[0].decoded_name(), OsString::from_wide(&name));
    }

    #[test]
    fn usn_v2_page_parses_multiple_records_and_unknown_reason() {
        let unknown_reason = 0x0200_0000;
        let mut page = 120_i64.to_le_bytes().to_vec();
        page.extend(v2_record_with(
            7,
            3,
            100,
            USN_REASON_DATA_EXTEND | USN_REASON_CLOSE,
            0,
            &[u16::from(b'a')],
        ));
        page.extend(v2_record_with(
            8,
            3,
            110,
            unknown_reason,
            FILE_ATTRIBUTE_DIRECTORY,
            &"directory-Δ".encode_utf16().collect::<Vec<_>>(),
        ));

        let (cursor, events) =
            parse_page(&page).expect("a V2 page with multiple records should parse completely");
        assert_eq!(cursor, 120);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].reason, USN_REASON_DATA_EXTEND | USN_REASON_CLOSE);
        assert_eq!(events[1].reason, unknown_reason);
        assert_eq!(events[1].attributes, FILE_ATTRIBUTE_DIRECTORY);
        assert_eq!(events[1].decoded_name(), OsString::from("directory-Δ"));
        assert!(validate_page_progress(90, cursor, &events)
            .expect("strictly increasing records should pass validation"));

        let mut summary = FilesystemChangeImpactSummary::default();
        for event in &events {
            observe_impact_reason(&mut summary, event)
                .expect("an unknown reason should be counted conservatively");
        }
        assert_eq!(summary.record_count, 2);
        assert_eq!(summary.data_change_records, 1);
        assert_eq!(summary.other_records, 1);
        assert_eq!(summary.directory_records, 1);
    }

    #[test]
    fn usn_v2_page_rejects_unsafe_name_and_minor_version() {
        for name in [
            Vec::new(),
            vec![u16::from(b'.')],
            vec![u16::from(b'.'), u16::from(b'.')],
            vec![u16::from(b'a'), 0, u16::from(b'b')],
            vec![u16::from(b'a'), u16::from(b'\\'), u16::from(b'b')],
            vec![u16::from(b'a'), u16::from(b'/'), u16::from(b'b')],
            vec![u16::from(b'a'), u16::from(b':'), u16::from(b'b')],
        ] {
            let mut page = 100_i64.to_le_bytes().to_vec();
            page.extend(v2_record(7, 3, 90, &name));
            assert!(
                parse_page(&page).is_err(),
                "an unsafe name must fail closed"
            );
        }

        let mut page = 100_i64.to_le_bytes().to_vec();
        let mut record = v2_record(7, 3, 90, &[u16::from(b'a')]);
        record[6..8].copy_from_slice(&1_u16.to_le_bytes());
        page.extend(record);
        assert!(parse_page(&page).is_err());
    }

    #[test]
    fn usn_v2_page_rejects_misalignment_and_odd_utf16_length() {
        let mut misaligned = 100_i64.to_le_bytes().to_vec();
        let mut record = v2_record(7, 3, 90, &[u16::from(b'a')]);
        record[0..4].copy_from_slice(&61_u32.to_le_bytes());
        misaligned.extend(record);
        assert!(parse_page(&misaligned).is_err());

        let mut odd_name = 100_i64.to_le_bytes().to_vec();
        let mut record = v2_record(7, 3, 90, &[u16::from(b'a')]);
        record[56..58].copy_from_slice(&1_u16.to_le_bytes());
        odd_name.extend(record);
        assert!(parse_page(&odd_name).is_err());
    }

    #[test]
    fn all_relevant_reason_groups_are_known_and_close_is_excluded() {
        assert_eq!(KNOWN_RELEVANT_REASONS & USN_REASON_CLOSE, 0);
        assert_ne!(DATA_CHANGE_REASONS & USN_REASON_DATA_OVERWRITE, 0);
        assert_ne!(CREATE_DELETE_REASONS & USN_REASON_FILE_DELETE, 0);
        assert_ne!(RENAME_REASONS & USN_REASON_RENAME_OLD_NAME, 0);
        assert_ne!(METADATA_CHANGE_REASONS & USN_REASON_REPARSE_POINT_CHANGE, 0);
    }

    #[test]
    fn impact_summary_counts_reason_bits_and_directory_hint() {
        let mut summary = FilesystemChangeImpactSummary::default();
        let event = UsnEvent {
            file_id: 25,
            parent_id: 5,
            usn: 90,
            reason: USN_REASON_DATA_EXTEND
                | USN_REASON_FILE_CREATE
                | USN_REASON_RENAME_NEW_NAME
                | USN_REASON_REPARSE_POINT_CHANGE
                | 0x0200_0000,
            attributes: FILE_ATTRIBUTE_DIRECTORY,
            name_bytes: &[],
        };

        observe_impact_reason(&mut summary, &event).expect("valid reason bits should aggregate");

        assert_eq!(summary.record_count, 1);
        assert_eq!(summary.data_change_records, 1);
        assert_eq!(summary.create_delete_records, 1);
        assert_eq!(summary.rename_records, 1);
        assert_eq!(summary.metadata_change_records, 1);
        assert_eq!(summary.other_records, 1);
        assert_eq!(summary.directory_records, 1);
    }

    #[test]
    fn dirty_directories_deduplicate_identity_and_compress_descendants() {
        let mut dirty = DirtyDirectorySet::default();
        for path in [
            PathBuf::from(r"R:\Users\sample-user\Downloads\child"),
            PathBuf::from(r"\\?\r:\users\SAMPLE-USER\downloads"),
            PathBuf::from(r"R:\Users\sample-user\Downloads\sibling"),
            PathBuf::from(r"R:\Users\sample-user\Downloads-archive"),
            PathBuf::from(r"R:\Users\sample-user\Documents"),
            PathBuf::from(r"R:\Users\sample-user\Documents"),
        ] {
            dirty
                .insert(path)
                .expect("test directories should stay below resource limits");
        }

        let compressed = dirty.finish();
        assert_eq!(compressed.len(), 3);
        assert_eq!(
            compressed
                .iter()
                .map(|path| windows_path_identity(path))
                .collect::<Vec<_>>(),
            vec![
                r"r:\users\sample-user\documents"
                    .encode_utf16()
                    .collect::<Vec<_>>(),
                r"r:\users\sample-user\downloads"
                    .encode_utf16()
                    .collect::<Vec<_>>(),
                r"r:\users\sample-user\downloads-archive"
                    .encode_utf16()
                    .collect::<Vec<_>>()
            ]
        );
    }

    #[test]
    fn dirty_directory_identity_preserves_non_unicode_utf16() {
        let path = |tail| {
            let mut units = r"R:\non-unicode-".encode_utf16().collect::<Vec<_>>();
            units.push(tail);
            PathBuf::from(OsString::from_wide(&units))
        };
        let first = path(0xd800);
        let second = path(0xd801);
        assert_ne!(
            windows_path_identity(&first),
            windows_path_identity(&second)
        );

        let mut dirty = DirtyDirectorySet::default();
        dirty
            .insert(first)
            .expect("the first lossless path should enter the set");
        dirty
            .insert(second)
            .expect("the second lossless path should enter the set");
        assert_eq!(dirty.finish().len(), 2);
    }

    #[test]
    fn ntfs_internal_reference_and_path_boundary_are_explicit() {
        assert!(is_ntfs_internal_reference(5));
        assert!(is_ntfs_internal_reference((7_u64 << 48) | 23));
        assert!(!is_ntfs_internal_reference((7_u64 << 48) | 24));
        assert!(same_file_reference(5, (9_u64 << 48) | 5));
        let candidates = HashSet::from([r"r:\users\sample-user".encode_utf16().collect()]);
        assert!(identity_has_ancestor(
            &r"r:\users\sample-user\downloads"
                .encode_utf16()
                .collect::<Vec<_>>(),
            &candidates
        ));
        assert!(!identity_has_ancestor(
            &r"r:\users\sample-user-old"
                .encode_utf16()
                .collect::<Vec<_>>(),
            &candidates
        ));
        assert!(impact_path_is_same_or_child(
            Path::new(r"\\?\R:\Users\sample-user\Downloads"),
            Path::new(r"r:\users\SAMPLE-USER")
        ));
    }

    #[test]
    #[ignore = "requires a real NTFS volume, read-only volume access, and a USN journal"]
    fn real_usn_validation_distinguishes_root_changes() {
        let _lock = real_usn_test_lock();
        let fixture =
            std::env::temp_dir().join(format!("mangodisk-usn-validation-{}", std::process::id()));
        let root = fixture.join("root");
        let outside = fixture.join("outside");
        fs::create_dir_all(&root).expect("the in-root USN fixture should be created");
        fs::create_dir_all(&outside).expect("the outside-root USN fixture should be created");

        let outside_token = capture_token(&root)
            .expect("capturing the USN token should succeed")
            .expect("the test process should have read-only volume access");
        fs::write(outside.join("unrelated.tmp"), b"outside")
            .expect("the outside-root fixture should be writable");
        // USN records are normally readable as soon as handles close. The short delay only reduces
        // scheduling noise from the VM filesystem.
        std::thread::sleep(Duration::from_millis(20));
        let outside_status = start_monitor(&root, &outside_token, &|| false)
            .expect("validating an outside-root change should succeed")
            .expect("Windows should return a one-shot validation status")
            .status();
        println!("windows_usn_outside_status={outside_status:?}");
        assert_eq!(outside_status, FilesystemChangeStatus::Clean);

        let inside_token = capture_token(&root)
            .expect("capturing the USN token should succeed")
            .expect("the test process should have read-only volume access");
        fs::write(root.join("related.tmp"), b"inside")
            .expect("the in-root fixture should be writable");
        std::thread::sleep(Duration::from_millis(20));
        let inside_status = start_monitor(&root, &inside_token, &|| false)
            .expect("validating an in-root change should succeed")
            .expect("Windows should return a one-shot validation status")
            .status();
        println!("windows_usn_inside_status={inside_status:?}");
        assert_eq!(inside_status, FilesystemChangeStatus::Changed);

        fs::remove_dir_all(fixture).expect("the USN fixture should be removed");
    }

    #[test]
    #[ignore = "requires a real NTFS volume, read-only volume access, and a USN journal"]
    fn real_usn_validation_fails_closed_for_identity_and_observes_delete() {
        let _lock = real_usn_test_lock();
        let fixture =
            std::env::temp_dir().join(format!("mangodisk-usn-fail-closed-{}", std::process::id()));
        let root = fixture.join("root");
        let child = root.join("child");
        fs::create_dir_all(&child).expect("the deletion fixture should be created");
        fs::write(child.join("sample.tmp"), b"sample")
            .expect("the deletion fixture should be writable");

        let token = capture_token(&root)
            .expect("capturing the USN token should succeed")
            .expect("the test process should have read-only volume access");

        let mut changed_journal = token;
        changed_journal.history_id ^= 1;
        let history_status = start_monitor(&root, &changed_journal, &|| false)
            .expect("a journal mismatch should return a status")
            .expect("Windows should return a one-shot validation status")
            .status();
        assert_eq!(history_status, FilesystemChangeStatus::HistoryUnavailable);

        let mut changed_volume = token;
        changed_volume.volume_id[0] ^= 1;
        let volume_status = start_monitor(&root, &changed_volume, &|| false)
            .expect("a volume mismatch should return a status")
            .expect("Windows should return a one-shot validation status")
            .status();
        assert_eq!(volume_status, FilesystemChangeStatus::HistoryUnavailable);

        let mut stale_cursor = token;
        stale_cursor.cursor = 0;
        let stale_status = start_monitor(&root, &stale_cursor, &|| false)
            .expect("a stale cursor should return a status")
            .expect("Windows should return a one-shot validation status")
            .status();
        assert_eq!(stale_status, FilesystemChangeStatus::HistoryUnavailable);

        let mut future_cursor = token;
        future_cursor.cursor = u64::MAX;
        let future_status = start_monitor(&root, &future_cursor, &|| false)
            .expect("a future cursor should return a status")
            .expect("Windows should return a one-shot validation status")
            .status();
        assert_eq!(future_status, FilesystemChangeStatus::HistoryUnavailable);
        let cancellation_error = match start_monitor(&root, &token, &|| true) {
            Err(error) => error,
            Ok(_) => panic!("pre-cancellation must propagate"),
        };
        assert_eq!(cancellation_error, "scan cancelled");

        fs::remove_dir_all(&child).expect("the in-root directory tree should be removed");
        std::thread::sleep(Duration::from_millis(20));
        let delete_status = start_monitor(&root, &token, &|| false)
            .expect("validating deletion should succeed")
            .expect("Windows should return a one-shot validation status")
            .status();
        println!("windows_usn_delete_status={delete_status:?}");
        assert!(matches!(
            delete_status,
            FilesystemChangeStatus::Changed | FilesystemChangeStatus::HistoryUnavailable
        ));
        fs::remove_dir_all(fixture).expect("the deletion fixture should be removed");
    }

    #[test]
    #[ignore = "requires a real NTFS volume, read-only volume access, and a USN journal"]
    fn real_usn_validation_observes_modify_rename_and_cross_root_move() {
        let _lock = real_usn_test_lock();
        let fixture =
            std::env::temp_dir().join(format!("mangodisk-usn-mutations-{}", std::process::id()));
        let root = fixture.join("root");
        let outside = fixture.join("outside");
        fs::create_dir_all(&root).expect("the changed root fixture should be created");
        fs::create_dir_all(&outside).expect("the outside-root fixture should be created");
        let file = root.join("sample.tmp");
        fs::write(&file, b"before").expect("the modification fixture should be created");

        let modified_token = capture_token(&root)
            .expect("capturing the USN token should succeed")
            .expect("the test process should have read-only volume access");
        fs::write(&file, b"after").expect("the in-root file should be modified");
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            start_monitor(&root, &modified_token, &|| false)
                .expect("validating modification should succeed")
                .expect("Windows should return a one-shot validation status")
                .status(),
            FilesystemChangeStatus::Changed
        );

        let renamed_token = capture_token(&root)
            .expect("capturing the USN token should succeed")
            .expect("the test process should have read-only volume access");
        let renamed = root.join("renamed.tmp");
        fs::rename(&file, &renamed).expect("the in-root file should be renamed");
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            start_monitor(&root, &renamed_token, &|| false)
                .expect("validating a rename should succeed")
                .expect("Windows should return a one-shot validation status")
                .status(),
            FilesystemChangeStatus::Changed
        );

        let moved_token = capture_token(&root)
            .expect("capturing the USN token should succeed")
            .expect("the test process should have read-only volume access");
        fs::rename(&renamed, outside.join("moved.tmp"))
            .expect("the file should move outside the scan root");
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            start_monitor(&root, &moved_token, &|| false)
                .expect("validating a cross-root move should succeed")
                .expect("Windows should return a one-shot validation status")
                .status(),
            FilesystemChangeStatus::Changed
        );

        let removed_root_token = capture_token(&root)
            .expect("capturing the USN token should succeed")
            .expect("the test process should have read-only volume access");
        fs::remove_dir_all(&root).expect("the scan root should be removed");
        std::thread::sleep(Duration::from_millis(20));
        let removed_root_status = start_monitor(&root, &removed_root_token, &|| false)
            .expect("root deletion should return a status")
            .expect("Windows should return a one-shot validation status")
            .status();
        assert_eq!(
            removed_root_status,
            FilesystemChangeStatus::HistoryUnavailable
        );
        fs::remove_dir_all(fixture).expect("the change fixture should be removed");
    }

    #[test]
    #[ignore = "requires a real NTFS volume, read-only volume access, and a USN journal"]
    fn real_usn_clean_validation_latency_samples() {
        let _lock = real_usn_test_lock();
        let root = std::env::temp_dir();
        let mut samples = Vec::with_capacity(25);
        for _ in 0..25 {
            let token = capture_token(&root)
                .expect("capturing the USN token should succeed")
                .expect("the test process should have read-only volume access");
            let started = Instant::now();
            let status = start_monitor(&root, &token, &|| false)
                .expect("validating empty history should succeed")
                .expect("Windows should return a one-shot validation status")
                .status();
            assert_eq!(status, FilesystemChangeStatus::Clean);
            samples.push(started.elapsed().as_micros());
        }
        samples.sort_unstable();
        println!(
            "windows_usn_clean_latency_us p50={} p95={} max={}",
            samples[samples.len() / 2],
            samples[samples.len() * 95 / 100],
            samples[samples.len() - 1]
        );
    }
}
