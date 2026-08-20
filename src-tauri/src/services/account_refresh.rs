//! Explicit token and quota refresh operations.

use crate::db::accounts::Account;
use crate::services::credentials::{authorization_secret, payload_for_account};
use crate::services::token_refresh::{oauth_from_payload, refresh_oauth_token};
use crate::AppState;
use futures::stream::{self, StreamExt};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QuotaWindow {
    pub key: String,
    pub label: String,
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub window_seconds: Option<i64>,
    pub reset_at: Option<i64>,
    pub reset_after_seconds: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RefreshResult {
    pub account_id: String,
    pub kind: String,
    pub success: bool,
    pub message: String,
    pub plan_type: Option<String>,
    pub quota_windows: Vec<QuotaWindow>,
}

pub async fn refresh_account_token(state: Arc<AppState>, account_id: String) -> RefreshResult {
    match refresh_account_token_inner(&state, &account_id).await {
        Ok(message) => RefreshResult {
            account_id,
            kind: "token".into(),
            success: true,
            message,
            plan_type: None,
            quota_windows: vec![],
        },
        Err(message) => RefreshResult {
            account_id,
            kind: "token".into(),
            success: false,
            message,
            plan_type: None,
            quota_windows: vec![],
        },
    }
}

async fn refresh_account_token_inner(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<String, String> {
    let account = state
        .db
        .accounts
        .get_by_id(&state.db.conn, account_id)?
        .ok_or_else(|| "账号不存在".to_string())?;
    if !matches!(
        account.credential_type.as_deref(),
        Some("oauth") | Some("token") | Some("codex_oauth")
    ) {
        return Err("该账号不是可刷新的 OAuth/Token 凭证".into());
    }
    let provider_id = account
        .provider_id
        .as_deref()
        .ok_or_else(|| "账号没有关联上游连接器".to_string())?;
    let provider = state
        .db
        .providers
        .get_by_id(&state.db.conn, provider_id)?
        .ok_or_else(|| "上游连接器不存在".to_string())?;
    let payload = payload_for_account(&account)?;
    if payload
        .refresh_token
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        return Err("账号没有 refresh_token，无法主动刷新".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
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

    let mut updated = account;
    if let Some(token) = new_payload.access_token.clone() {
        updated.api_key = token;
    }
    updated.expires_at = new_payload.expires_at.clone();
    updated.credential_data = Some(serde_json::to_string(&new_payload).map_err(|e| e.to_string())?);
    state.db.accounts.update(&state.db.conn, &updated)?;
    state
        .db
        .accounts
        .mark_token_refreshed(&state.db.conn, account_id)?;

    // 刷新成功后恢复账号状态，与后台 refresh_loop 的成功路径保持一致：
    // - 非 Codex：status 置回 active + health 写 healthy（清掉此前刷新失败遗留的
    //   token_expired / error，否则前端徽章会一直显示「Token 过期」）；
    // - Codex：status 恢复（token_expired/error → active，disabled/exhausted 不动），
    //   health 写 unchecked（不写 healthy，等待真实 Responses 连通性结果）。
    if updated.credential_type.as_deref() == Some("codex_oauth") {
        state
            .db
            .accounts
            .recover_status(&state.db.conn, account_id, updated.status.as_deref())?;
        state.db.accounts.update_health(
            &state.db.conn,
            account_id,
            "unchecked",
            0,
            "Token refreshed; awaiting Codex connectivity result",
            0,
        )?;
    } else {
        state
            .db
            .accounts
            .update_status(&state.db.conn, account_id, "active")?;
        state.db.accounts.update_health(
            &state.db.conn,
            account_id,
            "healthy",
            200,
            "Token refreshed",
            0,
        )?;
    }
    Ok("Token 已刷新".into())
}

pub async fn batch_refresh_tokens(
    state: Arc<AppState>,
    account_ids: Vec<String>,
) -> Vec<RefreshResult> {
    stream::iter(account_ids)
        .map(|account_id| {
            let state = state.clone();
            async move { refresh_account_token(state, account_id).await }
        })
        .buffer_unordered(5)
        .collect()
        .await
}

pub async fn refresh_account_quota(state: Arc<AppState>, account_id: String) -> RefreshResult {
    let result = refresh_account_quota_inner(&state, &account_id).await;
    match result {
        Ok((plan_type, quota_windows)) => RefreshResult {
            account_id,
            kind: "quota".into(),
            success: true,
            message: "额度已刷新".into(),
            plan_type,
            quota_windows,
        },
        Err(message) => RefreshResult {
            account_id,
            kind: "quota".into(),
            success: false,
            message,
            plan_type: None,
            quota_windows: vec![],
        },
    }
}

async fn refresh_account_quota_inner(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<(Option<String>, Vec<QuotaWindow>), String> {
    let account = state
        .db
        .accounts
        .get_by_id(&state.db.conn, account_id)?
        .ok_or_else(|| "账号不存在".to_string())?;
    let provider = account.provider_id.as_deref().and_then(|id| {
        state
            .db
            .providers
            .get_by_id(&state.db.conn, id)
            .ok()
            .flatten()
    });
    let provider_key = provider
        .as_ref()
        .map(|provider| provider.provider_type.to_lowercase())
        .unwrap_or_default();
    let is_codex = account.credential_type.as_deref() == Some("codex_oauth")
        || provider_key == "codex"
        || account.source_format.as_deref() == Some("codex_auth");

    // DeepSeek 网关账号：官方 /user/balance 余额 → account_usage（额度视图展示真实余额）。
    let is_deepseek = crate::token_monitor::quota::deepseek::is_deepseek(
        account.provider_id.as_deref(),
        provider.as_ref().map(|p| p.name.as_str()),
        provider.as_ref().map(|p| p.base_url.as_str()),
    );
    if is_deepseek {
        return refresh_deepseek_quota(state, account_id, &account).await;
    }

    // Google Antigravity quota comes from the Cloud Code `fetchAvailableModels`
    // response (`models[].quotaInfo`) + `loadCodeAssist` tier.
    let is_antigravity = provider_key == "antigravity"
        || crate::services::antigravity_adapter::is_antigravity_account(
            &account,
            provider
                .as_ref()
                .ok_or_else(|| "上游连接器不存在".to_string())?,
        );
    if is_antigravity {
        return refresh_antigravity_quota(state, account_id, &account).await;
    }

    if !is_codex {
        let error = "当前账号的厂商尚未配置在线额度适配器".to_string();
        state
            .db
            .accounts
            .update_usage_error(&state.db.conn, account_id, &provider_key, &error)?;
        return Err(error);
    }
    match fetch_codex_quota(&account).await {
        Ok((plan, windows)) => {
            let encoded = serde_json::to_string(&windows).map_err(|e| e.to_string())?;
            state.db.accounts.update_usage(
                &state.db.conn,
                account_id,
                "codex",
                plan.as_deref(),
                &encoded,
                None,
            )?;
            Ok((plan, windows))
        }
        Err(error) => {
            state
                .db
                .accounts
                .update_usage_error(&state.db.conn, account_id, "codex", &error)?;
            Err(error)
        }
    }
}

/// DeepSeek 网关账号余额（GET /user/balance）→ 写入 account_usage.quota_windows，
/// 由 `list_quota_accounts` 读取后以 prepaid_balance 窗口展示真实余额。
async fn refresh_deepseek_quota(
    state: &Arc<AppState>,
    account_id: &str,
    account: &Account,
) -> Result<(Option<String>, Vec<QuotaWindow>), String> {
    let api_key = authorization_secret(account)?;
    let connector = crate::token_monitor::quota::deepseek::DeepSeekConnector;
    let windows = connector
        .fetch_quota_inner(&api_key)
        .await
        .map_err(|error| error.to_string())?;
    if windows.is_empty() {
        let error = "DeepSeek 余额接口未返回数据".to_string();
        state
            .db
            .accounts
            .update_usage_error(&state.db.conn, account_id, "deepseek", &error)?;
        return Err(error);
    }
    let encoded = serde_json::to_string(&windows).map_err(|e| e.to_string())?;
    state
        .db
        .accounts
        .update_usage(&state.db.conn, account_id, "deepseek", None, &encoded, None)?;
    // 无百分比窗口（余额是金额）；前端 RefreshResult 的 quota_windows 留空即可。
    Ok((None, vec![]))
}

pub async fn batch_refresh_quotas(
    state: Arc<AppState>,
    account_ids: Vec<String>,
) -> Vec<RefreshResult> {
    stream::iter(account_ids)
        .map(|account_id| {
            let state = state.clone();
            async move { refresh_account_quota(state, account_id).await }
        })
        .buffer_unordered(5)
        .collect()
        .await
}

/// Refresh Google Antigravity quota from the Cloud Code v1internal API.
async fn refresh_antigravity_quota(
    state: &Arc<AppState>,
    account_id: &str,
    account: &Account,
) -> Result<(Option<String>, Vec<QuotaWindow>), String> {
    let payload = payload_for_account(account)?;
    let access_token = payload
        .access_token
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| "Antigravity account has no access token".to_string())?;
    let project_id = payload
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("project_id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let (tier, entries) =
        crate::services::antigravity_adapter::fetch_quota(&access_token, project_id.as_deref())
            .await?;
    if entries.is_empty() {
        let error = "fetchAvailableModels 未返回额度信息".to_string();
        state
            .db
            .accounts
            .update_usage_error(&state.db.conn, account_id, "antigravity", &error)?;
        return Err(error);
    }

    let now = chrono::Utc::now();
    let mut windows = Vec::new();
    for entry in entries {
        let remaining = entry.remaining_fraction.clamp(0.0, 1.0);
        let reset_at = entry
            .reset_time
            .as_deref()
            .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
            .map(|dt| dt.timestamp());
        let reset_after = reset_at.map(|ts| (ts - now.timestamp()).max(0));
        windows.push(QuotaWindow {
            key: entry.model.clone(),
            label: entry.model.clone(),
            used_percent: (1.0 - remaining) * 100.0,
            remaining_percent: remaining * 100.0,
            window_seconds: reset_after,
            reset_at,
            reset_after_seconds: reset_after,
        });
    }
    windows.sort_by(|a, b| {
        a.remaining_percent
            .partial_cmp(&b.remaining_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let encoded = serde_json::to_string(&windows).map_err(|e| e.to_string())?;
    state.db.accounts.update_usage(
        &state.db.conn,
        account_id,
        "antigravity",
        tier.as_deref(),
        &encoded,
        None,
    )?;
    Ok((tier, windows))
}

async fn fetch_codex_quota(
    account: &Account,
) -> Result<(Option<String>, Vec<QuotaWindow>), String> {
    let access_token = authorization_secret(account)?;
    let payload_account_id = payload_for_account(account)?.account_id;
    let account_id = account
        .external_account_id
        .clone()
        .or(payload_account_id)
        .ok_or_else(|| "Codex 账号缺少 ChatGPT Account ID".to_string())?;
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?
        .get("https://chatgpt.com/backend-api/wham/usage")
        .bearer_auth(access_token)
        .header("ChatGPT-Account-Id", account_id)
        .header("Accept", "application/json")
        .header("User-Agent", "PoolGate/0.1")
        .send()
        .await
        .map_err(|e| format!("Codex 额度请求失败: {}", e))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Codex 额度接口返回 {}: {}",
            status,
            truncate(&body, 320)
        ));
    }
    let value: Value =
        serde_json::from_str(&body).map_err(|e| format!("Codex 额度响应解析失败: {}", e))?;
    let plan_type = value
        .get("plan_type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let rate_limit = value.get("rate_limit").unwrap_or(&Value::Null);
    let mut windows = Vec::new();
    if let Some(window) =
        normalize_codex_window(rate_limit.get("primary_window"), "primary", "5 小时额度")
    {
        windows.push(window);
    }
    if let Some(window) =
        normalize_codex_window(rate_limit.get("secondary_window"), "secondary", "周额度")
    {
        windows.push(window);
    }
    if windows.is_empty() {
        return Err("Codex 额度响应中没有可识别的额度窗口".into());
    }
    Ok((plan_type, windows))
}

fn normalize_codex_window(value: Option<&Value>, key: &str, label: &str) -> Option<QuotaWindow> {
    let value = value?;
    let used = value.get("used_percent")?.as_f64()?.clamp(0.0, 100.0);
    Some(QuotaWindow {
        key: key.into(),
        label: label.into(),
        used_percent: used,
        remaining_percent: (100.0 - used).max(0.0),
        window_seconds: value.get("limit_window_seconds").and_then(Value::as_i64),
        reset_at: value.get("reset_at").and_then(Value::as_i64),
        reset_after_seconds: value.get("reset_after_seconds").and_then(Value::as_i64),
    })
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_codex_window() {
        let value = serde_json::json!({
            "used_percent": 37.5,
            "limit_window_seconds": 18000,
            "reset_at": 123,
            "reset_after_seconds": 90
        });
        let window = normalize_codex_window(Some(&value), "primary", "5 小时额度").unwrap();
        assert_eq!(window.remaining_percent, 62.5);
        assert_eq!(window.window_seconds, Some(18000));
    }
}
