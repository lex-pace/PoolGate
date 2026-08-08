use crate::db::accounts::Account;
use crate::AppState;
use std::collections::BTreeSet;
use std::sync::Arc;
use tauri::State;

#[derive(serde::Serialize)]
pub struct BatchDeleteAccountsResult {
    pub deleted_ids: Vec<String>,
    pub failures: Vec<BatchDeleteAccountFailure>,
}

#[derive(serde::Serialize)]
pub struct BatchDeleteAccountFailure {
    pub account_id: String,
    pub message: String,
}

#[tauri::command]
pub fn list_accounts(state: State<'_, Arc<AppState>>) -> Result<Vec<Account>, String> {
    state
        .db
        .accounts
        .list_all(&state.db.conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_account(
    state: State<'_, Arc<AppState>>,
    account: Account,
) -> Result<Account, String> {
    state
        .db
        .accounts
        .create(&state.db.conn, &account)
        .map_err(|e| e.to_string())?;
    Ok(account)
}

#[tauri::command]
pub fn update_account(state: State<'_, Arc<AppState>>, account: Account) -> Result<(), String> {
    state
        .db
        .accounts
        .update(&state.db.conn, &account)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_account(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    state
        .db
        .accounts
        .delete(&state.db.conn, &id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn batch_delete_accounts(
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
) -> Result<BatchDeleteAccountsResult, String> {
    let mut deleted_ids = Vec::new();
    let mut failures = Vec::new();
    let unique_ids = ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<BTreeSet<_>>();

    for id in unique_ids {
        match state.db.accounts.delete(&state.db.conn, &id) {
            Ok(()) => deleted_ids.push(id),
            Err(message) => failures.push(BatchDeleteAccountFailure {
                account_id: id,
                message,
            }),
        }
    }

    Ok(BatchDeleteAccountsResult {
        deleted_ids,
        failures,
    })
}

#[tauri::command]
pub fn batch_update_accounts(
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    status: String,
) -> Result<(), String> {
    state
        .db
        .accounts
        .batch_update_status(&state.db.conn, &ids, &status)
        .map_err(|e| e.to_string())
}

/// Get request counts for all accounts in the last N days.
#[tauri::command]
pub fn get_account_request_counts(
    state: State<'_, Arc<AppState>>,
    days: Option<i64>,
) -> Result<std::collections::HashMap<String, i64>, String> {
    let days = days.unwrap_or(7);
    let conn = state.db.conn.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT account_id, COUNT(*) as cnt
             FROM request_logs
             WHERE request_at > datetime('now', ?1 || ' days')
               AND account_id IS NOT NULL
             GROUP BY account_id",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(rusqlite::params![format!("-{}", days)], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;

    let mut counts = std::collections::HashMap::new();
    for row in rows {
        let (id, count) = row.map_err(|e| e.to_string())?;
        counts.insert(id, count);
    }
    Ok(counts)
}
