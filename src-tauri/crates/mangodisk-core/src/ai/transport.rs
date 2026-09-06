use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::watch;

// SSE gateways may repeat several hundred bytes of metadata for each token.
// Bound wire traffic separately from the parser's 32 KiB visible-answer limit.
const MAX_STREAM_WIRE_BYTES: usize = 4 * 1024 * 1024;

use super::{
    prompt::system_prompt, stream::AiStream, AiConfiguration, AiContext, AiError, ReasoningMode,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiRequest {
    pub context: Option<AiContext>,
    pub language: String,
}

/// Tagged streaming IPC payload; reasoning never becomes final-answer text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "text", rename_all = "camelCase")]
pub enum AiDelta {
    Text(String),
    Reasoning(String),
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

fn payload(config: &AiConfiguration, request: &AiRequest) -> Result<serde_json::Value, AiError> {
    if !["zh-CN", "zh-TW", "en-US", "ja-JP"].contains(&request.language.as_str()) {
        return Err(AiError::InvalidContext);
    }
    let user = if let Some(context) = &request.context {
        context.validate()?;
        serde_json::to_string(context).map_err(|_| AiError::InvalidContext)?
    } else {
        "Connection test only. Reply with OK.".to_owned()
    };
    let system = system_prompt(&request.language, request.context.as_ref());
    let mut body = json!({
        "model": config.model,
        "messages": [{"role":"system","content":system},{"role":"user","content":user}],
        "stream": true, "stream_options": {"include_usage":true},
    });
    // Let the provider choose its completion budget. A client token cap can
    // exhaust reasoning before any visible answer, even for connection tests.
    // Both flags are opt-in because OpenAI-compatible providers differ in their
    // extensions. Provider-default mode sends neither.
    if matches!(config.reasoning, ReasoningMode::Disabled) {
        body["thinking"] = json!({"type":"disabled"});
        body["enable_thinking"] = json!(false);
    }
    Ok(body)
}

fn network_error(error: reqwest::Error) -> AiError {
    // Provider errors can embed authenticated URLs or echoed inputs. Never
    // expose their text through logs or IPC.
    if error.is_timeout() {
        AiError::Timeout
    } else {
        AiError::ConnectionFailed
    }
}

pub async fn explain(
    config: AiConfiguration,
    request: AiRequest,
    operation_id: &str,
    mut cancel: watch::Receiver<bool>,
    mut emit: impl FnMut(AiDelta) -> bool,
) -> Result<AiUsage, AiError> {
    config.validate()?;
    let body = payload(&config, &request)?;
    if *cancel.borrow() {
        return Err(AiError::Cancelled);
    }
    let started = Instant::now();
    log::info!(
        "ai_request_policy operation_id={operation_id} reasoning={:?} token_budget=provider_default timeout_seconds=180 context_bytes={} wire_limit_bytes={MAX_STREAM_WIRE_BYTES}",
        config.reasoning,
        body["messages"][1]["content"].as_str().map(str::len).unwrap_or_default(),
    );
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(network_error)?;
    // Keep counters outside the cancellable future so every terminal path can
    // report partial progress, including cancellation during a provider stall.
    let mut stream = AiStream::default();
    let mut received = 0;
    let run = async {
        let mut builder = client
            .post(format!("{}/chat/completions", config.endpoint))
            .header("x-request-id", operation_id)
            .json(&body);
        if !config.api_key.is_empty() {
            builder = builder.bearer_auth(&config.api_key);
        }
        let mut response = builder.send().await.map_err(network_error)?;
        let status = response.status().as_u16();
        log::info!(
            "ai_response operation_id={operation_id} status={status} headers_ms={}",
            started.elapsed().as_millis()
        );
        if status != 200 {
            stream.provider_error = Some(super::provider_error::read(&mut response).await);
            return Err(match status {
                401 | 403 => AiError::Unauthorized,
                402 | 429 => AiError::QuotaExceeded,
                404 => AiError::ModelUnavailable,
                _ => AiError::ProviderRejected,
            });
        }
        let mut first_text = true;
        let mut first_reasoning = true;
        let result = async {
            while let Some(chunk) = response.chunk().await.map_err(network_error)? {
                received += chunk.len();
                if received > MAX_STREAM_WIRE_BYTES {
                    return Err(AiError::ResponseTooLarge);
                }
                stream.feed(&chunk, &mut |text| {
                    if first_reasoning && matches!(&text, AiDelta::Reasoning(_)) {
                        first_reasoning = false;
                        log::info!(
                            "ai_first_reasoning operation_id={operation_id} elapsed_ms={}",
                            started.elapsed().as_millis()
                        );
                    }
                    if first_text && matches!(&text, AiDelta::Text(_)) {
                        first_text = false;
                        log::info!(
                            "ai_first_text operation_id={operation_id} elapsed_ms={}",
                            started.elapsed().as_millis()
                        );
                    }
                    emit(text)
                })?;
                if stream.done {
                    break;
                }
            }
            Ok(())
        }
        .await;
        result?;
        stream.finish()
    };
    let result = tokio::select! {
        result = run => result,
        _ = cancel.changed() => Err(AiError::Cancelled),
    };
    let outcome = match &result {
        Ok(_) => "completed",
        Err(AiError::Cancelled) => "cancelled",
        Err(_) => "failed",
    };
    log::info!("ai_stream_finished operation_id={operation_id} received_bytes={received} text_bytes={} reasoning_bytes={} done={} finish_reason={} prompt_tokens={:?} completion_tokens={:?} outcome={outcome} reason={:?} provider_error={:?}", stream.text_bytes, stream.reasoning_bytes, stream.done, stream.finish_reason(), stream.usage.prompt_tokens, stream.usage.completion_tokens, result.as_ref().err(), stream.provider_error);
    result
}

#[cfg(test)]
mod tests {

    use super::*;

    fn fixture_config(endpoint: String) -> AiConfiguration {
        AiConfiguration {
            schema_version: 1,
            endpoint,
            model: "fixture".into(),
            api_key: "synthetic-key".into(),
            reasoning: ReasoningMode::Default,
        }
    }

    fn server(status: u16, body: impl Into<String>) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let body = body.into();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            // Consume the complete request before dropping the socket. A single
            // read may contain headers only; closing with an unread body can
            // reset the connection and make the streaming test intermittent.
            let mut request = Vec::new();
            loop {
                let mut chunk = [0u8; 4096];
                let length = stream.read(&mut chunk).unwrap();
                assert!(length > 0, "request ended before its body");
                request.extend_from_slice(&chunk[..length]);
                assert!(request.len() <= 16384);
                if let Some(end) = request.windows(4).position(|value| value == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]);
                    assert!(header.starts_with("POST /v1/chat/completions"));
                    let body_length: usize = header
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + body_length {
                        break;
                    }
                }
            }
            // Limit/cancellation tests may intentionally close the client mid-body.
            let _ = write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        });
        (format!("http://{address}/v1"), handle)
    }

    #[tokio::test]
    async fn wire_budget_still_rejects_unbounded_sse_comments() {
        let frame = format!(":{}\n\n", ".".repeat(65500));
        let body = frame.repeat(65);
        assert!(body.len() > MAX_STREAM_WIRE_BYTES);
        let (endpoint, thread) = server(200, body);
        let (_cancel, receiver) = watch::channel(false);
        let result = explain(
            fixture_config(endpoint),
            AiRequest {
                context: None,
                language: "en-US".into(),
            },
            "fixture-request",
            receiver,
            |_| true,
        )
        .await;
        assert!(matches!(result, Err(AiError::ResponseTooLarge)));
        thread.join().unwrap();
    }

    #[tokio::test]
    async fn repeated_gateway_metadata_does_not_truncate_a_short_answer() {
        let frame = format!(
            "data: {}\n\n",
            json!({"id": "metadata".repeat(120), "choices":[{"delta":{"reasoning_content":"x"}}]})
        );
        let mut body = frame.repeat(1400);
        body.push_str("data: {\"choices\":[{\"delta\":{\"content\":\"Short answer\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
        assert!(body.len() > 1024 * 1024 && body.len() < MAX_STREAM_WIRE_BYTES);
        let (endpoint, thread) = server(200, body);
        let (_cancel, receiver) = watch::channel(false);
        let mut text = String::new();
        explain(
            fixture_config(endpoint),
            AiRequest {
                context: None,
                language: "en-US".into(),
            },
            "fixture-request",
            receiver,
            |delta| {
                if let AiDelta::Text(delta) = delta {
                    text.push_str(&delta);
                }
                true
            },
        )
        .await
        .unwrap();
        assert_eq!(text, "Short answer");
        thread.join().unwrap();
    }

    #[tokio::test]
    async fn http_stream_completes_and_http_errors_are_typed() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Purpose: cache.\"}}]}\n\ndata: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        for (status, expected) in [
            (200, None),
            (401, Some(AiError::Unauthorized)),
            (429, Some(AiError::QuotaExceeded)),
            (302, Some(AiError::ProviderRejected)),
        ] {
            let (endpoint, thread) = server(status, body);
            let (_cancel, receiver) = watch::channel(false);
            let mut text = String::new();
            let result = explain(
                fixture_config(endpoint),
                AiRequest {
                    context: None,
                    language: "en-US".into(),
                },
                "fixture-request",
                receiver,
                |delta| {
                    if let AiDelta::Text(delta) = delta {
                        text.push_str(&delta);
                    }
                    true
                },
            )
            .await;
            assert_eq!(result.err(), expected);
            if status == 200 {
                assert_eq!(text, "Purpose: cache.");
            }
            thread.join().unwrap();
        }
    }

    #[tokio::test]
    async fn pre_cancelled_request_never_connects() {
        let (_cancel, receiver) = watch::channel(true);
        let result = explain(
            fixture_config("http://127.0.0.1:1/v1".into()),
            AiRequest {
                context: None,
                language: "en-US".into(),
            },
            "cancelled-fixture",
            receiver,
            |_| true,
        )
        .await;
        assert_eq!(result.unwrap_err(), AiError::Cancelled);
    }

    #[tokio::test]
    #[ignore = "calls the explicitly configured AI provider and incurs token usage"]
    async fn actual_provider_stream() {
        let mut config = fixture_config(
            std::env::var("MANGODISK_AI_TEST_ENDPOINT").expect("explicit test endpoint"),
        );
        config.model = std::env::var("MANGODISK_AI_TEST_MODEL").expect("explicit test model");
        config.api_key =
            std::env::var("ZENAI_AI_GATEWAY_API_KEY").expect("credential supplied in environment");
        config.reasoning =
            if std::env::var("MANGODISK_AI_TEST_REASONING").as_deref() == Ok("default") {
                ReasoningMode::Default
            } else {
                ReasoningMode::Disabled
            };
        let module = std::env::var("MANGODISK_AI_TEST_MODULE").unwrap_or_else(|_| "cleanup".into());
        let fixtures: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/ai-context-v2.json"
        ))
        .unwrap();
        let fixture = fixtures
            .into_iter()
            .find(|value| value["subject"]["module"] == module)
            .expect("unknown test module");
        let context: AiContext = serde_json::from_value(fixture).unwrap();
        let (_cancel, receiver) = watch::channel(false);
        let mut chunks = 0;
        let mut reasoning_chunks = 0;
        // Match the production adapter's UUID-shaped correlation header.
        let operation_id = format!(
            "00000000-0000-4000-8000-{:012x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                & 0xffff_ffff_ffff
        );
        let result = explain(
            config,
            AiRequest {
                context: Some(context),
                language: "zh-CN".into(),
            },
            &operation_id,
            receiver,
            |delta| {
                match delta {
                    AiDelta::Text(_) => chunks += 1,
                    AiDelta::Reasoning(_) => reasoning_chunks += 1,
                }
                true
            },
        )
        .await;
        assert!(result.is_ok(), "provider result: {:?}", result.err());
        assert!(chunks > 1, "expected multiple streamed text chunks");
        println!("provider_stream text_chunks={chunks} reasoning_chunks={reasoning_chunks}");
    }

    #[tokio::test]
    async fn every_module_uses_its_prompt_and_completes_a_real_http_stream() {
        let fixtures: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/ai-context-v2.json"
        ))
        .unwrap();
        let expected = [
            "requiresAppClose",
            "synchronizationMayPropagate",
            "does not start or stop",
            "unapplied draft",
            "not that a fault was detected",
        ];
        for (fixture, boundary) in fixtures.into_iter().zip(expected) {
            let context: AiContext = serde_json::from_value(fixture).unwrap();
            let (endpoint, thread) = server(200, "data: {\"choices\":[{\"delta\":{\"content\":\"Purpose.\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\" Impact.\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
            let config = fixture_config(endpoint);
            let request = AiRequest {
                context: Some(context),
                language: "zh-CN".into(),
            };
            let body = payload(&config, &request).unwrap();
            assert!(body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains(boundary));
            assert!(body.get("tools").is_none());
            let (_sender, receiver) = watch::channel(false);
            let mut deltas = Vec::new();
            explain(config, request, "fixture-request", receiver, |text| {
                deltas.push(text);
                true
            })
            .await
            .unwrap();
            assert_eq!(
                deltas,
                [
                    AiDelta::Text("Purpose.".into()),
                    AiDelta::Text(" Impact.".into())
                ]
            );
            thread.join().unwrap();
        }
    }

    #[test]
    fn request_uses_provider_token_budget_and_has_no_tools() {
        let mut config = AiConfiguration {
            schema_version: 1,
            endpoint: "https://example.com/v1".into(),
            model: "example".into(),
            api_key: "test".into(),
            reasoning: ReasoningMode::Default,
        };
        let request = AiRequest {
            context: None,
            language: "en-US".into(),
        };
        let body = payload(&config, &request).unwrap();
        assert_eq!(body["stream"], true);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("max_completion_tokens").is_none());
        assert!(body.get("tools").is_none());
        assert!(body.get("thinking").is_none());
        config.reasoning = ReasoningMode::Disabled;
        let body = payload(&config, &request).unwrap();
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("max_completion_tokens").is_none());
        assert_eq!(body["thinking"]["type"], "disabled");
        assert_eq!(body["enable_thinking"], false);
        let request = AiRequest {
            context: Some(
                serde_json::from_value(
                    serde_json::from_str::<Vec<serde_json::Value>>(include_str!(
                        "../../../../../tests/fixtures/ai-context-v2.json"
                    ))
                    .unwrap()
                    .remove(0),
                )
                .unwrap(),
            ),
            language: "en-US".into(),
        };
        for reasoning in [ReasoningMode::Default, ReasoningMode::Disabled] {
            config.reasoning = reasoning;
            let body = payload(&config, &request).unwrap();
            assert!(body.get("max_tokens").is_none());
            assert!(body.get("max_completion_tokens").is_none());
            assert!(body.get("tools").is_none());
        }
    }
}
