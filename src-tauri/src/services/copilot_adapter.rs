//! GitHub Copilot request adapter.
//!
//! Supports two authentication modes:
//! 1. PAT mode: user pastes a GitHub Personal Access Token (ghp_...) directly;
//!    the adapter validates it and exchanges it for a short-lived Copilot token.
//! 2. OAuth device flow: user authorizes via GitHub's device flow; the resulting
//!    access token is stored and refreshed like other OAuth credentials.
//!
//! Copilot exposes two upstream endpoints that share the same auth but use
//! different API shapes:
//! - `/chat/completions` — OpenAI Chat Completions (GPT-4o, Claude, Gemini, etc.)
//! - `/responses`       — OpenAI Responses (GPT-5 family, Codex variants)
//!
//! The adapter automatically selects the correct endpoint based on the model
//! requested by the client.

use crate::db::accounts::Account;
use crate::db::providers::Provider;
use crate::services::credentials::{payload_for_account, CredentialPayload};
use crate::AppState;
use chrono::{DateTime, Utc};
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::Value;
use sha2::Digest;
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;

// ── Constants ───────────────────────────────────────────────────────────────

/// Upstream Chat Completions endpoint.
pub const COPILOT_CHAT_URL: &str = "https://api.githubcopilot.com/chat/completions";

/// Upstream Responses endpoint (GPT-5+ models).
pub const COPILOT_RESPONSES_URL: &str = "https://api.githubcopilot.com/responses";

/// GitHub API endpoint to exchange a GitHub PAT for a short-lived Copilot token.
pub const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";

/// Models endpoint for health checks.
pub const COPILOT_MODELS_URL: &str = "https://api.githubcopilot.com/models";

/// Integration ID required by the Copilot API. `copilot-developer-cli` is
/// accepted for both PAT-based and OAuth-based requests.
pub const COPILOT_INTEGRATION_ID: &str = "copilot-developer-cli";

/// Copilot tokens expire after 25 minutes. Refresh 3 minutes early.
const COPILOT_TOKEN_TTL_SECS: i64 = 25 * 60;
const COPILOT_TOKEN_REFRESH_AHEAD_SECS: i64 = 3 * 60;

static REFRESH_LOCKS: LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

use std::collections::HashMap;

// ── Request context ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CopilotRequestContext {
    /// The active Copilot bearer token (not the raw GitHub PAT).
    pub copilot_token: String,
}

// ── Copilot token exchange response ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CopilotTokenResponse {
    token: Option<String>,
    expires_at: Option<String>,
    error: Option<String>,
}

// ── Account identification ──────────────────────────────────────────────────

/// Return `true` when the account uses a GitHub PAT that should be treated as a
/// Copilot credential.
pub fn is_copilot_pat(account: &Account, provider: &Provider) -> bool {
    let credential_type = account.credential_type.as_deref().unwrap_or("api_key");
    credential_type == "copilot_pat"
        || (credential_type == "api_key"
            && (provider.provider_type.eq_ignore_ascii_case("copilot")
                || provider
                    .provider_type
                    .eq_ignore_ascii_case("github_copilot")
                || provider.base_url.contains("githubcopilot.com")))
}

/// Return `true` when the account uses a GitHub Copilot OAuth credential
/// obtained via device flow.
pub fn is_copilot_oauth(account: &Account, provider: &Provider) -> bool {
    let credential_type = account.credential_type.as_deref().unwrap_or("api_key");
    credential_type == "copilot_oauth"
        || (credential_type == "oauth"
            && (provider.provider_type.eq_ignore_ascii_case("copilot")
                || provider
                    .provider_type
                    .eq_ignore_ascii_case("github_copilot")
                || provider.base_url.contains("githubcopilot.com")))
}

// ── Request context ─────────────────────────────────────────────────────────

/// Build a [`CopilotRequestContext`] from the stored account credentials.
///
/// For PAT accounts, the stored `api_key` / `access_token` is the GitHub PAT;
/// this function ensures a fresh Copilot token is available before returning.
pub async fn request_context(account: &Account) -> Result<CopilotRequestContext, String> {
    let payload = payload_for_account(account)?;
    let github_token = payload
        .api_key
        .clone()
        .filter(|k| !k.trim().is_empty())
        .or(payload.access_token.clone())
        .ok_or_else(|| "Copilot account has no GitHub token".to_string())?;

    // Check if a cached Copilot token is still valid.
    let now = Utc::now();
    let refresh_deadline = now + chrono::Duration::seconds(COPILOT_TOKEN_REFRESH_AHEAD_SECS);
    if let Some(ref expires_at) = payload.expires_at {
        if let Ok(expiry) = expires_at.parse::<DateTime<Utc>>() {
            if expiry > refresh_deadline {
                // Cached token is still fresh — use it directly.
                // The access_token field stores the Copilot token for PAT accounts.
                if let Some(copilot_token) = payload.access_token.filter(|t| !t.trim().is_empty()) {
                    return Ok(CopilotRequestContext { copilot_token });
                }
            }
        }
    }

    // Token is missing or expiring soon — fetch a fresh one from GitHub.
    let copilot_token = exchange_github_token_for_copilot(&github_token).await?;
    Ok(CopilotRequestContext { copilot_token })
}

// ── GitHub → Copilot token exchange ─────────────────────────────────────────

/// Exchange a GitHub Personal Access Token (or OAuth access token) for a
/// short-lived Copilot bearer token.
pub async fn exchange_github_token_for_copilot(github_token: &str) -> Result<String, String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;

    let response = client
        .post(COPILOT_TOKEN_URL)
        .header("Authorization", format!("token {}", github_token))
        .header("Accept", "application/json")
        .header(
            "User-Agent",
            crate::services::client_profiles::COPILOT_CHAT_USER_AGENT,
        )
        .send()
        .await
        .map_err(|error| format!("Copilot token request failed: {}", error))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!(
            "Copilot token exchange returned {}: {}",
            status,
            crate::services::redaction::redact_sensitive(&body)
        ));
    }

    let token_resp: CopilotTokenResponse = serde_json::from_str(&body)
        .map_err(|error| format!("Invalid Copilot token response: {}", error))?;

    if let Some(error) = token_resp.error {
        return Err(format!("Copilot token error: {}", error));
    }

    token_resp
        .token
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| "Copilot token response contained no token".to_string())
}

/// Persist the Copilot token and its expiry back to the account.
pub async fn persist_copilot_token(
    state: &AppState,
    account_id: &str,
    github_token: &str,
    copilot_token: &str,
    expires_at: Option<String>,
) -> Result<(), String> {
    let account = state
        .db
        .accounts
        .get_by_id(&state.db.conn, account_id)?
        .ok_or_else(|| "Account no longer exists".to_string())?;

    let mut payload = payload_for_account(&account)?;
    // Store the GitHub PAT in api_key and the Copilot token in access_token.
    payload.api_key = Some(github_token.to_string());
    payload.access_token = Some(copilot_token.to_string());
    payload.expires_at = expires_at;
    let credential_data = serde_json::to_string(&payload).map_err(|error| error.to_string())?;

    let mut updated = account;
    updated.credential_data = Some(credential_data);
    updated.api_key = github_token.to_string(); // Keep api_key column aligned.
    state.db.accounts.update(&state.db.conn, &updated)?;
    state
        .db
        .accounts
        .mark_token_refreshed(&state.db.conn, account_id)?;
    Ok(())
}

// ── Body preparation ────────────────────────────────────────────────────────

/// Prepare a Chat Completions request body. Copilot uses standard OpenAI format
/// so no conversion is needed; we just enforce `stream: true` for streaming
/// requests and ensure `store: false` is absent (Copilot ignores it).
pub fn prepare_chat_body(body: &Value) -> Value {
    // Copilot chat accepts the body as-is; no forced fields needed.
    body.clone()
}

/// Prepare a Responses request body. Like Codex, enforce `stream: true` and
/// `store: false`.
pub fn prepare_responses_body(body: &Value) -> Result<Value, String> {
    let mut value = body.clone();
    if let Some(object) = value.as_object_mut() {
        if object.get("stream").and_then(Value::as_bool) != Some(true) {
            return Err("Copilot Responses routing currently requires stream=true".into());
        }
        object.insert("store".into(), Value::Bool(false));
        Ok(value)
    } else {
        Err("Copilot Responses request body must be a JSON object".into())
    }
}

// ── Header helpers ──────────────────────────────────────────────────────────

pub fn apply_chat_headers(
    request: RequestBuilder,
    context: &CopilotRequestContext,
) -> RequestBuilder {
    crate::services::client_profiles::apply_copilot_profile(
        request
            .bearer_auth(&context.copilot_token)
            .header("Copilot-Integration-Id", COPILOT_INTEGRATION_ID)
            .header("Content-Type", "application/json"),
    )
}

pub fn apply_responses_headers(
    request: RequestBuilder,
    context: &CopilotRequestContext,
) -> RequestBuilder {
    crate::services::client_profiles::apply_copilot_profile(
        request
            .bearer_auth(&context.copilot_token)
            .header("Copilot-Integration-Id", COPILOT_INTEGRATION_ID)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream"),
    )
}

pub fn apply_health_headers(
    request: RequestBuilder,
    context: &CopilotRequestContext,
) -> RequestBuilder {
    crate::services::client_profiles::apply_copilot_profile(
        request
            .bearer_auth(&context.copilot_token)
            .header("Copilot-Integration-Id", COPILOT_INTEGRATION_ID)
            .header("Accept", "application/json"),
    )
}

// ── Endpoint selection ──────────────────────────────────────────────────────

/// Return the correct upstream URL for the requested model.
///
/// GPT-5 family and Codex variants require the Responses endpoint;
/// all other models use Chat Completions.
pub fn upstream_url_for_model(model: Option<&str>) -> &'static str {
    match model {
        Some(m)
            if m.starts_with("gpt-5")
                || m.contains("codex")
                || m.contains("o3")
                || m.contains("o4") =>
        {
            COPILOT_RESPONSES_URL
        }
        _ => COPILOT_CHAT_URL,
    }
}

// ── Health check ────────────────────────────────────────────────────────────

/// Perform a lightweight health check against the Copilot models endpoint.
pub async fn check_copilot_health(
    account: &Account,
) -> crate::services::health_check::HealthResult {
    use crate::services::health_check::HealthResult;
    use std::time::Instant;

    let context = match request_context(account).await {
        Ok(ctx) => ctx,
        Err(error) => return HealthResult::Error(error),
    };

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| Client::new());

    let start = Instant::now();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        apply_health_headers(client.get(COPILOT_MODELS_URL), &context).send(),
    )
    .await;

    match response {
        Ok(Ok(resp)) => {
            let latency = start.elapsed().as_millis() as u64;
            if resp.status().is_success() {
                HealthResult::Passed {
                    latency_ms: latency,
                }
            } else {
                let code = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                HealthResult::Failed { code, body }
            }
        }
        Ok(Err(error)) => HealthResult::Error(error.to_string()),
        Err(_) => HealthResult::Timeout,
    }
}

// ── 401 refresh retry ──────────────────────────────────────────────────────

/// Refresh the Copilot token after a real request returned 401.
///
/// For PAT accounts this simply re-exchanges the GitHub PAT for a fresh Copilot
/// token. Refreshes are serialized per account to prevent thundering herd.
pub async fn refresh_after_unauthorized(
    state: &AppState,
    account_id: &str,
) -> Result<Account, String> {
    use crate::services::credentials::payload_for_account;

    let lock = {
        let mut locks = REFRESH_LOCKS.lock().await;
        locks
            .entry(account_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = lock.lock().await;

    let account = state
        .db
        .accounts
        .get_by_id(&state.db.conn, account_id)?
        .ok_or_else(|| "Copilot account no longer exists".to_string())?;

    let payload = payload_for_account(&account)?;
    let github_token = payload
        .api_key
        .filter(|k| !k.trim().is_empty())
        .or(payload.access_token)
        .ok_or_else(|| "Copilot account has no GitHub token for refresh".to_string())?;

    let copilot_token = exchange_github_token_for_copilot(&github_token).await?;

    // Calculate approximate expiry.
    let expires_at =
        (Utc::now() + chrono::Duration::seconds(COPILOT_TOKEN_TTL_SECS - 30)).to_rfc3339();

    persist_copilot_token(
        state,
        account_id,
        &github_token,
        &copilot_token,
        Some(expires_at),
    )
    .await?;

    // Reload the updated account.
    state
        .db
        .accounts
        .get_by_id(&state.db.conn, account_id)?
        .ok_or_else(|| "Copilot account no longer exists after refresh".to_string())
}

// ── Device Flow ─────────────────────────────────────────────────────────────

/// GitHub OAuth device flow configuration.
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_DEVICE_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
/// Client ID for GitHub Copilot CLI (public, no secret).
const GITHUB_CLIENT_ID: &str = "Iv1.b507a2fd18a4e2c8";
const GITHUB_SCOPE: &str = "copilot";

#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
#[allow(dead_code)]
enum DeviceTokenResponse {
    Success {
        access_token: String,
        token_type: String,
        scope: String,
    },
    Pending {
        error: String,
        error_description: Option<String>,
        interval: Option<u64>,
    },
}

/// Start a GitHub OAuth device flow for Copilot.
///
/// Returns the device code, user code, verification URI, and polling interval.
pub async fn start_device_flow() -> Result<DeviceCodeResponse, String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;

    let response = client
        .post(GITHUB_DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .form(&[("client_id", GITHUB_CLIENT_ID), ("scope", GITHUB_SCOPE)])
        .send()
        .await
        .map_err(|error| format!("GitHub device code request failed: {}", error))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!(
            "GitHub device code request returned {}: {}",
            status, body
        ));
    }

    serde_json::from_str(&body).map_err(|error| format!("Invalid device code response: {}", error))
}

/// Poll GitHub for the device flow token.
///
/// Returns `Ok(Some(access_token))` when the user has authorized, `Ok(None)` if
/// still pending, and `Err` on permanent failure.
pub async fn poll_device_token(
    device_code: &str,
    interval_ms: u64,
) -> Result<Option<String>, String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;

    // Wait the required interval before polling.
    tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;

    let response = client
        .post(GITHUB_DEVICE_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", GITHUB_CLIENT_ID),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await
        .map_err(|error| format!("GitHub device token poll failed: {}", error))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!(
            "GitHub device token poll returned {}: {}",
            status, body
        ));
    }

    match serde_json::from_str::<DeviceTokenResponse>(&body)
        .map_err(|error| format!("Invalid device token response: {}", error))?
    {
        DeviceTokenResponse::Success { access_token, .. } => Ok(Some(access_token)),
        DeviceTokenResponse::Pending { error, .. } => {
            if error == "authorization_pending" || error == "slow_down" {
                Ok(None)
            } else {
                Err(format!("GitHub device flow error: {}", error))
            }
        }
    }
}

/// Complete a Copilot device flow login by exchanging the GitHub access token
/// for a Copilot token and persisting the account.
pub async fn complete_device_flow_login(
    state: &AppState,
    github_access_token: &str,
) -> Result<Account, String> {
    // Exchange GitHub token for Copilot token.
    let copilot_token = exchange_github_token_for_copilot(github_access_token).await?;
    let expires_at =
        (Utc::now() + chrono::Duration::seconds(COPILOT_TOKEN_TTL_SECS - 30)).to_rfc3339();

    // Create or reuse provider.
    let provider_id = "provider_github_copilot".to_string();
    if state
        .db
        .providers
        .get_by_id(&state.db.conn, &provider_id)?
        .is_none()
    {
        state.db.providers.create(
            &state.db.conn,
            &crate::db::providers::Provider {
                id: provider_id.clone(),
                name: "GitHub Copilot".into(),
                provider_type: "copilot".into(),
                base_url: "https://api.githubcopilot.com".into(),
                base_urls: Some(
                    "{\"chat\":\"https://api.githubcopilot.com\",\"responses\":\"https://api.githubcopilot.com\"}"
                        .into(),
                ),
                protocol: "openai".into(),
                protocols: Some("[\"chat\",\"responses\"]".into()),
                route_takeover: Some(1),
                api_keys: None,
                models: Some(
                    "[\"gpt-4o\",\"gpt-4o-mini\",\"claude-sonnet-4\",\"gemini-2.5-pro\",\"o3-mini\"]"
                        .into(),
                ),
                proxy_url: None,
                custom_headers: None,
                timeout_ms: Some(30000),
                priority: Some(0),
                enabled: Some(true),
                created_at: None,
                auth_mode: Some("oauth_device_flow".into()),
                oauth_config: Some(
                    serde_json::json!({
                        "device_code_url": GITHUB_DEVICE_CODE_URL,
                        "token_url": GITHUB_DEVICE_TOKEN_URL,
                        "client_id": GITHUB_CLIENT_ID,
                        "scope": GITHUB_SCOPE
                    })
                    .to_string(),
                ),
            },
        )?;
    }

    let fingerprint = hex::encode(sha2::Sha256::digest(
        format!("GitHubCopilot:{}", github_access_token).as_bytes(),
    ));

    // Check for duplicate.
    if let Some(existing) = state
        .db
        .accounts
        .find_by_fingerprint(&state.db.conn, &fingerprint)?
    {
        return Ok(existing);
    }

    let payload = CredentialPayload {
        access_token: Some(copilot_token.clone()),
        api_key: Some(github_access_token.to_string()),
        expires_at: Some(expires_at.clone()),
        token_type: Some("Bearer".into()),
        base_url: Some("https://api.githubcopilot.com".into()),
        ..CredentialPayload::default()
    };

    let account = Account {
        id: format!("acct_{}", uuid::Uuid::new_v4().simple()),
        provider_id: Some(provider_id),
        name: Some("GitHub Copilot account".into()),
        api_key: github_access_token.to_string(),
        models: Some(
            "[\"gpt-4o\",\"gpt-4o-mini\",\"claude-sonnet-4\",\"gemini-2.5-pro\",\"o3-mini\"]"
                .into(),
        ),
        quota_limit: None,
        quota_used: None,
        status: Some("active".into()),
        health_status: Some("unchecked".into()),
        health_code: None,
        health_msg: Some("GitHub Copilot OAuth 已启用".into()),
        health_latency: None,
        health_check_at: None,
        priority: Some(0),
        tags: Some("[\"official\",\"copilot\",\"oauth\"]".into()),
        last_used_at: None,
        created_at: None,
        credential_type: Some("copilot_oauth".into()),
        credential_data: Some(serde_json::to_string(&payload).map_err(|error| error.to_string())?),
        source_format: Some("oauth".into()),
        external_account_id: None,
        email: None,
        expires_at: Some(expires_at),
        metadata: None,
        credential_fingerprint: Some(fingerprint),
        protocols: Some("[\"chat\",\"responses\"]".into()),
        route_takeover: Some(1),
        plan_type: None,
        quota_windows: None,
        quota_refreshed_at: None,
        quota_error: None,
        token_refreshed_at: None,
        secret_ref: None,
    };

    {
        let conn = state.db.conn.lock().map_err(|error| error.to_string())?;
        crate::db::accounts::insert_account(&conn, &account)?;
    }

    // Auto-add to routing pool.
    // provider_id was moved into account.provider_id, so read it from account.
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
        "Copilot",
        "unified",
    )?;

    Ok(account)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chat_body_passthrough() {
        let body = json!({"model": "gpt-4o", "messages": [], "stream": true});
        let prepared = prepare_chat_body(&body);
        assert_eq!(prepared, body);
    }

    #[test]
    fn responses_body_enforces_streaming() {
        let body = json!({"model": "gpt-5", "input": "hello", "stream": true, "store": true});
        let prepared = prepare_responses_body(&body).unwrap();
        assert_eq!(prepared["stream"], true);
        assert_eq!(prepared["store"], false);
    }

    #[test]
    fn responses_body_rejects_non_streaming() {
        let body = json!({"model": "gpt-5", "input": "hello", "stream": false});
        let error = prepare_responses_body(&body).unwrap_err();
        assert!(error.contains("stream=true"));
    }

    #[test]
    fn endpoint_selection_gpt5() {
        assert_eq!(upstream_url_for_model(Some("gpt-5")), COPILOT_RESPONSES_URL);
        assert_eq!(
            upstream_url_for_model(Some("gpt-5-mini")),
            COPILOT_RESPONSES_URL
        );
        assert_eq!(
            upstream_url_for_model(Some("codex-mini")),
            COPILOT_RESPONSES_URL
        );
        assert_eq!(
            upstream_url_for_model(Some("o3-mini")),
            COPILOT_RESPONSES_URL
        );
        assert_eq!(
            upstream_url_for_model(Some("o4-mini")),
            COPILOT_RESPONSES_URL
        );
    }

    #[test]
    fn endpoint_selection_chat() {
        assert_eq!(upstream_url_for_model(Some("gpt-4o")), COPILOT_CHAT_URL);
        assert_eq!(
            upstream_url_for_model(Some("gpt-4o-mini")),
            COPILOT_CHAT_URL
        );
        assert_eq!(
            upstream_url_for_model(Some("claude-sonnet-4")),
            COPILOT_CHAT_URL
        );
        assert_eq!(upstream_url_for_model(None), COPILOT_CHAT_URL);
    }

    #[test]
    fn copilot_pat_identification() {
        let account = Account {
            id: "acct-test".into(),
            provider_id: None,
            name: None,
            api_key: "ghp_test".into(),
            models: None,
            quota_limit: None,
            quota_used: None,
            status: Some("active".into()),
            health_status: None,
            health_code: None,
            health_msg: None,
            health_latency: None,
            health_check_at: None,
            priority: None,
            tags: None,
            last_used_at: None,
            created_at: None,
            credential_type: Some("copilot_pat".into()),
            credential_data: Some(r#"{"api_key":"ghp_test"}"#.into()),
            source_format: None,
            external_account_id: None,
            email: None,
            expires_at: None,
            metadata: None,
            credential_fingerprint: None,
            protocols: None,
            route_takeover: None,
            plan_type: None,
            quota_windows: None,
            quota_refreshed_at: None,
            quota_error: None,
            token_refreshed_at: None,
            secret_ref: None,
        };
        let provider = provider("copilot", "https://api.githubcopilot.com");
        assert!(is_copilot_pat(&account, &provider));
        assert!(!is_copilot_oauth(&account, &provider));
    }

    #[test]
    fn copilot_oauth_identification() {
        let account = Account {
            id: "acct-test".into(),
            provider_id: None,
            name: None,
            api_key: "ghu_test".into(),
            models: None,
            quota_limit: None,
            quota_used: None,
            status: Some("active".into()),
            health_status: None,
            health_code: None,
            health_msg: None,
            health_latency: None,
            health_check_at: None,
            priority: None,
            tags: None,
            last_used_at: None,
            created_at: None,
            credential_type: Some("copilot_oauth".into()),
            credential_data: Some(r#"{"access_token":"ghu_test"}"#.into()),
            source_format: None,
            external_account_id: None,
            email: None,
            expires_at: None,
            metadata: None,
            credential_fingerprint: None,
            protocols: None,
            route_takeover: None,
            plan_type: None,
            quota_windows: None,
            quota_refreshed_at: None,
            quota_error: None,
            token_refreshed_at: None,
            secret_ref: None,
        };
        let provider = provider("copilot", "https://api.githubcopilot.com");
        assert!(!is_copilot_pat(&account, &provider));
        assert!(is_copilot_oauth(&account, &provider));
    }

    fn provider(provider_type: &str, base_url: &str) -> Provider {
        Provider {
            id: "test".into(),
            name: "Test".into(),
            provider_type: provider_type.into(),
            base_url: base_url.into(),
            base_urls: None,
            protocol: "openai".into(),
            protocols: Some("[\"chat\",\"responses\"]".into()),
            route_takeover: None,
            api_keys: None,
            models: None,
            proxy_url: None,
            custom_headers: None,
            timeout_ms: None,
            priority: None,
            enabled: None,
            created_at: None,
            auth_mode: None,
            oauth_config: None,
        }
    }
}
