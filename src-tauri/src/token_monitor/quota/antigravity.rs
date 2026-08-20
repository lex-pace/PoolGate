//! Google Antigravity（Cloud Code）额度连接器。
//!
//! 逻辑复用 `services/antigravity_adapter.rs::fetch_quota`（原 `refresh_antigravity_quota`
//! 的搬运，原 `account_refresh.rs` 不动）。每个模型一个窗口（model 专属）。

use async_trait::async_trait;

use super::{
    now_iso, resolve_credential, AccountProfile, AuthMethod, ProviderDescriptor, QuotaAccountRef,
    QuotaConfidence, QuotaConnector, QuotaError, QuotaSource, QuotaUnit, QuotaWindowSnapshot,
    QuotaWindowType,
};

#[derive(Default)]
pub struct AntigravityConnector;

#[async_trait]
impl QuotaConnector for AntigravityConnector {
    fn provider(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider_id: "antigravity".into(),
            display_name: "Google Antigravity".into(),
            supports_token_usage: true,
            windows_hint: vec!["monthly".into(), "model:gemini-2.5-pro".into()],
        }
    }

    fn auth_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::OfficialOauth]
    }

    async fn validate(&self, credential_ref: &str) -> Result<AccountProfile, QuotaError> {
        let account = QuotaAccountRef {
            account_id: "validate".into(),
            provider_id: "antigravity".into(),
            auth_method: AuthMethod::OfficialOauth,
            credential_ref: Some(credential_ref.to_string()),
            linked_route_account_id: None,
        };
        self.fetch_quota(&account).await?;
        Ok(AccountProfile {
            identity_masked: "Antigravity 账号".into(),
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

        let (_tier, entries) =
            crate::services::antigravity_adapter::fetch_quota(&access_token, None)
                .await
                .map_err(|error| {
                    let lower = error.to_lowercase();
                    if lower.contains("401")
                        || lower.contains("403")
                        || lower.contains("unauthorized")
                    {
                        QuotaError::AuthExpired
                    } else if lower.contains("429") || lower.contains("rate") {
                        QuotaError::RateLimited(60)
                    } else {
                        QuotaError::Network(error)
                    }
                })?;
        if entries.is_empty() {
            return Err(QuotaError::Parse(
                "fetchAvailableModels 未返回额度信息".into(),
            ));
        }

        let now = chrono::Utc::now();
        let mut windows = Vec::new();
        for entry in &entries {
            let remaining = entry.remaining_fraction.clamp(0.0, 1.0);
            let resets_at = entry
                .reset_time
                .as_deref()
                .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
                .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                .unwrap_or_else(|| {
                    (now + chrono::Duration::days(1))
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                });
            windows.push(QuotaWindowSnapshot {
                window_key: format!("model:{}", entry.model),
                window_type: QuotaWindowType::Monthly,
                unit: QuotaUnit::Percent,
                label: entry.model.clone(),
                used_value: Some((1.0 - remaining) * 100.0),
                limit_value: Some(100.0),
                remaining_value: Some(remaining * 100.0),
                remaining_percent: Some(remaining * 100.0),
                period_started_at: None,
                resets_at: Some(resets_at),
                source: QuotaSource::OfficialApi,
                confidence: QuotaConfidence::Reported,
                error_code: None,
                fetched_at: now_iso(),
                expires_at: None,
            });
        }
        windows.sort_by(|a, b| {
            a.remaining_percent
                .partial_cmp(&b.remaining_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(windows)
    }
}
