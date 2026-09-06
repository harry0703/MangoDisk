use serde::{Deserialize, Serialize};

use crate::{filesystem::DiskInfo, history::OperationRecord, ApplicationCloseMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Safe,
    Recoverable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanupCategory {
    Custom,
    System,
    Browser,
    Application,
    Development,
    Project,
    Xcode,
    ApplicationOptimization,
    Ai,
    Container,
}

/// Product-facing result groups remain separate from rule ownership.
///
/// A browser or development rule can still own its domain behavior while its
/// rebuildable macOS cache is presented together with other user caches. This
/// keeps contributor-facing rule categories stable and gives every adapter a
/// consistent grouping contract without inferring semantics from rule IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanupGroup {
    Custom,
    System,
    UserCache,
    Browser,
    Application,
    Development,
    Project,
    Xcode,
    ApplicationOptimization,
    Ai,
    Container,
}

impl From<CleanupCategory> for CleanupGroup {
    fn from(category: CleanupCategory) -> Self {
        match category {
            CleanupCategory::Custom => Self::Custom,
            CleanupCategory::System => Self::System,
            CleanupCategory::Browser => Self::Browser,
            CleanupCategory::Application => Self::Application,
            CleanupCategory::Development => Self::Development,
            CleanupCategory::Project => Self::Project,
            CleanupCategory::Xcode => Self::Xcode,
            CleanupCategory::ApplicationOptimization => Self::ApplicationOptimization,
            CleanupCategory::Ai => Self::Ai,
            CleanupCategory::Container => Self::Container,
        }
    }
}

impl CleanupGroup {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Custom => "custom",
            Self::System => "system",
            Self::UserCache => "userCache",
            Self::Browser => "browser",
            Self::Application => "application",
            Self::Development => "development",
            Self::Project => "project",
            Self::Xcode => "xcode",
            Self::ApplicationOptimization => "applicationOptimization",
            Self::Ai => "ai",
            Self::Container => "container",
        }
    }
}

impl CleanupCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Custom => "custom",
            Self::System => "system",
            Self::Browser => "browser",
            Self::Application => "application",
            Self::Development => "development",
            Self::Project => "project",
            Self::Xcode => "xcode",
            Self::ApplicationOptimization => "applicationOptimization",
            Self::Ai => "ai",
            Self::Container => "container",
        }
    }
}

impl TryFrom<&str> for CleanupCategory {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "custom" => Ok(Self::Custom),
            "system" => Ok(Self::System),
            "browser" => Ok(Self::Browser),
            "application" => Ok(Self::Application),
            "development" => Ok(Self::Development),
            "project" => Ok(Self::Project),
            "xcode" => Ok(Self::Xcode),
            "applicationOptimization" => Ok(Self::ApplicationOptimization),
            "ai" => Ok(Self::Ai),
            "container" => Ok(Self::Container),
            _ => Err(format!("unsupported cleanup category: {value}")),
        }
    }
}

pub const CUSTOM_CLEANUP_RULE_SCHEMA_VERSION: u32 = 1;

/// User-authored filename rules supplied by an adapter for one scan and cleanup
/// session. Core validates every field and resolves every root again before it
/// can enter the trusted cleanup plan.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomCleanupRule {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub roots: Vec<String>,
    pub name_patterns: Vec<String>,
    pub minimum_bytes: Option<u64>,
    pub maximum_bytes: Option<u64>,
    pub modified_time: CustomCleanupModifiedTime,
    pub recursive: bool,
    #[serde(default)]
    pub remove_empty_directories: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum CustomCleanupModifiedTime {
    Any,
    OlderThan { days: u64 },
    NewerThan { days: u64 },
}

#[cfg(test)]
mod cleanup_category_tests {
    use super::{CleanupCategory, CleanupGroup, CustomCleanupRule};

    #[test]
    fn multiword_category_keeps_the_camel_case_wire_value() {
        let encoded = serde_json::to_string(&CleanupCategory::ApplicationOptimization)
            .expect("cleanup category must serialize");

        assert_eq!(encoded, r#""applicationOptimization""#);
        assert_eq!(
            CleanupCategory::try_from("applicationOptimization"),
            Ok(CleanupCategory::ApplicationOptimization)
        );
    }

    #[test]
    fn result_group_keeps_the_camel_case_wire_value() {
        let encoded = serde_json::to_string(&CleanupGroup::UserCache)
            .expect("cleanup result group must serialize");

        assert_eq!(encoded, r#""userCache""#);
        assert_eq!(CleanupGroup::UserCache.as_str(), "userCache");
    }

    #[test]
    fn older_custom_rules_disable_empty_directory_removal_by_default() {
        let rule: CustomCleanupRule = serde_json::from_str(
            r#"{
                "schemaVersion": 1,
                "id": "legacy-rule",
                "name": "Legacy rule",
                "roots": ["/tmp/mangodisk-custom-cleanup"],
                "namePatterns": ["*.tmp"],
                "minimumBytes": null,
                "maximumBytes": null,
                "modifiedTime": { "mode": "any" },
                "recursive": true
            }"#,
        )
        .expect("rules saved by earlier releases must remain compatible");

        assert!(!rule.remove_empty_directories);
    }
}

/// Distinguishes an inspected clean rule from a rule that could not be inspected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScanItemStatus {
    Found,
    Clean,
    NotApplicable,
    RequiresClose,
    /// The item is intentionally visible for inspection but cannot be added
    /// to a cleanup request. This differs from `Limited`, which means the scan
    /// could not establish a complete result.
    ReviewOnly,
    Limited,
    /// The target is known to exist, but measuring or changing it requires an explicit elevation
    /// boundary. Adapters may offer a user-initiated privileged refresh without elevating the
    /// complete application process.
    RequiresElevation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupSourceDetail {
    pub path: String,
    /// Logical bytes matched by this rule. Cleanup sources can include sparse, compressed, shared,
    /// or tool-reported data, so adapters must present this value as an estimate rather than an
    /// exact change in volume free space.
    pub bytes: u64,
    pub file_count: u64,
    /// Latest modification time among content matched under this source.
    pub modified_at_ms: Option<u64>,
    /// Explains why a discovered source cannot be processed immediately.
    ///
    /// This status belongs to the measured source rather than the cleanup
    /// rule. Keeping it in the scan result lets the UI explain temporary
    /// execution requirements without promoting them to separate rules.
    pub block_reason: Option<CleanupSourceBlockReason>,
}

/// Native application-icon source associated with a running process identity.
///
/// The scan response keeps only the local source path, not encoded artwork.
/// Adapters resolve the image lazily when the close confirmation is visible so
/// routine scans do not pay the icon decoding or transport cost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupApplicationIcon {
    pub process_name: String,
    pub icon_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanupSourceBlockReason {
    RequiresClose,
    /// The source contains entries that could not be measured without
    /// crossing an access or filesystem safety boundary. Other complete
    /// sources in the same rule may still remain selectable.
    IncompleteMeasurement,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRuleResult {
    pub rule_id: String,
    pub category: CleanupCategory,
    /// Stable product grouping used by GUI and CLI result presenters.
    pub group: CleanupGroup,
    pub risk: RiskLevel,
    pub default_selected: bool,
    /// Shared desktop and CLI recommendation kept separate from automatic selection.
    pub recommended_selected: bool,
    pub bytes: u64,
    pub file_count: u64,
    pub available: bool,
    pub selectable: bool,
    pub status: ScanItemStatus,
    pub running_processes: Vec<String>,
    pub requires_app_close: bool,
    /// Largest matched source locations retained for local result inspection.
    ///
    /// The engine intentionally returns a bounded summary instead of every
    /// matched file. This keeps scan snapshots responsive and avoids exposing
    /// an unnecessarily detailed file inventory to the UI.
    pub sources: Vec<CleanupSourceDetail>,
    pub source_count: u64,
    pub sources_truncated: bool,
    /// Wall-clock time spent measuring this rule.
    ///
    /// Rules run concurrently, so individual values must not be summed to infer
    /// the total scan time. The metric is intended for regression diagnosis.
    pub scan_elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupScanResult {
    pub schema_version: String,
    /// Core-owned authorization for the exact custom rule set that produced
    /// this result. Standard scans do not publish one.
    pub custom_scan_id: Option<u64>,
    /// Missing saved roots skipped during preparation, independent from IO warnings.
    pub missing_custom_root_count: u64,
    pub scanned_at_ms: u64,
    pub disk: DiskInfo,
    pub rules: Vec<ScanRuleResult>,
    pub application_icons: Vec<CleanupApplicationIcon>,
    pub warning_count: u64,
    pub safe_bytes: u64,
    /// Aggregate logical/estimated bytes from selectable cleanup results, not a physical-volume
    /// allocation measurement. Actual free-space change can differ after cleanup.
    pub reclaimable_bytes: u64,
    /// Time spent applying low-cost applicability probes before traversal.
    pub applicability_elapsed_ms: u64,
    pub applicable_rule_count: u64,
    pub filtered_rule_count: u64,
    pub inventory_application_count: u64,
    pub inventory_process_count: u64,
    /// Total scan wall-clock time, excluding report rendering and persistence.
    pub elapsed_ms: u64,
}

/// Stable engine diagnostics recorded by baseline reports.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupScanEngineInfo {
    pub strategy: &'static str,
    pub rule_catalog_mode: &'static str,
    pub configured_worker_limit: usize,
    /// Whether derived scan results are persisted between process lifetimes.
    pub scan_result_persistence_enabled: bool,
    pub single_pass_rule_matching: bool,
    pub incremental_scan_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRequest {
    pub rule_ids: Vec<String>,
    pub dry_run: bool,
    #[serde(default)]
    pub project_roots: Vec<String>,
    /// Optional source-level overrides for selected rules.
    ///
    /// A missing override keeps the existing whole-rule behavior. Paths are
    /// untrusted UI input and must be matched against sources rediscovered by
    /// Core immediately before any mutation.
    #[serde(default)]
    pub source_selections: Vec<CleanupSourceSelection>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupApplicationCloseRequest {
    pub rule_ids: Vec<String>,
    pub mode: ApplicationCloseMode,
}

/// Stable product-level stages for cleanup execution.
///
/// The values intentionally avoid exposing platform commands or filesystem
/// paths across the Core boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanupExecutionStage {
    Validating,
    Cleaning,
    Finalizing,
}

/// A completed rule summary retained in later execution snapshots.
///
/// Keeping the bounded rule result list in the progress protocol lets adapters
/// show truthful completion feedback without waiting for the entire operation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupExecutionRuleResult {
    pub rule_id: String,
    pub status: CleanupActionStatus,
    pub affected_item_count: u64,
    pub released_bytes: u64,
}

/// A throttled progress snapshot emitted during cleanup execution.
///
/// Only the currently handled path is exposed, and it is neither persisted nor
/// logged. Core throttles these snapshots so large directories provide useful
/// feedback without making WebView events compete with filesystem work.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupExecutionProgress {
    pub stage: CleanupExecutionStage,
    /// Stable rule queue in the exact order used by the execution pipeline.
    /// The desktop keeps this order fixed while individual rule states change.
    pub planned_rule_ids: Vec<String>,
    pub current_rule_id: Option<String>,
    pub current_item_path: Option<String>,
    pub current_rule_affected_item_count: u64,
    pub current_rule_released_bytes: u64,
    pub completed_rule_results: Vec<CleanupExecutionRuleResult>,
    pub validated_rule_count: u64,
    pub completed_rule_count: u64,
    pub total_rule_count: u64,
    pub checked_item_count: u64,
    pub checked_bytes: u64,
    pub affected_item_count: u64,
    pub released_bytes: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanupSourceSelectionMode {
    Include,
    Exclude,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupSourceSelection {
    pub rule_id: String,
    pub mode: CleanupSourceSelectionMode,
    pub paths: Vec<String>,
}

pub const CLEANUP_PLAN_SCHEMA_VERSION: u32 = 1;
pub const CLEANUP_AUTOMATION_PROFILE_SCHEMA_VERSION: u32 = 1;

/// Versioned limits for unattended cleanup planning.
///
/// Profiles never promote recoverable or non-default rules. They only reduce
/// the low-risk default set selected by the same scan used by the GUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupAutomationProfile {
    pub schema_version: u32,
    pub max_rule_count: usize,
    pub max_reclaimable_bytes: Option<u64>,
    #[serde(default)]
    pub excluded_rule_ids: Vec<String>,
}

impl Default for CleanupAutomationProfile {
    fn default() -> Self {
        Self {
            schema_version: CLEANUP_AUTOMATION_PROFILE_SCHEMA_VERSION,
            max_rule_count: 100,
            max_reclaimable_bytes: None,
            excluded_rule_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupPlan {
    pub schema_version: u32,
    pub plan_id: String,
    pub plan_hash: String,
    pub created_at_ms: u64,
    pub source_scan_at_ms: u64,
    pub source_scan_schema_version: String,
    pub rule_ids: Vec<String>,
    pub expected_bytes: u64,
    pub profile: CleanupAutomationProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanupActionReason {
    Cancelled,
    RunningProcesses,
    ItemsSkipped,
    RequiredToolUnavailable,
    PreflightFailed,
    ExecutionFailed,
    VerificationFailed,
    CleanerUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanupActionKind {
    Delete,
    Command,
    Optimize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanupActionStatus {
    Blocked,
    Previewed,
    Completed,
    Partial,
    Failed,
}

impl CleanupActionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Previewed => "previewed",
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupActionResult {
    pub rule_id: String,
    pub action_kind: CleanupActionKind,
    pub status: CleanupActionStatus,
    /// Stable diagnostic reason rendered by the UI in the current locale.
    ///
    /// This remains optional so completed actions and pre-existing history
    /// records do not need a synthetic reason.
    #[serde(default)]
    pub reason_code: Option<CleanupActionReason>,
    pub bytes_expected: u64,
    pub released_bytes: u64,
    pub affected_item_count: u64,
    pub failed_item_count: u64,
    #[serde(default)]
    pub running_processes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResult {
    pub plan_id: String,
    pub plan_hash: String,
    pub expected_bytes: u64,
    pub released_bytes: u64,
    pub affected_item_count: u64,
    pub failed_item_count: u64,
    pub dry_run: bool,
    pub actions: Vec<CleanupActionResult>,
    pub record: OperationRecord,
    pub history_saved: bool,
}
