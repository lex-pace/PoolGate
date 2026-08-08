//! Native system-tray command center for PoolGate.
//!
//! The menu is rebuilt from live application state so it can surface the
//! gateway shortcut, enabled route pools and selectable token ranges without
//! opening the main window.

use crate::db::logs::{TokenRangeStats, TrayActivityPoint, TrayHeatmapPoint};
use crate::AppState;
use chrono::Local;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Emitter, LogicalPosition, Manager, Position, Runtime, WebviewUrl,
    WebviewWindowBuilder, WindowEvent,
};

const TRAY_ID: &str = "poolgate-main-tray";
const TRAY_WINDOW_LABEL: &str = "tray-card";
const TRAY_WINDOW_WIDTH: f64 = 410.0;
const TRAY_WINDOW_HEIGHT: f64 = 620.0;
const TRAY_WINDOW_MIN_WIDTH: f64 = 386.0;
const TRAY_WINDOW_MAX_WIDTH: f64 = 430.0;
const TRAY_WINDOW_MIN_HEIGHT: f64 = 360.0;
const TRAY_WINDOW_MAX_HEIGHT: f64 = 760.0;
const DEFAULT_PROXY_PORT: u16 = 9800;
const MAX_VISIBLE_POOLS: usize = 8;

#[derive(serde::Serialize)]
pub struct TrayPoolAccountSummary {
    id: String,
    name: String,
    email: Option<String>,
    status: String,
    health_status: String,
}

#[derive(serde::Serialize)]
pub struct TrayPoolProviderSummary {
    id: String,
    name: String,
    protocol: String,
    accounts: Vec<TrayPoolAccountSummary>,
}

#[derive(serde::Serialize)]
pub struct TrayPoolSummary {
    id: String,
    name: String,
    protocol: String,
    strategy: String,
    enabled: bool,
    resource_count: usize,
    healthy_resource_count: usize,
    model_count: usize,
    requests: i64,
    tokens: i64,
    providers: Vec<TrayPoolProviderSummary>,
}

#[derive(serde::Serialize)]
pub struct TrayActiveRoute {
    protocol: String,
    pool_id: String,
    pool_name: String,
    provider_id: String,
    provider_name: String,
    status: String,
    attempt: usize,
    latency_ms: Option<i64>,
    active_requests: u32,
    updated_at: String,
}

#[derive(serde::Serialize)]
pub struct TrayTopologySummary {
    protocol_count: usize,
    pool_count: usize,
    provider_count: usize,
    warning_count: usize,
    fault_count: usize,
}

#[derive(serde::Serialize)]
pub struct TraySnapshot {
    gateway_running: bool,
    port: u16,
    active_connections: u32,
    topology: TrayTopologySummary,
    resources_total: usize,
    resources_available: usize,
    resources_limited: usize,
    enabled_pool_count: usize,
    today_tokens: i64,
    seven_day_tokens: i64,
    month_tokens: i64,
    cumulative_tokens: i64,
    total_requests: i64,
    success_rate: f64,
    current_tps: f64,
    activity: Vec<TrayActivityPoint>,
    /// Daily token/request usage from the first logged request through today
    /// (tray heatmap view; not limited to a fixed window).
    tokens_heatmap: Vec<TrayHeatmapPoint>,
    pools: Vec<TrayPoolSummary>,
    active_route: Option<TrayActiveRoute>,
    updated_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenRange {
    Today,
    SevenDays,
    ThirtyDays,
    Month,
}

#[allow(dead_code)]
impl TokenRange {
    fn key(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::SevenDays => "7d",
            Self::ThirtyDays => "30d",
            Self::Month => "month",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Today => "今日",
            Self::SevenDays => "近 7 天",
            Self::ThirtyDays => "近 30 天",
            Self::Month => "本月",
        }
    }

    fn menu_id(self) -> &'static str {
        match self {
            Self::Today => "tokens_today",
            Self::SevenDays => "tokens_7d",
            Self::ThirtyDays => "tokens_30d",
            Self::Month => "tokens_month",
        }
    }

    fn from_menu_id(id: &str) -> Option<Self> {
        match id {
            "tokens_today" => Some(Self::Today),
            "tokens_7d" => Some(Self::SevenDays),
            "tokens_30d" => Some(Self::ThirtyDays),
            "tokens_month" => Some(Self::Month),
            _ => None,
        }
    }

    fn all() -> [Self; 4] {
        [Self::Today, Self::SevenDays, Self::ThirtyDays, Self::Month]
    }
}

pub fn setup_tray<R: Runtime>(app: &mut App<R>) -> Result<(), Box<dyn std::error::Error>> {
    let token_range = Arc::new(Mutex::new(TokenRange::Today));
    let event_range = token_range.clone();

    // Don't set a native menu - we use the custom tray window (command card) instead.
    // Both left and right clicks will toggle the tray window.
    TrayIconBuilder::with_id(TRAY_ID)
        .show_menu_on_left_click(false)
        .tooltip("PoolGate · 本地模型网关")
        .icon(app.default_window_icon().cloned().unwrap())
        .on_menu_event(move |app, event| {
            handle_menu_event(app, event.id.as_ref(), &event_range);
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                position,
                button_state: MouseButtonState::Up,
                button,
                ..
            } = event
            {
                // Left click and right click both toggle the tray window (command card)
                if button == MouseButton::Left || button == MouseButton::Right {
                    if let Err(error) = toggle_tray_window(tray.app_handle(), position.x, position.y) {
                        tracing::warn!("Unable to toggle tray command card: {}", error);
                    }
                }
            }
        })
        .build(app)?;

    // Keep runtime state, enabled pools and token usage fresh even when the main
    // window is hidden. Rebuilding every 10 seconds avoids per-request UI work.
    let refresh_app = app.app_handle().clone();
    let refresh_range = token_range;
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            let selected = refresh_range
                .lock()
                .map(|range| *range)
                .unwrap_or(TokenRange::Today);
            if let Err(error) = refresh_tray_menu(&refresh_app, selected) {
                tracing::warn!("Tray refresh failed: {}", error);
            }
        }
    });

    tracing::info!("PoolGate command-center tray initialized");
    Ok(())
}

fn handle_menu_event<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    token_range: &Arc<Mutex<TokenRange>>,
) {
    if let Some(range) = TokenRange::from_menu_id(id) {
        if let Ok(mut selected) = token_range.lock() {
            *selected = range;
        }
        if let Err(error) = refresh_tray_menu(app, range) {
            tracing::warn!("Tray token range refresh failed: {}", error);
        }
        return;
    }

    match id {
        "toggle_proxy" => {
            let app = app.clone();
            let selected = token_range
                .lock()
                .map(|range| *range)
                .unwrap_or(TokenRange::Today);
            tauri::async_runtime::spawn(async move {
                let state = app.state::<Arc<AppState>>().inner().clone();
                let running = proxy_is_running(&state);
                let result = if running {
                    crate::commands::proxy_commands::stop_proxy_with_state(state).await
                } else {
                    crate::commands::proxy_commands::start_proxy_with_state(state).await
                };
                match result {
                    Ok(()) => tracing::info!(
                        "Tray: gateway {} requested",
                        if running { "stop" } else { "start" }
                    ),
                    Err(error) => tracing::error!("Tray gateway operation failed: {}", error),
                }
                if let Err(error) = refresh_tray_menu(&app, selected) {
                    tracing::warn!("Tray refresh after gateway operation failed: {}", error);
                }
            });
        }
        "refresh_tray" => {
            let selected = token_range
                .lock()
                .map(|range| *range)
                .unwrap_or(TokenRange::Today);
            if let Err(error) = refresh_tray_menu(app, selected) {
                tracing::warn!("Manual tray refresh failed: {}", error);
            }
        }
        "show_window" => show_main_window(app),
        "show_tray_card" => {
            if let Some(window) = app.get_webview_window(TRAY_WINDOW_LABEL) {
                if window.is_visible().unwrap_or(false) {
                    let _ = window.hide();
                } else {
                    if let Some(monitor) = window.primary_monitor().ok().flatten() {
                        let size = monitor.size();
                        let scale = monitor.scale_factor();
                        let x = (size.width as f64) / (2.0 * scale);
                        let y = (size.height as f64) / (2.0 * scale);
                        let _ = toggle_tray_window(app, x, y);
                    } else {
                        let _ = toggle_tray_window(app, 400.0, 400.0);
                    }
                }
            } else {
                if let Some(monitor) = app.primary_monitor().ok().flatten() {
                    let size = monitor.size();
                    let scale = monitor.scale_factor();
                    let x = (size.width as f64) / (2.0 * scale);
                    let y = (size.height as f64) / (2.0 * scale);
                    let _ = toggle_tray_window(app, x, y);
                } else {
                    let _ = toggle_tray_window(app, 400.0, 400.0);
                }
            }
        }
        "copy_claude" => copy_config(app, &generate_claude_config(), "Claude Code"),
        "copy_codex" => copy_config(app, &generate_codex_config(), "Codex"),
        "quit" => app.exit(0),
        _ => {}
    }
}

#[allow(dead_code)]
fn build_tray_menu<R: Runtime>(app: &AppHandle<R>, range: TokenRange) -> tauri::Result<Menu<R>> {
    let state = app.state::<Arc<AppState>>();
    let running = proxy_is_running(state.inner());
    let enabled_pools = state
        .db
        .groups
        .list_all(&state.db.conn)
        .unwrap_or_else(|error| {
            tracing::warn!("Unable to load route pools for tray: {}", error);
            Vec::new()
        })
        .into_iter()
        .filter(|pool| pool.enabled.unwrap_or(true))
        .collect::<Vec<_>>();
    let selected_stats = load_token_stats(state.inner(), range);

    let menu = Menu::new(app)?;
    let runtime = MenuItem::with_id(
        app,
        "runtime_status",
        format!(
            "PoolGate · {} · 127.0.0.1:{}",
            if running {
                "网关运行中"
            } else {
                "网关已关闭"
            },
            DEFAULT_PROXY_PORT
        ),
        false,
        None::<&str>,
    )?;
    menu.append(&runtime)?;

    let toggle = MenuItem::with_id(
        app,
        "toggle_proxy",
        if running {
            "关闭网关"
        } else {
            "启动网关"
        },
        true,
        None::<&str>,
    )?;
    menu.append(&toggle)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let pools_title = MenuItem::with_id(
        app,
        "enabled_pools_title",
        format!("已开启路由池 · {} 个", enabled_pools.len()),
        false,
        None::<&str>,
    )?;
    menu.append(&pools_title)?;
    if enabled_pools.is_empty() {
        menu.append(&MenuItem::with_id(
            app,
            "enabled_pools_empty",
            "  暂无已开启路由池",
            false,
            None::<&str>,
        )?)?;
    } else {
        for (index, pool) in enabled_pools.iter().take(MAX_VISIBLE_POOLS).enumerate() {
            menu.append(&MenuItem::with_id(
                app,
                format!("enabled_pool_{}", index),
                format!("  ● {} · {}", pool.name, protocol_label(&pool.protocol)),
                false,
                None::<&str>,
            )?)?;
        }
        if enabled_pools.len() > MAX_VISIBLE_POOLS {
            menu.append(&MenuItem::with_id(
                app,
                "enabled_pools_more",
                format!(
                    "  另有 {} 个路由池",
                    enabled_pools.len() - MAX_VISIBLE_POOLS
                ),
                false,
                None::<&str>,
            )?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let token_menu = Submenu::with_id(
        app,
        "token_stats",
        format!(
            "Tokens 统计 · {} · {}",
            range.label(),
            format_tokens(selected_stats.total_tokens)
        ),
        true,
    )?;
    for candidate in TokenRange::all() {
        let stats = load_token_stats(state.inner(), candidate);
        token_menu.append(&CheckMenuItem::with_id(
            app,
            candidate.menu_id(),
            format!(
                "{} · {}",
                candidate.label(),
                format_tokens(stats.total_tokens)
            ),
            true,
            candidate == range,
            None::<&str>,
        )?)?;
    }
    token_menu.append(&PredefinedMenuItem::separator(app)?)?;
    token_menu.append(&MenuItem::with_id(
        app,
        "token_input",
        format!(
            "输入 Tokens · {}",
            format_tokens(selected_stats.input_tokens)
        ),
        false,
        None::<&str>,
    )?)?;
    token_menu.append(&MenuItem::with_id(
        app,
        "token_output",
        format!(
            "输出 Tokens · {}",
            format_tokens(selected_stats.output_tokens)
        ),
        false,
        None::<&str>,
    )?)?;
    token_menu.append(&MenuItem::with_id(
        app,
        "token_cache",
        format!(
            "缓存 Tokens · {}",
            format_tokens(selected_stats.cache_tokens)
        ),
        false,
        None::<&str>,
    )?)?;
    menu.append(&token_menu)?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "refresh_tray",
        "刷新运行数据",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "show_tray_card",
        "显示托盘面板",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "show_window",
        "打开 PoolGate",
        true,
        None::<&str>,
    )?)?;

    let tools = Submenu::with_id(app, "agent_tools", "Agent 配置", true)?;
    tools.append(&MenuItem::with_id(
        app,
        "copy_claude",
        "复制 Claude Code 配置",
        true,
        None::<&str>,
    )?)?;
    tools.append(&MenuItem::with_id(
        app,
        "copy_codex",
        "复制 Codex 配置",
        true,
        None::<&str>,
    )?)?;
    menu.append(&tools)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        "退出 PoolGate",
        true,
        Some("CmdOrCtrl+Q"),
    )?)?;
    Ok(menu)
}

fn refresh_tray_menu<R: Runtime>(app: &AppHandle<R>, _range: TokenRange) -> Result<(), String> {
    // We don't use the native menu anymore - only update the tooltip.
    let tray = app
        .tray_by_id(TRAY_ID)
        .ok_or_else(|| "找不到 PoolGate 托盘图标".to_string())?;
    tray.set_tooltip(Some(
        if proxy_is_running(app.state::<Arc<AppState>>().inner()) {
            "PoolGate · 网关运行中"
        } else {
            "PoolGate · 网关已关闭"
        },
    ))
    .map_err(|error| error.to_string())
}

fn proxy_is_running(state: &Arc<AppState>) -> bool {
    state
        .proxy
        .lock()
        .map(|proxy| proxy.is_some())
        .unwrap_or(false)
}

fn load_token_stats(state: &Arc<AppState>, range: TokenRange) -> TokenRangeStats {
    state
        .db
        .logs
        .get_token_range_stats(&state.db.conn, range.key())
        .unwrap_or_else(|error| {
            tracing::warn!(
                "Unable to load {} token stats for tray: {}",
                range.key(),
                error
            );
            TokenRangeStats::default()
        })
}

fn protocol_label(protocol: &str) -> &'static str {
    match protocol.to_ascii_lowercase().as_str() {
        "anthropic" => "Anthropic",
        "gemini" => "Gemini",
        "both" => "多协议",
        "responses" | "response" | "openai_responses" | "codex" => "Responses",
        _ => "OpenAI Compatible",
    }
}

fn strategy_label(strategy: Option<&str>) -> &'static str {
    match strategy.unwrap_or("round_robin") {
        "least_used" => "最少使用",
        "priority" => "优先级",
        "random" => "随机",
        "cost_optimized" => "成本优先",
        _ => "轮询",
    }
}

#[allow(dead_code)]
fn format_tokens(value: i64) -> String {
    let value = value.max(0);
    if value >= 1_000_000_000 {
        format!("{:.2}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.2}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn toggle_tray_window<R: Runtime>(
    app: &AppHandle<R>,
    click_x: f64,
    click_y: f64,
) -> Result<(), String> {
    // Always show/hide the tray window (command card).
    // The main window is only opened via the "打开" button in the tray window.
    let window = if let Some(window) = app.get_webview_window(TRAY_WINDOW_LABEL) {
        window
    } else {
        let window = WebviewWindowBuilder::new(
            app,
            TRAY_WINDOW_LABEL,
            WebviewUrl::App("index.html?view=tray".into()),
        )
        .title("PoolGate 状态")
        .inner_size(TRAY_WINDOW_WIDTH, TRAY_WINDOW_HEIGHT)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .visible(false)
        .shadow(true)
        .build()
        .map_err(|error| error.to_string())?;

        let hide_window = window.clone();
        window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Focused(false)) {
                let _ = hide_window.hide();
            }
        });
        window
    };

    if window.is_visible().unwrap_or(false) {
        return window.hide().map_err(|error| error.to_string());
    }

    let scale = window
        .monitor_from_point(click_x, click_y)
        .ok()
        .flatten()
        .map(|monitor| monitor.scale_factor())
        .unwrap_or(1.0);
    let logical_x = click_x / scale - TRAY_WINDOW_WIDTH / 2.0;
    let logical_y = click_y / scale + 8.0;
    window
        .set_position(Position::Logical(LogicalPosition::new(
            logical_x, logical_y,
        )))
        .map_err(|error| error.to_string())?;
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

#[tauri::command]
/// Opens the main window and navigates to the given page. An optional node
/// target (`pool-{id}`, `provider-{id}`, `protocol-{proto}` or `gateway`) is
/// forwarded as `page?node={node}` so the command center can focus it.
pub fn open_poolgate_from_tray<R: Runtime>(
    app: AppHandle<R>,
    page: Option<String>,
    node: Option<String>,
) {
    if let Some(window) = app.get_webview_window(TRAY_WINDOW_LABEL) {
        let _ = window.hide();
    }
    show_main_window(&app);
    if let Some(page) = page {
        let payload = match node {
            Some(node) if !node.is_empty() => format!("{}?node={}", page, node),
            _ => page,
        };
        let _ = app.emit_to("main", "tray:navigate", payload);
    }
}

#[tauri::command]
pub fn resize_tray_window<R: Runtime>(
    app: AppHandle<R>,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let window = app
        .get_webview_window(TRAY_WINDOW_LABEL)
        .ok_or_else(|| "找不到 PoolGate 托盘窗口".to_string())?;
    let clamped_width = width.clamp(TRAY_WINDOW_MIN_WIDTH, TRAY_WINDOW_MAX_WIDTH);
    let clamped_height = height.clamp(TRAY_WINDOW_MIN_HEIGHT, TRAY_WINDOW_MAX_HEIGHT);
    let current_position = window.outer_position().ok();
    window
        .set_size(tauri::Size::Logical(tauri::LogicalSize::new(
            clamped_width,
            clamped_height,
        )))
        .map_err(|error| error.to_string())?;
    if let Some(position) = current_position {
        let monitor = window
            .monitor_from_point(position.x as f64, position.y as f64)
            .ok()
            .flatten()
            .or_else(|| window.current_monitor().ok().flatten());
        if let Some(monitor) = monitor {
            let scale = monitor.scale_factor();
            let work = monitor.work_area();
            let right = work.position.x as f64 + work.size.width as f64;
            let bottom = work.position.y as f64 + work.size.height as f64;
            let max_x = right - clamped_width * scale;
            let max_y = bottom - clamped_height * scale;
            let next_x = (position.x as f64).min(max_x).max(work.position.x as f64);
            let next_y = (position.y as f64).min(max_y).max(work.position.y as f64);
            window
                .set_position(Position::Physical(tauri::PhysicalPosition::new(
                    next_x.round() as i32,
                    next_y.round() as i32,
                )))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn quit_poolgate_from_tray<R: Runtime>(app: AppHandle<R>) {
    app.exit(0);
}

#[tauri::command]
pub fn get_tray_snapshot(state: tauri::State<'_, Arc<AppState>>) -> Result<TraySnapshot, String> {
    let state = state.inner();
    let accounts = state.db.accounts.list_all(&state.db.conn)?;
    let resources_total = accounts.len();
    let resources_available = accounts
        .iter()
        .filter(|account| {
            crate::services::credentials::is_available_for_routing(account)
                && matches!(
                    account.health_status.as_deref(),
                    None | Some("healthy") | Some("unchecked")
                )
        })
        .count();
    let resources_limited = accounts
        .iter()
        .filter(|account| {
            matches!(
                account.health_status.as_deref(),
                Some("rate_limited") | Some("limited")
            )
        })
        .count();
    let providers = state.db.providers.list_all(&state.db.conn)?;
    let groups = state.db.groups.list_all(&state.db.conn)?;
    let enabled_pool_count = groups
        .iter()
        .filter(|pool| pool.enabled.unwrap_or(true))
        .count();
    let protocol_count = groups
        .iter()
        .flat_map(|pool| crate::commands::group_commands::pool_entry_protocols(&pool.protocol))
        .collect::<std::collections::HashSet<_>>()
        .len();
    let mut referenced_provider_ids = std::collections::HashSet::new();
    let mut pools = Vec::with_capacity(groups.len());
    for pool in groups {
        let resources = state
            .db
            .groups
            .get_model_resources(&state.db.conn, &pool.id)?;
        let mut provider_ids: Vec<_> = resources
            .iter()
            .map(|resource| resource.provider_id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        referenced_provider_ids.extend(provider_ids.iter().cloned());
        provider_ids.sort_by(|left, right| {
            let left_name = providers
                .iter()
                .find(|provider| provider.id == *left)
                .map(|provider| provider.name.as_str())
                .unwrap_or(left.as_str());
            let right_name = providers
                .iter()
                .find(|provider| provider.id == *right)
                .map(|provider| provider.name.as_str())
                .unwrap_or(right.as_str());
            left_name.cmp(right_name)
        });
        let pool_providers = provider_ids
            .into_iter()
            .filter_map(|provider_id| {
                let provider = providers
                    .iter()
                    .find(|provider| provider.id == provider_id)?;
                let provider_models =
                    crate::commands::group_commands::parse_models(provider.models.as_deref());
                let provider_resources: Vec<_> = resources
                    .iter()
                    .filter(|resource| resource.provider_id == provider.id)
                    .collect();
                let mut pool_accounts: Vec<_> = accounts
                    .iter()
                    .filter(|account| {
                        if account.provider_id.as_deref() != Some(provider.id.as_str())
                            || !crate::services::credentials::is_available_for_routing(account)
                        {
                            return false;
                        }
                        let account_models = crate::commands::group_commands::parse_models(
                            account.models.as_deref(),
                        );
                        let declared = if account_models.is_empty() {
                            &provider_models
                        } else {
                            &account_models
                        };
                        provider_resources.iter().any(|resource| {
                            declared.is_empty()
                                || declared.iter().any(|model| model == &resource.model)
                        })
                    })
                    .map(|account| TrayPoolAccountSummary {
                        id: account.id.clone(),
                        name: account
                            .name
                            .clone()
                            .filter(|name| !name.trim().is_empty())
                            .or_else(|| account.email.clone())
                            .unwrap_or_else(|| "未命名账号".to_string()),
                        email: account.email.clone(),
                        status: account
                            .status
                            .clone()
                            .unwrap_or_else(|| "enabled".to_string()),
                        health_status: account
                            .health_status
                            .clone()
                            .unwrap_or_else(|| "unchecked".to_string()),
                    })
                    .collect();
                pool_accounts.sort_by(|left, right| left.name.cmp(&right.name));
                Some(TrayPoolProviderSummary {
                    id: provider.id.clone(),
                    name: provider.name.clone(),
                    protocol: protocol_label(&provider.protocol).to_string(),
                    accounts: pool_accounts,
                })
            })
            .collect();
        let dashboard = crate::commands::group_commands::build_group_dashboard(
            state.as_ref(),
            &pool.id,
            Some("24h"),
        )?;
        pools.push(TrayPoolSummary {
            id: pool.id,
            name: pool.name,
            protocol: protocol_label(&pool.protocol).to_string(),
            strategy: strategy_label(pool.strategy.as_deref()).to_string(),
            enabled: pool.enabled.unwrap_or(true),
            resource_count: dashboard.resource_count,
            healthy_resource_count: dashboard.healthy_resource_count,
            model_count: dashboard.model_count,
            requests: dashboard.traffic.total_requests,
            tokens: dashboard.traffic.total_tokens,
            providers: pool_providers,
        });
    }
    pools.sort_by(|left, right| {
        right
            .enabled
            .cmp(&left.enabled)
            .then_with(|| left.name.cmp(&right.name))
    });

    let today = load_token_stats(state, TokenRange::Today);
    let seven_days = load_token_stats(state, TokenRange::SevenDays);
    let month = load_token_stats(state, TokenRange::Month);
    let today_stats = state
        .db
        .logs
        .get_stats_range(&state.db.conn, Some("today"))?;
    let cumulative = state.db.logs.get_stats(&state.db.conn)?;
    let activity = state.db.logs.get_tray_activity(&state.db.conn)?;
    let tokens_heatmap = state.db.logs.get_daily_token_series(&state.db.conn)?;
    let current_hour = Local::now()
        .format("%H")
        .to_string()
        .parse::<i64>()
        .unwrap_or(0);
    let current_tps = activity
        .iter()
        .find(|point| point.hour == current_hour)
        .map(|point| point.requests as f64 / 3600.0)
        .unwrap_or(0.0);
    let success_rate = if today_stats.total_requests > 0 {
        today_stats.success_count as f64 / today_stats.total_requests as f64 * 100.0
    } else {
        0.0
    };
    let (gateway_running, port) = state
        .proxy
        .lock()
        .map(|proxy| {
            proxy
                .as_ref()
                .map(|handle| (true, handle.port))
                .unwrap_or((false, DEFAULT_PROXY_PORT))
        })
        .unwrap_or((false, DEFAULT_PROXY_PORT));

    let most_active_path = state.gateway_runtime.active_paths().into_iter().next();
    let active_route_count = most_active_path
        .as_ref()
        .map(|path| path.active_requests)
        .unwrap_or(0);
    let active_route_source = most_active_path
        .map(|path| crate::proxy::runtime::RuntimeRoutePath {
            request_id: String::new(),
            protocol: path.protocol,
            pool_id: path.pool_id,
            provider_id: path.provider_id,
            account_id: String::new(),
            status: "active".into(),
            attempt: 1,
            latency_ms: None,
            started_at: path.last_active_at.clone(),
            updated_at: path.last_active_at,
        })
        .or_else(|| state.gateway_runtime.latest_route());
    let active_route = active_route_source.map(|route| {
        let pool_name = pools
            .iter()
            .find(|pool| pool.id == route.pool_id)
            .map(|pool| pool.name.clone())
            .unwrap_or_else(|| route.pool_id.clone());
        let provider_name = providers
            .iter()
            .find(|provider| provider.id == route.provider_id)
            .map(|provider| provider.name.clone())
            .unwrap_or_else(|| route.provider_id.clone());
        TrayActiveRoute {
            protocol: protocol_label(&route.protocol).to_string(),
            pool_id: route.pool_id,
            pool_name,
            provider_id: route.provider_id,
            provider_name,
            status: route.status,
            attempt: route.attempt,
            latency_ms: route.latency_ms,
            active_requests: active_route_count,
            updated_at: route.updated_at,
        }
    });

    let warning_count = pools
        .iter()
        .filter(|pool| pool.enabled && pool.healthy_resource_count < pool.resource_count)
        .count();
    let fault_count = pools
        .iter()
        .filter(|pool| pool.enabled && pool.healthy_resource_count == 0)
        .count();

    Ok(TraySnapshot {
        gateway_running,
        port,
        active_connections: state.gateway_runtime.active_connections(),
        topology: TrayTopologySummary {
            protocol_count,
            pool_count: pools.len(),
            provider_count: referenced_provider_ids.len(),
            warning_count,
            fault_count,
        },
        resources_total,
        resources_available,
        resources_limited,
        enabled_pool_count,
        today_tokens: today.total_tokens,
        seven_day_tokens: seven_days.total_tokens,
        month_tokens: month.total_tokens,
        cumulative_tokens: cumulative.total_tokens,
        total_requests: today_stats.total_requests,
        success_rate,
        current_tps,
        activity,
        tokens_heatmap,
        pools,
        active_route,
        updated_at: Local::now().format("%H:%M:%S").to_string(),
    })
}

fn copy_config<R: Runtime>(app: &AppHandle<R>, config: &str, label: &str) {
    if let Err(error) = copy_to_clipboard(app, config) {
        tracing::error!("Failed to copy {} config: {}", label, error);
    } else {
        tracing::info!("Tray: {} config copied", label);
    }
}

fn copy_to_clipboard<R: Runtime>(app: &AppHandle<R>, text: &str) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .write_text(text.to_string())
        .map_err(|error| error.to_string())
}

fn generate_claude_config() -> String {
    r#"{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:9800",
    "ANTHROPIC_AUTH_TOKEN": "pg_live_请替换为号池专属Key",
    "ANTHROPIC_MODEL": "请填写号池内的Anthropic模型ID"
  }
}"#
    .to_string()
}

fn generate_codex_config() -> String {
    r#"model = "请填写号池内支持Responses的模型ID"
model_provider = "poolgate"

[model_providers.poolgate]
name = "PoolGate"
base_url = "http://127.0.0.1:9800/v1"
env_key = "POOLGATE_API_KEY"
wire_api = "responses""#
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_ranges_map_to_stable_menu_ids() {
        for range in TokenRange::all() {
            assert_eq!(TokenRange::from_menu_id(range.menu_id()), Some(range));
        }
        assert_eq!(TokenRange::from_menu_id("unknown"), None);
    }

    #[test]
    fn token_format_is_compact_and_stable() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_250), "1.2K");
        assert_eq!(format_tokens(2_500_000), "2.50M");
        assert_eq!(format_tokens(3_000_000_000), "3.00B");
    }
}
