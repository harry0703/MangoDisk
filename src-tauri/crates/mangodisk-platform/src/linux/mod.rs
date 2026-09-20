mod ai_models;
mod directories;
mod inventory;
mod package_managers;
mod privacy;
mod process_control;
mod startup;
mod system_maintenance;
mod volumes;

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    ApplicationComponentAggregate, ApplicationComponentAggregateError, ApplicationDirectories,
    ApplicationProcessCloseMode, ApplicationProcessCloseResult, ApplicationProcessTarget,
    DirectPhysicalDirectoryEnumeration, DirectoryEntryIdentities, DirectoryTreeAggregate,
    DirectoryTreeAggregateError, FastAnalysisQuery, FastAnalysisRecord, FastAnalysisScanError,
    FastAnalysisSummary, FileSpaceUsage, Platform, PlatformCancellation, PlatformError,
    PlatformResult, ScanPurpose, SkipReason, SystemInventory, UserDirectories, VolumeInfo,
};

pub struct LinuxPlatform;

pub(crate) fn application_directories(identifier: &str) -> PlatformResult<ApplicationDirectories> {
    let home = dirs::home_dir()
        .ok_or_else(|| PlatformError::operation_failed("unable to determine home directory"))?;
    let local_data = dirs::data_local_dir().unwrap_or_else(|| home.join(".local/share"));
    let cache = dirs::cache_dir().unwrap_or_else(|| home.join(".cache"));
    Ok(ApplicationDirectories {
        local_data_directory: local_data.join(identifier),
        cache_directory: cache.join(identifier),
    })
}

impl Platform for LinuxPlatform {
    fn os_name(&self) -> &'static str {
        "linux"
    }

    fn system_volume_path(&self) -> PathBuf {
        PathBuf::from("/")
    }

    fn system_volume(&self) -> PlatformResult<VolumeInfo> {
        volumes::system_volume()
    }

    fn volumes(&self) -> PlatformResult<Vec<VolumeInfo>> {
        volumes::volumes()
    }

    fn user_directories(&self) -> PlatformResult<UserDirectories> {
        directories::user_directories()
    }

    fn system_inventory(&self) -> PlatformResult<SystemInventory> {
        inventory::system_inventory()
    }

    fn system_inventory_revision(&self) -> PlatformResult<String> {
        inventory::system_inventory_revision()
    }

    fn running_process_names(&self) -> PlatformResult<Vec<String>> {
        inventory::running_process_names()
    }

    fn close_application_processes(
        &self,
        _target: &ApplicationProcessTarget,
        _mode: ApplicationProcessCloseMode,
    ) -> PlatformResult<ApplicationProcessCloseResult> {
        process_control::close(_target, _mode)
    }

    fn close_application_processes_many(
        &self,
        targets: &[ApplicationProcessTarget],
        mode: ApplicationProcessCloseMode,
    ) -> Vec<PlatformResult<ApplicationProcessCloseResult>> {
        targets
            .iter()
            .map(|target| self.close_application_processes(target, mode))
            .collect()
    }

    fn is_link_like(&self, metadata: &fs::Metadata) -> bool {
        metadata.file_type().is_symlink()
    }

    fn is_same_filesystem(&self, root: &fs::Metadata, candidate: &fs::Metadata) -> bool {
        use std::os::unix::fs::MetadataExt;
        root.dev() == candidate.dev()
    }

    fn file_space_usage(&self, _path: &Path, metadata: &fs::Metadata) -> FileSpaceUsage {
        use std::os::unix::fs::MetadataExt;
        FileSpaceUsage {
            logical_bytes: metadata.len(),
            allocated_bytes: metadata.blocks().saturating_mul(512),
        }
    }

    fn file_has_allocated_content(
        &self,
        file: &fs::File,
        logical_bytes: u64,
    ) -> PlatformResult<Option<bool>> {
        use std::os::unix::io::AsRawFd;
        if logical_bytes == 0 {
            return Ok(Some(false));
        }
        let fd = file.as_raw_fd();
        let result = unsafe { libc::lseek(fd, 0, libc::SEEK_DATA) };
        if result >= 0 {
            return Ok(Some(true));
        }
        let errno = std::io::Error::last_os_error();
        let code = errno.raw_os_error().unwrap_or(0);
        match code {
            libc::ENXIO => Ok(Some(false)),
            libc::EINVAL | libc::ESPIPE => Ok(None),
            _ => Err(PlatformError::operation_failed(format!(
                "lseek SEEK_DATA failed: {errno}"
            ))),
        }
    }

    fn should_skip(
        &self,
        path: &Path,
        _scan_root: &Path,
        purpose: ScanPurpose,
    ) -> Option<SkipReason> {
        if purpose == ScanPurpose::Cleanup {
            return None;
        }
        if directories::is_system_critical(path) {
            return Some(SkipReason::SystemCritical);
        }
        if matches!(
            purpose,
            ScanPurpose::LargeFiles | ScanPurpose::DuplicateFiles
        ) {
            if directories::is_package_manager_owned(path) {
                return Some(SkipReason::SystemCritical);
            }
            if directories::is_unwritable_by_current_user(path) {
                return Some(SkipReason::PermissionDenied);
            }
        }
        None
    }

    fn validate_cleanup_root(&self, path: &Path) -> PlatformResult<()> {
        let canonical = fs::canonicalize(path)
            .map_err(|error| PlatformError::io("canonicalize path", &error))?;
        if canonical.parent().is_none() {
            return Err(PlatformError::invalid_path(
                "cleanup of a volume root is forbidden",
            ));
        }
        if directories::is_protected_cleanup_path(&canonical) {
            return Err(PlatformError::invalid_path(
                "cleanup of a protected Linux directory is forbidden",
            ));
        }
        Ok(())
    }

    fn directory_entry_identities(
        &self,
        directory: &Path,
        _cancellation: &PlatformCancellation,
    ) -> PlatformResult<Option<DirectoryEntryIdentities>> {
        use std::collections::HashMap;
        use std::os::unix::fs::MetadataExt;
        let mut map = HashMap::new();
        let entries = fs::read_dir(directory)
            .map_err(|error| PlatformError::io("read directory for entry identities", &error))?;
        for entry in entries.flatten() {
            let metadata = match fs::symlink_metadata(entry.path()) {
                Ok(m) => m,
                Err(_) => continue,
            };
            map.insert(
                entry.file_name(),
                crate::PhysicalFileIdentity {
                    volume: metadata.dev(),
                    index: metadata.ino(),
                },
            );
        }
        Ok(Some(map))
    }

    fn fast_direct_physical_directories(
        &self,
        root: &Path,
        maximum_entries: usize,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<DirectPhysicalDirectoryEnumeration>, DirectoryTreeAggregateError> {
        let mut directories = Vec::new();
        let mut observed_count = 0usize;
        let entries = fs::read_dir(root).map_err(|error| {
            DirectoryTreeAggregateError::Platform(format!("failed to read directory: {error}"))
        })?;
        for entry in entries.flatten() {
            observed_count += 1;
            if observed_count > maximum_entries {
                break;
            }
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if file_type.is_dir() && !file_type.is_symlink() {
                directories.push(entry.path());
            }
        }
        Ok(Some(DirectPhysicalDirectoryEnumeration {
            directories,
            observed_count,
            strategy: "linux-read-dir-d-type",
        }))
    }

    fn fast_directory_tree_aggregate(
        &self,
        _root: &Path,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
        _report_progress: &(dyn Fn(&Path, u64, u64) + Sync),
    ) -> Result<Option<DirectoryTreeAggregate>, DirectoryTreeAggregateError> {
        Ok(None)
    }

    fn fast_project_artifact_tree_aggregate(
        &self,
        _root: &Path,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
        _report_progress: &(dyn Fn(&Path, u64, u64) + Sync),
    ) -> Result<Option<DirectoryTreeAggregate>, DirectoryTreeAggregateError> {
        Ok(None)
    }

    fn fast_application_component_aggregate(
        &self,
        _root: &Path,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
        _report_progress: &(dyn Fn(&Path, u64, u64) + Sync),
    ) -> Result<Option<ApplicationComponentAggregate>, ApplicationComponentAggregateError> {
        Ok(None)
    }

    fn fast_analysis_records(
        &self,
        _query: FastAnalysisQuery<'_>,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
        _report_progress: &mut dyn FnMut(&Path, u64, u64),
        _consumer: &mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
    ) -> Result<Option<FastAnalysisSummary>, FastAnalysisScanError> {
        Ok(None)
    }
}

use crate::PrivacyPlatform;
use crate::StartupPlatform;
use crate::SystemMaintenancePlatform;
use crate::SystemSettingsPlatform;

impl crate::AiModelDiscoveryPlatform for LinuxPlatform {
    fn discover_ai_models(
        &self,
        cancellation: &PlatformCancellation,
    ) -> crate::PlatformResult<Vec<crate::InstalledAiModel>> {
        ai_models::discover_ollama_models(cancellation)
    }
}

impl PrivacyPlatform for LinuxPlatform {
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

    fn clear_application_privacy_trace(
        &self,
        trace: crate::PlatformPrivacyApplicationNativeTraceKind,
    ) -> PlatformResult<bool> {
        privacy::clear_application_trace(trace)
    }

    fn system_privacy_trace_details(
        &self,
        _trace: crate::PlatformPrivacySystemTraceKind,
        _offset: u64,
        _limit: u32,
    ) -> PlatformResult<Vec<crate::PlatformPrivacyDetailEntry>> {
        Ok(Vec::new())
    }

    fn application_privacy_trace_details(
        &self,
        _trace: crate::PlatformPrivacyApplicationNativeTraceKind,
        _offset: u64,
        _limit: u32,
    ) -> PlatformResult<Vec<crate::PlatformPrivacyDetailEntry>> {
        Ok(Vec::new())
    }
}

impl StartupPlatform for LinuxPlatform {
    fn scan_startup_sources(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<crate::PlatformStartupSourceResult>> {
        startup::scan_startup(cancellation)
    }

    fn change_startup_item(
        &self,
        request: &crate::PlatformStartupChangeRequest,
        authorization_prompt: Option<&str>,
    ) -> PlatformResult<crate::PlatformStartupChangeResult> {
        startup::change_startup_item(request, authorization_prompt)
    }
}

impl SystemSettingsPlatform for LinuxPlatform {
    fn scan_system_settings(
        &self,
        _setting_ids: &[&str],
        _cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<crate::PlatformSystemSettingState>> {
        Ok(Vec::new())
    }

    fn change_system_setting(
        &self,
        _request: &crate::PlatformSystemSettingChangeRequest,
    ) -> PlatformResult<crate::PlatformSystemSettingChangeResult> {
        Err(PlatformError::new(
            crate::PlatformErrorCode::Unsupported,
            "system settings are not yet supported on Linux",
        ))
    }
}

impl SystemMaintenancePlatform for LinuxPlatform {
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
