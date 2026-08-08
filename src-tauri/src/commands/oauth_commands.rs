use crate::services::oauth::OAuthStartResult;
use crate::AppState;
use std::sync::Arc;
use tauri::State;
use tauri_plugin_shell::ShellExt;

#[tauri::command]
pub async fn start_oauth_login(
    app: tauri::AppHandle,
    provider_id: String,
    email_hint: Option<String>,
    note: Option<String>,
) -> Result<OAuthStartResult, String> {
    if provider_id != "openai" && provider_id != "codex" {
        return Err(format!("{} 暂未配置真实 OAuth 适配器", provider_id));
    }
    let result = crate::services::oauth::start_codex_oauth(email_hint, note).await?;
    #[allow(deprecated)]
    app.shell()
        .open(result.authorization_url.clone(), None)
        .map_err(|e| format!("无法打开系统浏览器: {}", e))?;
    Ok(result)
}

#[tauri::command]
pub async fn complete_oauth_login(
    state: State<'_, Arc<AppState>>,
    login_id: String,
    callback_url: Option<String>,
) -> Result<crate::db::accounts::Account, String> {
    crate::services::oauth::complete_codex_oauth(state.inner().clone(), &login_id, callback_url)
        .await
}

#[tauri::command]
pub fn cancel_oauth_login(login_id: String) -> Result<(), String> {
    crate::services::oauth::cancel_oauth(&login_id)
}

/// Validate a GitHub PAT and exchange it for a Copilot token.
///
/// Returns the Copilot token expiry time on success. The PAT is NOT stored
/// until the caller confirms (via `execute_import` or similar).
#[tauri::command]
pub async fn copilot_pat_validate(github_pat: String) -> Result<CopilotValidation, String> {
    use crate::services::copilot_adapter;

    // Validate the PAT by exchanging it for a Copilot token.
    let copilot_token = copilot_adapter::exchange_github_token_for_copilot(&github_pat).await?;

    // The token is valid if we get here. Calculate expiry.
    let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(25 * 60 - 30))
        .to_rfc3339();

    Ok(CopilotValidation {
        valid: true,
        copilot_token_preview: copilot_token[..8.min(copilot_token.len())].to_string(),
        expires_at,
        message: "GitHub PAT 验证成功，已获取 Copilot token".into(),
    })
}

#[derive(serde::Serialize)]
pub struct CopilotValidation {
    pub valid: bool,
    pub copilot_token_preview: String,
    pub expires_at: String,
    pub message: String,
}

/// Validate a Google AI Studio API Key.
///
/// Performs a lightweight check against the Gemini models endpoint.
#[tauri::command]
pub async fn gemini_api_key_validate(api_key: String) -> Result<GeminiValidation, String> {
    use crate::services::gemini_adapter;

    // Create a temporary account-like structure for health check.
    let account = crate::db::accounts::Account {
        id: "temp_validation".into(),
        provider_id: None,
        name: None,
        api_key: api_key.clone(),
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
        credential_data: Some(format!("{{\"api_key\":\"{}\"}}", api_key)),
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

    let result = gemini_adapter::check_gemini_health(&account).await;
    match result {
        crate::services::health_check::HealthResult::Passed { latency_ms } => Ok(GeminiValidation {
            valid: true,
            message: format!("API Key 验证成功，延迟 {}ms", latency_ms),
        }),
        crate::services::health_check::HealthResult::Failed { code, body } => Err(format!(
            "API Key 验证失败 (HTTP {}): {}",
            code,
            &body[..200.min(body.len())]
        )),
        crate::services::health_check::HealthResult::Timeout => {
            Err("API Key 验证超时，请检查网络".into())
        }
        crate::services::health_check::HealthResult::Error(msg) => Err(format!("验证出错: {}", msg)),
    }
}

#[derive(serde::Serialize)]
pub struct GeminiValidation {
    pub valid: bool,
    pub message: String,
}

/// Start a GitHub Copilot OAuth device flow.
///
/// Returns the user code, verification URI, and polling interval.
/// The user should visit the verification URI and enter the user code.
#[tauri::command]
pub async fn start_copilot_device_flow() -> Result<CopilotDeviceCode, String> {
    let result = crate::services::copilot_adapter::start_device_flow().await?;
    Ok(CopilotDeviceCode {
        device_code: result.device_code,
        user_code: result.user_code,
        verification_uri: result.verification_uri,
        expires_in: result.expires_in,
        interval: result.interval,
    })
}

#[derive(serde::Serialize)]
pub struct CopilotDeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// Poll GitHub for the device flow authorization status.
///
/// Returns the access token if authorized, or an error if still pending or failed.
#[tauri::command]
pub async fn poll_copilot_device_token(
    device_code: String,
    interval_ms: u64,
) -> Result<CopilotDevicePollResult, String> {
    match crate::services::copilot_adapter::poll_device_token(&device_code, interval_ms).await? {
        Some(access_token) => Ok(CopilotDevicePollResult {
            authorized: true,
            access_token: Some(access_token),
            message: "授权成功".into(),
        }),
        None => Ok(CopilotDevicePollResult {
            authorized: false,
            access_token: None,
            message: "等待用户授权...".into(),
        }),
    }
}

#[derive(serde::Serialize)]
pub struct CopilotDevicePollResult {
    pub authorized: bool,
    pub access_token: Option<String>,
    pub message: String,
}

/// Complete a Copilot device flow login by exchanging the GitHub access token
/// for a Copilot token and persisting the account.
#[tauri::command]
pub async fn complete_copilot_device_flow(
    state: State<'_, Arc<AppState>>,
    github_access_token: String,
) -> Result<crate::db::accounts::Account, String> {
    crate::services::copilot_adapter::complete_device_flow_login(
        state.inner().as_ref(),
        &github_access_token,
    )
    .await
}

/// Start a Gemini OAuth PKCE login flow.
#[tauri::command]
pub async fn start_gemini_oauth(
    app: tauri::AppHandle,
) -> Result<crate::services::oauth_gemini::GeminiOAuthStartResult, String> {
    let result = crate::services::oauth_gemini::start_gemini_oauth().await?;
    #[allow(deprecated)]
    app.shell()
        .open(result.authorization_url.clone(), None)
        .map_err(|e| format!("无法打开系统浏览器: {}", e))?;
    Ok(result)
}

/// Complete a Gemini OAuth login.
#[tauri::command]
pub async fn complete_gemini_oauth(
    state: State<'_, Arc<AppState>>,
    login_id: String,
    callback_url: Option<String>,
) -> Result<crate::db::accounts::Account, String> {
    crate::services::oauth_gemini::complete_gemini_oauth(state.inner().clone(), &login_id, callback_url)
        .await
}

/// Cancel a pending Gemini OAuth flow.
#[tauri::command]
pub fn cancel_gemini_oauth(login_id: String) -> Result<(), String> {
    crate::services::oauth_gemini::cancel_gemini_oauth(&login_id)
}

/// Start a Google Antigravity OAuth login flow.
#[tauri::command]
pub async fn start_antigravity_oauth(
    app: tauri::AppHandle,
) -> Result<crate::services::antigravity_adapter::AntigravityOAuthStartResult, String> {
    let result = crate::services::antigravity_adapter::start_antigravity_oauth().await?;
    #[allow(deprecated)]
    app.shell()
        .open(result.authorization_url.clone(), None)
        .map_err(|e| format!("无法打开系统浏览器: {}", e))?;
    Ok(result)
}

/// Complete a Google Antigravity OAuth login.
#[tauri::command]
pub async fn complete_antigravity_oauth(
    state: State<'_, Arc<AppState>>,
    login_id: String,
    callback_url: Option<String>,
) -> Result<crate::db::accounts::Account, String> {
    crate::services::antigravity_adapter::complete_antigravity_oauth(
        state.inner().clone(),
        &login_id,
        callback_url,
    )
    .await
}

/// Cancel a pending Google Antigravity OAuth flow.
#[tauri::command]
pub fn cancel_antigravity_oauth(login_id: String) -> Result<(), String> {
    crate::services::antigravity_adapter::cancel_antigravity_oauth(&login_id)
}

#[derive(serde::Serialize)]
pub struct GeminiOAuthStartResult {
    pub login_id: String,
    pub authorization_url: String,
    pub redirect_uri: String,
    pub expires_in_seconds: u64,
}

/// Start a Grok OAuth PKCE login flow.
#[tauri::command]
pub async fn start_grok_oauth(
    app: tauri::AppHandle,
) -> Result<crate::services::oauth_grok::GrokOAuthStartResult, String> {
    let result = crate::services::oauth_grok::start_grok_oauth().await?;
    #[allow(deprecated)]
    app.shell()
        .open(result.authorization_url.clone(), None)
        .map_err(|e| format!("无法打开系统浏览器: {}", e))?;
    Ok(result)
}

/// Complete a Grok OAuth login.
#[tauri::command]
pub async fn complete_grok_oauth(
    state: State<'_, Arc<AppState>>,
    login_id: String,
    callback_url: Option<String>,
) -> Result<crate::db::accounts::Account, String> {
    crate::services::oauth_grok::complete_grok_oauth(state.inner().clone(), &login_id, callback_url)
        .await
}

/// Cancel a pending Grok OAuth flow.
#[tauri::command]
pub fn cancel_grok_oauth(login_id: String) -> Result<(), String> {
    crate::services::oauth_grok::cancel_grok_oauth(&login_id)
}

#[derive(serde::Serialize)]
pub struct GrokOAuthStartResult {
    pub login_id: String,
    pub authorization_url: String,
    pub redirect_uri: String,
    pub expires_in_seconds: u64,
}

/// Start a Claude Code OAuth PKCE login flow.
///
/// Opens the browser for user authorization and returns the login ID and
/// authorization URL.
#[tauri::command]
pub async fn start_claude_oauth(
    app: tauri::AppHandle,
    plan_type: String,
) -> Result<crate::services::oauth_claude::ClaudeOAuthStartResult, String> {
    let result = crate::services::oauth_claude::start_claude_oauth(plan_type).await?;
    #[allow(deprecated)]
    app.shell()
        .open(result.authorization_url.clone(), None)
        .map_err(|e| format!("无法打开系统浏览器: {}", e))?;
    Ok(result)
}

/// Complete a Claude Code OAuth login by exchanging the authorization code.
#[tauri::command]
pub async fn complete_claude_oauth(
    state: State<'_, Arc<AppState>>,
    login_id: String,
    callback_url: Option<String>,
) -> Result<crate::db::accounts::Account, String> {
    crate::services::oauth_claude::complete_claude_oauth(state.inner().clone(), &login_id, callback_url)
        .await
}

/// Cancel a pending Claude Code OAuth flow.
#[tauri::command]
pub fn cancel_claude_oauth(login_id: String) -> Result<(), String> {
    crate::services::oauth_claude::cancel_claude_oauth(&login_id)
}

#[derive(serde::Serialize)]
pub struct ClaudeOAuthStartResult {
    pub login_id: String,
    pub authorization_url: String,
    pub redirect_uri: String,
    pub expires_in_seconds: u64,
}
