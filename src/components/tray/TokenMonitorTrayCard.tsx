import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowLeft, Check, ChevronDown, ChevronRight, Clock, Coins, Gauge, Cpu, Wrench, MessageSquare,
  RefreshCw, TrendingUp, Settings2, Power, ServerCog, LogOut, Activity as ActivityIcon,
} from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import {
  useTokenMonitorSnapshot, useToolUsage, useModelUsage, useTrend,
  useQuotaAccounts, useAccountUsageStats, useActiveSessions, useCollectorStatus, useTokenRate,
  formatTokens, formatMessageCount, formatWan, MULTI_MODEL_MSG_HINT,
  refreshTokenMonitor, useTokenMonitorRealtime, useSessionEvents,
} from "@/components/token-monitor/token-monitor-data";
import {
  providerLabel, accountIdentity, accountBrandParts, accountTier,
  isBalanceWindow, currencyOf, formatBalance, balanceTone,
} from "@/lib/account-display";
import { useAccountDisplay } from "@/components/ui/AccountDisplay";
import type { Range, SessionSummary, SessionEventRow, ModelUsageRow, QuotaAccountView, QuotaWindowView, AccountUsageStat, ToolUsageRow, TrendDay } from "@/lib/token-monitor-commands";
import ActivityHeatmap, { type HeatmapDay } from "@/components/ActivityHeatmap";
import Sparkline from "@/components/tray/Sparkline";
import RingProgress from "@/components/tray/RingProgress";
import TokensQuotaMerged, { pickPrimary, type TokenRange } from "@/components/token-monitor/TokensQuotaMerged";
import ToolLogo, { ProviderLogo } from "@/components/token-monitor/ToolLogo";
import {
  openPoolGateFromTray, quitPoolGateFromTray, startProxy, stopProxy, getProxyStatus,
} from "@/lib/tauri-commands";
import { useAppMode } from "@/hooks/use-tauri";

/** Date → 本地 YYYY-MM-DD。 */
function localKey(date: Date): string {
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** 本地今天（YYYY-MM-DD），热力图锚定窗口右端（与桌面端 Overview 一致）。 */
function todayKey(): string {
  return localKey(new Date()  );
}



/** 范围起始日（YYYY-MM-DD），与后端 range_start_sql 口径一致（趋势明细页「本月」裁剪用）。 */
function rangeStartKey(range: Range): string {
  const now = new Date();
  if (range === "day") return localKey(now);
  if (range === "7d") {
    const d = new Date(now);
    d.setDate(d.getDate() - 6);
    return localKey(d);
  }
  if (range === "month") {
    return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-01`;
  }
  return "0000-00-00";
}

type TmView = "home" | "quota" | "tools" | "models" | "sessions" | "trend";

const RANGE_TABS: Array<{ key: Range; label: string }> = [
  { key: "day", label: "今日" },
  { key: "7d", label: "近 7 天" },
  { key: "month", label: "本月" },
  { key: "total", label: "累计" },
];

const RANGE_LABEL: Record<Range, string> = { day: "今日", "7d": "近 7 天", month: "本月", total: "累计" };

// 明细视图切换（非首页时显示；保留 工具/会话 等全部下钻入口）
const DETAIL_TABS: Array<{ key: Exclude<TmView, "home">; label: string; Icon: typeof Gauge }> = [
  { key: "quota", label: "额度", Icon: Gauge },
  { key: "tools", label: "工具", Icon: Wrench },
  { key: "models", label: "模型", Icon: Cpu },
  { key: "sessions", label: "会话", Icon: MessageSquare },
  { key: "trend", label: "趋势", Icon: TrendingUp },
];

// 每项配色（贴近开源 Token Monitor 的柔和多彩条）
const PALETTE = ["#d98a5c", "#5b8def", "#37b6a0", "#d56a54", "#9b7fe0", "#c2a24d", "#5cc2d9", "#d97fb0"];

// 与后端 display_name_for_tool 同源：托盘会话/工具/模型列表的显示名
const TOOL_NAMES: Record<string, string> = {
  claude_code: "Claude Code",
  codex: "Codex CLI",
  opencode: "OpenCode",
  cursor: "Cursor",
  github_copilot: "GitHub Copilot",
  workbuddy: "WorkBuddy",
  mimo: "MiMo Code",
  zcode: "ZCode",
  codebuddy: "CodeBuddy",
  antigravity: "Antigravity",
  kimi: "Kimi",
  qwen: "Qwen",
  grok_build: "Grok Build",
  hermes: "Hermes",
  zed: "Zed Agent",
  kiro: "Kiro",
  cline: "Cline",
  kilo_code: "Kilo Code",
  pi: "Pi",
  proma: "Proma",
  openclaw: "OpenClaw",
  gemini: "Gemini CLI",
};

function toolDisplay(toolId: string, fallback?: string): string {
  if (fallback && fallback.trim()) return fallback;
  return TOOL_NAMES[toolId] ?? toolId.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase()  );
}



/** 额度账号的「剩余」数值（百分比优先；无百分比但有真实值退化为值；全无 → null）。
 *  百分比统一钳制到 [0,100]；非有限数值（异常号码）一律按无数据处理，不在界面展示。 */
function remainingOf(window: QuotaWindowView): number | null {
  if (window.remaining_percent != null) {
    if (!Number.isFinite(window.remaining_percent)) return null;
    return Math.max(0, Math.min(100, window.remaining_percent));
  }
  if (window.remaining_value != null) {
    if (!Number.isFinite(window.remaining_value)) return null;
    return window.remaining_value;
  }
  return null;
}

/** 额度账号卡片（一账号一卡）：只保留有真实剩余数据的窗口（额度为 0 的保留展示），
 *  按「卡片展示的主窗口剩余」降序（与圆环百分比一致，0% 排在末尾）。 */
type QuotaCard = { account: QuotaAccountView; windows: QuotaWindowView[]; best: number | null };

function quotaCards(accounts: QuotaAccountView[]): QuotaCard[] {
  const cards = accounts
    .map((account) => {
      const windows = account.windows.filter((w) => remainingOf(w) != null);
      const primary = pickPrimary(windows);
      const best =
        primary != null
          ? remainingOf(primary)
          : windows.length
            ? Math.max(...windows.map((w) => remainingOf(w)!))
            : null;
      return { account, windows, best };
    })
    .filter((c) => c.windows.length > 0);
  // 按展示剩余降序（并列时按账号名稳定）
  cards.sort((a, b) => (b.best ?? -1) - (a.best ?? -1) || (a.account.label || "").localeCompare(b.account.label || ""));
  return cards;
}

/** 圆环颜色按剩余百分比分级：≤10% 红、≤30% 黄、其余绿；无数据灰。 */
function ringColorFor(pct: number | null): string {
  if (pct == null) return "var(--tg-text-3)";
  if (pct <= 10) return "var(--tg-danger)";
  if (pct <= 30) return "var(--tg-warn)";
  return "var(--tg-success)";
}

/** 距 iso 的剩余时长（"5天 14小时"）；超过 30 天回退日期；已过显示「已重置」。 */
function relativeUntil(iso?: string): string {
  if (!iso) return "";
  const target = new Date(iso).getTime();
  if (!Number.isFinite(target)) return "";
  const diff = target - Date.now();
  if (diff <= 0) return "已重置";
  const day = 86_400_000, hour = 3_600_000, min = 60_000;
  if (diff >= 30 * day) {
    const d = new Date(iso);
    return `${d.getMonth() + 1}月${d.getDate()}日`;
  }
  if (diff >= day) return `${Math.floor(diff / day)}天 ${Math.floor((diff % day) / hour)}小时`;
  if (diff >= hour) return `${Math.floor(diff / hour)}小时 ${Math.floor((diff % hour) / min)}分`;
  if (diff >= min) return `${Math.floor(diff / min)}分`;
  return "即将重置";
}

/** 窗口短标签（切换胶囊用）：`5 小时额度 → 5小时`、`周额度 → 周额度`、`账户余额（CNY）→ 账户余额`。 */
function shortWindowLabel(w: QuotaWindowView): string {
  const label = (w.label || "").replace(/[（(].*?[)）]/g, "").replace(/\s+/g, "").trim();
  const stripped = label.endsWith("额度") ? label.slice(0, -2) : label;
  return stripped.length >= 2 ? stripped : (label || w.window_type || "额度"  );
}



/** 次级窗口配色 */
function subBarColor(label: string): string {
  const s = (label || "").toLowerCase();
  if (s.includes("周") || s.includes("week")) return "#ff9500";
  if (s.includes("月") || s.includes("month")) return "#bf5af2";
  if (s.includes("季") || s.includes("quarter")) return "#5e5ce6";
  if (s.includes("年") || s.includes("year") || s.includes("annual")) return "#ff375f";
  if (s.includes("余额") || s.includes("balance")) return "#64d2ff";
  if (s.includes("日") || s.includes("day")) return "#34c759";
  return "#0a84ff";
}

/** 「重置时间」绝对时间标签：MM月DD日 HH:MM；无值或不可解析回退空。 */
function resetClock(iso?: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${pad(d.getMonth() + 1)}月${pad(d.getDate())}日 ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** Codex 风格账号额度大卡（Apple Wallet 观感）：三视觉区 ——
 *  左：厂商品牌 + 等级胶囊 + 脱敏邮箱；中：大圆环（焦点）+ 「剩余额度」；右：当前限制周期
 *  （进度条 + 百分比 + 剩余时长 + 重置时间）；底：其余窗口（周/月额度等）整行展开。
 *  主窗口取 remaining_percent 最低（最受约束 = 「当前限制周期」）；厂商差异由 windows 自然驱动。 */
function QuotaWalletCard({
  account,
  windows,
  autoHint,
  stats,
}: {
  account: QuotaAccountView;
  windows: QuotaWindowView[];
  /** 自动轮播提示（右上角）：undefined = 单账号不显示；{text, paused} 由轮播容器下发。 */
  autoHint?: { text: string; paused: boolean };
  /** 按账号 TOKEN 统计（方案 B 2×2 统计格）；无网关流量的账号为 undefined → 显示「—」。 */
  stats?: AccountUsageStat;
}) {
  const { mode } = useAccountDisplay();
  // 主窗口 = 5 小时额度（window_key "primary"）；无主窗口时取最受约束的（remaining 最低）
  const primary = useMemo(() => {
    if (windows.length === 0) return undefined;
    const byKey = windows.find((w) => w.window_key === "primary" || /5\s*小\s*时/.test(w.label || ""));
    if (byKey) return byKey;
    const score = (w: QuotaWindowView) => {
      if (w.remaining_percent != null) return w.remaining_percent;
      if (w.remaining_value != null && w.limit_value && w.limit_value > 0) {
        return (w.remaining_value / w.limit_value) * 100;
      }
      return Number.POSITIVE_INFINITY;
    };
    return [...windows].sort((a, b) => score(a) - score(b))[0];
  }, [windows]);
  const pct = primary ? remainingOf(primary) : null;
  const pctClamp = pct != null ? Math.max(0, Math.min(100, pct)) : null;
  const color = ringColorFor(pctClamp);
  const identity = accountIdentity(account, mode);
  const { main: brand, plan } = accountBrandParts(account);
  // 其余窗口：单行进度条（周额度/月额度/账户余额…），最多展示 2 行 + 折叠计数
  const extra = useMemo(
    () => windows.filter((w) => w.window_key !== primary?.window_key),
    [windows, primary],
  );
  return (
    <div className="pg-tm-qw-card">
      {/* 顶行：品牌 + 实时（位置对齐参考图 banner 顶行） */}
      <div className="pg-tm-qw-top">
        <span className="pg-tm-qw-brand">
          <span className="pg-tm-qw-glyph" aria-hidden><ProviderLogo brand={brand} size={15} /></span>
          <strong>{brand}</strong>
        </span>
        <span className="pg-tm-qw-top-right">
          {autoHint && (
            <span className={`pg-tm-qw-auto${autoHint.paused ? " paused" : ""}`} aria-live="polite">
              <Clock size={10} />
              {autoHint.text}
            </span>
          )}
          <span className="pg-tm-qw-live"><i />实时 {formatClock(primary?.fetched_at) || "--"}</span>
        </span>
      </div>

      {/* 三列主体（对齐参考图）：左=计划+身份 · 中=圆环 · 右=窗口进度条 */}
      <div className="pg-tm-qw-body">
        <div className="pg-tm-qw-side">
          {plan && <em className="pg-tm-qw-plan">{plan}</em>}
          <span className="pg-tm-qw-ident">{identity}</span>
        </div>
        <span className="pg-tm-qw-ring">
          <RingProgress value={pctClamp ?? 0} size={52} stroke={4.5} color={color} track="rgba(118,118,128,.14)">
            <span className="pg-tm-qw-ring-inner">
              <b className="pg-tm-qw-pct" style={{ color }}>{pctClamp != null ? `${Math.round(pctClamp)}%` : "—"}</b>
              <em className="pg-tm-qw-ring-label">剩余额度</em>
            </span>
          </RingProgress>
        </span>
        <div className="pg-tm-qw-wins">
          {/* 主窗口：窗口名 / 进度条 / 剩余值+时长 */}
          <div className="pg-tm-qw-win">
            <span className="pg-tm-qw-win-name">{primary?.label || "额度"}</span>
            <span className="pg-tm-qw-bar"><i style={{ width: `${pctClamp ?? 0}%`, background: color }} /></span>
            <span className="pg-tm-qw-win-row">
              <b style={{ color }}>{primary ? quotaValueLabel(primary) : "—"}</b>
              <em>{primary?.resets_at ? `剩余 ${relativeUntil(primary.resets_at)}` : ""}</em>
            </span>
          </div>
          <span className="pg-tm-qw-reset">重置时间 {resetClock(primary?.resets_at) || "—"}</span>
          {/* 其余窗口：单行迷你进度条 */}
          {extra.slice(0, 2).map((w) => {
            const wp = remainingOf(w);
            const wc = wp != null ? Math.max(0, Math.min(100, wp)) : null;
            const wcol = ringColorFor(wc);
            return (
              <div key={w.window_key} className="pg-tm-qw-win sm">
                <span className="pg-tm-qw-win-name">{shortWindowLabel(w)}</span>
                <span className="pg-tm-qw-bar sm"><i style={{ width: `${wc ?? 0}%`, background: wcol }} /></span>
                <b className="pg-tm-qw-win-pct" style={{ color: wcol }}>{wc != null ? `${Math.round(wc)}%` : "—"}</b>    </div>
  );
}
)}
          {extra.length > 2 && <em className="pg-tm-qw-more">+{extra.length - 2} 更多窗口</em>}
        </div>
      </div>

      {/* 方案 B：2×2 TOKEN 统计格（今日/近7天/本月/累计；无网关流量显示 —） */}
      <div className="pg-tm-qw-stats" aria-label="账号 TOKEN 统计">
        <div className="pg-tm-qw-stat"><span>今日 TOKEN</span><b>{stats ? formatWan(stats.today_tokens) : "—"}</b></div>
        <div className="pg-tm-qw-stat"><span>近7天 TOKEN</span><b>{stats ? formatWan(stats.week_tokens) : "—"}</b></div>
        <div className="pg-tm-qw-stat"><span>本月 TOKEN</span><b>{stats ? formatWan(stats.month_tokens) : "—"}</b></div>
        <div className="pg-tm-qw-stat"><span>累计 TOKEN</span><b>{stats ? formatWan(stats.total_tokens) : "—"}</b></div>
      </div>
    </div>
    );
}



const QUOTA_SWIPE_THRESHOLD = 60;
/** 自动轮播间隔（秒）——对齐参考图「自动轮播中 3s」。 */
const QUOTA_FLIP_SECONDS = 3;

/** 主页额度：Apple Wallet 式左右滑动轮播（Codex 大卡）——一次只显示一个账号，
 *  支持左右滑动手势（触摸板/鼠标按住拖动），松手后按位移翻到相邻卡（不足阈值回弹），
 *  自动轮播带右上角倒计时提示（悬停/拖动时显示「已暂停」）；不做下钻。 */
function QuotaCarousel({
  cards,
  statsByAccount,
}: {
  cards: QuotaCard[];
  statsByAccount: Map<string, AccountUsageStat>;
}) {
  const [index, setIndex] = useState(0);
  const [dragX, setDragX] = useState<number | null>(null);
  const drag = useRef({ startX: 0, startIndex: 0, active: false });
  const hover = useRef(false);
  const [paused, setPaused] = useState(false);
  const [countdown, setCountdown] = useState(QUOTA_FLIP_SECONDS);
  const countdownRef = useRef(QUOTA_FLIP_SECONDS);
  const [reduced, setReduced] = useState(false);
  useEffect(() => {
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReduced(mq.matches);
    update();
    mq.addEventListener?.("change", update);
    return () => mq.removeEventListener?.("change", update);
  }, []);
  // 自动轮播 + 倒计时：1s tick；悬停/拖动时冻结计数（显示「已暂停」）；
  // 账号列表变化时回到第一张并重置倒计时
  useEffect(() => {
    setIndex(0);
    setDragX(null);
    setPaused(false);
    countdownRef.current = QUOTA_FLIP_SECONDS;
    setCountdown(QUOTA_FLIP_SECONDS);
    if (cards.length <= 1) return;
    const timer = window.setInterval(() => {
      if (hover.current || drag.current.active) return;
      countdownRef.current -= 1;
      if (countdownRef.current <= 0) {
        countdownRef.current = QUOTA_FLIP_SECONDS;
        setIndex((i) => (i + 1) % cards.length);
      }
      setCountdown(countdownRef.current);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [cards.length]);

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (cards.length <= 1) return;
    if ((e.target as HTMLElement).closest("button")) return;
    drag.current = { startX: e.clientX, startIndex: index, active: true };
    setPaused(true);
    e.currentTarget.setPointerCapture?.(e.pointerId);
  };
  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!drag.current.active) return;
    let offset = e.clientX - drag.current.startX;
    // 边界弹性：第一张向右拖 / 最后一张向左拖时阻尼 1/3
    const edge =
      (drag.current.startIndex === 0 && offset > 0) ||
      (drag.current.startIndex === cards.length - 1 && offset < 0);
    if (edge) offset /= 3;
    setDragX(offset);
  };
  const endDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!drag.current.active) return;
    const offset = e.clientX - drag.current.startX;
    const from = drag.current.startIndex;
    drag.current.active = false;
    setDragX(null);
    setPaused(false);
    // 手动翻页后重置倒计时
    countdownRef.current = QUOTA_FLIP_SECONDS;
    setCountdown(QUOTA_FLIP_SECONDS);
    if (offset <= -QUOTA_SWIPE_THRESHOLD) setIndex((from + 1) % cards.length);
    else if (offset >= QUOTA_SWIPE_THRESHOLD) setIndex((from - 1 + cards.length) % cards.length);
  };

  if (cards.length === 0) {
    return <EmptyState label="暂无实时额度数据" />;
  }
  const dragging = drag.current.active;
  const autoText = paused ? "已暂停" : `自动轮播中 ${countdown}s`;
  const autoHint = cards.length > 1 ? { text: autoText, paused } : undefined;
  return (
    <div
      className={`pg-tm-qw-carousel${dragging ? " dragging" : ""}`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onMouseEnter={() => { hover.current = true; setPaused(true); }}
      onMouseLeave={() => { hover.current = false; setPaused(false); }}
      aria-label="账号额度轮播（左右滑动切换，悬停暂停，自动轮播）"
    >
      <div
        className={`pg-tm-qw-track${reduced ? " no-motion" : ""}`}
        style={{ transform: `translateX(calc(${-index * 100}% + ${dragX ?? 0}px))` }}
      >
        {cards.map(({ account, windows }) => (
          <QuotaWalletCard
            key={account.account_id}
            account={account}
            windows={windows}
            autoHint={autoHint}
            stats={statsByAccount.get(account.account_id)}
          />        ))}
      </div>
    </div>
  );
}




const formatInt = (n: number): string => Math.round(n).toLocaleString("en-US");

function formatUsd(cost?: number | null): string | null {
  if (cost == null) return null;
  return cost >= 1 ? `$${cost.toFixed(2)}` : `$${cost.toFixed(4)}`;
}

function formatClock(iso?: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false }  );
}



/** 带秒的时钟（HH:MM:SS），用于底部状态栏「最后刷新时间」（与网关托盘一致）。 */
function formatClockSecs(iso?: string): string {
  if (!iso) return "--:--:--";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "--:--:--";
  return d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }  );
}



/** 紧凑时间：今天只显示 HH:MM，非今天显示 MM/DD HH:MM（对齐开源 Token Monitor 会话行）。 */
function formatSessionTime(iso?: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const pad = (v: number) => String(v).padStart(2, "0");
  const time = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate();
  return sameDay ? time : `${pad(d.getMonth() + 1)}/${pad(d.getDate())} ${time}`;
}

/** 会话 ID 短标签（对齐开源 Token Monitor）：`rollout-2026-08-08T…-<suffix>` → `<suffix>`；
 *  标准 ISO 时间戳前缀 → 空（外部 ID 不可读时只显示内部短码）；其余原样。 */
function sessionIdLabel(id?: string): string {
  const raw = String(id ?? "").trim();
  if (!raw) return "";
  const rollout = raw.match(/^rollout-\d{4}-\d{2}-\d{2}T\d{2}[:-]\d{2}[:-]\d{2}-(.+)$/);
  if (rollout) return rollout[1];
  if (/^\d{4}-\d{2}-\d{2}T\d{2}[:-]\d{2}/.test(raw)) return "";
  return raw;
}

/** 活跃时间：与桌面端 Overview 一致，取后端 trend.active_time_ms（tokscale 权威口径）。 */
function activeTimeLabel(ms?: number): string {
  if (!ms || ms <= 0) return "—";
  const totalMin = Math.round(ms / 60000);
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

// TM_CONT_1

/**
 * Token Monitor 托盘视图（view=token-monitor）。
 * 外壳与网关托盘统一（Liquid Glass + 固定 380×720 + 底部 6 图标操作栏）；
 * 首页 额度/模型/活动/趋势 区块可点击下钻，保留 工具/模型/会话/趋势 明细视图。
 * Logo 点击轮询切换 Hero 副行：费用 → 分速率 → 秒速率（每次只显示一个）；
 * Hero 右上角为手动刷新。
 */

// Logo 点击轮询的速率模式：费用 / 分速率 / 秒速率，每次只显示一个
// （对齐开源版 Logo 点击显示速率，但不再把 tok/min 与 tok/s 并列展示）。
type RateMode = "cost" | "min" | "sec";

const RATE_CYCLE: RateMode[] = ["cost", "min", "sec"];

export default function TokenMonitorTrayCard() {
  const [range, setRange] = useState<Range>("day");
  const [view, setView] = useState<TmView>("home");
  // 速率展示模式：点击 Logo 在 费用 → 分速率 → 秒速率 间轮询
  const [rateMode, setRateMode] = useState<RateMode>("cost");
  // 手动刷新：idle → refreshing（旋转）→ done（勾）→ idle
  const [refreshing, setRefreshing] = useState(false);
  const [refreshDone, setRefreshDone] = useState(false);
  const doneTimer = useRef<number | null>(null);
  const { data: appMode } = useAppMode();
  const gatewayMode = appMode?.selected && appMode.mode === "gateway";
  const [gatewayRunning, setGatewayRunning] = useState(false);
  const [toggling, setToggling] = useState(false);
  const queryClient = useQueryClient();
  // 事件驱动实时刷新：采集落库 → `token-monitor:usage-delta` → 立即失效用量查询（TOKENS 秒级变化）。
  // 下方 10s invalidate 周期兜底保留，事件丢失时仍能刷新。
  useTokenMonitorRealtime();
  const { data: snapshot } = useTokenMonitorSnapshot(range);
  const { data: tools } = useToolUsage(range);
  const { data: models } = useModelUsage(range);
  const { data: trend, isLoading: trendLoading } = useTrend(range);
  const { data: quotaAccounts } = useQuotaAccounts();
  const { data: sessions } = useActiveSessions(range);
  const { data: collectors } = useCollectorStatus();
  const { data: rate } = useTokenRate(rateMode !== "cost");

  // Gateway 完整模式下，Monitor 托盘仍提供可逆的「切换到 Gateway 托盘」和网关启停；
  // Monitor 专注模式不会渲染这些控制，也不会调用 Gateway API。
  useEffect(() => {
    if (!gatewayMode) return;
    let alive = true;
    const sync = () => {
      void getProxyStatus()
        .then((status) => { if (alive) setGatewayRunning(status.running); })
        .catch(() => undefined);
    };
    sync();
    const timer = window.setInterval(sync, 10_000);
    return () => { alive = false; window.clearInterval(timer); };
  }, [gatewayMode]);

  const toggleGateway = useCallback(async () => {
    if (!gatewayMode || toggling) return;
    setToggling(true);
    try {
      if (gatewayRunning) await stopProxy();
      else await startProxy();
      const status = await getProxyStatus();
      setGatewayRunning(status.running);
    } catch {
      // 后端错误由托盘状态轮询兜底，不阻塞 Monitor 数据展示。
    } finally {
      setToggling(false);
    }
  }, [gatewayMode, toggling, gatewayRunning]);

  // 自动刷新：每 10s 失效 tm-* 查询并重取（对齐开源 Token Monitor 实时刷新；
  // Tokens 增长时 Hero 数字滚动，活动/趋势数据随之更新）。
  // 注意：tm-trend 排除在外——它后端可能跑 ~30s 的 tokscale graph 子进程，
  // 由 useTrend 自己的 90s refetchInterval 接管，避免反复触发重算。
  useEffect(() => {
    const timer = window.setInterval(() => {
      void queryClient.invalidateQueries({
        predicate: (q) => {
          const key = String(q.queryKey[0] ?? "");
          return key.startsWith("tm-") && key !== "tm-trend";
        },
      });
    }, 10_000);
    return () => window.clearInterval(timer);
  }, [queryClient]);

  useEffect(() => () => {
    if (doneTimer.current) window.clearTimeout(doneTimer.current);
  }, []);

  // 手动刷新：强制全量重扫 + 让所有 tm-* 查询立即重取。
  const manualRefresh = useCallback(async () => {
    if (refreshing) return;
    setRefreshing(true);
    try {
      await refreshTokenMonitor();
      await queryClient.invalidateQueries({ predicate: (q) => String(q.queryKey[0] ?? "").startsWith("tm-") });
      setRefreshDone(true);
      if (doneTimer.current) window.clearTimeout(doneTimer.current);
      doneTimer.current = window.setTimeout(() => setRefreshDone(false), 1400);
    } catch {
      // 静默：后端不可用时保持现状
    } finally {
      setRefreshing(false);
    }
  }, [refreshing, queryClient]);

  const usage = snapshot?.usage;
  const heroCost = formatUsd(usage?.cost_amount);

  // 数据是否陈旧：以上次采集完成时间（last_collected_at）为准，超 2 分钟 → 刷新按钮琥珀色脉冲提示。
  const stale = useMemo(() => {
    const times = (collectors ?? [])
      .map((c) => c.last_collected_at ? new Date(c.last_collected_at).getTime() : 0)
      .filter((t) => Number.isFinite(t) && t > 0);
    if (times.length === 0) return false;
    return Date.now() - Math.max(...times) > 120_000;
  }, [collectors]);

  // 趋势日序列升序（后端 DESC 返回）。首页「活动趋势」卡不随范围筛选——始终展示全量历史
  //（还原：范围选择只影响趋势明细页的展示口径，见 TrendView）。
  const dailyAsc = useMemo(() => {
    return [...(trend?.daily ?? [])].sort((a, b) => a.date.localeCompare(b.date));
  }, [trend]);

  // 全量统计（首页活动趋势卡元信息：活跃 N 天 / 峰值 X）。
  // 直接使用后端 trend 权威汇总（active_days / streak_days / peak_day，与开源 history.js
  // 逐值一致），不在前端从日序列重算——DB 回退路径的日序列有 LIMIT 400 窗口，重算会把
  // 更早的活跃日漏掉，导致「活跃天数」比开源少 1 天（95 vs 96）。
  const overallStats = useMemo(() => ({
    activeDays: trend?.active_days ?? 0,
    streakDays: trend?.streak_days ?? 0,
    peak: trend?.peak_day
      ? { date: trend.peak_day.date, tokens: trend.peak_day.tokens, requests: trend.peak_day.requests }
      : null,
  }), [trend]);

  return (
    <main className="pg-tray-card pg-tray-glass pg-tm-glass">
      <header className="pg-tg-header">
        <div className="pg-tg-brand">
          <button
            type="button"
            className={`pg-tg-logo-btn ${rateMode !== "cost" ? "rate-on" : ""}`}
            aria-label={rateMode === "cost" ? "显示分速率" : rateMode === "min" ? "显示秒速率" : "显示费用"}
            title={rateMode === "cost" ? "点击显示分速率" : rateMode === "min" ? "点击显示秒速率" : "点击显示费用"}
            onClick={() => setRateMode((m) => RATE_CYCLE[(RATE_CYCLE.indexOf(m) + 1) % RATE_CYCLE.length])}
          >
            <img src="/poolgate-icon.png" alt="PoolGate" />
          </button>
          <div className="pg-tg-title-group">
            <div className="pg-tg-title-row">
              <h1>PoolGate</h1>
              <span className={`pg-tg-online ${gatewayMode ? (gatewayRunning ? "on" : "") : "on"}`}>
                <i />{gatewayMode ? (gatewayRunning ? "网关在线" : "网关离线") : "Monitor"}
              </span>
            </div>
            <span className="pg-tg-subtitle">{gatewayMode ? "本地 Agent 用量监控 · Gateway 完整模式" : "本地 Agent 用量监控 · 网关已隐藏"}</span>
          </div>
        </div>
        <div className="pg-tg-segmented" role="tablist">
          {RANGE_TABS.map((t) => (
            <button key={t.key} role="tab" className={range === t.key ? "active" : ""} onClick={() => setRange(t.key)}>
              {t.label}
            </button>
          ))}
        </div>
      </header>

      {/* 今日 Tokens Hero（固定在顶部，不随内容滚动）*/}
      <section className="pg-tm-merged-section">
        <TokensQuotaMerged
          range={range}
          rangeLabel={RANGE_LABEL[range]}
          totalTokens={usage?.total_tokens ?? 0}
          costUsd={usage?.cost_amount ?? null}
          costCny={null}
          updatedAt={snapshot?.updated_at}
          refreshing={refreshing}
          refreshDone={refreshDone}
          stale={stale}
          onRefresh={() => void manualRefresh()}
          onOpenDetail={() => setView("quota")}
          accounts={quotaAccounts ?? []}
          heroOnly
          rateMode={rateMode}
          rate={rate}
        />
      </section>

      <div className="pg-tg-scroll pg-tm-body">
        {view !== "home" && (
          <div className="pg-tm-detail-bar">
            <button type="button" className="pg-tm-back" onClick={() => setView("home")}>
              <ArrowLeft size={14} /><span>主页</span>
            </button>
            <div className="pg-tm-detail-seg" role="tablist">
              {DETAIL_TABS.map(({ key, label, Icon }) => (
                <button key={key} role="tab" className={view === key ? "active" : ""} title={label} aria-label={label} onClick={() => setView(key)}>
                  <Icon size={13} />
                </button>
              ))}
            </div>
          </div>
        )}
        {view === "home" && (
          <HomeView
            models={models ?? []}
            trendLoading={trendLoading}
            dailyAsc={dailyAsc}
            quotaAccounts={quotaAccounts ?? []}
            quotaProps={{
              range, rangeLabel: RANGE_LABEL[range],
              totalTokens: usage?.total_tokens ?? 0,
              costUsd: usage?.cost_amount ?? null, costCny: null,
              updatedAt: snapshot?.updated_at,
              refreshing, refreshDone, stale,
              onRefresh: () => void manualRefresh(),
            }}
            overallStats={overallStats}
            onNavigate={setView}
          />
        )}
        {view === "quota" && <QuotaView accounts={quotaAccounts ?? []} />}
        {view === "tools" && <ToolsView tools={tools ?? []} />}
        {view === "models" && <ModelsView models={models ?? []} />}
        {view === "sessions" && <SessionsView sessions={sessions ?? []} />}
        {view === "trend" && <TrendView trend={trend} trendLoading={trendLoading} dailyAsc={dailyAsc} range={range} />}
      </div>

      <footer className="pg-tg-footer">
        {/* 底部状态栏：主行显示最后刷新时间（与网关托盘一致），副行保留范围标签；
             Hero 已展示 Tokens/费用，此处不再重复显示 */}
        <div className="pg-tg-footer-status">
          <span>{formatClockSecs(snapshot?.updated_at)} 更新</span>
          <small>Token Monitor · {RANGE_LABEL[range]}</small>
        </div>
        <div className="pg-tg-actions">
          {gatewayMode ? (
            <>
              <button className="pg-tg-icon-btn active" onClick={() => { window.location.search = "view=tray"; }} title="切换到 Gateway 托盘" aria-label="切换到 Gateway 托盘">
                <ServerCog size={17} /><span>Gateway</span>
              </button>
              <button className="pg-tg-icon-btn" onClick={() => void openPoolGateFromTray("tokenmonitor", undefined, "settings").catch(() => undefined)} title="Token Monitor 与模式设置" aria-label="Token Monitor 与模式设置">
                <Settings2 size={17} /><span>配置</span>
              </button>
              <button className={`pg-tg-icon-btn power ${gatewayRunning ? "running" : ""}`} onClick={() => void toggleGateway()} disabled={toggling} title={gatewayRunning ? "停止网关" : "启动网关"} aria-label={gatewayRunning ? "停止网关" : "启动网关"}>
                <Power size={17} /><span>{gatewayRunning ? "停止" : "启动"}</span>
              </button>
            </>
          ) : (
            <>
              <button className="pg-tg-icon-btn active" onClick={() => void openPoolGateFromTray("tokenmonitor").catch(() => undefined)} title="打开 Monitor 仪表盘" aria-label="打开 Monitor 仪表盘">
                <Coins size={17} /><span>Monitor</span>
              </button>
              <button className="pg-tg-icon-btn" onClick={() => void openPoolGateFromTray("tokenmonitor", undefined, "settings").catch(() => undefined)} title="打开 Monitor 与模式设置" aria-label="打开 Monitor 与模式设置">
                <Settings2 size={17} /><span>配置</span>
              </button>
            </>
          )}
          <button className="pg-tg-icon-btn danger" onClick={() => void quitPoolGateFromTray()} title="退出 PoolGate" aria-label="退出 PoolGate">
            <LogOut size={17} /><span>退出</span>
          </button>
        </div>
      </footer>
    </main>
    );
}


// TM_CONT_2

// ───────────────────────── 主页（各区块 Card 化，可点击下钻） ─────────────────────────
function HomeView({
  models, trendLoading, dailyAsc, overallStats, onNavigate, quotaAccounts, quotaProps,
}: {
  models: ModelUsageRow[];
  trendLoading: boolean;
  dailyAsc: HeatmapDay[];
  quotaAccounts?: QuotaAccountView[];
  quotaProps: {
    range: TokenRange; rangeLabel: string; totalTokens: number;
    costUsd: number | null; costCny: number | null; updatedAt?: string;
    refreshing?: boolean; refreshDone?: boolean; stale?: boolean;
    onRefresh?: () => void;
  };
  overallStats: { activeDays: number; peak: HeatmapDay | null; streakDays: number };
  onNavigate: (view: TmView) => void;
}) {
  const [atView, setAtView] = useState<"activity" | "trend">("activity");
  const peakLabel = overallStats.peak ? formatTokens(overallStats.peak.tokens) : "—";
  const topModels = models.slice(0, 4);

  return (
    <>
      {/* 额度卡：标题行（额度 + 自动轮播倒计时 + 箭头）+ 左右轮播卡主体（与模型使用/活动趋势卡片同构） */}
      <section className="pg-tg-card pg-tm-quota-nav-card">
        <TokensQuotaMerged
          {...quotaProps}
          accounts={quotaAccounts ?? []}
          cardOnly
          onOpenDetail={() => onNavigate("quota")}
        />
      </section>

      <section
        className="pg-tg-card pg-tm-clickable-card"
        role="button"
        tabIndex={0}
        onClick={() => onNavigate("models")}
        onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onNavigate("models"); } }}
        title="查看模型使用明细"
        aria-label="查看模型使用明细"
      >
        <div className="pg-tm-nav">
          <span className="pg-tm-nav-title"><Cpu size={14} /><span>模型使用</span><em className="pg-tm-nav-count">({topModels.length})</em></span>
          <ChevronRight size={15} className="pg-tm-nav-arrow" />
        </div>
        {topModels.length === 0 ? <EmptyState label="暂无模型用量" /> : (
          <div className="pg-tm-model-lines">
            {topModels.map((m, i) => (
              <div key={m.model} className="pg-tm-model-line">
                <span className="pg-tm-glyph"><ProviderLogo model={m.model} size={17} /></span>
                <span className="pg-tm-model-name">{m.model}</span>
                <b className="pg-tm-model-tok">{formatTokens(m.total_tokens)}</b>
                <span className="pg-tm-model-pct">{Math.round(m.share_percent)}%</span>
              </div>
            ))}
          </div>
        )}
      </section>

      {/* 活动/趋势：参考网关托盘「请求/Tokens」切换，同一张卡内切换查看；
          卡拉伸填满剩余高度（与底部操作栏无留白）；点击小方块/图表跳转趋势明细 */}
      <section className="pg-tg-card pg-tm-at-card">
        <div className="pg-tg-card-head pg-tm-at-head">
          <button type="button" className="pg-tm-at-title" onClick={() => onNavigate("trend")} title="查看趋势明细" aria-label="查看趋势明细">
            <strong>活动趋势</strong>
            <ChevronRight size={13} className="pg-tm-at-chevron" />
          </button>
          <span className="pg-tm-at-meta">
            {atView === "activity" ? `活跃 ${overallStats.activeDays} 天` : `峰值 ${peakLabel}`}
          </span>
          <div className="pg-tg-toggle" role="tablist" aria-label="活动趋势视图">
            <button role="tab" aria-selected={atView === "activity"} className={atView === "activity" ? "active" : ""} onClick={() => setAtView("activity")}>活动</button>
            <button role="tab" aria-selected={atView === "trend"} className={atView === "trend" ? "active" : ""} onClick={() => setAtView("trend")}>趋势</button>
          </div>
        </div>
        {atView === "activity" ? (
          <div
            className="pg-tm-at-zone"
            role="button"
            tabIndex={0}
            onClick={() => onNavigate("trend")}
            onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onNavigate("trend"); } }}
            title="查看趋势明细"
            aria-label="查看趋势明细"
          >
            <div className="pg-tm-heatmap">
              {/* 数据优先：有缓存/placeholder 数据时立即展示，加载中只在真正无数据时出现 */}
              {dailyAsc.length === 0 ? (
                <EmptyState label={trendLoading ? "加载中…" : "暂无活动记录"} />
              ) : (
                <ActivityHeatmap data={dailyAsc} compact cellSize={10} gap={2} rounded={2} showLegend={false} align="end" windowEnd={todayKey()} windowDays={365} fill minCellSize={14} maxCellSize={15} onCellClick={() => onNavigate("trend")} />
              )}
            </div>
          </div>
        ) : (
          <div
            className="pg-tm-at-zone"
            role="button"
            tabIndex={0}
            onClick={() => onNavigate("trend")}
            onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onNavigate("trend"); } }}
            title="查看趋势明细"
            aria-label="查看趋势明细"
          >
            {dailyAsc.length === 0 ? (
              <EmptyState label={trendLoading ? "加载中…" : "暂无趋势数据"} />
            ) : (
              <>
                <div className="pg-tm-spark">
                  <Sparkline values={dailyAsc.map((d) => d.tokens)} area stroke="var(--tg-primary)" fill="rgba(0,122,255,.12)" />
                </div>
                <div className="pg-tm-trend-axis">
                  <span>{dailyAsc[0] ? dailyAsc[0].date.slice(5).replace("-", "/") : ""}</span>
                  <span>今日 {dailyAsc.length ? dailyAsc[dailyAsc.length - 1].date.slice(5).replace("-", "/") : ""}</span>
                </div>
              </>
            )}
          </div>
        )}
      </section>
    </>
    );
}



/** 空状态：48×48 图标 + 40% 透明（设计规范 §12，禁止空白）。 */
function EmptyState({ label }: { label: string }) {
  return (
    <div className="pg-tm-empty-state">
      <Coins size={30} />
      <span>{label}</span>
    </div>
    );
}


// TM_CONT_3

// ───────────────────────── 额度明细（与轮播卡片同口径） ─────────────────────────
// 展示全部窗口：有剩余百分比 → 进度条 + %；无百分比但有数值（如网关账号未配置上限）
// → 展示剩余值/单位；完全无数据才空态。卡片样式与首页轮播一致，增量排列无需滚动。
function QuotaView({ accounts }: { accounts: QuotaAccountView[] }) {
  const cards = quotaCards(accounts);
  if (cards.length === 0) {
    return <div className="pg-tg-card"><EmptyState label="暂无实时额度数据" /></div>;
  }
  return (
    <div className="pg-tm-merged">
      <div className="pg-tm-merged-section-label">
        <span className="pg-tm-nav-title"><Coins size={14} /><span>额度</span><em className="pg-tm-nav-count">({cards.length} 个账号)</em></span>
      </div>
      {cards.map(({ account, windows }) => (
        <div key={account.account_id} className="pg-tm-merged-quota-card pg-tm-merged-quota-card--static">
          <QuotaAccountCard account={account} windows={windows} />
        </div>
      ))}
    </div>
    );
}



/** 额度页账号卡（与轮播卡片完全同口径）：三列 grid = 左额环 | 竖杆 | 右窗口 */
function QuotaAccountCard({ account, windows }: { account: QuotaAccountView; windows: QuotaWindowView[] }) {
  const { mode } = useAccountDisplay();
  const primary = pickPrimary(windows) ?? windows[0];
  // 预付费余额（DeepSeek 等）：圆环显示金额 + 阈值色，不走百分比/重置模型
  const isBalance = primary != null && isBalanceWindow(primary);
  const balanceAmt = isBalance ? (primary?.remaining_value ?? null) : null;
  const balCode = isBalance ? currencyOf(primary!) : "USD";
  const balTone = balanceTone(balanceAmt, balCode);
  const extras = windows.filter((w) => w.window_key !== primary?.window_key && !isBalanceWindow(w));
  const pct = !isBalance && primary ? remainingOf(primary) : null;
  const pctClamp = pct != null ? Math.max(0, Math.min(100, pct)) : null;
  const identity = accountIdentity(account, mode);
  const { main: brand, plan } = accountBrandParts(account);
  const remainingStr = primary?.resets_at ? relativeUntil(primary.resets_at) : "—";
  const resetStr = resetClock(primary?.resets_at) || "—";
  const source = providerLabel(account.provider_id, account.provider_label);

  // 会员等级胶囊：从 plan_name 中识别 Free/Plus/Pro/Max 等等级（带颜色）
  const tier = accountTier(account.plan_name);

  // 次级窗口排序
  const sortedExtras = [...extras].sort((a, b) => {
    const order = (label: string): number => {
      const s = (label || "").toLowerCase();
      if (s.includes("周") || s.includes("week")) return 1;
      if (s.includes("月") || s.includes("month")) return 2;
      if (s.includes("季") || s.includes("quarter")) return 3;
      if (s.includes("年") || s.includes("year") || s.includes("annual")) return 4;
      if (s.includes("余额") || s.includes("balance")) return 5;
      if (s.includes("日") || s.includes("day")) return 0;
      return 99;
    };
    return order(a.label || "") - order(b.label || "");
  });

  const ringStroke = isBalance
    ? balTone.color
    : pctClamp != null && pctClamp <= 10
      ? "#ff453a"
      : pctClamp != null && pctClamp <= 30
        ? "#ff9500"
        : "#34c759";
  const ringR = 33;
  const ringC = 2 * Math.PI * ringR;
  const ringDash = isBalance
    ? balanceAmt != null && balanceAmt > 0 ? ringC : 0
    : pctClamp != null ? (pctClamp / 100) * ringC : 0;
  const planLine = (plan || identity) ? `${plan ? `${plan} · ` : ""}${identity || account.label || ""}` : "";
  const subline = planLine || (source !== "账号" ? source : "");

  return (
    <div className="pg-tm-merged-card is-active">
      {/* 顶部状态：brand + 会员徽标 + 副行 + 来源 */}
      <div className="pg-tm-merged-card-status">
        <div className="pg-tm-merged-card-headline">
          <div className="pg-tm-merged-card-brand-row">
            <strong className="pg-tm-merged-card-brand">{brand}</strong>
            {tier && (
              <span className="pg-tm-merged-card-tier" style={{ background: tier.color }}>
                {tier.label}
              </span>
            )}
          </div>
          {subline ? <em className="pg-tm-merged-card-subline">{subline}</em> : null}
        </div>
      </div>

      {/* 左列：额度环（余额账号显示金额 + 阈值色） */}
      <div className="pg-tm-merged-acc" role="img" aria-label={isBalance ? `账户余额 ${formatBalance(balanceAmt, balCode)}` : `账户额度 ${pctClamp != null ? `${Math.round(pctClamp)}%` : "未知"}`}>
        <svg width="76" height="76" viewBox="0 0 76 76" aria-hidden>
          <circle cx="38" cy="38" r={ringR} fill="none" stroke="rgba(118,118,128,.18)" strokeWidth="5" />
          {(isBalance ? balanceAmt != null : pctClamp != null) && (
            <circle cx="38" cy="38" r={ringR} fill="none" stroke={ringStroke} strokeWidth="5" strokeLinecap="round" strokeDasharray={`${ringDash} ${ringC}`} transform="rotate(-90 38 38)" style={{ filter: `drop-shadow(0 0 5px ${ringStroke}66)` }} />
          )}
        </svg>
        <div className="pg-tm-merged-acc-center">
          {isBalance ? (
            <strong className="pg-tm-merged-acc-ring-pct is-amt" style={{ color: ringStroke }}>{formatBalance(balanceAmt, balCode)}</strong>
          ) : (
            <strong className="pg-tm-merged-acc-ring-pct" style={{ color: ringStroke }}>{pctClamp != null ? `${Math.round(pctClamp)}%` : "—"}</strong>
          )}
          <em className="pg-tm-merged-acc-ring-label">{isBalance ? "账户余额" : "剩余额度"}</em>
        </div>
      </div>

      {/* 竖杆分割 */}
      <div className="pg-tm-merged-divider-col" aria-hidden />

      {/* 右列：窗口进度条 */}
      <div className="pg-tm-merged-wins">
        <div className="pg-tm-merged-win primary">
          <div className="pg-tm-merged-win-head">
            <span className="pg-tm-merged-win-icon" aria-hidden><Clock size={11} /></span>
            <span className="pg-tm-merged-win-name">{primary?.label || "5 小时额度"}</span>
            <b className="pg-tm-merged-win-pct" style={isBalance ? { color: ringStroke } : undefined}>{isBalance ? formatBalance(balanceAmt, balCode) : pctClamp != null ? `${Math.round(pctClamp)}%` : "—"}</b>
          </div>
          <span className="pg-tm-merged-bar"><i style={isBalance ? { width: balanceAmt != null && balanceAmt > 0 ? "100%" : "0%", background: balTone.color } : { width: `${pctClamp ?? 0}%` }} /></span>
          {isBalance ? (
            <span className="pg-tm-merged-win-foot">
              <em style={{ color: balTone.color }}>余额 <b style={{ color: balTone.color }}>{balTone.label}</b></em>
            </span>
          ) : (
            primary?.resets_at && (
              <span className="pg-tm-merged-win-foot">
                <em><ArrowLeft size={9} className="rotate" />剩余 <b>{remainingStr}</b></em>
                <span className="dot-sep" />
                <em className="reset">重置 <b>{resetStr}</b></em>
              </span>
            )
          )}
        </div>
        {sortedExtras.length > 0 && <div className="pg-tm-merged-divider" />}
        {sortedExtras.slice(0, 3).map((w, i) => {
          const wp = remainingOf(w);
          const wc = wp != null ? Math.max(0, Math.min(100, wp)) : null;
          const wcColor = i === 0 ? "#bf5af2" : i === 1 ? "#ff9500" : subBarColor(w.label || "");
          return (
            <div key={w.window_key} className="pg-tm-merged-sub">
              <span className="pg-tm-merged-sub-swatch" style={{ background: wcColor }} />
              <span className="pg-tm-merged-sub-name">{shortWindowLabel(w)}额度</span>
              <span className="pg-tm-merged-sub-track"><i style={{ width: `${wc ?? 0}%`, background: wcColor }} /></span>
              <b className="pg-tm-merged-sub-pct" style={{ color: wcColor }}>{wc != null ? `${Math.round(wc)}%` : "—"}</b>
            </div>
          );
        })}
        {sortedExtras.length > 3 && <em className="pg-tm-merged-more">+{sortedExtras.length - 3} 更多</em>}
      </div>
    </div>
    );
}



/** 额度窗口的剩余值标签：百分比窗口 → `66%`；金额/数值窗口 → `$97.60` / `12.4T`；
 *  只有使用/上限 → `12.4 / 110`；全无 → `无数据`。 */
function quotaValueLabel(window: QuotaWindowView): string {
  if (window.unit === "percent" && window.remaining_percent != null) {
    return `${Math.round(window.remaining_percent)}%`;
  }
  if (window.remaining_value != null) {
    const v = formatQuotaValue(window.remaining_value);
    return window.unit === "currency" ? `$${v}` : `${v}${unitShort(window.unit)}`;
  }
  if (window.remaining_percent != null) return `${Math.round(window.remaining_percent)}%`;
  if (window.used_value != null || window.limit_value != null) {
    return `${formatQuotaValue(window.used_value ?? 0)} / ${formatQuotaValue(window.limit_value) ?? "∞"}`;
  }
  return "无数据";
}

function formatQuotaValue(v?: number | null): string {
  if (v == null) return "∞";
  return v >= 1000 ? Math.round(v).toLocaleString("en-US") : v >= 1 ? v.toFixed(2) : String(v  );
}



function unitShort(unit: string): string {
  switch (unit) {
    case "tokens": return "T";
    case "requests": return "req";
    case "credits": return "cr";
    case "currency": return "$";
    default: return "";
  }
}

// ───────────────────────── 工具（TOKENS 用量：可展开明细；监控配置在桌面端） ─────────────────────────
function ToolsView({ tools }: { tools: ToolUsageRow[] }) {
  const [expanded, setExpanded] = useState<string | null>(tools[0]?.tool_id ?? null);
  if (tools.length === 0) return <div className="pg-tg-card"><EmptyState label="暂无工具用量" /></div>;
  return (
    <div className="pg-tm-list">
      {tools.map((t, i) => {
        const color = PALETTE[i % PALETTE.length];
        const cost = formatUsd(t.cost_amount);
        const inputTotal = t.cache_tokens + t.input_tokens;
        const hitPct = inputTotal > 0 ? Math.round((t.cache_tokens / inputTotal) * 100) : 0;
        const missPct = inputTotal > 0 ? Math.round((t.input_tokens / inputTotal) * 100) : 0;
        const open = expanded === t.tool_id;
        return (
          <div key={t.tool_id} className="pg-tg-card pg-tm-tool">
            <button className="pg-tm-tool-head" onClick={() => setExpanded(open ? null : t.tool_id)}>
              <span className="pg-tm-glyph"><ToolLogo toolId={t.tool_id} displayName={t.display_name} size={18} /></span>
              <span className="pg-tm-tool-name">{toolDisplay(t.tool_id, t.display_name)}</span>
              <span className="pg-tm-tool-right">
                <b>{formatInt(t.total_tokens)}</b>
                <em>{cost ?? "—"}</em>
              </span>
            </button>
            <div className="pg-tm-bar"><i style={{ width: `${Math.max(2, t.share_percent)}%`, background: color }} /></div>
            {open && (
              <div className="pg-tm-sub">
                <div className="pg-tm-sub-row"><span>输入（缓存命中） <em>{hitPct}%</em></span><b>{formatInt(t.cache_tokens)}</b></div>
                <div className="pg-tm-sub-row"><span>输入（缓存未命中） <em>{missPct}%</em></span><b>{formatInt(t.input_tokens)}</b></div>
                <div className="pg-tm-sub-row"><span>输出</span><b>{formatInt(t.output_tokens)}</b></div>
              </div>
            )}
          </div>
        );
      })}
    </div>
    );
}



// ───────────────────────── 模型（可展开/折叠） ─────────────────────────
function ModelsView({ models }: { models: ModelUsageRow[] }) {
  const [expanded, setExpanded] = useState<string | null>(null);
  if (models.length === 0) return <div className="pg-tg-card"><EmptyState label="暂无模型用量" /></div>;
  return (
    <div className="pg-tm-list">
      {models.map((m, i) => {
        const color = PALETTE[i % PALETTE.length];
        const cost = formatUsd(m.cost_amount);
        const inputTotal = m.cache_tokens + m.input_tokens;
        const hitPct = inputTotal > 0 ? Math.round((m.cache_tokens / inputTotal) * 100) : 0;
        const missPct = inputTotal > 0 ? Math.round((m.input_tokens / inputTotal) * 100) : 0;
        const open = expanded === m.model;
        return (
          <div key={m.model} className="pg-tg-card pg-tm-tool">
            <button className="pg-tm-tool-head" onClick={() => setExpanded(open ? null : m.model)}>
              <span className="pg-tm-glyph"><ProviderLogo model={m.model} size={17} /></span>
              <span className="pg-tm-tool-name">{m.model}</span>
              <span className="pg-tm-tool-right">
                <b>{formatInt(m.total_tokens)}</b>
                <em>{cost ?? "—"}</em>
              </span>
            </button>
            <div className="pg-tm-bar"><i style={{ width: `${Math.max(2, m.share_percent)}%`, background: color }} /></div>
            {open && (
              <div className="pg-tm-sub">
                <div className="pg-tm-sub-row"><span>输入（缓存命中） <em>{hitPct}%</em></span><b>{formatInt(m.cache_tokens)}</b></div>
                <div className="pg-tm-sub-row"><span>输入（缓存未命中） <em>{missPct}%</em></span><b>{formatInt(m.input_tokens)}</b></div>
                <div className="pg-tm-sub-row"><span>输出</span><b>{formatInt(m.output_tokens)}</b></div>
              </div>
            )}
          </div>
        );
      })}
    </div>
    );
}



// ───────────────────────── 会话（列表 + 逐轮明细下钻，对齐开源 Token Monitor） ─────────────────────────
// 开源版点开会话展示 transcript 解析出的「prompt → turns」交换明细（sessionDetail.js）；
// PoolGate 隐私红线不读正文，等价替代为 usage_event 的逐轮元数据时间线（时间/模型/token/成本）。
function SessionsView({ sessions }: { sessions: SessionSummary[] }) {
  const [expanded, setExpanded] = useState<string | null>(null);
  if (sessions.length === 0) return <div className="pg-tg-card"><EmptyState label="暂无会话" /></div>;
  return (
    <div className="pg-tm-list">
      {sessions.slice(0, 20).map((s, i) => {
        const color = PALETTE[i % PALETTE.length];
        const singleModel = s.model_set.length === 1 ? s.model_set[0] : null;
        const modelLabel = singleModel ?? (s.model_set.length > 1 ? `${s.model_set.length} models` : "—");
        const cost = formatUsd(s.cost_amount);
        const msgLabel = formatMessageCount(s.message_count);
        const multiModel = s.model_set.length > 1;
        const open = expanded === s.session_id;
        return (
          <div key={s.session_id} className="pg-tg-card pg-tm-session">
            <button className="pg-tm-session-head" onClick={() => setExpanded(open ? null : s.session_id)}>
              <div className="pg-tm-session-row1">
                <span className="pg-tm-glyph"><ToolLogo toolId={s.tool_id} size={18} /></span>
                {singleModel && <ProviderLogo model={singleModel} size={14} />}
                <span className="pg-tm-session-title">{toolDisplay(s.tool_id)} · {modelLabel}</span>
                <b>{formatInt(s.total_tokens)}</b>
              </div>
              <div className="pg-tm-session-row2">
                <span>
                  {formatSessionTime(s.last_active_at) || "--:--"}
                  {msgLabel && (
                    <>
                      {" · "}
                      {/* 多模型会话：消息数为各模型分组之和（悬停提示口径，对齐开源版） */}
                      <span title={multiModel ? MULTI_MODEL_MSG_HINT : undefined}>{msgLabel}</span>
                    </>
                  )}
                </span>
                <em>{cost ?? "—"}</em>
              </div>
              <div className="pg-tm-session-row3">
                <code>{sessionIdLabel(s.external_session_id || s.session_id) || s.session_id}</code>
                <ChevronDown size={13} className={open ? "flip" : ""} />
              </div>
            </button>
            {open && <SessionDetailPanel session={s} />}
          </div>
        );
      })}
    </div>
    );
}



/** 会话逐轮明细面板：会话总量摘要 + 按轮次的元数据时间线（最新在前，对齐开源默认 time 排序）。
 *  每轮 = 一次 assistant 用量行（usage_event），只含 时间/模型/token/成本，不读正文。 */
function SessionDetailPanel({ session }: { session: SessionSummary }) {
  const { data: events, isLoading } = useSessionEvents(session.session_id);
  const rows = events ?? [];
  const inputTotal = session.cache_tokens + session.input_tokens;
  const hitPct = inputTotal > 0 ? Math.round((session.cache_tokens / inputTotal) * 100) : 0;
  const cost = formatUsd(session.cost_amount);
  const msgLabel = formatMessageCount(session.message_count);
  const multiModel = session.model_set.length > 1;
  return (
    <div className="pg-tm-sub">
      {/* 消息数与列表同口径：千分位 + 单复数；多模型为各模型分组之和（悬停提示） */}
      {msgLabel && (
        <div className="pg-tm-sub-row">
          <span>消息数</span>
          <b title={multiModel ? MULTI_MODEL_MSG_HINT : undefined}>{msgLabel}</b>
        </div>
      )}
      <div className="pg-tm-sub-row"><span>输入（缓存命中） <em>{hitPct}%</em></span><b>{formatInt(session.cache_tokens)}</b></div>
      <div className="pg-tm-sub-row"><span>输入（缓存未命中）</span><b>{formatInt(session.input_tokens)}</b></div>
      <div className="pg-tm-sub-row"><span>输出</span><b>{formatInt(session.output_tokens)}</b></div>
      {cost && <div className="pg-tm-sub-row"><span>成本</span><b>{cost}</b></div>}
      <div className="pg-tm-turn-list">
        <div className="pg-tm-turn-head">
          <span>轮次明细</span>
          <em>{rows.length > 0 ? `${rows.length} 轮` : ""}</em>
        </div>
        {isLoading && rows.length === 0 ? (
          <div className="pg-tm-turn-empty">加载中…</div>
        ) : rows.length === 0 ? (
          <div className="pg-tm-turn-empty">暂无逐轮明细</div>
        ) : (
          rows.map((e, i) => <TurnRow key={`${e.occurred_at}-${i}`} event={e} />)
        )}
      </div>
    </div>
    );
}



function TurnRow({ event }: { event: SessionEventRow }) {
  const cost = formatUsd(event.cost_amount);
  return (
    <div className="pg-tm-turn">
      <div className="pg-tm-turn-row1">
        <span className="pg-tm-turn-time">{formatSessionTime(event.occurred_at) || "--:--"}</span>
        <span className="pg-tm-turn-model">
          {event.model ? <><ProviderLogo model={event.model} size={12} />{event.model}</> : "—"}
        </span>
        <b>{formatInt(event.total_tokens)}</b>
      </div>
      <div className="pg-tm-turn-row2">
        <span>输入 <em>{formatInt(event.input_tokens)}</em></span>
        <span>缓存 <em>{formatInt(event.cache_tokens)}</em></span>
        <span>输出 <em>{formatInt(event.output_tokens)}</em></span>
        {cost && <em className="pg-tm-turn-cost">{cost}</em>}
      </div>
    </div>
  );
}

// ───────────────────────── 趋势 ─────────────────────────
function TrendView({
  trend, trendLoading, dailyAsc, range,
}: {
  trend: ReturnType<typeof useTrend>["data"];
  trendLoading: boolean;
  dailyAsc: TrendDay[];
  range: Range;
}) {
  // 柱状图窗口：今日 / 近 7 天都展示最近 7 天（作上下文），本月按月，累计按月聚合。
  const barDays = useMemo(() => {
    if (range === "day" || range === "7d") return dailyAsc.slice(-7);
    if (range === "month") return dailyAsc.filter((d) => d.date >= rangeStartKey("month"));
    return dailyAsc; // total 走 monthly，不使用
  }, [range, dailyAsc]);

  // 统计卡窗口：活跃时间 / 峰值单日随「今日 / 近 7 天 / 本月」条件变化（今日=今天）。
  const windowDays = useMemo(
    () => dailyAsc.filter((d) => d.date >= rangeStartKey(range)),
    [range, dailyAsc],
  );

  const view = useMemo(() => {
    if (range === "total") {
      return { kind: "monthly" as const, bars: (trend?.monthly ?? []).map((m) => ({ label: m.month, value: m.tokens })) };
    }
    return { kind: "daily" as const, bars: barDays.map((d) => ({ label: d.date.slice(5).replace("-", "/"), value: d.tokens })) };
  }, [range, trend, barDays]);

  const max = Math.max(1, ...view.bars.map((b) => b.value));
  const first = view.bars[0]?.label ?? "";
  const lastLabel = view.bars[view.bars.length - 1]?.label ?? "";

  // 4 张统计卡：活跃天数 / 连续天数 固定（全历史权威汇总），活跃时间 / 峰值单日
  // 随「今日 / 近 7 天 / 本月 / 累计」条件变化（今日=今天、近 7 天=7 天、本月=本月）。
  const stats = useMemo(() => {
    const windowPeak = windowDays.reduce<TrendDay | null>(
      (m, d) => (d.tokens > (m?.tokens ?? 0) ? d : m), null,
    );
    const windowActive = windowDays.reduce((sum, d) => sum + (d.active_time_ms ?? 0), 0);
    return [
      { label: "活跃天数", value: `${trend?.active_days ?? 0}` },
      { label: "连续天数", value: `${trend?.streak_days ?? 0}` },
      {
        label: "活跃时间",
        value: range === "total" ? activeTimeLabel(trend?.active_time_ms) : activeTimeLabel(windowActive),
      },
      {
        label: "峰值单日",
        value: range === "total"
          ? (trend?.peak_day ? formatTokens(trend.peak_day.tokens) : "—")
          : (windowPeak ? formatTokens(windowPeak.tokens) : "—"),
      },
    ];
  }, [range, windowDays, trend]);

  // 点击趋势页任意区域 → 打开桌面端 Token Monitor（趋势 tab），托盘仅作预览入口
  const openDesktopTrend = useCallback(() => {
    void openPoolGateFromTray("tokenmonitor", undefined, "trend").catch(() => undefined);
  }, []);

  return (
    <div
      className="pg-tm-detail-view pg-tm-trend-open"
      role="button"
      tabIndex={0}
      title="打开桌面端 Token Monitor 趋势详情"
      aria-label="打开桌面端 Token Monitor 趋势详情"
      onClick={openDesktopTrend}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openDesktopTrend(); } }}
    >
      <section className="pg-tg-card">
        <div className="pg-tm-nav static">
          <span className="pg-tm-nav-title"><ActivityIcon size={14} /><span>最近趋势</span></span>
        </div>
        <div className="pg-tm-trend-bars">
          {/* 数据优先：有缓存/placeholder 数据时立即展示，加载中只在真正无数据时出现 */}
          {view.bars.length === 0 ? (
            <div className="pg-tm-trend-empty"><EmptyState label={trendLoading ? "加载中…" : "暂无趋势数据"} /></div>
          ) : (
            view.bars.map((b, i) => (
              <div key={`${view.kind}-${b.label}`} className="pg-tm-trend-col" title={`${b.label} · ${formatTokens(b.value)}`}>
                <i style={{ height: `${Math.max(4, (b.value / max) * 100)}%`, opacity: i === view.bars.length - 1 ? 1 : 0.62 }} />
              </div>
            ))
          )}
        </div>
        <div className="pg-tm-trend-axis">
          <span>{first}</span>
          <span>{lastLabel}</span>
        </div>
      </section>
      <div className="pg-tm-stat-grid">
        {stats.map((s) => (
          <div key={s.label} className="pg-tg-card pg-tm-stat-card">
            <b>{s.value}</b>
            <span>{s.label}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
