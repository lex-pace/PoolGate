import { invoke } from "@tauri-apps/api/core";

// ============ Types ============

export interface Provider {
  id: string;
  name: string;
  type: string;
  base_url: string;
  /** JSON object mapping protocol names to upstream Base URLs. */
  base_urls?: string;
  protocol: string;
  protocols?: string;
  api_keys?: string;
  models?: string;
  proxy_url?: string;
  /** JSON object of additional upstream HTTP headers. */
  custom_headers?: string;
  timeout_ms?: number;
  priority?: number;
  enabled?: boolean;
  created_at?: string;
}

export interface ProviderTestResult {
  success: boolean;
  message: string;
  latency_ms: number;
  model_tested?: string;
  /** 模型回复（成功时）或错误详情（失败时） */
  error_details?: string;
}

export interface Account {
  id: string;
  provider_id?: string;
  name?: string;
  api_key?: string;
  models?: string;
  quota_limit?: number;
  quota_used?: number;
  status?: string;
  health_status?: string;
  health_code?: number;
  health_msg?: string;
  health_latency?: number;
  health_check_at?: string;
  priority?: number;
  tags?: string;
  last_used_at?: string;
  created_at?: string;
  credential_type?: "api_key" | "upstream_key" | "oauth" | "token" | "codex_oauth";
  source_format?: string;
  external_account_id?: string;
  email?: string;
  expires_at?: string;
  /** JSON-encoded protocol array persisted by the Rust account model. */
  protocols?: string;
  route_takeover?: number;
  plan_type?: string;
  quota_windows?: string;
  quota_refreshed_at?: string;
  quota_error?: string;
  token_refreshed_at?: string;
}

export interface ModelRefreshResult {
  account_id: string;
  success: boolean;
  message: string;
  models: string[];
}

export interface BatchDeleteAccountsResult {
  deleted_ids: string[];
  failures: Array<{
    account_id: string;
    message: string;
  }>;
}

export interface AgentGroup {
  id: string;
  name: string;
  description?: string;
  protocol: string;
  strategy?: string;
  api_key?: string;
  enabled?: boolean;
  created_at?: string;
}

export interface GroupModelResource {
  provider_id: string;
  model: string;
}

export interface AvailableGroupModelAccount {
  id: string;
  name: string;
  email?: string;
  status: string;
  health_status: string;
  routable: boolean;
  selected: boolean;
}

export interface AvailableGroupModelResource extends GroupModelResource {
  provider_name: string;
  account_count: number;
  healthy_count: number;
  already_added: boolean;
  accounts: AvailableGroupModelAccount[];
}

export interface RoutePoolCreated {
  group: AgentGroup;
  key: ClientKeyView;
  raw_key: string;
}

export interface TrafficStats {
  total_requests: number;
  success_count: number;
  error_count: number;
  success_rate: number;
  input_tokens: number;
  output_tokens: number;
  cache_tokens: number;
  total_tokens: number;
  total_cost: number;
  avg_latency_ms: number;
}

export interface ProviderTrafficStats {
  provider_id: string;
  total_requests: number;
  success_count: number;
  error_count: number;
  success_rate: number;
  input_tokens: number;
  output_tokens: number;
  cache_tokens: number;
  total_tokens: number;
  total_cost: number;
  avg_latency_ms?: number;
  p95_latency_ms?: number;
  avg_ttft_ms?: number;
  last_active_at?: string;
}

export interface AccountTrafficStats {
  account_id: string;
  provider_id: string;
  total_requests: number;
  success_count: number;
  error_count: number;
  success_rate: number;
  input_tokens: number;
  output_tokens: number;
  cache_tokens: number;
  total_tokens: number;
  total_cost: number;
  avg_latency_ms?: number;
  p95_latency_ms?: number;
  avg_ttft_ms?: number;
  last_active_at?: string;
}

export interface GroupDashboard {
  resource_count: number;
  healthy_resource_count: number;
  model_count: number;
  provider_count: number;
  traffic: TrafficStats;
  quota_by_provider: Array<{
    provider_id: string;
    provider_name: string;
    account_count: number;
    average_used_percent: number;
    max_used_percent: number;
    min_remaining_percent: number;
    abnormal_accounts: number;
  }>;
}

export type TopologyNodeStatus = "healthy" | "warning" | "failed" | "fault" | "disabled" | "offline";

export interface RuntimeRoutePath {
  request_id: string;
  protocol: string;
  pool_id: string;
  provider_id: string;
  account_id: string;
  status: "active" | "success" | "failed";
  attempt: number;
  latency_ms?: number;
  started_at: string;
  updated_at: string;
}

export interface RuntimeActivePath {
  protocol: string;
  pool_id: string;
  provider_id: string;
  account_id: string;
  active_requests: number;
  last_active_at: string;
}

export interface TopologyRuntimeDelta {
  sequence: number;
  emitted_at: string;
  active_connections: number;
  active_paths: RuntimeActivePath[];
  latest_route?: RuntimeRoutePath;
  node_deltas: Array<{ id: string; active_concurrency: number }>;
  edge_deltas: Array<{ id: string; active_requests: number }>;
  completed: Array<{ request_id: string; status: "success" | "failed"; latency_ms: number; path_edge_ids: string[] }>;
}

export interface RouteTopology {
  version: 2;
  topology_revision: number;
  gateway: {
    id: string;
    name: string;
    address: string;
    running: boolean;
    active_connections: number;
  };
  protocols: Array<{
    id: string;
    name: string;
    protocol: string;
    enabled: boolean;
    pool_ids: string[];
    request_count: number;
    traffic: TrafficStats;
  }>;
  pools: Array<{
    id: string;
    name: string;
    protocol: string;
    strategy?: string;
    enabled: boolean;
    provider_ids: string[];
    resource_count: number;
    healthy_resource_count: number;
    model_count: number;
    traffic: TrafficStats;
  }>;
  providers: Array<{
    id: string;
    name: string;
    protocol: string;
    enabled: boolean;
    account_count: number;
    healthy_account_count: number;
    traffic: ProviderTrafficStats;
    host?: string;
  }>;
  accounts: Array<{
    id: string;
    provider_id: string;
    name: string;
    email_masked?: string;
    status: string;
    health_status: string;
    routable: boolean;
    plan_type?: string;
  }>;
  edges: Array<{
    id: string;
    source: string;
    target: string;
    status: TopologyNodeStatus;
    active: boolean;
    active_requests: number;
  }>;
  active_route?: RuntimeRoutePath;
  runtime: TopologyRuntimeDelta;
  updated_at: string;
}

export interface ProviderTopologyDetail {
  id: string;
  name: string;
  provider_type: string;
  protocol: string;
  protocols: string[];
  base_url_masked: string;
  enabled: boolean;
  models: string[];
  traffic: ProviderTrafficStats;
  accounts: Array<{
    id: string;
    name: string;
    email_masked?: string;
    credential_type: string;
    source_format?: string;
    routable: boolean;
    status: string;
    health_status: string;
    health_latency_ms?: number;
    last_used_at?: string;
    plan_type?: string;
    models: string[];
    quota_remaining_percent?: number;
    concurrency_limit: number;
    concurrency_active: number;
    concurrency_available: number;
    queued_requests: number;
    expires_at?: string;
    last_error?: string;
    traffic: AccountTrafficStats;
  }>;
}

export interface AgentAppInfo {
  app_id: string;
  name: string;
  installed: boolean;
  executable?: string;
  config_paths: string[];
}

export interface AgentAppPreview {
  app: AgentAppInfo;
  group_id: string;
  affected_paths: string[];
  backup_root: string;
  warnings: string[];
}

export interface AgentAppLaunchResult {
  app_id: string;
  group_id: string;
  snapshot_id: string;
  config_path: string;
  backup_path: string;
  launched: boolean;
}

export interface RequestLog {
  id?: number;
  group_id?: string;
  client_key_id?: string;
  request_id?: string;
  attempt_count?: number;
  usage_available?: boolean;
  source?: string;
  provider_id?: string;
  account_id?: string;
  provider_name?: string;
  account_name?: string;
  group_name?: string;
  model?: string;
  endpoint?: string;
  status?: string;
  status_code?: number;
  input_tokens?: number;
  output_tokens?: number;
  cache_tokens?: number;
  cost?: number;
  latency_ms?: number;
  ttft_ms?: number;
  is_stream?: boolean;
  error_message?: string;
  request_at?: string;
}

export interface LogStats {
  total_requests: number;
  success_count: number;
  error_count: number;
  rate_limited_count: number;
  timeout_count: number;
  total_tokens: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_cost: number;
  avg_latency_ms: number;
}

export type LogRange = "today" | "6h" | "24h" | "7d" | "30d" | "90d" | "all";
export type AnalyticsGranularity = "day" | "week" | "month";

export interface LogQuery {
  page?: number;
  page_size?: number;
  group_id?: string;
  status?: string;
  source?: string;
  range?: LogRange;
  start_time?: string;
  end_time?: string;
  keyword?: string;
}

export interface AnalyticsData {
  summary: LogStats;
  daily: Array<{
    date: string;
    total_requests: number;
    success_count: number;
    error_count: number;
    input_tokens: number;
    output_tokens: number;
    cache_tokens: number;
    total_cost: number;
    avg_latency_ms: number;
  }>;
  model_distribution: Array<{
    model: string;
    count: number;
    input_tokens: number;
    output_tokens: number;
    total_cost: number;
  }>;
  account_ranking: Array<{
    account_id: string;
    name: string;
    count: number;
    success_count: number;
    total_tokens: number;
    total_cost: number;
  }>;
  /** Daily token/request series for the last 13 weeks (activity heatmap). */
  heatmap: Array<{
    date: string;
    tokens: number;
    requests: number;
  }>;
}

export interface AppLogPage {
  lines: string[];
  total: number;
  page: number;
  page_size: number;
  file_path: string;
  file_size: number;
}

export interface AppLogInfo {
  log_dir: string;
  file_path: string | null;
  file_size: number;
  line_count: number;
}

export interface TraySnapshot {
  gateway_running: boolean;
  port: number;
  active_connections: number;
  topology: {
    protocol_count: number;
    pool_count: number;
    provider_count: number;
    warning_count: number;
    fault_count: number;
  };
  resources_total: number;
  resources_available: number;
  resources_limited: number;
  enabled_pool_count: number;
  today_tokens: number;
  seven_day_tokens: number;
  month_tokens: number;
  cumulative_tokens: number;
  total_requests: number;
  success_rate: number;
  current_tps: number;
  activity: Array<{
    hour: number;
    requests: number;
    tokens: number;
    avg_latency_ms: number;
  }>;
  /** Daily token/request series for the last 13 weeks (tray heatmap view). */
  tokens_heatmap: Array<{
    date: string;
    tokens: number;
    requests: number;
  }>;
  pools: Array<{
    id: string;
    name: string;
    protocol: string;
    strategy: string;
    enabled: boolean;
    resource_count: number;
    healthy_resource_count: number;
    model_count: number;
    requests: number;
    tokens: number;
    providers: Array<{
      id: string;
      name: string;
      protocol: string;
      accounts: Array<{
        id: string;
        name: string;
        email?: string;
        status: string;
        health_status: string;
      }>;
    }>;
  }>;
  active_route?: {
    protocol: string;
    pool_id: string;
    pool_name: string;
    provider_id: string;
    provider_name: string;
    status: "active" | "success" | "failed";
    attempt: number;
    latency_ms?: number;
    active_requests: number;
    updated_at: string;
  };
  updated_at: string;
}

export interface HealthResult {
  account_id: string;
  status: string;
  latency_ms: number;
  code: number;
  message: string;
}

export interface OAuthStartResult {
  login_id: string;
  authorization_url: string;
  redirect_uri: string;
  expires_in_seconds: number;
}

export interface QuotaWindow {
  key: string;
  label: string;
  used_percent: number;
  remaining_percent: number;
  window_seconds?: number;
  reset_at?: number;
  reset_after_seconds?: number;
}

export interface RefreshResult {
  account_id: string;
  kind: "token" | "quota";
  success: boolean;
  message: string;
  plan_type?: string;
  quota_windows: QuotaWindow[];
}

export interface ImportSourceRequest {
  content?: string;
  source_name?: string;
  paths?: string[];
  provider_hint?: {
    id: string;
    name: string;
    protocol: string;
    base_url: string;
    /** Multi-protocol set for this upstream (e.g. ["chat","anthropic"]). */
    protocols?: string[];
    /** Protocol-specific upstream Base URLs. */
    base_urls?: Partial<Record<"responses" | "chat" | "anthropic" | "gemini", string>>;
    models: string[];
    tags: string[];
    credential_mode?: "apikey" | "token" | "batch";
  };
}

export interface ImportOptions {
  auto_create_providers: boolean;
  skip_duplicates: boolean;
  import_adapter_required: boolean;
  selected_fingerprints?: string[];
  /** Optional model subset keyed by full account fingerprint. */
  selected_models?: Record<string, string[]>;
  default_tags?: string[];
  /**
   * Conflict resolution when an account with the same credential fingerprint
   * already exists: "skip" (default), "overwrite", or "merge".
   */
  on_conflict?: "skip" | "overwrite" | "merge";
}

export interface ImportPreviewAccount {
  index: number;
  name: string;
  email?: string;
  provider_name: string;
  protocol: string;
  credential_type: string;
  source_format: string;
  masked_credential: string;
  external_account_id?: string;
  expires_at?: string;
  models: string[];
  tags: string[];
  fingerprint: string;
  action: "create" | "duplicate" | "adapter_required";
  routable: boolean;
  adapter: string;
  warning?: string;
}

export interface ImportPreview {
  format: string;
  source_count: number;
  accounts: ImportPreviewAccount[];
  auto_created_providers: Array<{ name: string; protocol: string; base_url: string }>;
  warnings: string[];
  skipped: string[];
  summary: {
    total: number;
    ready: number;
    duplicates: number;
    adapter_required: number;
    skipped: number;
  };
}

export interface ImportResult {
  imported: number;
  /** Accounts updated in place via on_conflict "overwrite" / "merge". */
  updated: number;
  skipped: number;
  duplicates: number;
  adapter_required: number;
  created_providers: number;
  account_ids: string[];
  errors: string[];
}

export interface DiscoveredAccount {
  name: string;
  email?: string;
  provider_name: string;
  credential_type: string;
  masked_credential: string;
  models: string[];
  /** Full credential fingerprint used for selective sync. */
  fingerprint: string;
  routable: boolean;
  warning?: string;
  source_apps: string[];
  /** Internal paths used only to reparse selected credentials on import. */
  source_paths: string[];
}

export interface DiscoveredModelResource {
  key: string;
  provider_name: string;
  model: string;
  source_apps: string[];
  account_fingerprints: string[];
}

export interface AgentConfigScanResult {
  accounts: DiscoveredAccount[];
  model_resources: DiscoveredModelResource[];
  /** Internal scan inputs; never rendered in the user-facing results. */
  source_paths: string[];
  warnings: string[];
}

export interface ImportCheckSummary {
  total: number;
  ready: number;
  abnormal: number;
  duplicates: number;
  adapter_required: number;
  skipped: number;
}

export interface CheckedAccount {
  index: number;
  name: string;
  email?: string;
  provider_name: string;
  protocol: string;
  credential_type: string;
  source_format: string;
  masked_credential: string;
  external_account_id?: string;
  expires_at?: string;
  models: string[];
  tags: string[];
  fingerprint: string;
  short_fingerprint: string;
  action: "create" | "duplicate" | "adapter_required";
  routable: boolean;
  adapter: string;
  warning?: string;
  health_status: string;
  health_code: number;
  health_message: string;
  health_latency_ms: number;
}

export interface ImportCheckResult {
  format: string;
  source_count: number;
  accounts: CheckedAccount[];
  auto_created_providers: Array<{ name: string; protocol: string; base_url: string }>;
  warnings: string[];
  skipped: string[];
  summary: ImportCheckSummary;
}

// ============ Provider Commands ============

export function listProviders(): Promise<Provider[]> {
  return invoke("list_providers");
}

export function createProvider(provider: Provider): Promise<Provider> {
  return invoke("create_provider", { provider });
}

export function updateProvider(provider: Provider): Promise<void> {
  return invoke("update_provider", { provider });
}

export function deleteProvider(id: string): Promise<void> {
  return invoke("delete_provider", { id });
}

export function testProviderConnection(providerId: string): Promise<ProviderTestResult> {
  return invoke("test_provider_connection", { providerId });
}

export function testAccountConnection(accountId: string): Promise<ProviderTestResult> {
  return invoke("test_account_connection", { accountId });
}

// ============ Account Commands ============

export function listAccounts(): Promise<Account[]> {
  return invoke("list_accounts");
}

export function createAccount(account: Account): Promise<Account> {
  return invoke("create_account", { account });
}

export function updateAccount(account: Account): Promise<void> {
  return invoke("update_account", { account });
}

export function deleteAccount(id: string): Promise<void> {
  return invoke("delete_account", { id });
}

export function batchDeleteAccounts(ids: string[]): Promise<BatchDeleteAccountsResult> {
  return invoke("batch_delete_accounts", { ids });
}

export function batchUpdateAccounts(ids: string[], status: string): Promise<void> {
  return invoke("batch_update_accounts", { ids, status });
}

export function getAccountRequestCounts(days?: number): Promise<Record<string, number>> {
  return invoke("get_account_request_counts", { days });
}

// ============ Health Commands ============

export function checkAccountHealth(accountId: string): Promise<HealthResult> {
  return invoke("check_account_health", { accountId });
}

export function batchCheckHealth(accountIds?: string[]): Promise<HealthResult[]> {
  return invoke("batch_check_health", { accountIds });
}

export function startOAuthLogin(
  providerId: string,
  emailHint?: string,
  note?: string,
): Promise<OAuthStartResult> {
  return invoke("start_oauth_login", { providerId, emailHint, note });
}

export function completeOAuthLogin(loginId: string, callbackUrl?: string): Promise<Account> {
  return invoke("complete_oauth_login", { loginId, callbackUrl });
}

export function cancelOAuthLogin(loginId: string): Promise<void> {
  return invoke("cancel_oauth_login", { loginId });
}

export interface CopilotValidation {
  valid: boolean;
  copilot_token_preview: string;
  expires_at: string;
  message: string;
}

export function copilotPatValidate(githubPat: string): Promise<CopilotValidation> {
  return invoke("copilot_pat_validate", { githubPat });
}

export interface GeminiValidation {
  valid: boolean;
  message: string;
}

export function geminiApiKeyValidate(apiKey: string): Promise<GeminiValidation> {
  return invoke("gemini_api_key_validate", { apiKey });
}

export interface ClaudeOAuthStartResult {
  login_id: string;
  authorization_url: string;
  redirect_uri: string;
  expires_in_seconds: number;
}

export function startClaudeOAuth(planType: string): Promise<ClaudeOAuthStartResult> {
  return invoke("start_claude_oauth", { planType });
}

export function completeClaudeOAuth(loginId: string, callbackUrl?: string): Promise<Account> {
  return invoke("complete_claude_oauth", { loginId, callbackUrl });
}

export function cancelClaudeOAuth(loginId: string): Promise<void> {
  return invoke("cancel_claude_oauth", { loginId });
}

export interface CopilotDeviceCode {
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export function startCopilotDeviceFlow(): Promise<CopilotDeviceCode> {
  return invoke("start_copilot_device_flow");
}

export interface CopilotDevicePollResult {
  authorized: boolean;
  access_token?: string;
  message: string;
}

export function pollCopilotDeviceToken(
  deviceCode: string,
  intervalMs: number,
): Promise<CopilotDevicePollResult> {
  return invoke("poll_copilot_device_token", { deviceCode, intervalMs });
}

export function completeCopilotDeviceFlow(githubAccessToken: string): Promise<Account> {
  return invoke("complete_copilot_device_flow", { githubAccessToken });
}

export interface GeminiOAuthStartResult {
  login_id: string;
  authorization_url: string;
  redirect_uri: string;
  expires_in_seconds: number;
}

export function startGeminiOAuth(): Promise<GeminiOAuthStartResult> {
  return invoke("start_gemini_oauth");
}

export function completeGeminiOAuth(loginId: string, callbackUrl?: string): Promise<Account> {
  return invoke("complete_gemini_oauth", { loginId, callbackUrl });
}

export function cancelGeminiOAuth(loginId: string): Promise<void> {
  return invoke("cancel_gemini_oauth", { loginId });
}

export interface AntigravityOAuthStartResult {
  login_id: string;
  authorization_url: string;
  redirect_uri: string;
  expires_in_seconds: number;
}

export function startAntigravityOAuth(): Promise<AntigravityOAuthStartResult> {
  return invoke("start_antigravity_oauth");
}

export function completeAntigravityOAuth(
  loginId: string,
  callbackUrl?: string,
): Promise<Account> {
  return invoke("complete_antigravity_oauth", { loginId, callbackUrl });
}

export function cancelAntigravityOAuth(loginId: string): Promise<void> {
  return invoke("cancel_antigravity_oauth", { loginId });
}

export interface GrokOAuthStartResult {
  login_id: string;
  authorization_url: string;
  redirect_uri: string;
  expires_in_seconds: number;
}

export function startGrokOAuth(): Promise<GrokOAuthStartResult> {
  return invoke("start_grok_oauth");
}

export function completeGrokOAuth(loginId: string, callbackUrl?: string): Promise<Account> {
  return invoke("complete_grok_oauth", { loginId, callbackUrl });
}

export function cancelGrokOAuth(loginId: string): Promise<void> {
  return invoke("cancel_grok_oauth", { loginId });
}

export function refreshAccountToken(accountId: string): Promise<RefreshResult> {
  return invoke("refresh_account_token", { accountId });
}

export function batchRefreshTokens(accountIds: string[]): Promise<RefreshResult[]> {
  return invoke("batch_refresh_tokens", { accountIds });
}

export function refreshAccountQuota(accountId: string): Promise<RefreshResult> {
  return invoke("refresh_account_quota", { accountId });
}

export function batchRefreshQuotas(accountIds: string[]): Promise<RefreshResult[]> {
  return invoke("batch_refresh_quotas", { accountIds });
}

export function cleanupExpired(): Promise<{ checked: number; disabled: string[] }> {
  return invoke("cleanup_expired");
}

// ============ Group Commands ============

export function listGroups(): Promise<AgentGroup[]> {
  return invoke("list_groups");
}

export function createGroup(group: AgentGroup): Promise<RoutePoolCreated> {
  return invoke("create_group", { group });
}

export function ensureGroupClientKey(groupId: string): Promise<ClientKeyCreated> {
  return invoke("ensure_group_client_key", { groupId });
}

export function rotateGroupClientKey(groupId: string): Promise<ClientKeyCreated> {
  return invoke("rotate_group_client_key", { groupId });
}

export function updateGroup(group: AgentGroup): Promise<void> {
  return invoke("update_group", { group });
}

export function deleteGroup(id: string): Promise<void> {
  return invoke("delete_group", { id });
}

export function getGroupAccounts(groupId: string): Promise<string[]> {
  return invoke("get_group_accounts", { groupId });
}

export function addAccountToGroup(groupId: string, accountId: string, weight?: number): Promise<void> {
  return invoke("add_account_to_group", { groupId, accountId, weight });
}

export function removeAccountFromGroup(groupId: string, accountId: string): Promise<void> {
  return invoke("remove_account_from_group", { groupId, accountId });
}

export function getGroupModelResources(groupId: string): Promise<GroupModelResource[]> {
  return invoke("get_group_model_resources", { groupId });
}

export function listAvailableGroupModelResources(groupId: string): Promise<AvailableGroupModelResource[]> {
  return invoke("list_available_group_model_resources", { groupId });
}

export function addGroupModelResources(groupId: string, resources: GroupModelResource[]): Promise<number> {
  return invoke("add_group_model_resources", { groupId, resources });
}

export function setGroupModelAccountIds(
  groupId: string,
  providerId: string,
  model: string,
  accountIds: string[],
): Promise<void> {
  return invoke("set_group_model_account_ids", {
    groupId,
    providerId,
    model,
    accountIds,
  });
}

export function removeGroupModelResource(groupId: string, providerId: string, model: string): Promise<boolean> {
  return invoke("remove_group_model_resource", { groupId, providerId, model });
}

export function setGroupModelResources(groupId: string, resources: GroupModelResource[]): Promise<void> {
  return invoke("set_group_model_resources", { groupId, resources });
}

export function getGroupDashboard(groupId: string, range?: string): Promise<GroupDashboard> {
  return invoke("get_group_dashboard", { groupId, range });
}

export function getRouteTopology(): Promise<RouteTopology> {
  return invoke("get_route_topology");
}

export function getProviderTopologyDetail(providerId: string): Promise<ProviderTopologyDetail> {
  return invoke("get_provider_topology_detail", { providerId });
}

export function detectAgentApps(): Promise<AgentAppInfo[]> {
  return invoke("detect_agent_apps");
}

export function previewAgentAppConfig(appId: string, groupId: string): Promise<AgentAppPreview> {
  return invoke("preview_agent_app_config", { appId, groupId });
}

export function configureAndLaunchAgentApp(
  appId: string,
  groupId: string,
  confirmed: boolean,
  workingDirectory?: string,
): Promise<AgentAppLaunchResult> {
  return invoke("configure_and_launch_agent_app", { appId, groupId, confirmed, workingDirectory });
}

export function restoreAgentAppConfig(snapshotId: string, force?: boolean): Promise<void> {
  return invoke("restore_agent_app_config", { snapshotId, force });
}

// ============ Log Commands ============

export function queryLogs(query: LogQuery): Promise<RequestLog[]> {
  return invoke("query_logs", { query });
}

export function getLogStats(range?: LogRange): Promise<LogStats> {
  return invoke("get_log_stats", { range });
}

export function getTraySnapshot(): Promise<TraySnapshot> {
  return invoke("get_tray_snapshot");
}

export function openPoolGateFromTray(page?: "dashboard" | "groups" | "settings", node?: string): Promise<void> {
  return invoke("open_poolgate_from_tray", { page, node });
}

export function resizeTrayWindow(width: number, height: number): Promise<void> {
  return invoke("resize_tray_window", { width, height });
}

export function quitPoolGateFromTray(): Promise<void> {
  return invoke("quit_poolgate_from_tray");
}

export function getAnalytics(
  startDate: string,
  endDate: string,
): Promise<AnalyticsData> {
  return invoke("get_analytics", { startDate, endDate });
}

// ============ App Log File Commands ============

export function readAppLogs(params: {
  page?: number;
  page_size?: number;
  keyword?: string;
} = {}): Promise<AppLogPage> {
  return invoke("read_app_logs", {
    page: params.page,
    page_size: params.page_size,
    keyword: params.keyword,
  });
}

export function getAppLogInfo(): Promise<AppLogInfo> {
  return invoke("get_app_log_info");
}

// ============ Proxy Commands ============

export function startProxy(): Promise<void> {
  return invoke("start_proxy");
}

export function stopProxy(): Promise<void> {
  return invoke("stop_proxy");
}

export function getProxyStatus(): Promise<{ running: boolean; port: number; active_connections: number }> {
  return invoke("get_proxy_status");
}

// ============ Gateway Settings Commands ============

export interface GatewaySettings {
  /** Whether an access key is currently configured. Secret values are write-only. */
  access_key_set: boolean;
  /** Close button behavior: "hide" to minimize to tray, "quit" to exit application */
  close_button_behavior: string;
}

export function getGatewaySettings(): Promise<GatewaySettings> {
  return invoke("get_gateway_settings");
}

/** Set (or clear, when passing an empty string) the gateway access key. */
export function setGatewayAccessKey(accessKey: string): Promise<void> {
  return invoke("set_gateway_access_key", { accessKey });
}

/** Set the close button behavior: "hide" or "quit" */
export function setCloseButtonBehavior(behavior: string): Promise<void> {
  return invoke("set_close_button_behavior", { behavior });
}

/** Get the close button behavior setting */
export function getCloseButtonBehavior(): Promise<string> {
  return invoke("get_close_button_behavior");
}

// ============ Client Key Commands (virtual keys → route pools) ============

export interface ClientKeyView {
  id: string;
  name: string;
  key_prefix: string;
  key_last_four: string;
  enabled: boolean;
  rpm_limit?: number;
  tpm_limit?: number;
  allowed_protocols?: string;
  allowed_models?: string;
  expires_at?: string;
  last_used_at?: string;
  created_at?: string;
  managed_pool_id?: string;
  rotated_at?: string;
  /** Route pools this key is bound to (empty = default pool / full pool). */
  pool_ids: string[];
}

export interface ClientKeyCreated {
  key: ClientKeyView;
  /** Plaintext virtual key — show once, then discard. */
  raw_key: string;
}

/** Create a virtual client key, optionally bound to route pools. */
export function createClientKey(
  name: string,
  poolIds?: string[],
  enabled?: boolean,
): Promise<ClientKeyCreated> {
  return invoke("create_client_key", { name, poolIds, enabled });
}

export function listClientKeys(): Promise<ClientKeyView[]> {
  return invoke("list_client_keys");
}

export function updateClientKey(
  id: string,
  opts: {
    name?: string;
    enabled?: boolean;
    rpmLimit?: number;
    tpmLimit?: number;
    allowedProtocols?: string;
    allowedModels?: string;
    expiresAt?: string;
  },
): Promise<void> {
  return invoke("update_client_key", { id, ...opts });
}

export function deleteClientKey(id: string): Promise<void> {
  return invoke("delete_client_key", { id });
}

/** Replace the route pool bindings of a key (empty list = unbind all). */
export function setClientKeyPools(clientKeyId: string, poolIds: string[]): Promise<void> {
  return invoke("set_client_key_pools", { clientKeyId, poolIds });
}

export function getClientKeyPools(clientKeyId: string): Promise<string[]> {
  return invoke("get_client_key_pools", { clientKeyId });
}

// ============ Import/Export Commands ============

export function previewImport(path: string): Promise<ImportPreview> {
  return invoke("preview_import", { path });
}

export function previewImportSource(request: ImportSourceRequest): Promise<ImportPreview> {
  return invoke("preview_import_source", { request });
}

export function executeImport(
  request: ImportSourceRequest,
  options: ImportOptions,
): Promise<ImportResult> {
  return invoke("execute_import", { request, options });
}

export function fetchUpstreamModels(
  baseUrl: string,
  apiKey?: string,
  protocol?: string,
): Promise<string[]> {
  return invoke("fetch_upstream_models", { baseUrl, apiKey, protocol });
}

export function refreshAccountModels(accountId: string): Promise<ModelRefreshResult> {
  return invoke("refresh_account_models", { accountId });
}

export function batchRefreshAccountModels(accountIds: string[]): Promise<ModelRefreshResult[]> {
  return invoke("batch_refresh_account_models", { accountIds });
}

export function previewAndCheckImport(
  request: ImportSourceRequest,
  options: ImportOptions,
): Promise<ImportCheckResult> {
  return invoke("preview_and_check_import", { request, options });
}

export function detectImportFormat(path: string): Promise<string> {
  return invoke("detect_import_format", { path });
}

/** Scan well-known locations for Cockpit-tools / Codex CLI config files. */
export function scanAgentConfigs(): Promise<AgentConfigScanResult> {
  return invoke("scan_agent_configs");
}

export function detectImportContent(content: string, sourceName?: string): Promise<string> {
  return invoke("detect_import_content", { content, sourceName });
}

export function exportAccounts(format: string, maskKeys?: boolean): Promise<string> {
  return invoke("export_accounts", { format, maskKeys });
}

export function exportAccount(accountId: string, format: string, maskKeys?: boolean): Promise<string> {
  return invoke("export_account", { accountId, format, maskKeys });
}

export function exportLogsCsv(startTime?: string, endTime?: string): Promise<string> {
  return invoke("export_logs_csv", { startTime, endTime });
}
