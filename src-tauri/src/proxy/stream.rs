//! SSE (Server-Sent Events) streaming utilities.
//! Provides zero-buffer forwarding of SSE events from upstream
//! (OpenAI / Anthropic / Gemini) to the downstream client.

use axum::{
    http::{header, HeaderValue},
    response::Response,
};
use bytes::Bytes;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::proxy::protocol::Usage;

/// Final metadata emitted after an upstream SSE body has completed.
#[derive(Clone, Debug, Default)]
pub struct StreamCompletion {
    pub usage: Usage,
    /// Present when the upstream failed or the downstream disconnected before
    /// the stream completed. Partial usage is deliberately marked unavailable.
    pub error_message: Option<String>,
}

/// Cloneable response extension used by the route handler to defer its final
/// request log until the background SSE forwarding task has finished.
#[derive(Clone)]
pub struct StreamCompletionHandle(Arc<Mutex<Option<oneshot::Receiver<StreamCompletion>>>>);

impl StreamCompletionHandle {
    pub async fn wait(self) -> Option<StreamCompletion> {
        let receiver = self.0.lock().await.take()?;
        receiver.await.ok()
    }
}

pub(crate) fn stream_completion_channel(
) -> (oneshot::Sender<StreamCompletion>, StreamCompletionHandle) {
    let (sender, receiver) = oneshot::channel();
    (
        sender,
        StreamCompletionHandle(Arc::new(Mutex::new(Some(receiver)))),
    )
}

fn merge_usage(target: &mut Usage, incoming: Usage) {
    if !incoming.available {
        return;
    }
    target.available = true;
    target.input_tokens = target.input_tokens.max(incoming.input_tokens);
    target.output_tokens = target.output_tokens.max(incoming.output_tokens);
    target.cache_read_tokens = target.cache_read_tokens.max(incoming.cache_read_tokens);
    target.cache_write_tokens = target.cache_write_tokens.max(incoming.cache_write_tokens);
}

/// Parse token usage from one complete SSE event. Supports native OpenAI Chat
/// and Responses events, Anthropic Messages events, and Gemini usageMetadata.
/// All vocabularies are decoded into the canonical fresh-input caliber (see
/// [`crate::proxy::protocol::Usage`]).
pub fn usage_from_sse_event(event_block: &str) -> Usage {
    let data = event_block
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() || data == "[DONE]" {
        return Usage::default();
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) else {
        return Usage::default();
    };

    let usage = value
        .get("usage")
        .or_else(|| value.get("response").and_then(|v| v.get("usage")))
        .or_else(|| value.get("message").and_then(|v| v.get("usage")));
    if let Some(usage) = usage {
        return if usage.get("prompt_tokens").is_some()
            || usage.get("prompt_tokens_details").is_some()
            || usage.get("input_tokens_details").is_some()
        {
            Usage::from_openai_usage(usage)
        } else {
            Usage::from_anthropic_usage(usage)
        };
    }

    if let Some(usage) = value
        .get("usageMetadata")
        .or_else(|| value.get("response").and_then(|node| node.get("usageMetadata")))
    {
        return Usage::from_gemini_metadata(usage);
    }

    Usage::default()
}

/// Incrementally parse all complete SSE events in `chunk` into `usage` while
/// retaining a split final event in `buffer` for the next network chunk.
pub fn collect_sse_usage(chunk: &[u8], buffer: &mut String, usage: &mut Usage) {
    buffer.push_str(&String::from_utf8_lossy(chunk));
    while let Some(index) = buffer.find("\n\n") {
        let event_block = buffer[..index].to_string();
        *buffer = buffer[index + 2..].to_string();
        merge_usage(usage, usage_from_sse_event(&event_block));
    }
}

/// Forward upstream SSE response body as an SSE response.
/// Uses channels to bridge the reqwest stream to axum's body stream.
pub fn forward_sse_stream(upstream_response: reqwest::Response) -> Response {
    forward_sse_stream_with_context(upstream_response, None, None, None)
}

pub fn forward_sse_stream_with_context(
    mut upstream_response: reqwest::Response,
    account_id: Option<String>,
    request_id: Option<String>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> Response {
    let (tx, rx) = mpsc::channel::<Bytes>(256);
    let (completion_tx, completion_handle) = stream_completion_channel();

    tokio::spawn(async move {
        // Keep account capacity reserved until upstream completion or downstream
        // disconnect. Dropping this task releases the permit automatically.
        let _permit = permit;
        let mut buf = String::new();
        let mut usage = Usage::default();
        let mut completion_error = None;
        let mut completed = false;

        loop {
            let chunk_result = tokio::select! {
                _ = tx.closed() => {
                    let message = "downstream disconnected before SSE completion".to_string();
                    tracing::info!(
                        "SSE downstream disconnected; cancelling upstream: account_id={} request_id={}",
                        account_id.as_deref().unwrap_or("unknown"),
                        request_id.as_deref().unwrap_or("unknown")
                    );
                    completion_error = Some(message);
                    break;
                }
                result = upstream_response.chunk() => result,
            };
            match chunk_result {
                Ok(Some(chunk)) => {
                    let chunk_str = String::from_utf8_lossy(&chunk);
                    buf.push_str(&chunk_str);

                    while let Some(double_newline) = buf.find("\n\n") {
                        let event_block = buf[..double_newline].to_string();
                        buf = buf[double_newline + 2..].to_string();
                        merge_usage(&mut usage, usage_from_sse_event(&event_block));

                        // Upstream can terminate a 200 stream with an error event
                        // (Anthropic `event: error`, OpenAI/Gemini error data).
                        // Forward it to the client, then record the failure with
                        // the downstream-provided message.
                        if let Some(message) =
                            crate::proxy::protocol::sse_event_error(&event_block)
                        {
                            let _ = tx.send(Bytes::from(event_block + "\n\n")).await;
                            completion_error = Some(message);
                            break;
                        }

                        let is_done = event_block.lines().any(|line| {
                            line.strip_prefix("data:")
                                .is_some_and(|data| data.trim() == "[DONE]")
                        });
                        if is_done {
                            if tx
                                .send(Bytes::from_static(b"data: [DONE]\n\n"))
                                .await
                                .is_err()
                            {
                                completion_error =
                                    Some("downstream disconnected before SSE completion".into());
                            } else {
                                completed = true;
                            }
                            break;
                        }
                        if tx.send(Bytes::from(event_block + "\n\n")).await.is_err() {
                            completion_error =
                                Some("downstream disconnected before SSE completion".into());
                            break;
                        }
                    }
                    if completed || completion_error.is_some() {
                        break;
                    }
                }
                Ok(None) => {
                    completed = true;
                    break;
                }
                Err(error) => {
                    let redacted = crate::services::redaction::redact_sensitive(&error.to_string());
                    tracing::warn!(
                        "SSE upstream stream failed: account_id={} request_id={} error={}",
                        account_id.as_deref().unwrap_or("unknown"),
                        request_id.as_deref().unwrap_or("unknown"),
                        redacted
                    );
                    completion_error = Some(redacted);
                    break;
                }
            }
        }

        if completed && !buf.is_empty() {
            merge_usage(&mut usage, usage_from_sse_event(&buf));
            if tx.send(Bytes::from(buf + "\n\n")).await.is_err() {
                completion_error = Some("downstream disconnected before SSE completion".into());
            }
        }
        if completion_error.is_some() {
            usage.available = false;
        }
        let _ = completion_tx.send(StreamCompletion {
            usage,
            error_message: completion_error,
        });
    });

    let body_stream =
        tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<_, std::convert::Infallible>);
    let mut response = Response::new(axum::body::Body::from_stream(body_stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response.extensions_mut().insert(completion_handle);
    response
}

/// Extract the "data: ..." SSE events into a Vec of strings
pub fn parse_sse_events(data: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(data);
    let mut events = Vec::new();

    for block in text.split("\n\n") {
        for line in block.lines() {
            if let Some(event_data) = line.strip_prefix("data: ") {
                events.push(event_data.to_string());
            }
        }
    }

    events
}

/// Build an SSE-format string from a JSON payload
pub fn format_sse_event(event: &str, data: &str) -> String {
    format!("event: {}\ndata: {}\n\n", event, data)
}

#[cfg(test)]
mod tests {
    use super::{collect_sse_usage, usage_from_sse_event};
    use crate::proxy::protocol::Usage;

    #[test]
    fn parses_openai_and_responses_stream_usage() {
        let chat = usage_from_sse_event(
            "data: {\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":3}}}",
        );
        assert!(chat.available);
        assert_eq!(
            (chat.input_tokens, chat.output_tokens, chat.cache_read_tokens),
            // OpenAI prompt_tokens is cache-inclusive → 12 - 3 fresh.
            (9, 7, 3)
        );

        let responses = usage_from_sse_event(
            "event: response.completed\ndata: {\"response\":{\"usage\":{\"input_tokens\":9,\"output_tokens\":4,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
        );
        assert!(responses.available);
        assert_eq!(
            (
                responses.input_tokens,
                responses.output_tokens,
                responses.cache_read_tokens
            ),
            (7, 4, 2)
        );
    }

    #[test]
    fn parses_anthropic_and_gemini_stream_usage_across_chunks() {
        let mut buffer = String::new();
        let mut usage = Usage::default();
        collect_sse_usage(
            b"event: message_start\ndata: {\"message\":{\"usage\":{\"input_tokens\":20,\"cache_read_input_tokens\":5,\"cache_creation_input_tokens\":2}}}\n\nevent: message_delta\ndata: {\"usage\":{\"output_",
            &mut buffer,
            &mut usage,
        );
        collect_sse_usage(b"tokens\":8}}\n\n", &mut buffer, &mut usage);
        assert!(usage.available);
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_write_tokens
            ),
            // Anthropic input_tokens excludes cache; read and write stay split.
            (20, 8, 5, 2)
        );

        let gemini = usage_from_sse_event(
            "data: {\"usageMetadata\":{\"promptTokenCount\":11,\"candidatesTokenCount\":6,\"cachedContentTokenCount\":1}}",
        );
        assert!(gemini.available);
        assert_eq!(
            (
                gemini.input_tokens,
                gemini.output_tokens,
                gemini.cache_read_tokens
            ),
            (10, 6, 1)
        );
    }
}
