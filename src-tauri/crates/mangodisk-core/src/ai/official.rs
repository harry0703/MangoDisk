//! Official-service request construction. It never forwards custom-provider credentials.
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use hmac::{Hmac, Mac};
use reqwest::{header::HeaderMap, Method, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use uuid::Uuid;

use super::official_protocol::response_error;
use super::{transport, AiConfiguration, AiDelta, AiError, AiRequest, AiServiceMode, AiUsage};

const QUOTA_PATH: &str = "/api/v1/ai/quotas/current";
const EXPLANATION_PATH: &str = "/api/v1/ai/explanations";

// Only this Rust module sees the embedded release key. An extracted shared key
// is not proof of a genuine installation; quotas and server budgets remain essential.
fn credentials() -> Result<(&'static str, Vec<u8>), AiError> {
    let id = option_env!("MANGODISK_AI_KEY_ID").ok_or(AiError::FreeUnavailable)?;
    let raw = option_env!("MANGODISK_AI_SIGNING_KEY").ok_or(AiError::FreeUnavailable)?;
    let key = URL_SAFE_NO_PAD
        .decode(raw)
        .or_else(|_| STANDARD.decode(raw))
        .map_err(|_| AiError::FreeUnavailable)?;
    if id.is_empty()
        || id.len() > 32
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || key.len() != 32
    {
        return Err(AiError::FreeUnavailable);
    }
    Ok((id, key))
}

pub(super) fn available() -> bool {
    credentials().is_ok() && origin().is_ok()
}

fn origin() -> Result<Url, AiError> {
    // A release build cannot redirect signed headers or private context to a debug URL.
    #[cfg(debug_assertions)]
    let value = option_env!("MANGODISK_AI_LOCAL_ORIGIN").unwrap_or("https://mangodisk.app");
    #[cfg(not(debug_assertions))]
    let value = "https://mangodisk.app";
    let url = Url::parse(value).map_err(|_| AiError::FreeUnavailable)?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || (url.scheme() != "https"
            && !(cfg!(debug_assertions)
                && url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
    {
        return Err(AiError::FreeUnavailable);
    }
    Ok(url)
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiClientMetadata {
    pub install_id: String,
    pub app_version: String,
    pub locale: String,
    pub distribution: String,
    pub os_version: String,
    pub timezone: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiQuota {
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub daily_limit: u32,
    pub remaining: u32,
    pub cooldown_seconds: u32,
    pub next_allowed_at: String,
    pub reset_at: String,
    pub server_time: String,
    pub active_requests: u32,
    pub max_concurrent_requests: u32,
    pub policy_version: String,
    pub prompt_version: String,
}

struct SignatureRequest<'a> {
    id: &'a str,
    method: &'a Method,
    origin: &'a str,
    path: &'a str,
    body: &'a [u8],
}

fn headers(
    metadata: &AiClientMetadata,
    request: SignatureRequest<'_>,
    key_id: &str,
    key: &[u8],
    timestamp: &str,
    nonce: &str,
) -> Result<HeaderMap, AiError> {
    let SignatureRequest {
        id,
        method,
        origin,
        path,
        body,
    } = request;
    let fields = [
        ("x-mangodisk-install-id", metadata.install_id.as_str(), 64),
        ("x-mangodisk-app-version", metadata.app_version.as_str(), 64),
        ("x-mangodisk-locale", metadata.locale.as_str(), 32),
        (
            "x-mangodisk-distribution",
            metadata.distribution.as_str(),
            32,
        ),
        ("x-mangodisk-os-version", metadata.os_version.as_str(), 64),
        ("x-mangodisk-timezone", metadata.timezone.as_str(), 64),
        ("x-request-id", id, 64),
        ("x-mangodisk-key-id", key_id, 32),
        ("x-mangodisk-timestamp", timestamp, 12),
        ("x-mangodisk-nonce", nonce, 43),
    ];
    let mut result = HeaderMap::new();
    for (name, value, limit) in fields {
        if value.is_empty()
            || value.len() > limit
            || value.bytes().any(|b| !(32..=126).contains(&b) || b == b',')
        {
            return Err(AiError::InvalidContext);
        }
        result.insert(name, value.parse().map_err(|_| AiError::InvalidContext)?);
    }
    if Uuid::parse_str(&metadata.install_id).is_err()
        || Uuid::parse_str(id).is_err()
        || !super::language::valid_language_tag(&metadata.locale)
    {
        return Err(AiError::InvalidContext);
    }
    let digest = format!("{:x}", Sha256::digest(body));
    let canonical = [
        "mangodisk-ai-v1",
        key_id,
        &metadata.install_id,
        &metadata.app_version,
        &metadata.locale,
        id,
        &metadata.distribution,
        &metadata.os_version,
        &metadata.timezone,
        timestamp,
        nonce,
        method.as_str(),
        origin,
        path,
        &digest,
    ]
    .join("\n");
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| AiError::FreeUnavailable)?;
    mac.update(canonical.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    result.insert(
        "x-mangodisk-signature",
        signature.parse().map_err(|_| AiError::FreeUnavailable)?,
    );
    result.insert(
        "content-type",
        "application/json".parse().expect("static header"),
    );
    Ok(result)
}

fn request(
    metadata: &AiClientMetadata,
    id: &str,
    method: Method,
    path: &str,
    body: Vec<u8>,
) -> Result<reqwest::RequestBuilder, AiError> {
    let (key_id, key) = credentials()?;
    let origin = origin()?;
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let nonce = URL_SAFE_NO_PAD.encode(Uuid::new_v4().as_bytes());
    let headers = headers(
        metadata,
        SignatureRequest {
            id,
            method: &method,
            origin: origin.as_str().trim_end_matches('/'),
            path,
            body: &body,
        },
        key_id,
        &key,
        &timestamp,
        &nonce,
    )?;
    let client = transport::client(if method == Method::GET { 15 } else { 220 })?;
    let mut builder = client
        .request(
            method.clone(),
            origin.join(path).map_err(|_| AiError::FreeUnavailable)?,
        )
        .headers(headers);
    if method == Method::POST {
        builder = builder.body(body);
    }
    Ok(builder)
}

pub async fn official_quota(metadata: AiClientMetadata) -> Result<AiQuota, AiError> {
    let id = Uuid::new_v4().to_string();
    let started = std::time::Instant::now();
    let mut status = None;
    let mut stage = "prepare";
    // Keep diagnostics outside the fallible future so transport, HTTP and
    // decoding failures share one terminal record without logging response data.
    let result: Result<AiQuota, AiError> = async {
        let builder = request(&metadata, &id, Method::GET, QUOTA_PATH, Vec::new())?;
        stage = "send";
        let mut response = builder.send().await.map_err(transport::network_error)?;
        status = Some(response.status().as_u16());
        stage = "http_status";
        if response.status() != 200 {
            return Err(response_error(response.headers()).unwrap_or(AiError::FreeUnavailable));
        }
        stage = "read_body";
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport::network_error)? {
            if bytes.len() + chunk.len() > 8192 {
                return Err(AiError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        #[derive(Deserialize)]
        struct Envelope {
            success: bool,
            data: AiQuota,
        }
        stage = "decode";
        let body: Envelope = serde_json::from_slice(&bytes).map_err(|_| AiError::InvalidStream)?;
        if !body.success {
            return Err(AiError::FreeUnavailable);
        }
        Ok(body.data)
    }
    .await;
    match &result {
        Ok(quota) => log::info!(
            "ai_quota_completed request_id={id} duration_ms={} status={status:?} available={} remaining={} daily_limit={}",
            started.elapsed().as_millis(), quota.available, quota.remaining, quota.daily_limit
        ),
        Err(reason) => log::warn!(
            "ai_quota_failed request_id={id} duration_ms={} status={status:?} stage={stage} reason={reason:?}",
            started.elapsed().as_millis()
        ),
    }
    result
}

pub async fn official_explain(
    config: AiConfiguration,
    input: AiRequest,
    metadata: AiClientMetadata,
    id: &str,
    cancel: watch::Receiver<bool>,
    emit: impl FnMut(AiDelta) -> bool,
) -> Result<AiUsage, AiError> {
    if config.mode != AiServiceMode::Free {
        return Err(AiError::InvalidConfiguration);
    }
    if !config.free_consent {
        return Err(AiError::FreeConsentRequired);
    }
    let context = input.context.ok_or(AiError::InvalidContext)?;
    context.validate()?;
    if input.language != metadata.locale {
        return Err(AiError::InvalidContext);
    }
    // Both providers receive the same Core-owned instructions. The server uses
    // its prompt only for older clients; this field is covered by the body signature.
    let body = explanation_body(&context, &input.language)?;
    let builder = request(&metadata, id, Method::POST, EXPLANATION_PATH, body)?;
    transport::stream_request(builder, id, cancel, emit, true).await
}

fn explanation_body(context: &super::AiContext, language: &str) -> Result<Vec<u8>, AiError> {
    let body = serde_json::to_vec(&serde_json::json!({
        "context": context,
        "systemPrompt": super::prompt::system_prompt(language, Some(context)),
    }))
    .map_err(|_| AiError::InvalidContext)?;
    if body.len() > 128 * 1024 {
        return Err(AiError::InvalidContext);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_body_preserves_the_same_module_prompts_as_custom_providers() {
        let contexts: Vec<super::super::AiContext> = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/ai-context-v2.json"
        ))
        .unwrap();
        for context in contexts {
            let body: serde_json::Value =
                serde_json::from_slice(&explanation_body(&context, "zh-CN").unwrap()).unwrap();
            assert_eq!(body["context"], serde_json::to_value(&context).unwrap());
            assert_eq!(
                body["systemPrompt"],
                super::super::prompt::system_prompt("zh-CN", Some(&context))
            );
            assert_eq!(body.as_object().unwrap().len(), 2);
        }
    }

    #[tokio::test]
    #[ignore = "requires a local website connected to a real provider; incurs one request"]
    async fn local_website_accepts_a_new_language_and_streams_an_answer() {
        assert_eq!(origin().unwrap().host_str(), Some("127.0.0.1"));
        let metadata = AiClientMetadata {
            install_id: Uuid::new_v4().to_string(),
            app_version: "1.0.9".into(),
            locale: "fr-FR".into(),
            distribution: "installed".into(),
            os_version: "test".into(),
            timezone: "UTC".into(),
        };
        let before = official_quota(metadata.clone()).await.unwrap();
        assert!(before.available);
        let contexts: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/ai-context-v2.json"
        ))
        .unwrap();
        let context = serde_json::from_value(contexts[0].clone()).unwrap();
        let mut config = AiConfiguration::initial();
        config.free_consent = true;
        config.api_key = "synthetic-custom-secret".into();
        let (_sender, cancel) = watch::channel(false);
        let mut answer = String::new();
        let mut chunks = 0;
        let started = std::time::Instant::now();
        let mut first_text_ms = None;
        let id = Uuid::new_v4().to_string();
        official_explain(
            config,
            AiRequest {
                context: Some(context),
                language: metadata.locale.clone(),
            },
            metadata.clone(),
            &id,
            cancel,
            |delta| {
                if let AiDelta::Text(text) = delta {
                    first_text_ms.get_or_insert_with(|| started.elapsed().as_millis());
                    chunks += 1;
                    answer.push_str(&text);
                }
                true
            },
        )
        .await
        .unwrap();
        assert!(!answer.trim().is_empty());
        let after = official_quota(metadata).await.unwrap();
        assert_eq!(after.remaining, before.remaining - 1);
        eprintln!("official_live_verified request_id={id} language=fr-FR chunks={chunks} text_bytes={} first_text_ms={first_text_ms:?} elapsed_ms={} remaining={}", answer.len(), started.elapsed().as_millis(), after.remaining);
    }

    #[test]
    fn signatures_match_the_server_golden_vectors() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/ai-signature-v1.json"
        ))
        .unwrap();
        for case in cases {
            let metadata: AiClientMetadata =
                serde_json::from_value(case["metadata"].clone()).unwrap();
            let method = Method::from_bytes(case["method"].as_str().unwrap().as_bytes()).unwrap();
            let result = headers(
                &metadata,
                SignatureRequest {
                    id: case["requestId"].as_str().unwrap(),
                    method: &method,
                    origin: case["origin"].as_str().unwrap(),
                    path: case["path"].as_str().unwrap(),
                    body: case["body"].as_str().unwrap().as_bytes(),
                },
                case["keyId"].as_str().unwrap(),
                &[7; 32],
                case["timestamp"].as_str().unwrap(),
                case["nonce"].as_str().unwrap(),
            )
            .unwrap();
            assert_eq!(
                result["x-mangodisk-signature"].to_str().unwrap(),
                case["signature"].as_str().unwrap()
            );
            assert!(!result.contains_key("authorization"));
        }
    }

    #[tokio::test]
    #[ignore = "requires the explicitly configured local website and synthetic build signing key"]
    async fn local_website_streams_all_modules_and_enforces_cooldown() {
        assert_eq!(origin().unwrap().host_str(), Some("127.0.0.1"));
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/ai-context-v2.json"
        ))
        .unwrap();
        for case in cases {
            let metadata = AiClientMetadata {
                install_id: Uuid::new_v4().to_string(),
                app_version: "1.0.9".into(),
                locale: "zh-CN".into(),
                distribution: "installed".into(),
                os_version: "test".into(),
                timezone: "Asia/Shanghai".into(),
            };
            assert_eq!(
                official_quota(metadata.clone()).await.unwrap().remaining,
                20
            );
            let mut config = AiConfiguration::initial();
            config.free_consent = true;
            // Retained BYOK credentials must never be sent to the official endpoint.
            config.api_key = "synthetic-custom-secret".into();
            let context: super::super::AiContext = serde_json::from_value(case).unwrap();
            let (_sender, cancel) = watch::channel(false);
            let mut text = String::new();
            let request_id = Uuid::new_v4().to_string();
            let explanation = official_explain(
                config.clone(),
                AiRequest {
                    context: Some(context.clone()),
                    language: "zh-CN".into(),
                },
                metadata.clone(),
                &request_id,
                cancel,
                |delta| {
                    if let AiDelta::Text(value) = delta {
                        text.push_str(&value);
                    }
                    true
                },
            );
            // Real reasoning models may stream for longer than the cooldown. Check
            // admission while the first request is running, not after it finishes.
            let cooldown = async {
                for _ in 0..100 {
                    let quota = official_quota(metadata.clone()).await.unwrap();
                    if quota.remaining == 19 {
                        assert_eq!(quota.unavailable_reason.as_deref(), Some("AI_RATE_LIMITED"));
                        let (_sender, cancel) = watch::channel(false);
                        let result = official_explain(
                            config,
                            AiRequest {
                                context: Some(context),
                                language: "zh-CN".into(),
                            },
                            metadata.clone(),
                            &Uuid::new_v4().to_string(),
                            cancel,
                            |_| true,
                        )
                        .await;
                        assert_eq!(result.unwrap_err(), AiError::FreeRateLimited);
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                panic!("The local website did not admit the request within the test deadline");
            };
            let (result, ()) = tokio::join!(explanation, cooldown);
            result.unwrap();
            // Markdown can be a plain paragraph; typography is not a transport invariant.
            assert!(!text.trim().is_empty());
            assert_eq!(official_quota(metadata).await.unwrap().remaining, 19);
        }
    }
}
