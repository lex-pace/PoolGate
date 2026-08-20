import { invoke } from "@tauri-apps/api/core";

// ============ 枚举（与 Rust serde snake_case 对齐）============
export type Range = "day" | "7d" | "month" | "total";
export type UsageAccuracy = "exact" | "provider_reported" | "derived" | "unavailable";
export type SupportLevel = "full" | "standard" | "basic" | "quota_only" | "experimental";
export type CollectorStatus =
  | "idle" | "active" | "waiting" | "permission"
  | "path_missing" | "format_changed" | "partial" | "error";
export type QuotaWindowType =
  | "rolling_5h" | "weekly" | "monthly" | "billing" | "credits" | "prepaid_balance" | "requests";
export type QuotaUnit = "tokens" | "requests" | "credits" | "currency" | "percent";
export type QuotaSource = "official_api" | "local_auth" | "dashboard_session" | "custom_endpoint";
export type QuotaConfidence = "reported" | "derived" | "stale";

// ============ 行/视图类型 ============
export interface ToolUsageRow {
  tool_id: string; display_name: string; support_level: SupportLevel;
  collector_status: CollectorStatus;
  input_tokens: number; output_tokens: number; cache_tokens: number; total_tokens: number;
  cost_amount?: number; share_percent: number;   // 占比条
}
export interface ModelUsageRow {
  model: string; input_tokens: number; output_tokens: number; cache_tokens: number;
  total_tokens: number; cost_amount?: number; share_percent: number;
}
export interface ProjectToolShare {
  tool_id: string; display_name: string; total_tokens: number; share_percent: number;
}
export interface ProjectRow {
  project_id: string; display_name: string; total_tokens: number;
  session_count: number; last_active_at?: string; cost_amount?: number;
  tools: ProjectToolShare[];   // 项目内按工具拆分（开源项目视图：堆叠条 + 图例）
}
export interface DeviceRow { device_id: string; label: string; total_tokens: number; cost_amount?: number; }
export interface SessionSummary {
  session_id: string; tool_id: string; external_session_id?: string; project_id?: string;
  title_redacted?: string; model_set: string[]; started_at?: string; last_active_at?: string;
  input_tokens: number; output_tokens: number; cache_tokens: number; total_tokens: number;
  message_count: number; status?: string; cost_amount?: number;
}
export interface SessionEventRow {
  /** UTC ISO8601 */
  occurred_at: string;
  model?: string | null;
  input_tokens: number; output_tokens: number;
  /** cache_read + cache_write（对齐 unified 口径） */
  cache_tokens: number;
  reasoning_tokens: number;
  total_tokens: number;
  /** DB 真实成本优先；缺失时按模型价格估算；未知模型为 null */
  cost_amount?: number | null;
}
export interface ToolCollectorState {
  tool_id: string; display_name: string; enabled: boolean; support_level: SupportLevel;
  status: CollectorStatus; last_collected_at?: string; error?: string; paths: string[];
}
export interface QuotaWindowView {
  window_key: string; window_type: QuotaWindowType; unit: QuotaUnit; label: string;
  used_value?: number; limit_value?: number; remaining_value?: number; remaining_percent?: number;
  resets_at?: string; period_started_at?: string; source: QuotaSource; confidence: QuotaConfidence;
  fetched_at: string; error_code?: string;
}
export interface QuotaAccountView {
  account_id: string; provider_id: string;
  /** 可读供应商名（网关 providers 表解析；TM 额度账号为 null → 按 provider_id 映射）。 */
  provider_label?: string | null;
  label?: string; identity_masked?: string;
  plan_name?: string; status: string; enabled: boolean; last_success_at?: string;
  windows: QuotaWindowView[];
}
/** 按账号聚合的用量统计（额度卡 2×2 TOKEN 统计格数据源；与 Rust `AccountUsageStat` serde 对齐）。
 *  account_id 与 quota 视图同 key（`gw:{id}` 或 `tm_*`）；无网关流量的账号不返回。 */
export interface AccountUsageStat {
  account_id: string;
  today_tokens: number;
  yesterday_tokens: number;
  week_tokens: number;
  month_tokens: number;
  total_tokens: number;
  request_count: number;
}
export interface SeriesSplit { key: string; tokens: number; }
export interface TrendDay {
  date: string; tokens: number; requests: number; cost_amount?: number;
  /** 当日活跃时长（毫秒），趋势明细页按范围汇总「活跃时间」用。 */
  active_time_ms?: number;
  per_client?: SeriesSplit[]; per_model?: SeriesSplit[];
}
export interface TrendMonth { month: string; tokens: number; }
export interface TrendSeries {
  daily: TrendDay[];
  active_days: number; streak_days: number; peak_day?: TrendDay;
  monthly: TrendMonth[];
  active_time_ms?: number; message_count?: number;
}
export interface UsageTotals {
  input_tokens: number; output_tokens: number; cache_tokens: number; total_tokens: number;
  cost_amount?: number;
}
export interface CriticalQuota {
  account_id: string; provider_id: string; window_type: QuotaWindowType;
  remaining_percent?: number; resets_at?: string;
}
export interface TokenMonitorSnapshot {
  updated_at: string; range: Range; collector_state: CollectorStatus; active_tools: number;
  usage: UsageTotals;
  top_tools: ToolUsageRow[]; top_models: ModelUsageRow[]; recent_projects: ProjectRow[];
  critical_quota?: CriticalQuota;
}
export interface TokenMonitorTraySnapshot extends TokenMonitorSnapshot {
  heatmap: Array<{ date: string; tokens: number; requests: number }>;
  trend: TrendSeries; quota_accounts: QuotaAccountView[]; devices: DeviceRow[];
}
export interface TrayPrimaryMetric { kind: string; text: string; level?: "ok" | "warn" | "critical"; }
export interface AddQuotaAccountInput {
  provider_id: string; auth_method: string; label?: string;
  linked_route_account_id?: string; credential_payload?: string; // 明文仅传输一次，后端写 Keychain
}
export interface ServiceIssue { name: string; status: string; }
export interface ServiceStatusView {
  provider_id: string; label: string; page_url: string;
  status: "ok" | "degraded" | "outage" | "unknown";
  indicator: string; description: string;
  checked_at: string; updated_at: string;
  component_issues: ServiceIssue[];
  incident_title: string; incident_count: number; maintenance_count: number;
  error?: string;
}
export interface TokenRateView {
  /** 近 60 秒输出口径 → tokens/秒 */
  tokens_per_sec?: number;
  /** 近 60 分钟全量口径 → tokens/分 */
  tokens_per_min?: number;
  window_secs: number;
  observed_at?: string;
}
export interface AlertThresholds { remind_percent: number; warn_percent: number; critical_percent: number; }
/** 自定义应用字段映射（JSON Pointer 路径，如 "/usage/input_tokens"；缺省用内置通用默认字段）。 */
export interface CustomFields {
  input: string[]; output: string[]; cache: string[];
  model: string[]; ts: string[]; cost: string[];
}
/** 新增自定义应用（自定义应用监控）输入。 */
export interface AddCustomAppInput {
  display_name: string; paths: string[]; fields?: CustomFields;
}
export interface ProviderDescriptor {
  provider_id: string; display_name: string; supports_token_usage: boolean; windows_hint: string[];
}
/** 本机检测到的 Agent 工具（一键扫描添加，与 Rust `DetectedAgent` serde 对齐）。 */
export interface DetectedAgent {
  tool_id: string; display_name: string; vendor?: string | null;
  has_adapter: boolean; installed: boolean; data_found: boolean; monitored: boolean;
  covered_by_tokscale: boolean; tokscale_available: boolean;
  cli?: string | null; data_sources: string[];
}

// ============ 封装 ============
export const tmRange = (range: Range) => ({ range });
export function getTokenMonitorSnapshot(range: Range): Promise<TokenMonitorSnapshot> {
  return invoke("get_token_monitor_snapshot", { filters: { range } });
}
export function getTokenMonitorTraySnapshot(range: Range): Promise<TokenMonitorTraySnapshot> {
  return invoke("get_token_monitor_tray_snapshot", { range });
}
export function listToolUsage(filters: { range: Range } & Partial<Record<string, string>>): Promise<ToolUsageRow[]> {
  return invoke("list_tool_usage", { filters });
}
export function listModelUsage(filters: { range: Range }): Promise<ModelUsageRow[]> {
  return invoke("list_model_usage", { filters });
}
export function listActiveSessions(filters: { range: Range; toolId?: string }): Promise<SessionSummary[]> {
  return invoke("list_active_sessions", { filters });
}
/** 会话逐轮用量明细（W7 会话下钻，纯元数据，不读正文）。 */
export function listSessionEvents(sessionId: string): Promise<SessionEventRow[]> {
  return invoke("list_session_events", { sessionId });
}
export function listProjects(filters: { range: Range }): Promise<ProjectRow[]> {
  return invoke("list_projects", { filters });
}
export function getUsageTrend(filters: { range: Range }): Promise<TrendSeries> {
  return invoke("get_usage_trend", { filters });
}
export function listDevices(): Promise<DeviceRow[]> {
  return invoke("list_devices");
}
export function getCollectorStatus(): Promise<ToolCollectorState[]> { return invoke("get_collector_status"); }
export function getServiceStatus(): Promise<ServiceStatusView[]> { return invoke("get_service_status"); }
export function setToolCollection(toolId: string, enabled: boolean): Promise<void> {
  return invoke("set_tool_collection", { toolId, enabled });
}
export function setToolPaths(toolId: string, paths: string[]): Promise<void> {
  return invoke("set_tool_paths", { toolId, paths });
}
export function rescanTool(toolId: string): Promise<void> { return invoke("rescan_tool", { toolId }); }
/** 重置某工具数据：删除旧事件 + 重建 rollup + 重新采集（用于适配器逻辑变更后清理旧数据）。 */
export function resetToolData(toolId: string): Promise<void> { return invoke("reset_tool_data", { toolId }); }
export function scanAllTools(): Promise<ToolCollectorState[]> { return invoke("scan_all_tools"); }
export function getTokenRate(): Promise<TokenRateView> { return invoke("get_token_rate"); }
/** 手动立即刷新：清空 checkpoint 强制全量重扫（托盘刷新按钮）。 */
export function refreshTokenMonitor(): Promise<ToolCollectorState[]> { return invoke("refresh_token_monitor"); }
export function listQuotaAccounts(): Promise<QuotaAccountView[]> { return invoke("list_quota_accounts"); }
/** 按账号聚合的用量统计（今日/近7天/本月/累计 tokens；无流量账号不返回）。 */
export function getAccountUsageStats(): Promise<AccountUsageStat[]> { return invoke("get_account_token_stats"); }
export function listQuotaProviders(): Promise<ProviderDescriptor[]> { return invoke("list_quota_providers"); }
export function addQuotaAccount(input: AddQuotaAccountInput): Promise<QuotaAccountView> {
  return invoke("add_quota_account", { input });
}
export function refreshQuotaAccount(accountId: string): Promise<QuotaAccountView> {
  return invoke("refresh_quota_account", { accountId });
}
export function removeQuotaAccount(accountId: string): Promise<void> {
  return invoke("remove_quota_account", { accountId });
}
export function setQuotaAlertThresholds(accountId: string, thresholds: AlertThresholds): Promise<void> {
  return invoke("set_quota_alert_thresholds", { accountId, thresholds });
}
export function getTrayPrimaryMetric(): Promise<TrayPrimaryMetric> { return invoke("get_tray_primary_metric"); }
/** 通用本机 Agent 工具扫描：返回全部已知工具的安装/数据/监控状态（一键扫描添加）。 */
export function detectLocalAgents(): Promise<DetectedAgent[]> {
  return invoke("detect_local_agents");
}
/** 一键添加：为所选本机 Agent 工具注册适配器并立即开始采集（加入 TOKENS 监控）。 */
export function enableToolMonitoring(toolIds: string[]): Promise<ToolCollectorState[]> {
  return invoke("enable_tool_monitoring", { toolIds });
}
/** 新增自定义应用：应用名 + JSONL 日志路径（+ 可选字段映射）。返回新 tool_id（custom:<id>）。 */
export function addCustomApp(input: AddCustomAppInput): Promise<string> {
  return invoke("add_custom_app", { input });
}
/** 删除自定义应用（连同其用量/会话记录级联删除）。 */
export function removeCustomApp(toolId: string): Promise<void> {
  return invoke("remove_custom_app", { toolId });
}
