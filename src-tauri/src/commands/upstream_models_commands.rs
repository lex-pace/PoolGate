use crate::services::upstream_models::{self, ModelRefreshResult};
use crate::AppState;
use std::sync::Arc;
use tauri::State;

/// Fetch available model IDs from an upstream endpoint during import preview.
#[tauri::command]
pub async fn fetch_upstream_models(
    base_url: String,
    api_key: Option<String>,
    protocol: Option<String>,
) -> Result<Vec<String>, String> {
    upstream_models::fetch_upstream_models(&base_url, api_key.as_deref(), protocol.as_deref()).await
}

/// Refresh one persisted account without exposing its credential to the UI.
#[tauri::command]
pub async fn refresh_account_models(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> Result<ModelRefreshResult, String> {
    Ok(upstream_models::refresh_account_models(state.inner().clone(), account_id).await)
}

/// Refresh several persisted accounts with bounded concurrency.
#[tauri::command]
pub async fn batch_refresh_account_models(
    state: State<'_, Arc<AppState>>,
    account_ids: Vec<String>,
) -> Result<Vec<ModelRefreshResult>, String> {
    Ok(upstream_models::batch_refresh_account_models(state.inner().clone(), account_ids).await)
}
