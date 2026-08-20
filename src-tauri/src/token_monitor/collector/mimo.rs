//! MiMo Code（SQLite mimocode.db / rowid）— W3c。
//! 数据源：`~/.local/share/mimocode/mimocode.db`。只读连接 + rowid 增量。

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
pub struct MimoAdapter;

impl ToolAdapter for MimoAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool_id: "mimo".into(),
            display_name: "MiMo Code".into(),
            vendor: Some("Xiaomi".into()),
            kind: ToolKind::Usage,
            support_level: SupportLevel::Basic,
            supported_os: vec!["macos".into(), "linux".into()],
            adapter_version: 1,
            privacy_note: "只读 mimocode.db 的 usage 元数据；不读取正文；路径仅 hash 入库".into(),
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
            .join("mimocode")
            .join("mimocode.db");
        if db.exists() {
            vec![DataSource {
                id: "mimo:main".into(),
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
            "mimo",
            &["message", "messages", "chat_message"],
            Some("model"),
            Some("created_at"),
        )
    }

    fn list_sessions(&self, _query: &SessionQuery) -> Result<Vec<SessionSummary>, CollectorError> {
        Ok(Vec::new())
    }
}
