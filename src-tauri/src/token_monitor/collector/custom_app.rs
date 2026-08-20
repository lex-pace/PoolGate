//! 自定义应用监控（用户自注册工具）。
//!
//! 用户在 UI 里注册「应用名 + 日志文件路径（JSONL）」，采集器按通用 usage 字段
//! 解析，事件以 `tool_id = custom:<id>` 落库，走与内置工具完全相同的
//! watcher → 采集 → `usage-delta` 实时链路（今日 Tokens / 托盘 Tokens 秒级刷新）。
//!
//! 自定义应用 = `tool_definition` 中 `tool_id LIKE 'custom:%'` 的行：
//! - `display_name`      用户填的应用名
//! - `custom_paths_json` 日志路径列表（与内置工具的路径覆盖同列）
//! - `custom_fields_json` 可选字段映射（020 迁移新增）；留空 = 内置通用默认字段
//!
//! 解析口径与 `common::collect_jsonl_incremental` 一致（增量 byte_offset、轮转感知、
//! 幂等指纹、不读 Prompt/Response 正文）。

use std::path::PathBuf;
use std::sync::Mutex;

use crate::token_monitor::collector::common;
use crate::token_monitor::collector::{CollectResult, ToolAdapter};
use crate::token_monitor::model::{
    AdapterCapabilities, CollectorCheckpoint, CollectorError, DataSource, IncrementalMode,
    NormalizedUsageEvent, SupportLevel, ToolDescriptor, ToolKind, UsageAccuracy,
};

/// 用户自定义字段映射（与 `GenericFields` 同构；路径支持 `/usage/input_tokens` JSON Pointer）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CustomFields {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
    #[serde(default)]
    pub cache: Vec<String>,
    #[serde(default)]
    pub model: Vec<String>,
    #[serde(default)]
    pub ts: Vec<String>,
    #[serde(default)]
    pub cost: Vec<String>,
}

/// 内置通用默认字段：覆盖常见 JSONL usage 形态（input/output/cache/model/ts/cost）。
pub(crate) fn default_fields() -> CustomFields {
    CustomFields {
        input: vec![
            "/usage/input_tokens".into(),
            "/input_tokens".into(),
            "/usage/prompt_tokens".into(),
            "/prompt_tokens".into(),
            "/tokens/input".into(),
            "/input".into(),
        ],
        output: vec![
            "/usage/output_tokens".into(),
            "/output_tokens".into(),
            "/usage/completion_tokens".into(),
            "/completion_tokens".into(),
            "/tokens/output".into(),
            "/output".into(),
        ],
        cache: vec![
            "/usage/cache_read_input_tokens".into(),
            "/cache_read_tokens".into(),
            "/usage/cache_read_tokens".into(),
            "/cache/read".into(),
            "/cache".into(),
        ],
        model: vec![
            "/model".into(),
            "/model_id".into(),
            "/model_name".into(),
            "/message/model".into(),
        ],
        ts: vec![
            "/timestamp".into(),
            "/ts".into(),
            "/created_at".into(),
            "/time".into(),
            "/occurred_at".into(),
        ],
        cost: vec!["/cost".into(), "/cost_amount".into(), "/usage/cost".into()],
    }
}

/// 从 tool_definition 读自定义应用的字段映射（缺省/NULL → 默认）。
pub(crate) fn load_fields(
    conn: &Mutex<rusqlite::Connection>,
    tool_id: &str,
) -> CustomFields {
    let Ok(conn) = conn.lock() else {
        return default_fields();
    };
    let fields: Option<String> = conn
        .query_row(
            "SELECT custom_fields_json FROM tool_definition WHERE tool_id=?1",
            rusqlite::params![tool_id],
            |row| row.get(0),
        )
        .ok();
    drop(conn);
    fields
        .and_then(|json| serde_json::from_str::<CustomFields>(&json).ok())
        .unwrap_or_else(default_fields)
}

/// 按 tool_id 构造自定义应用适配器（仅当该行存在且启用时返回 Some）。
pub(crate) fn adapter_for(
    conn: &Mutex<rusqlite::Connection>,
    tool_id: &str,
) -> Option<CustomAppAdapter> {
    let (display_name, enabled): (String, i64) = {
        let Ok(guard) = conn.lock() else {
            return None;
        };
        guard
            .query_row(
                "SELECT display_name, enabled FROM tool_definition WHERE tool_id=?1",
                rusqlite::params![tool_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok()?
    };
    if enabled == 0 {
        return None;
    }
    Some(CustomAppAdapter {
        tool_id: tool_id.into(),
        display_name,
        fields: load_fields(conn, tool_id),
    })
}

/// 自定义应用适配器（动态 ToolAdapter，复用标准采集流程）。
pub(crate) struct CustomAppAdapter {
    pub tool_id: String,
    pub display_name: String,
    pub fields: CustomFields,
}

impl ToolAdapter for CustomAppAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool_id: self.tool_id.clone(),
            display_name: self.display_name.clone(),
            vendor: None,
            kind: ToolKind::Usage,
            support_level: SupportLevel::Basic,
            supported_os: vec!["macos".into(), "windows".into(), "linux".into()],
            adapter_version: 1,
            privacy_note: "读取用户指定的 JSONL 日志元数据（tokens/模型/时间）".into(),
        }
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            token: true,
            model: true,
            session: false,
            project: false,
            cache_tokens: true,
            cost: true,
            accuracy: UsageAccuracy::Exact,
            incremental: IncrementalMode::FileOffset,
        }
    }

    fn discover(&self) -> Vec<DataSource> {
        // 源由 refresh_sources 按 custom_paths_json 动态构建，无需自发现。
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
    ) -> Result<CollectResult, CollectorError> {
        let (lines, new_offset, inode) = common::read_jsonl_incremental(&source.path, &checkpoint)?;
        if lines.is_empty() {
            return Ok(CollectResult {
                events: Vec::new(),
                sessions: Vec::new(),
                next_checkpoint: CollectorCheckpoint {
                    source_id: checkpoint.source_id.clone(),
                    byte_offset: Some(new_offset),
                    inode,
                    ..Default::default()
                },
            });
        }

        let mut events = Vec::new();
        for line in lines {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let input = common::pick(&value, &self.fields.input).and_then(|v| v.as_i64());
            let output = common::pick(&value, &self.fields.output).and_then(|v| v.as_i64());
            if input.is_none() && output.is_none() {
                continue; // 非 usage 行（元数据/事件行）
            }
            let occurred_at = common::pick(&value, &self.fields.ts)
                .and_then(common::to_utc_iso)
                .unwrap_or_else(|| {
                    chrono::Utc::now()
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                });
            let model_raw = common::pick(&value, &self.fields.model)
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let cache = common::pick(&value, &self.fields.cache).and_then(|v| v.as_i64());
            let cost = common::pick(&value, &self.fields.cost)
                .and_then(|v| v.as_f64())
                .or_else(|| {
                    common::pick(&value, &self.fields.cost)
                        .and_then(|v| v.as_i64())
                        .map(|n| n as f64)
                });
            let mut event = NormalizedUsageEvent {
                source_type: crate::token_monitor::model::SourceType::LocalDiscovered,
                tool_id: self.tool_id.clone(),
                device_id: "local".into(),
                model_raw,
                model_normalized: None,
                session_id: None,
                project_id: None,
                account_id: None,
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: cache,
                cache_write_tokens: None,
                reasoning_tokens: None,
                message_count: None,
                session_started_at: None,
                session_last_active_at: None,
                total_tokens: None,
                cost_amount: cost,
                cost_currency: None,
                usage_accuracy: UsageAccuracy::Exact,
                occurred_at,
                source_locator_hash: None,
            };
            event.source_locator_hash = Some(common::hash_short(&format!("{new_offset}:{line}")));
            events.push(event);
        }

        Ok(CollectResult {
            events,
            sessions: Vec::new(),
            next_checkpoint: CollectorCheckpoint {
                source_id: checkpoint.source_id.clone(),
                byte_offset: Some(new_offset),
                inode,
                ..Default::default()
            },
        })
    }
}

/// 生成唯一 tool_id：`custom:<slug>-<4 位 hex>`（slug 来自应用名，保证可读且唯一）。
pub(crate) fn make_tool_id(display_name: &str) -> String {
    let slug: String = display_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "app".into() } else { slug };
    let suffix: String = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(4)
        .collect();
    format!("custom:{slug}-{suffix}")
}

/// 工具 id 是否为自定义应用（`custom:` 前缀）。
pub(crate) fn is_custom_app(tool_id: &str) -> bool {
    tool_id.starts_with("custom:")
}

/// 解析自定义应用的有效路径（tool_definition 中 `custom:%` 且 enabled 的行）。
pub(crate) fn load_custom_app_paths(
    conn: &Mutex<rusqlite::Connection>,
) -> Vec<(String, PathBuf)> {
    let Ok(conn) = conn.lock() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT tool_id, COALESCE(custom_paths_json,'[]') FROM tool_definition \
         WHERE tool_id LIKE 'custom:%' AND enabled=1",
    ) else {
        return out;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return out;
    };
    for row in rows.flatten() {
        if let Ok(paths) = serde_json::from_str::<Vec<String>>(&row.1) {
            for path in paths {
                if !path.trim().is_empty() {
                    out.push((row.0.clone(), PathBuf::from(path)));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn sample_source(dir: &std::path::Path, name: &str, body: &str) -> DataSource {
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).expect("create log");
        file.write_all(body.as_bytes()).expect("write log");
        DataSource {
            id: "custom:test:src".into(),
            path,
            format: crate::token_monitor::model::DataFormat::Jsonl,
            watch: true,
        }
    }

    #[test]
    fn parses_default_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = sample_source(
            dir.path(),
            "session.jsonl",
            "{\"ts\":\"2026-08-13T10:00:00Z\",\"model\":\"gpt-4o\",\"input_tokens\":10,\"output_tokens\":20}\n\
             {\"ts\":\"2026-08-13T10:01:00Z\",\"model\":\"claude-3.5\",\"usage\":{\"input_tokens\":100,\"output_tokens\":50,\"cache_read_input_tokens\":30}}\n\
             {\"ts\":\"2026-08-13T10:02:00Z\",\"note\":\"metadata only\"}\n",
        );
        let adapter = CustomAppAdapter {
            tool_id: "custom:demo".into(),
            display_name: "Demo".into(),
            fields: default_fields(),
        };
        let result = adapter
            .collect_incremental(&source, CollectorCheckpoint::default())
            .expect("collect");
        assert_eq!(result.events.len(), 2, "metadata line skipped");
        assert_eq!(result.events[0].input_tokens, Some(10));
        assert_eq!(result.events[0].output_tokens, Some(20));
        assert_eq!(result.events[1].input_tokens, Some(100));
        assert_eq!(result.events[1].cache_read_tokens, Some(30));
        assert_eq!(result.events[1].model_raw.as_deref(), Some("claude-3.5"));
    }

    #[test]
    fn parses_custom_fields_and_incremental() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = sample_source(
            dir.path(),
            "log.ndjson",
            "{\"created\":\"2026-08-13T10:00:00Z\",\"prompt_tokens\":5,\"completion_tokens\":7}\n",
        );
        let adapter = CustomAppAdapter {
            tool_id: "custom:demo".into(),
            display_name: "Demo".into(),
            fields: CustomFields {
                input: vec!["/prompt_tokens".into()],
                output: vec!["/completion_tokens".into()],
                ts: vec!["/created".into()],
                ..Default::default()
            },
        };
        let result = adapter
            .collect_incremental(&source, CollectorCheckpoint::default())
            .expect("collect");
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].input_tokens, Some(5));
        assert_eq!(result.events[0].output_tokens, Some(7));
        let next = &result.next_checkpoint;
        assert!(next.byte_offset.unwrap_or(0) > 0);

        // 增量：再次采集无新行 → 0 事件
        let second = adapter
            .collect_incremental(&source, next.clone())
            .expect("collect again");
        assert!(second.events.is_empty());
    }

    #[test]
    fn tool_id_slug_unique() {
        let a = make_tool_id("我的 App v2");
        let b = make_tool_id("我的 App v2");
        assert!(a.starts_with("custom:app-v2-"));
        assert_ne!(a, b, "suffix keeps ids unique");
        assert!(is_custom_app(&a));
        assert!(!is_custom_app("claude_code"));
    }
}
