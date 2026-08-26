//! Token Monitor 共享数据类型（03-接口契约 §1/§5/§6 落地）。
//!
//! 本文件是**冻结契约**：改动任何字段/签名必须走 `00` §6 变更流程，由集成负责人统一操作。
//! 约定：结构体 `#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]`，
//! 字段 `snake_case`，与前端 TS 类型同名。

use std::path::PathBuf;

/// 采集能力声明——每个 Adapter 必须诚实填写；UI 据此决定展示哪些维度。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AdapterCapabilities {
    pub token: bool,
    pub model: bool,
    pub session: bool,
    pub project: bool,
    pub cache_tokens: bool,
    pub cost: bool,
    /// 该源能达到的最高精度
    pub accuracy: UsageAccuracy,
    /// 增量游标方式
    pub incremental: IncrementalMode,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageAccuracy {
    Exact,
    ProviderReported,
    Derived,
    Unavailable,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IncrementalMode {
    FileOffset,
    Mtime,
    RecordId,
    WalChange,
    ContentFingerprint,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    Full,
    Standard,
    Basic,
    QuotaOnly,
    Experimental,
}

/// 本机检测到的 Agent 工具（一键扫描添加，W12）。
/// 由 `detect_local_agents` 产出：报告安装/数据/适配器/监控状态，
/// 供前端勾选后经 `enable_tool_monitoring` 一键加入 TOKENS 监控。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DetectedAgent {
    pub tool_id: String,
    pub display_name: String,
    pub vendor: Option<String>,
    /// 注册表存在可用的采集适配器（可直接采集）。
    pub has_adapter: bool,
    /// 本机已安装（CLI 二进制 或 数据目录存在）。
    pub installed: bool,
    /// 发现用量数据源（可立即采集历史数据）。
    pub data_found: bool,
    /// 已加入 Token 监控（tool_definition.enabled=1）。
    pub monitored: bool,
    /// tokscale 聚合引擎覆盖该工具（tokscale 模式下由其统一采集）。
    pub covered_by_tokscale: bool,
    /// tokscale 聚合引擎本机可用（权威聚合模式开启）。
    pub tokscale_available: bool,
    /// 发现的 CLI 二进制路径（无则 None）。
    pub cli: Option<String>,
    /// 发现的数据源路径（隐私：仅展示目录级，最多 3 条）。
    pub data_sources: Vec<String>,
}

/// 采集状态——UI 状态点/文案据此渲染。
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CollectorStatus {
    #[default]
    Idle,
    Active,
    Waiting,
    Permission,
    PathMissing,
    FormatChanged,
    Partial,
    Error,
}

/// 工具静态描述（注册表项）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolDescriptor {
    pub tool_id: String,
    pub display_name: String,
    pub vendor: Option<String>,
    /// Usage | QuotaOnly | Gateway
    pub kind: ToolKind,
    pub support_level: SupportLevel,
    /// ["macos","windows","linux"]
    pub supported_os: Vec<String>,
    pub adapter_version: u32,
    /// 该工具会读取哪些路径、为何
    pub privacy_note: String,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Usage,
    QuotaOnly,
    Gateway,
}

/// 数据源（一个工具可有多个：不同目录/文件/DB）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DataSource {
    /// 稳定 id（用于 checkpoint key）
    pub id: String,
    pub path: PathBuf,
    pub format: DataFormat,
    /// true=可文件监听；false=只轮询
    pub watch: bool,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataFormat {
    Jsonl,
    Json,
    Sqlite,
    IdeGlobalStorage,
    Otel,
}

/// 增量游标（持久化到 checkpoint）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CollectorCheckpoint {
    pub source_id: String,
    pub byte_offset: Option<u64>,
    pub inode: Option<u64>,
    pub mtime_ms: Option<i64>,
    pub last_record_id: Option<String>,
    pub content_fingerprint: Option<String>,
}

/// 标准化用量事件——Adapter 产出的原始映射（total 交给 normalization 统一）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NormalizedUsageEvent {
    /// LocalDiscovered | Imported
    pub source_type: SourceType,
    pub tool_id: String,
    /// 默认 "local"
    pub device_id: String,
    pub model_raw: Option<String>,
    /// Adapter 可留空，normalization 填
    pub model_normalized: Option<String>,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub account_id: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    /// 该 (client, session, model) 组的消息数（tokscale 权威采集填充；
    /// 手写适配器/导入为 None）。会话投影据此求和，缺失时回退 COUNT(*)。
    pub message_count: Option<i64>,
    /// 会话开始时间（UTC ISO8601）：tokscale 权威采集从会话文件首条消息时间解析，
    /// 缺失回退会话 ID 内嵌时间戳；手写适配器/导入为 None。会话投影据此聚合。
    pub session_started_at: Option<String>,
    /// 会话最后活跃时间（UTC ISO8601）：tokscale 权威采集从会话文件末条消息时间解析，
    /// 缺失回退文件 mtime；手写适配器/导入为 None。会话投影据此聚合（排序/过滤精确）。
    pub session_last_active_at: Option<String>,
    /// Adapter 可留空，normalization 计算
    pub total_tokens: Option<i64>,
    pub cost_amount: Option<f64>,
    pub cost_currency: Option<String>,
    pub usage_accuracy: UsageAccuracy,
    /// UTC ISO8601
    pub occurred_at: String,
    pub source_locator_hash: Option<String>,
    // source_fingerprint 由 dedup::fingerprint(&event) 生成，Adapter 不填
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    GatewayObserved,
    LocalDiscovered,
    Imported,
}

/// 会话摘要（list_sessions 产出）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub tool_id: String,
    pub external_session_id: Option<String>,
    pub project_id: Option<String>,
    pub title_redacted: Option<String>,
    pub model_set: Vec<String>,
    pub started_at: Option<String>,
    pub last_active_at: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub total_tokens: i64,
    pub message_count: i64,
    pub status: Option<String>,
    /// 估算成本（USD）；未知模型价格时为 None（W6 追加，向后兼容）。
    #[serde(default)]
    pub cost_amount: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct SessionQuery {
    pub tool_id: Option<String>,
    pub project_id: Option<String>,
    pub limit: Option<i64>,
    pub since: Option<String>,
}

/// 会话单轮用量明细（list_session_events 产出，W7 追加）。
/// 由 usage_event 逐轮行聚合而来——只含元数据（时间/模型/token/成本），
/// 不含 Prompt/Response 正文，符合隐私红线（对齐开源 Token Monitor 的会话下钻，
/// 但开源版读 transcript 原文展示 prompt 预览，PoolGate 以「逐轮明细」等价替代）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionEventRow {
    /// UTC ISO8601
    pub occurred_at: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// cache_read + cache_write（对齐 unified 口径）
    pub cache_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_tokens: i64,
    /// DB 真实成本优先；缺失时按模型价格估算；未知模型为 None（不猜 0）
    #[serde(default)]
    pub cost_amount: Option<f64>,
}

/// 采集错误——解析失败隔离到单 Adapter，不 panic 全局。
#[derive(Debug, thiserror::Error)]
pub enum CollectorError {
    #[error("path not found: {0}")]
    PathMissing(String),
    #[error("permission denied: {0}")]
    Permission(String),
    #[error("format changed: {0}")]
    FormatChanged(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("parse error: {0}")]
    Parse(String),
}

// ============================================================
// 03 §5：命令过滤器
// ============================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct SnapshotFilters {
    /// day|month|total
    pub range: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageFilters {
    /// day|month|total
    pub range: String,
    pub tool_id: Option<String>,
    pub model: Option<String>,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub device_id: Option<String>,
}

// ============================================================
// 03 §5/§7：行/视图类型
// ============================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolUsageRow {
    pub tool_id: String,
    pub display_name: String,
    pub support_level: SupportLevel,
    pub collector_status: CollectorStatus,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub total_tokens: i64,
    pub cost_amount: Option<f64>,
    /// 占比条
    pub share_percent: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelUsageRow {
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub total_tokens: i64,
    pub cost_amount: Option<f64>,
    pub share_percent: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProjectRow {
    pub project_id: String,
    pub display_name: String,
    pub total_tokens: i64,
    pub session_count: i64,
    pub last_active_at: Option<String>,
    pub cost_amount: Option<f64>,
    /// 项目内按工具拆分（开源 Token Monitor 项目视图：堆叠条 + 图例）。
    pub tools: Vec<ProjectToolShare>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProjectToolShare {
    pub tool_id: String,
    pub display_name: String,
    pub total_tokens: i64,
    pub share_percent: f64,
}

/// 供应商服务状态（对齐开源 Token Monitor 状态页：Claude / OpenAI / Cursor / DeepSeek）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceIssue {
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceStatusView {
    pub provider_id: String,
    pub label: String,
    pub page_url: String,
    /// ok | degraded | outage | unknown
    pub status: String,
    /// 供应商原始 indicator：none | minor | major | critical | unknown
    pub indicator: String,
    pub description: String,
    pub checked_at: String,
    pub updated_at: String,
    pub component_issues: Vec<ServiceIssue>,
    pub incident_title: String,
    pub incident_count: i64,
    pub maintenance_count: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceRow {
    pub device_id: String,
    pub label: String,
    pub total_tokens: i64,
    pub cost_amount: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCollectorState {
    pub tool_id: String,
    pub display_name: String,
    /// 当前是否能在本机发现该应用（CLI 或数据目录）。
    pub installed: bool,
    pub enabled: bool,
    pub support_level: SupportLevel,
    pub status: CollectorStatus,
    pub last_collected_at: Option<String>,
    pub error: Option<String>,
    pub paths: Vec<String>,
}

/// 单日内的一个拆分项（工具或模型）及其 token 总量。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SeriesSplit {
    pub key: String,
    pub tokens: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrendDay {
    pub date: String,
    pub tokens: i64,
    pub requests: i64,
    pub cost_amount: Option<f64>,
    /// 当日活跃时长（毫秒）：趋势明细页按范围汇总「活跃时间」用。
    /// tokscale graph 逐日给出；DB 回退路径按会话活跃时长归到日。
    #[serde(default)]
    pub active_time_ms: i64,
    /// 按工具拆分（tm_daily_rollup 聚合；缺失时为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_client: Option<Vec<SeriesSplit>>,
    /// 按模型拆分（tm_daily_rollup 聚合；缺失时为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_model: Option<Vec<SeriesSplit>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrendMonth {
    pub month: String,
    pub tokens: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct TrendSeries {
    pub daily: Vec<TrendDay>,
    pub active_days: i64,
    pub streak_days: i64,
    pub peak_day: Option<TrendDay>,
    pub monthly: Vec<TrendMonth>,
    #[serde(default)]
    pub active_time_ms: i64,
    #[serde(default)]
    pub message_count: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct UsageTotals {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub total_tokens: i64,
    pub cost_amount: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CriticalQuota {
    pub account_id: String,
    pub provider_id: String,
    pub window_type: crate::token_monitor::quota::QuotaWindowType,
    pub remaining_percent: Option<f64>,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct TokenMonitorSnapshot {
    pub updated_at: String,
    pub range: String,
    pub collector_state: CollectorStatus,
    pub active_tools: i64,
    pub usage: UsageTotals,
    pub top_tools: Vec<ToolUsageRow>,
    pub top_models: Vec<ModelUsageRow>,
    pub recent_projects: Vec<ProjectRow>,
    pub critical_quota: Option<CriticalQuota>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrayHeatmapPoint {
    pub date: String,
    pub tokens: i64,
    pub requests: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct TokenMonitorTraySnapshot {
    pub updated_at: String,
    pub range: String,
    pub collector_state: CollectorStatus,
    pub active_tools: i64,
    pub usage: UsageTotals,
    pub top_tools: Vec<ToolUsageRow>,
    pub top_models: Vec<ModelUsageRow>,
    pub recent_projects: Vec<ProjectRow>,
    pub critical_quota: Option<CriticalQuota>,
    pub heatmap: Vec<TrayHeatmapPoint>,
    pub trend: TrendSeries,
    pub quota_accounts: Vec<QuotaAccountView>,
    pub devices: Vec<DeviceRow>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QuotaWindowView {
    pub window_key: String,
    pub window_type: crate::token_monitor::quota::QuotaWindowType,
    pub unit: crate::token_monitor::quota::QuotaUnit,
    pub label: String,
    pub used_value: Option<f64>,
    pub limit_value: Option<f64>,
    pub remaining_value: Option<f64>,
    pub remaining_percent: Option<f64>,
    pub resets_at: Option<String>,
    pub period_started_at: Option<String>,
    pub source: crate::token_monitor::quota::QuotaSource,
    pub confidence: crate::token_monitor::quota::QuotaConfidence,
    pub fetched_at: String,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct QuotaAccountView {
    pub account_id: String,
    pub provider_id: String,
    /// 可读供应商名（网关 providers 表解析；TM 额度账号为 None → 前端按 provider_id 映射）。
    pub provider_label: Option<String>,
    pub label: Option<String>,
    pub identity_masked: Option<String>,
    pub plan_name: Option<String>,
    pub status: String,
    pub enabled: bool,
    pub last_success_at: Option<String>,
    pub windows: Vec<QuotaWindowView>,
}

/// 按账号聚合的用量统计（额度卡 2×2 TOKEN 统计格数据源，W10）。
/// `account_id` 与 `list_quota_accounts` 视图同 key（`tm_*` 或 `gw:{id}`）。
/// 数据源 = request_logs（网关真实流量，最终请求去重口径，与网关仪表盘一致）；
/// 本地工具用量（usage_event）无账号归因，不参与。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct AccountUsageStat {
    pub account_id: String,
    /// 今日（本地日历）tokens
    pub today_tokens: i64,
    /// 昨日 tokens（日趋势/昨日胶囊）
    pub yesterday_tokens: i64,
    /// 近 7 天（含今日）tokens
    pub week_tokens: i64,
    /// 本月（含今日）tokens
    pub month_tokens: i64,
    /// 累计 tokens
    pub total_tokens: i64,
    /// 最终请求数（口径参考）
    pub request_count: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct TrayPrimaryMetric {
    pub kind: String,
    pub text: String,
    pub level: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct AddQuotaAccountInput {
    pub provider_id: String,
    pub auth_method: String,
    pub label: Option<String>,
    pub linked_route_account_id: Option<String>,
    /// 明文仅传输一次，后端写 Keychain
    pub credential_payload: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct AlertThresholds {
    pub remind_percent: f64,
    pub warn_percent: f64,
    pub critical_percent: f64,
}

/// 新增自定义应用（自定义应用监控）输入。
/// `fields` 为可选 JSONL 字段映射，缺省用内置通用默认字段。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct AddCustomAppInput {
    pub display_name: String,
    pub paths: Vec<String>,
    /// 可选字段映射（{input:[...],output:[...],cache:[...],model:[...],ts:[...],cost:[...]}）
    pub fields: Option<crate::token_monitor::collector::custom_app::CustomFields>,
}

/// 实时 Token 速率（托盘 Logo 点击展示，对齐开源 Token Monitor 的
/// speed（tokens/秒，输出口径）与 burn（tokens/分，全量口径）两种读数）。
/// 源数据缺失时返回 `None`（诚实标注「不可用」，不猜不填 0）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct TokenRateView {
    /// 近 60 秒窗口内 output 口径 → tokens/秒。
    pub tokens_per_sec: Option<f64>,
    /// 近 60 分钟窗口内 total 口径 → tokens/分。
    pub tokens_per_min: Option<f64>,
    /// 实际观测窗口秒数（计算用）。
    pub window_secs: i64,
    /// 窗口内最近一条事件时间（UTC ISO8601）。
    pub observed_at: Option<String>,
}

// ============================================================
// 03 §6：Tauri 事件 payload
// ============================================================

#[derive(Debug, Clone, serde::Serialize)]
pub struct UsageDelta {
    pub sequence: u64,
    pub emitted_at: String,
    pub range: String,
    pub total_tokens: i64,
    pub cost_amount: Option<f64>,
    pub active_tools: u32,
    /// 轻量，供托盘增量叠加
    pub top_tools: Vec<ToolUsageRow>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TmAlert {
    /// quota_low | quota_stale | auth_expired | collector_error
    pub kind: String,
    /// remind | warn | critical
    pub level: String,
    pub account_id: Option<String>,
    pub tool_id: Option<String>,
    pub message: String,
    pub emitted_at: String,
}
