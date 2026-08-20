//! 样板① Claude Code（JSONL 深度 Session）— W3a。
//!
//! - 数据源：`~/.claude/projects/**/*.jsonl`（每个工作目录一个子目录，子目录名是
//!   Claude Code 的路径编码）+ `~/.claude/transcripts/`。
//! - 行解析：只取含 `/message/usage` 的 assistant 行；**不读取 message.content**。
//! - Session：文件 = 会话；`external_session_id` = 文件名去扩展名；
//!   `title_redacted` = 项目名 + 起始时间（不取首条 Prompt）。
//! - 项目：完整路径仅 hash 入库；display 名取解码后的 basename。
//! - 增量：byte_offset + inode（轮转从 0 重读）。

use std::path::PathBuf;

use crate::token_monitor::collector::common::{
    claude_path_decode, collect_files, hash_short, parse_generic_event, read_jsonl_incremental,
    GenericFields,
};
use crate::token_monitor::collector::CollectResult;
use crate::token_monitor::collector::ToolAdapter;
use crate::token_monitor::model::{
    AdapterCapabilities, CollectorCheckpoint, CollectorError, DataFormat, DataSource,
    IncrementalMode, SessionQuery, SessionSummary, SupportLevel, ToolDescriptor, ToolKind,
    UsageAccuracy,
};

const FIELDS: GenericFields = GenericFields {
    input: &[
        "/message/usage/input_tokens",
        "/usage/input_tokens",
        "/input_tokens",
        "/input",
    ],
    output: &[
        "/message/usage/output_tokens",
        "/usage/output_tokens",
        "/output_tokens",
        "/output",
    ],
    cache: &[
        "/message/usage/cache_read_input_tokens",
        "/message/usage/cache_read_tokens",
        "/usage/cache_read_input_tokens",
        "/cache_read_tokens",
    ],
    model: &["/message/model", "/model", "/model_id"],
    ts: &["/timestamp", "/ts", "/created_at", "/time"],
    cost: &["/message/usage/cost_usd", "/cost_usd", "/cost"],
};

#[derive(Default)]
pub struct ClaudeCodeAdapter;

impl ToolAdapter for ClaudeCodeAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool_id: "claude_code".into(),
            display_name: "Claude Code".into(),
            vendor: Some("Anthropic".into()),
            kind: ToolKind::Usage,
            support_level: SupportLevel::Full,
            supported_os: vec!["macos".into(), "linux".into()],
            adapter_version: 1,
            privacy_note: "读取 ~/.claude/projects/**/*.jsonl 的 usage 元数据；不读取 Prompt/Response 正文；完整路径仅 hash 入库".into(),
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
        let Some(home) = dirs_home() else {
            return Vec::new();
        };
        let mut sources = Vec::new();
        let projects = home.join(".claude").join("projects");
        if projects.exists() {
            for file in collect_files(&projects, "jsonl", 4) {
                sources.push(DataSource {
                    id: format!("claude_code:{}", hash_short(&file.to_string_lossy())),
                    path: file,
                    format: DataFormat::Jsonl,
                    watch: true,
                });
            }
        }
        let transcripts = home.join(".claude").join("transcripts");
        if transcripts.exists() {
            for file in collect_files(&transcripts, "jsonl", 4) {
                sources.push(DataSource {
                    id: format!("claude_code:{}", hash_short(&file.to_string_lossy())),
                    path: file,
                    format: DataFormat::Jsonl,
                    watch: true,
                });
            }
        }
        sources
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
        let (lines, new_offset, inode) = read_jsonl_incremental(&source.path, &checkpoint)?;
        if lines.is_empty() {
            return Ok(empty_result(&checkpoint, new_offset, inode));
        }

        let file_stem = source
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("session")
            .to_string();
        // 项目：父目录名（Claude Code 路径编码）→ 解码显示名；完整路径仅 hash
        let project_dir_name = source
            .path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .map(ToString::to_string)
            .unwrap_or_default();
        let project_id = (!project_dir_name.is_empty()).then(|| hash_short(&project_dir_name));

        let mut events = Vec::new();
        let mut input_sum = 0i64;
        let mut output_sum = 0i64;
        let mut cache_sum = 0i64;
        let mut first_ts: Option<String> = None;
        let mut last_ts: Option<String> = None;

        for line in &lines {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(usage) = value.pointer("/message/usage") else {
                continue; // 非 assistant usage 行（user/tool 行、元数据行）
            };
            if usage.is_null() {
                continue;
            }
            let mut event = match parse_generic_event(
                &value,
                &FIELDS,
                "claude_code",
                "local",
                Some(file_stem.clone()),
                project_id.clone(),
            ) {
                Some(event) => event,
                None => continue,
            };
            event.source_locator_hash = Some(hash_short(&format!("{new_offset}:{line}")));
            // Anthropic 语义：total = input + output（normalization 兜底，这里直接留空）
            input_sum += event.input_tokens.unwrap_or(0);
            output_sum += event.output_tokens.unwrap_or(0);
            cache_sum += event.cache_read_tokens.unwrap_or(0);
            if first_ts.is_none() {
                first_ts = Some(event.occurred_at.clone());
            }
            last_ts = Some(event.occurred_at.clone());
            events.push(event);
        }

        let session = if events.is_empty() {
            None
        } else {
            let display_project = if project_dir_name.is_empty() {
                "会话".to_string()
            } else {
                claude_path_decode(&project_dir_name)
            };
            let display_project = display_project
                .rsplit('/')
                .next()
                .unwrap_or(&display_project)
                .to_string();
            Some(SessionSummary {
                session_id: hash_short(&format!("{}:claude_code:{file_stem}", source.id)),
                tool_id: "claude_code".into(),
                external_session_id: Some(file_stem),
                project_id,
                title_redacted: Some(format!(
                    "{display_project} · {}",
                    first_ts
                        .as_deref()
                        .and_then(|ts| ts.get(..16))
                        .unwrap_or("")
                )),
                model_set: Vec::new(),
                started_at: first_ts,
                last_active_at: last_ts,
                input_tokens: input_sum,
                output_tokens: output_sum,
                cache_tokens: cache_sum,
                total_tokens: input_sum + output_sum + cache_sum,
                message_count: events.len() as i64,
                status: Some("active".into()),
                cost_amount: None,
            })
        };

        Ok(CollectResult {
            events,
            sessions: session.into_iter().collect(),
            next_checkpoint: CollectorCheckpoint {
                source_id: checkpoint.source_id,
                byte_offset: Some(new_offset),
                inode,
                ..Default::default()
            },
        })
    }

    fn list_sessions(&self, query: &SessionQuery) -> Result<Vec<SessionSummary>, CollectorError> {
        let _ = query;
        // 会话视图由 W1 会话投影从 usage_event 聚合（本 Adapter 的 collect 已带 session 汇总）
        Ok(Vec::new())
    }
}

fn empty_result(cp: &CollectorCheckpoint, new_offset: u64, inode: Option<u64>) -> CollectResult {
    CollectResult {
        events: Vec::new(),
        sessions: Vec::new(),
        next_checkpoint: CollectorCheckpoint {
            source_id: cp.source_id.clone(),
            byte_offset: Some(new_offset),
            inode,
            ..Default::default()
        },
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_monitor::model::NormalizedUsageEvent;
    use std::io::Write;

    fn fixture(name: &str) -> DataSource {
        DataSource {
            id: "claude_code:test".into(),
            path: std::path::PathBuf::from(format!("tests/fixtures/claude_code/{name}")),
            format: DataFormat::Jsonl,
            watch: false,
        }
    }

    #[test]
    fn claude_code_incremental_is_idempotent() {
        let adapter = ClaudeCodeAdapter::default();
        let src = fixture("session_basic.jsonl");
        let r1 = adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .expect("basic collect");
        assert_eq!(r1.events.len(), 2, "basic 应解析出 2 条 usage");
        assert_eq!(r1.events[0].input_tokens, Some(100));
        assert_eq!(r1.events[0].cache_read_tokens, Some(50));
        assert!(r1.sessions.len() == 1, "应有 1 个 session 摘要");
        assert_eq!(
            r1.sessions[0].external_session_id.as_deref(),
            Some("session_basic")
        );
        // 不读正文：事件里不应有 prompt 内容字段
        assert!(r1.events[0].source_locator_hash.is_some());

        // 同一 checkpoint 再读 → 无新增
        let r2 = adapter
            .collect_incremental(&src, r1.next_checkpoint.clone())
            .expect("re-collect");
        assert!(r2.events.is_empty(), "重复采集应无新增");

        // 真实语义：同一文件追加 → 只读新增 1 行（拷贝到临时文件模拟 append）
        let temp = std::env::temp_dir().join(format!(
            "claude-test-{}.jsonl",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::copy(src.path.clone(), &temp).expect("copy basic");
        // 以临时文件为源重新采集（inode 对齐），再做 append 增量
        let temp_src = DataSource {
            path: temp.clone(),
            ..src.clone()
        };
        let base = adapter
            .collect_incremental(&temp_src, CollectorCheckpoint::default())
            .expect("temp basic");
        assert_eq!(base.events.len(), 2);
        let appended = std::fs::read_to_string("tests/fixtures/claude_code/session_append.jsonl")
            .expect("read append");
        let extra = appended
            .lines()
            .nth(3)
            .expect("append 应有第 4 行")
            .to_string();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&temp)
            .expect("open append")
            .write_all(format!("\n{extra}\n").as_bytes())
            .expect("append line");
        let r3 = adapter
            .collect_incremental(&temp_src, base.next_checkpoint)
            .expect("append collect");
        assert_eq!(r3.events.len(), 1, "append 应只多 1 条");
        std::fs::remove_file(&temp).ok();
    }

    #[test]
    fn claude_code_path_decode_roundtrip() {
        // '/' → -2f（2 位 hex）；多字节 UTF-8 → 4 位 hex 组（-e4b8ad = 中）
        assert_eq!(
            claude_path_decode("-2fUsers-2fxiangpeng-2fproject"),
            "/Users/xiangpeng/project"
        );
        assert_eq!(claude_path_decode("-e4-b8-ad-2fproject"), "中/project");
        assert_eq!(claude_path_decode("plain-name"), "plain-name");
    }

    #[test]
    fn claude_code_missing_file_is_error_not_panic() {
        let adapter = ClaudeCodeAdapter::default();
        let src = DataSource {
            id: "x".into(),
            path: PathBuf::from("/nonexistent/xyz.jsonl"),
            format: DataFormat::Jsonl,
            watch: false,
        };
        let result = adapter.collect_incremental(&src, CollectorCheckpoint::default());
        assert!(result.is_err());
    }

    #[test]
    fn claude_code_generates_stable_fingerprints() {
        // fingerprint 由 dedup 层基于事件内容生成；这里验证两行 usage 均非空事件
        let adapter = ClaudeCodeAdapter::default();
        let src = fixture("session_basic.jsonl");
        let r1 = adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .expect("collect");
        let events: Vec<NormalizedUsageEvent> = r1.events;
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .all(|e| e.input_tokens.is_some() && e.output_tokens.is_some()));
    }
}
