use crate::db::accounts::Account;
use crate::services::keychain;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct CredentialPayload {
    pub api_key: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub session_token: Option<String>,
    pub account_id: Option<String>,
    pub expires_at: Option<String>,
    pub token_type: Option<String>,
    pub base_url: Option<String>,
    pub agent_identity: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
}

pub fn payload_for_account(account: &Account) -> Result<CredentialPayload, String> {
    if let Some(secret_ref) = account.secret_ref.as_deref() {
        let bytes = keychain::get_secret(secret_ref)?;
        return serde_json::from_slice::<CredentialPayload>(&bytes)
            .map_err(|error| format!("Credential vault payload is invalid: {}", error));
    }
    if let Some(value) = account.credential_data.as_deref() {
        return serde_json::from_str::<CredentialPayload>(value)
            .map_err(|error| format!("Stored credential payload is invalid: {}", error));
    }
    if !account.api_key.trim().is_empty() {
        return Ok(CredentialPayload {
            api_key: Some(account.api_key.clone()),
            ..CredentialPayload::default()
        });
    }
    Err("Account has no stored credential".into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthCredential {
    ApiKey(String),
    Bearer(String),
}

/// Return both the secret and the authentication scheme implied by the stored
/// credential type. Anthropic and Gemini support API-key and OAuth variants,
/// so callers must not blindly put every secret in the same header/query slot.
pub fn auth_credential(account: &Account) -> Result<AuthCredential, String> {
    match account.credential_type.as_deref().unwrap_or("api_key") {
        "api_key" | "upstream_key" | "gemini_api_key" => {
            api_key_secret(account).map(AuthCredential::ApiKey)
        }
        "oauth" | "token" | "codex_oauth" | "gemini_oauth" => {
            authorization_secret(account).map(AuthCredential::Bearer)
        }
        kind => Err(format!("Unsupported credential type '{}'", kind)),
    }
}

pub fn authorization_secret(account: &Account) -> Result<String, String> {
    let payload = payload_for_account(account)?;
    match account.credential_type.as_deref().unwrap_or("api_key") {
        "api_key" | "upstream_key" | "gemini_api_key" => payload
            .api_key
            .filter(|value| !value.trim().is_empty())
            .or_else(|| (!account.api_key.trim().is_empty()).then(|| account.api_key.clone()))
            .ok_or_else(|| "Account has no API key".to_string()),
        "oauth" | "token" | "codex_oauth" | "gemini_oauth" => payload
            .access_token
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                // Fallback: some providers store the token in session_token
                payload
                    .session_token
                    .filter(|value| !value.trim().is_empty())
            })
            .ok_or_else(|| "OAuth account has no access token".to_string()),
        kind => Err(format!("Unsupported credential type '{}'", kind)),
    }
}

pub fn api_key_secret(account: &Account) -> Result<String, String> {
    let payload = payload_for_account(account)?;
    payload
        .api_key
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            (account.credential_type.as_deref().unwrap_or("api_key") == "api_key"
                && !account.api_key.trim().is_empty())
            .then(|| account.api_key.clone())
        })
        .ok_or_else(|| "Account credential cannot be used as an API key".to_string())
}

/// Single availability predicate shared by routing, tray summaries and model selection.
/// A credential can be structurally routable while still being unavailable because
/// its persisted status or latest health result is terminal.
pub fn is_available_for_routing(account: &Account) -> bool {
    is_directly_routable(account)
        && !matches!(
            account.status.as_deref(),
            Some("disabled") | Some("error") | Some("exhausted") | Some("token_expired")
        )
        && !matches!(account.health_status.as_deref(), Some("error"))
}

pub fn is_directly_routable(account: &Account) -> bool {
    matches!(
        account.credential_type.as_deref().unwrap_or("api_key"),
        "api_key" | "upstream_key" | "oauth" | "token" | "codex_oauth"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(kind: &str, payload: &str) -> Account {
        Account {
            id: "acct-test".into(),
            provider_id: None,
            name: None,
            api_key: "legacy-key".into(),
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
            credential_type: Some(kind.into()),
            credential_data: Some(payload.into()),
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
        }
    }

    #[test]
    fn selects_api_key_for_api_key_credentials() {
        let account = account("api_key", r#"{"api_key":"sk-test"}"#);
        assert_eq!(
            auth_credential(&account),
            Ok(AuthCredential::ApiKey("sk-test".into()))
        );
    }

    #[test]
    fn selects_bearer_for_oauth_credentials() {
        let account = account("oauth", r#"{"access_token":"oauth-test"}"#);
        assert_eq!(
            auth_credential(&account),
            Ok(AuthCredential::Bearer("oauth-test".into()))
        );
    }
}
