//! GitHub Copilot（OTel NDJSON / offset）— W3b。
//! 数据源：`~/.copilot/otel/` 下 OTel 导出日志（NDJSON，每行 JSON）。按行解析 usage/model。
//! 不读取 Prompt/Response 正文（otel 行里存在 content 时跳过正文字段）。

use std::path::PathBuf;

use crate::token_monitor::collector::common::{
    collect_files, collect_jsonl_incremental, hash_short, jsonl_adapter, GenericFields,
};
use crate::token_monitor::model::{SupportLevel, UsageAccuracy};

// 注意：OTel 行若只有 total_tokens（无 input/output 拆分）→ 不作为 usage 行（能力诚实）。
const FIELDS: GenericFields = GenericFields {
    input: &[
        "/attributes/llm.usage.prompt_tokens",
        "/attributes/token_usage/prompt",
        "/input_tokens",
    ],
    output: &[
        "/attributes/llm.usage.completion_tokens",
        "/attributes/token_usage/completion",
        "/output_tokens",
    ],
    cache: &[
        "/attributes/llm.usage.cache_read_tokens",
        "/cache_read_tokens",
    ],
    model: &[
        "/attributes/gen_ai.request.model",
        "/attributes/llm.model",
        "/model",
    ],
    ts: &["/timestamp", "/timeUnixNano", "/time", "/ts"],
    cost: &[],
};

jsonl_adapter! {
    GithubCopilotAdapter;
    tool_id = "github_copilot";
    display_name = "GitHub Copilot";
    vendor = "GitHub";
    level = SupportLevel::Standard;
    session = false;
    project = false;
    cache = false;
    cost = false;
    accuracy = UsageAccuracy::ProviderReported;
    dirs = {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        match home {
            Some(home) => vec![home.join(".copilot").join("otel")],
            None => Vec::new(),
        }
    };
    ext = "log";
    fields = FIELDS;
}
