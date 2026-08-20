use crate::services::keychain;
use crate::AppState;
use std::sync::Arc;
use tauri::State;

/// Legacy SQLite setting key. Existing values are migrated into the OS vault
/// on first read or write and then removed from SQLite.
pub const GATEWAY_ACCESS_KEY: &str = "gateway_access_key";

/// Setting key for close button behavior: "hide" or "quit"
pub const CLOSE_BUTTON_BEHAVIOR: &str = "close_button_behavior";

/// Setting key for the gateway listen address: "localhost" (default, binds
/// 127.0.0.1) or "lan" (binds 0.0.0.0 so teammates on the same network can
/// reach the gateway — always behind the gateway access key).
pub const LISTEN_ADDR: &str = "pg.listenAddr";

/// Setting key for the first-run onboarding wizard: "1" once the user has
/// completed (or dismissed) the three-step import → pool → config flow.
pub const ONBOARDING_COMPLETED: &str = "pg.onboarded";

/// Product mode is a startup-level choice. `gateway` is the complete product;
/// `monitor` hides and disables every gateway capability.
pub const APP_MODE: &str = "pg.appMode";

/// Appearance preference keys persisted in the SQLite `settings` table so the
/// Rust backend (tray window native Acrylic tint) can read them — the webview
/// localStorage mirror is not visible to Rust.
pub const THEME_PREFERENCE: &str = "pg.theme";
pub const GLASS_OPACITY: &str = "pg.glassOpacity";
pub const GLASS_BLUR: &str = "pg.glassBlur";
pub const ACCOUNT_DISPLAY: &str = "pg.accountDisplay";
/// 菜单栏主文本："tokens"（今日 Tokens 简写，默认）/ "top1_tool"（今日 Top1 工具）/
/// "top1_model"（今日 Top1 模型），三选一固定展示，不轮播。
pub const MENU_BAR_MAIN_TEXT: &str = "pg.menuBarMainText";

/// Persist theme / glass / account-display preferences to the Rust-readable
/// settings table and re-apply the tray window's native Acrylic tint (Windows).
/// macOS keeps the fixed `HudWindow` vibrancy material — the CSS glass overlay
/// handles themes there, so no native re-application is needed.
#[tauri::command]
#[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
pub fn set_appearance_prefs<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    theme: String,
    glass_opacity: Option<u8>,
    glass_blur: Option<u8>,
    account_display: Option<String>,
) -> Result<(), String> {
    if !matches!(theme.as_str(), "system" | "light" | "dark") {
        return Err("Invalid theme. Must be 'system', 'light' or 'dark'.".to_string());
    }
    if let Some(display) = &account_display {
        if !matches!(display.as_str(), "mask" | "full") {
            return Err("Invalid account display. Must be 'mask' or 'full'.".to_string());
        }
    }
    state
        .db
        .settings
        .set(&state.db.conn, THEME_PREFERENCE, &theme)?;
    if let Some(opacity) = glass_opacity {
        state
            .db
            .settings
            .set(&state.db.conn, GLASS_OPACITY, &opacity.to_string())?;
    }
    if let Some(blur) = glass_blur {
        state
            .db
            .settings
            .set(&state.db.conn, GLASS_BLUR, &blur.to_string())?;
    }
    if let Some(display) = account_display {
        state
            .db
            .settings
            .set(&state.db.conn, ACCOUNT_DISPLAY, &display)?;
    }
    // Windows: 切换主题/玻璃时立即重设托盘原生 Acrylic tint（窗口已创建时）。
    #[cfg(target_os = "windows")]
    crate::services::tray::apply_tray_acrylic_from_settings(&app, state.inner())?;
    Ok(())
}

/// 读取菜单栏主文本（默认 "tokens"：今日 Tokens 简写）。
#[tauri::command]
pub fn get_menu_bar_main_text(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    Ok(state
        .db
        .settings
        .get(&state.db.conn, MENU_BAR_MAIN_TEXT)?
        .unwrap_or_else(|| "tokens".to_string()))
}

/// 设置菜单栏主文本并立即重新渲染托盘图标：tokens = 今日 Tokens 简写（默认）；
/// top1_tool = 今日 Top1 工具；top1_model = 今日 Top1 模型。三选一固定展示，不轮播。
#[tauri::command]
pub fn set_menu_bar_main_text<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    main_text: String,
) -> Result<(), String> {
    if !matches!(main_text.as_str(), "tokens" | "top1_tool" | "top1_model") {
        return Err(
            "Invalid menu bar main text. Must be 'tokens', 'top1_tool' or 'top1_model'."
                .to_string(),
        );
    }
    state
        .db
        .settings
        .set(&state.db.conn, MENU_BAR_MAIN_TEXT, &main_text)?;
    crate::services::menu_bar::apply_menu_bar(&app, state.inner())
}

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

/// Resolve the persisted product mode. Existing installations with accounts
/// and route pools are treated as Gateway users for a non-breaking upgrade.
pub fn get_app_mode_value(state: &Arc<AppState>) -> Result<String, String> {
    if let Some(value) = state.db.settings.get(&state.db.conn, APP_MODE)? {
        return Ok(value);
    }
    let has_existing_gateway_data = !state.db.accounts.list_all(&state.db.conn)?.is_empty()
        && !state.db.groups.list_all(&state.db.conn)?.is_empty();
    Ok(if has_existing_gateway_data {
        "gateway"
    } else {
        ""
    }
    .to_string())
}

pub fn is_monitor_mode(state: &Arc<AppState>) -> Result<bool, String> {
    Ok(get_app_mode_value(state)? == "monitor")
}

/// Resolve the gateway listen mode (default "localhost").
pub fn get_listen_addr(state: &Arc<AppState>) -> Result<String, String> {
    Ok(state
        .db
        .settings
        .get(&state.db.conn, LISTEN_ADDR)?
        .unwrap_or_else(|| "localhost".to_string()))
}

#[derive(serde::Serialize)]
pub struct GatewaySettings {
    pub access_key_set: bool,
    pub close_button_behavior: String,
    /// "localhost" (127.0.0.1) or "lan" (0.0.0.0).
    pub listen_addr: String,
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
    if is_monitor_mode(state.inner())? {
        return Err("Monitor 模式已禁用 Gateway 设置".to_string());
    }
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
    let listen_addr = get_listen_addr(state.inner())?;
    Ok(GatewaySettings {
        access_key_set,
        close_button_behavior,
        listen_addr,
    })
}

/// Switch the gateway listen address between "localhost" (127.0.0.1, default)
/// and "lan" (0.0.0.0). LAN mode is only allowed when a gateway access key is
/// configured — otherwise the whole account pool would be exposed to the
/// network unauthenticated. If the gateway is running the change is applied
/// immediately by restarting it on the new address.
#[tauri::command]
pub async fn set_listen_addr(state: State<'_, Arc<AppState>>, mode: String) -> Result<(), String> {
    if is_monitor_mode(state.inner())? {
        return Err("Monitor 模式已禁用 Gateway 设置".to_string());
    }
    if !matches!(mode.as_str(), "localhost" | "lan") {
        return Err("Invalid listen mode. Must be 'localhost' or 'lan'.".to_string());
    }
    if mode == "lan" {
        // Load from the OS vault first so a key persisted in a previous run is
        // honoured even before the proxy has been started this session.
        load_gateway_access_key(state.inner())?;
        let access_key_set = state
            .gateway_access_key
            .read()
            .map_err(|error| error.to_string())?
            .is_some();
        if !access_key_set {
            return Err(
                "局域网监听必须先设置网关访问密钥，否则局域网内任何设备都能无鉴权调用网关。请先在上方保存访问密钥。"
                    .to_string(),
            );
        }
    }
    state.db.settings.set(&state.db.conn, LISTEN_ADDR, &mode)?;
    // 立即生效：若网关正在运行，自动重启以应用新的监听地址。
    let running = state
        .proxy
        .lock()
        .map_err(|error| error.to_string())?
        .is_some();
    if running {
        crate::commands::proxy_commands::stop_proxy_with_state(state.inner().clone()).await?;
        crate::commands::proxy_commands::start_proxy_with_state(state.inner().clone()).await?;
    }
    Ok(())
}

/// First-run onboarding state: whether the wizard was completed/dismissed and
/// how much the user has already set up (accounts + route pools). The app
/// shows the wizard only when `completed` is false AND the user does not
/// already have both accounts and pools.
#[derive(serde::Serialize)]
pub struct AppModeState {
    pub selected: bool,
    pub mode: String,
}

#[tauri::command]
pub fn get_app_mode(state: State<'_, Arc<AppState>>) -> Result<AppModeState, String> {
    let persisted = state.db.settings.get(&state.db.conn, APP_MODE)?;
    let mode = persisted.clone().unwrap_or_else(|| "gateway".to_string());
    if !matches!(mode.as_str(), "gateway" | "monitor") {
        return Err("Invalid persisted PoolGate app mode".to_string());
    }
    Ok(AppModeState {
        selected: persisted.is_some()
            || (!state.db.accounts.list_all(&state.db.conn)?.is_empty()
                && !state.db.groups.list_all(&state.db.conn)?.is_empty()),
        mode,
    })
}

#[tauri::command]
pub async fn set_app_mode(state: State<'_, Arc<AppState>>, mode: String) -> Result<(), String> {
    if !matches!(mode.as_str(), "gateway" | "monitor") {
        return Err("Invalid app mode. Must be 'gateway' or 'monitor'.".to_string());
    }
    if mode == "monitor" {
        crate::commands::proxy_commands::stop_proxy_with_state(state.inner().clone()).await?;
    }
    state.db.settings.set(&state.db.conn, APP_MODE, &mode)
}

#[derive(serde::Serialize)]
pub struct OnboardingState {
    pub completed: bool,
    pub account_count: i64,
    pub pool_count: i64,
}

#[tauri::command]
pub fn get_onboarding_state(state: State<'_, Arc<AppState>>) -> Result<OnboardingState, String> {
    let completed = state
        .db
        .settings
        .get(&state.db.conn, ONBOARDING_COMPLETED)?
        .map(|value| value == "1")
        .unwrap_or(false);
    let account_count = state.db.accounts.list_all(&state.db.conn)?.len() as i64;
    let pool_count = state.db.groups.list_all(&state.db.conn)?.len() as i64;
    Ok(OnboardingState {
        completed,
        account_count,
        pool_count,
    })
}

/// Mark the onboarding wizard as completed (or re-open it). Called when the
/// user finishes or skips the wizard, and silently when the user already has
/// accounts + pools from a previous session.
#[tauri::command]
pub fn set_onboarding_completed(
    state: State<'_, Arc<AppState>>,
    completed: bool,
) -> Result<(), String> {
    state.db.settings.set(
        &state.db.conn,
        ONBOARDING_COMPLETED,
        if completed { "1" } else { "0" },
    )
}

/// Set or clear the gateway access key. The secret is never stored in SQLite
/// and is never returned to the UI after it is saved.
#[tauri::command]
pub fn set_gateway_access_key(
    state: State<'_, Arc<AppState>>,
    access_key: String,
) -> Result<(), String> {
    if is_monitor_mode(state.inner())? {
        return Err("Monitor 模式已禁用 Gateway 设置".to_string());
    }
    migrate_legacy_access_key(&state)?;
    let trimmed = access_key.trim();
    if trimmed.is_empty() {
        // Clearing the key while listening on the LAN would expose the gateway.
        if get_listen_addr(&state)? == "lan" {
            return Err(
                "当前为局域网监听模式，不能清除网关访问密钥。请先切回「仅本机」监听后再清除。"
                    .to_string(),
            );
        }
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
