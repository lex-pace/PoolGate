//! Cursor 额度连接器。
//!
//! Cursor 的额度/订阅接口为私有接口（experimental），无稳定公开端点；
//! 诚实标注不可用，不伪造窗口。

use async_trait::async_trait;

use super::{
    AccountProfile, AuthMethod, ProviderDescriptor, QuotaAccountRef, QuotaConnector, QuotaError,
};

#[derive(Default)]
pub struct CursorConnector;

#[async_trait]
impl QuotaConnector for CursorConnector {
    fn provider(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider_id: "cursor".into(),
            display_name: "Cursor".into(),
            supports_token_usage: true,
            windows_hint: vec!["monthly".into(), "credits".into()],
        }
    }

    fn auth_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::LocalAuthImport, AuthMethod::ApiKey]
    }

    async fn validate(&self, _credential_ref: &str) -> Result<AccountProfile, QuotaError> {
        Err(QuotaError::Unsupported(
            "Cursor 额度接口为私有接口（实验性），暂不可自动校验；请在 Cursor 设置中确认登录态"
                .into(),
        ))
    }

    async fn fetch_quota(
        &self,
        _account: &QuotaAccountRef,
    ) -> Result<Vec<super::QuotaWindowSnapshot>, QuotaError> {
        Err(QuotaError::Unsupported(
            "Cursor 额度接口为私有接口（实验性），暂不可自动拉取".into(),
        ))
    }
}
