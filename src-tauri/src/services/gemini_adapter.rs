//! Google Gemini request adapter.
//!
//! Supports two authentication modes:
//! 1. API Key mode (AI Studio): user pastes a Google AI Studio API key; the
//!    adapter passes it as a query parameter to the Gemini REST API.
//! 2. OAuth mode: user authorizes via Google OAuth; the resulting access token
//!    is stored and used as a Bearer token.
//!
//! Gemini uses its own request/response format (`contents` + `generationConfig`)
//! rather than OpenAI Chat Completions. PoolGate's existing `gemini.rs` protocol
//! converter handles the transformation from Responses / Chat Completions format
//! to Gemini native format before the request reaches this adapter.

use crate::db::accounts::Account;
use crate::db::providers::Provider;
use crate::services::credentials::payload_for_account;
use crate::AppState;
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;

// ── Constants ───────────────────────────────────────────────────────────────

pub const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";
pub const GEMINI_MODELS_PATH: &str = "/v1beta/models";

/// OAuth token endpoint.
pub const GEMINI_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// OAuth authorization endpoint.
pub const GEMINI_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/auth";

static REFRESH_LOCKS: LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

use std::collections::HashMap;

// ── Request context ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct GeminiRequestContext {
    /// API Key mode: `Some("AIza...")`.
    /// OAuth mode: `None` (token is applied as Bearer).
    pub api_key: Option<String>,
    /// OAuth mode: `Some("ya29...")`.
    /// API Key mode: `None`.
    pub access_token: Option<String>,
}

// ── Account identification ──────────────────────────────────────────────────

/// Return `true` when the account uses a Google AI Studio API Key.
pub fn is_gemini_api_key(account: &Account, provider: &Provider) -> bool {
    let credential_type = account.credential_type.as_deref().unwrap_or("api_key");
    credential_type == "gemini_api_key"
        || (credential_type == "api_key"
            && (provider.provider_type.eq_ignore_ascii_case("gemini")
                || provider.provider_type.eq_ignore_ascii_case("google")
                || provider.provider_type.eq_ignore_ascii_case("google_ai")
                || provider.base_url.contains("generativelanguage.googleapis.com")))
}

/// Return `true` when the account uses Google OAuth (Gemini subscription).
pub fn is_gemini_oauth(account: &Account, provider: &Provider) -> bool {
    let credential_type = account.credential_type.as_deref().unwrap_or("api_key");
    credential_type == "gemini_oauth"
        || (credential_type == "oauth"
            && (provider.provider_type.eq_ignore_ascii_case("gemini")
                || provider.provider_type.eq_ignore_ascii_case("google")
                || provider.provider_type.eq_ignore_ascii_case("google_ai")
                || provider.base_url.contains("generativelanguage.googleapis.com")))
}

// ── Request context ─────────────────────────────────────────────────────────

/// Build a [`GeminiRequestContext`] from stored account credentials.
pub async fn request_context(account: &Account) -> Result<GeminiRequestContext, String> {
    let payload = payload_for_account(account)?;

    // API Key mode: the key lives in api_key.
    if let Some(api_key) = payload.api_key.filter(|k| !k.trim().is_empty()) {
        return Ok(GeminiRequestContext {
            api_key: Some(api_key),
            access_token: None,
        });
    }

    // OAuth mode: the token lives in access_token.
    if let Some(access_token) = payload.access_token.filter(|t| !t.trim().is_empty()) {
        return Ok(GeminiRequestContext {
            api_key: None,
            access_token: Some(access_token),
        });
    }

    Err("Gemini account has no API key or access token".to_string())
}

// ── Body preparation ────────────────────────────────────────────────────────

/// Prepare a Gemini native request body.
///
/// The caller (PoolGate's protocol converter) should already have transformed
/// the body from Responses / Chat Completions format into Gemini native format.
/// This function just ensures `stream: true` is set for streaming requests.
pub fn prepare_generate_body(body: &Value, stream: bool) -> Value {
    let mut value = body.clone();
    if let Some(object) = value.as_object_mut() {
        if stream {
            // Gemini uses a separate endpoint for streaming rather than a body
            // field, but some callers may set it; ensure it's present for clarity.
            object.insert("stream".into(), Value::Bool(true));
        }
    }
    value
}

// ── URL helpers ─────────────────────────────────────────────────────────────

/// Build the Gemini REST API URL for a model action.
///
/// Example: `https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent`
pub fn gemini_model_action_url(model: &str, action: &str) -> String {
    format!(
        "{}/v1beta/models/{}:{}",
        GEMINI_BASE_URL, model, action
    )
}

// ── Header helpers ──────────────────────────────────────────────────────────

pub fn apply_headers(
    request: RequestBuilder,
    context: &GeminiRequestContext,
    stream: bool,
) -> RequestBuilder {
    let request = if let Some(ref api_key) = context.api_key {
        request.query(&[("key", api_key.as_str())])
    } else if let Some(ref access_token) = context.access_token {
        request.bearer_auth(access_token)
    } else {
        request // No auth — will fail upstream.
    };

    let request = request
        .header("Content-Type", "application/json")
        .header("User-Agent", "PoolGate/1.0");

    if stream {
        // Gemini uses `alt=sse` query parameter for streaming.
        request.query(&[("alt", "sse")])
    } else {
        request
    }
}

// ── Health check ────────────────────────────────────────────────────────────

/// Perform a lightweight health check against the Gemini models endpoint.
pub async fn check_gemini_health(account: &Account) -> crate::services::health_check::HealthResult {
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

    let url = format!("{}{}", GEMINI_BASE_URL, GEMINI_MODELS_PATH);
    let start = Instant::now();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        apply_headers(client.get(&url), &context, false).send(),
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

// ── 401 refresh retry (OAuth mode only) ────────────────────────────────────

/// Refresh a Google OAuth access token using the stored refresh token.
///
/// For API Key accounts (no OAuth), this is a no-op that returns an error.
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
        .ok_or_else(|| "Gemini account no longer exists".to_string())?;

    let payload = payload_for_account(&account)?;
    let refresh_token = payload
        .refresh_token
        .clone()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| {
            "Gemini OAuth account has no refresh_token; cannot refresh. \
             Re-authorize or use an API Key instead."
                .to_string()
        })?;

    // Read client_id / client_secret from payload.metadata.
    let metadata = payload.metadata.clone().unwrap_or(Value::Null);
    let client_id = metadata
        .get("client_id")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| "Gemini OAuth metadata missing client_id".to_string())?;
    let client_secret = metadata
        .get("client_secret")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_default();

    let mut params = std::collections::HashMap::new();
    params.insert("grant_type", "refresh_token");
    params.insert("refresh_token", &refresh_token);
    params.insert("client_id", &client_id);
    if !client_secret.is_empty() {
        params.insert("client_secret", &client_secret);
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;

    let response = client
        .post(GEMINI_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|error| format!("Google token refresh failed: {}", error))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!(
            "Google token refresh returned {}: {}",
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

    // Google typically does not rotate refresh tokens; preserve existing.
    let new_refresh_token = json["refresh_token"]
        .as_str()
        .map(String::from)
        .or(payload.refresh_token.clone());

    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
    let new_expires_at =
        (chrono::Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339();

    // Persist.
    let mut new_payload = payload;
    new_payload.access_token = Some(new_access_token);
    new_payload.refresh_token = new_refresh_token;
    new_payload.expires_at = Some(new_expires_at.clone());

    let credential_data = serde_json::to_string(&new_payload).map_err(|error| error.to_string())?;
    let mut updated = account;
    updated.api_key = String::new(); // API key column not used for OAuth.
    updated.credential_data = Some(credential_data);
    updated.expires_at = Some(new_expires_at);
    state.db.accounts.update(&state.db.conn, &updated)?;
    state.db.accounts.mark_token_refreshed(&state.db.conn, account_id)?;

    state
        .db
        .accounts
        .get_by_id(&state.db.conn, account_id)?
        .ok_or_else(|| "Gemini account no longer exists after refresh".to_string())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gemini_url_construction() {
        assert_eq!(
            gemini_model_action_url("gemini-2.5-pro", "generateContent"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent"
        );
        assert_eq!(
            gemini_model_action_url("gemini-2.5-flash", "streamGenerateContent"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:streamGenerateContent"
        );
    }

    #[test]
    fn prepare_body_sets_stream() {
        let body = json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]});
        let prepared = prepare_generate_body(&body, true);
        assert_eq!(prepared["stream"], true);
    }

    #[test]
    fn prepare_body_no_stream() {
        let body = json!({"contents": []});
        let prepared = prepare_generate_body(&body, false);
        assert!(!prepared.as_object().unwrap().contains_key("stream"));
    }

    #[test]
    fn api_key_identification() {
        let account = Account {
            id: "acct-test".into(),
            provider_id: None,
            name: None,
            api_key: "AIza_test".into(),
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
            credential_type: Some("api_key".into()),
            credential_data: Some(r#"{"api_key":"AIza_test"}"#.into()),
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
        let provider = provider("gemini", "https://generativelanguage.googleapis.com");
        assert!(is_gemini_api_key(&account, &provider));
        assert!(!is_gemini_oauth(&account, &provider));
    }

    #[test]
    fn oauth_identification() {
        let account = Account {
            id: "acct-test".into(),
            provider_id: None,
            name: None,
            api_key: "ya29_test".into(),
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
            credential_type: Some("oauth".into()),
            credential_data: Some(r#"{"access_token":"ya29_test"}"#.into()),
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
        let provider = provider("gemini", "https://generativelanguage.googleapis.com");
        assert!(!is_gemini_api_key(&account, &provider));
        assert!(is_gemini_oauth(&account, &provider));
    }

    fn provider(provider_type: &str, base_url: &str) -> Provider {
        Provider {
            id: "test".into(),
            name: "Test".into(),
            provider_type: provider_type.into(),
            base_url: base_url.into(),
            base_urls: None,
            protocol: "gemini".into(),
            protocols: Some("[\"gemini\"]".into()),
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
