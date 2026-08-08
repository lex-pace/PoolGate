use std::sync::Arc;
use tauri::State;

use crate::services::import::{ImportOptions, ImportPreview, ImportResult, ImportSourceRequest};
use crate::AppState;

#[tauri::command]
pub async fn preview_import(
    state: State<'_, Arc<AppState>>,
    path: String,
) -> Result<ImportPreview, String> {
    crate::services::import::preview_request(
        &state.db.conn,
        &ImportSourceRequest {
            content: None,
            source_name: None,
            paths: vec![path],
            provider_hint: None,
        },
    )
}

#[tauri::command]
pub async fn preview_import_source(
    state: State<'_, Arc<AppState>>,
    request: ImportSourceRequest,
) -> Result<ImportPreview, String> {
    crate::services::import::preview_request(&state.db.conn, &request)
}

#[tauri::command]
pub fn execute_import(
    state: State<'_, Arc<AppState>>,
    request: ImportSourceRequest,
    options: ImportOptions,
) -> Result<ImportResult, String> {
    crate::services::import::execute_request(&state.db.conn, &request, &options)
}

#[tauri::command]
pub fn detect_import_format(path: String) -> Result<String, String> {
    crate::services::import::detect_format_from_path(&path)
}

#[tauri::command]
pub fn detect_import_content(
    content: String,
    source_name: Option<String>,
) -> Result<String, String> {
    Ok(crate::services::import::detect_format_name(
        &content,
        source_name.as_deref(),
    ))
}

/// Scan well-known locations for Cockpit-tools / Codex CLI configuration and
/// credential files so the user can import them without manually locating the
/// files.
#[tauri::command]
pub fn scan_agent_configs() -> crate::services::import::AgentConfigScanResult {
    crate::services::import::scan_agent_configs()
}
