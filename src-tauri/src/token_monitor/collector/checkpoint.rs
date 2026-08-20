//! checkpoint 持久化（W2）：复用现有 `settings` 表存 JSON（07 §8 决策，不新建迁移）。
//!
//! key = `tm.checkpoint.<source_id>`，value = `CollectorCheckpoint` 序列化。
//! 路径类字段仅存不可逆 hash（隐私红线）。

use rusqlite::Connection;
use std::sync::Mutex;

use crate::token_monitor::model::CollectorCheckpoint;

const KEY_PREFIX: &str = "tm.checkpoint.";

/// 保存 checkpoint（upsert）。
pub fn save(conn: &Mutex<Connection>, checkpoint: &CollectorCheckpoint) -> Result<(), String> {
    let json = serde_json::to_string(checkpoint).map_err(|e| e.to_string())?;
    crate::db::settings::SettingsRepo.set(
        conn,
        &format!("{KEY_PREFIX}{}", checkpoint.source_id),
        &json,
    )
}

/// 加载 checkpoint；无则返回 source_id 已填充的默认值（幂等恢复）。
pub fn load(conn: &Mutex<Connection>, source_id: &str) -> CollectorCheckpoint {
    crate::db::settings::SettingsRepo
        .get(conn, &format!("{KEY_PREFIX}{source_id}"))
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_else(|| CollectorCheckpoint {
            source_id: source_id.to_string(),
            ..Default::default()
        })
}

/// 删除 checkpoint（工具被禁用/移除时）。
pub fn remove(conn: &Mutex<Connection>, source_id: &str) -> Result<(), String> {
    crate::db::settings::SettingsRepo.delete(conn, &format!("{KEY_PREFIX}{source_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn checkpoint_round_trips_via_settings_table() {
        let conn = Connection::open_in_memory().expect("open database");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("settings table");
        let db = Mutex::new(conn);

        let cp = CollectorCheckpoint {
            source_id: "src-1".into(),
            byte_offset: Some(42),
            inode: Some(7),
            mtime_ms: Some(123456),
            last_record_id: Some("r-9".into()),
            content_fingerprint: None,
        };
        save(&db, &cp).expect("save");
        let loaded = load(&db, "src-1");
        assert_eq!(loaded.byte_offset, Some(42));
        assert_eq!(loaded.inode, Some(7));
        assert_eq!(loaded.last_record_id.as_deref(), Some("r-9"));

        // 缺失 → 默认
        let missing = load(&db, "src-2");
        assert_eq!(missing.byte_offset, None);
        assert_eq!(missing.source_id, "src-2");
    }
}
