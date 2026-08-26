import { useEffect, useRef } from "react";
import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import {
  getAccountUsageStats,
  getCollectorStatus,
  getServiceStatus,
  getTokenMonitorSnapshot,
  getTokenRate,
  getUsageTrend,
  listActiveSessions,
  listModelUsage,
  listProjects,
  listQuotaAccounts,
  listSessionEvents,
  listToolUsage,
  type AccountUsageStat,
  type CollectorStatus as CS,
  type QuotaAccountView,
  type Range,
  type ServiceStatusView,
  type SessionEventRow,
  type SessionSummary,
  type TokenMonitorSnapshot,
  type TokenRateView,
  type ToolCollectorState,
  type ToolUsageRow,
  type TrendSeries,
} from "@/lib/token-monitor-commands";

/**
 * W6 数据层：优先 invoke 真数据；invoke 不可用（未连接/未启动）时回退到本地 mock，
 * 便于托盘/桌面预览。联调期（批次 3）接真数据后仍保持同一接口。
 */

// ---------------- mock 数据 ----------------
const mockSnapshot: TokenMonitorSnapshot = {
  updated_at: new Date().toISOString(),
  range: "day",
  collector_state: "active",
  active_tools: 4,
  usage: { input_tokens: 482_000, output_tokens: 156_000, cache_tokens: 1_210_000, total_tokens: 638_000, cost_amount: 1.86 },
  top_tools: [
    { tool_id: "claude_code", display_name: "Claude Code", support_level: "full", collector_status: "active", input_tokens: 310_000, output_tokens: 98_000, cache_tokens: 900_000, total_tokens: 408_000, cost_amount: 1.24, share_percent: 63.9 },
    { tool_id: "codex", display_name: "Codex CLI", support_level: "standard", collector_status: "active", input_tokens: 96_000, output_tokens: 34_000, cache_tokens: 190_000, total_tokens: 130_000, cost_amount: 0.42, share_percent: 20.4 },
    { tool_id: "opencode", display_name: "OpenCode", support_level: "standard", collector_status: "active", input_tokens: 54_000, output_tokens: 16_000, cache_tokens: 80_000, total_tokens: 70_000, cost_amount: 0.12, share_percent: 11.0 },
    { tool_id: "cursor", display_name: "Cursor", support_level: "basic", collector_status: "idle", input_tokens: 22_000, output_tokens: 8_000, cache_tokens: 40_000, total_tokens: 30_000, cost_amount: 0.08, share_percent: 4.7 },
  ],
  top_models: [
    { model: "claude-3.5-sonnet", input_tokens: 300_000, output_tokens: 92_000, cache_tokens: 860_000, total_tokens: 392_000, cost_amount: 1.18, share_percent: 61.4 },
    { model: "gpt-4o", input_tokens: 120_000, output_tokens: 44_000, cache_tokens: 260_000, total_tokens: 164_000, cost_amount: 0.52, share_percent: 25.7 },
    { model: "deepseek-chat", input_tokens: 62_000, output_tokens: 20_000, cache_tokens: 90_000, total_tokens: 82_000, cost_amount: 0.16, share_percent: 12.9 },
  ],
  recent_projects: [
    { project_id: "p1", display_name: "poolgate", total_tokens: 410_000, session_count: 24, last_active_at: new Date().toISOString(), cost_amount: 1.24, tools: [{ tool_id: "claude_code", display_name: "Claude Code", total_tokens: 330_000, share_percent: 80.5 }, { tool_id: "codex", display_name: "Codex CLI", total_tokens: 80_000, share_percent: 19.5 }] },
    { project_id: "p2", display_name: "web-app", total_tokens: 128_000, session_count: 9, last_active_at: new Date().toISOString(), cost_amount: 0.42, tools: [{ tool_id: "codex", display_name: "Codex CLI", total_tokens: 128_000, share_percent: 100 }] },
  ],
  critical_quota: { account_id: "tm_claude_1", provider_id: "claude", window_type: "rolling_5h", remaining_percent: 12, resets_at: new Date().toISOString() },
};

const mockTrend: TrendSeries = {
  daily: Array.from({ length: 14 }, (_, i) => {
    const date = new Date(Date.now() - (13 - i) * 86_400_000).toISOString().slice(0, 10);
    return { date, tokens: 300_000 + Math.round(Math.sin(i / 2) * 160_000) + i * 22_000, requests: 80 + i * 7, cost_amount: 0.8 + i * 0.09 };
  }),
  active_days: 14,
  streak_days: 6,
  peak_day: { date: new Date().toISOString().slice(0, 10), tokens: 780_000, requests: 142, cost_amount: 2.4 },
  monthly: [
    { month: "2026-06", tokens: 9_400_000 },
    { month: "2026-07", tokens: 12_800_000 },
    { month: "2026-08", tokens: 7_200_000 },
  ],
  active_time_ms: 0,
  message_count: 50,
};

const mockCollectors: ToolCollectorState[] = [
  { tool_id: "claude_code", display_name: "Claude Code", installed: true, enabled: true, support_level: "full", status: "active", last_collected_at: new Date().toISOString(), paths: ["~/.claude/projects/**/*.jsonl"] },
  { tool_id: "codex", display_name: "Codex CLI", installed: true, enabled: true, support_level: "standard", status: "active", last_collected_at: new Date().toISOString(), paths: ["~/.codex/sessions/**/*.jsonl"] },
  { tool_id: "cursor", display_name: "Cursor", installed: true, enabled: true, support_level: "basic", status: "idle", paths: [] },
  { tool_id: "opencode", display_name: "OpenCode", installed: true, enabled: true, support_level: "standard", status: "active", paths: ["~/.opencode/sessions/**"] },
];

const mockQuotaAccounts: QuotaAccountView[] = [
  {
    account_id: "gw:acct_086f9de9d9a445c9b3a3c7d4cbdf8f1d",
    // 真实网关数据形态：provider_id 是 prov_<hex> 不透明 id，可读名由 providers 表解析
    provider_id: "prov_3fbdc88ab3b54ffdbab72772d5fdb951", provider_label: "OpenAI Codex",
    label: "ivenkral@gmail.com", identity_masked: "ivenkral@gmail.com",
    plan_name: "free", status: "active", enabled: true, last_success_at: new Date().toISOString(),
    windows: [
      { window_key: "primary", window_type: "rolling_5h", unit: "percent", label: "5 小时额度", used_value: 0, limit_value: 100, remaining_value: 100, remaining_percent: 100, resets_at: new Date(Date.now() + 86400000 * 10).toISOString(), source: "official_api", confidence: "reported", fetched_at: new Date().toISOString() },
      { window_key: "secondary", window_type: "weekly", unit: "percent", label: "周额度", used_value: 8, limit_value: 100, remaining_value: 92, remaining_percent: 92, resets_at: new Date(Date.now() + 86400000 * 3).toISOString(), source: "official_api", confidence: "reported", fetched_at: new Date().toISOString() },
    ],
  },
  {
    account_id: "tm_claude_1", provider_id: "claude", label: "Claude Pro", identity_masked: "user@example.com",
    plan_name: "Pro", status: "active", enabled: true, last_success_at: new Date().toISOString(),
    windows: [
      { window_key: "primary", window_type: "rolling_5h", unit: "percent", label: "5 小时额度", used_value: 88, limit_value: 100, remaining_value: 12, remaining_percent: 12, resets_at: new Date().toISOString(), source: "official_api", confidence: "reported", fetched_at: new Date().toISOString() },
      { window_key: "secondary", window_type: "weekly", unit: "percent", label: "周额度", used_value: 41, limit_value: 100, remaining_value: 59, remaining_percent: 59, resets_at: new Date().toISOString(), source: "official_api", confidence: "reported", fetched_at: new Date().toISOString() },
    ],
  },
  {
    account_id: "tm_codex_1", provider_id: "codex", provider_label: "OpenAI Codex", label: "ChatGPT Plus", identity_masked: "chen.li@gmail.com",
    plan_name: "Plus", status: "active", enabled: true, last_success_at: new Date().toISOString(),
    windows: [
      { window_key: "primary", window_type: "rolling_5h", unit: "percent", label: "5 小时额度", used_value: 34, limit_value: 100, remaining_value: 66, remaining_percent: 66, resets_at: new Date().toISOString(), source: "official_api", confidence: "reported", fetched_at: new Date().toISOString() },
      { window_key: "secondary", window_type: "weekly", unit: "percent", label: "周额度", used_value: 21, limit_value: 100, remaining_value: 79, remaining_percent: 79, resets_at: new Date().toISOString(), source: "official_api", confidence: "reported", fetched_at: new Date().toISOString() },
    ],
  },
  {
    account_id: "tm_ds_1", provider_id: "deepseek", label: "DeepSeek", identity_masked: "ds-deploy@deepseek.com",
    plan_name: "按量", status: "active", enabled: true, last_success_at: new Date().toISOString(),
    windows: [
      { window_key: "balance", window_type: "prepaid_balance", unit: "currency", label: "账户余额（CNY）", remaining_value: 97.6, source: "official_api", confidence: "reported", fetched_at: new Date().toISOString() },
    ],
  },
  {
    account_id: "tm_ds_2", provider_id: "deepseek", label: "DeepSeek 备用", identity_masked: "ds-backup@deepseek.com",
    plan_name: "按量", status: "active", enabled: true, last_success_at: new Date().toISOString(),
    windows: [
      { window_key: "balance", window_type: "prepaid_balance", unit: "currency", label: "账户余额（CNY）", remaining_value: 0, source: "official_api", confidence: "reported", fetched_at: new Date().toISOString() },
    ],
  },
  {
    account_id: "tm_exhausted_1", provider_id: "codex", provider_label: "OpenAI Codex", label: "ChatGPT 已用尽", identity_masked: "exhausted@example.com",
    plan_name: "Plus", status: "active", enabled: true, last_success_at: new Date().toISOString(),
    windows: [
      { window_key: "primary", window_type: "rolling_5h", unit: "percent", label: "5 小时额度", used_value: 100, limit_value: 100, remaining_value: 0, remaining_percent: 0, resets_at: new Date().toISOString(), source: "official_api", confidence: "reported", fetched_at: new Date().toISOString() },
    ],
  },
  {
    account_id: "tm_gemini_1", provider_id: "gemini", provider_label: "Gemini", label: "Gemini", identity_masked: "gemini@example.com",
    plan_name: "Pro", status: "active", enabled: true, last_success_at: new Date().toISOString(),
    windows: [
      { window_key: "unavailable", window_type: "billing", unit: "percent", label: "额度接口不可用（供应商未提供公开接口）", source: "official_api", confidence: "derived", fetched_at: new Date().toISOString(), error_code: "unsupported" },
    ],
  },
];

/** 实时速率 mock（Logo 点击轮询展示 tok/min、tok/s）。 */
const mockRate: TokenRateView = { window_secs: 60, tokens_per_sec: 245.6, tokens_per_min: 14_736 };

/** 按账号 TOKEN 统计 mock：只给有网关流量的 gw Codex 账号（对齐参考图 555万/7692万/1.2亿/14.1亿），
 *  其余账号无统计 → 卡片对应格显示「—」（与真实后端口径一致：request_logs 无数据不返回）。 */
const mockUsageStats: AccountUsageStat[] = [
  {
    account_id: "gw:acct_086f9de9d9a445c9b3a3c7d4cbdf8f1d",
    today_tokens: 5_550_000,
    yesterday_tokens: 5_440_000,
    week_tokens: 76_920_000,
    month_tokens: 120_000_000,
    total_tokens: 1_410_000_000,
    request_count: 312,
  },
];

const mockSessions: SessionSummary[] = [
  { session_id: "s1", tool_id: "claude_code", external_session_id: "ext-1", project_id: "p1", title_redacted: "poolgate · 08-08 14:20", model_set: ["claude-3.5-sonnet"], started_at: new Date().toISOString(), last_active_at: new Date().toISOString(), input_tokens: 128_000, output_tokens: 42_000, cache_tokens: 320_000, total_tokens: 170_000, message_count: 38, status: "active" },
  { session_id: "s2", tool_id: "codex", project_id: "p1", title_redacted: "poolgate · 08-08 16:05", model_set: ["gpt-4o"], started_at: new Date().toISOString(), last_active_at: new Date().toISOString(), input_tokens: 24_000, output_tokens: 9_000, cache_tokens: 40_000, total_tokens: 33_000, message_count: 12, status: "idle" },
];

/** 容错：invoke 失败时回退 mock（未连接/预览环境）。 */
async function withFallback<T>(fetcher: () => Promise<T>, mock: T): Promise<T> {
  try {
    return await fetcher();
  } catch {
    return mock;
  }
}

// ---------------- hooks ----------------

/**
 * 后端采集事件负载（与 Rust `UsageDelta` serde 对齐，`service.rs` 250ms 合并后 emit）。
 */
export interface TmUsageDelta {
  sequence: number;
  emitted_at: string;
  range: string;
  total_tokens: number;
  cost_amount?: number | null;
  active_tools: number;
  top_tools: ToolUsageRow[];
}

/**
 * 实时刷新（对齐开源 Token Monitor 的「文件监听事件驱动采集 + 展示端周期刷新」两段式）：
 * 后端 watcher 采集落库后经 `token-monitor:usage-delta`（250ms 合并窗口）广播，
 * 这里监听该事件 → 立即失效今日范围用量查询，让 TOKENS 秒级变化；
 * 各查询自身的 `refetchInterval` 作为兜底，事件丢失时也能周期刷新。
 *
 * 调用方：桌面 Token Monitor 仪表盘 + 托盘卡片（各自 webview 实例独立注册一次）。
 */
export function useTokenMonitorRealtime(enabled = true) {
  const queryClient = useQueryClient();
  useEffect(() => {
    if (!enabled) return;
    const usageUnlisten = listen<TmUsageDelta>("token-monitor:usage-delta", () => {
    // 采集落库后立即重取所有直接依赖 Token 事件的视图。账号 Token 统计与
    // 实时速率此前仅按 60/15 秒轮询，事件到达后仍会显示旧值；一并失效后，
    // 活跃 Webview 会立即重新调用后端。
      invalidateUsageQueries(queryClient);
    });
    // W7：`token-monitor:session-changed`（会话新增/更新）→ 只失效会话列表/明细，
    // 比用量事件的宽失效面更轻量、更即时（新会话一落库列表即刷新）。
    const sessionUnlisten = listen<SessionSummary>("token-monitor:session-changed", () => {
      invalidateSessionQueries(queryClient);
    });
    return () => {
      void usageUnlisten.then((unlisten) => unlisten());
      void sessionUnlisten.then((unlisten) => unlisten());
    };
  }, [queryClient, enabled]);
}
/** 用量事件失效面：快照/明细/账号 Token 统计/速率（趋势与额度各有专属节奏）。 */
function invalidateUsageQueries(queryClient: QueryClient) {
  void queryClient.invalidateQueries({
    predicate: (q) => {
      const key = String(q.queryKey[0] ?? "");
      return ["tm-snapshot", "tm-tools", "tm-models", "tm-projects", "tm-sessions", "tm-session-events", "tm-account-usage-stats", "tm-rate"].includes(key);
    },
  });
}

/** 会话变更失效面：会话列表（所有范围）+ 逐轮明细。 */
function invalidateSessionQueries(queryClient: QueryClient) {
  void queryClient.invalidateQueries({
    predicate: (q) => {
      const key = String(q.queryKey[0] ?? "");
      return ["tm-sessions", "tm-session-events"].includes(key);
    },
  });
}

// ============ 额度/采集告警（W5：`token-monitor:alert` → 系统通知 + 应用内 Toast）============

/** 后端告警负载（与 Rust `TmAlert` serde 对齐）。 */
export interface TmAlert {
  /** quota_low | quota_stale | auth_expired | collector_error */
  kind: string;
  /** remind | warn | critical */
  level: string;
  account_id?: string | null;
  tool_id?: string | null;
  message: string;
  emitted_at: string;
}

/** 进程内去抖：同一告警（kind+level+account+emitted_at）60s 内不重复处理
 *  （防 StrictMode 双挂与事件重发；后端自身已按账号/窗口/级别 30 分钟去抖一次）。 */
const recentAlerts = new Map<string, number>();
const ALERT_DEDUP_MS = 60_000;

/**
 * 监听后端 `token-monitor:alert`（额度越阈 / 数据过期等，W5）：
 *  - **系统通知由 Rust 后端直接发送**（publish_alerts），前端不再发，天然无跨窗口竞态；
 *  - 这里只做应用内 Toast（回调 `onAlert`）与额度视图刷新；
 *  - 进程内 60s 去抖防 StrictMode 双挂/事件重发。
 *
 * 调用方：主窗口（App 内）与托盘窗口（main.tsx 挂 ToastProvider 旁），各挂载一次。
 */
export function useTokenMonitorAlerts(onAlert?: (alert: TmAlert) => void) {
  const queryClient = useQueryClient();
  const onAlertRef = useRef(onAlert);
  onAlertRef.current = onAlert;

  useEffect(() => {
    const unlistenPromise = listen<TmAlert>("token-monitor:alert", (event) => {
      const alert = event.payload;
      const dedupKey = `${alert.kind}|${alert.level}|${alert.account_id ?? ""}|${alert.emitted_at}`;
      const now = Date.now();
      const last = recentAlerts.get(dedupKey);
      if (last && now - last < ALERT_DEDUP_MS) return;
      recentAlerts.set(dedupKey, now);
      // 顺带清理过期键，避免 Map 无限增长
      if (recentAlerts.size > 64) {
        for (const [key, t] of recentAlerts) {
          if (now - t >= ALERT_DEDUP_MS) recentAlerts.delete(key);
        }
      }
      // 告警 = 额度快照已更新 → 刷新额度视图
      void queryClient.invalidateQueries({ queryKey: ["tm-quota"] });
      onAlertRef.current?.(alert);
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [queryClient]);
}

export function useTokenMonitorSnapshot(range: Range) {
  return useQuery({
    queryKey: ["tm-snapshot", range],
    queryFn: () => withFallback(() => getTokenMonitorSnapshot(range), { ...mockSnapshot, range }),
    staleTime: 5_000,
    // 10s 轮询兜底（对齐托盘节奏；事件驱动到达时刷新更即时）
    refetchInterval: 10_000,
  });
}

export function useToolUsage(range: Range) {
  return useQuery({
    queryKey: ["tm-tools", range],
    queryFn: () => withFallback(() => listToolUsage({ range }), mockSnapshot.top_tools),
    staleTime: 5_000,
    refetchInterval: 10_000,
  });
}

export function useModelUsage(range: Range) {
  return useQuery({
    queryKey: ["tm-models", range],
    queryFn: () => withFallback(() => listModelUsage({ range }), mockSnapshot.top_models),
    staleTime: 5_000,
    refetchInterval: 10_000,
  });
}

export function useProjects(range: Range) {
  return useQuery({
    queryKey: ["tm-projects", range],
    queryFn: () => withFallback(() => listProjects({ range }), mockSnapshot.recent_projects),
    staleTime: 5_000,
    refetchInterval: 10_000,
  });
}

// 趋势数据模块级缓存：`get_usage_trend` 后端可能运行 ~30s 的 `tokscale graph` 子进程，
// 缓存上次成功结果作为 placeholder，保证切换视图/托盘后立即展示（后台再刷新），
// 对齐开源 Token Monitor 的「历史数据即时可见」体验。
const trendCache = new Map<Range, TrendSeries>();

export function useTrend(range: Range) {
  return useQuery({
    queryKey: ["tm-trend", range],
    queryFn: async () => {
      const data = await withFallback(() => getUsageTrend({ range }), mockTrend);
      trendCache.set(range, data);
      return data;
    },
    // 数据未就绪时先展示上次结果，避免每次都白屏等待子进程
    placeholderData: () => trendCache.get(range),
    // 刷新频率对齐后端 90s 子进程缓存 TTL，避免 10s 自动刷新反复触发重算
    staleTime: 90_000,
    refetchInterval: 90_000,
  });
}

export function useCollectorStatus() {
  return useQuery({
    queryKey: ["tm-collectors"],
    queryFn: () => withFallback(() => getCollectorStatus(), mockCollectors),
    staleTime: 15_000,
    refetchInterval: 30_000,
  });
}

export function useQuotaAccounts() {
  return useQuery({
    queryKey: ["tm-quota"],
    queryFn: () => withFallback(() => listQuotaAccounts(), mockQuotaAccounts),
    staleTime: 30_000,
    // 额度后端 5 分钟循环刷新（<20% 缩到 2 分钟），前端 60s 轮询对齐其节奏
    refetchInterval: 60_000,
  });
}

/** 按账号聚合的用量统计（额度卡 2×2 TOKEN 统计格）：60s 轮询与额度同节奏；
 *  无网关流量的账号不在返回列表 → 卡片对应格显示「—」。 */
export function useAccountUsageStats() {
  return useQuery({
    queryKey: ["tm-account-usage-stats"],
    queryFn: () => withFallback(() => getAccountUsageStats(), mockUsageStats),
    staleTime: 5_000,
    // 实时事件会立即失效；5 秒轮询只处理事件不可达时的安全回补。
    refetchInterval: 5_000,
  });
}

/** 会话列表按范围过滤（托盘范围选择器驱动）；默认 total = 全部会话（桌面端行为不变）。 */
export function useActiveSessions(range: Range = "total") {
  return useQuery({
    queryKey: ["tm-sessions", range],
    queryFn: () => withFallback(() => listActiveSessions({ range }), mockSessions),
    staleTime: 15_000,
    refetchInterval: 30_000,
  });
}

/** 会话逐轮用量明细（W7 会话下钻）：展开某会话时按需加载，收起/离开自动停用。
 *  invoke 不可用回退空数组（预览环境显示「暂无明细」，不伪造数据）。
 *  30s 轮询 + `token-monitor:usage-delta` 事件失效，与列表同节奏。 */
export function useSessionEvents(sessionId: string | null) {
  return useQuery({
    queryKey: ["tm-session-events", sessionId],
    queryFn: () => withFallback(() => listSessionEvents(sessionId ?? ""), []),
    enabled: !!sessionId,
    staleTime: 15_000,
    refetchInterval: 30_000,
  });
}

/** 供应商服务状态（Claude / OpenAI / Cursor / DeepSeek 状态页）。60s 轮询对齐后端缓存。 */
export function useServiceStatus() {
  return useQuery({
    queryKey: ["tm-service-status"],
    queryFn: () => withFallback(() => getServiceStatus(), []),
    staleTime: 60_000,
    refetchInterval: 60_000,
  });
}

/** 实时速率（托盘 Logo 点击展示 tokens/s 与 tokens/min）。
 *  `enabled` 用于托盘仅在速率读数可见时才轮询，避免无谓查询。 */
export function useTokenRate(enabled = true) {
  return useQuery({
    queryKey: ["tm-rate"],
    queryFn: () => withFallback(() => getTokenRate(), mockRate),
    staleTime: 3_000,
    refetchInterval: 5_000,
    enabled,
  });
}
// 命令直通（Tools 页操作按钮）：invoke 不可用时由调用方静默兜底
export { scanAllTools, refreshTokenMonitor, rescanTool, resetToolData } from "@/lib/token-monitor-commands";

// ---------------- 展示辅助 ----------------
/** 中文量级格式化（对齐参考图）：`1410000000 → 14.1亿`、`76920000 → 7692万`、`5550000 → 555万`。 */
export const formatWan = (n: number): string => {
  if (n >= 100_000_000) return `${(n / 100_000_000).toFixed(1).replace(/\.0$/, "")}亿`;
  if (n >= 10_000) return `${Math.round(n / 10_000)}万`;
  return `${n}`;
};

export const formatTokens = (n: number): string => {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return `${n}`;
};

/** 会话消息数（对齐开源 Token Monitor sessionRows.js messageLabel）：
 *  千分位 + 单复数（`1 msg` / `N msgs`）；0/缺失返回空串，副标题中省略该段。 */
export const formatMessageCount = (count: number | undefined | null): string => {
  const n = Math.round(Number(count) || 0);
  if (n <= 0) return "";
  return `${n.toLocaleString("en-US")} msg${n === 1 ? "" : "s"}`;
};

/** 多模型会话的消息数口径提示（悬停 tooltip）：tokscale 按 (client, session, model)
 *  分组上报 messageCount，跨模型会话的消息数为各分组之和（与开源 Token Monitor 一致）。 */
export const MULTI_MODEL_MSG_HINT =
  "多模型会话：消息数为各模型分组消息数之和（tokscale 口径，与开源 Token Monitor 一致）";

export const collectorStatusText: Record<CS, string> = {
  idle: "空闲", active: "采集中", waiting: "等待中", permission: "需授权",
  path_missing: "路径缺失", format_changed: "格式变化", partial: "部分采集", error: "错误",
};

export const collectorStatusTone: Record<CS, "mute" | "ok" | "warn" | "err" | "info"> = {
  idle: "mute", active: "ok", waiting: "info", permission: "warn",
  path_missing: "warn", format_changed: "warn", partial: "info", error: "err",
};

export type { Range, TokenMonitorSnapshot, ToolUsageRow, TrendSeries, QuotaAccountView, SessionSummary, ToolCollectorState, ServiceStatusView };
