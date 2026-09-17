use mangodisk_platform::{PlatformSystemSettingDiagnosticCode, PlatformSystemSettingValue};
use serde::{Deserialize, Serialize};

pub const SYSTEM_SETTINGS_CATALOG_SCHEMA_VERSION: u32 = 4;
pub const SYSTEM_SETTINGS_CHANGE_PLAN_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemSettingsPlatform {
    Macos,
    Windows,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemSettingCategory {
    Performance,
    Productivity,
    Privacy,
    Storage,
    Gaming,
    Appearance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemSettingSelectionKind {
    OneClick,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemSettingStatus {
    Recommended,
    Optimized,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemSettingRiskLevel {
    Standard,
    Caution,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSettingItem {
    pub setting_id: String,
    pub category: SystemSettingCategory,
    pub selection_kind: SystemSettingSelectionKind,
    pub risk_level: SystemSettingRiskLevel,
    pub status: SystemSettingStatus,
    pub selected_by_default: bool,
    /// True only when MangoDisk has a valid original value for this exact setting state.
    pub restore_available: bool,
    pub requires_restart: bool,
    pub requires_elevation: bool,
    pub diagnostic: Option<PlatformSystemSettingDiagnosticCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSettingsCatalogSummary {
    pub item_count: u64,
    pub recommended_count: u64,
    pub optimized_count: u64,
    pub selected_count: u64,
    pub unavailable_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSettingsCatalog {
    pub schema_version: u32,
    pub scan_id: String,
    pub catalog_revision: String,
    pub platform: SystemSettingsPlatform,
    pub scanned_at_ms: u64,
    pub items: Vec<SystemSettingItem>,
    pub summary: SystemSettingsCatalogSummary,
    pub elapsed_ms: u64,
    pub recovery_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SystemSettingsChangeSelection {
    pub scan_id: String,
    pub items: Vec<SystemSettingChangeSelectionItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SystemSettingChangeSelectionItem {
    pub setting_id: String,
    pub target: SystemSettingTargetState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemSettingTargetState {
    Optimized,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemSettingChangeSkipReason {
    AlreadyOptimized,
    AlreadyDefault,
    CatalogExpired,
    SettingChanged,
    SettingMissing,
    StateUnavailable,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSettingChangePlanItem {
    pub setting_id: String,
    pub category: SystemSettingCategory,
    pub target: SystemSettingTargetState,
    pub requires_restart: bool,
    pub requires_elevation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSettingChangeSkippedItem {
    pub setting_id: String,
    pub reason: SystemSettingChangeSkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSettingsChangePlan {
    pub schema_version: u32,
    pub plan_id: String,
    pub scan_id: String,
    pub catalog_revision: String,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub items: Vec<SystemSettingChangePlanItem>,
    pub skipped_items: Vec<SystemSettingChangeSkippedItem>,
    pub requires_confirmation: bool,
    pub requires_restart: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemSettingChangeOutcomeStatus {
    Changed,
    Unchanged,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemSettingChangeFailureReason {
    SettingChanged,
    PermissionDenied,
    Unsupported,
    VerificationFailed,
    PlatformFailure,
    UserCancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSettingChangeItemResult {
    pub setting_id: String,
    pub status: SystemSettingChangeOutcomeStatus,
    pub verified: bool,
    pub failure_reason: Option<SystemSettingChangeFailureReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSettingsChangeResult {
    pub plan_id: String,
    pub changed_count: u64,
    pub failed_count: u64,
    pub requires_restart: bool,
    pub recovery_available: bool,
    pub items: Vec<SystemSettingChangeItemResult>,
    pub catalog: Option<SystemSettingsCatalog>,
}

#[derive(Clone)]
pub(super) struct CatalogNativeItem {
    pub public: SystemSettingItem,
    pub current_value: PlatformSystemSettingValue,
    pub effective_value: PlatformSystemSettingValue,
    pub disabled_value: PlatformSystemSettingValue,
    pub recommended_value: PlatformSystemSettingValue,
}

#[derive(Clone)]
pub(super) struct CatalogSession {
    pub public: SystemSettingsCatalog,
    pub native_items: Vec<CatalogNativeItem>,
}

#[derive(Clone)]
pub(super) struct PendingChangeItem {
    pub public: SystemSettingChangePlanItem,
    pub expected_value: PlatformSystemSettingValue,
    pub desired_value: PlatformSystemSettingValue,
}

#[derive(Clone)]
pub(super) struct PendingChangePlan {
    pub public: SystemSettingsChangePlan,
    pub items: Vec<PendingChangeItem>,
}
