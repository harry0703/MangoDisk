//! Safe provider diagnostics: never retain echoed input or free-form error text.
use serde_json::Value;

const MAX_ERROR_BYTES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ErrorBody {
    Json,
    Invalid,
    TooLarge,
    ReadFailed,
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderCode {
    UnsupportedParameter,
    InvalidRequest,
    InvalidApiKey,
    InsufficientQuota,
    RateLimitExceeded,
    ModelNotFound,
    ContextLengthExceeded,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderParameter {
    Thinking,
    EnableThinking,
    ReasoningEffort,
    StreamOptions,
    Model,
    Messages,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProviderDiagnostic {
    pub body: ErrorBody,
    pub code: ProviderCode,
    pub parameter: ProviderParameter,
}

impl ProviderDiagnostic {
    fn unreadable(body: ErrorBody) -> Self {
        Self {
            body,
            code: ProviderCode::Unknown,
            parameter: ProviderParameter::Unknown,
        }
    }

    pub fn from_error(error: &Value) -> Self {
        // Even code/param fields can echo credentials. Only exact known values
        // are mapped; unknown values and the message are deliberately discarded.
        let code = match error["code"].as_str() {
            Some("unsupported_parameter") => ProviderCode::UnsupportedParameter,
            Some("invalid_request_error") => ProviderCode::InvalidRequest,
            Some("invalid_api_key") => ProviderCode::InvalidApiKey,
            Some("insufficient_quota") => ProviderCode::InsufficientQuota,
            Some("rate_limit_exceeded") => ProviderCode::RateLimitExceeded,
            Some("model_not_found") => ProviderCode::ModelNotFound,
            Some("context_length_exceeded") => ProviderCode::ContextLengthExceeded,
            _ => ProviderCode::Unknown,
        };
        let parameter = match error["param"].as_str() {
            Some("thinking") => ProviderParameter::Thinking,
            Some("enable_thinking") => ProviderParameter::EnableThinking,
            Some("reasoning_effort" | "reasoning.effort") => ProviderParameter::ReasoningEffort,
            Some("stream_options" | "stream_options.include_usage") => {
                ProviderParameter::StreamOptions
            }
            Some("model") => ProviderParameter::Model,
            Some("messages") => ProviderParameter::Messages,
            _ => ProviderParameter::Unknown,
        };
        Self {
            body: ErrorBody::Json,
            code,
            parameter,
        }
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(value) => Self::from_error(&value["error"]),
            Err(_) => Self::unreadable(ErrorBody::Invalid),
        }
    }
}

pub(super) async fn read(response: &mut reqwest::Response) -> ProviderDiagnostic {
    // An error body is diagnostic only. Bound both size and latency, preserving
    // the original HTTP error if a gateway sends HTML, stalls, or disconnects.
    let read = async {
        let mut bytes = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if bytes.len() + chunk.len() > MAX_ERROR_BYTES {
                        return ProviderDiagnostic::unreadable(ErrorBody::TooLarge);
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(None) => return ProviderDiagnostic::from_bytes(&bytes),
                Err(_) => return ProviderDiagnostic::unreadable(ErrorBody::ReadFailed),
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), read)
        .await
        .unwrap_or_else(|_| ProviderDiagnostic::unreadable(ErrorBody::Timeout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_map_known_fields_without_retaining_untrusted_text() {
        let known = ProviderDiagnostic::from_bytes(
            br#"{"error":{"code":"unsupported_parameter","param":"thinking","message":"secret"}}"#,
        );
        assert_eq!(known.code, ProviderCode::UnsupportedParameter);
        assert_eq!(known.parameter, ProviderParameter::Thinking);
        for input in [
            br#"{"error":{"code":"secret","param":"/private/path","message":"secret"}}"#.as_slice(),
            br#"{"error":"secret"}"#,
            br#"{"error":{"code":123,"param":[]}}"#,
            b"<html>secret</html>",
        ] {
            let diagnostic = ProviderDiagnostic::from_bytes(input);
            assert_eq!(diagnostic.code, ProviderCode::Unknown);
            assert_eq!(diagnostic.parameter, ProviderParameter::Unknown);
            assert!(!format!("{diagnostic:?}").contains("secret"));
            assert!(!format!("{diagnostic:?}").contains("/private/path"));
        }
    }
}
