//! Kiro（CLI Session / JSONL）— W3b。
//! 数据源：`~/.kiro/sessions/`（CLI 会话 JSONL）+ `kiro-cli` DB。

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
    KiroAdapter;
    tool_id = "kiro";
    display_name = "Kiro";
    vendor = "Kiro";
    level = SupportLevel::Standard;
    session = false;
    project = true;
    cache = false;
    cost = false;
    accuracy = UsageAccuracy::Exact;
    dirs = {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        match home {
            Some(home) => vec![home.join(".kiro").join("sessions")],
            None => Vec::new(),
        }
    };
    ext = "jsonl";
    fields = FIELDS;
}
