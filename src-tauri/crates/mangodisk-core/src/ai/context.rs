use serde::{Deserialize, Serialize};

use super::AiError;
use crate::cleanup::{CleanupSourceDetail, RiskLevel, ScanItemStatus};
use crate::privacy::{
    PrivacyCapabilityState, PrivacyDataKind, PrivacyImpact, PrivacyRecommendation, PrivacyTimeRange,
};
use crate::startup::{
    StartupConfiguredState, StartupControlCapability, StartupDiagnosticCode, StartupRuntimeState,
    StartupSourceKind, StartupTrigger, StartupTrustState,
};
use crate::system_maintenance::{SystemMaintenanceRiskLevel, SystemMaintenanceStatus};
use crate::system_settings::{
    SystemSettingRiskLevel, SystemSettingSelectionKind, SystemSettingStatus,
    SystemSettingTargetState,
};
use mangodisk_platform::{
    PlatformSystemMaintenanceDiagnosticCode, PlatformSystemSettingDiagnosticCode,
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AiPlatform {
    Macos,
    Windows,
    Unknown,
}

/// Transient IPC schema. Older preview contexts are rejected, not migrated;
/// provider configuration has its own independent persisted schema.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiContext {
    pub schema_version: u8,
    pub platform: AiPlatform,
    pub title: String,
    pub description: String,
    pub subject: AiSubject,
}

/// Domain enum types are reused, but operational objects and their private fields
/// never cross this allowlist. The subject tag also selects the module prompt.
#[derive(Clone, Deserialize, Serialize)]
#[serde(
    tag = "module",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AiSubject {
    Cleanup {
        impact: String,
        bytes: u64,
        item_count: u64,
        requires_app_close: bool,
        scan: AiCleanupScan,
    },
    Privacy {
        kind: PrivacyDataKind,
        impact: PrivacyImpact,
        capability: PrivacyCapabilityState,
        recommendation: PrivacyRecommendation,
        time_range: PrivacyTimeRange,
        item_count: u64,
        estimated_bytes: u64,
        requires_browser_close: bool,
        synchronization_may_propagate: bool,
    },
    Startup {
        entries: Vec<AiStartupEntry>,
        omitted_count: u64,
    },
    SystemOptimization {
        status: SystemSettingStatus,
        selection_kind: SystemSettingSelectionKind,
        diagnostic: Option<PlatformSystemSettingDiagnosticCode>,
        risk_level: SystemSettingRiskLevel,
        has_recorded_original_value: bool,
        requires_restart: bool,
        requires_elevation: bool,
        pending_target: Option<SystemSettingTargetState>,
    },
    SystemMaintenance {
        task_id: String,
        status: SystemMaintenanceStatus,
        risk_level: SystemMaintenanceRiskLevel,
        requires_restart: bool,
        requires_elevation: bool,
        estimated_duration_seconds: u64,
        diagnostic: Option<PlatformSystemMaintenanceDiagnosticCode>,
    },
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiCleanupScan {
    pub rule_id: String,
    pub risk: RiskLevel,
    pub status: ScanItemStatus,
    pub available: bool,
    pub selectable: bool,
    pub running_processes: Vec<String>,
    pub sources: Vec<CleanupSourceDetail>,
    pub source_count: u64,
    pub sources_truncated: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiStartupEntry {
    pub name: String,
    pub identity: AiStartupIdentity,
    pub source_kind: StartupSourceKind,
    pub triggers: Vec<StartupTrigger>,
    pub configured_state: StartupConfiguredState,
    pub runtime_state: StartupRuntimeState,
    pub control_capability: StartupControlCapability,
    pub diagnostics: Vec<StartupDiagnosticCode>,
    pub removal_supported: bool,
}

/// Preserve original software metadata for attribution. These are untrusted
/// descriptions, never instructions, and must not enter diagnostic logs.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiStartupIdentity {
    pub application_name: String,
    pub publisher: String,
    pub executable_name: String,
    pub executable_path: String,
    pub configuration_path: String,
    pub description: String,
    pub version: String,
    pub trust: StartupTrustState,
}

impl AiStartupIdentity {
    fn valid(&self) -> bool {
        [
            &self.application_name,
            &self.publisher,
            &self.executable_name,
            &self.executable_path,
            &self.configuration_path,
            &self.description,
            &self.version,
        ]
        .into_iter()
        .all(|value| value.len() <= 32768)
    }
}

impl AiSubject {
    pub fn module_name(&self) -> &'static str {
        match self {
            Self::Cleanup { .. } => "cleanup",
            Self::Privacy { .. } => "privacy",
            Self::Startup { .. } => "startup",
            Self::SystemOptimization { .. } => "systemOptimization",
            Self::SystemMaintenance { .. } => "systemMaintenance",
        }
    }
}

impl AiContext {
    pub(super) fn validate(&self) -> Result<(), AiError> {
        let invalid_subject = match &self.subject {
            AiSubject::Cleanup { impact, scan, .. } => {
                impact.len() > 32768
                    || scan.rule_id.len() > 1024
                    || scan.sources.len() > 256
                    || scan.running_processes.len() > 256
                    || scan.running_processes.iter().any(|name| name.len() > 32768)
                    || scan.sources.iter().any(|source| source.path.len() > 32768)
            }
            AiSubject::Startup { entries, .. } => {
                entries.is_empty()
                    || entries.len() > 256
                    || entries.iter().any(|entry| {
                        entry.name.trim().is_empty()
                            || entry.name.len() > 32768
                            || entry.triggers.len() > 8
                            || entry.diagnostics.len() > 6
                            || !entry.identity.valid()
                    })
            }
            AiSubject::SystemMaintenance { task_id, .. } => {
                task_id.is_empty() || task_id.len() > 256
            }
            _ => false,
        };
        if self.schema_version != 2
            || self.title.trim().is_empty()
            || self.title.len() > 32768
            || self.description.len() > 2048
            || invalid_subject
        {
            return Err(AiError::InvalidContext);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixtures() -> Vec<serde_json::Value> {
        serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/ai-context-v2.json"
        ))
        .unwrap()
    }

    #[test]
    fn frontend_contexts_round_trip_with_explicit_metadata_fields() {
        for fixture in fixtures() {
            let context: AiContext = serde_json::from_value(fixture.clone()).unwrap();
            context.validate().unwrap();
            assert_eq!(serde_json::to_value(&context).unwrap(), fixture);
            let mut invalid = fixture.clone();
            invalid["subject"]["command"] = serde_json::json!("/private/command");
            assert!(serde_json::from_value::<AiContext>(invalid).is_err());
            let mut invalid = fixture;
            invalid["path"] = serde_json::json!("/private/path");
            assert!(serde_json::from_value::<AiContext>(invalid).is_err());
        }
    }

    #[test]
    fn startup_identity_preserves_paths_but_rejects_unbounded_or_execution_fields() {
        let mut fixture = fixtures().remove(2);
        fixture["subject"]["entries"][0]["identity"]["executablePath"] =
            serde_json::json!("C:\\Users\\Example\\Applications\\Agent.exe");
        let context: AiContext = serde_json::from_value(fixture.clone()).unwrap();
        context.validate().unwrap();
        assert_eq!(serde_json::to_value(context).unwrap(), fixture);
        let mut oversized = fixture.clone();
        oversized["subject"]["entries"][0]["identity"]["description"] =
            serde_json::json!("x".repeat(32769));
        assert_eq!(
            serde_json::from_value::<AiContext>(oversized)
                .unwrap()
                .validate(),
            Err(AiError::InvalidContext)
        );
        fixture["subject"]["entries"][0]["identity"]["arguments"] =
            serde_json::json!(["--secret=value"]);
        assert!(serde_json::from_value::<AiContext>(fixture).is_err());
    }

    #[test]
    fn cleanup_sources_round_trip_without_silent_redaction_or_truncation() {
        let mut fixture = fixtures().remove(0);
        fixture["subject"]["scan"]["sources"] = serde_json::json!([{
            "path": "C:\\Users\\Example\\Application Data\\Vendor\\cache.bin",
            "bytes": 128,
            "fileCount": 1,
            "modifiedAtMs": null,
            "blockReason": "requiresClose"
        }]);
        fixture["subject"]["scan"]["sourceCount"] = serde_json::json!(300);
        fixture["subject"]["scan"]["sourcesTruncated"] = serde_json::json!(true);
        let mut context: AiContext = serde_json::from_value(fixture.clone()).unwrap();
        context.validate().unwrap();
        assert_eq!(serde_json::to_value(&context).unwrap(), fixture);
        if let AiSubject::Cleanup { scan, .. } = &mut context.subject {
            scan.sources = vec![scan.sources[0].clone(); 257];
        }
        assert_eq!(context.validate(), Err(AiError::InvalidContext));
    }

    #[test]
    fn unsupported_schema_empty_titles_invalid_enums_and_unbounded_groups_are_rejected() {
        let fixture = fixtures().remove(2);
        let mut context: AiContext = serde_json::from_value(fixture.clone()).unwrap();
        context.schema_version = 1;
        assert_eq!(context.validate(), Err(AiError::InvalidContext));
        context.schema_version = 2;
        context.title = " ".into();
        assert_eq!(context.validate(), Err(AiError::InvalidContext));
        context.title = "Fixture".into();
        if let AiSubject::Startup { entries, .. } = &mut context.subject {
            *entries = vec![entries[0].clone(); 257];
        }
        assert_eq!(context.validate(), Err(AiError::InvalidContext));
        let mut invalid = fixture;
        invalid["subject"]["entries"][0]["controlCapability"] = serde_json::json!("freelyMutable");
        assert!(serde_json::from_value::<AiContext>(invalid).is_err());
    }
}
