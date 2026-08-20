//! 额度连接器契约与注册表（03 §3 落地 + W4 实现）。
//!
//! **共享文件**：注册区在本文件锚点后按字典序追加，由集成负责人统一合并。
//! 异步方案：`async-trait = "0.1"`（07 §8 决策，00 §9 已登记）。

pub mod antigravity;
pub mod claude;
pub mod cursor;
pub mod custom;
pub mod deepseek;
pub mod github_copilot;
pub mod minimax;
pub mod ollama_cloud;
pub mod openai_codex;
pub mod openrouter;
pub mod qoder;
pub mod volcengine_ark;

use std::sync::Arc;

use async_trait::async_trait;

use crate::AppState;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderDescriptor {
    /// claude|codex|cursor|copilot|deepseek|openrouter|minimax|...
    pub provider_id: String,
    pub display_name: String,
    /// 21 工具型 true；7 额度型多为 false
    pub supports_token_usage: bool,
    /// 该 Provider 典型窗口（仅提示，实际以返回为准）
    pub windows_hint: Vec<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    OfficialOauth,
    ApiKey,
    LocalAuthImport,
    DashboardCookie,
    CustomEndpoint,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AccountProfile {
    pub identity_masked: String,
    pub plan_name: Option<String>,
}

/// 传给 connector 的账号引用（含 Keychain 凭证引用，connector 自行取凭证）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QuotaAccountRef {
    pub account_id: String,
    pub provider_id: String,
    pub auth_method: AuthMethod,
    /// Keychain 引用；对 linked 路由账号为 None（命令层解析）
    pub credential_ref: Option<String>,
    pub linked_route_account_id: Option<String>,
}

/// 额度窗口快照——扩展自现有 account_refresh::QuotaWindow。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QuotaWindowSnapshot {
    pub window_key: String,
    pub window_type: QuotaWindowType,
    pub unit: QuotaUnit,
    pub label: String,
    pub used_value: Option<f64>,
    pub limit_value: Option<f64>,
    pub remaining_value: Option<f64>,
    pub remaining_percent: Option<f64>,
    pub period_started_at: Option<String>,
    /// UTC ISO8601
    pub resets_at: Option<String>,
    pub source: QuotaSource,
    pub confidence: QuotaConfidence,
    pub error_code: Option<String>,
    pub fetched_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaWindowType {
    Rolling5h,
    Weekly,
    Monthly,
    Billing,
    Credits,
    PrepaidBalance,
    Requests,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaUnit {
    Tokens,
    Requests,
    Credits,
    Currency,
    Percent,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    OfficialApi,
    LocalAuth,
    DashboardSession,
    CustomEndpoint,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaConfidence {
    Reported,
    Derived,
    Stale,
}

#[derive(Debug, thiserror::Error)]
pub enum QuotaError {
    /// 401/403 → 熔断
    #[error("auth expired")]
    AuthExpired,
    #[error("rate limited, retry after {0}s")]
    RateLimited(i64),
    #[error("network: {0}")]
    Network(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("parse: {0}")]
    Parse(String),
}

/// 额度连接器契约。W4 每个 Provider 实现一次。
#[async_trait]
pub trait QuotaConnector: Send + Sync {
    fn provider(&self) -> ProviderDescriptor;

    /// 该 Provider 支持的认证方式（按优先级）。
    fn auth_methods(&self) -> Vec<AuthMethod>;

    /// 校验凭证并返回账号画像（绑定向导第 4 步调用）。
    async fn validate(&self, credential_ref: &str) -> Result<AccountProfile, QuotaError>;

    /// 拉取额度窗口快照。
    async fn fetch_quota(
        &self,
        account: &QuotaAccountRef,
    ) -> Result<Vec<QuotaWindowSnapshot>, QuotaError>;
}

/// `QuotaWindowSnapshot` → 视图类型 `QuotaWindowView`（命令层返回）。
impl From<QuotaWindowSnapshot> for crate::token_monitor::model::QuotaWindowView {
    fn from(snapshot: QuotaWindowSnapshot) -> Self {
        crate::token_monitor::model::QuotaWindowView {
            window_key: snapshot.window_key,
            window_type: snapshot.window_type,
            unit: snapshot.unit,
            label: snapshot.label,
            used_value: snapshot.used_value,
            limit_value: snapshot.limit_value,
            remaining_value: snapshot.remaining_value,
            remaining_percent: snapshot.remaining_percent,
            resets_at: snapshot.resets_at,
            period_started_at: snapshot.period_started_at,
            source: snapshot.source,
            confidence: snapshot.confidence,
            fetched_at: snapshot.fetched_at,
            error_code: snapshot.error_code,
        }
    }
}

/// 可绑定的 Provider id 全集（供 `list_quota_providers` 与注册表一致性校验）。
pub const PROVIDER_IDS: &[&str] = &[
    "antigravity",
    "claude",
    "codex",
    "cursor",
    "custom",
    "deepseek",
    "github_copilot",
    "minimax",
    "ollama_cloud",
    "openrouter",
    "qoder",
    "volcengine_ark",
];

/// 连接器注册表——共享文件，锚点追加。
pub struct ConnectorRegistry;
impl ConnectorRegistry {
    pub fn get(provider_id: &str) -> Option<Box<dyn QuotaConnector>> {
        match provider_id {
            "antigravity" => Some(Box::new(antigravity::AntigravityConnector)),
            "claude" => Some(Box::new(claude::ClaudeConnector)),
            "codex" => Some(Box::new(openai_codex::CodexConnector)),
            "cursor" => Some(Box::new(cursor::CursorConnector)),
            "custom" => Some(Box::new(custom::CustomConnector)),
            "deepseek" => Some(Box::new(deepseek::DeepSeekConnector)),
            "github_copilot" => Some(Box::new(github_copilot::CopilotConnector)),
            "minimax" => Some(Box::new(minimax::MinimaxConnector)),
            "ollama_cloud" => Some(Box::new(ollama_cloud::OllamaCloudConnector)),
            "openrouter" => Some(Box::new(openrouter::OpenRouterConnector)),
            "qoder" => Some(Box::new(qoder::QoderConnector)),
            "volcengine_ark" => Some(Box::new(volcengine_ark::VolcengineArkConnector)),
            // ==== connector registry ====  (新 Provider 在此锚点后按字典序追加)
            _ => None,
        }
    }
}

/// 解析凭证：优先当 Keychain 引用取；取不到且形似明文则按明文用；否则视为失效。
pub fn resolve_credential(credential_ref: &str) -> Result<String, QuotaError> {
    if let Ok(Some(bytes)) = crate::services::keychain::get_optional_secret(credential_ref) {
        if let Ok(secret) = String::from_utf8(bytes) {
            if !secret.trim().is_empty() {
                return Ok(secret.trim().to_string());
            }
        }
    }
    if credential_ref.starts_with("tm.") {
        return Err(QuotaError::AuthExpired);
    }
    if credential_ref.trim().is_empty() {
        return Err(QuotaError::AuthExpired);
    }
    // 非 tm. 前缀且 Keychain 取不到 → 视为字面明文（绑定时由命令层注入）
    Ok(credential_ref.to_string())
}

/// 由 HTTP 状态码映射错误（401/403 → AuthExpired；429 → RateLimited）。
pub fn status_to_error(status: reqwest::StatusCode) -> QuotaError {
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        QuotaError::AuthExpired
    } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        QuotaError::RateLimited(60)
    } else {
        QuotaError::Network(format!("HTTP {status}"))
    }
}

/// 简化的 ISO8601 秒级时间戳（UTC）。
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---------------------------------------------------------------------------
// 后台刷新循环（W4）：默认 5 分钟；剩余 <20% 缩到 2 分钟；401/403 熔断；429 退避。
// ---------------------------------------------------------------------------

/// 刷新一个账号（命令层与循环共用）。返回 Ok(()) 表示成功/已记录失败。
pub async fn refresh_account(state: &Arc<AppState>, account_id: &str) -> Result<(), String> {
    let Some(account) = state
        .db
        .quota_accounts
        .get_account(&state.db.conn, account_id)?
    else {
        return Err(format!("额度账号不存在：{account_id}"));
    };
    let provider_id = account["provider_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let enabled = account["enabled"].as_bool().unwrap_or(true);
    if !enabled {
        return Ok(());
    }
    let connector = ConnectorRegistry::get(&provider_id)
        .ok_or_else(|| format!("未知额度 Provider：{provider_id}"))?;

    let credential_ref = account["credential_ref"].as_str().map(ToString::to_string);
    // linked 路由账号：从 accounts 表解析凭证（复用现有 authorization_secret）
    let resolved_ref = if credential_ref.is_none() {
        if let Some(route_id) = account["linked_route_account_id"].as_str() {
            match crate::db::accounts::AccountRepo.get_by_id(&state.db.conn, route_id) {
                Ok(Some(route_account)) => {
                    match crate::services::credentials::authorization_secret(&route_account) {
                        Ok(secret) => Some(secret),
                        Err(error) => {
                            let _ = state.db.quota_accounts.update_status(
                                &state.db.conn,
                                account_id,
                                "auth_expired",
                                false,
                            );
                            return Err(format!("路由账号凭证不可用：{error}"));
                        }
                    }
                }
                _ => {
                    let _ = state.db.quota_accounts.update_status(
                        &state.db.conn,
                        account_id,
                        "error",
                        false,
                    );
                    return Err(format!("关联的路由账号不存在：{route_id}"));
                }
            }
        } else {
            None
        }
    } else {
        credential_ref
    };

    let quota_ref = QuotaAccountRef {
        account_id: account_id.to_string(),
        provider_id,
        auth_method: serde_json::from_str(&format!(
            "\"{}\"",
            account["auth_method"].as_str().unwrap_or("api_key")
        ))
        .unwrap_or(AuthMethod::ApiKey),
        credential_ref: resolved_ref.clone(),
        linked_route_account_id: account["linked_route_account_id"]
            .as_str()
            .map(ToString::to_string),
    };

    // 凭证缺失 → 熔断状态
    let Some(ref _credential) = resolved_ref else {
        let _ = state.db.quota_accounts.update_status(
            &state.db.conn,
            account_id,
            "auth_expired",
            false,
        );
        return Err("账号缺少凭证，请重新绑定".into());
    };

    match connector.fetch_quota(&quota_ref).await {
        Ok(windows) => {
            if windows.is_empty() {
                let _ = state.db.quota_accounts.update_status(
                    &state.db.conn,
                    account_id,
                    "unavailable",
                    false,
                );
                return Err("额度接口未返回任何窗口".into());
            }
            for window in &windows {
                let _ = state
                    .db
                    .quota_windows
                    .upsert_snapshot(&state.db.conn, account_id, window);
            }
            let plan = account["plan_name"].as_str().map(ToString::to_string);
            if plan.is_none() {
                let _ = state.db.quota_accounts.upsert_account(
                    &state.db.conn,
                    account_id,
                    &quota_ref.provider_id,
                    account["label"].as_str(),
                    account["identity_masked"].as_str(),
                    None,
                    account["auth_method"].as_str().unwrap_or("api_key"),
                    account["credential_ref"].as_str(),
                    account["linked_route_account_id"].as_str(),
                    enabled,
                    "active",
                );
            }
            let _ =
                state
                    .db
                    .quota_accounts
                    .update_status(&state.db.conn, account_id, "active", true);
            Ok(())
        }
        Err(QuotaError::AuthExpired) => {
            let _ = state.db.quota_accounts.update_status(
                &state.db.conn,
                account_id,
                "auth_expired",
                false,
            );
            Err("授权已过期，请重新绑定".into())
        }
        Err(QuotaError::RateLimited(_secs)) => {
            let _ = state.db.quota_accounts.update_status(
                &state.db.conn,
                account_id,
                "rate_limited",
                false,
            );
            Err("接口限流，稍后重试".into())
        }
        Err(error) => {
            let _ =
                state
                    .db
                    .quota_accounts
                    .update_status(&state.db.conn, account_id, "error", false);
            Err(error.to_string())
        }
    }
}

/// 后台刷新循环：5 分钟一轮；任一窗口剩余 <20% → 下一轮 2 分钟。
/// 每轮刷新后评估告警（W5）：emit `token-monitor:alert` + 更新托盘 tooltip 主指标。
pub async fn refresh_loop(app: tauri::AppHandle, state: Arc<AppState>) {
    loop {
        let interval = if any_low_quota(&state) {
            2 * 60
        } else {
            5 * 60
        };
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        let ids: Vec<String> = match state
            .db
            .quota_accounts
            .list_accounts(&state.db.conn)
            .map(|views| views.into_iter().map(|v| v.account_id).collect())
        {
            Ok(ids) => ids,
            Err(error) => {
                tracing::warn!("token_monitor: quota refresh list failed: {error}");
                continue;
            }
        };
        for account_id in ids {
            match refresh_account(&state, &account_id).await {
                Ok(()) => {
                    // 刷新成功 → 按新快照评估并发布告警（去抖在 alerts 层）
                    if let Ok(windows) = state
                        .db
                        .quota_windows
                        .list_snapshots(&state.db.conn, &account_id)
                    {
                        publish_alerts(&app, &state, &account_id, &windows);
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        "token_monitor: quota refresh {} failed: {}",
                        account_id,
                        error
                    );
                }
            }
        }
        // W7：同步刷新网关侧账号额度（OpenAI/ChatGPT OAuth 5h/周 + DeepSeek 余额），
        // 写入 account_usage → list_quota_accounts 额度视图自动更新。
        // Gemini 无公开额度接口，跳过（视图内诚实标注「不可用」）。
        refresh_gateway_quota(&state).await;

        // 每轮结束后统一刷新托盘主指标 tooltip
        update_tray_metric(&app, &state);
    }
}

/// 刷新网关侧账号额度：`codex_oauth`（OpenAI/ChatGPT OAuth 5h/周）与 DeepSeek 余额。
/// 失败静默（下轮再试）；无此类账号时零开销返回。
async fn refresh_gateway_quota(state: &Arc<AppState>) {
    let ids: Vec<String> = {
        let Ok(conn) = state.db.conn.lock() else {
            return;
        };
        let Ok(mut stmt) = conn.prepare(
            "SELECT a.id FROM accounts a
             LEFT JOIN providers p ON p.id = a.provider_id
             WHERE (a.credential_type = 'codex_oauth'
                    OR lower(COALESCE(a.provider_id,'')) LIKE '%deepseek%'
                    OR lower(COALESCE(p.name,'')) LIKE '%deepseek%'
                    OR lower(COALESCE(p.base_url,'')) LIKE '%deepseek%')
               AND (a.status IS NULL OR a.status != 'disabled')",
        ) else {
            return;
        };
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>())
            .unwrap_or_default();
        drop(stmt);
        ids
    };
    for id in ids {
        let _ = crate::services::account_refresh::refresh_account_quota(state.clone(), id).await;
    }
}

/// 评估并发布某账号的额度告警：系统通知由**后端直接发送**（一次，无跨窗口竞态），
/// 同时 emit `token-monitor:alert` 事件供前端做应用内 Toast（前端不再发系统通知）。
/// 发布逻辑统一走 `alerts::publish_alert`（采集告警复用同一入口）。
fn publish_alerts(
    app: &tauri::AppHandle,
    state: &Arc<AppState>,
    account_id: &str,
    windows: &[QuotaWindowSnapshot],
) {
    let provider_id = state
        .db
        .quota_accounts
        .get_account(&state.db.conn, account_id)
        .ok()
        .flatten()
        .and_then(|account| account["provider_id"].as_str().map(ToString::to_string));
    let alerts = crate::token_monitor::alerts::evaluate_account_alerts(
        Some(account_id),
        provider_id.as_deref(),
        windows,
    );
    for alert in alerts {
        crate::token_monitor::alerts::publish_alert(app, alert);
    }
}

/// 每轮额度刷新后统一更新菜单栏状态胶囊（单一图标：Logo + 状态点 + 主文本，
/// 图标 + 原生标题 + tooltip）。状态计算（额度最差剩余 + 网关运行 + 流量活跃）
/// 在 menu_bar 模块内完成。
fn update_tray_metric(app: &tauri::AppHandle, state: &Arc<AppState>) {
    let _ = crate::services::menu_bar::apply_menu_bar(app, state);
}

fn any_low_quota(state: &Arc<AppState>) -> bool {
    let Ok(conn) = state.db.conn.lock() else {
        return false;
    };
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM quota_window_snapshot
             WHERE remaining_percent IS NOT NULL AND remaining_percent < 20",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    count > 0
}

/// 供 `service.rs::init` 拉起（与 W1/W2 的循环并列，均挂 AppState 生命周期）。
/// W5 起需要 AppHandle（告警事件 + 托盘 tooltip 主指标）。
pub fn spawn_refresh_loop(app: tauri::AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        refresh_loop(app, state).await;
    });
}
