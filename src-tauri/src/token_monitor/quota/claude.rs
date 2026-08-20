//! Claude（Anthropic）额度连接器。
//!
//! 校验走公开接口（GET /v1/models，验证 API Key 可用）；额度拉取**诚实标注**：
//! Anthropic 未提供公开的额度/用量接口，本地登录态额度接口为私有接口（实验性），
//! 暂无稳定实现 → `QuotaError::Unsupported`（UI 显示「不可用」，不猜不填）。

use async_trait::async_trait;

use super::{
    resolve_credential, status_to_error, AccountProfile, AuthMethod, ProviderDescriptor,
    QuotaAccountRef, QuotaConnector, QuotaError, QuotaWindowSnapshot,
};

const MODELS_URL: &str = "https://api.anthropic.com/v1/models";

#[derive(Default)]
pub struct ClaudeConnector;

#[async_trait]
impl QuotaConnector for ClaudeConnector {
    fn provider(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider_id: "claude".into(),
            display_name: "Claude（Anthropic）".into(),
            supports_token_usage: true,
            windows_hint: vec!["rolling_5h".into(), "weekly".into()],
        }
    }

    fn auth_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::OfficialOauth, AuthMethod::ApiKey]
    }

    async fn validate(&self, credential_ref: &str) -> Result<AccountProfile, QuotaError> {
        let api_key = resolve_credential(credential_ref)?;
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| QuotaError::Network(e.to_string()))?
            .get(MODELS_URL)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(|e| QuotaError::Network(format!("Claude 校验请求失败: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_to_error(status));
        }
        Ok(AccountProfile {
            identity_masked: "Claude 账号".into(),
            plan_name: None,
        })
    }

    async fn fetch_quota(
        &self,
        _account: &QuotaAccountRef,
    ) -> Result<Vec<QuotaWindowSnapshot>, QuotaError> {
        // 能力诚实：Anthropic 无公开额度接口 → 明确「不可用」，不猜不填 0。
        Err(QuotaError::Unsupported(
            "Anthropic 未提供公开额度接口；本地登录态额度为私有接口（实验性），暂不可自动拉取"
                .into(),
        ))
    }
}
