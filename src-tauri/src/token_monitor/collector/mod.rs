//! 采集适配器契约与注册表（03 §2 落地）。
//!
//! **共享文件**：W3 各组只在此文件做锚点追加（`// ==== adapter registry ====` 后按字典序
//! 追加一行 `Box::new(...)`，顶部追加 `pub mod <tool>;`），不得改动他人条目。
//! 本文件由集成负责人统一合并。

pub mod checkpoint;
pub mod common;
pub mod watcher;
// W3 在此追加 `pub mod <tool>;`（字典序）
pub mod antigravity;
pub mod atomcode;
pub mod claude_code;
pub mod cline;
pub mod codebuddy;
pub mod codex;
pub mod cursor;
pub mod dsh;
pub mod freebuff;
pub mod custom_app;
pub mod github_copilot;
pub mod grok_build;
pub mod hermes;
pub mod kilo_code;
pub mod kimi;
pub mod kiro;
pub mod mimo;
pub mod openclaw;
pub mod opencode;
pub mod pi;
pub mod proma;
pub mod qwen;
pub mod repair;
pub mod tokscale;
pub mod workbuddy;
pub mod zcode;
pub mod zed;

#[cfg(test)]
mod real_data; // 真数据联调（#[ignore]，显式 cargo test -- --ignored real_data）

use crate::token_monitor::model::{
    AdapterCapabilities, CollectorCheckpoint, CollectorError, DataSource, NormalizedUsageEvent,
    SessionQuery, SessionSummary, ToolDescriptor,
};

/// 一次增量采集的产物。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CollectResult {
    pub events: Vec<NormalizedUsageEvent>,
    /// 可空
    pub sessions: Vec<SessionSummary>,
    pub next_checkpoint: CollectorCheckpoint,
}

/// 采集适配器契约。W3 每个工具实现一次，放独立文件。
pub trait ToolAdapter: Send + Sync {
    /// 静态描述（tool_id、能力等级、支持 OS、隐私说明）。
    fn descriptor(&self) -> ToolDescriptor;

    /// 声明该源能提供哪些维度（token/model/session/project/...）。
    fn capabilities(&self) -> AdapterCapabilities;

    /// 发现本机数据源（默认路径 + 用户自定义路径覆盖）。空 = 未安装/无数据。
    fn discover(&self) -> Vec<DataSource>;

    /// 返回某数据源当前 checkpoint（无则 Default）。
    fn checkpoint(&self, source_id: &str) -> CollectorCheckpoint;

    /// 从 checkpoint 增量解析新事件；必须更新并返回新 checkpoint。
    /// 不得读取/返回 Prompt/Response 正文。
    fn collect_incremental(
        &self,
        source: &DataSource,
        checkpoint: CollectorCheckpoint,
    ) -> Result<CollectResult, CollectorError>;

    /// 会话摘要（仅当 capabilities().session == true）。
    fn list_sessions(&self, query: &SessionQuery) -> Result<Vec<SessionSummary>, CollectorError> {
        let _ = query;
        Ok(vec![])
    }
}

/// 采集器注册表——共享文件，锚点追加（每个 Adapter 一行）。
pub struct AdapterRegistry;
impl AdapterRegistry {
    pub fn all() -> Vec<Box<dyn ToolAdapter>> {
        vec![
            // ==== adapter registry ====  (W3 在此锚点后按字典序追加)
            Box::new(antigravity::AntigravityAdapter::default()),
            Box::new(atomcode::AtomCodeAdapter::default()),
            Box::new(claude_code::ClaudeCodeAdapter::default()),
            Box::new(cline::ClineAdapter::default()),
            Box::new(codebuddy::CodebuddyAdapter::default()),
            Box::new(codex::CodexAdapter::default()),
            Box::new(cursor::CursorAdapter::default()),
            Box::new(dsh::DshAdapter::default()),
            Box::new(freebuff::FreebuffAdapter::default()),
            Box::new(github_copilot::GithubCopilotAdapter::default()),
            Box::new(grok_build::GrokBuildAdapter::default()),
            Box::new(hermes::HermesAdapter::default()),
            Box::new(kilo_code::KiloCodeAdapter::default()),
            Box::new(kimi::KimiAdapter::default()),
            Box::new(kiro::KiroAdapter::default()),
            Box::new(mimo::MimoAdapter::default()),
            Box::new(openclaw::OpenclawAdapter::default()),
            Box::new(opencode::OpenCodeAdapter::default()),
            Box::new(pi::PiAdapter::default()),
            Box::new(proma::PromaAdapter::default()),
            Box::new(qwen::QwenAdapter::default()),
            Box::new(tokscale::TokscaleAdapter::default()),
            Box::new(workbuddy::WorkbuddyAdapter::default()),
            Box::new(zcode::ZcodeAdapter::default()),
            Box::new(zed::ZedAdapter::default()),
        ]
    }
}
