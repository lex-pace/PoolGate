//! Key/value settings operations (single `settings` table).

use rusqlite::Connection;
use std::sync::Mutex;

pub struct SettingsRepo;

impl SettingsRepo {
    /// Read a setting value by key. Returns `None` when the key is absent.
    pub fn get(&self, conn: &Mutex<Connection>, key: &str) -> Result<Option<String>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT value FROM settings WHERE key=?1")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query(rusqlite::params![key])
            .map_err(|e| e.to_string())?;
        match rows.next().map_err(|e| e.to_string())? {
            Some(row) => Ok(Some(row.get::<_, String>(0).map_err(|e| e.to_string())?)),
            None => Ok(None),
        }
    }

    /// Upsert a setting value.
    pub fn set(&self, conn: &Mutex<Connection>, key: &str, value: &str) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            rusqlite::params![key, value],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Delete a setting key (no-op when absent).
    pub fn delete(&self, conn: &Mutex<Connection>, key: &str) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM settings WHERE key=?1", rusqlite::params![key])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
