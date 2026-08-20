//! Grok Build（JSONL sessions / offset）— W3c。
//! 数据源：`$GROK_HOME/sessions/` 或 `~/.grok/sessions/`。

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
    GrokBuildAdapter;
    tool_id = "grok_build";
    display_name = "Grok Build";
    vendor = "xAI";
    level = SupportLevel::Standard;
    session = false;
    project = true;
    cache = false;
    cost = false;
    accuracy = UsageAccuracy::Exact;
    dirs = {
        let mut roots = Vec::new();
        if let Some(grok_home) = std::env::var_os("GROK_HOME").map(PathBuf::from) {
            roots.push(grok_home.join("sessions"));
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            roots.push(home.join(".grok").join("sessions"));
        }
        roots
    };
    ext = "jsonl";
    fields = FIELDS;
}
