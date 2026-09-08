//! Pure official wire-error mapping, independent of request construction and HTTP IO.
use super::AiError;
use reqwest::header::HeaderMap;

pub(super) fn response_error(headers: &HeaderMap) -> Option<AiError> {
    error_code(headers.get("x-mangodisk-ai-error-code")?.to_str().ok()?)
}

pub(super) fn error_code(code: &str) -> Option<AiError> {
    Some(match code {
        "AI_SERVICE_DISABLED"
        | "AI_GLOBAL_LIMIT_REACHED"
        | "AI_QUOTA_UNAVAILABLE"
        | "AI_UPSTREAM_UNAVAILABLE" => AiError::FreeUnavailable,
        "AI_DAILY_LIMIT_REACHED" => AiError::FreeDailyLimit,
        "AI_RATE_LIMITED" => AiError::FreeRateLimited,
        "AI_CONCURRENCY_LIMITED" => AiError::FreeConcurrent,
        "AI_CLOCK_SKEW" => AiError::FreeClockSkew,
        "AI_SIGNATURE_INVALID" => AiError::FreeSignatureInvalid,
        "AI_REPLAY_DETECTED" | "AI_REQUEST_EXISTS" | "AI_REQUEST_CONFLICT" => {
            AiError::FreeRequestExists
        }
        "AI_ARCHIVE_UNAVAILABLE" => AiError::FreeArchiveUnavailable,
        "AI_UPSTREAM_TIMEOUT" => AiError::Timeout,
        "AI_INCOMPLETE_STREAM" => AiError::IncompleteStream,
        // Input rejection happens before generation and must not be described as
        // a reply being truncated by the client receive limit.
        "AI_CONTEXT_TOO_LARGE" | "AI_INVALID_CONTEXT" => AiError::InvalidContext,
        "AI_CANCELLED" => AiError::Cancelled,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_context_is_not_reported_as_an_oversized_reply() {
        for code in ["AI_CONTEXT_TOO_LARGE", "AI_INVALID_CONTEXT"] {
            assert_eq!(error_code(code), Some(AiError::InvalidContext));
        }
    }
}
