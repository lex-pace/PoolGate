//! Freebuff（本应用）— SQLite `.freebuff/desktop-v2.db` / seq 增量。
//!
//! 数据源：Freebuff 桌面端把对话与用量写入**工作区根目录**下的
//! `.freebuff/desktop-v2.db`（SQLite，每个工作区一个）。`messages` 表按
//! `seq` 自增；assistant 消息的 `metrics_json` 携带该轮完整 token 用量：
//! - `usage.inputTokens`：本轮输入（**含**缓存读取部分）
//! - `usage.cachedInputTokens`：其中缓存读的部分（cache read）
//! - `usage.outputTokens`：输出（**含** reasoning 部分，不重复计）
//! - `usage.reasoningOutputTokens`：推理输出（单独存列，供明细展示）
//! - `usage.totalTokens`：total = inputTokens + outputTokens（含缓存）
//! - `context.model`：模型名（如 `deepseek/deepseek-v4-flash`）
//!
//! 只读连接 + seq 增量；**不读取** `parts_json`（Prompt/Response 正文，隐私红线）；
//! 路径仅 hash 入库。工作区列表从 `~/.config/freebuff-desktop/state.json`
//! 的 `recentProjects` / `workspace.tabs[].projectPath` 读取（Freebuff 自己的
//! 工作区注册表，保证能找到各工作区的 DB）。
//!
//! **模型继承**：Freebuff 的部分 assistant 行只有 `usage` 而没有 `context.model`（
//! 同线程其它行带模型）。这类行按线程继承最近已知模型（按 seq 就近归属），并回填
//! 已入库的历史 NULL 模型事件（`backfill_missing_models`，幂等：只补 NULL + 重算指纹）。

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::token_monitor::collector::common::{hash_short, open_readonly};
use crate::token_monitor::collector::CollectResult;
use crate::token_monitor::collector::ToolAdapter;
use crate::token_monitor::model::{
    AdapterCapabilities, CollectorCheckpoint, CollectorError, DataFormat, DataSource,
    IncrementalMode, NormalizedUsageEvent, SessionQuery, SessionSummary, SourceType, SupportLevel,
    ToolDescriptor, ToolKind, UsageAccuracy,
};

/// 数据目录名（工作区根目录下）。
const DATA_DIR: &str = ".freebuff";
/// SQLite 文件名。
const DB_FILE: &str = "desktop-v2.db";
/// 一次最多读的行数（增量分批，防大 DB 一次打爆内存）。
const BATCH_LIMIT: i64 = 500;
/// 解析字段兼容性升级标记。升级后第一次采集会从 seq=0 重扫，避免旧版适配器
/// 已推进 checkpoint、却把 MiniMax/GPT 等非 camelCase usage 行静默跳过后无法补采。
const PARSER_VERSION: &str = "freebuff-usage-v2";

#[derive(Default)]
pub struct FreebuffAdapter;

/// 候选 DB 路径：环境变量 → 用户目录 → state.json 工作区 → 当前目录。
/// 仅收集**存在**的路径（去重后），不存在的静默跳过。
fn candidate_dbs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("FREEBUFF_DATA").map(PathBuf::from) {
        out.push(dir.join(DB_FILE));
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return out;
    };
    out.push(home.join(DATA_DIR).join(DB_FILE));
    // Freebuff 桌面端把工作区列表写在 state.json（recentProjects + 打开的 tabs）
    let config_root: PathBuf = {
        #[cfg(windows)]
        {
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData").join("Roaming"))
        }
        #[cfg(not(windows))]
        {
            home.join(".config")
        }
    };
    if let Ok(state) =
        std::fs::read_to_string(config_root.join("freebuff-desktop").join("state.json"))
    {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&state) {
            let mut workspaces: Vec<String> = Vec::new();
            if let Some(projects) = json.get("recentProjects").and_then(|v| v.as_array()) {
                for p in projects {
                    if let Some(s) = p.as_str() {
                        workspaces.push(s.to_string());
                    }
                }
            }
            if let Some(tabs) = json
                .get("workspace")
                .and_then(|v| v.get("tabs"))
                .and_then(|v| v.as_array())
            {
                for t in tabs {
                    if let Some(s) = t.get("projectPath").and_then(|v| v.as_str()) {
                        workspaces.push(s.to_string());
                    }
                }
            }
            for ws in workspaces {
                let workspace = PathBuf::from(ws);
                // 兼容旧版工作区布局：<workspace>/.freebuff/desktop-v2.db。
                out.push(workspace.join(DATA_DIR).join(DB_FILE));
                // Freebuff 新版把数据库直接放在工作区目录：
                // <workspace>/desktop-v2.db。
                out.push(workspace.join(DB_FILE));
            }
        }
    }
    // Freebuff Desktop 的真实持久化布局还可能是
    // ~/.config/freebuff-desktop/projects/<project-id>/desktop-v2.db；project-id
    // 是内部 UUID，无法由 recentProjects 的工作区路径直接拼出，因此扫描该目录。
    let projects_root = config_root.join("freebuff-desktop").join("projects");
    if let Ok(projects) = std::fs::read_dir(projects_root) {
        for project in projects.flatten() {
            let path = project.path();
            if path.is_dir() {
                out.push(path.join(DB_FILE));
                out.push(path.join(DATA_DIR).join(DB_FILE));
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        out.push(cwd.join(DATA_DIR).join(DB_FILE));
    }
    out
}

/// epoch 毫秒 → UTC ISO8601（秒精度）；非法值回退 1970。
fn iso_from_epoch_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

/// 从 JSON 中读取 token 数值，兼容 Freebuff 内部 camelCase 与各供应商常见的
/// snake_case/OpenAI/Anthropic 字段。Freebuff 的模型不应决定解析分支；字段形态才是
/// 唯一判断依据。
fn token_number(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| value.as_f64().filter(|n| n.is_finite()).map(|n| n as i64))
        .or_else(|| value.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
}

fn first_token_number(value: &serde_json::Value, paths: &[&str]) -> Option<i64> {
    paths
        .iter()
        .filter_map(|path| value.pointer(path).and_then(token_number))
        .next()
}

fn usage_object(metrics: &serde_json::Value) -> Option<&serde_json::Value> {
    ["/usage", "/providerData/usage", "/response/usage", "/data/usage"]
        .iter()
        .find_map(|path| metrics.pointer(path).filter(|value| value.is_object()))
}

/// 取模型名。新版 Freebuff 通常写在线程 `model`，旧版/部分 provider 则写在
/// metrics.context.model 或 providerData.model；这里统一兼容，不按具体模型白名单过滤。
fn model_from_value(value: &serde_json::Value) -> Option<String> {
    [
        "/context/model",
        "/model",
        "/modelId",
        "/model_id",
        "/requestModelId",
        "/requestModelName",
        "/providerData/model",
        "/providerData/requestModelId",
        "/providerData/requestModelName",
    ]
    .iter()
    .find_map(|path| {
        value
            .pointer(path)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string)
    })
}

fn freebuff_locator(source_id: &str, thread_id: &str, seq: i64) -> String {
    hash_short(&format!("freebuff:{source_id}:{thread_id}:{seq}"))
}

fn model_from_metrics(metrics: &serde_json::Value) -> Option<String> {
    model_from_value(metrics).or_else(|| {
        usage_object(metrics).and_then(|usage| {
            ["/model", "/modelId", "/model_id"]
                .iter()
                .find_map(|path| usage.pointer(path).and_then(|v| v.as_str()))
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string)
        })
    })
}

/// 解析单条 assistant 消息的 metrics_json → 事件；无用量返回 None。
/// 口径：`input_tokens` 存「去缓存的新增输入」（避免与 cache_read 双算），
/// `cache_read_tokens` 存缓存读部分，`total_tokens` 优先使用 provider 报告值。
/// `inherited_model`：该线程最近已知模型（本行缺模型时继承）。
fn parse_metrics(
    thread_id: &str,
    ts: i64,
    seq: i64,
    metrics_json: &str,
    inherited_model: Option<&str>,
) -> Option<NormalizedUsageEvent> {
    let metrics: serde_json::Value = serde_json::from_str(metrics_json).ok()?;
    usage_object(&metrics)?;
    let input_total = first_token_number(
        &metrics,
        &[
            "/usage/inputTokens",
            "/usage/input_tokens",
            "/usage/promptTokens",
            "/usage/prompt_tokens",
            "/usage/prompt",
            "/providerData/usage/inputTokens",
            "/providerData/usage/input_tokens",
            "/providerData/usage/prompt_tokens",
        ],
    );
    let cached = first_token_number(
        &metrics,
        &[
            "/usage/cachedInputTokens",
            "/usage/cacheReadTokens",
            "/usage/cache_read_tokens",
            "/usage/cacheReadInputTokens",
            "/usage/cache_read_input_tokens",
            "/usage/inputTokensDetails/0/cached_tokens",
            "/usage/inputTokensDetails/cached_tokens",
            "/usage/input_tokens_details/0/cached_tokens",
            "/usage/input_tokens_details/cached_tokens",
            "/usage/input_token_details/cached_tokens",
            "/usage/prompt_tokens_details/0/cached_tokens",
            "/usage/prompt_tokens_details/cached_tokens",
            "/providerData/usage/cachedInputTokens",
            "/providerData/usage/cacheReadTokens",
            "/providerData/usage/inputTokensDetails/0/cached_tokens",
        ],
    );
    let output = first_token_number(
        &metrics,
        &[
            "/usage/outputTokens",
            "/usage/output_tokens",
            "/usage/completionTokens",
            "/usage/completion_tokens",
            "/usage/completion",
            "/providerData/usage/outputTokens",
            "/providerData/usage/output_tokens",
            "/providerData/usage/completion_tokens",
        ],
    );
    if input_total.is_none() && output.is_none() {
        return None; // 非用量行（仅 context 等），跳过
    }
    let fresh_input = match (input_total, cached) {
        (Some(total), Some(cached)) => Some((total - cached).max(0)),
        (Some(total), None) => Some(total),
        (None, _) => None,
    };
    let total = first_token_number(
        &metrics,
        &[
            "/usage/totalTokens",
            "/usage/total_tokens",
            "/providerData/usage/totalTokens",
            "/providerData/usage/total_tokens",
        ],
    )
    .or_else(|| match (input_total, output) {
        (Some(input), Some(output)) => Some(input + output),
        (Some(input), None) => Some(input),
        (None, Some(output)) => Some(output),
        (None, None) => None,
    });
    // 模型本身和线程继承都不限制具体供应商；MiniMax M3、GPT-5.6 Luna 等新模型
    // 会直接作为 model_raw 保存。
    let model = model_from_metrics(&metrics).or_else(|| inherited_model.map(str::to_string));
    Some(NormalizedUsageEvent {
        source_type: SourceType::LocalDiscovered,
        tool_id: "freebuff".into(),
        device_id: "local".into(),
        model_raw: model,
        model_normalized: None,
        session_id: Some(thread_id.to_string()),
        project_id: None,
        account_id: None,
        input_tokens: fresh_input,
        output_tokens: output,
        cache_read_tokens: cached,
        cache_write_tokens: first_token_number(
            &metrics,
            &[
                "/usage/cacheWriteTokens",
                "/usage/cache_write_tokens",
                "/usage/cache_creation_input_tokens",
                "/usage/inputTokensDetails/0/cacheWriteTokens",
            ],
        ),
        reasoning_tokens: first_token_number(
            &metrics,
            &[
                "/usage/reasoningOutputTokens",
                "/usage/reasoning_tokens",
                "/usage/outputTokensDetails/0/reasoning_tokens",
                "/usage/outputTokensDetails/reasoning_tokens",
                "/usage/output_tokens_details/0/reasoning_tokens",
                "/usage/output_tokens_details/reasoning_tokens",
            ],
        )
        .filter(|n| *n > 0),
        message_count: Some(1),
        session_started_at: None,
        session_last_active_at: None,
        total_tokens: total,
        cost_amount: None,
        cost_currency: None,
        usage_accuracy: UsageAccuracy::Exact,
        occurred_at: iso_from_epoch_ms(ts),
        source_locator_hash: Some(hash_short(&format!("freebuff:{thread_id}:{seq}"))),
    })
}

/// 某 thread 的累计会话摘要（全量重算该 thread 的 assistant 用量行，幂等 upsert）。
fn thread_summary(
    conn: &rusqlite::Connection,
    source_id: &str,
    thread_id: &str,
) -> Option<SessionSummary> {
    // Claude Code harness 线程由 claude_code 采集，Freebuff 不产出会话摘要。
    if is_claude_code_harness(conn, thread_id) {
        return None;
    }
    let mut stmt = conn
        .prepare(
            "SELECT ts, metrics_json FROM messages \
             WHERE thread_id=?1 AND role='assistant' \
               AND metrics_json IS NOT NULL AND metrics_json != '{}'",
        )
        .ok()?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([thread_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .ok()?
        .collect::<Result<_, _>>()
        .ok()?;
    if rows.is_empty() {
        return None;
    }
    let mut input_fresh = 0i64;
    let mut output = 0i64;
    let mut cache = 0i64;
    let mut total = 0i64;
    let mut count = 0i64;
    let mut first_ts: Option<i64> = None;
    let mut last_ts: Option<i64> = None;
    let mut models: BTreeSet<String> = BTreeSet::new();
    for (ts, metrics_json) in rows {
        let Ok(metrics) = serde_json::from_str::<serde_json::Value>(&metrics_json) else {
            continue;
        };
        if usage_object(&metrics).is_none() {
            continue;
        }
        let input_total = first_token_number(
            &metrics,
            &[
                "/usage/inputTokens",
                "/usage/input_tokens",
                "/usage/promptTokens",
                "/usage/prompt_tokens",
                "/usage/prompt",
                "/providerData/usage/inputTokens",
                "/providerData/usage/input_tokens",
                "/providerData/usage/prompt_tokens",
            ],
        );
        let cached = first_token_number(
            &metrics,
            &[
                "/usage/cachedInputTokens",
                "/usage/cacheReadTokens",
                "/usage/cache_read_tokens",
                "/usage/cacheReadInputTokens",
                "/usage/cache_read_input_tokens",
                "/usage/inputTokensDetails/0/cached_tokens",
                "/usage/input_tokens_details/0/cached_tokens",
                "/usage/input_token_details/cached_tokens",
                "/usage/prompt_tokens_details/cached_tokens",
                "/providerData/usage/cachedInputTokens",
                "/providerData/usage/cacheReadTokens",
            ],
        );
        let out = first_token_number(
            &metrics,
            &[
                "/usage/outputTokens",
                "/usage/output_tokens",
                "/usage/completionTokens",
                "/usage/completion_tokens",
                "/usage/completion",
                "/providerData/usage/outputTokens",
                "/providerData/usage/output_tokens",
                "/providerData/usage/completion_tokens",
            ],
        );
        if input_total.is_none() && out.is_none() {
            continue;
        }
        count += 1;
        first_ts = Some(first_ts.map_or(ts, |f| f.min(ts)));
        last_ts = Some(last_ts.map_or(ts, |l| l.max(ts)));
        cache += cached.unwrap_or(0);
        output += out.unwrap_or(0);
        input_fresh += match (input_total, cached) {
            (Some(t), Some(c)) => (t - c).max(0),
            (Some(t), None) => t,
            (None, _) => 0,
        };
        total += first_token_number(
            &metrics,
            &[
                "/usage/totalTokens",
                "/usage/total_tokens",
                "/providerData/usage/totalTokens",
                "/providerData/usage/total_tokens",
            ],
        )
        .or_else(|| match (input_total, out) {
            (Some(input), Some(output)) => Some(input + output),
            (Some(input), None) => Some(input),
            (None, Some(output)) => Some(output),
            (None, None) => None,
        })
        .unwrap_or(0);
        if let Some(model) = model_from_metrics(&metrics) {
            models.insert(model);
        }
    }
    // 新版 Freebuff 将模型放到 threads.model/world_snapshot；会话摘要也必须带上它。
    if let Some(model) = thread_models(conn, &[thread_id.to_string()]).remove(thread_id) {
        models.insert(model);
    }
    if count == 0 {
        return None;
    }
    // 项目路径来自 threads 表（仅 basename 进标题，不读会话标题/正文）
    let project_path: Option<String> = conn
        .query_row(
            "SELECT project_path FROM threads WHERE id=?1",
            [thread_id],
            |row| row.get(0),
        )
        .ok();
    let project_base = project_path
        .as_deref()
        .map(|p| PathBuf::from(p))
        .and_then(|p| p.file_name().map(|n| n.to_os_string()))
        .and_then(|n| n.into_string().ok())
        .unwrap_or_else(|| "工作区".to_string());
    let started_at = first_ts.map(iso_from_epoch_ms);
    let date_tag = started_at
        .as_deref()
        .and_then(|iso| iso.get(..16))
        .unwrap_or("");
    Some(SessionSummary {
        session_id: hash_short(&format!("{source_id}:{thread_id}")),
        tool_id: "freebuff".into(),
        external_session_id: Some(thread_id.to_string()),
        project_id: project_path.as_deref().map(hash_short),
        title_redacted: Some(format!("Freebuff · {project_base} · {date_tag}")),
        model_set: models.into_iter().collect(),
        started_at,
        last_active_at: last_ts.map(iso_from_epoch_ms),
        input_tokens: input_fresh,
        output_tokens: output,
        cache_tokens: cache,
        total_tokens: total,
        message_count: count,
        status: Some("active".into()),
        cost_amount: None,
    })
}

/// 计算数据库所属工作区。新版 Freebuff 的 UUID 目录和旧版 `.freebuff` 目录
/// 可能同时存在；优先使用同一工作区的新版数据库，避免把镜像库重复统计。
fn workspace_for_db(path: &std::path::Path) -> PathBuf {
    if let Ok(conn) = open_readonly(path) {
        if let Ok(project) = conn.query_row(
            "SELECT project_path FROM threads WHERE project_path IS NOT NULL AND project_path != '' LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        ) {
            let project_path = PathBuf::from(project);
            return project_path
                .canonicalize()
                .unwrap_or(project_path);
        }
    }
    let parent = path.parent().unwrap_or(path);
    if parent.file_name().and_then(|name| name.to_str()) == Some(DATA_DIR) {
        parent.parent().unwrap_or(parent).to_path_buf()
    } else {
        parent.to_path_buf()
    }
}

/// `threads.harness_id` 为 `claude-code` 的会话是 Freebuff 前端驱动 Claude Code
/// 产生的，同一轮用量会同时出现在 Claude Code 的 transcript 中，由 `claude_code`/
/// tokscale 采集。这里跳过这些线程，避免同一条消息被 Freebuff 与 Claude Code
/// 双算。旧版 schema 无 `harness_id` 列时返回 false（不拦截，保持兼容）。
fn is_claude_code_harness(conn: &rusqlite::Connection, thread_id: &str) -> bool {
    let Ok(harness) = conn.query_row(
        "SELECT harness_id FROM threads WHERE id=?1",
        [thread_id],
        |row| row.get::<_, Option<String>>(0),
    ) else {
        return false;
    };
    harness.as_deref() == Some("claude-code")
}

fn db_layout_rank(path: &std::path::Path) -> u8 {
    let parent = path.parent().unwrap_or(path);
    if parent.file_name().and_then(|name| name.to_str()) == Some("projects") {
        3 // 未来若直接存于 projects 根目录
    } else if parent
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        == Some("projects")
    {
        3 // ~/.config/freebuff-desktop/projects/<uuid>/desktop-v2.db
    } else if parent.file_name().and_then(|name| name.to_str()) == Some(DATA_DIR) {
        1 // legacy workspace/.freebuff/desktop-v2.db
    } else {
        2
    }
}

impl ToolAdapter for FreebuffAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool_id: "freebuff".into(),
            display_name: "Freebuff".into(),
            vendor: Some("Freebuff".into()),
            kind: ToolKind::Usage,
            support_level: SupportLevel::Basic,
            supported_os: vec!["macos".into(), "linux".into(), "windows".into()],
            adapter_version: 2,
            privacy_note: "只读 .freebuff/desktop-v2.db 的 messages.metrics_json 用量元数据；不读取 Prompt/Response 正文；路径仅 hash 入库".into(),
        }
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            token: true,
            model: true,
            session: true,
            project: true,
            cache_tokens: true,
            cost: false,
            accuracy: UsageAccuracy::Exact,
            incremental: IncrementalMode::RecordId,
        }
    }

    fn discover(&self) -> Vec<DataSource> {
        let mut chosen: HashMap<PathBuf, (u8, PathBuf)> = HashMap::new();
        for path in candidate_dbs().into_iter().filter(|path| path.exists()) {
            let canonical = path.canonicalize().unwrap_or(path);
            let workspace = workspace_for_db(&canonical);
            let rank = db_layout_rank(&canonical);
            match chosen.get(&workspace) {
                Some((old_rank, _)) if *old_rank >= rank => {}
                _ => {
                    chosen.insert(workspace, (rank, canonical));
                }
            }
        }
        chosen
            .into_values()
            .map(|(_, path)| DataSource {
                id: format!("freebuff:{}", hash_short(&path.to_string_lossy())),
                path,
                format: DataFormat::Sqlite,
                // DB 由 Freebuff 自己常驻写入，走 20s 轮询回补（与 hermes 同策略）
                watch: false,
            })
            .collect()
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
        let conn = open_readonly(&source.path)?;
        // schema 防御：表/列不符 → FormatChanged（保留旧数据，不 panic）
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .map_err(|e| CollectorError::Io(format!("schema: {e}")))?;
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| CollectorError::Io(format!("schema: {e}")))?
            .collect::<Result<_, _>>()
            .map_err(|e| CollectorError::Io(format!("schema: {e}")))?;
        drop(stmt);
        if !tables.iter().any(|t| t == "messages") || !tables.iter().any(|t| t == "threads") {
            return Err(CollectorError::FormatChanged(format!(
                "freebuff: 无 messages/threads 表（{:?}）",
                source.path
            )));
        }

        // 字段解析器升级后强制回看历史记录；落库由 source_fingerprint 幂等去重，
        // 因此不会重复计算已成功采集的 DeepSeek 等旧事件。
        let parser_changed = checkpoint.content_fingerprint.as_deref() != Some(PARSER_VERSION);
        let last_seq: i64 = if parser_changed {
            0
        } else {
            checkpoint
                .last_record_id
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        };
        let mut stmt = conn
            .prepare(
                "SELECT seq, thread_id, ts, metrics_json FROM messages \
                 WHERE seq > ?1 AND role='assistant' \
                   AND metrics_json IS NOT NULL AND metrics_json != '{}' \
                 ORDER BY seq LIMIT ?2",
            )
            .map_err(|e| CollectorError::Io(format!("query: {e}")))?;
        let rows: Vec<(i64, String, i64, String)> = stmt
            .query_map(rusqlite::params![last_seq, BATCH_LIMIT], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|e| CollectorError::Io(format!("query: {e}")))?
            .collect::<Result<_, _>>()
            .map_err(|e| CollectorError::Io(format!("query: {e}")))?;
        drop(stmt);

        // Claude Code harness 的线程已被 claude_code/tokscale 采集，跳过防双算。
        let rows: Vec<(i64, String, i64, String)> = rows
            .into_iter()
            .filter(|(_, thread_id, _, _)| !is_claude_code_harness(&conn, thread_id))
            .collect();

        // 本批涉及的 thread（去重、保持顺序），用于模型继承索引 + 增量补会话摘要
        let mut touched: Vec<String> = Vec::new();
        let mut seen_thread: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (_, thread_id, _, _) in &rows {
            if seen_thread.insert(thread_id.clone()) {
                touched.push(thread_id.clone());
            }
        }
        // 每线程带 model 的行（seq, model）：缺 model 的行按线程继承最近已知模型
        let model_index: HashMap<String, Vec<(i64, String)>> = touched
            .iter()
            .filter_map(|tid| thread_model_index(&conn, tid).map(|v| (tid.clone(), v)))
            .collect();
        // 线程级权威模型（threads.model / world_snapshot.model）：
        // Freebuff 新版消息 metrics 不再携带 context.model，模型只写在线程记录里。
        let thread_model = thread_models(&conn, &touched);

        let mut events = Vec::new();
        let mut max_seq = last_seq;
        for (seq, thread_id, ts, metrics_json) in rows {
            max_seq = max_seq.max(seq);
            // 继承：本线程中 seq 最近（且早于本行）的已知模型；自身带模型时优先。
            // 线程级模型（threads.model / world_snapshot.model）为权威，优先于消息继承。
            let inherited = model_index.get(&thread_id).and_then(|list| {
                list.iter()
                    .rev()
                    .find(|(s, _)| *s < seq)
                    .map(|(_, m)| m.as_str())
            });
            // 优先使用当前行之前最近的模型；线程模型只是没有任何逐消息模型时的
            // fallback，避免模型切换后把历史 token 全归到最新模型。
            let resolved = inherited.or_else(|| {
                thread_model
                    .get(&thread_id)
                    .map(String::as_str)
            });
            if let Some(mut event) = parse_metrics(&thread_id, ts, seq, &metrics_json, resolved) {
                // 同一用户可能有多个 Freebuff workspace；thread UUID/seq 只在单库内唯一，
                // 必须把 source_id 放入 locator，避免跨 workspace 被去重成一条。
                event.source_locator_hash = Some(freebuff_locator(&source.id, &thread_id, seq));
                events.push(event);
            }
        }

        let mut sessions = Vec::new();
        for thread_id in touched {
            if let Some(summary) = thread_summary(&conn, &source.id, &thread_id) {
                sessions.push(summary);
            }
        }

        Ok(CollectResult {
            events,
            sessions,
            next_checkpoint: CollectorCheckpoint {
                source_id: checkpoint.source_id.clone(),
                last_record_id: Some(max_seq.to_string()),
                content_fingerprint: Some(PARSER_VERSION.into()),
                ..Default::default()
            },
        })
    }

    fn list_sessions(&self, _query: &SessionQuery) -> Result<Vec<SessionSummary>, CollectorError> {
        Ok(Vec::new())
    }
}

/// 线程级权威模型映射（thread_id → model）：`threads.model` 优先，缺失时
/// 读 `world_snapshot` JSON 的 `model` 字段（harness 世界快照，Freebuff 新版
/// 把当前模型写在这里；消息 metrics 已不再携带 context.model）。
/// 任一查询失败（旧版 schema 无这些列）时静默降级为空映射。
fn thread_models(conn: &rusqlite::Connection, thread_ids: &[String]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if thread_ids.is_empty() {
        return out;
    }
    let placeholders: Vec<&str> = thread_ids.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT id, model, world_snapshot FROM threads WHERE id IN ({})",
        placeholders.join(",")
    );
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return out;
    };
    let rows = stmt
        .query_map(rusqlite::params_from_iter(thread_ids.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map(|it| it.filter_map(|r| r.ok()).collect::<Vec<_>>());
    drop(stmt);
    for (id, model, snapshot) in rows.unwrap_or_default() {
        let m = model
            .filter(|m| !m.trim().is_empty())
            .or_else(|| {
                snapshot.as_deref().and_then(|s| {
                    serde_json::from_str::<serde_json::Value>(s)
                        .ok()
                        .and_then(|value| model_from_value(&value))
                })
            });
        if let Some(m) = m {
            out.insert(id, m);
        }
    }
    out
}

/// 某线程中带 `context.model` 的行（seq, model，按 seq 升序）。
/// Freebuff 部分 assistant 行缺 model，但同一线程其它行带——据此继承。
fn thread_model_index(conn: &rusqlite::Connection, thread_id: &str) -> Option<Vec<(i64, String)>> {
    let mut stmt = conn
        .prepare(
            "SELECT seq, metrics_json FROM messages \
             WHERE thread_id=?1 AND role='assistant' AND metrics_json IS NOT NULL \
               AND metrics_json != '{}' \
             ORDER BY seq",
        )
        .ok()?;
    let rows = stmt
        .query_map([thread_id], |row| {
            let seq: i64 = row.get(0)?;
            let json: String = row.get(1)?;
            let parsed = serde_json::from_str::<serde_json::Value>(&json).ok();
            let model = parsed.as_ref().and_then(model_from_metrics);
            Ok((seq, model))
        })
        .ok()?
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    drop(stmt);
    Some(
        rows.into_iter()
            .filter_map(|(seq, model)| {
                // 空串模型视为缺失：继承链路不能把 "" 当作已知模型传递
                model.filter(|m| !m.trim().is_empty()).map(|m| (seq, m))
            })
            .collect(),
    )
}

/// 修复 Freebuff 历史事件（自愈），两类问题一起处理：
///
/// 1. **残留重复（双算）**：适配器模型逻辑变更后重扫（扫描/重扫/刷新会清 checkpoint
///    重采），同一条消息会生成「旧指纹（无模型）+ 新指纹（带模型）」两条行 → TOKENS
///    双算 + 模型视图残留「未知模型」。
/// 2. **缺模型**：早期采集的部分行没有 `context.model`（`model_normalized=NULL`，无孪生
///    行可去重）。读 Freebuff DB 按线程+seq 继承最近已知模型回填。
///
/// 统一清理（去重 + 回填 + 重建 rollup）交给 `super::repair::repair_missing_models`；
/// 本函数只负责从 Freebuff DB 重建 `locator_hash → model` 映射（线程继承）。
/// 幂等：可反复调用；无待修时快速短路返回 0（不碰 Freebuff DB）。
/// 清理适配器升级期间产生的 Freebuff 镜像重复：旧版 locator 不含 workspace，
/// 同一轮消息可能以旧模型名和新模型名各落一行（例如 opus → fable），或同一模型
/// 因多个 checkpoint 重扫落两行。按同一会话/时间/token 组合保留最新行，避免双算。
fn dedupe_legacy_rows(conn: &Mutex<rusqlite::Connection>) -> Result<usize, String> {
    let removed = {
        let guard = conn.lock().map_err(|e| e.to_string())?;
        guard
            .execute(
                "DELETE FROM usage_event
                 WHERE tool_id='freebuff' AND source_type='local_discovered'
                   AND id NOT IN (
                       SELECT MAX(id) FROM usage_event
                       WHERE tool_id='freebuff' AND source_type='local_discovered'
                       GROUP BY session_id, occurred_at,
                                input_tokens, output_tokens,
                                cache_read_tokens, cache_write_tokens, total_tokens
                   )",
                [],
            )
            .map_err(|e| e.to_string())?
    };
    if removed > 0 {
        crate::db::token_usage::UsageEventRepo.rebuild_rollup(conn)?;
    }
    Ok(removed)
}

/// 删除历史上被 Freebuff 采集、但实际属于 Claude Code harness 的线程行（这些
/// 用量已由 claude_code/tokscale 采集）。升级后首次采集触发，幂等。
/// 除 usage_event 外，同时清理这些线程残留的 tm_session 幽灵行（旧版适配器在
/// 加入 harness 过滤前产出过会话摘要，usage_event 已删、tm_session 仍残留）。
fn cleanup_claude_harness_rows(
    conn: &Mutex<rusqlite::Connection>,
    source: &DataSource,
) -> Result<usize, String> {
    let fb = open_readonly(&source.path).map_err(|e| e.to_string())?;
    let Ok(mut stmt) = fb.prepare(
        "SELECT DISTINCT t.id FROM threads t \
         JOIN messages m ON m.thread_id=t.id \
         WHERE t.harness_id='claude-code' \
           AND m.role='assistant' AND m.metrics_json!='{}'",
    ) else {
        return Ok(0); // 旧版 schema 无 harness_id：无 claude-code 线程可清
    };
    let ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .ok()
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>().ok())
        .unwrap_or_default();
    drop(stmt);
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
    let (removed, session_removed) = {
        let guard = conn.lock().map_err(|e| e.to_string())?;
        let removed = guard
            .execute(
                &format!(
                    "DELETE FROM usage_event \
                     WHERE tool_id='freebuff' AND source_type='local_discovered' \
                       AND session_id IN ({})",
                    placeholders.join(",")
                ),
                rusqlite::params_from_iter(ids.iter()),
            )
            .map_err(|e| e.to_string())?;
        // tm_session 以 external_session_id 存 Freebuff 线程 id。
        let session_removed = guard
            .execute(
                &format!(
                    "DELETE FROM tm_session \
                     WHERE tool_id='freebuff' AND external_session_id IN ({})",
                    placeholders.join(",")
                ),
                rusqlite::params_from_iter(ids.iter()),
            )
            .map_err(|e| e.to_string())?;
        (removed, session_removed)
    };
    if removed > 0 {
        crate::db::token_usage::UsageEventRepo.rebuild_rollup(conn)?;
    }
    Ok(removed + session_removed)
}

/// 清理 Freebuff 历史会话摘要的残留重复：适配器 session_id 生成规则变更
/// （旧版 locator 不含 workspace → 新版含 workspace）后，同一线程会留下两条
/// session_id 不同的 tm_session 行（model_set 也可能不同）。按 external_session_id
/// 保留最新（updated_at 最大）行，并把各行的 model_set 合并进保留行，幂等。
fn dedupe_legacy_sessions(conn: &Mutex<rusqlite::Connection>) -> Result<usize, String> {
    let guard = conn.lock().map_err(|e| e.to_string())?;
    let dup_ext: Vec<String> = {
        let mut stmt = guard
            .prepare(
                "SELECT external_session_id FROM tm_session \
                 WHERE tool_id='freebuff' \
                   AND external_session_id IS NOT NULL AND external_session_id != '' \
                 GROUP BY external_session_id HAVING COUNT(*) > 1",
            )
            .map_err(|e| e.to_string())?;
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        ids
    };
    let mut removed = 0usize;
    for ext in &dup_ext {
        // 该 external 下全部行（session_id, model_set_json），updated_at 升序 → 末行最新
        let rows: Vec<(String, String)> = {
            let mut stmt = guard
                .prepare(
                    "SELECT session_id, COALESCE(model_set_json,'[]') FROM tm_session \
                     WHERE tool_id='freebuff' AND external_session_id=?1 ORDER BY updated_at",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params![ext], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            drop(stmt);
            rows
        };
        if rows.len() <= 1 {
            continue;
        }
        // 合并全部行的 model_set（同一线程可能先后使用多个模型）。
        let mut merged: Vec<String> = Vec::new();
        for (_, json) in &rows {
            if let Ok(list) = serde_json::from_str::<Vec<String>>(json) {
                for model in list {
                    if !merged.contains(&model) {
                        merged.push(model);
                    }
                }
            }
        }
        let keep_id = rows.last().map(|(id, _)| id.clone());
        let Some(keep_id) = keep_id else { continue };
        guard
            .execute(
                "UPDATE tm_session SET model_set_json=?1 WHERE session_id=?2",
                rusqlite::params![
                    serde_json::to_string(&merged).unwrap_or_else(|_| "[]".into()),
                    keep_id
                ],
            )
            .map_err(|e| e.to_string())?;
        for (id, _) in &rows {
            if *id != keep_id {
                removed += guard
                    .execute("DELETE FROM tm_session WHERE session_id=?1", rusqlite::params![id])
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(removed)
}

pub(crate) fn backfill_missing_models(
    conn: &Mutex<rusqlite::Connection>,
    source: &DataSource,
) -> Result<usize, String> {
    // 必须先做重复清理；不能被「没有 NULL 模型」的快速短路挡住。
    let removed = dedupe_legacy_rows(conn)?;
    let harness_removed = cleanup_claude_harness_rows(conn, source)?;
    let session_removed = dedupe_legacy_sessions(conn)?;
    if !super::repair::has_null_model_rows(conn, "freebuff") {
        return Ok(removed + harness_removed + session_removed);
    }
    // 全量 assistant 用量行（按线程+seq 顺序推进继承模型）
    let fb = open_readonly(&source.path).map_err(|e| e.to_string())?;
    let mut stmt = fb
        .prepare(
            "SELECT seq, thread_id, metrics_json FROM messages \
             WHERE role='assistant' AND metrics_json IS NOT NULL AND metrics_json != '{}' \
             ORDER BY thread_id, seq",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    // Claude Code harness 线程由 claude_code 采集，跳过防双算。
    let rows: Vec<(i64, String, String)> = rows
        .into_iter()
        .filter(|(_, thread_id, _)| !is_claude_code_harness(&fb, thread_id))
        .collect();

    // 线程级权威模型预置（threads.model / world_snapshot.model）：
    // 新版 Freebuff 消息 metrics 不再携带 context.model，仅线程记录有模型。
    let mut touched: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (_, thread_id, _) in &rows {
        if seen.insert(thread_id.clone()) {
            touched.push(thread_id.clone());
        }
    }
    let mut last_model = thread_models(&fb, &touched);
    let mut models: HashMap<String, String> = HashMap::new(); // locator_hash → model
    for (seq, thread_id, metrics_json) in rows {
        let metrics: serde_json::Value = match serde_json::from_str(&metrics_json) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // 空串模型视为缺失（与 parse_metrics 一致）：回填继承链同样跳过空值
        // 兼容 context.model、线程/消息顶层 model、providerData.model 等写法。
        let own = model_from_metrics(&metrics);
        let model = match own {
            Some(m) => {
                last_model.insert(thread_id.clone(), m.clone());
                Some(m)
            }
            None => last_model.get(&thread_id).cloned(),
        };
        if let Some(model) = model {
            models.insert(freebuff_locator(&source.id, &thread_id, seq), model.clone());
            // 兼容旧版 locator，给升级前已落库的 Freebuff 事件做回填。
            models.insert(hash_short(&format!("freebuff:{thread_id}:{seq}")), model);
        }
    }
    if models.is_empty() {
        return Ok(0);
    }
    super::repair::repair_missing_models(conn, "freebuff", &models)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> FreebuffAdapter {
        FreebuffAdapter::default()
    }

    #[test]
    fn parse_metrics_maps_freebuff_usage() {
        let json = r#"{"context":{"usedTokens":197905,"model":"deepseek/deepseek-v4-flash"},"usage":{"inputTokens":10497930,"cachedInputTokens":10381056,"outputTokens":91381,"reasoningOutputTokens":60217,"totalTokens":10589311}}"#;
        let event = parse_metrics("thread-1", 1786518366174, 42, json, None).expect("event");
        assert_eq!(event.tool_id, "freebuff");
        assert_eq!(event.session_id.as_deref(), Some("thread-1"));
        // input 存去缓存的新增部分，cache 单独存
        assert_eq!(event.input_tokens, Some(10497930 - 10381056));
        assert_eq!(event.cache_read_tokens, Some(10381056));
        assert_eq!(event.output_tokens, Some(91381));
        assert_eq!(event.reasoning_tokens, Some(60217));
        // total 用 Freebuff 报告值（含缓存，与 tokscale 口径一致）
        assert_eq!(event.total_tokens, Some(10589311));
        assert_eq!(event.model_raw.as_deref(), Some("deepseek/deepseek-v4-flash"));
        assert_eq!(event.occurred_at, "2026-08-12T07:06:06Z");
    }

    #[test]
    fn parse_metrics_skips_rows_without_usage() {
        assert!(parse_metrics("t", 0, 1, r#"{"context":{"usedTokens":100}}"#, None).is_none());
        assert!(parse_metrics("t", 0, 2, "not-json", None).is_none());
    }

    #[test]
    fn parse_metrics_handles_missing_cache_field() {
        let json = r#"{"usage":{"inputTokens":50,"outputTokens":10,"totalTokens":60}}"#;
        let event = parse_metrics("t", 0, 3, json, None).expect("event");
        assert_eq!(event.input_tokens, Some(50));
        assert_eq!(event.cache_read_tokens, None);
        assert_eq!(event.total_tokens, Some(60));
    }

    #[test]
    fn parse_metrics_supports_minimax_and_openai_usage_shapes() {
        // MiniMax 等 provider 可能保留 OpenAI 风格 snake_case 字段，不能因为不是
        // Freebuff 内部的 camelCase 就整行丢弃。
        let minimax = r#"{"model":"minimax/MiniMax-M3","usage":{"prompt_tokens":1234,"completion_tokens":321,"total_tokens":1555,"prompt_tokens_details":{"cached_tokens":200}}}"#;
        let event = parse_metrics("minimax-thread", 0, 6, minimax, None).expect("MiniMax event");
        assert_eq!(event.model_raw.as_deref(), Some("minimax/MiniMax-M3"));
        assert_eq!(event.input_tokens, Some(1034));
        assert_eq!(event.cache_read_tokens, Some(200));
        assert_eq!(event.output_tokens, Some(321));
        assert_eq!(event.total_tokens, Some(1555));

        // GPT-5.6 Luna 的模型名同样只作为数据保存；解析不依赖模型白名单。
        let luna = r#"{"context":{"model":"openai/gpt-5.6-luna"},"usage":{"input_tokens":1000,"output_tokens":100,"total_tokens":1100,"input_tokens_details":{"cached_tokens":700},"output_tokens_details":{"reasoning_tokens":80}}}"#;
        let event = parse_metrics("luna-thread", 0, 7, luna, None).expect("Luna event");
        assert_eq!(event.model_raw.as_deref(), Some("openai/gpt-5.6-luna"));
        assert_eq!(event.input_tokens, Some(300));
        assert_eq!(event.cache_read_tokens, Some(700));
        assert_eq!(event.output_tokens, Some(100));
        assert_eq!(event.reasoning_tokens, Some(80));
        assert_eq!(event.total_tokens, Some(1100));
    }

    #[test]
    fn parse_metrics_inherits_model_when_row_lacks_it() {
        // 行自身无 context.model → 用线程继承的最近已知模型
        let json = r#"{"context":{"usedTokens":100},"usage":{"inputTokens":50,"outputTokens":10,"totalTokens":60}}"#;
        let event = parse_metrics("t", 0, 4, json, Some("deepseek/deepseek-v4-flash")).expect("event");
        assert_eq!(
            event.model_raw.as_deref(),
            Some("deepseek/deepseek-v4-flash")
        );
        // 空串 model 同样视为缺失 → 继承生效（Freebuff 偶发 context.model=""）
        let json_empty = r#"{"context":{"model":""},"usage":{"inputTokens":50,"outputTokens":10,"totalTokens":60}}"#;
        let event2 = parse_metrics("t", 0, 5, json_empty, Some("deepseek/deepseek-v4-flash")).expect("event");
        assert_eq!(
            event2.model_raw.as_deref(),
            Some("deepseek/deepseek-v4-flash")
        );
        // 行自身带模型时优先（忽略继承）
        let own = r#"{"context":{"model":"gpt-5"},"usage":{"inputTokens":1,"outputTokens":1,"totalTokens":2}}"#;
        let event = parse_metrics("t", 0, 5, own, Some("deepseek/deepseek-v4-flash")).expect("event");
        assert_eq!(event.model_raw.as_deref(), Some("gpt-5"));
    }

    fn fixture_db() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = rusqlite::Connection::open(dir.path().join(DB_FILE)).expect("open");
        conn.execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, project_path TEXT NOT NULL, title TEXT NOT NULL DEFAULT '');\
             CREATE TABLE messages (seq INTEGER PRIMARY KEY AUTOINCREMENT, thread_id TEXT NOT NULL REFERENCES threads(id), role TEXT NOT NULL, ts INTEGER NOT NULL, metrics_json TEXT NOT NULL DEFAULT '{}');\
             INSERT INTO threads (id, project_path, title) VALUES ('t1', '/ws/proj-a', 'secret title');",
        )
        .expect("schema");
        // 两条 assistant 用量行（第 2 条缓存缺省）+ 一条 user 行（应被跳过）
        conn.execute_batch(
            "INSERT INTO messages (thread_id, role, ts, metrics_json) VALUES \
             ('t1','user',1000,'{}'), \
             ('t1','assistant',1001,'{\"context\":{\"model\":\"deepseek/deepseek-v4-flash\"},\"usage\":{\"inputTokens\":1000,\"cachedInputTokens\":900,\"outputTokens\":80,\"totalTokens\":1080}}'), \
             ('t1','assistant',1002,'{\"usage\":{\"inputTokens\":200,\"outputTokens\":10,\"totalTokens\":210}}');",
        )
        .expect("rows");
        drop(conn);
        dir
    }

    #[test]
    fn collect_incremental_inserts_and_advances_checkpoint() {
        let dir = fixture_db();
        let source = DataSource {
            id: "freebuff:test".into(),
            path: dir.path().join(DB_FILE),
            format: DataFormat::Sqlite,
            watch: false,
        };
        let r1 = adapter()
            .collect_incremental(&source, CollectorCheckpoint::default())
            .expect("first collect");
        assert_eq!(r1.events.len(), 2);
        // 首条：input=1000-900=100, cache=900, output=80, total=1080
        assert_eq!(r1.events[0].input_tokens, Some(100));
        assert_eq!(r1.events[0].cache_read_tokens, Some(900));
        assert_eq!(r1.events[0].total_tokens, Some(1080));
        // 会话摘要：同 thread 聚合
        assert_eq!(r1.sessions.len(), 1);
        let session = &r1.sessions[0];
        assert_eq!(session.input_tokens, 100 + 200);
        assert_eq!(session.output_tokens, 80 + 10);
        assert_eq!(session.cache_tokens, 900);
        assert_eq!(session.total_tokens, 1080 + 210);
        assert_eq!(session.message_count, 2);
        assert_eq!(session.external_session_id.as_deref(), Some("t1"));
        // 标题用项目 basename，不含 thread 标题（隐私）
        assert!(session.title_redacted.as_deref().unwrap().contains("proj-a"));
        assert!(!session.title_redacted.as_deref().unwrap().contains("secret"));
        let cp = &r1.next_checkpoint;
        assert_eq!(cp.last_record_id.as_deref(), Some("3")); // 最大 seq

        // 增量：无新行 → 空事件，checkpoint 前进到 0（无 3 以上行）
        let r2 = adapter()
            .collect_incremental(&source, cp.clone())
            .expect("second collect");
        assert!(r2.events.is_empty());
        assert_eq!(r2.next_checkpoint.last_record_id.as_deref(), Some("3"));
    }

    #[test]
    fn collect_inherits_model_for_rows_without_context_model() {
        let dir = fixture_db();
        let source = DataSource {
            id: "freebuff:test".into(),
            path: dir.path().join(DB_FILE),
            format: DataFormat::Sqlite,
            watch: false,
        };
        let r1 = adapter()
            .collect_incremental(&source, CollectorCheckpoint::default())
            .expect("collect");
        // seq 2 自带 model；seq 3 缺 model → 继承线程最近已知模型
        assert_eq!(r1.events.len(), 2);
        assert_eq!(
            r1.events[0].model_raw.as_deref(),
            Some("deepseek/deepseek-v4-flash")
        );
        assert_eq!(
            r1.events[1].model_raw.as_deref(),
            Some("deepseek/deepseek-v4-flash")
        );
    }

    #[test]
    fn backfill_fills_historical_null_model_rows() {
        // 池侧 DB：跑全量迁移（含 usage_event）
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("gateway.db");
        let database = crate::db::Database::new(&db_path).expect("open db");
        database.run_migrations().expect("migrate");

        // 模拟旧适配器采集：手工插入一条 model_normalized=NULL 的事件（seq 3）
        let locator = hash_short("freebuff:t1:3");
        {
            let conn = database.conn.lock().expect("lock");
            conn.execute(
                "INSERT INTO tool_definition (tool_id, display_name, kind, support_level) \
                 VALUES ('freebuff','Freebuff','usage','basic')",
                [],
            )
            .expect("tool def");
            conn.execute(
                "INSERT INTO usage_event (source_type, tool_id, device_id, model_raw, model_normalized, \
                 session_id, input_tokens, output_tokens, cache_read_tokens, total_tokens, \
                 usage_accuracy, occurred_at, source_fingerprint, source_locator_hash) \
                 VALUES ('local_discovered','freebuff','local',NULL,NULL,'sess',200,10,0,210, \
                 'exact','2026-08-12T07:06:07Z','stale-fingerprint',?1)",
                rusqlite::params![locator],
            )
            .expect("insert bad row");
        }

        // Freebuff 侧 fixture：seq 2 带 model，seq 3 缺 model
        let fb_dir = fixture_db();
        let source = DataSource {
            id: "freebuff:test".into(),
            path: fb_dir.path().join(DB_FILE),
            format: DataFormat::Sqlite,
            watch: false,
        };

        let fixed = backfill_missing_models(&database.conn, &source).expect("backfill");
        assert_eq!(fixed, 1);

        // 断言块：锁守卫在块末释放，否则下面「再跑一次」会与同线程持有的 conn 锁自死锁
        {
            let conn = database.conn.lock().expect("lock");
            let row: (Option<String>, Option<String>, String) = conn
                .query_row(
                    "SELECT model_raw, model_normalized, source_fingerprint FROM usage_event \
                     WHERE source_locator_hash=?1",
                    rusqlite::params![locator],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .expect("read back");
            assert_eq!(row.0.as_deref(), Some("deepseek/deepseek-v4-flash"));
            assert_eq!(row.1.as_deref(), Some("deepseek/deepseek-v4-flash"));
            // 指纹已重算（不再是旧值；含模型后与重扫重采结果一致，避免双算）
            assert_ne!(row.2, "stale-fingerprint");
            assert_eq!(row.2.len(), 64);
        }

        // 幂等：再跑一次返回 0（只补 NULL）
        let again = backfill_missing_models(&database.conn, &source).expect("backfill again");
        assert_eq!(again, 0);
    }

    #[test]
    fn backfill_dedupes_stale_null_duplicates() {
        // 复现线上场景：旧适配器（无模型）与修复后适配器（有模型）重扫后，
        // 同一条消息留下 NULL + 带模型两行（不同指纹，INSERT OR IGNORE 去不掉）
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("gateway.db");
        let database = crate::db::Database::new(&db_path).expect("open db");
        database.run_migrations().expect("migrate");
        let locator = hash_short("freebuff:t1:3");
        {
            let conn = database.conn.lock().expect("lock");
            conn.execute(
                "INSERT INTO tool_definition (tool_id, display_name, kind, support_level) \
                 VALUES ('freebuff','Freebuff','usage','basic')",
                [],
            )
            .expect("tool def");
            // 旧行：NULL 模型 + 旧指纹
            conn.execute(
                "INSERT INTO usage_event (source_type, tool_id, device_id, model_raw, model_normalized, \
                 session_id, input_tokens, output_tokens, cache_read_tokens, total_tokens, \
                 usage_accuracy, occurred_at, source_fingerprint, source_locator_hash) \
                 VALUES ('local_discovered','freebuff','local',NULL,NULL,'sess',200,10,0,210, \
                 'exact','2026-08-12T07:06:07Z','old-fp-without-model',?1)",
                rusqlite::params![locator],
            )
            .expect("insert null row");
            // 新行：带模型 + 新指纹（同 hash、同 token）
            conn.execute(
                "INSERT INTO usage_event (source_type, tool_id, device_id, model_raw, model_normalized, \
                 session_id, input_tokens, output_tokens, cache_read_tokens, total_tokens, \
                 usage_accuracy, occurred_at, source_fingerprint, source_locator_hash) \
                 VALUES ('local_discovered','freebuff','local','deepseek/deepseek-v4-flash','deepseek/deepseek-v4-flash','sess',200,10,0,210, \
                 'exact','2026-08-12T07:06:07Z','new-fp-with-model',?1)",
                rusqlite::params![locator],
            )
            .expect("insert model row");
        }

        let fb_dir = fixture_db();
        let source = DataSource {
            id: "freebuff:test".into(),
            path: fb_dir.path().join(DB_FILE),
            format: DataFormat::Sqlite,
            watch: false,
        };

        let fixed = backfill_missing_models(&database.conn, &source).expect("backfill");
        // 删除 1 条 NULL 残留（带模型孪生行保留）
        assert_eq!(fixed, 1);

        let conn = database.conn.lock().expect("lock");
        let (rows, models): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), SUM(CASE WHEN model_normalized IS NOT NULL THEN 1 ELSE 0 END) \
                 FROM usage_event WHERE source_locator_hash=?1",
                rusqlite::params![locator],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("count");
        // 只保留一条带模型的行（不双算、不残留未知模型）
        assert_eq!(rows, 1);
        assert_eq!(models, 1);
    }

    /// 线程级模型（threads.model / world_snapshot.model）解析：
    /// 新版 Freebuff 消息 metrics 不再携带 context.model，模型只在线程记录里。
    #[test]
    fn thread_models_reads_thread_and_world_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = rusqlite::Connection::open(dir.path().join(DB_FILE)).expect("open");
        conn.execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, model TEXT, world_snapshot TEXT);\
             INSERT INTO threads (id, model, world_snapshot) VALUES \
               ('t-direct', 'deepseek/deepseek-v4-flash', NULL), \
               ('t-snap', NULL, '{\"model\":\"deepseek/deepseek-v4-flash\",\"effort\":null}'), \
               ('t-none', NULL, NULL), \
               ('t-empty', '', '{\"model\":\"\"}');",
        )
        .expect("schema");
        let map = thread_models(&conn, &["t-direct".into(), "t-snap".into(), "t-none".into(), "t-empty".into()]);
        assert_eq!(map.get("t-direct").map(String::as_str), Some("deepseek/deepseek-v4-flash"));
        assert_eq!(map.get("t-snap").map(String::as_str), Some("deepseek/deepseek-v4-flash"));
        // 无模型 / 空串模型 → 不产出条目
        assert!(!map.contains_key("t-none"));
        assert!(!map.contains_key("t-empty"));
    }

    /// 采集时消息无 context.model，但线程记录有模型（threads.model / world_snapshot）→
    /// 事件必须带上线程模型，不再产生「未知模型」。
    #[test]
    fn collect_uses_thread_model_when_messages_lack_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = rusqlite::Connection::open(dir.path().join(DB_FILE)).expect("open");
        conn.execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, project_path TEXT NOT NULL, title TEXT NOT NULL DEFAULT '', \
                                  model TEXT, world_snapshot TEXT);\
             CREATE TABLE messages (seq INTEGER PRIMARY KEY AUTOINCREMENT, thread_id TEXT NOT NULL REFERENCES threads(id), role TEXT NOT NULL, ts INTEGER NOT NULL, metrics_json TEXT NOT NULL DEFAULT '{}');\
             INSERT INTO threads (id, project_path, title, model, world_snapshot) VALUES \
               ('t1', '/ws/proj-a', 't', NULL, '{\"model\":\"deepseek/deepseek-v4-flash\"}');\
             INSERT INTO messages (thread_id, role, ts, metrics_json) VALUES \
               ('t1','assistant',1001,'{\"context\":{\"usedTokens\":100},\"usage\":{\"inputTokens\":1000,\"cachedInputTokens\":900,\"outputTokens\":80,\"totalTokens\":1080}}'), \
               ('t1','assistant',1002,'{\"usage\":{\"inputTokens\":200,\"outputTokens\":10,\"totalTokens\":210}}');",
        )
        .expect("schema");
        drop(conn);
        let source = DataSource {
            id: "freebuff:test".into(),
            path: dir.path().join(DB_FILE),
            format: DataFormat::Sqlite,
            watch: false,
        };
        let r1 = adapter()
            .collect_incremental(&source, CollectorCheckpoint::default())
            .expect("collect");
        assert_eq!(r1.events.len(), 2);
        for e in &r1.events {
            // parse_metrics 只填 model_raw；model_normalized 由落库时 finalize 补齐
            assert_eq!(
                e.model_raw.as_deref(),
                Some("deepseek/deepseek-v4-flash"),
                "消息无 context.model 时线程级模型必须生效"
            );
        }
    }

    /// session_id 生成规则变更（旧 locator → 新 locator）后同一线程会残留两条
    /// tm_session 行：按 external_session_id 保留最新行并合并 model_set。
    #[test]
    fn dedupe_legacy_sessions_merges_duplicate_thread_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("gateway.db");
        let database = crate::db::Database::new(&db_path).expect("open db");
        database.run_migrations().expect("migrate");
        {
            let conn = database.conn.lock().expect("lock");
            conn.execute(
                "INSERT INTO tool_definition (tool_id, display_name, kind, support_level) \
                 VALUES ('freebuff','Freebuff','usage','basic')",
                [],
            )
            .expect("tool def");
            conn.execute(
                "INSERT INTO tm_session (session_id, tool_id, external_session_id, model_set_json, total_tokens, updated_at) \
                 VALUES ('old-hash','freebuff','thread-x','[\"deepseek/deepseek-v4-flash\"]',100,'2026-08-17T10:00:00Z')",
                [],
            )
            .expect("old row");
            conn.execute(
                "INSERT INTO tm_session (session_id, tool_id, external_session_id, model_set_json, total_tokens, updated_at) \
                 VALUES ('new-hash','freebuff','thread-x','[\"deepseek/deepseek-v4-pro\"]',100,'2026-08-17T11:00:00Z')",
                [],
            )
            .expect("new row");
        }
        let removed = dedupe_legacy_sessions(&database.conn).expect("dedupe");
        assert_eq!(removed, 1);
        let conn = database.conn.lock().expect("lock");
        let (session_id, model_set, count): (String, String, i64) = conn
            .query_row(
                "SELECT session_id, model_set_json, \
                        (SELECT COUNT(*) FROM tm_session WHERE external_session_id='thread-x') \
                 FROM tm_session WHERE external_session_id='thread-x'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("read");
        assert_eq!(session_id, "new-hash"); // 保留 updated_at 最新行
        assert_eq!(count, 1);
        let models: Vec<String> = serde_json::from_str(&model_set).unwrap();
        assert!(models.contains(&"deepseek/deepseek-v4-flash".to_string()));
        assert!(models.contains(&"deepseek/deepseek-v4-pro".to_string()));
    }
}
