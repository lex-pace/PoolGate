//! Zed（SQLite threads.db / rowid）— W3c。
//! 数据源：`~/.local/share/zed/threads.db`。只读连接 + rowid 增量。

use std::path::PathBuf;

use crate::token_monitor::collector::common::collect_sqlite_generic;
use crate::token_monitor::collector::CollectResult;
use crate::token_monitor::collector::ToolAdapter;
use crate::token_monitor::model::{
    AdapterCapabilities, CollectorCheckpoint, CollectorError, DataFormat, DataSource,
    IncrementalMode, SessionQuery, SessionSummary, SupportLevel, ToolDescriptor, ToolKind,
    UsageAccuracy,
};

#[derive(Default)]
pub struct ZedAdapter;

impl ToolAdapter for ZedAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool_id: "zed".into(),
            display_name: "Zed".into(),
            vendor: Some("Zed Industries".into()),
            kind: ToolKind::Usage,
            support_level: SupportLevel::Basic,
            supported_os: vec!["macos".into(), "linux".into()],
            adapter_version: 1,
            privacy_note: "只读 threads.db 的 usage 元数据；不读取正文；路径仅 hash 入库".into(),
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
            .join("zed")
            .join("threads.db");
        if db.exists() {
            vec![DataSource {
                id: "zed:main".into(),
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
        collect_sqlite_generic(
            source,
            &checkpoint,
            "zed",
            &["message", "messages", "assistant_message"],
            Some("model"),
            Some("created_at"),
        )
    }

    fn list_sessions(&self, _query: &SessionQuery) -> Result<Vec<SessionSummary>, CollectorError> {
        Ok(Vec::new())
    }
}
