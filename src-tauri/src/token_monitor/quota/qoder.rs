//! Qoder 额度连接器（Dashboard 登录态 Credits；接口私有，experimental）。

use async_trait::async_trait;

use super::{
    AccountProfile, AuthMethod, ProviderDescriptor, QuotaAccountRef, QuotaConnector, QuotaError,
};

#[derive(Default)]
pub struct QoderConnector;

#[async_trait]
impl QuotaConnector for QoderConnector {
    fn provider(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider_id: "qoder".into(),
            display_name: "Qoder".into(),
            supports_token_usage: false,
            windows_hint: vec!["credits".into()],
        }
    }

    fn auth_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::DashboardCookie]
    }

    async fn validate(&self, _credential_ref: &str) -> Result<AccountProfile, QuotaError> {
        Err(QuotaError::Unsupported(
            "Qoder Credits 接口为私有接口（实验性，需用户粘贴 Dashboard Cookie），暂不可自动校验"
                .into(),
        ))
    }

    async fn fetch_quota(
        &self,
        _account: &QuotaAccountRef,
    ) -> Result<Vec<super::QuotaWindowSnapshot>, QuotaError> {
        Err(QuotaError::Unsupported(
            "Qoder Credits 接口为私有接口（实验性），暂不可自动拉取".into(),
        ))
    }
}
