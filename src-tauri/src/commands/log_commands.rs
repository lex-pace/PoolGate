use crate::db::logs::LogQuery;
use crate::services::app_log;
use crate::AppState;
use std::sync::Arc;
use tauri::State;

// ─── Request logs (existing API, function names preserved) ───────────────────

#[tauri::command]
pub fn query_logs(
    state: State<'_, Arc<AppState>>,
    query: LogQuery,
) -> Result<Vec<crate::db::logs::RequestLog>, String> {
    state.db.logs.query(&state.db.conn, &query)
}

#[tauri::command]
pub fn get_log_stats(
    state: State<'_, Arc<AppState>>,
    range: Option<String>,
) -> Result<crate::db::logs::LogStats, String> {
    state.db.logs.get_stats_range(&state.db.conn, range.as_deref())
}

#[tauri::command]
pub fn get_analytics(
    state: State<'_, Arc<AppState>>,
    start_date: Option<String>,
    end_date: Option<String>,
) -> Result<serde_json::Value, String> {
    state.db.logs.get_analytics(
        &state.db.conn,
        start_date.as_deref(),
        end_date.as_deref(),
    )
}

// ─── Application tracing log file (new) ──────────────────────────────────────

/// Read the latest application tracing log file with optional keyword
/// filtering and reverse-chronological pagination.
#[tauri::command]
pub fn read_app_logs(
    state: State<'_, Arc<AppState>>,
    page: Option<u32>,
    page_size: Option<u32>,
    keyword: Option<String>,
) -> Result<app_log::AppLogPage, String> {
    let app_data_dir = state
        .app_data_dir
        .as_ref()
        .ok_or("应用数据目录不可用")?;
    let log_dir = app_log::log_dir_for(app_data_dir);
    app_log::read_logs(
        &log_dir,
        page.unwrap_or(1),
        page_size.unwrap_or(50),
        keyword.as_deref(),
    )
}

/// Return metadata about the current tracing log file (path, size, line count).
#[tauri::command]
pub fn get_app_log_info(
    state: State<'_, Arc<AppState>>,
) -> Result<app_log::AppLogInfo, String> {
    let app_data_dir = state
        .app_data_dir
        .as_ref()
        .ok_or("应用数据目录不可用")?;
    let log_dir = app_log::log_dir_for(app_data_dir);
    app_log::get_log_info(&log_dir)
}
