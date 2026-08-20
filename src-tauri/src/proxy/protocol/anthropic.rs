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
/// Claude Code subscription (OAuth) accounts get the full official-client
/// treatment: the mandatory Claude Code system prefix and the CLI fingerprint
/// headers (see `claude_adapter`). Without them Anthropic's backend rejects or
/// risk-flags the request.
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

    // Claude OAuth: inject the required Claude Code system prefix so the
    // Anthropic backend accepts subscription traffic from any client.
    let claude_oauth = crate::services::claude_adapter::is_claude_oauth(account, provider);
    let request_body: Vec<u8> = if claude_oauth {
        match serde_json::from_slice::<serde_json::Value>(&body) {
            Ok(value) => serde_json::to_vec(&crate::services::claude_adapter::prepare_messages_body(
                &value, is_streaming,
            ))
            .map_err(|_| StatusCode::BAD_REQUEST)?,
            // Non-JSON bodies cannot receive the prefix; forward as-is and let
            // the upstream reject it.
            Err(_) => body.to_vec(),
        }
    } else {
        body.to_vec()
    };

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
    if claude_oauth {
        // Present the official Claude Code fingerprint (UA + beta flags + app
        // id); see services/client_profiles.rs for why this must be consistent.
        for (name, value) in [
            (
                "User-Agent",
                crate::services::client_profiles::CLAUDE_CODE_USER_AGENT,
            ),
            (
                "anthropic-beta",
                crate::services::client_profiles::CLAUDE_CODE_BETA_FLAGS,
            ),
            ("x-app", "cli"),
        ] {
            let name = name
                .parse::<axum::http::HeaderName>()
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            headers.insert(name, HeaderValue::from_static(value));
        }
    }

    // 4. Build client and forward
    let client = build_client(provider);

    let upstream_response = client
        .post(&upstream_url)
        .headers(headers)
        .body(request_body)
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

        let usage = crate::proxy::protocol::usage_from_response_body(&response_body);
        // Keep the downstream-provided error detail for the request log.
        let upstream_error = if status.is_success() {
            None
        } else {
            crate::proxy::protocol::upstream_error_message(&response_body)
        };
        let mut response = Response::new(Body::from(response_body));
        *response.status_mut() = status;
        response
            .headers_mut()
            .insert("Content-Type", HeaderValue::from_static("application/json"));
        if let Some(message) = upstream_error {
            response
                .extensions_mut()
                .insert(crate::proxy::UpstreamErrorDetail(message));
        }
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

/// Extract canonical token usage from an Anthropic response. Cache read and
/// write tokens are kept separate (they bill at different rates); the input
/// caliber matches Anthropic's native fresh-input reporting.
pub fn extract_usage(body: &[u8]) -> crate::proxy::protocol::Usage {
    crate::proxy::protocol::usage_from_response_body(body)
}
