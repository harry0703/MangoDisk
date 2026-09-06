use serde::{Deserialize, Serialize};

pub const STARTUP_CATALOG_SCHEMA_VERSION: u32 = 3;
pub const STARTUP_CHANGE_PLAN_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupSourceKind {
    RegistryRun,
    StartupFolder,
    ScheduledTask,
    Service,
    PackagedStartupTask,
    LaunchAgent,
    LaunchDaemon,
    LoginItem,
    BackgroundTask,
    EmbeddedItem,
    AdvancedAutoRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupScope {
    CurrentUser,
    User,
    AllUsers,
    Machine,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupTrigger {
    Boot,
    UserLogon,
    Scheduled,
    Event,
    KeepAlive,
    ShellLoad,
    ApplicationLaunch,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupConfiguredState {
    Enabled,
    Disabled,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupRuntimeState {
    Running,
    Stopped,
    Loaded,
    Unloaded,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupControlCapability {
    Toggleable,
    ElevationRequired,
    RemoveOnly,
    SystemManaged,
    PolicyManaged,
    ViewOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupTrustState {
    System,
    Verified,
    Invalid,
    Unsigned,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupIdentityConfidence {
    Exact,
    Strong,
    Probable,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupSummarySource {
    ServiceDescription,
    TaskDescription,
    PackageManifest,
    VersionInfo,
    BundleMetadata,
    SourceLabel,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupTargetKind {
    Executable,
    Application,
    Script,
    Service,
    Task,
    Other,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupDiagnosticCode {
    AccessDenied,
    InvalidData,
    MissingIdentity,
    MissingTarget,
    StateUnavailable,
    UnsupportedFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupCoverageStatus {
    Complete,
    Partial,
    Unavailable,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupCoverageReason {
    AccessDenied,
    ApiUnavailable,
    Cancelled,
    InvalidData,
    NotImplemented,
    StateUnavailable,
    UnsupportedOperatingSystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupAggregateConfiguredState {
    AllEnabled,
    PartiallyEnabled,
    AllDisabled,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupAggregateControlState {
    AllToggleable,
    RequiresElevation,
    PartiallyManageable,
    ViewOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupTarget {
    pub kind: StartupTargetKind,
    pub path: Option<String>,
    pub executable_name: Option<String>,
    pub arguments: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupArtifact {
    pub item_id: String,
    pub source_id: String,
    pub source_kind: StartupSourceKind,
    pub scope: StartupScope,
    pub triggers: Vec<StartupTrigger>,
    pub display_name: String,
    pub configuration_path: Option<String>,
    pub target: StartupTarget,
    pub owner_name: Option<String>,
    pub publisher: Option<String>,
    pub summary: Option<String>,
    pub summary_source: StartupSummarySource,
    pub version: Option<String>,
    pub icon_path: Option<String>,
    pub identity_confidence: StartupIdentityConfidence,
    pub configured_state: StartupConfiguredState,
    pub runtime_state: StartupRuntimeState,
    pub control_capability: StartupControlCapability,
    pub trust: StartupTrustState,
    pub modified_at_ms: Option<u64>,
    pub diagnostics: Vec<StartupDiagnosticCode>,
    pub removal_supported: bool,
    pub removable_orphan: bool,
    #[serde(skip)]
    pub(crate) group_identity_key: String,
    #[serde(skip)]
    pub(crate) fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupOwnerGroup {
    pub group_id: String,
    pub name: String,
    pub publisher: Option<String>,
    pub summary: Option<String>,
    pub summary_source: StartupSummarySource,
    pub version: Option<String>,
    pub icon_path: Option<String>,
    pub identity_confidence: StartupIdentityConfidence,
    pub item_ids: Vec<String>,
    pub source_kinds: Vec<StartupSourceKind>,
    pub triggers: Vec<StartupTrigger>,
    pub scopes: Vec<StartupScope>,
    pub configured_state: StartupAggregateConfiguredState,
    pub control_state: StartupAggregateControlState,
    pub system_item: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupSourceCoverage {
    pub source_id: String,
    pub required: bool,
    pub status: StartupCoverageStatus,
    pub reason: Option<StartupCoverageReason>,
    pub item_count: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupCatalogSummary {
    pub item_count: u64,
    pub group_count: u64,
    pub enabled_count: u64,
    pub disabled_count: u64,
    pub unknown_state_count: u64,
    pub elevation_required_count: u64,
    pub system_item_count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupCatalog {
    pub schema_version: u32,
    pub scan_id: String,
    pub catalog_revision: String,
    pub scanned_at_ms: u64,
    pub complete: bool,
    pub artifacts: Vec<StartupArtifact>,
    pub groups: Vec<StartupOwnerGroup>,
    pub coverage: Vec<StartupSourceCoverage>,
    pub summary: StartupCatalogSummary,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupDesiredState {
    Enabled,
    Disabled,
    Removed,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartupChangeSelection {
    pub scan_id: String,
    pub item_ids: Vec<String>,
    pub desired_state: StartupDesiredState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupChangeWarning {
    AffectsOtherTriggers,
    ItemCurrentlyRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupChangeSkipReason {
    AlreadyInDesiredState,
    CatalogExpired,
    ItemChanged,
    ItemMissing,
    StateUnknown,
    UnsupportedCapability,
    RequiresElevation,
    TargetUnavailable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupChangePlanItem {
    pub item_id: String,
    pub display_name: String,
    pub source_kind: StartupSourceKind,
    pub scope: StartupScope,
    pub previous_state: StartupConfiguredState,
    pub desired_state: StartupDesiredState,
    pub warnings: Vec<StartupChangeWarning>,
    pub requires_elevation: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupChangeSkippedItem {
    pub item_id: String,
    pub display_name: String,
    pub reason: StartupChangeSkipReason,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupChangePlan {
    pub schema_version: u32,
    pub plan_id: String,
    pub scan_id: String,
    pub catalog_revision: String,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub desired_state: StartupDesiredState,
    pub items: Vec<StartupChangePlanItem>,
    pub skipped_items: Vec<StartupChangeSkippedItem>,
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupChangeOutcomeStatus {
    Changed,
    Unchanged,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupChangeFailureReason {
    ItemChanged,
    PermissionDenied,
    UserCancelled,
    Unsupported,
    VerificationFailed,
    PlatformFailure,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupChangeItemResult {
    pub item_id: String,
    pub status: StartupChangeOutcomeStatus,
    pub configured_state: StartupConfiguredState,
    pub verified: bool,
    pub failure_reason: Option<StartupChangeFailureReason>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupChangeResult {
    pub plan_id: String,
    pub changed_count: u64,
    pub failed_count: u64,
    pub items: Vec<StartupChangeItemResult>,
    pub catalog: Option<StartupCatalog>,
}
