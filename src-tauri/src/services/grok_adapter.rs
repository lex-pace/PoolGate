//! xAI Grok request adapter.
//!
//! Grok subscriptions authenticate via OAuth against xAI's authorization
//! servers. The resulting access token is used as a Bearer token against the
//! xAI Chat Completions API (`api.x.ai/v1/chat/completions`).
//!
//! Grok uses the OpenAI-compatible API format, so no special body transformation
//! is needed beyond standard Chat Completions / Responses conversion handled by
//! PoolGate's existing `openai.rs` protocol converter.

use crate::db::accounts::Account;
use crate::db::providers::Provider;
use crate::services::credentials::payload_for_account;
use crate::AppState;
use chrono::{DateTime, Utc};
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;

// ── Constants ───────────────────────────────────────────────────────────────

/// xAI Chat Completions endpoint.
pub const GROK_CHAT_URL: &str = "https://api.x.ai/v1/chat/completions";

/// xAI Responses endpoint (if supported, otherwise same as chat).
pub const GROK_RESPONSES_URL: &str = "https://api.x.ai/v1/responses";

/// xAI models endpoint for health checks.
pub const GROK_MODELS_URL: &str = "https://api.x.ai/v1/models";

/// Token refresh ahead of expiry (5 minutes).
const GROK_TOKEN_REFRESH_AHEAD_SECS: i64 = 5 * 60;

static REFRESH_LOCKS: LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

use std::collections::HashMap;

// ── Request context ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct GrokRequestContext {
    pub access_token: String,
}

// ── Account identification ──────────────────────────────────────────────────

/// Return `true` when the account uses xAI Grok subscription OAuth.
pub fn is_grok_oauth(account: &Account, provider: &Provider) -> bool {
    let credential_type = account.credential_type.as_deref().unwrap_or("api_key");
    credential_type == "grok_oauth"
        || (credential_type == "oauth"
            && (provider.provider_type.eq_ignore_ascii_case("grok")
                || provider.provider_type.eq_ignore_ascii_case("xai")
                || provider.base_url.contains("api.x.ai")))
}

// ── Request context ─────────────────────────────────────────────────────────

/// Build a [`GrokRequestContext`] from stored account credentials.
pub async fn request_context(account: &Account) -> Result<GrokRequestContext, String> {
    let payload = payload_for_account(account)?;

    // Check if token is still valid.
    let now = Utc::now();
    let refresh_deadline = now + chrono::Duration::seconds(GROK_TOKEN_REFRESH_AHEAD_SECS);
    if let Some(ref expires_at) = payload.expires_at {
        if let Ok(expiry) = expires_at.parse::<DateTime<Utc>>() {
            if expiry > refresh_deadline {
                if let Some(access_token) = payload.access_token.clone().filter(|t| !t.trim().is_empty()) {
                    return Ok(GrokRequestContext { access_token });
                }
            }
        }
    }

    // Token is missing or expiring soon.
    payload
        .access_token
        .filter(|t| !t.trim().is_empty())
        .map(|access_token| GrokRequestContext { access_token })
        .ok_or_else(|| "Grok account has no access token".to_string())
}

// ── Body preparation ────────────────────────────────────────────────────────

/// Prepare a Chat Completions request body. Grok uses standard OpenAI format.
pub fn prepare_chat_body(body: &Value) -> Value {
    body.clone()
}

/// Prepare a Responses request body (if xAI supports it).
pub fn prepare_responses_body(body: &Value) -> Result<Value, String> {
    let mut value = body.clone();
    if let Some(object) = value.as_object_mut() {
        if object.get("stream").and_then(Value::as_bool) != Some(true) {
            return Err("Grok Responses routing currently requires stream=true".into());
        }
        object.insert("store".into(), Value::Bool(false));
        Ok(value)
    } else {
        Err("Grok Responses request body must be a JSON object".into())
    }
}

// ── Header helpers ──────────────────────────────────────────────────────────

pub fn apply_chat_headers(request: RequestBuilder, context: &GrokRequestContext) -> RequestBuilder {
    request
        .bearer_auth(&context.access_token)
        .header("Content-Type", "application/json")
        .header("User-Agent", "PoolGate/1.0")
}

pub fn apply_responses_headers(request: RequestBuilder, context: &GrokRequestContext) -> RequestBuilder {
    request
        .bearer_auth(&context.access_token)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("User-Agent", "PoolGate/1.0")
}

pub fn apply_health_headers(request: RequestBuilder, context: &GrokRequestContext) -> RequestBuilder {
    request
        .bearer_auth(&context.access_token)
        .header("Accept", "application/json")
        .header("User-Agent", "PoolGate/1.0")
}

// ── Endpoint selection ──────────────────────────────────────────────────────

/// Return the correct upstream URL for the requested model.
pub fn upstream_url_for_model(model: Option<&str>) -> &'static str {
    match model {
        Some(m) if m.contains("responses") || m.contains("reasoning") => GROK_RESPONSES_URL,
        _ => GROK_CHAT_URL,
    }
}

// ── Health check ────────────────────────────────────────────────────────────

/// Perform a lightweight health check against the xAI models endpoint.
pub async fn check_grok_health(account: &Account) -> crate::services::health_check::HealthResult {
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
        apply_health_headers(client.get(GROK_MODELS_URL), &context).send(),
    )
    .await;

    match response {
        Ok(Ok(resp)) => {
            let latency = start.elapsed().as_millis() as u64;
            if resp.status().is_success() {
                HealthResult::Passed { latency_ms: latency }
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

/// Refresh a Grok OAuth access token after a real request returned 401.
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
        .ok_or_else(|| "Grok account no longer exists".to_string())?;

    let payload = payload_for_account(&account)?;
    let refresh_token = payload
        .refresh_token
        .clone()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| {
            "Grok OAuth account has no refresh_token; re-authorize required.".to_string()
        })?;

    // Read token endpoint from metadata.
    let metadata = payload.metadata.clone().unwrap_or(Value::Null);
    let token_url = metadata
        .get("token_url")
        .and_then(Value::as_str)
        .unwrap_or("https://accounts.x.ai/oauth2/token")
        .to_string();
    let client_id = metadata
        .get("client_id")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| "grok-cli".to_string());

    let mut params = std::collections::HashMap::new();
    params.insert("grant_type", "refresh_token");
    params.insert("refresh_token", &refresh_token);
    params.insert("client_id", &client_id);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;

    let response = client
        .post(&token_url)
        .form(&params)
        .send()
        .await
        .map_err(|error| format!("Grok token refresh failed: {}", error))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!(
            "Grok token refresh returned {}: {}",
            status,
            crate::services::redaction::redact_sensitive(&body)
        ));
    }

    let json: Value =
        serde_json::from_str(&body).map_err(|error| format!("Invalid refresh response: {}", error))?;

    let new_access_token = json["access_token"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "Refresh response missing access_token".to_string())?;

    let new_refresh_token = json["refresh_token"]
        .as_str()
        .map(String::from)
        .or(payload.refresh_token.clone());

    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
    let new_expires_at =
        (Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339();

    // Persist.
    let mut new_payload = payload;
    new_payload.access_token = Some(new_access_token);
    new_payload.refresh_token = new_refresh_token;
    new_payload.expires_at = Some(new_expires_at.clone());

    let credential_data = serde_json::to_string(&new_payload).map_err(|error| error.to_string())?;
    let mut updated = account;
    updated.api_key = String::new();
    updated.credential_data = Some(credential_data);
    updated.expires_at = Some(new_expires_at);
    state.db.accounts.update(&state.db.conn, &updated)?;
    state.db.accounts.mark_token_refreshed(&state.db.conn, account_id)?;

    state
        .db
        .accounts
        .get_by_id(&state.db.conn, account_id)?
        .ok_or_else(|| "Grok account no longer exists after refresh".to_string())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chat_body_passthrough() {
        let body = json!({"model": "grok-4", "messages": [], "stream": true});
        let prepared = prepare_chat_body(&body);
        assert_eq!(prepared, body);
    }

    #[test]
    fn responses_body_enforces_streaming() {
        let body = json!({"model": "grok-4", "input": "hello", "stream": true, "store": true});
        let prepared = prepare_responses_body(&body).unwrap();
        assert_eq!(prepared["stream"], true);
        assert_eq!(prepared["store"], false);
    }

    #[test]
    fn responses_body_rejects_non_streaming() {
        let body = json!({"model": "grok-4", "input": "hello", "stream": false});
        let error = prepare_responses_body(&body).unwrap_err();
        assert!(error.contains("stream=true"));
    }

    #[test]
    fn endpoint_selection_default() {
        assert_eq!(upstream_url_for_model(Some("grok-4")), GROK_CHAT_URL);
        assert_eq!(upstream_url_for_model(None), GROK_CHAT_URL);
    }

    #[test]
    fn grok_oauth_identification() {
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
            credential_type: Some("grok_oauth".into()),
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
            provider_type: "xai".into(),
            base_url: "https://api.x.ai".into(),
            base_urls: None,
            protocol: "openai".into(),
            protocols: Some("[\"chat\"]".into()),
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
        assert!(is_grok_oauth(&account, &provider));
    }
}
