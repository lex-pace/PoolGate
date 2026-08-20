//! OAuth token refresh service.
//!
//! Runs a background loop every 60 seconds that finds routable OAuth/token
//! accounts whose access token is expiring (within the next 5 minutes) and
//! attempts to refresh them using the provider's OAuth endpoint.
//!
//! Credentials live in `Account.credential_data` as a JSON-encoded
//! [`CredentialPayload`]. Client id / secret / scope, when required by the
//! provider, are read from `credential_data.metadata` (keys `client_id`,
//! `client_secret`, `scope`). Refreshed tokens are written back to
//! `credential_data`, and the `api_key` / `expires_at` columns are kept in
//! sync so the proxy and UI observe the new state immediately.

use crate::services::credentials::CredentialPayload;
use crate::AppState;
use chrono::{DateTime, Utc};
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::{interval_at, Duration, Instant};

/// OpenAI Codex ChatGPT 订阅 OAuth（auth.openai.com 公共客户端，与 Cockpit-tools
/// 官方实现一致：刷新时使用 JSON body + client_id，无 client_secret）。
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";

/// Parsed OAuth secrets used to talk to a token endpoint.
#[derive(Debug, Clone, Default)]
pub(crate) struct OAuthCredentials {
    pub(crate) access_token: Option<String>,
    pub(crate) refresh_token: Option<String>,
    pub(crate) expires_at: Option<String>,
    pub(crate) token_type: Option<String>,
    pub(crate) client_id: Option<String>,
    pub(crate) client_secret: Option<String>,
}

/// Read a string field from a JSON object by any of the given keys.
fn json_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
    })
}

/// Build an [`OAuthCredentials`] from a stored [`CredentialPayload`].
///
/// Client id / secret / scope are optional and, when present, come from the
/// payload's `metadata` object.
pub(crate) fn oauth_from_payload(payload: &CredentialPayload) -> OAuthCredentials {
    let metadata = payload.metadata.clone().unwrap_or(serde_json::Value::Null);
    let client_id = json_string(&metadata, &["client_id", "clientId"]).or_else(|| {
        payload
            .agent_identity
            .as_ref()
            .map(|_| "app_EMoamEEZ73f0CkXaXp7hrann".to_string())
    });
    OAuthCredentials {
        access_token: payload.access_token.clone(),
        refresh_token: payload.refresh_token.clone(),
        expires_at: payload.expires_at.clone(),
        token_type: payload.token_type.clone(),
        client_id,
        client_secret: json_string(&metadata, &["client_secret", "clientSecret"]),
    }
}

/// Start the background token refresh loop.
///
/// Shares the application's [`AppState`] (and therefore its single database
/// connection) so it never opens a competing SQLite handle. Every 60 seconds it:
/// 1. Lists all accounts.
/// 2. Skips any account that is not a routable `oauth` / `token` credential
///    with a usable refresh token and a parseable expiry.
/// 3. Refreshes tokens that expire within the next 5 minutes (or already have).
/// 4. On success, writes the new credentials back to `credential_data`,
///    `api_key`, and `expires_at`, and resets health.
/// 5. On failure, marks the account `token_expired` and records the error.
pub async fn refresh_loop(state: Arc<AppState>) {
    tracing::info!("Token refresh loop started");

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    // `tokio::time::interval` ticks immediately. Defer the first sweep so
    // merely opening PoolGate never walks every OAuth keychain entry before
    // the user can interact with the app.
    let mut ticker = interval_at(
        Instant::now() + Duration::from_secs(60),
        Duration::from_secs(60),
    );
    let db = &state.db;

    loop {
        ticker.tick().await;

        // Token maintenance is useful only while the local gateway is active.
        // Keeping it dormant while PoolGate is merely open guarantees that the
        // background loop cannot trigger credential-vault authorization dialogs.
        let proxy_running = state
            .proxy
            .lock()
            .map(|proxy| proxy.is_some())
            .unwrap_or(false);
        if !proxy_running {
            continue;
        }
        tracing::debug!("Token refresh: checking expiring tokens");

        let accounts = match db.accounts.list_all(&db.conn) {
            Ok(accs) => accs,
            Err(e) => {
                tracing::warn!("Token refresh: failed to list accounts: {}", e);
                continue;
            }
        };

        let now: DateTime<Utc> = Utc::now();
        let refresh_window = now + chrono::Duration::minutes(5);

        for account in &accounts {
            // Only manage refreshable OAuth/token credentials.
            match account.credential_type.as_deref() {
                Some("oauth") | Some("token") | Some("codex_oauth") => {}
                _ => continue,
            }

            // `accounts.expires_at` is intentionally duplicated outside the
            // encrypted payload. Use it as the cheap prefilter so healthy OAuth
            // accounts do not trigger macOS Keychain access every minute.
            let column_expiry = account.expires_at.as_deref().and_then(parse_expiry);
            if !should_load_secret_for_refresh(account.expires_at.as_deref(), refresh_window) {
                continue;
            }

            let payload = match crate::services::credentials::payload_for_account(account) {
                Ok(payload) => payload,
                Err(error) => {
                    tracing::warn!(
                        "Token refresh: cannot load credential for '{}': {}",
                        account.id,
                        error
                    );
                    continue;
                }
            };
            let creds = oauth_from_payload(&payload);

            match &creds.refresh_token {
                Some(rt) if !rt.trim().is_empty() => {}
                _ => continue, // No refresh token available.
            }

            // The encrypted payload remains authoritative when the duplicated
            // column is absent or stale.
            let expires_at = match creds
                .expires_at
                .as_deref()
                .and_then(parse_expiry)
                .or(column_expiry)
            {
                Some(dt) => dt,
                None => continue, // Unknown expiry — don't guess.
            };

            // Only refresh if expiring within 5 min or already expired.
            if expires_at > refresh_window {
                continue;
            }

            tracing::info!(
                "Token refresh: refreshing token for account '{}' (expires at {})",
                account.name.as_deref().unwrap_or(&account.id),
                expires_at
            );

            let provider_id = match &account.provider_id {
                Some(pid) => pid.clone(),
                None => {
                    tracing::warn!("Token refresh: account '{}' has no provider", account.id);
                    continue;
                }
            };

            let provider = match db.providers.get_by_id(&db.conn, &provider_id) {
                Ok(Some(p)) => p,
                _ => {
                    tracing::warn!("Token refresh: provider '{}' not found", provider_id);
                    continue;
                }
            };

            match refresh_oauth_token(&client, &provider.provider_type, &provider.protocol, &creds)
                .await
            {
                Ok(new_creds) => {
                    // Merge refreshed secrets back into the stored payload.
                    let mut new_payload = payload.clone();
                    new_payload.access_token = new_creds.access_token.clone();
                    new_payload.refresh_token = new_creds
                        .refresh_token
                        .clone()
                        .or_else(|| payload.refresh_token.clone());
                    new_payload.expires_at = new_creds.expires_at.clone();
                    new_payload.token_type = new_creds
                        .token_type
                        .clone()
                        .or_else(|| payload.token_type.clone());

                    let credential_data = serde_json::to_string(&new_payload)
                        .unwrap_or_else(|_| account.credential_data.clone().unwrap_or_default());

                    let mut updated = account.clone();
                    updated.credential_data = Some(credential_data);
                    // Keep api_key column aligned with the routable secret.
                    if let Some(token) = new_payload.access_token.clone() {
                        updated.api_key = token;
                    }
                    updated.expires_at = new_payload.expires_at.clone();

                    if let Err(e) = db.accounts.update(&db.conn, &updated) {
                        tracing::error!(
                            "Token refresh: failed to update account '{}': {}",
                            account.id,
                            e
                        );
                    } else {
                        let (health_status, health_code, health_message) =
                            if account.credential_type.as_deref() == Some("codex_oauth") {
                                (
                                    "unchecked",
                                    0,
                                    "Token refreshed; awaiting Codex connectivity result",
                                )
                            } else {
                                ("healthy", 200, "Token refreshed")
                            };
                        db.accounts
                            .update_health(
                                &db.conn,
                                &account.id,
                                health_status,
                                health_code,
                                health_message,
                                0,
                            )
                            .ok();
                        if account.credential_type.as_deref() == Some("codex_oauth") {
                            // Codex 刷新成功：status 从 token_expired/error 恢复
                            // active（disabled/exhausted 不动），health 写
                            // unchecked 等待真实连通性结果。
                            db.accounts
                                .recover_status(&db.conn, &account.id, account.status.as_deref())
                                .ok();
                        } else {
                            db.accounts
                                .update_status(&db.conn, &account.id, "active")
                                .ok();
                        }
                        db.accounts.mark_token_refreshed(&db.conn, &account.id).ok();
                        tracing::info!(
                            "Token refresh: successfully refreshed token for '{}'",
                            account.id
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Token refresh: failed to refresh '{}': {}. Marking as token_expired",
                        account.id,
                        e
                    );
                    db.accounts
                        .update_status(&db.conn, &account.id, "token_expired")
                        .ok();
                    db.accounts
                        .update_health(
                            &db.conn,
                            &account.id,
                            "error",
                            401,
                            &format!("Token refresh failed: {}", e),
                            0,
                        )
                        .ok();
                }
            }
        }
    }
}

/// Decide whether an OAuth account may need its encrypted payload loaded.
/// Missing or malformed duplicate metadata falls back to the secure payload;
/// a known expiry outside the refresh window avoids Keychain access entirely.
fn should_load_secret_for_refresh(expires_at: Option<&str>, refresh_window: DateTime<Utc>) -> bool {
    // Background maintenance must never open the credential vault merely to
    // discover whether a token needs work. Missing or malformed non-secret
    // expiry metadata is handled by explicit refresh or by the real request
    // path, both of which are user-initiated and may legitimately access the
    // credential. This keeps ordinary application launch completely silent.
    expires_at
        .and_then(parse_expiry)
        .is_some_and(|expiry| expiry <= refresh_window)
}

/// Parse an expiry timestamp. Accepts RFC 3339, or a Unix epoch (seconds or
/// milliseconds) encoded as a numeric string.
fn parse_expiry(raw: &str) -> Option<DateTime<Utc>> {
    let trimmed = raw.trim();
    if let Ok(dt) = trimmed.parse::<DateTime<Utc>>() {
        return Some(dt);
    }
    if let Ok(num) = trimmed.parse::<i64>() {
        // Heuristic: values past ~year 2286 in seconds are actually ms.
        let (secs, nanos) = if num > 100_000_000_000 {
            (num / 1000, ((num % 1000) * 1_000_000) as u32)
        } else {
            (num, 0)
        };
        return DateTime::<Utc>::from_timestamp(secs, nanos);
    }
    None
}

/// Attempt to refresh an OAuth token using the appropriate endpoint for the
/// given provider type / protocol.
pub(crate) async fn refresh_oauth_token(
    client: &Client,
    provider_type: &str,
    protocol: &str,
    creds: &OAuthCredentials,
) -> Result<OAuthCredentials, String> {
    let key = if provider_type.trim().is_empty() {
        protocol.to_lowercase()
    } else {
        provider_type.to_lowercase()
    };
    // Map known provider type aliases to canonical names. Antigravity stays
    // distinct from plain Gemini because its refresh tokens are issued against
    // the Antigravity (Cloud Code) OAuth client, not the Gemini CLI client.
    let canonical = match key.as_str() {
        "anthropic" | "claude" => "anthropic",
        "gemini" | "google" | "googleantigravity" => "google",
        "antigravity" => "antigravity",
        // Codex 走专用刷新：auth.openai.com 的 refresh_token grant 只接受 JSON
        // body，不能复用 OpenAI Platform 的 form 编码路径。
        "codex" => "codex",
        other => other,
    };
    match canonical {
        "anthropic" => refresh_anthropic(client, creds).await,
        "google" => refresh_google(client, creds, None, None).await,
        "antigravity" => {
            refresh_google(
                client,
                creds,
                Some(crate::services::antigravity_adapter::ANTIGRAVITY_CLIENT_ID.as_str()),
                Some(crate::services::antigravity_adapter::ANTIGRAVITY_CLIENT_SECRET.as_str()),
            )
            .await
        }
        "codex" => refresh_codex(client, creds).await,
        "openai" => refresh_generic(client, "openai", creds).await,
        other => refresh_generic(client, other, creds).await,
    }
}

/// Extract an OAuth error code from a token endpoint error body.
///
/// OpenAI's auth.openai.com returns error bodies in a few shapes:
/// `{"error":"refresh_token_reused"}`, `{"error":{"code":"..."}}`, or a bare
/// top-level `{"code":"..."}`. The code is appended to refresh failures so the
/// UI can tell "token needs re-login" (reused/expired/invalidated/invalid_grant)
/// apart from transient failures.
fn extract_oauth_error_code(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    value
        .get("error")
        .and_then(|item| item.as_str())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("error")
                .and_then(|item| item.get("code"))
                .and_then(|item| item.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            value
                .get("code")
                .and_then(|item| item.as_str())
                .map(str::to_string)
        })
}

/// OpenAI Codex ChatGPT subscription OAuth token refresh.
///
/// POST to `https://auth.openai.com/oauth/token` with a **JSON** body
/// `{client_id, grant_type: "refresh_token", refresh_token}`. The login
/// (authorization_code) exchange accepts form encoding, but the refresh grant on
/// this endpoint requires JSON — matching the official Codex CLI / Cockpit-tools
/// (`refresh_access_token_with_fallback`) implementation. The client is a
/// public client: no client_secret and no PKCE parameters on refresh.
async fn refresh_codex(
    client: &Client,
    creds: &OAuthCredentials,
) -> Result<OAuthCredentials, String> {
    let client_id = creds
        .client_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(CODEX_CLIENT_ID);
    let response = client
        .post(CODEX_TOKEN_ENDPOINT)
        .json(&serde_json::json!({
            "client_id": client_id,
            "grant_type": "refresh_token",
            "refresh_token": creds.refresh_token.as_deref().unwrap_or(""),
        }))
        .send()
        .await
        .map_err(|e| format!("Codex OAuth 刷新请求失败: {}", e))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let error_code = extract_oauth_error_code(&body);
        let mut message = format!("Codex OAuth 刷新失败 (HTTP {})", status);
        if let Some(code) = error_code {
            message.push_str(&format!(", error_code={}", code));
        }
        message.push_str(&format!(": {}", truncate_body(&body, 320)));
        return Err(message);
    }

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Bad JSON: {}", e))?;

    let new_expires_at = json["expires_in"]
        .as_i64()
        .map(|secs| (Utc::now() + chrono::Duration::seconds(secs)).to_rfc3339());

    Ok(OAuthCredentials {
        access_token: json["access_token"].as_str().map(String::from),
        refresh_token: json["refresh_token"]
            .as_str()
            .map(String::from)
            .or_else(|| creds.refresh_token.clone()),
        expires_at: new_expires_at.or_else(|| creds.expires_at.clone()),
        token_type: json["token_type"].as_str().map(String::from),
        client_id: Some(client_id.to_string()),
        client_secret: creds.client_secret.clone(),
    })
}

fn truncate_body(body: &str, max: usize) -> String {
    body.chars().take(max).collect()
}

/// Anthropic OAuth token refresh.
///
/// POST to `https://api.anthropic.com/v1/oauth/token` with grant_type=refresh_token.
async fn refresh_anthropic(
    client: &Client,
    creds: &OAuthCredentials,
) -> Result<OAuthCredentials, String> {
    let url = "https://api.anthropic.com/v1/oauth/token";
    let mut params = HashMap::new();
    params.insert("grant_type", "refresh_token");
    params.insert(
        "refresh_token",
        creds.refresh_token.as_deref().unwrap_or(""),
    );
    if let Some(ref cid) = creds.client_id {
        params.insert("client_id", cid);
    }
    if let Some(ref cs) = creds.client_secret {
        params.insert("client_secret", cs);
    }

    let resp = client
        .post(url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Anthropic OAuth request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Anthropic OAuth returned {}: {}", status, body));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| format!("Bad JSON: {}", e))?;

    let new_expires_at = json["expires_in"]
        .as_i64()
        .map(|secs| (Utc::now() + chrono::Duration::seconds(secs)).to_rfc3339());

    Ok(OAuthCredentials {
        access_token: json["access_token"].as_str().map(String::from),
        refresh_token: json["refresh_token"]
            .as_str()
            .map(String::from)
            .or_else(|| creds.refresh_token.clone()),
        expires_at: new_expires_at.or_else(|| creds.expires_at.clone()),
        token_type: json["token_type"].as_str().map(String::from),
        client_id: creds.client_id.clone(),
        client_secret: creds.client_secret.clone(),
    })
}

/// Google OAuth token refresh.
///
/// POST to `https://oauth2.googleapis.com/token` with grant_type=refresh_token.
/// `default_client_id` / `default_client_secret` override the credential-level
/// values when the account was imported without OAuth client metadata (e.g.
/// Cockpit-imported Antigravity accounts).
async fn refresh_google(
    client: &Client,
    creds: &OAuthCredentials,
    default_client_id: Option<&str>,
    default_client_secret: Option<&str>,
) -> Result<OAuthCredentials, String> {
    let url = "https://oauth2.googleapis.com/token";
    let mut params = HashMap::new();
    params.insert("grant_type", "refresh_token");
    params.insert(
        "refresh_token",
        creds.refresh_token.as_deref().unwrap_or(""),
    );
    // Use the credential's client_id when present; otherwise the provider
    // default (Gemini CLI client for plain Gemini, Antigravity client otherwise).
    let client_id = creds
        .client_id
        .as_deref()
        .or(default_client_id)
        .unwrap_or("764086051850-6qr4p6gpi6hn506pt8ejuq83di341hur.apps.googleusercontent.com");
    params.insert("client_id", client_id);
    let client_secret = creds.client_secret.as_deref().or(default_client_secret);
    if let Some(cs) = client_secret {
        params.insert("client_secret", cs);
    }

    let resp = client
        .post(url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Google OAuth request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Google OAuth returned {}: {}", status, body));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| format!("Bad JSON: {}", e))?;

    let new_expires_at = json["expires_in"]
        .as_i64()
        .map(|secs| (Utc::now() + chrono::Duration::seconds(secs)).to_rfc3339());

    Ok(OAuthCredentials {
        access_token: json["access_token"].as_str().map(String::from),
        refresh_token: json["refresh_token"]
            .as_str()
            .map(String::from)
            .or_else(|| creds.refresh_token.clone()),
        expires_at: new_expires_at.or_else(|| creds.expires_at.clone()),
        token_type: json["token_type"].as_str().map(String::from),
        client_id: creds.client_id.clone(),
        client_secret: creds.client_secret.clone(),
    })
}

/// Generic OAuth token refresh.
///
/// Attempts to discover the token endpoint via provider base_url or falls back
/// to a well-known OAuth endpoint.
async fn refresh_generic(
    client: &Client,
    provider_type: &str,
    creds: &OAuthCredentials,
) -> Result<OAuthCredentials, String> {
    // Default OAuth endpoint guess based on provider type
    let url: String = match provider_type.to_lowercase().as_str() {
        "openai" => "https://auth.openai.com/oauth/token".into(),
        "azure" => format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            creds.client_id.as_deref().unwrap_or("common")
        ),
        _ => {
            return Err(format!(
                "No known OAuth endpoint for provider type '{}'",
                provider_type
            ))
        }
    };

    let mut params = HashMap::new();
    params.insert("grant_type", "refresh_token");
    params.insert(
        "refresh_token",
        creds.refresh_token.as_deref().unwrap_or(""),
    );
    if let Some(ref cid) = creds.client_id {
        params.insert("client_id", cid);
    }
    if let Some(ref cs) = creds.client_secret {
        params.insert("client_secret", cs);
    }

    let resp = client
        .post(&url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("OAuth request to {} failed: {}", url, e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("OAuth returned {} from {}: {}", status, url, body));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad JSON from {}: {}", url, e))?;

    let new_expires_at = json["expires_in"]
        .as_i64()
        .map(|secs| (Utc::now() + chrono::Duration::seconds(secs)).to_rfc3339());

    Ok(OAuthCredentials {
        access_token: json["access_token"].as_str().map(String::from),
        refresh_token: json["refresh_token"]
            .as_str()
            .map(String::from)
            .or_else(|| creds.refresh_token.clone()),
        expires_at: new_expires_at.or_else(|| creds.expires_at.clone()),
        token_type: json["token_type"].as_str().map(String::from),
        client_id: creds.client_id.clone(),
        client_secret: creds.client_secret.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::{extract_oauth_error_code, parse_expiry, should_load_secret_for_refresh};
    use chrono::{Duration, Utc};

    #[test]
    fn parses_rfc3339_and_epoch_expiry() {
        assert!(parse_expiry("2026-08-01T15:00:00Z").is_some());
        assert!(parse_expiry("1785596400").is_some());
        assert!(parse_expiry("1785596400000").is_some());
        assert!(parse_expiry("not-a-time").is_none());
    }

    #[test]
    fn only_loads_keychain_for_known_expiring_token() {
        let now = Utc::now();
        let refresh_window = now + Duration::minutes(5);
        let fresh = (now + Duration::hours(1)).to_rfc3339();
        let expiring = (now + Duration::minutes(2)).to_rfc3339();

        assert!(!should_load_secret_for_refresh(
            Some(&fresh),
            refresh_window
        ));
        assert!(should_load_secret_for_refresh(
            Some(&expiring),
            refresh_window
        ));
        assert!(!should_load_secret_for_refresh(None, refresh_window));
        assert!(!should_load_secret_for_refresh(
            Some("invalid"),
            refresh_window
        ));
    }

    #[test]
    fn extracts_oauth_error_code_from_all_shapes() {
        // Auth0-style flat string error (the common OpenAI case).
        assert_eq!(
            extract_oauth_error_code(r#"{"error":"refresh_token_reused"}"#),
            Some("refresh_token_reused".into())
        );
        // Nested `error.code` shape.
        assert_eq!(
            extract_oauth_error_code(r#"{"error":{"code":"invalid_grant"}}"#),
            Some("invalid_grant".into())
        );
        // Bare top-level code.
        assert_eq!(
            extract_oauth_error_code(r#"{"code":"token_invalidated"}"#),
            Some("token_invalidated".into())
        );
        // Non-JSON / empty bodies carry no code.
        assert_eq!(extract_oauth_error_code("not json"), None);
        assert_eq!(extract_oauth_error_code(""), None);
    }
}
