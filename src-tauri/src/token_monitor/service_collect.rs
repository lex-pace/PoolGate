//! 采集调度（W2 独占文件，勿与 W1 抢 `service.rs`）。
//!
//! - watcher：notify 文件监听 + 700ms 去抖沉降 → `collect_source`。
//! - 快速今日刷新：独立线程每 10s 重扫 `tokscale --today` 并重绘菜单栏（对齐开源
//!   Token Monitor 的秒级刷新机制——开源版由 chokidar 文件事件驱动，这里用等价
//!   的 10s 轮询，today 数字不再等 5 分钟全量扫描节流）。
//! - 降级轮询：`watch=false` 的源每 30s 轮询一次（禁止定时全盘扫描）。
//! - 状态机：`CollectorError` → `CollectorStatus`；单 Adapter 故障隔离，不 panic 全局。
//! - checkpoint 存现有 settings 表（`collector/checkpoint.rs`）。
//! - 采集落库后调 `state.token_monitor.publish_usage_delta` 广播增量。
//!
//! 入口 `start(app, state)` 由集成负责人在 `service.rs::init` 中追加一行调用。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use sha2::Digest;

use crate::token_monitor::collector::checkpoint;
use crate::token_monitor::collector::custom_app;
use crate::token_monitor::collector::watcher::{FileWatch, PendingSource};
use crate::token_monitor::collector::{AdapterRegistry, ToolAdapter};
use crate::token_monitor::model::{
    CollectorError, CollectorStatus, DataFormat, DataSource, ToolCollectorState,
};
use crate::AppState;

/// 进程内「采集中的 source_id」守卫（防同一源并发重入；命令与循环共用）。
fn in_flight() -> &'static Mutex<HashSet<String>> {
    static IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 启动采集调度（挂 AppState 生命周期；空注册表 = 静默无操作，W3 注册后自动生效）。
pub fn start(app: &tauri::AppHandle, state: Arc<AppState>) -> Result<(), String> {
    spawn_watch_loop(app.clone(), state.clone());
    spawn_poll_loop(state.clone());
    tracing::info!("token_monitor::service_collect: collection started");
    Ok(())
}

// ---------------------------------------------------------------------------
// 数据源与注册表
// ---------------------------------------------------------------------------

/// 从注册表 + 用户自定义路径构建有效数据源列表（每次调用现算，避免共享状态）。
///
/// 复刻开源 Token Monitor:当本机存在 tokscale 聚合引擎时,它成为唯一权威采集源
/// (覆盖 36+ 工具,口径与开源版逐值一致),手写逐工具适配器自动退场(避免双算)。
/// tokscale 不可用时回退到逐工具适配器(降级路径)。
pub(crate) fn refresh_sources(state: &Arc<AppState>) -> Vec<(String, DataSource)> {
    let mut sources: Vec<(String, DataSource)> = Vec::new();
    // 已关闭监控的工具（enabled=0）：跳过其数据源，停止采集新事件
    // （历史数据保留；自定义应用在 load_custom_app_paths 内已按 enabled=1 过滤）。
    let disabled = load_disabled_tool_ids(&state.db.conn);
    let is_disabled = |tool_id: &str| tool_id != "tokscale_aggregate" && disabled.contains(tool_id);
    // 自定义应用（tool_id = custom:*）：与注册表适配器并列，始终采集
    // （tokscale 聚合引擎不覆盖用户自注册的应用，不参与其退场判定）。
    for (tool_id, path) in custom_app::load_custom_app_paths(&state.db.conn) {
        let id = format!("{tool_id}:custom:{}", hash_path(&path));
        let format = infer_format(&path);
        sources.push((
            tool_id.clone(),
            DataSource {
                id,
                path,
                format,
                watch: true,
            },
        ));
    }
    // 用进程级缓存的定位(只探测一次子进程),避免每个 30s 轮询都 spawn --version
    if crate::token_monitor::collector::tokscale::locate_cached().is_some() {
        for adapter in AdapterRegistry::all() {
            let tool_id = adapter.descriptor().tool_id.clone();
            // tokscale 是权威源：聚合本身 + tokscale 未覆盖的 companion 工具（如 atomcode）
            // 并行采集（covers_tool 判定，避免双算）；覆盖清单内的手写适配器退场。
            if tool_id == "tokscale_aggregate"
                || !crate::token_monitor::collector::tokscale::covers_tool(&tool_id)
            {
                if is_disabled(&tool_id) {
                    continue;
                }
                for source in adapter.discover() {
                    sources.push((tool_id.clone(), source));
                }
            }
        }
        return sources;
    }
    for adapter in AdapterRegistry::all() {
        let tool_id = adapter.descriptor().tool_id.clone();
        if is_disabled(&tool_id) {
            continue;
        }
        for source in adapter.discover() {
            sources.push((tool_id.clone(), source));
        }
        for path in load_custom_paths(&state.db.conn, &tool_id) {
            sources.push((
                tool_id.clone(),
                DataSource {
                    id: format!("{}:custom:{}", tool_id, hash_path(&path)),
                    path: path.clone(),
                    format: infer_format(&path),
                    watch: true,
                },
            ));
        }
    }
    sources
}

/// tool_definition 中 enabled=0 的 tool_id 集合（关闭监控 = 停止采集新事件）。
fn load_disabled_tool_ids(conn: &std::sync::Mutex<rusqlite::Connection>) -> HashSet<String> {
    let Ok(conn) = conn.lock() else {
        return HashSet::new();
    };
    let ids = conn
        .prepare("SELECT tool_id FROM tool_definition WHERE enabled=0")
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(0))
                .ok()
                .and_then(|rows| rows.collect::<Result<HashSet<_>, _>>().ok())
        })
        .unwrap_or_default();
    drop(conn);
    ids
}

/// 自定义路径覆盖（tool_definition.custom_paths_json）。
fn load_custom_paths(conn: &std::sync::Mutex<rusqlite::Connection>, tool_id: &str) -> Vec<PathBuf> {
    let Ok(conn) = conn.lock() else {
        return Vec::new();
    };
    let json: Option<String> = conn
        .query_row(
            "SELECT custom_paths_json FROM tool_definition WHERE tool_id=?1",
            rusqlite::params![tool_id],
            |row| row.get(0),
        )
        .ok();
    drop(conn);
    json.and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
        .unwrap_or_default()
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

/// 路径不可逆 hash（隐私红线：DB 只存 hash，不存明文路径）。
pub(crate) fn hash_path(path: &PathBuf) -> String {
    let digest = sha2::Sha256::digest(path.to_string_lossy().as_bytes());
    hex::encode(&digest[..12])
}

fn infer_format(path: &PathBuf) -> DataFormat {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "jsonl" | "ndjson" => DataFormat::Jsonl,
        "json" => DataFormat::Json,
        "sqlite" | "db" | "sqlite3" => DataFormat::Sqlite,
        _ => DataFormat::Jsonl,
    }
}

// ---------------------------------------------------------------------------
// 监听 / 轮询循环
// ---------------------------------------------------------------------------

fn spawn_watch_loop(app: tauri::AppHandle, state: Arc<AppState>) {
    std::thread::spawn(move || {
        // 监听集合变化时重建 watcher（scan/路径变更后生效）
        let mut watched: HashSet<PathBuf> = HashSet::new();
        let mut watch: Option<FileWatch> = None;

        loop {
            let sources = refresh_sources(&state);
            let targets: HashSet<PathBuf> = sources
                .iter()
                .filter(|(_, source)| source.watch)
                .map(|(_, source)| source.path.clone())
                .collect();
            // 首次建表失败、目标目录暂不存在或文件系统临时不可用时必须重试；此前
            // 失败后 targets 不变便会持续停在 None，只能等用户手动刷新。
            if targets != watched || watch.is_none() {
                watched = targets.clone();
                watch = match FileWatch::watch(&sources) {
                    Ok(w) => Some(w),
                    Err(error) => {
                        tracing::warn!("token_monitor: watcher rebuild failed: {error}");
                        None
                    }
                };
            }
            let Some(watch) = watch.as_ref() else {
                std::thread::sleep(Duration::from_secs(5));
                continue;
            };

            // 等第一条事件（3s 超时）：超时回到循环顶部重检 targets，
            // 新增/删除自定义应用、路径变更在几秒内生效，不必等下一次文件事件。
            let Some(first) = watch.recv_timeout(Duration::from_secs(3)) else {
                continue;
            };
            std::thread::sleep(Duration::from_millis(700));
            let mut pending = watch.drain();
            pending.push(first);
            let deduped = dedupe_pending(pending);
            for p in deduped {
                let sources = refresh_sources(&state);
                if let Some((tool_id, source)) = sources
                    .iter()
                    .find(|(tool, source)| *tool == p.tool_id && source.id == p.source_id)
                {
                    collect_source(&state, tool_id, source);
                }
            }
            let _ = &app; // 预留：采集状态事件可经 app.emit（后续 W5 告警用）
        }
    });
}

fn spawn_poll_loop(state: Arc<AppState>) {
    // 后台预热趋势缓存：`tokscale graph` 子进程约 30s，单独线程驱动，
    // 保证用户打开 Token Monitor 页/托盘时趋势数据已就绪、瞬时返回。
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(20));
        crate::token_monitor::collector::tokscale::warm_trend_cache();
    });
    // 对齐开源 Token Monitor 的秒级刷新：独立于 5 分钟全量扫描节流，每 10s
    // 快速重扫一次 `tokscale --today`（函数内部再按 10s 节流合并）并立即重绘
    // 菜单栏/托盘「今日 Tokens」。开源版由 chokidar 文件事件驱动（1.5s 去抖 +
    // 快速 today 扫描）；这里用等价的 10s 轮询，today 数字不再等 5 分钟全量扫描。
    let today_state = state.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(10));
        crate::token_monitor::collector::tokscale::refresh_today_snapshot(&today_state.db.conn);
        if let Some(app) = crate::token_monitor::service::app_handle() {
            let _ = crate::services::menu_bar::apply_menu_bar(app, &today_state);
        }
    });
    std::thread::spawn(move || loop {
        // 实时路径由原生文件监听在 700ms 去抖后触发；这里是增量安全回补，用于
        // 原子轮转、文件系统漏事件和暂时不可监听目录，确保不会等待数分钟。
        std::thread::sleep(Duration::from_secs(20));
        let sources = refresh_sources(&state);
        for (tool_id, source) in &sources {
            collect_source(&state, tool_id, source);
        }
    });
}

fn dedupe_pending(pending: Vec<PendingSource>) -> Vec<PendingSource> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for p in pending {
        let key = format!("{}:{}", p.tool_id, p.source_id);
        if seen.insert(key) {
            out.push(p);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 采集执行（单 Adapter 隔离）
// ---------------------------------------------------------------------------

/// DSH v2 一次性迁移：删除 v1（cache 增量口径）的旧 usage_event 并重建 rollup。
/// 幂等：settings 标记 `tm.adapter.dsh.v2` 存在即跳过。返回是否本次执行了迁移
/// （调用方据此在新事件落库后再全量重建一次 rollup，避免增量更新残留旧值）。
pub(crate) fn migrate_dsh_v1_to_v2(state: &Arc<AppState>) -> Result<bool, String> {
    use crate::db::settings::SettingsRepo;
    if SettingsRepo
        .get(&state.db.conn, "tm.adapter.dsh.v2")
        .ok()
        .flatten()
        .is_some()
    {
        return Ok(false);
    }
    {
        let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM usage_event WHERE tool_id='dsh' AND source_type='local_discovered'",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    state.db.usage_events.rebuild_rollup(&state.db.conn)?;
    SettingsRepo
        .set(&state.db.conn, "tm.adapter.dsh.v2", "1")
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// 对单个数据源执行增量采集（生产入口：从注册表解析 Adapter）。
/// 任何错误都映射为状态，不 panic、不波及其他源。
pub(crate) fn collect_source(state: &Arc<AppState>, tool_id: &str, source: &DataSource) {
    // 自定义应用：动态构造适配器，复用标准采集/落库/事件广播流程。
    if custom_app::is_custom_app(tool_id) {
        let Some(adapter) = custom_app::adapter_for(&state.db.conn, tool_id) else {
            return;
        };
        collect_with_adapter(state, &adapter, tool_id, source);
        return;
    }
    let adapter = AdapterRegistry::all()
        .into_iter()
        .find(|adapter| adapter.descriptor().tool_id == tool_id);
    let Some(adapter) = adapter else {
        return;
    };
    collect_with_adapter(state, adapter.as_ref(), tool_id, source);
}

/// 内部/测试入口：用显式 Adapter 执行一次增量采集（checkpoint → adapter → 落库 → 状态）。
pub(crate) fn collect_with_adapter(
    state: &Arc<AppState>,
    adapter: &dyn ToolAdapter,
    tool_id: &str,
    source: &DataSource,
) {
    let key = source.id.clone();
    {
        let Ok(mut guard) = in_flight().lock() else {
            return;
        };
        if !guard.insert(key.clone()) {
            return; // 正在采集，跳过重入
        }
    }
    // 注册静态描述（状态更新才有行可写；幂等 upsert）
    let _ = upsert_definition(state, adapter);
    update_status(&state.db.conn, tool_id, CollectorStatus::Active, None);

    let checkpoint = checkpoint::load(&state.db.conn, &source.id);
    let result = adapter.collect_incremental(source, checkpoint);
    match result {
        Ok(collect_result) => {
            checkpoint::save(&state.db.conn, &collect_result.next_checkpoint).ok();
            // DSH v2 迁移（一次性）：cache 口径从「增量」改为「每请求原始值」（对齐 xiaomi
            // 官方用量）。指纹含 cache 值，新旧口径指纹不同，旧行不清理会导致双算——
            // 在 v2 首次采集落库前清掉 v1 旧事件；落库后全量重建 rollup。
            let mut dsh_migrated = false;
            if tool_id == "dsh" {
                dsh_migrated = migrate_dsh_v1_to_v2(&state).unwrap_or(false);
            }
            // 已关闭监控（enabled=0）的工具：事件不再落库（统计层也已剔除其历史数据）。
            // 对 tokscale 聚合扫描尤其必要——它一次性产出全部 covered 工具的事件。
            let disabled = load_disabled_tool_ids(&state.db.conn);
            let events: Vec<crate::token_monitor::model::NormalizedUsageEvent> =
                collect_result
                    .events
                    .into_iter()
                    .filter(|e| !disabled.contains(&e.tool_id))
                    .collect();
            // tokscale 聚合引擎 = 快照替换语义(清空旧本地事件防双算);其余适配器增量幂等。
            // 注意:仅当本次确实产出了事件(未命中节流跳过)才替换——空快照绝不能清空已有数据。
            let is_tokscale = tool_id == "tokscale_aggregate";
            let covered = crate::token_monitor::collector::tokscale::covered_tool_ids();
            let inserted = if is_tokscale && !events.is_empty() {
                let n = state
                    .db
                    .usage_events
                    .replace_local_snapshot(&state.db.conn, &events, &covered)
                    .unwrap_or(0);
                // 同步刷新权威 period 快照(day/month),供统计查询直接读 tokscale 口径
                crate::token_monitor::collector::tokscale::refresh_period_snapshots(&state.db.conn);
                // W8：tokscale 权威模式会话投影——聚合快照生成 tm_session，让 tokscale
                // 覆盖的工具也出现在会话列表（tokscale 适配器本身不产出 sessions）。
                // 只投影 covered 工具；companion 工具的会话仍由各自手写适配器产出。
                let covered_refs: Vec<&str> =
                    covered.iter().map(|s| s.as_str()).collect();
                match state
                    .db
                    .sessions
                    .project_from_usage_events(&state.db.conn, &covered_refs)
                {
                    Ok(projected) => {
                        for session in &projected {
                            state.token_monitor.publish_session_changed(session.clone());
                        }
                    }
                    Err(error) => tracing::warn!(
                        "token_monitor: session projection failed: {error}"
                    ),
                }
                n
            } else if is_tokscale {
                0 // 节流跳过:保留现有快照,不清空
            } else {
                state
                    .db
                    .usage_events
                    .insert_batch(&state.db.conn, &events)
                    .unwrap_or(0)
            };
            // DSH v2 迁移：新事件落库后全量重建 rollup（一次性），
            // 避免增量更新在「删旧→插新」序列下残留旧口径值导致趋势/今日不一致。
            if dsh_migrated && inserted > 0 {
                let _ = state.db.usage_events.rebuild_rollup(&state.db.conn);
            }
            for session in &collect_result.sessions {
                let _ = state.db.sessions.upsert_session(&state.db.conn, session);
                // W7：会话新增/更新 → 广播 `token-monitor:session-changed`
                // （前端监听后实时失效会话列表/明细查询；无接收者（测试）时静默丢弃）
                state.token_monitor.publish_session_changed(session.clone());
            }
            if inserted > 0 {
                state.token_monitor.publish_usage_delta(state);
            }
            // 历史修复（自愈）：适配器模型逻辑变更后重扫会残留「旧指纹(无模型)+新指纹
            // (带模型)」双行 → TOKENS 双算 + 模型视图「未知模型」。按 source_locator_hash
            // 去重残留 + 回填缺模型行 + 重建 rollup（幂等，无待修快速短路）。
            let repaired = match tool_id {
                "freebuff" => {
                    crate::token_monitor::collector::freebuff::backfill_missing_models(
                        &state.db.conn,
                        source,
                    )
                }
                "atomcode" => {
                    crate::token_monitor::collector::atomcode::backfill_missing_models(
                        &state.db.conn,
                        source,
                    )
                }
                _ => Ok(0),
            }
            .unwrap_or(0);
            if repaired > 0 {
                state.token_monitor.publish_usage_delta(state);
            }
            update_status(&state.db.conn, tool_id, CollectorStatus::Idle, None);
        }
        Err(error) => {
            let status = error_to_status(&error);
            let error_text = error.to_string();
            update_status(&state.db.conn, tool_id, status, Some(error_text.clone()));
            // 采集失败告警（W5 collector_error）：错误状态族触发，emit + 后端直接发系统通知；
            // 去抖按 (tool, status) 30 分钟一次；测试环境无 AppHandle 时静默跳过。
            if let Some(app) = crate::token_monitor::service::app_handle() {
                if let Some(alert) = crate::token_monitor::alerts::evaluate_collector_alert(
                    &adapter.descriptor().display_name,
                    tool_id,
                    &status,
                    Some(&error_text),
                ) {
                    crate::token_monitor::alerts::publish_alert(app, alert);
                }
            }
            tracing::warn!(
                "token_monitor: collect '{}' ({}) failed: {}",
                tool_id,
                source.id,
                error
            );
        }
    }
    in_flight().lock().ok().map(|mut guard| guard.remove(&key));
}

/// 手动「立即刷新」：清空全部源 checkpoint 后强制全量重扫（托盘刷新按钮用，
/// 绕过 tokscale 5 分钟节流，让用户点一下就能看到最新数字）。
pub(crate) fn force_refresh_all(state: &Arc<AppState>) -> Result<Vec<ToolCollectorState>, String> {
    let sources = refresh_sources(state);
    for (_, source) in &sources {
        let _ = checkpoint::remove(&state.db.conn, &source.id);
    }
    scan_all_tools(state)
}

/// 手动重扫某工具的全部有效数据源（**全量** + 幂等：清 checkpoint 后重采，
/// 事件按 `source_fingerprint` 去重，不产生重复行）。用于修复历史数据被
/// 清空/漏采后一键恢复（如 W12 前 companion 工具事件被 tokscale 快照替换清掉）。
pub(crate) fn rescan_tool(state: &Arc<AppState>, tool_id: &str) {
    let sources = refresh_sources(state);
    for (tool, source) in &sources {
        if tool == tool_id {
            let _ = checkpoint::remove(&state.db.conn, &source.id);
            collect_source(state, tool_id, source);
        }
    }
}

/// 重置某工具的全部数据：删除 usage_event + tm_daily_rollup + checkpoint，然后重新采集。
/// 用于适配器逻辑变更后清理旧数据（如 DSH cacheReadTokens 从累计值改为增量）。
pub(crate) fn reset_tool_data(state: &Arc<AppState>, tool_id: &str) -> Result<(), String> {
    // 1. 删除 usage_event
    {
        let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM usage_event WHERE tool_id=?1",
            rusqlite::params![tool_id],
        )
        .map_err(|e| e.to_string())?;
    }
    // 2. 重建 tm_daily_rollup
    state.db.usage_events.rebuild_rollup(&state.db.conn)?;
    // 3. 清 checkpoint + 重新采集
    let sources = refresh_sources(state);
    for (tool, source) in &sources {
        if tool == tool_id {
            let _ = checkpoint::remove(&state.db.conn, &source.id);
            collect_source(state, tool_id, source);
        }
    }
    Ok(())
}

/// 新增自定义应用（自定义应用监控）：写 tool_definition 行 + 立即首采。
/// 返回新 tool_id（`custom:<slug>-<suffix>`，slug 来自应用名）。
pub(crate) fn add_custom_app(
    state: &Arc<AppState>,
    display_name: &str,
    paths: &[String],
    fields: Option<&crate::token_monitor::collector::custom_app::CustomFields>,
) -> Result<String, String> {
    let name = display_name.trim();
    if name.is_empty() {
        return Err("应用名称不能为空".into());
    }
    if name.chars().count() > 60 {
        return Err("应用名称过长（最多 60 字符）".into());
    }
    let paths: Vec<String> = paths
        .iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if paths.is_empty() {
        return Err("至少需要一个日志文件路径".into());
    }
    let tool_id = crate::token_monitor::collector::custom_app::make_tool_id(name);
    let fields_json = match fields {
        Some(fields) => Some(serde_json::to_string(fields).map_err(|e| e.to_string())?),
        None => None,
    };
    {
        let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO tool_definition \
                (tool_id, display_name, kind, support_level, custom_paths_json, custom_fields_json, enabled) \
             VALUES (?1,?2,'usage','basic',?3,?4,1) \
             ON CONFLICT(tool_id) DO UPDATE SET \
                display_name=excluded.display_name, custom_paths_json=excluded.custom_paths_json, \
                custom_fields_json=excluded.custom_fields_json, enabled=1, \
                updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')",
            rusqlite::params![
                tool_id,
                name,
                serde_json::to_string(&paths).map_err(|e| e.to_string())?,
                fields_json,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    // 立即首采（增量 + 幂等）：今日/托盘 Tokens 即刻可见，不等 10s 轮询。
    rescan_tool(state, &tool_id);
    Ok(tool_id)
}

/// 删除自定义应用：删 tool_definition 行（usage_event/tm_session 级联删除）+ 清 checkpoint。
pub(crate) fn remove_custom_app(state: &Arc<AppState>, tool_id: &str) -> Result<(), String> {
    if !crate::token_monitor::collector::custom_app::is_custom_app(tool_id) {
        return Err(format!("{tool_id} 不是自定义应用，不能删除"));
    }
    let sources = refresh_sources(state);
    for (tool, source) in &sources {
        if tool == tool_id {
            let _ = checkpoint::remove(&state.db.conn, &source.id);
        }
    }
    let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM tool_definition WHERE tool_id=?1",
        rusqlite::params![tool_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn error_to_status(error: &CollectorError) -> CollectorStatus {
    match error {
        CollectorError::PathMissing(_) => CollectorStatus::PathMissing,
        CollectorError::Permission(_) => CollectorStatus::Permission,
        CollectorError::FormatChanged(_) => CollectorStatus::FormatChanged,
        CollectorError::Io(_) | CollectorError::Parse(_) => CollectorStatus::Error,
    }
}

fn status_str(status: CollectorStatus) -> String {
    serde_json::to_string(&status)
        .map(|s| s.trim_matches('"').to_string())
        .unwrap_or_else(|_| "idle".into())
}

fn update_status(
    conn: &std::sync::Mutex<rusqlite::Connection>,
    tool_id: &str,
    status: CollectorStatus,
    error: Option<String>,
) {
    let Ok(conn) = conn.lock() else {
        return;
    };
    let _ = conn.execute(
        "UPDATE tool_definition SET collector_status=?1, collector_error=?2,
         last_collected_at=strftime('%Y-%m-%dT%H:%M:%SZ','now'),
         updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
         WHERE tool_id=?3",
        rusqlite::params![status_str(status), error, tool_id],
    );
}

// ---------------------------------------------------------------------------
// 命令支撑：scan / status / paths
// ---------------------------------------------------------------------------

/// 首次「扫描本机」：为每个注册 Adapter 写入/更新 tool_definition，并立即增量采集。
pub(crate) fn scan_all_tools(state: &Arc<AppState>) -> Result<Vec<ToolCollectorState>, String> {
    for adapter in AdapterRegistry::all() {
        upsert_definition(state, adapter.as_ref())?;
    }
    // 对刚发现的源做一次初始增量采集（从 checkpoint 恢复，幂等）
    let sources = refresh_sources(state);
    for (tool_id, source) in &sources {
        collect_source(state, tool_id, source);
    }
    collector_states(state)
}

/// 注册/更新单个工具静态描述。
pub(crate) fn upsert_definition(
    state: &Arc<AppState>,
    adapter: &dyn ToolAdapter,
) -> Result<(), String> {
    let descriptor = adapter.descriptor();
    let capabilities = adapter.capabilities();
    let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO tool_definition (
            tool_id, display_name, vendor, kind, adapter_version, capabilities_json,
            supported_os_json, support_level, enabled
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1)
         ON CONFLICT(tool_id) DO UPDATE SET
            display_name=excluded.display_name, vendor=excluded.vendor,
            kind=excluded.kind, adapter_version=excluded.adapter_version,
            capabilities_json=excluded.capabilities_json,
            supported_os_json=excluded.supported_os_json,
            support_level=excluded.support_level,
            updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')",
        rusqlite::params![
            descriptor.tool_id,
            descriptor.display_name,
            descriptor.vendor,
            serde_json::to_string(&descriptor.kind)
                .map(|s| s.trim_matches('"').to_string())
                .unwrap_or_else(|_| "usage".into()),
            descriptor.adapter_version as i64,
            serde_json::to_string(&capabilities).unwrap_or_else(|_| "{}".into()),
            serde_json::to_string(&descriptor.supported_os).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&descriptor.support_level)
                .map(|s| s.trim_matches('"').to_string())
                .unwrap_or_else(|_| "basic".into()),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 从 tool_definition 读取全部采集状态（paths 为默认+自定义的有效路径）。
pub(crate) fn collector_states(state: &Arc<AppState>) -> Result<Vec<ToolCollectorState>, String> {
    let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT tool_id, display_name, enabled, support_level, collector_status,
                    last_collected_at, collector_error, COALESCE(custom_paths_json,'')
             FROM tool_definition ORDER BY tool_id",
        )
        .map_err(|e| e.to_string())?;
    let mut rows: Vec<(
        String,
        String,
        i64,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
    )> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    // 默认路径（来自注册表 discover），用于展示
    let default_paths: HashMap<String, Vec<String>> = AdapterRegistry::all()
        .into_iter()
        .map(|adapter| {
            let descriptor = adapter.descriptor();
            let paths = adapter
                .discover()
                .into_iter()
                .map(|source| source.path.to_string_lossy().to_string())
                .collect();
            (descriptor.tool_id, paths)
        })
        .collect();

    Ok(rows
        .drain(..)
        .map(
            |(
                tool_id,
                display_name,
                enabled,
                support_level,
                status,
                last_collected_at,
                error,
                custom_json,
            )| {
                let custom: Vec<String> = serde_json::from_str(&custom_json).unwrap_or_default();
                let mut paths = if custom.is_empty() {
                    default_paths.get(&tool_id).cloned().unwrap_or_default()
                } else {
                    custom
                };
                paths.sort();
                ToolCollectorState {
                    tool_id,
                    display_name,
                    enabled: enabled != 0,
                    support_level: serde_json::from_str(&format!("\"{support_level}\""))
                        .unwrap_or(crate::token_monitor::model::SupportLevel::Basic),
                    status: serde_json::from_str(&format!("\"{status}\""))
                        .unwrap_or(CollectorStatus::Idle),
                    last_collected_at,
                    error,
                    paths,
                }
            },
        )
        .collect())
}

// ---------------------------------------------------------------------------
// 测试（用假 Adapter 打通「数据源变更 → 采集 → 落库」）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use crate::token_monitor::model::{
        AdapterCapabilities, CollectorCheckpoint, DataFormat, IncrementalMode, SourceType,
        SupportLevel, ToolDescriptor, ToolKind, UsageAccuracy,
    };

    /// 极简假 Adapter：解析 JSONL（每行 {"input":n,"output":m,"model":s}），byte_offset 增量。
    #[derive(Default)]
    struct FakeAdapter;

    impl ToolAdapter for FakeAdapter {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor {
                tool_id: "fake_tool".into(),
                display_name: "Fake Tool".into(),
                vendor: Some("test".into()),
                kind: ToolKind::Usage,
                support_level: SupportLevel::Experimental,
                supported_os: vec!["macos".into(), "linux".into()],
                adapter_version: 1,
                privacy_note: "读取测试 JSONL 元数据".into(),
            }
        }

        fn capabilities(&self) -> AdapterCapabilities {
            AdapterCapabilities {
                token: true,
                model: true,
                session: false,
                project: false,
                cache_tokens: false,
                cost: false,
                accuracy: UsageAccuracy::Exact,
                incremental: IncrementalMode::FileOffset,
            }
        }

        fn discover(&self) -> Vec<DataSource> {
            Vec::new()
        }

        fn checkpoint(&self, source_id: &str) -> CollectorCheckpoint {
            CollectorCheckpoint {
                source_id: source_id.into(),
                ..Default::default()
            }
        }

        fn collect_incremental(
            &self,
            source: &DataSource,
            checkpoint: CollectorCheckpoint,
        ) -> Result<crate::token_monitor::collector::CollectResult, CollectorError> {
            use std::io::{BufRead, BufReader};
            let file = std::fs::File::open(&source.path).map_err(|e| {
                CollectorError::Io(format!("open {}: {}", source.path.display(), e))
            })?;
            let reader = BufReader::new(file);
            let offset = checkpoint.byte_offset.unwrap_or(0) as usize;
            let mut events = Vec::new();
            let mut cursor = 0usize;
            for line in reader.lines() {
                let Ok(line) = line else { break };
                cursor += line.len() + 1;
                if cursor <= offset {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                    events.push(crate::token_monitor::model::NormalizedUsageEvent {
                        source_type: SourceType::LocalDiscovered,
                        tool_id: "fake_tool".into(),
                        device_id: "local".into(),
                        model_raw: value
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string),
                        model_normalized: value
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string),
                        session_id: None,
                        project_id: None,
                        account_id: None,
                        input_tokens: value.get("input").and_then(|v| v.as_i64()),
                        output_tokens: value.get("output").and_then(|v| v.as_i64()),
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                        reasoning_tokens: None,
                        message_count: None,
                        session_started_at: None,
                        session_last_active_at: None,
                        total_tokens: None,
                        cost_amount: None,
                        cost_currency: None,
                        usage_accuracy: UsageAccuracy::Exact,
                        occurred_at: value
                            .get("ts")
                            .and_then(|v| v.as_str())
                            .unwrap_or("2026-08-08T00:00:00Z")
                            .to_string(),
                        source_locator_hash: None,
                    });
                }
            }
            let next = CollectorCheckpoint {
                source_id: checkpoint.source_id.clone(),
                byte_offset: Some(cursor as u64),
                ..Default::default()
            };
            Ok(crate::token_monitor::collector::CollectResult {
                events,
                sessions: Vec::new(),
                next_checkpoint: next,
            })
        }
    }

    fn test_db() -> (tempfile::TempDir, std::path::PathBuf, Arc<AppState>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("test.db");
        let database = crate::db::Database::new(&db_path).expect("open database");
        database.run_migrations().expect("run migrations");
        let state = Arc::new(AppState {
            db: database,
            gateway_access_key: std::sync::RwLock::new(None),
            gateway_access_key_loaded: std::sync::atomic::AtomicBool::new(false),
            proxy: std::sync::Mutex::new(None),
            gateway_runtime: crate::proxy::runtime::GatewayRuntime::default(),
            account_concurrency: crate::proxy::concurrency::AccountConcurrency::default(),
            account_throttle: crate::proxy::concurrency::AccountThrottle::default(),
            agent_app_operations: std::sync::Mutex::new(std::collections::HashSet::new()),
            app_data_dir: Some(dir.path().to_path_buf()),
            token_monitor: crate::token_monitor::TokenMonitorRuntime::default(),
        });
        (dir, db_path, state)
    }

    #[test]
    fn collect_source_inserts_events_and_advances_checkpoint() {
        // 注册假 Adapter 到全局注册表（测试专用：直接临时替换 all() 不可行，
        // 这里通过一个本地 adapter 手动驱动 collect_source 的等价路径）。
        // 改为直接验证：insert 幂等 + checkpoint 恢复（用 FakeAdapter 走完整链路）。
        let (_dir, _db_path, state) = test_db();
        let file =
            std::env::temp_dir().join(format!("fake-{}.jsonl", uuid::Uuid::new_v4().simple()));
        std::fs::write(
            &file,
            "{\"input\":100,\"output\":50,\"model\":\"fake-1\",\"ts\":\"2026-08-08T00:00:00Z\"}\n\
             {\"input\":10,\"output\":5,\"model\":\"fake-1\",\"ts\":\"2026-08-08T00:00:01Z\"}\n",
        )
        .expect("write fixture");
        let source = DataSource {
            id: "fake:src".into(),
            path: file.clone(),
            format: DataFormat::Jsonl,
            watch: false,
        };

        let adapter = FakeAdapter;
        collect_with_adapter(&state, &adapter, "fake_tool", &source);

        let (input, output, _cache, total) = state
            .db
            .usage_events
            .unified_range_stats(&state.db.conn, "total")
            .expect("stats");
        assert_eq!((input, output, total), (110, 55, 165));

        // 追加一行 → 增量只采新行
        std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .expect("open append")
            .write_all(b"{\"input\":7,\"output\":3,\"model\":\"fake-1\",\"ts\":\"2026-08-08T00:00:02Z\"}\n")
            .expect("append");
        collect_with_adapter(&state, &adapter, "fake_tool", &source);
        let (input, output, _cache, total) = state
            .db
            .usage_events
            .unified_range_stats(&state.db.conn, "total")
            .expect("stats 2");
        assert_eq!((input, output, total), (117, 58, 175));
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn error_maps_to_status_without_panic() {
        let (_dir, _db_path, state) = test_db();
        let missing = DataSource {
            id: "fake:missing".into(),
            path: std::path::PathBuf::from("/nonexistent/definitely-missing.jsonl"),
            format: DataFormat::Jsonl,
            watch: false,
        };
        let adapter = FakeAdapter;
        collect_with_adapter(&state, &adapter, "fake_tool", &missing);
        // 状态应落为 error/path_missing，且全局不 panic（此处即通过）
        let states = collector_states(&state).expect("states");
        assert!(
            states
                .iter()
                .any(|s| s.tool_id == "fake_tool" && s.error.is_some()),
            "tool should carry an error state"
        );
    }
}
