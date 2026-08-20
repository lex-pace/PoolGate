//! Kilo Code（VS Code globalStorage tasks / mtime）— W3b。
//! 数据源：`~/.vscode-oss`/Code 的 globalStorage/kilo.code.code-oss.tasks。整文件 JSON，mtime 判新。

use std::path::PathBuf;

use crate::token_monitor::collector::common::{
    collect_files, collect_json_array, hash_short, json_adapter, GenericFields,
};
use crate::token_monitor::model::{SupportLevel, UsageAccuracy};

const FIELDS: GenericFields = GenericFields {
    input: &[
        "/api_metrics/input_tokens",
        "/tokens/input",
        "/usage/input_tokens",
        "/input_tokens",
    ],
    output: &[
        "/api_metrics/output_tokens",
        "/tokens/output",
        "/usage/output_tokens",
        "/output_tokens",
    ],
    cache: &[
        "/api_metrics/cache_read_tokens",
        "/cache/read",
        "/cache_read_tokens",
    ],
    model: &["/model", "/api_metrics/model", "/request/model"],
    ts: &["/ts", "/timestamp", "/created_at"],
    cost: &[],
};

fn storage_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for ext in ["Code", "Code - Insiders", "VSCodium"] {
            roots.push(
                home.join("Library")
                    .join("Application Support")
                    .join(ext)
                    .join("User")
                    .join("globalStorage")
                    .join("kilocode.kilocode")
                    .join("tasks"),
            );
        }
    }
    roots
}

json_adapter! {
    KiloCodeAdapter;
    tool_id = "kilo_code";
    display_name = "Kilo Code";
    vendor = "Kilo Code";
    level = SupportLevel::Standard;
    session = false;
    project = false;
    cache = false;
    cost = false;
    accuracy = UsageAccuracy::Exact;
    dirs = { storage_roots() };
    ext = "json";
    fields = FIELDS;
}
