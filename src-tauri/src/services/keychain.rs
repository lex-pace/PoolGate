use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub const SERVICE_NAME: &str = "com.poolgate.gateway";
pub const KEYCHAIN_REF_PREFIX: &str = "keychain:";
pub const GATEWAY_ACCESS_KEY_REF: &str = "gateway-access-key";

fn runtime_cache() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn credential_connection() -> &'static OnceLock<Mutex<Connection>> {
    static CONNECTION: OnceLock<Mutex<Connection>> = OnceLock::new();
    &CONNECTION
}

fn configured_vault_path() -> &'static OnceLock<PathBuf> {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    &PATH
}

pub fn initialize_with_db(path: impl Into<PathBuf>) -> Result<(), String> {
    let conn = Connection::open(path.into()).map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
    credential_connection()
        .set(Mutex::new(conn))
        .map_err(|_| "Credential database is already initialized".to_string())
}

pub fn initialize_vault(path: impl Into<PathBuf>) -> Result<(), String> {
    let path = path.into();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    configured_vault_path()
        .set(path.clone())
        .or_else(|_| {
            if configured_vault_path().get() == Some(&path) { Ok(()) } else { Err(()) }
        })
        .map_err(|_| "Credential vault path is already configured".to_string())?;
    migrate_legacy_vault()
}

fn connection() -> Result<&'static Mutex<Connection>, String> {
    credential_connection().get().ok_or_else(|| "Credential database is not initialized".into())
}

fn cache_secret(secret_ref: &str, secret: &[u8]) {
    if let Ok(mut cache) = runtime_cache().lock() {
        if let Some(mut old) = cache.insert(secret_ref.to_string(), secret.to_vec()) { old.fill(0); }
    }
}

pub fn clear_runtime_cache() {
    if let Ok(mut cache) = runtime_cache().lock() {
        for secret in cache.values_mut() { secret.fill(0); }
        cache.clear();
    }
}

fn db_get(secret_ref: &str) -> Result<Option<Vec<u8>>, String> {
    let conn = connection()?.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT credential FROM credential_store WHERE secret_ref=?1",
        rusqlite::params![secret_ref],
        |row| row.get(0),
    ).map(Some).or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other.to_string()),
    })
}

fn db_store(secret_ref: &str, secret: &[u8]) -> Result<(), String> {
    let conn = connection()?.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO credential_store(secret_ref, credential, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP) ON CONFLICT(secret_ref) DO UPDATE SET credential=excluded.credential, updated_at=CURRENT_TIMESTAMP",
        rusqlite::params![secret_ref, secret],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

fn db_delete(secret_ref: &str) -> Result<(), String> {
    let conn = connection()?.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM credential_store WHERE secret_ref=?1", rusqlite::params![secret_ref]).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyVault { version: u8, entries: HashMap<String, LegacyEntry> }
#[derive(Debug, Serialize, Deserialize)]
struct LegacyEntry { nonce: Vec<u8>, ciphertext: Vec<u8> }

fn migrate_legacy_vault() -> Result<(), String> {
    let Some(path) = configured_vault_path().get() else { return Ok(()); };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    // Old encrypted vaults cannot be safely imported without the old master-key
    // implementation. Preserve the file rather than deleting potentially useful data.
    let _: LegacyVault = serde_json::from_slice(&bytes).map_err(|e| format!("Credential vault is corrupted: {e}"))?;
    Ok(())
}

pub fn account_secret_ref(account_id: &str) -> String { format!("account:{account_id}") }
pub fn client_key_secret_ref(key_id: &str, version: &str) -> String { format!("client-key:{key_id}:{version}") }
pub fn encode_reference(secret_ref: &str) -> String { format!("{KEYCHAIN_REF_PREFIX}{secret_ref}") }
pub fn decode_reference(value: &str) -> Option<&str> { value.strip_prefix(KEYCHAIN_REF_PREFIX) }

pub fn store_verified(secret_ref: &str, secret: &[u8]) -> Result<(), String> {
    db_store(secret_ref, secret)?;
    let stored = db_get(secret_ref)?.ok_or_else(|| "Credential verification failed".to_string())?;
    if stored != secret {
        let _ = db_delete(secret_ref);
        return Err("Credential verification returned different data".into());
    }
    cache_secret(secret_ref, secret);
    Ok(())
}

pub fn get_secret(secret_ref: &str) -> Result<Vec<u8>, String> {
    get_optional_secret(secret_ref)?.ok_or_else(|| "No credential found".into())
}

pub fn get_optional_secret(secret_ref: &str) -> Result<Option<Vec<u8>>, String> {
    if let Some(secret) = runtime_cache().lock().map_err(|e| e.to_string())?.get(secret_ref).cloned() { return Ok(Some(secret)); }
    let secret = db_get(secret_ref)?;
    if let Some(value) = &secret { cache_secret(secret_ref, value); }
    Ok(secret)
}

pub fn delete_secret(secret_ref: &str) -> Result<(), String> {
    db_delete(secret_ref)?;
    if let Ok(mut cache) = runtime_cache().lock() {
        if let Some(mut secret) = cache.remove(secret_ref) { secret.fill(0); }
    }
    Ok(())
}

pub fn restore_secret(secret_ref: &str, previous: Option<&[u8]>) {
    match previous { Some(value) => { let _ = store_verified(secret_ref, value); }, None => { let _ = delete_secret(secret_ref); } }
}

pub fn preload_account_secrets(secret_refs: &[String]) -> Result<usize, String> {
    let mut loaded = 0;
    for secret_ref in secret_refs {
        if !runtime_cache().lock().map_err(|e| e.to_string())?.contains_key(secret_ref) {
            let _ = get_secret(secret_ref)?;
            loaded += 1;
        }
    }
    Ok(loaded)
}

#[cfg(test)]
static TEST_VAULT_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(test)]
pub fn test_vault_serial_guard() -> std::sync::MutexGuard<'static, ()> { TEST_VAULT_SERIAL.lock().unwrap() }
