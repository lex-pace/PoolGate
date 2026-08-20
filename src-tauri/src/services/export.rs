//! Export service — exports accounts and logs in various formats.
//!
//! Supports:
//! - `export_accounts` as JSON in "sub2api" or "cockpit" format, with optional key masking.
//! - `export_logs_csv` as a CSV file with standard headers.

use crate::db::accounts::Account;
use rusqlite::Connection;
use std::sync::Mutex;

/// Export all accounts as a JSON string in the requested format.
///
/// # Formats
/// - `"sub2api"` → array of `{api_key, name, models, ...}` objects.
/// - `"cockpit"` → object with `{accounts: [...], providers: [...]}`.
/// - any other → defaults to "sub2api" style.
///
/// When `mask_keys` is true, each API key is replaced with `sk-****xxxx` where
/// `xxxx` are the last 4 characters of the original key.
pub fn export_accounts(
    conn: &Mutex<Connection>,
    format: &str,
    mask_keys: bool,
) -> Result<String, String> {
    let accounts = crate::db::accounts::AccountRepo.list_all(conn)?;
    export_account_list(&accounts, format, mask_keys)
}

/// Export one account as JSON. Secrets are masked by default at the command layer.
pub fn export_account(
    conn: &Mutex<Connection>,
    account_id: &str,
    format: &str,
    mask_keys: bool,
) -> Result<String, String> {
    let account = crate::db::accounts::AccountRepo
        .get_by_id(conn, account_id)?
        .ok_or_else(|| "账号不存在".to_string())?;
    export_account_list(&[account], format, mask_keys)
}

fn export_account_list(
    accounts: &[Account],
    format: &str,
    mask_keys: bool,
) -> Result<String, String> {
    match format.to_lowercase().as_str() {
        "cockpit" => export_cockpit(accounts, mask_keys),
        _ => export_sub2api(accounts, mask_keys), // "sub2api" or default
    }
}

/// Export request logs as CSV within the optional time range.
///
/// CSV headers:
/// `id, group_id, source, provider_id, account_id, model, endpoint, status,
///  status_code, input_tokens, output_tokens, cache_tokens, cost, latency_ms,
///  ttft_ms, is_stream, error_message, request_at`
pub fn export_logs_csv(
    conn: &Mutex<Connection>,
    start_time: Option<&str>,
    end_time: Option<&str>,
) -> Result<String, String> {
    let query = crate::db::logs::LogQuery {
        page: None,
        page_size: None, // no limit
        group_id: None,
        status: None,
        source: None,
        range: None,
        start_time: start_time.map(|s| s.to_string()),
        end_time: end_time.map(|s| s.to_string()),
        keyword: None,
    };

    let logs = crate::db::logs::LogRepo.query(conn, &query)?;

    let mut csv = String::new();

    // Write header
    csv.push_str("id,group_id,source,provider_id,account_id,model,endpoint,status,status_code,");
    csv.push_str(
        "input_tokens,output_tokens,cache_tokens,cost,latency_ms,ttft_ms,is_stream,error_message,request_at\n"
    );

    for log in &logs {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&log.id.map(|v| v.to_string())),
            csv_escape(&log.group_id),
            csv_escape(&log.source),
            csv_escape(&log.provider_id),
            csv_escape(&log.account_id),
            csv_escape(&log.model),
            csv_escape(&log.endpoint),
            csv_escape(&log.status),
            csv_escape(&log.status_code.map(|v| v.to_string())),
            csv_escape(&log.input_tokens.map(|v| v.to_string())),
            csv_escape(&log.output_tokens.map(|v| v.to_string())),
            csv_escape(&log.cache_tokens.map(|v| v.to_string())),
            csv_escape(&log.cost.map(|v| format!("{:.6}", v))),
            csv_escape(&log.latency_ms.map(|v| v.to_string())),
            csv_escape(&log.ttft_ms.map(|v| v.to_string())),
            csv_escape(&log.is_stream.map(|v| v.to_string())),
            csv_escape(&log.error_message),
            csv_escape(&log.request_at),
        ));
    }

    Ok(csv)
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn csv_escape(val: &Option<String>) -> String {
    let raw = val.clone().unwrap_or_default();
    if raw.contains(',') || raw.contains('"') || raw.contains('\n') {
        format!("\"{}\"", raw.replace('"', "\"\""))
    } else {
        raw
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────

fn export_sub2api(accounts: &[Account], mask_keys: bool) -> Result<String, String> {
    let entries: Vec<serde_json::Value> = accounts
        .iter()
        .map(|a| {
            let secret = crate::services::credentials::authorization_secret(a)?;
            let key = if mask_keys {
                mask_api_key(&secret)
            } else {
                secret
            };
            Ok(serde_json::json!({
                "api_key": key,
                "name": a.name,
                "models": parse_models(&a.models),
                "provider_id": a.provider_id,
                "status": a.status,
                "priority": a.priority,
                "tags": parse_tags(&a.tags),
                "quota_limit": a.quota_limit,
                "quota_used": a.quota_used,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;

    serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())
}

fn export_cockpit(accounts: &[Account], mask_keys: bool) -> Result<String, String> {
    // Collect unique provider IDs referenced by accounts
    let mut provider_ids: Vec<&str> = accounts
        .iter()
        .filter_map(|a| a.provider_id.as_deref())
        .collect();
    provider_ids.sort();
    provider_ids.dedup();

    let entries: Vec<serde_json::Value> = accounts
        .iter()
        .map(|a| {
            let secret = crate::services::credentials::authorization_secret(a)?;
            let key = if mask_keys {
                mask_api_key(&secret)
            } else {
                secret
            };
            Ok(serde_json::json!({
                "id": a.id,
                "api_key": key,
                "name": a.name,
                "provider_id": a.provider_id,
                "models": parse_models(&a.models),
                "status": a.status,
                "health_status": a.health_status,
                "health_latency": a.health_latency,
                "health_check_at": a.health_check_at,
                "priority": a.priority,
                "tags": parse_tags(&a.tags),
                "quota_limit": a.quota_limit,
                "quota_used": a.quota_used,
                "last_used_at": a.last_used_at,
                "created_at": a.created_at,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let result = serde_json::json!({
        "accounts": entries,
        "provider_ids": provider_ids,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "total": accounts.len(),
    });

    serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
}

/// Mask an API key without ever returning the complete secret. Long values
/// retain only the last four characters; short values are fully obscured.
fn mask_api_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 4 {
        return "****".to_string();
    }
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("****{}", suffix)
}

fn parse_models(models: &Option<String>) -> Vec<String> {
    models
        .as_ref()
        .map(|s| {
            s.split(',')
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_tags(tags: &Option<String>) -> Vec<String> {
    tags.as_ref()
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::mask_api_key;

    #[test]
    fn short_keys_are_never_returned_verbatim() {
        assert_eq!(mask_api_key("abc"), "****");
        assert_ne!(mask_api_key("secret7"), "secret7");
        assert_eq!(mask_api_key("sk-123456789"), "****6789");
    }
}
