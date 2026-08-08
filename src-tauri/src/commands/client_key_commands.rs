//! Management commands for virtual client keys.
//!
//! Responsibilities:
//!   * CRUD for client keys (the formal gateway authentication identity).
//!   * Binding / unbinding keys to route pools (`agent_groups`).
//!   * Returning the raw key exactly once, at creation time.
//!
//! Extension points: `rpm_limit` / `tpm_limit` (rate limiting), and
//! `allowed_protocols` / `allowed_models` (permission scope) columns are
//! already stored; later features read them from the auth context.

use crate::db::client_keys::{hash_key, ClientKey, ClientKeyView, KEY_PREFIX};
use crate::AppState;
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

/// Creation result — the raw key is returned exactly once and never again.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ClientKeyCreated {
    pub key: ClientKeyView,
    /// Plaintext virtual client key. Show it to the user once, then discard.
    pub raw_key: String,
}

/// Create a new virtual client key, optionally bound to route pools.
#[tauri::command]
pub fn create_client_key(
    state: State<'_, Arc<AppState>>,
    name: String,
    pool_ids: Option<Vec<String>>,
    enabled: Option<bool>,
) -> Result<ClientKeyCreated, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Key name must not be empty".to_string());
    }
    // Validate pool ids exist before binding.
    if let Some(ids) = &pool_ids {
        for id in ids {
            if state.db.groups.get_by_id(&state.db.conn, id)?.is_none() {
                return Err(format!("Route pool '{}' does not exist", id));
            }
        }
    }

    let raw = crate::db::client_keys::generate_key();
    let id = Uuid::new_v4().to_string();
    let key = ClientKey {
        id: id.clone(),
        name,
        key_prefix: KEY_PREFIX.to_string(),
        key_hash: hash_key(&raw),
        key_last_four: raw[raw.len() - 4..].to_string(),
        enabled: Some(enabled.unwrap_or(true)),
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
    state.db.client_keys.create(&state.db.conn, &key)?;
    let ids = pool_ids.unwrap_or_default();
    if !ids.is_empty() {
        state.db.client_keys.set_pools(&state.db.conn, &id, &ids)?;
    }

    let view = state
        .db
        .client_keys
        .get_by_id(&state.db.conn, &id)?
        .ok_or_else(|| "Key was created but could not be read back".to_string())?;
    Ok(ClientKeyCreated {
        key: view,
        raw_key: raw,
    })
}

/// List all virtual client keys (metadata only, never the raw key).
#[tauri::command]
pub fn list_client_keys(state: State<'_, Arc<AppState>>) -> Result<Vec<ClientKeyView>, String> {
    state.db.client_keys.list_all(&state.db.conn)
}

/// Update mutable fields of a client key (name / enabled / limits / scope).
#[tauri::command]
pub fn update_client_key(
    state: State<'_, Arc<AppState>>,
    id: String,
    name: Option<String>,
    enabled: Option<bool>,
    rpm_limit: Option<i64>,
    tpm_limit: Option<i64>,
    allowed_protocols: Option<String>,
    allowed_models: Option<String>,
    expires_at: Option<String>,
) -> Result<(), String> {
    let existing = state
        .db
        .client_keys
        .get_by_id(&state.db.conn, &id)?
        .ok_or_else(|| format!("Client key '{}' does not exist", id))?;
    // Keep the stored key hash untouched — we never persist or expose the raw
    // key, so identity is preserved via the hash column.
    let stored = state
        .db
        .client_keys
        .get_by_id_raw(&state.db.conn, &id)?
        .ok_or_else(|| format!("Client key '{}' does not exist", id))?;
    let key = ClientKey {
        id: existing.id.clone(),
        name: name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or(existing.name),
        key_prefix: existing.key_prefix,
        key_hash: stored.key_hash,
        key_last_four: existing.key_last_four,
        enabled: Some(enabled.unwrap_or(existing.enabled)),
        rpm_limit: rpm_limit.or(existing.rpm_limit),
        tpm_limit: tpm_limit.or(existing.tpm_limit),
        allowed_protocols: allowed_protocols.or(existing.allowed_protocols),
        allowed_models: allowed_models.or(existing.allowed_models),
        expires_at: expires_at.or(existing.expires_at),
        last_used_at: None,
        created_at: None,
        managed_pool_id: stored.managed_pool_id,
        secret_ref: stored.secret_ref,
        rotated_at: stored.rotated_at,
    };
    state.db.client_keys.update(&state.db.conn, &key)
}

/// Delete a virtual client key (revokes access immediately).
#[tauri::command]
pub fn delete_client_key(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    state.db.client_keys.delete(&state.db.conn, &id)
}

/// Replace the route pool bindings of a key (empty list = unbind all).
#[tauri::command]
pub fn set_client_key_pools(
    state: State<'_, Arc<AppState>>,
    client_key_id: String,
    pool_ids: Vec<String>,
) -> Result<(), String> {
    for id in &pool_ids {
        if state.db.groups.get_by_id(&state.db.conn, id)?.is_none() {
            return Err(format!("Route pool '{}' does not exist", id));
        }
    }
    state
        .db
        .client_keys
        .set_pools(&state.db.conn, &client_key_id, &pool_ids)
}

/// Get the route pools bound to a key.
#[tauri::command]
pub fn get_client_key_pools(
    state: State<'_, Arc<AppState>>,
    client_key_id: String,
) -> Result<Vec<String>, String> {
    state
        .db
        .client_keys
        .get_pool_ids(&state.db.conn, &client_key_id)
}
