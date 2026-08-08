use crate::services::keychain;
use crate::AppState;
use std::sync::Arc;
use tauri::State;

/// Legacy SQLite setting key. Existing values are migrated into the OS vault
/// on first read or write and then removed from SQLite.
pub const GATEWAY_ACCESS_KEY: &str = "gateway_access_key";

/// Setting key for close button behavior: "hide" or "quit"
pub const CLOSE_BUTTON_BEHAVIOR: &str = "close_button_behavior";

pub fn load_gateway_access_key(state: &Arc<AppState>) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    if state.gateway_access_key_loaded.load(Ordering::Acquire) {
        return Ok(());
    }
    let value = keychain::get_optional_secret(keychain::GATEWAY_ACCESS_KEY_REF)?
        .map(String::from_utf8)
        .transpose()
        .map_err(|error| format!("Gateway access key is not valid UTF-8: {}", error))?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    *state
        .gateway_access_key
        .write()
        .map_err(|error| error.to_string())? = value;
    state
        .gateway_access_key_loaded
        .store(true, Ordering::Release);
    Ok(())
}

#[derive(serde::Serialize)]
pub struct GatewaySettings {
    pub access_key_set: bool,
    pub close_button_behavior: String,
}

fn migrate_legacy_access_key(state: &Arc<AppState>) -> Result<(), String> {
    let Some(value) = state.db.settings.get(&state.db.conn, GATEWAY_ACCESS_KEY)? else {
        return Ok(());
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        state
            .db
            .settings
            .delete(&state.db.conn, GATEWAY_ACCESS_KEY)?;
        return Ok(());
    }
    keychain::store_verified(keychain::GATEWAY_ACCESS_KEY_REF, trimmed.as_bytes())?;
    *state
        .gateway_access_key
        .write()
        .map_err(|error| error.to_string())? = Some(trimmed.to_string());
    state
        .gateway_access_key_loaded
        .store(true, std::sync::atomic::Ordering::Release);
    state
        .db
        .settings
        .delete(&state.db.conn, GATEWAY_ACCESS_KEY)?;
    Ok(())
}

#[tauri::command]
pub fn get_gateway_settings(state: State<'_, Arc<AppState>>) -> Result<GatewaySettings, String> {
    migrate_legacy_access_key(&state)?;
    load_gateway_access_key(state.inner())?;
    let access_key_set = state
        .gateway_access_key
        .read()
        .map_err(|error| error.to_string())?
        .is_some();
    let close_button_behavior = state
        .db
        .settings
        .get(&state.db.conn, CLOSE_BUTTON_BEHAVIOR)?
        .unwrap_or_else(|| "hide".to_string());
    Ok(GatewaySettings {
        access_key_set,
        close_button_behavior,
    })
}

/// Set or clear the gateway access key. The secret is never stored in SQLite
/// and is never returned to the UI after it is saved.
#[tauri::command]
pub fn set_gateway_access_key(
    state: State<'_, Arc<AppState>>,
    access_key: String,
) -> Result<(), String> {
    migrate_legacy_access_key(&state)?;
    let trimmed = access_key.trim();
    if trimmed.is_empty() {
        if keychain::get_optional_secret(keychain::GATEWAY_ACCESS_KEY_REF)?.is_some() {
            keychain::delete_secret(keychain::GATEWAY_ACCESS_KEY_REF)?;
        }
        *state
            .gateway_access_key
            .write()
            .map_err(|error| error.to_string())? = None;
    } else {
        keychain::store_verified(keychain::GATEWAY_ACCESS_KEY_REF, trimmed.as_bytes())?;
        *state
            .gateway_access_key
            .write()
            .map_err(|error| error.to_string())? = Some(trimmed.to_string());
    }
    state
        .gateway_access_key_loaded
        .store(true, std::sync::atomic::Ordering::Release);
    Ok(())
}

/// Set the close button behavior: "hide" or "quit"
#[tauri::command]
pub fn set_close_button_behavior(
    state: State<'_, Arc<AppState>>,
    behavior: String,
) -> Result<(), String> {
    if behavior != "hide" && behavior != "quit" {
        return Err("Invalid close button behavior. Must be 'hide' or 'quit'.".to_string());
    }
    state
        .db
        .settings
        .set(&state.db.conn, CLOSE_BUTTON_BEHAVIOR, &behavior)?;
    Ok(())
}

/// Get the close button behavior setting
#[tauri::command]
pub fn get_close_button_behavior(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    state
        .db
        .settings
        .get(&state.db.conn, CLOSE_BUTTON_BEHAVIOR)
        .map(|opt| opt.unwrap_or_else(|| "hide".to_string()))
}
