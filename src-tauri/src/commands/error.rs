use std::{any::Any, collections::BTreeMap, fmt::Display};

use serde::Serialize;

use mangodisk_core::{CoreError, CoreErrorCode};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandErrorCode {
    InvalidInput,
    OperationBusy,
    OperationCancelled,
    OperationFailed,
    PermissionDenied,
    PersistenceFailed,
    TaskJoinFailed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: CommandErrorCode,
    pub details: BTreeMap<&'static str, &'static str>,
    pub retryable: bool,
}

pub type CommandResult<T> = Result<T, CommandError>;

/// Converts an adapter result into the stable command error protocol when the
/// native operation is already non-blocking and does not need a worker task.
pub fn into_command_result<T, E>(operation: &'static str, result: Result<T, E>) -> CommandResult<T>
where
    E: Any + Display,
{
    result.map_err(|error| CommandError::operation(operation, error))
}

impl CommandError {
    pub(super) fn invalid_input(operation: &'static str) -> Self {
        Self::new(CommandErrorCode::InvalidInput, operation, false)
    }

    fn operation<E>(operation: &'static str, error: E) -> Self
    where
        E: Any + Display,
    {
        let diagnostic = error.to_string();
        if let Some(error) = (&error as &dyn Any).downcast_ref::<CoreError>() {
            let (code, retryable) = match error.code() {
                CoreErrorCode::InvalidInput => (CommandErrorCode::InvalidInput, false),
                CoreErrorCode::OperationBusy => {
                    log::info!("command_deferred operation={operation} reason=operation_busy");
                    (CommandErrorCode::OperationBusy, true)
                }
                CoreErrorCode::OperationCancelled => (CommandErrorCode::OperationCancelled, false),
                CoreErrorCode::PermissionDenied => (CommandErrorCode::PermissionDenied, false),
                CoreErrorCode::Persistence => (CommandErrorCode::PersistenceFailed, true),
                CoreErrorCode::OperationFailed | CoreErrorCode::Platform => {
                    (CommandErrorCode::OperationFailed, true)
                }
            };

            match error.code() {
                CoreErrorCode::OperationBusy => {}
                CoreErrorCode::OperationCancelled => {
                    log::info!("command_cancelled operation={operation}");
                }
                _ => {
                    log::error!(
                        "command_failed operation={operation} code={:?} error={diagnostic}",
                        error.code()
                    );
                }
            }
            let mut command_error = Self::new(code, operation, retryable);
            if let Some(reason) = error.reason() {
                command_error.details.insert("reason", reason.as_str());
            }
            // A failed postflight can follow a successful write. Preserve that uncertainty so
            // adapters refresh native state instead of presenting a retry against stale data.
            if error.mutation_state() == mangodisk_platform::PlatformMutationState::MayHaveChanged {
                command_error
                    .details
                    .insert("mutationState", "mayHaveChanged");
            }
            return command_error;
        }

        log::error!("command_failed operation={operation} error={diagnostic}");
        Self::new(CommandErrorCode::OperationFailed, operation, true)
    }

    fn task_join(operation: &'static str, error: impl Display) -> Self {
        log::error!("command_worker_join_failed operation={operation} error={error}");
        Self::new(CommandErrorCode::TaskJoinFailed, operation, true)
    }

    fn new(code: CommandErrorCode, operation: &'static str, retryable: bool) -> Self {
        Self {
            code,
            details: BTreeMap::from([("operation", operation)]),
            retryable,
        }
    }
}

/// Runs blocking domain work without leaking platform diagnostics across the
/// Tauri boundary. Full errors remain in the native log while the UI receives
/// a stable code that it can localize independently.
pub async fn run_blocking<T, E, F>(operation: &'static str, task: F) -> CommandResult<T>
where
    T: Send + 'static,
    E: Display + Send + 'static,
    F: FnOnce() -> Result<T, E> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| CommandError::task_join(operation, error))?
        .map_err(|error| CommandError::operation(operation, error))
}

pub async fn run_blocking_value<T, F>(operation: &'static str, task: F) -> CommandResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| CommandError::task_join(operation, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_error_serialization_excludes_internal_diagnostics() {
        let error = CommandError::operation("scan_cleanup_candidates", "sensitive path");
        let json = serde_json::to_value(error).expect("command errors must serialize");

        assert_eq!(json["code"], "operationFailed");
        assert_eq!(json["details"]["operation"], "scan_cleanup_candidates");
        assert_eq!(json["retryable"], true);
        assert!(!json.to_string().contains("sensitive path"));
    }

    #[test]
    fn operation_contention_uses_stable_busy_code() {
        let error = CommandError::operation(
            "scan_application_uninstall_catalog",
            CoreError::operation_busy(
                "another MangoDisk operation is already running: cleanup_scan (1)",
            ),
        );
        let json = serde_json::to_value(error).expect("command errors must serialize");

        assert_eq!(json["code"], "operationBusy");
        assert!(!json.to_string().contains("cleanup_scan"));
    }

    #[test]
    fn worker_cleanup_busy_reason_is_forwarded_without_native_diagnostics() {
        let error = CommandError::operation(
            "analyze_path",
            CoreError::operation_busy("native worker detail")
                .with_reason(mangodisk_core::CoreErrorReason::ScanResourcesReleasing),
        );
        let json = serde_json::to_value(error).expect("command errors must serialize");

        assert_eq!(json["code"], "operationBusy");
        assert_eq!(json["details"]["reason"], "scanResourcesReleasing");
        assert!(!json.to_string().contains("native worker detail"));
    }

    #[test]
    fn cancellation_uses_the_stable_non_retryable_code() {
        let error = CommandError::operation("analyze_path", CoreError::operation_cancelled());
        let json = serde_json::to_value(error).expect("command errors must serialize");

        assert_eq!(json["code"], "operationCancelled");
        assert_eq!(json["retryable"], false);
    }

    #[test]
    fn permission_errors_are_not_reported_as_retryable_failures() {
        let error = CommandError::operation(
            "execute_cleanup",
            CoreError::new(CoreErrorCode::PermissionDenied, "private path"),
        );
        let json = serde_json::to_value(error).expect("command errors must serialize");

        assert_eq!(json["code"], "permissionDenied");
        assert_eq!(json["retryable"], false);
        assert!(!json.to_string().contains("private path"));
    }

    #[test]
    fn stable_failure_reason_is_forwarded_without_native_diagnostics() {
        let error = CommandError::operation(
            "delete_analysis_entry_permanently",
            CoreError::operation_failed("private native diagnostic")
                .with_reason(mangodisk_core::CoreErrorReason::ResourceBusy),
        );
        let json = serde_json::to_value(error).expect("command errors must serialize");

        assert_eq!(json["code"], "operationFailed");
        assert_eq!(json["details"]["reason"], "resourceBusy");
        assert!(!json.to_string().contains("private native diagnostic"));
    }
    #[test]
    fn native_cancellation_and_uncertain_writes_reach_the_frontend() {
        let cancelled = CoreError::from(mangodisk_platform::PlatformError::new(
            mangodisk_platform::PlatformErrorCode::UserCancelled,
            "private cancellation detail",
        ));
        let json = serde_json::to_value(CommandError::operation(
            "remove_application_record",
            cancelled,
        ))
        .unwrap();
        assert_eq!(json["code"], "operationCancelled");
        assert_eq!(json["retryable"], false);
        assert!(json["details"].get("mutationState").is_none());
        let uncertain = CoreError::from(
            mangodisk_platform::PlatformError::operation_failed("private verification detail")
                .with_possible_side_effects(),
        );
        let json = serde_json::to_value(CommandError::operation(
            "remove_application_record",
            uncertain,
        ))
        .unwrap();
        assert_eq!(json["details"]["mutationState"], "mayHaveChanged");
        assert!(!json.to_string().contains("private"));
    }
}
