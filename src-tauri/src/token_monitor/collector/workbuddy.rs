//! WorkBuddy（JSON/JSONL + SQLite / offset+rowid）— W3c。
//! 数据源：`~/.workbuddy/projects/`、`~/.workbuddy/workbuddy.db`（本机存在 ~/.workbuddy11111/ 目录，
//! 可作为首个真实联调对象）。JSONL 按 offset 增量；SQLite 源走行数增量。

use std::path::PathBuf;

use crate::token_monitor::collector::common::{
    collect_files, collect_jsonl_incremental, hash_short, jsonl_adapter, GenericFields,
};
use crate::token_monitor::model::{SupportLevel, UsageAccuracy};

// 实测真实格式（2026-08-08 本机 ~/.workbuddy/projects/**/<session>.jsonl）：
// 每行带 providerData.usage（camelCase，OpenAI 风格）：
//   providerData.usage = {requests, inputTokens, outputTokens, totalTokens,
//                         inputTokensDetails:[{cached_tokens}], outputTokensDetails:[{reasoning_tokens}]}
//   providerData.model / providerData.requestModelId
//   timestamp = epoch 毫秒；行类型 function_call / message 才带 usage。
const FIELDS: GenericFields = GenericFields {
    input: &[
        "/providerData/usage/inputTokens",
        "/providerData/rawUsage/prompt_tokens",
        "/usage/inputTokens",
        "/inputTokens",
    ],
    output: &[
        "/providerData/usage/outputTokens",
        "/providerData/rawUsage/completion_tokens",
        "/usage/outputTokens",
        "/outputTokens",
    ],
    cache: &[
        "/providerData/usage/inputTokensDetails/0/cached_tokens",
        "/providerData/rawUsage/prompt_tokens_details/cached_tokens",
        "/providerData/usage/cacheReadTokens",
    ],
    model: &[
        "/providerData/model",
        "/providerData/requestModelId",
        "/model",
    ],
    ts: &["/timestamp", "/ts", "/created_at"],
    cost: &[],
};

jsonl_adapter! {
    WorkbuddyAdapter;
    tool_id = "workbuddy";
    display_name = "WorkBuddy";
    vendor = "WorkBuddy";
    level = SupportLevel::Standard;
    // 每个 JSONL 文件 = 一个会话（文件 = session，external id = 文件名），
    // 与 Claude/Codex 同构；会话摘要只存元数据（时间/token/模型），不读正文。
    session = true;
    project = true;
    cache = true;
    cost = false;
    accuracy = UsageAccuracy::Exact;
    dirs = {
        let mut roots = Vec::new();
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            roots.push(home.join(".workbuddy").join("projects"));
            roots.push(home.join(".workbuddy11111").join("projects"));
        }
        roots
    };
    ext = "jsonl";
    fields = FIELDS;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_monitor::collector::ToolAdapter;
    use crate::token_monitor::model::{CollectorCheckpoint, DataFormat, DataSource};

    #[test]
    fn workbuddy_parses_provider_data_usage() {
        let adapter = WorkbuddyAdapter;
        let src = DataSource {
            id: "workbuddy:test".into(),
            path: PathBuf::from("tests/fixtures/workbuddy/session_usage.jsonl"),
            format: DataFormat::Jsonl,
            watch: false,
        };
        let r1 = adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .expect("collect");
        assert_eq!(r1.events.len(), 2, "应解析出 2 条 usage");
        assert_eq!(r1.events[0].input_tokens, Some(35951));
        assert_eq!(r1.events[0].output_tokens, Some(800));
        assert_eq!(r1.events[0].cache_read_tokens, Some(13824));
        assert_eq!(r1.events[0].model_raw.as_deref(), Some("gpt-5.6-sol"));
        assert!(
            r1.events[0].occurred_at.starts_with("2026"),
            "毫秒时间戳应归一化"
        );
        // 幂等：同 checkpoint 无新增
        let r2 = adapter
            .collect_incremental(&src, r1.next_checkpoint.clone())
            .expect("recollect");
        assert!(r2.events.is_empty());
    }

    #[test]
    fn workbuddy_emits_session_summary_per_file() {
        // W7 回归：WorkBuddy 会话必须出现在会话列表（tm_session），此前 session=false 导致
        // 「今日有 WorkBuddy 用量但会话视图显示暂无会话」。
        let adapter = WorkbuddyAdapter;
        let src = DataSource {
            id: "workbuddy:test".into(),
            path: PathBuf::from("tests/fixtures/workbuddy/session_usage.jsonl"),
            format: DataFormat::Jsonl,
            watch: false,
        };
        let r1 = adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .expect("collect");
        assert_eq!(r1.sessions.len(), 1, "每个文件应产出 1 个会话摘要");
        let s = &r1.sessions[0];
        assert_eq!(s.tool_id, "workbuddy");
        assert_eq!(s.external_session_id.as_deref(), Some("session_usage"));
        assert!(s.total_tokens > 0);
        assert_eq!(s.message_count, 2, "两条 usage 行 → 2 轮");
        // 会话摘要不读正文：title 为 项目名+时间 生成
        assert!(s.title_redacted.as_deref().unwrap_or("").contains("·"));
    }

    #[test]
    fn workbuddy_rebuilds_session_when_file_consumed_but_no_new_lines() {
        // 迁移场景：session=false 时文件已消费到 EOF（checkpoint 无 mtime），升级后
        // 无新行也应重建会话摘要（mtime 守卫：空闲轮询不再重复重建）。
        let adapter = WorkbuddyAdapter;
        let src = DataSource {
            id: "workbuddy:test".into(),
            path: PathBuf::from("tests/fixtures/workbuddy/session_usage.jsonl"),
            format: DataFormat::Jsonl,
            watch: false,
        };
        let r1 = adapter
            .collect_incremental(&src, CollectorCheckpoint::default())
            .expect("collect");
        // 用「已消费到 EOF」的 checkpoint 再采：无新行，但应重建出会话（首轮 mtime 未知）
        let r2 = adapter
            .collect_incremental(&src, r1.next_checkpoint.clone())
            .expect("recollect");
        assert!(r2.events.is_empty(), "无新行不重发事件");
        assert_eq!(r2.sessions.len(), 1, "无新行也应重建会话摘要（迁移补数）");
        assert_eq!(
            r2.sessions[0].external_session_id.as_deref(),
            Some("session_usage")
        );
        // 再采：mtime 未变 → 跳过重建（防事件风暴）
        let r3 = adapter
            .collect_incremental(&src, r2.next_checkpoint.clone())
            .expect("recollect again");
        assert!(r3.sessions.is_empty(), "mtime 未变不重复重建");
    }
}
