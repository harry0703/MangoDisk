use serde::{Deserialize, Serialize};

// Only catalog identifiers, enum values and numbers may cross this diagnostic boundary.
// PowerShell messages can contain user paths or localized text and must never reach the log.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MaintenanceStepDiagnostic {
    pub index: u8,
    pub stage: MaintenanceStage,
    pub exit_code: Option<i32>,
    pub output_bytes: u64,
    pub output_digest: Option<[u8; 32]>,
    pub component: MaintenanceComponent,
    pub action: MaintenanceAction,
    pub result: MaintenanceStepResult,
    pub before: Option<ServiceStatus>,
    pub after: Option<ServiceStatus>,
    pub startup: Option<ServiceStartup>,
    pub state_query_failed: bool,
    pub native_error: Option<i32>,
    pub hresult: Option<i32>,
    pub root_hresult: Option<i32>,
    pub error_category: Option<u32>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum MaintenanceComponent {
    #[serde(rename = "bits")]
    Bits,
    #[serde(rename = "cryptsvc")]
    CryptSvc,
    #[serde(rename = "wuauserv")]
    WindowsUpdate,
    #[serde(rename = "usoClient")]
    UsoClient,
    WSearch,
    Spooler,
    Audiosrv,
    W32Time,
    Dism,
    Sfc,
    Lodctr,
    LodctrWow,
    Winmgmt,
    PerformanceData,
    DiskCheck,
    Ipconfig,
    StoreCache,
    SearchSetting,
    PrintQueue,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum MaintenanceStage {
    Execute,
    Verify,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum MaintenanceAction {
    Query,
    Restart,
    Start,
    RequestScan,
    Stop,
    Run,
    Write,
    Delete,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum MaintenanceStepResult {
    Started,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum ServiceStatus {
    Stopped,
    StartPending,
    StopPending,
    Running,
    ContinuePending,
    PausePending,
    Paused,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum ServiceStartup {
    Boot,
    System,
    Automatic,
    Manual,
    Disabled,
}

/// Native exit codes are not universally Win32 errors. Only the native-error field
/// (an actual Win32 exception/HRESULT) is classified; tool exit codes remain evidence.
pub(crate) fn classify_failure(record: &MaintenanceStepDiagnostic) -> crate::PlatformError {
    use crate::{PlatformError, PlatformErrorCode, PlatformFailureReason as Reason};
    // A missing cleanup target or registry key is not a missing system executable.
    let missing_tool = matches!(
        record.action,
        MaintenanceAction::Run | MaintenanceAction::RequestScan
    );
    let code = match record.native_error {
        Some(5 | 1314) => PlatformErrorCode::AccessDenied,
        Some(2 | 3) if missing_tool => PlatformErrorCode::Unsupported,
        // PowerShell ErrorCategory.PermissionDenied is stable and independent of UI language.
        None if record.error_category == Some(18) => PlatformErrorCode::AccessDenied,
        _ => PlatformErrorCode::OperationFailed,
    };
    let mut error = PlatformError::new(code, format!(
        "maintenance step failed: component={:?} action={:?} stage={:?} native_error={:?} exit_code={:?} hresult={:?} root_hresult={:?} output_digest={:?}",
        record.component, record.action, record.stage, record.native_error, record.exit_code,
        record.hresult, record.root_hresult, record.output_digest,
    ));
    let reason = if record.stage == MaintenanceStage::Verify {
        Some(if code == PlatformErrorCode::AccessDenied {
            Reason::VerificationPermissionDenied
        } else {
            Reason::VerificationFailed
        })
    } else {
        match record.native_error {
            Some(2 | 3) if missing_tool => Some(Reason::ToolUnavailable),
            Some(1058) => Some(Reason::ServiceDisabled),
            Some(1060) => Some(Reason::ServiceUnavailable),
            Some(1068 | 1075) => Some(Reason::DependencyUnavailable),
            Some(1051 | 1061) => Some(Reason::ServiceBusy),
            Some(1053 | 1460) => Some(Reason::TimedOut),
            _ => None,
        }
    };
    if let Some(reason) = reason {
        error = error.with_failure_reason(reason);
    }
    error
}

pub(crate) fn log_step_diagnostics(
    session_id: &str,
    task_id: &str,
    request_id: u64,
    records: &[MaintenanceStepDiagnostic],
) {
    for record in records {
        let output_digest = record
            .output_digest
            .map(|bytes| {
                bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            })
            .unwrap_or_else(|| "none".to_owned());
        log::info!(
            "windows_maintenance_step_result session_id={} task_id={} request_id={} step={} component={:?} action={:?} stage={:?} result={:?} status_before={:?} status_after={:?} startup={:?} state_query_failed={} native_error_code={:?} exit_code={:?} hresult={:?} root_hresult={:?} error_category={:?} output_bytes={} output_digest={} elapsed_ms={}",
            session_id, task_id, request_id, record.index, record.component, record.action, record.stage, record.result,
            record.before, record.after, record.startup, record.state_query_failed, record.native_error, record.exit_code, record.hresult,
            record.root_hresult, record.error_category, record.output_bytes, output_digest, record.elapsed_ms,
        );
    }
    if records.is_empty() {
        log::warn!("windows_maintenance_step_diagnostics_missing session_id={session_id} task_id={task_id} request_id={request_id}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAILURE: &str = r#"{"index":1,"stage":"Execute","exitCode":null,"outputBytes":0,"outputDigest":null,"component":"wuauserv","action":"Start","result":"Failed","before":"Stopped","after":"Stopped","startup":"Disabled","stateQueryFailed":false,"nativeError":1058,"hresult":-2146233087,"rootHresult":-2147467259,"errorCategory":7,"elapsedMs":42}"#;

    #[test]
    fn preserves_service_failure_evidence_across_the_helper_protocol() {
        let record: MaintenanceStepDiagnostic = serde_json::from_str(FAILURE).unwrap();
        assert_eq!(record.component, MaintenanceComponent::WindowsUpdate);
        assert_eq!(record.startup, Some(ServiceStartup::Disabled));
        assert_eq!(record.native_error, Some(1058));
        assert_eq!(record.result, MaintenanceStepResult::Failed);
        let encoded = serde_json::to_string(&record).unwrap();
        assert_eq!(
            serde_json::from_str::<MaintenanceStepDiagnostic>(&encoded).unwrap(),
            record
        );
    }

    #[test]
    fn native_classification_is_shared_and_does_not_guess_from_tool_exit_codes() {
        use crate::{PlatformErrorCode, PlatformFailureReason as Reason};
        let mut record: MaintenanceStepDiagnostic = serde_json::from_str(FAILURE).unwrap();
        for (code, reason) in [
            (1058, Reason::ServiceDisabled),
            (1060, Reason::ServiceUnavailable),
            (1068, Reason::DependencyUnavailable),
            (1061, Reason::ServiceBusy),
            (1053, Reason::TimedOut),
        ] {
            record.native_error = Some(code);
            assert_eq!(classify_failure(&record).failure_reason(), Some(reason));
        }
        record.native_error = Some(2);
        record.action = MaintenanceAction::Run;
        assert_eq!(
            classify_failure(&record).failure_reason(),
            Some(Reason::ToolUnavailable)
        );
        record.action = MaintenanceAction::Delete;
        assert_eq!(classify_failure(&record).failure_reason(), None);
        assert_eq!(
            classify_failure(&record).code(),
            PlatformErrorCode::OperationFailed
        );
        record.native_error = Some(5);
        assert_eq!(
            classify_failure(&record).code(),
            PlatformErrorCode::AccessDenied
        );
        record.stage = MaintenanceStage::Verify;
        assert_eq!(
            classify_failure(&record).failure_reason(),
            Some(Reason::VerificationPermissionDenied)
        );
        record.native_error = None;
        record.error_category = Some(18);
        assert_eq!(
            classify_failure(&record).code(),
            PlatformErrorCode::AccessDenied
        );
        record.error_category = None;
        record.exit_code = Some(5);
        assert_eq!(
            classify_failure(&record).code(),
            PlatformErrorCode::OperationFailed
        );
        assert_eq!(
            classify_failure(&record).failure_reason(),
            Some(Reason::VerificationFailed)
        );
    }

    #[test]
    fn rejects_private_text_and_unknown_diagnostic_values() {
        for invalid in [
            FAILURE.replace("wuauserv", "private-service-name"),
            FAILURE.replace("Failed", "raw exception message"),
            FAILURE.replace(
                "\"elapsedMs\":42",
                "\"elapsedMs\":42,\"message\":\"private path\"",
            ),
            FAILURE.replace("1058", "\"private path\""),
        ] {
            assert!(serde_json::from_str::<MaintenanceStepDiagnostic>(&invalid).is_err());
        }
    }
}
