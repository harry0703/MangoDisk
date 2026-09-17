mod directories;
mod inventory;
mod process_control;
mod volumes;

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    ApplicationComponentAggregate, ApplicationComponentAggregateError, ApplicationDirectories,
    ApplicationProcessCloseMode, ApplicationProcessCloseResult, ApplicationProcessTarget,
    DirectoryTreeAggregate, DirectoryTreeAggregateError, FastAnalysisQuery, FastAnalysisRecord,
    FastAnalysisScanError, FastAnalysisSummary, FileSpaceUsage, Platform, PlatformCancellation,
    PlatformError, PlatformResult, ScanPurpose, SkipReason, SystemInventory, UserDirectories,
    VolumeInfo,
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

impl PrivacyPlatform for LinuxPlatform {
    fn discover_privacy_sources(
        &self,
        _cancellation: &PlatformCancellation,
    ) -> PlatformResult<crate::PlatformPrivacyDiscovery> {
        Ok(crate::PlatformPrivacyDiscovery {
            browsers: Vec::new(),
            applications: Vec::new(),
            system_traces: Vec::new(),
        })
    }

    fn clear_system_privacy_trace(
        &self,
        _trace: crate::PlatformPrivacySystemTraceKind,
    ) -> PlatformResult<bool> {
        Ok(false)
    }

    fn clear_application_privacy_trace(
        &self,
        _trace: crate::PlatformPrivacyApplicationNativeTraceKind,
    ) -> PlatformResult<bool> {
        Ok(false)
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
        _cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<crate::PlatformStartupSourceResult>> {
        Ok(Vec::new())
    }

    fn change_startup_item(
        &self,
        _request: &crate::PlatformStartupChangeRequest,
        _authorization_prompt: Option<&str>,
    ) -> PlatformResult<crate::PlatformStartupChangeResult> {
        Err(PlatformError::new(
            crate::PlatformErrorCode::Unsupported,
            "startup item management is not yet supported on Linux",
        ))
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
        _task_ids: &[&str],
        _cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<crate::PlatformSystemMaintenanceState>> {
        Ok(Vec::new())
    }

    fn execute_system_maintenance(
        &self,
        _task_id: &str,
        _cancellation: &PlatformCancellation,
        _authorization_prompt: Option<&str>,
        _progress: &crate::PlatformSystemMaintenanceProgressSink,
    ) -> PlatformResult<crate::PlatformSystemMaintenanceExecution> {
        Err(PlatformError::new(
            crate::PlatformErrorCode::Unsupported,
            "system maintenance is not yet supported on Linux",
        ))
    }
}
