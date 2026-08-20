//! Logs query/stats/analytics operations

use rusqlite::Connection;
use std::sync::Mutex;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct RequestLog {
    pub id: Option<i64>,
    pub group_id: Option<String>,
    /// Audit identity: virtual client key that authenticated the request.
    pub client_key_id: Option<String>,
    /// Stable identifier shared by every upstream attempt for one client request.
    pub request_id: Option<String>,
    /// 1-based upstream attempt number within the client request.
    pub attempt_count: Option<i64>,
    /// Whether token usage was present and parsed from the upstream response.
    pub usage_available: Option<bool>,
    pub source: Option<String>,
    pub provider_id: Option<String>,
    pub account_id: Option<String>,
    /// Resolved display name from the providers table (via LEFT JOIN).
    pub provider_name: Option<String>,
    /// Resolved display name from the accounts table (via LEFT JOIN).
    pub account_name: Option<String>,
    /// Resolved display name from the agent_groups table (via LEFT JOIN).
    pub group_name: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub status: Option<String>,
    pub status_code: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_tokens: Option<i64>,
    /// Cache read/write split (canonical Usage caliber; Anthropic bills the
    /// two at different rates). `cache_tokens` stays the derived sum.
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub cost: Option<f64>,
    pub latency_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub is_stream: Option<bool>,
    pub error_message: Option<String>,
    pub request_at: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct LogQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub group_id: Option<String>,
    pub status: Option<String>,
    pub source: Option<String>,
    /// Named local-calendar range: today, 6h, 24h, 7d, 30d or 90d.
    pub range: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub keyword: Option<String>,
}

#[derive(serde::Serialize)]
pub struct LogStats {
    pub total_requests: i64,
    pub success_count: i64,
    pub error_count: i64,
    pub rate_limited_count: i64,
    pub timeout_count: i64,
    pub total_tokens: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost: f64,
    pub avg_latency_ms: f64,
}

#[derive(serde::Serialize, Clone, Debug, Default)]
pub struct GroupTrafficStats {
    pub total_requests: i64,
    pub success_count: i64,
    pub error_count: i64,
    pub success_rate: f64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub total_tokens: i64,
    pub total_cost: f64,
    pub avg_latency_ms: f64,
}

#[derive(serde::Serialize, Clone, Debug, Default)]
pub struct ProviderTrafficStats {
    pub provider_id: String,
    pub total_requests: i64,
    pub success_count: i64,
    pub error_count: i64,
    pub success_rate: f64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub total_tokens: i64,
    pub total_cost: f64,
    pub avg_latency_ms: Option<f64>,
    pub p95_latency_ms: Option<f64>,
    pub avg_ttft_ms: Option<f64>,
    pub last_active_at: Option<String>,
}

/// Per-account traffic aggregated from real upstream attempts. Consumed by the
/// provider inspector in the command center topology (V2).
#[derive(serde::Serialize, Clone, Debug, Default)]
pub struct AccountTrafficStats {
    pub account_id: String,
    pub provider_id: String,
    pub total_requests: i64,
    pub success_count: i64,
    pub error_count: i64,
    pub success_rate: f64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub total_tokens: i64,
    pub total_cost: f64,
    pub avg_latency_ms: Option<f64>,
    pub p95_latency_ms: Option<f64>,
    pub avg_ttft_ms: Option<f64>,
    pub last_active_at: Option<String>,
}

#[derive(serde::Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenRangeStats {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub total_tokens: i64,
}

#[derive(serde::Serialize, Clone, Debug, Default, PartialEq)]
pub struct TrayActivityPoint {
    pub hour: i64,
    pub requests: i64,
    pub tokens: i64,
    pub avg_latency_ms: f64,
}

/// One calendar day of usage for the activity heatmap (analytics page and
/// tray command card). `tokens`/`requests` count only the final row of each
/// client request, matching the dashboard semantics.
#[derive(serde::Serialize, Clone, Debug, Default, PartialEq)]
pub struct TrayHeatmapPoint {
    pub date: String,
    pub tokens: i64,
    pub requests: i64,
}

pub struct LogRepo;

impl LogRepo {
    /// Query logs with pagination and optional filters (group_id, status, source, time range, keyword).
    pub fn query(
        &self,
        conn: &Mutex<Connection>,
        query: &LogQuery,
    ) -> Result<Vec<RequestLog>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut sql = String::from(
            "SELECT l.id, l.group_id, l.client_key_id, l.request_id, l.attempt_count, \
             l.usage_available, l.source, l.provider_id, l.account_id, \
             p.name, a.name, g.name, \
             l.model, l.endpoint, l.status, l.status_code, \
             l.input_tokens, l.output_tokens, l.cache_tokens, l.cost, l.latency_ms, l.ttft_ms, \
             l.is_stream, l.error_message, l.request_at, \
             l.cache_read_tokens, l.cache_write_tokens \
             FROM request_logs l \
             LEFT JOIN providers p ON p.id = l.provider_id \
             LEFT JOIN accounts a ON a.id = l.account_id \
             LEFT JOIN agent_groups g ON g.id = l.group_id \
             WHERE 1=1",
        );

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1;

        if let Some(ref gid) = query.group_id {
            sql.push_str(&format!(" AND l.group_id=?{}", param_idx));
            param_values.push(Box::new(gid.clone()));
            param_idx += 1;
        }
        if let Some(ref s) = query.status {
            if s == "failed" {
                sql.push_str(" AND l.status!='success'");
            } else {
                sql.push_str(&format!(" AND l.status=?{}", param_idx));
                param_values.push(Box::new(s.clone()));
                param_idx += 1;
            }
        }
        if let Some(ref src) = query.source {
            sql.push_str(&format!(" AND l.source=?{}", param_idx));
            param_values.push(Box::new(src.clone()));
            param_idx += 1;
        }
        if let Some(ref range) = query.range {
            match range.as_str() {
                "today" => sql.push_str(" AND datetime(l.request_at, 'localtime') >= datetime('now', 'localtime', 'start of day')"),
                "6h" => sql.push_str(" AND datetime(l.request_at, 'localtime') >= datetime('now', 'localtime', '-6 hours')"),
                "24h" => sql.push_str(" AND datetime(l.request_at, 'localtime') >= datetime('now', 'localtime', '-24 hours')"),
                "7d" => sql.push_str(" AND datetime(l.request_at, 'localtime') >= datetime('now', 'localtime', '-7 days')"),
                "30d" => sql.push_str(" AND datetime(l.request_at, 'localtime') >= datetime('now', 'localtime', '-30 days')"),
                "90d" => sql.push_str(" AND datetime(l.request_at, 'localtime') >= datetime('now', 'localtime', '-90 days')"),
                _ => {}
            }
        }
        if let Some(ref start) = query.start_time {
            sql.push_str(&format!(" AND l.request_at >= ?{}", param_idx));
            param_values.push(Box::new(start.clone()));
            param_idx += 1;
        }
        if let Some(ref end) = query.end_time {
            sql.push_str(&format!(" AND l.request_at <= ?{}", param_idx));
            param_values.push(Box::new(end.clone()));
            param_idx += 1;
        }
        if let Some(ref kw) = query.keyword {
            sql.push_str(&format!(
                " AND (l.model LIKE ?{} OR l.endpoint LIKE ?{} OR l.error_message LIKE ?{})",
                param_idx,
                param_idx + 1,
                param_idx + 2
            ));
            let pattern = format!("%{}%", kw);
            param_values.push(Box::new(pattern.clone()));
            param_values.push(Box::new(pattern.clone()));
            param_values.push(Box::new(pattern));
            param_idx += 3;
        }

        sql.push_str(" ORDER BY l.request_at DESC");

        // Pagination
        let page_size = query.page_size.unwrap_or(50);
        let page = query.page.unwrap_or(1);
        let offset = (page - 1) * page_size;
        sql.push_str(&format!(" LIMIT ?{} OFFSET ?{}", param_idx, param_idx + 1));
        param_values.push(Box::new(page_size));
        param_values.push(Box::new(offset));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query(params_ref.as_slice())
            .map_err(|e| e.to_string())?;
        let mut logs = Vec::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            logs.push(RequestLog {
                id: row.get(0).map_err(|e| e.to_string())?,
                group_id: row.get(1).map_err(|e| e.to_string())?,
                client_key_id: row.get(2).map_err(|e| e.to_string())?,
                request_id: row.get(3).map_err(|e| e.to_string())?,
                attempt_count: row.get(4).map_err(|e| e.to_string())?,
                usage_available: row.get(5).map_err(|e| e.to_string())?,
                source: row.get(6).map_err(|e| e.to_string())?,
                provider_id: row.get(7).map_err(|e| e.to_string())?,
                account_id: row.get(8).map_err(|e| e.to_string())?,
                provider_name: row.get(9).map_err(|e| e.to_string())?,
                account_name: row.get(10).map_err(|e| e.to_string())?,
                group_name: row.get(11).map_err(|e| e.to_string())?,
                model: row.get(12).map_err(|e| e.to_string())?,
                endpoint: row.get(13).map_err(|e| e.to_string())?,
                status: row.get(14).map_err(|e| e.to_string())?,
                status_code: row.get(15).map_err(|e| e.to_string())?,
                input_tokens: row.get(16).map_err(|e| e.to_string())?,
                output_tokens: row.get(17).map_err(|e| e.to_string())?,
                cache_tokens: row.get(18).map_err(|e| e.to_string())?,
                cost: row.get(19).map_err(|e| e.to_string())?,
                latency_ms: row.get(20).map_err(|e| e.to_string())?,
                ttft_ms: row.get(21).map_err(|e| e.to_string())?,
                is_stream: row.get(22).map_err(|e| e.to_string())?,
                error_message: row.get(23).map_err(|e| e.to_string())?,
                request_at: row.get(24).map_err(|e| e.to_string())?,
                cache_read_tokens: row.get(25).map_err(|e| e.to_string())?,
                cache_write_tokens: row.get(26).map_err(|e| e.to_string())?,
            });
        }
        Ok(logs)
    }

    /// 按网关账号聚合的用量桶（额度卡 2×2 TOKEN 统计格数据源）。
    ///
    /// key = `request_logs.account_id`（accounts.id）。复用仪表盘「最终请求去重」口径
    /// （ranked CTE：gateway 行优先 + attempt_count 降序 + id 降序），一次扫描同时
    /// 产出今日/昨日/近7天/本月/累计 tokens 与最终请求数。
    /// 返回 (account_id, today, yesterday, week, month, total, requests)。
    pub fn account_token_buckets(
        &self,
        conn: &Mutex<Connection>,
    ) -> Result<Vec<(String, i64, i64, i64, i64, i64, i64)>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "WITH ranked AS (
                     SELECT account_id, request_at,
                            COALESCE(input_tokens,0)+COALESCE(output_tokens,0)+COALESCE(cache_tokens,0) AS tok,
                            ROW_NUMBER() OVER (
                                PARTITION BY COALESCE(request_id, 'legacy:' || id)
                                ORDER BY CASE WHEN source='gateway' THEN 1 ELSE 0 END DESC,
                                         COALESCE(attempt_count, 1) DESC, id DESC
                            ) AS request_rank
                     FROM request_logs
                     WHERE account_id IS NOT NULL AND account_id != ''
                 )
                 SELECT account_id,
                        COALESCE(SUM(CASE WHEN datetime(request_at,'localtime') >= datetime('now','localtime','start of day')
                                          THEN tok ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN date(request_at,'localtime') = date('now','localtime','-1 day')
                                          THEN tok ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN datetime(request_at,'localtime') >= datetime('now','localtime','start of day','-6 days')
                                          THEN tok ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN datetime(request_at,'localtime') >= datetime('now','localtime','start of month')
                                          THEN tok ELSE 0 END),0),
                        COALESCE(SUM(tok),0),
                        COUNT(*)
                 FROM ranked WHERE request_rank=1
                 GROUP BY account_id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// Aggregate tokens for the selected tray-menu range. Request timestamps
    /// are stored in UTC, so the SQLite `localtime` modifier is applied before
    /// comparing calendar boundaries. As with the dashboards, only the final
    /// row for each client request contributes usage.
    pub fn get_token_range_stats(
        &self,
        conn: &Mutex<Connection>,
        range: &str,
    ) -> Result<TokenRangeStats, String> {
        let boundary = match range {
            "today" => "start of day",
            "7d" => "-7 days",
            "30d" => "-30 days",
            "month" => "start of month",
            _ => return Err(format!("不支持的 Token 统计范围：{}", range)),
        };
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "WITH ranked AS (
                 SELECT *, ROW_NUMBER() OVER (
                     PARTITION BY COALESCE(request_id, 'legacy:' || id)
                     ORDER BY CASE WHEN source='gateway' THEN 1 ELSE 0 END DESC,
                              COALESCE(attempt_count, 1) DESC, id DESC
                 ) AS request_rank
                 FROM request_logs
             )
             SELECT COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_tokens), 0),
                    COALESCE(SUM(COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0) + COALESCE(cache_tokens, 0)), 0)
             FROM ranked
             WHERE request_rank=1
               AND datetime(request_at, 'localtime') >= datetime('now', 'localtime', ?1)",
            rusqlite::params![boundary],
            |row| {
                Ok(TokenRangeStats {
                    input_tokens: row.get(0)?,
                    output_tokens: row.get(1)?,
                    cache_tokens: row.get(2)?,
                    total_tokens: row.get(3)?,
                })
            },
        )
        .map_err(|e| e.to_string())
    }

    /// Hourly request and token activity for the local day. Every client request
    /// contributes only its final audit row, matching the dashboard semantics.
    pub fn get_tray_activity(
        &self,
        conn: &Mutex<Connection>,
    ) -> Result<Vec<TrayActivityPoint>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "WITH RECURSIVE hours(hour) AS (
                     SELECT 0 UNION ALL SELECT hour + 1 FROM hours WHERE hour < 23
                 ), ranked AS (
                     SELECT *, ROW_NUMBER() OVER (
                         PARTITION BY COALESCE(request_id, 'legacy:' || id)
                         ORDER BY CASE WHEN source='gateway' THEN 1 ELSE 0 END DESC,
                                  COALESCE(attempt_count, 1) DESC, id DESC
                     ) AS request_rank
                     FROM request_logs
                     WHERE datetime(request_at, 'localtime') >= datetime('now', 'localtime', 'start of day')
                 ), hourly AS (
                     SELECT CAST(strftime('%H', request_at, 'localtime') AS INTEGER) AS hour,
                            COUNT(*) AS requests,
                            COALESCE(SUM(COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0) + COALESCE(cache_tokens, 0)), 0) AS tokens,
                            COALESCE(AVG(latency_ms), 0.0) AS avg_latency_ms
                     FROM ranked WHERE request_rank=1 GROUP BY hour
                 )
                 SELECT hours.hour, COALESCE(hourly.requests, 0), COALESCE(hourly.tokens, 0),
                        COALESCE(hourly.avg_latency_ms, 0.0)
                 FROM hours LEFT JOIN hourly ON hourly.hour=hours.hour ORDER BY hours.hour",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(TrayActivityPoint {
                    hour: row.get(0)?,
                    requests: row.get(1)?,
                    tokens: row.get(2)?,
                    avg_latency_ms: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Daily token/request usage series from the earliest logged request through
    /// today, padded with zero-activity days so the heatmap grid stays
    /// continuous. The window is not limited: every calendar day with (or
    /// between) activity is included, so the full history is shown.
    pub fn get_daily_token_series(
        &self,
        conn: &Mutex<Connection>,
    ) -> Result<Vec<TrayHeatmapPoint>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "WITH RECURSIVE dates(day) AS (
                     SELECT date((SELECT MIN(request_at) FROM request_logs), 'localtime')
                     UNION ALL SELECT date(day, '+1 day')
                     FROM dates WHERE day < date('now', 'localtime')
                 ), ranked AS (
                     SELECT *, ROW_NUMBER() OVER (
                         PARTITION BY COALESCE(request_id, 'legacy:' || id)
                         ORDER BY CASE WHEN source='gateway' THEN 1 ELSE 0 END DESC,
                                  COALESCE(attempt_count, 1) DESC, id DESC
                     ) AS request_rank
                     FROM request_logs
                 )
                 SELECT dates.day,
                        COALESCE(SUM(CASE WHEN ranked.request_rank=1
                          THEN COALESCE(ranked.input_tokens,0)+COALESCE(ranked.output_tokens,0)+COALESCE(ranked.cache_tokens,0)
                          ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN ranked.request_rank=1 THEN 1 ELSE 0 END), 0)
                 FROM dates
                 LEFT JOIN ranked ON date(ranked.request_at, 'localtime') = dates.day
                 WHERE dates.day IS NOT NULL
                 GROUP BY dates.day ORDER BY dates.day",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(TrayHeatmapPoint {
                    date: row.get(0)?,
                    tokens: row.get(1)?,
                    requests: row.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Aggregate statistics by client request while retaining every upstream
    /// attempt in the raw audit log. A gateway row (`attempt_count = 0`) exists
    /// only when no final routed response was emitted, so it takes precedence
    /// over earlier failed attempts for the same request ID.
    pub fn get_stats(&self, conn: &Mutex<Connection>) -> Result<LogStats, String> {
        self.get_stats_range(conn, None)
    }

    /// Aggregate final client requests for a named time range. `today` uses the
    /// local calendar day; rolling ranges use the local SQLite clock.
    pub fn get_stats_range(
        &self,
        conn: &Mutex<Connection>,
        range: Option<&str>,
    ) -> Result<LogStats, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let range_sql = match range {
            Some("today") => " AND datetime(request_at, 'localtime') >= datetime('now', 'localtime', 'start of day')",
            Some("6h") => " AND datetime(request_at, 'localtime') >= datetime('now', 'localtime', '-6 hours')",
            Some("24h") => " AND datetime(request_at, 'localtime') >= datetime('now', 'localtime', '-24 hours')",
            Some("7d") => " AND datetime(request_at, 'localtime') >= datetime('now', 'localtime', '-7 days')",
            Some("30d") => " AND datetime(request_at, 'localtime') >= datetime('now', 'localtime', '-30 days')",
            Some("90d") => " AND datetime(request_at, 'localtime') >= datetime('now', 'localtime', '-90 days')",
            _ => "",
        };
        let sql = format!(
            "WITH ranked AS (
                 SELECT *, ROW_NUMBER() OVER (
                     PARTITION BY COALESCE(request_id, 'legacy:' || id)
                     ORDER BY CASE WHEN source='gateway' THEN 1 ELSE 0 END DESC,
                              COALESCE(attempt_count, 1) DESC, id DESC
                 ) AS request_rank
                 FROM request_logs
             )
             SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN status='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status='error' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status='rate_limited' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status='timeout' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(input_tokens + output_tokens + cache_tokens), 0),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cost), 0.0),
                    COALESCE(AVG(latency_ms), 0.0)
             FROM ranked WHERE request_rank=1{}",
            range_sql,
        );
        conn.query_row(&sql, [], |row| {
            Ok(LogStats {
                total_requests: row.get(0)?,
                success_count: row.get(1)?,
                error_count: row.get(2)?,
                rate_limited_count: row.get(3)?,
                timeout_count: row.get(4)?,
                total_tokens: row.get(5)?,
                total_input_tokens: row.get(6)?,
                total_output_tokens: row.get(7)?,
                total_cost: row.get(8)?,
                avg_latency_ms: row.get(9)?,
            })
        })
        .map_err(|e| e.to_string())
    }

    pub fn get_group_stats(
        &self,
        conn: &Mutex<Connection>,
        group_id: &str,
        range: Option<&str>,
    ) -> Result<GroupTrafficStats, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let modifier = match range.unwrap_or("all") {
            "24h" => Some("-24 hours"),
            "7d" => Some("-7 days"),
            "30d" => Some("-30 days"),
            _ => None,
        };
        // Keep every upstream attempt for audit, but count only the final attempt
        // of each client request in the route-pool dashboard. Legacy rows without
        // request_id remain individually countable through their database id.
        let sql = if modifier.is_some() {
            "WITH ranked AS (
                 SELECT *, ROW_NUMBER() OVER (
                     PARTITION BY COALESCE(request_id, 'legacy:' || id)
                     ORDER BY CASE WHEN source='gateway' THEN 1 ELSE 0 END DESC,
                              COALESCE(attempt_count, 1) DESC, id DESC
                 ) AS request_rank
                 FROM request_logs
                 WHERE group_id=?1 AND request_at >= datetime('now', ?2)
             )
             SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN status='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status!='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_tokens), 0), COALESCE(SUM(cost), 0.0),
                    COALESCE(AVG(latency_ms), 0.0)
             FROM ranked WHERE request_rank=1"
        } else {
            "WITH ranked AS (
                 SELECT *, ROW_NUMBER() OVER (
                     PARTITION BY COALESCE(request_id, 'legacy:' || id)
                     ORDER BY CASE WHEN source='gateway' THEN 1 ELSE 0 END DESC,
                              COALESCE(attempt_count, 1) DESC, id DESC
                 ) AS request_rank
                 FROM request_logs WHERE group_id=?1
             )
             SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN status='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status!='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_tokens), 0), COALESCE(SUM(cost), 0.0),
                    COALESCE(AVG(latency_ms), 0.0)
             FROM ranked WHERE request_rank=1"
        };
        let map = |row: &rusqlite::Row<'_>| -> rusqlite::Result<GroupTrafficStats> {
            let total_requests: i64 = row.get(0)?;
            let success_count: i64 = row.get(1)?;
            let error_count: i64 = row.get(2)?;
            let input_tokens: i64 = row.get(3)?;
            let output_tokens: i64 = row.get(4)?;
            let cache_tokens: i64 = row.get(5)?;
            Ok(GroupTrafficStats {
                total_requests,
                success_count,
                error_count,
                success_rate: if total_requests > 0 {
                    success_count as f64 * 100.0 / total_requests as f64
                } else {
                    0.0
                },
                input_tokens,
                output_tokens,
                cache_tokens,
                total_tokens: input_tokens + output_tokens + cache_tokens,
                total_cost: row.get(6)?,
                avg_latency_ms: row.get(7)?,
            })
        };
        if let Some(modifier) = modifier {
            conn.query_row(sql, rusqlite::params![group_id, modifier], map)
        } else {
            conn.query_row(sql, rusqlite::params![group_id], map)
        }
        .map_err(|e| e.to_string())
    }

    /// Aggregate final routed requests for one protocol across the selected pools.
    /// Protocols are identified from the concrete upstream endpoint rather than
    /// inferred from the pool's compatibility declaration, so Chat and Responses
    /// remain distinct even when they share one route pool.
    pub fn get_protocol_stats(
        &self,
        conn: &Mutex<Connection>,
        protocol: &str,
        pool_ids: &[String],
    ) -> Result<GroupTrafficStats, String> {
        if pool_ids.is_empty() {
            return Ok(GroupTrafficStats::default());
        }
        let endpoint_pattern = match protocol {
            "chat" => "%/v1/chat/completions",
            "responses" => "%/v1/responses",
            "anthropic" => "%/v1/messages",
            "gemini" => "%/v1beta/models%",
            _ => return Ok(GroupTrafficStats::default()),
        };
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let placeholders: Vec<_> = (1..=pool_ids.len())
            .map(|index| format!("?{index}"))
            .collect();
        let endpoint_param = pool_ids.len() + 1;
        let sql = format!(
            "WITH ranked AS (
                 SELECT *, ROW_NUMBER() OVER (
                     PARTITION BY COALESCE(request_id, 'legacy:' || id)
                     ORDER BY CASE WHEN source='proxy' THEN 1 ELSE 0 END DESC,
                              COALESCE(attempt_count, 1) DESC, id DESC
                 ) AS request_rank
                 FROM request_logs
                 WHERE group_id IN ({})
                   AND endpoint LIKE ?{}
             )
             SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN status='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status!='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_tokens), 0),
                    COALESCE(SUM(cost), 0.0),
                    COALESCE(AVG(latency_ms), 0.0)
             FROM ranked WHERE request_rank=1",
            placeholders.join(","),
            endpoint_param,
        );
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = pool_ids
            .iter()
            .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        values.push(Box::new(endpoint_pattern));
        let params: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|value| value.as_ref()).collect();
        conn.query_row(&sql, params.as_slice(), |row| {
            let total_requests: i64 = row.get(0)?;
            let success_count: i64 = row.get(1)?;
            let input_tokens: i64 = row.get(3)?;
            let output_tokens: i64 = row.get(4)?;
            let cache_tokens: i64 = row.get(5)?;
            Ok(GroupTrafficStats {
                total_requests,
                success_count,
                error_count: row.get(2)?,
                success_rate: if total_requests > 0 {
                    success_count as f64 * 100.0 / total_requests as f64
                } else {
                    0.0
                },
                input_tokens,
                output_tokens,
                cache_tokens,
                total_tokens: input_tokens + output_tokens + cache_tokens,
                total_cost: row.get(6)?,
                avg_latency_ms: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())
    }

    /// Aggregate actual upstream attempts by provider for the live topology.
    /// Gateway-only rows are excluded; failover attempts count for the provider
    /// that truly received them, while usage remains attached to the attempt that
    /// reported it. Latency / TTFT percentiles are computed from the 95th rank.
    pub fn get_provider_stats(
        &self,
        conn: &Mutex<Connection>,
    ) -> Result<Vec<ProviderTrafficStats>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "WITH ranked AS (
                     SELECT provider_id, latency_ms,
                            ROW_NUMBER() OVER (
                                PARTITION BY provider_id ORDER BY latency_ms
                            ) AS lat_rank,
                            COUNT(*) OVER (PARTITION BY provider_id) AS lat_total
                     FROM request_logs
                     WHERE provider_id IS NOT NULL
                       AND COALESCE(attempt_count, 1) > 0
                       AND latency_ms IS NOT NULL
                 )
                 SELECT rl.provider_id, COUNT(*),
                        COALESCE(SUM(CASE WHEN rl.status='success' THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN rl.status!='success' THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(rl.input_tokens), 0), COALESCE(SUM(rl.output_tokens), 0),
                        COALESCE(SUM(rl.cache_tokens), 0), COALESCE(SUM(rl.cost), 0.0),
                        AVG(rl.latency_ms), AVG(rl.ttft_ms), MAX(rl.request_at),
                        (SELECT r2.latency_ms FROM ranked r2
                         WHERE r2.provider_id = rl.provider_id
                           AND r2.lat_rank = CAST(r2.lat_total * 0.95 AS INTEGER) + 1
                         LIMIT 1)
                 FROM request_logs rl
                 WHERE rl.provider_id IS NOT NULL AND COALESCE(rl.attempt_count, 1) > 0
                 GROUP BY rl.provider_id ORDER BY COUNT(*) DESC, rl.provider_id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                let input_tokens: i64 = row.get(4)?;
                let output_tokens: i64 = row.get(5)?;
                let cache_tokens: i64 = row.get(6)?;
                let total_requests: i64 = row.get(1)?;
                let success_count: i64 = row.get(2)?;
                Ok(ProviderTrafficStats {
                    provider_id: row.get(0)?,
                    total_requests,
                    success_count,
                    error_count: row.get(3)?,
                    success_rate: if total_requests > 0 {
                        success_count as f64 * 100.0 / total_requests as f64
                    } else {
                        0.0
                    },
                    input_tokens,
                    output_tokens,
                    cache_tokens,
                    total_tokens: input_tokens + output_tokens + cache_tokens,
                    total_cost: row.get(7)?,
                    avg_latency_ms: row.get(8)?,
                    p95_latency_ms: row.get(11)?,
                    avg_ttft_ms: row.get(9)?,
                    last_active_at: row.get(10)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Aggregate actual upstream attempts per account for a set of providers.
    /// Used by the provider inspector to render per-account runtime metrics
    /// without pulling every request row into the topology snapshot.
    pub fn get_account_stats(
        &self,
        conn: &Mutex<Connection>,
        provider_ids: &[String],
    ) -> Result<Vec<AccountTrafficStats>, String> {
        if provider_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let placeholders: Vec<_> = (1..=provider_ids.len())
            .map(|index| format!("?{index}"))
            .collect();
        let in_clause = placeholders.join(",");
        let sql = format!(
            "WITH ranked AS (
                 SELECT account_id, provider_id, latency_ms,
                        ROW_NUMBER() OVER (
                            PARTITION BY account_id ORDER BY latency_ms
                        ) AS lat_rank,
                        COUNT(*) OVER (PARTITION BY account_id) AS lat_total
                 FROM request_logs
                 WHERE account_id IS NOT NULL AND provider_id IN ({in_clause})
                   AND COALESCE(attempt_count, 1) > 0 AND latency_ms IS NOT NULL
             )
             SELECT rl.account_id, rl.provider_id, COUNT(*),
                    COALESCE(SUM(CASE WHEN rl.status='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN rl.status!='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(rl.input_tokens), 0), COALESCE(SUM(rl.output_tokens), 0),
                    COALESCE(SUM(rl.cache_tokens), 0), COALESCE(SUM(rl.cost), 0.0),
                    AVG(rl.latency_ms), AVG(rl.ttft_ms), MAX(rl.request_at),
                    (SELECT r2.latency_ms FROM ranked r2
                     WHERE r2.account_id = rl.account_id
                       AND r2.lat_rank = CAST(r2.lat_total * 0.95 AS INTEGER) + 1
                     LIMIT 1)
             FROM request_logs rl
             WHERE rl.account_id IS NOT NULL AND rl.provider_id IN ({in_clause})
               AND COALESCE(rl.attempt_count, 1) > 0
             GROUP BY rl.account_id, rl.provider_id
             ORDER BY COUNT(*) DESC, rl.account_id",
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        // Both IN clauses intentionally reuse the same numbered placeholders,
        // so SQLite expects one bound value per provider, not two copies.
        let params = rusqlite::params_from_iter(provider_ids.iter().map(|id| id.as_str()));
        let rows = stmt
            .query_map(params, |row| {
                let total_requests: i64 = row.get(2)?;
                let success_count: i64 = row.get(3)?;
                let input_tokens: i64 = row.get(5)?;
                let output_tokens: i64 = row.get(6)?;
                let cache_tokens: i64 = row.get(7)?;
                Ok(AccountTrafficStats {
                    account_id: row.get(0)?,
                    provider_id: row.get(1)?,
                    total_requests,
                    success_count,
                    error_count: row.get(4)?,
                    success_rate: if total_requests > 0 {
                        success_count as f64 * 100.0 / total_requests as f64
                    } else {
                        0.0
                    },
                    input_tokens,
                    output_tokens,
                    cache_tokens,
                    total_tokens: input_tokens + output_tokens + cache_tokens,
                    total_cost: row.get(8)?,
                    avg_latency_ms: row.get(9)?,
                    p95_latency_ms: row.get(12)?,
                    avg_ttft_ms: row.get(10)?,
                    last_active_at: row.get(11)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Returns range-aware usage analytics using the same final-request
    /// deduplication rule as the dashboard and log KPI cards.
    pub fn get_analytics(
        &self,
        conn: &Mutex<Connection>,
        start_date: Option<&str>,
        end_date: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        // Default: last 7 days
        let default_start = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(7))
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "2020-01-01".to_string());
        let default_end = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let start = start_date.unwrap_or(&default_start);
        let end = end_date.unwrap_or(&default_end);

        // Convert datetime-local format (YYYY-MM-DDThh:mm) to SQLite format (YYYY-MM-DD hh:mm)
        let start_sql = start.replace('T', " ");
        let end_sql = end.replace('T', " ");

        // Activity heatmap: every calendar day from the first logged request
        // through today (zero days padded), independent of the selected range.
        // Fetched before the conn lock below since this repo method locks on its own.
        let heatmap = self.get_daily_token_series(conn)?;

        let period = "date(request_at, 'localtime')";
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let ranked_cte = format!(
            "WITH ranked AS (
                 SELECT *, ROW_NUMBER() OVER (
                     PARTITION BY COALESCE(request_id, 'legacy:' || id)
                     ORDER BY CASE WHEN source='gateway' THEN 1 ELSE 0 END DESC,
                              COALESCE(attempt_count, 1) DESC, id DESC
                 ) AS request_rank
                 FROM request_logs
                 WHERE datetime(request_at, 'localtime') >= ?1
                   AND datetime(request_at, 'localtime') < ?2
             )"
        );

        // Compute summary inline using the same ranked CTE
        let summary_sql = format!(
            "{ranked_cte}
             SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN status='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status!='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_tokens), 0),
                    COALESCE(SUM(cost), 0.0),
                    COALESCE(AVG(latency_ms), 0.0)
             FROM ranked WHERE request_rank=1"
        );
        let start_param = start_sql.clone();
        let end_param = end_sql.clone();
        let summary: serde_json::Value = conn
            .query_row(
                &summary_sql,
                rusqlite::params![start_param, end_param],
                |row| {
                    Ok(serde_json::json!({
                        "total_requests": row.get::<_, i64>(0)?,
                        "success_count": row.get::<_, i64>(1)?,
                        "error_count": row.get::<_, i64>(2)?,
                        "total_tokens": row.get::<_, i64>(3)?,
                        "total_input_tokens": row.get::<_, i64>(4)?,
                        "total_output_tokens": row.get::<_, i64>(5)?,
                        "total_cost": row.get::<_, f64>(6)?,
                        "avg_latency_ms": row.get::<_, f64>(7)?,
                    }))
                },
            )
            .map_err(|e| e.to_string())?;

        let daily_sql = format!(
            "{ranked_cte}
             SELECT {period} AS period,
                    COUNT(*),
                    COALESCE(SUM(CASE WHEN status='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status!='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_tokens), 0),
                    COALESCE(SUM(cost), 0.0),
                    COALESCE(AVG(latency_ms), 0.0)
             FROM ranked WHERE request_rank=1
             GROUP BY period ORDER BY period"
        );
        let mut stmt = conn.prepare(&daily_sql).map_err(|e| e.to_string())?;
        let daily = stmt
            .query_map(rusqlite::params![start_param, end_param], |row| {
                Ok(serde_json::json!({
                    "date": row.get::<_, String>(0)?,
                    "total_requests": row.get::<_, i64>(1)?,
                    "success_count": row.get::<_, i64>(2)?,
                    "error_count": row.get::<_, i64>(3)?,
                    "input_tokens": row.get::<_, i64>(4)?,
                    "output_tokens": row.get::<_, i64>(5)?,
                    "cache_tokens": row.get::<_, i64>(6)?,
                    "total_cost": row.get::<_, f64>(7)?,
                    "avg_latency_ms": row.get::<_, f64>(8)?,
                }))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        let model_sql = format!(
            "{ranked_cte}
             SELECT model, COUNT(*),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cost), 0.0)
             FROM ranked
             WHERE request_rank=1 AND model IS NOT NULL AND model!=''
             GROUP BY model ORDER BY COUNT(*) DESC"
        );
        let mut stmt = conn.prepare(&model_sql).map_err(|e| e.to_string())?;
        let model_distribution = stmt
            .query_map(rusqlite::params![start_param, end_param], |row| {
                Ok(serde_json::json!({
                    "model": row.get::<_, String>(0)?,
                    "count": row.get::<_, i64>(1)?,
                    "input_tokens": row.get::<_, i64>(2)?,
                    "output_tokens": row.get::<_, i64>(3)?,
                    "total_cost": row.get::<_, f64>(4)?,
                }))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        let account_sql = format!(
            "{ranked_cte}
             SELECT r.account_id,
                    COALESCE(NULLIF(a.name, ''), NULLIF(a.email, ''), r.account_id),
                    COUNT(*),
                    COALESCE(SUM(CASE WHEN r.status='success' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(COALESCE(r.input_tokens, 0) + COALESCE(r.output_tokens, 0) + COALESCE(r.cache_tokens, 0)), 0),
                    COALESCE(SUM(r.cost), 0.0)
             FROM ranked r
             LEFT JOIN accounts a ON a.id=r.account_id
             WHERE r.request_rank=1 AND r.account_id IS NOT NULL AND r.account_id!=''
             GROUP BY r.account_id, a.name, a.email
             ORDER BY COUNT(*) DESC LIMIT 20"
        );
        let mut stmt = conn.prepare(&account_sql).map_err(|e| e.to_string())?;
        let account_ranking = stmt
            .query_map(rusqlite::params![start_param, end_param], |row| {
                Ok(serde_json::json!({
                    "account_id": row.get::<_, String>(0)?,
                    "name": row.get::<_, String>(1)?,
                    "count": row.get::<_, i64>(2)?,
                    "success_count": row.get::<_, i64>(3)?,
                    "total_tokens": row.get::<_, i64>(4)?,
                    "total_cost": row.get::<_, f64>(5)?,
                }))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(serde_json::json!({
            "summary": summary,
            "daily": daily,
            "model_distribution": model_distribution,
            "account_ranking": account_ranking,
            "heatmap": heatmap,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        .expect("migrate route pool management");
        conn.execute_batch(include_str!(
            "../../migrations/021_request_log_cache_split.sql"
        ))
        .expect("migrate cache split");
        Mutex::new(conn)
    }

    #[test]
    fn daily_token_series_pads_zero_days_and_dedupes_final_attempt() {
        let db = test_db();
        let conn = db.lock().expect("lock database");
        conn.execute_batch(
            "INSERT INTO request_logs
                (request_id, attempt_count, source, status, input_tokens, output_tokens, cache_tokens, request_at)
             VALUES
                ('req-a', 1, 'proxy', 'error', 99, 99, 99, datetime('now')),
                ('req-a', 2, 'proxy', 'success', 12, 8, 2, datetime('now')),
                ('req-b', 1, 'proxy', 'success', 100, 50, 5, datetime('now', '-2 days'));",
        )
        .expect("insert logs");
        drop(conn);

        let series = LogRepo
            .get_daily_token_series(&db)
            .expect("daily token series");
        // Spans the earliest log (2 days ago) through today, padded to 3 days.
        assert_eq!(series.len(), 3);
        let today = series.last().expect("today");
        assert_eq!(today.tokens, 22);
        assert_eq!(today.requests, 1);
        let two_days_ago = series.first().expect("two days ago");
        assert_eq!(two_days_ago.tokens, 155);
        assert_eq!(two_days_ago.requests, 1);
        // Zero-activity days are present and padded to zero.
        assert!(series
            .iter()
            .any(|point| point.tokens == 0 && point.requests == 0));
    }

    #[test]
    fn token_range_stats_use_local_calendar_and_final_request_row() {
        let db = test_db();
        let conn = db.lock().expect("lock database");
        conn.execute_batch(
            "INSERT INTO request_logs
                (request_id, attempt_count, source, status, input_tokens, output_tokens, cache_tokens, request_at)
             VALUES
                ('request-today', 1, 'proxy', 'error', 99, 99, 99, datetime('now')),
                ('request-today', 2, 'proxy', 'success', 12, 8, 2, datetime('now')),
                ('request-old', 1, 'proxy', 'success', 100, 50, 5, datetime('now', '-40 days'));",
        )
        .expect("insert logs");
        drop(conn);

        let today = LogRepo
            .get_token_range_stats(&db, "today")
            .expect("today tokens");
        assert_eq!(
            today,
            TokenRangeStats {
                input_tokens: 12,
                output_tokens: 8,
                cache_tokens: 2,
                total_tokens: 22,
            }
        );
        let thirty_days = LogRepo
            .get_token_range_stats(&db, "30d")
            .expect("30 day tokens");
        assert_eq!(thirty_days.total_tokens, 22);
        assert!(LogRepo.get_token_range_stats(&db, "invalid").is_err());
    }

    #[test]
    fn account_token_buckets_split_by_calendar_windows_and_dedupe_final_row() {
        let db = test_db();
        let conn = db.lock().expect("lock database");
        conn.execute_batch(
            "INSERT INTO request_logs
                (account_id, request_id, attempt_count, source, status, input_tokens, output_tokens, cache_tokens, request_at)
             VALUES
                ('acc-1', 'r1', 1, 'proxy', 'error', 99, 0, 0, datetime('now')),
                ('acc-1', 'r1', 2, 'proxy', 'success', 12, 8, 2, datetime('now')),
                ('acc-1', 'r2', 1, 'proxy', 'success', 100, 50, 5, datetime('now', '-1 day')),
                ('acc-1', 'r3', 1, 'proxy', 'success', 40, 10, 0, datetime('now', '-8 days')),
                ('acc-2', 'r4', 1, 'proxy', 'success', 10, 5, 1, datetime('now', '-40 days')),
                (NULL,    'r5', 1, 'proxy', 'success', 999, 999, 999, datetime('now'));",
        )
        .expect("insert logs");
        drop(conn);

        let buckets = LogRepo
            .account_token_buckets(&db)
            .expect("account token buckets");
        assert_eq!(buckets.len(), 2, "NULL account_id excluded");
        let acc1 = buckets
            .iter()
            .find(|(id, ..)| id == "acc-1")
            .expect("acc-1 present");
        // 今日 22（最终行 12+8+2），昨日 155，近7天 22+155=177
        assert_eq!(acc1.1, 22);
        assert_eq!(acc1.2, 155);
        assert_eq!(acc1.3, 177);
        // 本月含 -8 天的 r3(50)：跨月则本月=177，同月则本月=227（日期无关断言）
        assert!(acc1.4 == 177 || acc1.4 == 227);
        // 累计恒为 22 + 155 + 50 = 227
        assert_eq!(acc1.5, 227);
        assert_eq!(acc1.6, 3);
        let acc2 = buckets
            .iter()
            .find(|(id, ..)| id == "acc-2")
            .expect("acc-2 present");
        assert_eq!(acc2.1, 0);
        assert_eq!(acc2.2, 0);
        assert_eq!(acc2.3, 0);
        assert_eq!(acc2.4, 0); // -40 天不在本月
        assert_eq!(acc2.5, 16);
        assert_eq!(acc2.6, 1);
    }

    #[test]
    fn aggregate_stats_count_each_client_request_once() {
        let db = test_db();
        let conn = db.lock().expect("lock database");
        conn.execute_batch(
            "INSERT INTO request_logs
                (group_id, request_id, attempt_count, source, status, status_code, input_tokens, output_tokens, cache_tokens, latency_ms)
             VALUES
                ('pool-1', 'request-a', 1, 'proxy', 'error', 429, NULL, NULL, NULL, 10),
                ('pool-1', 'request-a', 2, 'proxy', 'success', 200, 12, 8, 2, 20),
                ('pool-1', 'request-b', 0, 'gateway', 'error', 401, NULL, NULL, NULL, 5),
                ('pool-1', 'request-c', 1, 'proxy', 'error', 502, NULL, NULL, NULL, 15),
                ('pool-1', 'request-c', 0, 'gateway', 'error', 503, NULL, NULL, NULL, 18);",
        )
        .expect("insert logs");
        drop(conn);

        let stats = LogRepo.get_stats(&db).expect("get stats");
        assert_eq!(stats.total_requests, 3);
        assert_eq!(stats.success_count, 1);
        assert_eq!(stats.error_count, 2);
        assert_eq!(stats.total_input_tokens, 12);
        assert_eq!(stats.total_output_tokens, 8);
        assert_eq!(stats.total_tokens, 22);
        assert_eq!(stats.avg_latency_ms, 43.0 / 3.0);
    }

    #[test]
    fn provider_stats_count_final_routed_request_once() {
        let db = test_db();
        let conn = db.lock().expect("lock database");
        conn.execute_batch(
            "INSERT INTO request_logs
                (provider_id, request_id, attempt_count, source, status, status_code, input_tokens, output_tokens, cache_tokens)
             VALUES
                ('provider-a', 'request-a', 1, 'proxy', 'error', 429, NULL, NULL, NULL),
                ('provider-b', 'request-a', 2, 'proxy', 'success', 200, 12, 8, 2),
                ('provider-a', 'request-b', 1, 'proxy', 'error', 502, NULL, NULL, NULL),
                (NULL, 'request-b', 0, 'gateway', 'error', 503, NULL, NULL, NULL),
                ('provider-a', 'request-c', 1, 'proxy', 'success', 200, 5, 3, 0);",
        )
        .expect("insert logs");
        drop(conn);

        let stats = LogRepo.get_provider_stats(&db).expect("provider stats");
        assert_eq!(stats.len(), 2);
        let provider_a = stats
            .iter()
            .find(|item| item.provider_id == "provider-a")
            .expect("provider a");
        assert_eq!(provider_a.total_requests, 3);
        assert_eq!(provider_a.success_count, 1);
        assert_eq!(provider_a.error_count, 2);
        assert_eq!(provider_a.total_tokens, 8);
        let provider_b = stats
            .iter()
            .find(|item| item.provider_id == "provider-b")
            .expect("provider b");
        assert_eq!(provider_b.total_requests, 1);
        assert_eq!(provider_b.success_count, 1);
        assert_eq!(provider_b.total_tokens, 22);
    }

    #[test]
    fn protocol_stats_are_separated_by_upstream_endpoint() {
        let db = test_db();
        let conn = db.lock().expect("lock database");
        conn.execute_batch(
            "INSERT INTO request_logs
                (group_id, request_id, attempt_count, source, endpoint, status, input_tokens, output_tokens, cache_tokens, latency_ms)
             VALUES
                ('pool-1', 'chat-1', 1, 'proxy', '/v1/chat/completions', 'success', 10, 5, 1, 100),
                ('pool-1', 'response-1', 1, 'proxy', '/v1/responses', 'success', 20, 8, 2, 200),
                ('pool-1', 'chat-2', 1, 'proxy', '/v1/chat/completions', 'error', 4, 2, 0, 300);",
        )
        .expect("insert protocol logs");
        drop(conn);

        let chat = LogRepo
            .get_protocol_stats(&db, "chat", &["pool-1".into()])
            .expect("chat protocol stats");
        assert_eq!(chat.total_requests, 2);
        assert_eq!(chat.success_count, 1);
        assert_eq!(chat.total_tokens, 22);
        assert_eq!(chat.avg_latency_ms, 200.0);

        let responses = LogRepo
            .get_protocol_stats(&db, "responses", &["pool-1".into()])
            .expect("responses protocol stats");
        assert_eq!(responses.total_requests, 1);
        assert_eq!(responses.success_count, 1);
        assert_eq!(responses.total_tokens, 30);
        assert_eq!(responses.avg_latency_ms, 200.0);
    }

    #[test]
    fn account_stats_bind_provider_ids_once_for_reused_placeholders() {
        let db = test_db();
        let conn = db.lock().expect("lock database");
        conn.execute_batch(
            "INSERT INTO request_logs
                (provider_id, account_id, attempt_count, source, status, latency_ms)
             VALUES
                ('provider-a', 'account-a', 1, 'proxy', 'success', 120);",
        )
        .expect("insert account log");
        drop(conn);

        let stats = LogRepo
            .get_account_stats(&db, &["provider-a".into()])
            .expect("account stats");
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].account_id, "account-a");
        assert_eq!(stats[0].total_requests, 1);
    }

    #[test]
    fn group_stats_count_final_attempt_once() {
        let db = test_db();
        let conn = db.lock().expect("lock database");
        conn.execute_batch(
            "INSERT INTO request_logs
                (group_id, request_id, attempt_count, source, status, status_code, input_tokens, output_tokens, cache_tokens, latency_ms)
             VALUES
                ('pool-1', 'request-a', 1, 'proxy', 'error', 429, NULL, NULL, NULL, 10),
                ('pool-1', 'request-a', 2, 'proxy', 'success', 200, 12, 8, 2, 20),
                ('pool-1', 'request-b', 1, 'proxy', 'error', 500, NULL, NULL, NULL, 30),
                ('pool-1', 'request-b', 0, 'gateway', 'error', 503, NULL, NULL, NULL, 35);",
        )
        .expect("insert logs");
        drop(conn);

        let stats = LogRepo
            .get_group_stats(&db, "pool-1", Some("all"))
            .expect("get stats");
        assert_eq!(stats.total_requests, 2);
        assert_eq!(stats.success_count, 1);
        assert_eq!(stats.error_count, 1);
        assert_eq!(stats.input_tokens, 12);
        assert_eq!(stats.output_tokens, 8);
        assert_eq!(stats.cache_tokens, 2);
        assert_eq!(stats.total_tokens, 22);
        assert_eq!(stats.avg_latency_ms, 27.5);
    }
}

#[cfg(test)]
mod status_filter_query {
    use super::*;

    fn status_db() -> Mutex<Connection> {
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
        .expect("migrate route pool management");
        conn.execute_batch(include_str!(
            "../../migrations/021_request_log_cache_split.sql"
        ))
        .expect("migrate cache split");
        conn.execute_batch(
            "INSERT INTO request_logs (request_id, attempt_count, source, status, status_code, request_at)
             VALUES
                ('r1', 1, 'proxy', 'success', 200, datetime('now')),
                ('r2', 1, 'proxy', 'error', 502, datetime('now')),
                ('r3', 1, 'proxy', 'error', 503, datetime('now')),
                ('r4', 1, 'proxy', 'success', 200, datetime('now')),
                ('r5', 1, 'proxy', 'error', 400, datetime('now')),
                ('r6', 1, 'proxy', 'success', 200, datetime('now')),
                ('r7', 1, 'proxy', 'error', 403, datetime('now'));",
        )
        .expect("insert logs");
        Mutex::new(conn)
    }

    fn q(status: Option<&str>) -> LogQuery {
        LogQuery {
            page: Some(1),
            page_size: Some(50),
            group_id: None,
            status: status.map(|s| s.to_string()),
            source: None,
            range: Some("24h".to_string()),
            start_time: None,
            end_time: None,
            keyword: None,
        }
    }

    #[test]
    fn status_filter_query_success_and_failed() {
        let db = status_db();

        let all = LogRepo.query(&db, &q(None)).expect("all");
        assert_eq!(all.len(), 7, "all should be 7");

        let success = LogRepo.query(&db, &q(Some("success"))).expect("success");
        assert_eq!(success.len(), 3, "success should be 3");

        let failed = LogRepo.query(&db, &q(Some("failed"))).expect("failed");
        assert_eq!(failed.len(), 4, "failed should be 4");

        let bad = LogRepo.query(&db, &q(Some("bogus"))).expect("bogus");
        assert_eq!(bad.len(), 0, "bogus status should be 0");
    }
}
