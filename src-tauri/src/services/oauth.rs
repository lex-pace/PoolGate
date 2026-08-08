//! OAuth login flows for model resources.
//! Currently implements OpenAI Codex Authorization Code + PKCE.

use crate::db::accounts::{insert_account, Account};
use crate::db::providers::Provider;
use crate::services::credentials::CredentialPayload;
use crate::AppState;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use rand::{distributions::Alphanumeric, Rng};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use url::Url;
use uuid::Uuid;

const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_AUTH_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const CODEX_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const CODEX_SCOPE: &str = "openid profile email offline_access";

struct PendingFlow {
    state: String,
    code_verifier: String,
    email_hint: Option<String>,
    note: Option<String>,
    callback_rx: oneshot::Receiver<String>,
    callback_task: tokio::task::JoinHandle<()>,
}

static PENDING_FLOWS: LazyLock<Mutex<HashMap<String, PendingFlow>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, serde::Serialize)]
pub struct OAuthStartResult {
    pub login_id: String,
    pub authorization_url: String,
    pub redirect_uri: String,
    pub expires_in_seconds: u64,
}

#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<i64>,
}

pub async fn start_codex_oauth(
    email_hint: Option<String>,
    note: Option<String>,
) -> Result<OAuthStartResult, String> {
    let login_id = format!("oauth_{}", Uuid::new_v4().simple());
    let state = random_string(32);
    let code_verifier = random_string(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));
    let mut url = Url::parse(CODEX_AUTH_ENDPOINT).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_id", CODEX_CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", CODEX_REDIRECT_URI)
        .append_pair("scope", CODEX_SCOPE)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("originator", "codex_vscode");

    let listener = TcpListener::bind("127.0.0.1:1455")
        .await
        .map_err(|e| format!("无法监听 OAuth 回调端口 1455: {}", e))?;
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
                    let callback = format!("http://localhost:1455{}", path);
                    let _ = callback_tx.send(callback);
                }
            }
            let body = "<!doctype html><meta charset=\"utf-8\"><title>PoolGate OAuth</title><style>body{font:14px -apple-system,BlinkMacSystemFont,sans-serif;margin:48px;color:#1d1d1f}main{max-width:520px;margin:auto;padding:24px;border:1px solid #d2d2d7;border-radius:14px}</style><main><h2>授权已返回 PoolGate</h2><p>可以关闭此窗口，并回到 PoolGate 完成账号接入。</p></main>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    PENDING_FLOWS.lock().map_err(|e| e.to_string())?.insert(
        login_id.clone(),
        PendingFlow {
            state,
            code_verifier,
            email_hint: clean_optional(email_hint),
            note: clean_optional(note),
            callback_rx,
            callback_task,
        },
    );

    Ok(OAuthStartResult {
        login_id,
        authorization_url: url.to_string(),
        redirect_uri: CODEX_REDIRECT_URI.into(),
        expires_in_seconds: 300,
    })
}

pub async fn complete_codex_oauth(
    state: Arc<AppState>,
    login_id: &str,
    callback_url: Option<String>,
) -> Result<Account, String> {
    let flow = PENDING_FLOWS
        .lock()
        .map_err(|e| e.to_string())?
        .remove(login_id)
        .ok_or_else(|| "OAuth 会话不存在或已过期，请重新发起授权".to_string())?;
    let PendingFlow {
        state: oauth_state,
        code_verifier,
        email_hint,
        note,
        callback_rx,
        callback_task,
    } = flow;

    let callback = match clean_optional(callback_url) {
        Some(value) => {
            callback_task.abort();
            value
        }
        None => tokio::time::timeout(std::time::Duration::from_secs(300), callback_rx)
            .await
            .map_err(|_| {
                callback_task.abort();
                "等待 OAuth 回调超时，请重新授权或粘贴回调地址".to_string()
            })?
            .map_err(|_| "OAuth 回调监听已关闭".to_string())?,
    };
    let callback = Url::parse(&callback).map_err(|e| format!("无效的回调地址: {}", e))?;
    let query: HashMap<String, String> = callback.query_pairs().into_owned().collect();
    if let Some(error) = query.get("error") {
        return Err(format!(
            "OAuth 授权失败: {}",
            query.get("error_description").unwrap_or(error)
        ));
    }
    if query.get("state") != Some(&oauth_state) {
        return Err("OAuth state 校验失败，请重新授权".into());
    }
    let code = query
        .get("code")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "回调地址中缺少 authorization code".to_string())?;

    let token = exchange_code(code, &code_verifier).await?;
    persist_codex_account(&state, token, email_hint, note)
}

pub fn cancel_oauth(login_id: &str) -> Result<(), String> {
    if let Some(flow) = PENDING_FLOWS
        .lock()
        .map_err(|e| e.to_string())?
        .remove(login_id)
    {
        flow.callback_task.abort();
    }
    Ok(())
}

async fn exchange_code(code: &str, verifier: &str) -> Result<TokenResponse, String> {
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .post(CODEX_TOKEN_ENDPOINT)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CODEX_CLIENT_ID),
            ("code", code),
            ("redirect_uri", CODEX_REDIRECT_URI),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|e| format!("OAuth Token 交换请求失败: {}", e))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("OAuth Token 交换失败 ({}): {}", status, body));
    }
    serde_json::from_str(&body).map_err(|e| format!("OAuth Token 响应解析失败: {}", e))
}

fn persist_codex_account(
    state: &Arc<AppState>,
    token: TokenResponse,
    email_hint: Option<String>,
    note: Option<String>,
) -> Result<Account, String> {
    let claims = token
        .id_token
        .as_deref()
        .and_then(decode_jwt_claims)
        .unwrap_or(serde_json::Value::Null);
    let email = json_string(&claims, &["email"]).or(email_hint);
    let account_id = json_string(
        &claims,
        &["chatgpt_account_id", "account_id", "organization_id", "sub"],
    );
    let plan_type = json_string(&claims, &["chatgpt_plan_type", "plan_type"]);
    let expires_at = token
        .expires_in
        .map(|seconds| (Utc::now() + chrono::Duration::seconds(seconds)).to_rfc3339());
    let metadata = serde_json::json!({
        "client_id": CODEX_CLIENT_ID,
        "scope": CODEX_SCOPE,
        "token_endpoint": CODEX_TOKEN_ENDPOINT,
        "plan_type": plan_type,
        "note": note,
    });
    let payload = CredentialPayload {
        access_token: Some(token.access_token.clone()),
        refresh_token: token.refresh_token,
        id_token: token.id_token,
        account_id: account_id.clone(),
        expires_at: expires_at.clone(),
        token_type: token.token_type.or_else(|| Some("Bearer".into())),
        base_url: Some("https://chatgpt.com/backend-api/codex".into()),
        metadata: Some(metadata.clone()),
        ..CredentialPayload::default()
    };
    let fingerprint = hex::encode(Sha256::digest(
        format!("OpenAI Codex:{}", token.access_token).as_bytes(),
    ));

    if let Some(existing) = state
        .db
        .accounts
        .find_by_fingerprint(&state.db.conn, &fingerprint)?
    {
        return Ok(existing);
    }

    let provider_id = "provider_openai_codex".to_string();
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
                name: "OpenAI Codex".into(),
                provider_type: "codex".into(),
                base_url: "https://chatgpt.com/backend-api/codex".into(),
                base_urls: Some("{\"responses\":\"https://chatgpt.com/backend-api/codex\"}".into()),
                protocol: "openai".into(),
                protocols: Some("[\"responses\"]".into()),
                route_takeover: Some(1),
                api_keys: None,
                models: Some("[\"codex\",\"gpt-5-codex\"]".into()),
                proxy_url: None,
                custom_headers: None,
                timeout_ms: Some(30000),
                priority: Some(0),
                enabled: Some(true),
                created_at: None,
                auth_mode: Some("oauth_pkce".into()),
                oauth_config: Some("{\"authorize_url\":\"https://auth.openai.com/oauth/authorize\",\"token_url\":\"https://auth.openai.com/oauth/token\",\"client_id\":\"app_EMoamEEZ73f0CkXaXp7hrann\",\"scope\":\"openid profile email offline_access\",\"redirect_port\":1455}".into()),
            },
        )?;
    }

    let account = Account {
        id: format!("acct_{}", Uuid::new_v4().simple()),
        provider_id: Some(provider_id),
        name: note
            .or_else(|| email.clone())
            .or_else(|| Some("OpenAI Codex account".into())),
        api_key: token.access_token,
        models: Some("[\"codex\",\"gpt-5-codex\"]".into()),
        quota_limit: None,
        quota_used: None,
        status: Some("active".into()),
        health_status: Some("unchecked".into()),
        health_code: None,
        health_msg: Some("Codex Responses 专用适配器已启用；仅限本机本人账号私有使用".into()),
        health_latency: None,
        health_check_at: None,
        priority: Some(0),
        tags: Some("[\"official\",\"codex\",\"oauth\"]".into()),
        last_used_at: None,
        created_at: None,
        credential_type: Some("codex_oauth".into()),
        credential_data: Some(serde_json::to_string(&payload).map_err(|e| e.to_string())?),
        source_format: Some("oauth".into()),
        external_account_id: account_id,
        email,
        expires_at,
        metadata: Some(metadata.to_string()),
        credential_fingerprint: Some(fingerprint),
        protocols: Some("[\"responses\"]".into()),
        route_takeover: Some(1),
        plan_type,
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
    state.db.accounts.update_usage(
        &state.db.conn,
        &account.id,
        "codex",
        account.plan_type.as_deref(),
        "[]",
        None,
    )?;

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
        "Codex",
        "unified",
    )?;

    Ok(account)
}

fn decode_jwt_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn json_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| find_json_string(value, key))
}

fn find_json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    match value {
        serde_json::Value::Object(object) => object
            .get(key)
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                object
                    .values()
                    .find_map(|value| find_json_string(value, key))
            }),
        serde_json::Value::Array(values) => {
            values.iter().find_map(|value| find_json_string(value, key))
        }
        _ => None,
    }
}

fn random_string(length: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

pub(crate) fn clean_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_jwt_payload() {
        let payload = URL_SAFE_NO_PAD.encode(r#"{"email":"test@example.com"}"#);
        let claims = decode_jwt_claims(&format!("header.{}.sig", payload)).unwrap();
        assert_eq!(claims["email"], "test@example.com");
    }
}
