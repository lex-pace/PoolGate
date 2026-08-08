use crate::{AppState, ProxyHandle};
use std::sync::Arc;
use tauri::State;
use tokio::sync::oneshot;

#[derive(serde::Serialize, Clone)]
pub struct ProxyStatus {
    pub running: bool,
    pub port: u16,
    pub active_connections: u32,
}

/// Default port the proxy binds to. Kept here (not AppState) so the
/// frontend can document it without querying.
const DEFAULT_PROXY_PORT: u16 = 9800;

/// Start the proxy server on `DEFAULT_PROXY_PORT`.
///
/// Real semantics:
/// * binds `127.0.0.1:9800` via `start_proxy_server`
/// * stores the shutdown sender in `AppState.proxy` so `stop_proxy` can
///   trigger graceful shutdown
/// * refuses to start twice — returns `Err` if a server is already running
/// * clears the stored handle when the server task exits (success or panic)
#[tauri::command]
pub async fn start_proxy(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    start_proxy_with_state(state.inner().clone()).await
}

pub async fn start_proxy_with_state(app_state: Arc<AppState>) -> Result<(), String> {
    // ── Refuse double start ────────────────────────────────────────────
    {
        let guard = app_state.proxy.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Err("代理已在运行中，请先停止".into());
        }
    }

    // Secure credential migration and optional gateway-key loading happen only
    // after an explicit user action. Merely opening PoolGate must not prompt for
    // vault access.
    match crate::db::accounts::migrate_plaintext_credentials(&app_state.db.conn) {
        Ok(count) if count > 0 => {
            tracing::info!("Migrated {} account credential(s) into the OS vault", count)
        }
        Ok(_) => {}
        Err(error) => {
            tracing::error!(
                "Account credential migration was not completed; plaintext was retained: {}",
                error
            );
            return Err(error);
        }
    }
    crate::commands::settings_commands::load_gateway_access_key(&app_state)?;

    // Resolve all account credentials once during this explicit start action.
    // Subsequent Agent requests use only the in-memory cache, preventing a
    // macOS Keychain prompt for every upstream attempt or failover.
    let account_secret_refs: Vec<String> = app_state
        .db
        .accounts
        .list_all(&app_state.db.conn)?
        .into_iter()
        .filter(|account| account.status.as_deref() != Some("disabled"))
        .filter_map(|account| account.secret_ref)
        .collect();
    let loaded = crate::services::keychain::preload_account_secrets(&account_secret_refs)?;
    tracing::info!("Loaded {} account credential(s) into runtime cache", loaded);

    // ── Allocate shutdown channel and store the sender ─────────────────
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    {
        let mut guard = app_state.proxy.lock().map_err(|e| e.to_string())?;
        *guard = Some(ProxyHandle {
            shutdown_tx,
            port: DEFAULT_PROXY_PORT,
        });
    }

    // ── Spawn the real server task ─────────────────────────────────────
    let state_for_task = app_state.clone();
    tokio::spawn(async move {
        let result = crate::proxy::server::start_proxy_server(
            state_for_task.clone(),
            DEFAULT_PROXY_PORT,
            shutdown_rx,
        )
        .await;

        match result {
            Ok(()) => tracing::info!("Proxy server task exited cleanly"),
            Err(e) => tracing::error!("Proxy server error: {}", e),
        }

        // ── Clear the handle so status reflects truth and start_proxy
        // can be called again.
        if let Ok(mut guard) = state_for_task.proxy.lock() {
            *guard = None;
        }
        crate::services::keychain::clear_runtime_cache();
    });

    Ok(())
}

/// Stop the running proxy server by signalling graceful shutdown.
///
/// Returns `Ok(())` if no server was running (idempotent), or after the
/// shutdown sender has been consumed. The actual server exit is
/// asynchronous; `get_proxy_status` will report `running: false` once
/// the server task has finished draining connections.
#[tauri::command]
pub async fn stop_proxy(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    stop_proxy_with_state(state.inner().clone()).await
}

pub async fn stop_proxy_with_state(app_state: Arc<AppState>) -> Result<(), String> {
    let handle = {
        let mut guard = app_state.proxy.lock().map_err(|e| e.to_string())?;
        guard.take()
    };
    match handle {
        Some(h) => {
            app_state.gateway_runtime.reset();
            // Sending consumes the sender; ignore the error that occurs
            // when the receiver already dropped (server already exited).
            let _ = h.shutdown_tx.send(());
            crate::services::keychain::clear_runtime_cache();
            tracing::info!("Proxy shutdown signalled and credential cache cleared");
            Ok(())
        }
        None => {
            // Nothing to stop — idempotent. Clear any stale cache left by an
            // unexpectedly terminated server task.
            crate::services::keychain::clear_runtime_cache();
            Ok(())
        }
    }
}

/// Report the REAL proxy status based on shared state.
///
/// `running` is true iff a `ProxyHandle` is currently stored in
/// `AppState.proxy`. The proxy server task clears it on exit, so this
/// accurately reflects whether the server is bound to the port.
#[tauri::command]
pub fn get_proxy_status(state: State<'_, Arc<AppState>>) -> Result<ProxyStatus, String> {
    let guard = state.inner().proxy.lock().map_err(|e| e.to_string())?;
    match guard.as_ref() {
        Some(h) => Ok(ProxyStatus {
            running: true,
            port: h.port,
            active_connections: state.inner().gateway_runtime.active_connections(),
        }),
        None => Ok(ProxyStatus {
            running: false,
            port: DEFAULT_PROXY_PORT,
            active_connections: 0,
        }),
    }
}
