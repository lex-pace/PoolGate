//! Anthropic-compatible request forwarding.
//! Parses incoming POST /v1/messages, injects the real API key,
//! forwards to the provider's base_url/v1/messages, and handles
//! streaming (SSE) vs non-streaming responses.

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::Response,
};
use bytes::Bytes;
use reqwest::Client;

use crate::db::{accounts::Account, providers::Provider};

/// Build a reqwest client respecting provider config.
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

/// Handle an Anthropic-compatible /v1/messages request.
///
/// # Arguments
/// * `body` - Raw request bytes.
/// * `account` - The selected account (API key source).
/// * `provider` - Provider config (base_url, etc.).
pub async fn handle_anthropic_request(
    body: Bytes,
    account: &Account,
    provider: &Provider,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> Result<(Response, crate::proxy::protocol::Usage), StatusCode> {
    // 1. Check if streaming
    let is_streaming = is_anthropic_streaming_request(&body);

    // 2. Build upstream URL
    let upstream_url = build_upstream_url(provider, "/v1/messages");

    // 3. Build headers
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    let credential = crate::services::credentials::auth_credential(account).map_err(|error| {
        tracing::warn!(
            "Account '{}' cannot authorize Anthropic request: {}",
            account.id,
            error
        );
        StatusCode::UNAUTHORIZED
    })?;
    match credential {
        crate::services::credentials::AuthCredential::ApiKey(secret) => {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(&secret).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            );
        }
        crate::services::credentials::AuthCredential::Bearer(secret) => {
            headers.insert(
                "Authorization",
                HeaderValue::from_str(&format!("Bearer {}", secret))
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            );
        }
    }
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));

    // 4. Build client and forward
    let client = build_client(provider);

    let upstream_response = client
        .post(&upstream_url)
        .headers(headers)
        .body(body.to_vec())
        .send()
        .await
        .map_err(|e| {
            tracing::error!(
                "Anthropic upstream request failed: {}",
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
            tracing::error!("Failed to read Anthropic upstream response: {}", e);
            StatusCode::BAD_GATEWAY
        })?;

        let (input_tokens, output_tokens) = extract_usage(&response_body);
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

/// Check if an Anthropic request body requests streaming.
fn is_anthropic_streaming_request(body: &[u8]) -> bool {
    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(stream) = val.get("stream") {
            return stream.as_bool().unwrap_or(false);
        }
    }
    false
}

/// Build upstream URL from provider base_url + path.
fn build_upstream_url(provider: &Provider, path: &str) -> String {
    crate::proxy::protocol::build_upstream_url(&provider.base_url_for_protocol("anthropic"), path)
}

/// Extract model name from an Anthropic request body.
pub fn extract_model(body: &[u8]) -> Option<String> {
    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(body) {
        return val
            .get("model")
            .and_then(|m| m.as_str().map(|s| s.to_string()));
    }
    None
}

/// Estimate token usage from an Anthropic response.
/// Anthropic returns `usage.input_tokens` and `usage.output_tokens`.
pub fn extract_usage(body: &[u8]) -> (i64, i64) {
    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(usage) = val.get("usage") {
            let input = usage
                .get("input_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let output = usage
                .get("output_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            return (input, output);
        }
    }
    (0, 0)
}
