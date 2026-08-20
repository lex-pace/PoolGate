//! Virtual client key repository.
//!
//! A client key is the formal authentication & routing identity for gateway
//! access. It binds to zero or more route pools (`agent_groups`) through the
//! `client_key_pools` join table.
//!
//! Security: the raw key (`pg_live_...`) is generated once and shown at
//! creation time only; the repository stores only its SHA-256 hash plus
//! display metadata (prefix / last-four).

use rusqlite::{Connection, Transaction};
use std::sync::Mutex;

/// Public shape of a stored client key. `key_hash` is never serialized to the
/// frontend — management commands build a `ClientKeyView` instead.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ClientKey {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub key_hash: String,
    pub key_last_four: String,
    pub enabled: Option<bool>,
    pub rpm_limit: Option<i64>,
    pub tpm_limit: Option<i64>,
    pub allowed_protocols: Option<String>,
    pub allowed_models: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: Option<String>,
    pub managed_pool_id: Option<String>,
    pub secret_ref: Option<String>,
    pub rotated_at: Option<String>,
}

/// Key metadata + bound pool ids, safe to return to the frontend.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ClientKeyView {
    pub id: String,
    pub name: String,
    /// Display prefix, e.g. `pg_live_`.
    pub key_prefix: String,
    /// Display-only suffix, e.g. `a8f3`.
    pub key_last_four: String,
    pub enabled: bool,
    pub rpm_limit: Option<i64>,
    pub tpm_limit: Option<i64>,
    pub allowed_protocols: Option<String>,
    pub allowed_models: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: Option<String>,
    pub managed_pool_id: Option<String>,
    pub rotated_at: Option<String>,
    /// Route pools this key is bound to (empty = default pool / full pool).
    pub pool_ids: Vec<String>,
}

/// Stable, user-facing prefix for generated virtual client keys.
pub const KEY_PREFIX: &str = "pg_live_";

/// Prefix used by the gateway access key feature; recognized but not managed
/// by this repository.
pub const GATEWAY_KEY_PREFIX: &str = "pg_admin_";

/// Constant-time-safe SHA-256 hashing of a raw key for storage/lookup.
pub fn hash_key(raw: &str) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(raw.as_bytes());
    hex::encode(hasher.finalize())
}

/// Generate a new random virtual client key (`pg_live_` + 24 hex chars).
pub fn generate_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{}{}", KEY_PREFIX, hex::encode(bytes))
}

pub struct ClientKeyRepo;

impl ClientKeyRepo {
    const COLS: &'static str = "id, name, key_prefix, key_hash, key_last_four, enabled, \
         rpm_limit, tpm_limit, allowed_protocols, allowed_models, expires_at, last_used_at, \
         created_at, managed_pool_id, secret_ref, rotated_at";

    fn row_to_key(row: &rusqlite::Row) -> Result<ClientKey, String> {
        Ok(ClientKey {
            id: row.get(0).map_err(|e| e.to_string())?,
            name: row.get(1).map_err(|e| e.to_string())?,
            key_prefix: row.get(2).map_err(|e| e.to_string())?,
            key_hash: row.get(3).map_err(|e| e.to_string())?,
            key_last_four: row.get(4).map_err(|e| e.to_string())?,
            enabled: row.get(5).map_err(|e| e.to_string())?,
            rpm_limit: row.get(6).map_err(|e| e.to_string())?,
            tpm_limit: row.get(7).map_err(|e| e.to_string())?,
            allowed_protocols: row.get(8).map_err(|e| e.to_string())?,
            allowed_models: row.get(9).map_err(|e| e.to_string())?,
            expires_at: row.get(10).map_err(|e| e.to_string())?,
            last_used_at: row.get(11).map_err(|e| e.to_string())?,
            created_at: row.get(12).map_err(|e| e.to_string())?,
            managed_pool_id: row.get(13).map_err(|e| e.to_string())?,
            secret_ref: row.get(14).map_err(|e| e.to_string())?,
            rotated_at: row.get(15).map_err(|e| e.to_string())?,
        })
    }

    /// List all client keys with their bound pool ids (no key_hash exposed).
    pub fn list_all(&self, conn: &Mutex<Connection>) -> Result<Vec<ClientKeyView>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM client_keys ORDER BY created_at DESC",
                Self::COLS
            ))
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let key = Self::row_to_key(row)?;
            out.push(Self::to_view(&conn, &key)?);
        }
        Ok(out)
    }

    fn to_view(conn: &Connection, key: &ClientKey) -> Result<ClientKeyView, String> {
        let pool_ids = Self::get_pool_ids_conn(conn, &key.id)?;
        Ok(ClientKeyView {
            id: key.id.clone(),
            name: key.name.clone(),
            key_prefix: key.key_prefix.clone(),
            key_last_four: key.key_last_four.clone(),
            enabled: key.enabled.unwrap_or(true),
            rpm_limit: key.rpm_limit,
            tpm_limit: key.tpm_limit,
            allowed_protocols: key.allowed_protocols.clone(),
            allowed_models: key.allowed_models.clone(),
            expires_at: key.expires_at.clone(),
            last_used_at: key.last_used_at.clone(),
            created_at: key.created_at.clone(),
            managed_pool_id: key.managed_pool_id.clone(),
            rotated_at: key.rotated_at.clone(),
            pool_ids,
        })
    }

    /// Get a single key by id (for management), with bound pools.
    pub fn get_by_id(
        &self,
        conn: &Mutex<Connection>,
        id: &str,
    ) -> Result<Option<ClientKeyView>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM client_keys WHERE id=?1",
                Self::COLS
            ))
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query(rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        match rows.next().map_err(|e| e.to_string())? {
            Some(row) => {
                let key = Self::row_to_key(row)?;
                Ok(Some(Self::to_view(&conn, &key)?))
            }
            None => Ok(None),
        }
    }

    pub fn get_managed_for_pool(
        &self,
        conn: &Mutex<Connection>,
        pool_id: &str,
    ) -> Result<Option<ClientKey>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM client_keys WHERE managed_pool_id=?1 LIMIT 1",
                Self::COLS
            ))
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query(rusqlite::params![pool_id])
            .map_err(|e| e.to_string())?;
        match rows.next().map_err(|e| e.to_string())? {
            Some(row) => Ok(Some(Self::row_to_key(row)?)),
            None => Ok(None),
        }
    }

    /// Get the full stored record (including `key_hash`) for internal use.
    /// Never serialize the result of this to the frontend.
    pub fn get_by_id_raw(
        &self,
        conn: &Mutex<Connection>,
        id: &str,
    ) -> Result<Option<ClientKey>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM client_keys WHERE id=?1",
                Self::COLS
            ))
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query(rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        match rows.next().map_err(|e| e.to_string())? {
            Some(row) => Ok(Some(Self::row_to_key(row)?)),
            None => Ok(None),
        }
    }

    /// Authenticate a raw presented key: returns the stored key if the hash
    /// matches and the key is enabled and not expired.
    pub fn authenticate(
        &self,
        conn: &Mutex<Connection>,
        raw: &str,
    ) -> Result<Option<ClientKey>, String> {
        let hash = hash_key(raw.trim());
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM client_keys WHERE key_hash=?1",
                Self::COLS
            ))
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query(rusqlite::params![hash])
            .map_err(|e| e.to_string())?;
        match rows.next().map_err(|e| e.to_string())? {
            Some(row) => {
                let key = Self::row_to_key(row)?;
                if !key.enabled.unwrap_or(true) {
                    return Ok(None);
                }
                if let Some(exp) = &key.expires_at {
                    if let Some(expired) = Self::is_expired(exp) {
                        if expired {
                            return Ok(None);
                        }
                    }
                }
                Ok(Some(key))
            }
            None => Ok(None),
        }
    }

    fn is_expired(expires_at: &str) -> Option<bool> {
        chrono::DateTime::parse_from_rfc3339(expires_at)
            .map(|t| t < chrono::Utc::now())
            .ok()
    }

    pub fn insert_tx(tx: &Transaction<'_>, key: &ClientKey) -> Result<(), String> {
        tx.execute(
            "INSERT INTO client_keys (id, name, key_prefix, key_hash, key_last_four, enabled, \
             rpm_limit, tpm_limit, allowed_protocols, allowed_models, expires_at, managed_pool_id, secret_ref) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                key.id,
                key.name,
                key.key_prefix,
                key.key_hash,
                key.key_last_four,
                key.enabled.unwrap_or(true),
                key.rpm_limit,
                key.tpm_limit,
                key.allowed_protocols,
                key.allowed_models,
                key.expires_at,
                key.managed_pool_id,
                key.secret_ref,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn bind_pool_tx(tx: &Transaction<'_>, key_id: &str, pool_id: &str) -> Result<(), String> {
        tx.execute(
            "INSERT INTO client_key_pools (client_key_id, pool_id) VALUES (?1, ?2)",
            rusqlite::params![key_id, pool_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Create a key. The raw key is passed in so the caller can show it once;
    /// only its hash is stored.
    pub fn create(&self, conn: &Mutex<Connection>, key: &ClientKey) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO client_keys (id, name, key_prefix, key_hash, key_last_four, enabled, \
             rpm_limit, tpm_limit, allowed_protocols, allowed_models, expires_at, managed_pool_id, secret_ref) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                key.id,
                key.name,
                key.key_prefix,
                key.key_hash,
                key.key_last_four,
                key.enabled.unwrap_or(true),
                key.rpm_limit,
                key.tpm_limit,
                key.allowed_protocols,
                key.allowed_models,
                key.expires_at,
                key.managed_pool_id,
                key.secret_ref,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Update mutable fields (name, enabled, limits, permission scope, expiry).
    pub fn update(&self, conn: &Mutex<Connection>, key: &ClientKey) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE client_keys SET name=?1, enabled=?2, rpm_limit=?3, tpm_limit=?4, \
             allowed_protocols=?5, allowed_models=?6, expires_at=?7 WHERE id=?8",
            rusqlite::params![
                key.name,
                key.enabled.unwrap_or(true),
                key.rpm_limit,
                key.tpm_limit,
                key.allowed_protocols,
                key.allowed_models,
                key.expires_at,
                key.id,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Delete a key (join rows cascade via foreign keys).
    pub fn delete(&self, conn: &Mutex<Connection>, id: &str) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM client_keys WHERE id=?1", rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ── Pool bindings ─────────────────────────────────────────────────────

    /// Replace the full pool binding set of a key (diff = unbind then bind).
    pub fn set_pools(
        &self,
        conn: &Mutex<Connection>,
        key_id: &str,
        pool_ids: &[String],
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM client_key_pools WHERE client_key_id=?1",
            rusqlite::params![key_id],
        )
        .map_err(|e| e.to_string())?;
        for pool_id in pool_ids {
            conn.execute(
                "INSERT OR IGNORE INTO client_key_pools (client_key_id, pool_id) VALUES (?1, ?2)",
                rusqlite::params![key_id, pool_id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Bound pool ids for a key.
    pub fn get_pool_ids(
        &self,
        conn: &Mutex<Connection>,
        key_id: &str,
    ) -> Result<Vec<String>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        Self::get_pool_ids_conn(&conn, key_id)
    }

    fn get_pool_ids_conn(conn: &Connection, key_id: &str) -> Result<Vec<String>, String> {
        let mut stmt = conn
            .prepare("SELECT pool_id FROM client_key_pools WHERE client_key_id=?1")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query(rusqlite::params![key_id])
            .map_err(|e| e.to_string())?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            ids.push(row.get::<_, String>(0).map_err(|e| e.to_string())?);
        }
        Ok(ids)
    }

    /// Record the last successful use of a key (audit/statistics hook).
    pub fn touch(&self, conn: &Mutex<Connection>, key_id: &str) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE client_keys SET last_used_at=CURRENT_TIMESTAMP WHERE id=?1",
            rusqlite::params![key_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("open");
        // 008 depends on agent_groups (001); load both so FK/PK tests pass.
        conn.execute_batch(include_str!("../../migrations/001_initial.sql"))
            .expect("migrate base");
        conn.execute_batch(include_str!("../../migrations/008_client_keys.sql"))
            .expect("migrate keys");
        conn.execute_batch(include_str!(
            "../../migrations/010_route_pool_management.sql"
        ))
        .expect("migrate pool management");
        Mutex::new(conn)
    }

    #[test]
    fn key_generation_shape_and_uniqueness() {
        let k1 = generate_key();
        let k2 = generate_key();
        assert!(k1.starts_with(KEY_PREFIX));
        assert_eq!(k1.len(), KEY_PREFIX.len() + 24);
        assert_ne!(k1, k2);
    }

    #[test]
    fn hash_is_stable_and_hex() {
        let h1 = hash_key("abc");
        let h2 = hash_key("abc");
        let h3 = hash_key("abd");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn create_authenticate_and_ban() {
        let conn = test_conn();
        let repo = ClientKeyRepo;
        let raw = generate_key();
        let key = ClientKey {
            id: "ck-1".into(),
            name: "cursor".into(),
            key_prefix: KEY_PREFIX.into(),
            key_hash: hash_key(&raw),
            key_last_four: raw[raw.len() - 4..].to_string(),
            enabled: Some(true),
            rpm_limit: None,
            tpm_limit: None,
            allowed_protocols: None,
            allowed_models: None,
            expires_at: None,
            last_used_at: None,
            created_at: None,
            managed_pool_id: None,
            secret_ref: None,
            rotated_at: None,
        };
        repo.create(&conn, &key).unwrap();

        // Authenticate with the raw key.
        let found = repo.authenticate(&conn, &raw).unwrap().unwrap();
        assert_eq!(found.id, "ck-1");

        // Wrong key must not authenticate.
        assert!(repo.authenticate(&conn, &generate_key()).unwrap().is_none());

        // Disabled key must be rejected.
        repo.update(
            &conn,
            &ClientKey {
                enabled: Some(false),
                ..key
            },
        )
        .unwrap();
        assert!(repo.authenticate(&conn, &raw).unwrap().is_none());
    }

    #[test]
    fn pool_binding_roundtrip() {
        let conn = test_conn();
        // FK enforcement requires real agent_groups rows for the bound pools.
        conn.lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO agent_groups (id, name, protocol) VALUES ('pool-a', 'A', 'openai'); \
                 INSERT INTO agent_groups (id, name, protocol) VALUES ('pool-b', 'B', 'openai');",
            )
            .unwrap();
        let repo = ClientKeyRepo;
        let raw = generate_key();
        repo.create(
            &conn,
            &ClientKey {
                id: "ck-2".into(),
                name: "claude-code".into(),
                key_prefix: KEY_PREFIX.into(),
                key_hash: hash_key(&raw),
                key_last_four: "abcd".into(),
                enabled: Some(true),
                rpm_limit: None,
                tpm_limit: None,
                allowed_protocols: None,
                allowed_models: None,
                expires_at: None,
                last_used_at: None,
                created_at: None,
                managed_pool_id: None,
                secret_ref: None,
                rotated_at: None,
            },
        )
        .unwrap();

        repo.set_pools(&conn, "ck-2", &["pool-a".into(), "pool-b".into()])
            .unwrap();
        let ids = repo.get_pool_ids(&conn, "ck-2").unwrap();
        assert_eq!(ids.len(), 2);

        // Replace with a subset.
        repo.set_pools(&conn, "ck-2", &["pool-b".into()]).unwrap();
        let ids = repo.get_pool_ids(&conn, "ck-2").unwrap();
        assert_eq!(ids, vec!["pool-b".to_string()]);
    }
}
