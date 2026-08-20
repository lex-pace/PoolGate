//! Token Monitor 命令层（03 §5 全部命令）。
//!
//! W0：全部命令桩，返回 `Ok(Default)`/`Ok(vec![])`/`Ok(())`，可被前端 invoke 而不 panic。
//! 函数体由各包填充（W1：查询类；W4：额度类；W5：托盘主指标），签名**冻结**，不得改动。
//! 注册见 `lib.rs` 的 `// ==== token_monitor commands ====` 锚点。

use std::sync::Arc;

use tauri::State;

use crate::token_monitor::model::*;
use crate::token_monitor::quota::ProviderDescriptor;
use crate::AppState;

// —— 查询 ——

#[tauri::command]
pub async fn get_token_monitor_snapshot(
    state: State<'_, Arc<AppState>>,
    filters: SnapshotFilters,
) -> Result<TokenMonitorSnapshot, String> {
    let range = if filters.range.is_empty() {
        "day"
    } else {
        filters.range.as_str()
    };
    let conn = &state.db.conn;
    let (input, output, cache, total) = state.db.usage_events.unified_range_stats(conn, range)?;
    let top_tools: Vec<ToolUsageRow> = state
        .db
        .usage_events
        .tool_usage_rows(conn, range)?
        .into_iter()
        .take(8)
        .collect();
    let top_models: Vec<ModelUsageRow> = state
        .db
        .usage_events
        .model_usage_rows(conn, range)?
        .into_iter()
        .take(8)
        .collect();
    let recent_projects = state.db.projects.project_rows(conn, range)?;
    let total_cost = state
        .db
        .usage_events
        .total_cost(conn, range)
        .unwrap_or(None);

    // 采集状态聚合：任一工具 error → error；任一 active → active；否则 idle
    let collector_state = {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let worst: String = conn
            .query_row(
                "SELECT collector_status FROM tool_definition
                 ORDER BY CASE collector_status
                     WHEN 'error' THEN 3 WHEN 'format_changed' THEN 3 WHEN 'permission' THEN 2
                     WHEN 'active' THEN 1 ELSE 0 END DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "idle".to_string());
        serde_json::from_str::<CollectorStatus>(&format!("\"{worst}\""))
            .unwrap_or(CollectorStatus::Idle)
    };

    // 最紧张额度（<=20% 才作为 critical 提示；其余不展示）
    let critical_quota = {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT account_id, provider_id, window_type, remaining_percent, resets_at
             FROM quota_window_snapshot
             WHERE remaining_percent IS NOT NULL AND remaining_percent <= 20
             ORDER BY remaining_percent ASC LIMIT 1",
            [],
            |row| {
                Ok(CriticalQuota {
                    account_id: row.get(0)?,
                    provider_id: row.get(1)?,
                    window_type:
                        serde_json::from_str::<crate::token_monitor::quota::QuotaWindowType>(
                            &format!("\"{}\"", row.get::<_, String>(2)?),
                        )
                        .unwrap_or(crate::token_monitor::quota::QuotaWindowType::Monthly),
                    remaining_percent: row.get(3)?,
                    resets_at: row.get(4)?,
                })
            },
        )
        .ok()
    };

    Ok(TokenMonitorSnapshot {
        updated_at: chrono::Utc::now().to_rfc3339(),
        range: filters.range,
        collector_state,
        active_tools: top_tools.len() as i64,
        usage: UsageTotals {
            input_tokens: input,
            output_tokens: output,
            cache_tokens: cache,
            total_tokens: total,
            cost_amount: total_cost,
        },
        top_tools,
        top_models,
        recent_projects,
        critical_quota,
    })
}

#[tauri::command]
pub async fn list_tool_usage(
    state: State<'_, Arc<AppState>>,
    filters: UsageFilters,
) -> Result<Vec<ToolUsageRow>, String> {
    let range = if filters.range.is_empty() {
        "day"
    } else {
        filters.range.as_str()
    };
    state.db.usage_events.tool_usage_rows(&state.db.conn, range)
}

#[tauri::command]
pub async fn list_model_usage(
    state: State<'_, Arc<AppState>>,
    filters: UsageFilters,
) -> Result<Vec<ModelUsageRow>, String> {
    let range = if filters.range.is_empty() {
        "day"
    } else {
        filters.range.as_str()
    };
    state
        .db
        .usage_events
        .model_usage_rows(&state.db.conn, range)
}

#[tauri::command]
pub async fn list_active_sessions(
    state: State<'_, Arc<AppState>>,
    filters: UsageFilters,
) -> Result<Vec<SessionSummary>, String> {
    state.db.sessions.list_sessions(
        &state.db.conn,
        filters.tool_id.as_deref(),
        filters.project_id.as_deref(),
        None,
        Some(filters.range.as_str()),
    )
}

/// 会话逐轮用量明细（W7 会话下钻，对齐开源 Token Monitor 的 sessionDetail）。
/// 只返回 usage_event 元数据（时间/模型/token/成本），不读正文（隐私红线）。
#[tauri::command]
pub async fn list_session_events(
    state: State<'_, Arc<AppState>>,
    session_id: String,
) -> Result<Vec<SessionEventRow>, String> {
    state
        .db
        .usage_events
        .session_event_rows(&state.db.conn, &session_id)
}

#[tauri::command]
pub async fn list_projects(
    state: State<'_, Arc<AppState>>,
    filters: UsageFilters,
) -> Result<Vec<ProjectRow>, String> {
    let range = if filters.range.is_empty() {
        "day"
    } else {
        filters.range.as_str()
    };
    state.db.projects.project_rows(&state.db.conn, range)
}

#[tauri::command]
pub async fn get_usage_trend(
    state: State<'_, Arc<AppState>>,
    filters: UsageFilters,
) -> Result<TrendSeries, String> {
    // async：`trend_series` 可能运行 ~30s 的 `tokscale graph` 子进程，
    // 必须放到异步运行时（后台线程），否则会卡死主线程 / 冻结 UI。
    state
        .db
        .usage_events
        .trend_series(&state.db.conn, &filters.range)
}

#[tauri::command]
pub async fn list_devices(state: State<'_, Arc<AppState>>) -> Result<Vec<DeviceRow>, String> {
    let conn = &state.db.conn;
    let mut devices: Vec<DeviceRow> = {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT device_id, COALESCE(SUM(total_tokens),0), SUM(cost_amount)
                 FROM usage_event GROUP BY device_id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<f64>>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        rows.into_iter()
            .map(|(device_id, tokens, cost)| DeviceRow {
                label: if device_id == "local" {
                    "本机".to_string()
                } else {
                    device_id.clone()
                },
                device_id,
                total_tokens: tokens,
                cost_amount: cost,
            })
            .collect()
    };

    // Gateway 作为内建设备（request_logs final row 总量）
    if let Ok((_i, _o, _c, total)) = state.db.usage_events.unified_range_stats(conn, "total") {
        devices.push(DeviceRow {
            device_id: "gateway".into(),
            label: "PoolGate 网关".into(),
            total_tokens: total,
            cost_amount: None,
        });
    }
    Ok(devices)
}

// —— 采集控制 ——

#[tauri::command]
pub async fn get_collector_status(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<ToolCollectorState>, String> {
    crate::token_monitor::service_collect::collector_states(&state)
}

#[tauri::command]
pub fn set_tool_collection(
    state: State<'_, Arc<AppState>>,
    tool_id: String,
    enabled: bool,
) -> Result<(), String> {
    let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE tool_definition SET enabled=?1, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE tool_id=?2",
        rusqlite::params![enabled as i64, tool_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn set_tool_paths(
    state: State<'_, Arc<AppState>>,
    tool_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let json = serde_json::to_string(&paths).map_err(|e| e.to_string())?;
    let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE tool_definition SET custom_paths_json=?1, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE tool_id=?2",
        rusqlite::params![json, tool_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn rescan_tool(state: State<'_, Arc<AppState>>, tool_id: String) -> Result<(), String> {
    crate::token_monitor::service_collect::rescan_tool(&state, &tool_id);
    Ok(())
}

/// 重置某工具的全部数据：删除旧事件 + 重建 rollup + 重新采集。
/// 用于适配器逻辑变更后清理旧数据（如 DSH cacheReadTokens 口径变更）。
#[tauri::command]
pub async fn reset_tool_data(
    state: State<'_, Arc<AppState>>,
    tool_id: String,
) -> Result<(), String> {
    crate::token_monitor::service_collect::reset_tool_data(&state, &tool_id)
}

/// 首次「扫描本机」。
#[tauri::command]
pub async fn scan_all_tools(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<ToolCollectorState>, String> {
    crate::token_monitor::service_collect::scan_all_tools(&state)
}

/// 通用本机 Agent 工具扫描（一键扫描添加）：返回全部已知工具的安装/数据/监控状态。
#[tauri::command]
pub async fn detect_local_agents(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<DetectedAgent>, String> {
    crate::token_monitor::detect::detect_local_agents(&state)
}

/// 一键添加：为所选本机 Agent 工具注册适配器并立即开始采集（加入 TOKENS 监控）。
/// 返回更新后的采集状态列表（前端据此刷新「Token 监控应用」分区）。
#[tauri::command]
pub async fn enable_tool_monitoring(
    state: State<'_, Arc<AppState>>,
    tool_ids: Vec<String>,
) -> Result<Vec<ToolCollectorState>, String> {
    crate::token_monitor::detect::enable_tool_monitoring(&state, &tool_ids)
}

/// 新增自定义应用（自定义应用监控）：应用名 + JSONL 日志路径（+ 可选字段映射）。
/// 返回新 tool_id（`custom:<slug>-<suffix>`），采集立即执行。
#[tauri::command]
pub async fn add_custom_app(
    state: State<'_, Arc<AppState>>,
    input: AddCustomAppInput,
) -> Result<String, String> {
    crate::token_monitor::service_collect::add_custom_app(
        &state,
        &input.display_name,
        &input.paths,
        input.fields.as_ref(),
    )
}

/// 删除自定义应用（连同其 usage_event / 会话记录级联删除）。
#[tauri::command]
pub async fn remove_custom_app(
    state: State<'_, Arc<AppState>>,
    tool_id: String,
) -> Result<(), String> {
    crate::token_monitor::service_collect::remove_custom_app(&state, &tool_id)
}

/// 实时 Token 速率（托盘 Logo 点击展示）。
///
/// 对齐开源 Token Monitor 口径：优先用「当日 tokscale 快照」的 timed 性能计算——
///   speed（tokens/s）= timedOutputTokens * 1000 / timedDurationMs
///   burn （tokens/min）= timedTokens * 60000 / timedDurationMs
/// 快照缺失或当日无带时长的条目时，回退近 60s/1h 的 usage_event 墙钟窗口。
#[tauri::command]
pub async fn get_token_rate(state: State<'_, Arc<AppState>>) -> Result<TokenRateView, String> {
    let (duration_ms, timed_tokens, timed_output) = state
        .db
        .usage_events
        .period_timed_performance(&state.db.conn, "day");
    if duration_ms > 0 {
        return Ok(TokenRateView {
            tokens_per_sec: if timed_output > 0 {
                Some(timed_output as f64 * 1000.0 / duration_ms as f64)
            } else {
                None
            },
            tokens_per_min: if timed_tokens > 0 {
                Some(timed_tokens as f64 * 60000.0 / duration_ms as f64)
            } else {
                None
            },
            window_secs: 60,
            observed_at: None,
        });
    }
    let (out_60s, total_1h, observed) = state.db.usage_events.token_rate_window(&state.db.conn)?;
    let tokens_per_sec = out_60s.map(|n| n as f64 / 60.0);
    let tokens_per_min = total_1h.map(|n| n as f64 / 60.0);
    Ok(TokenRateView {
        tokens_per_sec,
        tokens_per_min,
        window_secs: 60,
        observed_at: observed,
    })
}

/// 手动「立即刷新」：清空 checkpoint 强制全量重扫所有工具（托盘刷新按钮）。
#[tauri::command]
pub async fn refresh_token_monitor(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<ToolCollectorState>, String> {
    crate::token_monitor::service_collect::force_refresh_all(&state)
}

// —— 额度账号 ——

/// 按账号聚合的用量统计（额度卡 2×2 TOKEN 统计格数据源）。
///
/// key 与 `list_quota_accounts` 视图一致：网关账号 → `gw:{id}`，TM 额度账号
/// （`tm_*`）经 `linked_route_account_id` 关联到网关账号。数据源 = request_logs
/// （网关真实流量，最终请求去重口径）；`usage_event` 本地工具用量无账号归因，不参与。
/// 无流量数据的账号不返回（前端诚实展示 —）。
#[tauri::command]
pub fn get_account_token_stats(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<AccountUsageStat>, String> {
    use std::collections::HashMap;

    let buckets = state.db.logs.account_token_buckets(&state.db.conn)?;
    if buckets.is_empty() {
        return Ok(Vec::new());
    }
    // gateway 账号 id → (today, yesterday, week, month, total, requests)
    let by_gateway: HashMap<String, (i64, i64, i64, i64, i64, i64)> = buckets
        .into_iter()
        .map(|(id, t, y, w, m, tot, r)| (id, (t, y, w, m, tot, r)))
        .collect();

    // 视图 key → 网关账号 id（与 list_quota_accounts 同 key 口径）
    let mut view_keys: Vec<(String, String)> = Vec::new();
    {
        let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
        // 网关账号：accounts.id → gw:{id}
        let mut stmt = conn
            .prepare("SELECT id FROM accounts WHERE status IS NULL OR status != 'disabled'")
            .map_err(|e| e.to_string())?;
        let ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        for id in ids {
            view_keys.push((format!("gw:{id}"), id));
        }
        // TM 额度账号：linked_route_account_id → tm_*
        let mut stmt = conn
            .prepare(
                "SELECT account_id, linked_route_account_id FROM quota_account
                 WHERE linked_route_account_id IS NOT NULL AND linked_route_account_id != ''",
            )
            .map_err(|e| e.to_string())?;
        let linked: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        for (tm_id, gw_id) in linked {
            view_keys.push((tm_id, gw_id));
        }
    }

    let mut out: Vec<AccountUsageStat> = Vec::new();
    for (view_id, gw_id) in view_keys {
        if let Some(&(today, yesterday, week, month, total, requests)) = by_gateway.get(&gw_id) {
            if requests > 0 {
                out.push(AccountUsageStat {
                    account_id: view_id,
                    today_tokens: today,
                    yesterday_tokens: yesterday,
                    week_tokens: week,
                    month_tokens: month,
                    total_tokens: total,
                    request_count: requests,
                });
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub fn list_quota_accounts(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<QuotaAccountView>, String> {
    let mut views = state.db.quota_accounts.list_accounts(&state.db.conn)?;

    // W7：把网关侧账号并入额度视图，让托盘/桌面额度卡展示真实可用的额度数据。
    // 优先展示 **OAuth 授权登录账号**（OpenAI/ChatGPT、Gemini 等）：
    //  - OpenAI/ChatGPT（codex_oauth）：account_usage 已存官方 wham 接口拉到的
    //    5 小时额度 + 周额度窗口（primary/secondary），plan_type 即 Token Plan；
    //  - Gemini（gemini_oauth）：Google 未提供公开额度接口 → 账号可见、窗口诚实标注「不可用」；
    // 其次展示其余网关账号的 quota_limit/quota_used（用户配置上限 + 路由用量累计）。
    // 全部为真实数据，无伪造。
    let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.provider_id, a.name, a.quota_limit, a.quota_used, a.status,
                    a.credential_type, a.email,
                    u.plan_type, u.quota_windows, u.last_refreshed_at,
                    p.name AS provider_label, p.base_url AS provider_base_url
             FROM accounts a
             LEFT JOIN account_usage u ON u.account_id = a.id
             LEFT JOIN providers p ON p.id = a.provider_id
             WHERE a.status IS NULL OR a.status != 'disabled'
             ORDER BY a.credential_type IS NULL, COALESCE(a.name, a.provider_id, a.id)",
        )
        .map_err(|e| e.to_string())?;
    #[allow(clippy::type_complexity)]
    let gateway_rows: Vec<(
        String,
        Option<String>,
        Option<String>,
        Option<f64>,
        Option<f64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
                row.get(10)?,
                row.get(11)?,
                row.get(12)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    for (
        id,
        provider_id,
        name,
        quota_limit,
        quota_used,
        status,
        credential_type,
        email,
        plan_type,
        quota_windows_json,
        last_refreshed_at,
        provider_label,
        provider_base_url,
    ) in gateway_rows
    {
        let is_oauth = matches!(
            credential_type.as_deref(),
            Some("oauth") | Some("codex_oauth") | Some("gemini_oauth") | Some("claude_oauth")
        );
        let oauth_windows = quota_windows_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<Vec<serde_json::Value>>(json).ok());

        // DeepSeek 网关账号：余额由后台 refresh_deepseek_quota 拉取，存为
        // quota_window_snapshot JSON（prepaid_balance / currency），直接映射为额度视图。
        let deepseek_snapshots = if crate::token_monitor::quota::deepseek::is_deepseek(
            provider_id.as_deref(),
            provider_label.as_deref(),
            provider_base_url.as_deref(),
        ) {
            quota_windows_json
                .as_deref()
                .and_then(|json| {
                    serde_json::from_str::<Vec<crate::token_monitor::quota::QuotaWindowSnapshot>>(
                        json,
                    )
                    .ok()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let windows: Vec<QuotaWindowView> = if !deepseek_snapshots.is_empty() {
            deepseek_snapshots.into_iter().map(Into::into).collect()
        } else if is_oauth {
            match oauth_windows {
                Some(list) if !list.is_empty() => list
                    .iter()
                    .filter_map(|w| oauth_window_to_view(w, last_refreshed_at.as_deref()))
                    .collect(),
                // Gemini 等无公开额度接口的 OAuth 账号：账号可见 + 诚实标注不可用
                _ => vec![QuotaWindowView {
                    window_key: "unavailable".into(),
                    window_type: crate::token_monitor::quota::QuotaWindowType::Billing,
                    unit: crate::token_monitor::quota::QuotaUnit::Percent,
                    label: "额度接口不可用（供应商未提供公开接口）".into(),
                    used_value: None,
                    limit_value: None,
                    remaining_value: None,
                    remaining_percent: None,
                    resets_at: None,
                    period_started_at: None,
                    source: crate::token_monitor::quota::QuotaSource::OfficialApi,
                    confidence: crate::token_monitor::quota::QuotaConfidence::Derived,
                    fetched_at: last_refreshed_at.clone().unwrap_or_else(now_iso),
                    error_code: Some("unsupported".into()),
                }],
            }
        } else {
            let limit = quota_limit.filter(|l| *l > 0.0);
            let used = quota_used.unwrap_or(0.0);
            let (remaining_value, remaining_percent) = match limit {
                Some(l) => {
                    let rem = (l - used).max(0.0);
                    (Some(rem), Some((rem / l * 100.0).clamp(0.0, 100.0)))
                }
                None => (None, None),
            };
            vec![QuotaWindowView {
                window_key: "gateway".into(),
                window_type: crate::token_monitor::quota::QuotaWindowType::Billing,
                // quota_limit/quota_used 是网关配置上限与已用量（tokens/额度，单位随配置），
                // 用百分比口径展示，避免 unit=currency 被前端误当“账户余额”而显示伪造金额。
                unit: crate::token_monitor::quota::QuotaUnit::Percent,
                label: "账号额度（网关配置）".into(),
                used_value: Some(used),
                limit_value: limit,
                remaining_value,
                remaining_percent,
                resets_at: None,
                period_started_at: None,
                source: crate::token_monitor::quota::QuotaSource::LocalAuth,
                confidence: crate::token_monitor::quota::QuotaConfidence::Reported,
                fetched_at: chrono::Utc::now().to_rfc3339(),
                error_code: None,
            }]
        };
        views.push(QuotaAccountView {
            account_id: format!("gw:{id}"),
            provider_id: provider_id.unwrap_or_else(|| "gateway".into()),
            provider_label,
            label: name,
            identity_masked: email,
            // Token Plan：ChatGPT OAuth 的 plan_type（chatgpt_plan_type）等
            plan_name: plan_type,
            status: status.unwrap_or_else(|| "active".into()),
            enabled: true,
            last_success_at: last_refreshed_at,
            windows,
        });
    }
    Ok(views)
}

/// 把网关 `account_usage.quota_windows` 里的一个窗口（`QuotaWindow` JSON）映射为额度视图。
/// key：primary → 5 小时滚动额度，secondary → 周额度，其余 → 月度；单位 percent。
fn oauth_window_to_view(
    w: &serde_json::Value,
    fetched_at: Option<&str>,
) -> Option<QuotaWindowView> {
    let key = w.get("key")?.as_str()?.to_string();
    let label = w
        .get("label")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "额度".into());
    let used = w.get("used_percent")?.as_f64()?.clamp(0.0, 100.0);
    let remaining = w
        .get("remaining_percent")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| (100.0 - used).max(0.0))
        .clamp(0.0, 100.0);
    let resets_at = w.get("reset_at").and_then(|v| v.as_i64()).and_then(|secs| {
        chrono::DateTime::from_timestamp(secs, 0)
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    });
    let window_type = match key.as_str() {
        "primary" => crate::token_monitor::quota::QuotaWindowType::Rolling5h,
        "secondary" => crate::token_monitor::quota::QuotaWindowType::Weekly,
        _ => crate::token_monitor::quota::QuotaWindowType::Monthly,
    };
    Some(QuotaWindowView {
        window_key: key,
        window_type,
        unit: crate::token_monitor::quota::QuotaUnit::Percent,
        label,
        used_value: Some(used),
        limit_value: Some(100.0),
        remaining_value: Some(remaining),
        remaining_percent: Some(remaining),
        resets_at,
        period_started_at: None,
        source: crate::token_monitor::quota::QuotaSource::OfficialApi,
        confidence: crate::token_monitor::quota::QuotaConfidence::Reported,
        fetched_at: fetched_at.map(str::to_string).unwrap_or_else(now_iso),
        error_code: None,
    })
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[tauri::command]
pub fn list_quota_providers(
    _state: State<'_, Arc<AppState>>,
) -> Result<Vec<ProviderDescriptor>, String> {
    use crate::token_monitor::quota::ConnectorRegistry;
    let mut providers = Vec::new();
    for provider_id in crate::token_monitor::quota::PROVIDER_IDS {
        if let Some(connector) = ConnectorRegistry::get(provider_id) {
            providers.push(connector.provider());
        }
    }
    Ok(providers)
}

#[tauri::command]
pub async fn add_quota_account(
    state: State<'_, Arc<AppState>>,
    input: AddQuotaAccountInput,
) -> Result<QuotaAccountView, String> {
    use crate::token_monitor::quota::{refresh_account, ConnectorRegistry};

    let connector = ConnectorRegistry::get(&input.provider_id)
        .ok_or_else(|| format!("未知额度 Provider：{}", input.provider_id))?;
    let account_id = format!("tm_{}", uuid::Uuid::new_v4().simple());

    // 凭证只进 Keychain，DB 只存引用；明文仅传输一次，立即丢弃
    let credential_ref =
        if let Some(payload) = input.credential_payload.filter(|p| !p.trim().is_empty()) {
            let key = format!("tm.quota.{account_id}");
            crate::services::keychain::store_verified(&key, payload.trim().as_bytes())
                .map_err(|e| format!("写入 Keychain 失败：{e}"))?;
            Some(key)
        } else {
            None
        };
    if credential_ref.is_none() && input.linked_route_account_id.is_none() {
        return Err("请提供 API Key/凭证，或关联一个已有路由账号".into());
    }

    // 校验凭证（validate）；失败则回滚已写入的 Keychain 项
    let auth_method = input.auth_method.clone();
    let profile = match credential_ref.as_deref() {
        Some(credential) => connector.validate(credential).await.map_err(|error| {
            if let Some(key) = &credential_ref {
                let _ = crate::services::keychain::delete_secret(key);
            }
            format!("凭证校验失败：{error}")
        })?,
        None => crate::token_monitor::quota::AccountProfile {
            identity_masked: "路由账号关联".into(),
            plan_name: None,
        },
    };

    let enabled = true;
    state.db.quota_accounts.upsert_account(
        &state.db.conn,
        &account_id,
        &input.provider_id,
        input.label.as_deref(),
        Some(profile.identity_masked.as_str()),
        profile.plan_name.as_deref(),
        &auth_method,
        credential_ref.as_deref(),
        input.linked_route_account_id.as_deref(),
        enabled,
        "active",
    )?;

    // 首次拉取额度（失败不阻断绑定，状态如实反映）
    let _ = refresh_account(&state, &account_id).await;

    let views = state.db.quota_accounts.list_accounts(&state.db.conn)?;
    views
        .into_iter()
        .find(|view| view.account_id == account_id)
        .ok_or_else(|| "额度账号写入后未读回".into())
}

#[tauri::command]
pub async fn refresh_quota_account(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> Result<QuotaAccountView, String> {
    crate::token_monitor::quota::refresh_account(&state, &account_id).await?;
    let views = state.db.quota_accounts.list_accounts(&state.db.conn)?;
    views
        .into_iter()
        .find(|view| view.account_id == account_id)
        .ok_or_else(|| format!("额度账号不存在：{account_id}"))
}

#[tauri::command]
pub fn remove_quota_account(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> Result<(), String> {
    // M4 关键验收：删快照（级联）→ 清 Keychain 凭证 → 删账号
    let credential_ref: Option<String> = {
        let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT credential_ref FROM quota_account WHERE account_id=?1",
            rusqlite::params![account_id],
            |row| row.get(0),
        )
        .ok()
    };
    state
        .db
        .quota_windows
        .delete_for_account(&state.db.conn, &account_id)?;
    state
        .db
        .quota_accounts
        .delete_account(&state.db.conn, &account_id)?;
    if let Some(key) = credential_ref {
        let _ = crate::services::keychain::delete_secret(&key);
    }
    Ok(())
}

#[tauri::command]
pub fn set_quota_alert_thresholds(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    thresholds: AlertThresholds,
) -> Result<(), String> {
    let json = serde_json::to_string(&thresholds).map_err(|e| e.to_string())?;
    let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE quota_account SET alert_thresholds_json=?1, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE account_id=?2",
        rusqlite::params![json, account_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// —— 状态 ——

/// 各供应商服务状态（Claude / OpenAI / Cursor / DeepSeek 状态页，带 60s 缓存）。
#[tauri::command]
pub async fn get_service_status() -> Vec<ServiceStatusView> {
    crate::token_monitor::status::fetch_service_statuses().await
}

// —— 托盘 ——

#[tauri::command]
pub fn get_token_monitor_tray_snapshot(
    _state: State<'_, Arc<AppState>>,
    range: String,
) -> Result<TokenMonitorTraySnapshot, String> {
    let mut snapshot = TokenMonitorTraySnapshot::default();
    snapshot.range = range;
    Ok(snapshot)
}

#[tauri::command]
pub async fn get_tray_primary_metric(
    state: State<'_, Arc<AppState>>,
) -> Result<TrayPrimaryMetric, String> {
    // W5：额度优先（最低剩余窗口），其次今日 Tokens，否则就绪。
    let windows = state.db.quota_windows.list_all_snapshots(&state.db.conn)?;
    let today_tokens = state
        .db
        .usage_events
        .unified_range_stats(&state.db.conn, "day")
        .map(|(_, _, _, total)| total)
        .unwrap_or(0);
    Ok(crate::token_monitor::alerts::tray_primary_metric(
        &windows,
        today_tokens,
    ))
}
