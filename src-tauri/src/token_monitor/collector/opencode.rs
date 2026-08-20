//! 样板③ OpenCode（SQLite 只读增量）— W3a。
//!
//! - 数据源：`~/.local/share/opencode/opencode.db`（SQLite，`watch=false` 轮询）。
//! - 真实 schema（本机实测 2026-08）：`message(id TEXT PK, session_id, time_created, time_updated,
//!   data TEXT JSON)`。usage/model/时间都在 `data` JSON 里：
//!   `data.tokens = {total, input, output, reasoning, cache:{read, write}}`、
//!   `data.modelID`、`data.time.created`（epoch 毫秒）、`data.cost`、`data.path.cwd`。
//! - 增量：rowid 游标（`WHERE rowid > ?`，append-only 语义；重复由 fingerprint 去重）。
//! - 防御：表/列缺失 → `FormatChanged`（保留旧数据，不 panic）。

use std::path::PathBuf;

use crate::token_monitor::collector::common::{hash_short, open_readonly, to_utc_iso};
use crate::token_monitor::collector::CollectResult;
use crate::token_monitor::collector::ToolAdapter;
use crate::token_monitor::model::{
    AdapterCapabilities, CollectorCheckpoint, CollectorError, DataFormat, DataSource,
    IncrementalMode, NormalizedUsageEvent, SessionQuery, SessionSummary, SourceType, SupportLevel,
    ToolDescriptor, ToolKind, UsageAccuracy,
};

#[derive(Default)]
pub struct OpenCodeAdapter;

impl ToolAdapter for OpenCodeAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool_id: "opencode".into(),
            display_name: "OpenCode".into(),
            vendor: Some("opencode-ai".into()),
            kind: ToolKind::Usage,
            support_level: SupportLevel::Full,
            supported_os: vec!["macos".into(), "linux".into()],
            adapter_version: 2,
            privacy_note: "只读 opencode.db 的 message.data 元数据（tokens/model/时间）；不读取正文；只读连接不改动 WAL".into(),
        }
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            token: true,
            model: true,
            session: true,
            project: true,
            cache_tokens: true,
            cost: true,
            accuracy: UsageAccuracy::Exact,
            incremental: IncrementalMode::RecordId,
        }
    }

    fn discover(&self) -> Vec<DataSource> {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return Vec::new();
        };
        let db = home
            .join(".local")
            .join("share")
            .join("opencode")
            .join("opencode.db");
        if db.exists() {
            vec![DataSource {
                id: "opencode:main".into(),
                path: db,
                format: DataFormat::Sqlite,
                watch: false,
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
        let conn = open_readonly(&source.path)?;
        let last_rowid: i64 = checkpoint
            .last_record_id
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        // 防御式检查表与列
        let cols: Vec<String> = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(message)")
                .map_err(|e| CollectorError::Io(format!("schema: {e}")))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|e| CollectorError::Io(format!("schema: {e}")))?;
            rows.flatten().collect()
        };
        if cols.is_empty() {
            return Err(CollectorError::FormatChanged(
                "opencode.db 无 message 表（schema 变化）".into(),
            ));
        }
        if !cols.iter().any(|c| c == "data") {
            return Err(CollectorError::FormatChanged(
                "opencode message 表缺 data 列（schema 变化）".into(),
            ));
        }

        // 循环排空：一次扫描处理完所有新增行（LIMIT 500 分批，直到不足一批）
        let mut events = Vec::new();
        let mut max_rowid = last_rowid;
        loop {
            let sql = "SELECT rowid, session_id, data FROM message WHERE rowid > ?1 ORDER BY rowid LIMIT 500";
            let mut stmt = conn
                .prepare(sql)
                .map_err(|e| CollectorError::Io(format!("query: {e}")))?;
            let rows = stmt
                .query_map([max_rowid], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| CollectorError::Io(format!("query: {e}")))?;
            let rows: Vec<(i64, Option<String>, String)> = rows
                .collect::<Result<_, _>>()
                .map_err(|e| CollectorError::Io(format!("query: {e}")))?;
            drop(stmt);
            let batch = rows.len();
            if batch == 0 {
                break;
            }
            for (rowid, session_id, data) in rows {
                max_rowid = max_rowid.max(rowid);
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) else {
                    continue;
                };
                let tokens = value.get("tokens");
                let input = tokens.and_then(|t| t.get("input")).and_then(|v| v.as_i64());
                let output = tokens
                    .and_then(|t| t.get("output"))
                    .and_then(|v| v.as_i64());
                if input.is_none() && output.is_none() {
                    continue;
                }
                let occurred_at = value
                    .pointer("/time/created")
                    .or_else(|| value.get("time"))
                    .and_then(to_utc_iso)
                    .unwrap_or_else(|| "1970-01-01T00:00:00Z".into());
                let session_hash = session_id.map(|sid| hash_short(&format!("opencode:{sid}")));
                events.push(NormalizedUsageEvent {
                    source_type: SourceType::LocalDiscovered,
                    tool_id: "opencode".into(),
                    device_id: "local".into(),
                    model_raw: value
                        .get("modelID")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    model_normalized: None,
                    session_id: session_hash,
                    project_id: value
                        .pointer("/path/cwd")
                        .and_then(|v| v.as_str())
                        .map(hash_short),
                    account_id: None,
                    input_tokens: input,
                    output_tokens: output,
                    cache_read_tokens: tokens
                        .and_then(|t| t.pointer("/cache/read"))
                        .and_then(|v| v.as_i64()),
                    cache_write_tokens: tokens
                        .and_then(|t| t.pointer("/cache/write"))
                        .and_then(|v| v.as_i64()),
                    reasoning_tokens: tokens
                        .and_then(|t| t.get("reasoning"))
                        .and_then(|v| v.as_i64()),
                    message_count: None,
                    session_started_at: None,
                    session_last_active_at: None,
                    total_tokens: tokens.and_then(|t| t.get("total")).and_then(|v| v.as_i64()),
                    cost_amount: value.get("cost").and_then(|v| v.as_f64()),
                    cost_currency: None,
                    usage_accuracy: UsageAccuracy::Exact,
                    occurred_at,
                    source_locator_hash: Some(hash_short(&format!("opencode:{rowid}"))),
                });
            }
            if batch < 500 {
                break;
            }
        }

        Ok(CollectResult {
            events,
            sessions: Vec::new(),
            next_checkpoint: CollectorCheckpoint {
                source_id: checkpoint.source_id,
                last_record_id: Some(max_rowid.to_string()),
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
    use crate::token_monitor::model::CollectorCheckpoint;

    fn temp_db_with_rows(rows: &[(&str, &str, &str)]) -> (PathBuf, rusqlite::Connection) {
        let path = std::env::temp_dir().join(format!(
            "opencode-test-{}.db",
            uuid::Uuid::new_v4().simple()
        ));
        let conn = rusqlite::Connection::open(&path).expect("create db");
        conn.execute_batch(
            "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT NOT NULL);",
        )
        .expect("create table");
        for (id, session_id, data) in rows {
            conn.execute(
                "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, 0, 0, ?3)",
                rusqlite::params![id, session_id, data],
            )
            .expect("insert row");
        }
        (path, conn)
    }

    #[test]
    fn opencode_parses_real_data_json() {
        let (path, conn) = temp_db_with_rows(&[
            (
                "msg_a",
                "sess_1",
                r#"{"role":"assistant","tokens":{"total":37025,"input":35935,"output":24,"reasoning":42,"cache":{"write":0,"read":1024}},"modelID":"mimo-v2.5-free","providerID":"opencode","path":{"cwd":"/Users/xiangpeng/work/a"},"cost":0.12,"time":{"created":1780571052680}}"#,
            ),
            (
                "msg_b",
                "sess_1",
                r#"{"role":"user","content":"hello","path":{"cwd":"/Users/xiangpeng/work/a"},"time":{"created":1780571052000}}"#,
            ),
        ]);
        drop(conn);

        let adapter = OpenCodeAdapter::default();
        let src = DataSource {
            id: "opencode:main".into(),
            path,
            format: DataFormat::Sqlite,
            watch: false,
        };
        let r1 = adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .expect("collect");
        assert_eq!(r1.events.len(), 1, "user 行无 tokens 应跳过");
        let e = &r1.events[0];
        assert_eq!(e.input_tokens, Some(35935));
        assert_eq!(e.output_tokens, Some(24));
        assert_eq!(e.cache_read_tokens, Some(1024));
        assert_eq!(e.cache_write_tokens, Some(0));
        assert_eq!(e.reasoning_tokens, Some(42));
        assert_eq!(e.total_tokens, Some(37025));
        assert_eq!(e.model_raw.as_deref(), Some("mimo-v2.5-free"));
        assert_eq!(e.cost_amount, Some(0.12));
        assert!(e.project_id.is_some(), "cwd 应投影 project hash");
        assert!(e.session_id.is_some());

        // 幂等：同 checkpoint 无新增
        let r2 = adapter
            .collect_incremental(&src, r1.next_checkpoint.clone())
            .expect("recollect");
        assert!(r2.events.is_empty());
        std::fs::remove_file(src.path).ok();
    }

    #[test]
    fn opencode_missing_db_returns_path_missing() {
        let adapter = OpenCodeAdapter::default();
        let src = DataSource {
            id: "opencode:main".into(),
            path: PathBuf::from("/nonexistent/opencode.db"),
            format: DataFormat::Sqlite,
            watch: false,
        };
        assert!(adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .is_err());
    }

    #[test]
    fn opencode_readonly_does_not_create_file() {
        let adapter = OpenCodeAdapter::default();
        let src = DataSource {
            id: "x".into(),
            path: PathBuf::from("/nonexistent/opencode-x.db"),
            format: DataFormat::Sqlite,
            watch: false,
        };
        let result = adapter.collect_incremental(&src, CollectorCheckpoint::default());
        assert!(matches!(result, Err(CollectorError::PathMissing(_))));
    }
}
