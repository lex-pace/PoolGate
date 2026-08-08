//! Import pre-flight health checks.
//!
//! Parses a source without persisting anything, then performs real HTTP health
//! checks against each routable account so the user can review which accounts
//! are healthy, abnormal, duplicate, or need an adapter before importing.

use crate::db::accounts::Account;
use crate::db::providers::Provider;
use crate::services::health_check::{HealthChecker, HealthResult};
use crate::services::import::{
    load_existing_fingerprints, load_existing_provider_names, mask_secret, parse_request,
    primary_secret, short_fingerprint, ImportOptions, ImportProviderPreview, ImportSourceRequest,
};
use futures::stream::{self, StreamExt};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct ImportCheckSummary {
    pub total: usize,
    pub ready: usize,
    pub abnormal: usize,
    pub duplicates: usize,
    pub adapter_required: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckedAccount {
    pub index: usize,
    pub name: String,
    pub email: Option<String>,
    pub provider_name: String,
    pub protocol: String,
    pub credential_type: String,
    pub source_format: String,
    pub masked_credential: String,
    pub external_account_id: Option<String>,
    pub expires_at: Option<String>,
    pub models: Vec<String>,
    pub tags: Vec<String>,
    /// Full SHA-256 fingerprint. Use this value (not `short_fingerprint`) to
    /// filter accounts during `execute_import`.
    pub fingerprint: String,
    /// Short fingerprint suitable for display.
    pub short_fingerprint: String,
    pub action: String,
    pub routable: bool,
    pub adapter: String,
    pub warning: Option<String>,
    pub health_status: String,
    pub health_code: u16,
    pub health_message: String,
    pub health_latency_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportCheckResult {
    pub format: String,
    pub source_count: usize,
    pub accounts: Vec<CheckedAccount>,
    pub auto_created_providers: Vec<ImportProviderPreview>,
    pub warnings: Vec<String>,
    pub skipped: Vec<String>,
    pub summary: ImportCheckSummary,
}

/// Parse an import source and run real health checks against every routable
/// account without writing anything to the database.
pub async fn preview_and_check_import(
    conn: &Mutex<Connection>,
    request: &ImportSourceRequest,
    _options: &ImportOptions,
) -> Result<ImportCheckResult, String> {
    let batch = parse_request(request)?;
    let existing_fingerprints = load_existing_fingerprints(conn)?;
    let existing_providers = load_existing_provider_names(conn)?;
    let mut seen = HashSet::new();
    let mut providers = HashMap::<String, ImportProviderPreview>::new();
    let mut accounts = Vec::with_capacity(batch.accounts.len());
    let mut summary = ImportCheckSummary {
        total: batch.accounts.len(),
        skipped: batch.skipped.len(),
        ..ImportCheckSummary::default()
    };

    // Step 1: classify duplicates and build provider previews synchronously.
    let mut items = Vec::with_capacity(batch.accounts.len());
    for (index, normalized) in batch.accounts.into_iter().enumerate() {
        let is_duplicate = existing_fingerprints.contains(&normalized.fingerprint)
            || !seen.insert(normalized.fingerprint.clone());

        if !existing_providers.contains(&normalized.provider_name.to_lowercase()) {
            providers
                .entry(normalized.provider_name.to_lowercase())
                .or_insert_with(|| ImportProviderPreview {
                    name: normalized.provider_name.clone(),
                    protocol: normalized.protocol.clone(),
                    base_url: normalized.base_url.clone(),
                });
        }

        items.push((index, normalized, is_duplicate));
    }

    // Step 2: run health checks concurrently. Each async block owns its data.
    let checks = stream::iter(items)
        .map(|(index, normalized, is_duplicate)| async move {
            let provider = Provider {
                id: format!("tmp_provider_{}", index),
                name: normalized.provider_name.clone(),
                provider_type: normalized.provider_type.clone(),
                base_url: normalized.base_url.clone(),
                base_urls: Some(
                    serde_json::to_string(&normalized.base_urls).unwrap_or_else(|_| "{}".into()),
                ),
                protocol: normalized.protocol.clone(),
                protocols: Some(
                    serde_json::to_string(&normalized.protocols).unwrap_or_else(|_| "[]".into()),
                ),
                route_takeover: Some(1),
                api_keys: None,
                models: Some(
                    serde_json::to_string(&normalized.models).unwrap_or_else(|_| "[]".into()),
                ),
                proxy_url: None,
                custom_headers: None,
                timeout_ms: None,
                priority: None,
                enabled: Some(true),
                created_at: None,
                auth_mode: None,
                oauth_config: None,
            };

            let account = Account {
                id: format!("tmp_account_{}", index),
                provider_id: Some(provider.id.clone()),
                name: Some(normalized.name.clone()),
                api_key: primary_secret(&normalized),
                models: Some(
                    serde_json::to_string(&normalized.models).unwrap_or_else(|_| "[]".into()),
                ),
                quota_limit: None,
                quota_used: Some(0.0),
                status: Some("disabled".into()),
                health_status: Some("unchecked".into()),
                health_code: None,
                health_msg: normalized.warning.clone(),
                health_latency: None,
                health_check_at: None,
                priority: Some(0),
                tags: Some(serde_json::to_string(&normalized.tags).unwrap_or_else(|_| "[]".into())),
                last_used_at: None,
                created_at: None,
                credential_type: Some(normalized.credential_type.clone()),
                credential_data: Some(
                    serde_json::to_string(&normalized.credential).unwrap_or_else(|_| "{}".into()),
                ),
                source_format: Some(normalized.source_format.clone()),
                external_account_id: normalized.external_account_id.clone(),
                email: normalized.email.clone(),
                expires_at: normalized.expires_at.clone(),
                metadata: Some(normalized.metadata.to_string()),
                credential_fingerprint: Some(normalized.fingerprint.clone()),
                protocols: Some(
                    serde_json::to_string(&normalized.protocols).unwrap_or_else(|_| "[]".into()),
                ),
                route_takeover: Some(1),
                plan_type: None,
                quota_windows: None,
                quota_refreshed_at: None,
                quota_error: None,
                token_refreshed_at: None,
                secret_ref: None,
            };

            let checker = HealthChecker::new(10);
            let result = if is_duplicate {
                HealthResult::Error("Account already exists".into())
            } else if !normalized.routable {
                HealthResult::Error("Account requires an upstream adapter".into())
            } else {
                checker.check_account(&account, &provider).await
            };
            (index, normalized, is_duplicate, result)
        })
        .buffer_unordered(10)
        .collect::<Vec<_>>()
        .await;

    for (index, normalized, is_duplicate, result) in checks {
        let secret = primary_secret(&normalized);
        let (action, health_status, health_code, health_message, health_latency_ms) =
            if is_duplicate {
                summary.duplicates += 1;
                ("duplicate", "duplicate".into(), 0, "账号已存在".into(), 0)
            } else if !normalized.routable {
                summary.adapter_required += 1;
                (
                    "adapter_required",
                    "adapter_required".into(),
                    0,
                    "需要上游适配器".into(),
                    0,
                )
            } else {
                match result {
                    HealthResult::Passed { latency_ms } => {
                        summary.ready += 1;
                        ("create", "healthy".into(), 200, "OK".into(), latency_ms)
                    }
                    HealthResult::Failed { code, body } => {
                        summary.abnormal += 1;
                        ("create", "failed".into(), code, body, 0)
                    }
                    HealthResult::Timeout => {
                        summary.abnormal += 1;
                        ("create", "timeout".into(), 0, "请求超时".into(), 5000)
                    }
                    HealthResult::Error(msg) => {
                        summary.abnormal += 1;
                        ("create", "error".into(), 0, msg, 0)
                    }
                }
            };

        accounts.push(CheckedAccount {
            index,
            name: normalized.name.clone(),
            email: normalized.email.clone(),
            provider_name: normalized.provider_name.clone(),
            protocol: normalized.protocol.clone(),
            credential_type: normalized.credential_type.clone(),
            source_format: normalized.source_format.clone(),
            masked_credential: mask_secret(&secret),
            external_account_id: normalized.external_account_id.clone(),
            expires_at: normalized.expires_at.clone(),
            models: normalized.models.clone(),
            tags: normalized.tags.clone(),
            fingerprint: normalized.fingerprint.clone(),
            short_fingerprint: short_fingerprint(&normalized.fingerprint),
            action: action.to_string(),
            routable: normalized.routable,
            adapter: normalized.adapter.clone(),
            warning: normalized.warning.clone(),
            health_status,
            health_code,
            health_message,
            health_latency_ms,
        });
    }

    accounts.sort_by_key(|account| account.index);

    let format = if batch.formats.iter().collect::<HashSet<_>>().len() > 1 {
        "mixed".to_string()
    } else {
        batch
            .formats
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown".to_string())
    };

    Ok(ImportCheckResult {
        format,
        source_count: batch.source_count,
        accounts,
        auto_created_providers: providers.into_values().collect(),
        warnings: batch.warnings,
        skipped: batch.skipped,
        summary,
    })
}
