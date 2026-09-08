mod allocated_content;
mod change_tracking;
mod directories;
mod directory_aggregate;
mod directory_entry;
mod directory_identity;
mod disk_cleanup;
mod file_layout;
mod file_space;
mod inventory;
mod large_files;
mod native_io;
mod native_uninstall;
mod package_evidence;
mod package_locations;
mod package_reconciliation;
mod package_sources;
mod path_identity;
mod privacy;
mod process_control;
mod project_markers;
mod shortcut_overlay;
mod startup;
mod system_maintenance;
mod system_settings;
mod volumes;

use std::{
    ffi::OsStr,
    fs,
    os::windows::ffi::OsStrExt,
    os::windows::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

use crate::{
    ApplicationDirectories, ApplicationProcessCloseMode, ApplicationProcessCloseResult,
    ApplicationProcessTarget, ApplicationUninstallExecutionOutcome,
    ApplicationUninstallPlatformError, ApplicationUninstallRegistration,
    ApplicationUninstallRegistrationState, DirectPhysicalDirectoryEnumeration,
    DirectoryEntryIdentities, DirectoryTreeAggregate, DirectoryTreeAggregateError,
    FastAnalysisQuery, FastAnalysisRecord, FastAnalysisScanError, FastAnalysisSummary,
    FileSpaceUsage, FilesystemChangeImpactError, FilesystemChangeImpactOutcome,
    FilesystemChangeMonitor, FilesystemChangeToken, LargeFileCandidateScanError,
    LargeFileCandidateSummary, Platform, PlatformCancellation, PlatformError, PlatformErrorCode,
    PlatformResult, PlatformSystemSettingChangeRequest, PlatformSystemSettingChangeResult,
    PlatformSystemSettingState, PrivacyPlatform, ProjectMarkerCandidateProgress,
    ProjectMarkerCandidateQuery, ProjectMarkerCandidateScanError, ProjectMarkerCandidateSummary,
    RunningProcessIdentity, ScanPurpose, SkipReason, StartupPlatform, SystemInventory,
    SystemMaintenancePlatform, SystemSettingsPlatform, UserDirectories, VolumeInfo,
    WindowsDiskCleanupEstimate, WindowsDiskCleanupExecution, WindowsDiskCleanupKind,
};

pub struct WindowsPlatform;

impl PrivacyPlatform for WindowsPlatform {
    fn discover_privacy_sources(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<crate::PlatformPrivacyDiscovery> {
        privacy::discover(cancellation)
    }

    fn clear_system_privacy_trace(
        &self,
        trace: crate::PlatformPrivacySystemTraceKind,
    ) -> PlatformResult<bool> {
        privacy::clear(trace)
    }

    fn system_privacy_trace_revision(
        &self,
        trace: crate::PlatformPrivacySystemTraceKind,
    ) -> PlatformResult<Option<String>> {
        privacy::system_revision(trace)
    }

    fn clear_application_privacy_trace(
        &self,
        trace: crate::PlatformPrivacyApplicationNativeTraceKind,
    ) -> PlatformResult<bool> {
        privacy::clear_application_trace(trace)
    }

    fn system_privacy_trace_details(
        &self,
        trace: crate::PlatformPrivacySystemTraceKind,
        offset: u64,
        limit: u32,
    ) -> PlatformResult<Vec<crate::PlatformPrivacyDetailEntry>> {
        privacy::system_details(trace, offset, limit)
    }

    fn application_privacy_trace_details(
        &self,
        trace: crate::PlatformPrivacyApplicationNativeTraceKind,
        offset: u64,
        limit: u32,
    ) -> PlatformResult<Vec<crate::PlatformPrivacyDetailEntry>> {
        privacy::application_details(trace, offset, limit)
    }
}

impl StartupPlatform for WindowsPlatform {
    fn scan_startup_sources(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<crate::PlatformStartupSourceResult>> {
        startup::scan(cancellation)
    }

    fn change_startup_item(
        &self,
        request: &crate::PlatformStartupChangeRequest,
        _authorization_prompt: Option<&str>,
    ) -> PlatformResult<crate::PlatformStartupChangeResult> {
        startup::change(request)
    }

    fn change_startup_items(
        &self,
        requests: &[crate::PlatformStartupChangeRequest],
        _authorization_prompt: Option<&str>,
    ) -> PlatformResult<Vec<PlatformResult<crate::PlatformStartupChangeResult>>> {
        startup::change_many(requests)
    }
}

impl SystemSettingsPlatform for WindowsPlatform {
    fn scan_system_settings(
        &self,
        setting_ids: &[&str],
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<PlatformSystemSettingState>> {
        system_settings::scan(setting_ids, cancellation)
    }

    fn change_system_setting(
        &self,
        request: &PlatformSystemSettingChangeRequest,
    ) -> PlatformResult<PlatformSystemSettingChangeResult> {
        system_settings::change(request)
    }

    fn change_system_settings(
        &self,
        requests: &[PlatformSystemSettingChangeRequest],
    ) -> PlatformResult<Vec<PlatformResult<PlatformSystemSettingChangeResult>>> {
        system_settings::change_many(requests)
    }
}

impl SystemMaintenancePlatform for WindowsPlatform {
    fn scan_system_maintenance(
        &self,
        task_ids: &[&str],
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<crate::PlatformSystemMaintenanceState>> {
        system_maintenance::scan(task_ids, cancellation)
    }

    fn execute_system_maintenance(
        &self,
        task_id: &str,
        cancellation: &PlatformCancellation,
        authorization_prompt: Option<&str>,
        progress: &crate::PlatformSystemMaintenanceProgressSink,
    ) -> PlatformResult<crate::PlatformSystemMaintenanceExecution> {
        system_maintenance::execute(task_id, cancellation, authorization_prompt, progress)
    }
}

pub(crate) fn system_settings_helper_change_many(
    requests: &[PlatformSystemSettingChangeRequest],
) -> Vec<PlatformResult<PlatformSystemSettingChangeResult>> {
    system_settings::helper_change_many(requests)
}

pub(crate) fn system_maintenance_helper_execute(
    task_id: &str,
    progress: &crate::PlatformSystemMaintenanceProgressSink,
) -> crate::system_maintenance_helper::PrivilegedMaintenanceResult {
    system_maintenance::execute_with_current_privileges(task_id, progress)
}

pub(crate) fn disk_cleanup_helper_is_elevated() -> PlatformResult<bool> {
    disk_cleanup::current_process_is_elevated().map_err(|error_code| {
        PlatformError::operation_failed(format!(
            "read disk cleanup helper elevation state failed with Windows error {error_code}"
        ))
    })
}

pub(crate) fn startup_helper_change_many(
    requests: &[crate::startup_helper::StartupHelperChangeRequest],
) -> Vec<PlatformResult<crate::PlatformStartupChangeResult>> {
    startup::helper_change_many(requests)
}

const FILE_ATTRIBUTE_REPARSE_POINT_VALUE: u32 = 0x0000_0400;
const FILE_ATTRIBUTE_OFFLINE_VALUE: u32 = 0x0000_1000;
const FILE_ATTRIBUTE_RECALL_ON_OPEN_VALUE: u32 = 0x0004_0000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS_VALUE: u32 = 0x0040_0000;

/// Returns whether opening an entry can recall content from remote or offline storage.
///
/// Directory enumeration already returns these bits with the ordinary file attributes. Keeping
/// the check as a bit mask adds no filesystem call and protects providers that expose recall
/// semantics without a reparse-point bit.
pub(crate) fn is_remote_placeholder_attributes(attributes: u32) -> bool {
    attributes
        & (FILE_ATTRIBUTE_OFFLINE_VALUE
            | FILE_ATTRIBUTE_RECALL_ON_OPEN_VALUE
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS_VALUE)
        != 0
}

pub(crate) fn application_directories(identifier: &str) -> PlatformResult<ApplicationDirectories> {
    directories::application_directories(identifier)
}

pub fn windows_disk_cleanup_estimates(
    kinds: &[WindowsDiskCleanupKind],
    cancellation: &PlatformCancellation,
) -> Vec<WindowsDiskCleanupEstimate> {
    disk_cleanup::estimates(kinds, cancellation, true)
}

/// Re-measures native handlers without using the short-lived preview cache.
///
/// Cleanup preflight calls this immediately before reporting a dry-run result,
/// so the expected byte count describes the current system state.
pub fn fresh_windows_disk_cleanup_estimates(
    kinds: &[WindowsDiskCleanupKind],
    cancellation: &PlatformCancellation,
) -> Vec<WindowsDiskCleanupEstimate> {
    disk_cleanup::estimates(kinds, cancellation, false)
}

pub fn execute_windows_disk_cleanup(
    kind: WindowsDiskCleanupKind,
    cancellation: &PlatformCancellation,
) -> WindowsDiskCleanupExecution {
    disk_cleanup::execute(kind, cancellation)
}

pub fn estimate_windows_previous_installations_with_privileges(
) -> PlatformResult<WindowsDiskCleanupEstimate> {
    crate::disk_cleanup_helper::estimate_previous_installations_with_privileges()
}

pub fn execute_windows_previous_installations_with_privileges(
    cancellation: &PlatformCancellation,
) -> PlatformResult<WindowsDiskCleanupExecution> {
    crate::disk_cleanup_helper::execute_previous_installations_with_privileges(cancellation)
}

impl Platform for WindowsPlatform {
    fn os_name(&self) -> &'static str {
        "windows"
    }

    fn system_volume_path(&self) -> PathBuf {
        volumes::system_drive_path()
    }

    fn system_volume(&self) -> PlatformResult<VolumeInfo> {
        volumes::system_volume().map_err(Into::into)
    }

    fn volumes(&self) -> PlatformResult<Vec<VolumeInfo>> {
        volumes::volumes().map_err(Into::into)
    }

    fn user_directories(&self) -> PlatformResult<UserDirectories> {
        directories::user_directories()
    }

    fn system_inventory(&self) -> PlatformResult<SystemInventory> {
        inventory::system_inventory(&PlatformCancellation::new(|| false)).map_err(Into::into)
    }

    fn system_inventory_with_cancellation(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<SystemInventory> {
        inventory::system_inventory(cancellation).map_err(Into::into)
    }

    fn system_inventory_revision(&self) -> PlatformResult<String> {
        inventory::system_inventory_revision().map_err(Into::into)
    }

    fn system_inventory_revision_with_cancellation(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<String> {
        inventory::system_inventory_revision_with_cancellation(cancellation).map_err(Into::into)
    }

    fn application_uninstall_registration_state(
        &self,
        registration: &ApplicationUninstallRegistration,
    ) -> Result<ApplicationUninstallRegistrationState, ApplicationUninstallPlatformError> {
        native_uninstall::registration_state(registration)
    }

    fn execute_application_uninstall_registration(
        &self,
        registration: &ApplicationUninstallRegistration,
    ) -> Result<ApplicationUninstallExecutionOutcome, ApplicationUninstallPlatformError> {
        native_uninstall::execute_registration(registration)
    }

    fn running_process_names(&self) -> PlatformResult<Vec<String>> {
        inventory::running_process_names(&PlatformCancellation::new(|| false)).map_err(Into::into)
    }

    fn running_process_names_with_cancellation(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<String>> {
        inventory::running_process_names(cancellation).map_err(Into::into)
    }

    fn running_process_identities_with_cancellation(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<RunningProcessIdentity>> {
        process_control::running_process_identities(cancellation)
    }

    fn close_application_processes(
        &self,
        target: &ApplicationProcessTarget,
        mode: ApplicationProcessCloseMode,
    ) -> PlatformResult<ApplicationProcessCloseResult> {
        process_control::close(target, mode)
    }

    fn close_application_processes_many(
        &self,
        targets: &[ApplicationProcessTarget],
        mode: ApplicationProcessCloseMode,
    ) -> Vec<PlatformResult<ApplicationProcessCloseResult>> {
        process_control::close_many(targets, mode)
    }

    fn resolve_directory_entry(&self, path: &Path) -> PlatformResult<PathBuf> {
        directory_entry::resolve(path)
    }

    fn is_link_like(&self, metadata: &fs::Metadata) -> bool {
        let attributes = metadata.file_attributes();
        metadata.file_type().is_symlink()
            || attributes & FILE_ATTRIBUTE_REPARSE_POINT_VALUE != 0
            || is_remote_placeholder_attributes(attributes)
    }

    fn file_space_usage(&self, path: &Path, metadata: &fs::Metadata) -> FileSpaceUsage {
        file_space::usage(path, metadata)
    }

    fn file_has_allocated_content(
        &self,
        file: &fs::File,
        logical_bytes: u64,
    ) -> PlatformResult<Option<bool>> {
        allocated_content::has_allocated_content(file, logical_bytes)
    }

    fn directory_entry_identities(
        &self,
        directory: &Path,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Option<DirectoryEntryIdentities>> {
        directory_identity::read(directory, cancellation)
            .map(Some)
            .map_err(|error| {
                if cancellation.is_cancelled() {
                    PlatformError::new(
                        PlatformErrorCode::UserCancelled,
                        "directory identity enumeration cancelled",
                    )
                } else {
                    PlatformError::io("read directory entry identities", &error)
                }
            })
    }

    fn display_path(&self, path: &Path) -> String {
        path_identity::display(path)
    }

    fn path_identity_key(&self, path: &Path) -> String {
        path_identity::comparison_key(path)
    }

    fn paths_equal(&self, left: &Path, right: &Path) -> bool {
        path_identity::equal(left, right)
    }

    fn path_is_same_or_child(&self, path: &Path, root: &Path) -> bool {
        path_identity::is_same_or_child(path, root)
    }

    fn relative_path(&self, path: &Path, root: &Path) -> Option<PathBuf> {
        if path_identity::equal(path, root) {
            return Some(PathBuf::new());
        }
        path_identity::relative_child_path(path, root)
    }

    fn should_skip(
        &self,
        path: &Path,
        scan_root: &Path,
        purpose: ScanPurpose,
    ) -> Option<SkipReason> {
        if purpose == ScanPurpose::Cleanup {
            return None;
        }
        let mut components = path.components().filter_map(|component| match component {
            Component::Normal(value) => Some(value),
            _ => None,
        });
        let first = components.next();
        let second = components.next();

        // This runs in the full-volume traversal hot path. Comparing Windows
        // UTF-16 components directly avoids allocating a lowercase String and
        // formatting temporary values for every visited entry.
        let first_is = |value| first.is_some_and(|part| os_str_eq_ascii_case(part, value));
        let second_is = |value| second.is_some_and(|part| os_str_eq_ascii_case(part, value));
        if matches!(
            purpose,
            ScanPurpose::LargeFiles | ScanPurpose::DuplicateFiles
        ) && second.is_none()
            && [
                "hiberfil.sys",
                "pagefile.sys",
                "swapfile.sys",
                "dumpstack.log.tmp",
            ]
            .into_iter()
            .any(first_is)
        {
            // These files are owned by Windows memory and power management.
            // They can be changed only through the corresponding system
            // settings and must never be presented as ordinary deletable
            // files, even when they dominate a volume's free space.
            return Some(SkipReason::SystemCritical);
        }
        if purpose == ScanPurpose::DuplicateFiles
            && [
                "$windows.~bt",
                "$windows.~ws",
                "windows.old",
                "config.msi",
                "msocache",
                "perflogs",
            ]
            .into_iter()
            .any(first_is)
        {
            // Upgrade, installer, and diagnostic staging trees are managed as complete units by
            // Windows or Deep Cleanup. Presenting byte-equal members as independently removable
            // duplicates can leave an update or rollback image inconsistent.
            return Some(SkipReason::SystemCritical);
        }
        if first_is("system volume information")
            || first_is("$recycle.bin")
            || first_is("recovery")
            || first_is("$winreagent")
            || (first_is("windows")
                && (second_is("winsxs") || second_is("installer") || second_is("system32")))
        {
            return Some(SkipReason::SystemCritical);
        }
        if matches!(
            purpose,
            ScanPurpose::LargeFiles | ScanPurpose::DuplicateFiles
        ) && (first_is("windows")
            || first_is("program files")
            || first_is("program files (x86)")
            || first_is("programdata"))
        {
            return Some(SkipReason::SystemCritical);
        }
        if purpose == ScanPurpose::DuplicateFiles
            && is_transient_duplicate_scope(path)
            && !is_transient_duplicate_scope(scan_root)
        {
            return Some(SkipReason::SystemCritical);
        }
        None
    }

    fn validate_cleanup_root(&self, path: &Path) -> PlatformResult<()> {
        let canonical = fs::canonicalize(path)
            .map_err(|error| PlatformError::io("canonicalize path", &error))?;
        if canonical.parent().is_none()
            || path_identity::equal(&canonical, &self.system_volume_path())
        {
            return Err(PlatformError::invalid_path("cleanup root is a volume root"));
        }
        let system_directory = directories::system_directory()?;
        let program_files_directories = directories::program_files_directories()?;
        if path_identity::is_same_or_child(&canonical, &system_directory)
            || program_files_directories
                .iter()
                .any(|root| path_identity::is_same_or_child(&canonical, root))
        {
            return Err(PlatformError::invalid_path(
                "cleanup root is system protected",
            ));
        }
        let user_directories = self.user_directories()?;
        for name in [
            "Desktop",
            "Documents",
            "Downloads",
            "Pictures",
            "Videos",
            "Music",
            "OneDrive",
            "Saved Games",
        ] {
            if path_identity::is_same_or_child(
                &canonical,
                &user_directories.home_directory().join(name),
            ) {
                return Err(PlatformError::invalid_path(
                    "cleanup root contains user content",
                ));
            }
        }
        Ok(())
    }

    fn validate_user_selected_cleanup_root(&self, path: &Path) -> PlatformResult<()> {
        let canonical = fs::canonicalize(path)
            .map_err(|error| PlatformError::io("canonicalize path", &error))?;
        if canonical.parent().is_none()
            || path_identity::equal(&canonical, &self.system_volume_path())
        {
            return Err(PlatformError::invalid_path("cleanup root is a volume root"));
        }
        let system_directory = directories::system_directory()?;
        let program_files_directories = directories::program_files_directories()?;
        if path_identity::is_same_or_child(&canonical, &system_directory)
            || program_files_directories
                .iter()
                .any(|root| path_identity::is_same_or_child(&canonical, root))
        {
            return Err(PlatformError::invalid_path(
                "cleanup root is system protected",
            ));
        }
        let per_user_programs = self
            .user_directories()?
            .home_directory()
            .join("AppData/Local/Programs");
        if path_identity::is_same_or_child(&canonical, &per_user_programs) {
            return Err(PlatformError::invalid_path(
                "cleanup root is an application installation directory",
            ));
        }
        Ok(())
    }

    fn fast_directory_tree_aggregate(
        &self,
        root: &Path,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        report_progress: &(dyn Fn(&Path, u64, u64) + Sync),
    ) -> Result<Option<DirectoryTreeAggregate>, DirectoryTreeAggregateError> {
        directory_aggregate::measure(root, false, is_cancelled, report_progress).map(Some)
    }

    fn fast_project_artifact_tree_aggregate(
        &self,
        root: &Path,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        report_progress: &(dyn Fn(&Path, u64, u64) + Sync),
    ) -> Result<Option<DirectoryTreeAggregate>, DirectoryTreeAggregateError> {
        directory_aggregate::measure(root, true, is_cancelled, report_progress).map(Some)
    }

    fn fast_direct_physical_directories(
        &self,
        root: &Path,
        maximum_entries: usize,
        is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<DirectPhysicalDirectoryEnumeration>, DirectoryTreeAggregateError> {
        directory_aggregate::direct_physical_directories(root, maximum_entries, is_cancelled)
            .map(Some)
    }

    fn capture_filesystem_change_token(
        &self,
        root: &Path,
    ) -> PlatformResult<Option<FilesystemChangeToken>> {
        change_tracking::capture_token(root).map_err(Into::into)
    }

    fn start_filesystem_change_monitor(
        &self,
        root: &Path,
        token: &FilesystemChangeToken,
        is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> PlatformResult<Option<FilesystemChangeMonitor>> {
        change_tracking::start_monitor(root, token, is_cancelled).map_err(Into::into)
    }

    fn filesystem_change_impact_plan(
        &self,
        root: &Path,
        token: &FilesystemChangeToken,
        is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<FilesystemChangeImpactOutcome>, FilesystemChangeImpactError> {
        change_tracking::impact_plan(self, root, token, is_cancelled).map(Some)
    }

    fn fast_large_file_candidates(
        &self,
        root: &Path,
        minimum_bytes: u64,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        consumer: &mut dyn FnMut(PathBuf) -> Result<(), String>,
    ) -> Result<Option<LargeFileCandidateSummary>, LargeFileCandidateScanError> {
        large_files::find_candidates(self, root, minimum_bytes, is_cancelled, consumer).map(Some)
    }

    fn fast_large_file_candidates_are_complete(&self) -> bool {
        true
    }

    fn fast_project_marker_candidates(
        &self,
        query: ProjectMarkerCandidateQuery<'_>,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        report_progress: &(dyn Fn(ProjectMarkerCandidateProgress) + Sync),
        consumer: &mut dyn FnMut(PathBuf) -> Result<(), String>,
    ) -> Result<Option<ProjectMarkerCandidateSummary>, ProjectMarkerCandidateScanError> {
        project_markers::scan(
            project_markers::ProjectMarkerScanRequest {
                root: query.root,
                file_names: query.file_names,
                file_suffixes: query.file_suffixes,
                pruned_directory_names: query.pruned_directory_names,
                maximum_depth: query.maximum_depth,
                is_cancelled,
                report_progress,
            },
            consumer,
        )
        .map(Some)
    }

    fn fast_analysis_records(
        &self,
        query: FastAnalysisQuery<'_>,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        report_progress: &mut dyn FnMut(&Path, u64, u64),
        consumer: &mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
    ) -> Result<Option<FastAnalysisSummary>, FastAnalysisScanError> {
        file_layout::analyze_records(
            self,
            file_layout::AnalysisScanRequest {
                root: query.root,
                purpose: query.purpose,
                large_file_minimum_bytes: query.large_file_minimum_bytes,
                is_cancelled,
                should_prune_directory: query.should_prune_directory,
                report_progress,
            },
            consumer,
        )
    }
}

fn os_str_eq_ascii_case(value: &OsStr, expected: &str) -> bool {
    let mut value = value.encode_wide();
    let mut expected = expected.encode_utf16();
    loop {
        match (value.next(), expected.next()) {
            (Some(left), Some(right)) if ascii_lowercase(left) == ascii_lowercase(right) => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

fn ascii_lowercase(value: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&value) {
        value + u16::from(b'a' - b'A')
    } else {
        value
    }
}

/// Returns whether a Windows path belongs to per-user application state.
///
/// Broad duplicate scans skip AppData because it contains app-managed caches, package state,
/// databases, and support files whose byte-identical members are not independent user copies.
/// The caller compares both the candidate and scan root so an explicit request targeting AppData
/// remains inspectable.
fn is_transient_duplicate_scope(path: &Path) -> bool {
    let mut components = path.components().filter_map(|component| match component {
        Component::Normal(value) => Some(value),
        _ => None,
    });
    let values = [
        components.next(),
        components.next(),
        components.next(),
        components.next(),
        components.next(),
        components.next(),
        components.next(),
    ];
    let is = |index: usize, expected: &str| {
        values[index].is_some_and(|value| os_str_eq_ascii_case(value, expected))
    };

    is(0, "temp") || (is(0, "users") && is(2, "appdata"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_storage_attributes_fail_closed_without_reparse_points() {
        for attributes in [
            FILE_ATTRIBUTE_OFFLINE_VALUE,
            FILE_ATTRIBUTE_RECALL_ON_OPEN_VALUE,
            FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS_VALUE,
        ] {
            assert!(is_remote_placeholder_attributes(attributes));
        }
        assert!(!is_remote_placeholder_attributes(0));
        assert!(!is_remote_placeholder_attributes(
            FILE_ATTRIBUTE_REPARSE_POINT_VALUE
        ));
    }

    #[test]
    fn windows_path_identity_ignores_verbatim_prefix_and_casing() {
        let platform = WindowsPlatform;
        assert!(platform.path_is_same_or_child(
            Path::new(r"\\?\C:\Windows\System32"),
            Path::new(r"c:\WINDOWS")
        ));
        assert!(
            !platform.path_is_same_or_child(Path::new(r"C:\Windows.old"), Path::new(r"C:\Windows"))
        );
        assert_eq!(
            platform.relative_path(
                Path::new(r"\\?\C:\Users\Developer\Cache\File.tmp"),
                Path::new(r"c:/users/developer")
            ),
            Some(PathBuf::from(r"Cache\File.tmp"))
        );
    }

    #[test]
    fn actual_windows_system_root_is_never_a_cleanup_root() {
        let platform = WindowsPlatform;
        let system_root =
            directories::system_directory().expect("Windows system directory should be available");

        assert!(platform.validate_cleanup_root(&system_root).is_err());
    }

    #[test]
    fn removable_file_scans_exclude_windows_managed_volume_files() {
        let platform = WindowsPlatform;
        for path in [
            r"C:\hiberfil.sys",
            r"C:\pagefile.sys",
            r"D:\swapfile.sys",
            r"D:\DumpStack.log.tmp",
        ] {
            for purpose in [ScanPurpose::LargeFiles, ScanPurpose::DuplicateFiles] {
                assert_eq!(
                    platform.should_skip(Path::new(path), Path::new(r"C:\"), purpose),
                    Some(SkipReason::SystemCritical),
                    "{path} must not be exposed as an ordinary removable file"
                );
            }
        }
        assert_eq!(
            platform.should_skip(
                Path::new(r"C:\archives\pagefile.sys"),
                Path::new(r"C:\"),
                ScanPurpose::LargeFiles
            ),
            None,
            "a matching user file outside the volume root must not be hidden"
        );
    }

    #[test]
    fn broad_duplicate_scans_skip_windows_staging_and_temporary_data() {
        let platform = WindowsPlatform;
        for path in [
            r"C:\Windows.old\Windows\System32\example.dll",
            r"C:\$Windows.~BT\Sources\install.esd",
            r"C:\Config.Msi\rollback.rbf",
            r"C:\Users\MangoDiskUser\AppData\Local\Temp\archive.bin",
            r"C:\Users\MangoDiskUser\AppData\Local\Microsoft\Windows\INetCache\cache.dat",
            r"C:\Users\MangoDiskUser\AppData\Roaming\Example\managed-copy.bin",
        ] {
            assert_eq!(
                platform.should_skip(
                    Path::new(path),
                    Path::new(r"C:\"),
                    ScanPurpose::DuplicateFiles
                ),
                Some(SkipReason::SystemCritical),
                "{path} must not enter a broad duplicate scan"
            );
        }
    }

    #[test]
    fn explicit_duplicate_scans_allow_temporary_directories() {
        let platform = WindowsPlatform;
        assert_eq!(
            platform.should_skip(
                Path::new(r"C:\Users\MangoDiskUser\AppData\Local\Temp\archive.bin"),
                Path::new(r"C:\Users\MangoDiskUser\AppData\Local\Temp"),
                ScanPurpose::DuplicateFiles
            ),
            None
        );
        assert_eq!(
            platform.should_skip(
                Path::new(r"C:\Users\MangoDiskUser\AppData\Roaming\Example\managed-copy.bin"),
                Path::new(r"C:\Users\MangoDiskUser\AppData"),
                ScanPurpose::DuplicateFiles
            ),
            None
        );
        assert_eq!(
            platform.should_skip(
                Path::new(r"C:\Users\MangoDiskUser\Documents\Temp\report.pdf"),
                Path::new(r"C:\Users\MangoDiskUser"),
                ScanPurpose::DuplicateFiles
            ),
            None
        );
    }
}
