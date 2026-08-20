//! OpenAI Codex ChatGPT OAuth request adapter.
//!
//! ChatGPT subscription credentials are not OpenAI Platform API keys. They are
//! valid only against the Codex backend and require the ChatGPT account context.

use crate::db::accounts::Account;
use crate::db::providers::Provider;
use crate::services::credentials::{authorization_secret, payload_for_account};
use crate::services::token_refresh::{oauth_from_payload, refresh_oauth_token};
use crate::AppState;
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;

pub const CODEX_ORIGINATOR: &str = crate::services::client_profiles::CODEX_ORIGINATOR;

/// Codex model-list endpoint, versioned with the same client version the
/// User-Agent presents (an inconsistent pair is a fingerprint signal).
pub fn codex_models_url() -> String {
    format!(
        "https://chatgpt.com/backend-api/codex/models?client_version={}",
        crate::services::client_profiles::CODEX_CLI_VERSION
    )
}

static REFRESH_LOCKS: LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
pub struct CodexRequestContext {
    pub access_token: String,
    pub account_id: String,
}

pub fn is_codex_oauth(account: &Account, provider: &Provider) -> bool {
    let oauth_credential = matches!(
        account.credential_type.as_deref().unwrap_or("api_key"),
        "codex_oauth" | "oauth" | "token"
    );
    let codex_backend = account.source_format.as_deref() == Some("codex_auth")
        || provider.base_url.contains("chatgpt.com/backend-api/codex")
        || provider.provider_type.eq_ignore_ascii_case("codex");
    oauth_credential && codex_backend
}

pub fn request_context(account: &Account) -> Result<CodexRequestContext, String> {
    let payload = payload_for_account(account)?;
    let account_id = account
        .external_account_id
        .clone()
        .or(payload.account_id)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Codex OAuth account is missing ChatGPT Account ID".to_string())?;
    Ok(CodexRequestContext {
        access_token: authorization_secret(account)?,
        account_id,
    })
}

pub fn prepare_responses_body(body: &Value) -> Result<Value, String> {
    let mut value = body.clone();
    let object = value
        .as_object_mut()
        .ok_or_else(|| "Codex Responses request body must be a JSON object".to_string())?;
    if object.get("stream").and_then(Value::as_bool) != Some(true) {
        return Err("Codex OAuth routing currently requires stream=true".into());
    }
    // Keep Codex traffic stateless and prevent gateway prompts from becoming
    // stored ChatGPT conversations. Do not override the caller's model/input.
    object.insert("store".into(), Value::Bool(false));
    Ok(value)
}

pub fn apply_responses_headers(
    request: RequestBuilder,
    context: &CodexRequestContext,
) -> RequestBuilder {
    crate::services::client_profiles::apply_codex_profile(
        request
            .bearer_auth(&context.access_token)
            .header("ChatGPT-Account-Id", &context.account_id)
            .header("originator", CODEX_ORIGINATOR)
            .header("Accept", "text/event-stream"),
    )
}

pub fn apply_json_headers(
    request: RequestBuilder,
    context: &CodexRequestContext,
) -> RequestBuilder {
    crate::services::client_profiles::apply_codex_profile(
        request
            .bearer_auth(&context.access_token)
            .header("ChatGPT-Account-Id", &context.account_id)
            .header("originator", CODEX_ORIGINATOR)
            .header("Accept", "application/json"),
    )
}

/// Refresh a Codex OAuth token after a real request returned 401.
///
/// Refreshes are serialized per account. After acquiring the lock the account is
/// reloaded; if another request already rotated the failed token, that fresh
/// account is returned without issuing another refresh request.
pub async fn refresh_after_unauthorized(
    state: &AppState,
    account_id: &str,
    failed_access_token: &str,
) -> Result<Account, String> {
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
        .ok_or_else(|| "Codex OAuth account no longer exists".to_string())?;
    let payload = payload_for_account(&account)?;
    if payload
        .access_token
        .as_deref()
        .is_some_and(|token| token != failed_access_token)
    {
        return Ok(account);
    }
    if payload
        .refresh_token
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Err("Codex OAuth account has no refresh_token".into());
    }

    let provider_id = account
        .provider_id
        .as_deref()
        .ok_or_else(|| "Codex OAuth account has no provider".to_string())?;
    let provider = state
        .db
        .providers
        .get_by_id(&state.db.conn, provider_id)?
        .ok_or_else(|| "Codex OAuth provider no longer exists".to_string())?;
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;
    let refreshed = refresh_oauth_token(
        &client,
        &provider.provider_type,
        &provider.protocol,
        &oauth_from_payload(&payload),
    )
    .await?;

    let mut new_payload = payload;
    new_payload.access_token = refreshed.access_token;
    new_payload.refresh_token = refreshed.refresh_token.or(new_payload.refresh_token);
    new_payload.expires_at = refreshed.expires_at;
    new_payload.token_type = refreshed.token_type.or(new_payload.token_type);
    let new_access_token = new_payload
        .access_token
        .clone()
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| "Codex token refresh returned no access_token".to_string())?;

    let mut updated = account;
    updated.api_key = new_access_token;
    updated.expires_at = new_payload.expires_at.clone();
    updated.credential_data =
        Some(serde_json::to_string(&new_payload).map_err(|error| error.to_string())?);
    state.db.accounts.update(&state.db.conn, &updated)?;
    state
        .db
        .accounts
        .mark_token_refreshed(&state.db.conn, account_id)?;
    state.db.accounts.update_health(
        &state.db.conn,
        account_id,
        "unchecked",
        0,
        "Token refreshed after 401; awaiting Codex connectivity result",
        0,
    )?;
    // A successful refresh proves the credential usable again: recover
    // token_expired / error status without touching manual states.
    state
        .db
        .accounts
        .recover_status(&state.db.conn, account_id, updated.status.as_deref())?;

    state
        .db
        .accounts
        .get_by_id(&state.db.conn, account_id)?
        .ok_or_else(|| "Codex OAuth account no longer exists".to_string())
}

#[cfg(test)]
mod tests {
    use super::prepare_responses_body;
    use serde_json::json;

    #[test]
    fn codex_body_requires_streaming_and_disables_storage() {
        let prepared = prepare_responses_body(&json!({
            "model": "gpt-5-codex",
            "input": "hello",
            "stream": true,
            "store": true
        }))
        .unwrap();
        assert_eq!(prepared["stream"], true);
        assert_eq!(prepared["store"], false);
    }

    #[test]
    fn codex_body_rejects_non_streaming_requests() {
        let error = prepare_responses_body(&json!({
            "model": "gpt-5-codex",
            "input": "hello",
            "stream": false
        }))
        .unwrap_err();
        assert!(error.contains("stream=true"));
    }
}
