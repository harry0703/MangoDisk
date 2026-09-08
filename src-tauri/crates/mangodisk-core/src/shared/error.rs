use std::{error::Error, fmt};

use mangodisk_platform::{PlatformError, PlatformErrorCode, PlatformMutationState};

/// Stable error categories shared by GUI, CLI, and automation adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreErrorCode {
    InvalidInput,
    OperationBusy,
    OperationCancelled,
    OperationFailed,
    PermissionDenied,
    Persistence,
    Platform,
}

/// Privacy-safe failure reasons that adapters may show to users.
///
/// Native diagnostics can contain private paths and file names, so they remain
/// in Core logs. These stable reasons preserve the actionable part of an error
/// without sending the original diagnostic across the process boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreErrorReason {
    ResourceBusy,
    AccessDeniedOrBusy,
    ItemChanged,
    ScanResourcesReleasing,
    QuickScanUnavailable,
}

impl CoreErrorReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResourceBusy => "resourceBusy",
            Self::AccessDeniedOrBusy => "accessDeniedOrBusy",
            Self::ItemChanged => "itemChanged",
            Self::ScanResourcesReleasing => "scanResourcesReleasing",
            Self::QuickScanUnavailable => "quickScanUnavailable",
        }
    }
}

#[derive(Debug)]
pub struct CoreError {
    code: CoreErrorCode,
    diagnostic: String,
    reason: Option<CoreErrorReason>,
    mutation_state: PlatformMutationState,
}

impl CoreError {
    pub fn new(code: CoreErrorCode, diagnostic: impl Into<String>) -> Self {
        Self {
            code,
            diagnostic: diagnostic.into(),
            reason: None,
            mutation_state: PlatformMutationState::NotAttempted,
        }
    }

    pub fn code(&self) -> CoreErrorCode {
        self.code
    }

    pub fn diagnostic(&self) -> &str {
        &self.diagnostic
    }

    pub fn reason(&self) -> Option<CoreErrorReason> {
        self.reason
    }

    /// Preserves whether a failed platform operation may already have changed native state.
    /// Destructive domains use this evidence to avoid reporting an uncertain write as a clean
    /// zero-effect failure.
    pub fn mutation_state(&self) -> PlatformMutationState {
        self.mutation_state
    }

    pub fn with_reason(mut self, reason: CoreErrorReason) -> Self {
        self.reason = Some(reason);
        self
    }

    pub fn with_possible_side_effects(mut self) -> Self {
        self.mutation_state = PlatformMutationState::MayHaveChanged;
        self
    }

    pub fn operation_busy(diagnostic: impl Into<String>) -> Self {
        Self::new(CoreErrorCode::OperationBusy, diagnostic)
    }

    pub(crate) fn scan_resources_releasing() -> Self {
        Self::operation_busy("native analysis workers are still shutting down")
            .with_reason(CoreErrorReason::ScanResourcesReleasing)
    }

    pub fn invalid_input(diagnostic: impl Into<String>) -> Self {
        Self::new(CoreErrorCode::InvalidInput, diagnostic)
    }

    pub fn operation_cancelled() -> Self {
        Self::new(CoreErrorCode::OperationCancelled, "operation cancelled")
    }

    pub fn operation_failed(diagnostic: impl Into<String>) -> Self {
        Self::new(CoreErrorCode::OperationFailed, diagnostic)
    }

    pub fn persistence(diagnostic: impl Into<String>) -> Self {
        Self::new(CoreErrorCode::Persistence, diagnostic)
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.diagnostic)
    }
}

impl Error for CoreError {}

impl From<String> for CoreError {
    fn from(diagnostic: String) -> Self {
        Self::operation_failed(diagnostic)
    }
}

impl From<&str> for CoreError {
    fn from(diagnostic: &str) -> Self {
        Self::operation_failed(diagnostic)
    }
}

impl From<PlatformError> for CoreError {
    fn from(error: PlatformError) -> Self {
        let mutation_state = error.mutation_state();
        let code = match error.code() {
            PlatformErrorCode::UserCancelled => CoreErrorCode::OperationCancelled,
            PlatformErrorCode::AccessDenied => CoreErrorCode::PermissionDenied,
            PlatformErrorCode::ItemChanged => CoreErrorCode::OperationFailed,
            _ => CoreErrorCode::Platform,
        };
        let mut core_error = Self::new(code, error.to_string());
        core_error.mutation_state = mutation_state;
        if error.code() == PlatformErrorCode::ItemChanged {
            core_error.with_reason(CoreErrorReason::ItemChanged)
        } else {
            core_error
        }
    }
}

pub type CoreResult<T> = Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use mangodisk_platform::{PlatformError, PlatformMutationState};

    use super::CoreError;

    #[test]
    fn platform_mutation_uncertainty_survives_core_conversion() {
        let error = CoreError::from(
            PlatformError::operation_failed("post-write verification failed")
                .with_possible_side_effects(),
        );

        assert_eq!(
            error.mutation_state(),
            PlatformMutationState::MayHaveChanged
        );
    }
    #[test]
    fn platform_user_cancellation_remains_non_failure_in_core() {
        let error = CoreError::from(PlatformError::new(
            mangodisk_platform::PlatformErrorCode::UserCancelled,
            "elevation cancelled",
        ));
        assert_eq!(error.code(), super::CoreErrorCode::OperationCancelled);
        assert_eq!(error.mutation_state(), PlatformMutationState::NotAttempted);
    }
}
