use crate::services::account_refresh::RefreshResult;
use crate::AppState;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn refresh_account_token(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> Result<RefreshResult, String> {
    Ok(
        crate::services::account_refresh::refresh_account_token(state.inner().clone(), account_id)
            .await,
    )
}

#[tauri::command]
pub async fn batch_refresh_tokens(
    state: State<'_, Arc<AppState>>,
    account_ids: Vec<String>,
) -> Result<Vec<RefreshResult>, String> {
    Ok(
        crate::services::account_refresh::batch_refresh_tokens(state.inner().clone(), account_ids)
            .await,
    )
}

#[tauri::command]
pub async fn refresh_account_quota(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> Result<RefreshResult, String> {
    Ok(
        crate::services::account_refresh::refresh_account_quota(state.inner().clone(), account_id)
            .await,
    )
}

#[tauri::command]
pub async fn batch_refresh_quotas(
    state: State<'_, Arc<AppState>>,
    account_ids: Vec<String>,
) -> Result<Vec<RefreshResult>, String> {
    Ok(
        crate::services::account_refresh::batch_refresh_quotas(state.inner().clone(), account_ids)
            .await,
    )
}
