//! 通用本机 Agent 工具扫描（「一键扫描添加」，W12）。
//!
//! 用户需求：选择本机安装的 Agent 工具，自动检测适配器后一键加入 TOKENS 监控，
//! 形成 tokscale 聚合 + 自定义工具的汇总 TOKENS 量。
//!
//! - `detect_local_agents`：扫描注册表适配器（`discover()` 数据源 + CLI 二进制）
//!   以及 tokscale 覆盖但无手写适配器的工具（gemini/trae/trae_solo），报告每个
//!   工具的安装/数据/适配器/监控状态，供前端勾选。
//! - `enable_tool_monitoring`：对所选工具注册定义行 + 置 enabled=1 + 立即增量采集，
//!   新工具的用量即刻计入今日/托盘 TOKENS 汇总（与 tokscale + 自定义同一链路）。

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use crate::token_monitor::collector::tokscale;
use crate::token_monitor::collector::AdapterRegistry;
use crate::token_monitor::model::{DetectedAgent, ToolCollectorState};
use crate::AppState;

/// tokscale 覆盖但无手写适配器的工具（检测用元数据）。
/// (tool_id, display_name, vendor, cli_names, home 下数据目录)
const COVERED_NO_ADAPTER: &[(&str, &str, Option<&str>, &[&str], &[&str])] = &[
    ("gemini", "Gemini CLI", Some("Google"), &["gemini"], &[".gemini", ".config/gemini"]),
    ("trae", "Trae", Some("ByteDance"), &["trae"], &[".trae"]),
    ("trae_solo", "Trae Solo", Some("ByteDance"), &[], &[".trae-solo"]),
];

/// 注册表适配器的 CLI 二进制提示（`discover()` 已覆盖数据目录；这里补
/// 「已安装但尚无数据」的检测信号，用于诚实提示可添加）。
const ADAPTER_CLI: &[(&str, &[&str])] = &[
    ("antigravity", &["antigravity"]),
    ("atomcode", &["atom"]),
    ("claude_code", &["claude"]),
    ("cline", &["cline"]),
    ("codebuddy", &["codebuddy"]),
    ("codex", &["codex"]),
    ("cursor", &["cursor"]),
    ("freebuff", &["freebuff"]),
    ("github_copilot", &["copilot"]),
    ("grok_build", &["grok"]),
    ("hermes", &["hermes"]),
    ("kilo_code", &["kilo"]),
    ("kimi", &["kimi"]),
    ("kiro", &["kiro"]),
    ("mimo", &["mimo", "micode"]),
    ("openclaw", &["openclaw"]),
    ("opencode", &["opencode"]),
    ("pi", &["pi"]),
    ("proma", &["proma"]),
    ("qwen", &["qwen"]),
    ("workbuddy", &["workbuddy"]),
    ("zcode", &["zcode"]),
    ("zed", &["zed"]),
];

/// 扫描本机：返回全部已知 Agent 工具的安装/数据/监控状态（排序：已监控 → 有数据 → 已安装 → 未安装）。
pub(crate) fn detect_local_agents(state: &Arc<AppState>) -> Result<Vec<DetectedAgent>, String> {
    let enabled = enabled_tool_ids(state);
    let tokscale_available = tokscale::locate_cached().is_some();
    let mut out: Vec<DetectedAgent> = Vec::new();

    // 1) 注册表适配器（含 tokscale 聚合引擎）
    for adapter in AdapterRegistry::all() {
        let descriptor = adapter.descriptor();
        if descriptor.tool_id == "tokscale_aggregate" {
            let installed = tokscale_available;
            out.push(DetectedAgent {
                tool_id: descriptor.tool_id.clone(),
                display_name: descriptor.display_name.clone(),
                vendor: descriptor.vendor.clone(),
                has_adapter: true,
                installed,
                data_found: installed,
                monitored: enabled.contains(&descriptor.tool_id),
                covered_by_tokscale: false,
                tokscale_available,
                cli: tokscale::locate_cached().map(|p| p.to_string_lossy().to_string()),
                data_sources: vec![],
            });
            continue;
        }
        let sources = adapter.discover();
        let data_found = !sources.is_empty();
        let cli = cli_hint(&descriptor.tool_id);
        out.push(DetectedAgent {
            tool_id: descriptor.tool_id.clone(),
            display_name: descriptor.display_name.clone(),
            vendor: descriptor.vendor.clone(),
            has_adapter: true,
            installed: cli.is_some() || data_found,
            data_found,
            monitored: enabled.contains(&descriptor.tool_id),
            covered_by_tokscale: tokscale::covers_tool(&descriptor.tool_id),
            tokscale_available,
            cli,
            data_sources: sources
                .iter()
                .take(3)
                .map(|source| source.path.to_string_lossy().to_string())
                .collect(),
        });
    }

    // 2) tokscale 覆盖但无手写适配器的工具
    for (tool_id, display_name, vendor, clis, dirs) in COVERED_NO_ADAPTER {
        let cli = find_cli(clis);
        let dirs_found = find_home_dirs(dirs);
        out.push(DetectedAgent {
            tool_id: tool_id.to_string(),
            display_name: display_name.to_string(),
            vendor: vendor.map(str::to_string),
            has_adapter: false,
            installed: cli.is_some() || !dirs_found.is_empty(),
            data_found: !dirs_found.is_empty(),
            monitored: enabled.contains(*tool_id),
            covered_by_tokscale: true,
            tokscale_available,
            cli,
            data_sources: dirs_found,
        });
    }

    out.sort_by(|a, b| {
        let rank = |x: &DetectedAgent| {
            if x.monitored {
                0
            } else if x.data_found {
                1
            } else if x.installed {
                2
            } else {
                3
            }
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    Ok(out)
}

/// 一键添加：为所选工具注册定义行 + enabled=1 + 立即增量采集。
/// 返回更新后的采集状态列表（前端据此刷新「Token 监控应用」分区）。
pub(crate) fn enable_tool_monitoring(
    state: &Arc<AppState>,
    tool_ids: &[String],
) -> Result<Vec<ToolCollectorState>, String> {
    if tool_ids.is_empty() {
        return Err("请先勾选要添加的工具".into());
    }
    let tokscale_available = tokscale::locate_cached().is_some();

    // 先校验全部 tool_id 已知，再逐个注册（避免部分成功部分失败的不一致）
    let registry_ids: HashSet<String> = AdapterRegistry::all()
        .iter()
        .map(|a| a.descriptor().tool_id.clone())
        .collect();
    for id in tool_ids {
        if !registry_ids.contains(id)
            && !COVERED_NO_ADAPTER.iter().any(|(tid, ..)| *tid == id)
        {
            return Err(format!("「{id}」没有可用的采集适配器，无法加入监控"));
        }
    }

    let mut rescan_ids: Vec<String> = Vec::new();
    for id in tool_ids {
        // 注册表适配器：完整描述 upsert + enabled=1
        if registry_ids.contains(id) {
            if let Some(adapter) = AdapterRegistry::all()
                .into_iter()
                .find(|a| a.descriptor().tool_id == *id)
            {
                crate::token_monitor::service_collect::upsert_definition(state, adapter.as_ref())?;
                set_enabled(state, id, true)?;
                if id != "tokscale_aggregate" {
                    rescan_ids.push(id.clone());
                }
                continue;
            }
        }
        // tokscale 覆盖但无手写适配器：仅 tokscale 模式可采
        if let Some((_tid, display_name, ..)) = COVERED_NO_ADAPTER
            .iter()
            .find(|(tid, ..)| *tid == id.as_str())
        {
            if !tokscale_available {
                return Err(format!(
                    "「{display_name}」由 tokscale 聚合引擎采集，但本机未检测到 tokscale（可设置 TOKSCALE_BIN 或安装 tokscale）"
                ));
            }
            let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO tool_definition (tool_id, display_name, kind, support_level, enabled) \
                 VALUES (?1,?2,'usage','basic',1) \
                 ON CONFLICT(tool_id) DO UPDATE SET \
                    display_name=excluded.display_name, enabled=1, \
                    updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')",
                rusqlite::params![id, display_name],
            )
            .map_err(|e| e.to_string())?;
            continue;
        }
    }

    // 立即首采（增量 + 幂等）：新工具今日/托盘 Tokens 即刻可见，不等 10s 轮询
    for id in &rescan_ids {
        crate::token_monitor::service_collect::rescan_tool(state, id);
    }
    crate::token_monitor::service_collect::collector_states(state)
}

// ---------------------------------------------------------------------------
// 内部辅助
// ---------------------------------------------------------------------------

/// tool_definition 中 enabled=1 的 tool_id 集合。
fn enabled_tool_ids(state: &Arc<AppState>) -> HashSet<String> {
    let Ok(conn) = state.db.conn.lock() else {
        return HashSet::new();
    };
    let mut stmt = match conn.prepare("SELECT tool_id FROM tool_definition WHERE enabled=1") {
        Ok(stmt) => stmt,
        Err(_) => return HashSet::new(),
    };
    let ids = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .ok()
        .and_then(|rows| rows.collect::<Result<HashSet<_>, _>>().ok())
        .unwrap_or_default();
    drop(stmt);
    ids
}

fn set_enabled(state: &Arc<AppState>, tool_id: &str, enabled: bool) -> Result<(), String> {
    let conn = state.db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE tool_definition SET enabled=?1, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE tool_id=?2",
        rusqlite::params![enabled as i64, tool_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 查表取某工具应探测的 CLI 二进制名。
fn cli_hint(tool_id: &str) -> Option<String> {
    let names = ADAPTER_CLI
        .iter()
        .find(|(tid, _)| *tid == tool_id)
        .map(|(_, names)| *names)?;
    find_cli(names)
}

/// 在 PATH + 常见安装目录里找二进制（Windows 自动补 .exe）。
fn find_cli(names: &[&str]) -> Option<String> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(path_var) = std::env::var("PATH") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for dir in path_var.split(sep) {
            if !dir.is_empty() {
                dirs.push(PathBuf::from(dir));
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for sub in [".local/bin", ".bin", "bin"] {
            dirs.push(home.join(sub));
        }
    }
    #[cfg(target_os = "macos")]
    dirs.extend([PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")]);
    #[cfg(target_os = "linux")]
    dirs.extend([PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")]);

    for dir in dirs {
        for name in names {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p.to_string_lossy().to_string());
            }
            #[cfg(windows)]
            {
                let p = dir.join(format!("{name}.exe"));
                if p.is_file() {
                    return Some(p.to_string_lossy().to_string());
                }
            }
        }
    }
    None
}

/// 检查 home 下数据目录是否存在，返回存在的路径列表。
fn find_home_dirs(dirs: &[&str]) -> Vec<String> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    dirs.iter()
        .filter_map(|dir| {
            let p = home.join(dir);
            p.exists().then(|| p.to_string_lossy().to_string())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_hint_known_tools() {
        // 已知工具应有 CLI 提示表项（返回 None 或 Some 均合法，取决于本机是否安装）
        for (tool_id, names) in ADAPTER_CLI {
            assert!(!names.is_empty(), "{tool_id} 缺少 CLI 提示");
            let found = find_cli(names);
            // 只要探测不 panic 即可；是否命中取决于 CI 环境
            let _ = found;
        }
        for (tool_id, display_name, _, clis, dirs) in COVERED_NO_ADAPTER {
            assert!(!display_name.is_empty());
            let _ = find_cli(clis);
            let _ = find_home_dirs(dirs);
            assert!(!tool_id.is_empty());
        }
    }

    #[test]
    fn find_home_dirs_returns_only_existing() {
        let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
        // 不存在目录返回空
        let missing = find_home_dirs(&["__definitely_missing__"]);
        assert!(missing.is_empty());
        // 存在目录（home 本身）返回 1 条
        let existing = find_home_dirs(&[""]);
        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0], home.join("").to_string_lossy().to_string());
    }

    #[test]
    fn enable_tool_monitoring_rejects_unknown_tool() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = crate::db::Database::new(&dir.path().join("test.db")).expect("open database");
        database.run_migrations().expect("migrate");
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
        let err = enable_tool_monitoring(&state, &["not_a_real_tool".into()]).unwrap_err();
        assert!(err.contains("没有可用的采集适配器"));
        // 空列表也报错
        let err = enable_tool_monitoring(&state, &[]).unwrap_err();
        assert!(err.contains("勾选"));
    }

    #[test]
    fn enable_tool_monitoring_upserts_and_enables_registry_tool() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = crate::db::Database::new(&dir.path().join("test.db")).expect("open database");
        database.run_migrations().expect("migrate");
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
        let states = enable_tool_monitoring(&state, &["claude_code".into()]).expect("enable");
        let row = states
            .iter()
            .find(|s| s.tool_id == "claude_code")
            .expect("row created");
        assert!(row.enabled);
        assert_eq!(row.display_name, "Claude Code");
    }
}
