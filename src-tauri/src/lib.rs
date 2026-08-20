pub mod commands;
pub mod db;
pub mod proxy;
pub mod services;
pub mod token_monitor;

use std::sync::Arc;
use tauri::Manager;
use tokio::sync::oneshot;

/// Handle to a running proxy server, held in shared state so `stop_proxy`
/// can trigger graceful shutdown and `get_proxy_status` can report truth.
pub struct ProxyHandle {
    /// Sender used to signal graceful shutdown to the running server task.
    pub shutdown_tx: oneshot::Sender<()>,
    /// Port the server bound to.
    pub port: u16,
    /// Listen mode the running server bound with (localhost / lan).
    pub listen_mode: crate::proxy::server::ListenMode,
}

/// Shared application state
pub struct AppState {
    pub db: db::Database,
    /// Gateway access key is kept only in memory and the OS credential vault.
    /// The proxy reads this lock for every request, so changes apply instantly.
    pub gateway_access_key: std::sync::RwLock<Option<String>>,
    /// Distinguishes an intentionally empty gateway key from a vault value
    /// that has not been loaded yet. This prevents credential-vault access on
    /// ordinary application launch.
    pub gateway_access_key_loaded: std::sync::atomic::AtomicBool,
    /// Running proxy server handle, if any. Guarded by a mutex so that
    /// `start_proxy` / `stop_proxy` / `get_proxy_status` can safely
    /// coordinate from multiple tauri commands.
    pub proxy: std::sync::Mutex<Option<ProxyHandle>>,
    /// Lightweight request runtime used by the dashboard and tray. It tracks
    /// active gateway requests and the latest selected route without querying
    /// SQLite on every animation refresh.
    pub gateway_runtime: proxy::runtime::GatewayRuntime,
    /// Shared per-account semaphore registry. Keeping it in AppState allows the
    /// provider inspector to report real active, available and queued capacity.
    pub account_concurrency: proxy::concurrency::AccountConcurrency,
    /// Per-account request pacing (spacing + jitter) for subscription/OAuth
    /// upstreams; shared between the proxy handlers and the app runtime.
    pub account_throttle: proxy::concurrency::AccountThrottle,
    /// Serializes configuration and restore operations per Agent application.
    /// A set is used instead of exposing arbitrary process commands to the UI.
    pub agent_app_operations: std::sync::Mutex<std::collections::HashSet<String>>,
    /// Canonical app data directory, resolved once in setup so commands that
    /// need the path (e.g. log file readers) can access it without holding
    /// an AppHandle.
    pub app_data_dir: Option<std::path::PathBuf>,
    // ==== token_monitor: state ====
    /// Token Monitor 运行时（W0 占位；W1 填充聚合/快照/事件广播，W2 填充采集调度）。
    pub token_monitor: token_monitor::TokenMonitorRuntime,
}

/// Initialise tracing with a dual-layer subscriber: stdout for development,
/// rolling daily file for production.
///
/// The file layer writes to `{app_data}/.poolgate/logs/app.YYYY-MM-DD.log`
/// via [`tracing_appender::rolling::daily`].  Old files are pruned on startup.
fn init_logging(log_dir: &std::path::Path) {
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    std::fs::create_dir_all(log_dir).ok();

    // Prune files older than the retention window.
    services::app_log::cleanup_old_logs(log_dir);

    let file_appender = tracing_appender::rolling::daily(log_dir, "app.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    // The non_blocking guard must outlive the process — leak it so tracing
    // never silently drops pending log lines.
    std::mem::forget(guard);

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_writer(std::io::stdout);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_ansi(false)
        .with_writer(file_writer);

    tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .with(tracing_subscriber::filter::EnvFilter::new("info"))
        .init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Tracing is initialised inside `.setup()` (see `init_logging`) so the
    // log directory can be derived from Tauri's app_data_dir.  Any tracing
    // output before setup completes goes to stdout only (acceptable: no
    // critical events occur before setup).

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;

            let poolgate_dir = app_data_dir.join(".poolgate");
            std::fs::create_dir_all(&poolgate_dir)?;

            // Initialise tracing with rolling daily file before any service
            // or background loop starts, so all subsequent log output is
            // captured to disk.
            init_logging(&poolgate_dir.join("logs"));

            services::keychain::initialize_vault(poolgate_dir.join("credentials.vault.json"))?;

            let db_path = poolgate_dir.join("gateway.db");
            let database = db::Database::new(&db_path)?;
            database.run_migrations()?;

            // Ordinary application launch performs no credential-vault I/O.
            // Historical plaintext migration and optional gateway-key loading
            // are deferred until the user explicitly starts the proxy or opens
            // the corresponding settings control.
            app.manage(Arc::new(AppState {
                db: database,
                gateway_access_key: std::sync::RwLock::new(None),
                gateway_access_key_loaded: std::sync::atomic::AtomicBool::new(false),
                proxy: std::sync::Mutex::new(None),
                gateway_runtime: proxy::runtime::GatewayRuntime::default(),
                account_concurrency: proxy::concurrency::AccountConcurrency::default(),
                account_throttle: proxy::concurrency::AccountThrottle::default(),
                agent_app_operations: std::sync::Mutex::new(std::collections::HashSet::new()),
                app_data_dir: Some(app_data_dir.clone()),
                token_monitor: token_monitor::TokenMonitorRuntime::default(),
            }));

            // ==== token_monitor: setup ====
            // Start the Token Monitor service (W0: empty; W1 aggregates/snapshots,
            // W2 starts the watcher/polling loops). It lives on AppState, not the
            // ProxyHandle, so it keeps running when the gateway is stopped.
            {
                let tm_state = app.state::<Arc<AppState>>().inner().clone();
                token_monitor::init(app.handle(), tm_state)?;
            }

            // Gateway-only background work is not started in Monitor mode. The
            // Token Monitor collector above remains independent and continues
            // to run in either product mode.
            let state = app.state::<Arc<AppState>>().inner().clone();
            if !commands::settings_commands::is_monitor_mode(&state)? {
                tauri::async_runtime::spawn(async move {
                    services::token_refresh::refresh_loop(state).await;
                });
            }

            // Forward the in-memory route lifecycle to both the command center
            // and tray webviews. Events are merged on a 250ms window so the UI
            // never receives per-request spam; only the latest snapshot within
            // each window is emitted. The broadcast channel drops stale
            // intermediate states when a hidden window cannot keep up.
            let runtime_state = app.state::<Arc<AppState>>().inner().clone();
            let runtime_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri::Emitter;
                let mut events = runtime_state.gateway_runtime.subscribe();
                loop {
                    match events.recv().await {
                        Ok(_) => {
                            // 250ms merge window: swallow intermediate events
                            // and emit the latest consolidated snapshot.
                            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                            while events.try_recv().is_ok() {}
                            let _ = runtime_app.emit(
                                "topology:runtime-delta",
                                runtime_state.gateway_runtime.snapshot(),
                            );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let _ = runtime_app.emit(
                                "topology:runtime-delta",
                                runtime_state.gateway_runtime.snapshot(),
                            );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            // Initialize tray
            services::tray::setup_tray(app)?;

            // Configure main window close behavior based on settings, and react
            // to system appearance changes by re-rendering the menu bar icon
            // (浅色/深色菜单栏 → 深色/近白云朵-P Logo).
            if let Some(main_window) = app.get_webview_window("main") {
                let main_window_clone = main_window.clone();
                let app_state_clone = app.state::<Arc<AppState>>().inner().clone();
                let menu_bar_app = app.handle().clone();

                // 主窗口也使用原生 Liquid Glass：CSS 透明层只负责 tint，
                // vibrancy 负责磨砂桌面/壁纸。托盘窗口在 tray.rs 中使用同一材质。
                #[cfg(target_os = "macos")]
                {
                    use window_vibrancy::{
                        apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState,
                    };
                    let _ = apply_vibrancy(
                        &main_window,
                        NSVisualEffectMaterial::HudWindow,
                        Some(NSVisualEffectState::Active),
                        Some(28.0),
                    );
                }
                #[cfg(target_os = "windows")]
                {
                    use window_vibrancy::apply_acrylic;
                    let tint = match main_window.theme() {
                        Ok(tauri::Theme::Dark) => (26, 30, 36, 120),
                        _ => (246, 248, 252, 112),
                    };
                    let _ = apply_acrylic(&main_window, Some(tint));
                }

                main_window.on_window_event(move |event| match event {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        // Check the close button behavior setting
                        let close_behavior = app_state_clone
                            .db
                            .settings
                            .get(
                                &app_state_clone.db.conn,
                                commands::settings_commands::CLOSE_BUTTON_BEHAVIOR,
                            )
                            .unwrap_or_else(|_| Some("hide".to_string()))
                            .unwrap_or_else(|| "hide".to_string());

                        if close_behavior == "quit" {
                            // Allow the window to close and exit the application
                            // Don't call api.prevent_close()
                            std::process::exit(0);
                        } else {
                            // Prevent the window from being destroyed
                            api.prevent_close();
                            // Hide the window instead
                            let _ = main_window_clone.hide();
                        }
                    }
                    tauri::WindowEvent::ThemeChanged(theme) => {
                        #[cfg(target_os = "windows")]
                        {
                            use window_vibrancy::apply_acrylic;
                            let tint = match theme {
                                tauri::Theme::Dark => (26, 30, 36, 120),
                                _ => (246, 248, 252, 112),
                            };
                            let _ = apply_acrylic(&main_window_clone, Some(tint));
                        }
                        // 系统外观切换（macOS 菜单栏浅色 ↔ 深色，或 Windows 应用
                        // 模式切换）时立即重绘菜单栏图标，不必等 10s 刷新循环。
                        // 使用传入的 `theme` 而非 `apply_menu_bar` 重新读窗口：
                        // 事件载荷是 Tauri 从 NSAppleInterfaceThemeChanged
                        // 实时发布的，比 `window.theme()` 更稳。
                        let appearance = match theme {
                            tauri::Theme::Dark => crate::services::menu_bar::Appearance::Dark,
                            _ => crate::services::menu_bar::Appearance::Light,
                        };
                        if let Err(error) =
                            crate::services::menu_bar::apply_menu_bar_with_appearance(
                                &menu_bar_app,
                                &app_state_clone,
                                appearance,
                            )
                        {
                            tracing::warn!("menu bar refresh on theme change failed: {error}");
                        }
                    }
                    _ => {}
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Provider commands
            commands::provider_commands::list_providers,
            commands::provider_commands::create_provider,
            commands::provider_commands::update_provider,
            commands::provider_commands::delete_provider,
            commands::provider_commands::test_provider_connection,
            commands::provider_commands::test_account_connection,
            commands::provider_commands::test_account_connection,
            // Account commands
            commands::account_commands::list_accounts,
            commands::account_commands::create_account,
            commands::account_commands::update_account,
            commands::account_commands::delete_account,
            commands::account_commands::batch_delete_accounts,
            commands::account_commands::batch_update_accounts,
            commands::account_commands::get_account_request_counts,
            // Account health commands
            commands::account_health::check_account_health,
            commands::account_health::batch_check_health,
            commands::account_health::cleanup_expired,
            // OAuth commands
            commands::oauth_commands::start_oauth_login,
            commands::oauth_commands::complete_oauth_login,
            commands::oauth_commands::cancel_oauth_login,
            commands::oauth_commands::copilot_pat_validate,
            commands::oauth_commands::gemini_api_key_validate,
            commands::oauth_commands::start_claude_oauth,
            commands::oauth_commands::complete_claude_oauth,
            commands::oauth_commands::cancel_claude_oauth,
            commands::oauth_commands::start_copilot_device_flow,
            commands::oauth_commands::poll_copilot_device_token,
            commands::oauth_commands::complete_copilot_device_flow,
            commands::oauth_commands::start_gemini_oauth,
            commands::oauth_commands::complete_gemini_oauth,
            commands::oauth_commands::cancel_gemini_oauth,
            commands::oauth_commands::start_antigravity_oauth,
            commands::oauth_commands::complete_antigravity_oauth,
            commands::oauth_commands::cancel_antigravity_oauth,
            commands::oauth_commands::start_grok_oauth,
            commands::oauth_commands::complete_grok_oauth,
            commands::oauth_commands::cancel_grok_oauth,
            // Account refresh commands
            commands::account_refresh::refresh_account_token,
            commands::account_refresh::batch_refresh_tokens,
            commands::account_refresh::refresh_account_quota,
            commands::account_refresh::batch_refresh_quotas,
            // Group commands
            commands::group_commands::list_groups,
            commands::group_commands::create_group,
            commands::group_commands::ensure_group_client_key,
            commands::group_commands::rotate_group_client_key,
            commands::group_commands::update_group,
            commands::group_commands::delete_group,
            commands::group_commands::get_group_accounts,
            commands::group_commands::get_group_model_resources,
            commands::group_commands::list_available_group_model_resources,
            commands::group_commands::add_group_model_resources,
            commands::group_commands::set_group_model_account_ids,
            commands::group_commands::remove_group_model_resource,
            commands::group_commands::set_group_model_resources,
            commands::group_commands::get_group_dashboard,
            commands::group_commands::get_route_topology,
            commands::group_commands::get_provider_topology_detail,
            commands::group_commands::add_account_to_group,
            commands::group_commands::remove_account_from_group,
            // Agent application commands
            commands::agent_app_commands::detect_agent_apps,
            commands::agent_app_commands::preview_agent_app_config,
            commands::agent_app_commands::configure_and_launch_agent_app,
            commands::agent_app_commands::restore_agent_app_config,
            // Log commands
            commands::log_commands::query_logs,
            commands::log_commands::get_log_stats,
            commands::log_commands::get_analytics,
            commands::log_commands::read_app_logs,
            commands::log_commands::get_app_log_info,
            services::tray::get_tray_snapshot,
            services::tray::open_poolgate_from_tray,
            services::tray::resize_tray_window,
            services::tray::quit_poolgate_from_tray,
            // Proxy commands
            commands::proxy_commands::start_proxy,
            commands::proxy_commands::stop_proxy,
            commands::proxy_commands::get_proxy_status,
            commands::proxy_commands::get_lan_addresses,
            // Import commands
            commands::import_commands::preview_import,
            commands::import_commands::preview_import_source,
            commands::import_commands::execute_import,
            commands::import_check_commands::preview_and_check_import,
            commands::import_commands::detect_import_format,
            commands::import_commands::detect_import_content,
            commands::import_commands::scan_agent_configs,
            commands::upstream_models_commands::fetch_upstream_models,
            commands::upstream_models_commands::refresh_account_models,
            commands::upstream_models_commands::batch_refresh_account_models,
            // Export commands
            commands::export_commands::export_accounts,
            commands::export_commands::export_account,
            commands::export_commands::export_logs_csv,
            // Settings commands
            commands::settings_commands::get_gateway_settings,
            commands::settings_commands::set_gateway_access_key,
            commands::settings_commands::get_app_mode,
            commands::settings_commands::set_app_mode,
            commands::settings_commands::get_onboarding_state,
            commands::settings_commands::set_onboarding_completed,
            commands::settings_commands::set_close_button_behavior,
            commands::settings_commands::get_close_button_behavior,
            commands::settings_commands::set_appearance_prefs,
            commands::settings_commands::get_menu_bar_main_text,
            commands::settings_commands::set_menu_bar_main_text,
            // Client key commands
            commands::client_key_commands::create_client_key,
            commands::client_key_commands::list_client_keys,
            commands::client_key_commands::update_client_key,
            commands::client_key_commands::delete_client_key,
            commands::client_key_commands::set_client_key_pools,
            commands::client_key_commands::get_client_key_pools,
            // ==== token_monitor commands ====
            commands::token_monitor_commands::get_token_monitor_snapshot,
            commands::token_monitor_commands::list_tool_usage,
            commands::token_monitor_commands::list_model_usage,
            commands::token_monitor_commands::list_active_sessions,
            commands::token_monitor_commands::list_session_events,
            commands::token_monitor_commands::list_projects,
            commands::token_monitor_commands::get_usage_trend,
            commands::token_monitor_commands::list_devices,
            commands::token_monitor_commands::get_collector_status,
            commands::token_monitor_commands::set_tool_collection,
            commands::token_monitor_commands::set_tool_paths,
            commands::token_monitor_commands::rescan_tool,
            commands::token_monitor_commands::reset_tool_data,
            commands::token_monitor_commands::scan_all_tools,
            commands::token_monitor_commands::get_token_rate,
            commands::token_monitor_commands::get_service_status,
            commands::token_monitor_commands::refresh_token_monitor,
            commands::token_monitor_commands::list_quota_accounts,
            commands::token_monitor_commands::get_account_token_stats,
            commands::token_monitor_commands::list_quota_providers,
            commands::token_monitor_commands::add_quota_account,
            commands::token_monitor_commands::refresh_quota_account,
            commands::token_monitor_commands::remove_quota_account,
            commands::token_monitor_commands::set_quota_alert_thresholds,
            commands::token_monitor_commands::get_token_monitor_tray_snapshot,
            commands::token_monitor_commands::get_tray_primary_metric,
            commands::token_monitor_commands::add_custom_app,
            commands::token_monitor_commands::remove_custom_app,
            commands::token_monitor_commands::detect_local_agents,
            commands::token_monitor_commands::enable_tool_monitoring,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
