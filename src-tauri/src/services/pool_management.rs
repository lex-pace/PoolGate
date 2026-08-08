use crate::db::client_keys::{hash_key, ClientKey, ClientKeyView, KEY_PREFIX};
use crate::db::groups::AgentGroup;
use crate::services::keychain;
use crate::AppState;
use uuid::Uuid;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ManagedPoolKeyCreated {
    pub key: ClientKeyView,
    pub raw_key: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct RoutePoolCreated {
    pub group: AgentGroup,
    pub key: ClientKeyView,
    pub raw_key: String,
}

fn validate_group(group: &AgentGroup) -> Result<(), String> {
    if group.name.trim().is_empty() {
        return Err("POOL_NAME_REQUIRED: 路由池名称不能为空".into());
    }
    if !matches!(
        group.protocol.as_str(),
        "openai" | "anthropic" | "both" | "gemini"
    ) {
        return Err("POOL_PROTOCOL_INVALID: 不支持的路由池协议".into());
    }
    Ok(())
}

fn build_managed_key(group: &AgentGroup, raw: &str, key_id: &str, secret_ref: &str) -> ClientKey {
    ClientKey {
        id: key_id.to_string(),
        name: format!("{} · 专属 Key", group.name.trim()),
        key_prefix: KEY_PREFIX.to_string(),
        key_hash: hash_key(raw),
        key_last_four: raw[raw.len() - 4..].to_string(),
        enabled: Some(true),
        rpm_limit: None,
        tpm_limit: None,
        allowed_protocols: None,
        allowed_models: None,
        expires_at: None,
        last_used_at: None,
        created_at: None,
        managed_pool_id: Some(group.id.clone()),
        secret_ref: Some(secret_ref.to_string()),
        rotated_at: None,
    }
}

fn key_view(key: &ClientKey, pool_id: &str) -> ClientKeyView {
    ClientKeyView {
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
        pool_ids: vec![pool_id.to_string()],
    }
}

pub fn create_pool_with_key(
    state: &AppState,
    mut group: AgentGroup,
) -> Result<RoutePoolCreated, String> {
    group.name = group.name.trim().to_string();
    group.api_key = None;
    validate_group(&group)?;
    let raw_key = crate::db::client_keys::generate_key();
    let key_id = Uuid::new_v4().to_string();
    let version = Uuid::new_v4().simple().to_string();
    let secret_ref = keychain::client_key_secret_ref(&key_id, &version);
    keychain::store_verified(&secret_ref, raw_key.as_bytes())?;
    let key = build_managed_key(&group, &raw_key, &key_id, &secret_ref);

    let result = (|| {
        let mut conn = state.db.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        crate::db::groups::GroupRepo::insert_tx(&tx, &group)?;
        crate::db::client_keys::ClientKeyRepo::insert_tx(&tx, &key)?;
        crate::db::client_keys::ClientKeyRepo::bind_pool_tx(&tx, &key.id, &group.id)?;
        tx.commit().map_err(|e| e.to_string())
    })();
    if let Err(error) = result {
        let _ = keychain::delete_secret(&secret_ref);
        return Err(error);
    }

    Ok(RoutePoolCreated {
        group: group.clone(),
        key: key_view(&key, &group.id),
        raw_key,
    })
}

pub fn ensure_pool_key(state: &AppState, pool_id: &str) -> Result<ManagedPoolKeyCreated, String> {
    if state
        .db
        .client_keys
        .get_managed_for_pool(&state.db.conn, pool_id)?
        .is_some()
    {
        return Err("POOL_KEY_EXISTS: 路由池已存在专属 Key，可使用刷新替换".into());
    }
    let group = state
        .db
        .groups
        .get_by_id(&state.db.conn, pool_id)?
        .ok_or_else(|| "POOL_NOT_FOUND: 路由池不存在".to_string())?;
    let raw_key = crate::db::client_keys::generate_key();
    let key_id = Uuid::new_v4().to_string();
    let secret_ref = keychain::client_key_secret_ref(&key_id, &Uuid::new_v4().simple().to_string());
    keychain::store_verified(&secret_ref, raw_key.as_bytes())?;
    let key = build_managed_key(&group, &raw_key, &key_id, &secret_ref);
    let result = (|| {
        let mut conn = state.db.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        crate::db::client_keys::ClientKeyRepo::insert_tx(&tx, &key)?;
        crate::db::client_keys::ClientKeyRepo::bind_pool_tx(&tx, &key.id, pool_id)?;
        tx.commit().map_err(|e| e.to_string())
    })();
    if let Err(error) = result {
        let _ = keychain::delete_secret(&secret_ref);
        return Err(error);
    }
    Ok(ManagedPoolKeyCreated {
        key: key_view(&key, pool_id),
        raw_key,
    })
}

pub fn rotate_pool_key(state: &AppState, pool_id: &str) -> Result<ManagedPoolKeyCreated, String> {
    let existing = state
        .db
        .client_keys
        .get_managed_for_pool(&state.db.conn, pool_id)?
        .ok_or_else(|| "POOL_KEY_MISSING: 路由池尚未生成专属 Key".to_string())?;
    let raw_key = crate::db::client_keys::generate_key();
    let new_ref =
        keychain::client_key_secret_ref(&existing.id, &Uuid::new_v4().simple().to_string());
    keychain::store_verified(&new_ref, raw_key.as_bytes())?;
    let old_ref = existing.secret_ref.clone();
    let result = (|| {
        let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE client_keys SET key_hash=?1, key_last_four=?2, secret_ref=?3, rotated_at=CURRENT_TIMESTAMP, enabled=1 WHERE id=?4 AND managed_pool_id=?5",
            rusqlite::params![
                hash_key(&raw_key),
                &raw_key[raw_key.len() - 4..],
                new_ref,
                existing.id,
                pool_id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })();
    if let Err(error) = result {
        let _ = keychain::delete_secret(&new_ref);
        return Err(error);
    }
    if let Some(old_ref) = old_ref {
        if let Err(error) = keychain::delete_secret(&old_ref) {
            tracing::warn!(
                "Failed to delete rotated client-key secret ref={}: {}",
                old_ref,
                error
            );
        }
    }
    let current = state
        .db
        .client_keys
        .get_managed_for_pool(&state.db.conn, pool_id)?
        .ok_or_else(|| "POOL_KEY_MISSING: 轮换后无法读取专属 Key".to_string())?;
    Ok(ManagedPoolKeyCreated {
        key: key_view(&current, pool_id),
        raw_key,
    })
}

pub fn managed_pool_secret(state: &AppState, pool_id: &str) -> Result<String, String> {
    let key = state
        .db
        .client_keys
        .get_managed_for_pool(&state.db.conn, pool_id)?
        .ok_or_else(|| "POOL_KEY_MISSING: 路由池尚未生成专属 Key".to_string())?;
    let secret_ref = key
        .secret_ref
        .ok_or_else(|| "POOL_KEY_SECRET_MISSING: 专属 Key 的安全凭据引用缺失".to_string())?;
    String::from_utf8(keychain::get_secret(&secret_ref)?)
        .map_err(|_| "POOL_KEY_SECRET_INVALID: 专属 Key 编码无效".to_string())
}
