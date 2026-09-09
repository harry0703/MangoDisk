mod batch;
#[cfg(any(target_os = "macos", test))]
mod execution;
#[cfg(target_os = "macos")]
mod macos;
mod models;
mod plan;
mod preflight;
mod service;
mod system_classification;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_observations;

pub use models::{
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
    ApplicationUninstallRisk, ApplicationUninstallScanResult,
};
pub use service::ApplicationUninstallService;
