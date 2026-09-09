mod applications;
mod directory_aggregate;
mod disk_cleanup;
mod error;
mod platform;
mod privacy;
mod processes;
mod scan;
mod startup;
mod system_maintenance;
mod system_settings;
mod volumes;

pub use applications::{
    application_uninstall_diagnostic_id, registered_application_path_is_missing,
    ApplicationComponentAggregate, ApplicationComponentAggregateError, ApplicationInstallScope,
    ApplicationInventorySource, ApplicationSourceIdentity, ApplicationUninstallDiagnostic,
    ApplicationUninstallExecutionOutcome, ApplicationUninstallPlatformError,
    ApplicationUninstallRegistration, ApplicationUninstallRegistrationState, DetectedTool,
    InstalledApplication, MacosPrivilegedApplicationRemovalOutcome, SystemInventory,
    WindowsRegisteredUninstallKind, WindowsRegistryView,
};
#[cfg(test)]
pub(crate) use directory_aggregate::reference_directory_tree_aggregate;
pub(crate) use directory_aggregate::DirectoryAggregateProgress;
pub use directory_aggregate::{
    DirectPhysicalDirectoryEnumeration, DirectoryTreeAggregate, DirectoryTreeAggregateError,
    DirectoryTreeSourceAggregate,
};
pub use disk_cleanup::{
    PlatformCancellation, WindowsDiskCleanupAvailability, WindowsDiskCleanupEstimate,
    WindowsDiskCleanupExecution, WindowsDiskCleanupExecutionStatus, WindowsDiskCleanupKind,
};
pub use error::{
    PlatformError, PlatformErrorCode, PlatformFailureReason, PlatformMutationState, PlatformResult,
};
pub use platform::Platform;
pub use privacy::{
    PlatformPrivacyApplication, PlatformPrivacyApplicationNativeTraceKind,
    PlatformPrivacyApplicationTrace, PlatformPrivacyApplicationTraceAvailability,
    PlatformPrivacyApplicationTraceKind, PlatformPrivacyBrowser, PlatformPrivacyBrowserKind,
    PlatformPrivacyDetailEntry, PlatformPrivacyDiscovery, PlatformPrivacyProfile,
    PlatformPrivacySystemTrace, PlatformPrivacySystemTraceKind, PrivacyPlatform,
};
pub use processes::{
    ApplicationProcessCloseMode, ApplicationProcessCloseResult, ApplicationProcessTarget,
    RunningProcessIdentity,
};
pub(crate) use scan::FilesystemChangeMonitorBackend;
pub use scan::{
    DirectoryEntryIdentities, FastAnalysisQuery, FastAnalysisRecord, FastAnalysisScanError,
    FastAnalysisSummary, FileSpaceUsage, FilesystemChangeImpactError,
    FilesystemChangeImpactOutcome, FilesystemChangeImpactPlan, FilesystemChangeImpactSummary,
    FilesystemChangeImpactUnavailable, FilesystemChangeMonitor, FilesystemChangeStatus,
    FilesystemChangeToken, LargeFileCandidateScanError, LargeFileCandidateSummary,
    PhysicalFileIdentity, ProjectMarkerCandidateProgress, ProjectMarkerCandidateQuery,
    ProjectMarkerCandidateScanError, ProjectMarkerCandidateSummary, ScanPurpose, SkipReason,
};
pub use startup::{
    PlatformStartupArtifact, PlatformStartupChangeRequest, PlatformStartupChangeResult,
    PlatformStartupConfiguredState, PlatformStartupControlCapability,
    PlatformStartupCoverageReason, PlatformStartupCoverageStatus, PlatformStartupDesiredState,
    PlatformStartupDiagnosticCode, PlatformStartupIdentityConfidence, PlatformStartupOwner,
    PlatformStartupRuntimeState, PlatformStartupScope, PlatformStartupSourceKind,
    PlatformStartupSourceResult, PlatformStartupSummarySource, PlatformStartupTarget,
    PlatformStartupTargetKind, PlatformStartupTrigger, PlatformStartupTrustState, StartupPlatform,
};
pub use system_maintenance::{
    PlatformSystemMaintenanceCompletion, PlatformSystemMaintenanceDiagnosticCode,
    PlatformSystemMaintenanceExecution, PlatformSystemMaintenancePhase,
    PlatformSystemMaintenanceProgress, PlatformSystemMaintenanceProgressSink,
    PlatformSystemMaintenanceState, PlatformSystemMaintenanceStatus, SystemMaintenancePlatform,
};
#[cfg(target_os = "macos")]
pub(crate) use system_settings::preflight_system_setting_change;
pub use system_settings::{
    PlatformSystemSettingChangeRequest, PlatformSystemSettingChangeResult,
    PlatformSystemSettingDiagnosticCode, PlatformSystemSettingSnapshot, PlatformSystemSettingState,
    PlatformSystemSettingValue, SystemSettingsPlatform,
};
pub use volumes::{
    ApplicationDirectories, ScanConcurrency, ScanDeviceClass, UserDirectories, VolumeInfo,
};
