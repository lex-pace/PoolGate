//! DeepSeek Harness (DSH) — 采集 `~/.dsh/sessions/` 下的 zstd 压缩 JSONL 会话文件。
//!
//! 数据源：DSH 把会话事件写入 `~/.dsh/sessions/<workspace-path>/session-<uuid>/session.jsonl.zstd`。
//! 每个会话是一个 zstd 压缩的 JSONL 文件，事件类型包括：
//! - `session`：会话头（id, createdAt, cwd）
//! - `request/header`：包含 provider, model 配置
//! - `request/context`：包含 provider, model, contextWindow
//! - `assistant/chunk` + `chunk.type == "usage"`：每步 token 用量
//! - `assistant/message`：完整消息（含 usage 字段）
//! - `finish`：步骤结束标记
//!
//! 采集策略：
//! - 发现：递归扫描 `~/.dsh/sessions/*/session-*/session.jsonl.zstd`
//! - 增量：按文件大小（byte_offset）增量读取新事件
//! - 模型：从 `request/header` 或 `request/context` 提取 provider + model
//! - 会话：每个 session-<uuid> 目录对应一个会话摘要
//!
//! 适配器设计为「companion」——tokscale 未来若覆盖 DSH，本适配器作为 fallback 保留，
//! tokscale 覆盖后 `refresh_sources` 会自动跳过已覆盖工具的 companion 源。
//!
//! 解压：运行时在 PATH / /opt/homebrew/bin / /usr/local/bin / /opt/anaconda3/bin 搜索 zstd 命令。

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::token_monitor::collector::common::hash_short;
use crate::token_monitor::collector::CollectResult;
use crate::token_monitor::collector::ToolAdapter;
use crate::token_monitor::model::{
    AdapterCapabilities, CollectorCheckpoint, CollectorError, DataFormat, DataSource,
    IncrementalMode, NormalizedUsageEvent, SessionQuery, SessionSummary, SourceType, SupportLevel,
    ToolDescriptor, ToolKind, UsageAccuracy,
};

/// DSH 数据根目录（`~/.dsh`）。
const DSH_DIR: &str = ".dsh";
/// 会话子目录名。
const SESSIONS_DIR: &str = "sessions";
/// 会话文件名（zstd 压缩 JSONL）。
const SESSION_FILE: &str = "session.jsonl.zstd";

#[derive(Default)]
pub struct DshAdapter;

/// 发现所有 DSH 会话文件：`~/.dsh/sessions/*/session-*/session.jsonl.zstd`。
fn discover_session_files() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let sessions_root = home.join(DSH_DIR).join(SESSIONS_DIR);
    if !sessions_root.is_dir() {
        return Vec::new();
    }
    let mut files = Vec::new();
    let Ok(workspaces) = std::fs::read_dir(&sessions_root) else {
        return files;
    };
    for ws in workspaces.flatten() {
        if !ws.path().is_dir() {
            continue;
        }
        let Ok(sessions) = std::fs::read_dir(ws.path()) else {
            continue;
        };
        for sess in sessions.flatten() {
            let session_file = sess.path().join(SESSION_FILE);
            if session_file.is_file() {
                files.push(session_file);
            }
        }
    }
    files
}

/// 在常见位置查找 zstd 二进制。
/// 搜索顺序：PATH → /opt/homebrew/bin → /usr/local/bin → /opt/anaconda3/bin
fn locate_zstd() -> Option<PathBuf> {
    // 1. PATH 中查找
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            if dir.is_empty() {
                continue;
            }
            let p = PathBuf::from(dir).join("zstd");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    // 2. 常见固定位置
    for candidate in [
        "/opt/homebrew/bin/zstd",
        "/usr/local/bin/zstd",
        "/opt/anaconda3/bin/zstd",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// 用系统 zstd 命令解压文件，返回解压后的 UTF-8 文本。
fn decompress_zstd(path: &std::path::Path) -> Result<String, CollectorError> {
    let zstd_bin = locate_zstd().ok_or_else(|| {
        CollectorError::Io(
            "zstd not found: install via 'brew install zstd' or ensure it's in PATH /opt/homebrew/bin /opt/anaconda3/bin"
                .into(),
        )
    })?;
    let output = std::process::Command::new(&zstd_bin)
        .args(["-dc", "--quiet"])
        .arg(path)
        .output()
        .map_err(|e| CollectorError::Io(format!("zstd exec failed: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CollectorError::Io(format!(
            "zstd decompress failed ({}): {}",
            path.display(),
            stderr.trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|e| CollectorError::Io(format!("zstd output not UTF-8: {e}")))
}

/// 从 JSONL 行列表中提取 usage 事件。
///
/// **cache 口径（对齐 xiaomi 官方用量）**：`cacheReadTokens` 是该请求本次从缓存读取的
/// 上下文量（随对话增长而增大，属「每请求重读即计费」的缓存用量），官方网站按每请求
/// 原始值直接求和。因此这里**不做增量**，直接记录原始值，total = input + output + cacheRead。
fn extract_events(
    tool_id: &str,
    lines: &[&str],
    source_id: &str,
) -> Vec<NormalizedUsageEvent> {
    let mut events = Vec::new();
    let mut session_id: Option<String> = None;
    let mut model: Option<String> = None;

    for line in lines {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let seq = value.get("seq").and_then(|v| v.as_i64()).unwrap_or(0);
        let time = value.get("time").and_then(|v| v.as_i64()).unwrap_or(0);

        match event_type {
            "session" => {
                if let Some(id) = value.get("id").and_then(|v| v.as_str()) {
                    session_id = Some(id.to_string());
                }
            }
            "request/header" => {
                if let Some(config) = value.pointer("/data/header/config") {
                    let provider = config.get("provider").and_then(|v| v.as_str()).unwrap_or("");
                    let model_name = config.get("model").and_then(|v| v.as_str()).unwrap_or("");
                    if !model_name.is_empty() {
                        model = Some(if provider.is_empty() {
                            model_name.to_string()
                        } else {
                            format!("{provider}/{model_name}")
                        });
                    }
                }
            }
            "request/context" => {
                if model.is_none() {
                    let provider = value.pointer("/data/provider").and_then(|v| v.as_str()).unwrap_or("");
                    let model_name = value.pointer("/data/model").and_then(|v| v.as_str()).unwrap_or("");
                    if !model_name.is_empty() {
                        model = Some(if provider.is_empty() {
                            model_name.to_string()
                        } else {
                            format!("{provider}/{model_name}")
                        });
                    }
                }
            }
            "assistant/chunk" => {
                if let Some(chunk) = value.get("data").and_then(|d| d.get("chunk")) {
                    if chunk.get("type").and_then(|v| v.as_str()) == Some("usage") {
                        if let Some(usage) = chunk.get("usage") {
                            let input = usage.get("inputTokens").and_then(|v| v.as_i64());
                            let output = usage.get("outputTokens").and_then(|v| v.as_i64());
                            let cache_read = usage.get("cacheReadTokens").and_then(|v| v.as_i64());
                            let reasoning = usage.get("reasoningTokens").and_then(|v| v.as_i64());

                            if input.is_some() || output.is_some() {
                                // 原始口径：total = input + output + 本次 cacheRead（每请求重读计费）
                                let total = input.unwrap_or(0)
                                    + output.unwrap_or(0)
                                    + cache_read.unwrap_or(0);
                                let occurred_at = if time > 0 {
                                    iso_from_epoch_ms(time)
                                } else {
                                    chrono::Utc::now()
                                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                                };
                                events.push(NormalizedUsageEvent {
                                    source_type: SourceType::LocalDiscovered,
                                    tool_id: tool_id.into(),
                                    device_id: "local".into(),
                                    model_raw: model.clone(),
                                    model_normalized: None,
                                    session_id: session_id.clone(),
                                    project_id: None,
                                    account_id: None,
                                    input_tokens: input,
                                    output_tokens: output,
                                    cache_read_tokens: cache_read,
                                    cache_write_tokens: None,
                                    reasoning_tokens: reasoning.filter(|&n| n > 0),
                                    message_count: Some(1),
                                    session_started_at: None,
                                    session_last_active_at: None,
                                    total_tokens: Some(total),
                                    cost_amount: None,
                                    cost_currency: None,
                                    usage_accuracy: UsageAccuracy::Exact,
                                    occurred_at,
                                    source_locator_hash: Some(hash_short(&format!(
                                        "dsh:{source_id}:{seq}"
                                    ))),
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    events
}

/// 从文件路径提取 workspace 名（basename 的 sessions 下一级目录）。
fn workspace_from_path(path: &std::path::Path) -> Option<String> {
    path.parent() // session-<uuid>
        .and_then(|p| p.parent()) // <workspace>
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// epoch 毫秒 → UTC ISO8601（秒精度）。
fn iso_from_epoch_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

/// 从 JSONL 内容中提取会话摘要信息。
fn extract_session_info(
    content: &str,
    source_id: &str,
    path: &std::path::Path,
) -> Option<SessionSummary> {
    let mut session_id: Option<String> = None;
    let mut model: Option<String> = None;
    let mut project_path: Option<String> = None;
    let mut started_at: Option<i64> = None;
    let mut last_active_at: Option<i64> = None;
    let mut message_count = 0i64;
    let mut total_input = 0i64;
    let mut total_output = 0i64;
    let mut total_cache = 0i64;
    let mut total_tokens = 0i64;

    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let time = value.get("time").and_then(|v| v.as_i64()).unwrap_or(0);

        match event_type {
            "session" => {
                session_id = value.get("id").and_then(|v| v.as_str()).map(String::from);
                project_path = value.get("cwd").and_then(|v| v.as_str()).map(String::from);
                started_at = Some(
                    value
                        .get("createdAt")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(time),
                );
            }
            "request/header" => {
                if let Some(config) = value.pointer("/data/header/config") {
                    let provider = config.get("provider").and_then(|v| v.as_str()).unwrap_or("");
                    let model_name = config.get("model").and_then(|v| v.as_str()).unwrap_or("");
                    if !model_name.is_empty() {
                        model = Some(if provider.is_empty() {
                            model_name.to_string()
                        } else {
                            format!("{provider}/{model_name}")
                        });
                    }
                }
            }
            "request/context" => {
                if model.is_none() {
                    let provider = value
                        .pointer("/data/provider")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let model_name = value
                        .pointer("/data/model")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !model_name.is_empty() {
                        model = Some(if provider.is_empty() {
                            model_name.to_string()
                        } else {
                            format!("{provider}/{model_name}")
                        });
                    }
                }
            }
            "assistant/chunk" => {
                if let Some(chunk) = value.get("data").and_then(|d| d.get("chunk")) {
                    if chunk.get("type").and_then(|v| v.as_str()) == Some("usage") {
                        if let Some(usage) = chunk.get("usage") {
                            let input = usage
                                .get("inputTokens")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            let output = usage
                                .get("outputTokens")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            let cache = usage
                                .get("cacheReadTokens")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            // 原始口径（对齐官方）：每次请求的缓存读取都计费，直接求和
                            total_input += input;
                            total_output += output;
                            total_cache += cache;
                            total_tokens += input + output + cache;
                            message_count += 1;
                            if time > 0 {
                                last_active_at =
                                    Some(last_active_at.map_or(time, |l| l.max(time)));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if message_count == 0 {
        return None;
    }

    let workspace = workspace_from_path(path).unwrap_or_else(|| "DSH".to_string());
    let session_label = session_id
        .as_deref()
        .and_then(|id| id.strip_prefix("session-"))
        .unwrap_or("unknown")
        .to_string();
    let date_tag = started_at
        .map(iso_from_epoch_ms)
        .and_then(|iso| iso.get(..16).map(String::from))
        .unwrap_or_default();

    let mut model_set = BTreeSet::new();
    if let Some(ref m) = model {
        model_set.insert(m.clone());
    }

    Some(SessionSummary {
        session_id: hash_short(&format!(
            "dsh:{source_id}:{}",
            session_id.as_deref().unwrap_or("")
        )),
        tool_id: "dsh".into(),
        external_session_id: session_id,
        project_id: project_path.as_deref().map(hash_short),
        title_redacted: Some(format!(
            "DSH · {workspace} · {session_label} · {date_tag}"
        )),
        model_set: model_set.into_iter().collect(),
        started_at: started_at.map(iso_from_epoch_ms),
        last_active_at: last_active_at.map(iso_from_epoch_ms),
        input_tokens: total_input,
        output_tokens: total_output,
        cache_tokens: total_cache,
        total_tokens,
        message_count,
        status: Some("active".into()),
        cost_amount: None,
    })
}

impl ToolAdapter for DshAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool_id: "dsh".into(),
            display_name: "DeepSeek Harness".into(),
            vendor: Some("DeepSeek".into()),
            kind: ToolKind::Usage,
            support_level: SupportLevel::Basic,
            supported_os: vec!["macos".into(), "linux".into(), "windows".into()],
            adapter_version: 2, // v2：cacheReadTokens 改按每请求原始值记录（对齐 xiaomi 官方用量口径）
            privacy_note:
                "只读 ~/.dsh/sessions/ 下会话文件的 usage 元数据（tokens/模型/时间）；\
                 不读取 Prompt/Response 正文；路径仅 hash 入库"
                    .into(),
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
            incremental: IncrementalMode::FileOffset,
        }
    }

    fn discover(&self) -> Vec<DataSource> {
        let mut seen = std::collections::HashSet::new();
        discover_session_files()
            .into_iter()
            .filter(|p| seen.insert(p.canonicalize().unwrap_or_else(|_| p.clone())))
            .map(|path| {
                let path_str = path.to_string_lossy();
                DataSource {
                    id: format!("dsh:{}", hash_short(&path_str)),
                    path,
                    format: DataFormat::Jsonl,
                    watch: false,
                }
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
        let metadata = std::fs::metadata(&source.path)
            .map_err(|e| CollectorError::Io(format!("stat {}: {}", source.path.display(), e)))?;
        let file_size = metadata.len();
        let last_offset = checkpoint.byte_offset.unwrap_or(0);

        // 增量：文件大小未变 → 无新数据
        if file_size <= last_offset && last_offset > 0 {
            return Ok(CollectResult {
                events: Vec::new(),
                sessions: Vec::new(),
                next_checkpoint: CollectorCheckpoint {
                    source_id: checkpoint.source_id.clone(),
                    byte_offset: Some(file_size),
                    ..Default::default()
                },
            });
        }

        // 解压并解析
        let content = decompress_zstd(&source.path)?;
        let lines: Vec<&str> = content.lines().collect();
        let events = extract_events("dsh", &lines, &source.id);
        let session = extract_session_info(&content, &source.id, &source.path);
        let sessions: Vec<SessionSummary> = session.into_iter().collect();

        Ok(CollectResult {
            events,
            sessions,
            next_checkpoint: CollectorCheckpoint {
                source_id: checkpoint.source_id.clone(),
                byte_offset: Some(file_size),
                ..Default::default()
            },
        })
    }

    fn list_sessions(&self, _query: &SessionQuery) -> Result<Vec<SessionSummary>, CollectorError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_events_parses_dsh_usage() {
        let lines = vec![
            r#"{"type":"session","version":0,"id":"session-test-001","createdAt":1786626349099,"cwd":"/ws/proj"}"#,
            r#"{"type":"request/context","seq":1,"time":1786626349100,"data":{"provider":"deepseek-official","model":"deepseek-v4-flash"}}"#,
            r#"{"type":"assistant/chunk","seq":10,"time":1786626350000,"data":{"turn":1,"step":1,"chunk":{"type":"usage","usage":{"inputTokens":1000,"outputTokens":50,"cacheReadTokens":200,"reasoningTokens":10}}}}"#,
            r#"{"type":"assistant/chunk","seq":20,"time":1786626351000,"data":{"turn":1,"step":2,"chunk":{"type":"usage","usage":{"inputTokens":500,"outputTokens":30,"cacheReadTokens":400,"reasoningTokens":5}}}}"#,
        ];
        let events = extract_events("dsh", &lines, "test-source");

        assert_eq!(events.len(), 2);

        assert_eq!(events[0].input_tokens, Some(1000));
        assert_eq!(events[0].output_tokens, Some(50));
        assert_eq!(events[0].cache_read_tokens, Some(200)); // 原始值
        assert_eq!(events[0].reasoning_tokens, Some(10));
        assert_eq!(events[0].total_tokens, Some(1250)); // 1000 + 50 + 200
        assert_eq!(
            events[0].model_raw.as_deref(),
            Some("deepseek-official/deepseek-v4-flash")
        );
        assert_eq!(events[0].session_id.as_deref(), Some("session-test-001"));

        assert_eq!(events[1].input_tokens, Some(500));
        assert_eq!(events[1].cache_read_tokens, Some(400)); // 原始值（非增量）
        assert_eq!(events[1].total_tokens, Some(930)); // 500 + 30 + 400
    }

    #[test]
    fn extract_events_skips_non_usage_chunks() {
        let lines = vec![
            r#"{"type":"assistant/chunk","seq":1,"time":1000,"data":{"turn":1,"step":1,"chunk":{"type":"text","text":"hello"}}}"#,
            r#"{"type":"finish","seq":2,"time":1001,"data":{"turn":1,"step":1,"reason":{"kind":"stop"}}}"#,
        ];
        let events = extract_events("dsh", &lines, "test");
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn extract_events_uses_request_header_for_model() {
        let lines = vec![
            r#"{"type":"session","id":"s1","createdAt":1000,"cwd":"/ws"}"#,
            r#"{"type":"request/header","seq":1,"time":1001,"data":{"header":{"config":{"provider":"openai","model":"gpt-4o"}}}}"#,
            r#"{"type":"assistant/chunk","seq":10,"time":1002,"data":{"turn":1,"step":1,"chunk":{"type":"usage","usage":{"inputTokens":100,"outputTokens":20}}}}"#,
        ];
        let events = extract_events("dsh", &lines, "test");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].model_raw.as_deref(), Some("openai/gpt-4o"));
    }

    #[test]
    fn decompress_zstd_reads_real_file() {
        // 用真实 DSH 会话文件测试（如果存在）
        let files = discover_session_files();
        if files.is_empty() {
            return; // 跳过：无 DSH 会话文件
        }
        let content = decompress_zstd(&files[0]).expect("decompress");
        assert!(!content.is_empty());
        // 至少应包含 session 头
        assert!(content.contains("\"type\":\"session\""));
    }

    /// 完整验证：读取所有真实 DSH 会话文件，提取 TOKENS 并打印报告。
    #[test]
    fn verify_real_dsh_tokens() {
        let files = discover_session_files();
        if files.is_empty() {
            println!("⚠️  无 DSH 会话文件，跳过验证");
            return;
        }

        let mut total_files = 0;
        let mut total_usage_events = 0i64;
        let mut total_input: i64 = 0;
        let mut total_output: i64 = 0;
        let mut total_cache: i64 = 0;
        let mut total_reasoning: i64 = 0;
        let mut adapter_total_events = 0i64;
        let mut adapter_total_tokens: i64 = 0;
        let mut models = std::collections::HashSet::new();
        let mut session_ids = Vec::new();

        for path in &files {
            total_files += 1;
            let content = decompress_zstd(path).expect("decompress");
            let lines: Vec<&str> = content.lines().collect();

            // 用同一份解压内容跑适配器提取（避免活文件两次解压结果不一致）
            let events = extract_events("dsh", &lines, &format!("dsh:test:{}", hash_short(&path.to_string_lossy())));
            adapter_total_events += events.len() as i64;
            for e in &events {
                adapter_total_tokens += e.total_tokens.unwrap_or(0);
            }

            for line in &lines {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                let event_type = v.get("type").and_then(|x| x.as_str()).unwrap_or("");

                if event_type == "session" {
                    if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
                        session_ids.push(id.to_string());
                    }
                }
                if event_type == "request/header" {
                    if let Some(config) = v.pointer("/data/header/config") {
                        let provider = config.get("provider").and_then(|x| x.as_str()).unwrap_or("");
                        let model_name = config.get("model").and_then(|x| x.as_str()).unwrap_or("");
                        if !model_name.is_empty() {
                            models.insert(format!("{provider}/{model_name}"));
                        }
                    }
                }
                if event_type == "request/context" {
                    let provider = v.pointer("/data/provider").and_then(|x| x.as_str()).unwrap_or("");
                    let model_name = v.pointer("/data/model").and_then(|x| x.as_str()).unwrap_or("");
                    if !model_name.is_empty() {
                        models.insert(format!("{provider}/{model_name}"));
                    }
                }
                if event_type == "assistant/chunk" {
                    if let Some(chunk) = v.get("data").and_then(|d| d.get("chunk")) {
                        if chunk.get("type").and_then(|x| x.as_str()) == Some("usage") {
                            if let Some(usage) = chunk.get("usage") {
                                let input = usage.get("inputTokens").and_then(|x| x.as_i64()).unwrap_or(0);
                                let output = usage.get("outputTokens").and_then(|x| x.as_i64()).unwrap_or(0);
                                let cache = usage.get("cacheReadTokens").and_then(|x| x.as_i64()).unwrap_or(0);
                                let reasoning = usage.get("reasoningTokens").and_then(|x| x.as_i64()).unwrap_or(0);
                                total_usage_events += 1;
                                total_input += input;
                                total_output += output;
                                total_cache += cache;
                                total_reasoning += reasoning;
                            }
                        }
                    }
                }
            }
        }

        let grand_total = total_input + total_output + total_cache;

        println!();
        println!("=== DSH Token Monitor 验证报告 ===");
        println!("📂 会话文件: {} 个", total_files);
        println!("📋 会话 ID: {:?}", session_ids);
        println!("🤖 模型: {:?}", models);
        println!();
        println!("--- 原始解析 ---");
        println!("Usage 事件数: {}", total_usage_events);
        println!("  inputTokens:      {:>12}", total_input);
        println!("  outputTokens:     {:>12}", total_output);
        println!("  cacheReadTokens:  {:>12}（每请求缓存读取原值，官方口径直接求和）", total_cache);
        println!("  reasoningTokens:  {:>12}", total_reasoning);
        println!("  total (in+out+cache): {:>8}", grand_total);
        println!();
        println!("--- 适配器采集（原始 cache 口径，同一份输入） ---");
        println!("事件数: {}", adapter_total_events);
        println!("总 tokens: {}（input + output + cacheRead 原值求和）", adapter_total_tokens);

        assert!(total_usage_events > 0, "未采集到任何 usage 事件");
        assert!(adapter_total_tokens > 0, "适配器 token 总数为 0");
        assert_eq!(adapter_total_events, total_usage_events, "适配器事件数与原始解析不一致");
        // 适配器按原始 cache 求和，应与原始解析完全一致（对齐官方口径）
        assert_eq!(adapter_total_tokens, grand_total,
            "适配器 token 总数 {} 应与原始解析 {} 一致（原始 cache 口径）",
            adapter_total_tokens, grand_total);
        println!();
        println!("✅ 验证通过：DSH 适配器可正确采集 {} 个 usage 事件，共 {} tokens（原始 cache 口径）", total_usage_events, adapter_total_tokens);
    }
}
