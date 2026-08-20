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

pub(crate) const TRAY_ID: &str = "poolgate-main-tray";
pub(crate) const TRAY_WINDOW_LABEL: &str = "tray-card";
// 设计规范：固定 380×720，禁止自适应/拖拽缩放（min=max）。
const TRAY_WINDOW_WIDTH: f64 = 380.0;
const TRAY_WINDOW_HEIGHT: f64 = 720.0;
const TRAY_WINDOW_MIN_WIDTH: f64 = TRAY_WINDOW_WIDTH;
const TRAY_WINDOW_MAX_WIDTH: f64 = TRAY_WINDOW_WIDTH;
const TRAY_WINDOW_MIN_HEIGHT: f64 = TRAY_WINDOW_HEIGHT;
const TRAY_WINDOW_MAX_HEIGHT: f64 = TRAY_WINDOW_HEIGHT;
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

    // 原生右键菜单（启动/停止网关、路由池、Tokens 统计、退出等），按启动时状态构建；
    // 之后由 refresh_tray_menu 每 10s 重建以保持实时。左键不弹菜单，交给点击事件。
    let menu = build_tray_menu(&app.handle().clone(), TokenRange::Today)?;
    // 初始图标：macOS 用深色单色云朵-P（`LOGO_COLOR`）剪影 + 状态点（不启用
    // NSImage 模板模式，否则彩色状态点会被染成黑白）；其他平台用应用图标。
    // 启动后 apply_menu_bar 会按实时状态持续刷新图标与标题。
    #[cfg(target_os = "macos")]
    let initial_icon = {
        use super::menu_bar::{render_status_icon, Appearance, MenuBarItem, Tone};
        let initial_item = MenuBarItem {
            tone: Tone::Offline, // 启动瞬间网关未运行；后续刷新会校正为真实状态
            text: String::new(),
            tooltip: String::new(),
        };
        // 启动瞬间可能没有可见 webview 能读取系统外观，按当前系统的
        // `app.default_window_icon` 上下文推断不出——退化为 Light，
        // 第一个 10s 刷新循环会自动校正为真实外观。
        render_status_icon(&initial_item, app.default_window_icon(), Appearance::Light)
    };
    #[cfg(not(target_os = "macos"))]
    let initial_icon = app.default_window_icon().cloned().unwrap();
    TrayIconBuilder::with_id(TRAY_ID)
        .show_menu_on_left_click(false)
        .menu(&menu)
        .tooltip("PoolGate · 本地模型网关")
        .icon(initial_icon)
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
                // 左键打开/关闭托盘面板；右键由系统弹出原生菜单（启动/停止网关等）
                if button == MouseButton::Left {
                    if let Err(error) =
                        toggle_tray_window(tray.app_handle(), position.x, position.y)
                    {
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

    // 启动后立即按设置渲染菜单栏状态胶囊（10s 刷新循环与额度循环会持续更新）
    {
        let state = app.state::<Arc<AppState>>().inner().clone();
        let _ = super::menu_bar::apply_menu_bar(&app.handle().clone(), &state);
    }

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
        // 局域网访问地址：点击任一 IP 即复制完整入口（http://<ip>:9800）。
        id if id.starts_with("copy_lan_") => {
            let ip = &id["copy_lan_".len()..];
            copy_config(
                app,
                &format!("http://{}:{}", ip, DEFAULT_PROXY_PORT),
                "局域网访问地址",
            );
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

fn build_monitor_tray_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(
        app,
        "monitor_status",
        "PoolGate Monitor · 仅 Token Monitor",
        false,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "refresh_tray",
        "刷新监控数据",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "show_tray_card",
        "显示 Monitor 托盘",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "show_window",
        "打开 Monitor 仪表盘",
        true,
        None::<&str>,
    )?)?;
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

fn build_tray_menu<R: Runtime>(app: &AppHandle<R>, range: TokenRange) -> tauri::Result<Menu<R>> {
    let state = app.state::<Arc<AppState>>();
    if crate::commands::settings_commands::is_monitor_mode(state.inner()).unwrap_or(false) {
        return build_monitor_tray_menu(app);
    }
    let running = proxy_is_running(state.inner());
    let listen_host = listen_host(state.inner());
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
            "PoolGate · {} · {}:{}",
            if running {
                "网关运行中"
            } else {
                "网关已关闭"
            },
            listen_host,
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

    // 实时状态行：最近 5 分钟网关流量 + 活跃会话数（每 10s 随菜单重建刷新）。
    // 数据源为 request_logs（只统计走网关的请求），标注「网关流量」与电脑整体用量区分。
    let (recent_requests, recent_tokens) = recent_traffic(state.inner());
    let active_sessions = active_sessions_count(state.inner());
    menu.append(&MenuItem::with_id(
        app,
        "live_recent_traffic",
        format!(
            "网关流量 · 最近 5 分钟 · {} 请求 · {} Tokens",
            recent_requests,
            format_tokens(recent_tokens)
        ),
        false,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "live_active_sessions",
        format!("活跃会话 · {active_sessions}"),
        false,
        None::<&str>,
    )?)?;
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

    // Tokens 统计子菜单：所有数字均来自 request_logs（走网关的请求），
    // 标题标注「网关」以区别于电脑整体 Tokens；子菜单内条目沿用其作用域。
    let token_menu = Submenu::with_id(
        app,
        "token_stats",
        format!(
            "网关 Tokens 统计 · {} · {}",
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

    // 局域网访问地址：LAN 监听时列出本机 IP，点击一键复制完整入口供同事配置；
    // 仅本机监听时给出提示（开启入口在设置页「代理 → 监听地址」）。菜单每 10s
    // 重建，IP 变化会自动跟随。
    let lan_menu = Submenu::with_id(app, "lan_addresses", "局域网访问地址", true)?;
    if listen_host == "0.0.0.0" {
        match crate::commands::proxy_commands::lan_ipv4_addresses() {
            Ok(ips) if !ips.is_empty() => {
                for ip in ips {
                    lan_menu.append(&MenuItem::with_id(
                        app,
                        format!("copy_lan_{}", ip),
                        format!("复制 http://{}:{}", ip, DEFAULT_PROXY_PORT),
                        true,
                        None::<&str>,
                    )?)?;
                }
            }
            _ => {
                lan_menu.append(&MenuItem::with_id(
                    app,
                    "lan_addresses_empty",
                    "未检测到局域网地址",
                    false,
                    None::<&str>,
                )?)?;
            }
        }
    } else {
        lan_menu.append(&MenuItem::with_id(
            app,
            "lan_addresses_localhost_only",
            "仅本机监听 · 设置中可开启局域网共享",
            false,
            None::<&str>,
        )?)?;
    }
    menu.append(&lan_menu)?;
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

fn refresh_tray_menu<R: Runtime>(app: &AppHandle<R>, range: TokenRange) -> Result<(), String> {
    let state = app.state::<Arc<AppState>>().inner().clone();
    // 原生右键菜单按实时状态重建（网关运行/停止、路由池、Tokens 统计、退出等）。
    // 菜单重建与图标应用都在主线程执行（NSMenu / NSStatusItem 不允许非主线程修改）。
    let menu = build_tray_menu(app, range).ok();
    let app_for_menu = app.clone();
    app.run_on_main_thread(move || {
        if let (Some(tray), Some(menu)) = (app_for_menu.tray_by_id(TRAY_ID), menu) {
            if let Err(error) = tray.set_menu(Some(menu)) {
                tracing::warn!("tray menu rebuild failed: {error}");
            }
        }
    })
    .map_err(|e| e.to_string())?;
    // 菜单栏状态胶囊（图标 + 标题 + tooltip）由 menu_bar 统一渲染（单一图标：
    // Logo + 状态点 + 主文本）。
    super::menu_bar::apply_menu_bar(app, &state)
}

fn proxy_is_running(state: &Arc<AppState>) -> bool {
    state
        .proxy
        .lock()
        .map(|proxy| proxy.is_some())
        .unwrap_or(false)
}

/// Host the gateway binds to (127.0.0.1 in localhost mode, 0.0.0.0 in LAN
/// mode). When no server is running, falls back to the configured mode so the
/// tray shows the address the gateway would bind to once started.
fn listen_host(state: &Arc<AppState>) -> &'static str {
    state
        .proxy
        .lock()
        .map(|proxy| {
            proxy
                .as_ref()
                .map(|handle| handle.listen_mode.bind_host())
                .unwrap_or_else(|| {
                    let mode = crate::commands::settings_commands::get_listen_addr(state)
                        .unwrap_or_else(|_| "localhost".to_string());
                    crate::proxy::server::ListenMode::from_setting(&mode).bind_host()
                })
        })
        .unwrap_or("127.0.0.1")
}

/// 最近 5 分钟请求数与 Tokens（request_logs 聚合，用于右键菜单实时行）。
/// 非精确审计口径（含全部尝试行），仅作实时流量指示。
fn recent_traffic(state: &Arc<AppState>) -> (i64, i64) {
    let Ok(conn) = state.db.conn.lock() else {
        return (0, 0);
    };
    conn.query_row(
        "SELECT COUNT(*), \
         COALESCE(SUM(COALESCE(input_tokens,0) + COALESCE(output_tokens,0) + COALESCE(cache_tokens,0)), 0) \
         FROM request_logs WHERE request_at >= datetime('now', '-5 minutes')",
        [],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )
    .unwrap_or((0, 0))
}

/// 活跃会话数：`tm_session` 中最近 5 分钟仍有活跃的会话（token-monitor 采集口径）；
/// 采集未运行/表缺失时返回 0。
fn active_sessions_count(state: &Arc<AppState>) -> i64 {
    let Ok(conn) = state.db.conn.lock() else {
        return 0;
    };
    conn.query_row(
        "SELECT COUNT(*) FROM tm_session \
         WHERE datetime(last_active_at, 'localtime') >= datetime('now', 'localtime', '-5 minutes')",
        [],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
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

pub(crate) fn toggle_tray_window<R: Runtime>(
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
            WebviewUrl::App(
                if crate::commands::settings_commands::is_monitor_mode(
                    &app.state::<Arc<AppState>>().inner().clone(),
                )
                .unwrap_or(false)
                {
                    "index.html?view=token-monitor"
                } else {
                    "index.html?view=tray"
                }
                .into(),
            ),
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
        // 页面加载完成后广播一次外观主题事件（此时前端监听器已就绪，不会丢失）。
        // 事件载荷来自 settings 表（Rust 可读的镜像），让新加载的 webview 尽快应用
        // 持久化主题，消除首次打开瞬间的默认浅色闪烁；localStorage 中的显式选择优先，
        // 事件仅作兑底校正（见前端 ThemeProvider）。
        .on_page_load(|window, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let state = window.app_handle().state::<Arc<AppState>>().inner().clone();
                match resolve_appearance_theme(&state, &window) {
                    Ok((preference, theme)) => {
                        use crate::commands::settings_commands::ACCOUNT_DISPLAY;
                        // 账号脱敏/全展示偏好也随主题事件广播：新加载的托盘 webview
                        // 在无本地显式选择时按 settings 表兜底（前端 localStorage 优先）。
                        let account_display = state
                            .db
                            .settings
                            .get(&state.db.conn, ACCOUNT_DISPLAY)
                            .ok()
                            .flatten();
                        let mut payload = serde_json::json!({
                            "theme": theme,
                            "preference": preference,
                        });
                        if let Some(display) = account_display {
                            payload["accountDisplay"] = serde_json::Value::String(display);
                        }
                        let _ = window.app_handle().emit_to(
                            TRAY_WINDOW_LABEL,
                            "appearance:theme",
                            payload,
                        );
                    }
                    Err(error) => {
                        tracing::warn!("Unable to resolve tray appearance theme: {}", error)
                    }
                }
            }
        })
        .build()
        .map_err(|error| error.to_string())?;

        // Liquid Glass 毛玻璃：透明窗口下唯有原生 vibrancy 能磨砂窗口背后的桌面。
        // macOS 用 HudWindow 材质 + 28px 圆角，Windows 用 Acrylic。失败时静默降级为
        // 纯 CSS 半透明外壳（不影响功能）。
        #[cfg(target_os = "macos")]
        {
            use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
            let _ = apply_vibrancy(
                &window,
                NSVisualEffectMaterial::HudWindow,
                Some(NSVisualEffectState::Active),
                Some(28.0),
            );
        }
        #[cfg(target_os = "windows")]
        {
            // Acrylic tint 跟随主题：优先读 settings 表中 Rust 可读的偏好（浅色/深色/
            // 随系统），玻璃不透明度自定义时同步映射到原生 alpha。读取失败时回退原浅色。
            let state = app.state::<Arc<AppState>>().inner().clone();
            match tray_acrylic_tint(&state, &window) {
                Ok(tint) => {
                    use window_vibrancy::apply_acrylic;
                    let _ = apply_acrylic(&window, Some(tint));
                }
                Err(error) => tracing::warn!("Unable to resolve tray acrylic tint: {}", error),
            }
        }

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
    tab: Option<String>,
) {
    if let Some(window) = app.get_webview_window(TRAY_WINDOW_LABEL) {
        let _ = window.hide();
    }
    show_main_window(&app);
    if let Some(page) = page {
        let payload = if let Some(node) = node.filter(|node| !node.is_empty()) {
            format!("{}?node={}", page, node)
        } else if let Some(tab) = tab.filter(|tab| !tab.is_empty()) {
            format!("{}?tab={}", page, tab)
        } else {
            page
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

/// 从 settings 解析外观偏好：(preference, resolved_theme)。"system" 用窗口系统主题
/// 解析（Windows/macOS 上 `window.theme()` 即系统主题；Linux 回退浅色，前端
/// matchMedia 仍主导 CSS）。
fn resolve_appearance_theme<R: Runtime>(
    state: &Arc<AppState>,
    window: &tauri::WebviewWindow<R>,
) -> Result<(String, String), String> {
    use crate::commands::settings_commands::THEME_PREFERENCE;
    let preference = state
        .db
        .settings
        .get(&state.db.conn, THEME_PREFERENCE)?
        .unwrap_or_else(|| "system".to_string());
    let resolved = match preference.as_str() {
        "light" => "light",
        "dark" => "dark",
        _ => match window.theme().map_err(|error| error.to_string())? {
            tauri::Theme::Dark => "dark",
            _ => "light",
        },
    };
    Ok((preference, resolved.to_string()))
}

/// Windows: 按 settings 中的主题/玻璃偏好重设托盘窗口的原生 Acrylic tint。
/// 托盘窗口尚未创建时静默返回（创建分支会自行读取 settings）。
#[cfg(target_os = "windows")]
pub(crate) fn apply_tray_acrylic_from_settings<R: Runtime>(
    app: &AppHandle<R>,
    state: &Arc<AppState>,
) -> Result<(), String> {
    let Some(window) = app.get_webview_window(TRAY_WINDOW_LABEL) else {
        return Ok(());
    };
    let tint = tray_acrylic_tint(state, &window)?;
    use window_vibrancy::apply_acrylic;
    apply_acrylic(&window, Some(tint)).map_err(|error| error.to_string())
}

/// 解析托盘窗口的 Acrylic tint：主题决定色调（深色 26,30,36 / 浅色 246,248,252），
/// 用户自定义过玻璃不透明度时原生 alpha 跟随（0-100 → 0-255），否则使用低透明度
/// Liquid Glass 默认（深 120 / 浅 112）。
#[cfg(target_os = "windows")]
fn tray_acrylic_tint<R: Runtime>(
    state: &Arc<AppState>,
    window: &tauri::WebviewWindow<R>,
) -> Result<(u8, u8, u8, u8), String> {
    use crate::commands::settings_commands::GLASS_OPACITY;
    let (_preference, theme) = resolve_appearance_theme(state, window)?;
    let dark = theme == "dark";
    let (r, g, b, default_alpha) = if dark {
        (26, 30, 36, 120)
    } else {
        (246, 248, 252, 112)
    };
    let alpha = match state.db.settings.get(&state.db.conn, GLASS_OPACITY)? {
        Some(raw) => raw
            .parse::<f64>()
            .ok()
            .map(|value| (value.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8)
            .unwrap_or(default_alpha),
        None => default_alpha,
    };
    Ok((r, g, b, alpha))
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
