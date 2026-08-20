//! 服务状态采集（对齐开源 Token Monitor 状态页）：Claude / OpenAI / Cursor / DeepSeek。
//!
//! 拉取各供应商 statuspage.io 的 `/api/v2/summary.json`，归一化为统一状态
//! （ok / degraded / outage / unknown）。成功缓存 60s、任一失败缓存 10s
//! （瞬断可快速恢复），单供应商 5s 超时，单个失败不影响其余供应商。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::token_monitor::model::{ServiceIssue, ServiceStatusView};

const CACHE_OK_MS: u128 = 60_000;
const CACHE_ERR_MS: u128 = 10_000;
const TIMEOUT_MS: u64 = 5_000;
const USER_AGENT: &str = "PoolGate-TokenMonitor/1.0";

struct Provider {
    id: &'static str,
    label: &'static str,
    page_url: &'static str,
    summary_url: &'static str,
}

const PROVIDERS: &[Provider] = &[
    Provider {
        id: "claude",
        label: "Claude",
        page_url: "https://status.claude.com",
        summary_url: "https://status.claude.com/api/v2/summary.json",
    },
    Provider {
        id: "openai",
        label: "OpenAI",
        page_url: "https://status.openai.com",
        summary_url: "https://status.openai.com/api/v2/summary.json",
    },
    Provider {
        id: "cursor",
        label: "Cursor",
        page_url: "https://status.cursor.com",
        summary_url: "https://status.cursor.com/api/v2/summary.json",
    },
    Provider {
        id: "deepseek",
        label: "DeepSeek",
        // status.deepseek.com 的 /api/v2 对程序客户端不返回 JSON（官方页只服务浏览器），
        // 从 Atlassian 托管的镜像取 JSON，页面仍链接官方页（对齐开源实现）。
        page_url: "https://status.deepseek.com",
        summary_url: "https://deepseek.statuspage.io/api/v2/summary.json",
    },
];

type CacheEntry = (Instant, Duration, Vec<ServiceStatusView>);
static CACHE: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

/// statuspage indicator → 统一状态（对齐开源 providerTone）。
fn tone(indicator: &str) -> &'static str {
    match indicator {
        "none" => "ok",
        "minor" => "degraded",
        "major" | "critical" => "outage",
        _ => "unknown",
    }
}

/// 过滤出活跃的 incidents / scheduled_maintenances（排除已结束状态）。
fn active_items(items: &serde_json::Value, inactive: &[&str]) -> Vec<serde_json::Value> {
    items
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|it| {
                    let st = normalize(it.get("status").and_then(|v| v.as_str()).unwrap_or(""));
                    !st.is_empty() && !inactive.contains(&st.as_str())
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// 非 operational / under_maintenance 的组件视为异常项。
fn component_issues(components: &serde_json::Value) -> Vec<ServiceIssue> {
    const OK: &[&str] = &["operational", "under_maintenance"];
    components
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|c| {
                    let st = normalize(c.get("status").and_then(|v| v.as_str()).unwrap_or(""));
                    !st.is_empty() && !OK.contains(&st.as_str())
                })
                .map(|c| ServiceIssue {
                    name: c
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown")
                        .trim()
                        .to_string(),
                    status: normalize(
                        c.get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown"),
                    ),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 归一化成功响应的 summary.json。
fn summarize_ok(
    id: &str,
    label: &str,
    page_url: &str,
    payload: &serde_json::Value,
    checked_at: &str,
) -> ServiceStatusView {
    let indicator = normalize(
        payload
            .pointer("/status/indicator")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown"),
    );
    let issues = component_issues(
        payload
            .get("components")
            .unwrap_or(&serde_json::Value::Null),
    );
    let incidents = active_items(
        payload.get("incidents").unwrap_or(&serde_json::Value::Null),
        &["resolved", "completed", "postmortem"],
    );
    let maintenances = active_items(
        payload
            .get("scheduled_maintenances")
            .unwrap_or(&serde_json::Value::Null),
        &["completed", "canceled"],
    );
    ServiceStatusView {
        provider_id: id.into(),
        label: label.into(),
        page_url: page_url.into(),
        status: tone(&indicator).into(),
        indicator,
        description: payload
            .pointer("/status/description")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "未知".into()),
        checked_at: checked_at.into(),
        updated_at: payload
            .pointer("/page/updated_at")
            .and_then(|v| v.as_str())
            .or_else(|| {
                payload
                    .pointer("/status/updated_at")
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("")
            .trim()
            .to_string(),
        component_issues: issues,
        incident_title: incidents
            .first()
            .and_then(|i| i.get("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        incident_count: incidents.len() as i64,
        maintenance_count: maintenances.len() as i64,
        error: None,
    }
}

/// 失败/异常时的占位视图（能力诚实：状态标 unknown + 原因）。
fn summarize_err(
    id: &str,
    label: &str,
    page_url: &str,
    checked_at: &str,
    error: String,
) -> ServiceStatusView {
    ServiceStatusView {
        provider_id: id.into(),
        label: label.into(),
        page_url: page_url.into(),
        status: "unknown".into(),
        indicator: "unknown".into(),
        description: "无法检查状态".into(),
        checked_at: checked_at.into(),
        updated_at: String::new(),
        component_issues: vec![],
        incident_title: String::new(),
        incident_count: 0,
        maintenance_count: 0,
        error: Some(error),
    }
}

/// 拉取并归一化各供应商服务状态（带内存缓存：全成功 60s / 任一失败 10s）。
pub async fn fetch_service_statuses() -> Vec<ServiceStatusView> {
    if let Some((at, dur, views)) = cache().lock().ok().and_then(|m| m.get("all").cloned()) {
        if at.elapsed() < dur {
            return views;
        }
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(TIMEOUT_MS))
        .user_agent(USER_AGENT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let checked_at = chrono::Utc::now().to_rfc3339();
    let mut results: Vec<ServiceStatusView> = Vec::with_capacity(PROVIDERS.len());
    for provider in PROVIDERS {
        let outcome = match client.get(provider.summary_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<serde_json::Value>().await {
                    Ok(json) => summarize_ok(
                        provider.id,
                        provider.label,
                        provider.page_url,
                        &json,
                        &checked_at,
                    ),
                    Err(e) => summarize_err(
                        provider.id,
                        provider.label,
                        provider.page_url,
                        &checked_at,
                        format!("解析失败: {e}"),
                    ),
                }
            }
            Ok(resp) => summarize_err(
                provider.id,
                provider.label,
                provider.page_url,
                &checked_at,
                format!("HTTP {}", resp.status()),
            ),
            Err(e) => summarize_err(
                provider.id,
                provider.label,
                provider.page_url,
                &checked_at,
                format!("请求失败: {e}"),
            ),
        };
        results.push(outcome);
    }

    let cache_ms = if results.iter().any(|s| s.error.is_some()) {
        CACHE_ERR_MS
    } else {
        CACHE_OK_MS
    };
    if let Ok(mut guard) = cache().lock() {
        guard.retain(|_, (at, _, _)| at.elapsed().as_secs() < 3600);
        guard.insert(
            "all".into(),
            (
                Instant::now(),
                Duration::from_millis(cache_ms as u64),
                results.clone(),
            ),
        );
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_statuspage_summary_json() {
        let json = serde_json::json!({
            "page": { "id": "p", "updated_at": "2026-08-08T00:00:00Z" },
            "status": { "indicator": "minor", "description": "Partial System Outage" },
            "components": [
                { "name": "API", "status": "operational" },
                { "name": "Chat", "status": "degraded_performance" }
            ],
            "incidents": [
                { "name": "Elevated errors on Haiku", "status": "investigating" },
                { "name": "Resolved issue", "status": "resolved" }
            ],
            "scheduled_maintenances": [
                { "name": "Upgrade", "status": "in_progress" },
                { "name": "Done", "status": "completed" }
            ]
        });
        let view = summarize_ok(
            "claude",
            "Claude",
            "https://status.claude.com",
            &json,
            "2026-08-08T01:00:00Z",
        );
        assert_eq!(view.status, "degraded"); // minor → degraded
        assert_eq!(view.indicator, "minor");
        assert_eq!(view.description, "Partial System Outage");
        assert_eq!(view.component_issues.len(), 1);
        assert_eq!(view.component_issues[0].name, "Chat");
        assert_eq!(view.incident_count, 1); // resolved 被过滤
        assert_eq!(view.incident_title, "Elevated errors on Haiku");
        assert_eq!(view.maintenance_count, 1); // completed 被过滤
        assert_eq!(view.updated_at, "2026-08-08T00:00:00Z");
        assert!(view.error.is_none());
    }

    #[test]
    fn tone_mapping_and_error_view() {
        assert_eq!(tone("none"), "ok");
        assert_eq!(tone("minor"), "degraded");
        assert_eq!(tone("major"), "outage");
        assert_eq!(tone("critical"), "outage");
        assert_eq!(tone("bogus"), "unknown");

        let err = summarize_err(
            "openai",
            "OpenAI",
            "https://status.openai.com",
            "2026-08-08T00:00:00Z",
            "请求失败: timeout".into(),
        );
        assert_eq!(err.status, "unknown");
        assert_eq!(err.description, "无法检查状态");
        assert_eq!(err.error.as_deref(), Some("请求失败: timeout"));
        assert_eq!(err.incident_count, 0);
    }
}
