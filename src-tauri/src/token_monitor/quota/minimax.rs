//! Minimax 额度连接器（API Key，Token Plan 周期额度；接口私有，experimental）。

use async_trait::async_trait;

use super::{
    AccountProfile, AuthMethod, ProviderDescriptor, QuotaAccountRef, QuotaConnector, QuotaError,
};

#[derive(Default)]
pub struct MinimaxConnector;

#[async_trait]
impl QuotaConnector for MinimaxConnector {
    fn provider(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider_id: "minimax".into(),
            display_name: "Minimax".into(),
            supports_token_usage: false,
            windows_hint: vec!["monthly".into()],
        }
    }

    fn auth_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::ApiKey]
    }

    async fn validate(&self, _credential_ref: &str) -> Result<AccountProfile, QuotaError> {
        Err(QuotaError::Unsupported(
            "Minimax Token Plan 周期额度接口为私有接口（实验性），暂不可自动校验".into(),
        ))
    }

    async fn fetch_quota(
        &self,
        _account: &QuotaAccountRef,
    ) -> Result<Vec<super::QuotaWindowSnapshot>, QuotaError> {
        Err(QuotaError::Unsupported(
            "Minimax Token Plan 周期额度接口为私有接口（实验性），暂不可自动拉取".into(),
        ))
    }
}
