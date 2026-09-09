#[cfg(windows)]
use mangodisk_platform::ApplicationUninstallRegistration;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::ApplicationCloseMode;

pub const APPLICATION_UNINSTALL_SCAN_SCHEMA_VERSION: u32 = 11;
pub const APPLICATION_UNINSTALL_INSPECTION_SCHEMA_VERSION: u32 = 3;
pub const APPLICATION_UNINSTALL_PLAN_SCHEMA_VERSION: u32 = 2;
pub const APPLICATION_UNINSTALL_BATCH_PLAN_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallPlatform {
    MacosBundle,
    WindowsRegistry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallInstallerKind {
    WindowsMsi,
    WindowsAppx,
    WindowsScoop,
    WindowsChocolatey,
    WindowsRegistered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallInventorySource {
    MacosBundle,
    WindowsRegistry,
    WindowsMsi,
    WindowsAppx,
    Winget,
    Steam,
    Scoop,
    Chocolatey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallExecutionMode {
    Silent,
    Interactive,
    ExternalClient,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallSourceIdentity {
    pub source: ApplicationUninstallInventorySource,
    pub identifier: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallCapability {
    Ready,
    ApplicationRunning,
    RequiresElevation,
    ProtectedApplication,
    ViewOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallRecordState {
    Installed,
    OrphanedRegistration,
}

impl ApplicationUninstallCapability {
    pub(super) const fn supports_execution(self) -> bool {
        match self {
            Self::Ready => true,
            #[cfg(any(target_os = "macos", windows))]
            Self::RequiresElevation => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallCandidate {
    pub application_id: String,
    pub primary_identifier: String,
    pub source_identities: Vec<ApplicationUninstallSourceIdentity>,
    /// Presentation classification only. Explicit uninstall and record removal remain independent.
    pub system_kind: super::system_classification::ApplicationSystemKind,
    pub name: String,
    pub version: Option<String>,
    pub publisher: Option<String>,
    /// Platform catalog estimate. Windows registrations and macOS metadata normally expose
    /// logical sizes and cannot account exactly for compression, clones, or shared package data.
    pub estimated_bytes: u64,
    pub last_used_at_ms: Option<u64>,
    pub installed_at_ms: Option<u64>,
    pub platform: ApplicationUninstallPlatform,
    pub installer_kind: Option<ApplicationUninstallInstallerKind>,
    pub execution_mode: Option<ApplicationUninstallExecutionMode>,
    pub capability: ApplicationUninstallCapability,
    pub record_state: ApplicationUninstallRecordState,
    pub uninstall_diagnostic: Option<mangodisk_platform::ApplicationUninstallDiagnostic>,
    pub application_path: Option<String>,
    /// Existing user-data locations whose names match multiple stable catalog
    /// facts. These paths are read-only hints, not verified uninstall
    /// components, and never contribute to selection or byte totals.
    pub possible_related_paths: Vec<String>,
    pub icon_path: Option<String>,
    pub running_processes: Vec<String>,
    /// Exact executable identities retained only inside Core for process
    /// control. The WebView receives stable application IDs and display names,
    /// but cannot turn an uninstall request into an arbitrary path-based close.
    #[serde(skip)]
    pub(super) executable_paths: Vec<PathBuf>,
    /// Sum of component estimates used for catalog comparison, never an exact free-space promise.
    pub total_bytes: u64,
    /// Estimated bytes represented by components selected by default.
    pub default_selected_bytes: u64,
    pub associated_data_complete: bool,
    pub components: Vec<ApplicationUninstallComponentSummary>,
    #[cfg(windows)]
    #[serde(skip)]
    pub(super) uninstall_registration: Option<ApplicationUninstallRegistration>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallComponentSummary {
    pub component_id: String,
    pub kind: ApplicationUninstallComponentKind,
    pub risk: ApplicationUninstallRisk,
    pub path: Option<String>,
    pub bytes: u64,
    pub file_count: u64,
    pub default_selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallScanResult {
    pub schema_version: u32,
    pub scanned_at_ms: u64,
    pub supported: bool,
    pub execution_supported: bool,
    /// Whether the captured catalog is stable enough to inspect and remove
    /// verified applications. Unreadable bundles are excluded from a partial
    /// macOS catalog without restricting independently verified candidates.
    pub catalog_actionable: bool,
    pub inventory_complete: bool,
    pub catalog_revision: Option<String>,
    pub candidates: Vec<ApplicationUninstallCandidate>,
    pub ready_count: u64,
    pub blocked_count: u64,
    pub hidden_count: u64,
    pub related_directory_count: u64,
    pub related_path_scan_elapsed_ms: u64,
    pub elapsed_ms: u64,
}

impl ApplicationUninstallScanResult {
    /// Remove a natively verified record from a display snapshot without enumerating inventory.
    /// This does not refresh the catalog revision or authorize further native operations.
    pub fn remove_record(&mut self, application_id: &str) -> usize {
        let previous_count = self.candidates.len();
        self.candidates
            .retain(|candidate| candidate.application_id != application_id);
        self.ready_count = self
            .candidates
            .iter()
            .filter(|candidate| candidate.capability.supports_execution())
            .count() as u64;
        self.blocked_count = self.candidates.len() as u64 - self.ready_count;
        previous_count - self.candidates.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallComponentKind {
    ApplicationBinary,
    NativeInstaller,
    Cache,
    ApplicationSupport,
    Preferences,
    Logs,
    SavedState,
    SandboxContainer,
    WebData,
}

impl ApplicationUninstallComponentKind {
    pub(super) fn stable_code(self) -> &'static str {
        match self {
            Self::ApplicationBinary => "applicationBinary",
            Self::NativeInstaller => "nativeInstaller",
            Self::Cache => "cache",
            Self::ApplicationSupport => "applicationSupport",
            Self::Preferences => "preferences",
            Self::Logs => "logs",
            Self::SavedState => "savedState",
            Self::SandboxContainer => "sandboxContainer",
            Self::WebData => "webData",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallRisk {
    Required,
    Rebuildable,
    UserData,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallComponent {
    pub component_id: String,
    pub kind: ApplicationUninstallComponentKind,
    pub risk: ApplicationUninstallRisk,
    pub path: Option<String>,
    pub bytes: u64,
    pub file_count: u64,
    pub default_selected: bool,
    pub snapshot_fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallInspection {
    pub schema_version: u32,
    pub inspected_at_ms: u64,
    pub application_id: String,
    pub application_name: String,
    pub primary_identifier: String,
    pub platform: ApplicationUninstallPlatform,
    pub installer_kind: Option<ApplicationUninstallInstallerKind>,
    pub capability: ApplicationUninstallCapability,
    pub catalog_revision: String,
    pub components: Vec<ApplicationUninstallComponent>,
    pub total_bytes: u64,
    pub default_selected_bytes: u64,
    pub elapsed_ms: u64,
    #[cfg(windows)]
    #[serde(skip)]
    pub(super) uninstall_registration: Option<ApplicationUninstallRegistration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationUninstallPlanItem {
    pub component_id: String,
    pub kind: ApplicationUninstallComponentKind,
    pub expected_bytes: u64,
    pub expected_file_count: u64,
    pub expected_snapshot_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationUninstallPlan {
    pub schema_version: u32,
    pub plan_id: String,
    pub plan_hash: String,
    pub created_at_ms: u64,
    pub application_id: String,
    pub catalog_revision: String,
    pub items: Vec<ApplicationUninstallPlanItem>,
    pub expected_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationUninstallBatchSelection {
    pub application_id: String,
    pub component_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationUninstallCloseRequest {
    pub application_ids: Vec<String>,
    pub mode: ApplicationCloseMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationUninstallBatchPlan {
    pub schema_version: u32,
    pub batch_id: String,
    pub batch_hash: String,
    pub created_at_ms: u64,
    pub catalog_revision: String,
    pub plans: Vec<ApplicationUninstallPlan>,
    pub expected_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallBatchPreparation {
    pub plan: ApplicationUninstallBatchPlan,
    pub preview: ApplicationUninstallBatchResult,
}

/// Stable execution stages for a prepared uninstall batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallExecutionStage {
    Validating,
    Uninstalling,
    Finalizing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallExecutionItemStatus {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallExecutionItemResult {
    pub application_id: String,
    pub status: ApplicationUninstallExecutionItemStatus,
    pub released_bytes: u64,
}

/// A low-frequency batch snapshot emitted only at application boundaries.
///
/// Application IDs let the desktop adapter resolve the user-facing name from
/// its current catalog without exposing application names in native logs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallExecutionProgress {
    pub stage: ApplicationUninstallExecutionStage,
    pub current_application_id: Option<String>,
    pub completed_applications: Vec<ApplicationUninstallExecutionItemResult>,
    pub completed_application_count: u64,
    pub total_application_count: u64,
    pub affected_application_count: u64,
    pub failed_application_count: u64,
    pub released_bytes: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallActionStatus {
    Previewed,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationUninstallActionReason {
    ApplicationUnavailable,
    ApplicationRunning,
    ProcessStateUnavailable,
    CatalogChanged,
    ComponentUnavailable,
    ComponentChanged,
    UnsupportedExecutor,
    ExecutionAborted,
    ExternalUninstallerContinuing,
    #[serde(alias = "moveToTrashFailed")]
    PermanentDeleteFailed,
    RecoveryRequired,
    NativeInstallerFailed,
    NativeInstallerFailedAfterRemoval,
    VerificationFailed,
}

impl ApplicationUninstallActionReason {
    pub(super) const fn stable_code(self) -> &'static str {
        match self {
            Self::ApplicationUnavailable => "application_unavailable",
            Self::ApplicationRunning => "application_running",
            Self::ProcessStateUnavailable => "process_state_unavailable",
            Self::CatalogChanged => "catalog_changed",
            Self::ComponentUnavailable => "component_unavailable",
            Self::ComponentChanged => "component_changed",
            Self::UnsupportedExecutor => "unsupported_executor",
            Self::ExecutionAborted => "execution_aborted",
            Self::ExternalUninstallerContinuing => "external_uninstaller_continuing",
            Self::PermanentDeleteFailed => "permanent_delete_failed",
            Self::RecoveryRequired => "recovery_required",
            Self::NativeInstallerFailed => "native_installer_failed",
            Self::NativeInstallerFailedAfterRemoval => "native_installer_failed_after_removal",
            Self::VerificationFailed => "verification_failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallActionResult {
    pub component_id: String,
    pub kind: ApplicationUninstallComponentKind,
    pub status: ApplicationUninstallActionStatus,
    pub reason: Option<ApplicationUninstallActionReason>,
    pub expected_bytes: u64,
    pub released_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallResult {
    pub plan_id: String,
    pub application_id: String,
    pub application_name: Option<String>,
    pub expected_bytes: u64,
    pub previewed_bytes: u64,
    pub released_bytes: u64,
    pub previewed_item_count: u64,
    pub affected_item_count: u64,
    pub failed_item_count: u64,
    pub released_bytes_is_estimate: bool,
    pub restart_required: bool,
    pub dry_run: bool,
    pub actions: Vec<ApplicationUninstallActionResult>,
    pub history_saved: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallBatchResult {
    pub batch_id: String,
    pub expected_bytes: u64,
    pub previewed_bytes: u64,
    pub released_bytes: u64,
    pub selected_application_count: u64,
    pub previewed_application_count: u64,
    pub affected_application_count: u64,
    pub failed_application_count: u64,
    pub previewed_item_count: u64,
    pub affected_item_count: u64,
    pub failed_item_count: u64,
    pub released_bytes_is_estimate: bool,
    pub restart_required: bool,
    pub dry_run: bool,
    pub results: Vec<ApplicationUninstallResult>,
}

#[cfg(test)]
mod tests {
    use super::{ApplicationUninstallActionReason, ApplicationUninstallCapability};

    #[test]
    fn executable_capabilities_remain_platform_scoped() {
        assert!(ApplicationUninstallCapability::Ready.supports_execution());
        #[cfg(any(target_os = "macos", windows))]
        assert!(ApplicationUninstallCapability::RequiresElevation.supports_execution());
        #[cfg(not(any(target_os = "macos", windows)))]
        assert!(!ApplicationUninstallCapability::RequiresElevation.supports_execution());
        assert!(!ApplicationUninstallCapability::ApplicationRunning.supports_execution());
        assert!(!ApplicationUninstallCapability::ProtectedApplication.supports_execution());
        assert!(!ApplicationUninstallCapability::ViewOnly.supports_execution());
    }

    #[test]
    fn legacy_trash_failure_reason_migrates_to_permanent_delete_failure() {
        let reason =
            serde_json::from_str::<ApplicationUninstallActionReason>("\"moveToTrashFailed\"")
                .expect("legacy persisted action reason must remain readable");

        assert_eq!(
            reason,
            ApplicationUninstallActionReason::PermanentDeleteFailed
        );
        assert_eq!(
            serde_json::to_string(&reason).expect("current action reason must serialize"),
            "\"permanentDeleteFailed\""
        );
    }
}
