//! axum HTTP proxy server for PoolGate.
//! Binds to 127.0.0.1:{port} and provides routes for:
//! - POST /v1/messages (Anthropic)
//! - POST /v1/chat/completions (OpenAI)
//! - GET /v1/models (aggregated model list)
//! - GET /health (gateway health)
//! - GET /v1/stats (real-time stats)
//!
//! Includes CORS middleware and graceful shutdown via a oneshot channel.

use crate::proxy::auth::{self, ClientKeyAuth};
use crate::proxy::concurrency::AccountConcurrency;
use crate::proxy::health::CircuitBreaker;
use crate::proxy::logger::{LogEvent, LogWriter};
use crate::proxy::protocol::{anthropic, gemini, openai, responses};
use crate::proxy::router::{self, ModelCache, RoutingStrategy};
use crate::services::credentials::authorization_secret;
use crate::AppState;

use axum::{
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use sha2::Digest;
use std::collections::HashSet;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use uuid::Uuid;

const MAX_POOL_ATTEMPTS: usize = 3;

fn should_failover(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::UNAUTHORIZED
            | StatusCode::FORBIDDEN
            | StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
    ) || status.is_server_error()
}

fn attach_route_headers(
    response: &mut Response,
    account_id: &str,
    provider_id: &str,
    attempt: usize,
    request_id: &str,
) {
    // Expose only short fingerprints to clients. Full internal IDs stay in the
    // local request log and are not unnecessarily sent across the API boundary.
    let account_fingerprint = short_fingerprint(account_id);
    let provider_fingerprint = short_fingerprint(provider_id);
    if let Ok(value) = account_fingerprint.parse() {
        response.headers_mut().insert("x-poolgate-account", value);
    }
    if let Ok(value) = provider_fingerprint.parse() {
        response.headers_mut().insert("x-poolgate-provider", value);
    }
    if let Ok(value) = attempt.to_string().parse() {
        response.headers_mut().insert("x-poolgate-attempt", value);
    }
    if let Ok(value) = request_id.parse() {
        response.headers_mut().insert("x-request-id", value);
    }
    response.extensions_mut().insert(RouteAttemptRecorded);
}

fn short_fingerprint(value: &str) -> String {
    let digest = sha2::Sha256::digest(value.as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect()
}

fn enforce_permission_scope(
    auth: &auth::AuthContext,
    protocol: &str,
    model: Option<&str>,
) -> Option<Response> {
    let key = auth.client_key()?;
    auth::permission_scope_check(key, Some(protocol), model)
        .map(|message| auth::auth_error(StatusCode::FORBIDDEN, message))
}

/// Verified audit metadata propagated from authentication to the outer logging
/// middleware without exposing credentials in response headers.
#[derive(Clone, Debug, Default)]
struct GatewayAuditContext {
    group_id: Option<String>,
    client_key_id: Option<String>,
    /// Requested model, captured from the body at the gateway layer so
    /// gateway-level failures (auth, pool-empty, bad JSON) still record it.
    model: Option<String>,
}

/// Structured error detail for gateway-generated responses. The outer logging
/// middleware reads this extension without buffering or consuming the body.
#[derive(Clone, Debug)]
pub(crate) struct GatewayErrorDetail(pub String);

/// Internal-only response marker. Unlike the public diagnostic header, this
/// cannot be forged by an upstream response or client request.
#[derive(Clone, Copy, Debug)]
struct RouteAttemptRecorded;

#[derive(Clone)]
struct RouteLogContext {
    group_id: String,
    client_key_id: Option<String>,
    request_id: String,
    attempt_count: usize,
    account_id: String,
    provider_id: String,
    model: Option<String>,
    endpoint: &'static str,
    status: StatusCode,
    started_at: std::time::Instant,
}

fn defer_stream_route_log_from_response(
    state: Arc<ProxyState>,
    response: &Response,
    context: RouteLogContext,
) -> bool {
    let Some(handle) = response
        .extensions()
        .get::<crate::proxy::stream::StreamCompletionHandle>()
        .cloned()
    else {
        return false;
    };
    defer_stream_route_log(state, handle, context);
    true
}

fn defer_stream_route_log(
    state: Arc<ProxyState>,
    handle: crate::proxy::stream::StreamCompletionHandle,
    context: RouteLogContext,
) {
    tokio::spawn(async move {
        let completion =
            handle
                .wait()
                .await
                .unwrap_or_else(|| crate::proxy::stream::StreamCompletion {
                    usage: crate::proxy::protocol::Usage::default(),
                    error_message: Some("SSE completion callback was dropped".into()),
                });
        // The streaming body has finished (or the client disconnected) —
        // release the connection counter that logging_middleware kept open.
        state.app_state.gateway_runtime.connection_closed();
        let status = if completion.error_message.is_some() {
            StatusCode::BAD_GATEWAY
        } else {
            context.status
        };
        write_route_log(
            &state,
            &context.group_id,
            context.client_key_id,
            &context.request_id,
            context.attempt_count,
            &context.account_id,
            &context.provider_id,
            context.model,
            context.endpoint,
            Some(status),
            context.started_at.elapsed().as_millis() as i64,
            true,
            completion.error_message,
            completion.usage,
        )
        .await;
    });
}

async fn write_route_log(
    state: &ProxyState,
    group_id: &str,
    client_key_id: Option<String>,
    request_id: &str,
    attempt_count: usize,
    account_id: &str,
    provider_id: &str,
    model: Option<String>,
    endpoint: &str,
    status_code: Option<StatusCode>,
    latency_ms: i64,
    is_streaming: bool,
    error_message: Option<String>,
    usage: crate::proxy::protocol::Usage,
) {
    let success = status_code
        .map(|status| status.is_success())
        .unwrap_or(false);
    let event = LogEvent {
        group_id: Some(group_id.to_string()),
        client_key_id,
        request_id: Some(request_id.to_string()),
        attempt_count: attempt_count as i64,
        usage_available: usage.available,
        source: Some("proxy".to_string()),
        provider_id: Some(provider_id.to_string()),
        account_id: Some(account_id.to_string()),
        model,
        endpoint: Some(endpoint.to_string()),
        status: if success { "success" } else { "error" }.to_string(),
        status_code: status_code.map(|status| status.as_u16() as i64),
        input_tokens: usage.available.then_some(usage.input_tokens),
        output_tokens: usage.available.then_some(usage.output_tokens),
        cache_tokens: usage.available.then_some(usage.cache_tokens),
        cost: None,
        latency_ms: Some(latency_ms),
        ttft_ms: None,
        is_stream: Some(is_streaming),
        error_message,
    };
    let _ = state.log_writer.write_log(event).await;
}

/// Shared proxy runtime state.
pub struct ProxyState {
    pub app_state: Arc<AppState>,
    pub model_cache: ModelCache,
    pub circuit_breaker: CircuitBreaker,
    pub account_concurrency: AccountConcurrency,
    pub log_writer: LogWriter,
    pub startup_time: std::time::Instant,
}

/// Start the proxy server.
///
/// # Arguments
/// * `app_state` - Shared application state (database etc.).
/// * `port` - Port to bind to (default 9800).
/// * `shutdown_rx` - Receiver for graceful shutdown signal.
pub async fn start_proxy_server(
    app_state: Arc<AppState>,
    port: u16,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let model_cache = ModelCache::default();
    let circuit_breaker = CircuitBreaker::default();
    // The LogWriter clones the inner Database; we need to re-create from the same path
    // We use a separate database connection for logging to avoid contention
    let log_writer = LogWriter::new_with_app_state(&app_state);

    let proxy_state = Arc::new(ProxyState {
        app_state: app_state.clone(),
        model_cache,
        circuit_breaker,
        account_concurrency: app_state.account_concurrency.clone(),
        log_writer,
        startup_time: std::time::Instant::now(),
    });

    // Browser access is limited to local desktop/web development origins and
    // the exact methods/headers used by supported Agent clients.
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _parts| {
            is_allowed_local_origin(origin)
        }))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            "x-api-key".parse().expect("valid header name"),
            "x-goog-api-key".parse().expect("valid header name"),
            "x-group-id".parse().expect("valid header name"),
            "x-pool-group".parse().expect("valid header name"),
            "x-request-id".parse().expect("valid header name"),
            "anthropic-version".parse().expect("valid header name"),
        ])
        .expose_headers([
            "x-poolgate-account".parse().expect("valid header name"),
            "x-poolgate-provider".parse().expect("valid header name"),
            "x-poolgate-attempt".parse().expect("valid header name"),
            "x-request-id".parse().expect("valid header name"),
        ]);

    // Build router
    let app = Router::new()
        // Anthropic-compatible endpoint
        .route("/v1/messages", post(anthropic_handler))
        // OpenAI-compatible endpoint (legacy direct passthrough)
        .route("/v1/chat/completions", post(openai_handler))
        // Unified Responses API entry: route takeover converts chat/anthropic → Responses
        .route("/v1/responses", post(responses_handler))
        // Gemini-compatible endpoints (native path shape)
        .route("/v1beta/models/{model_action}", post(gemini_handler))
        .route("/v1/models/{model_action}", post(gemini_handler))
        // Aggregated model list
        .route("/v1/models", get(models_handler))
        // Gateway health
        .route("/health", get(health_handler))
        // Real-time stats
        .route("/v1/stats", get(stats_handler))
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
        .layer(axum::middleware::from_fn_with_state(
            proxy_state.clone(),
            auth_middleware,
        ))
        // Layers execute bottom-up. CORS wraps authentication so local browser
        // preflight and 401 responses receive the required CORS headers.
        .layer(cors)
        .layer(axum::middleware::from_fn_with_state(
            proxy_state.clone(),
            logging_middleware,
        ))
        .with_state(proxy_state.clone());

    let addr = format!("127.0.0.1:{}", port);
    tracing::info!("PoolGate proxy server starting on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
            tracing::info!("Proxy server shutting down gracefully...");
        })
        .await?;

    tracing::info!("Proxy server stopped");
    Ok(())
}

// ─── Route Handlers ──────────────────────────────────────────────────────────

/// Handle POST /v1/chat/completions (OpenAI-compatible).
async fn openai_handler(
    State(state): State<Arc<ProxyState>>,
    auth: auth::AuthContext,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let start = std::time::Instant::now();
    let request_id = request_id_from_headers(&headers);
    let group_id = match auth::resolve_routing_group(
        auth::explicit_group_from_headers(&headers),
        auth.client_key(),
    ) {
        Ok(g) => g,
        Err(response) => return response,
    };
    let strategy = get_group_strategy(&state, &group_id).await;
    let model = openai::extract_model(&body);
    if let Some(response) = enforce_permission_scope(&auth, "chat", model.as_deref()) {
        return response;
    }
    let is_streaming = is_streaming_request(&body);
    let mut excluded = HashSet::new();
    let mut last_response = None;

    for attempt in 1..=MAX_POOL_ATTEMPTS {
        let (account, provider) = match router::select_account_excluding(
            &group_id,
            &strategy,
            model.as_deref(),
            Some("chat"),
            &excluded,
            &state.app_state,
        )
        .await
        {
            Ok(pair) => pair,
            Err(reason) => {
                return last_response
                    .unwrap_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, reason));
            }
        };
        excluded.insert(account.id.clone());
        state.app_state.gateway_runtime.select_route(
            &request_id,
            "chat",
            &group_id,
            &provider.id,
            &account.id,
            attempt,
        );

        if !state.circuit_breaker.allow_request(&account.id).await {
            continue;
        }
        let permit = match state.account_concurrency.acquire(&account.id).await {
            Ok(permit) => permit,
            Err(error) => {
                tracing::warn!("Account '{}' is at capacity: {}", account.id, error);
                continue;
            }
        };

        let result =
            openai::handle_openai_request(body.clone(), &account, &provider, Some(permit)).await;
        let latency_ms = start.elapsed().as_millis() as i64;
        match result {
            Ok((mut response, usage)) => {
                let status = response.status();
                let retry = should_failover(status) && attempt < MAX_POOL_ATTEMPTS;
                if status.is_success() {
                    state.circuit_breaker.record_success(&account.id).await;
                } else if should_failover(status) {
                    state.circuit_breaker.record_failure(&account.id).await;
                }
                let deferred_stream_log = is_streaming
                    && status.is_success()
                    && defer_stream_route_log_from_response(
                        state.clone(),
                        &response,
                        RouteLogContext {
                            group_id: group_id.clone(),
                            client_key_id: auth.client_key().map(|k| k.key_id.clone()),
                            request_id: request_id.clone(),
                            attempt_count: attempt,
                            account_id: account.id.clone(),
                            provider_id: provider.id.clone(),
                            model: model.clone(),
                            endpoint: "/v1/chat/completions",
                            status,
                            started_at: start,
                        },
                    );
                if !deferred_stream_log {
                    write_route_log(
                        &state,
                        &group_id,
                        auth.client_key().map(|k| k.key_id.clone()),
                        &request_id,
                        attempt,
                        &account.id,
                        &provider.id,
                        model.clone(),
                        "/v1/chat/completions",
                        Some(status),
                        latency_ms,
                        is_streaming,
                        (!status.is_success()).then(|| status.to_string()),
                        usage,
                    )
                    .await;
                }
                attach_route_headers(
                    &mut response,
                    &account.id,
                    &provider.id,
                    attempt,
                    &request_id,
                );
                if retry {
                    last_response = Some(response);
                    continue;
                }
                return response;
            }
            Err(status) => {
                state.circuit_breaker.record_failure(&account.id).await;
                write_route_log(
                    &state,
                    &group_id,
                    auth.client_key().map(|k| k.key_id.clone()),
                    &request_id,
                    attempt,
                    &account.id,
                    &provider.id,
                    model.clone(),
                    "/v1/chat/completions",
                    Some(status),
                    latency_ms,
                    is_streaming,
                    Some(status.to_string()),
                    crate::proxy::protocol::Usage::default(),
                )
                .await;
                if attempt < MAX_POOL_ATTEMPTS {
                    continue;
                }
                let mut response = error_response(status, "Upstream request failed");
                attach_route_headers(
                    &mut response,
                    &account.id,
                    &provider.id,
                    attempt,
                    &request_id,
                );
                return response;
            }
        }
    }

    last_response
        .unwrap_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "账号池中没有可用账号"))
}

/// Handle POST /v1/responses (unified Responses API entry with route takeover).
async fn responses_handler(
    State(state): State<Arc<ProxyState>>,
    auth: auth::AuthContext,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let start = std::time::Instant::now();
    let request_id = request_id_from_headers(&headers);
    let group_id = match auth::resolve_routing_group(
        auth::explicit_group_from_headers(&headers),
        auth.client_key(),
    ) {
        Ok(g) => g,
        Err(response) => return response,
    };
    let strategy = get_group_strategy(&state, &group_id).await;
    let model = openai::extract_model(&body);
    if let Some(response) = enforce_permission_scope(&auth, "responses", model.as_deref()) {
        return response;
    }
    let is_streaming = is_streaming_request(&body);
    let mut parsed: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid JSON body"),
    };
    // /v1/responses 规范要求 `input`，但不少客户端按 Chat Completions 风格发
    // `messages`。messages 数组与 Responses input 数组的简化形式（{role,
    // content}）兼容，统一映射到 input，避免上游（尤其 Gemini）收到未知字段
    // 报 400 Invalid JSON payload。
    if parsed.get("input").is_none() {
        if let Some(messages) = parsed.get("messages").cloned() {
            parsed["input"] = messages;
            if let Some(object) = parsed.as_object_mut() {
                object.remove("messages");
            }
        }
    }
    let mut excluded = HashSet::new();
    let mut last_response = None;

    for attempt in 1..=MAX_POOL_ATTEMPTS {
        let (account, provider) = match router::select_account_excluding(
            &group_id,
            &strategy,
            model.as_deref(),
            Some("responses"),
            &excluded,
            &state.app_state,
        )
        .await
        {
            Ok(pair) => pair,
            Err(reason) => {
                return last_response
                    .unwrap_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, reason))
            }
        };
        excluded.insert(account.id.clone());
        state.app_state.gateway_runtime.select_route(
            &request_id,
            "responses",
            &group_id,
            &provider.id,
            &account.id,
            attempt,
        );
        if !state.circuit_breaker.allow_request(&account.id).await {
            continue;
        }
        let mut permit = match state.account_concurrency.acquire(&account.id).await {
            Ok(permit) => Some(permit),
            Err(error) => {
                tracing::warn!("Account '{}' is at capacity: {}", account.id, error);
                continue;
            }
        };

        let acct_protocols = router::parse_protocols(account.protocols.as_deref());
        let protocols = if acct_protocols.is_empty() {
            router::parse_protocols(provider.protocols.as_deref())
        } else {
            acct_protocols
        };
        let mut upstream_body = responses::convert_request(&protocols, &parsed);
        let upstream_kind = responses::upstream_kind(&protocols);
        let upstream_protocol = match upstream_kind {
            responses::UpstreamKind::Responses => "responses",
            responses::UpstreamKind::Chat => "chat",
            responses::UpstreamKind::Anthropic => "anthropic",
            responses::UpstreamKind::Gemini => "gemini",
            responses::UpstreamKind::Unsupported => {
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    "Selected account has no supported upstream protocol",
                );
            }
        };

        // Antigravity (Cloud Code) speaks only the private v1internal endpoint,
        // not the public `/v1beta/models/...` Gemini shape. Inject the Cloud
        // Code project id (mock fallback included) and keep `model` set on the
        // generateContent body so the unified Responses entry can reach it.
        let antigravity_gemini = upstream_kind == responses::UpstreamKind::Gemini
            && crate::proxy::protocol::gemini::is_antigravity(&provider);
        if antigravity_gemini {
            crate::proxy::protocol::gemini::inject_antigravity_context(
                &mut upstream_body,
                &account,
                model.as_deref(),
            );
        }

        // Copilot accounts need dynamic endpoint selection based on model.
        // GPT-5/o3/o4 families use /responses, others use /chat/completions.
        let is_copilot = crate::services::copilot_adapter::is_copilot_pat(&account, &provider)
            || crate::services::copilot_adapter::is_copilot_oauth(&account, &provider);

        let selected_base_url = provider.base_url_for_protocol(upstream_protocol);
        let is_codex = upstream_kind == responses::UpstreamKind::Responses
            && crate::services::codex_adapter::is_codex_oauth(&account, &provider);
        let codex_context = if is_codex {
            match crate::services::codex_adapter::request_context(&account) {
                Ok(context) => Some(context),
                Err(error) => {
                    state.circuit_breaker.record_failure(&account.id).await;
                    if attempt < MAX_POOL_ATTEMPTS {
                        continue;
                    }
                    return error_response(StatusCode::UNAUTHORIZED, error);
                }
            }
        } else {
            None
        };
        if is_codex {
            upstream_body =
                match crate::services::codex_adapter::prepare_responses_body(&upstream_body) {
                    Ok(body) => body,
                    Err(error) => {
                        // Codex OAuth incompatible with this request (e.g. stream
                        // not enabled). Record failure and retry with another account
                        // instead of hard-failing, so non-Codex accounts can handle it.
                        state.circuit_breaker.record_failure(&account.id).await;
                        excluded.insert(account.id.clone());
                        if attempt < MAX_POOL_ATTEMPTS {
                            tracing::debug!(
                                "Codex OAuth incompatible ({}), trying next account: {}",
                                error, account.id
                            );
                            continue;
                        }
                        return error_response(StatusCode::BAD_REQUEST, error);
                    }
                };
        }
        let mut path = upstream_kind.path().to_string();
        if is_codex && selected_base_url.contains("/backend-api/codex") {
            path = "/responses".to_string();
        }
        // Copilot dynamic endpoint: GPT-5/o3/o4 → /responses, others → /chat/completions
        if is_copilot && upstream_kind == responses::UpstreamKind::Responses {
            let should_use_responses = model.as_deref().is_some_and(|m| {
                m.starts_with("gpt-5")
                    || m.starts_with("o3")
                    || m.starts_with("o4")
                    || m.contains("codex")
            });
            if !should_use_responses {
                path = "/v1/chat/completions".to_string();
            }
        }
        if upstream_kind == responses::UpstreamKind::Gemini {
            if let Some(model) = model.as_deref() {
                let action = if is_streaming {
                    "streamGenerateContent"
                } else {
                    "generateContent"
                };
                if antigravity_gemini {
                    // Antigravity upstream has no /v1beta/models; only the
                    // private v1internal methods exist on cloudcode-pa.
                    path = format!("/v1internal:{}", action);
                } else {
                    path = format!("/v1beta/models/{}:{}", model, action);
                }
            }
        }
        let url = crate::proxy::protocol::build_upstream_url(&selected_base_url, &path);
        let credential = match upstream_kind {
            responses::UpstreamKind::Anthropic | responses::UpstreamKind::Gemini => {
                crate::services::credentials::auth_credential(&account)
            }
            _ => authorization_secret(&account)
                .map(crate::services::credentials::AuthCredential::Bearer),
        };
        let credential = match credential {
            Ok(value) => value,
            Err(error) => {
                state.circuit_breaker.record_failure(&account.id).await;
                excluded.insert(account.id.clone());
                if attempt < MAX_POOL_ATTEMPTS {
                    continue;
                }
                return error_response(StatusCode::UNAUTHORIZED, error);
            }
        };
        let mut client_builder = reqwest::Client::builder().timeout(
            std::time::Duration::from_millis(provider.timeout_ms.unwrap_or(60_000).max(1) as u64),
        );
        if let Some(proxy_url) = provider
            .proxy_url
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            if let Ok(proxy) = reqwest::Proxy::all(proxy_url) {
                client_builder = client_builder.proxy(proxy);
            }
        }
        let client = client_builder
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let mut req = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&upstream_body);
        // Add Copilot-Integration-Id header for Copilot accounts.
        if is_copilot {
            req = req.header(
                "Copilot-Integration-Id",
                crate::services::copilot_adapter::COPILOT_INTEGRATION_ID,
            );
        }
        req = if let Some(context) = codex_context.as_ref() {
            crate::services::codex_adapter::apply_responses_headers(req, context)
        } else {
            match (upstream_kind, credential) {
                (
                    responses::UpstreamKind::Anthropic,
                    crate::services::credentials::AuthCredential::ApiKey(secret),
                ) => req
                    .header("x-api-key", secret)
                    .header("anthropic-version", "2023-06-01"),
                (
                    responses::UpstreamKind::Anthropic,
                    crate::services::credentials::AuthCredential::Bearer(secret),
                ) => req
                    .header("Authorization", format!("Bearer {}", secret))
                    .header("anthropic-version", "2023-06-01"),
                (
                    responses::UpstreamKind::Gemini,
                    crate::services::credentials::AuthCredential::ApiKey(secret),
                ) => {
                    let req = req.query(&[("key", secret)]);
                    if is_streaming {
                        req.query(&[("alt", "sse")])
                    } else {
                        req
                    }
                }
                (
                    responses::UpstreamKind::Gemini,
                    crate::services::credentials::AuthCredential::Bearer(secret),
                ) => {
                    let req = req.header("Authorization", format!("Bearer {}", secret));
                    if is_streaming {
                        req.query(&[("alt", "sse")])
                    } else {
                        req
                    }
                }
                (_, crate::services::credentials::AuthCredential::Bearer(secret)) => {
                    req.header("Authorization", format!("Bearer {}", secret))
                }
                (_, crate::services::credentials::AuthCredential::ApiKey(secret)) => {
                    req.header("Authorization", format!("Bearer {}", secret))
                }
            }
        };
        // Antigravity v1internal requires the official client User-Agent.
        if antigravity_gemini {
            req = req.header(
                "User-Agent",
                crate::services::antigravity_adapter::ANTIGRAVITY_USER_AGENT,
            );
            req = req.header(
                "X-Goog-Api-Client",
                "google-cloud-sdk vscode_cloudshelleditor/0.1",
            );
            req = req.header(
                "Client-Metadata",
                r#"{"ideType":"IDE_UNSPECIFIED","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}"#,
            );
        }
        let mut send_result = match provider.apply_custom_headers(req) {
            Ok(request) => request.send().await,
            Err(error) => {
                state.circuit_breaker.record_failure(&account.id).await;
                return error_response(StatusCode::BAD_REQUEST, error);
            }
        };
        if send_result
            .as_ref()
            .is_ok_and(|response| response.status() == StatusCode::UNAUTHORIZED)
        {
            if let Some(context) = codex_context.as_ref() {
                match crate::services::codex_adapter::refresh_after_unauthorized(
                    &state.app_state,
                    &account.id,
                    &context.access_token,
                )
                .await
                {
                    Ok(refreshed_account) => {
                        match crate::services::codex_adapter::request_context(&refreshed_account) {
                            Ok(refreshed_context) => {
                                let retry_request = client
                                    .post(&url)
                                    .header("Content-Type", "application/json")
                                    .json(&upstream_body);
                                let retry_request =
                                    crate::services::codex_adapter::apply_responses_headers(
                                        retry_request,
                                        &refreshed_context,
                                    );
                                send_result = match provider.apply_custom_headers(retry_request) {
                                    Ok(request) => request.send().await,
                                    Err(error) => {
                                        tracing::warn!(
                                            "Invalid custom headers for provider '{}': {}",
                                            provider.id,
                                            error
                                        );
                                        break;
                                    }
                                };
                            }
                            Err(error) => {
                                tracing::warn!(
                                    "Codex token refreshed but request context is invalid for '{}': {}",
                                    account.id,
                                    error
                                );
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            "Codex 401 token refresh failed for '{}': {}",
                            account.id,
                            crate::services::redaction::redact_sensitive(&error)
                        );
                    }
                }
            }
        }
        let latency_ms = start.elapsed().as_millis() as i64;
        match send_result {
            Ok(response) => {
                let status = response.status();
                let retry = should_failover(status) && attempt < MAX_POOL_ATTEMPTS;
                if status.is_success() && is_codex {
                    state
                        .app_state
                        .db
                        .accounts
                        .update_health(
                            &state.app_state.db.conn,
                            &account.id,
                            "healthy",
                            200,
                            "Codex Responses connectivity verified",
                            latency_ms,
                        )
                        .ok();
                    state
                        .app_state
                        .db
                        .accounts
                        .recover_status(
                            &state.app_state.db.conn,
                            &account.id,
                            account.status.as_deref(),
                        )
                        .ok();
                } else if status == StatusCode::UNAUTHORIZED && is_codex {
                    state
                        .app_state
                        .db
                        .accounts
                        .update_health(
                            &state.app_state.db.conn,
                            &account.id,
                            "error",
                            401,
                            "Codex OAuth authorization failed after one token refresh retry",
                            latency_ms,
                        )
                        .ok();
                }
                if status.is_success() && is_streaming {
                    let mut routed = if upstream_kind == responses::UpstreamKind::Responses {
                        crate::proxy::stream::forward_sse_stream_with_context(
                            response,
                            Some(account.id.clone()),
                            Some(request_id.clone()),
                            permit.take(),
                        )
                    } else {
                        responses::forward_responses_sse_with_context(
                            response,
                            protocols.clone(),
                            Some(account.id.clone()),
                            Some(request_id.clone()),
                            permit.take(),
                        )
                    };
                    state.circuit_breaker.record_success(&account.id).await;
                    if !defer_stream_route_log_from_response(
                        state.clone(),
                        &routed,
                        RouteLogContext {
                            group_id: group_id.clone(),
                            client_key_id: auth.client_key().map(|k| k.key_id.clone()),
                            request_id: request_id.clone(),
                            attempt_count: attempt,
                            account_id: account.id.clone(),
                            provider_id: provider.id.clone(),
                            model: model.clone(),
                            endpoint: "/v1/responses",
                            status,
                            started_at: start,
                        },
                    ) {
                        write_route_log(
                            &state,
                            &group_id,
                            auth.client_key().map(|k| k.key_id.clone()),
                            &request_id,
                            attempt,
                            &account.id,
                            &provider.id,
                            model.clone(),
                            "/v1/responses",
                            Some(status),
                            latency_ms,
                            true,
                            None,
                            crate::proxy::protocol::Usage::default(),
                        )
                        .await;
                    }
                    attach_route_headers(
                        &mut routed,
                        &account.id,
                        &provider.id,
                        attempt,
                        &request_id,
                    );
                    return routed;
                }
                let raw_text = response.text().await.unwrap_or_default();
                let normalized = responses::normalize_response(&protocols, &raw_text);
                if status.is_success() {
                    state.circuit_breaker.record_success(&account.id).await;
                } else if should_failover(status) {
                    state.circuit_breaker.record_failure(&account.id).await;
                }
                write_route_log(
                    &state,
                    &group_id,
                    auth.client_key().map(|k| k.key_id.clone()),
                    &request_id,
                    attempt,
                    &account.id,
                    &provider.id,
                    model.clone(),
                    "/v1/responses",
                    Some(status),
                    latency_ms,
                    is_streaming,
                    (!status.is_success()).then(|| status.to_string()),
                    usage_from_response_text(upstream_kind, &raw_text),
                )
                .await;
                let mut routed = if status.is_success() && is_streaming {
                    ([(header::CONTENT_TYPE, "text/event-stream")], raw_text).into_response()
                } else if status.is_success() {
                    ([(header::CONTENT_TYPE, "application/json")], normalized).into_response()
                } else {
                    error_response(status, normalized)
                };
                attach_route_headers(&mut routed, &account.id, &provider.id, attempt, &request_id);
                if retry {
                    last_response = Some(routed);
                    continue;
                }
                return routed;
            }
            Err(error) => {
                state.circuit_breaker.record_failure(&account.id).await;
                write_route_log(
                    &state,
                    &group_id,
                    auth.client_key().map(|k| k.key_id.clone()),
                    &request_id,
                    attempt,
                    &account.id,
                    &provider.id,
                    model.clone(),
                    "/v1/responses",
                    None,
                    latency_ms,
                    is_streaming,
                    Some(error.to_string()),
                    crate::proxy::protocol::Usage::default(),
                )
                .await;
                if attempt < MAX_POOL_ATTEMPTS {
                    continue;
                }
                let mut routed = error_response(StatusCode::BAD_GATEWAY, "Upstream request failed");
                attach_route_headers(&mut routed, &account.id, &provider.id, attempt, &request_id);
                return routed;
            }
        }
    }
    last_response
        .unwrap_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "账号池中没有可用账号"))
}

/// Handle POST /v1/messages (Anthropic-compatible).
async fn anthropic_handler(
    State(state): State<Arc<ProxyState>>,
    auth: auth::AuthContext,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let start = std::time::Instant::now();
    let request_id = request_id_from_headers(&headers);
    let group_id = match auth::resolve_routing_group(
        auth::explicit_group_from_headers(&headers),
        auth.client_key(),
    ) {
        Ok(g) => g,
        Err(response) => return response,
    };
    let strategy = get_group_strategy(&state, &group_id).await;
    let model = anthropic::extract_model(&body);
    if let Some(response) = enforce_permission_scope(&auth, "anthropic", model.as_deref()) {
        return response;
    }
    let is_streaming = is_streaming_request(&body);
    let mut excluded = HashSet::new();
    let mut last_response = None;

    for attempt in 1..=MAX_POOL_ATTEMPTS {
        let (account, provider) = match router::select_account_excluding(
            &group_id,
            &strategy,
            model.as_deref(),
            Some("anthropic"),
            &excluded,
            &state.app_state,
        )
        .await
        {
            Ok(pair) => pair,
            Err(reason) => {
                return last_response
                    .unwrap_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, reason))
            }
        };
        excluded.insert(account.id.clone());
        state.app_state.gateway_runtime.select_route(
            &request_id,
            "anthropic",
            &group_id,
            &provider.id,
            &account.id,
            attempt,
        );
        if !state.circuit_breaker.allow_request(&account.id).await {
            continue;
        }
        let permit = match state.account_concurrency.acquire(&account.id).await {
            Ok(permit) => permit,
            Err(error) => {
                tracing::warn!("Account '{}' is at capacity: {}", account.id, error);
                continue;
            }
        };

        let result =
            anthropic::handle_anthropic_request(body.clone(), &account, &provider, Some(permit))
                .await;
        let latency_ms = start.elapsed().as_millis() as i64;
        match result {
            Ok((mut response, usage)) => {
                let status = response.status();
                let retry = should_failover(status) && attempt < MAX_POOL_ATTEMPTS;
                if status.is_success() {
                    state.circuit_breaker.record_success(&account.id).await;
                } else if should_failover(status) {
                    state.circuit_breaker.record_failure(&account.id).await;
                }
                let deferred_stream_log = is_streaming
                    && status.is_success()
                    && defer_stream_route_log_from_response(
                        state.clone(),
                        &response,
                        RouteLogContext {
                            group_id: group_id.clone(),
                            client_key_id: auth.client_key().map(|k| k.key_id.clone()),
                            request_id: request_id.clone(),
                            attempt_count: attempt,
                            account_id: account.id.clone(),
                            provider_id: provider.id.clone(),
                            model: model.clone(),
                            endpoint: "/v1/messages",
                            status,
                            started_at: start,
                        },
                    );
                if !deferred_stream_log {
                    write_route_log(
                        &state,
                        &group_id,
                        auth.client_key().map(|k| k.key_id.clone()),
                        &request_id,
                        attempt,
                        &account.id,
                        &provider.id,
                        model.clone(),
                        "/v1/messages",
                        Some(status),
                        latency_ms,
                        is_streaming,
                        (!status.is_success()).then(|| status.to_string()),
                        usage,
                    )
                    .await;
                }
                attach_route_headers(
                    &mut response,
                    &account.id,
                    &provider.id,
                    attempt,
                    &request_id,
                );
                if retry {
                    last_response = Some(response);
                    continue;
                }
                return response;
            }
            Err(status) => {
                state.circuit_breaker.record_failure(&account.id).await;
                write_route_log(
                    &state,
                    &group_id,
                    auth.client_key().map(|k| k.key_id.clone()),
                    &request_id,
                    attempt,
                    &account.id,
                    &provider.id,
                    model.clone(),
                    "/v1/messages",
                    Some(status),
                    latency_ms,
                    is_streaming,
                    Some(status.to_string()),
                    crate::proxy::protocol::Usage::default(),
                )
                .await;
                if attempt < MAX_POOL_ATTEMPTS {
                    continue;
                }
                let mut response = error_response(status, "Upstream request failed");
                attach_route_headers(
                    &mut response,
                    &account.id,
                    &provider.id,
                    attempt,
                    &request_id,
                );
                return response;
            }
        }
    }
    last_response
        .unwrap_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "账号池中没有可用账号"))
}

/// Handle GET /v1/models, scoped to the authenticated route pool and key.
async fn models_handler(
    State(state): State<Arc<ProxyState>>,
    auth: auth::AuthContext,
    headers: HeaderMap,
) -> Response {
    let group_id = match auth::resolve_routing_group(
        auth::explicit_group_from_headers(&headers),
        auth.client_key(),
    ) {
        Ok(group_id) => group_id,
        Err(response) => return response,
    };
    let allowed_models = auth
        .client_key()
        .and_then(|key| key.allowed_models.as_deref());
    let models = router::get_models_for_scope(
        &state.app_state,
        &state.model_cache,
        &group_id,
        allowed_models,
    )
    .await;
    Json(serde_json::json!({
        "object": "list",
        "data": models,
    }))
    .into_response()
}

/// Handle Gemini-compatible requests.
///
/// The captured path segment looks like `{model}:generateContent` or
/// `{model}:streamGenerateContent`. We use the model portion for
/// capability-aware routing, then forward the raw body to the selected
/// account's Gemini provider.
async fn gemini_handler(
    State(state): State<Arc<ProxyState>>,
    auth: auth::AuthContext,
    headers: HeaderMap,
    axum::extract::Path(model_action): axum::extract::Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let start = std::time::Instant::now();
    let request_id = request_id_from_headers(&headers);
    let group_id = match auth::resolve_routing_group(
        auth::explicit_group_from_headers(&headers),
        auth.client_key(),
    ) {
        Ok(g) => g,
        Err(response) => return response,
    };
    let strategy = get_group_strategy(&state, &group_id).await;
    let model = model_action
        .split(':')
        .next()
        .filter(|m| !m.is_empty())
        .map(|m| m.to_string());
    if let Some(response) = enforce_permission_scope(&auth, "gemini", model.as_deref()) {
        return response;
    }
    let is_streaming =
        model_action.contains("streamGenerateContent") || is_streaming_request(&body);
    let mut excluded = HashSet::new();
    let mut last_response = None;

    for attempt in 1..=MAX_POOL_ATTEMPTS {
        let (account, provider) = match router::select_account_excluding(
            &group_id,
            &strategy,
            model.as_deref(),
            Some("gemini"),
            &excluded,
            &state.app_state,
        )
        .await
        {
            Ok(pair) => pair,
            Err(reason) => {
                return last_response
                    .unwrap_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, reason))
            }
        };
        excluded.insert(account.id.clone());
        state.app_state.gateway_runtime.select_route(
            &request_id,
            "gemini",
            &group_id,
            &provider.id,
            &account.id,
            attempt,
        );
        if !state.circuit_breaker.allow_request(&account.id).await {
            continue;
        }
        let permit = match state.account_concurrency.acquire(&account.id).await {
            Ok(permit) => permit,
            Err(error) => {
                tracing::warn!("Account '{}' is at capacity: {}", account.id, error);
                continue;
            }
        };

        let result = gemini::handle_gemini_request(
            body.clone(),
            &account,
            &provider,
            &model_action,
            Some(permit),
        )
        .await;
        let latency_ms = start.elapsed().as_millis() as i64;
        match result {
            Ok((mut response, usage)) => {
                let status = response.status();
                let retry = should_failover(status) && attempt < MAX_POOL_ATTEMPTS;
                if status.is_success() {
                    state.circuit_breaker.record_success(&account.id).await;
                } else if should_failover(status) {
                    state.circuit_breaker.record_failure(&account.id).await;
                }
                let deferred_stream_log = is_streaming
                    && status.is_success()
                    && defer_stream_route_log_from_response(
                        state.clone(),
                        &response,
                        RouteLogContext {
                            group_id: group_id.clone(),
                            client_key_id: auth.client_key().map(|k| k.key_id.clone()),
                            request_id: request_id.clone(),
                            attempt_count: attempt,
                            account_id: account.id.clone(),
                            provider_id: provider.id.clone(),
                            model: model.clone(),
                            endpoint: "/v1beta/models",
                            status,
                            started_at: start,
                        },
                    );
                if !deferred_stream_log {
                    write_route_log(
                        &state,
                        &group_id,
                        auth.client_key().map(|k| k.key_id.clone()),
                        &request_id,
                        attempt,
                        &account.id,
                        &provider.id,
                        model.clone(),
                        "/v1beta/models",
                        Some(status),
                        latency_ms,
                        is_streaming,
                        (!status.is_success()).then(|| status.to_string()),
                        usage,
                    )
                    .await;
                }
                attach_route_headers(
                    &mut response,
                    &account.id,
                    &provider.id,
                    attempt,
                    &request_id,
                );
                if retry {
                    last_response = Some(response);
                    continue;
                }
                return response;
            }
            Err(status) => {
                state.circuit_breaker.record_failure(&account.id).await;
                write_route_log(
                    &state,
                    &group_id,
                    auth.client_key().map(|k| k.key_id.clone()),
                    &request_id,
                    attempt,
                    &account.id,
                    &provider.id,
                    model.clone(),
                    "/v1beta/models",
                    Some(status),
                    latency_ms,
                    is_streaming,
                    Some(status.to_string()),
                    crate::proxy::protocol::Usage::default(),
                )
                .await;
                if attempt < MAX_POOL_ATTEMPTS {
                    continue;
                }
                let mut response = error_response(status, "Upstream request failed");
                attach_route_headers(
                    &mut response,
                    &account.id,
                    &provider.id,
                    attempt,
                    &request_id,
                );
                return response;
            }
        }
    }
    last_response
        .unwrap_or_else(|| error_response(StatusCode::SERVICE_UNAVAILABLE, "账号池中没有可用账号"))
}

/// Handle GET /health (gateway health check).
async fn health_handler(State(state): State<Arc<ProxyState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": state.startup_time.elapsed().as_secs(),
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}

/// Handle GET /v1/stats (real-time statistics).
async fn stats_handler(State(state): State<Arc<ProxyState>>) -> Json<serde_json::Value> {
    let stats = router::get_stats_response(&state.app_state).await;
    Json(stats)
}

// ─── Middleware ───────────────────────────────────────────────────────────────

/// Outermost request audit middleware.
///
/// Every request that reaches the axum router receives one stable request ID.
/// Routed proxy requests already persist one row per upstream attempt; all
/// other requests receive a request-level row here with `attempt_count = 0`.
async fn logging_middleware(
    State(state): State<Arc<ProxyState>>,
    method: Method,
    uri: Uri,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let start = std::time::Instant::now();
    // Manual connection counter (not the RAII guard): SSE streaming responses
    // keep the body alive after the response head is sent, so the count must
    // stay >0 until the body finishes. Non-streaming responses release it
    // right here; streaming ones release it in `defer_stream_route_log`.
    let runtime = &state.app_state.gateway_runtime;
    runtime.connection_opened();
    let request_id = request_id_from_headers(request.headers());
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        request.headers_mut().insert("x-request-id", value);
    }

    let mut response = next.run(request).await;
    let latency = start.elapsed();
    let status = response.status();
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", value);
    }

    // Release the connection counter unless the response is streaming — those
    // are released by the stream completion callback in defer_stream_route_log.
    if response
        .extensions()
        .get::<crate::proxy::stream::StreamCompletionHandle>()
        .is_none()
    {
        runtime.connection_closed();
    }

    tracing::info!(
        "request_id={} {} {} -> {} ({}ms)",
        request_id,
        method,
        uri,
        status.as_u16(),
        latency.as_millis(),
    );

    // A private response extension is attached only after at least one upstream
    // attempt has been persisted (or its SSE completion log has been scheduled).
    // Do not emit a duplicate request-level row in that case.
    state.app_state.gateway_runtime.complete_route(
        &request_id,
        if status.is_success() {
            "success"
        } else {
            "failed"
        },
        latency.as_millis() as i64,
    );

    if should_write_gateway_log(&response) {
        let audit = response
            .extensions()
            .get::<GatewayAuditContext>()
            .cloned()
            .unwrap_or_default();
        let error_message = response
            .extensions()
            .get::<GatewayErrorDetail>()
            .map(|detail| detail.0.clone())
            .or_else(|| (!status.is_success()).then(|| status.to_string()));
        let event = gateway_log_event(
            audit,
            request_id,
            method.as_str(),
            &uri,
            status,
            latency.as_millis() as i64,
            error_message,
        );
        if let Err(error) = state.log_writer.write_log(event).await {
            tracing::error!("Failed to queue gateway request log: {}", error);
        }
    }

    response
}

fn should_write_gateway_log(response: &Response) -> bool {
    response
        .extensions()
        .get::<RouteAttemptRecorded>()
        .is_none()
}

fn gateway_log_event(
    audit: GatewayAuditContext,
    request_id: String,
    method: &str,
    uri: &Uri,
    status: StatusCode,
    latency_ms: i64,
    error_message: Option<String>,
) -> LogEvent {
    LogEvent {
        group_id: audit.group_id,
        client_key_id: audit.client_key_id,
        request_id: Some(request_id),
        attempt_count: 0,
        usage_available: false,
        source: Some("gateway".to_string()),
        provider_id: None,
        account_id: None,
        endpoint: Some(format!("{} {}", method, uri.path())),
        status: gateway_log_status(status).to_string(),
        status_code: Some(status.as_u16() as i64),
        input_tokens: None,
        output_tokens: None,
        cache_tokens: None,
        cost: None,
        latency_ms: Some(latency_ms),
        ttft_ms: None,
        is_stream: None,
        error_message,
        model: audit.model.or_else(|| model_from_gateway_path(uri.path())),
    }
}

fn gateway_log_status(status: StatusCode) -> &'static str {
    if status.is_success() || status.is_redirection() {
        "success"
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        "rate_limited"
    } else if status == StatusCode::REQUEST_TIMEOUT || status == StatusCode::GATEWAY_TIMEOUT {
        "timeout"
    } else {
        "error"
    }
}

fn model_from_gateway_path(path: &str) -> Option<String> {
    ["/v1beta/models/", "/v1/models/"]
        .iter()
        .find_map(|prefix| path.strip_prefix(prefix))
        .and_then(|model_action| model_action.split(':').next())
        .filter(|model| !model.is_empty())
        .map(ToString::to_string)
}

/// Gateway authentication middleware.
///
/// Authentication identity hierarchy (highest first):
///   1. Gateway access key — admin identity. Sets `ClientKeyAuth{is_admin}`.
///   2. Virtual client key — the formal routing identity. Sets
///      `ClientKeyAuth{is_admin:false, pool_ids}` for route-pool resolution.
///
/// When no gateway access key is configured the gateway stays open for local
/// use (backward compatible), but a presented virtual client key is still
/// resolved and attached so bound-pool routing works.
async fn auth_middleware(
    State(state): State<Arc<ProxyState>>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    // No key configured → open gateway (localhost convenience). Read from the
    // shared lock on every request so settings changes apply without restart.
    let expected = match state.app_state.gateway_access_key.read() {
        Ok(key) => key.clone(),
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Gateway authentication state is unavailable",
            )
        }
    };
    let Some(expected) = expected else {
        // Open mode: attach virtual-key context if a valid client key was
        // presented, then pass through (no auth required, localhost-only).
        attach_client_key_if_present(&state, &mut request);
        return run_with_gateway_audit(request, next).await;
    };

    // Always allow the health probe unauthenticated so monitoring/UI can check
    // liveness without a key.
    if request.uri().path() == "/health" {
        return run_with_gateway_audit(request, next).await;
    }

    let headers = request.headers().clone();
    let presented = extract_access_key(&headers);

    // 1) Admin: gateway access key.
    if let Some(key) = &presented {
        if constant_time_eq(key.as_bytes(), expected.as_bytes()) {
            request
                .extensions_mut()
                .insert(ClientKeyAuth::admin(&state));
            return run_with_gateway_audit(request, next).await;
        }
    }

    // 2) Virtual client key: the formal routing identity.
    if attach_client_key_if_present(&state, &mut request) {
        return run_with_gateway_audit(request, next).await;
    }

    error_response(
        StatusCode::UNAUTHORIZED,
        "Missing or invalid gateway access key / virtual client key. Provide it via \
         'Authorization: Bearer <key>', 'x-api-key', or 'x-goog-api-key'.",
    )
}

async fn run_with_gateway_audit(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let auth = request.extensions().get::<ClientKeyAuth>().cloned();
    let path = request.uri().path().to_string();
    let is_post = request.method() == Method::POST;
    let explicit_group = auth::explicit_group_from_headers(request.headers());

    // Capture the requested model from the body for pool-scoped POST endpoints
    // (chat/responses/messages). The body is buffered here, before the handler
    // consumes it, then re-injected so downstream handlers still see it. This
    // lets gateway-level failures (auth errors, pool-empty, bad JSON) record
    // the model instead of leaving it blank.
    let model = if is_pool_scoped_path(&path) && is_post {
        let (parts, body) = request.into_parts();
        let bytes = match axum::body::to_bytes(body, 8 * 1024 * 1024).await {
            Ok(bytes) => bytes,
            Err(_) => axum::body::Bytes::new(),
        };
        let model = extract_model_for_gateway_audit(&path, &bytes);
        request = axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes));
        model
    } else {
        model_from_gateway_path(&path)
    };

    let mut response = next.run(request).await;
    let group_id = is_pool_scoped_path(&path)
        .then(|| auth::resolve_routing_group(explicit_group, auth.as_ref()).ok())
        .flatten();
    response.extensions_mut().insert(GatewayAuditContext {
        group_id,
        client_key_id: auth.map(|key| key.key_id),
        model,
    });
    response
}

/// Extract the requested model for a gateway-level audit log. Pool-scoped POST
/// endpoints carry the model in the JSON body; the Gemini family carries it in
/// the URL path (`/v1beta/models/{model}:action`).
fn extract_model_for_gateway_audit(path: &str, body: &[u8]) -> Option<String> {
    let from_body = match path {
        "/v1/chat/completions" | "/v1/responses" => openai::extract_model(body),
        "/v1/messages" => anthropic::extract_model(body),
        _ => None,
    };
    from_body.or_else(|| model_from_gateway_path(path))
}

fn is_pool_scoped_path(path: &str) -> bool {
    matches!(
        path,
        "/v1/messages" | "/v1/chat/completions" | "/v1/responses" | "/v1/models"
    ) || path.starts_with("/v1/models/")
        || path.starts_with("/v1beta/models/")
}

/// Try to resolve the presented key as a virtual client key. On success the
/// `ClientKeyAuth` context is attached to the request and `true` is returned.
fn attach_client_key_if_present(state: &ProxyState, request: &mut axum::extract::Request) -> bool {
    let presented = extract_access_key(request.headers());
    let Some(presented) = presented else {
        return false;
    };
    let db = &state.app_state.db;
    let key = match db.client_keys.authenticate(&db.conn, &presented) {
        Ok(Some(key)) => key,
        _ => return false,
    };
    let pool_ids = match db.client_keys.get_pool_ids(&db.conn, &key.id) {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("Failed to load pools for client key '{}': {}", key.id, e);
            Vec::new()
        }
    };
    // Audit / statistics hook: record last successful use.
    if let Err(e) = db.client_keys.touch(&db.conn, &key.id) {
        tracing::warn!("Failed to record client key usage '{}': {}", key.id, e);
    }
    let ctx = ClientKeyAuth {
        key_id: key.id,
        name: key.name,
        key_last_four: key.key_last_four,
        pool_ids,
        is_admin: false,
        rpm_limit: key.rpm_limit,
        tpm_limit: key.tpm_limit,
        allowed_protocols: key.allowed_protocols,
        allowed_models: key.allowed_models,
    };
    request.extensions_mut().insert(ctx);
    true
}

/// Extract a presented access key from the standard auth headers.
fn is_allowed_local_origin(origin: &HeaderValue) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    if matches!(
        origin,
        "tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost"
    ) {
        return true;
    }
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && matches!(
            url.host_str(),
            Some("localhost") | Some("127.0.0.1") | Some("::1") | Some("[::1]")
        )
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let a = left.get(index).copied().unwrap_or_default();
        let b = right.get(index).copied().unwrap_or_default();
        difference |= (a ^ b) as usize;
    }
    difference == 0
}

fn extract_access_key(headers: &HeaderMap) -> Option<String> {
    // Authorization: Bearer <key>
    if let Some(val) = headers.get("authorization") {
        if let Ok(s) = val.to_str() {
            let s = s.trim();
            if let Some(rest) = s
                .strip_prefix("Bearer ")
                .or_else(|| s.strip_prefix("bearer "))
            {
                let key = rest.trim();
                if !key.is_empty() {
                    return Some(key.to_string());
                }
            } else if !s.is_empty() {
                // Some clients send the raw key without the Bearer prefix.
                return Some(s.to_string());
            }
        }
    }

    // x-api-key (Anthropic) or x-goog-api-key (Gemini)
    for name in ["x-api-key", "x-goog-api-key"] {
        if let Some(val) = headers.get(name) {
            if let Ok(s) = val.to_str() {
                let key = s.trim();
                if !key.is_empty() {
                    return Some(key.to_string());
                }
            }
        }
    }

    None
}

/// Extract group_id from request headers (X-Group-Id or X-Pool-Group).
fn request_id_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty() && value.len() <= 128)
        .map(|value| value.to_string())
        .unwrap_or_else(|| format!("pg_{}", Uuid::new_v4().simple()))
}

/// Check if a request body asks for streaming.
fn usage_from_response_text(
    kind: responses::UpstreamKind,
    body: &str,
) -> crate::proxy::protocol::Usage {
    let bytes = body.as_bytes();
    match kind {
        responses::UpstreamKind::Anthropic => {
            let (input_tokens, output_tokens) = anthropic::extract_usage(bytes);
            crate::proxy::protocol::Usage {
                input_tokens,
                output_tokens,
                cache_tokens: 0,
                available: serde_json::from_str::<serde_json::Value>(body)
                    .ok()
                    .and_then(|value| value.get("usage").cloned())
                    .is_some(),
            }
        }
        responses::UpstreamKind::Gemini => {
            let (input_tokens, output_tokens) = gemini::extract_usage(bytes);
            crate::proxy::protocol::Usage {
                input_tokens,
                output_tokens,
                cache_tokens: 0,
                available: serde_json::from_str::<serde_json::Value>(body)
                    .ok()
                    .and_then(|value| value.get("usageMetadata").cloned())
                    .is_some(),
            }
        }
        responses::UpstreamKind::Responses | responses::UpstreamKind::Chat => {
            let value = serde_json::from_str::<serde_json::Value>(body).ok();
            let usage = value.as_ref().and_then(|value| value.get("usage"));
            crate::proxy::protocol::Usage {
                input_tokens: usage
                    .and_then(|value| {
                        value
                            .get("input_tokens")
                            .or_else(|| value.get("prompt_tokens"))
                    })
                    .and_then(|value| value.as_i64())
                    .unwrap_or_default(),
                output_tokens: usage
                    .and_then(|value| {
                        value
                            .get("output_tokens")
                            .or_else(|| value.get("completion_tokens"))
                    })
                    .and_then(|value| value.as_i64())
                    .unwrap_or_default(),
                cache_tokens: 0,
                available: usage.is_some(),
            }
        }
        responses::UpstreamKind::Unsupported => crate::proxy::protocol::Usage::default(),
    }
}

fn is_streaming_request(body: &[u8]) -> bool {
    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(stream) = val.get("stream") {
            return stream.as_bool().unwrap_or(false);
        }
    }
    false
}

/// Get the routing strategy for a given group from the database.
///
/// NOTE: `db.conn` is a non-reentrant `std::sync::Mutex`. The repository
/// methods (`groups.get_by_id`) acquire that lock internally, so we must NOT
/// hold the lock here as well — doing so deadlocks the request path on every
/// call. We simply delegate to the repository and map the result.
async fn get_group_strategy(state: &ProxyState, group_id: &str) -> RoutingStrategy {
    let db = &state.app_state.db;
    let group = match db.groups.get_by_id(&db.conn, group_id) {
        Ok(Some(g)) => g,
        _ => return RoutingStrategy::RoundRobin,
    };

    match group.strategy.as_deref() {
        Some(s) => RoutingStrategy::from_str(s),
        None => RoutingStrategy::RoundRobin,
    }
}

/// Build an error response with a JSON body and the given status code.
fn error_response(status: StatusCode, message: impl std::fmt::Display) -> Response {
    let message = message.to_string();
    let body = serde_json::json!({
        "error": {
            "message": message.clone(),
            "type": "proxy_error",
            "code": status.as_u16(),
        }
    });
    let mut response = Json(body).into_response();
    *response.status_mut() = status;
    response
        .extensions_mut()
        .insert(GatewayErrorDetail(message));
    response
}

#[cfg(test)]
mod tests {
    use super::{
        auth_middleware, constant_time_eq, error_response, gateway_log_event, gateway_log_status,
        is_allowed_local_origin, logging_middleware, model_from_gateway_path,
        request_id_from_headers, short_fingerprint, should_failover, should_write_gateway_log,
        GatewayAuditContext, ProxyState,
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode, Uri},
        response::Response,
        routing::{get, post},
        Router,
    };
    use std::{
        collections::HashSet,
        sync::{atomic::AtomicBool, Arc, Mutex, RwLock},
        time::Duration,
    };
    use tower::ServiceExt;

    fn audit_test_state(gateway_key: Option<&str>) -> (Arc<ProxyState>, std::path::PathBuf) {
        let db_path = std::env::temp_dir().join(format!(
            "poolgate-audit-{}.db",
            uuid::Uuid::new_v4().simple()
        ));
        let database = crate::db::Database::new(&db_path).expect("open audit database");
        database.run_migrations().expect("migrate audit database");
        let app_state = Arc::new(crate::AppState {
            db: database,
            gateway_access_key: RwLock::new(gateway_key.map(ToString::to_string)),
            gateway_access_key_loaded: AtomicBool::new(true),
            proxy: Mutex::new(None),
            gateway_runtime: Default::default(),
            account_concurrency: Default::default(),
            agent_app_operations: Mutex::new(HashSet::new()),
            app_data_dir: Some(std::env::temp_dir()),
        });
        let state = Arc::new(ProxyState {
            app_state: app_state.clone(),
            model_cache: Default::default(),
            circuit_breaker: Default::default(),
            account_concurrency: app_state.account_concurrency.clone(),
            log_writer: crate::proxy::logger::LogWriter::new_with_app_state(&app_state),
            startup_time: std::time::Instant::now(),
        });
        (state, db_path)
    }

    fn audit_test_router(state: Arc<ProxyState>) -> Router {
        async fn ok_handler() -> &'static str {
            "ok"
        }
        async fn invalid_handler() -> Response {
            error_response(StatusCode::BAD_REQUEST, "invalid JSON body")
        }

        Router::new()
            .route("/ok", get(ok_handler))
            .route("/invalid", post(invalid_handler))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                logging_middleware,
            ))
            .with_state(state)
    }

    async fn wait_for_audit_rows(state: &ProxyState, expected: i64) {
        for _ in 0..30 {
            let count: i64 = state
                .app_state
                .db
                .conn
                .lock()
                .expect("lock audit database")
                .query_row("SELECT COUNT(*) FROM request_logs", [], |row| row.get(0))
                .expect("count audit rows");
            if count >= expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("audit rows were not flushed in time");
    }

    #[tokio::test]
    async fn gateway_middleware_persists_success_and_failure_responses() {
        let (state, db_path) = audit_test_state(Some("admin-secret"));
        let app = audit_test_router(state.clone());
        let cases = [
            ("GET", "/ok", Some("Bearer admin-secret"), 200),
            ("GET", "/ok", None, 401),
            ("POST", "/invalid", Some("Bearer admin-secret"), 400),
            ("GET", "/missing", Some("Bearer admin-secret"), 404),
            ("POST", "/ok", Some("Bearer admin-secret"), 405),
        ];

        for (method, path, authorization, expected_status) in cases {
            let mut builder = Request::builder().method(method).uri(path);
            if let Some(value) = authorization {
                builder = builder.header("authorization", value);
            }
            let response = app
                .clone()
                .oneshot(builder.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), expected_status);
            assert!(response.headers().contains_key("x-request-id"));
        }

        wait_for_audit_rows(&state, 5).await;
        let rows = {
            let conn = state.app_state.db.conn.lock().expect("lock audit database");
            let mut stmt = conn
                .prepare(
                    "SELECT status_code, status, source, attempt_count, endpoint, error_message
                     FROM request_logs ORDER BY id",
                )
                .expect("prepare audit query");
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .expect("query audit rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("read audit rows")
        };
        assert_eq!(
            rows.iter().map(|row| row.0).collect::<Vec<_>>(),
            vec![200, 401, 400, 404, 405]
        );
        assert!(rows.iter().all(|row| row.2 == "gateway" && row.3 == 0));
        assert_eq!(rows[0].1, "success");
        assert!(rows[1..].iter().all(|row| row.1 == "error"));
        assert_eq!(rows[2].5.as_deref(), Some("invalid JSON body"));
        assert_eq!(rows[3].4, "GET /missing");

        drop(app);
        drop(state);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn route_fingerprints_are_stable_and_do_not_expose_ids() {
        let value = short_fingerprint("acct_123456789");
        assert_eq!(value.len(), 12);
        assert_eq!(value, short_fingerprint("acct_123456789"));
        assert!(!value.contains("acct"));
    }

    #[test]
    fn request_id_accepts_bounded_client_value() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-request-id", "client-trace-1".parse().unwrap());
        assert_eq!(request_id_from_headers(&headers), "client-trace-1");
    }

    #[test]
    fn gateway_audit_event_preserves_verified_context_and_route_shape() {
        let uri: Uri = "/v1beta/models/gemini-2.5-pro:generateContent?alt=sse"
            .parse()
            .unwrap();
        let event = gateway_log_event(
            GatewayAuditContext {
                group_id: Some("pool-1".into()),
                client_key_id: Some("key-1".into()),
                model: None,
            },
            "request-1".into(),
            "POST",
            &uri,
            StatusCode::SERVICE_UNAVAILABLE,
            14,
            Some("POOL_EMPTY".into()),
        );
        assert_eq!(event.group_id.as_deref(), Some("pool-1"));
        assert_eq!(event.client_key_id.as_deref(), Some("key-1"));
        assert_eq!(event.request_id.as_deref(), Some("request-1"));
        assert_eq!(event.attempt_count, 0);
        assert_eq!(event.source.as_deref(), Some("gateway"));
        assert_eq!(event.model.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(
            event.endpoint.as_deref(),
            Some("POST /v1beta/models/gemini-2.5-pro:generateContent")
        );
        assert_eq!(event.status, "error");
        assert_eq!(event.status_code, Some(503));
        assert_eq!(event.error_message.as_deref(), Some("POOL_EMPTY"));
    }

    #[test]
    fn routed_response_marker_prevents_duplicate_gateway_row() {
        let mut response = Response::new(axum::body::Body::empty());
        assert!(should_write_gateway_log(&response));
        response
            .extensions_mut()
            .insert(super::RouteAttemptRecorded);
        assert!(!should_write_gateway_log(&response));
    }

    #[test]
    fn gateway_status_categories_cover_rate_limit_and_timeout() {
        assert_eq!(gateway_log_status(StatusCode::OK), "success");
        assert_eq!(
            gateway_log_status(StatusCode::TOO_MANY_REQUESTS),
            "rate_limited"
        );
        assert_eq!(gateway_log_status(StatusCode::GATEWAY_TIMEOUT), "timeout");
        assert_eq!(gateway_log_status(StatusCode::NOT_FOUND), "error");
        assert_eq!(model_from_gateway_path("/health"), None);
    }

    #[test]
    fn local_cors_origins_are_allowed_but_remote_origins_are_rejected() {
        for origin in [
            "tauri://localhost",
            "http://localhost:1420",
            "http://127.0.0.1:3000",
            "https://[::1]:8443",
        ] {
            assert!(
                is_allowed_local_origin(&origin.parse().unwrap()),
                "{}",
                origin
            );
        }
        assert!(!is_allowed_local_origin(
            &"https://example.com".parse().unwrap()
        ));
    }

    #[test]
    fn access_key_comparison_requires_exact_equal_bytes() {
        assert!(constant_time_eq(b"poolgate-secret", b"poolgate-secret"));
        assert!(!constant_time_eq(b"poolgate-secret", b"poolgate-secreu"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }

    #[test]
    fn failover_only_for_account_or_upstream_failures() {
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert!(should_failover(status), "{} should fail over", status);
        }
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::NOT_FOUND,
            StatusCode::UNPROCESSABLE_ENTITY,
        ] {
            assert!(!should_failover(status), "{} should not fail over", status);
        }
    }
}
