//! Accounts CRUD operations

use crate::services::keychain;
use rusqlite::{Connection, Row};
use std::sync::Mutex;

const ACCOUNT_COLUMNS: &str = "a.id, a.provider_id, a.name, a.api_key, a.models, a.quota_limit, a.quota_used, \
a.status, a.health_status, a.health_code, a.health_msg, a.health_latency, a.health_check_at, a.priority, a.tags, \
a.last_used_at, a.created_at, a.credential_type, a.credential_data, a.source_format, a.external_account_id, \
a.email, a.expires_at, a.metadata, a.credential_fingerprint, a.protocols, a.route_takeover, u.plan_type, u.quota_windows, \
u.last_refreshed_at, u.last_error, u.token_refreshed_at, a.secret_ref";

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Account {
    pub id: String,
    pub provider_id: Option<String>,
    pub name: Option<String>,
    #[serde(default, skip_serializing)]
    pub api_key: String,
    pub models: Option<String>,
    pub quota_limit: Option<f64>,
    pub quota_used: Option<f64>,
    pub status: Option<String>,
    pub health_status: Option<String>,
    pub health_code: Option<i64>,
    pub health_msg: Option<String>,
    pub health_latency: Option<i64>,
    pub health_check_at: Option<String>,
    pub priority: Option<i64>,
    pub tags: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: Option<String>,
    #[serde(default)]
    pub credential_type: Option<String>,
    #[serde(default, skip_serializing)]
    pub credential_data: Option<String>,
    #[serde(default)]
    pub source_format: Option<String>,
    #[serde(default)]
    pub external_account_id: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing)]
    pub metadata: Option<String>,
    #[serde(default, skip_serializing)]
    pub credential_fingerprint: Option<String>,
    #[serde(default)]
    pub protocols: Option<String>,
    #[serde(default)]
    pub route_takeover: Option<i64>,
    #[serde(default)]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub quota_windows: Option<String>,
    #[serde(default)]
    pub quota_refreshed_at: Option<String>,
    #[serde(default)]
    pub quota_error: Option<String>,
    #[serde(default)]
    pub token_refreshed_at: Option<String>,
    #[serde(default, skip_serializing)]
    pub secret_ref: Option<String>,
}

fn account_from_row(row: &Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        provider_id: row.get(1)?,
        name: row.get(2)?,
        api_key: row.get(3)?,
        models: row.get(4)?,
        quota_limit: row.get(5)?,
        quota_used: row.get(6)?,
        status: row.get(7)?,
        health_status: row.get(8)?,
        health_code: row.get(9)?,
        health_msg: row.get(10)?,
        health_latency: row.get(11)?,
        health_check_at: row.get(12)?,
        priority: row.get(13)?,
        tags: row.get(14)?,
        last_used_at: row.get(15)?,
        created_at: row.get(16)?,
        credential_type: row.get(17)?,
        credential_data: row.get(18)?,
        source_format: row.get(19)?,
        external_account_id: row.get(20)?,
        email: row.get(21)?,
        expires_at: row.get(22)?,
        metadata: row.get(23)?,
        credential_fingerprint: row.get(24)?,
        protocols: row.get(25)?,
        route_takeover: row.get(26)?,
        plan_type: row.get(27)?,
        quota_windows: row.get(28)?,
        quota_refreshed_at: row.get(29)?,
        quota_error: row.get(30)?,
        token_refreshed_at: row.get(31)?,
        secret_ref: row.get(32)?,
    })
}

pub struct AccountRepo;

impl AccountRepo {
    pub fn list_all(&self, conn: &Mutex<Connection>) -> Result<Vec<Account>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "SELECT {} FROM accounts a LEFT JOIN account_usage u ON u.account_id=a.id LEFT JOIN providers p ON p.id=a.provider_id ORDER BY CASE WHEN a.status IN ('exhausted', 'error', 'token_expired', 'disabled') OR a.health_status = 'error' OR p.enabled = 0 THEN 1 ELSE 0 END, COALESCE(p.created_at, a.created_at) DESC, p.rowid DESC, a.created_at DESC, a.rowid DESC",
            ACCOUNT_COLUMNS
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], account_from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn get_by_id(&self, conn: &Mutex<Connection>, id: &str) -> Result<Option<Account>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "SELECT {} FROM accounts a LEFT JOIN account_usage u ON u.account_id=a.id WHERE a.id=?1",
            ACCOUNT_COLUMNS
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query(rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        rows.next()
            .map_err(|e| e.to_string())?
            .map(account_from_row)
            .transpose()
            .map_err(|e| e.to_string())
    }

    pub fn find_by_fingerprint(
        &self,
        conn: &Mutex<Connection>,
        fingerprint: &str,
    ) -> Result<Option<Account>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "SELECT {} FROM accounts a LEFT JOIN account_usage u ON u.account_id=a.id WHERE a.credential_fingerprint=?1 LIMIT 1",
            ACCOUNT_COLUMNS
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query(rusqlite::params![fingerprint])
            .map_err(|e| e.to_string())?;
        rows.next()
            .map_err(|e| e.to_string())?
            .map(account_from_row)
            .transpose()
            .map_err(|e| e.to_string())
    }

    pub fn create(&self, conn: &Mutex<Connection>, account: &Account) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        insert_account(&conn, account)
    }

    pub fn update(&self, conn: &Mutex<Connection>, account: &Account) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let existing_ref: Option<String> = conn
            .query_row(
                "SELECT secret_ref FROM accounts WHERE id=?1",
                rusqlite::params![account.id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let incoming = credential_bytes(account)?;
        let secret_ref = existing_ref
            .or_else(|| account.secret_ref.clone())
            .or_else(|| {
                incoming
                    .as_ref()
                    .map(|_| keychain::account_secret_ref(&account.id))
            });
        let previous = match (incoming.as_ref(), secret_ref.as_deref()) {
            (Some(_), Some(secret_ref)) => keychain::get_optional_secret(secret_ref)?,
            _ => None,
        };
        if let (Some(secret), Some(secret_ref)) = (incoming.as_deref(), secret_ref.as_deref()) {
            keychain::store_verified(secret_ref, secret)?;
        }

        let result = conn.execute(
            "UPDATE accounts SET provider_id=?1, name=?2, \
             api_key=CASE WHEN ?19 IS NULL THEN api_key ELSE '' END, models=?3, quota_limit=?4, \
             quota_used=?5, status=?6, health_status=?7, priority=?8, tags=?9, credential_type=?10, \
             credential_data=CASE WHEN ?19 IS NULL THEN credential_data ELSE NULL END, \
             source_format=?11, external_account_id=?12, email=?13, expires_at=?14, \
             metadata=COALESCE(?15, metadata), credential_fingerprint=COALESCE(?16, credential_fingerprint), \
             protocols=?17, route_takeover=?18, secret_ref=COALESCE(?19, secret_ref) WHERE id=?20",
            rusqlite::params![
                account.provider_id, account.name, account.models, account.quota_limit,
                account.quota_used, account.status, account.health_status, account.priority,
                account.tags, account.credential_type, account.source_format,
                account.external_account_id, account.email, account.expires_at, account.metadata,
                account.credential_fingerprint, account.protocols, account.route_takeover,
                secret_ref, account.id
            ],
        );
        match result {
            Ok(0) => {
                if let Some(secret_ref) = secret_ref.as_deref() {
                    if incoming.is_some() {
                        keychain::restore_secret(secret_ref, previous.as_deref());
                    }
                }
                Err("Account does not exist".into())
            }
            Ok(_) => Ok(()),
            Err(error) => {
                if let Some(secret_ref) = secret_ref.as_deref() {
                    if incoming.is_some() {
                        keychain::restore_secret(secret_ref, previous.as_deref());
                    }
                }
                Err(error.to_string())
            }
        }
    }

    pub fn delete(&self, conn: &Mutex<Connection>, id: &str) -> Result<(), String> {
        let mut conn = conn.lock().map_err(|e| e.to_string())?;
        let secret_ref: Option<String> = conn
            .query_row(
                "SELECT secret_ref FROM accounts WHERE id=?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let previous = match secret_ref.as_deref() {
            Some(secret_ref) => keychain::get_optional_secret(secret_ref)?,
            None => None,
        };
        if let (Some(secret_ref), Some(_)) = (secret_ref.as_deref(), previous.as_ref()) {
            keychain::delete_secret(secret_ref)?;
        }

        let result = (|| -> Result<(), String> {
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            tx.execute(
                "DELETE FROM group_accounts WHERE account_id=?1",
                rusqlite::params![id],
            )
            .map_err(|e| e.to_string())?;
            let deleted = tx
                .execute("DELETE FROM accounts WHERE id=?1", rusqlite::params![id])
                .map_err(|e| e.to_string())?;
            if deleted == 0 {
                return Err("账号不存在".into());
            }
            tx.commit().map_err(|e| e.to_string())
        })();

        if let Err(error) = result {
            if let Some(secret_ref) = secret_ref.as_deref() {
                keychain::restore_secret(secret_ref, previous.as_deref());
            }
            return Err(error);
        }
        Ok(())
    }

    pub fn batch_update_status(
        &self,
        conn: &Mutex<Connection>,
        ids: &[String],
        status: &str,
    ) -> Result<(), String> {
        let mut conn = conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for id in ids {
            tx.execute(
                "UPDATE accounts SET status=?1 WHERE id=?2",
                rusqlite::params![status, id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn update_health(
        &self,
        conn: &Mutex<Connection>,
        id: &str,
        health_status: &str,
        health_code: u16,
        health_msg: &str,
        latency_ms: i64,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE accounts SET health_status=?1, health_code=?2, health_msg=?3, \
             health_latency=?4, health_check_at=CURRENT_TIMESTAMP WHERE id=?5",
            rusqlite::params![health_status, health_code, health_msg, latency_ms, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn mark_used(&self, conn: &Mutex<Connection>, id: &str) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE accounts SET last_used_at=CURRENT_TIMESTAMP WHERE id=?1",
            rusqlite::params![id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update_models(
        &self,
        conn: &Mutex<Connection>,
        id: &str,
        models: &[String],
    ) -> Result<(), String> {
        let models = serde_json::to_string(models).map_err(|e| e.to_string())?;
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let updated = conn
            .execute(
                "UPDATE accounts SET models=?1 WHERE id=?2",
                rusqlite::params![models, id],
            )
            .map_err(|e| e.to_string())?;
        if updated == 0 {
            return Err("账号不存在".into());
        }
        Ok(())
    }

    pub fn update_status(
        &self,
        conn: &Mutex<Connection>,
        id: &str,
        status: &str,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE accounts SET status=?1 WHERE id=?2",
            rusqlite::params![status, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Recover a terminal credential state (`token_expired` / `error`) back to
    /// `active` once the credential has been proven usable (token refresh,
    /// health check, or a real upstream success). Manual states
    /// (`disabled` / `exhausted`) are respected and never overwritten.
    pub fn recover_status(
        &self,
        conn: &Mutex<Connection>,
        id: &str,
        current_status: Option<&str>,
    ) -> Result<(), String> {
        if matches!(current_status, Some("token_expired") | Some("error")) {
            self.update_status(conn, id, "active")?;
        }
        Ok(())
    }

    pub fn update_usage(
        &self,
        conn: &Mutex<Connection>,
        account_id: &str,
        provider: &str,
        plan_type: Option<&str>,
        quota_windows: &str,
        last_error: Option<&str>,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO account_usage (account_id, provider, plan_type, quota_windows, last_refreshed_at, last_error) \
             VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP, ?5) \
             ON CONFLICT(account_id) DO UPDATE SET provider=excluded.provider, plan_type=excluded.plan_type, \
             quota_windows=excluded.quota_windows, last_refreshed_at=CURRENT_TIMESTAMP, last_error=excluded.last_error",
            rusqlite::params![account_id, provider, plan_type, quota_windows, last_error],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update_usage_error(
        &self,
        conn: &Mutex<Connection>,
        account_id: &str,
        provider: &str,
        error: &str,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO account_usage (account_id, provider, quota_windows, last_refreshed_at, last_error) \
             VALUES (?1, ?2, '[]', CURRENT_TIMESTAMP, ?3) \
             ON CONFLICT(account_id) DO UPDATE SET provider=excluded.provider, \
             last_refreshed_at=CURRENT_TIMESTAMP, last_error=excluded.last_error",
            rusqlite::params![account_id, provider, error],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn mark_token_refreshed(
        &self,
        conn: &Mutex<Connection>,
        account_id: &str,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO account_usage (account_id, provider, quota_windows, token_refreshed_at) \
             VALUES (?1, 'oauth', '[]', CURRENT_TIMESTAMP) \
             ON CONFLICT(account_id) DO UPDATE SET token_refreshed_at=CURRENT_TIMESTAMP",
            rusqlite::params![account_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub fn insert_account(conn: &Connection, account: &Account) -> Result<(), String> {
    let secret_ref = account
        .secret_ref
        .clone()
        .unwrap_or_else(|| keychain::account_secret_ref(&account.id));
    let secret =
        credential_bytes(account)?.ok_or_else(|| "Account credential is empty".to_string())?;
    let previous = keychain::get_optional_secret(&secret_ref)?;
    keychain::store_verified(&secret_ref, &secret)?;

    let result = conn.execute(
        "INSERT INTO accounts (id, provider_id, name, api_key, models, quota_limit, quota_used, \
         status, health_status, priority, tags, credential_type, credential_data, source_format, \
         external_account_id, email, expires_at, metadata, credential_fingerprint, protocols, route_takeover, secret_ref) \
         VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        rusqlite::params![
            account.id, account.provider_id, account.name, account.models,
            account.quota_limit, account.quota_used.unwrap_or(0.0), account.status,
            account.health_status, account.priority, account.tags, account.credential_type,
            account.source_format, account.external_account_id, account.email, account.expires_at,
            account.metadata, account.credential_fingerprint, account.protocols,
            account.route_takeover, secret_ref
        ],
    );
    match result {
        Ok(_) => Ok(()),
        Err(error) => {
            keychain::restore_secret(&secret_ref, previous.as_deref());
            Err(error.to_string())
        }
    }
}

fn credential_bytes(account: &Account) -> Result<Option<Vec<u8>>, String> {
    if let Some(data) = account
        .credential_data
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        serde_json::from_str::<serde_json::Value>(data)
            .map_err(|error| format!("Invalid credential payload: {}", error))?;
        return Ok(Some(data.as_bytes().to_vec()));
    }
    if !account.api_key.trim().is_empty() {
        return serde_json::to_vec(&serde_json::json!({ "api_key": account.api_key }))
            .map(Some)
            .map_err(|error| error.to_string());
    }
    Ok(None)
}

/// Move historical plaintext account credentials into the OS vault. Each row
/// is cleared only after a verified vault write, so failures leave the original
/// plaintext available for a later retry.
pub fn migrate_plaintext_credentials(conn: &Mutex<Connection>) -> Result<usize, String> {
    let conn = conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, api_key, credential_data, secret_ref FROM accounts \
             WHERE (api_key <> '' OR credential_data IS NOT NULL)",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    let mut migrated = 0;
    for (id, api_key, credential_data, current_ref) in rows {
        let secret_ref = current_ref.unwrap_or_else(|| keychain::account_secret_ref(&id));
        let account = Account {
            id: id.clone(),
            provider_id: None,
            name: None,
            api_key,
            models: None,
            quota_limit: None,
            quota_used: None,
            status: None,
            health_status: None,
            health_code: None,
            health_msg: None,
            health_latency: None,
            health_check_at: None,
            priority: None,
            tags: None,
            last_used_at: None,
            created_at: None,
            credential_type: None,
            credential_data,
            source_format: None,
            external_account_id: None,
            email: None,
            expires_at: None,
            metadata: None,
            credential_fingerprint: None,
            protocols: None,
            route_takeover: None,
            plan_type: None,
            quota_windows: None,
            quota_refreshed_at: None,
            quota_error: None,
            token_refreshed_at: None,
            secret_ref: Some(secret_ref.clone()),
        };
        let Some(secret) = credential_bytes(&account)? else {
            continue;
        };
        let previous = keychain::get_optional_secret(&secret_ref)?;
        keychain::store_verified(&secret_ref, &secret)?;
        if let Err(error) = conn.execute(
            "UPDATE accounts SET api_key='', credential_data=NULL, secret_ref=?1 WHERE id=?2",
            rusqlite::params![secret_ref, id],
        ) {
            keychain::restore_secret(&secret_ref, previous.as_deref());
            return Err(error.to_string());
        }
        migrated += 1;
    }
    Ok(migrated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_all_orders_by_provider_created_at_and_puts_unavailable_last() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE providers (
                    id TEXT PRIMARY KEY,
                    created_at DATETIME,
                    enabled BOOLEAN DEFAULT 1
                 );
                 CREATE TABLE accounts (
                    id TEXT PRIMARY KEY,
                    provider_id TEXT,
                    name TEXT,
                    api_key TEXT NOT NULL DEFAULT '',
                    models TEXT, quota_limit REAL, quota_used REAL,
                    status TEXT, health_status TEXT, health_code INTEGER,
                    health_msg TEXT, health_latency INTEGER, health_check_at DATETIME,
                    priority INTEGER, tags TEXT, last_used_at DATETIME,
                    created_at DATETIME, credential_type TEXT, credential_data TEXT,
                    source_format TEXT, external_account_id TEXT, email TEXT,
                    expires_at DATETIME, metadata TEXT, credential_fingerprint TEXT,
                    protocols TEXT, route_takeover INTEGER, secret_ref TEXT
                 );
                 CREATE TABLE account_usage (
                    account_id TEXT PRIMARY KEY, plan_type TEXT, quota_windows TEXT,
                    last_refreshed_at DATETIME, last_error TEXT, token_refreshed_at DATETIME
                 );
                 INSERT INTO providers VALUES
                    ('old-provider', '2026-01-01 00:00:00', 1),
                    ('new-provider', '2026-01-02 00:00:00', 1),
                    ('disabled-provider', '2026-12-31 00:00:00', 0);
                 INSERT INTO accounts (id, provider_id, name, created_at, status, health_status)
                    VALUES ('old-account', 'old-provider', 'Old', '2026-01-03 00:00:00', 'active', 'healthy');
                 INSERT INTO accounts (id, provider_id, name, created_at, status, health_status)
                    VALUES ('new-account', 'new-provider', 'New', '2026-01-01 00:00:00', 'active', 'healthy');
                 INSERT INTO accounts (id, provider_id, name, created_at, status, health_status)
                    VALUES ('disabled-account', 'disabled-provider', 'Disabled', '2026-12-31 00:00:00', 'active', 'healthy');",
            )
            .unwrap();

        let ids: Vec<String> = AccountRepo
            .list_all(&Mutex::new(connection))
            .unwrap()
            .into_iter()
            .map(|account| account.id)
            .collect();
        assert_eq!(ids, vec!["new-account", "old-account", "disabled-account"]);
    }

    #[test]
    fn plaintext_migration_clears_sqlite_and_preserves_readable_secret() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE accounts (
                    id TEXT PRIMARY KEY,
                    api_key TEXT NOT NULL,
                    credential_data TEXT,
                    secret_ref TEXT
                );
                INSERT INTO accounts (id, api_key, credential_data)
                VALUES ('acct-migrate', 'legacy-secret', NULL);",
            )
            .unwrap();
        let connection = Mutex::new(connection);

        assert_eq!(migrate_plaintext_credentials(&connection).unwrap(), 1);
        let conn = connection.lock().unwrap();
        let (api_key, credential_data, secret_ref): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT api_key, credential_data, secret_ref FROM accounts WHERE id='acct-migrate'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert!(api_key.is_empty());
        assert!(credential_data.is_none());
        let secret_ref = secret_ref.unwrap();
        drop(conn);

        let payload: serde_json::Value =
            serde_json::from_slice(&keychain::get_secret(&secret_ref).unwrap()).unwrap();
        assert_eq!(payload["api_key"], "legacy-secret");
    }

    #[test]
    fn recover_status_restores_terminal_states_only() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE accounts (
                    id TEXT PRIMARY KEY,
                    api_key TEXT NOT NULL DEFAULT '',
                    status TEXT
                );
                INSERT INTO accounts (id, status) VALUES ('a', 'token_expired');
                INSERT INTO accounts (id, status) VALUES ('b', 'error');
                INSERT INTO accounts (id, status) VALUES ('c', 'disabled');
                INSERT INTO accounts (id, status) VALUES ('d', 'active');",
            )
            .unwrap();
        let connection = Mutex::new(connection);
        let repo = AccountRepo;

        repo.recover_status(&connection, "a", Some("token_expired"))
            .unwrap();
        repo.recover_status(&connection, "b", Some("error"))
            .unwrap();
        repo.recover_status(&connection, "c", Some("disabled"))
            .unwrap();
        repo.recover_status(&connection, "d", Some("active"))
            .unwrap();

        let conn = connection.lock().unwrap();
        let statuses: Vec<String> = conn
            .prepare("SELECT status FROM accounts ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(statuses, vec!["active", "active", "disabled", "active"]);
    }

    #[test]
    fn delete_removes_group_membership_before_account() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;
                 CREATE TABLE accounts (
                    id TEXT PRIMARY KEY,
                    secret_ref TEXT
                 );
                 CREATE TABLE agent_groups (
                    id TEXT PRIMARY KEY
                 );
                 CREATE TABLE group_accounts (
                    group_id TEXT REFERENCES agent_groups(id),
                    account_id TEXT REFERENCES accounts(id),
                    PRIMARY KEY (group_id, account_id)
                 );
                 INSERT INTO accounts (id, secret_ref) VALUES ('acct-delete', NULL);
                 INSERT INTO agent_groups (id) VALUES ('group-delete');
                 INSERT INTO group_accounts (group_id, account_id)
                 VALUES ('group-delete', 'acct-delete');",
            )
            .unwrap();
        let connection = Mutex::new(connection);

        AccountRepo.delete(&connection, "acct-delete").unwrap();

        let conn = connection.lock().unwrap();
        let account_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE id='acct-delete'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let membership_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM group_accounts WHERE account_id='acct-delete'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(account_count, 0);
        assert_eq!(membership_count, 0);
    }

    #[test]
    fn metadata_update_preserves_unmigrated_plaintext_credential() {
        let connection = Connection::open_in_memory().unwrap();
        for migration in [
            include_str!("../../migrations/001_initial.sql"),
            include_str!("../../migrations/002_account_credentials.sql"),
            include_str!("../../migrations/003_account_usage.sql"),
            include_str!("../../migrations/004_protocol_multi.sql"),
            include_str!("../../migrations/005_protocol_canonical.sql"),
            include_str!("../../migrations/006_secure_credentials.sql"),
        ] {
            connection.execute_batch(migration).unwrap();
        }
        connection
            .execute(
                "INSERT INTO accounts (id, api_key, name, credential_type) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["acct-legacy", "legacy-secret", "before", "api_key"],
            )
            .unwrap();
        let connection = Mutex::new(connection);
        let account = AccountRepo
            .get_by_id(&connection, "acct-legacy")
            .unwrap()
            .unwrap();
        let mut updated = account;
        updated.name = Some("after".into());
        updated.api_key.clear();
        updated.credential_data = None;
        AccountRepo.update(&connection, &updated).unwrap();

        let conn = connection.lock().unwrap();
        let (name, api_key, secret_ref): (String, String, Option<String>) = conn
            .query_row(
                "SELECT name, api_key, secret_ref FROM accounts WHERE id='acct-legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(name, "after");
        assert_eq!(api_key, "legacy-secret");
        assert!(secret_ref.is_none());
    }
}
