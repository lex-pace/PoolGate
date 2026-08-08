use crate::AppState;
use futures::stream::{self, StreamExt};
use std::sync::Arc;
use tauri::State;

#[derive(serde::Serialize)]
pub struct HealthResult {
    pub account_id: String,
    pub status: String,
    pub latency_ms: u64,
    pub code: u16,
    pub message: String,
}

#[tauri::command]
pub async fn check_account_health(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> Result<HealthResult, String> {
    let checker = crate::services::health_check::HealthChecker::new(10);

    let account = state
        .db
        .accounts
        .get_by_id(&state.db.conn, &account_id)?
        .ok_or_else(|| "Account not found".to_string())?;

    let provider = state
        .db
        .providers
        .get_by_id(
            &state.db.conn,
            &account.provider_id.as_deref().unwrap_or(""),
        )?
        .ok_or_else(|| "Provider not found".to_string())?;

    let result = checker.check_account(&account, &provider).await;

    // Persist health result to DB
    match &result {
        crate::services::health_check::HealthResult::Passed { latency_ms } => {
            state
                .db
                .accounts
                .update_health(
                    &state.db.conn,
                    &account_id,
                    "healthy",
                    200,
                    "OK",
                    *latency_ms as i64,
                )
                .ok();
            // Also update status to "active" if it was "disabled", or recover
            // a terminal credential state (token_expired / error) — the check
            // just proved the credential usable again.
            if account.status.as_deref() == Some("disabled") {
                state
                    .db
                    .accounts
                    .update_status(&state.db.conn, &account_id, "active")
                    .ok();
            }
            state
                .db
                .accounts
                .recover_status(&state.db.conn, &account_id, account.status.as_deref())
                .ok();
        }
        crate::services::health_check::HealthResult::Failed { code, body } => {
            state
                .db
                .accounts
                .update_health(&state.db.conn, &account_id, "failed", *code, body, 0)
                .ok();
        }
        crate::services::health_check::HealthResult::Timeout => {
            state
                .db
                .accounts
                .update_health(
                    &state.db.conn,
                    &account_id,
                    "timeout",
                    0,
                    "Request timed out",
                    5000,
                )
                .ok();
        }
        crate::services::health_check::HealthResult::Error(msg) => {
            state
                .db
                .accounts
                .update_health(&state.db.conn, &account_id, "error", 0, msg, 0)
                .ok();
        }
    }

    match result {
        crate::services::health_check::HealthResult::Passed { latency_ms } => Ok(HealthResult {
            account_id,
            status: "healthy".into(),
            latency_ms,
            code: 200,
            message: "OK".into(),
        }),
        crate::services::health_check::HealthResult::Failed { code, body } => Ok(HealthResult {
            account_id,
            status: "failed".into(),
            latency_ms: 0,
            code,
            message: body,
        }),
        crate::services::health_check::HealthResult::Timeout => Ok(HealthResult {
            account_id,
            status: "timeout".into(),
            latency_ms: 5000,
            code: 0,
            message: "Request timed out".into(),
        }),
        crate::services::health_check::HealthResult::Error(msg) => Err(msg),
    }
}

#[tauri::command]
pub async fn batch_check_health(
    state: State<'_, Arc<AppState>>,
    account_ids: Option<Vec<String>>,
) -> Result<Vec<HealthResult>, String> {
    let accounts = if let Some(ref ids) = account_ids {
        let mut accs = Vec::new();
        for id in ids {
            if let Some(acc) = state.db.accounts.get_by_id(&state.db.conn, id)? {
                accs.push(acc);
            }
        }
        accs
    } else {
        state.db.accounts.list_all(&state.db.conn)?
    };

    let checks = stream::iter(accounts)
        .map(|account| {
            let provider = state
                .db
                .providers
                .get_by_id(&state.db.conn, account.provider_id.as_deref().unwrap_or(""));
            async move {
                let checker = crate::services::health_check::HealthChecker::new(10);
                let result = match provider {
                    Ok(Some(provider)) => checker.check_account(&account, &provider).await,
                    Ok(None) => crate::services::health_check::HealthResult::Error(
                        "Provider not found".into(),
                    ),
                    Err(error) => crate::services::health_check::HealthResult::Error(error),
                };
                (account.id, result)
            }
        })
        .buffer_unordered(10)
        .collect::<Vec<_>>()
        .await;

    let mut results = Vec::with_capacity(checks.len());
    for (account_id, result) in checks {
        let output = match result {
            crate::services::health_check::HealthResult::Passed { latency_ms } => {
                state
                    .db
                    .accounts
                    .update_health(
                        &state.db.conn,
                        &account_id,
                        "healthy",
                        200,
                        "OK",
                        latency_ms as i64,
                    )
                    .ok();
                HealthResult {
                    account_id,
                    status: "healthy".into(),
                    latency_ms,
                    code: 200,
                    message: "OK".into(),
                }
            }
            crate::services::health_check::HealthResult::Failed { code, body } => {
                state
                    .db
                    .accounts
                    .update_health(&state.db.conn, &account_id, "failed", code, &body, 0)
                    .ok();
                HealthResult {
                    account_id,
                    status: "failed".into(),
                    latency_ms: 0,
                    code,
                    message: body,
                }
            }
            crate::services::health_check::HealthResult::Timeout => {
                state
                    .db
                    .accounts
                    .update_health(&state.db.conn, &account_id, "timeout", 0, "Timeout", 5000)
                    .ok();
                HealthResult {
                    account_id,
                    status: "timeout".into(),
                    latency_ms: 5000,
                    code: 0,
                    message: "Timeout".into(),
                }
            }
            crate::services::health_check::HealthResult::Error(message) => {
                state
                    .db
                    .accounts
                    .update_health(&state.db.conn, &account_id, "error", 0, &message, 0)
                    .ok();
                HealthResult {
                    account_id,
                    status: "error".into(),
                    latency_ms: 0,
                    code: 0,
                    message,
                }
            }
        };
        results.push(output);
    }

    Ok(results)
}

#[tauri::command]
pub async fn cleanup_expired(state: State<'_, Arc<AppState>>) -> Result<serde_json::Value, String> {
    let checker = crate::services::health_check::HealthChecker::new(10);
    let accounts = state.db.accounts.list_all(&state.db.conn)?;
    let mut disabled = Vec::new();

    for account in &accounts {
        if account.status.as_deref() == Some("disabled") {
            continue;
        }
        if let Some(provider) = state.db.providers.get_by_id(
            &state.db.conn,
            &account.provider_id.as_deref().unwrap_or(""),
        )? {
            let result = checker.check_account(account, &provider).await;

            match &result {
                crate::services::health_check::HealthResult::Passed { latency_ms } => {
                    state
                        .db
                        .accounts
                        .update_health(
                            &state.db.conn,
                            &account.id,
                            "healthy",
                            200,
                            "OK",
                            *latency_ms as i64,
                        )
                        .ok();
                    state
                        .db
                        .accounts
                        .recover_status(&state.db.conn, &account.id, account.status.as_deref())
                        .ok();
                }
                crate::services::health_check::HealthResult::Failed { code, body } => {
                    state
                        .db
                        .accounts
                        .update_health(&state.db.conn, &account.id, "failed", *code, body, 0)
                        .ok();
                    state
                        .db
                        .accounts
                        .update_status(&state.db.conn, &account.id, "disabled")
                        .ok();
                    disabled.push(account.name.clone().unwrap_or_else(|| account.id.clone()));
                }
                crate::services::health_check::HealthResult::Timeout => {
                    state
                        .db
                        .accounts
                        .update_health(&state.db.conn, &account.id, "timeout", 0, "Timeout", 5000)
                        .ok();
                }
                crate::services::health_check::HealthResult::Error(msg) => {
                    state
                        .db
                        .accounts
                        .update_health(&state.db.conn, &account.id, "error", 0, msg, 0)
                        .ok();
                }
            }
        }
    }

    Ok(serde_json::json!({
        "checked": accounts.len(),
        "disabled": disabled,
    }))
}
