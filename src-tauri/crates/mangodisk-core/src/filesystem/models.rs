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
    /// Entries already absent are reconciled without counting reclaimed storage.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub already_absent_paths: Vec<String>,
    pub failed: Vec<PermanentDeleteFailure>,
    /// True when any completed directory used scan-time storage accounting.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub released_bytes_is_estimate: bool,
    /// Physical storage released by successful selections, or its marked estimate.
    pub released_bytes: u64,
}

#[cfg(test)]
mod batch_protocol_tests {
    use super::*;

    #[test]
    fn direct_batch_protocol_keeps_absence_and_estimate_metadata_optional() {
        let mut result = PermanentDeleteBatchResult {
            removed_paths: vec![],
            already_absent_paths: vec![],
            failed: vec![],
            released_bytes: 0,
            released_bytes_is_estimate: false,
        };
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 3);
        result.already_absent_paths.push("/fixture/missing".into());
        result.released_bytes_is_estimate = true;
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(
            value["alreadyAbsentPaths"],
            serde_json::json!(["/fixture/missing"])
        );
        assert_eq!(value["releasedBytesIsEstimate"], true);
        assert_eq!(value["releasedBytes"], 0);
        assert_eq!(value["removedPaths"], serde_json::json!([]));
    }
}
