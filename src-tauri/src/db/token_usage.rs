//! usage_event 写入与聚合（W1 实现）。
//!
//! 独立口径：Token Monitor 只统计本地工具用量（`usage_event`，由 tokscale 聚合引擎
//! 采集）。网关流量（`request_logs`）由网关仪表盘独立统计，两个功能数据源完全分离、
//! 不重复 —— 用户决策：网关是主功能，Token Monitor 是附带小工具。
//! 写入批量、离请求热路径、同一事务内增量更新 rollup。

use rusqlite::Connection;
use std::sync::Mutex;

use crate::token_monitor::dedup;
use crate::token_monitor::model::{
    CollectorStatus, ModelUsageRow, NormalizedUsageEvent, SeriesSplit, SessionEventRow, SupportLevel,
    ToolUsageRow, TrendDay, TrendMonth, TrendSeries,
};
use crate::token_monitor::normalization;
use crate::token_monitor::pricing;

pub struct UsageEventRepo;

/// 工具分组查询行（(tool, model) 维度，含展示元数据与成本），供工具行聚合复用。
type ToolGroupRow = (String, String, String, String, String, i64, i64, i64, i64, Option<f64>);
/// 模型分组查询行（模型维度，含成本）。
type ModelGroupRow = (String, i64, i64, i64, i64, Option<f64>);

/// Token Monitor 统一口径 CTE：仅本地工具用量（usage_event）。
/// 网关流量（request_logs）由网关仪表盘独立统计，二者数据源分离、不重复。
/// 只统计**仍在监控中**（tool_definition.enabled=1）的工具——关闭监控的工具
/// 及其模型从总量/工具/模型/会话/趋势等所有统计中剔除（历史数据保留，
/// 重新开启即恢复显示）。
/// 列：tool_id, model_normalized, input_tokens, output_tokens, cache_tokens, total_tokens,
/// cost_amount, occurred_at。
const UNIFIED_CTE: &str = "
WITH unified AS (
    SELECT tool_id, model_normalized,
           input_tokens, output_tokens,
           (COALESCE(cache_read_tokens,0)+COALESCE(cache_write_tokens,0)) AS cache_tokens,
           total_tokens, cost_amount, occurred_at
    FROM usage_event
    WHERE tool_id IN (SELECT tool_id FROM tool_definition WHERE enabled=1)
)";

/// 已关闭监控（enabled=0）的 tool_id 集合（统计剔除用）。
pub(crate) fn disabled_tool_ids(
    conn: &Mutex<Connection>,
) -> std::collections::HashSet<String> {
    let Ok(conn) = conn.lock() else {
        return std::collections::HashSet::new();
    };
    let mut stmt = match conn.prepare("SELECT tool_id FROM tool_definition WHERE enabled=0") {
        Ok(stmt) => stmt,
        Err(_) => return std::collections::HashSet::new(),
    };
    let ids = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .ok()
        .and_then(|rows| rows.collect::<Result<std::collections::HashSet<_>, _>>().ok())
        .unwrap_or_default();
    drop(stmt);
    ids
}

/// 读取 tokscale 权威 period 快照(settings 表,采集时刷新)。
/// day/7d/month 优先用 tokscale 自身口径(--today/--week/--month),避免 mtime 归因虚高;
/// total 无快照(全量口径在 usage_event 已是权威,见 range_boundary 回退)。
fn period_snapshot_json(conn: &Mutex<Connection>, range: &str) -> Option<serde_json::Value> {
    if range != "day" && range != "7d" && range != "month" {
        return None;
    }
    crate::db::settings::SettingsRepo
        .get(conn, &format!("tm.period.{range}"))
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
}

/// 从 tokscale 快照取 (input, output, cache, total, cost)。
/// 关闭监控的工具（enabled=0）从 entries 中剔除后再汇总；无 entries 的旧快照
/// 回退读顶层总量（无法按工具过滤，历史格式兜底）。
fn period_totals(
    conn: &Mutex<Connection>,
    snap: &serde_json::Value,
) -> (i64, i64, i64, i64, Option<f64>) {
    let n = |k: &str| snap.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    let Some(entries) = snap.get("entries").and_then(|v| v.as_array()) else {
        let input = n("totalInput");
        let output = n("totalOutput");
        let cache_read = n("totalCacheRead");
        let cache_write = n("totalCacheWrite");
        let cost = snap.get("totalCost").and_then(|v| v.as_f64());
        // 对齐开源 total = input + output + cacheRead + cacheWrite
        return (
            input,
            output,
            cache_read + cache_write,
            input + output + cache_read + cache_write,
            cost,
        );
    };
    let disabled = disabled_tool_ids(conn);
    let mut input = 0i64;
    let mut output = 0i64;
    let mut cache_read = 0i64;
    let mut cache_write = 0i64;
    let mut cost: Option<f64> = None;
    for e in entries {
        let client = e.get("client").and_then(|v| v.as_str()).unwrap_or("");
        let tool_id = crate::token_monitor::collector::tokscale::tool_id_for_client(client);
        if disabled.contains(&tool_id) {
            continue;
        }
        input += e.get("input").and_then(|v| v.as_i64()).unwrap_or(0);
        output += e.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
        cache_read += e.get("cacheRead").and_then(|v| v.as_i64()).unwrap_or(0);
        cache_write += e.get("cacheWrite").and_then(|v| v.as_i64()).unwrap_or(0);
        if let Some(c) = e.get("cost").and_then(|v| v.as_f64()) {
            cost = Some(cost.unwrap_or(0.0) + c);
        }
    }
    (
        input,
        output,
        cache_read + cache_write,
        input + output + cache_read + cache_write,
        cost,
    )
}

/// 对齐开源 Token Monitor：从 tokscale 快照 entries 聚合「timed 性能」。
/// 返回 (timedDurationMs, timedTokens, timedOutputTokens)。
/// 语义与开源 usage.js 一致：
///   timedDurationMs   = Σ entry.performance.totalDurationMs
///   timedTokens       = Σ entry.performance.timedTokens
///   timedOutputTokens = 仅对带生成时长(totalDurationMs>0)的条目累计 output。
fn timed_performance_from_snap(snap: &serde_json::Value) -> (i64, i64, i64) {
    let mut duration_ms = 0i64;
    let mut timed_tokens = 0i64;
    let mut timed_output = 0i64;
    let Some(entries) = snap.get("entries").and_then(|v| v.as_array()) else {
        return (0, 0, 0);
    };
    for e in entries {
        let perf = e.get("performance");
        let entry_duration = perf
            .and_then(|v| v.get("totalDurationMs"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);
        let entry_timed = perf
            .and_then(|v| v.get("timedTokens"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);
        let output = e.get("output").and_then(|v| v.as_i64()).unwrap_or(0).max(0);
        duration_ms += entry_duration;
        timed_tokens += entry_timed;
        if entry_duration > 0 {
            timed_output += output;
        }
    }
    (duration_ms, timed_tokens, timed_output)
}

/// tokscale 快照 entries 按指定 key 聚合（单趟，容错跳过缺 client/model 的条目）。
/// 返回 (key → 聚合) + 全部 total 之和。
fn period_entry_aggregate<F: Fn(&serde_json::Value) -> String>(
    entries: &[&serde_json::Value],
    key_of: F,
) -> (std::collections::HashMap<String, PeriodAgg>, i64) {
    use std::collections::HashMap;
    let mut acc: HashMap<String, PeriodAgg> = HashMap::new();
    for e in entries {
        let input = e.get("input").and_then(|v| v.as_i64()).unwrap_or(0);
        let output = e.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
        let cache_r = e.get("cacheRead").and_then(|v| v.as_i64()).unwrap_or(0);
        let cache_w = e.get("cacheWrite").and_then(|v| v.as_i64()).unwrap_or(0);
        if input == 0 && output == 0 && cache_r == 0 && cache_w == 0 {
            continue;
        }
        let cost = e.get("cost").and_then(|v| v.as_f64());
        let agg = acc.entry(key_of(e)).or_default();
        agg.input += input;
        agg.output += output;
        agg.cache += cache_r + cache_w;
        agg.total += input + output + cache_r + cache_w;
        if let Some(c) = cost {
            agg.cost = Some(agg.cost.unwrap_or(0.0) + c);
        }
    }
    let grand: i64 = acc.values().map(|a| a.total).sum();
    (acc, grand)
}

#[derive(Default)]
struct PeriodAgg {
    input: i64,
    output: i64,
    cache: i64,
    total: i64,
    cost: Option<f64>,
}

/// 从 tokscale 快照 entries 聚合工具行（按 client → tool_id）。
/// 关闭监控的工具（enabled=0）从 entries 中剔除后再聚合。
fn period_tool_rows(conn: &Mutex<Connection>, range: &str) -> Option<Vec<ToolUsageRow>> {
    let snap = period_snapshot_json(conn, range)?;
    let entries = snap.get("entries")?.as_array()?;
    let entries = filter_entries_by_enabled(conn, entries);
    let (acc, grand) = period_entry_aggregate(&entries, |e| {
        let client = e.get("client").and_then(|v| v.as_str()).unwrap_or("");
        crate::token_monitor::collector::tokscale::tool_id_for_client(client)
    });
    let mut rows: Vec<(String, PeriodAgg)> = acc.into_iter().collect();
    rows.sort_by(|a, b| b.1.total.cmp(&a.1.total));
    Some(
        rows.into_iter()
            .map(|(tool_id, a)| ToolUsageRow {
                tool_id: tool_id.clone(),
                display_name: display_name_for_tool(&tool_id),
                support_level: SupportLevel::Standard,
                collector_status: CollectorStatus::Active,
                input_tokens: a.input,
                output_tokens: a.output,
                cache_tokens: a.cache,
                total_tokens: a.total,
                cost_amount: a.cost,
                share_percent: if grand > 0 {
                    a.total as f64 * 100.0 / grand as f64
                } else {
                    0.0
                },
            })
            .collect(),
    )
}

/// 从 tokscale 快照 entries 聚合模型行（按 model）。
/// 关闭监控的工具（enabled=0）从 entries 中剔除后再聚合。
fn period_model_rows(conn: &Mutex<Connection>, range: &str) -> Option<Vec<ModelUsageRow>> {
    let snap = period_snapshot_json(conn, range)?;
    let entries = snap.get("entries")?.as_array()?;
    let entries = filter_entries_by_enabled(conn, entries);
    let (acc, grand) = period_entry_aggregate(&entries, |e| {
        // tokscale 快照里的模型名是原始值；应用别名归一，让 MAAS 端点名
        // （如 maas_cl_opus_4.8_20260528_cache）与 usage_event 口径一致，避免
        // 同一模型在「今日/近7天/本月」模型卡片里裂成多张。
        let raw = e
            .get("model")
            .and_then(|v| v.as_str())
            .filter(|m| !m.is_empty())
            .unwrap_or("未知模型");
        normalization::alias_model(raw).unwrap_or(raw).to_string()
    });
    let mut rows: Vec<(String, PeriodAgg)> = acc.into_iter().collect();
    rows.sort_by(|a, b| b.1.total.cmp(&a.1.total));
    Some(
        rows.into_iter()
            .map(|(model, a)| ModelUsageRow {
                model,
                input_tokens: a.input,
                output_tokens: a.output,
                cache_tokens: a.cache,
                total_tokens: a.total,
                cost_amount: a.cost,
                share_percent: if grand > 0 {
                    a.total as f64 * 100.0 / grand as f64
                } else {
                    0.0
                },
            })
            .collect(),
    )
}

/// 过滤掉已关闭监控（enabled=0）工具的 tokscale 快照条目（client → tool_id 判定）。
fn filter_entries_by_enabled<'a>(
    conn: &Mutex<Connection>,
    entries: &'a [serde_json::Value],
) -> Vec<&'a serde_json::Value> {
    let disabled = disabled_tool_ids(conn);
    entries
        .iter()
        .filter(|e| {
            let client = e.get("client").and_then(|v| v.as_str()).unwrap_or("");
            let tool_id = crate::token_monitor::collector::tokscale::tool_id_for_client(client);
            !disabled.contains(&tool_id)
        })
        .collect()
}

/// day|7d|month|total 的本地日历起始边界（不含列名，可复用于 occurred_at / last_active_at）。
pub(crate) fn range_start_sql(range: &str) -> &'static str {
    match range {
        "day" => "datetime('now','localtime','start of day')",
        "7d" => "datetime('now','localtime','start of day','-6 days')",
        "month" => "datetime('now','localtime','start of month')",
        _ => "'1970-01-01'",
    }
}

/// day|7d|month|total 的本地日历边界（usage_event 的 occurred_at 列）。
fn range_boundary(range: &str) -> String {
    format!("datetime(occurred_at,'localtime') >= {}", range_start_sql(range))
}

/// companion 工具的 tool_id 过滤条件（可指定表别名）：排除 tokscale 覆盖清单内工具
/// （其口径以 tokscale 快照为权威，避免双算）+ 聚合引擎自身，剩下即 freebuff /
/// atomcode / 自定义应用 等「tokscale 不覆盖、只能靠 usage_event 统计」的工具。
/// 清单为编译期常量，无注入风险。
fn companion_tool_condition(alias: &str) -> String {
    let ids = crate::token_monitor::collector::tokscale::covered_tool_ids();
    let quoted: Vec<String> = ids.iter().map(|id| format!("'{id}'")).collect();
    if quoted.is_empty() {
        format!("{alias}.tool_id != 'tokscale_aggregate'")
    } else {
        format!(
            "{alias}.tool_id NOT IN ({}) AND {alias}.tool_id != 'tokscale_aggregate'",
            quoted.join(",")
        )
    }
}

/// companion 工具过滤 SQL 片段（usage_event 的 `unified` CTE 口径）。
fn companion_filter_sql() -> String {
    companion_tool_condition("unified")
}

pub(crate) fn display_name_for_tool(tool_id: &str) -> String {
    match tool_id {
        // 注：无 poolgate_gateway 分支 —— 网关流量由网关仪表盘独立统计，不进 Token Monitor。
        "claude_code" => "Claude Code".to_string(),
        "freebuff" => "Freebuff".to_string(),
        "codex" => "Codex CLI".to_string(),
        "opencode" => "OpenCode".to_string(),
        "cursor" => "Cursor".to_string(),
        "github_copilot" => "GitHub Copilot".to_string(),
        "mimo" => "MiMo Code".to_string(),
        "zcode" => "ZCode".to_string(),
        "codebuddy" => "CodeBuddy".to_string(),
        "antigravity" => "Antigravity".to_string(),
        "kimi" => "Kimi".to_string(),
        "qwen" => "Qwen".to_string(),
        "grok_build" => "Grok Build".to_string(),
        "hermes" => "Hermes".to_string(),
        "zed" => "Zed Agent".to_string(),
        "kiro" => "Kiro".to_string(),
        "cline" => "Cline".to_string(),
        "kilo_code" => "Kilo Code".to_string(),
        "pi" => "Pi".to_string(),
        "proma" => "Proma".to_string(),
        "openclaw" => "OpenClaw".to_string(),
        "gemini" => "Gemini CLI".to_string(),
        _ => tool_id
            .split('_')
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

impl UsageEventRepo {
    /// 单条事件的最终化（归一模型/计算 total）→ fingerprint → 插入，返回是否新插入。
    /// 必须在事务内调用（持有连接）。
    fn insert_event_locked(
        tx: &rusqlite::Transaction<'_>,
        raw: &NormalizedUsageEvent,
    ) -> Result<bool, String> {
        let mut event = raw.clone();
        normalization::finalize(&mut event);
        let fingerprint = dedup::fingerprint(&event);
        // 注意：不做跨面去重 —— Token Monitor 与网关独立，本地事件不因
        // request_logs 中存在网关行而被丢弃（幂等由 source_fingerprint UNIQUE 保证）。
        // 确保 tool_definition 存在（FK）
        let _ = tx.execute(
            "INSERT INTO tool_definition (tool_id, display_name, kind, support_level) \
             VALUES (?1, ?2, 'usage', 'basic') \
             ON CONFLICT(tool_id) DO NOTHING",
            rusqlite::params![event.tool_id, display_name_for_tool(&event.tool_id)],
        );
        let affected = tx
            .execute(
                "INSERT OR IGNORE INTO usage_event (
                    source_type, tool_id, device_id, model_raw, model_normalized,
                    session_id, project_id, account_id, input_tokens, output_tokens,
                    cache_read_tokens, cache_write_tokens, reasoning_tokens, message_count,
                    total_tokens, cost_amount, cost_currency, usage_accuracy, occurred_at,
                    session_started_at, session_last_active_at,
                    source_fingerprint, source_locator_hash
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
                rusqlite::params![
                    serde_json::to_string(&event.source_type)
                        .unwrap_or_else(|_| "local_discovered".into())
                        .trim_matches('"'),
                    event.tool_id,
                    event.device_id,
                    event.model_raw,
                    event.model_normalized,
                    event.session_id,
                    event.project_id,
                    event.account_id,
                    event.input_tokens,
                    event.output_tokens,
                    event.cache_read_tokens,
                    event.cache_write_tokens,
                    event.reasoning_tokens,
                    event.message_count,
                    event.total_tokens,
                    event.cost_amount,
                    event.cost_currency,
                    serde_json::to_string(&event.usage_accuracy)
                        .unwrap_or_else(|_| "\"unavailable\"".into())
                        .trim_matches('"'),
                    event.occurred_at,
                    event.session_started_at,
                    event.session_last_active_at,
                    fingerprint,
                    event.source_locator_hash,
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(affected > 0)
    }

    /// 全量重建按日聚合（事务内；用于快照替换后重算 rollup）。
    fn rebuild_rollup_locked(tx: &rusqlite::Transaction<'_>) -> Result<(), String> {
        tx.execute("DELETE FROM tm_daily_rollup", [])
            .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO tm_daily_rollup (
                day, tool_id, model_normalized, input_tokens, output_tokens, cache_tokens,
                total_tokens, cost_amount, request_count
             )
             SELECT date(occurred_at,'localtime'),
                    tool_id, COALESCE(model_normalized,''),
                    COALESCE(input_tokens,0), COALESCE(output_tokens,0),
                    COALESCE(cache_read_tokens,0)+COALESCE(cache_write_tokens,0),
                    COALESCE(total_tokens,0), COALESCE(cost_amount,0), COUNT(*)
             FROM usage_event
             WHERE tool_id IN (SELECT tool_id FROM tool_definition WHERE enabled=1)
             GROUP BY 1,2,3",
            [],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 全量重建按日聚合（公开接口，用于 reset_tool_data 等外部调用）。
    pub fn rebuild_rollup(&self, conn: &Mutex<Connection>) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        Self::rebuild_rollup_locked(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 批量写入本地工具事件（幂等：按 `source_fingerprint` `INSERT OR IGNORE`）。
    ///
    /// 流程：finalize（归一模型/计算 total）→ fingerprint → 跨面去重（丢弃 gateway 重复）
    /// → INSERT OR IGNORE → 同一事务内增量更新 `tm_daily_rollup`。
    /// 单个事件失败不中断整批（隔离单点故障）。
    pub fn insert_batch(
        &self,
        conn: &Mutex<Connection>,
        events: &[NormalizedUsageEvent],
    ) -> Result<i64, String> {
        if events.is_empty() {
            return Ok(0);
        }
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

        let max_before: i64 = tx
            .query_row("SELECT COALESCE(MAX(id),0) FROM usage_event", [], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;

        let mut inserted = 0i64;
        for raw in events {
            if Self::insert_event_locked(&tx, raw)? {
                inserted += 1;
            }
        }

        // 增量更新按日聚合（仅新插入的行；同一事务）
        tx.execute(
            "INSERT INTO tm_daily_rollup (
                day, tool_id, model_normalized, input_tokens, output_tokens, cache_tokens,
                total_tokens, cost_amount, request_count
             )
             SELECT date(occurred_at,'localtime'),
                    tool_id, COALESCE(model_normalized,''),
                    COALESCE(input_tokens,0), COALESCE(output_tokens,0),
                    COALESCE(cache_read_tokens,0)+COALESCE(cache_write_tokens,0),
                    COALESCE(total_tokens,0), COALESCE(cost_amount,0), 1
             FROM usage_event WHERE id > ?1
             ON CONFLICT(day, tool_id, model_normalized) DO UPDATE SET
                input_tokens   = tm_daily_rollup.input_tokens   + excluded.input_tokens,
                output_tokens  = tm_daily_rollup.output_tokens  + excluded.output_tokens,
                cache_tokens   = tm_daily_rollup.cache_tokens   + excluded.cache_tokens,
                total_tokens   = tm_daily_rollup.total_tokens   + excluded.total_tokens,
                cost_amount    = tm_daily_rollup.cost_amount    + excluded.cost_amount,
                request_count  = tm_daily_rollup.request_count  + excluded.request_count",
            rusqlite::params![max_before],
        )
        .map_err(|e| e.to_string())?;

        tx.commit().map_err(|e| e.to_string())?;
        Ok(inserted)
    }

    /// 快照替换（tokscale 聚合引擎语义）：清空 tokscale **覆盖清单内工具**的本地采集事件后
    /// 写入新快照，并重建 rollup。
    ///
    /// 用于权威聚合源接管时，避免与旧的手写适配器数据双算。只删除 `covered_tool_ids`
    /// 清单内工具的本地采集行（`source_type='local_discovered'`），**保留 companion 工具**
    /// （tokscale 未覆盖的手写适配器，如 freebuff / atomcode、自定义应用 `custom:*`）的
    /// 事件——这些工具的用量 tokscale 快照不包含，删掉即永久丢失。用户手动导入
    /// （`imported`）的数据不受影响；Gateway 事件本就不在此表（request_logs 为权威源）。
    pub fn replace_local_snapshot(
        &self,
        conn: &Mutex<Connection>,
        events: &[NormalizedUsageEvent],
        covered_tool_ids: &[String],
    ) -> Result<i64, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        if !covered_tool_ids.is_empty() {
            let placeholders: Vec<&str> = covered_tool_ids.iter().map(|_| "?").collect();
            let sql = format!(
                "DELETE FROM usage_event \
                 WHERE source_type = 'local_discovered' AND tool_id IN ({})",
                placeholders.join(",")
            );
            tx.execute(&sql, rusqlite::params_from_iter(covered_tool_ids.iter()))
                .map_err(|e| e.to_string())?;
        }
        tx.execute("DELETE FROM tm_daily_rollup", [])
            .map_err(|e| e.to_string())?;
        let mut inserted = 0i64;
        for raw in events {
            if Self::insert_event_locked(&tx, raw)? {
                inserted += 1;
            }
        }
        // rollup 从剩余的 usage_event 全量重建：companion 工具（freebuff 等）的用量一并保留。
        Self::rebuild_rollup_locked(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(inserted)
    }

    /// 实时速率窗口（对齐开源 Token Monitor 的 timed 窗口读数）。
    /// 返回 (近60秒 output 总量, 近60分钟 total 总量, 最近事件时间)。
    /// 空窗口返回 None 字段由上层诚实标注「不可用」。
    pub fn token_rate_window(
        &self,
        conn: &Mutex<Connection>,
    ) -> Result<(Option<i64>, Option<i64>, Option<String>), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        // 与 range_boundary 一致：occurred_at 经 datetime() 归一后再比较（ISO ↔ SQLite 格式兼容）。
        // 单次扫描 + 条件聚合：只引用 unified 一次，避免 SQLite 对未物化 CTE 的多次重算。
        let sql = format!(
            "{UNIFIED_CTE}
             SELECT
               COALESCE(SUM(CASE WHEN datetime(occurred_at,'localtime') >= datetime('now','localtime','-60 seconds')
                                THEN output_tokens END),0),
               COALESCE(SUM(CASE WHEN datetime(occurred_at,'localtime') >= datetime('now','localtime','-3600 seconds')
                                THEN total_tokens END),0),
               MAX(occurred_at)
             FROM unified WHERE occurred_at IS NOT NULL"
        );
        let (out_60s, total_1h, latest): (i64, i64, Option<String>) = conn
            .query_row(&sql, [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|e| e.to_string())?;
        Ok((
            if out_60s > 0 { Some(out_60s) } else { None },
            if total_1h > 0 { Some(total_1h) } else { None },
            latest,
        ))
    }

    /// 对齐开源 Token Monitor 的速率口径：读取当日 tokscale 快照的 timed 性能，
    /// 返回 (timedDurationMs, timedTokens, timedOutputTokens)；无快照返回全 0。
    pub fn period_timed_performance(
        &self,
        conn: &Mutex<Connection>,
        range: &str,
    ) -> (i64, i64, i64) {
        match period_snapshot_json(conn, range) {
            Some(snap) => timed_performance_from_snap(&snap),
            None => (0, 0, 0),
        }
    }

    /// 统一口径范围聚合（02 §7）。返回 (input, output, cache, total)。
    ///
    /// **day|7d|month**：优先使用 tokscale 权威 period 快照（`--today/--week/--month`，
    /// 按消息真实时间逐日归因——避免 usage_event 里「会话生命周期累计值」按最后活跃日
    /// 归因导致的今日虚高），并叠加 tokscale 未覆盖的 companion 工具
    /// （freebuff/atomcode/dsh/自定义应用）的 usage_event 用量；快照缺失时回退
    /// usage_event 聚合。
    /// **total**：始终走 usage_event（全量口径无窗口，usage_event 即权威）。
    pub fn unified_range_stats(
        &self,
        conn: &Mutex<Connection>,
        range: &str,
    ) -> Result<(i64, i64, i64, i64), String> {
        if range != "total" {
            if let Some(snap) = period_snapshot_json(conn, range) {
                let (input, output, cache, total, _cost) = period_totals(conn, &snap);
                let (ci, co, cc, ct) = self
                    .companion_range_stats(conn, range)
                    .unwrap_or((0, 0, 0, 0));
                return Ok((input + ci, output + co, cache + cc, total + ct));
            }
        }
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "{UNIFIED_CTE}
             SELECT COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(cache_tokens),0), COALESCE(SUM(total_tokens),0)
             FROM unified WHERE {}",
            range_boundary(range)
        );
        conn.query_row(&sql, [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|e| e.to_string())
    }

    /// companion 工具（tokscale 未覆盖 + 自定义应用）在范围内的 usage_event 聚合。
    fn companion_range_stats(
        &self,
        conn: &Mutex<Connection>,
        range: &str,
    ) -> Result<(i64, i64, i64, i64), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "{UNIFIED_CTE}
             SELECT COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(cache_tokens),0), COALESCE(SUM(total_tokens),0)
             FROM unified WHERE {} AND {}",
            range_boundary(range),
            companion_filter_sql(),
        );
        conn.query_row(&sql, [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|e| e.to_string())
    }

    /// 按工具聚合行（ToolUsageRow）。
    ///
    /// **day|7d|month**：优先合并 tokscale 权威快照工具行 + companion 工具行
    /// （share 按合并后总量重算）；快照缺失回退 usage_event。**total** 始终走 usage_event。
    pub fn tool_usage_rows(
        &self,
        conn: &Mutex<Connection>,
        range: &str,
    ) -> Result<Vec<ToolUsageRow>, String> {
        let mut rows = if range != "total" {
            match (period_tool_rows(conn, range), self.companion_tool_rows(conn, range)) {
                (Some(snapshot_rows), Ok(companion_rows)) => {
                    Self::merge_tool_rows(snapshot_rows.into_iter().chain(companion_rows).collect())
                }
                // companion 查询失败不吞掉权威快照数据（快照已含 covered 工具）
                (Some(snapshot_rows), Err(_)) => snapshot_rows,
                (None, Ok(companion_rows)) => companion_rows,
                (None, Err(_)) => self.usage_event_tool_rows(conn, range)?,
            }
        } else {
            self.usage_event_tool_rows(conn, range)?
        };
        let grand_total: i64 = rows.iter().map(|r| r.total_tokens).sum();
        for row in &mut rows {
            row.share_percent = if grand_total > 0 {
                row.total_tokens as f64 * 100.0 / grand_total as f64
            } else {
                0.0
            };
        }
        Ok(rows)
    }

    /// usage_event 全量聚合工具行（无快照回退口径）。
    fn usage_event_tool_rows(
        &self,
        conn: &Mutex<Connection>,
        range: &str,
    ) -> Result<Vec<ToolUsageRow>, String> {
        let guard = conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "{UNIFIED_CTE}
             SELECT unified.tool_id,
                    COALESCE(td.display_name, ''), COALESCE(td.support_level, 'standard'),
                    COALESCE(td.collector_status, 'active'),
                    COALESCE(unified.model_normalized, ''),
                    COALESCE(SUM(unified.input_tokens),0), COALESCE(SUM(unified.output_tokens),0),
                    COALESCE(SUM(unified.cache_tokens),0), COALESCE(SUM(unified.total_tokens),0),
                    SUM(unified.cost_amount)
             FROM unified
             LEFT JOIN tool_definition td ON td.tool_id = unified.tool_id
             WHERE {}
             GROUP BY unified.tool_id, unified.model_normalized",
            range_boundary(range)
        );
        let group_rows = Self::query_tool_groups(&guard, &sql)?;
        drop(guard);
        Ok(Self::aggregate_tool_groups(group_rows))
    }

    /// companion 工具行：与 usage_event_tool_rows 同构，仅限定 tokscale 未覆盖工具。
    fn companion_tool_rows(
        &self,
        conn: &Mutex<Connection>,
        range: &str,
    ) -> Result<Vec<ToolUsageRow>, String> {
        let guard = conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "{UNIFIED_CTE}
             SELECT unified.tool_id,
                    COALESCE(td.display_name, ''), COALESCE(td.support_level, 'standard'),
                    COALESCE(td.collector_status, 'active'),
                    COALESCE(unified.model_normalized, ''),
                    COALESCE(SUM(unified.input_tokens),0), COALESCE(SUM(unified.output_tokens),0),
                    COALESCE(SUM(unified.cache_tokens),0), COALESCE(SUM(unified.total_tokens),0),
                    SUM(unified.cost_amount)
             FROM unified
             LEFT JOIN tool_definition td ON td.tool_id = unified.tool_id
             WHERE {} AND {}
             GROUP BY unified.tool_id, unified.model_normalized",
            range_boundary(range),
            companion_filter_sql(),
        );
        let group_rows = Self::query_tool_groups(&guard, &sql)?;
        drop(guard);
        Ok(Self::aggregate_tool_groups(group_rows))
    }

    /// 执行工具分组查询（(tool, model) 行）。
    fn query_tool_groups(conn: &Connection, sql: &str) -> Result<Vec<ToolGroupRow>, String> {
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let group_rows = stmt
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
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        Ok(group_rows)
    }

    /// 按工具汇总各 (tool, model) 分组，成本逐组估算后相加（share 由调用方重算）。
    fn aggregate_tool_groups(group_rows: Vec<ToolGroupRow>) -> Vec<ToolUsageRow> {
        use std::collections::HashMap;
        struct Agg {
            display_name: String,
            support_level: String,
            collector_status: String,
            input: i64,
            output: i64,
            cache: i64,
            total: i64,
            cost: Option<f64>,
            order: usize,
        }
        let mut acc: HashMap<String, Agg> = HashMap::new();
        for (
            tool_id,
            display_name,
            support_level,
            collector_status,
            model,
            input,
            output,
            cache,
            total,
            db_cost,
        ) in group_rows
        {
            let group_cost = db_cost
                .filter(|c| *c > 0.0)
                .or_else(|| pricing::estimate_cost(&model, input, output, cache));
            let next_order = acc.len();
            let entry = acc.entry(tool_id.clone()).or_insert_with(|| Agg {
                display_name,
                support_level,
                collector_status,
                input: 0,
                output: 0,
                cache: 0,
                total: 0,
                cost: None,
                order: next_order,
            });
            entry.input += input;
            entry.output += output;
            entry.cache += cache;
            entry.total += total;
            if let Some(c) = group_cost {
                entry.cost = Some(entry.cost.unwrap_or(0.0) + c);
            }
        }

        let mut rows: Vec<(String, Agg)> = acc.into_iter().collect();
        // 总量降序（与旧行为一致），并列时按首次出现顺序稳定。
        rows.sort_by(|a, b| b.1.total.cmp(&a.1.total).then(a.1.order.cmp(&b.1.order)));

        rows.into_iter()
            .map(|(tool_id, agg)| {
                let display_name = if agg.display_name.is_empty() {
                    display_name_for_tool(&tool_id)
                } else {
                    agg.display_name
                };
                ToolUsageRow {
                    tool_id,
                    display_name,
                    support_level: parse_support_level(&agg.support_level),
                    collector_status: parse_collector_status(&agg.collector_status),
                    input_tokens: agg.input,
                    output_tokens: agg.output,
                    cache_tokens: agg.cache,
                    total_tokens: agg.total,
                    cost_amount: agg.cost,
                    share_percent: 0.0, // 调用方合并后重算
                }
            })
            .collect()
    }

    /// 按模型聚合行（ModelUsageRow）。
    ///
    /// **day|7d|month**：优先合并 tokscale 权威快照模型行 + companion 模型行
    /// （同模型累加、重算 share）；快照缺失回退 usage_event。**total** 始终走 usage_event。
    pub fn model_usage_rows(
        &self,
        conn: &Mutex<Connection>,
        range: &str,
    ) -> Result<Vec<ModelUsageRow>, String> {
        if range != "total" {
            if let Some(snapshot_rows) = period_model_rows(conn, range) {
                let companion_rows = self.companion_model_rows(conn, range).unwrap_or_default();
                let merged: Vec<ModelUsageRow> =
                    snapshot_rows.into_iter().chain(companion_rows).collect();
                return Ok(Self::merge_model_rows(merged));
            }
        }
        self.usage_event_model_rows(conn, range)
    }

    /// usage_event 全量聚合模型行（无快照回退口径）。
    fn usage_event_model_rows(
        &self,
        conn: &Mutex<Connection>,
        range: &str,
    ) -> Result<Vec<ModelUsageRow>, String> {
        let guard = conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "{UNIFIED_CTE}
             SELECT COALESCE(NULLIF(unified.model_normalized,''), '未知模型'),
                    COALESCE(SUM(unified.input_tokens),0), COALESCE(SUM(unified.output_tokens),0),
                    COALESCE(SUM(unified.cache_tokens),0), COALESCE(SUM(unified.total_tokens),0),
                    SUM(unified.cost_amount)
             FROM unified WHERE {}
             GROUP BY unified.model_normalized ORDER BY 5 DESC",
            range_boundary(range)
        );
        let raw_rows = Self::query_model_groups(&guard, &sql)?;
        drop(guard);

        let grand_total: i64 = raw_rows.iter().map(|row| row.4).sum();
        Ok(raw_rows
            .into_iter()
            .map(|(model, input, output, cache, total, cost)| {
                // DB 有真实 cost 时优先，否则按价格表估算（未知模型 → None）。
                let cost_amount = cost
                    .filter(|c| *c > 0.0)
                    .or_else(|| pricing::estimate_cost(&model, input, output, cache));
                ModelUsageRow {
                    model,
                    input_tokens: input,
                    output_tokens: output,
                    cache_tokens: cache,
                    total_tokens: total,
                    cost_amount,
                    share_percent: if grand_total > 0 {
                        total as f64 * 100.0 / grand_total as f64
                    } else {
                        0.0
                    },
                }
            })
            .collect())
    }

    /// companion 工具模型行：与 usage_event_model_rows 同构，仅限定 tokscale 未覆盖工具。
    fn companion_model_rows(
        &self,
        conn: &Mutex<Connection>,
        range: &str,
    ) -> Result<Vec<ModelUsageRow>, String> {
        let guard = conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "{UNIFIED_CTE}
             SELECT COALESCE(NULLIF(unified.model_normalized,''), '未知模型'),
                    COALESCE(SUM(unified.input_tokens),0), COALESCE(SUM(unified.output_tokens),0),
                    COALESCE(SUM(unified.cache_tokens),0), COALESCE(SUM(unified.total_tokens),0),
                    SUM(unified.cost_amount)
             FROM unified WHERE {} AND {}
             GROUP BY unified.model_normalized ORDER BY 5 DESC",
            range_boundary(range),
            companion_filter_sql(),
        );
        let raw_rows = Self::query_model_groups(&guard, &sql)?;
        drop(guard);

        Ok(raw_rows
            .into_iter()
            .map(|(model, input, output, cache, total, cost)| {
                let cost_amount = cost
                    .filter(|c| *c > 0.0)
                    .or_else(|| pricing::estimate_cost(&model, input, output, cache));
                ModelUsageRow {
                    model,
                    input_tokens: input,
                    output_tokens: output,
                    cache_tokens: cache,
                    total_tokens: total,
                    cost_amount,
                    share_percent: 0.0, // 合并后重算
                }
            })
            .collect())
    }

    /// 执行模型分组查询。
    fn query_model_groups(conn: &Connection, sql: &str) -> Result<Vec<ModelGroupRow>, String> {
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let raw_rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        Ok(raw_rows)
    }

    /// 合并工具行（快照 + companion 可能同名）：同 tool_id 累加 token/成本，
    /// 展示元数据取首个非空，总量降序（share 由调用方重算）。
    fn merge_tool_rows(rows: Vec<ToolUsageRow>) -> Vec<ToolUsageRow> {
        use std::collections::HashMap;
        let mut acc: HashMap<String, ToolUsageRow> = HashMap::new();
        for r in rows {
            match acc.get_mut(&r.tool_id) {
                Some(existing) => {
                    if existing.display_name.is_empty() && !r.display_name.is_empty() {
                        existing.display_name = r.display_name;
                    }
                    existing.input_tokens += r.input_tokens;
                    existing.output_tokens += r.output_tokens;
                    existing.cache_tokens += r.cache_tokens;
                    existing.total_tokens += r.total_tokens;
                    existing.cost_amount = match (existing.cost_amount, r.cost_amount) {
                        (Some(a), Some(b)) => Some(a + b),
                        (Some(a), None) => Some(a),
                        (None, Some(b)) => Some(b),
                        (None, None) => None,
                    };
                }
                None => {
                    acc.insert(r.tool_id.clone(), r);
                }
            }
        }
        let mut merged: Vec<ToolUsageRow> = acc.into_values().collect();
        merged.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
        merged
    }

    /// 合并模型行（快照 + companion 可能同名）：同模型累加，成本相加，总量降序，重算 share。
    fn merge_model_rows(rows: Vec<ModelUsageRow>) -> Vec<ModelUsageRow> {
        use std::collections::HashMap;
        let mut acc: HashMap<String, ModelUsageRow> = HashMap::new();
        for r in rows {
            match acc.get_mut(&r.model) {
                Some(existing) => {
                    existing.input_tokens += r.input_tokens;
                    existing.output_tokens += r.output_tokens;
                    existing.cache_tokens += r.cache_tokens;
                    existing.total_tokens += r.total_tokens;
                    existing.cost_amount = match (existing.cost_amount, r.cost_amount) {
                        (Some(a), Some(b)) => Some(a + b),
                        (Some(a), None) => Some(a),
                        (None, Some(b)) => Some(b),
                        (None, None) => None,
                    };
                }
                None => {
                    acc.insert(r.model.clone(), r);
                }
            }
        }
        let mut merged: Vec<ModelUsageRow> = acc.into_values().collect();
        merged.sort_by_key(|r| std::cmp::Reverse(r.total_tokens));
        let grand_total: i64 = merged.iter().map(|r| r.total_tokens).sum();
        for row in &mut merged {
            row.share_percent = if grand_total > 0 {
                row.total_tokens as f64 * 100.0 / grand_total as f64
            } else {
                0.0
            };
        }
        merged
    }

    /// 该 range 的总估算成本（USD）；把各模型行的成本求和（None 视为 0，但整体全 None 时返回 None）。
    pub fn total_cost(&self, conn: &Mutex<Connection>, range: &str) -> Result<Option<f64>, String> {
        let rows = self.model_usage_rows(conn, range)?;
        let mut sum = 0.0;
        let mut any = false;
        for row in &rows {
            if let Some(c) = row.cost_amount {
                sum += c;
                any = true;
            }
        }
        Ok(if any { Some(sum) } else { None })
    }

    /// 会话逐轮用量明细（list_session_events 数据源，W7 会话下钻）。
    ///
    /// 对齐开源 Token Monitor 的会话下钻（sessionDetail：prompt→turns 逐轮明细），但只返回
    /// usage_event 的元数据行（时间/模型/token/成本），**不读 Prompt/Response 正文**（隐私红线）。
    /// 每行 = 一次 assistant 轮次；cache = cache_read + cache_write（与 unified 口径一致）；
    /// 成本 DB 真实值优先，缺失时按模型价格估算，未知模型为 None。
    /// 排序：occurred_at 倒序（最新在前，对齐开源默认 time 排序）。
    ///
    /// **id 兼容**：tm_session.session_id 是 `hash(source_id:tool:file)`，而部分适配器（claude/
    /// jsonl 宏）写入 usage_event.session_id 的是文件名（external_session_id）；tokscale 重写
    /// usage_event 后 session_id 又是 tokscale 的原始 id。因此同时匹配 tm_session 的
    /// session_id 与 external_session_id，保证「列表能点开 → 明细能查到」。
    pub fn session_event_rows(
        &self,
        conn: &Mutex<Connection>,
        session_id: &str,
    ) -> Result<Vec<SessionEventRow>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let external: Option<String> = conn
            .query_row(
                "SELECT external_session_id FROM tm_session WHERE session_id=?1",
                rusqlite::params![session_id],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        let mut stmt = conn
            .prepare(
                "SELECT occurred_at, model_normalized,
                        COALESCE(input_tokens,0), COALESCE(output_tokens,0),
                        (COALESCE(cache_read_tokens,0)+COALESCE(cache_write_tokens,0)),
                        COALESCE(reasoning_tokens,0), COALESCE(total_tokens,0),
                        cost_amount
                 FROM usage_event
                 WHERE (session_id=?1
                    OR (session_id=?2 AND ?2 IS NOT NULL AND ?2 != ''))
                   AND tool_id IN (SELECT tool_id FROM tool_definition WHERE enabled=1)
                 ORDER BY occurred_at DESC LIMIT 200",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![session_id, external], |row| {
                let input: i64 = row.get(2)?;
                let output: i64 = row.get(3)?;
                let cache: i64 = row.get(4)?;
                let model: Option<String> = row.get(1)?;
                let cost_amount: Option<f64> = row.get(7)?;
                let cost = cost_amount
                    .filter(|c| *c > 0.0)
                    .or_else(|| {
                        model
                            .as_deref()
                            .and_then(|m| pricing::estimate_cost(m, input, output, cache))
                    });
                Ok(SessionEventRow {
                    occurred_at: row.get(0)?,
                    model,
                    input_tokens: input,
                    output_tokens: output,
                    cache_tokens: cache,
                    reasoning_tokens: row.get(5)?,
                    total_tokens: row.get(6)?,
                    cost_amount: cost,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        Ok(rows)
    }

    /// 趋势序列（日 + 月 + 活跃/连续天数 + 活跃时间 + 消息数 + 峰值）。
    ///
    /// 优先使用 tokscale graph 权威口径（按消息真实时间逐日归因，含按工具/按模型拆分、
    /// 活跃/连续/峰值/活跃时间/消息数），并叠加 tokscale 未覆盖的 companion 工具
    /// （freebuff/atomcode/dsh/自定义应用）的逐日用量（tm_daily_rollup 按真实行时间
    /// 落库，逐日准确）；tokscale 缺失/失败时回退 usage_event DB 聚合。
    pub fn trend_series(
        &self,
        conn: &Mutex<Connection>,
        range: &str,
    ) -> Result<TrendSeries, String> {
        match crate::token_monitor::collector::tokscale::try_trend_series() {
            Some(mut series) => {
                self.merge_companion_trend(conn, &mut series)?;
                // 汇总口径对齐开源（history.js + daily-history-archive）：用持久化每日归档
                // 补回 tokscale 源文件清理后消失的活跃日，活跃天数/连续天数不缩水
                // （95 vs 96）；自定义工具与 companion 工具也按「工具维度」一并归档。
                crate::token_monitor::daily_archive::merge_and_capture(conn, &mut series)?;
                Ok(series)
            }
            None => self.trend_series_from_db(conn, range),
        }
    }

    /// 把 companion 工具（tokscale 未覆盖）的逐日用量合并进 tokscale graph 趋势，
    /// 并重算活跃天数/连续天数/峰值/月度/消息数汇总。
    ///
    /// **数据源用 usage_event 直接按日聚合**（与今日总量 `companion_range_stats` 同源，
    /// 保证「趋势·今日」与「今日总 TOKENS」一致），不依赖 tm_daily_rollup 派生表——
    /// 该表在采集重建时序（如 DSH v2 迁移 + 重新采集）下可能残留过期值导致两个视图不一致。
    fn merge_companion_trend(
        &self,
        conn: &Mutex<Connection>,
        series: &mut TrendSeries,
    ) -> Result<(), String> {
        use std::collections::{BTreeMap, HashMap};
        let conn = conn.lock().map_err(|e| e.to_string())?;

        // companion = tokscale 未覆盖的工具（freebuff/atomcode/dsh/自定义应用），
        // 事件按真实行时间落库，逐日归因准确。
        let sql = format!(
            "{UNIFIED_CTE}
             SELECT date(unified.occurred_at,'localtime') AS day,
                    unified.tool_id, COALESCE(unified.model_normalized,''),
                    COALESCE(SUM(unified.total_tokens),0), COUNT(*)
             FROM unified
             WHERE unified.occurred_at IS NOT NULL AND {}
             GROUP BY day, unified.tool_id, unified.model_normalized",
            companion_filter_sql(),
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);

        // companion 工具的当日活跃时长（毫秒）：tokscale graph 只含 tokscale 自身的
        // activeTimeMs，这里从 tm_session 按最后活跃日归并补齐自定义/companion 工具的
        // 活跃时间（与 DB 回退路径同口径），避免「活跃时间」只统计 tokscale。
        let active_sql = format!(
            "SELECT date(last_active_at,'localtime') AS day,
                    COALESCE(SUM(
                        CASE WHEN started_at IS NOT NULL AND last_active_at IS NOT NULL
                             AND unixepoch(last_active_at) > unixepoch(started_at)
                        THEN (unixepoch(last_active_at) - unixepoch(started_at)) * 1000
                        ELSE 0 END),0)
             FROM tm_session
             WHERE last_active_at IS NOT NULL AND {}
             GROUP BY day",
            companion_tool_condition("tm_session"),
        );
        let mut active_stmt = conn.prepare(&active_sql).map_err(|e| e.to_string())?;
        let companion_active_by_day: HashMap<String, i64> = active_stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        drop(active_stmt);
        let companion_active_total: i64 = companion_active_by_day.values().sum();

        // day → 已存在日序列下标；不存在（companion 独有活跃日）则追加
        let mut index: HashMap<String, usize> = HashMap::new();
        for (i, d) in series.daily.iter().enumerate() {
            index.insert(d.date.clone(), i);
        }
        let mut extra_messages = 0i64;
        let mut by_day: HashMap<String, (i64, i64, Vec<(String, i64)>, Vec<(String, i64)>)> =
            HashMap::new();
        for (day, tool, model, tokens, reqs) in rows {
            if tokens <= 0 {
                continue;
            }
            extra_messages += reqs;
            let e = by_day.entry(day).or_default();
            e.0 += tokens;
            e.1 += reqs;
            if !tool.is_empty() {
                e.2.push((tool, tokens));
            }
            // 模型缺失（如 Freebuff 空 model）也归入 per_model——挂到「未知模型」桶，
            // 保证 sum(per_model) == 日总量，按模型拆分不丢用量（趋势页与总览一致）。
            if model.is_empty() {
                e.3.push(("未知模型".to_string(), tokens));
            } else {
                e.3.push((model, tokens));
            }
        }
        for (day, (tokens, reqs, clients, models)) in by_day {
            let split = |pairs: Vec<(String, i64)>| {
                let mut list: Vec<SeriesSplit> = pairs
                    .into_iter()
                    .map(|(key, t)| SeriesSplit { key, tokens: t })
                    .collect();
                list.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.key.cmp(&b.key)));
                list
            };
            match index.get(&day) {
                Some(&i) => {
                    let d = &mut series.daily[i];
                    d.tokens += tokens;
                    d.requests += reqs;
                    let c = d.per_client.get_or_insert_with(Vec::new);
                    c.extend(split(clients));
                    c.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.key.cmp(&b.key)));
                    let m = d.per_model.get_or_insert_with(Vec::new);
                    m.extend(split(models));
                    m.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.key.cmp(&b.key)));
                }
                None => {
                    series.daily.push(TrendDay {
                        date: day.clone(),
                        tokens,
                        requests: reqs,
                        cost_amount: None,
                        active_time_ms: 0,
                        per_client: Some(split(clients)),
                        per_model: Some(split(models)),
                    });
                    index.insert(day, series.daily.len() - 1);
                }
            }
        }

        // 把 companion 活跃时长补到对应日的 active_time_ms（已存在的 graph 日 / 新增的
        // companion 独有日都补齐）；仅活跃日（已有该日期）计入，0 时长日不补。
        for (day, active_ms) in &companion_active_by_day {
            if *active_ms <= 0 {
                continue;
            }
            if let Some(&i) = index.get(day) {
                series.daily[i].active_time_ms += active_ms;
            }
        }
        // 累计口径：活跃时间 = tokscale 总时长 + companion 总时长（不再只算 tokscale）。
        series.active_time_ms += companion_active_total;
        series.message_count += extra_messages;

        series.daily.sort_by(|a, b| a.date.cmp(&b.date));
        series.active_days = series.daily.iter().filter(|d| d.tokens > 0).count() as i64;
        series.peak_day = series
            .daily
            .iter()
            .max_by_key(|d| d.tokens)
            .filter(|d| d.tokens > 0)
            .cloned();
        let active_set: std::collections::HashSet<String> = series
            .daily
            .iter()
            .filter(|d| d.tokens > 0)
            .map(|d| d.date.clone())
            .collect();
        let mut streak_days = 0i64;
        let mut cursor = chrono::Local::now().format("%Y-%m-%d").to_string();
        loop {
            if active_set.contains(&cursor) {
                streak_days += 1;
                cursor = prev_local_day(&cursor);
            } else {
                break;
            }
        }
        series.streak_days = streak_days;
        let mut months: BTreeMap<String, i64> = BTreeMap::new();
        for d in series.daily.iter().filter(|d| d.tokens > 0) {
            *months.entry(d.date[..7].to_string()).or_default() += d.tokens;
        }
        series.monthly = months
            .into_iter()
            .map(|(month, tokens)| TrendMonth { month, tokens })
            .collect();
        Ok(())
    }

    /// 全历史按月聚合（usage_event，无窗口）——累计视图按月展示的权威口径。
    /// 调用方需已持有连接锁（传入 `&MutexGuard<Connection>` 解引用）。
    fn monthly_from_db(&self, conn: &Connection) -> Result<Vec<TrendMonth>, String> {
        let mut stmt = conn
            .prepare(&format!(
                "{UNIFIED_CTE}
                 SELECT substr(date(unified.occurred_at,'localtime'),1,7) AS month,
                        COALESCE(SUM(unified.total_tokens),0)
                 FROM unified
                 WHERE unified.occurred_at IS NOT NULL
                 GROUP BY month ORDER BY month"
            ))
            .map_err(|e| e.to_string())?;
        let monthly: Vec<TrendMonth> = stmt
            .query_map([], |row| {
                Ok(TrendMonth {
                    month: row.get(0)?,
                    tokens: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        Ok(monthly)
    }

    /// DB 聚合口径（usage_event + tm_session；tokscale 缺失时的回退路径）。
    fn trend_series_from_db(
        &self,
        conn: &Mutex<Connection>,
        _range: &str,
    ) -> Result<TrendSeries, String> {
        let db = conn;
        let conn = conn.lock().map_err(|e| e.to_string())?;

        // 日序列（全历史，按本地日历）
        let mut stmt = conn
            .prepare(&format!(
                "{UNIFIED_CTE}
                 SELECT date(unified.occurred_at,'localtime') AS day,
                        COALESCE(SUM(unified.total_tokens),0),
                        COUNT(*), SUM(unified.cost_amount)
                 FROM unified
                 WHERE unified.occurred_at IS NOT NULL
                 GROUP BY day ORDER BY day DESC LIMIT 400"
            ))
            .map_err(|e| e.to_string())?;
        let mut daily: Vec<TrendDay> = stmt
            .query_map([], |row| {
                Ok(TrendDay {
                    date: row.get(0)?,
                    tokens: row.get(1)?,
                    requests: row.get(2)?,
                    cost_amount: row.get(3)?,
                    active_time_ms: 0,
                    per_client: None,
                    per_model: None,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);

        // 当日活跃时长（毫秒）：tm_session 按最后活跃日归并（与日序列 occurred_at 口径一致），
        // 供趋势明细页按范围汇总「活跃时间」。
        let mut active_stmt = conn
            .prepare(
                "SELECT date(last_active_at,'localtime') AS day,
                        COALESCE(SUM(
                            CASE WHEN started_at IS NOT NULL AND last_active_at IS NOT NULL
                                 AND unixepoch(last_active_at) > unixepoch(started_at)
                            THEN (unixepoch(last_active_at) - unixepoch(started_at)) * 1000
                            ELSE 0 END),0)
                 FROM tm_session
                 WHERE last_active_at IS NOT NULL
                 GROUP BY day",
            )
            .map_err(|e| e.to_string())?;
        let active_by_day: std::collections::HashMap<String, i64> = active_stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        drop(active_stmt);
        for d in daily.iter_mut() {
            d.active_time_ms = active_by_day.get(&d.date).copied().unwrap_or(0);
        }

        // 按工具/按模型拆分：与日序列同源（UNIFIED_CTE 直接按 usage_event 逐日聚合），
        // 保证 sum(per_client) == sum(per_model) == d.tokens——趋势页每日统计与总览热力图
        // 永远一致。不再读 tm_daily_rollup：该表在采集重建时序（DSH v2 迁移 + 重新采集）
        // 下可能残留过期值，导致按工具/按模型拆分与日总量对不上（两个视图数值不一致）。
        let mut split_stmt = conn
            .prepare(&format!(
                "{UNIFIED_CTE}
                 SELECT date(unified.occurred_at,'localtime') AS day,
                        unified.tool_id, COALESCE(unified.model_normalized,''),
                        COALESCE(SUM(unified.total_tokens),0)
                 FROM unified
                 WHERE unified.occurred_at IS NOT NULL
                 GROUP BY day, unified.tool_id, unified.model_normalized"
            ))
            .map_err(|e| e.to_string())?;
        let mut per_client: std::collections::HashMap<String, Vec<(String, i64)>> =
            std::collections::HashMap::new();
        let mut per_model: std::collections::HashMap<String, Vec<(String, i64)>> =
            std::collections::HashMap::new();
        {
            let rows = split_stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (day, tool, model, tokens) = row.map_err(|e| e.to_string())?;
                if !tool.is_empty() {
                    per_client
                        .entry(day.clone())
                        .or_default()
                        .push((tool, tokens));
                }
                // 模型缺失（NULL/空串）归入「未知模型」桶：按模型拆分不丢用量，
                // 与日总量恒等（总览热力图/趋势页每日统计对齐）。
                if model.is_empty() {
                    per_model
                        .entry(day.clone())
                        .or_default()
                        .push(("未知模型".to_string(), tokens));
                } else {
                    per_model
                        .entry(day.clone())
                        .or_default()
                        .push((model, tokens));
                }
            }
        }
        drop(split_stmt);
        for d in daily.iter_mut() {
            if let Some(list) = per_client.remove(&d.date) {
                let mut list = list;
                list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                d.per_client = Some(
                    list.into_iter()
                        .map(|(key, tokens)| SeriesSplit { key, tokens })
                        .collect(),
                );
            }
            if let Some(list) = per_model.remove(&d.date) {
                let mut list = list;
                list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                d.per_model = Some(
                    list.into_iter()
                        .map(|(key, tokens)| SeriesSplit { key, tokens })
                        .collect(),
                );
            }
        }

        // 月序列（全历史，无窗口）
        let monthly = self.monthly_from_db(&conn)?;

        // 累计消息数与活跃时间：使用全量会话摘要，避免 UI 近 50 会话窗口导致与开源累计口径不一致。
        let (message_count, active_time_ms): (i64, i64) = conn
            .query_row(
                "SELECT COALESCE(SUM(COALESCE(message_count,0)),0),
                        COALESCE(SUM(
                            CASE
                                WHEN started_at IS NOT NULL AND last_active_at IS NOT NULL
                                     AND unixepoch(last_active_at) > unixepoch(started_at)
                                THEN (unixepoch(last_active_at) - unixepoch(started_at)) * 1000
                                ELSE 0
                            END
                        ),0)
                 FROM tm_session",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or((0, 0));

        let mut series = TrendSeries {
            daily,
            active_days: 0,
            streak_days: 0,
            peak_day: None,
            monthly,
            active_time_ms,
            message_count,
        };
        drop(conn);
        // 对齐开源（history.js + daily-history-archive）：补回归档里已丢失的活跃日并重算
        // 活跃天数/连续天数/峰值/月度/活跃时间。
        crate::token_monitor::daily_archive::merge_and_capture(db, &mut series)?;
        Ok(series)
    }
}

/// 前一天的本地日历（YYYY-MM-DD）。
fn prev_local_day(day: &str) -> String {
    if let Ok(date) = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d") {
        return (date - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
    }
    day.to_string()
}

fn parse_support_level(value: &str) -> SupportLevel {
    serde_json::from_str(&format!("\"{value}\"")).unwrap_or(SupportLevel::Basic)
}

fn parse_collector_status(value: &str) -> CollectorStatus {
    serde_json::from_str(&format!("\"{value}\"")).unwrap_or(CollectorStatus::Idle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_monitor::model::{SourceType, UsageAccuracy};

    fn local_event(
        tool_id: &str,
        input: i64,
        output: i64,
        occurred_at: &str,
    ) -> NormalizedUsageEvent {
        NormalizedUsageEvent {
            source_type: SourceType::LocalDiscovered,
            tool_id: tool_id.into(),
            device_id: "local".into(),
            model_raw: Some("claude-3-5-sonnet-20241022".into()),
            model_normalized: Some("claude-3-5-sonnet".into()),
            session_id: None,
            project_id: None,
            account_id: None,
            input_tokens: Some(input),
            output_tokens: Some(output),
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
            message_count: None,
            session_started_at: None,
            session_last_active_at: None,
            total_tokens: None,
            cost_amount: None,
            cost_currency: None,
            usage_accuracy: UsageAccuracy::Exact,
            occurred_at: occurred_at.into(),
            source_locator_hash: None,
        }
    }

    fn test_db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("open database");
        conn.execute_batch(include_str!("../../migrations/001_initial.sql"))
            .expect("migrate base");
        conn.execute_batch(include_str!("../../migrations/008_client_keys.sql"))
            .expect("migrate client keys");
        conn.execute_batch(include_str!(
            "../../migrations/009_request_logs_client_key.sql"
        ))
        .expect("migrate client key audit");
        conn.execute_batch(include_str!(
            "../../migrations/010_route_pool_management.sql"
        ))
        .expect("migrate route pool");
        conn.execute_batch(include_str!("../../migrations/016_token_monitor.sql"))
            .expect("migrate token monitor");
        conn.execute_batch(include_str!(
            "../../migrations/017_usage_event_message_count.sql"
        ))
        .expect("migrate message_count");
        conn.execute_batch(include_str!(
            "../../migrations/018_tm_period_session_membership.sql"
        ))
        .expect("migrate period session membership");
        conn.execute_batch(include_str!(
            "../../migrations/019_usage_event_session_times.sql"
        ))
        .expect("migrate session times");
        Mutex::new(conn)
    }

    /// 权威 period 快照优先：day/7d/month 读 tokscale 快照口径（逐日归因正确），
    /// covered 工具的 usage_event 行不再按 mtime 归因计入今日；total 仍走 usage_event。
    #[test]
    fn unified_range_stats_uses_period_snapshot_for_day() {
        let db = test_db();
        let today = chrono::Utc::now().format("%Y-%m-%dT00:00:00Z").to_string();
        let event = local_event("claude_code", 500, 500, &today);
        UsageEventRepo.insert_batch(&db, &[event]).expect("insert");

        // 写入 tokscale 快照（day 权威口径：claude 今日 100 tokens）
        let snapshot_json = r#"{"groupBy":"client,model","totalInput":40,"totalOutput":20,"totalCacheRead":30,"totalCacheWrite":10,"totalCost":0.5,"entries":[{"client":"claude","model":"claude-3-5-sonnet","input":40,"output":20,"cacheRead":30,"cacheWrite":10,"cost":0.5}]}"#;
        crate::db::settings::SettingsRepo
            .set(&db, "tm.period.day", snapshot_json)
            .expect("set day snapshot");

        // day：快照 40+20+30+10=100 为权威（claude_code 的 usage_event 行是 covered
        // 工具旧数据，不再按 mtime 归因计入今日，避免会话生命周期累计值虚高今日）
        let (input, output, cache, total) = UsageEventRepo
            .unified_range_stats(&db, "day")
            .expect("day stats");
        assert_eq!((input, output, cache, total), (40, 20, 40, 100));

        let tool_rows = UsageEventRepo
            .tool_usage_rows(&db, "day")
            .expect("day tools");
        assert_eq!(tool_rows.len(), 1);
        assert_eq!(tool_rows[0].tool_id, "claude_code");
        assert_eq!(tool_rows[0].total_tokens, 100);

        let model_rows = UsageEventRepo
            .model_usage_rows(&db, "day")
            .expect("day models");
        assert_eq!(model_rows.len(), 1);
        assert_eq!(model_rows[0].model, "claude-3-5-sonnet");
        assert_eq!(model_rows[0].total_tokens, 100);

        // total：仍走 usage_event（全量口径权威）
        let (_, _, _, total_all) = UsageEventRepo
            .unified_range_stats(&db, "total")
            .expect("total stats");
        assert_eq!(total_all, 1000);
    }

    /// 趋势日序列携带按工具/按模型拆分（tm_daily_rollup 聚合），供堆叠柱状图使用。
    #[test]
    fn trend_series_carries_per_client_per_model_splits() {
        let db = test_db();
        let ev = |tool: &str, model: &str, input: i64, output: i64, day: &str| {
            let mut e = local_event(tool, input, output, &format!("{day}T00:00:00Z"));
            e.model_normalized = Some(model.into());
            e
        };
        UsageEventRepo
            .insert_batch(
                &db,
                &[
                    ev("claude_code", "claude-3-5-sonnet", 100, 0, "2026-08-01"),
                    ev("codex", "gpt-5.5", 40, 0, "2026-08-01"),
                ],
            )
            .expect("insert day1");
        UsageEventRepo
            .insert_batch(
                &db,
                &[ev("claude_code", "claude-3-5-sonnet", 30, 0, "2026-08-02")],
            )
            .expect("insert day2");

        let series = UsageEventRepo.trend_series_from_db(&db, "total").expect("trend");
        let day1 = series
            .daily
            .iter()
            .find(|d| d.date == "2026-08-01")
            .expect("day1 present");
        assert_eq!(day1.tokens, 140);
        let clients = day1.per_client.as_ref().expect("per_client");
        assert_eq!(clients.len(), 2);
        // 按 token 降序
        assert_eq!(clients[0].key, "claude_code");
        assert_eq!(clients[0].tokens, 100);
        assert_eq!(clients[1].key, "codex");
        assert_eq!(clients[1].tokens, 40);
        let models = day1.per_model.as_ref().expect("per_model");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].key, "claude-3-5-sonnet");
        assert_eq!(models[0].tokens, 100);

        let day2 = series
            .daily
            .iter()
            .find(|d| d.date == "2026-08-02")
            .expect("day2 present");
        let clients2 = day2.per_client.as_ref().expect("day2 per_client");
        assert_eq!(clients2.len(), 1);
        assert_eq!(clients2[0].key, "claude_code");
        assert_eq!(clients2[0].tokens, 30);
    }

    /// 趋势拆分与日总量同源（usage_event）：即使 tm_daily_rollup 过期/被清空
    /// （采集重建时序下可能残留旧值），按工具/按模型拆分仍与 d.tokens 一致，
    /// 趋势页「每日用量」与总览热力图不出现偏差（对齐回归）。
    #[test]
    fn trend_splits_reconcile_with_daily_total_despite_stale_rollup() {
        let db = test_db();
        let ev = |tool: &str, model: &str, input: i64, output: i64, day: &str| {
            let mut e = local_event(tool, input, output, &format!("{day}T00:00:00Z"));
            e.model_normalized = Some(model.into());
            e
        };
        UsageEventRepo
            .insert_batch(
                &db,
                &[
                    ev("claude_code", "claude-3-5-sonnet", 100, 0, "2026-08-01"),
                    ev("codex", "gpt-5.5", 40, 0, "2026-08-01"),
                    ev("freebuff", "", 60, 0, "2026-08-01"),
                ],
            )
            .expect("insert day1");
        // 模拟 rollup 残留过期值（采集重建时序后未重算）
        {
            let conn = db.lock().expect("lock");
            conn.execute("UPDATE tm_daily_rollup SET total_tokens = 1", [])
                .expect("corrupt rollup");
        }

        let series = UsageEventRepo.trend_series_from_db(&db, "total").expect("trend");
        let day1 = series
            .daily
            .iter()
            .find(|d| d.date == "2026-08-01")
            .expect("day1 present");
        assert_eq!(day1.tokens, 200);
        // 拆分项之和必须等于日总量（与总览热力图同源），不被过期 rollup 带偏
        let clients = day1.per_client.as_ref().expect("per_client");
        assert_eq!(clients.iter().map(|s| s.tokens).sum::<i64>(), day1.tokens);
        assert_eq!(
            clients.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(),
            vec!["claude_code", "freebuff", "codex"]
        );
        let models = day1.per_model.as_ref().expect("per_model");
        assert_eq!(models.iter().map(|s| s.tokens).sum::<i64>(), day1.tokens);
        // 缺模型（freebuff 空 model）归入「未知模型」桶，按模型拆分不丢用量
        let unknown = models.iter().find(|s| s.key == "未知模型").expect("unknown bucket");
        assert_eq!(unknown.tokens, 60);
    }

    /// 累计视图按月聚合：必须覆盖所有有使用的月份（跨月），不受 tokscale graph
    /// ~90 天窗口截断——trend.monthly 由 monthly_from_db 全量 GROUP BY month 提供。
    #[test]
    fn trend_monthly_covers_all_usage_months() {
        let db = test_db();
        let ev = |tool: &str, input: i64, output: i64, day: &str| {
            local_event(tool, input, output, &format!("{day}T00:00:00Z"))
        };
        UsageEventRepo
            .insert_batch(
                &db,
                &[
                    ev("claude_code", 100, 0, "2026-03-22"),
                    ev("claude_code", 200, 0, "2026-05-10"),
                    ev("codex", 50, 0, "2026-06-15"),
                    ev("claude_code", 300, 0, "2026-08-01"),
                ],
            )
            .expect("insert across months");

        let series = UsageEventRepo
            .trend_series_from_db(&db, "total")
            .expect("trend");
        let months: Vec<String> = series.monthly.iter().map(|m| m.month.clone()).collect();
        assert_eq!(months, vec!["2026-03", "2026-05", "2026-06", "2026-08"]);
        assert_eq!(series.monthly.iter().map(|m| m.tokens).sum::<i64>(), 650);
    }

    #[test]
    fn insert_batch_is_idempotent_and_updates_rollup() {
        let db = test_db();
        let event = local_event("claude_code", 10, 5, "2026-08-08T00:00:00Z");
        let inserted = UsageEventRepo
            .insert_batch(&db, &[event.clone()])
            .expect("first insert");
        assert_eq!(inserted, 1);
        // 相同事件再插一次 → 幂等跳过
        let inserted_again = UsageEventRepo
            .insert_batch(&db, &[event.clone()])
            .expect("second insert");
        assert_eq!(inserted_again, 0);

        let conn = db.lock().expect("lock");
        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(total_tokens),0) FROM usage_event",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(total, 15);
        let rollup: (String, i64, i64) = conn
            .query_row(
                "SELECT day, total_tokens, request_count FROM tm_daily_rollup",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(rollup.1, 15);
        assert_eq!(rollup.2, 1);
        assert_eq!(rollup.0.len(), 10);
    }

    /// 独立口径：Token Monitor 只统计 usage_event（本地工具）。
    /// 即使本地事件与网关 final row 完全吻合（同 model + 同 token + ±5s），
    /// 也不被丢弃 —— 网关流量归网关仪表盘，TM 不受 request_logs 影响。
    #[test]
    fn token_monitor_is_independent_from_gateway_logs() {
        let db = test_db();
        let gateway_at: String = {
            let conn = db.lock().expect("lock");
            conn.execute_batch(
                "INSERT INTO request_logs (request_id, attempt_count, source, status, model, input_tokens, output_tokens, cache_tokens, request_at)
                 VALUES ('req-a', 1, 'proxy', 'error', 'gpt-4o', 99, 99, 99, datetime('now')),
                        ('req-a', 2, 'proxy', 'success', 'gpt-4o', 12, 8, 2, datetime('now'));",
            )
            .expect("gateway rows");
            conn.query_row("SELECT MAX(request_at) FROM request_logs", [], |row| {
                row.get::<_, String>(0)
            })
            .expect("max request_at")
        };
        // 本地事件：同 model（gpt-4o）+ 同 token + 落在 gateway final row ±5s 内
        let mut local = local_event("claude_code", 12, 8, &iso_utc(&gateway_at));
        local.model_raw = Some("gpt-4o".into());
        local.model_normalized = Some("gpt-4o".into());
        local.cache_read_tokens = Some(2);
        let inserted = UsageEventRepo
            .insert_batch(&db, &[local])
            .expect("local insert");
        assert_eq!(inserted, 1, "本地事件不因网关行存在而被丢弃");

        let (input, output, cache, total) = UsageEventRepo
            .unified_range_stats(&db, "total")
            .expect("stats");
        // 只计 usage_event 的本地事件（12,8,2 → total 20）；网关流量不进 TM
        assert_eq!((input, output, cache, total), (12, 8, 2, 20));
    }

    /// "YYYY-MM-DD HH:MM:SS"（SQLite now，UTC）→ "YYYY-MM-DDTHH:MM:SSZ"（事件语义）。
    fn iso_utc(sqlite_now: &str) -> String {
        format!("{}Z", sqlite_now.trim().replace(' ', "T"))
    }

    /// 独立口径：request_logs 的网关流量不产生工具行（TM 工具列表不再出现
    /// 「PoolGate 网关」），工具维度只有本地工具。
    #[test]
    fn tool_rows_exclude_gateway_logs() {
        let db = test_db();
        {
            let conn = db.lock().expect("lock");
            conn.execute_batch(
                "INSERT INTO request_logs (request_id, attempt_count, source, status, model, input_tokens, output_tokens, request_at)
                 VALUES ('g1', 1, 'proxy', 'success', 'gpt-4o', 100, 100, datetime('now'));",
            )
            .expect("gateway rows");
        }
        let rows = UsageEventRepo.tool_usage_rows(&db, "total").expect("rows");
        // 网关流量独立于 TM：request_logs 不产生工具行
        assert!(
            rows.is_empty(),
            "工具行应只来自 usage_event，实际: {rows:?}"
        );
        assert!(rows.iter().all(|r| r.tool_id != "poolgate_gateway"));
    }

    /// 会话逐轮明细（W7 会话下钻）：只返回该会话的元数据轮次、最新在前、
    /// cache = read+write、成本缺失时按模型估算（不读正文）。
    #[test]
    fn session_event_rows_return_turns_newest_first() {
        let db = test_db();
        let mut e1 = local_event("claude_code", 100, 50, "2026-08-08T08:00:00Z");
        e1.session_id = Some("s1".into());
        e1.cache_read_tokens = Some(20);
        let mut e2 = local_event("claude_code", 40, 10, "2026-08-08T09:00:00Z");
        e2.session_id = Some("s1".into());
        e2.cache_read_tokens = Some(5);
        e2.cache_write_tokens = Some(3);
        e2.reasoning_tokens = Some(2);
        // 其他会话的行不应混入
        let mut other = local_event("codex", 7, 7, "2026-08-08T08:30:00Z");
        other.session_id = Some("s2".into());
        UsageEventRepo
            .insert_batch(&db, &[e1, e2, other])
            .expect("insert");

        let rows = UsageEventRepo
            .session_event_rows(&db, "s1")
            .expect("session events");
        assert_eq!(rows.len(), 2, "只返回 s1 的 2 轮");
        // 最新在前
        assert_eq!(rows[0].occurred_at, "2026-08-08T09:00:00Z");
        assert_eq!(rows[1].occurred_at, "2026-08-08T08:00:00Z");
        // cache = read + write；reasoning 单独带出
        assert_eq!(rows[0].cache_tokens, 8);
        assert_eq!(rows[0].reasoning_tokens, 2);
        assert_eq!(rows[1].cache_tokens, 20);
        // 模型/总量透传
        assert_eq!(rows[0].model.as_deref(), Some("claude-3-5-sonnet"));
        assert_eq!(rows[0].total_tokens, 50);
        // 成本：DB 无真实值 → 按模型估算（claude-3-5-sonnet 在价格表内）
        assert!(rows[0].cost_amount.is_some());
    }

    /// 会话明细 id 兼容：claude/jsonl 适配器的 usage_event.session_id 是文件名
    /// （tm_session.external_session_id），而 tm_session.session_id 是 hash——
    /// 明细查询必须同时匹配两者，否则「列表能点开、明细查不到」。
    #[test]
    fn session_event_rows_match_external_session_id() {
        let db = test_db();
        {
            let conn = db.lock().expect("lock");
            conn.execute_batch(
                "INSERT INTO tool_definition (tool_id, display_name) VALUES ('claude_code', 'Claude Code');
                 INSERT INTO tm_session (session_id, tool_id, external_session_id, message_count)
                     VALUES ('hash-s1', 'claude_code', 'session-file-1', 1);",
            )
            .expect("seed session");
        }
        let mut e = local_event("claude_code", 100, 50, "2026-08-08T08:00:00Z");
        e.session_id = Some("session-file-1".into()); // 适配器写的是文件名
        UsageEventRepo.insert_batch(&db, &[e]).expect("insert");

        let rows = UsageEventRepo
            .session_event_rows(&db, "hash-s1")
            .expect("session events via external id");
        assert_eq!(rows.len(), 1, "应通过 external_session_id 匹配到轮次");
        assert_eq!(rows[0].input_tokens, 100);
    }

    /// 回归（W12 扫描添加冷门 Agent）：tokscale 全量扫描的「快照替换」绝不能清掉
    /// companion 工具（freebuff / atomcode / 自定义应用 `custom:*`）的事件——它们不在
    /// tokscale 覆盖清单内，快照不含其数据，删掉即永久丢失（线上症状：Freebuff 今天
    /// 用了很多但 TOKENS 永远不显示）。
    #[test]
    fn replace_local_snapshot_preserves_companion_events() {
        let db = test_db();
        // covered 工具（claude_code）+ companion 工具（freebuff）
        UsageEventRepo
            .insert_batch(
                &db,
                &[
                    local_event("claude_code", 500, 500, "2026-08-12T08:00:00Z"),
                    local_event("freebuff", 100, 40, "2026-08-12T09:00:00Z"),
                ],
            )
            .expect("insert both");

        // tokscale 新快照只含 covered 工具的事件（旧 covered 数据被替换）
        let snapshot = local_event("claude_code", 10, 5, "2026-08-13T00:00:00Z");
        let covered_ids = crate::token_monitor::collector::tokscale::covered_tool_ids();
        UsageEventRepo
            .replace_local_snapshot(&db, &[snapshot], &covered_ids)
            .expect("replace");

        let rows = UsageEventRepo.tool_usage_rows(&db, "total").expect("rows");
        let freebuff = rows
            .iter()
            .find(|r| r.tool_id == "freebuff")
            .expect("freebuff 事件必须保留");
        assert_eq!(freebuff.total_tokens, 140);
        let claude = rows
            .iter()
            .find(|r| r.tool_id == "claude_code")
            .expect("claude 被新快照替换");
        assert_eq!(claude.total_tokens, 15);

        // rollup 同样保留 companion 用量（重建自剩余 usage_event）
        let rollup: i64 = db
            .lock()
            .expect("lock")
            .query_row(
                "SELECT COALESCE(SUM(total_tokens),0) FROM tm_daily_rollup WHERE tool_id='freebuff'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rollup, 140);
    }

    /// 回归（同上）：day/7d/month 走 tokscale 权威快照口径时，必须叠加 companion 工具
    /// 的 usage_event 用量——快照不含这些工具，不合并则「今日 Tokens」/工具列表/模型
    /// 列表全部漏掉它们。
    #[test]
    fn day_snapshot_merges_companion_tools() {
        let db = test_db();
        // companion 工具今天的用量（快照不含）
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let mut fb = local_event("freebuff", 1000, 100, &now);
        fb.model_normalized = Some("deepseek/deepseek-v4-flash".into());
        UsageEventRepo.insert_batch(&db, &[fb]).expect("insert freebuff");

        // tokscale 权威快照：今天只有 covered 工具 100 tokens
        let snapshot_json = r#"{"groupBy":"client,model","totalInput":40,"totalOutput":20,"totalCacheRead":30,"totalCacheWrite":10,"totalCost":0.5,"entries":[{"client":"claude","model":"claude-3-5-sonnet","input":40,"output":20,"cacheRead":30,"cacheWrite":10,"cost":0.5}]}"#;
        crate::db::settings::SettingsRepo
            .set(&db, "tm.period.day", snapshot_json)
            .expect("set day snapshot");

        // 总量 = 快照 100 + freebuff 1100 = 1200（旧行为只回快照 100）
        let (input, output, cache, total) = UsageEventRepo
            .unified_range_stats(&db, "day")
            .expect("day stats");
        assert_eq!(total, 1200);
        assert_eq!(input, 40 + 1000);
        assert_eq!(output, 20 + 100);
        assert_eq!(cache, 40);

        // 工具行：freebuff（companion）与 claude_code（covered）都在
        let rows = UsageEventRepo.tool_usage_rows(&db, "day").expect("day tools");
        let fb_row = rows
            .iter()
            .find(|r| r.tool_id == "freebuff")
            .expect("freebuff 应在工具列表");
        assert_eq!(fb_row.total_tokens, 1100);
        let claude_row = rows
            .iter()
            .find(|r| r.tool_id == "claude_code")
            .expect("claude 应在工具列表");
        assert_eq!(claude_row.total_tokens, 100);
        // share 按合并后总量重算
        assert!((fb_row.share_percent - 1100.0 * 100.0 / 1200.0).abs() < 1e-6);

        // 模型行：快照模型 + companion 模型都在
        let model_rows = UsageEventRepo
            .model_usage_rows(&db, "day")
            .expect("day models");
        let fb_model = model_rows
            .iter()
            .find(|r| r.model == "deepseek/deepseek-v4-flash")
            .expect("freebuff 模型应在列表");
        assert_eq!(fb_model.total_tokens, 1100);
        let claude_model = model_rows
            .iter()
            .find(|r| r.model == "claude-3-5-sonnet")
            .expect("快照模型应在列表");
        assert_eq!(claude_model.total_tokens, 100);
    }

    /// 关闭监控（enabled=0）后：该工具及其模型的 TOKENS 从所有统计中剔除
    /// （总量/工具/模型），历史数据保留，重新开启即恢复。
    #[test]
    fn disabled_tools_excluded_from_stats() {
        let db = test_db();
        let mut fb = local_event("freebuff", 100, 40, "2026-08-12T09:00:00Z");
        fb.model_normalized = Some("freebuff-model".into());
        UsageEventRepo
            .insert_batch(
                &db,
                &[
                    local_event("claude_code", 500, 500, "2026-08-12T08:00:00Z"),
                    fb,
                ],
            )
            .expect("insert");
        // 关闭 freebuff 监控
        {
            let conn = db.lock().expect("lock");
            conn.execute(
                "UPDATE tool_definition SET enabled=0 WHERE tool_id='freebuff'",
                [],
            )
            .expect("disable");
        }
        // 总量只含 claude_code（freebuff 140 被剔除）
        let (_, _, _, total) = UsageEventRepo
            .unified_range_stats(&db, "total")
            .expect("stats");
        assert_eq!(total, 1000);
        // 工具行无 freebuff
        let rows = UsageEventRepo.tool_usage_rows(&db, "total").expect("rows");
        assert!(rows.iter().all(|r| r.tool_id != "freebuff"));
        assert!(rows.iter().any(|r| r.tool_id == "claude_code"));
        // 模型行无 freebuff 的模型
        let models = UsageEventRepo.model_usage_rows(&db, "total").expect("models");
        assert!(models.iter().all(|r| r.model != "freebuff-model"));
        assert!(models.iter().any(|r| r.model == "claude-3-5-sonnet"));

        // 重新开启 → 数据恢复显示
        {
            let conn = db.lock().expect("lock");
            conn.execute(
                "UPDATE tool_definition SET enabled=1 WHERE tool_id='freebuff'",
                [],
            )
            .expect("re-enable");
        }
        let (_, _, _, total2) = UsageEventRepo
            .unified_range_stats(&db, "total")
            .expect("stats2");
        assert_eq!(total2, 1140);
    }

    /// 关闭的 covered 工具在 tokscale 快照口径（day）下同样被剔除。
    #[test]
    fn disabled_tools_excluded_from_period_snapshot() {
        let db = test_db();
        {
            let conn = db.lock().expect("lock");
            conn.execute(
                "INSERT INTO tool_definition (tool_id, display_name) VALUES ('gemini', 'Gemini')",
                [],
            )
            .expect("seed gemini");
            conn.execute(
                "UPDATE tool_definition SET enabled=0 WHERE tool_id='gemini'",
                [],
            )
            .expect("disable gemini");
        }
        // 快照含 claude（covered）+ gemini 两个工具，顶层 total 是两者之和
        let snapshot_json = r#"{"groupBy":"client,model","totalInput":140,"totalOutput":70,"totalCacheRead":30,"totalCacheWrite":10,"totalCost":1.0,"entries":[
            {"client":"claude","model":"claude-3-5-sonnet","input":40,"output":20,"cacheRead":30,"cacheWrite":10,"cost":0.5},
            {"client":"gemini","model":"gemini-2.5","input":100,"output":50,"cacheRead":0,"cacheWrite":0,"cost":0.5}
        ]}"#;
        crate::db::settings::SettingsRepo
            .set(&db, "tm.period.day", snapshot_json)
            .expect("set day snapshot");

        // day 总量只统计未关闭的工具（claude 40+20+30+10=100），gemini 被剔除
        let (input, output, cache, total) = UsageEventRepo
            .unified_range_stats(&db, "day")
            .expect("day stats");
        assert_eq!((input, output, cache, total), (40, 20, 40, 100));
        let rows = UsageEventRepo.tool_usage_rows(&db, "day").expect("day tools");
        assert!(rows.iter().all(|r| r.tool_id != "gemini"));
        assert!(rows.iter().any(|r| r.tool_id == "claude_code"));
    }

    /// 趋势合并：tokscale graph 序列 + companion 工具（tokscale 未覆盖）逐日用量。
    /// companion 用量直接来自 usage_event（与今日总量同源，保证「趋势·今日」与
    /// 「今日总 TOKENS」一致）；covered 工具的 usage_event 行不合并（graph 权威）。
    /// 日期相对今天生成（避免跨日必挂的连续天数断言）。
    #[test]
    fn trend_merges_companion_daily_usage() {
        let db = test_db();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let yesterday = prev_local_day(&today);
        let two_days_ago = prev_local_day(&yesterday);
        // companion（dsh）+ covered（claude_code）usage_event 行（不同时间避免指纹去重）
        let mut dsh1 = local_event("dsh", 100, 0, &format!("{two_days_ago}T08:00:00Z"));
        dsh1.model_normalized = Some("deepseek-v4-flash".into());
        dsh1.total_tokens = Some(100);
        let mut dsh1b = local_event("dsh", 100, 0, &format!("{two_days_ago}T08:30:00Z"));
        dsh1b.model_normalized = Some("deepseek-v4-flash".into());
        dsh1b.total_tokens = Some(100);
        let mut dsh2 = local_event("dsh", 50, 0, &format!("{yesterday}T09:00:00Z"));
        dsh2.model_normalized = Some("deepseek-v4-flash".into());
        dsh2.total_tokens = Some(50);
        // covered 工具行：graph 权威，不应被合并
        let mut covered = local_event("claude_code", 9999, 0, &format!("{two_days_ago}T10:00:00Z"));
        covered.model_normalized = Some("claude-3-5-sonnet".into());
        covered.total_tokens = Some(9999);
        UsageEventRepo
            .insert_batch(&db, &[dsh1, dsh1b, dsh2, covered])
            .expect("insert usage events");
        // 模拟 tokscale graph 序列：昨天 + 今天只有 covered 工具用量（各 1000 tokens）
        let mut series = TrendSeries {
            daily: vec![
                TrendDay {
                    date: yesterday.clone(),
                    tokens: 1000,
                    requests: 0,
                    cost_amount: None,
                    active_time_ms: 0,
                    per_client: Some(vec![SeriesSplit { key: "workbuddy".into(), tokens: 1000 }]),
                    per_model: Some(vec![SeriesSplit { key: "hy3".into(), tokens: 1000 }]),
                },
                TrendDay {
                    date: today.clone(),
                    tokens: 1000,
                    requests: 0,
                    cost_amount: None,
                    active_time_ms: 0,
                    per_client: Some(vec![SeriesSplit { key: "workbuddy".into(), tokens: 1000 }]),
                    per_model: Some(vec![SeriesSplit { key: "hy3".into(), tokens: 1000 }]),
                },
            ],
            active_days: 2,
            streak_days: 2,
            peak_day: None,
            monthly: vec![TrendMonth { month: today[..7].to_string(), tokens: 2000 }],
            active_time_ms: 0,
            message_count: 10,
        };
        UsageEventRepo
            .merge_companion_trend(&db, &mut series)
            .expect("merge");

        // two_days_ago 是 companion 独有活跃日（2 条 dsh 行 → 200 tokens）→ 新增；
        // yesterday 合并 companion 用量
        assert_eq!(series.daily.len(), 3);
        let d_prev = series
            .daily
            .iter()
            .find(|d| d.date == two_days_ago)
            .expect("two_days_ago");
        assert_eq!(d_prev.tokens, 200);
        assert!(d_prev.per_client.as_ref().unwrap().iter().any(|s| s.key == "dsh"));
        let d_y = series
            .daily
            .iter()
            .find(|d| d.date == yesterday)
            .expect("yesterday");
        assert_eq!(d_y.tokens, 1050, "covered 1000 + companion 50");
        // covered（claude_code 9999）不合并（graph 权威），two_days_ago 只有 dsh 200
        assert!(d_prev
            .per_client
            .as_ref()
            .unwrap()
            .iter()
            .all(|s| s.key != "claude_code"));
        // 汇总重算：活跃 3 天、连续 3 天、月度 = 2250、消息数 = graph 10 + companion 3
        assert_eq!(series.active_days, 3);
        assert_eq!(series.streak_days, 3);
        assert_eq!(series.monthly[0].tokens, 2250);
        assert_eq!(series.message_count, 13);
        assert_eq!(series.peak_day.as_ref().map(|d| d.date.as_str()), Some(yesterday.as_str()));
    }
}
