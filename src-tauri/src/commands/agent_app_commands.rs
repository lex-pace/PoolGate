use crate::services::agent_apps::{AgentAppInfo, AgentAppLaunchResult, AgentAppPreview};
use crate::AppState;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Manager, State};

#[tauri::command]
pub fn detect_agent_apps() -> Result<Vec<AgentAppInfo>, String> {
    crate::services::agent_apps::detect_apps()
}

#[tauri::command]
pub fn preview_agent_app_config(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    app_id: String,
    group_id: String,
) -> Result<AgentAppPreview, String> {
    crate::services::agent_apps::validate_route_pool(state.inner().as_ref(), &group_id)?;
    let backup_root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join(".poolgate")
        .join("app-config-backups");
    crate::services::agent_apps::preview(&app_id, &group_id, &backup_root)
}

#[tauri::command]
pub fn configure_and_launch_agent_app(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    app_id: String,
    group_id: String,
    confirmed: bool,
    working_directory: Option<String>,
) -> Result<AgentAppLaunchResult, String> {
    let proxy = state.proxy.lock().map_err(|error| error.to_string())?;
    let port = proxy
        .as_ref()
        .map(|handle| handle.port)
        .ok_or_else(|| "GATEWAY_NOT_RUNNING: 请先启动本地 Agent 网关".to_string())?;
    drop(proxy);
    let backup_root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join(".poolgate")
        .join("app-config-backups");
    let working_directory = working_directory.map(PathBuf::from);
    crate::services::agent_apps::configure_and_launch(
        state.inner().as_ref(),
        &app_id,
        &group_id,
        &format!("http://127.0.0.1:{}", port),
        &backup_root,
        confirmed,
        working_directory.as_deref(),
    )
}

#[tauri::command]
pub fn restore_agent_app_config(
    state: State<'_, Arc<AppState>>,
    snapshot_id: String,
    force: Option<bool>,
) -> Result<(), String> {
    crate::services::agent_apps::restore(
        state.inner().as_ref(),
        &snapshot_id,
        force.unwrap_or(false),
    )
}
