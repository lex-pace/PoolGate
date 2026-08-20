//! GitHub Copilot 额度连接器。
//!
//! PAT 校验走公开接口（GET /user）；订阅/配额接口为私有（experimental），
//! 诚实标注不可用。

use async_trait::async_trait;

use super::{
    resolve_credential, status_to_error, AccountProfile, AuthMethod, ProviderDescriptor,
    QuotaAccountRef, QuotaConnector, QuotaError,
};

const USER_URL: &str = "https://api.github.com/user";

#[derive(Default)]
pub struct CopilotConnector;

#[async_trait]
impl QuotaConnector for CopilotConnector {
    fn provider(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider_id: "github_copilot".into(),
            display_name: "GitHub Copilot".into(),
            supports_token_usage: true,
            windows_hint: vec!["monthly".into(), "requests".into()],
        }
    }

    fn auth_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::OfficialOauth, AuthMethod::ApiKey]
    }

    async fn validate(&self, credential_ref: &str) -> Result<AccountProfile, QuotaError> {
        let token = resolve_credential(credential_ref)?;
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| QuotaError::Network(e.to_string()))?
            .get(USER_URL)
            .bearer_auth(token)
            .header("User-Agent", "PoolGate/0.1")
            .send()
            .await
            .map_err(|e| QuotaError::Network(format!("GitHub 校验请求失败: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_to_error(status));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|e| QuotaError::Parse(e.to_string()))?;
        let login = value
            .get("login")
            .and_then(|v| v.as_str())
            .unwrap_or("GitHub 用户");
        Ok(AccountProfile {
            identity_masked: format!("@{login}"),
            plan_name: None,
        })
    }

    async fn fetch_quota(
        &self,
        _account: &QuotaAccountRef,
    ) -> Result<Vec<super::QuotaWindowSnapshot>, QuotaError> {
        Err(QuotaError::Unsupported(
            "Copilot 订阅/配额接口为私有接口（实验性），暂不可自动拉取".into(),
        ))
    }
}
