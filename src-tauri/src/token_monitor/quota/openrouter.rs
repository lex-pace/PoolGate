//! OpenRouter 额度连接器（公开接口：GET /api/v1/key）。

use async_trait::async_trait;
use serde_json::Value;

use super::{
    now_iso, resolve_credential, status_to_error, AccountProfile, AuthMethod, ProviderDescriptor,
    QuotaAccountRef, QuotaConfidence, QuotaConnector, QuotaError, QuotaSource, QuotaUnit,
    QuotaWindowSnapshot, QuotaWindowType,
};

const KEY_URL: &str = "https://openrouter.ai/api/v1/key";

#[derive(Default)]
pub struct OpenRouterConnector;

#[async_trait]
impl QuotaConnector for OpenRouterConnector {
    fn provider(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider_id: "openrouter".into(),
            display_name: "OpenRouter".into(),
            supports_token_usage: false,
            windows_hint: vec!["credits".into(), "billing".into()],
        }
    }

    fn auth_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::ApiKey]
    }

    async fn validate(&self, credential_ref: &str) -> Result<AccountProfile, QuotaError> {
        let data = self.fetch_key(credential_ref).await?;
        Ok(AccountProfile {
            identity_masked: data
                .get("label")
                .and_then(Value::as_str)
                .map(|s| {
                    if s.len() > 8 {
                        format!("{}****", &s[..4])
                    } else {
                        s.to_string()
                    }
                })
                .unwrap_or_else(|| "OpenRouter 账号".into()),
            plan_name: Some("free".into()),
        })
    }

    async fn fetch_quota(
        &self,
        account: &QuotaAccountRef,
    ) -> Result<Vec<QuotaWindowSnapshot>, QuotaError> {
        let credential = account
            .credential_ref
            .as_deref()
            .ok_or(QuotaError::AuthExpired)
            .and_then(resolve_credential)?;
        let data = self.fetch_key(&credential).await?;

        let usage: f64 = data.get("usage").and_then(Value::as_f64).unwrap_or(0.0);
        let limit: Option<f64> = data
            .get("limit")
            .and_then(Value::as_f64)
            .filter(|limit| *limit > 0.0);
        let is_free = data
            .get("is_free_tier")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let mut windows = Vec::new();
        windows.push(QuotaWindowSnapshot {
            window_key: "credits".into(),
            window_type: QuotaWindowType::Credits,
            unit: QuotaUnit::Credits,
            label: if is_free {
                "免费额度（按量）"
            } else {
                "Credits 用量"
            }
            .into(),
            used_value: Some(usage),
            limit_value: limit,
            remaining_value: limit.map(|l| (l - usage).max(0.0)),
            remaining_percent: limit.map(|l| {
                if l > 0.0 {
                    ((l - usage).max(0.0) / l) * 100.0
                } else {
                    0.0
                }
            }),
            period_started_at: None,
            resets_at: None,
            source: QuotaSource::OfficialApi,
            confidence: QuotaConfidence::Reported,
            error_code: None,
            fetched_at: now_iso(),
            expires_at: None,
        });
        Ok(windows)
    }
}

impl OpenRouterConnector {
    async fn fetch_key(&self, api_key: &str) -> Result<Value, QuotaError> {
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| QuotaError::Network(e.to_string()))?
            .get(KEY_URL)
            .bearer_auth(api_key)
            .send()
            .await
            .map_err(|e| QuotaError::Network(format!("OpenRouter 请求失败: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_to_error(status));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|e| QuotaError::Parse(format!("OpenRouter 响应解析失败: {e}")))?;
        value
            .get("data")
            .cloned()
            .ok_or_else(|| QuotaError::Parse("OpenRouter 响应缺少 data".into()))
    }
}
