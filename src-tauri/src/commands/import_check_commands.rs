use crate::services::import::{ImportOptions, ImportSourceRequest};
use crate::services::import_check::ImportCheckResult;
use crate::AppState;
use std::sync::Arc;
use tauri::State;

/// Parse an import source and run real upstream health checks without
/// persisting any accounts. The returned accounts include full fingerprints
/// that can later be passed to `execute_import` via
/// `ImportOptions.selected_fingerprints` to import only the selected subset.
#[tauri::command]
pub async fn preview_and_check_import(
    state: State<'_, Arc<AppState>>,
    request: ImportSourceRequest,
    options: ImportOptions,
) -> Result<ImportCheckResult, String> {
    crate::services::import_check::preview_and_check_import(&state.db.conn, &request, &options)
        .await
}
