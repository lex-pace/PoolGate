//! Custom / Third-party 额度连接器（声明式 Balance Endpoint，实验性）。
//!
//! 用户需在设置中配置 GET URL 与取值路径（JSONPath 简式）；配置前诚实返回
//! `Unsupported`。本版本未实现端点配置界面，先占位保持可注册。

use async_trait::async_trait;

use super::{
    AccountProfile, AuthMethod, ProviderDescriptor, QuotaAccountRef, QuotaConnector, QuotaError,
};

#[derive(Default)]
pub struct CustomConnector;

#[async_trait]
impl QuotaConnector for CustomConnector {
    fn provider(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider_id: "custom".into(),
            display_name: "自定义 / New API".into(),
            supports_token_usage: false,
            windows_hint: vec!["prepaid_balance".into(), "credits".into()],
        }
    }

    fn auth_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::ApiKey, AuthMethod::CustomEndpoint]
    }

    async fn validate(&self, _credential_ref: &str) -> Result<AccountProfile, QuotaError> {
        Err(QuotaError::Unsupported(
            "自定义端点需在设置中配置 Balance Endpoint（实验性）".into(),
        ))
    }

    async fn fetch_quota(
        &self,
        _account: &QuotaAccountRef,
    ) -> Result<Vec<super::QuotaWindowSnapshot>, QuotaError> {
        Err(QuotaError::Unsupported(
            "自定义端点需在设置中配置 Balance Endpoint（实验性）".into(),
        ))
    }
}
