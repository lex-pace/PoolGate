//! Google Antigravity OAuth login + reverse-proxy adapter.
//!
//! Reference implementation: Antigravity Tools (Antigravity-Manager) and
//! CLIProxyAPI (router-for-me). Google Antigravity (Cloud Code) accounts are
//! plain Google accounts (Google One AI Pro / Gemini subscription). After OAuth
//! the access token is used as a Bearer token against the Cloud Code private
//! v1internal API — NOT the public `generativelanguage.googleapis.com`.
//!
//! Upstream endpoints (production first, daily/sandbox as fallback):
//! - `https://cloudcode-pa.googleapis.com/v1internal`
//! - `https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal`
//!
//! Methods (POST, `{base}:method`):
//! - `generateContent` / `streamGenerateContent?alt=sse` — generation
//! - `fetchAvailableModels` — model catalog
//! - `loadCodeAssist` — resolve `cloudaicompanionProject` (project_id)
//!
//! Response wrapper: the upstream wraps every Gemini-shaped payload in
//! `{"response": {...}}`; streaming SSE lines carry `data: {"response": {...}}`.
//! Both must be unwrapped before the gateway can treat it as native Gemini.

use crate::db::accounts::{insert_account, Account};
use crate::db::providers::Provider;
use crate::services::credentials::CredentialPayload;
use crate::AppState;
use chrono::Utc;
use rand::distributions::Alphanumeric;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use url::Url;
use uuid::Uuid;

// ── OAuth configuration (Antigravity Tools web-app client) ─────────────────

/// Google OAuth authorization endpoint.
pub const GOOGLE_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";

/// Google OAuth token endpoint.
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// Google userinfo endpoint.
pub const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";

/// Antigravity OAuth client ID (public web-app client used by Antigravity Tools).
///
/// Loaded from the `ANTIGRAVITY_CLIENT_ID` environment variable at runtime to
/// keep credentials out of version control. Set it (and the secret) in the
/// environment used to launch the app.
pub static ANTIGRAVITY_CLIENT_ID: LazyLock<String> = LazyLock::new(|| {
    std::env::var("ANTIGRAVITY_CLIENT_ID").unwrap_or_default()
});

/// Antigravity OAuth client secret.
///
/// Loaded from the `ANTIGRAVITY_CLIENT_SECRET` environment variable at runtime.
pub static ANTIGRAVITY_CLIENT_SECRET: LazyLock<String> = LazyLock::new(|| {
    std::env::var("ANTIGRAVITY_CLIENT_SECRET").unwrap_or_default()
});

/// OAuth scopes required for Antigravity (Cloud Code) API access.
pub const ANTIGRAVITY_SCOPES: &str = concat!(
    "https://www.googleapis.com/auth/cloud-platform ",
    "https://www.googleapis.com/auth/userinfo.email ",
    "https://www.googleapis.com/auth/userinfo.profile ",
    "https://www.googleapis.com/auth/cclog ",
    "https://www.googleapis.com/auth/experimentsandconfigs"
);

/// Local callback port (must not collide with other OAuth flows).
pub const ANTIGRAVITY_REDIRECT_PORT: u16 = 8088;

fn antigravity_redirect_uri() -> String {
    format!(
        "http://localhost:{}/callback",
        ANTIGRAVITY_REDIRECT_PORT
    )
}

// ── Upstream API configuration ──────────────────────────────────────────────

/// Production Cloud Code v1internal base URL.
pub const ANTIGRAVITY_BASE_URL: &str = "https://cloudcode-pa.googleapis.com";

/// Daily / sandbox fallback base URL.
pub const ANTIGRAVITY_DAILY_BASE_URL: &str =
    "https://daily-cloudcode-pa.sandbox.googleapis.com";

/// Upstream UA fingerprint (matches the reference Antigravity client).
pub const ANTIGRAVITY_USER_AGENT: &str = "antigravity/1.11.9 windows/amd64";

const V1_INTERNAL_PATH: &str = "/v1internal";

/// Response field of `loadCodeAssist` carrying the project id.
const PROJECT_ID_FIELD: &str = "cloudaicompanionProject";

// ── Callback listener management ────────────────────────────────────────────

struct PendingFlow {
    state: String,
    callback_rx: oneshot::Receiver<String>,
    callback_task: tokio::task::JoinHandle<()>,
}

static PENDING_FLOWS: LazyLock<Mutex<HashMap<String, PendingFlow>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// ── Public API ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct AntigravityOAuthStartResult {
    pub login_id: String,
    pub authorization_url: String,
    pub redirect_uri: String,
    pub expires_in_seconds: u64,
}

/// Start an Antigravity OAuth authorization-code flow (client-secret mode,
/// no PKCE — matches the reference Antigravity Tools client).
pub async fn start_antigravity_oauth() -> Result<AntigravityOAuthStartResult, String> {
    let login_id = format!("antigravity_oauth_{}", Uuid::new_v4().simple());
    let state = random_string(32);

    let redirect_uri = antigravity_redirect_uri();
    let mut url = Url::parse(GOOGLE_AUTHORIZE_URL).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_id", ANTIGRAVITY_CLIENT_ID.as_str())
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", ANTIGRAVITY_SCOPES)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("include_granted_scopes", "true")
        .append_pair("state", &state);

    let listener = TcpListener::bind(format!("127.0.0.1:{}", ANTIGRAVITY_REDIRECT_PORT))
        .await
        .map_err(|e| {
            format!(
                "无法监听 Antigravity OAuth 回调端口 {}: {}",
                ANTIGRAVITY_REDIRECT_PORT, e
            )
        })?;

    let (callback_tx, callback_rx) = oneshot::channel();
    let callback_task = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buffer = vec![0_u8; 8192];
            if let Ok(size) = stream.read(&mut buffer).await {
                let request = String::from_utf8_lossy(&buffer[..size]);
                if let Some(path) = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                {
                    let callback = format!("http://localhost:{}{}", ANTIGRAVITY_REDIRECT_PORT, path);
                    let _ = callback_tx.send(callback);
                }
            }
            let html = "<!doctype html><html><head><meta charset=\"utf-8\"><title>PoolGate - Antigravity 授权完成</title></head><body><h2>Antigravity 授权已返回 PoolGate</h2><p>可以关闭此窗口，回到 PoolGate 完成账号接入。</p></body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });

    PENDING_FLOWS.lock().map_err(|e| e.to_string())?.insert(
        login_id.clone(),
        PendingFlow {
            state,
            callback_rx,
            callback_task,
        },
    );

    Ok(AntigravityOAuthStartResult {
        login_id,
        authorization_url: url.to_string(),
        redirect_uri,
        expires_in_seconds: 300,
    })
}

/// Complete an Antigravity OAuth login: exchange the code, resolve the Cloud
/// Code project id, and persist the account into a routable pool.
pub async fn complete_antigravity_oauth(
    state: Arc<AppState>,
    login_id: &str,
    callback_url: Option<String>,
) -> Result<Account, String> {
    let flow = PENDING_FLOWS
        .lock()
        .map_err(|e| e.to_string())?
        .remove(login_id)
        .ok_or_else(|| "Antigravity OAuth 会话不存在或已过期，请重新发起授权".to_string())?;

    let PendingFlow {
        state: oauth_state,
        callback_rx,
        callback_task,
    } = flow;

    let callback = match crate::services::oauth::clean_optional(callback_url) {
        Some(value) => {
            callback_task.abort();
            value
        }
        None => tokio::time::timeout(std::time::Duration::from_secs(300), callback_rx)
            .await
            .map_err(|_| {
                callback_task.abort();
                "等待 Antigravity OAuth 回调超时，请重新授权或粘贴回调地址".to_string()
            })?
            .map_err(|_| "Antigravity OAuth 回调监听已关闭".to_string())?,
    };

    let callback = Url::parse(&callback).map_err(|e| format!("无效的回调地址: {}", e))?;
    let query: HashMap<String, String> = callback.query_pairs().into_owned().collect();

    if let Some(error) = query.get("error") {
        return Err(format!(
            "Antigravity OAuth 授权失败: {}",
            query.get("error_description").unwrap_or(error)
        ));
    }
    if query.get("state") != Some(&oauth_state) {
        return Err("Antigravity OAuth state 校验失败，请重新授权".into());
    }
    let code = query
        .get("code")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "回调地址中缺少 authorization code".to_string())?;

    let redirect_uri = antigravity_redirect_uri();
    let token = exchange_code(code, &redirect_uri).await?;
    let access_token = token.access_token.clone();

    // Resolve the Cloud Code project id (may fall back to a generated id).
    let project_id = fetch_project_id(&access_token).await.unwrap_or_else(|e| {
        tracing::warn!("Antigravity loadCodeAssist failed, using mock project id: {}", e);
        generate_mock_project_id()
    });

    persist_antigravity_account(&state, token, project_id)
}

/// Cancel a pending Antigravity OAuth flow.
pub fn cancel_antigravity_oauth(login_id: &str) -> Result<(), String> {
    if let Some(flow) = PENDING_FLOWS
        .lock()
        .map_err(|e| e.to_string())?
        .remove(login_id)
    {
        flow.callback_task.abort();
    }
    Ok(())
}

// ── Token exchange / refresh ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: i64,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

async fn exchange_code(code: &str, redirect_uri: &str) -> Result<TokenResponse, String> {
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .post(GOOGLE_TOKEN_URL)
        .form(&[
            ("client_id", ANTIGRAVITY_CLIENT_ID.as_str()),
            ("client_secret", ANTIGRAVITY_CLIENT_SECRET.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| format!("Antigravity Token 交换请求失败: {}", e))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("Antigravity Token 交换失败 ({}): {}", status, body));
    }
    serde_json::from_str(&body).map_err(|e| format!("Antigravity Token 响应解析失败: {}", e))
}

/// Refresh an Antigravity access token using the stored refresh token.
pub async fn refresh_access_token(refresh_token: &str) -> Result<TokenResponse, String> {
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .post(GOOGLE_TOKEN_URL)
        .form(&[
            ("client_id", ANTIGRAVITY_CLIENT_ID.as_str()),
            ("client_secret", ANTIGRAVITY_CLIENT_SECRET.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| format!("Antigravity 刷新请求失败: {}", e))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("Antigravity 刷新失败 ({}): {}", status, body));
    }
    serde_json::from_str(&body).map_err(|e| format!("Antigravity 刷新响应解析失败: {}", e))
}

// ── Cloud Code v1internal helpers ───────────────────────────────────────────

fn build_v1_internal_url(base: &str, method: &str, query: Option<&str>) -> String {
    let url = format!("{}{}:{}", base, V1_INTERNAL_PATH, method);
    match query {
        Some(q) => format!("{}?{}", url, q),
        None => url,
    }
}

fn v1_internal_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Resolve the Cloud Code project id via `loadCodeAssist`.
pub async fn fetch_project_id(access_token: &str) -> Result<String, String> {
    let body = serde_json::json!({ "metadata": { "ideType": "ANTIGRAVITY" } });
    for base in [ANTIGRAVITY_BASE_URL, ANTIGRAVITY_DAILY_BASE_URL] {
        let url = build_v1_internal_url(base, "loadCodeAssist", None);
        let response = v1_internal_client()
            .post(&url)
            .bearer_auth(access_token)
            .header("Host", base.trim_start_matches("https://"))
            .header("User-Agent", ANTIGRAVITY_USER_AGENT)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("loadCodeAssist 请求失败: {}", e))?;

        if response.status().is_success() {
            let json: serde_json::Value = response
                .json()
                .await
                .map_err(|e| format!("loadCodeAssist 解析失败: {}", e))?;
            if let Some(pid) = json.get(PROJECT_ID_FIELD).and_then(|v| v.as_str()) {
                if !pid.trim().is_empty() {
                    return Ok(pid.to_string());
                }
            }
        }
    }
    // Fall back to a generated mock project id (reference behavior).
    Ok(generate_mock_project_id())
}

/// Fetch the available model catalog from `fetchAvailableModels`.
pub async fn fetch_models(
    access_token: &str,
    project_id: Option<&str>,
) -> Result<Vec<serde_json::Value>, String> {
    let payload = match project_id {
        Some(pid) if !pid.trim().is_empty() => {
            serde_json::json!({ "project": pid })
        }
        _ => serde_json::json!({}),
    };

    for base in [ANTIGRAVITY_BASE_URL, ANTIGRAVITY_DAILY_BASE_URL] {
        let url = build_v1_internal_url(base, "fetchAvailableModels", None);
        let response = v1_internal_client()
            .post(&url)
            .bearer_auth(access_token)
            .header("User-Agent", ANTIGRAVITY_USER_AGENT)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("fetchAvailableModels 请求失败: {}", e))?;

        if !response.status().is_success() {
            continue;
        }
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("fetchAvailableModels 解析失败: {}", e))?;
        let models = json
            .get("models")
            .and_then(|v| v.as_object())
            .map(|map| {
                map.iter()
                    .filter_map(|(name, meta)| {
                        let display = meta
                            .get("displayName")
                            .and_then(|v| v.as_str())
                            .unwrap_or(name);
                        Some(serde_json::json!({
                            "id": name,
                            "name": name,
                            "display_name": display,
                            "owned_by": "antigravity"
                        }))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        return Ok(models);
    }
    Err("fetchAvailableModels 在所有上游端点均失败".into())
}

/// Fetch quota info (per-model remaining fraction + reset time) and the
/// subscription tier.
///
/// Reference: Antigravity Tools `modules/quota.rs` — quota data lives in the
/// same `fetchAvailableModels` response under `models[<id>].quotaInfo`
/// (`remainingFraction`, `resetTime`); the tier comes from `loadCodeAssist`
/// (`paidTier.id`, falling back to `currentTier.id`).
pub async fn fetch_quota(
    access_token: &str,
    project_id: Option<&str>,
) -> Result<(Option<String>, Vec<QuotaEntry>), String> {
    let project = project_id
        .filter(|pid| !pid.trim().is_empty())
        .map(|pid| pid.to_string())
        // Reference Antigravity Tools falls back to a fixed project id.
        .unwrap_or_else(|| "bamboo-precept-lgxtn".to_string());

    let tier = fetch_subscription_tier(access_token).await;

    let payload = serde_json::json!({ "project": project });
    let mut entries: Vec<QuotaEntry> = Vec::new();

    for base in [ANTIGRAVITY_BASE_URL, ANTIGRAVITY_DAILY_BASE_URL] {
        let url = build_v1_internal_url(base, "fetchAvailableModels", None);
        let response = v1_internal_client()
            .post(&url)
            .bearer_auth(access_token)
            .header("User-Agent", ANTIGRAVITY_USER_AGENT)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("fetchAvailableModels 请求失败: {}", e))?;

        if !response.status().is_success() {
            continue;
        }
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("fetchAvailableModels 解析失败: {}", e))?;
        if let Some(models) = json.get("models").and_then(|v| v.as_object()) {
            for (name, meta) in models {
                let Some(quota) = meta.get("quotaInfo") else {
                    continue;
                };
                let remaining = quota
                    .get("remainingFraction")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0);
                let reset = quota
                    .get("resetTime")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                entries.push(QuotaEntry {
                    model: name.clone(),
                    remaining_fraction: remaining,
                    reset_time: reset,
                });
            }
        }
        if !entries.is_empty() {
            return Ok((tier, entries));
        }
    }
    Ok((tier, entries))
}

/// A single model quota window returned by `fetchAvailableModels`.
#[derive(Debug, Clone)]
pub struct QuotaEntry {
    pub model: String,
    pub remaining_fraction: f64,
    pub reset_time: Option<String>,
}

/// Resolve the subscription tier from `loadCodeAssist` (`paidTier.id` preferred).
async fn fetch_subscription_tier(access_token: &str) -> Option<String> {
    let body = serde_json::json!({ "metadata": { "ideType": "ANTIGRAVITY" } });
    for base in [ANTIGRAVITY_BASE_URL, ANTIGRAVITY_DAILY_BASE_URL] {
        let url = build_v1_internal_url(base, "loadCodeAssist", None);
        let Ok(response) = v1_internal_client()
            .post(&url)
            .bearer_auth(access_token)
            .header("Host", base.trim_start_matches("https://"))
            .header("User-Agent", ANTIGRAVITY_USER_AGENT)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
        else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(json) = response.json::<serde_json::Value>().await else {
            continue;
        };
        let tier = json
            .get("paidTier")
            .and_then(|t| t.get("id"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                json.get("currentTier")
                    .and_then(|t| t.get("id"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });
        if tier.is_some() {
            return tier;
        }
    }
    None
}

/// Generate a mock project id when the account has no entitlement.
pub fn generate_mock_project_id() -> String {
    let adjectives = ["useful", "bright", "swift", "calm", "bold"];
    let nouns = ["fuze", "wave", "spark", "flow", "core"];
    let mut rng = rand::thread_rng();
    let adj = adjectives[rng.gen_range(0..adjectives.len())];
    let noun = nouns[rng.gen_range(0..nouns.len())];
    let random_num: String = (0..5)
        .map(|_| {
            let chars = "abcdefghijklmnopqrstuvwxyz0123456789";
            let idx = rng.gen_range(0..chars.len());
            chars.chars().nth(idx).unwrap()
        })
        .collect();
    format!("{}-{}-{}", adj, noun, random_num)
}

// ── Account identification & health ─────────────────────────────────────────

/// True when the account belongs to a Google Antigravity (Cloud Code) provider.
pub fn is_antigravity_account(account: &Account, provider: &Provider) -> bool {
    provider.provider_type.eq_ignore_ascii_case("antigravity")
        || provider.base_url.contains("cloudcode-pa.googleapis.com")
        || account
            .tags
            .as_deref()
            .map(|tags| tags.contains("antigravity"))
            .unwrap_or(false)
}

/// Lightweight health check: fetch the available models from the Cloud Code
/// v1internal API using the stored OAuth access token.
pub async fn check_antigravity_health(
    account: &Account,
) -> crate::services::health_check::HealthResult {
    use crate::services::health_check::HealthResult;
    use std::time::Instant;

    let payload = match crate::services::credentials::payload_for_account(account) {
        Ok(payload) => payload,
        Err(error) => return HealthResult::Error(error),
    };
    let access_token = match payload.access_token.filter(|t| !t.trim().is_empty()) {
        Some(token) => token,
        None => {
            return HealthResult::Error(
                "Antigravity account has no access token".to_string(),
            )
        }
    };

    let start = Instant::now();
    let project_id = payload
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("project_id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    match fetch_models(&access_token, project_id.as_deref()).await {
        Ok(models) if !models.is_empty() => HealthResult::Passed {
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Ok(_) => HealthResult::Failed {
            code: 404,
            body: "fetchAvailableModels returned no models".into(),
        },
        Err(error) => HealthResult::Error(error),
    }
}

// ── Account persistence ─────────────────────────────────────────────────────

fn persist_antigravity_account(
    state: &Arc<AppState>,
    token: TokenResponse,
    project_id: String,
) -> Result<Account, String> {
    let expires_at = (Utc::now() + chrono::Duration::seconds(token.expires_in)).to_rfc3339();

    let metadata = serde_json::json!({
        "client_id": ANTIGRAVITY_CLIENT_ID.as_str(),
        "client_secret": ANTIGRAVITY_CLIENT_SECRET.as_str(),
        "token_endpoint": GOOGLE_TOKEN_URL,
        "scope": ANTIGRAVITY_SCOPES,
        "project_id": project_id,
    });

    let payload = CredentialPayload {
        access_token: Some(token.access_token.clone()),
        refresh_token: token.refresh_token,
        id_token: None,
        account_id: None,
        expires_at: Some(expires_at.clone()),
        token_type: Some(token.token_type).filter(|t| !t.is_empty()).or_else(|| Some("Bearer".into())),
        base_url: Some(ANTIGRAVITY_BASE_URL.into()),
        metadata: Some(metadata.clone()),
        ..CredentialPayload::default()
    };

    let fingerprint = hex::encode(Sha256::digest(
        format!("Antigravity:{}", token.access_token).as_bytes(),
    ));

    if let Some(existing) = state
        .db
        .accounts
        .find_by_fingerprint(&state.db.conn, &fingerprint)?
    {
        return Ok(existing);
    }

    let provider_id = "provider_google_antigravity".to_string();
    if state
        .db
        .providers
        .get_by_id(&state.db.conn, &provider_id)?
        .is_none()
    {
        state.db.providers.create(
            &state.db.conn,
            &Provider {
                id: provider_id.clone(),
                name: "Google Antigravity".into(),
                provider_type: "antigravity".into(),
                base_url: ANTIGRAVITY_BASE_URL.into(),
                base_urls: Some(format!(
                    "{{\"gemini\":\"{}\",\"antigravity\":\"{}\"}}",
                    ANTIGRAVITY_BASE_URL, ANTIGRAVITY_BASE_URL
                )),
                protocol: "gemini".into(),
                protocols: Some("[\"gemini\"]".into()),
                route_takeover: Some(1),
                api_keys: None,
                models: Some("[\"gemini-2.5-flash\",\"gemini-3-flash\",\"gemini-3-pro-low\",\"gemini-3-pro-high\"]".into()),
                proxy_url: None,
                custom_headers: None,
                timeout_ms: Some(30000),
                priority: Some(0),
                enabled: Some(true),
                created_at: None,
                auth_mode: Some("oauth_pkce".into()),
                oauth_config: Some(serde_json::json!({
                    "authorize_url": GOOGLE_AUTHORIZE_URL,
                    "token_url": GOOGLE_TOKEN_URL,
                    "client_id": ANTIGRAVITY_CLIENT_ID.as_str(),
                    "client_secret": ANTIGRAVITY_CLIENT_SECRET.as_str(),
                    "scope": ANTIGRAVITY_SCOPES,
                    "redirect_port": ANTIGRAVITY_REDIRECT_PORT
                }).to_string()),
            },
        )?;
    }

    let models_json = "[\"gemini-2.5-flash\",\"gemini-3-flash\",\"gemini-3-pro-low\",\"gemini-3-pro-high\"]".to_string();
    let account = Account {
        id: format!("acct_{}", Uuid::new_v4().simple()),
        provider_id: Some(provider_id),
        name: Some("Google Antigravity account".into()),
        api_key: String::new(),
        models: Some(models_json),
        quota_limit: None,
        quota_used: None,
        status: Some("active".into()),
        health_status: Some("unchecked".into()),
        health_code: None,
        health_msg: Some("Google Antigravity OAuth 已启用；仅限本机本人账号私有使用".into()),
        health_latency: None,
        health_check_at: None,
        priority: Some(0),
        tags: Some("[\"official\",\"google\",\"antigravity\",\"oauth\"]".into()),
        last_used_at: None,
        created_at: None,
        credential_type: Some("gemini_oauth".into()),
        credential_data: Some(serde_json::to_string(&payload).map_err(|e| e.to_string())?),
        source_format: Some("oauth".into()),
        external_account_id: None,
        email: None,
        expires_at: Some(expires_at),
        metadata: Some(metadata.to_string()),
        credential_fingerprint: Some(fingerprint),
        protocols: Some("[\"gemini\"]".into()),
        route_takeover: Some(1),
        plan_type: None,
        quota_windows: None,
        quota_refreshed_at: None,
        quota_error: None,
        token_refreshed_at: None,
        secret_ref: None,
    };

    {
        let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
        insert_account(&conn, &account)?;
    }

    // Auto-add to routing pool.
    let provider = state
        .db
        .providers
        .get_by_id(&state.db.conn, account.provider_id.as_deref().unwrap_or(""))?
        .ok_or_else(|| "Provider not found after creation".to_string())?;
    let models: Vec<String> = provider
        .models
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    crate::services::pool_onboarding::ensure_account_in_pool(
        state,
        &account.id,
        &provider.id,
        &models,
        "Antigravity",
        "unified",
    )?;

    Ok(account)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn random_string(length: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_internal_url_building() {
        assert_eq!(
            build_v1_internal_url(ANTIGRAVITY_BASE_URL, "generateContent", None),
            "https://cloudcode-pa.googleapis.com/v1internal:generateContent"
        );
        assert_eq!(
            build_v1_internal_url(ANTIGRAVITY_BASE_URL, "streamGenerateContent", Some("alt=sse")),
            "https://cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn mock_project_id_shape() {
        let pid = generate_mock_project_id();
        assert!(pid.matches('-').count() == 2, "unexpected shape: {}", pid);
    }

    #[test]
    fn oauth_constants_are_set() {
        assert!(ANTIGRAVITY_SCOPES.contains("cloud-platform"));
        assert!(ANTIGRAVITY_SCOPES.contains("cclog"));
    }
}
