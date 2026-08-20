//! CodeBuddy（JSON/JSONL / offset+mtime）— W3b。
//! 数据源：`~/.codebuddy/projects/` + IDE/扩展日志。

use std::path::PathBuf;

use crate::token_monitor::collector::common::{
    collect_files, collect_jsonl_incremental, hash_short, jsonl_adapter, GenericFields,
};
use crate::token_monitor::model::{SupportLevel, UsageAccuracy};

const FIELDS: GenericFields = GenericFields {
    input: &[
        "/usage/input_tokens",
        "/input_tokens",
        "/tokens/input",
        "/input",
    ],
    output: &[
        "/usage/output_tokens",
        "/output_tokens",
        "/tokens/output",
        "/output",
    ],
    cache: &[
        "/usage/cache_read_input_tokens",
        "/cache_read_tokens",
        "/cache/read",
    ],
    model: &["/model", "/model_id", "/message/model"],
    ts: &["/timestamp", "/ts", "/created_at", "/time"],
    cost: &[],
};

jsonl_adapter! {
    CodebuddyAdapter;
    tool_id = "codebuddy";
    display_name = "CodeBuddy";
    vendor = "Tencent";
    level = SupportLevel::Standard;
    session = false;
    project = true;
    cache = false;
    cost = false;
    accuracy = UsageAccuracy::Exact;
    dirs = {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        match home {
            Some(home) => vec![home.join(".codebuddy").join("projects")],
            None => Vec::new(),
        }
    };
    ext = "jsonl";
    fields = FIELDS;
}
