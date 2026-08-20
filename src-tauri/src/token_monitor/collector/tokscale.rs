//! tokscale 聚合采集器 —— 复刻开源 Token Monitor 的采集引擎。
//!
//! 开源 Token Monitor 不自研各工具格式解析器,而是调用外部聚合引擎 `tokscale`
//! (https://github.com/jasonkang/tokscale, npm `tokscale`, 内置 36+ 工具的真实格式解析)。
//! 本适配器照搬该架构:发现本机 tokscale 二进制 → 全量扫描
//! `tokscale --json --group-by client,session,model --since <checkpoint>` → 把每个
//! (client, session, model) 条目映射为一条 `NormalizedUsageEvent` 落库。
//!
//! 口径对齐开源版 (src/shared/usage.js):
//! - `total_tokens = input + output + cacheRead + cacheWrite`(含缓存,开源 emptyPeriod 的
//!   tokenValue 即各分量求和;`reasoning` 在 output 内,不重复计,单独存 reasoning_tokens)。
//! - `model_normalized` 保留完整版本号(`claude-opus-4-8`),不再剥离。
//! - `cost_amount` 直接用 tokscale 报告的权威 cost(它内置 models.dev 定价)。
//! - `occurred_at` 取该 session 文件 mtime(与开源 applySessionTimestamps 同思路;文件缺失
//!   回退扫描时刻)。
//!
//! 时间窗口:checkpoint 记录 `mtime_ms = 上次扫描完成时刻`,首次/过期时全量 `--since`(370 天)。
//! 二进制发现优先级:环境变量 TOKSCALE_BIN → PATH → 已安装 Token Monitor.app 内嵌 → 无。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::token_monitor::collector::CollectResult;
use crate::token_monitor::collector::ToolAdapter;
use crate::token_monitor::model::{
    AdapterCapabilities, CollectorCheckpoint, CollectorError, DataFormat, DataSource,
    IncrementalMode, NormalizedUsageEvent, SeriesSplit, SourceType, SupportLevel, ToolDescriptor,
    ToolKind, TrendDay, TrendMonth, TrendSeries, UsageAccuracy,
};

/// 每次全量扫描的覆盖窗口(天)。开源 HISTORY_CAP_DAYS=370。
const SCAN_WINDOW_DAYS: i64 = 370;
/// 两次全量扫描的最小间隔(秒)——tokscale 扫描 ~1.7s,每 5 分钟一次足够跟手。
const SCAN_INTERVAL_SECS: i64 = 300;
/// 快速「今日」快照刷新的最小间隔(秒)——对齐开源版 watch 驱动的秒级刷新：
/// 独立于全量扫描节流，today 数字最迟 10s 追平最新用量。
const TODAY_REFRESH_MIN_INTERVAL_SECS: i64 = 10;

/// tokscale client id → 我们 tool_id 的映射(其余保持原名)。
pub(crate) fn tool_id_for_client(client: &str) -> String {
    let c = client.to_ascii_lowercase();
    match c.as_str() {
        "claude" => "claude_code".into(),
        "codex" => "codex".into(),
        "opencode" => "opencode".into(),
        "cursor" => "cursor".into(),
        "copilot" => "github_copilot".into(),
        "antigravity" | "antigravity-cli" => "antigravity".into(),
        "kimi" => "kimi".into(),
        "qwen" => "qwen".into(),
        "grok" => "grok_build".into(),
        "hermes" => "hermes".into(),
        "zed" => "zed".into(),
        "zcode" => "zcode".into(),
        "kiro" => "kiro".into(),
        "workbuddy" => "workbuddy".into(),
        "codebuddy" => "codebuddy".into(),
        "micode" => "mimo".into(),
        "cline" => "cline".into(),
        "kilocode" | "kilo" => "kilo_code".into(),
        "pi" => "pi".into(),
        "proma" => "proma".into(),
        "openclaw" => "openclaw".into(),
        "gemini" => "gemini".into(),
        "openrouter" => "openrouter".into(),
        "minimax" => "minimax".into(),
        "volcengine" => "volcengine_ark".into(),
        // Trae 系列：IDE 与 Solo 是独立账号体系；Trae Work/Code 等新变体按原名透传
        "trae" | "trae-ide" => "trae".into(),
        "trae-solo" => "trae_solo".into(),
        _ => c,
    }
}

// ---------------------------------------------------------------------------
// graph 聚合（对齐开源 history.js：活跃天数/连续天数/消息数/活跃时间逐值一致）
// ---------------------------------------------------------------------------

/// 单个客户端 tokens 求和（{input,output,cacheRead,cacheWrite} 或标量），
/// 与开源 history.js `sumTokens` 一致：浮点/字符串数字也能正确解析。
fn client_tokens(cl: &serde_json::Value) -> f64 {
    match cl.get("tokens") {
        Some(v) if v.is_object() => ["input", "output", "cacheRead", "cacheWrite"]
            .iter()
            .map(|k| v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0))
            .sum(),
        Some(v) => v.as_f64().unwrap_or(0.0),
        None => 0.0,
    }
}

/// 从 `tokscale graph --no-spinner` 输出构建 TrendSeries（纯函数，便于单测）。
/// 口径复刻开源 src/shared/history.js 的 normalizeHistory：
/// - `active_days` = 活跃日天数（graph 按消息真实时间逐日归因，
///   不会像 usage_event 那样把跨天会话全记到文件 mtime 日）；
/// - `streak_days` = currentStreak（从今天往回数连续活跃日，今天无活动为 0）；
/// - `message_count` = 各日 totals.messages 之和（tokscale 会话 messageCount）；
/// - `active_time_ms` = timeMetrics.totalActiveTimeMs（tokscale 会话活跃时长，
///   缺失时回退各日 activeTimeMs 之和）；
/// - `peak_day` / `monthly` / 按工具按模型拆分均来自贡献数据。
pub(crate) fn build_trend_series(json: &serde_json::Value) -> TrendSeries {
    use std::collections::BTreeMap;

    let mut days: Vec<TrendDay> = Vec::new();
    let mut by_month: BTreeMap<String, i64> = BTreeMap::new();
    let mut messages = 0i64;
    let mut sum_active_time = 0i64;

    if let Some(contribs) = json.get("contributions").and_then(|v| v.as_array()) {
        for c in contribs {
            let Some(date) = c.get("date").and_then(|v| v.as_str()).map(str::to_string) else {
                continue;
            };
            let totals = c
                .get("totals")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let totals_tokens = totals.get("tokens").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let cost = totals.get("cost").and_then(|v| v.as_f64());
            let day_messages = totals.get("messages").and_then(|v| v.as_i64()).unwrap_or(0);
            let active_time_ms = c.get("activeTimeMs").and_then(|v| v.as_i64()).unwrap_or(0);

            let mut per_client: BTreeMap<String, i64> = BTreeMap::new();
            let mut per_model: BTreeMap<String, i64> = BTreeMap::new();
            // 与开源 history.js parseGraphResult 同口径：日总量按客户端 tokens 求和，
            // 不依赖 totals.tokens——totals 缺失/为 0/为浮点时不会把活跃日漏掉（95 vs 96）。
            let mut tokens_from_clients = 0i64;
            if let Some(clients) = c.get("clients").and_then(|v| v.as_array()) {
                for cl in clients {
                    let client = cl.get("client").and_then(|v| v.as_str()).unwrap_or("");
                    let model = cl.get("modelId").and_then(|v| v.as_str()).unwrap_or("");
                    // 客户端 tokens 是嵌套对象 {input,output,cacheRead,cacheWrite}(与 entry 口径一致)
                    let t = client_tokens(cl).round() as i64;
                    tokens_from_clients += t;
                    if !client.is_empty() {
                        *per_client.entry(tool_id_for_client(client)).or_default() += t;
                    }
                    if !model.is_empty() {
                        // 应用模型别名归一，让 MAAS 端点名与 usage_event 口径一致，
                        // 趋势视图的按模型拆分不把同一模型拆成多份。
                        let canonical = crate::token_monitor::normalization::alias_model(model)
                            .unwrap_or(model);
                        *per_model.entry(canonical.to_string()).or_default() += t;
                    }
                }
            }
            let tokens = if totals_tokens > 0.0 {
                totals_tokens.round() as i64
            } else {
                tokens_from_clients
            };
            if tokens <= 0 {
                continue; // 只保留活跃日
            }
            messages += day_messages;
            sum_active_time += active_time_ms;
            *by_month.entry(date[..7].to_string()).or_default() += tokens;

            let split = |map: BTreeMap<String, i64>| {
                let mut list: Vec<SeriesSplit> = map
                    .into_iter()
                    .map(|(key, t)| SeriesSplit { key, tokens: t })
                    .collect();
                list.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.key.cmp(&b.key)));
                list
            };
            days.push(TrendDay {
                date,
                tokens,
                requests: 0,
                cost_amount: cost,
                active_time_ms,
                per_client: Some(split(per_client)),
                per_model: Some(split(per_model)),
            });
        }
    }
    days.sort_by(|a, b| a.date.cmp(&b.date));

    let active_days = days.len() as i64;
    let peak_day = days
        .iter()
        .max_by_key(|d| d.tokens)
        .cloned()
        .filter(|d| d.tokens > 0);

    // currentStreak：从今天往回数连续活跃日（今天无活动则为 0）
    let active_set: std::collections::HashSet<String> =
        days.iter().map(|d| d.date.clone()).collect();
    let mut streak_days = 0i64;
    let mut cursor = chrono::Local::now().format("%Y-%m-%d").to_string();
    loop {
        if active_set.contains(&cursor) {
            streak_days += 1;
            cursor = prev_local_day(&cursor);
        } else {
            break;
        }
    }

    // timeMetrics.totalActiveTimeMs 缺失时回退各日 activeTimeMs 之和
    let active_time_ms = json
        .pointer("/timeMetrics/totalActiveTimeMs")
        .and_then(|v| v.as_i64())
        .unwrap_or(sum_active_time);

    TrendSeries {
        daily: days,
        active_days,
        streak_days,
        peak_day,
        monthly: by_month
            .into_iter()
            .map(|(month, tokens)| TrendMonth { month, tokens })
            .collect(),
        active_time_ms,
        message_count: messages,
    }
}

/// 前一天的本地日历（YYYY-MM-DD）——与 db/token_usage 的实现保持一致。
fn prev_local_day(day: &str) -> String {
    if let Ok(date) = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d") {
        return (date - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
    }
    day.to_string()
}

/// `tokscale graph --no-spinner` 结果缓存：避免每次「趋势」读取都重跑 ~30s 的子进程
/// （否则每切换一次 Token Monitor 页/托盘都会把主线程卡住几十秒）。
/// 缓存键为空表示尚未计算；TTL 内直接返回缓存，过期后在后台线程重算。
const TREND_CACHE_TTL_SECS: u64 = 90;
static TREND_CACHE: Mutex<Option<(Instant, TrendSeries)>> = Mutex::new(None);

/// 运行 `tokscale graph --no-spinner` 并构建 TrendSeries；二进制缺失/失败返回 None
/// （调用方回退 usage_event 路径）。
///
/// 带 TTL 内存缓存：TTL 内命中直接返回（瞬时），过期后重算并更新缓存。
/// 注意：本函数是同步阻塞调用，绝不可在 Tauri 主线程命令里直接调用——上层须
/// 通过 async 命令/后台线程驱动，避免阻塞 UI。
pub(crate) fn try_trend_series() -> Option<TrendSeries> {
    if let Ok(guard) = TREND_CACHE.lock() {
        if let Some((at, cached)) = guard.as_ref() {
            if at.elapsed().as_secs() < TREND_CACHE_TTL_SECS {
                return Some(cached.clone());
            }
        }
    }
    let bin = locate_cached()?;
    let output = Command::new(&bin)
        .args(["graph", "--no-spinner"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let series = build_trend_series(&json);
    if let Ok(mut guard) = TREND_CACHE.lock() {
        *guard = Some((Instant::now(), series.clone()));
    }
    Some(series)
}

/// 后台预热趋势缓存（供采集轮询循环调用）：TTL 内重复调用无额外开销，
/// 确保用户打开 Token Monitor 页/托盘时趋势数据已在缓存中、瞬时返回。
pub(crate) fn warm_trend_cache() {
    let _ = try_trend_series();
}

/// tokscale 覆盖的 tool_id 集合（与 `tool_id_for_client` 反查一致，另含 API 账号类 trae*）。
/// 用于 `refresh_sources`：tokscale 存在时它成为权威源，覆盖清单内的手写适配器退场
/// （避免双算）；清单外的工具（如 atomcode）作为 companion 并行采集。
/// `covers_tool` 与 `covered_tool_ids` 共用此清单（单一事实源）。
pub(crate) const COVERED_TOOL_IDS: &[&str] = &[
    "antigravity",
    "claude_code",
    "cline",
    "codebuddy",
    "codex",
    "cursor",
    "gemini",
    "github_copilot",
    "grok_build",
    "hermes",
    "kilo_code",
    "kimi",
    "kiro",
    "mimo",
    "minimax",
    "openclaw",
    "opencode",
    "openrouter",
    "pi",
    "proma",
    "qwen",
    "trae",
    "trae_solo",
    "volcengine_ark",
    "workbuddy",
    "zcode",
    "zed",
];

pub(crate) fn covers_tool(tool_id: &str) -> bool {
    COVERED_TOOL_IDS.contains(&tool_id)
}

/// tokscale 覆盖的 tool_id 清单（供会话投影过滤使用）。
/// 会话投影只重建 covered 工具的 tm_session——companion 工具（如 atomcode）的会话
/// 由各自手写适配器产出，投影若覆盖它们会生成双份会话（hash id + 投影 id）。
pub(crate) fn covered_tool_ids() -> Vec<String> {
    COVERED_TOOL_IDS.iter().map(|s| s.to_string()).collect()
}

/// 二进制发现候选:环境变量、PATH、已安装的 Token Monitor.app 内嵌二进制。
fn candidate_binaries() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(bin) = std::env::var("TOKSCALE_BIN") {
        if !bin.trim().is_empty() {
            candidates.push(PathBuf::from(bin.trim()));
        }
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            if dir.is_empty() {
                continue;
            }
            for name in ["tokscale", "tokscale.exe"] {
                let p = Path::new(dir).join(name);
                if p.is_file() {
                    candidates.push(p);
                }
            }
        }
    }
    // 开源 Token Monitor.app 打包的内嵌二进制(用户装过开源版时的常见位置)
    let home = std::env::var("HOME").unwrap_or_default();
    let app_roots = [
        "/Applications/Token Monitor.app/Contents/Resources/app.asar.unpacked/node_modules/@tokscale".to_string(),
        Path::new(&home)
            .join("Applications/Token Monitor.app/Contents/Resources/app.asar.unpacked/node_modules/@tokscale")
            .to_string_lossy()
            .to_string(),
    ];
    for app_path in app_roots {
        for arch in ["cli-darwin-arm64", "cli-darwin-x64"] {
            let p = Path::new(&app_path).join(arch).join("bin").join("tokscale");
            if p.is_file() {
                candidates.push(p);
            }
        }
    }
    candidates
}

/// 定位可用的 tokscale 二进制;找不到返回 None。
pub(crate) fn locate_binary() -> Option<PathBuf> {
    for candidate in candidate_binaries() {
        if !candidate.is_file() {
            continue;
        }
        // 轻量探测:--version 成功即认为可用
        if let Ok(output) = Command::new(&candidate).arg("--version").output() {
            if output.status.success() {
                return Some(candidate);
            }
        }
    }
    None
}

/// tokscale 聚合采集器(单例,不依赖具体工具目录)。
#[derive(Default)]
pub struct TokscaleAdapter;

impl TokscaleAdapter {
    /// 执行 tokscale 命令并返回原始 stdout(二进制取自进程级缓存)。
    fn run_scan(&self, since: &str) -> Result<std::process::Output, CollectorError> {
        let Some(bin) = locate_cached() else {
            return Err(CollectorError::PathMissing(
                "tokscale 二进制未找到(设置 TOKSCALE_BIN 或安装 tokscale)".into(),
            ));
        };
        let output = Command::new(&bin)
            .args([
                "--json",
                "--group-by",
                "client,session,model",
                "--since",
                since,
            ])
            .output()
            .map_err(|e| CollectorError::Io(e.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CollectorError::Parse(format!(
                "tokscale 退出码 {}: {}",
                output.status.code().unwrap_or(-1),
                stderr.trim()
            )));
        }
        Ok(output)
    }

    /// Trae 授权同步（最佳努力）：已 `tokscale trae login` 的用户，扫描前刷新 Trae 用量缓存；
    /// 未授权时子进程 ~10ms 快速失败，静默跳过（能力诚实：Trae 数据在授权后自动出现）。
    fn sync_trae(&self) {
        let Some(bin) = locate_cached() else { return };
        let _ = Command::new(&bin).args(["trae", "sync"]).output();
    }

    /// 全量扫描一次,产出事件列表。
    /// 修复：tokscale 的 cacheRead/cacheWrite 是累计值（跨 entry 递增），
    /// 需要追踪上一步的值计算增量，避免求和翻倍。
    fn scan_events(
        &self,
        since: &str,
        scanned_at: &str,
    ) -> Result<Vec<NormalizedUsageEvent>, CollectorError> {
        let output = self.run_scan(since)?;
        let json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| CollectorError::Parse(format!("tokscale JSON 解析失败: {e}")))?;

        let entries = json
            .get("entries")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // 会话时间解析缓存：同一 (client, session) 的多模型组共享一次文件读取
        let mut time_cache: std::collections::HashMap<
            (String, String),
            (Option<String>, Option<String>),
        > = std::collections::HashMap::new();

        // 追踪每个 (client, session, model) 的累计 cache 值，计算增量
        let mut prev_cache: std::collections::HashMap<(String, String, String), (i64, i64)> =
            std::collections::HashMap::new();

        let mut events = Vec::with_capacity(entries.len());
        for entry in &entries {
            let client = entry.get("client").and_then(|v| v.as_str()).unwrap_or("");
            let session_id = entry
                .get("sessionId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let model = entry.get("model").and_then(|v| v.as_str()).unwrap_or("");
            if client.is_empty() || session_id.is_empty() {
                if let Some(event) = entry_to_event(entry, scanned_at, &(None, None), None) {
                    events.push(event);
                }
                continue;
            }
            let key = (client.to_string(), session_id.to_string());
            let times = time_cache
                .entry(key.clone())
                .or_insert_with(|| resolve_session_times(client, session_id))
                .clone();

            // 获取上一步的 cache 值
            let cache_key = (
                client.to_string(),
                session_id.to_string(),
                model.to_string(),
            );
            let prev = prev_cache.get(&cache_key).cloned();

            if let Some(event) = entry_to_event(entry, scanned_at, &times, prev) {
                // 更新累计值
                prev_cache.insert(
                    cache_key,
                    (
                        event.cache_read_tokens.unwrap_or(0),
                        event.cache_write_tokens.unwrap_or(0),
                    ),
                );
                events.push(event);
            }
        }
        Ok(events)
    }
}

/// 单条 tokscale entry → NormalizedUsageEvent;全 0 的合成条目返回 None。
/// `session_times` = (会话开始, 会话最后活跃)（同 scan 内按会话缓存解析）。
/// `prev_cache` = 上一步的 (cacheRead, cacheWrite) 累计值，用于计算增量。
/// 独立成函数便于单元测试(不依赖子进程)。
fn entry_to_event(
    entry: &serde_json::Value,
    scanned_at: &str,
    session_times: &(Option<String>, Option<String>),
    prev_cache: Option<(i64, i64)>,
) -> Option<NormalizedUsageEvent> {
    let client = entry.get("client").and_then(|v| v.as_str())?;
    let tool_id = tool_id_for_client(client);
    let model = entry
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let session_id = entry
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let input = entry.get("input").and_then(|v| v.as_i64()).unwrap_or(0);
    let output = entry.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
    let cache_read = entry.get("cacheRead").and_then(|v| v.as_i64()).unwrap_or(0);
    let cache_write = entry
        .get("cacheWrite")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let reasoning = entry.get("reasoning").and_then(|v| v.as_i64()).unwrap_or(0);
    let cost = entry.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);
    // 该 (client, session, model) 组的消息数（tokscale 条目自带；会话投影据此求和）
    let message_count = entry
        .get("messageCount")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    // 全 0 的合成条目(synthetic/无用量)不落库
    if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 {
        return None;
    }

    // cache 增量：tokscale 的 cacheRead/cacheWrite 是累计值，计算增量避免求和翻倍
    let (prev_read, prev_write) = prev_cache.unwrap_or((0, 0));
    let cache_read_delta = (cache_read - prev_read).max(0);
    let cache_write_delta = (cache_write - prev_write).max(0);

    let (session_started_at, session_last_active_at) = session_times.clone();
    // occurred_at:会话最后活跃时间(真实文件时间戳);缺失回退扫描时刻。
    // 旧逻辑只取文件 mtime,解析不到文件的工具全部塌缩到扫描时刻——现在由
    // resolve_session_times 提供精确时间(首/末条消息时间戳 → mtime → 空)。
    let occurred_at = session_last_active_at
        .clone()
        .unwrap_or_else(|| scanned_at.to_string());

    Some(NormalizedUsageEvent {
        source_type: SourceType::LocalDiscovered,
        tool_id,
        device_id: "local".into(),
        model_raw: if model.is_empty() {
            None
        } else {
            Some(model.clone())
        },
        model_normalized: if model.is_empty() {
            None
        } else {
            Some(model.clone())
        },
        session_id,
        project_id: None,
        account_id: None,
        input_tokens: Some(input),
        output_tokens: Some(output),
        cache_read_tokens: if cache_read_delta > 0 {
            Some(cache_read_delta)
        } else {
            None
        },
        cache_write_tokens: if cache_write_delta > 0 {
            Some(cache_write_delta)
        } else {
            None
        },
        reasoning_tokens: Some(reasoning),
        // 缺失/0 记 None（未知），会话投影 SUM 缺失回退 COUNT(*)
        message_count: (message_count > 0).then_some(message_count),
        // 会话级真实时间戳（投影据此聚合 started_at / last_active_at）
        session_started_at,
        session_last_active_at,
        // 使用 cache 增量计算 total（避免累计值导致求和翻倍）
        total_tokens: Some(input + output + cache_read_delta + cache_write_delta),
        cost_amount: Some(cost),
        cost_currency: Some("USD".into()),
        usage_accuracy: UsageAccuracy::ProviderReported,
        occurred_at,
        source_locator_hash: None,
    })
}

/// 采集完成后刷新权威 period 快照(day/7d/month)存 settings 表。
/// 对齐开源版:开源直接调 `tokscale --today / --week / --month` 取当日/近7天/本月口径,
/// 而非用 session 文件 mtime 归因(那会让跨天 session 的累计 token 全记到今天)。
/// 仅当二进制可用且命令成功时写入;失败静默跳过(查询侧回退 usage_event 聚合)。
///
/// W9：快照命令带 `--group-by client,session,model`,并在每次刷新后把该周期
/// 内出现的会话 ID 写入 `tm_period_session`(会话列表按此成员过滤——开源版会话
/// 视图的「今日/近7天/本月」正是各周期扫描返回的会话集合,而非按文件 mtime 日期
/// 过滤;旧逻辑对解析不到文件的部分工具把全部会话塌缩到扫描时刻,导致今日/近7天
/// 会话缺失、活动时间错误)。
/// 执行一次 tokscale 周期扫描（`--json --group-by client,session,model <flag>`），
/// 返回 stdout 文本；二进制缺失/命令失败返回 None。供全量刷新与快速今日刷新共用。
fn period_scan_json(bin: &Path, flag: &str) -> Option<String> {
    let output = Command::new(bin)
        .args(["--json", "--group-by", "client,session,model", flag])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) fn refresh_period_snapshots(conn: &std::sync::Mutex<rusqlite::Connection>) {
    use crate::db::settings::SettingsRepo;
    let Some(bin) = locate_cached() else { return };
    for (period, flag) in [("day", "--today"), ("7d", "--week"), ("month", "--month")] {
        let Some(stdout) = period_scan_json(&bin, flag) else {
            continue;
        };
        let _ = SettingsRepo.set(conn, &format!("tm.period.{period}"), &stdout);
        // 周期会话成员:entries 的 (client, sessionId, messageCount),按 session 聚合
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
            store_period_sessions(conn, period, &json);
        }
    }
}

/// 快速刷新「今日」权威快照（对齐开源 Token Monitor 的秒级刷新机制）。
///
/// 开源版由 chokidar 文件监听驱动：会话文件一变，1.5s 去抖后仅重扫一次
/// `tokscale --today`（~1.7s 子进程），today 窗口几秒内追平最新写入，托盘/菜单栏
/// 数字随之跳动。本函数是等价的快速路径：只跑一次 `--today` 子进程写
/// `tm.period.day`，**独立于 5 分钟全量扫描节流**——全量扫描负责 usage_event 快照
/// 替换与 7d/month 快照，today 数字不再等 5 分钟。自带 10s 节流，避免文件事件
/// 突发时打爆子进程。仅当二进制可用且命令成功时写入；失败静默跳过（查询侧回退
/// usage_event 聚合）。
pub(crate) fn refresh_today_snapshot(conn: &std::sync::Mutex<rusqlite::Connection>) {
    use crate::db::settings::SettingsRepo;
    let Some(bin) = locate_cached() else { return };
    // 10s 节流：与 watch 去抖（700ms）配合，突发事件合并为至多每 10s 一次扫描
    static LAST: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
    let now = now_epoch_secs();
    let last = LAST.load(std::sync::atomic::Ordering::Relaxed);
    if now - last < TODAY_REFRESH_MIN_INTERVAL_SECS {
        return;
    }
    let Some(stdout) = period_scan_json(&bin, "--today") else {
        return;
    };
    LAST.store(now, std::sync::atomic::Ordering::Relaxed);
    let _ = SettingsRepo.set(conn, "tm.period.day", &stdout);
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
        store_period_sessions(conn, "day", &json);
    }
}

/// 把一次 tokscale 周期扫描的 entries 按 (client, sessionId) 聚合写入 `tm_period_session`
/// (全量重建该周期,幂等)。message_count 为该周期内该会话各组之和,仅供展示。
fn store_period_sessions(
    conn: &std::sync::Mutex<rusqlite::Connection>,
    period: &str,
    json: &serde_json::Value,
) {
    use std::collections::BTreeMap;
    let mut by_session: BTreeMap<(String, String), i64> = BTreeMap::new();
    if let Some(entries) = json.get("entries").and_then(|v| v.as_array()) {
        for e in entries {
            let Some(sid) = e.get("sessionId").and_then(|v| v.as_str()) else {
                continue;
            };
            if sid.trim().is_empty() {
                continue;
            }
            let client = e.get("client").and_then(|v| v.as_str()).unwrap_or("");
            let tool_id = tool_id_for_client(client);
            let msg = e.get("messageCount").and_then(|v| v.as_i64()).unwrap_or(0);
            let key = (tool_id, sid.to_string());
            *by_session.entry(key).or_default() += msg.max(0);
        }
    }
    let Ok(conn) = conn.lock() else {
        return;
    };
    let _ = conn.execute(
        "DELETE FROM tm_period_session WHERE period=?1",
        rusqlite::params![period],
    );
    for ((tool_id, session_id), message_count) in by_session {
        let _ = conn.execute(
            "INSERT INTO tm_period_session (period, session_id, tool_id, message_count) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![period, session_id, tool_id, message_count],
        );
    }
}

/// 定位 tokscale 二进制(进程级缓存,只探测一次子进程),找不到返回 None。
pub(crate) fn locate_cached() -> Option<PathBuf> {
    static CACHE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let found = locate_binary();
            if found.is_none() {
                tracing::warn!(
                "token_monitor: tokscale 未找到;Token Monitor 采集器将保持空闲(可设 TOKSCALE_BIN)"
            );
            }
            found
        })
        .clone()
}

/// 该 client 的会话文件根目录候选（codex 额外按日期路径直解）。
fn session_roots(client: &str, session_id: &str) -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut roots: Vec<PathBuf> = Vec::new();
    match client {
        "claude" => roots.push(home.join(".claude").join("projects")),
        "codex" => {
            roots.push(home.join(".codex").join("sessions"));
            // codex 会话文件按日期分目录:~/.codex/sessions/YYYY/MM/DD/<rollout-…>.jsonl
            if let Some(ts) = session_id
                .strip_prefix("rollout-")
                .and_then(|s| s.get(..10))
            {
                if let (Some(y), Some(m), Some(d)) = (ts.get(..4), ts.get(5..7), ts.get(8..10)) {
                    roots.push(home.join(".codex").join("sessions").join(y).join(m).join(d));
                }
            }
        }
        "opencode" => roots.push(home.join(".local/share/opencode/storage/message")),
        "workbuddy" => {
            roots.push(home.join(".workbuddy").join("projects"));
            roots.push(home.join(".workbuddy11111").join("projects"));
        }
        "zcode" => roots.push(home.join(".zcode").join("cli/agents")),
        // mimo/micode 会话是目录:~/.local/share/mimocode/memory/sessions/<sessionId>/
        "mimo" | "micode" => roots.push(home.join(".local/share/mimocode/memory/sessions")),
        "kimi" => roots.push(home.join(".kimi").join("sessions")),
        "qwen" => roots.push(home.join(".qwen")),
        "cursor" => roots.push(home.join(".cursor").join("ai-tracking")),
        "antigravity" => roots.push(home.join(".gemini").join("antigravity")),
        _ => {}
    }
    roots
}

/// 定位会话文件/目录路径;找不到返回 None。
/// 对齐开源版 `sessionTimestampMap`(claude/codex/opencode 读真实会话文件)。
fn session_file_path(client: &str, session_id: &str) -> Option<PathBuf> {
    for root in session_roots(client, session_id) {
        if let Some(path) = find_path_in_dir(&root, session_id, 0) {
            return Some(path);
        }
    }
    None
}

/// 解析会话的真实时间戳：(开始, 最后活跃)。
/// - 文件:首条/末条 JSON 行时间戳（对齐开源 lastJsonlTimestamp 读 transcript 尾部）;
/// - 目录(mimo/zcode):目录内 transcript.jsonl(≤2 层)的首/末条;找不到用目录 mtime;
/// - 兜底:开始 = 会话 ID 内嵌时间戳(rollout-…/ISO 前缀),最后活跃 = 文件 mtime;
/// - 全无:两个都 None(调用方 occurred_at 回退扫描时刻)。
fn resolve_session_times(client: &str, session_id: &str) -> (Option<String>, Option<String>) {
    let Some(path) = session_file_path(client, session_id) else {
        return (timestamp_from_session_id(session_id), None);
    };
    let mtime_iso = file_mtime(&path).map(iso_from_epoch_ms);
    let transcript = if path.is_file() {
        Some(path)
    } else {
        find_transcript_in_dir(&path, 0)
    };
    if let Some(tx) = transcript {
        let (first, last) = jsonl_first_last_timestamps(&tx);
        let started = first.or_else(|| timestamp_from_session_id(session_id));
        let last_active = last.or(mtime_iso);
        (started, last_active)
    } else {
        (timestamp_from_session_id(session_id), mtime_iso)
    }
}

/// 读取 JSONL 文件的首行与末行时间戳：(开始, 最后活跃)。读不到返回 (None, None)。
fn jsonl_first_last_timestamps(path: &Path) -> (Option<String>, Option<String>) {
    use std::io::Read;
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return (None, None),
    };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let first = {
        let mut head = vec![0u8; size.min(8 * 1024) as usize];
        let mut filled = 0usize;
        let mut tmp = [0u8; 4096];
        loop {
            match file.read(&mut tmp) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let take = (head.len() - filled).min(n);
                    head[filled..filled + take].copy_from_slice(&tmp[..take]);
                    filled += take;
                    if filled >= head.len() {
                        break;
                    }
                }
            }
        }
        first_jsonl_ts(&head[..filled])
    };
    let last = {
        let tail_len = size.min(64 * 1024) as usize;
        let mut tail = vec![0u8; tail_len];
        let _ = std::io::Seek::seek(
            &mut file,
            std::io::SeekFrom::Start(size.saturating_sub(tail_len as u64)),
        );
        if file.read_exact(&mut tail).is_ok() {
            last_jsonl_ts(&tail)
        } else {
            None
        }
    };
    (first, last)
}

/// 从文件开头若干字节中取第一条非空 JSON 行的时间戳。
fn first_jsonl_ts(buf: &[u8]) -> Option<String> {
    for line in buf.split(|b| *b == b'\n') {
        let line = String::from_utf8_lossy(line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(ts) = json_ts(&v) {
                return Some(ts);
            }
        }
    }
    None
}

/// 从文件末尾若干字节中取最后一条非空 JSON 行的时间戳。
fn last_jsonl_ts(buf: &[u8]) -> Option<String> {
    for line in buf.split(|b| *b == b'\n').rev() {
        let line = String::from_utf8_lossy(line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(ts) = json_ts(&v) {
                return Some(ts);
            }
        }
    }
    None
}

/// 从 JSON 对象取时间戳字段（对齐开源 firstString/STARTED_AT_KEYS/LAST_USED_AT_KEYS）。
fn json_ts(obj: &serde_json::Value) -> Option<String> {
    for key in [
        "timestamp",
        "ts",
        "updatedAt",
        "updated_at",
        "createdAt",
        "created_at",
        "lastUsedAt",
        "last_used_at",
        "startedAt",
        "started_at",
        "time",
    ] {
        if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
            if let Some(iso) = normalize_iso(v.trim()) {
                return Some(iso);
            }
        }
    }
    None
}

/// 时间字符串 → UTC ISO8601（RFC3339 / 无时区 ISO 按 UTC 近似，对齐开源 new Date 语义）。
fn normalize_iso(s: &str) -> Option<String> {
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Some(
                chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc)
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            );
        }
    }
    None
}

/// 从会话 ID 中提取内嵌时间戳（codex `rollout-2026-08-02T21-51-07-…`、
/// ISO 前缀 `2026-08-02T21:51:07-…`），归一为 UTC ISO8601。
fn timestamp_from_session_id(id: &str) -> Option<String> {
    let start = id.find(|c: char| c.is_ascii_digit())?;
    let slice = id.get(start..)?;
    let b = slice.as_bytes();
    if b.len() < 16 {
        return None;
    }
    // YYYY-MM-DDTHH:MM 或 YYYY-MM-DDTHH-MM
    if b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let sep = b[13];
    if sep != b':' && sep != b'-' {
        return None;
    }
    let y = slice.get(..4)?;
    let mo = slice.get(5..7)?;
    let d = slice.get(8..10)?;
    let h = slice.get(11..13)?;
    let mi = slice.get(14..16)?;
    // 秒可选：':' 或 '-' 后跟 2 位数字
    let sec = if b.len() >= 19 && (b[16] == b':' || b[16] == b'-') {
        slice.get(17..19)?
    } else {
        "00"
    };
    Some(format!("{y}-{mo}-{d}T{h}:{mi}:{sec}Z"))
}

/// 在目录树中找名字包含 `needle` 的文件**或目录**,返回其路径。
/// 优先精确匹配(避免子代理文件误中);再退化为包含匹配 + 子目录递归。
fn find_path_in_dir(dir: &Path, needle: &str, depth: u32) -> Option<PathBuf> {
    if depth > 5 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    // 第一趟:精确匹配(文件名或目录名 == needle,或 needle.jsonl / needle.md)
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name == needle || name == format!("{needle}.jsonl") || name == format!("{needle}.md") {
            return Some(path);
        }
    }
    // 第二趟:包含匹配(文件或目录) + 子目录递归
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.contains(needle) {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_path_in_dir(&path, needle, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

/// 在会话目录内找 transcript.jsonl(≤2 层)。
fn find_transcript_in_dir(dir: &Path, depth: u32) -> Option<PathBuf> {
    if depth > 2 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .map(|n| n == "transcript.jsonl")
                .unwrap_or(false)
        {
            return Some(path);
        }
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_transcript_in_dir(&path, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

/// 文件/目录的 mtime(epoch ms)。目录取目录自身 mtime(会话目录的活动时间)。
fn file_mtime(path: &Path) -> Option<i64> {
    let meta = path.metadata().ok()?;
    let t = meta.modified().ok()?;
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

/// epoch 毫秒 → UTC ISO8601。
fn iso_from_epoch_ms(ms: i64) -> String {
    match chrono::DateTime::from_timestamp_millis(ms) {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        None => chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    }
}

/// 当前 epoch 秒。
fn now_epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl ToolAdapter for TokscaleAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool_id: "tokscale_aggregate".into(),
            display_name: "Tokscale 聚合引擎".into(),
            vendor: Some("tokscale".into()),
            kind: ToolKind::Usage,
            support_level: SupportLevel::Full,
            supported_os: vec!["macos".into(), "windows".into(), "linux".into()],
            adapter_version: 1,
            privacy_note: "调用本机 tokscale 二进制聚合 36+ 工具的用量元数据;不读取/持久化 Prompt/Response/源码正文".into(),
        }
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            token: true,
            model: true,
            session: true,
            project: false,
            cache_tokens: true,
            cost: true,
            accuracy: UsageAccuracy::ProviderReported,
            incremental: IncrementalMode::ContentFingerprint,
        }
    }

    fn discover(&self) -> Vec<DataSource> {
        if locate_cached().is_some() {
            vec![DataSource {
                id: "tokscale:aggregate".into(),
                // 非真实路径,仅作展示;采集时不使用
                path: PathBuf::from("tokscale://aggregate"),
                format: DataFormat::Json,
                watch: false, // 走轮询(每 30s 调度 tick 由 poll loop 触发;内部再按 5min 间隔跳过)
            }]
        } else {
            Vec::new()
        }
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
    ) -> Result<CollectResult, CollectorError> {
        let _ = source;
        // 二进制不可用 → 静默空闲(能力诚实,不报错刷屏)
        let Some(_bin) = locate_cached() else {
            return Ok(CollectResult {
                events: vec![],
                sessions: vec![],
                next_checkpoint: checkpoint,
            });
        };

        // 节流:距上次全量 < SCAN_INTERVAL_SECS 直接跳过(下一轮再扫)
        let now = now_epoch_secs();
        if let Some(last) = checkpoint.mtime_ms {
            if now - last < SCAN_INTERVAL_SECS {
                return Ok(CollectResult {
                    events: vec![],
                    sessions: vec![],
                    next_checkpoint: checkpoint,
                });
            }
        }

        // since:checkpoint 未存时间 → 全量窗口
        let since = checkpoint
            .last_record_id
            .as_deref()
            .filter(|s| s.len() == 10)
            .map(str::to_string)
            .unwrap_or_else(|| {
                let days = chrono::Duration::days(SCAN_WINDOW_DAYS);
                let start = chrono::Utc::now() - days;
                start.format("%Y-%m-%d").to_string()
            });
        let scanned_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        // Trae 走账号 API（tokscale trae sync），本地无逐会话用量文件；授权后自动进扫描
        self.sync_trae();
        let events = self.scan_events(&since, &scanned_at)?;

        let mut next = checkpoint.clone();
        next.mtime_ms = Some(now);
        next.last_record_id = Some(since);
        next.content_fingerprint = Some(format!("scan@{}", scanned_at));

        Ok(CollectResult {
            events,
            sessions: vec![],
            next_checkpoint: next,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_id_mapping_covers_major_clients() {
        assert_eq!(tool_id_for_client("claude"), "claude_code");
        assert_eq!(tool_id_for_client("codex"), "codex");
        assert_eq!(tool_id_for_client("micode"), "mimo");
        assert_eq!(tool_id_for_client("workbuddy"), "workbuddy");
        assert_eq!(tool_id_for_client("copilot"), "github_copilot");
        assert_eq!(tool_id_for_client("opencode"), "opencode");
        assert_eq!(tool_id_for_client("trae"), "trae");
        assert_eq!(tool_id_for_client("trae-ide"), "trae");
        assert_eq!(tool_id_for_client("trae-solo"), "trae_solo");
        assert_eq!(tool_id_for_client("some-new-tool"), "some-new-tool");
    }

    #[test]
    fn tool_id_mapping_is_stable_for_all_clients() {
        assert_eq!(tool_id_for_client("CLAUDE"), "claude_code"); // 大小写不敏感
        assert_eq!(tool_id_for_client("antigravity-cli"), "antigravity");
        assert_eq!(tool_id_for_client("grok"), "grok_build");
        assert_eq!(tool_id_for_client("hermes"), "hermes");
    }

    #[test]
    fn entry_to_event_maps_tokscale_row_to_usage_event() {
        // 与真实 tokscale --json 输出结构一致的 fixture
        let json = serde_json::json!({
            "client": "claude",
            "sessionId": "sess-1",
            "model": "claude-opus-4-8",
            "provider": "anthropic",
            "input": 100, "output": 200, "cacheRead": 300, "cacheWrite": 50,
            "reasoning": 0, "messageCount": 3, "cost": 1.25,
            "performance": {"totalDurationMs": 1000}
        });
        // 会话时间由解析器提供（此处注入精确值）
        let times = (
            Some("2026-08-08T00:00:00Z".to_string()),
            Some("2026-08-08T01:02:03Z".to_string()),
        );
        let event = entry_to_event(&json, "2026-08-08T00:00:00Z", &times, None).expect("event");
        // 口径对齐开源:total = in+out+cacheRead+cacheWrite(含缓存)
        // 无 prev_cache 时，delta = 原始值
        assert_eq!(event.total_tokens, Some(650));
        assert_eq!(event.input_tokens, Some(100));
        assert_eq!(event.output_tokens, Some(200));
        assert_eq!(event.cache_read_tokens, Some(300));
        assert_eq!(event.cache_write_tokens, Some(50));
        // 模型保留完整版本号
        assert_eq!(event.model_raw.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(event.model_normalized.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(event.tool_id, "claude_code");
        assert_eq!(event.session_id.as_deref(), Some("sess-1"));
        // 会话级真实时间戳透传；occurred_at = 最后活跃（非扫描时刻）
        assert_eq!(
            event.session_started_at.as_deref(),
            Some("2026-08-08T00:00:00Z")
        );
        assert_eq!(
            event.session_last_active_at.as_deref(),
            Some("2026-08-08T01:02:03Z")
        );
        assert_eq!(event.occurred_at, "2026-08-08T01:02:03Z");
        // 权威成本直通
        assert!((event.cost_amount.unwrap() - 1.25).abs() < 1e-9);
        // 消息数取自条目 messageCount（会话投影据此求和）
        assert_eq!(event.message_count, Some(3));
    }

    #[test]
    fn entry_to_event_falls_back_to_scan_time_without_session_times() {
        let json = serde_json::json!({
            "client": "opencode",
            "sessionId": "ses_abc",
            "model": "gpt-5.5",
            "input": 10, "output": 20, "cacheRead": 0, "cacheWrite": 0,
            "reasoning": 0, "messageCount": 1, "cost": 0.01,
            "performance": null
        });
        let event =
            entry_to_event(&json, "2026-08-08T12:00:00Z", &(None, None), None).expect("event");
        assert_eq!(event.session_started_at, None);
        assert_eq!(event.session_last_active_at, None);
        assert_eq!(event.occurred_at, "2026-08-08T12:00:00Z");
    }

    #[test]
    fn timestamp_from_session_id_extracts_codex_and_iso_prefixes() {
        // codex rollout 前缀（T 后分隔符为 '-'）
        assert_eq!(
            timestamp_from_session_id("rollout-2026-08-02T21-51-07-019fc2be").as_deref(),
            Some("2026-08-02T21:51:07Z")
        );
        // ISO 前缀（':' 分隔）
        assert_eq!(
            timestamp_from_session_id("2026-08-02T21:51:07-abcdef").as_deref(),
            Some("2026-08-02T21:51:07Z")
        );
        // UUID / mimo 短码：无内嵌时间 → None
        assert_eq!(timestamp_from_session_id("4ab39b05-5ec6-4a5d-94ca"), None);
        assert_eq!(timestamp_from_session_id("ses_0209e6357ffe"), None);
    }

    #[test]
    fn normalize_iso_accepts_rfc3339_and_bare_iso() {
        assert_eq!(
            normalize_iso("2026-08-08T01:02:03.456Z").as_deref(),
            Some("2026-08-08T01:02:03Z")
        );
        assert_eq!(
            normalize_iso("2026-08-08T01:02:03").as_deref(),
            Some("2026-08-08T01:02:03Z")
        );
        assert_eq!(normalize_iso(""), None);
        assert_eq!(normalize_iso("not-a-date"), None);
    }

    #[test]
    fn build_trend_series_parses_graph_contributions() {
        // 与真实 `tokscale graph --no-spinner` 输出结构一致的 fixture
        let json = serde_json::json!({
            "contributions": [
                {"date": "2026-08-06", "totals": {"tokens": 5000, "cost": 0.05, "messages": 10},
                 "activeTimeMs": 3600000,
                 "clients": [{"client": "claude", "modelId": "claude-opus-4-8", "tokens": {"input": 3000, "output": 2000, "cacheRead": 0, "cacheWrite": 0}, "cost": 0.03, "messages": 6},
                              {"client": "codex", "modelId": "gpt-5.5", "tokens": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}, "cost": 0.02, "messages": 4}]},
                {"date": "2026-08-07", "totals": {"tokens": 8000, "cost": 0.08, "messages": 15},
                 "activeTimeMs": 5400000,
                 "clients": [{"client": "workbuddy", "modelId": "mimo-v2.5-pro", "tokens": {"input": 8000, "output": 0, "cacheRead": 0, "cacheWrite": 0}, "cost": 0.08, "messages": 15}]},
                {"date": "2026-08-08", "totals": {"tokens": 3000, "cost": 0.03, "messages": 5},
                 "activeTimeMs": 1800000,
                 "clients": [{"client": "claude", "modelId": "claude-opus-4-8", "tokens": {"input": 1000, "output": 2000, "cacheRead": 0, "cacheWrite": 0}, "cost": 0.03, "messages": 5}]}
            ],
            "timeMetrics": {"totalActiveTimeMs": 10800000, "sessionCount": 9}
        });
        let s = build_trend_series(&json);
        assert_eq!(s.active_days, 3);
        assert_eq!(s.message_count, 10 + 15 + 5);
        assert_eq!(s.active_time_ms, 10_800_000); // timeMetrics 优先
        assert_eq!(s.daily.len(), 3);
        assert_eq!(s.daily[0].date, "2026-08-06"); // 升序
        assert_eq!(
            s.peak_day.as_ref().map(|d| d.date.as_str()),
            Some("2026-08-07")
        );
        assert_eq!(s.monthly[0].month, "2026-08");
        assert_eq!(s.monthly[0].tokens, 16_000);
        // 按工具拆分：claude→claude_code；8-06 两工具
        let day0 = &s.daily[0];
        let clients = day0.per_client.as_ref().expect("per_client");
        assert!(clients
            .iter()
            .any(|x| x.key == "claude_code" && x.tokens == 5000));
        assert!(clients.iter().any(|x| x.key == "codex" && x.tokens == 0));
        // 按模型拆分
        let models = day0.per_model.as_ref().expect("per_model");
        assert!(models.iter().any(|x| x.key == "claude-opus-4-8"));
        // 活跃时间缺失时回退各日之和
        let s2 = build_trend_series(
            &serde_json::json!({"contributions": json["contributions"].clone()}),
        );
        assert_eq!(s2.active_time_ms, 3_600_000 + 5_400_000 + 1_800_000);
    }

    #[test]
    fn build_trend_series_skips_zero_token_days_and_maps_streak() {
        let json = serde_json::json!({
            "contributions": [
                {"date": "2026-08-08", "totals": {"tokens": 0, "messages": 0}, "activeTimeMs": 0, "clients": []},
                {"date": "2026-08-07", "totals": {"tokens": 100, "messages": 1}, "activeTimeMs": 0, "clients": []}
            ],
            "timeMetrics": {}
        });
        let s = build_trend_series(&json);
        assert_eq!(s.active_days, 1); // 0 token 日不计
        assert_eq!(s.daily.len(), 1);
        // 连续天数：今天(测试运行日)无活动 → 0；历史连续不回溯到今天
        assert_eq!(s.streak_days, 0);
    }

    #[test]
    fn covers_tool_matches_tokscale_clients() {
        // tokscale 覆盖：主流的 21+ 工具与 API 账号类
        assert!(covers_tool("claude_code"));
        assert!(covers_tool("workbuddy"));
        assert!(covers_tool("mimo"));
        assert!(covers_tool("trae"));
        assert!(covers_tool("trae_solo"));
        // 未覆盖：companion 适配器（atomcode 等）应并行采集
        assert!(!covers_tool("atomcode"));
        assert!(!covers_tool("tokscale_aggregate"));
    }

    #[test]
    fn entry_to_event_skips_all_zero_synthetic_rows() {
        let json = serde_json::json!({
            "client": "opencode",
            "sessionId": "sess-0",
            "model": "<synthetic>",
            "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0,
            "reasoning": 0, "messageCount": 0, "cost": 0.0,
            "performance": null
        });
        assert!(entry_to_event(&json, "2026-08-08T00:00:00Z", &(None, None), None).is_none());
    }
}
