//! 样板② Codex（JSONL sessions）— W3a。
//!
//! - 数据源：`~/.codex/sessions/**/*.jsonl`（每会话一文件）。
//! - 真实格式（本机实测 2026-08）：usage 在 `event_msg` 行的
//!   `payload.info.total_token_usage`（`input_tokens / cached_input_tokens / output_tokens /
//!   reasoning_output_tokens / total_tokens`）；model 与 cwd 在 `turn_context` /
//!   `session_meta` 行（`payload.model` / `payload.cwd`），需跨行追踪。
//!   同时兼容老格式 `payload.usage` + `payload.model`（fixture 与早期版本）。
//! - 只读 usage/model/cwd，不读正文。额度走 `quota/openai_codex.rs`。
//! - 增量：byte_offset + inode。

use std::path::PathBuf;

use chrono::Utc;
use serde_json::Value;

use crate::token_monitor::collector::common::{
    collect_files, hash_short, read_jsonl_incremental, to_utc_iso,
};
use crate::token_monitor::collector::CollectResult;
use crate::token_monitor::collector::ToolAdapter;
use crate::token_monitor::model::{
    AdapterCapabilities, CollectorCheckpoint, CollectorError, DataFormat, DataSource,
    IncrementalMode, NormalizedUsageEvent, SessionQuery, SessionSummary, SourceType, SupportLevel,
    ToolDescriptor, ToolKind, UsageAccuracy,
};

#[derive(Default)]
pub struct CodexAdapter;

impl ToolAdapter for CodexAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool_id: "codex".into(),
            display_name: "Codex CLI".into(),
            vendor: Some("OpenAI".into()),
            kind: ToolKind::Usage,
            support_level: SupportLevel::Full,
            supported_os: vec!["macos".into(), "linux".into()],
            adapter_version: 2,
            privacy_note: "读取 ~/.codex/sessions/**/*.jsonl 的 usage 元数据；不读取 Prompt/Response 正文；完整路径仅 hash 入库".into(),
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
            accuracy: UsageAccuracy::ProviderReported,
            incremental: IncrementalMode::FileOffset,
        }
    }

    fn discover(&self) -> Vec<DataSource> {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return Vec::new();
        };
        let sessions = home.join(".codex").join("sessions");
        if !sessions.exists() {
            return Vec::new();
        }
        collect_files(&sessions, "jsonl", 4)
            .into_iter()
            .map(|path| DataSource {
                id: format!("codex:{}", hash_short(&path.to_string_lossy())),
                path,
                format: DataFormat::Jsonl,
                watch: true,
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

        let mut current_model: Option<String> = None;
        let mut current_cwd: Option<String> = None;
        // 真实新格式：`total_token_usage` 是**会话级累计值**（同一值会重复出现、单调递增），
        // 若按行计数会重复计量百倍。只保留文件内最终累计值，每会话只产出一条事件。
        let mut cumulative: Option<(i64, i64, i64, i64, i64, String)> = None; // (input,output,cache,reasoning,total,ts)
        let mut cumulative_seen = false;
        let mut events = Vec::new(); // 老格式 /payload/usage（逐条）
        let mut input_sum = 0i64;
        let mut output_sum = 0i64;
        let mut cache_sum = 0i64;
        let mut first_ts: Option<String> = None;
        let mut last_ts: Option<String> = None;

        for line in &lines {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            // 跨行追踪 model / cwd（turn_context、session_meta、response_item 行携带）
            if let Some(m) = value.pointer("/payload/model").and_then(|v| v.as_str()) {
                if !m.trim().is_empty() {
                    current_model = Some(m.to_string());
                }
            }
            if let Some(c) = value.pointer("/payload/cwd").and_then(|v| v.as_str()) {
                if !c.trim().is_empty() {
                    current_cwd = Some(c.to_string());
                }
            }

            // 累计新格式：/payload/info/total_token_usage
            if let Some(usage) = value.pointer("/payload/info/total_token_usage") {
                if !usage.is_null() {
                    cumulative_seen = true;
                    let input = usage
                        .get("input_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let output = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    if input == 0 && output == 0 {
                        continue;
                    }
                    let total = usage
                        .get("total_tokens")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(input + output);
                    let grew = cumulative.as_ref().map(|c| total > c.4).unwrap_or(true);
                    if grew {
                        let occurred_at = value
                            .get("timestamp")
                            .and_then(to_utc_iso)
                            .unwrap_or_else(|| {
                                Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                            });
                        cumulative = Some((
                            input,
                            output,
                            usage
                                .get("cached_input_tokens")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0),
                            usage
                                .get("reasoning_output_tokens")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0),
                            total,
                            occurred_at,
                        ));
                    }
                    continue;
                }
            }
            // 老格式：/payload/usage（逐条）。一旦本文件出现过累计格式，不再走逐条
            // （避免混格式文件双重计数）
            if cumulative_seen {
                continue;
            }
            let Some(usage) = value.pointer("/payload/usage") else {
                continue;
            };
            if usage.is_null() {
                continue;
            }
            let input = usage.get("input_tokens").and_then(|v| v.as_i64());
            let output = usage.get("output_tokens").and_then(|v| v.as_i64());
            if input.is_none() && output.is_none() {
                continue;
            }
            let cache = usage
                .get("cached_input_tokens")
                .and_then(|v| v.as_i64())
                .or_else(|| {
                    usage
                        .get("cache_read_input_tokens")
                        .and_then(|v| v.as_i64())
                });
            let occurred_at = value
                .get("timestamp")
                .and_then(to_utc_iso)
                .unwrap_or_else(|| Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
            let project_id = current_cwd
                .as_ref()
                .filter(|c| !c.is_empty())
                .map(|c| hash_short(c));
            let session_id = Some(hash_short(&format!("{}:codex:{file_stem}", source.id)));

            input_sum += input.unwrap_or(0);
            output_sum += output.unwrap_or(0);
            cache_sum += cache.unwrap_or(0);
            if first_ts.is_none() {
                first_ts = Some(occurred_at.clone());
            }
            last_ts = Some(occurred_at.clone());

            events.push(NormalizedUsageEvent {
                source_type: SourceType::LocalDiscovered,
                tool_id: "codex".into(),
                device_id: "local".into(),
                model_raw: current_model.clone(),
                model_normalized: None,
                session_id,
                project_id,
                account_id: None,
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: cache,
                cache_write_tokens: usage
                    .get("cache_creation_input_tokens")
                    .and_then(|v| v.as_i64()),
                reasoning_tokens: usage
                    .get("reasoning_output_tokens")
                    .and_then(|v| v.as_i64()),
                message_count: None,
                session_started_at: None,
                session_last_active_at: None,
                total_tokens: usage.get("total_tokens").and_then(|v| v.as_i64()),
                cost_amount: None,
                cost_currency: None,
                usage_accuracy: UsageAccuracy::ProviderReported,
                occurred_at,
                source_locator_hash: Some(hash_short(&format!("{new_offset}:{line}"))),
            });
        }

        // 累计格式：只产出最终累计值一条
        if let Some((input, output, cache, reasoning, total, occurred_at)) = cumulative.take() {
            let project_id = current_cwd
                .as_ref()
                .filter(|c| !c.is_empty())
                .map(|c| hash_short(c));
            let session_id = Some(hash_short(&format!("{}:codex:{file_stem}", source.id)));
            input_sum += input;
            output_sum += output;
            cache_sum += cache;
            if first_ts.is_none() {
                first_ts = Some(occurred_at.clone());
            }
            last_ts = Some(occurred_at.clone());
            events.push(NormalizedUsageEvent {
                source_type: SourceType::LocalDiscovered,
                tool_id: "codex".into(),
                device_id: "local".into(),
                model_raw: current_model.clone(),
                model_normalized: None,
                session_id,
                project_id,
                account_id: None,
                input_tokens: Some(input),
                output_tokens: Some(output),
                cache_read_tokens: Some(cache),
                cache_write_tokens: None,
                reasoning_tokens: Some(reasoning),
                message_count: None,
                session_started_at: None,
                session_last_active_at: None,
                total_tokens: Some(total),
                cost_amount: None,
                cost_currency: None,
                usage_accuracy: UsageAccuracy::ProviderReported,
                occurred_at,
                source_locator_hash: Some(hash_short(&format!(
                    "{}:codex:{file_stem}:cumulative",
                    source.id
                ))),
            });
        }

        let session = if events.is_empty() {
            None
        } else {
            let display_project = current_cwd
                .as_ref()
                .and_then(|c| c.rsplit('/').next())
                .filter(|s| !s.is_empty())
                .unwrap_or("会话");
            Some(SessionSummary {
                session_id: hash_short(&format!("{}:codex:{file_stem}", source.id)),
                tool_id: "codex".into(),
                external_session_id: Some(file_stem),
                project_id: current_cwd.as_ref().map(|c| hash_short(c)),
                title_redacted: Some(format!(
                    "{display_project} · {}",
                    first_ts
                        .as_deref()
                        .and_then(|ts| ts.get(..16))
                        .unwrap_or("")
                )),
                model_set: current_model.iter().cloned().collect(),
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

    fn list_sessions(&self, _query: &SessionQuery) -> Result<Vec<SessionSummary>, CollectorError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture(name: &str) -> DataSource {
        DataSource {
            id: "codex:test".into(),
            path: std::path::PathBuf::from(format!("tests/fixtures/codex/{name}")),
            format: DataFormat::Jsonl,
            watch: false,
        }
    }

    #[test]
    fn codex_incremental_is_idempotent_and_append_safe() {
        let adapter = CodexAdapter::default();
        let src = fixture("session_basic.jsonl");
        let r1 = adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .expect("basic");
        assert_eq!(r1.events.len(), 2, "basic 应有 2 条 usage");
        assert!(r1.events[0].input_tokens.is_some() && r1.events[0].output_tokens.is_some());

        let r2 = adapter
            .collect_incremental(&src, r1.next_checkpoint.clone())
            .expect("recollect");
        assert!(r2.events.is_empty());

        // 同一文件追加 → 只读新增 1 条（临时文件模拟）
        let temp = std::env::temp_dir().join(format!(
            "codex-test-{}.jsonl",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::copy(src.path.clone(), &temp).expect("copy basic");
        let temp_src = DataSource {
            path: temp.clone(),
            ..src.clone()
        };
        let base = adapter
            .collect_incremental(&temp_src, CollectorCheckpoint::default())
            .expect("temp basic");
        assert_eq!(base.events.len(), 2);
        let appended = std::fs::read_to_string("tests/fixtures/codex/session_append.jsonl")
            .expect("read append");
        let extra = appended.lines().nth(2).expect("append 第 3 行").to_string();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&temp)
            .expect("open append")
            .write_all(format!("\n{extra}\n").as_bytes())
            .expect("append line");
        let r3 = adapter
            .collect_incremental(&temp_src, base.next_checkpoint)
            .expect("append");
        assert_eq!(r3.events.len(), 1, "append 应只多 1 条");
        std::fs::remove_file(&temp).ok();
    }

    #[test]
    fn codex_real_info_format_with_cross_line_model() {
        // 真实新格式：usage 在 event_msg.payload.info.total_token_usage，
        // model/cwd 在 turn_context 行（跨行追踪）
        let adapter = CodexAdapter::default();
        let src = fixture("session_info.jsonl");
        let r1 = adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .expect("collect");
        // 累计格式：重复值与单调递增 → 只产出最终累计值一条（避免重复计量百倍）
        assert_eq!(r1.events.len(), 1, "info 累计格式应只产出一条最终累计事件");
        assert_eq!(r1.events[0].model_raw.as_deref(), Some("gpt-5.4"));
        assert_eq!(r1.events[0].input_tokens, Some(22472));
        assert_eq!(r1.events[0].cache_read_tokens, Some(12000));
        assert_eq!(r1.events[0].reasoning_tokens, Some(15));
        assert_eq!(r1.events[0].total_tokens, Some(22498));
        assert!(r1.events[0].project_id.is_some(), "cwd 应投影 project hash");
        // 第二次同 checkpoint → 无新增（幂等）
        let r2 = adapter
            .collect_incremental(&src, r1.next_checkpoint.clone())
            .expect("recollect");
        assert!(r2.events.is_empty());
    }

    #[test]
    fn codex_missing_file_returns_error() {
        let adapter = CodexAdapter::default();
        let src = DataSource {
            id: "x".into(),
            path: PathBuf::from("/nonexistent/codex.jsonl"),
            format: DataFormat::Jsonl,
            watch: false,
        };
        assert!(adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .is_err());
    }
}
