//! Hermes Agent（SQLite state.db / rowid）— W3c。
//! 数据源：`$HERMES_HOME/state.db` 或 `~/.hermes/state.db`。只读连接 + rowid 增量。

use std::path::PathBuf;

use crate::token_monitor::collector::common::{collect_sqlite_generic, hash_short};
use crate::token_monitor::collector::CollectResult;
use crate::token_monitor::collector::ToolAdapter;
use crate::token_monitor::model::{
    AdapterCapabilities, CollectorCheckpoint, CollectorError, DataFormat, DataSource,
    IncrementalMode, SessionQuery, SessionSummary, SupportLevel, ToolDescriptor, ToolKind,
    UsageAccuracy,
};

#[derive(Default)]
pub struct HermesAdapter;

impl ToolAdapter for HermesAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool_id: "hermes".into(),
            display_name: "Hermes Agent".into(),
            vendor: Some("Hermes".into()),
            kind: ToolKind::Usage,
            support_level: SupportLevel::Basic,
            supported_os: vec!["macos".into(), "linux".into()],
            adapter_version: 1,
            privacy_note: "只读 state.db 的 usage 元数据；不读取正文；路径仅 hash 入库".into(),
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
        let mut candidates = Vec::new();
        if let Some(home) = std::env::var_os("HERMES_HOME").map(PathBuf::from) {
            candidates.push(home.join("state.db"));
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            candidates.push(home.join(".hermes").join("state.db"));
        }
        candidates
            .into_iter()
            .filter(|p| p.exists())
            .map(|path| DataSource {
                id: format!("hermes:{}", hash_short(&path.to_string_lossy())),
                path,
                format: DataFormat::Sqlite,
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
        collect_sqlite_generic(
            source,
            &checkpoint,
            "hermes",
            &["message", "messages", "chat_message"],
            Some("model"),
            Some("created_at"),
        )
    }

    fn list_sessions(&self, _query: &SessionQuery) -> Result<Vec<SessionSummary>, CollectorError> {
        Ok(Vec::new())
    }
}
