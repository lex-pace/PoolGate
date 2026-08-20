//! 通用历史修复（自愈）：清理「模型逻辑变更后重扫」产生的残留重复 + 回填缺模型行。
//!
//! 背景：`source_fingerprint` 含 `model_normalized`。当适配器的模型提取逻辑改进后
//! （如 Freebuff 线程继承、AtomCode env 正则放宽），用户重扫（扫描/刷新会清 checkpoint
//! 重采）时，同一条消息会生成「旧指纹（无模型）+ 新指纹（带模型）」两行——`INSERT OR
//! IGNORE` 按指纹去不掉，于是 TOKENS 双算、模型视图残留「未知模型」。
//!
//! `source_locator_hash`（文件+offset/seq 的不可逆 hash）对同一条消息是稳定的，
//! 因此可据此定位清理。各适配器（freebuff/atomcode…）负责从自己的数据源重建
//! `locator_hash → model` 映射，本模块做统一清理：
//!
//! 1. 清残留重复：同 hash 已有带模型的行 → 删 NULL 残留（保留更完整的一条）。
//! 2. 按映射回填剩余 NULL 行，并**重算 `source_fingerprint`**（与未来重扫重采一致，
//!    避免再次双算）。
//! 3. 有变更时重建按日 rollup（趋势图按模型拆分不再残留「未知模型」）。
//!
//! 幂等：只动 NULL 行、无待修时快速短路返回 0，可反复调用。

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use rusqlite::OptionalExtension;

use crate::token_monitor::dedup;
use crate::token_monitor::model::{NormalizedUsageEvent, SourceType, UsageAccuracy};

/// 该工具是否还有 `model_normalized` 为 NULL 的行（快速短路用，不解析数据源）。
pub(crate) fn has_null_model_rows(
    conn: &Mutex<rusqlite::Connection>,
    tool_id: &str,
) -> bool {
    let Ok(conn) = conn.lock() else {
        return false;
    };
    let has: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM usage_event WHERE tool_id=?1 \
             AND source_locator_hash IS NOT NULL \
             AND (model_normalized IS NULL OR model_normalized=''))",
            rusqlite::params![tool_id],
            |row| row.get(0),
        )
        .unwrap_or(false);
    drop(conn);
    has
}

/// 修复某工具的历史事件（幂等，无待修返回 0）。
///
/// `models`：`source_locator_hash → model_raw`（由适配器从数据源重建）。
pub(crate) fn repair_missing_models(
    conn: &Mutex<rusqlite::Connection>,
    tool_id: &str,
    models: &HashMap<String, String>,
) -> Result<usize, String> {
    let mut fixed = 0usize;
    // 1. 清残留重复：同 hash 已有带模型的行 → 删 NULL 残留（防双算 + 避免回填时
    //    指纹 UNIQUE 冲突）
    {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let removed = conn
            .execute(
                "DELETE FROM usage_event WHERE tool_id=?1 \
                 AND source_locator_hash IS NOT NULL \
                 AND (model_normalized IS NULL OR model_normalized='') \
                 AND EXISTS (SELECT 1 FROM usage_event o2 \
                             WHERE o2.tool_id=?1 \
                               AND o2.source_locator_hash=usage_event.source_locator_hash \
                               AND o2.model_normalized IS NOT NULL AND o2.model_normalized != '')",
                rusqlite::params![tool_id],
            )
            .map_err(|e| e.to_string())?;
        fixed += removed;
    }
    // 2. 剩余待回填：DB 里的 NULL 行 ∩ 数据源能提供模型的 hash
    let need: HashMap<String, String> = {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT source_locator_hash FROM usage_event \
                 WHERE tool_id=?1 AND source_locator_hash IS NOT NULL \
                   AND (model_normalized IS NULL OR model_normalized='')",
            )
            .map_err(|e| e.to_string())?;
        let hashes = stmt
            .query_map(rusqlite::params![tool_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<HashSet<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        hashes
            .into_iter()
            .filter_map(|h| models.get(&h).map(|m| (h, m.clone())))
            .collect()
    };
    if need.is_empty() {
        return Ok(fixed);
    }
    // 3. UPDATE 剩余 NULL 行（只补 NULL；同步重算指纹，保证之后重扫不双算）
    {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        for (hash, model) in &need {
            let row: Option<(
                i64,
                Option<String>,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                String,
            )> = conn
                .query_row(
                    "SELECT id, session_id, input_tokens, output_tokens, \
                            cache_read_tokens, cache_write_tokens, occurred_at \
                     FROM usage_event WHERE tool_id=?1 AND source_locator_hash=?2",
                    rusqlite::params![tool_id, hash],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                        ))
                    },
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let Some((id, session_id, input, output, cache_read, cache_write, occurred_at)) = row
            else {
                continue;
            };
            let normalized = crate::token_monitor::normalization::normalize_model(model);
            let fingerprint = dedup::fingerprint(&NormalizedUsageEvent {
                source_type: SourceType::LocalDiscovered,
                tool_id: tool_id.into(),
                device_id: "local".into(),
                model_raw: Some(model.clone()),
                model_normalized: Some(normalized.clone()),
                session_id,
                project_id: None,
                account_id: None,
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: cache_read,
                cache_write_tokens: cache_write,
                reasoning_tokens: None,
                message_count: None,
                session_started_at: None,
                session_last_active_at: None,
                total_tokens: None,
                cost_amount: None,
                cost_currency: None,
                usage_accuracy: UsageAccuracy::Exact,
                occurred_at,
                source_locator_hash: Some(hash.clone()),
            });
            let affected = conn
                .execute(
                    "UPDATE usage_event SET model_raw=?1, model_normalized=?2, source_fingerprint=?3 \
                     WHERE id=?4 AND (model_normalized IS NULL OR model_normalized='')",
                    rusqlite::params![model, normalized, fingerprint, id],
                )
                .map_err(|e| e.to_string())?;
            fixed += affected;
        }
    }
    // 4. 重建按日 rollup（删除/更新后保持一致；趋势图按模型拆分不再残留「未知模型」）
    if fixed > 0 {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tm_daily_rollup", [])
            .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO tm_daily_rollup (\
                day, tool_id, model_normalized, input_tokens, output_tokens, cache_tokens,\
                total_tokens, cost_amount, request_count\
             )\
             SELECT date(occurred_at,'localtime'),\
                    tool_id, COALESCE(model_normalized,''),\
                    COALESCE(input_tokens,0), COALESCE(output_tokens,0),\
                    COALESCE(cache_read_tokens,0)+COALESCE(cache_write_tokens,0),\
                    COALESCE(total_tokens,0), COALESCE(cost_amount,0), COUNT(*)\
             FROM usage_event GROUP BY 1,2,3",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(fixed)
}
