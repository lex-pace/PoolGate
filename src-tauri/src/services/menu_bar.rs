//! macOS 菜单栏 / Windows 托盘状态图标（PoolGate 菜单栏设计规范）。
//!
//! macOS：菜单栏 Logo 颜色随系统外观切换——
//! **浅色和深色菜单栏都使用白色实心方块**，与系统菜单栏里的其它图标保持一致；
//! 两种外观的 P 与符号都保持透明镂空，直接露出菜单栏背景。
//! 也**不在离线时降半透明**。右侧保留**彩色状态点 + 状态色光晕**表达
//! 综合状态（绿/蓝/橙/红）。不启用 NSImage 模板模式：模板会把整图当
//! 蒙版染成黑白，彩色状态点也会丢，这里按系统外观预先着色再普通模式
//! 应用。状态与数字同时在系统原生标题（`set_title`，颜色随菜单栏深浅
//! 自适应）与 tooltip 中：
//! - 🟢 正常运行：状态点绿，tooltip「PoolGate · 在线 · 今日 240,408 Tokens」
//! - 🔵 流量活跃：近 5 分钟有流量增长，状态点蓝
//! - 🟠 轻度告警：Token 用量接近额度上限（剩余 <15%），状态点橙
//! - 🔴 网关离线：网关未运行，状态点红，tooltip「PoolGate · 网关离线」
//!
//! Windows/Linux：通知区保持真实彩色应用 Logo + 右下角状态点（按**综合严重度**
//! 着色：网关离线（红）> 额度告警/耗尽（橙）> 流量活跃（蓝）> 正常（绿））；
//! 文本不支持标题，状态与数字均在 tooltip 中。
//!
//! 主文本（今日 Tokens / Top1 工具 / Top1 模型）由系统原生渲染，额度剩余
//! 百分比（`92%`）在 tooltip 中。
//!
//! 设置 `pg.menuBarMainText`（`settings_commands::MENU_BAR_MAIN_TEXT`）决定主文本，
//! 三选一固定展示（不轮播）：`tokens`（默认）= 今日 Tokens 简写（`今日 240K`）；
//! `top1_tool` = 今日用量第一的工具名（`工具 Claude Code`）；`top1_model` = 今日
//! 用量第一的模型名（`模型 claude-sonnet-4…`）。标签说明该数字的含义，名称超长
//! 自动截断。完整信息（状态 + 今日 Tokens + Top1 工具/模型 + 额度百分比）始终在
//! tooltip 中。

use std::sync::Arc;

use tauri::image::Image;
use tauri::{AppHandle, Manager, Runtime};

use crate::AppState;

/// 旧版「第二个托盘」（网关状态图标）的 ID，仅用于启动时清理历史残留。
const TRAY_GATEWAY_ID: &str = "poolgate-gateway-tray";

/// 状态色调（与托盘卡片/仪表盘一致的语义色）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// 正常（绿）
    Online,
    /// 流量活跃（蓝）
    Active,
    /// 额度告警（橙）
    QuotaWarn,
    /// 离线 / 额度耗尽（红）
    Offline,
}

impl Tone {
    fn rgb(self) -> [u8; 3] {
        match self {
            Tone::Online => [52, 199, 89],    // #34c759
            Tone::Active => [10, 132, 255],   // #0a84ff
            Tone::QuotaWarn => [255, 149, 0], // #ff9500
            Tone::Offline => [255, 69, 58],   // #ff453a
        }
    }
}

/// 单个菜单栏状态项：色调 + 标题文本 + 悬浮 tooltip。文本在 macOS 上由系统
/// 原生渲染（带含义标签：`今日 240K` / `工具 Claude Code` / `模型 claude-sonnet-4…`）。
pub struct MenuBarItem {
    pub tone: Tone,
    pub text: String,
    pub tooltip: String,
}

/// 菜单栏主文本模式（`settings_commands::MENU_BAR_MAIN_TEXT`）：三选一固定展示，不轮播。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MainText {
    /// 今日 Tokens 简写（`今日 240K`，默认）
    Tokens,
    /// 今日用量第一的工具名（`工具 Claude Code`）
    Top1Tool,
    /// 今日用量第一的模型名（`模型 claude-sonnet-4…`）
    Top1Model,
}

impl MainText {
    fn from_setting(s: &str) -> MainText {
        match s {
            "top1_tool" => MainText::Top1Tool,
            "top1_model" => MainText::Top1Model,
            _ => MainText::Tokens,
        }
    }
}

// ───────────────────────── 状态计算 ─────────────────────────

/// 今日 Tokens（统一口径，与额度刷新循环的托盘主指标一致）。
fn today_tokens(state: &Arc<AppState>) -> i64 {
    state
        .db
        .usage_events
        .unified_range_stats(&state.db.conn, "day")
        .map(|(_, _, _, total)| total)
        .unwrap_or(0)
}

/// 今日 Top1 工具名（按今日 total_tokens 降序第一；无数据返回 None）。
fn top1_tool(state: &Arc<AppState>) -> Option<String> {
    let rows = state
        .db
        .usage_events
        .tool_usage_rows(&state.db.conn, "day")
        .unwrap_or_default();
    rows.first().map(|row| {
        if row.display_name.is_empty() {
            row.tool_id.clone()
        } else {
            row.display_name.clone()
        }
    })
}

/// 今日 Top1 模型名（按今日 total_tokens 降序第一；无数据返回 None）。
fn top1_model(state: &Arc<AppState>) -> Option<String> {
    let rows = state
        .db
        .usage_events
        .model_usage_rows(&state.db.conn, "day")
        .unwrap_or_default();
    rows.first().map(|row| row.model.clone())
}

/// 网关是否在运行。
fn gateway_running(state: &Arc<AppState>) -> bool {
    state
        .proxy
        .lock()
        .map(|proxy| proxy.is_some())
        .unwrap_or(false)
}

/// 近 5 分钟是否有网关请求（「流量活跃」判定：较最近 5 分钟有增长）。
fn recent_activity(state: &Arc<AppState>) -> bool {
    let Ok(conn) = state.db.conn.lock() else {
        return false;
    };
    conn.query_row(
        "SELECT COUNT(*) FROM request_logs WHERE request_at >= datetime('now', '-5 minutes')",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .unwrap_or(false)
}

/// 最受约束的额度窗口剩余百分比（无额度数据返回 None）。
fn worst_quota_percent(state: &Arc<AppState>) -> Option<f64> {
    let windows = state
        .db
        .quota_windows
        .list_all_snapshots(&state.db.conn)
        .unwrap_or_default();
    windows
        .iter()
        .filter_map(|w| w.remaining_percent)
        .fold(None, |best: Option<f64>, pct| {
            Some(best.map_or(pct, |b| b.min(pct)))
        })
}

/// 额度阈值 → 色调：<5% 红（耗尽）、<15% 橙（偏低）、其余绿。
fn quota_tone(percent: f64) -> Tone {
    if percent < 5.0 {
        Tone::Offline
    } else if percent < 15.0 {
        Tone::QuotaWarn
    } else {
        Tone::Online
    }
}

/// 主文本总字符预算（含标签）：标签 2 字 + 空格，名称最多再取 `MAX_TEXT_CHARS - 3`
/// 个字符，整体不超过菜单栏可接受宽度。
const MAX_TEXT_CHARS: usize = 16;

/// 主文本标签长度（「今日 / 工具 / 模型」+ 空格），名称截断预算随之收缩。
const LABEL_CHARS: usize = 3;

/// 主文本内容（带含义标签，避免菜单栏里一串裸数字/裸名）：
/// - tokens → `今日 240K`（与设计稿「今日 Tokens」一致）；
/// - top1_tool → `工具 Claude Code`；top1_model → `模型 claude-sonnet-4…`
///   （名称超长截断，避免撑爆菜单栏）。
/// 无 Top1 数据时回退到 `今日 {compact}`（换用「今日 」标签，避免出现
/// 「工具 240.4K」这类标签与内容不符的文案）。
fn main_text_value(
    main: MainText,
    today: i64,
    top1_tool: Option<&str>,
    top1_model: Option<&str>,
) -> String {
    let fallback = format!("今日 {}", compact_tokens(today));
    let name_max = MAX_TEXT_CHARS - LABEL_CHARS;
    match main {
        MainText::Tokens => fallback,
        MainText::Top1Tool => match top1_tool.filter(|s| !s.is_empty()) {
            Some(name) => format!("工具 {}", truncate_display(name, name_max)),
            None => fallback,
        },
        MainText::Top1Model => match top1_model.filter(|s| !s.is_empty()) {
            Some(name) => format!("模型 {}", truncate_display(name, name_max)),
            None => fallback,
        },
    }
}

/// tooltip：状态 + 今日 Tokens 完整数 + （有数据时）Top1 工具/模型，任何主文本
/// 模式下都给出完整信息。
fn build_tooltip(
    state_label: &str,
    today: i64,
    top1_tool: Option<&str>,
    top1_model: Option<&str>,
) -> String {
    let mut tip = format!(
        "PoolGate · {state_label} · 今日 {} Tokens",
        with_commas(today)
    );
    if let Some(name) = top1_tool.filter(|s| !s.is_empty()) {
        tip.push_str(&format!(" · Top1 工具 {name}"));
    }
    if let Some(name) = top1_model.filter(|s| !s.is_empty()) {
        tip.push_str(&format!(" · Top1 模型 {name}"));
    }
    tip
}

/// 计算单一状态项（Logo + 状态点 + 主文本）：状态点按**综合严重度**着色——
/// 网关离线（红）> 额度告警/耗尽（橙）> 流量活跃（蓝）> 正常（绿）。主文本恒
/// 显示（今日 / 工具 / 模型），额度剩余百分比与完整信息放 tooltip。
fn compute_combined_item(
    state: &Arc<AppState>,
    main: MainText,
    today: i64,
    top1_tool: Option<&str>,
    top1_model: Option<&str>,
) -> MenuBarItem {
    let main_text = main_text_value(main, today, top1_tool, top1_model);
    if crate::commands::settings_commands::is_monitor_mode(state).unwrap_or(false) {
        let quota_pct = worst_quota_percent(state);
        let tone = quota_pct.map(quota_tone).unwrap_or(Tone::Online);
        let mut tooltip = build_tooltip("Monitor 正常", today, top1_tool, top1_model);
        if let Some(pct) = quota_pct {
            tooltip = format!("额度 {pct:.0}% · {tooltip}");
        }
        return MenuBarItem {
            tone,
            text: main_text,
            tooltip: tooltip.replace("PoolGate ·", "PoolGate Monitor ·"),
        };
    }

    let running = gateway_running(state);
    let active = running && recent_activity(state);

    let quota_pct = worst_quota_percent(state);
    let (tone, state_label) = combined_status(running, active, quota_pct);

    let mut tooltip = build_tooltip(state_label, today, top1_tool, top1_model);
    if let Some(pct) = quota_pct {
        tooltip = format!("额度 {pct:.0}% · {tooltip}");
    }

    MenuBarItem {
        tone,
        text: main_text,
        tooltip,
    }
}

/// 综合严重度：**网关离线（红）> 额度告警/耗尽（橙）> 流量活跃（蓝）> 正常（绿）**。
/// 返回（状态点色调, 状态文案）。额度耗尽（<5%）与告警（<15%）同为橙色点，文案
/// 区分「额度耗尽 / 额度告警」。
fn combined_status(running: bool, active: bool, quota_pct: Option<f64>) -> (Tone, &'static str) {
    let quota_tone_opt = quota_pct.map(quota_tone);
    let quota_bad = matches!(quota_tone_opt, Some(Tone::QuotaWarn | Tone::Offline));
    if !running {
        (Tone::Offline, "网关离线")
    } else if quota_bad {
        let label = if quota_tone_opt == Some(Tone::Offline) {
            "额度耗尽"
        } else {
            "额度告警"
        };
        (Tone::QuotaWarn, label)
    } else if active {
        (Tone::Active, "流量活跃")
    } else {
        (Tone::Online, "在线")
    }
}

/// 按设置把当前状态应用到托盘图标（单一图标：Logo + 状态点 + 主文本）。
///
/// 线程安全：状态计算（DB 读取、图标渲染）在当前线程完成，**托盘图标的创建/移除/
/// 图标更新全部派发到主线程执行**（`run_on_main_thread`，主线程调用时同步内联）。
/// macOS 上 `NSStatusItem` 的移除必须发生在主线程——从 tokio 工作线程直接移除会在
/// Drop 时触发 `removeStatusItem` 崩溃（EXC_BREAKPOINT / assertBarrierOnQueue）。
///
/// 菜单栏外观随系统自动适配：本函数每次调用都会从 `app.get_webview_window("main")`
/// 的 `theme()` 读取当前系统外观（macOS 上即 `NSApp.effectiveAppearance`），
/// 浅色菜单栏下渲染深色 Logo，深色菜单栏下渲染近白 Logo；并把 LogoMode
/// 切到 Silhouette 让深色菜单栏下的形态与系统模板图标一致（Wi-Fi /
/// Battery / Clock）。主线程事件 `WindowEvent::ThemeChanged` 会触发本函数
/// 立即重绘，不必等 10s 刷新循环。
pub fn apply_menu_bar<R: Runtime>(app: &AppHandle<R>, state: &Arc<AppState>) -> Result<(), String> {
    let appearance = Appearance::resolve(app);
    apply_menu_bar_with_appearance(app, state, appearance)
}

/// 同 [`apply_menu_bar`]，但调用方直接传入已确定的外观。`WindowEvent::ThemeChanged`
/// 处理器优先使用此变体——从事件载荷拿到的主题比再读 `window.theme()` 更稳，
/// 且主题切换瞬间不会因窗口未被查询到而恢复到默认 Light。
pub fn apply_menu_bar_with_appearance<R: Runtime>(
    app: &AppHandle<R>,
    state: &Arc<AppState>,
    appearance: Appearance,
) -> Result<(), String> {
    let main_setting = state
        .db
        .settings
        .get(
            &state.db.conn,
            crate::commands::settings_commands::MENU_BAR_MAIN_TEXT,
        )?
        .unwrap_or_else(|| "tokens".to_string());
    let main = MainText::from_setting(&main_setting);
    let today = today_tokens(state);
    let top1_tool_name = top1_tool(state);
    let top1_model_name = top1_model(state);
    let item = compute_combined_item(
        state,
        main,
        today,
        top1_tool_name.as_deref(),
        top1_model_name.as_deref(),
    );

    // 每次渲染重新解析系统外观：macOS 菜单栏深浅随 NSAppearance 切换，
    // 浅色菜单栏 → 深色 Logo（含 Cutout），深色菜单栏 → 近白 Logo（Silhouette）。
    // 不缓存 appearance —— `WindowEvent::ThemeChanged` 会立即重新调用本函数。
    let items: Vec<(String, MenuBarItem, Appearance)> =
        vec![(crate::services::tray::TRAY_ID.to_string(), item, appearance)];

    let app = app.clone();
    let task_app = app.clone();
    app.run_on_main_thread(move || {
        if let Err(error) = apply_items_main_thread(&task_app, &items) {
            tracing::warn!("menu bar apply failed: {error}");
        }
    })
    .map_err(|e| e.to_string())
}

/// 在主线程上应用托盘状态：清理历史残留的网关托盘，然后设置图标、标题与
/// tooltip。TrayIcon 的创建与 Drop 都发生在主线程，避免 macOS 非主线程
/// `removeStatusItem` 崩溃。
fn apply_items_main_thread<R: Runtime>(
    app: &AppHandle<R>,
    items: &[(String, MenuBarItem, Appearance)],
) -> Result<(), String> {
    // 旧版 separate 模式创建过第二个托盘（网关状态点）；统一为单一图标后移除
    // 残留，避免菜单栏出现多余的状态点（返回值在主线程 Drop，原生移除安全）。
    let _ = app.remove_tray_by_id(TRAY_GATEWAY_ID);
    let app_icon = app.default_window_icon().map(|icon| icon.to_owned());
    for (id, item, appearance) in items {
        let Some(tray) = app.tray_by_id(id.as_str()) else {
            continue;
        };
        // macOS：按 appearance 选择主色与 LogoMode（浅色 → 深色 + Cutout，
        // 深色 → 近白 + Silhouette），普通模式应用（不启用 NSImage template——
        // 模板模式会把整图染成黑白，状态点彩色就丢了）。
        #[cfg(target_os = "macos")]
        let image = render_status_icon(item, app_icon.as_ref(), *appearance);
        #[cfg(not(target_os = "macos"))]
        let image = render_tray_icon(item.tone, app_icon.as_ref());
        tray.set_icon(Some(image)).map_err(|e| e.to_string())?;
        // macOS：主文本交给系统原生渲染（`NSStatusItem.button.title`）——系统字体
        // SF Pro、原生抗锯齿、颜色随菜单栏深浅自动适配（深色菜单栏 → 白色文字），
        // 与开源 Token Monitor 一致；Windows 托盘不支持标题，文本在 tooltip。
        #[cfg(target_os = "macos")]
        tray.set_title(Some(item.text.clone()))
            .map_err(|e| e.to_string())?;
        tray.set_tooltip(Some(item.tooltip.clone()))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ───────────────────────── 文本与数字格式化 ─────────────────────────

/// 超长文本截断：字符数 ≤ max 原样返回；否则保留前 max-1 字符并追加省略号。
fn truncate_display(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let mut out: String = chars[..max - 1].iter().collect();
    out.push('…');
    out
}

/// 紧凑 Tokens（240K / 1.2M / 3.00B），用于 tooltip 与状态计算。
fn compact_tokens(value: i64) -> String {
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

/// 千分位整数（tooltip「今日 240,408 Tokens」）。
fn with_commas(value: i64) -> String {
    let digits = value.max(0).to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(bytes.len() + 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

// ───────────────────────── 图标渲染（纯 RGBA） ─────────────────────────

/// **渲染源像素倍率**：在 RGBA 缓冲区层把画布放大到 @2x 像素密度，所有坐标
/// 同步放大（NSImage.size 仍由 tray-icon 强制为 18pt 高，物理形状不变）。
/// 之前用 1x 源时 macOS 在 Retina 上把 37×18 px 拉成 74×36 px 显示，边缘糊化、
/// 视觉额外感觉「图标偏小」；改 2x 源后 NSImage 解出 2x rep，恰好匹配 Retina
/// 的 2x 显示比例——1x 屏上 NSImage 自动把 2x 源下采样，仍干净。
const RENDER_SCALE: u32 = 2;
/// macOS 菜单栏图标高度（pt 等价）：菜单栏 ≈ 24pt，状态栏图标 slot = 18pt。
const STATUS_LOGO_SIZE: u32 = 18 * RENDER_SCALE;
/// 状态点与两侧元素的间距（pt 等价 5）：云朵图标 |—GAP—| 状态点 |—GAP—| 文本。
/// 从 8 收紧到 5，让画布右侧空 strip 减少，图标看起来不再被框在过宽容器里。
const GAP: u32 = 5 * RENDER_SCALE;
/// **云朵-P Logo 边长**：从 16 → 18pt，**填满 18pt 视觉高度**。之前 LOGO_SIZE 16
/// + LOGO_Y 1 = 占画布 89%，比典型 18pt 图标（AirPods / Wi-Fi 等）矮约 11%，肉眼
/// 可辨的「偏小」。改后 LOGO 占据完整 18pt 高度，跟其他应用齐平。
const LOGO_SIZE: u32 = 18 * RENDER_SCALE;
/// 状态点直径（5pt + 亚像素抗锯齿）。
const DOT_DIAMETER: u32 = 5 * RENDER_SCALE;
/// 菜单栏图标画布宽：pt 价 = Logo(18) + GAP(5) + 点(5) + GAP(5) = 33；
/// @2x 源 = 66 px。
const STATUS_CANVAS_W: u32 = LOGO_SIZE + GAP + DOT_DIAMETER + GAP;
/// Logo 从画布 (0, 0) 起步——填满垂直 slot，不再像 LOGO_Y = 1 那样顶部留 1px 空带。
const LOGO_X: u32 = 0;
const LOGO_Y: u32 = 0;
/// 状态点：垂直居中于画布中线，水平 = Logo 右缘 + GAP + 点半径。
const DOT_CX: f32 = (LOGO_SIZE + GAP) as f32 + DOT_DIAMETER as f32 * 0.5;
const DOT_CY: f32 = STATUS_LOGO_SIZE as f32 * 0.5;
const DOT_R: f32 = DOT_DIAMETER as f32 * 0.5;
/// 状态光晕：圆心在 Logo 中心；半径包到画布外缘（角落 alpha 自然归 0，
/// 无硬边/方块）。半径适度放大让光晕柔化整个 18pt Logo 而不是只染中间。
const GLOW_CX: f32 = LOGO_SIZE as f32 * 0.5;
const GLOW_CY: f32 = STATUS_LOGO_SIZE as f32 * 0.5;
const GLOW_R: f32 = STATUS_LOGO_SIZE as f32 * 0.55;
const GLOW_MAX_ALPHA: f32 = 0.42;
/// **外框圆角半径**（pt 等价 2.5）：菜单栏图标上的「轻圆角」而非 iOS 22% 那种
/// 大圆角（约 18pt slot 的 14%）。Cutout 模式下 `LOGO_COLOR` 深色实心块 4 直角
/// 变 ~5px 弧，与 macOS 菜单栏右其他需要外观应用的图标（右表 Pixpin 等）潜贴
/// 一致——看起来「软」但不「鼓」。上 @2x = 5 px。
const CORNER_RADIUS: f32 = 2.5 * RENDER_SCALE as f32;
/// **Cutout 下实心方块内边描边 alpha**（255 = 不描边）。Pixpin 同口味——亮菜单栏
/// 上 `LOGO_COLOR` 深色实心方块与背景之间加一层 1px 透明描边，让二者不完全融合。
/// 38% 透明沱合 macOS App Icon / Pixpin 的「软缝」条谈，上 @2x = 38% * 255 ≈ 98。
const CUTOUT_STROKE_ALPHA: u8 = (0.38 * 255.0) as u8;
/// macOS 菜单栏外观（决定 Logo 主色 + LogoMode）。随系统外观切换：
/// - `Light` → 浅色菜单栏（NSLightAppearance 系），Logo 使用暖灰近黑
///   `#242523`（RGB 36, 37, 35），与 macOS 浅色菜单栏下第三方应用
///   （Warp / GitHub Desktop / Cursor 等）的常见图标色 RGB(~36, 36, 33)
///   — RGB(~40, 41, 38) 接近，对比柔和、与背景不融合；
/// - `Dark`  → 深色菜单栏（NSDarkAppearance 系），Logo 使用近白色
///   `#e6e6e8`（RGB 230, 230, 232），与 macOS 系统模板图标在 Dark Mode
///   下的渲染一致（Wi-Fi / Battery / Clock 都是白色剪影）。
///
/// 为什么不直接用黑/纯白？
/// - 纯黑 `#000000`：在浅色菜单栏上对比度过强、视觉噪音大；
/// - 纯白 `#ffffff`：在深色菜单栏上对比同样过强；
/// - 系统 `Label` 色（macOS 自动跟随 Light/Dark 切换）：理论上最准确，但
///   我们的菜单栏图标是动态渲染的 RGBA 缓冲区，不走 NSImage 模板模式也
///   拿不到自动适配；
/// - 改用 [Appearance] 在渲染期根据 `window.theme()` 选择浅色/深色菜单栏
///   对应的「暖灰近黑 / 近白色」基色，兼得柔和对齐与深浅适配。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Appearance {
    /// 浅色菜单栏：白色实心方块，P 与符号为透明镂空。
    #[default]
    Light,
    /// 深色菜单栏：同样使用白色实心方块，P 与符号为透明镂空。
    Dark,
}

impl Appearance {
    /// 从任意 webview 窗口解析系统外观：macOS 的 `window.theme()` 即等于
    /// `NSApp.effectiveAppearance`（Light/Dark），与菜单栏背景一致；
    /// Windows 上 `window.theme()` 跟随系统「应用模式 / 默认应用模式」，
    /// 与托盘通知区的明暗基色一致；非 macOS 平台对菜单栏外观无影响，
    /// 故直接退化为 Light（不参与颜色计算）。
    pub fn from_window<R: Runtime>(window: &tauri::WebviewWindow<R>) -> Self {
        match window.theme() {
            #[cfg(target_os = "macos")]
            Ok(tauri::Theme::Dark) => Self::Dark,
            #[cfg(not(target_os = "macos"))]
            Ok(tauri::Theme::Dark) => Self::Dark,
            _ => Self::Light,
        }
    }

    /// 从 AppHandle 任意可用 webview 取外观（main > tray-card > 默认 Light）。
    /// 找不到窗口时不强行报错——主线程渲染仍可继续，缺省用浅色基色。
    pub fn resolve<R: Runtime>(app: &AppHandle<R>) -> Self {
        for label in ["main", crate::services::tray::TRAY_WINDOW_LABEL] {
            if let Some(window) = app.get_webview_window(label) {
                return Self::from_window(&window);
            }
        }
        Self::Light
    }

    /// 当前外观下的 Logo 主色（单色填到云朵-P 形状 / 实心方块）。
    fn logo_color(self) -> [u8; 3] {
        match self {
            // 两种外观都对齐 macOS 菜单栏其它图标，使用同一白色 Logo。
            Self::Light => LIGHT_LOGO_COLOR,
            Self::Dark => DARK_LOGO_COLOR,
        }
    }

    /// 当前外观下的 LogoMode：两种菜单栏外观都使用 `Cutout`。
    /// Logo 整体保持单一深色/白色，云朵-P 里的 P 与符号保持透明，直接露出
    /// 菜单栏背景；只有整体色值随浅色/深色菜单栏切换。
    fn logo_mode(self) -> LogoMode {
        LogoMode::Cutout
    }
}

/// 浅色菜单栏下的 Logo 主色：与其它菜单栏图标一致的白色。
const LIGHT_LOGO_COLOR: [u8; 3] = [255, 255, 255];
/// 深色菜单栏下也使用同一白色 Logo，不随外观切换成深色。
const DARK_LOGO_COLOR: [u8; 3] = [255, 255, 255];
/// macOS 菜单栏 Logo：设计稿「云朵-P」抠图（透明底 PNG，编译期内嵌）。取自设计稿
/// 底部「24×24」尺寸示意实例（即设计稿中菜单栏尺寸的云朵-P），其他平台继续使用
/// 应用图标，本资源仅 macOS 使用。
const CLOUD_P_PNG: &[u8] = include_bytes!("../../icons/cloud-p.png");

/// 渲染 macOS 菜单栏图标：**云朵-P 主色（随系统外观深浅切换）+ 状态点 + 状态色光晕**。
/// 布局：光晕（软径向，状态色，背景）→ 云朵-P Logo（单色实心底，P 与符号为
/// 透明镂空）→ 右侧状态点（状态色实心圆，位于图标与主文本之间、垂直居中）。
///
/// Logo 颜色固定为白色，与 macOS 菜单栏其它图标保持一致。两种外观统一使用
/// Cutout，不再把暗色菜单栏切成云朵-P 剪影，P/符号始终保持透明。`appearance` 仍由
/// 调用方从 `theme()` 解析，并由 `WindowEvent::ThemeChanged` 触发重绘。
///
/// 不启用 NSImage 模板模式，否则状态点彩色会被系统染成黑白。
///
/// **主文本不画进图标**——由系统原生渲染（`set_title`）。
#[allow(dead_code)] // Windows/Linux 构建时仅测试使用
pub(crate) fn render_status_icon(
    item: &MenuBarItem,
    app_icon: Option<&Image>,
    appearance: Appearance,
) -> Image<'static> {
    const W: u32 = STATUS_CANVAS_W;
    const H: u32 = STATUS_LOGO_SIZE;
    let mut buf = vec![0u8; (W * H * 4) as usize];
    // 状态点 / 光晕按状态色调（绿/蓝/橙/红）保持不变——之前反复验证过在
    // 深浅两种菜单栏下都可辨识。Logo 主色 + 渲染模式按 appearance 选择。
    let logo_color = appearance.logo_color();
    let logo_mode = appearance.logo_mode();
    // 1) 状态光晕：状态色软径向渐变，先画（作为 Logo 背景色）；到半径处 alpha
    //    归零且边界在画布内，背景仍是完全透明（无方块/硬边）。
    fill_glow(
        &mut buf,
        W,
        H,
        GLOW_CX,
        GLOW_CY,
        GLOW_R,
        GLOW_MAX_ALPHA,
        item.tone.rgb(),
    );
    // 2) 云朵-P Logo：两种外观都使用单色实心底 + 透明镂空，只有主色不同。
    if let Some(icon) = cloud_p_icon().or(app_icon) {
        draw_icon_scaled(
            &mut buf,
            W,
            H,
            icon,
            LOGO_X,
            LOGO_Y,
            LOGO_SIZE,
            Some(logo_color),
            logo_mode.invert_alpha(),
        );
    }
    // 3) 状态点：右侧实心圆（状态色，1px 抗锯齿），位于图标与主文本之间。
    fill_circle_aa(&mut buf, W, H, DOT_CX, DOT_CY, DOT_R, item.tone.rgb());
    // 4) 在 LOGO 框外周应用「圆角矩形」蒙版——Cutout 模式下 `logo_color`
    //    实心方块的 4 个直角变为软圆角，菜单栏上不超边 / 不僵御。
    //    同时给 Cutout 下的实心方块加上 1px 半透明描边（Pixpin 同口味）：
    //    亮菜单栏上深色块与背景之间有 1px 透明过渡，不会完全融为一体。
    //    当前两种外观都使用 Cutout，因此统一应用圆角与轻描边。
    let stroke_alpha = Some(CUTOUT_STROKE_ALPHA);
    apply_rounded_corners(
        &mut buf,
        W,
        H,
        LOGO_X,
        LOGO_Y,
        LOGO_X + LOGO_SIZE,
        LOGO_Y + LOGO_SIZE,
        CORNER_RADIUS,
        stroke_alpha,
    );
    Image::new_owned(buf, W, H)
}

/// 状态光晕：以 `(cx, cy)` 为圆心的软径向渐变，alpha 从 `max_alpha` 线性衰减到
/// 半径处为 0（覆盖度 = max(0, 1 - d/r)），在画布内自然淡出，无硬边。
#[allow(dead_code)] // Windows/Linux 构建时随 render_status_icon 仅测试使用
fn fill_glow(
    buf: &mut [u8],
    w: u32,
    h: u32,
    cx: f32,
    cy: f32,
    r: f32,
    max_alpha: f32,
    color: [u8; 3],
) {
    let x0 = (cx - r).max(0.0) as u32;
    let x1 = (cx + r).min(w as f32) as u32;
    let y0 = (cy - r).max(0.0) as u32;
    let y1 = (cy + r).min(h as f32) as u32;
    for py in y0..y1 {
        for px in x0..x1 {
            let dx = px as f32 + 0.5 - cx;
            let dy = py as f32 + 0.5 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let a = (max_alpha * (1.0 - dist / r)).max(0.0);
            if a <= 0.0 {
                continue;
            }
            let idx = ((py * w + px) * 4) as usize;
            for k in 0..3 {
                buf[idx + k] = (color[k] as f32 * a + buf[idx + k] as f32 * (1.0 - a)) as u8;
            }
            buf[idx + 3] = (a * 255.0 + buf[idx + 3] as f32 * (1.0 - a)).round() as u8;
        }
    }
}

/// 编译期内嵌的「云朵-P」Logo（macOS 菜单栏专用），首次使用时按 PNG 解码并缓存。
/// 解码失败返回 None（由调用方回退到应用图标）。
#[allow(dead_code)] // Windows/Linux 构建时仅测试使用（随 render_status_icon）
fn cloud_p_icon() -> Option<&'static Image<'static>> {
    static CLOUD_P_ICON: std::sync::OnceLock<Option<Image<'static>>> = std::sync::OnceLock::new();
    CLOUD_P_ICON
        .get_or_init(|| Image::from_bytes(CLOUD_P_PNG).ok())
        .as_ref()
}

/// macOS 菜单栏 Logo 渲染模式：
/// - `Cutout`：透明背景填成主色，原始 P 与符号保留为透明窗孔，形成
///   「深色/白色整体 + 透明镂空」的菜单栏图标。
#[derive(Clone, Copy)]
enum LogoMode {
    Cutout,
}

impl LogoMode {
    fn invert_alpha(self) -> bool {
        true
    }
}

/// 渲染 Windows 通知区小图标：32×32 真实 Logo + 右下角平滑状态点（tooltip 承载文本）。
#[allow(dead_code)] // macOS 构建时未使用（仅测试引用 render_status_icon）
fn render_tray_icon(tone: Tone, app_icon: Option<&Image>) -> Image<'static> {
    const S: u32 = 32;
    let mut buf = vec![0u8; (S * S * 4) as usize];
    if let Some(icon) = app_icon {
        draw_icon_scaled(&mut buf, S, S, icon, 0, 0, S, None, false);
    }
    fill_circle_aa(&mut buf, S, S, 27.0, 27.0, 4.5, tone.rgb());
    Image::new_owned(buf, S, S)
}

/// 把应用图标（RGBA）双线性缩放到 size×size 嵌入缓冲区（保留 alpha 形状）。
/// `tint = Some(rgb)` 时输出单色（macOS 菜单栏深色单色 Logo）；`invert_alpha = true`
/// 时再做一次 `alpha_out = 255 - alpha_src`，配合 `LOGO_MODE = Cutout` 把云朵-P
/// 形状的源（白色填充）输出成「**深色实心方块中的云朵-P 窗孔**」——原透明的位置
/// 变深色、原白色的剪影变成透明。`None` 保留原图颜色（Windows/Linux 托盘）。
fn draw_icon_scaled(
    buf: &mut [u8],
    w: u32,
    h: u32,
    icon: &Image,
    x: u32,
    y: u32,
    size: u32,
    tint: Option<[u8; 3]>,
    invert_alpha: bool,
) {
    let (iw, ih) = (icon.width(), icon.height());
    let rgba = icon.rgba();
    if iw == 0 || ih == 0 {
        return;
    }
    let px_at = |sx: u32, sy: u32| -> [u8; 4] {
        let i = ((sy * iw + sx) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    };
    for py in 0..size {
        for px in 0..size {
            let sx = (px as f32 + 0.5) / size as f32 * iw as f32 - 0.5;
            let sy = (py as f32 + 0.5) / size as f32 * ih as f32 - 0.5;
            let x0 = sx.max(0.0).floor() as u32;
            let y0 = sy.max(0.0).floor() as u32;
            let x1 = (x0 + 1).min(iw - 1);
            let y1 = (y0 + 1).min(ih - 1);
            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;
            let c00 = px_at(x0, y0);
            let c10 = px_at(x1, y0);
            let c01 = px_at(x0, y1);
            let c11 = px_at(x1, y1);
            let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
            let mut color = [0u8; 4];
            for k in 0..4 {
                let top = lerp(c00[k] as f32, c10[k] as f32, fx);
                let bottom = lerp(c01[k] as f32, c11[k] as f32, fx);
                color[k] = lerp(top, bottom, fy).round() as u8;
            }
            if let Some(t) = tint {
                color[0] = t[0];
                color[1] = t[1];
                color[2] = t[2];
                if invert_alpha {
                    // 反转：源透明背景变成主色实心底，源 P/符号变成透明窗孔。
                    color[3] = 255 - color[3];
                }
            }
            let target_x = x + px;
            let target_y = y + py;
            if invert_alpha {
                // Cutout 不是普通 alpha over：先前的状态光晕不能混入 Logo。
                // 直接替换 RGBA，才能保证背景是纯深色/白色，P/符号是纯透明。
                if target_x < w && target_y < h {
                    let idx = ((target_y * w + target_x) * 4) as usize;
                    buf[idx..idx + 4].copy_from_slice(&color);
                }
            } else {
                blend_pixel(buf, w, h, target_x, target_y, color);
            }
        }
    }
}

/// 单像素 alpha 混合（RGBA over 现有缓冲；透明源像素保持不变）。
fn blend_pixel(buf: &mut [u8], w: u32, h: u32, px: u32, py: u32, color: [u8; 4]) {
    if px >= w || py >= h {
        return;
    }
    let a = color[3] as f32 / 255.0;
    if a <= 0.0 {
        return; // 源透明：保持目标像素原样（包括 alpha）
    }
    let idx = ((py * w + px) * 4) as usize;
    if a >= 1.0 {
        buf[idx..idx + 4].copy_from_slice(&color);
        return;
    }
    for k in 0..3 {
        buf[idx + k] = (color[k] as f32 * a + buf[idx + k] as f32 * (1.0 - a)) as u8;
    }
    buf[idx + 3] = (a * 255.0 + buf[idx + 3] as f32 * (1.0 - a)).round() as u8;
}

/// 「圆角矩形」蒙版：在 `[x0, y0) × [x1, y1)` 矩形框的四个角按 `radius`
/// 软化到角外 alpha 为 0。Cutout 模式下 `LOGO_COLOR` 深色实心方块的 4 直角变软圆角，与
/// App Icon 视觉语言一致。算法：
///   1) 每个角的圆心在矩形「外推 r」后的角点 (x0+r, y0+r) / (x1-r-1, y0+r)…
///   2) 像素落在角的扇形（TL: x∈[x0,x0+r) ∧ y∈[y0,y0+r)）内时，比较到对应
///      圆心的距离 dist 与 r：dist ≤ r-0.5 → 全保留；r-0.5 < dist < r+0.5 →
///      按比例衰减 alpha；dist ≥ r+0.5 → 完全透明。
/// 「圆角 + 可选 1px 内边描边」统一蒙版：
///   - `radius = 0`：退化为 1px 内描边（在矩形四条边内 1px 处 fade）。
///   - `radius > 0`：先按 `radius` 把矩形外扩→角外的像素裁 alpha=0（圆角）。
///   - `stroke_alpha = Some(s)`：在外圆角保留不变的前提下，把矩形「内边界
///     1px ring」（靠近外圆角 1pt 的像素）的 alpha 乘以 `s/255`，形成
///     「半透明描边」——效果是菜单栏高亮度底（亮色模式）下 Cutout 实心方块
///     (`LOGO_COLOR` 深色) 与背景之间有 1px 透明过渡，避免二者与背景完全融为一体。
/// 「圆角 + 可选 1px 内边描边」统一蒙版（iq SDF 风格）：
///   - `radius` = 圆角半径（绝对像素）。
///   - `stroke_alpha = Some(s)`：把距外边界 1px 以内像素的 alpha 乘以 `s/255`，
///     形成「半透明描边」（Pixpin 同口味），让 Cutout `LOGO_COLOR` 深色实心方块
///     在亮菜单栏上有 1px 透明缝，不与背景完全融为一体。
///
/// 算法：把像素坐标重映射到「圆角矩形相对中心」的 SDF；按符号函数决定如何
/// 处理 alpha：
///   - sdf ≥ 0.5  → 像素在圆角矩形之外，alpha=0。
///   - 0 ≤ sdf < 0.5  → 1px 抗锯齿外缘带（rect 含圆角）× coverage。
///   - -1 < sdf ≤ 0  → 1px 内描边带，alpha *= stroke_factor（如果提供）。
///   - sdf ≤ -1  → 矩形内部深处，alpha 不变。
fn apply_rounded_corners(
    buf: &mut [u8],
    w: u32,
    h: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    radius: f32,
    stroke_alpha: Option<u8>,
) {
    let r = radius.max(0.0);
    let stroke_factor = stroke_alpha.map(|a| a as f32 / 255.0).unwrap_or(1.0);
    if r <= 0.0 && stroke_factor >= 1.0 {
        return;
    }
    let x0f = x0 as f32;
    let y0f = y0 as f32;
    let x1f = x1 as f32 - 1.0; // 闭区间 [x0f, x1f]
    let y1f = y1 as f32 - 1.0;
    let cx = (x0f + x1f) * 0.5;
    let cy = (y0f + y1f) * 0.5;
    let half_w = (x1f - x0f) * 0.5;
    let half_h = (y1f - y0f) * 0.5;
    // 扫描略外扩 1px 让外缘抗锯齿正确。
    let xs = x0.saturating_sub(1)..(x1 + 1).min(w);
    let ys = y0.saturating_sub(1)..(y1 + 1).min(h);
    for py in ys {
        for px in xs.clone() {
            let pxf = px as f32 + 0.5;
            let pyf = py as f32 + 0.5;
            // iq 圆角矩形 SDF：以中心为原点的 q = (|p-cx|, |p-cy|) - half + r。
            let qx = (pxf - cx).abs() - half_w + r;
            let qy = (pyf - cy).abs() - half_h + r;
            let qx_pos = qx.max(0.0);
            let qy_pos = qy.max(0.0);
            let len2 = (qx_pos * qx_pos + qy_pos * qy_pos).sqrt();
            let min_max = qx.max(qy).min(0.0);
            let sdf = len2 + min_max - r;
            let idx = ((py * w + px) * 4) as usize;
            if sdf >= 0.5 {
                // 圆角矩形之外（含外缘抗锯齿带外半）：alpha = 0
                buf[idx + 3] = 0;
                continue;
            }
            if sdf >= 0.0 {
                // 1px 外缘抗锯齿带 内半部分：coverage = 0.5 - sdf（0..0.5）
                let coverage = 0.5 - sdf;
                let prev_a = buf[idx + 3] as f32;
                buf[idx + 3] = (prev_a * coverage).round() as u8;
                continue;
            }
            // sdf < 0：在矩形内部。
            if stroke_factor < 1.0 && sdf > -1.0 {
                // 1px 内描边带：把 alpha 乘以 stroke_factor
                let prev_a = buf[idx + 3] as f32;
                buf[idx + 3] = (prev_a * stroke_factor).round() as u8;
            }
            // 否则保持原 alpha
        }
    }
}

/// 抗锯齿圆（覆盖度 = clamp(r + 0.5 - dist, 0, 1)，1px 平滑边缘）。
fn fill_circle_aa(buf: &mut [u8], w: u32, h: u32, cx: f32, cy: f32, r: f32, color: [u8; 3]) {
    let x0 = (cx - r - 1.0).max(0.0) as u32;
    let x1 = (cx + r + 1.0).min(w as f32) as u32;
    let y0 = (cy - r - 1.0).max(0.0) as u32;
    let y1 = (cy + r + 1.0).min(h as f32) as u32;
    for py in y0..y1 {
        for px in x0..x1 {
            let dx = px as f32 + 0.5 - cx;
            let dy = py as f32 + 0.5 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let coverage = (r + 0.5 - dist).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let idx = ((py * w + px) * 4) as usize;
            for k in 0..3 {
                buf[idx + k] =
                    (color[k] as f32 * coverage + buf[idx + k] as f32 * (1.0 - coverage)) as u8;
            }
            buf[idx + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造测试图标：16×16，中央 8×8 蓝色方块、周围透明（模拟真实 Logo 的
    /// 透明背景 + 彩色主体的结构）。tauri 的 Image::from_bytes 需要 image-png
    /// feature，测试不启用，直接构造 RGBA。
    fn test_app_icon() -> Image<'static> {
        const S: u32 = 16;
        let mut rgba = vec![0u8; (S * S * 4) as usize];
        for y in 4..12 {
            for x in 4..12 {
                let i = ((y * S + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[0, 122, 255, 255]); // #007aff
            }
        }
        Image::new_owned(rgba, S, S)
    }

    /// 渲染各状态菜单栏图标为 PPM 预览图（`cargo test -- --ignored dump_menu_bar_preview`），
    /// 用于人工核对视觉（sips 转 PNG 后在浏览器预览）。加载真实应用图标验证最终效果。
    /// 按 [`Appearance`] 分别渲染：浅色菜单栏渲染主色 = `LIGHT_LOGO_COLOR`
    ///（暖灰近黑 + Cutout）+ 浅色底；深色菜单栏渲染主色 = `DARK_LOGO_COLOR`
    ///（近白 + Silhouette）+ 深色底。状态点/光晕按综合状态着色，两种
    /// 外观共用一套语义色（绿/蓝/橙/红）。
    #[test]
    #[ignore]
    fn dump_menu_bar_preview() {
        let dir = std::env::temp_dir().join("menubar-preview");
        std::fs::create_dir_all(&dir).ok();
        let icon_path = concat!(env!("CARGO_MANIFEST_DIR"), "/icons/32x32.png");
        let icon = std::fs::read(icon_path)
            .ok()
            .and_then(|bytes| Image::from_bytes(&bytes).ok());
        eprintln!("real icon loaded: {}", icon.is_some());
        let items: &[(&str, MenuBarItem)] = &[
            (
                "online",
                MenuBarItem {
                    tone: Tone::Online,
                    text: "今日 240K".into(),
                    tooltip: String::new(),
                },
            ),
            (
                "active",
                MenuBarItem {
                    tone: Tone::Active,
                    text: "今日 240K".into(),
                    tooltip: String::new(),
                },
            ),
            (
                "warn",
                MenuBarItem {
                    tone: Tone::QuotaWarn,
                    text: "今日 240K".into(),
                    tooltip: String::new(),
                },
            ),
            (
                "offline",
                MenuBarItem {
                    tone: Tone::Offline,
                    text: "今日 240K".into(),
                    tooltip: String::new(),
                },
            ),
        ];
        for (name, item) in items {
            let img_light = render_status_icon(item, icon.as_ref(), Appearance::Light);
            let img_dark = render_status_icon(item, icon.as_ref(), Appearance::Dark);
            // 亮菜单栏下暖灰近黑色块主场；深菜单栏下近白剪影主场。
            dump_ppm(
                &dir,
                &format!("{name}-lightbar-light"),
                &img_light,
                [232, 232, 236],
            );
            dump_ppm(
                &dir,
                &format!("{name}-darkbar-dark"),
                &img_dark,
                [38, 38, 42],
            );
            // 也输出原始 RGBA（保 alpha），便于带透明背景的渲染 / 检查
            // 圆角蒙版。两种外观各一份。
            for (suffix, img) in [("light", &img_light), ("dark", &img_dark)] {
                let rgba = img.rgba();
                let mut bytes = Vec::with_capacity(rgba.len());
                bytes.extend_from_slice(rgba);
                std::fs::write(dir.join(format!("{name}-raw-{suffix}.rgba")), &bytes).unwrap();
                std::fs::write(
                    dir.join(format!("{name}-raw-{suffix}.meta")),
                    format!(
                        "{} {}
",
                        img.width(),
                        img.height()
                    )
                    .as_bytes(),
                )
                .unwrap();
            }
        }
        eprintln!("preview PPMs + raw RGBA -> {}", dir.display());
    }

    /// 把 RGBA 合成到背景色上输出 PPM（模拟菜单栏浅色/深色底）。
    fn dump_ppm(dir: &std::path::Path, name: &str, img: &Image<'static>, bg: [u8; 3]) {
        let rgba = img.rgba();
        let mut rgb = Vec::with_capacity((img.width() * img.height() * 3) as usize);
        for p in rgba.chunks_exact(4) {
            let a = p[3] as f32 / 255.0;
            for k in 0..3 {
                rgb.push((p[k] as f32 * a + bg[k] as f32 * (1.0 - a)).round() as u8);
            }
        }
        let mut data = format!("P6\n{} {}\n255\n", img.width(), img.height()).into_bytes();
        data.extend_from_slice(&rgb);
        std::fs::write(dir.join(format!("{name}.ppm")), data).unwrap();
    }

    #[test]
    fn compact_tokens_formats() {
        assert_eq!(compact_tokens(0), "0");
        assert_eq!(compact_tokens(999), "999");
        assert_eq!(compact_tokens(1_250), "1.2K");
        assert_eq!(compact_tokens(240_408), "240.4K");
        assert_eq!(compact_tokens(2_500_000), "2.50M");
        assert_eq!(compact_tokens(3_000_000_000), "3.00B");
    }

    #[test]
    fn commas_format() {
        assert_eq!(with_commas(0), "0");
        assert_eq!(with_commas(999), "999");
        assert_eq!(with_commas(1_000), "1,000");
        assert_eq!(with_commas(240_408), "240,408");
    }

    #[test]
    fn quota_tone_thresholds() {
        assert_eq!(quota_tone(2.0), Tone::Offline);
        assert_eq!(quota_tone(5.0), Tone::QuotaWarn);
        assert_eq!(quota_tone(14.9), Tone::QuotaWarn);
        assert_eq!(quota_tone(15.0), Tone::Online);
        assert_eq!(quota_tone(92.0), Tone::Online);
    }

    #[test]
    fn combined_severity_priority() {
        // 网关离线（红）优先于一切
        assert_eq!(
            combined_status(false, false, Some(90.0)),
            (Tone::Offline, "网关离线")
        );
        assert_eq!(
            combined_status(false, false, Some(8.0)),
            (Tone::Offline, "网关离线")
        );
        assert_eq!(
            combined_status(false, false, None),
            (Tone::Offline, "网关离线")
        );
        // 额度告警/耗尽（橙）> 流量活跃（蓝）> 正常（绿）
        assert_eq!(
            combined_status(true, true, Some(8.0)),
            (Tone::QuotaWarn, "额度告警")
        );
        assert_eq!(
            combined_status(true, false, Some(2.0)),
            (Tone::QuotaWarn, "额度耗尽")
        );
        assert_eq!(
            combined_status(true, true, Some(90.0)),
            (Tone::Active, "流量活跃")
        );
        assert_eq!(
            combined_status(true, false, Some(90.0)),
            (Tone::Online, "在线")
        );
        // 无额度数据时只看网关：活跃 → 蓝，否则绿
        assert_eq!(
            combined_status(true, true, None),
            (Tone::Active, "流量活跃")
        );
        assert_eq!(combined_status(true, false, None), (Tone::Online, "在线"));
    }

    #[test]
    fn main_text_selects_content() {
        // 三选一主文本（带含义标签）：tokens → 「今日 240.4K」；top1_tool/top1_model
        // → 「工具 / 模型 第一名」
        assert_eq!(
            main_text_value(
                MainText::Tokens,
                240_408,
                Some("Claude Code"),
                Some("gpt-4o")
            ),
            "今日 240.4K"
        );
        assert_eq!(
            main_text_value(MainText::Top1Tool, 240_408, Some("Claude Code"), None),
            "工具 Claude Code"
        );
        assert_eq!(
            main_text_value(MainText::Top1Model, 240_408, None, Some("gpt-4o")),
            "模型 gpt-4o"
        );
        // Top1 超长名截断：名称预算 = MAX_TEXT_CHARS - 标签长度（3），总长不超 MAX_TEXT_CHARS
        let long = main_text_value(
            MainText::Top1Model,
            240_408,
            None,
            Some("claude-sonnet-4-5-20241022"),
        );
        assert_eq!(long.chars().count(), MAX_TEXT_CHARS);
        assert!(long.starts_with("模型 "));
        assert!(long.ends_with('…'));
        // 无 Top1 数据时回退到今日 Tokens（带「今日 」标签）
        assert_eq!(
            main_text_value(MainText::Top1Tool, 240_408, None, None),
            "今日 240.4K"
        );
        assert_eq!(
            main_text_value(MainText::Top1Model, 240_408, Some(""), Some("")),
            "今日 240.4K"
        );
        // 设置字符串解析
        assert_eq!(MainText::from_setting("top1_tool"), MainText::Top1Tool);
        assert_eq!(MainText::from_setting("top1_model"), MainText::Top1Model);
        assert_eq!(MainText::from_setting("tokens"), MainText::Tokens);
        assert_eq!(MainText::from_setting("whatever"), MainText::Tokens);
    }

    #[test]
    fn truncate_display_shortens() {
        assert_eq!(truncate_display("Claude Code", 16), "Claude Code");
        assert_eq!(
            truncate_display("claude-sonnet-4-5", 16),
            "claude-sonnet-4…"
        );
        // 中文字符按字符截断
        assert_eq!(
            truncate_display("这是一个非常长的工具名字", 8),
            "这是一个非常长…"
        );
    }

    #[test]
    fn menu_bar_icon_uses_appearance_logo_color() {
        // 图标 = 白色 Logo + 彩色状态点 + 状态色光晕。
        // 画布 33×18（Logo 18 + GAP + 状态点）。
        // - Light (Cutout)：云朵-P 源 alpha 反转 → 实心方块填底色 + 云朵窗孔；
        //   LOGO 区域大部分像素为主色且 alpha 接近 255（实心方块）。
        // - Dark (Cutout)：与 Light 完全相同的白色透明镂空结构。
        let icon = test_app_icon();
        let item = MenuBarItem {
            tone: Tone::Online,
            text: String::new(),
            tooltip: String::new(),
        };
        // Light: Cutout 实心方块。LOGO 区域应充满主色。
        let img_light = render_status_icon(&item, Some(&icon), Appearance::Light);
        assert_eq!(img_light.width(), STATUS_CANVAS_W, "画布 = Logo + 状态点");
        assert_eq!(img_light.height(), STATUS_LOGO_SIZE);
        let rgba_light = img_light.rgba();
        let w = img_light.width();
        let hits_light = count_logo_color_pixels(rgba_light, w, LIGHT_LOGO_COLOR, 200, 1);
        assert!(
            hits_light > 200,
            "Light 主色近不透明像素仅 {hits_light}: Cutout 实心方块应填满 Logo 区域"
        );
        // Dark: Silhouette 云朵-P 剪影。云朵-P 源 1378×1378 (19% 软 alpha)
        // 缩放到 LOGO_SIZE 后命中像素 ≈ 0.19 * 1296 ≈ 246（实测 288）。
        // 容差放宽到 t·(α/255) 后的最大偏移（约 2），alpha 阈值降到 50。
        let img_dark = render_status_icon(&item, Some(&icon), Appearance::Dark);
        assert_eq!(img_dark.width(), STATUS_CANVAS_W);
        assert_eq!(img_dark.height(), STATUS_LOGO_SIZE);
        let rgba_dark = img_dark.rgba();
        let hits_dark = count_logo_color_pixels(rgba_dark, w, DARK_LOGO_COLOR, 50, 6);
        assert!(
            hits_dark > 30,
            "Dark 主色命中像素仅 {hits_dark}: Silhouette 云朵-P 上主色至少 30 个像素"
        );
        // 两种外观共有不变量：
        // 1) 右上/右下角外侧仍透明（光晕不溢出右侧 GAP，无方块/容器硬边）；
        // 2) 状态点/光晕存在：强彩色像素（绿/蓝/橙/红）> 0。
        let corner =
            |img: &[u8], width: u32, x: u32, y: u32| img[((y * width + x) * 4) as usize + 3];
        assert_eq!(corner(rgba_light, w, w - 1, 0), 0, "Light 右上角透明");
        assert_eq!(
            corner(rgba_light, w, w - 1, STATUS_LOGO_SIZE - 1),
            0,
            "Light 右下角透明"
        );
        assert_eq!(corner(rgba_dark, w, w - 1, 0), 0, "Dark 右上角透明");
        assert_eq!(
            corner(rgba_dark, w, w - 1, STATUS_LOGO_SIZE - 1),
            0,
            "Dark 右下角透明"
        );
        assert!(
            count_chromatic(rgba_light) > 20,
            "Light 图标应含状态点/光晕等彩色像素"
        );
        assert!(
            count_chromatic(rgba_dark) > 20,
            "Dark 图标应含状态点/光晕等彩色像素"
        );
    }

    #[test]
    fn menu_bar_icon_logo_is_present_regardless_of_tone_and_appearance() {
        // Logo 在两种 appearance 下都保持白色，状态点/光晕只受 tone 影响。
        // 两种外观都使用 Cutout：实心方块 + 透明镂空。
        // 两种情况下都要避免「颜色完全没应用」的回归。
        let icon = test_app_icon();
        for appearance in [Appearance::Light, Appearance::Dark] {
            let alpha_threshold = match appearance {
                Appearance::Light => 200,
                Appearance::Dark => 200,
            };
            let logo_max = |tone: Tone| {
                let item = MenuBarItem {
                    tone,
                    text: String::new(),
                    tooltip: String::new(),
                };
                let img = render_status_icon(&item, Some(&icon), appearance);
                let w = img.width();
                img.rgba()
                    .chunks_exact(4)
                    .enumerate()
                    .filter(|(i, _)| {
                        let x = (i % w as usize) as u32;
                        let y = (i / w as usize) as u32;
                        (LOGO_X..LOGO_X + LOGO_SIZE).contains(&x)
                            && (LOGO_Y..LOGO_Y + LOGO_SIZE).contains(&y)
                    })
                    .map(|(_, p)| p[3])
                    .max()
                    .unwrap_or(0)
            };
            for tone in [Tone::Online, Tone::Active, Tone::QuotaWarn, Tone::Offline] {
                assert!(
                    logo_max(tone) >= alpha_threshold,
                    "appearance {:?} + tone {:?} 的 Logo 区域应有 alpha≥{alpha_threshold} 像素，实际最大 alpha={}",
                    appearance,
                    tone,
                    logo_max(tone)
                );
            }
            // 主色命中（任意 alpha>50）至少有几个像素，避免回归。
            let item = MenuBarItem {
                tone: Tone::Online,
                text: String::new(),
                tooltip: String::new(),
            };
            let expected = appearance.logo_color();
            let img = render_status_icon(&item, Some(&icon), appearance);
            let w = img.width();
            // Cutout 反转后源透明区域变成主色实心（α≈255），P/符号区域被清空；
            // 给边缘抗锯齿保留少量容差。
            let hits = count_logo_color_pixels(img.rgba(), w, expected, 50, 6);
            assert!(
                hits > 30,
                "appearance {:?} 期望主色 {expected:?} 至少出现 30 次，实际 {hits}",
                appearance
            );
        }
    }

    /// 统计「LOGO 区域内 RGB 与 `expected` 相差 ≤ `tolerance`、alpha ≥ `min_alpha`」
    /// 的像素数。混合 alpha 后 dest RGB = tint · α/255，最大偏移不超过
    /// `tint · (1 - min_alpha/255)`；`tolerance = 6` 可在 min_alpha ≥ 50 时
    /// 覆盖到足以正确匹配。
    fn count_logo_color_pixels(
        rgba: &[u8],
        width: u32,
        expected: [u8; 3],
        min_alpha: u8,
        tolerance: u8,
    ) -> usize {
        rgba.chunks_exact(4)
            .enumerate()
            .filter(|(i, p)| {
                let x = (i % width as usize) as u32;
                let y = (i / width as usize) as u32;
                (LOGO_X..LOGO_X + LOGO_SIZE).contains(&x)
                    && (LOGO_Y..LOGO_Y + LOGO_SIZE).contains(&y)
                    && p[3] >= min_alpha
                    && p[0].abs_diff(expected[0]) <= tolerance
                    && p[1].abs_diff(expected[1]) <= tolerance
                    && p[2].abs_diff(expected[2]) <= tolerance
            })
            .count()
    }

    #[test]
    fn status_dot_and_glow_use_tone_color() {
        // 状态点/光晕颜色随状态变化：在线 → 绿点绿光晕；离线 → 红点红光晕。
        // 取浅色菜单栏外观（LogoMode=Cutout）渲染，与之前断言一致。
        let icon = test_app_icon();
        let online = MenuBarItem {
            tone: Tone::Online,
            text: String::new(),
            tooltip: String::new(),
        };
        let offline = MenuBarItem {
            tone: Tone::Offline,
            text: String::new(),
            tooltip: String::new(),
        };
        let img_online = render_status_icon(&online, Some(&icon), Appearance::Light);
        let img_offline = render_status_icon(&offline, Some(&icon), Appearance::Light);
        let green = Tone::Online.rgb();
        let red = Tone::Offline.rgb();
        let dot_region = |img: &Image, color: [u8; 3]| {
            let w = img.width();
            img.rgba()
                .chunks_exact(4)
                .enumerate()
                .filter(|(i, p)| {
                    let x = (i % w as usize) as u32;
                    let y = (i / w as usize) as u32;
                    x >= DOT_CX as u32 - 3
                        && x <= DOT_CX as u32 + 3
                        && y >= DOT_CY as u32 - 3
                        && y <= DOT_CY as u32 + 3
                        && (p[0] as i32 - color[0] as i32).abs() < 40
                        && (p[1] as i32 - color[1] as i32).abs() < 40
                        && (p[2] as i32 - color[2] as i32).abs() < 40
                        && p[3] > 200
                })
                .count()
        };
        let online_green = dot_region(&img_online, green);
        let online_red = dot_region(&img_online, red);
        let offline_red = dot_region(&img_offline, red);
        assert!(online_green > 5, "在线状态点应为绿色: {online_green}");
        assert_eq!(online_red, 0, "在线不应有红色状态点: {online_red}");
        assert!(offline_red > 5, "离线状态点应为红色: {offline_red}");
    }

    #[test]
    fn appearance_logo_mode_matches_menu_bar_spec() {
        // 两种外观都必须是「白色整体 + P/符号透明镂空」，不因浅色/深色菜单栏变色。
        assert!(matches!(Appearance::Light.logo_mode(), LogoMode::Cutout));
        assert!(matches!(Appearance::Dark.logo_mode(), LogoMode::Cutout));
        assert_eq!(Appearance::Light.logo_color(), [255, 255, 255]);
        assert_eq!(Appearance::Dark.logo_color(), [255, 255, 255]);
        assert_eq!(LIGHT_LOGO_COLOR, DARK_LOGO_COLOR);
    }

    #[test]
    fn menu_bar_icon_has_transparent_cutouts_in_both_appearances() {
        let icon = test_app_icon();
        // 直接用测试图标验证 Cutout 的核心不变量：透明背景填主色，
        // 原始 P/符号（不透明区域）真正清空，而不是被之前的光晕覆盖。
        let mut cutout = vec![0u8; (STATUS_CANVAS_W * STATUS_LOGO_SIZE * 4) as usize];
        draw_icon_scaled(
            &mut cutout,
            STATUS_CANVAS_W,
            STATUS_LOGO_SIZE,
            &icon,
            LOGO_X,
            LOGO_Y,
            LOGO_SIZE,
            Some(LIGHT_LOGO_COLOR),
            true,
        );
        let alpha_at =
            |rgba: &[u8], x: u32, y: u32| rgba[((y * STATUS_CANVAS_W + x) * 4) as usize + 3];
        assert_eq!(
            alpha_at(&cutout, LOGO_SIZE / 2, LOGO_SIZE / 2),
            0,
            "P/符号应透明"
        );
        assert!(
            alpha_at(&cutout, LOGO_SIZE / 8, LOGO_SIZE / 8) > 100,
            "Logo 整体应填满"
        );

        // 两种外观都走同一套透明镂空结构；Logo 框外不应出现硬边或画布方块。
        let item = MenuBarItem {
            tone: Tone::Online,
            text: String::new(),
            tooltip: String::new(),
        };
        for appearance in [Appearance::Light, Appearance::Dark] {
            let img = render_status_icon(&item, Some(&icon), appearance);
            let w = img.width();
            let rgba = img.rgba();
            assert_eq!(alpha_at(rgba, w - 1, 0), 0, "{appearance:?} 右上角应透明");
            assert_eq!(
                alpha_at(rgba, w - 1, img.height() - 1),
                0,
                "{appearance:?} 右下角应透明"
            );
        }
    }

    /// 统计「强彩色」像素数（RGB 通道最大差 > 24 且有 alpha）。白色/灰色剪影
    /// （r≈g≈b）不计入，用于断言状态点/光晕（绿/蓝/橙/红）存在。
    fn count_chromatic(rgba: &[u8]) -> usize {
        rgba.chunks_exact(4)
            .filter(|p| {
                p[3] > 0 && {
                    let max = *p.iter().take(3).max().unwrap();
                    let min = *p.iter().take(3).min().unwrap();
                    (max as i32 - min as i32) > 24
                }
            })
            .count()
    }
}
