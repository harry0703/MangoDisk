pub mod ai;
mod applications;
mod cleanup;
mod filesystem;
mod history;
mod privacy;
mod reporting;
mod shared;
mod startup;
mod storage;
mod system_maintenance;
mod system_settings;

pub const APPLICATION_IDENTIFIER: &str = "app.mangodisk.desktop";

pub use applications::leftovers::{
    ApplicationLeftoverActionReason, ApplicationLeftoverActionResult,
    ApplicationLeftoverActionStatus, ApplicationLeftoverCandidate, ApplicationLeftoverConfidence,
    ApplicationLeftoverEvidence, ApplicationLeftoverPlan, ApplicationLeftoverPlanItem,
    ApplicationLeftoverResult, ApplicationLeftoverScanResult, ApplicationLeftoverService,
    ApplicationLeftoverSource,
};
pub use applications::process_control::{
    ApplicationCloseBatchResult, ApplicationCloseMode, ApplicationCloseTargetResult,
    ApplicationCloseTargetStatus,
};
pub use applications::uninstall::{
    ApplicationUninstallActionReason, ApplicationUninstallActionResult,
    ApplicationUninstallActionStatus, ApplicationUninstallBatchPlan,
    ApplicationUninstallBatchPreparation, ApplicationUninstallBatchResult,
    ApplicationUninstallBatchSelection, ApplicationUninstallCandidate,
    ApplicationUninstallCapability, ApplicationUninstallCloseRequest,
    ApplicationUninstallComponent, ApplicationUninstallComponentKind,
    ApplicationUninstallComponentSummary, ApplicationUninstallExecutionItemResult,
    ApplicationUninstallExecutionItemStatus, ApplicationUninstallExecutionProgress,
    ApplicationUninstallExecutionStage, ApplicationUninstallInspection,
    ApplicationUninstallInstallerKind, ApplicationUninstallPlan, ApplicationUninstallPlanItem,
    ApplicationUninstallPlatform, ApplicationUninstallRecordState, ApplicationUninstallResult,
    ApplicationUninstallRisk, ApplicationUninstallScanResult, ApplicationUninstallService,
};
pub use cleanup::{
    CleanupActionKind, CleanupActionReason, CleanupActionResult, CleanupActionStatus,
    CleanupApplicationCloseRequest, CleanupApplicationIcon, CleanupAutomationProfile,
    CleanupCategory, CleanupExecutionProgress, CleanupExecutionStage, CleanupGroup, CleanupPlan,
    CleanupRequest, CleanupResult, CleanupScanEngineInfo, CleanupScanResult,
    CleanupSourceBlockReason, CleanupSourceDetail, CleanupSourceSelection,
    CleanupSourceSelectionMode, CustomCleanupModifiedTime, CustomCleanupRule, RiskLevel,
    ScanItemStatus, ScanRuleResult, CLEANUP_AUTOMATION_PROFILE_SCHEMA_VERSION,
    CLEANUP_PLAN_SCHEMA_VERSION, CUSTOM_CLEANUP_RULE_SCHEMA_VERSION,
};
pub use cleanup::{CleanupPlanService, CleanupScanService, CleanupService};
pub use filesystem::{
    metadata::diagnostic_path, DirectorySelectionOutcome, DirectorySelectionService, DiskInfo,
    PermanentDeleteBatchResult, PermanentDeleteCandidate, PermanentDeleteFailure,
    ResolvedDirectory,
};
pub use history::{
    ApplicationLeftoverOperationDetails, ApplicationUninstallOperationDetails,
    CleanupOperationDetails, DeepCleanupOperationDetails, FileCleanupHistoryItem,
    FileCleanupHistoryItemStatus, FileCleanupOperationDetails, HistoryService, OperationCategory,
    OperationDetails, OperationOutcome, OperationRecord, PrivacyCleanupHistoryItem,
    PrivacyCleanupHistoryItemStatus, PrivacyCleanupOperationDetails, StartupHistoryItem,
    StartupHistoryItemStatus, StartupHistoryState, StartupManagementOperationDetails,
    SystemOptimizationHistoryItem, SystemOptimizationHistoryItemStatus,
    SystemOptimizationOperationDetails, OPERATION_RECORD_SCHEMA_VERSION,
};
pub use privacy::{
    PrivacyBrowserCloseRequest, PrivacyBrowserCloseRequirement, PrivacyBrowserStatusRequest,
    PrivacyBrowserStatusResult, PrivacyBrowserStatusTarget, PrivacyCapabilityState,
    PrivacyCategory, PrivacyDataKind, PrivacyDetailEntry, PrivacyDetailsPage,
    PrivacyDetailsPresentation, PrivacyDetailsRequest, PrivacyExecutionItemResult,
    PrivacyExecutionItemStatus, PrivacyExecutionPlan, PrivacyExecutionPlanItem,
    PrivacyExecutionProgress, PrivacyExecutionProgressItem, PrivacyExecutionRequest,
    PrivacyExecutionResult, PrivacyExecutionRunRequest, PrivacyExecutionStage, PrivacyImpact,
    PrivacyItem, PrivacyRecommendation, PrivacyScanRequest, PrivacyScanResult, PrivacySensitivity,
    PrivacyService, PrivacySourceCoverage, PrivacyTimeRange, PRIVACY_DETAILS_SCHEMA_VERSION,
    PRIVACY_PLAN_SCHEMA_VERSION, PRIVACY_SCAN_SCHEMA_VERSION,
};
pub use reporting::{
    BaselineArtifacts, BaselineComparisonArtifacts, BaselineComparisonOptions,
    BenchmarkDatasetArtifacts, BenchmarkDatasetOptions, BenchmarkDatasetService,
    BenchmarkSourceInfo, CleanupBaselineComparisonService, CleanupBaselineOptions,
    CleanupBaselineService, EngineBenchmarkArtifacts, EngineBenchmarkComparisonArtifacts,
    EngineBenchmarkComparisonOptions, EngineBenchmarkComparisonService, EngineBenchmarkOptions,
    EngineBenchmarkService,
};
pub use shared::{
    configure_application_paths, ApplicationPaths, CoreError, CoreErrorCode, CoreErrorReason,
    CoreResult, OperationCancellationToken, ProgressSink, TraversalProgress, TraversalStage,
};
pub use startup::{
    StartupAggregateConfiguredState, StartupAggregateControlState, StartupArtifact, StartupCatalog,
    StartupCatalogSummary, StartupChangeFailureReason, StartupChangeItemResult,
    StartupChangeOutcomeStatus, StartupChangePlan, StartupChangePlanItem, StartupChangeResult,
    StartupChangeSelection, StartupChangeSkipReason, StartupChangeSkippedItem,
    StartupChangeWarning, StartupConfiguredState, StartupControlCapability, StartupCoverageReason,
    StartupCoverageStatus, StartupDesiredState, StartupDiagnosticCode, StartupIdentityConfidence,
    StartupOwnerGroup, StartupRuntimeState, StartupScope, StartupService, StartupSourceCoverage,
    StartupSourceKind, StartupSummarySource, StartupTarget, StartupTargetKind, StartupTrigger,
    StartupTrustState, STARTUP_CATALOG_SCHEMA_VERSION, STARTUP_CHANGE_PLAN_SCHEMA_VERSION,
};
pub use storage::analysis::{
    AnalysisDeleteResult, AnalysisResult, AnalysisService, DirectoryEntryInfo,
};
pub use storage::duplicates::{
    DuplicateFileEntry, DuplicateFileService, DuplicateFilesResult, DuplicateGroup,
    DuplicateGroupBatch, DuplicateGroupPage,
};
pub use storage::large_files::{
    LargeFileEntry, LargeFileScanMode, LargeFileService, LargeFilesResult,
};
pub use system_maintenance::{
    SystemMaintenanceCatalog, SystemMaintenanceCatalogSummary, SystemMaintenanceCategory,
    SystemMaintenanceExecutionItemResult, SystemMaintenanceExecutionRequest,
    SystemMaintenanceExecutionStatus, SystemMaintenanceFailureReason, SystemMaintenanceItem,
    SystemMaintenanceJob, SystemMaintenanceJobStatus, SystemMaintenanceJobUpdateSink,
    SystemMaintenanceMutationState, SystemMaintenancePlatform, SystemMaintenanceRiskLevel,
    SystemMaintenanceRuntimeState, SystemMaintenanceService, SystemMaintenanceStatus,
    SYSTEM_MAINTENANCE_CATALOG_SCHEMA_VERSION,
};
pub use system_settings::{
    SystemSettingCategory, SystemSettingChangeFailureReason, SystemSettingChangeItemResult,
    SystemSettingChangeOutcomeStatus, SystemSettingChangePlanItem,
    SystemSettingChangeSelectionItem, SystemSettingChangeSkipReason,
    SystemSettingChangeSkippedItem, SystemSettingItem, SystemSettingSelectionKind,
    SystemSettingStatus, SystemSettingTargetState, SystemSettingsCatalog,
    SystemSettingsCatalogSummary, SystemSettingsChangePlan, SystemSettingsChangeResult,
    SystemSettingsChangeSelection, SystemSettingsPlatform, SystemSettingsService,
    SYSTEM_SETTINGS_CATALOG_SCHEMA_VERSION, SYSTEM_SETTINGS_CHANGE_PLAN_SCHEMA_VERSION,
};
