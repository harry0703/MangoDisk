use mangodisk_platform::VolumeInfo as NativeVolumeInfo;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeInfo {
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
}

impl From<NativeVolumeInfo> for VolumeInfo {
    fn from(value: NativeVolumeInfo) -> Self {
        Self {
            name: value.name,
            mount_point: value.mount_point,
            total_bytes: value.total_bytes,
            available_bytes: value.available_bytes,
            used_bytes: value.used_bytes,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermanentDeleteCandidate {
    pub path: String,
    /// Logical content length used only to reject stale file selections.
    pub expected_bytes: u64,
    pub expected_modified_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermanentDeleteFailure {
    pub path: String,
    #[serde(flatten)]
    pub details: PermanentDeleteFailureDetails,
}

/// Stable presentation metadata. Diagnostics never determine UI behavior.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermanentDeleteFailureDetails {
    pub message: String,
    #[serde(default)]
    pub reason: PermanentDeleteFailureReason,
    #[serde(default)]
    pub stage: PermanentDeleteFailureStage,
    #[serde(default)]
    pub native_code: Option<i32>,
    #[serde(default)]
    pub partial: bool,
    #[serde(default)]
    pub remaining_restored: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermanentDeleteFailureReason {
    AccessDenied,
    ResourceBusy,
    ItemChanged,
    Protected,
    NotFound,
    #[default]
    Other,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermanentDeleteFailureStage {
    #[default]
    Validate,
    PrepareTarget,
    CreateStaging,
    MoveToStaging,
    Remove,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermanentDeleteBatchResult {
    pub removed_paths: Vec<String>,
    pub failed: Vec<PermanentDeleteFailure>,
    /// Physical storage released by successfully removed selections.
    pub released_bytes: u64,
}
