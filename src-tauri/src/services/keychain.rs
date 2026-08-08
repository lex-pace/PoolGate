//! Operating-system credential vault integration.
//!
//! PoolGate stores only opaque `secret_ref` values in SQLite. A single random
//! master key is persisted in macOS Keychain, Windows Credential Manager, or
//! Linux Secret Service. Individual credentials are AEAD-encrypted in the app
//! data directory, so starting the proxy unlocks only one OS-vault item.

#[cfg(not(test))]
use base64::{engine::general_purpose::STANDARD, Engine as _};
use ring::{aead, rand as ring_rand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const SERVICE_NAME: &str = "com.poolgate.gateway";
pub const KEYCHAIN_REF_PREFIX: &str = "keychain:";
pub const GATEWAY_ACCESS_KEY_REF: &str = "gateway-access-key";
const MASTER_KEY_REF: &str = "poolgate-vault-master-key-v1";
const MASTER_KEY_CACHE_REF: &str = "__poolgate_vault_master_key";
const VAULT_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedEntry {
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedVault {
    version: u8,
    entries: HashMap<String, EncryptedEntry>,
}

impl Default for EncryptedVault {
    fn default() -> Self {
        Self {
            version: VAULT_VERSION,
            entries: HashMap::new(),
        }
    }
}

fn runtime_cache() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn vault_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn configured_vault_path() -> &'static OnceLock<PathBuf> {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    &PATH
}

/// Configure the encrypted credential-vault file in PoolGate's app-data
/// directory. This performs no OS-vault access and is safe during app setup.
pub fn initialize_vault(path: impl Into<PathBuf>) -> Result<(), String> {
    let path = path.into();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Cannot create encrypted credential vault directory: {}",
                error
            )
        })?;
    }
    match configured_vault_path().set(path.clone()) {
        Ok(()) => Ok(()),
        Err(_) if configured_vault_path().get() == Some(&path) => Ok(()),
        Err(_) => Err("Encrypted credential vault path is already configured".into()),
    }
}

fn vault_path() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        let path = std::env::temp_dir().join(format!(
            "poolgate-credential-vault-{}.json",
            std::process::id()
        ));
        let _ = configured_vault_path().set(path);
    }
    configured_vault_path()
        .get()
        .cloned()
        .ok_or_else(|| "Encrypted credential vault is not initialized".to_string())
}

fn cache_secret(secret_ref: &str, secret: &[u8]) {
    if let Ok(mut cache) = runtime_cache().lock() {
        if let Some(mut previous) = cache.insert(secret_ref.to_string(), secret.to_vec()) {
            previous.fill(0);
        }
    }
}

/// Clear decrypted credentials and the decrypted master key when the proxy
/// stops or the process exits.
pub fn clear_runtime_cache() {
    if let Ok(mut cache) = runtime_cache().lock() {
        for value in cache.values_mut() {
            value.fill(0);
        }
        cache.clear();
    }
}

/// A trait for secure credential storage.
pub trait Keychain {
    fn store(service: &str, account: &str, secret: &[u8]) -> Result<(), String>;
    fn get(service: &str, account: &str) -> Result<Vec<u8>, String>;
    fn delete(service: &str, account: &str) -> Result<(), String>;
}

/// Production implementation backed by the current operating system's secure
/// credential vault. Only the PoolGate master key is stored here for new data.
pub struct SystemKeychain;

#[cfg(not(test))]
impl SystemKeychain {
    fn entry(service: &str, account: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(service, account)
            .map_err(|error| format!("Cannot open OS credential vault entry: {}", error))
    }
}

#[cfg(not(test))]
impl Keychain for SystemKeychain {
    fn store(service: &str, account: &str, secret: &[u8]) -> Result<(), String> {
        let encoded = STANDARD.encode(secret);
        Self::entry(service, account)?
            .set_password(&encoded)
            .map_err(|error| format!("Cannot store credential in OS vault: {}", error))
    }

    fn get(service: &str, account: &str) -> Result<Vec<u8>, String> {
        let encoded = Self::entry(service, account)?
            .get_password()
            .map_err(|error| format!("Cannot read credential from OS vault: {}", error))?;
        STANDARD
            .decode(encoded)
            .map_err(|error| format!("Stored credential is corrupted: {}", error))
    }

    fn delete(service: &str, account: &str) -> Result<(), String> {
        Self::entry(service, account)?
            .delete_credential()
            .map_err(|error| format!("Cannot delete credential from OS vault: {}", error))
    }
}

#[cfg(test)]
fn test_vault() -> &'static Mutex<HashMap<(String, String), Vec<u8>>> {
    static VAULT: OnceLock<Mutex<HashMap<(String, String), Vec<u8>>>> = OnceLock::new();
    VAULT.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
impl Keychain for SystemKeychain {
    fn store(service: &str, account: &str, secret: &[u8]) -> Result<(), String> {
        test_vault()
            .lock()
            .map_err(|error| error.to_string())?
            .insert((service.to_string(), account.to_string()), secret.to_vec());
        Ok(())
    }

    fn get(service: &str, account: &str) -> Result<Vec<u8>, String> {
        test_vault()
            .lock()
            .map_err(|error| error.to_string())?
            .get(&(service.to_string(), account.to_string()))
            .cloned()
            .ok_or_else(|| "NoEntry".to_string())
    }

    fn delete(service: &str, account: &str) -> Result<(), String> {
        test_vault()
            .lock()
            .map_err(|error| error.to_string())?
            .remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
}

#[cfg(not(test))]
fn system_get_optional(account: &str) -> Result<Option<Vec<u8>>, String> {
    let entry = SystemKeychain::entry(SERVICE_NAME, account)?;
    let encoded = match entry.get_password() {
        Ok(encoded) => encoded,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(error) => return Err(format!("Cannot read credential from OS vault: {}", error)),
    };
    STANDARD
        .decode(encoded)
        .map(Some)
        .map_err(|error| format!("Stored credential is corrupted: {}", error))
}

#[cfg(test)]
fn system_get_optional(account: &str) -> Result<Option<Vec<u8>>, String> {
    match SystemKeychain::get(SERVICE_NAME, account) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error == "NoEntry" => Ok(None),
        Err(error) => Err(error),
    }
}

fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    use ring_rand::SecureRandom;
    let rng = ring_rand::SystemRandom::new();
    let mut bytes = [0u8; N];
    rng.fill(&mut bytes)
        .map_err(|_| "Cannot generate secure random bytes".to_string())?;
    Ok(bytes)
}

/// Unlock or create the single PoolGate master key. Repeated calls during one
/// proxy session are served from memory and never reopen the OS vault.
fn master_key() -> Result<Vec<u8>, String> {
    if let Some(key) = runtime_cache()
        .lock()
        .map_err(|error| error.to_string())?
        .get(MASTER_KEY_CACHE_REF)
        .cloned()
    {
        return Ok(key);
    }

    let key = match system_get_optional(MASTER_KEY_REF)? {
        Some(key) if key.len() == 32 => key,
        Some(_) => return Err("PoolGate credential-vault master key is corrupted".into()),
        None => {
            let key = random_bytes::<32>()?.to_vec();
            SystemKeychain::store(SERVICE_NAME, MASTER_KEY_REF, &key)?;
            let verified = SystemKeychain::get(SERVICE_NAME, MASTER_KEY_REF)?;
            if verified != key {
                let _ = SystemKeychain::delete(SERVICE_NAME, MASTER_KEY_REF);
                return Err("OS credential vault verification returned different data".into());
            }
            key
        }
    };
    cache_secret(MASTER_KEY_CACHE_REF, &key);
    Ok(key)
}

fn load_encrypted_vault(path: &Path) -> Result<EncryptedVault, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let vault: EncryptedVault = serde_json::from_slice(&bytes)
                .map_err(|error| format!("Encrypted credential vault is corrupted: {}", error))?;
            if vault.version != VAULT_VERSION {
                return Err(format!(
                    "Unsupported encrypted credential vault version: {}",
                    vault.version
                ));
            }
            Ok(vault)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(EncryptedVault::default()),
        Err(error) => Err(format!("Cannot read encrypted credential vault: {}", error)),
    }
}

fn persist_encrypted_vault(path: &Path, vault: &EncryptedVault) -> Result<(), String> {
    let bytes = serde_json::to_vec(vault)
        .map_err(|error| format!("Cannot encode encrypted credential vault: {}", error))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|error| format!("Cannot write encrypted credential vault: {}", error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Cannot protect encrypted credential vault: {}", error))?;
    }
    #[cfg(target_os = "windows")]
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|error| format!("Cannot replace encrypted credential vault: {}", error))?;
    }
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("Cannot replace encrypted credential vault: {}", error))
}

fn encrypt_secret(secret_ref: &str, secret: &[u8], key: &[u8]) -> Result<EncryptedEntry, String> {
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| "Cannot initialize credential encryption".to_string())?;
    let sealing_key = aead::LessSafeKey::new(unbound);
    let nonce = random_bytes::<12>()?;
    let mut ciphertext = secret.to_vec();
    sealing_key
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(secret_ref.as_bytes()),
            &mut ciphertext,
        )
        .map_err(|_| "Cannot encrypt credential".to_string())?;
    Ok(EncryptedEntry {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

fn decrypt_secret(secret_ref: &str, entry: &EncryptedEntry, key: &[u8]) -> Result<Vec<u8>, String> {
    let nonce: [u8; 12] = entry
        .nonce
        .as_slice()
        .try_into()
        .map_err(|_| "Encrypted credential nonce is corrupted".to_string())?;
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| "Cannot initialize credential decryption".to_string())?;
    let opening_key = aead::LessSafeKey::new(unbound);
    let mut ciphertext = entry.ciphertext.clone();
    let plaintext = opening_key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(secret_ref.as_bytes()),
            &mut ciphertext,
        )
        .map_err(|_| "Encrypted credential failed authentication".to_string())?;
    Ok(plaintext.to_vec())
}

fn encrypted_get_optional(secret_ref: &str) -> Result<Option<Vec<u8>>, String> {
    let path = vault_path()?;
    let entry = {
        let _guard = vault_lock().lock().map_err(|error| error.to_string())?;
        load_encrypted_vault(&path)?
            .entries
            .get(secret_ref)
            .cloned()
    };
    match entry {
        Some(entry) => decrypt_secret(secret_ref, &entry, &master_key()?).map(Some),
        None => Ok(None),
    }
}

fn encrypted_store(secret_ref: &str, secret: &[u8]) -> Result<(), String> {
    let path = vault_path()?;
    let entry = encrypt_secret(secret_ref, secret, &master_key()?)?;
    let _guard = vault_lock().lock().map_err(|error| error.to_string())?;
    let mut vault = load_encrypted_vault(&path)?;
    vault.entries.insert(secret_ref.to_string(), entry);
    persist_encrypted_vault(&path, &vault)
}

fn encrypted_delete(secret_ref: &str) -> Result<bool, String> {
    let path = vault_path()?;
    let _guard = vault_lock().lock().map_err(|error| error.to_string())?;
    let mut vault = load_encrypted_vault(&path)?;
    if vault.entries.remove(secret_ref).is_none() {
        return Ok(false);
    }
    persist_encrypted_vault(&path, &vault)?;
    Ok(true)
}

fn migrate_legacy_secret(secret_ref: &str, secret: &[u8]) -> Result<(), String> {
    encrypted_store(secret_ref, secret)?;
    if let Err(error) = SystemKeychain::delete(SERVICE_NAME, secret_ref) {
        tracing::warn!(
            "Credential migrated but legacy OS-vault item could not be deleted ref={}: {}",
            secret_ref,
            error
        );
    }
    Ok(())
}

pub fn account_secret_ref(account_id: &str) -> String {
    format!("account:{}", account_id)
}

pub fn client_key_secret_ref(key_id: &str, version: &str) -> String {
    format!("client-key:{}:{}", key_id, version)
}

pub fn encode_reference(secret_ref: &str) -> String {
    format!("{}{}", KEYCHAIN_REF_PREFIX, secret_ref)
}

pub fn decode_reference(value: &str) -> Option<&str> {
    value.strip_prefix(KEYCHAIN_REF_PREFIX)
}

/// Store and immediately decrypt a secret. Callers may clear plaintext only
/// after this function succeeds.
pub fn store_verified(secret_ref: &str, secret: &[u8]) -> Result<(), String> {
    encrypted_store(secret_ref, secret)?;
    match encrypted_get_optional(secret_ref)? {
        Some(stored) if stored == secret => {
            cache_secret(secret_ref, secret);
            Ok(())
        }
        Some(_) => {
            let _ = encrypted_delete(secret_ref);
            Err("Encrypted credential verification returned different data".into())
        }
        None => Err("Encrypted credential verification failed".into()),
    }
}

pub fn get_secret(secret_ref: &str) -> Result<Vec<u8>, String> {
    get_optional_secret(secret_ref)?.ok_or_else(|| "No credential found".to_string())
}

pub fn get_optional_secret(secret_ref: &str) -> Result<Option<Vec<u8>>, String> {
    if let Some(secret) = runtime_cache()
        .lock()
        .map_err(|error| error.to_string())?
        .get(secret_ref)
        .cloned()
    {
        return Ok(Some(secret));
    }
    if let Some(secret) = encrypted_get_optional(secret_ref)? {
        cache_secret(secret_ref, &secret);
        return Ok(Some(secret));
    }

    // Compatibility path for versions that stored one OS-vault item per
    // logical secret. A legacy item is migrated immediately after it is read.
    // macOS may ask once per old item during this one-time upgrade only.
    let Some(secret) = system_get_optional(secret_ref)? else {
        return Ok(None);
    };
    migrate_legacy_secret(secret_ref, &secret)?;
    cache_secret(secret_ref, &secret);
    Ok(Some(secret))
}

pub fn delete_secret(secret_ref: &str) -> Result<(), String> {
    let removed = encrypted_delete(secret_ref)?;
    if !removed && system_get_optional(secret_ref)?.is_some() {
        SystemKeychain::delete(SERVICE_NAME, secret_ref)?;
    }
    if let Ok(mut cache) = runtime_cache().lock() {
        if let Some(mut secret) = cache.remove(secret_ref) {
            secret.fill(0);
        }
    }
    Ok(())
}

/// Restore a previous vault value after a database operation failed.
pub fn restore_secret(secret_ref: &str, previous: Option<&[u8]>) {
    match previous {
        Some(value) => {
            if let Err(error) = store_verified(secret_ref, value) {
                tracing::error!(
                    "Failed to restore encrypted credential ref={}: {}",
                    secret_ref,
                    error
                );
            }
        }
        None => {
            let _ = delete_secret(secret_ref);
        }
    }
}

/// Load account credentials once when the user explicitly starts the proxy.
/// All new-format credentials require only the already-unlocked master key.
pub fn preload_account_secrets(secret_refs: &[String]) -> Result<usize, String> {
    let _ = master_key()?;
    let mut loaded = 0usize;
    for secret_ref in secret_refs {
        if runtime_cache()
            .lock()
            .map_err(|error| error.to_string())?
            .contains_key(secret_ref)
        {
            continue;
        }
        let secret = get_secret(secret_ref)?;
        cache_secret(secret_ref, &secret);
        loaded += 1;
    }
    Ok(loaded)
}

#[cfg(test)]
static TEST_VAULT_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Serialize test access to the process-global credential vault statics.
///
/// The vault file, its master key and the runtime cache are shared across the
/// whole test process. Other modules (e.g. import conflict resolution) write
/// credentials through `store_verified`; acquiring this guard prevents a
/// concurrent keychain test from clearing the master key mid-operation.
#[cfg(test)]
pub fn test_vault_serial_guard() -> std::sync::MutexGuard<'static, ()> {
    TEST_VAULT_SERIAL.lock().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset_test_state() {
        clear_runtime_cache();
        if let Ok(path) = vault_path() {
            let _ = std::fs::remove_file(path);
        }
        if let Ok(mut vault) = test_vault().lock() {
            vault.clear();
        }
    }

    #[test]
    fn references_are_opaque_and_round_trip() {
        let _guard = test_vault_serial_guard();
        let secret_ref = account_secret_ref("acct_123");
        assert_eq!(secret_ref, "account:acct_123");
        let encoded = encode_reference(&secret_ref);
        assert_eq!(decode_reference(&encoded), Some(secret_ref.as_str()));
        assert!(decode_reference("plaintext-secret").is_none());
    }

    #[test]
    fn multiple_secrets_share_one_os_vault_master_key() {
        let _guard = test_vault_serial_guard();
        reset_test_state();
        store_verified("account:first", b"first-secret").unwrap();
        store_verified("account:second", b"second-secret").unwrap();
        clear_runtime_cache();

        assert_eq!(get_secret("account:first").unwrap(), b"first-secret");
        assert_eq!(get_secret("account:second").unwrap(), b"second-secret");

        let vault = test_vault().lock().unwrap();
        assert_eq!(vault.len(), 1);
        assert!(vault.contains_key(&(SERVICE_NAME.into(), MASTER_KEY_REF.into())));
        assert!(!vault.contains_key(&(SERVICE_NAME.into(), "account:first".into())));
        assert!(!vault.contains_key(&(SERVICE_NAME.into(), "account:second".into())));
    }

    #[test]
    fn legacy_secret_is_migrated_to_encrypted_vault() {
        let _guard = test_vault_serial_guard();
        reset_test_state();
        SystemKeychain::store(SERVICE_NAME, "account:legacy", b"legacy-secret").unwrap();

        assert_eq!(get_secret("account:legacy").unwrap(), b"legacy-secret");
        clear_runtime_cache();
        assert_eq!(get_secret("account:legacy").unwrap(), b"legacy-secret");
        assert!(SystemKeychain::get(SERVICE_NAME, "account:legacy").is_err());
    }

    #[test]
    fn preloaded_secret_remains_available_without_vault_file_read() {
        let _guard = test_vault_serial_guard();
        reset_test_state();
        store_verified("account:cache-test", b"secret-value").unwrap();
        clear_runtime_cache();
        assert_eq!(
            preload_account_secrets(&["account:cache-test".into()]).unwrap(),
            1
        );
        std::fs::remove_file(vault_path().unwrap()).unwrap();
        assert_eq!(get_secret("account:cache-test").unwrap(), b"secret-value");
        clear_runtime_cache();
    }
}
