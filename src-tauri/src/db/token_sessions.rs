//! 会话摘要 / 项目归属（W1 实现）。
//!
//! 隐私红线：只存摘要，不存正文；`title_redacted` 默认「项目名+时间」生成，
//! 非首条 Prompt；`tm_project` 只存不可逆 `canonical_path_hash`，完整路径不入库。

use rusqlite::Connection;
use std::sync::Mutex;

use crate::token_monitor::model::{ProjectRow, SessionSummary};

pub struct SessionRepo;

/// tokscale 覆盖工具列表（SQL 片段，tm_session 过滤时区分权威成员口径与 companion 日期口径）。
/// 清单为编译期常量，无注入风险。
fn covered_ids_sql() -> String {
    let ids = crate::token_monitor::collector::tokscale::covered_tool_ids();
    let quoted: Vec<String> = ids.iter().map(|id| format!("'{id}'")).collect();
    if quoted.is_empty() {
        "'__no_covered_tools__'".to_string()
    } else {
        quoted.join(",")
    }
}

impl SessionRepo {
    /// Upsert 会话摘要（model_set 存 JSON、token 累加、message_count 累加）。
    /// 先确保 tool_definition 存在（tm_session.tool_id 有 FK），避免约束失败。
    pub fn upsert_session(
        &self,
        conn: &Mutex<Connection>,
        session: &SessionSummary,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let _ = conn.execute(
            "INSERT INTO tool_definition (tool_id, display_name, kind, support_level) \
             VALUES (?1, ?2, 'usage', 'basic') \
             ON CONFLICT(tool_id) DO NOTHING",
            rusqlite::params![
                session.tool_id,
                crate::db::token_usage::display_name_for_tool(&session.tool_id)
            ],
        );
        // 读取既有 model_set 做合并（不覆盖历史模型集合）
        let existing_model_set: Option<String> = conn
            .query_row(
                "SELECT model_set_json FROM tm_session WHERE session_id=?1",
                rusqlite::params![session.session_id],
                |row| row.get(0),
            )
            .ok();
        let mut merged: Vec<String> = existing_model_set
            .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
            .unwrap_or_default();
        for model in &session.model_set {
            if !merged.contains(model) {
                merged.push(model.clone());
            }
        }
        // 归一模型别名：同一底层模型的新旧写法（如 MAAS 端点名 maas_cl_opus_4.8_*
        // 与 claude-opus-4-8）归一为一个，避免 model_set 里并存两个等价名称。
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        merged = merged
            .into_iter()
            .map(|model| {
                crate::token_monitor::normalization::alias_model(&model)
                    .map(str::to_string)
                    .unwrap_or(model)
            })
            .filter(|model| seen.insert(model.clone()))
            .collect();
        let model_set_json = serde_json::to_string(&merged).unwrap_or_else(|_| "[]".into());

        conn.execute(
            "INSERT INTO tm_session (
                session_id, tool_id, device_id, external_session_id, project_id,
                title_redacted, model_set_json, started_at, last_active_at,
                input_tokens, output_tokens, cache_tokens, total_tokens,
                message_count, status
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
             ON CONFLICT(session_id) DO UPDATE SET
                tool_id=excluded.tool_id,
                external_session_id=excluded.external_session_id,
                project_id=COALESCE(excluded.project_id, tm_session.project_id),
                title_redacted=COALESCE(excluded.title_redacted, tm_session.title_redacted),
                model_set_json=excluded.model_set_json,
                started_at=COALESCE(tm_session.started_at, excluded.started_at),
                last_active_at=excluded.last_active_at,
                input_tokens=excluded.input_tokens, output_tokens=excluded.output_tokens,
                cache_tokens=excluded.cache_tokens, total_tokens=excluded.total_tokens,
                message_count=excluded.message_count, status=excluded.status,
                updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')",
            rusqlite::params![
                session.session_id,
                session.tool_id,
                "local",
                session.external_session_id,
                session.project_id,
                session.title_redacted,
                model_set_json,
                session.started_at,
                session.last_active_at,
                session.input_tokens,
                session.output_tokens,
                session.cache_tokens,
                session.total_tokens,
                session.message_count,
                session.status,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 按过滤器查询会话（list_active_sessions 数据源），按最近活跃倒序。
    /// range 为 day|7d|month|total：
    /// - **day|7d|month**：按 tokscale 周期扫描的会话成员过滤（`tm_period_session`，
    ///   对齐开源 Token Monitor——开源会话视图的「今日/近7天/本月」正是各周期扫描
    ///   `--today/--week/--month` 返回的会话集合）；成员表缺失时（旧库/采集未跑）
    ///   回退按 started_at/last_active_at 日期过滤，保证兼容。
    /// - **total**：不过滤。
    pub fn list_sessions(
        &self,
        conn: &Mutex<Connection>,
        tool_id: Option<&str>,
        project_id: Option<&str>,
        limit: Option<i64>,
        range: Option<&str>,
    ) -> Result<Vec<SessionSummary>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let limit = limit.unwrap_or(50).clamp(1, 500);
        let mut sql = String::from(
            "SELECT session_id, tool_id, external_session_id, project_id, title_redacted,
                    model_set_json, started_at, last_active_at,
                    COALESCE(input_tokens,0), COALESCE(output_tokens,0),
                    COALESCE(cache_tokens,0), COALESCE(total_tokens,0),
                    COALESCE(message_count,0), status
             FROM tm_session WHERE 1=1
               AND tool_id IN (SELECT tool_id FROM tool_definition WHERE enabled=1)",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(range) = range.filter(|r| !r.is_empty() && *r != "total") {
            // W9：周期成员过滤（对齐开源）——命中 `tm_period_session` 则按成员过滤；
            // 成员表为空（旧库/采集未刷新）时回退旧日期口径，避免会话列表骤空。
            let member_ids: Vec<String> = {
                let mut stmt = conn
                    .prepare(
                        "SELECT session_id FROM tm_period_session WHERE period=?1",
                    )
                    .map_err(|e| e.to_string())?;
                let ids = stmt
                    .query_map(rusqlite::params![range], |row| row.get(0))
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<String>, _>>()
                    .map_err(|e| e.to_string())?;
                drop(stmt);
                ids
            };
            if !member_ids.is_empty() {
                // tm_session.session_id 是 `{tool}:{external}`，external 与成员 sessionId 同值
                let placeholders = (0..member_ids.len())
                    .map(|_| "?".to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                // W12 companion 工具合并：tm_period_session 只含 tokscale 覆盖工具的会话
                // 成员（对齐开源口径）；tokscale 未覆盖的工具（freebuff / atomcode /
                // 自定义应用）不在成员表，按日期口径回退过滤，否则「今日/近7天/本月」
                // 会话列表会漏掉它们。
                let covered = covered_ids_sql();
                let start = crate::db::token_usage::range_start_sql(range);
                sql.push_str(&format!(
                    " AND (\n                    (tool_id IN ({covered}) AND (external_session_id IN ({placeholders}) OR session_id IN ({placeholders})))\n                  OR (tool_id NOT IN ({covered}) AND (datetime(last_active_at,'localtime') >= {start} OR datetime(started_at,'localtime') >= {start}))\n                  )"
                ));
                for id in member_ids.iter() {
                    params.push(Box::new(id.clone()));
                }
                for id in member_ids.iter() {
                    params.push(Box::new(id.clone()));
                }
            } else {
                let start = crate::db::token_usage::range_start_sql(range);
                sql.push_str(&format!(
                    " AND (datetime(last_active_at,'localtime') >= {start} OR datetime(started_at,'localtime') >= {start})"
                ));
            }
        }
        if let Some(tool) = tool_id {
            sql.push_str(" AND tool_id=?1");
            params.push(Box::new(tool.to_string()));
        }
        if let Some(project) = project_id {
            sql.push_str(&format!(" AND project_id=?{}", params.len() + 1));
            params.push(Box::new(project.to_string()));
        }
        // 会话按 TOKENS 总量降序（用户要求），并列时最近活跃优先。
        sql.push_str(&format!(
            " ORDER BY total_tokens DESC, last_active_at DESC LIMIT ?{}",
            params.len() + 1
        ));
        params.push(Box::new(limit));
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                let model_set: Vec<String> =
                    serde_json::from_str(&row.get::<_, String>(5).unwrap_or_else(|_| "[]".into()))
                        .unwrap_or_default();
                let input_tokens: i64 = row.get(8)?;
                let output_tokens: i64 = row.get(9)?;
                let cache_tokens: i64 = row.get(10)?;
                let cost_amount = model_set.first().and_then(|model| {
                    crate::token_monitor::pricing::estimate_cost(
                        model,
                        input_tokens,
                        output_tokens,
                        cache_tokens,
                    )
                });
                Ok(SessionSummary {
                    session_id: row.get(0)?,
                    tool_id: row.get(1)?,
                    external_session_id: row.get(2)?,
                    project_id: row.get(3)?,
                    title_redacted: row.get(4)?,
                    model_set,
                    started_at: row.get(6)?,
                    last_active_at: row.get(7)?,
                    input_tokens,
                    output_tokens,
                    cache_tokens,
                    total_tokens: row.get(11)?,
                    message_count: row.get(12)?,
                    status: row.get(13)?,
                    cost_amount,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// tokscale 权威模式会话投影（W8）：从 usage_event 按 (tool_id, session_id) 聚合
    /// 生成 tm_session 会话摘要，让 tokscale 覆盖的工具也出现在会话列表。
    ///
    /// tokscale 聚合采集器只产出事件、不产出 `CollectResult.sessions`，此前在 tokscale
    /// 权威模式下 tm_session 永远是空的（逐工具适配器已退场）。本函数在快照替换后重建：
    /// - `session_id` 归一为 `{tool_id}:{external}`（全局唯一，防跨工具 sessionId 碰撞，
    ///   符合 tm_session 契约注释「tool_id + external」）；
    /// - `external_session_id` 保留 tokscale 原始 sessionId——`session_event_rows` 按
    ///   external 兜底匹配，点开下钻仍能查到逐轮明细；
    /// - `title_redacted` 用「会话短码 · 时间」生成（tokscale 无项目概念；不读正文）；
    /// - `message_count` = SUM(usage_event.message_count)（tokscale 条目自带，与开源版
    ///   逐值对齐；旧数据/缺失时回退 COUNT(*)）；
    /// - `started_at` / `last_active_at` 优先取 `session_started_at` / `session_last_active_at`
    ///   （采集时从会话文件真实时间戳解析，替代 mtime 兜底），缺失时回退 occurred_at；
    /// - `total_tokens` = input + output + cache（保持 SessionSummary 内部恒等，与适配器一致）；
    /// - **快照替换语义**：清理 covered 工具下已无 usage_event 支撑的幽灵行（旧手写适配器
    ///   时代的 hash 会话），避免列表出现无法下钻的假会话；有 usage 支撑的行（如 imported）保留。
    ///
    /// 仅处理 `source_type='local_discovered'`（tokscale 快照）；imported 数据不在替换语义内。
    /// 返回投影出的会话摘要（调用方据此广播 `token-monitor:session-changed`）。
    pub fn project_from_usage_events(
        &self,
        conn: &Mutex<Connection>,
        tool_ids: &[&str],
    ) -> Result<Vec<SessionSummary>, String> {
        if tool_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (0..tool_ids.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        // 聚合查询在作用域内持锁；guard 在作用域结束时释放，避免与 upsert_session 内部锁死
        let rows = {
            let conn = conn.lock().map_err(|e| e.to_string())?;
            let sql = format!(
                "SELECT tool_id, session_id,
                        COALESCE(SUM(message_count), COUNT(*)),
                        MIN(COALESCE(session_started_at, occurred_at)),
                        MAX(COALESCE(session_last_active_at, occurred_at)),
                        COALESCE(SUM(COALESCE(input_tokens,0)),0),
                        COALESCE(SUM(COALESCE(output_tokens,0)),0),
                        COALESCE(SUM(COALESCE(cache_read_tokens,0)+COALESCE(cache_write_tokens,0)),0),
                        COALESCE(SUM(COALESCE(cost_amount,0)),0),
                        json_group_array(DISTINCT model_normalized)
                            FILTER (WHERE model_normalized IS NOT NULL AND model_normalized != '')
                 FROM usage_event
                 WHERE source_type='local_discovered'
                   AND session_id IS NOT NULL AND session_id != ''
                   AND tool_id IN ({placeholders})
                 GROUP BY tool_id, session_id
                 ORDER BY 5 DESC"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> =
                tool_ids.iter().map(|t| &*t as &dyn rusqlite::types::ToSql).collect();
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    let tool_id: String = row.get(0)?;
                    let external: String = row.get(1)?;
                    let model_set: Vec<String> =
                        serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default();
                    let first_ts: Option<String> = row.get(3)?;
                    let title = match first_ts
                        .as_deref()
                        .and_then(|ts| ts.get(..16))
                        .filter(|s| !s.is_empty())
                    {
                        Some(ts) => format!("{} · {}", short_session_label(&external), ts),
                        None => short_session_label(&external),
                    };
                    let last_active: Option<String> = row.get(4)?;
                    let is_active = last_active
                        .as_deref()
                        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
                        .map(|dt| {
                            chrono::Utc::now()
                                .signed_duration_since(dt.with_timezone(&chrono::Utc))
                                < chrono::Duration::minutes(30)
                        })
                        .unwrap_or(false);
                    let input: i64 = row.get(5)?;
                    let output: i64 = row.get(6)?;
                    let cache: i64 = row.get(7)?;
                    let cost: f64 = row.get(8)?;
                    Ok(SessionSummary {
                        session_id: format!("{tool_id}:{external}"),
                        tool_id,
                        external_session_id: Some(external),
                        project_id: None,
                        title_redacted: Some(title),
                        model_set,
                        started_at: first_ts,
                        last_active_at: last_active,
                        input_tokens: input,
                        output_tokens: output,
                        cache_tokens: cache,
                        total_tokens: input + output + cache,
                        message_count: row.get(2)?,
                        status: Some(if is_active { "active" } else { "idle" }.into()),
                        cost_amount: if cost > 0.0 { Some(cost) } else { None },
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            drop(stmt);
            rows
        };

        // 逐条 upsert（幂等；model_set 合并、token 覆盖）
        for session in &rows {
            self.upsert_session(conn, session)?;
        }

        // 幽灵行清理：covered 工具下、已无任何 usage_event 支撑的旧行（旧手写适配器
        // 时代的 hash 会话在快照替换后 usage 已删除，下钻必然为空）。无需额外的
        // 「不在本次投影」条件——本次投影的行恰好都有 usage 支撑，NOT EXISTS 天然保护：
        // 投影行经 external_session_id 匹配、imported 支撑行经 session/external 匹配。
        let del_sql = format!(
            "DELETE FROM tm_session
             WHERE tool_id IN ({})
               AND NOT EXISTS (
                   SELECT 1 FROM usage_event u
                   WHERE u.tool_id = tm_session.tool_id
                     AND (u.session_id = tm_session.session_id
                          OR u.session_id = tm_session.external_session_id)
               )",
            placeholders
        );
        let del_params: Vec<&dyn rusqlite::types::ToSql> =
            tool_ids.iter().map(|t| &*t as &dyn rusqlite::types::ToSql).collect();
        let _ = conn
            .lock()
            .map_err(|e| e.to_string())?
            .execute(&del_sql, del_params.as_slice())
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }
}

/// 会话 id 的展示短码（title_redacted 用）：取最后一个路径/冒号段，超长截尾。
fn short_session_label(id: &str) -> String {
    let seg = id.rsplit(['/', '\\', ':']).next().unwrap_or(id);
    if seg.len() > 14 {
        format!("…{}", &seg[seg.len() - 14..])
    } else {
        seg.to_string()
    }
}

pub struct ProjectRepo;

impl ProjectRepo {
    /// Upsert 项目归属（仅 hash 与 basename，不存完整路径）。
    pub fn upsert_project(
        &self,
        conn: &Mutex<Connection>,
        project_id: &str,
        display_name: Option<&str>,
        canonical_path_hash: &str,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO tm_project (project_id, canonical_path_hash, display_name)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(project_id) DO UPDATE SET
                display_name=COALESCE(excluded.display_name, tm_project.display_name)",
            rusqlite::params![project_id, canonical_path_hash, display_name],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 按项目聚合用量行（ProjectRow 数据源），仅统计本地 usage_event 中带 project_id 的事件。
    /// 对齐开源 Token Monitor 项目视图：项目含总 token、成本，以及按工具的堆叠拆分。
    pub fn project_rows(
        &self,
        conn: &Mutex<Connection>,
        _range: &str,
    ) -> Result<Vec<ProjectRow>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT u.project_id,
                        COALESCE(p.display_name, ''),
                        COALESCE(SUM(COALESCE(u.total_tokens,0)),0),
                        COUNT(DISTINCT u.session_id),
                        MAX(u.occurred_at),
                        COALESCE(SUM(COALESCE(u.cost_amount,0)),0)
                 FROM usage_event u
                 LEFT JOIN tm_project p ON p.project_id = u.project_id
                 WHERE u.project_id IS NOT NULL AND u.project_id != ''
                   AND u.tool_id IN (SELECT tool_id FROM tool_definition WHERE enabled=1)
                 GROUP BY u.project_id
                 ORDER BY 3 DESC LIMIT 20",
            )
            .map_err(|e| e.to_string())?;
        let mut raw_rows: Vec<(String, String, i64, i64, Option<String>, f64)> = stmt
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

        // 每个项目内按工具拆分（仅取总量前 6 个工具，其余并入「其他」）
        let mut stmt = conn
            .prepare(
                "SELECT u.project_id, u.tool_id,
                        COALESCE(SUM(COALESCE(u.total_tokens,0)),0)
                 FROM usage_event u
                 WHERE u.project_id IS NOT NULL AND u.project_id != ''
                   AND u.tool_id IN (SELECT tool_id FROM tool_definition WHERE enabled=1)
                 GROUP BY u.project_id, u.tool_id
                 ORDER BY u.project_id, 3 DESC",
            )
            .map_err(|e| e.to_string())?;
        let tool_rows: Vec<(String, String, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);

        use std::collections::HashMap;
        let mut by_project: HashMap<String, Vec<(String, i64)>> = HashMap::new();
        for (project_id, tool_id, tokens) in tool_rows {
            by_project
                .entry(project_id)
                .or_default()
                .push((tool_id, tokens));
        }

        Ok(raw_rows
            .drain(..)
            .map(
                |(
                    project_id,
                    display_name,
                    total_tokens,
                    session_count,
                    last_active_at,
                    cost_sum,
                )| {
                    let mut tools: Vec<crate::token_monitor::model::ProjectToolShare> = by_project
                        .get(&project_id)
                        .map(|rows| {
                            rows.iter()
                                .take(6)
                                .map(|(tool_id, tokens)| {
                                    crate::token_monitor::model::ProjectToolShare {
                                        tool_id: tool_id.clone(),
                                        display_name: crate::db::token_usage::display_name_for_tool(
                                            tool_id,
                                        ),
                                        total_tokens: *tokens,
                                        share_percent: if total_tokens > 0 {
                                            *tokens as f64 * 100.0 / total_tokens as f64
                                        } else {
                                            0.0
                                        },
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    // 总量前 6 名之后的工具合并为「其他」
                    let rest: i64 = by_project
                        .get(&project_id)
                        .map(|rows| rows.iter().skip(6).map(|(_, t)| *t).sum())
                        .unwrap_or(0);
                    if rest > 0 {
                        tools.push(crate::token_monitor::model::ProjectToolShare {
                            tool_id: "other".into(),
                            display_name: "其他工具".into(),
                            total_tokens: rest,
                            share_percent: rest as f64 * 100.0 / total_tokens as f64,
                        });
                    }
                    ProjectRow {
                        project_id,
                        display_name,
                        total_tokens,
                        session_count,
                        last_active_at,
                        cost_amount: if cost_sum > 0.0 { Some(cost_sum) } else { None },
                        tools,
                    }
                },
            )
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("open database");
        conn.execute_batch(include_str!("../../migrations/001_initial.sql"))
            .expect("migrate base");
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

    fn session(session_id: &str, tool_id: &str, tokens: i64) -> SessionSummary {
        SessionSummary {
            session_id: session_id.into(),
            tool_id: tool_id.into(),
            external_session_id: None,
            project_id: Some("proj-1".into()),
            title_redacted: Some("demo-2026-08-08".into()),
            model_set: vec!["claude-3.5-sonnet".into()],
            started_at: Some("2026-08-08T00:00:00Z".into()),
            last_active_at: Some("2026-08-08T01:00:00Z".into()),
            input_tokens: tokens,
            output_tokens: 0,
            cache_tokens: 0,
            total_tokens: tokens,
            message_count: 3,
            status: Some("active".into()),
            cost_amount: None,
        }
    }

    #[test]
    fn sessions_upsert_merges_model_set() {
        let db = test_db();
        {
            let conn = db.lock().expect("lock");
            conn.execute_batch(
                "INSERT INTO tool_definition (tool_id, display_name) VALUES ('claude_code', 'Claude Code');",
            )
            .expect("seed tool_definition");
        }
        let repo = SessionRepo;
        repo.upsert_session(&db, &session("s1", "claude_code", 10))
            .expect("upsert");
        let mut s2 = session("s1", "claude_code", 20);
        s2.model_set = vec!["gpt-4o".into()];
        s2.message_count = 5;
        repo.upsert_session(&db, &s2).expect("upsert again");

        let rows = repo.list_sessions(&db, None, None, None, None).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].message_count, 5);
        // model_set 合并：旧模型保留 + 新模型加入
        assert!(rows[0].model_set.contains(&"claude-3.5-sonnet".into()));
        assert!(rows[0].model_set.contains(&"gpt-4o".into()));
    }

    #[test]
    fn sessions_upsert_normalizes_model_aliases() {
        let db = test_db();
        {
            let conn = db.lock().expect("lock");
            conn.execute_batch(
                "INSERT INTO tool_definition (tool_id, display_name) VALUES ('freebuff', 'Freebuff');",
            )
            .expect("seed tool_definition");
        }
        let repo = SessionRepo;
        // Freebuff 前端名 + 旧历史名都是同一模型，应归一为 claude-opus-4-8 且不重复。
        let mut s = session("s1", "freebuff", 10);
        s.model_set = vec![
            "claude-fable-5".into(),
            "maas_cl_opus_4.8_20260528_cache".into(),
        ];
        repo.upsert_session(&db, &s).expect("upsert");
        let rows = repo.list_sessions(&db, None, None, None, None).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_set, vec!["claude-opus-4-8".to_string()]);
    }

    #[test]
    fn project_rows_aggregate_tokens_and_sessions() {
        let db = test_db();
        {
            let conn = db.lock().expect("lock");
            conn.execute_batch(
                "INSERT INTO tool_definition (tool_id, display_name) VALUES ('claude_code', 'Claude Code');
                 INSERT INTO tm_project (project_id, canonical_path_hash, display_name)
                     VALUES ('proj-1', 'abc123', 'demo');
                 INSERT INTO usage_event (source_type, tool_id, project_id, session_id, total_tokens, cost_amount, occurred_at, source_fingerprint, usage_accuracy)
                     VALUES ('local_discovered', 'claude_code', 'proj-1', 's1', 30, 0.5, '2026-08-08T00:00:00Z', 'fp-1', 'exact'),
                            ('local_discovered', 'claude_code', 'proj-1', 's2', 20, 0.25, '2026-08-08T00:00:01Z', 'fp-2', 'exact');",
            )
            .expect("seed");
        }
        let rows = ProjectRepo.project_rows(&db, "total").expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].total_tokens, 50);
        assert_eq!(rows[0].session_count, 2);
        assert_eq!(rows[0].display_name, "demo");
        // 成本合计
        assert_eq!(rows[0].cost_amount, Some(0.75));
        // 项目内按工具拆分：单工具 claude_code 占 100%
        assert_eq!(rows[0].tools.len(), 1);
        assert_eq!(rows[0].tools[0].tool_id, "claude_code");
        assert_eq!(rows[0].tools[0].total_tokens, 50);
        assert!((rows[0].tools[0].share_percent - 100.0).abs() < 0.001);
    }

    #[test]
    fn project_rows_split_by_tool_with_other_bucket() {
        let db = test_db();
        {
            let conn = db.lock().expect("lock");
            conn.execute_batch(
                "INSERT INTO tool_definition (tool_id, display_name) VALUES ('claude_code', 'Claude Code'), ('codex', 'Codex CLI');
                 INSERT INTO tm_project (project_id, canonical_path_hash, display_name)
                     VALUES ('proj-1', 'abc123', 'demo');
                 INSERT INTO usage_event (source_type, tool_id, project_id, session_id, total_tokens, occurred_at, source_fingerprint, usage_accuracy)
                     VALUES ('local_discovered', 'claude_code', 'proj-1', 's1', 70, '2026-08-08T00:00:00Z', 'fp-1', 'exact'),
                            ('local_discovered', 'codex', 'proj-1', 's2', 30, '2026-08-08T00:00:01Z', 'fp-2', 'exact');",
            )
            .expect("seed");
        }
        let rows = ProjectRepo.project_rows(&db, "total").expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].total_tokens, 100);
        // 工具拆分按量降序：claude_code 70% / codex 30%
        assert_eq!(rows[0].tools.len(), 2);
        assert_eq!(rows[0].tools[0].tool_id, "claude_code");
        assert_eq!(rows[0].tools[0].total_tokens, 70);
        assert!((rows[0].tools[0].share_percent - 70.0).abs() < 0.001);
        assert_eq!(rows[0].tools[1].tool_id, "codex");
        assert!((rows[0].tools[1].share_percent - 30.0).abs() < 0.001);
    }

    #[test]
    fn project_from_usage_events_builds_namespaced_summaries() {
        let db = test_db();
        {
            let conn = db.lock().expect("lock");
            conn.execute_batch(
                "INSERT INTO tool_definition (tool_id, display_name) VALUES
                    ('claude_code', 'Claude Code'),
                    ('workbuddy', 'WorkBuddy');
                 INSERT INTO usage_event (source_type, tool_id, session_id, model_normalized,
                    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                    message_count, total_tokens, cost_amount, occurred_at,
                    session_started_at, session_last_active_at,
                    source_fingerprint, usage_accuracy)
                 VALUES
                    ('local_discovered', 'claude_code', 'sess-claude', 'claude-opus-4-8',
                     100, 200, 50, 10, 5, 360, 0.02, '2026-08-08T00:00:00Z', NULL, NULL, 'fp-1', 'provider_reported'),
                    ('local_discovered', 'claude_code', 'sess-claude', 'claude-sonnet-4-5',
                     300, 100, 0, 0, 3, 400, 0.01, '2026-08-08T01:00:00Z', NULL, NULL, 'fp-2', 'provider_reported'),
                    ('local_discovered', 'workbuddy', 'sess-wb', 'mimo-v2.5-pro',
                     50, 30, 0, 0, NULL, 80, 0.005, '2026-08-08T02:00:00Z',
                     '2026-08-08T01:30:00Z', '2026-08-08T01:45:00Z', 'fp-3', 'provider_reported'),
                    ('local_discovered', 'claude_code', '', 'claude-opus-4-8',
                     10, 10, 0, 0, 1, 20, 0.0, '2026-08-08T03:00:00Z', NULL, NULL, 'fp-4', 'provider_reported'),
                    ('imported', 'claude_code', 'sess-imported', 'claude-opus-4-8',
                     5, 5, 0, 0, 1, 10, 0.0, '2026-08-08T04:00:00Z', NULL, NULL, 'fp-5', 'exact');",
            )
            .expect("seed");
        }
        let repo = SessionRepo;
        let rows = repo
            .project_from_usage_events(&db, &["claude_code", "workbuddy"])
            .expect("project");
        // 空 session_id 行与 imported 行不投影 → 只有 2 个会话
        assert_eq!(rows.len(), 2);

        let claude = rows.iter().find(|s| s.tool_id == "claude_code").expect("claude");
        // session_id 命名空间化 + external 保留原始 id（供下钻 external 兜底匹配）
        assert_eq!(claude.session_id, "claude_code:sess-claude");
        assert_eq!(claude.external_session_id.as_deref(), Some("sess-claude"));
        assert!(claude.model_set.contains(&"claude-opus-4-8".into()));
        assert!(claude.model_set.contains(&"claude-sonnet-4-5".into()));
        assert_eq!(claude.input_tokens, 400);
        assert_eq!(claude.output_tokens, 300);
        assert_eq!(claude.cache_tokens, 60); // cache_read + cache_write 合并
        assert_eq!(claude.total_tokens, 760); // in + out + cache（保持内部恒等）
        // message_count = SUM(message_count)：5 + 3 = 8（非行数 2）
        assert_eq!(claude.message_count, 8);
        assert_eq!(claude.started_at.as_deref(), Some("2026-08-08T00:00:00Z"));
        assert_eq!(claude.last_active_at.as_deref(), Some("2026-08-08T01:00:00Z"));
        assert!((claude.cost_amount.unwrap() - 0.03).abs() < 1e-9);
        assert!(claude.title_redacted.as_deref().unwrap_or("").contains("sess-claude"));

        // workbuddy 行 message_count 为 NULL → 回退 COUNT(*) = 1
        let wb = rows.iter().find(|s| s.tool_id == "workbuddy").expect("wb");
        assert_eq!(wb.session_id, "workbuddy:sess-wb");
        assert_eq!(wb.message_count, 1);
        assert_eq!(wb.total_tokens, 80);
        // 会话级真实时间戳优先于 occurred_at：started=01:30、last=01:45（文件解析值），
        // 而非 occurred_at 的 02:00（扫描时刻兜底）——列表排序/过滤精确
        assert_eq!(wb.started_at.as_deref(), Some("2026-08-08T01:30:00Z"));
        assert_eq!(wb.last_active_at.as_deref(), Some("2026-08-08T01:45:00Z"));

        // 已落库：列表可读且可下钻（external 兜底匹配到 usage_event）
        let listed = repo.list_sessions(&db, None, None, None, None).expect("list");
        assert_eq!(listed.len(), 2);
        let detail = crate::db::token_usage::UsageEventRepo
            .session_event_rows(&db, &claude.session_id)
            .expect("detail");
        assert_eq!(detail.len(), 2);
    }

    /// W9：day/7d/month 按 tokscale 周期会话成员过滤（对齐开源 --today/--week/--month），
    /// 成员表缺失时回退日期口径；total 不过滤。
    /// W12：tokscale 未覆盖的 companion 工具（freebuff）不在成员表，按日期口径合并，
    /// 否则「今日」会话列表会漏掉它们。
    #[test]
    fn list_sessions_filters_by_period_membership() {
        let db = test_db();
        {
            let conn = db.lock().expect("lock");
            conn.execute_batch(
                "INSERT INTO tool_definition (tool_id, display_name) VALUES
                    ('claude_code', 'Claude Code'), ('workbuddy', 'WorkBuddy'), ('freebuff', 'Freebuff');
                 INSERT INTO tm_session (session_id, tool_id, external_session_id, model_set_json,
                    started_at, last_active_at, total_tokens, message_count)
                 VALUES
                    ('claude_code:sess-a', 'claude_code', 'sess-a', '[]', datetime('now','-2 days'), datetime('now'), 100, 2),
                    ('claude_code:sess-b', 'claude_code', 'sess-b', '[]', datetime('now','-12 days'), datetime('now','-10 days'), 200, 3),
                    ('workbuddy:sess-c', 'workbuddy', 'sess-c', '[]', datetime('now','-2 days'), datetime('now','+1 hour'), 300, 4),
                    ('freebuff:sess-d', 'freebuff', 'sess-d', '[]', datetime('now','-2 days'), datetime('now','-1 hour'), 400, 5);",
            )
            .expect("seed");
        }
        let repo = SessionRepo;

        // 无成员表数据 → 回退日期口径：sess-a/sess-c/sess-d 落在今天，sess-b 旧
        let fallback = repo
            .list_sessions(&db, None, None, None, Some("day"))
            .expect("fallback day");
        let fb_ids: Vec<&str> = fallback.iter().map(|s| s.session_id.as_str()).collect();
        assert!(fb_ids.contains(&"claude_code:sess-a"));
        assert!(fb_ids.contains(&"workbuddy:sess-c"));
        assert!(fb_ids.contains(&"freebuff:sess-d"));
        assert!(!fb_ids.contains(&"claude_code:sess-b"));

        // 成员表：今日只含 sess-a 与 sess-c → covered 工具按成员过滤；
        // companion 工具 freebuff（sess-d）不在成员表，按日期口径仍应出现。
        {
            let conn = db.lock().expect("lock");
            conn.execute_batch(
                "INSERT INTO tm_period_session (period, session_id, tool_id, message_count) VALUES
                    ('day', 'sess-a', 'claude_code', 2),
                    ('day', 'sess-c', 'workbuddy', 4),
                    ('7d', 'sess-a', 'claude_code', 2),
                    ('7d', 'sess-b', 'claude_code', 3),
                    ('7d', 'sess-c', 'workbuddy', 4);",
            )
            .expect("seed members");
        }
        let day = repo.list_sessions(&db, None, None, None, Some("day")).expect("day");
        let day_ids: Vec<&str> = day.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(day_ids.len(), 3, "covered 成员 2 个 + companion freebuff 1 个");
        assert!(day_ids.contains(&"claude_code:sess-a"));
        assert!(day_ids.contains(&"workbuddy:sess-c"));
        assert!(day_ids.contains(&"freebuff:sess-d"));
        assert!(!day_ids.contains(&"claude_code:sess-b"));

        // 7d：covered 三个成员 + companion freebuff 都在
        let week = repo.list_sessions(&db, None, None, None, Some("7d")).expect("7d");
        assert_eq!(week.len(), 4);

        // total：不过滤（4 个）
        let total = repo.list_sessions(&db, None, None, None, Some("total")).expect("total");
        assert_eq!(total.len(), 4);

        // 按 TOKENS 总量降序：sess-d(400) > sess-c(300) > sess-b(200) > sess-a(100)
        let total_ids: Vec<&str> = total.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(
            total_ids,
            vec!["freebuff:sess-d", "workbuddy:sess-c", "claude_code:sess-b", "claude_code:sess-a"]
        );
    }

    #[test]
    fn project_from_usage_events_cleans_ghosts_keeps_supported_and_companion() {
        let db = test_db();
        {
            let conn = db.lock().expect("lock");
            conn.execute_batch(
                "INSERT INTO tool_definition (tool_id, display_name) VALUES
                    ('claude_code', 'Claude Code'),
                    ('atomcode', 'Atom Code');
                 INSERT INTO usage_event (source_type, tool_id, session_id, model_normalized,
                    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                    total_tokens, cost_amount, occurred_at, source_fingerprint, usage_accuracy)
                 VALUES
                    ('local_discovered', 'claude_code', 'sess-a', 'claude-opus-4-8',
                     100, 0, 0, 0, 100, 0.0, '2026-08-08T00:00:00Z', 'fp-a', 'provider_reported'),
                    ('imported', 'claude_code', 'imported-1', 'claude-opus-4-8',
                     10, 0, 0, 0, 10, 0.0, '2026-08-08T00:00:01Z', 'fp-i', 'exact');
                 -- 幽灵会话：旧手写适配器时代的 hash 行，usage 已被快照替换删除 → 应清理
                 INSERT INTO tm_session (session_id, tool_id, external_session_id, total_tokens, message_count)
                     VALUES ('abc123def', 'claude_code', 'gone-file', 999, 5);
                 -- imported 支撑的行：usage 还在 → 应保留
                 INSERT INTO tm_session (session_id, tool_id, external_session_id, total_tokens, message_count)
                     VALUES ('imp-hash', 'claude_code', 'imported-1', 10, 1);
                 -- companion 工具行：不在 covered 清单 → 投影不碰
                 INSERT INTO tm_session (session_id, tool_id, external_session_id, total_tokens, message_count)
                     VALUES ('atom-hash', 'atomcode', 'atom-file', 50, 2);",
            )
            .expect("seed");
        }
        let repo = SessionRepo;
        let rows = repo
            .project_from_usage_events(&db, &["claude_code"])
            .expect("project");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, "claude_code:sess-a");

        let listed = repo.list_sessions(&db, None, None, None, None).expect("list");
        let ids: Vec<&str> = listed.iter().map(|s| s.session_id.as_str()).collect();
        // 投影行 + imported 支撑行保留；幽灵 hash 行被清理；companion 行不受影响
        assert!(ids.contains(&"claude_code:sess-a"));
        assert!(ids.contains(&"imp-hash"));
        assert!(!ids.contains(&"abc123def"));
        assert!(ids.contains(&"atom-hash"));
    }
}
