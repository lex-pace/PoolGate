//! DeepSeek 余额连接器（公开接口：GET /user/balance）。

use async_trait::async_trait;
use serde_json::Value;

use super::{
    now_iso, resolve_credential, status_to_error, AccountProfile, AuthMethod, ProviderDescriptor,
    QuotaAccountRef, QuotaConfidence, QuotaConnector, QuotaError, QuotaSource, QuotaUnit,
    QuotaWindowSnapshot, QuotaWindowType,
};

const BALANCE_URL: &str = "https://api.deepseek.com/user/balance";

#[derive(Default)]
pub struct DeepSeekConnector;

#[async_trait]
impl QuotaConnector for DeepSeekConnector {
    fn provider(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider_id: "deepseek".into(),
            display_name: "DeepSeek".into(),
            supports_token_usage: false,
            windows_hint: vec!["prepaid_balance".into()],
        }
    }

    fn auth_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::ApiKey]
    }

    async fn validate(&self, credential_ref: &str) -> Result<AccountProfile, QuotaError> {
        let windows = self.fetch_quota_inner(credential_ref).await?;
        let balance = windows.first();
        Ok(AccountProfile {
            identity_masked: "DeepSeek 账号".into(),
            plan_name: balance.map(|b| b.label.clone()),
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
        self.fetch_quota_inner(&credential).await
    }
}

/// 判断一个网关账号/供应商是否为 DeepSeek（模板 id、供应商名或 base_url 含 deepseek）。
/// 用于把「模型供应商」里新增的 DeepSeek 账号也接入余额展示。
pub(crate) fn is_deepseek(
    provider_id: Option<&str>,
    name: Option<&str>,
    base_url: Option<&str>,
) -> bool {
    let id = provider_id.unwrap_or("").to_lowercase();
    let nm = name.unwrap_or("").to_lowercase();
    let url = base_url.unwrap_or("").to_lowercase();
    id.contains("deepseek") || nm.contains("deepseek") || url.contains("deepseek")
}

impl DeepSeekConnector {
    pub(crate) async fn fetch_quota_inner(
        &self,
        api_key: &str,
    ) -> Result<Vec<QuotaWindowSnapshot>, QuotaError> {
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| QuotaError::Network(e.to_string()))?
            .get(BALANCE_URL)
            .bearer_auth(api_key)
            .send()
            .await
            .map_err(|e| QuotaError::Network(format!("DeepSeek 余额请求失败: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_to_error(status));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|e| QuotaError::Parse(format!("DeepSeek 余额响应解析失败: {e}")))?;
        let (currency, total, is_available) = Self::parse_balance(&value)?;

        // 余额是预付费充值池：无窗口上限、无重置周期，remaining_percent/used_value 无从谈起。
        // 前端对 prepaid_balance 窗口特殊展示（金额 + 阈值色）；remaining_value 承载余额金额。
        Ok(vec![QuotaWindowSnapshot {
            window_key: "balance".into(),
            window_type: QuotaWindowType::PrepaidBalance,
            unit: QuotaUnit::Currency,
            label: format!("账户余额（{currency}）"),
            used_value: None,
            limit_value: None,
            remaining_value: Some(total),
            remaining_percent: None,
            period_started_at: None,
            resets_at: None,
            source: QuotaSource::OfficialApi,
            confidence: QuotaConfidence::Reported,
            error_code: if is_available {
                None
            } else {
                Some("insufficient_balance".into())
            },
            fetched_at: now_iso(),
            expires_at: None,
        }])
    }

    /// 解析 /user/balance 响应：真实响应中金额为字符串（"110.00"），需 as_str 兑底。
    fn parse_balance(value: &Value) -> Result<(String, f64, bool), QuotaError> {
        let balance_info = value
            .get("balance_infos")
            .and_then(Value::as_array)
            .and_then(|infos| infos.first())
            .ok_or_else(|| QuotaError::Parse("DeepSeek 响应缺少 balance_infos".into()))?;
        let currency = balance_info
            .get("currency")
            .and_then(Value::as_str)
            .unwrap_or("CNY")
            .to_string();
        let total = Self::as_f64(balance_info.get("total_balance")).unwrap_or(0.0);
        // is_available 是响应顶层字段（余额是否足够发起请求），不在 balance_infos 内
        let is_available = value
            .get("is_available")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        Ok((currency, total, is_available))
    }

    /// 数字字段可能为 JSON 数字或字符串数字，统一解析。
    fn as_f64(v: Option<&Value>) -> Option<f64> {
        v.and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_balance_response() {
        // 真实响应：金额为字符串数字，含 is_available
        let value = serde_json::json!({
            "is_available": true,
            "balance_infos": [{
                "currency": "CNY",
                "total_balance": "110.00",
                "granted_balance": "10.00",
                "topped_up_balance": "100.00"
            }]
        });
        let (currency, total, is_available) = DeepSeekConnector::parse_balance(&value).unwrap();
        assert_eq!(currency, "CNY");
        assert_eq!(total, 110.0);
        assert!(is_available);
    }

    #[test]
    fn balance_window_is_honest() {
        // 无 remaining_percent（不再恒 100%）；remaining_value 承载余额金额
        let value = serde_json::json!({
            "is_available": false,
            "balance_infos": [{
                "currency": "USD",
                "total_balance": "0.00",
                "granted_balance": "0.00",
                "topped_up_balance": "0.00"
            }]
        });
        let (currency, total, is_available) = DeepSeekConnector::parse_balance(&value).unwrap();
        assert_eq!(currency, "USD");
        assert_eq!(total, 0.0);
        assert!(!is_available);
    }
}
