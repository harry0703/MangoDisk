use mangodisk_platform::{PlatformSystemMaintenanceDiagnosticCode, PlatformSystemMaintenancePhase};
use serde::{Deserialize, Serialize};

pub const SYSTEM_MAINTENANCE_CATALOG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemMaintenancePlatform {
    Macos,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemMaintenanceCategory {
    Network,
    SearchAndInterface,
    SystemRepair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemMaintenanceRiskLevel {
    Standard,
    Caution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemMaintenanceStatus {
    Healthy,
    Recommended,
    Available,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMaintenanceItem {
    pub task_id: String,
    pub category: SystemMaintenanceCategory,
    pub risk_level: SystemMaintenanceRiskLevel,
    pub status: SystemMaintenanceStatus,
    pub requires_elevation: bool,
    pub requires_restart: bool,
    pub estimated_duration_seconds: u64,
    pub diagnostic: Option<PlatformSystemMaintenanceDiagnosticCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMaintenanceCatalogSummary {
    pub item_count: u64,
    pub recommended_count: u64,
    pub available_count: u64,
    pub healthy_count: u64,
    pub unavailable_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMaintenanceCatalog {
    pub schema_version: u32,
    pub scan_id: String,
    pub platform: SystemMaintenancePlatform,
    pub scanned_at_ms: u64,
    pub elapsed_ms: u64,
    pub items: Vec<SystemMaintenanceItem>,
    pub summary: SystemMaintenanceCatalogSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SystemMaintenanceExecutionRequest {
    pub scan_id: String,
    pub task_id: String,
    pub authorization_prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemMaintenanceExecutionStatus {
    Completed,
    Started,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemMaintenanceFailureReason {
    PermissionDenied,
    Unsupported,
    VerificationFailed,
    VerificationPermissionDenied,
    ToolUnavailable,
    ServiceDisabled,
    ServiceUnavailable,
    DependencyUnavailable,
    ServiceBusy,
    TimedOut,
    PlatformFailure,
    UserCancelled,
}

/// Describes the observable system-state impact of one maintenance execution.
///
/// `MayHaveChanged` is deliberately distinct from `NotChanged`: native maintenance commands can
/// fail after an earlier step has already changed system state. Preserving that uncertainty keeps
/// UI feedback and diagnostics truthful without claiming an unverified result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemMaintenanceMutationState {
    NotChanged,
    Changed,
    MayHaveChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMaintenanceExecutionItemResult {
    pub task_id: String,
    pub status: SystemMaintenanceExecutionStatus,
    pub mutation_state: SystemMaintenanceMutationState,
    pub verified: bool,
    pub requires_restart: bool,
    pub failure_reason: Option<SystemMaintenanceFailureReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemMaintenanceJobStatus {
    Queued,
    Running,
    Cancelling,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMaintenanceProgress {
    pub phase: PlatformSystemMaintenancePhase,
    pub current_step: Option<u8>,
    pub total_steps: Option<u8>,
    pub percent: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMaintenanceJob {
    pub execution_id: String,
    /// Identifies the catalog snapshot that authorized this execution. Keeping this association
    /// in the public job protocol prevents a retained terminal job from blocking the same task
    /// after a later scan has produced a new, independently verified system state.
    pub scan_id: String,
    pub task_id: String,
    /// Monotonically increases for every public state mutation. Desktop events and runtime
    /// restoration use separate IPC paths, so consumers need an ordering key that cannot tie like
    /// millisecond timestamps or regress while a job remains in the same `running` state.
    pub revision: u64,
    pub status: SystemMaintenanceJobStatus,
    pub cancelable: bool,
    pub queued_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub progress: Option<SystemMaintenanceProgress>,
    pub result: Option<SystemMaintenanceExecutionItemResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMaintenanceRuntimeState {
    pub catalog: Option<SystemMaintenanceCatalog>,
    pub executions: Vec<SystemMaintenanceJob>,
}

#[derive(Clone)]
pub(super) struct CatalogSession {
    pub public: SystemMaintenanceCatalog,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_protocol_uses_single_task_and_typed_mutation_state() {
        let request = SystemMaintenanceExecutionRequest {
            scan_id: "scan-1".to_string(),
            task_id: "macos.maintenance.dns-cache".to_string(),
            authorization_prompt: "Authorize maintenance".to_string(),
        };
        let request_json = serde_json::to_value(request).expect("request must serialize");
        assert_eq!(request_json["taskId"], "macos.maintenance.dns-cache");
        assert!(request_json.get("taskIds").is_none());

        let item = SystemMaintenanceExecutionItemResult {
            task_id: "macos.maintenance.dns-cache".to_string(),
            status: SystemMaintenanceExecutionStatus::Failed,
            mutation_state: SystemMaintenanceMutationState::MayHaveChanged,
            verified: false,
            requires_restart: false,
            failure_reason: Some(SystemMaintenanceFailureReason::PlatformFailure),
        };
        let item_json = serde_json::to_value(item).expect("result item must serialize");
        assert_eq!(item_json["mutationState"], "mayHaveChanged");
        assert!(item_json.get("changed").is_none());

        let job = SystemMaintenanceJob {
            execution_id: "maintenance-7-1".to_string(),
            scan_id: "scan-1".to_string(),
            task_id: "macos.maintenance.dns-cache".to_string(),
            revision: 1,
            status: SystemMaintenanceJobStatus::Queued,
            cancelable: true,
            queued_at_ms: 10,
            started_at_ms: None,
            finished_at_ms: None,
            progress: None,
            result: None,
        };
        let job_json = serde_json::to_value(job).expect("job must serialize");
        assert_eq!(job_json["status"], "queued");
        assert_eq!(job_json["executionId"], "maintenance-7-1");
        assert_eq!(job_json["scanId"], "scan-1");
        assert_eq!(job_json["revision"], 1);
    }
}
