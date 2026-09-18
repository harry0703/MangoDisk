//! AI explanations and configuration; model output never receives filesystem capabilities.
mod configuration;
mod configuration_file;
mod context;
mod discovery;
#[cfg(test)]
mod evaluation;
mod language;
mod official;
mod official_protocol;
mod preferences;
mod prompt;
mod prompt_schema;
mod provider_error;
mod stream;
mod transport;

pub use configuration::{
    AiConfiguration, AiConfigurationUpdate, AiServiceMode, AiSettings, ReasoningMode,
};
pub use context::{AiContext, AiPlatform, AiSubject};
pub use discovery::{discover_local_models, InstalledLocalModel, LocalModelProvider};
pub use official::{official_explain, official_quota, AiClientMetadata, AiQuota};
pub use preferences::AiPreferences;
pub use transport::{explain, AiDelta, AiRequest, AiUsage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AiError {
    Disabled,
    InvalidConfiguration,
    InvalidContext,
    NotConfigured,
    ConfigurationUnavailable,
    Busy,
    Cancelled,
    Unauthorized,
    QuotaExceeded,
    ModelUnavailable,
    ProviderRejected,
    ConnectionFailed,
    Timeout,
    InvalidStream,
    IncompleteStream,
    OutputLimit,
    EmptyResponse,
    ResponseTooLarge,
    FreeUnavailable,
    FreeConsentRequired,
    FreeDailyLimit,
    FreeRateLimited,
    FreeConcurrent,
    FreeClockSkew,
    FreeSignatureInvalid,
    FreeRequestExists,
    FreeArchiveUnavailable,
}
