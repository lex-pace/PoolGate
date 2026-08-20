//! 额度/采集告警（W5 实现）。
//!
//! 阈值：剩余 ≤20% 提醒 remind / ≤10% 警告 warn / ≤5% 严重 critical；
//! 数据过期：fetch 后 >30min 未成功刷新 → stale 提醒。
//! 去抖：同一 (account_id, window_key, level) 30 分钟一个周期只发一次（不骚扰）。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::token_monitor::model::{CollectorStatus, TmAlert};
use crate::token_monitor::quota::{QuotaConfidence, QuotaWindowSnapshot};

pub const REMIND_PERCENT: f64 = 20.0;
pub const WARN_PERCENT: f64 = 10.0;
pub const CRITICAL_PERCENT: f64 = 5.0;
/// 数据超过该时长未成功刷新 → stale（30 分钟）
pub const STALE_AFTER_SECS: i64 = 30 * 60;

/// 去抖表：key=(account_id, window_key, level) → 上次发出时间。
fn last_emitted() -> &'static Mutex<HashMap<(String, String, String), Instant>> {
    static LAST: OnceLock<Mutex<HashMap<(String, String, String), Instant>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 按阈值分级的告警等级（无告警返回 None）。
fn level_for(percent: f64) -> Option<&'static str> {
    if percent <= CRITICAL_PERCENT {
        Some("critical")
    } else if percent <= WARN_PERCENT {
        Some("warn")
    } else if percent <= REMIND_PERCENT {
        Some("remind")
    } else {
        None
    }
}

/// 评估额度窗口快照，产出需要触发的告警（去抖后）。
/// 冻结签名：仅传窗口；account_id 置空（测试/无账号上下文时用）。
pub fn evaluate_quota_alerts(windows: &[QuotaWindowSnapshot]) -> Vec<TmAlert> {
    evaluate_account_alerts(None, None, windows)
}

/// 按账号评估额度告警（后台刷新循环调用）：带 account_id/provider_id 与去抖。
pub fn evaluate_account_alerts(
    account_id: Option<&str>,
    provider_id: Option<&str>,
    windows: &[QuotaWindowSnapshot],
) -> Vec<TmAlert> {
    let now = Instant::now();
    let mut alerts: Vec<TmAlert> = Vec::new();

    for window in windows {
        let level = window.remaining_percent.and_then(level_for);

        // 1) 低额度告警
        if let Some(level) = level {
            let key = (
                account_id.unwrap_or("?").to_string(),
                window.window_key.clone(),
                level.to_string(),
            );
            if !is_duplicate(&key, now) {
                let percent = window.remaining_percent.unwrap_or(0.0);
                alerts.push(TmAlert {
                    kind: "quota_low".into(),
                    level: level.into(),
                    account_id: account_id.map(ToString::to_string),
                    tool_id: provider_id.map(ToString::to_string),
                    message: format!(
                        "{} 额度剩余 {:.0}%（{}，重置于 {}）",
                        provider_id.unwrap_or("额度"),
                        percent,
                        window.label,
                        window.resets_at.as_deref().unwrap_or("未知")
                    ),
                    emitted_at: chrono::Utc::now().to_rfc3339(),
                });
                mark_emitted(&key, now);
            }
        }

        // 2) 数据过期（stale）提醒——限 remind 级，避免骚扰。
        //    判据：connector 显式标记 Stale，或 fetched_at 距今 >30 分钟（age 兜底）。
        let stale_by_age = chrono::DateTime::parse_from_rfc3339(&window.fetched_at)
            .ok()
            .map(|fetched| {
                chrono::Utc::now().timestamp() - fetched.with_timezone(&chrono::Utc).timestamp()
                    > STALE_AFTER_SECS
            })
            .unwrap_or(false);
        if window.confidence == QuotaConfidence::Stale || stale_by_age {
            let key = (
                account_id.unwrap_or("?").to_string(),
                window.window_key.clone(),
                "stale".into(),
            );
            if !is_duplicate(&key, now) {
                alerts.push(TmAlert {
                    kind: "quota_stale".into(),
                    level: "remind".into(),
                    account_id: account_id.map(ToString::to_string),
                    tool_id: provider_id.map(ToString::to_string),
                    message: format!(
                        "{} 额度数据已过期（>30 分钟未刷新）",
                        provider_id.unwrap_or("额度")
                    ),
                    emitted_at: chrono::Utc::now().to_rfc3339(),
                });
                mark_emitted(&key, now);
            }
        }
    }

    alerts
}

fn is_duplicate(key: &(String, String, String), now: Instant) -> bool {
    last_emitted()
        .lock()
        .map(|map| {
            map.get(key)
                .map(|last| now.duration_since(*last).as_secs() < 30 * 60)
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn mark_emitted(key: &(String, String, String), now: Instant) {
    if let Ok(mut map) = last_emitted().lock() {
        map.insert(key.clone(), now);
    }
}

// ---------------------------------------------------------------------------
// 采集失败告警（collector_error）与统一发布
// ---------------------------------------------------------------------------

/// 采集告警去抖表：key=(tool_id, status) → 上次发出时间（30 分钟一周期，同配额）。
fn collector_last_emitted() -> &'static Mutex<HashMap<(String, String), Instant>> {
    static LAST: OnceLock<Mutex<HashMap<(String, String), Instant>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(HashMap::new()))
}

fn collector_status_key(status: &CollectorStatus) -> String {
    serde_json::to_string(status)
        .map(|s| s.trim_matches('"').to_string())
        .unwrap_or_else(|_| "error".into())
}

fn collector_status_label(status: &CollectorStatus) -> &'static str {
    match status {
        CollectorStatus::PathMissing => "路径缺失",
        CollectorStatus::Permission => "需授权",
        CollectorStatus::FormatChanged => "格式变化",
        CollectorStatus::Error => "解析/IO 错误",
        // 其余非错误状态不应走到采集告警
        _ => "状态异常",
    }
}

/// 错误状态族：只有这些状态才触发采集告警。
fn is_collector_error_status(status: &CollectorStatus) -> bool {
    matches!(
        status,
        CollectorStatus::PathMissing
            | CollectorStatus::Permission
            | CollectorStatus::FormatChanged
            | CollectorStatus::Error
    )
}

/// 采集失败告警：错误状态族触发，同一 (tool_id, status) 30 分钟内只发一次。
/// 返回 Some 即应发布（调用方负责 `publish_alert`）。
pub fn evaluate_collector_alert(
    display_name: &str,
    tool_id: &str,
    status: &CollectorStatus,
    detail: Option<&str>,
) -> Option<TmAlert> {
    if !is_collector_error_status(status) {
        return None;
    }
    let now = Instant::now();
    let key = (tool_id.to_string(), collector_status_key(status));
    if collector_last_emitted()
        .lock()
        .map(|map| {
            map.get(&key)
                .map(|last| now.duration_since(*last).as_secs() < 30 * 60)
                .unwrap_or(false)
        })
        .unwrap_or(false)
    {
        return None;
    }
    if let Ok(mut map) = collector_last_emitted().lock() {
        map.insert(key, now);
    }
    let label = collector_status_label(status);
    let message = match detail.map(str::trim).filter(|d| !d.is_empty()) {
        Some(detail) => format!("{display_name} 采集失败（{label}）：{detail}"),
        None => format!("{display_name} 采集失败（{label}）"),
    };
    Some(TmAlert {
        kind: "collector_error".into(),
        level: "warn".into(),
        account_id: None,
        tool_id: Some(tool_id.to_string()),
        message,
        emitted_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// 统一发布告警：emit `token-monitor:alert`（前端应用内 Toast）+ 后端直接发系统通知
/// （一次、无跨窗口竞态；桌面端插件权限为 no-op，OS 首次 show 时引导授权）。
pub fn publish_alert(app: &tauri::AppHandle, alert: TmAlert) {
    use tauri::Emitter;
    use tauri_plugin_notification::NotificationExt;
    let _ = app.emit("token-monitor:alert", &alert);
    let title = match (alert.kind.as_str(), alert.level.as_str()) {
        ("collector_error", _) => "PoolGate · 采集告警".to_string(),
        (_, "critical") => "PoolGate · 额度严重告警".to_string(),
        (_, "warn") => "PoolGate · 额度警告".to_string(),
        _ => "PoolGate · 额度提醒".to_string(),
    };
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(alert.message.clone())
        .show();
}

/// 计算托盘主指标（W5：额度优先，其次今日 Tokens）。
/// 由命令层 `get_tray_primary_metric` 与托盘 tooltip 刷新共用。
pub fn tray_primary_metric(
    windows: &[QuotaWindowSnapshot],
    today_tokens: i64,
) -> crate::token_monitor::model::TrayPrimaryMetric {
    // 最低剩余额度
    let worst = windows
        .iter()
        .filter(|w| w.remaining_percent.is_some())
        .min_by(|a, b| {
            a.remaining_percent
                .partial_cmp(&b.remaining_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    if let Some(worst) = worst {
        let percent = worst.remaining_percent.unwrap_or(0.0);
        let level = level_for(percent).unwrap_or("ok");
        return crate::token_monitor::model::TrayPrimaryMetric {
            kind: "quota".into(),
            text: format!("额度 {:.0}%", percent),
            level: Some(level.into()),
        };
    }
    if today_tokens > 0 {
        let text = if today_tokens >= 1_000_000 {
            format!("今日 {:.2}M Tokens", today_tokens as f64 / 1_000_000.0)
        } else if today_tokens >= 1_000 {
            format!("今日 {:.1}K Tokens", today_tokens as f64 / 1_000.0)
        } else {
            format!("今日 {} Tokens", today_tokens)
        };
        return crate::token_monitor::model::TrayPrimaryMetric {
            kind: "tokens".into(),
            text,
            level: Some("ok".into()),
        };
    }
    crate::token_monitor::model::TrayPrimaryMetric {
        kind: "idle".into(),
        text: "PoolGate 就绪".into(),
        level: Some("ok".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_monitor::quota::{QuotaSource, QuotaUnit, QuotaWindowType};

    fn window(key: &str, percent: Option<f64>, stale: bool) -> QuotaWindowSnapshot {
        QuotaWindowSnapshot {
            window_key: key.into(),
            window_type: QuotaWindowType::Rolling5h,
            unit: QuotaUnit::Percent,
            label: "5 小时额度".into(),
            used_value: None,
            limit_value: None,
            remaining_value: None,
            remaining_percent: percent,
            period_started_at: None,
            resets_at: None,
            source: QuotaSource::OfficialApi,
            confidence: if stale {
                QuotaConfidence::Stale
            } else {
                QuotaConfidence::Reported
            },
            error_code: None,
            fetched_at: chrono::Utc::now().to_rfc3339(),
            expires_at: None,
        }
    }

    #[test]
    fn thresholds_map_to_levels() {
        assert_eq!(level_for(3.0), Some("critical"));
        assert_eq!(level_for(8.0), Some("warn"));
        assert_eq!(level_for(15.0), Some("remind"));
        assert_eq!(level_for(50.0), None);
    }

    #[test]
    fn low_quota_produces_alert_and_dedups() {
        // 用唯一 window_key 隔离去抖键,避免并行测试互相污染。
        let alerts = evaluate_quota_alerts(&[window("low-key", Some(4.0), false)]);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, "quota_low");
        assert_eq!(alerts[0].level, "critical");
        // 30 分钟内重复评估 → 去抖（无新告警）
        let again = evaluate_quota_alerts(&[window("low-key", Some(4.0), false)]);
        assert!(again.is_empty(), "同周期重复触发应被去抖");
        // 重置去抖表，避免污染其他测试
        last_emitted().lock().unwrap().clear();
    }

    #[test]
    fn healthy_window_no_alert() {
        let alerts = evaluate_quota_alerts(&[window("healthy-key", Some(55.0), false)]);
        assert!(alerts.is_empty());
    }

    #[test]
    fn stale_window_emits_remind() {
        let alerts = evaluate_quota_alerts(&[window("stale-key", Some(60.0), true)]);
        assert!(alerts
            .iter()
            .any(|a| a.kind == "quota_stale" && a.level == "remind"));
        // 不清空去抖表(避免并行测试间隙破坏 low_quota 的两次调用);key 已唯一隔离
    }

    #[test]
    fn collector_error_emits_alert_with_warn_level() {
        let alert = evaluate_collector_alert(
            "Claude Code",
            "claude_code",
            &CollectorStatus::PathMissing,
            Some("no such file"),
        )
        .expect("error status should produce alert");
        assert_eq!(alert.kind, "collector_error");
        assert_eq!(alert.level, "warn");
        assert_eq!(alert.tool_id.as_deref(), Some("claude_code"));
        assert!(alert.message.contains("Claude Code"));
        assert!(alert.message.contains("路径缺失"));
        // 30 分钟内同 (tool, status) 去抖（测试 key 唯一隔离，不清去抖表——
        // 与配额测试同理，避免并行测试间隙清掉其他用例的键）
        assert!(evaluate_collector_alert(
            "Claude Code",
            "claude_code",
            &CollectorStatus::PathMissing,
            Some("no such file"),
        )
        .is_none());
    }

    #[test]
    fn collector_healthy_status_no_alert() {
        assert!(evaluate_collector_alert("Codex", "codex", &CollectorStatus::Idle, None).is_none());
        assert!(
            evaluate_collector_alert("Codex", "codex", &CollectorStatus::Active, None).is_none()
        );
    }

    #[test]
    fn collector_alert_dedup_keyed_by_status() {
        // 不同错误状态视为不同告警（各自独立去抖）
        let first =
            evaluate_collector_alert("Cursor", "cursor", &CollectorStatus::Permission, None);
        assert!(first.is_some());
        let second = evaluate_collector_alert("Cursor", "cursor", &CollectorStatus::Error, None);
        assert!(second.is_some());
        // 同 (tool, status) 立即重复 → 去抖
        assert!(
            evaluate_collector_alert("Cursor", "cursor", &CollectorStatus::Error, None).is_none()
        );
    }

    #[test]
    fn primary_metric_prefers_quota_then_tokens() {
        let metric = tray_primary_metric(&[window("metric-key", Some(3.0), false)], 500_000);
        assert_eq!(metric.kind, "quota");
        assert_eq!(metric.level.as_deref(), Some("critical"));
        let metric = tray_primary_metric(&[], 1_250_000);
        assert_eq!(metric.kind, "tokens");
        assert!(metric.text.contains("1.25M"));
        let metric = tray_primary_metric(&[], 0);
        assert_eq!(metric.kind, "idle");
    }
}
