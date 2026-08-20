//! 额度账号 / 额度窗口快照（W4 实现）。
//!
//! 凭证只进 Keychain，`quota_account.credential_ref` 仅存引用；删除账号时级联删除
//! `quota_window_snapshot` 并清 Keychain（M4 关键验收，Keychain 清理由命令层负责）。

use rusqlite::Connection;
use std::sync::Mutex;

use crate::token_monitor::model::QuotaAccountView;
use crate::token_monitor::quota::{
    QuotaConfidence, QuotaSource, QuotaUnit, QuotaWindowSnapshot, QuotaWindowType,
};

/// 从快照行构造 `QuotaWindowSnapshot`（列序与 list_snapshots/list_all_snapshots 一致）。
/// `label` 由窗口类型推导中文名（表无 label 列）。
fn snapshot_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<QuotaWindowSnapshot> {
    let window_key: String = row.get(0)?;
    let window_type: QuotaWindowType =
        serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(1)?))
            .unwrap_or(QuotaWindowType::Monthly);
    let unit: QuotaUnit = serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(2)?))
        .unwrap_or(QuotaUnit::Percent);
    let label = match window_type {
        QuotaWindowType::Rolling5h => "5 小时额度".to_string(),
        QuotaWindowType::Weekly => "周额度".to_string(),
        QuotaWindowType::Monthly => "月额度".to_string(),
        QuotaWindowType::Billing => "账单周期".to_string(),
        QuotaWindowType::Credits => "Credits".to_string(),
        QuotaWindowType::PrepaidBalance => "预付费余额".to_string(),
        QuotaWindowType::Requests => "请求额度".to_string(),
    };
    Ok(QuotaWindowSnapshot {
        window_key,
        window_type,
        unit,
        label,
        used_value: row.get(3)?,
        limit_value: row.get(4)?,
        remaining_value: row.get(5)?,
        remaining_percent: row.get(6)?,
        period_started_at: row.get(7)?,
        resets_at: row.get(8)?,
        source: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(9)?))
            .unwrap_or(QuotaSource::OfficialApi),
        confidence: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(10)?))
            .unwrap_or(QuotaConfidence::Reported),
        error_code: row.get(11)?,
        fetched_at: row.get(12)?,
        expires_at: row.get(13)?,
    })
}

pub struct QuotaAccountRepo;

impl QuotaAccountRepo {
    /// Upsert 额度账号（监控绑定）。
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_account(
        &self,
        conn: &Mutex<Connection>,
        account_id: &str,
        provider_id: &str,
        label: Option<&str>,
        identity_masked: Option<&str>,
        plan_name: Option<&str>,
        auth_method: &str,
        credential_ref: Option<&str>,
        linked_route_account_id: Option<&str>,
        enabled: bool,
        status: &str,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO quota_account (
                account_id, provider_id, label, identity_masked, plan_name, auth_method,
                credential_ref, linked_route_account_id, enabled, status
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(account_id) DO UPDATE SET
                label=COALESCE(excluded.label, quota_account.label),
                identity_masked=COALESCE(excluded.identity_masked, quota_account.identity_masked),
                plan_name=COALESCE(excluded.plan_name, quota_account.plan_name),
                auth_method=excluded.auth_method,
                credential_ref=COALESCE(excluded.credential_ref, quota_account.credential_ref),
                linked_route_account_id=COALESCE(excluded.linked_route_account_id, quota_account.linked_route_account_id),
                enabled=excluded.enabled, status=excluded.status,
                updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')",
            rusqlite::params![
                account_id,
                provider_id,
                label,
                identity_masked,
                plan_name,
                auth_method,
                credential_ref,
                linked_route_account_id,
                enabled as i64,
                status,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 更新账号状态（刷新后写回：auth_expired / rate_limited / active / error）。
    pub fn update_status(
        &self,
        conn: &Mutex<Connection>,
        account_id: &str,
        status: &str,
        last_success: bool,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let sql = if last_success {
            "UPDATE quota_account SET status=?1, last_success_at=strftime('%Y-%m-%dT%H:%M:%SZ','now'), updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE account_id=?2"
        } else {
            "UPDATE quota_account SET status=?1, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE account_id=?2"
        };
        conn.execute(sql, rusqlite::params![status, account_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 读取单个账号（含 credential_ref 等敏感引用，仅后端使用）。
    pub fn get_account(
        &self,
        conn: &Mutex<Connection>,
        account_id: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT account_id, provider_id, label, identity_masked, plan_name, auth_method,
                        credential_ref, linked_route_account_id, enabled, status, last_success_at
                 FROM quota_account WHERE account_id=?1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(rusqlite::params![account_id], |row| {
                Ok(serde_json::json!({
                    "account_id": row.get::<_, String>(0)?,
                    "provider_id": row.get::<_, String>(1)?,
                    "label": row.get::<_, Option<String>>(2)?,
                    "identity_masked": row.get::<_, Option<String>>(3)?,
                    "plan_name": row.get::<_, Option<String>>(4)?,
                    "auth_method": row.get::<_, String>(5)?,
                    "credential_ref": row.get::<_, Option<String>>(6)?,
                    "linked_route_account_id": row.get::<_, Option<String>>(7)?,
                    "enabled": row.get::<_, i64>(8)? != 0,
                    "status": row.get::<_, String>(9)?,
                    "last_success_at": row.get::<_, Option<String>>(10)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok(row)) => Ok(Some(row)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }

    /// 列出全部账号 + 各自窗口（QuotaAccountView）。
    pub fn list_accounts(&self, conn: &Mutex<Connection>) -> Result<Vec<QuotaAccountView>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT a.account_id, a.provider_id, a.label, a.identity_masked, a.plan_name,
                        a.status, a.enabled, a.last_success_at,
                        s.window_key, s.window_type, s.unit, s.used_value, s.limit_value,
                        s.remaining_value, s.remaining_percent, s.period_started_at, s.resets_at,
                        s.source, s.confidence, s.error_code, s.fetched_at, s.expires_at
                 FROM quota_account a
                 LEFT JOIN quota_window_snapshot s
                        ON s.account_id = a.account_id
                     AND s.window_key = (
                         SELECT window_key FROM quota_window_snapshot s2
                         WHERE s2.account_id = a.account_id
                         ORDER BY s2.fetched_at DESC, s2.id DESC LIMIT 1
                     )
                 ORDER BY a.created_at, s.window_key",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                let window = if let Ok(key) = row.get::<_, String>(8) {
                    Some(QuotaWindowSnapshot {
                        window_key: key,
                        window_type: serde_json::from_str(&format!(
                            "\"{}\"",
                            row.get::<_, String>(9)?
                        ))
                        .unwrap_or(QuotaWindowType::Monthly),
                        unit: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(10)?))
                            .unwrap_or(QuotaUnit::Percent),
                        label: row.get(8)?,
                        used_value: row.get(11)?,
                        limit_value: row.get(12)?,
                        remaining_value: row.get(13)?,
                        remaining_percent: row.get(14)?,
                        period_started_at: row.get(15)?,
                        resets_at: row.get(16)?,
                        source: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(17)?))
                            .unwrap_or(QuotaSource::OfficialApi),
                        confidence: serde_json::from_str(&format!(
                            "\"{}\"",
                            row.get::<_, String>(18)?
                        ))
                        .unwrap_or(QuotaConfidence::Reported),
                        error_code: row.get(19)?,
                        fetched_at: row.get(20)?,
                        expires_at: row.get(21)?,
                    })
                } else {
                    None
                };
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)? != 0,
                    row.get::<_, Option<String>>(7)?,
                    window,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        // 按账号聚合窗口（JOIN 只取每账号最新一窗；历史窗口由 list_snapshots 提供）
        let mut order: Vec<String> = Vec::new();
        let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut views: Vec<QuotaAccountView> = Vec::new();
        for (
            account_id,
            provider_id,
            label,
            identity,
            plan,
            status,
            enabled,
            last_success,
            window,
        ) in rows
        {
            let idx = *index.entry(account_id.clone()).or_insert_with(|| {
                order.push(account_id.clone());
                views.push(QuotaAccountView {
                    account_id: account_id.clone(),
                    provider_id,
                    provider_label: None,
                    label,
                    identity_masked: identity,
                    plan_name: plan,
                    status,
                    enabled,
                    last_success_at: last_success,
                    windows: Vec::new(),
                });
                views.len() - 1
            });
            if let Some(window) = window {
                views[idx].windows.push(window.into());
            }
        }
        Ok(views)
    }

    /// 删除账号（级联删快照；Keychain 清理由调用方负责）。
    pub fn delete_account(&self, conn: &Mutex<Connection>, account_id: &str) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM quota_account WHERE account_id=?1",
            rusqlite::params![account_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub struct QuotaWindowRepo;

impl QuotaWindowRepo {
    /// Upsert 快照（UNIQUE(account_id, window_key) 只保留最新）。
    pub fn upsert_snapshot(
        &self,
        conn: &Mutex<Connection>,
        account_id: &str,
        snapshot: &QuotaWindowSnapshot,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO quota_window_snapshot (
                account_id, window_key, window_type, unit, used_value, limit_value,
                remaining_value, remaining_percent, period_started_at, resets_at,
                source, confidence, error_code, fetched_at, expires_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
             ON CONFLICT(account_id, window_key) DO UPDATE SET
                window_type=excluded.window_type, unit=excluded.unit,
                used_value=excluded.used_value, limit_value=excluded.limit_value,
                remaining_value=excluded.remaining_value, remaining_percent=excluded.remaining_percent,
                period_started_at=excluded.period_started_at, resets_at=excluded.resets_at,
                source=excluded.source, confidence=excluded.confidence,
                error_code=excluded.error_code, fetched_at=excluded.fetched_at,
                expires_at=excluded.expires_at",
            rusqlite::params![
                account_id,
                snapshot.window_key,
                serde_json::to_string(&snapshot.window_type)
                    .map(|s| s.trim_matches('"').to_string())
                    .unwrap_or_default(),
                serde_json::to_string(&snapshot.unit)
                    .map(|s| s.trim_matches('"').to_string())
                    .unwrap_or_default(),
                snapshot.used_value,
                snapshot.limit_value,
                snapshot.remaining_value,
                snapshot.remaining_percent,
                snapshot.period_started_at,
                snapshot.resets_at,
                serde_json::to_string(&snapshot.source)
                    .map(|s| s.trim_matches('"').to_string())
                    .unwrap_or_default(),
                serde_json::to_string(&snapshot.confidence)
                    .map(|s| s.trim_matches('"').to_string())
                    .unwrap_or_default(),
                snapshot.error_code,
                snapshot.fetched_at,
                snapshot.expires_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 读取某账号全部窗口快照（按 fetched_at 倒序）。
    pub fn list_snapshots(
        &self,
        conn: &Mutex<Connection>,
        account_id: &str,
    ) -> Result<Vec<QuotaWindowSnapshot>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT window_key, window_type, unit, used_value, limit_value,
                        remaining_value, remaining_percent, period_started_at, resets_at,
                        source, confidence, error_code, fetched_at, expires_at
                 FROM quota_window_snapshot
                 WHERE account_id=?1 ORDER BY fetched_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![account_id], |row| snapshot_from_row(row))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// 全部账号的全部窗口快照（W5 托盘主指标/告警汇总用）。
    pub fn list_all_snapshots(
        &self,
        conn: &Mutex<Connection>,
    ) -> Result<Vec<QuotaWindowSnapshot>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT window_key, window_type, unit, used_value, limit_value,
                        remaining_value, remaining_percent, period_started_at, resets_at,
                        source, confidence, error_code, fetched_at, expires_at
                 FROM quota_window_snapshot ORDER BY fetched_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| snapshot_from_row(row))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// 删除某账号全部快照。
    pub fn delete_for_account(
        &self,
        conn: &Mutex<Connection>,
        account_id: &str,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM quota_window_snapshot WHERE account_id=?1",
            rusqlite::params![account_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
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
        Mutex::new(conn)
    }

    fn snapshot(window_key: &str, remaining: f64) -> QuotaWindowSnapshot {
        QuotaWindowSnapshot {
            window_key: window_key.into(),
            window_type: QuotaWindowType::Rolling5h,
            unit: QuotaUnit::Percent,
            label: window_key.into(),
            used_value: Some(40.0),
            limit_value: Some(100.0),
            remaining_value: Some(remaining),
            remaining_percent: Some(remaining),
            period_started_at: None,
            resets_at: None,
            source: QuotaSource::OfficialApi,
            confidence: QuotaConfidence::Reported,
            error_code: None,
            fetched_at: "2026-08-08T00:00:00Z".into(),
            expires_at: None,
        }
    }

    #[test]
    fn account_and_window_crud_round_trip() {
        let db = test_db();
        QuotaAccountRepo
            .upsert_account(
                &db,
                "tm_test_1",
                "deepseek",
                Some("我的 DeepSeek"),
                Some("ds****"),
                Some("Pro"),
                "api_key",
                Some("tm.quota.tm_test_1"),
                None,
                true,
                "active",
            )
            .expect("upsert account");
        QuotaWindowRepo
            .upsert_snapshot(&db, "tm_test_1", &snapshot("primary", 62.5))
            .expect("upsert window");
        QuotaWindowRepo
            .upsert_snapshot(&db, "tm_test_1", &snapshot("primary", 12.5))
            .expect("upsert window again");

        let views = QuotaAccountRepo.list_accounts(&db).expect("list");
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].identity_masked.as_deref(), Some("ds****"));
        assert_eq!(views[0].windows.len(), 1);
        assert!(
            (views[0].windows[0].remaining_percent.unwrap() - 12.5).abs() < 0.001,
            "latest upsert wins"
        );

        // 删除账号 → 快照级联删除
        QuotaAccountRepo
            .delete_account(&db, "tm_test_1")
            .expect("delete");
        assert!(QuotaAccountRepo
            .list_accounts(&db)
            .expect("list")
            .is_empty());
        let conn = db.lock().expect("lock");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM quota_window_snapshot", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, 0, "snapshots cascade-deleted");
    }
}
