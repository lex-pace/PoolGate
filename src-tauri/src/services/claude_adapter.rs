//! Anthropic Claude Code request adapter.
//!
//! Claude Code subscriptions authenticate via OAuth (PKCE) against Anthropic's
//! authorization servers. The resulting access token is used as a Bearer token
//! against the standard Anthropic Messages API (`api.anthropic.com/v1/messages`).
//!
//! **Critical constraint**: Claude Code OAuth requests MUST include a fixed
//! system prompt prefix as the first system message block:
//!
//! > "You are Claude Code, Anthropic's official CLI for Claude."
//!
//! Omitting this prefix causes the Anthropic backend to reject the request.
//!
//! This adapter handles the prefix injection automatically when preparing the
//! request body, and ensures the Authorization header uses Bearer (not x-api-key).

use crate::db::accounts::Account;
use crate::db::providers::Provider;
use crate::services::credentials::payload_for_account;
use crate::AppState;
use chrono::{DateTime, Utc};
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::Value;
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;

// ── Constants ───────────────────────────────────────────────────────────────

/// Anthropic Messages API endpoint.
pub const CLAUDE_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";

/// OAuth token endpoint for refreshing tokens.
pub const CLAUDE_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";

/// OAuth authorization endpoint for Pro plan.
pub const CLAUDE_AUTHORIZE_URL_PRO: &str = "https://console.anthropic.com/oauth/authorize";

/// OAuth authorization endpoint for Max plan.
pub const CLAUDE_AUTHORIZE_URL_MAX: &str = "https://claude.ai/oauth/authorize";

/// The system prompt prefix required by Claude Code OAuth requests.
pub const CLAUDE_CODE_SYSTEM_PREFIX: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";

/// Anthropic API version header.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Token refresh ahead of expiry (5 minutes).
const CLAUDE_TOKEN_REFRESH_AHEAD_SECS: i64 = 5 * 60;

static REFRESH_LOCKS: LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

use std::collections::HashMap;

// ── Request context ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ClaudeRequestContext {
    /// The OAuth access token.
    pub access_token: String,
}

// ── OAuth token response ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    token_type: Option<String>,
}

// ── Account identification ──────────────────────────────────────────────────

/// Return `true` when the account uses Claude Code subscription OAuth.
pub fn is_claude_oauth(account: &Account, provider: &Provider) -> bool {
    let credential_type = account.credential_type.as_deref().unwrap_or("api_key");
    credential_type == "claude_oauth"
        || (credential_type == "oauth"
            && (provider.provider_type.eq_ignore_ascii_case("claude")
                || provider.provider_type.eq_ignore_ascii_case("anthropic")
                || provider.base_url.contains("api.anthropic.com")
                || provider.base_url.contains("claude.ai")))
}

// ── Request context ─────────────────────────────────────────────────────────

/// Build a [`ClaudeRequestContext`] from stored account credentials.
pub async fn request_context(account: &Account) -> Result<ClaudeRequestContext, String> {
    let payload = payload_for_account(account)?;

    // Check if the token is still valid.
    let now = Utc::now();
    let refresh_deadline = now + chrono::Duration::seconds(CLAUDE_TOKEN_REFRESH_AHEAD_SECS);
    if let Some(ref expires_at) = payload.expires_at {
        if let Ok(expiry) = expires_at.parse::<DateTime<Utc>>() {
            if expiry > refresh_deadline && payload.access_token.is_some() {
                return Ok(ClaudeRequestContext {
                    access_token: payload.access_token.unwrap(),
                });
            }
        }
    }

    // Token is missing or expiring soon — try to use it directly.
    // The caller will handle 401 retries if the token is actually expired.
    payload
        .access_token
        .filter(|t| !t.trim().is_empty())
        .map(|access_token| ClaudeRequestContext { access_token })
        .ok_or_else(|| "Claude account has no access token".to_string())
}

// ── Body preparation ────────────────────────────────────────────────────────

/// Prepare an Anthropic Messages API request body.
///
/// **Critical**: Injects the required Claude Code system prompt prefix as the
/// first system message block. If the caller already provided a `system` field,
/// the prefix is prepended. If no `system` field exists, one is created.
pub fn prepare_messages_body(body: &Value, _stream: bool) -> Value {
    let mut value = body.clone();
    if let Some(object) = value.as_object_mut() {
        // Inject or prepend system prefix. When the client is Claude Code
        // itself the prefix is already the first system block — skip so the
        // gateway never duplicates it.
        let already_prefixed = object
            .get("system")
            .and_then(|system| system.as_array())
            .and_then(|blocks| blocks.first())
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str)
            .is_some_and(|text| text.starts_with(CLAUDE_CODE_SYSTEM_PREFIX));
        if !already_prefixed {
            match object.get("system") {
                Some(existing) => {
                    // Existing system field: prepend the prefix.
                    let prefix_block = serde_json::json!({
                        "type": "text",
                        "text": CLAUDE_CODE_SYSTEM_PREFIX
                    });

                    let new_system = match existing {
                        Value::String(s) => {
                            // Simple string system prompt: convert to array with prefix first.
                            serde_json::json!([
                                prefix_block,
                                {
                                    "type": "text",
                                    "text": s
                                }
                            ])
                        }
                        Value::Array(arr) => {
                            // Array of system blocks: prepend prefix block.
                            let mut new_arr = vec![prefix_block];
                            new_arr.extend(arr.clone());
                            Value::Array(new_arr)
                        }
                        _ => {
                            // Other types: wrap as text block after prefix.
                            serde_json::json!([
                                prefix_block,
                                {
                                    "type": "text",
                                    "text": existing.to_string()
                                }
                            ])
                        }
                    };
                    object.insert("system".into(), new_system);
                }
                None => {
                    // No system field: create one with just the prefix.
                    object.insert(
                        "system".into(),
                        serde_json::json!([
                            {
                                "type": "text",
                                "text": CLAUDE_CODE_SYSTEM_PREFIX
                            }
                        ]),
                    );
                }
            }
        }

        // Enforce max_tokens if not present (required by Anthropic API).
        if !object.contains_key("max_tokens") {
            object.insert("max_tokens".into(), Value::Number(4096.into()));
        }
    }
    value
}

// ── Header helpers ──────────────────────────────────────────────────────────

pub fn apply_headers(
    request: RequestBuilder,
    context: &ClaudeRequestContext,
    stream: bool,
) -> RequestBuilder {
    // Present the full Claude Code fingerprint: subscription backends profile
    // the client, and a self-identifying gateway UA is a third-party signal.
    let request = crate::services::client_profiles::apply_claude_code_profile(
        request
            .bearer_auth(&context.access_token)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("Content-Type", "application/json"),
    );

    if stream {
        request.header("Accept", "text/event-stream")
    } else {
        request.header("Accept", "application/json")
    }
}

// ── Health check ────────────────────────────────────────────────────────────

/// Perform a lightweight health check against the Anthropic Messages API.
pub async fn check_claude_health(account: &Account) -> crate::services::health_check::HealthResult {
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

    let body = serde_json::json!({
        "model": "claude-3-haiku-20240307",
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "ping"}]
    });

    let start = Instant::now();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        apply_headers(client.post(CLAUDE_MESSAGES_URL), &context, false)
            .json(&body)
            .send(),
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

/// Refresh a Claude OAuth access token after a real request returned 401.
///
/// Uses the stored refresh token to obtain a new access token from Anthropic's
/// token endpoint. Refreshes are serialized per account.
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
        .ok_or_else(|| "Claude account no longer exists".to_string())?;

    let payload = payload_for_account(&account)?;
    let refresh_token = payload
        .refresh_token
        .clone()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| {
            "Claude OAuth account has no refresh_token; re-authorize required.".to_string()
        })?;

    // Read client_id from metadata.
    let metadata = payload.metadata.clone().unwrap_or(Value::Null);
    let client_id = metadata
        .get("client_id")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| "public-claude-code".to_string());

    let mut params = std::collections::HashMap::new();
    params.insert("grant_type", "refresh_token");
    params.insert("refresh_token", &refresh_token);
    params.insert("client_id", &client_id);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;

    let response = client
        .post(CLAUDE_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|error| format!("Claude token refresh failed: {}", error))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!(
            "Claude token refresh returned {}: {}",
            status,
            crate::services::redaction::redact_sensitive(&body)
        ));
    }

    let json: Value = serde_json::from_str(&body)
        .map_err(|error| format!("Invalid refresh response: {}", error))?;

    let new_access_token = json["access_token"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "Refresh response missing access_token".to_string())?;

    // Anthropic may or may not rotate refresh tokens; preserve existing.
    let new_refresh_token = json["refresh_token"]
        .as_str()
        .map(String::from)
        .or(payload.refresh_token.clone());

    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
    let new_expires_at = (Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339();

    // Persist.
    let mut new_payload = payload;
    new_payload.access_token = Some(new_access_token);
    new_payload.refresh_token = new_refresh_token;
    new_payload.expires_at = Some(new_expires_at.clone());

    let credential_data = serde_json::to_string(&new_payload).map_err(|error| error.to_string())?;
    let mut updated = account;
    updated.api_key = String::new(); // Not used for OAuth.
    updated.credential_data = Some(credential_data);
    updated.expires_at = Some(new_expires_at);
    state.db.accounts.update(&state.db.conn, &updated)?;
    state
        .db
        .accounts
        .mark_token_refreshed(&state.db.conn, account_id)?;

    state
        .db
        .accounts
        .get_by_id(&state.db.conn, account_id)?
        .ok_or_else(|| "Claude account no longer exists after refresh".to_string())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_body_injects_system_prefix_when_absent() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let prepared = prepare_messages_body(&body, true);

        let system = prepared["system"].as_array().unwrap();
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["text"], CLAUDE_CODE_SYSTEM_PREFIX);
        assert_eq!(prepared["max_tokens"], 4096);
    }

    #[test]
    fn messages_body_prepends_system_prefix_to_existing_string() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "system": "You are helpful.",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let prepared = prepare_messages_body(&body, false);

        let system = prepared["system"].as_array().unwrap();
        assert_eq!(system.len(), 2);
        assert_eq!(system[0]["text"], CLAUDE_CODE_SYSTEM_PREFIX);
        assert_eq!(system[1]["text"], "You are helpful.");
    }

    #[test]
    fn messages_body_prepends_system_prefix_to_existing_array() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "system": [
                {"type": "text", "text": "Be concise."}
            ],
            "messages": [{"role": "user", "content": "hi"}]
        });
        let prepared = prepare_messages_body(&body, true);

        let system = prepared["system"].as_array().unwrap();
        assert_eq!(system.len(), 2);
        assert_eq!(system[0]["text"], CLAUDE_CODE_SYSTEM_PREFIX);
        assert_eq!(system[1]["text"], "Be concise.");
    }

    #[test]
    fn messages_body_preserves_existing_max_tokens() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 2048,
            "messages": [{"role": "user", "content": "hi"}]
        });
        let prepared = prepare_messages_body(&body, false);
        assert_eq!(prepared["max_tokens"], 2048);
    }

    #[test]
    fn messages_body_does_not_duplicate_prefix_from_claude_code_client() {
        // Claude Code already sends the prefix as the first system block.
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "system": [
                {"type": "text", "text": CLAUDE_CODE_SYSTEM_PREFIX},
                {"type": "text", "text": "Be concise."}
            ],
            "messages": [{"role": "user", "content": "hi"}]
        });
        let prepared = prepare_messages_body(&body, true);
        let system = prepared["system"].as_array().unwrap();
        assert_eq!(system.len(), 2, "prefix must not be injected twice");
        assert_eq!(system[0]["text"], CLAUDE_CODE_SYSTEM_PREFIX);
    }

    #[test]
    fn claude_oauth_identification() {
        let account = Account {
            id: "acct-test".into(),
            provider_id: None,
            name: None,
            api_key: "".into(),
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
            credential_type: Some("claude_oauth".into()),
            credential_data: Some(r#"{"access_token":"test"}"#.into()),
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
        let provider = Provider {
            id: "test".into(),
            name: "Test".into(),
            provider_type: "anthropic".into(),
            base_url: "https://api.anthropic.com".into(),
            base_urls: None,
            protocol: "messages".into(),
            protocols: Some("[\"messages\"]".into()),
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
        };
        assert!(is_claude_oauth(&account, &provider));
    }
}
