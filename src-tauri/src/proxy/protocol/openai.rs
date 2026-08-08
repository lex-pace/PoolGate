//! OpenAI-compatible request forwarding.
//! Parses incoming POST /v1/chat/completions, injects the real API key
//! from the selected account, forwards to the provider's base_url,
//! and handles streaming vs non-streaming responses.

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::Response,
};
use bytes::Bytes;
use reqwest::Client;

use crate::db::{accounts::Account, providers::Provider};

/// Build a reqwest client with optional proxy and timeout from provider config.
fn build_client(provider: &Provider) -> Client {
    let mut builder = Client::builder().timeout(std::time::Duration::from_millis(
        provider.timeout_ms.unwrap_or(60_000) as u64,
    ));

    if let Some(ref proxy_url) = provider.proxy_url {
        if !proxy_url.is_empty() {
            if let Ok(proxy) = reqwest::Proxy::all(proxy_url) {
                builder = builder.proxy(proxy);
            }
        }
    }

    builder.build().unwrap_or_else(|_| Client::new())
}

/// Handle an OpenAI-compatible /v1/chat/completions request.
///
/// # Arguments
/// * `body` - Raw request body from the incoming HTTP request.
/// * `account` - The selected account whose API key will be injected.
/// * `provider` - The provider configuration (base_url, protocol, etc.).
///
/// # Returns
/// An axum Response (streaming SSE if the request asked for streaming, or JSON otherwise).
pub async fn handle_openai_request(
    body: Bytes,
    account: &Account,
    provider: &Provider,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> Result<(Response, crate::proxy::protocol::Usage), StatusCode> {
    // 1. Parse the incoming request to determine if streaming is requested
    let is_streaming = is_streaming_request(&body);

    // 2. Determine upstream URL
    let upstream_url = build_upstream_url(provider, "/v1/chat/completions");

    // 3. Build headers
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    let credential =
        crate::services::credentials::authorization_secret(account).map_err(|error| {
            tracing::warn!(
                "Account '{}' cannot authorize OpenAI request: {}",
                account.id,
                error
            );
            StatusCode::UNAUTHORIZED
        })?;
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", credential))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );

    // 4. Build the reqwest client
    let client = build_client(provider);

    // 5. Forward the request
    let upstream_response = client
        .post(&upstream_url)
        .headers(headers)
        .body(body.to_vec())
        .send()
        .await
        .map_err(|e| {
            tracing::error!(
                "OpenAI upstream request failed: {}",
                crate::services::redaction::redact_sensitive(&e.to_string())
            );
            StatusCode::BAD_GATEWAY
        })?;

    let status = upstream_response.status();

    if is_streaming && status.is_success() {
        // Return streaming SSE response
        let response = crate::proxy::stream::forward_sse_stream_with_context(
            upstream_response,
            Some(account.id.clone()),
            None,
            permit,
        );
        Ok((response, crate::proxy::protocol::Usage::default()))
    } else {
        // Return JSON response
        let response_body = upstream_response.bytes().await.map_err(|e| {
            tracing::error!("Failed to read upstream response: {}", e);
            StatusCode::BAD_GATEWAY
        })?;

        let (input_tokens, output_tokens, _) = extract_usage(&response_body);
        let usage = crate::proxy::protocol::Usage {
            input_tokens,
            output_tokens,
            cache_tokens: 0,
            available: serde_json::from_slice::<serde_json::Value>(&response_body)
                .ok()
                .and_then(|value| value.get("usage").cloned())
                .is_some(),
        };
        let mut response = Response::new(Body::from(response_body));
        *response.status_mut() = status;
        response
            .headers_mut()
            .insert("Content-Type", HeaderValue::from_static("application/json"));
        Ok((response, usage))
    }
}

/// Determine if the incoming request body asks for streaming.
fn is_streaming_request(body: &[u8]) -> bool {
    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(stream) = val.get("stream") {
            return stream.as_bool().unwrap_or(false);
        }
    }
    false
}

/// Build the upstream URL for a given provider and path.
fn build_upstream_url(provider: &Provider, path: &str) -> String {
    crate::proxy::protocol::build_upstream_url(&provider.base_url_for_protocol("chat"), path)
}

/// Extract model name from an OpenAI-compatible request body.
pub fn extract_model(body: &[u8]) -> Option<String> {
    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(body) {
        return val
            .get("model")
            .and_then(|m| m.as_str().map(|s| s.to_string()));
    }
    None
}

/// Extract token usage from an OpenAI-compatible response body.
pub fn extract_usage(body: &[u8]) -> (i64, i64, i64) {
    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(usage) = val.get("usage") {
            let input = usage
                .get("prompt_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let output = usage
                .get("completion_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let total = usage
                .get("total_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            return (input, output, total);
        }
    }
    (0, 0, 0)
}
