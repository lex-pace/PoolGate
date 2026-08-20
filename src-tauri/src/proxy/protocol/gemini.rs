//! Gemini pass-through proxy handler.
//! Forwards requests to Google Gemini API (generativelanguage.googleapis.com).
//! Supports both API key (query param) and OAuth Bearer token auth.
//!
//! Also handles Google Antigravity (Cloud Code) accounts: those talk to the
//! private `cloudcode-pa.googleapis.com/v1internal` endpoint with a
//! `{"response": ...}` wrapper that must be unwrapped (reference: Antigravity
//! Tools / Antigravity-Manager).

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::Response,
};
use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::db::{accounts::Account, providers::Provider};

/// True when the provider is Google Antigravity (Cloud Code) rather than the
/// public Gemini API.
pub(crate) fn is_antigravity(provider: &Provider) -> bool {
    provider.provider_type.eq_ignore_ascii_case("antigravity")
        || provider.base_url.contains("cloudcode-pa.googleapis.com")
}

/// Resolve the Cloud Code project id from account metadata (plaintext column
/// first, then the encrypted credential payload).
pub(crate) fn antigravity_project_id(account: &Account) -> Option<String> {
    if let Some(meta) = account.metadata.as_deref() {
        if let Ok(value) = serde_json::from_str::<Value>(meta) {
            if let Some(pid) = value.get("project_id").and_then(|v| v.as_str()) {
                if !pid.trim().is_empty() {
                    return Some(pid.to_string());
                }
            }
        }
    }
    if let Ok(payload) = crate::services::credentials::payload_for_account(account) {
        if let Some(meta) = payload.metadata {
            if let Some(pid) = meta.get("project_id").and_then(|v| v.as_str()) {
                if !pid.trim().is_empty() {
                    return Some(pid.to_string());
                }
            }
        }
    }
    None
}

/// Inject the Cloud Code project id (mock fallback included) and, when the
/// body has no model, the requested model into an Antigravity v1internal
/// generateContent body. Shared by the native Gemini entry and the unified
/// `/v1/responses` entry.
pub(crate) fn inject_antigravity_context(body: &mut Value, account: &Account, model: Option<&str>) {
    let project_id = antigravity_project_id(account)
        .or_else(|| Some(crate::services::antigravity_adapter::generate_mock_project_id()));
    if let Some(pid) = project_id {
        body["project"] = Value::String(pid);
    }
    let has_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .map(|m| !m.trim().is_empty())
        .unwrap_or(false);
    if !has_model {
        if let Some(model) = model.filter(|m| !m.trim().is_empty()) {
            body["model"] = Value::String(model.to_string());
        }
    }

    // v1internal wraps the Gemini request payload inside a `request` envelope.
    // Fields like `contents`, `systemInstruction`, `generationConfig`, and `tools`
    // must move under `request`; top-level `model` and `project` stay outside.
    // If `request` already exists (caller did manual wrapping), skip.
    if body.get("request").is_none() {
        let mut request_obj = serde_json::Map::new();
        // Move Gemini-standard fields into the request envelope.
        for key in &["contents", "systemInstruction", "generationConfig", "tools"] {
            if let Some(value) = body.as_object_mut().and_then(|o| o.remove(*key)) {
                request_obj.insert((*key).to_string(), value);
            }
        }
        if !request_obj.is_empty() {
            body["request"] = Value::Object(request_obj);
        }
    }

    // v1internal expects `userAgent` and `requestId` at the top level.
    if body.get("userAgent").is_none() {
        body["userAgent"] = Value::String("antigravity".into());
    }
    if body.get("requestId").is_none() {
        let rid = format!(
            "poolgate-{}-{}",
            chrono::Utc::now().timestamp_millis(),
            &uuid::Uuid::new_v4().to_string()[..8],
        );
        body["requestId"] = Value::String(rid);
    }
}

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

/// Handle a Gemini-compatible request.
///
/// The incoming path is expected to be something like:
/// `/v1beta/models/{model}:generateContent` or `/v1/models/{model}:generateContent`
/// or `/v1beta/models/{model}:streamGenerateContent`.
///
/// The account's `api_key` is injected as a query parameter `?key=...`.
pub async fn handle_gemini_request(
    body: Bytes,
    account: &Account,
    provider: &Provider,
    model_action: &str,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> Result<(Response, crate::proxy::protocol::Usage), StatusCode> {
    // 1. Check if streaming
    let is_streaming =
        model_action.contains("streamGenerateContent") || is_gemini_streaming_request(&body);

    let credential = crate::services::credentials::auth_credential(account).map_err(|error| {
        tracing::warn!(
            "Account '{}' cannot authorize Gemini request: {}",
            account.id,
            error
        );
        StatusCode::UNAUTHORIZED
    })?;

    // 2. Build the upstream path & body.
    //    Antigravity uses the private v1internal shape `{base}/v1internal:{action}`
    //    with `project` injected into the body; standard Gemini uses
    //    `/v1beta/models/{model}:{action}`.
    let antigravity = is_antigravity(provider);
    let (path, request_body) = if antigravity {
        let action = model_action
            .rsplit(':')
            .next()
            .filter(|a| !a.is_empty() && *a != model_action)
            .unwrap_or("generateContent");
        let mut body_json: Value =
            serde_json::from_slice(&body).unwrap_or_else(|_| Value::Object(Default::default()));
        let project_id = antigravity_project_id(account)
            .or_else(|| Some(crate::services::antigravity_adapter::generate_mock_project_id()));
        if let Some(pid) = project_id {
            body_json["project"] = Value::String(pid);
        }
        let has_model = body_json
            .get("model")
            .and_then(|v| v.as_str())
            .map(|m| !m.trim().is_empty())
            .unwrap_or(false);
        if !has_model {
            if let Some(model) = model_action.split(':').next() {
                if !model.is_empty() {
                    body_json["model"] = Value::String(model.to_string());
                }
            }
        }
        // v1internal wraps the Gemini request payload inside a `request` envelope.
        // Fields like `contents`, `systemInstruction`, `generationConfig`, and `tools`
        // must move under `request`; top-level `model` and `project` stay outside.
        if body_json.get("request").is_none() {
            let mut request_obj = serde_json::Map::new();
            for key in &["contents", "systemInstruction", "generationConfig", "tools"] {
                if let Some(value) = body_json.as_object_mut().and_then(|o| o.remove(*key)) {
                    request_obj.insert((*key).to_string(), value);
                }
            }
            if !request_obj.is_empty() {
                body_json["request"] = Value::Object(request_obj);
            }
        }
        if body_json.get("userAgent").is_none() {
            body_json["userAgent"] = Value::String("antigravity".into());
        }
        if body_json.get("requestId").is_none() {
            let rid = format!(
                "poolgate-{}-{}",
                chrono::Utc::now().timestamp_millis(),
                &uuid::Uuid::new_v4().to_string()[..8],
            );
            body_json["requestId"] = Value::String(rid);
        }
        (
            format!("/v1internal:{}", action),
            serde_json::to_vec(&body_json).unwrap_or_else(|_| body.to_vec()),
        )
    } else {
        (format!("/v1beta/models/{}", model_action), body.to_vec())
    };

    let base_url = crate::proxy::protocol::build_upstream_url(
        &provider.base_url_for_protocol("gemini"),
        &path,
    );
    let mut upstream_url = base_url;
    let bearer = match credential {
        crate::services::credentials::AuthCredential::ApiKey(secret) => {
            let separator = if upstream_url.contains('?') { '&' } else { '?' };
            upstream_url.push_str(&format!("{}key={}", separator, secret));
            None
        }
        crate::services::credentials::AuthCredential::Bearer(secret) => Some(secret),
    };
    if is_streaming && !upstream_url.contains("alt=sse") {
        let separator = if upstream_url.contains('?') { '&' } else { '?' };
        upstream_url.push_str(&format!("{}alt=sse", separator));
    }

    // 3. Build headers
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    if antigravity {
        headers.insert(
            "User-Agent",
            HeaderValue::from_static(crate::services::antigravity_adapter::ANTIGRAVITY_USER_AGENT),
        );
        // v1internal requires these additional headers for successful request routing.
        headers.insert(
            "X-Goog-Api-Client",
            HeaderValue::from_static("google-cloud-sdk vscode_cloudshelleditor/0.1"),
        );
        headers.insert(
            "Client-Metadata",
            HeaderValue::from_static(r#"{"ideType":"IDE_UNSPECIFIED","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}"#),
        );
    }
    if let Some(secret) = bearer {
        headers.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", secret))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        );
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
                "Gemini upstream request failed: {}",
                crate::services::redaction::redact_sensitive(&e.to_string())
            );
            StatusCode::BAD_GATEWAY
        })?;

    let status = upstream_response.status();

    if is_streaming && status.is_success() {
        let response = if antigravity {
            forward_antigravity_sse(upstream_response, permit)
        } else {
            crate::proxy::stream::forward_sse_stream_with_context(
                upstream_response,
                Some(account.id.clone()),
                None,
                permit,
            )
        };
        Ok((response, crate::proxy::protocol::Usage::default()))
    } else {
        let response_body = upstream_response.bytes().await.map_err(|e| {
            tracing::error!("Failed to read Gemini upstream response: {}", e);
            StatusCode::BAD_GATEWAY
        })?;

        // Antigravity wraps the Gemini payload in {"response": {...}}.
        let unwrapped = if antigravity {
            unwrap_antigravity_response(&response_body)
        } else {
            response_body.clone()
        };

        let usage = crate::proxy::protocol::usage_from_response_body(&unwrapped);
        // Keep the downstream-provided error detail for the request log.
        let upstream_error = if status.is_success() {
            None
        } else {
            crate::proxy::protocol::upstream_error_message(&unwrapped)
        };
        let mut response = Response::new(Body::from(unwrapped));
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

/// Unwrap an Antigravity v1internal JSON body (`{"response": {...}}`).
fn unwrap_antigravity_response(body: &[u8]) -> Bytes {
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        if let Some(inner) = value.get("response") {
            if let Ok(encoded) = serde_json::to_vec(inner) {
                return Bytes::from(encoded);
            }
        }
    }
    Bytes::from(body.to_vec())
}

/// Forward an Antigravity SSE stream, unwrapping every `data: {"response": ...}`
/// line into `data: {...}` (native Gemini shape). Passes everything else through.
fn forward_antigravity_sse(
    mut upstream: reqwest::Response,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> Response {
    let (tx, rx) = mpsc::channel::<Bytes>(256);

    tokio::spawn(async move {
        // Keep account capacity reserved until upstream completion or downstream
        // disconnect. Dropping this task releases the permit automatically.
        let _permit = permit;
        let mut buffer = BytesMut::new();

        loop {
            match upstream.chunk().await {
                Ok(Some(chunk)) => {
                    buffer.extend_from_slice(&chunk);
                    while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                        let line_raw = buffer.split_to(pos + 1);
                        let trimmed = String::from_utf8_lossy(&line_raw).trim().to_string();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let out: Option<Bytes> =
                            if let Some(payload) = trimmed.strip_prefix("data: ") {
                                let payload = payload.trim();
                                if payload == "[DONE]" {
                                    Some(Bytes::from_static(b"data: [DONE]\n\n"))
                                } else {
                                    match serde_json::from_str::<Value>(payload) {
                                        Ok(mut json) => {
                                            // Unwrap the v1internal response wrapper.
                                            if let Some(inner) =
                                                json.get_mut("response").map(|v| v.take())
                                            {
                                                let encoded = serde_json::to_string(&inner)
                                                    .unwrap_or_default();
                                                Some(Bytes::from(format!("data: {}\n\n", encoded)))
                                            } else {
                                                Some(Bytes::from(format!("data: {}\n\n", payload)))
                                            }
                                        }
                                        Err(_) => {
                                            // Not JSON — pass the raw line through.
                                            Some(Bytes::from(format!("{}\n\n", trimmed)))
                                        }
                                    }
                                }
                            } else {
                                // Non-data line (comment / blank / event name).
                                Some(Bytes::from(line_raw))
                            };
                        if let Some(bytes) = out {
                            if tx.send(bytes).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
    });

    let body_stream =
        tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<_, std::convert::Infallible>);
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::from_stream(body_stream))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// Check if a Gemini request asks for streaming.
fn is_gemini_streaming_request(body: &[u8]) -> bool {
    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(stream) = val.get("stream") {
            return stream.as_bool().unwrap_or(false);
        }
    }
    false
}

/// Extract canonical token usage from a Gemini response (`usageMetadata`,
/// cache-inclusive `promptTokenCount` normalized to the fresh-input caliber).
pub fn extract_usage(body: &[u8]) -> crate::proxy::protocol::Usage {
    crate::proxy::protocol::usage_from_response_body(body)
}
