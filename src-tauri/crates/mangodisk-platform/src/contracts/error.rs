use std::{error::Error, fmt};

/// Stable error categories exposed by platform contracts.
///
/// Diagnostics remain native and English for logs. Product adapters localize
/// the code instead of interpreting diagnostic text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformErrorCode {
    AccessDenied,
    UserCancelled,
    ItemChanged,
    InvalidData,
    InvalidPath,
    Io,
    OperationFailed,
    Unsupported,
}

/// Optional, stable context for failures that a broad platform error code cannot explain.
/// Callers may localize this reason without interpreting native messages or exit-code text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformFailureReason {
    ToolUnavailable,
    ServiceDisabled,
    ServiceUnavailable,
    DependencyUnavailable,
    ServiceBusy,
    TimedOut,
    VerificationFailed,
    VerificationPermissionDenied,
}

/// Describes whether a failed native operation can still have changed operating-system state.
///
/// Callers use this signal to retain preflight recovery data when a write or its verification
/// fails. Treating every error as side-effect free would make a successfully written setting
/// irreversible when only the post-write read failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformMutationState {
    NotAttempted,
    MayHaveChanged,
}

impl PlatformError {
    pub fn item_changed(diagnostic: impl Into<String>) -> Self {
        Self::new(PlatformErrorCode::ItemChanged, diagnostic)
    }
}

#[derive(Debug, Clone)]
pub struct PlatformError {
    code: PlatformErrorCode,
    diagnostic: String,
    mutation_state: PlatformMutationState,
    failure_reason: Option<PlatformFailureReason>,
}

impl PlatformError {
    pub fn new(code: PlatformErrorCode, diagnostic: impl Into<String>) -> Self {
        Self {
            code,
            diagnostic: diagnostic.into(),
            mutation_state: PlatformMutationState::NotAttempted,
            failure_reason: None,
        }
    }

    pub fn with_failure_reason(mut self, reason: PlatformFailureReason) -> Self {
        self.failure_reason = Some(reason);
        self
    }

    pub fn failure_reason(&self) -> Option<PlatformFailureReason> {
        self.failure_reason
    }

    pub fn code(&self) -> PlatformErrorCode {
        self.code
    }

    pub fn diagnostic(&self) -> &str {
        &self.diagnostic
    }

    pub fn mutation_state(&self) -> PlatformMutationState {
        self.mutation_state
    }

    /// Marks an error returned after a native write was attempted. The original diagnostic and
    /// stable code stay unchanged so existing product error mapping remains compatible.
    pub fn with_possible_side_effects(mut self) -> Self {
        self.mutation_state = PlatformMutationState::MayHaveChanged;
        self
    }

    /// Returns the stable diagnostic payload for hashing or structured logs.
    pub fn as_bytes(&self) -> &[u8] {
        self.diagnostic.as_bytes()
    }

    pub fn operation_failed(diagnostic: impl Into<String>) -> Self {
        Self::new(PlatformErrorCode::OperationFailed, diagnostic)
    }

    pub fn invalid_path(diagnostic: impl Into<String>) -> Self {
        Self::new(PlatformErrorCode::InvalidPath, diagnostic)
    }

    pub fn io(operation: &'static str, error: &std::io::Error) -> Self {
        let code = match error.kind() {
            std::io::ErrorKind::PermissionDenied => PlatformErrorCode::AccessDenied,
            std::io::ErrorKind::InvalidData | std::io::ErrorKind::InvalidInput => {
                PlatformErrorCode::InvalidData
            }
            _ => PlatformErrorCode::Io,
        };
        Self::new(code, format!("{operation}: {:?}", error.kind()))
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.diagnostic)
    }
}

impl Error for PlatformError {}

impl From<String> for PlatformError {
    fn from(diagnostic: String) -> Self {
        Self::operation_failed(diagnostic)
    }
}

impl From<&str> for PlatformError {
    fn from(diagnostic: &str) -> Self {
        Self::operation_failed(diagnostic)
    }
}

pub type PlatformResult<T> = Result<T, PlatformError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn possible_side_effects_preserve_the_original_error_code() {
        let error = PlatformError::new(PlatformErrorCode::AccessDenied, "test")
            .with_possible_side_effects();

        assert_eq!(error.code(), PlatformErrorCode::AccessDenied);
        assert_eq!(
            error.mutation_state(),
            PlatformMutationState::MayHaveChanged
        );
    }
}
