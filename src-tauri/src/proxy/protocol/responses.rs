//! Request/response adaptation for the unified `/v1/responses` entry.
//!
//! PoolGate's unified entry speaks the OpenAI *Responses API*. An upstream
//! account, however, may speak any of several native protocols:
//!
//! * `responses`  — native Responses API: proxied as-is (no conversion).
//! * `chat`       — Chat Completions: request `input` -> `messages`; response
//!                  `choices[].message.content` -> Responses `output[]`.
//! * `anthropic`  — Anthropic Messages: request `input` -> `messages` (+ inject
//!                  `max_tokens`); response `content[].text` -> Responses `output[]`.
//! * `gemini`     — Gemini generateContent (best-effort passthrough shape).
//!
//! `chat` / `anthropic` only reach this entry when route takeover is enabled
//! (see `router::protocol_matches`); Gemini remains on its native endpoint. The
//! conversion below is what makes "one Responses endpoint, any upstream
//! protocol" work.

use axum::{
    body::Body,
    http::{header, HeaderValue},
    response::Response,
};
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;

/// Convert one complete upstream SSE event into a Responses API SSE event.
/// Returns `None` for comments/empty blocks and preserves unknown events as
/// `response.output_text.delta` data where possible.
pub fn normalize_sse_event(protocols: &[String], event_block: &str) -> Option<String> {
    let kind = upstream_kind(protocols);
    if kind == UpstreamKind::Responses || kind == UpstreamKind::Unsupported {
        return Some(format!("{}\n\n", event_block.trim_end()));
    }

    let mut event_name = None;
    let mut data_lines = Vec::new();
    for line in event_block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start());
        }
    }
    if data_lines.is_empty() {
        return None;
    }
    let data = data_lines.join("\n");
    if data == "[DONE]" {
        return Some(
            "event: response.completed\ndata: {\"type\":\"response.completed\"}\n\n".into(),
        );
    }
    let value = serde_json::from_str::<Value>(&data).ok()?;
    let text = match kind {
        UpstreamKind::Chat => value
            .get("choices")
            .and_then(|choices| choices.as_array())
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"))
            .and_then(|delta| delta.get("content"))
            .and_then(Value::as_str),
        UpstreamKind::Anthropic => {
            let upstream_event = event_name.unwrap_or_default();
            if upstream_event == "content_block_delta" {
                value
                    .get("delta")
                    .and_then(|delta| delta.get("text"))
                    .and_then(Value::as_str)
            } else {
                None
            }
        }
        UpstreamKind::Gemini => {
            // Antigravity (Cloud Code v1internal) wraps every event in
            // `{"response": {...}}`; unwrap transparently before extracting text.
            let inner = value.get("response").unwrap_or(&value);
            inner
                .get("candidates")
                .and_then(|candidates| candidates.as_array())
                .and_then(|candidates| candidates.first())
                .and_then(|candidate| candidate.get("content"))
                .and_then(|content| content.get("parts"))
                .and_then(|parts| parts.as_array())
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("text"))
                .and_then(Value::as_str)
        }
        _ => None,
    }?;

    let payload = serde_json::json!({
        "type": "response.output_text.delta",
        "delta": text,
    });
    Some(format!(
        "event: response.output_text.delta\ndata: {}\n\n",
        payload
    ))
}

/// Normalize an upstream SSE byte stream into Responses API events.
pub fn normalize_sse_stream(
    protocols: &[String],
    chunk: &[u8],
    buffer: &mut String,
) -> Vec<bytes::Bytes> {
    buffer.push_str(&String::from_utf8_lossy(chunk));
    let mut events = Vec::new();
    while let Some(index) = buffer.find("\n\n") {
        let block = buffer[..index].to_string();
        *buffer = buffer[index + 2..].to_string();
        if let Some(event) = normalize_sse_event(protocols, &block) {
            events.push(bytes::Bytes::from(event));
        }
    }
    events
}

/// Stream an upstream SSE response incrementally and translate native events
/// into the Responses API event vocabulary. Failover remains safe before this
/// function is called because no downstream bytes have been committed yet.
pub fn forward_responses_sse(
    upstream_response: reqwest::Response,
    protocols: Vec<String>,
) -> Response {
    forward_responses_sse_with_context(upstream_response, protocols, None, None, None)
}

pub fn forward_responses_sse_with_context(
    mut upstream_response: reqwest::Response,
    protocols: Vec<String>,
    account_id: Option<String>,
    request_id: Option<String>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> Response {
    let (tx, rx) = mpsc::channel::<bytes::Bytes>(256);
    let (completion_tx, completion_handle) = crate::proxy::stream::stream_completion_channel();
    tokio::spawn(async move {
        let _permit = permit;
        let mut buffer = String::new();
        let mut usage_buffer = String::new();
        let mut usage = crate::proxy::protocol::Usage::default();
        let mut completion_error = None;
        let mut completed = false;
        loop {
            let chunk_result = tokio::select! {
                _ = tx.closed() => {
                    tracing::info!(
                        "Responses SSE downstream disconnected; cancelling upstream: account_id={} request_id={}",
                        account_id.as_deref().unwrap_or("unknown"),
                        request_id.as_deref().unwrap_or("unknown")
                    );
                    completion_error = Some("downstream disconnected before SSE completion".into());
                    break;
                }
                result = upstream_response.chunk() => result,
            };
            let chunk = match chunk_result {
                Ok(Some(chunk)) => chunk,
                Ok(None) => {
                    completed = true;
                    break;
                }
                Err(error) => {
                    let redacted = crate::services::redaction::redact_sensitive(&error.to_string());
                    tracing::warn!(
                        "Responses SSE upstream stream failed: account_id={} request_id={} error={}",
                        account_id.as_deref().unwrap_or("unknown"),
                        request_id.as_deref().unwrap_or("unknown"),
                        redacted
                    );
                    completion_error = Some(redacted);
                    break;
                }
            };
            crate::proxy::stream::collect_sse_usage(&chunk, &mut usage_buffer, &mut usage);
            for event in normalize_sse_stream(&protocols, &chunk, &mut buffer) {
                if tx.send(event).await.is_err() {
                    completion_error = Some("downstream disconnected before SSE completion".into());
                    break;
                }
            }
            if completion_error.is_some() {
                break;
            }
        }
        if completed {
            if !usage_buffer.trim().is_empty() {
                let final_usage = crate::proxy::stream::usage_from_sse_event(&usage_buffer);
                if final_usage.available {
                    usage = final_usage;
                }
            }
            if !buffer.trim().is_empty() {
                if let Some(event) = normalize_sse_event(&protocols, &buffer) {
                    if tx.send(bytes::Bytes::from(event)).await.is_err() {
                        completion_error =
                            Some("downstream disconnected before SSE completion".into());
                    }
                }
            }
        }
        if completion_error.is_some() {
            usage.available = false;
        }
        let _ = completion_tx.send(crate::proxy::stream::StreamCompletion {
            usage,
            error_message: completion_error,
        });
    });

    let body_stream =
        tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<_, std::convert::Infallible>);
    let mut response = Response::new(Body::from_stream(body_stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response.extensions_mut().insert(completion_handle);
    response
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamKind {
    Responses,
    Chat,
    Anthropic,
    Gemini,
    Unsupported,
}

impl UpstreamKind {
    /// Upstream path suffix for this native protocol. The shared URL builder
    /// removes a duplicate `v1` when the provider Base URL is already versioned.
    pub fn path(self) -> &'static str {
        match self {
            UpstreamKind::Responses => "/v1/responses",
            UpstreamKind::Chat => "/v1/chat/completions",
            UpstreamKind::Anthropic => "/v1/messages",
            UpstreamKind::Gemini => "/v1beta/models:generateContent",
            UpstreamKind::Unsupported => "",
        }
    }
}

/// Choose the upstream native protocol for the unified entry.
/// Preference: native responses > anthropic > chat > gemini.
pub fn upstream_kind(protocols: &[String]) -> UpstreamKind {
    if protocols
        .iter()
        .any(|p| p.eq_ignore_ascii_case("responses"))
    {
        UpstreamKind::Responses
    } else if protocols
        .iter()
        .any(|p| p.eq_ignore_ascii_case("anthropic"))
    {
        UpstreamKind::Anthropic
    } else if protocols.iter().any(|p| p.eq_ignore_ascii_case("chat")) {
        UpstreamKind::Chat
    } else if protocols.iter().any(|p| p.eq_ignore_ascii_case("gemini")) {
        UpstreamKind::Gemini
    } else {
        UpstreamKind::Unsupported
    }
}

/// Rewrite a Responses API request body into the upstream's native shape.
pub fn convert_request(protocols: &[String], body: &Value) -> Value {
    match upstream_kind(protocols) {
        UpstreamKind::Responses | UpstreamKind::Unsupported => body.clone(),
        UpstreamKind::Chat => responses_to_chat(body),
        UpstreamKind::Anthropic => responses_to_anthropic(body),
        UpstreamKind::Gemini => responses_to_gemini(body),
    }
}

fn drop_input(v: &mut Value) {
    if let Some(obj) = v.as_object_mut() {
        obj.remove("input");
    }
}

fn responses_to_chat(body: &Value) -> Value {
    let mut v = body.clone();
    if let Some(input) = v.get("input") {
        v["messages"] = input.clone();
    }
    drop_input(&mut v);
    v
}

fn responses_to_anthropic(body: &Value) -> Value {
    let mut v = body.clone();
    if let Some(input) = v.get("input") {
        v["messages"] = input.clone();
    }
    drop_input(&mut v);
    if v.get("max_tokens").is_none() {
        v["max_tokens"] = serde_json::json!(4096);
    }
    v
}

fn responses_to_gemini(body: &Value) -> Value {
    let mut v = body.clone();
    // Prefer the Responses `input`; tolerate Chat-style `messages` for clients
    // that hit /v1/responses with a Chat Completions payload.
    let input = v.get("input").or_else(|| v.get("messages"));
    if let Some(input) = input {
        v["contents"] = input_to_gemini_contents(input);
    }
    // OpenAI-only fields are invalid on the Gemini upstream and would trigger
    // `400 Invalid JSON payload: Unknown name` — strip them after conversion.
    drop_input(&mut v);
    if let Some(object) = v.as_object_mut() {
        object.remove("messages");
    }
    v
}

fn input_to_gemini_contents(input: &Value) -> Value {
    match input {
        Value::String(s) => serde_json::json!([{ "role": "user", "parts": [{ "text": s }] }]),
        Value::Array(arr) => {
            let contents: Vec<Value> = arr
                .iter()
                .map(|m| {
                    let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                    let text = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
                    serde_json::json!({ "role": role, "parts": [{ "text": text }] })
                })
                .collect();
            Value::Array(contents)
        }
        _ => serde_json::json!([]),
    }
}

/// Normalize an upstream native response body back into Responses API shape.
/// Non-JSON upstream bodies are passed through unchanged.
pub fn normalize_response(protocols: &[String], raw: &str) -> String {
    match upstream_kind(protocols) {
        UpstreamKind::Responses | UpstreamKind::Unsupported => raw.to_string(),
        kind => match serde_json::from_str::<Value>(raw) {
            Ok(v) => match kind {
                UpstreamKind::Chat => chat_to_responses(&v),
                UpstreamKind::Anthropic => anthropic_to_responses(&v),
                UpstreamKind::Gemini => gemini_to_responses(&v),
                _ => raw.to_string(),
            },
            Err(_) => raw.to_string(),
        },
    }
}

fn wrap_output(content: Option<&Value>) -> String {
    let out = serde_json::json!({
        "object": "response",
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": content }]
        }]
    });
    out.to_string()
}

fn chat_to_responses(v: &Value) -> String {
    // {"choices":[{"message":{"content":"..."}}]}
    if let Some(choices) = v.get("choices").and_then(|c| c.as_array()) {
        if let Some(first) = choices.first() {
            return wrap_output(first.get("message").and_then(|m| m.get("content")));
        }
    }
    v.to_string()
}

fn anthropic_to_responses(v: &Value) -> String {
    // {"content":[{"type":"text","text":"..."}]}
    if let Some(content) = v.get("content").and_then(|c| c.as_array()) {
        if let Some(first) = content.first() {
            return wrap_output(first.get("text"));
        }
    }
    v.to_string()
}

fn gemini_to_responses(v: &Value) -> String {
    // {"candidates":[{"content":{"parts":[{"text":"..."}]}}]}
    // Antigravity (Cloud Code v1internal) wraps the same payload in
    // `{"response": {...}}`; unwrap transparently.
    let v = v.get("response").unwrap_or(v);
    if let Some(cands) = v.get("candidates").and_then(|c| c.as_array()) {
        if let Some(first) = cands.first() {
            let text = first
                .get("content")
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array())
                .and_then(|p| p.first())
                .and_then(|p| p.get("text"));
            return wrap_output(text);
        }
    }
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_chat_sse_delta_to_responses_event() {
        let event = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        let normalized = normalize_sse_event(&["chat".into()], event).unwrap();
        assert!(normalized.starts_with("event: response.output_text.delta\n"));
        assert!(normalized.contains(r#""delta":"hello""#));
    }

    #[test]
    fn converts_anthropic_sse_delta_to_responses_event() {
        let event = concat!(
            "event: content_block_delta\n",
            r#"data: {"delta":{"type":"text_delta","text":"hi"}}"#
        );
        let normalized = normalize_sse_event(&["anthropic".into()], event).unwrap();
        assert!(normalized.contains(r#""delta":"hi""#));
    }

    #[test]
    fn preserves_native_responses_sse_events() {
        let event = concat!(
            "event: response.output_text.delta\n",
            r#"data: {"type":"response.output_text.delta","delta":"ok"}"#
        );
        let normalized = normalize_sse_event(&["responses".into()], event).unwrap();
        assert_eq!(normalized, format!("{}\n\n", event));
    }

    #[test]
    fn handles_sse_events_split_across_chunks() {
        let protocols = vec!["chat".into()];
        let mut buffer = String::new();
        assert!(normalize_sse_stream(
            &protocols,
            br#"data: {"choices":[{"delta":{"content":"hel"#,
            &mut buffer,
        )
        .is_empty());
        let events = normalize_sse_stream(&protocols, b"lo\"}}]}\n\n", &mut buffer);
        assert_eq!(events.len(), 1);
        assert!(String::from_utf8_lossy(&events[0]).contains(r#""delta":"hello""#));
        assert!(buffer.is_empty());
    }

    #[test]
    fn gemini_sse_unwraps_antigravity_response_wrapper() {
        let protocols = vec!["gemini".into()];
        let event = r#"data: {"response":{"candidates":[{"content":{"parts":[{"text":"hi"}]}}]}}"#;
        let normalized = normalize_sse_event(&protocols, event).unwrap();
        assert!(normalized.starts_with("event: response.output_text.delta\n"));
        assert!(normalized.contains(r#""delta":"hi""#));
    }

    #[test]
    fn gemini_non_streaming_unwraps_antigravity_response_wrapper() {
        let protocols = vec!["gemini".into()];
        let raw = r#"{"response":{"candidates":[{"content":{"parts":[{"text":"hello world"}]}}]}}"#;
        let normalized = normalize_response(&protocols, raw);
        assert!(normalized.contains(r#""text":"hello world""#));
        assert!(normalized.contains(r#""object":"response""#));
    }

    #[test]
    fn gemini_plain_payload_unchanged_without_wrapper() {
        let protocols = vec!["gemini".into()];
        let raw = r#"{"candidates":[{"content":{"parts":[{"text":"ok"}]}}]}"#;
        let normalized = normalize_response(&protocols, raw);
        assert!(normalized.contains(r#""text":"ok""#));
    }

    #[test]
    fn chat_style_messages_convert_to_gemini_contents() {
        let protocols = vec!["gemini".into()];
        let body = serde_json::json!({
            "model": "gemini-2.5-pro",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let converted = convert_request(&protocols, &body);
        assert_eq!(converted["contents"][0]["parts"][0]["text"], "hello");
        // OpenAI-only fields must not leak to the Gemini upstream.
        assert!(converted.get("messages").is_none());
        assert!(converted.get("input").is_none());
    }
}
