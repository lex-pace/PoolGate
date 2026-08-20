//! Cursor（IDE storage / mtime）— W3b。
//! 数据源：`~/.cursor/` 下的 composer/session JSON。整文件 JSON，mtime 判新 + fingerprint 去重。

use std::path::PathBuf;

use crate::token_monitor::collector::common::{
    collect_files, collect_json_array, hash_short, json_adapter, GenericFields,
};
use crate::token_monitor::model::{SupportLevel, UsageAccuracy};

const FIELDS: GenericFields = GenericFields {
    input: &[
        "/requestTokens",
        "/input_tokens",
        "/usage/input_tokens",
        "/tokens/input",
    ],
    output: &[
        "/responseTokens",
        "/output_tokens",
        "/usage/output_tokens",
        "/tokens/output",
    ],
    cache: &["/cache_read_tokens", "/cache/read", "/tokens/cache"],
    model: &["/model", "/model_id", "/request/body/model"],
    ts: &["/timestamp", "/created_at", "/time"],
    cost: &[],
};

json_adapter! {
    CursorAdapter;
    tool_id = "cursor";
    display_name = "Cursor";
    vendor = "Anysphere";
    level = SupportLevel::Basic;
    session = false;
    project = false;
    cache = false;
    cost = false;
    accuracy = UsageAccuracy::Exact;
    dirs = {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        match home {
            Some(home) => vec![home.join(".cursor")],
            None => Vec::new(),
        }
    };
    ext = "json";
    fields = FIELDS;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_monitor::collector::ToolAdapter;
    use crate::token_monitor::model::{CollectorCheckpoint, DataFormat, DataSource};

    #[test]
    fn cursor_json_array_collect_and_mtime_skip() {
        let adapter = CursorAdapter::default();
        let src = DataSource {
            id: "cursor:test".into(),
            path: PathBuf::from("tests/fixtures/cursor/sessions.json"),
            format: DataFormat::Json,
            watch: false,
        };
        let r1 = adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .expect("collect");
        assert_eq!(r1.events.len(), 2, "sessions.json 应解析出 2 条 usage");
        // mtime 未变 → 第二次直接跳过
        let r2 = adapter
            .collect_incremental(&src, r1.next_checkpoint.clone())
            .expect("recollect");
        assert!(r2.events.is_empty(), "mtime 未变应跳过");
    }
}
