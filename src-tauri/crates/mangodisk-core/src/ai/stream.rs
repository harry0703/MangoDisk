use super::provider_error::ProviderDiagnostic;
use super::{AiDelta, AiError, AiUsage};

/// Incremental SSE framing operates on bytes so a split UTF-8 character is not
/// replaced before its remaining bytes arrive. Provider-supplied reasoning remains separate from the final answer.
#[derive(Default)]
pub(super) struct AiStream {
    pending: Vec<u8>,
    data: String,
    pub text_bytes: usize,
    pub done: bool,
    stopped: bool,
    output_limited: bool,
    pub reasoning_bytes: usize,
    pub usage: AiUsage,
    pub provider_error: Option<ProviderDiagnostic>,
}

impl AiStream {
    pub fn feed(
        &mut self,
        bytes: &[u8],
        emit: &mut impl FnMut(AiDelta) -> bool,
    ) -> Result<(), AiError> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line = self.pending.drain(..=end).collect::<Vec<_>>();
            let line = std::str::from_utf8(&line).map_err(|_| AiError::InvalidStream)?;
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                self.dispatch(emit)?;
            } else if let Some(data) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(data.strip_prefix(' ').unwrap_or(data));
            }
            if self.data.len() > 65536 {
                return Err(AiError::ResponseTooLarge);
            }
        }
        if self.pending.len() > 65536 {
            return Err(AiError::ResponseTooLarge);
        }
        Ok(())
    }

    fn dispatch(&mut self, emit: &mut impl FnMut(AiDelta) -> bool) -> Result<(), AiError> {
        let data = std::mem::take(&mut self.data);
        if data.is_empty() || self.done {
            return Ok(());
        }
        if data.trim() == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        let event: serde_json::Value =
            serde_json::from_str(&data).map_err(|_| AiError::InvalidStream)?;
        if event.get("error").is_some_and(|error| !error.is_null()) {
            self.provider_error = Some(ProviderDiagnostic::from_error(&event["error"]));
            return Err(AiError::ProviderRejected);
        }
        if let Some(usage) = event.get("usage").filter(|v| !v.is_null()) {
            self.usage.prompt_tokens = usage["prompt_tokens"].as_u64();
            self.usage.completion_tokens = usage["completion_tokens"].as_u64();
        }
        if let Some(choice) = event["choices"].as_array().and_then(|v| v.first()) {
            // Only explicit, readable provider fields are displayable. Prefer the
            // canonical field to avoid duplicating gateways that expose both aliases.
            // Encrypted reasoning_details and inferred <think> tags are not decoded.
            if let Some(reasoning) = choice["delta"]["reasoning_content"]
                .as_str()
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    choice["delta"]["reasoning"]
                        .as_str()
                        .filter(|value| !value.is_empty())
                })
            {
                self.reasoning_bytes += reasoning.len();
                if self.reasoning_bytes > 256 * 1024 {
                    return Err(AiError::ResponseTooLarge);
                }
                if !emit(AiDelta::Reasoning(reasoning.to_owned())) {
                    return Err(AiError::Cancelled);
                }
            }
            if let Some(text) = choice["delta"]["content"]
                .as_str()
                .filter(|v| !v.is_empty())
            {
                self.text_bytes += text.len();
                if self.text_bytes > 32768 {
                    return Err(AiError::ResponseTooLarge);
                }
                if !emit(AiDelta::Text(text.to_owned())) {
                    return Err(AiError::Cancelled);
                }
            }
            if let Some(reason) = choice["finish_reason"].as_str() {
                if reason == "length" {
                    // Keep consuming usage and [DONE] for useful diagnostics.
                    self.output_limited = true;
                } else if reason == "stop" {
                    self.stopped = true;
                } else {
                    return Err(AiError::IncompleteStream);
                }
            }
        }
        Ok(())
    }

    pub fn finish_reason(&self) -> &'static str {
        if self.output_limited {
            "length"
        } else if self.stopped {
            "stop"
        } else {
            "missing"
        }
    }

    pub fn finish(&self) -> Result<AiUsage, AiError> {
        if self.output_limited {
            Err(AiError::OutputLimit)
        } else if !self.done || !self.stopped {
            Err(AiError::IncompleteStream)
        } else if self.text_bytes == 0 {
            Err(AiError::EmptyResponse)
        } else {
            Ok(self.usage.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fragmented_utf8_and_separates_reasoning() {
        let data = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"private\",\"content\":\"\u{7528}\u{9014}\"}}]}\r\n\r\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let mut stream = AiStream::default();
        let mut output = String::new();
        let mut reasoning = String::new();
        for byte in data.as_bytes() {
            stream
                .feed(&[*byte], &mut |text| {
                    match text {
                        AiDelta::Text(text) => output.push_str(&text),
                        AiDelta::Reasoning(text) => reasoning.push_str(&text),
                    }
                    true
                })
                .unwrap();
        }
        assert_eq!(output, "\u{7528}\u{9014}");
        assert_eq!(reasoning, "private");
        assert!(stream.finish().is_ok());
    }

    #[test]
    fn supports_reasoning_alias_without_duplicating_or_rendering_encrypted_details() {
        let mut stream = AiStream::default();
        let mut output = Vec::new();
        stream.feed(b"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"first\",\"reasoning\":\"duplicate\",\"reasoning_details\":[{\"type\":\"reasoning.encrypted\",\"data\":\"opaque\"}]}}]}\n\ndata: {\"choices\":[{\"delta\":{\"reasoning\":\"second\"}}]}\n\n", &mut |delta| { output.push(delta); true }).unwrap();
        assert_eq!(
            output,
            [
                AiDelta::Reasoning("first".into()),
                AiDelta::Reasoning("second".into())
            ]
        );
        assert_eq!(
            serde_json::to_value(&output[0]).unwrap(),
            serde_json::json!({"kind":"reasoning","text":"first"})
        );
        assert_eq!(
            serde_json::to_value(AiDelta::Text("answer".into())).unwrap(),
            serde_json::json!({"kind":"text","text":"answer"})
        );
    }

    #[test]
    fn reasoning_is_cancellable_bounded_and_never_a_complete_answer() {
        let frame = b"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\n\n";
        assert_eq!(
            AiStream::default().feed(frame, &mut |_| false),
            Err(AiError::Cancelled)
        );
        let mut stream = AiStream::default();
        stream.feed(frame, &mut |_| true).unwrap();
        stream.feed(b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n", &mut |_| true).unwrap();
        assert_eq!(stream.finish().unwrap_err(), AiError::EmptyResponse);
        let frame = format!(
            "data: {}\n\n",
            serde_json::json!({"choices":[{"delta":{"reasoning_content":"x".repeat(16384)}}]})
        );
        let mut stream = AiStream::default();
        for _ in 0..16 {
            stream.feed(frame.as_bytes(), &mut |_| true).unwrap();
        }
        assert_eq!(
            stream.feed(frame.as_bytes(), &mut |_| true),
            Err(AiError::ResponseTooLarge)
        );
    }

    #[test]
    fn rejects_truncated_error_and_oversized_streams() {
        assert_eq!(
            AiStream::default().finish().unwrap_err(),
            AiError::IncompleteStream
        );
        assert_eq!(
            AiStream::default().feed(
                b"data: {\"error\":{\"message\":\"secret\"}}\n\n",
                &mut |_| true
            ),
            Err(AiError::ProviderRejected)
        );
        assert_eq!(
            AiStream::default().feed(&vec![b'x'; 65537], &mut |_| true),
            Err(AiError::ResponseTooLarge)
        );
    }

    #[test]
    fn consumer_can_cancel_and_length_is_not_success() {
        assert_eq!(
            AiStream::default().feed(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n",
                &mut |_| false
            ),
            Err(AiError::Cancelled)
        );
        let mut stream = AiStream::default();
        stream.feed(b"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hidden\"},\"finish_reason\":\"length\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"completion_tokens\":500}}\n\ndata: [DONE]\n\n", &mut |_| true).unwrap();
        assert!(stream.done);
        assert_eq!(stream.text_bytes, 0);
        assert_eq!(stream.reasoning_bytes, 6);
        assert_eq!(stream.usage.completion_tokens, Some(500));
        assert_eq!(stream.finish().unwrap_err(), AiError::OutputLimit);
    }
}
