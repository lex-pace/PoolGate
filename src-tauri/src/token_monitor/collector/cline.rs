//! Cline（VS Code globalStorage tasks / mtime）— W3b。
//! 数据源：`~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/`
//! 与 Linux 的 `~/.config/Code/.../tasks/` 下的任务 JSON（含 usage 明细）。
//! 整文件 JSON，mtime 判新 + fingerprint 去重。P 维度待 fixture 实测后开启。

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
        roots.push(
            home.join("Library")
                .join("Application Support")
                .join("Code")
                .join("User")
                .join("globalStorage")
                .join("saoudrizwan.claude-dev")
                .join("tasks"),
        );
        roots.push(
            home.join(".config")
                .join("Code")
                .join("User")
                .join("globalStorage")
                .join("saoudrizwan.claude-dev")
                .join("tasks"),
        );
    }
    roots
}

json_adapter! {
    ClineAdapter;
    tool_id = "cline";
    display_name = "Cline";
    vendor = "Cline Bot";
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
