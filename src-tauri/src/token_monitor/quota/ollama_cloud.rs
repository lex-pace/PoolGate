//! Ollama Cloud 额度连接器（API Key / 登录态；周用量窗口接口私有，experimental）。

use async_trait::async_trait;

use super::{
    AccountProfile, AuthMethod, ProviderDescriptor, QuotaAccountRef, QuotaConnector, QuotaError,
};

#[derive(Default)]
pub struct OllamaCloudConnector;

#[async_trait]
impl QuotaConnector for OllamaCloudConnector {
    fn provider(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider_id: "ollama_cloud".into(),
            display_name: "Ollama Cloud".into(),
            supports_token_usage: false,
            windows_hint: vec!["weekly".into()],
        }
    }

    fn auth_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::ApiKey, AuthMethod::LocalAuthImport]
    }

    async fn validate(&self, _credential_ref: &str) -> Result<AccountProfile, QuotaError> {
        Err(QuotaError::Unsupported(
            "Ollama Cloud 周用量窗口接口为私有接口（实验性），暂不可自动校验".into(),
        ))
    }

    async fn fetch_quota(
        &self,
        _account: &QuotaAccountRef,
    ) -> Result<Vec<super::QuotaWindowSnapshot>, QuotaError> {
        Err(QuotaError::Unsupported(
            "Ollama Cloud 周用量窗口接口为私有接口（实验性），暂不可自动拉取".into(),
        ))
    }
}
