use crate::AppState;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub fn export_accounts(
    state: State<'_, Arc<AppState>>,
    format: String,
    mask_keys: Option<bool>,
) -> Result<String, String> {
    crate::services::export::export_accounts(&state.db.conn, &format, mask_keys.unwrap_or(true))
}

#[tauri::command]
pub fn export_account(
    state: State<'_, Arc<AppState>>,
    account_id: String,
    format: String,
    mask_keys: Option<bool>,
) -> Result<String, String> {
    crate::services::export::export_account(
        &state.db.conn,
        &account_id,
        &format,
        mask_keys.unwrap_or(true),
    )
}

#[tauri::command]
pub fn export_logs_csv(
    state: State<'_, Arc<AppState>>,
    start_time: Option<String>,
    end_time: Option<String>,
) -> Result<String, String> {
    crate::services::export::export_logs_csv(
        &state.db.conn,
        start_time.as_deref(),
        end_time.as_deref(),
    )
}
