//! Codex（OpenAI）额度连接器。
//!
//! 逻辑从 `services/account_refresh.rs::fetch_codex_quota` 搬入并实现 `QuotaConnector`
//! （原文件不动，服务 Gateway 路由账号）。输出升级为 `QuotaWindowSnapshot`
//! （window_type：primary→Rolling5h、secondary→Weekly）。

use async_trait::async_trait;
use serde_json::Value;

use super::{
    now_iso, resolve_credential, status_to_error, AccountProfile, AuthMethod, ProviderDescriptor,
    QuotaAccountRef, QuotaConfidence, QuotaConnector, QuotaError, QuotaSource, QuotaUnit,
    QuotaWindowSnapshot, QuotaWindowType,
};

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

#[derive(Default)]
pub struct CodexConnector;

#[async_trait]
impl QuotaConnector for CodexConnector {
    fn provider(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider_id: "codex".into(),
            display_name: "Codex（OpenAI）".into(),
            supports_token_usage: false,
            windows_hint: vec!["rolling_5h".into(), "weekly".into()],
        }
    }

    fn auth_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::OfficialOauth]
    }

    async fn validate(&self, credential_ref: &str) -> Result<AccountProfile, QuotaError> {
        // 校验 = 尝试拉取额度（凭证有效则能拉到窗口）
        let account = QuotaAccountRef {
            account_id: "validate".into(),
            provider_id: "codex".into(),
            auth_method: AuthMethod::OfficialOauth,
            credential_ref: Some(credential_ref.to_string()),
            linked_route_account_id: None,
        };
        self.fetch_quota(&account).await?;
        Ok(AccountProfile {
            identity_masked: "Codex 账号".into(),
            plan_name: None,
        })
    }

    async fn fetch_quota(
        &self,
        account: &QuotaAccountRef,
    ) -> Result<Vec<QuotaWindowSnapshot>, QuotaError> {
        let access_token = account
            .credential_ref
            .as_deref()
            .ok_or(QuotaError::AuthExpired)
            .and_then(resolve_credential)?;
        // Codex 的 account_id（external_account_id）无法从 QuotaAccountRef 拿到——
        // linked 路由账号场景由命令层把凭证解析为 access_token；ChatGPT-Account-Id
        // 头部仅在凭证为 OAuth token 且账号已关联时才有意义，缺失时服务端按默认处理。
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| QuotaError::Network(e.to_string()))?
            .get(USAGE_URL)
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .header("User-Agent", "PoolGate/0.1")
            .send()
            .await
            .map_err(|e| QuotaError::Network(format!("Codex 额度请求失败: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_to_error(status));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|e| QuotaError::Parse(format!("Codex 额度响应解析失败: {e}")))?;

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
            return Err(QuotaError::Parse(
                "Codex 额度响应中没有可识别的额度窗口".into(),
            ));
        }
        Ok(windows)
    }
}

fn normalize_codex_window(
    value: Option<&Value>,
    key: &str,
    label: &str,
) -> Option<QuotaWindowSnapshot> {
    let value = value?;
    let used = value.get("used_percent")?.as_f64()?.clamp(0.0, 100.0);
    let resets_at = value.get("reset_at").and_then(Value::as_i64).map(|secs| {
        chrono::DateTime::from_timestamp(secs, 0)
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_default()
    });
    Some(QuotaWindowSnapshot {
        window_key: key.into(),
        window_type: if key == "primary" {
            QuotaWindowType::Rolling5h
        } else {
            QuotaWindowType::Weekly
        },
        unit: QuotaUnit::Percent,
        label: label.into(),
        used_value: Some(used),
        limit_value: Some(100.0),
        remaining_value: Some((100.0 - used).max(0.0)),
        remaining_percent: Some((100.0 - used).max(0.0)),
        period_started_at: None,
        resets_at,
        source: QuotaSource::OfficialApi,
        confidence: QuotaConfidence::Reported,
        error_code: None,
        fetched_at: now_iso(),
        expires_at: None,
    })
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
        assert!((window.remaining_percent.unwrap() - 62.5).abs() < 0.001);
        assert_eq!(window.window_type, QuotaWindowType::Rolling5h);
    }

    #[test]
    fn status_mapping_handles_401_and_429() {
        assert!(matches!(
            status_to_error(reqwest::StatusCode::UNAUTHORIZED),
            QuotaError::AuthExpired
        ));
        assert!(matches!(
            status_to_error(reqwest::StatusCode::TOO_MANY_REQUESTS),
            QuotaError::RateLimited(_)
        ));
    }
}
