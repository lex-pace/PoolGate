//! ZCode / GLM（JSONL transcript / offset）— W3c。
//! 真实格式（本机实测 2026-08）：`~/.zcode/cli/agents/sess_*/agent_*/transcript.jsonl`。
//! - usage 在 `model_complete` 行：`payload.usage = {inputTokens, outputTokens, totalTokens,
//!   cacheReadTokens, cacheWriteTokens}`（camelCase）。
//! - model 在 `model_request` 行：`payload.model`（跨行追踪；model_complete 不带 model）。
//! - 只读 usage/model，不读 `payload.messages`（Prompt 正文，隐私红线）。

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
pub struct ZcodeAdapter;

impl ToolAdapter for ZcodeAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool_id: "zcode".into(),
            display_name: "ZCode".into(),
            vendor: Some("Zhipu AI".into()),
            kind: ToolKind::Usage,
            support_level: SupportLevel::Standard,
            supported_os: vec!["macos".into(), "linux".into()],
            adapter_version: 2,
            privacy_note: "读取 ~/.zcode/cli/agents/**/transcript.jsonl 的 usage/model 元数据；不读取 Prompt/Response 正文；路径仅 hash 入库".into(),
        }
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            token: true,
            model: true,
            session: true,
            project: false,
            cache_tokens: true,
            cost: false,
            accuracy: UsageAccuracy::Exact,
            incremental: IncrementalMode::FileOffset,
        }
    }

    fn discover(&self) -> Vec<DataSource> {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return Vec::new();
        };
        let agents = home.join(".zcode").join("cli").join("agents");
        if !agents.exists() {
            return Vec::new();
        }
        collect_files(&agents, "jsonl", 3)
            .into_iter()
            .map(|path| DataSource {
                id: format!("zcode:{}", hash_short(&path.to_string_lossy())),
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
            return Ok(CollectResult {
                events: Vec::new(),
                sessions: Vec::new(),
                next_checkpoint: CollectorCheckpoint {
                    source_id: checkpoint.source_id,
                    byte_offset: Some(new_offset),
                    inode,
                    ..Default::default()
                },
            });
        }

        let file_stem = source
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("session")
            .to_string();
        let mut session_external: Option<String> = None; // 真实 sessionId（如 sess_xxx），行内捕获
        let mut current_model: Option<String> = None;
        let mut events = Vec::new();
        let mut input_sum = 0i64;
        let mut output_sum = 0i64;
        let mut cache_sum = 0i64;
        let mut first_ts: Option<String> = None;
        let mut last_ts: Option<String> = None;

        for line in &lines {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            // 捕获行级 sessionId（transcript 文件名恒为 transcript.jsonl，不能作会话标识）
            if session_external.is_none() {
                if let Some(s) = value.get("sessionId").and_then(|v| v.as_str()) {
                    if !s.trim().is_empty() {
                        session_external = Some(s.to_string());
                    }
                }
            }
            // 跨行追踪 model（model_request 行）；真实值形如 "<uuid>/gpt-5.6-sol"，取最后一段
            if let Some(m) = value.pointer("/payload/model").and_then(|v| v.as_str()) {
                if !m.trim().is_empty() {
                    current_model = Some(m.rsplit('/').next().unwrap_or(m).trim().to_string());
                }
            }
            let Some(usage) = value.pointer("/payload/usage") else {
                continue;
            };
            if usage.is_null() {
                continue;
            }
            let input = usage.get("inputTokens").and_then(|v| v.as_i64());
            let output = usage.get("outputTokens").and_then(|v| v.as_i64());
            if input.is_none() && output.is_none() {
                continue;
            }
            let occurred_at = value
                .get("timestamp")
                .and_then(to_utc_iso)
                .unwrap_or_else(|| Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
            let session_id = Some(hash_short(&format!("{}:zcode:{file_stem}", source.id)));

            input_sum += input.unwrap_or(0);
            output_sum += output.unwrap_or(0);
            let cache = usage.get("cacheReadTokens").and_then(|v| v.as_i64());
            cache_sum += cache.unwrap_or(0);
            if first_ts.is_none() {
                first_ts = Some(occurred_at.clone());
            }
            last_ts = Some(occurred_at.clone());

            events.push(NormalizedUsageEvent {
                source_type: SourceType::LocalDiscovered,
                tool_id: "zcode".into(),
                device_id: "local".into(),
                model_raw: current_model.clone(),
                model_normalized: None,
                session_id,
                project_id: None,
                account_id: None,
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: cache,
                cache_write_tokens: usage.get("cacheWriteTokens").and_then(|v| v.as_i64()),
                reasoning_tokens: None,
                message_count: None,
                session_started_at: None,
                session_last_active_at: None,
                total_tokens: usage.get("totalTokens").and_then(|v| v.as_i64()),
                cost_amount: None,
                cost_currency: None,
                usage_accuracy: UsageAccuracy::Exact,
                occurred_at,
                source_locator_hash: Some(hash_short(&format!("{new_offset}:{line}"))),
            });
        }

        let session = if events.is_empty() {
            None
        } else {
            Some(SessionSummary {
                session_id: hash_short(&format!("{}:zcode:{file_stem}", source.id)),
                tool_id: "zcode".into(),
                external_session_id: session_external.clone().or(Some(file_stem.clone())),
                project_id: None,
                title_redacted: Some(format!(
                    "会话 · {}",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> DataSource {
        DataSource {
            id: "zcode:test".into(),
            path: PathBuf::from("tests/fixtures/zcode/transcript.jsonl"),
            format: DataFormat::Jsonl,
            watch: false,
        }
    }

    #[test]
    fn zcode_parses_model_complete_usage_with_cross_line_model() {
        let adapter = ZcodeAdapter::default();
        let src = fixture();
        let r1 = adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .expect("collect");
        assert_eq!(r1.events.len(), 2, "应解析出 2 条 usage");
        assert_eq!(r1.events[0].model_raw.as_deref(), Some("mimo-v2.5-pro"));
        assert_eq!(r1.events[0].input_tokens, Some(3982));
        assert_eq!(r1.events[0].output_tokens, Some(96));
        assert_eq!(r1.events[0].cache_read_tokens, Some(512));
        assert_eq!(r1.events[0].cache_write_tokens, Some(0));
        assert_eq!(r1.events[0].total_tokens, Some(4078));
        // 幂等
        let r2 = adapter
            .collect_incremental(&src, r1.next_checkpoint.clone())
            .expect("recollect");
        assert!(r2.events.is_empty());
    }

    #[test]
    fn zcode_missing_file_returns_error() {
        let adapter = ZcodeAdapter::default();
        let src = DataSource {
            id: "x".into(),
            path: PathBuf::from("/nonexistent/zcode.jsonl"),
            format: DataFormat::Jsonl,
            watch: false,
        };
        assert!(adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .is_err());
    }
}
