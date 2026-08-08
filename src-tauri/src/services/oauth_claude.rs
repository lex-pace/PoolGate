//! Claude Code OAuth PKCE login flow.
//!
//! Implements the Anthropic OAuth 2.0 Authorization Code + PKCE flow used by
//! Claude Code CLI. Supports both Pro and Max plans with different authorize
//! endpoints.
//!
//! Flow:
//! 1. Generate PKCE code_verifier and code_challenge (S256).
//! 2. Build authorize URL with client_id, scope, redirect_uri, code_challenge.
//! 3. Open browser for user authorization.
//! 4. Listen on localhost:54545 for callback.
//! 5. Exchange authorization_code for access_token + refresh_token.
//! 6. Persist account with credential_type "claude_oauth".

use crate::db::accounts::{insert_account, Account};
use crate::db::providers::Provider;
use crate::services::claude_adapter::CLAUDE_CODE_SYSTEM_PREFIX;
use crate::services::credentials::CredentialPayload;
use crate::AppState;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use url::Url;
use uuid::Uuid;

// ── OAuth Configuration ─────────────────────────────────────────────────────

/// Claude Code OAuth client ID (public, no secret required).
const CLAUDE_CLIENT_ID: &str = "public-claude-code";

/// OAuth scope for Claude Code subscriptions.
const CLAUDE_SCOPE: &str = "org:create_api_key user:profile user:inference";

/// Redirect URI for local callback (port 54545).
const CLAUDE_REDIRECT_URI: &str = "http://localhost:54545/callback";

/// OAuth authorization endpoint for Claude Pro plan.
const CLAUDE_AUTHORIZE_URL_PRO: &str = "https://console.anthropic.com/oauth/authorize";

/// OAuth authorization endpoint for Claude Max plan.
const CLAUDE_AUTHORIZE_URL_MAX: &str = "https://claude.ai/oauth/authorize";

/// Token endpoint for exchanging codes and refreshing tokens.
const CLAUDE_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";

/// Callback HTML shown after successful authorization.
const CALLBACK_HTML: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>PoolGate - Claude 授权完成</title>
<style>body{font:14px -apple-system,BlinkMacSystemFont,sans-serif;margin:48px;color:#1d1d1f}
main{max-width:520px;margin:auto;padding:24px;border:1px solid #d2d2d7;border-radius:14px}
h2{color:#10a37f}</style></head>
<body><main>
<h2>Claude 授权已返回 PoolGate</h2>
<p>可以关闭此窗口，并回到 PoolGate 完成账号接入。</p>
</main></body></html>"#;

// ── Pending flow management ─────────────────────────────────────────────────

struct PendingFlow {
    state: String,
    code_verifier: String,
    plan_type: String,
    callback_rx: oneshot::Receiver<String>,
    callback_task: tokio::task::JoinHandle<()>,
}

static PENDING_FLOWS: LazyLock<Mutex<HashMap<String, PendingFlow>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// ── Public API ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct ClaudeOAuthStartResult {
    pub login_id: String,
    pub authorization_url: String,
    pub redirect_uri: String,
    pub expires_in_seconds: u64,
}

/// Start a Claude Code OAuth PKCE login flow.
///
/// `plan_type` must be "pro" or "max" to select the correct authorize endpoint.
pub async fn start_claude_oauth(
    plan_type: String,
) -> Result<ClaudeOAuthStartResult, String> {
    let plan = plan_type.trim().to_lowercase();
    let authorize_url = match plan.as_str() {
        "pro" => CLAUDE_AUTHORIZE_URL_PRO,
        "max" => CLAUDE_AUTHORIZE_URL_MAX,
        _ => return Err(format!("不支持的 Claude 计划类型 '{}'，请选择 'pro' 或 'max'", plan_type)),
    };

    let login_id = format!("claude_oauth_{}", Uuid::new_v4().simple());
    let state = random_string(32);
    let code_verifier = random_string(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));

    let mut url = Url::parse(authorize_url).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_id", CLAUDE_CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", CLAUDE_REDIRECT_URI)
        .append_pair("scope", CLAUDE_SCOPE)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state);

    // Start listening for callback.
    let listener = TcpListener::bind("127.0.0.1:54545")
        .await
        .map_err(|e| format!("无法监听 Claude OAuth 回调端口 54545: {}", e))?;

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
                    let callback = format!("http://localhost:54545{}", path);
                    let _ = callback_tx.send(callback);
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                CALLBACK_HTML.len(),
                CALLBACK_HTML
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });

    PENDING_FLOWS.lock().map_err(|e| e.to_string())?.insert(
        login_id.clone(),
        PendingFlow {
            state,
            code_verifier,
            plan_type: plan.clone(),
            callback_rx,
            callback_task,
        },
    );

    Ok(ClaudeOAuthStartResult {
        login_id,
        authorization_url: url.to_string(),
        redirect_uri: CLAUDE_REDIRECT_URI.into(),
        expires_in_seconds: 300,
    })
}

/// Complete a Claude Code OAuth login by exchanging the authorization code for tokens.
pub async fn complete_claude_oauth(
    state: Arc<AppState>,
    login_id: &str,
    callback_url: Option<String>,
) -> Result<Account, String> {
    let flow = PENDING_FLOWS
        .lock()
        .map_err(|e| e.to_string())?
        .remove(login_id)
        .ok_or_else(|| "Claude OAuth 会话不存在或已过期，请重新发起授权".to_string())?;

    let PendingFlow {
        state: oauth_state,
        code_verifier,
        plan_type,
        callback_rx,
        callback_task,
    } = flow;

    // Wait for callback or use provided URL.
    let callback = match super::oauth::clean_optional(callback_url) {
        Some(value) => {
            callback_task.abort();
            value
        }
        None => tokio::time::timeout(std::time::Duration::from_secs(300), callback_rx)
            .await
            .map_err(|_| {
                callback_task.abort();
                "等待 Claude OAuth 回调超时，请重新授权或粘贴回调地址".to_string()
            })?
            .map_err(|_| "Claude OAuth 回调监听已关闭".to_string())?,
    };

    let callback = Url::parse(&callback).map_err(|e| format!("无效的回调地址: {}", e))?;
    let query: HashMap<String, String> = callback.query_pairs().into_owned().collect();

    if let Some(error) = query.get("error") {
        return Err(format!(
            "Claude OAuth 授权失败: {}",
            query.get("error_description").unwrap_or(error)
        ));
    }

    if query.get("state") != Some(&oauth_state) {
        return Err("Claude OAuth state 校验失败，请重新授权".into());
    }

    let code = query
        .get("code")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "回调地址中缺少 authorization code".to_string())?;

    let token = exchange_claude_code(code, &code_verifier).await?;
    persist_claude_account(&state, token, &plan_type)
}

/// Cancel a pending Claude OAuth flow.
pub fn cancel_claude_oauth(login_id: &str) -> Result<(), String> {
    if let Some(flow) = PENDING_FLOWS
        .lock()
        .map_err(|e| e.to_string())?
        .remove(login_id)
    {
        flow.callback_task.abort();
    }
    Ok(())
}

// ── Token exchange ──────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
struct ClaudeTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    token_type: Option<String>,
    scope: Option<String>,
}

async fn exchange_claude_code(code: &str, verifier: &str) -> Result<ClaudeTokenResponse, String> {
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .post(CLAUDE_TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLAUDE_CLIENT_ID),
            ("code", code),
            ("redirect_uri", CLAUDE_REDIRECT_URI),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|e| format!("Claude Token 交换请求失败: {}", e))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!("Claude Token 交换失败 ({}): {}", status, body));
    }

    serde_json::from_str(&body).map_err(|e| format!("Claude Token 响应解析失败: {}", e))
}

// ── Account persistence ─────────────────────────────────────────────────────

fn persist_claude_account(
    state: &Arc<AppState>,
    token: ClaudeTokenResponse,
    plan_type: &str,
) -> Result<Account, String> {
    let expires_at = token
        .expires_in
        .map(|seconds| (Utc::now() + chrono::Duration::seconds(seconds)).to_rfc3339());

    let metadata = serde_json::json!({
        "client_id": CLAUDE_CLIENT_ID,
        "scope": CLAUDE_SCOPE,
        "token_endpoint": CLAUDE_TOKEN_URL,
        "plan_type": plan_type,
        "system_prefix": CLAUDE_CODE_SYSTEM_PREFIX,
    });

    let payload = CredentialPayload {
        access_token: Some(token.access_token.clone()),
        refresh_token: token.refresh_token,
        id_token: None,
        account_id: None, // Claude does not provide a stable account ID in token response.
        expires_at: expires_at.clone(),
        token_type: token.token_type.or_else(|| Some("Bearer".into())),
        base_url: Some("https://api.anthropic.com".into()),
        metadata: Some(metadata.clone()),
        ..CredentialPayload::default()
    };

    let fingerprint = hex::encode(Sha256::digest(
        format!("Claude:{}", token.access_token).as_bytes(),
    ));

    // Check for duplicate.
    if let Some(existing) = state
        .db
        .accounts
        .find_by_fingerprint(&state.db.conn, &fingerprint)?
    {
        return Ok(existing);
    }

    // Create or reuse provider.
    let provider_id = format!("provider_claude_{}", plan_type);
    if state
        .db
        .providers
        .get_by_id(&state.db.conn, &provider_id)?
        .is_none()
    {
        let models = match plan_type {
            "max" => "[\"claude-opus-4-20250514\",\"claude-sonnet-4-20250514\",\"claude-3-haiku-20240307\"]",
            _ => "[\"claude-sonnet-4-20250514\",\"claude-3-haiku-20240307\"]",
        };
        state.db.providers.create(
            &state.db.conn,
            &Provider {
                id: provider_id.clone(),
                name: format!("Claude {}", if plan_type == "max" { "Max" } else { "Pro" }),
                provider_type: "anthropic".into(),
                base_url: "https://api.anthropic.com".into(),
                base_urls: Some("{\"messages\":\"https://api.anthropic.com\"}".into()),
                protocol: "anthropic".into(),
                protocols: Some("[\"messages\"]".into()),
                route_takeover: Some(1),
                api_keys: None,
                models: Some(models.into()),
                proxy_url: None,
                custom_headers: None,
                timeout_ms: Some(30000),
                priority: Some(0),
                enabled: Some(true),
                created_at: None,
                auth_mode: Some("oauth_pkce".into()),
                oauth_config: Some(serde_json::json!({
                    "authorize_url": if plan_type == "max" { CLAUDE_AUTHORIZE_URL_MAX } else { CLAUDE_AUTHORIZE_URL_PRO },
                    "token_url": CLAUDE_TOKEN_URL,
                    "client_id": CLAUDE_CLIENT_ID,
                    "scope": CLAUDE_SCOPE,
                    "redirect_port": 54545
                }).to_string()),
            },
        )?;
    }

    let models = match plan_type {
        "max" => "[\"claude-opus-4-20250514\",\"claude-sonnet-4-20250514\",\"claude-3-haiku-20240307\"]",
        _ => "[\"claude-sonnet-4-20250514\",\"claude-3-haiku-20240307\"]",
    };

    let account = Account {
        id: format!("acct_{}", Uuid::new_v4().simple()),
        provider_id: Some(provider_id),
        name: Some(format!("Claude {} account", if plan_type == "max" { "Max" } else { "Pro" })),
        api_key: token.access_token,
        models: Some(models.into()),
        quota_limit: None,
        quota_used: None,
        status: Some("active".into()),
        health_status: Some("unchecked".into()),
        health_code: None,
        health_msg: Some("Claude 订阅 OAuth 已启用；仅限本机本人账号私有使用".into()),
        health_latency: None,
        health_check_at: None,
        priority: Some(0),
        tags: Some("[\"official\",\"anthropic\",\"claude\",\"oauth\"]".into()),
        last_used_at: None,
        created_at: None,
        credential_type: Some("claude_oauth".into()),
        credential_data: Some(serde_json::to_string(&payload).map_err(|e| e.to_string())?),
        source_format: Some("oauth".into()),
        external_account_id: None,
        email: None,
        expires_at,
        metadata: Some(metadata.to_string()),
        credential_fingerprint: Some(fingerprint),
        protocols: Some("[\"messages\"]".into()),
        route_takeover: Some(1),
        plan_type: Some(plan_type.to_string()),
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
    // provider_id was moved into account.provider_id, so read it from account.
    let provider = state.db.providers.get_by_id(
        &state.db.conn,
        account.provider_id.as_deref().unwrap_or(""),
    )?
    .ok_or_else(|| "Provider not found after creation".to_string())?;
    let models: Vec<String> = provider.models.as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    crate::services::pool_onboarding::ensure_account_in_pool(
        state,
        &account.id,
        &provider.id,
        &models,
        "Claude",
        "unified",
    )?;

    Ok(account)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn random_string(length: usize) -> String {
    use rand::distributions::Alphanumeric;
    use rand::Rng;
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}
