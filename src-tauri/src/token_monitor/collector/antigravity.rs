//! Antigravity（IDE storage / mtime）— W3b。
//! 数据源：`~/.antigravity/` 会话 JSON。整文件 JSON，mtime 判新。

use std::path::PathBuf;

use crate::token_monitor::collector::common::{
    collect_files, collect_json_array, hash_short, json_adapter, GenericFields,
};
use crate::token_monitor::model::{SupportLevel, UsageAccuracy};

const FIELDS: GenericFields = GenericFields {
    input: &[
        "/input_tokens",
        "/usage/input_tokens",
        "/tokens/input",
        "/input",
    ],
    output: &[
        "/output_tokens",
        "/usage/output_tokens",
        "/tokens/output",
        "/output",
    ],
    cache: &["/cache_read_tokens", "/cache/read", "/tokens/cache"],
    model: &["/model", "/model_id", "/request/body/model"],
    ts: &["/timestamp", "/created_at", "/time"],
    cost: &[],
};

json_adapter! {
    AntigravityAdapter;
    tool_id = "antigravity";
    display_name = "Antigravity";
    vendor = "Google";
    level = SupportLevel::Basic;
    session = false;
    project = false;
    cache = false;
    cost = false;
    accuracy = UsageAccuracy::Exact;
    dirs = {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        match home {
            Some(home) => vec![home.join(".antigravity")],
            None => Vec::new(),
        }
    };
    ext = "json";
    fields = FIELDS;
}
