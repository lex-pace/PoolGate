import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, Check, ChevronRight, Clock, Coins, RefreshCw } from "lucide-react";
import RingProgress from "@/components/tray/RingProgress";
import {
  providerLabel, accountIdentity, accountBrandParts, accountTier,
  isBalanceWindow, currencyOf, formatBalance, balanceTone,
} from "@/lib/account-display";
import { useAccountDisplay } from "@/components/ui/AccountDisplay";
import type {
  QuotaAccountView, QuotaWindowView, AccountUsageStat, TokenRateView,
} from "@/lib/token-monitor-commands";
import RollingNumber from "@/components/tray/RollingNumber";
import { ProviderLogo } from "@/components/token-monitor/ToolLogo";

// ──────────────────── 类型与工具 ────────────────────
export type TokenRange = "day" | "7d" | "month" | "total";

export interface TokensQuotaMergedProps {
  /** 当前范围 */
  range: TokenRange;
  /** 范围展示标签（"今日" / "近 7 天" / "本月" / "累计"） */
  rangeLabel: string;
  /** 累计 TOKEN 数（数字 count-up 用） */
  totalTokens: number;
  /** 美金价格（数字，可为 null） */
  costUsd: number | null;
  /** 人民币价格（数字，可为 null） */
  costCny: number | null;
  /** 上次更新时间（ISO 字符串），用于"实时更新 HH:MM" */
  updatedAt?: string;
  /** 后端类型（"official_api" / "scraping" ...）—— 决定圆点颜色 */
  liveConfidence?: string;
  /** 数据陈旧（>2 分钟未更新）→ 触发 refresh 按钮琥珀脉冲 */
  stale?: boolean;
  /** 是否处于刷新中（旋转）/ 刷新完成（勾） */
  refreshing?: boolean;
  refreshDone?: boolean;
  /** 触发手动刷新 */
  onRefresh?: () => void;
  /** 跳转到 Token Monitor 明细页（额度明细） */
  onOpenDetail?: () => void;
  /** 账号额度数据（已有真实额度窗口的） */
  accounts: QuotaAccountView[];
  /** 仅渲染 Hero 区域（今日 Tokens + 价格），隐藏额度大卡 */
  heroOnly?: boolean;
  /** 仅渲染额度大卡，隐藏 Hero 区域 */
  cardOnly?: boolean;
  /** 点击 Logo 轮询的速率模式（费用/分速率/秒速率）；非 cost 时价格行位置替换为速率读数。 */
  rateMode?: "cost" | "min" | "sec";
  /** 实时速率数据（tok/min、tok/s），配合 rateMode 展示。 */
  rate?: TokenRateView | null;
}

const RANGE_LABEL: Record<string, string> = {
  day: "今日", "7d": "近 7 天", month: "本月", total: "累计",
};

/** 主窗口：5 小时额度 / primary key，其余窗口用于下方次级进度条。 */
function pickPrimary(windows: QuotaWindowView[]): QuotaWindowView | undefined {
  if (windows.length === 0) return undefined;
  const byKey = windows.find((w) => w.window_key === "primary" || /5\s*小\s*时/i.test(w.label || ""));
  if (byKey) return byKey;
  const score = (w: QuotaWindowView): number => {
    if (w.remaining_percent != null) return w.remaining_percent;
    if (w.remaining_value != null && w.limit_value && w.limit_value > 0) return (w.remaining_value / w.limit_value) * 100;
    return Number.POSITIVE_INFINITY;
  };
  return [...windows].sort((a, b) => score(a) - score(b))[0];
}

/** 额度剩余值（百分比 → 数值 → null）。
 *  百分比统一钳制到 [0,100]；非有限数值（异常号码）一律按无数据处理，不在界面展示。 */
function remainingOf(w: QuotaWindowView): number | null {
  if (w.remaining_percent != null) {
    if (!Number.isFinite(w.remaining_percent)) return null;
    return Math.max(0, Math.min(100, w.remaining_percent));
  }
  if (w.remaining_value != null) {
    if (!Number.isFinite(w.remaining_value)) return null;
    return w.remaining_value;
  }
  return null;
}

/** 圆环按剩余百分比分级。 */
function ringColor(pct: number | null): { stroke: string; from: string; to: string; text: string } {
  if (pct == null) return { stroke: "#9a9da3", from: "#b6bcc4", to: "#9a9da3", text: "#9a9da3" };
  if (pct <= 10) return { stroke: "#ff453a", from: "#ff6961", to: "#ff453a", text: "#ff453a" };
  if (pct <= 30) return { stroke: "#ff9500", from: "#ffb84d", to: "#ff9500", text: "#ff9500" };
  return { stroke: "#34c759", from: "#34c759", to: "#34c759", text: "#34c759" }; // 绿色默认（与截图一致）
}

/** 距 iso 的剩余时长（"3 小时 42 分钟"），超过 30 天回退日期。 */
function relativeUntil(iso?: string): string {
  if (!iso) return "";
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return "";
  const diff = t - Date.now();
  if (diff <= 0) return "已重置";
  const day = 86_400_000, hour = 3_600_000, min = 60_000;
  if (diff >= 30 * day) { const d = new Date(iso); return `${d.getMonth() + 1}月${d.getDate()}日`; }
  if (diff >= day) return `${Math.floor(diff / day)} 天 ${Math.floor((diff % day) / hour)} 小时`;
  if (diff >= hour) return `${Math.floor(diff / hour)} 小时 ${Math.floor((diff % hour) / min)} 分钟`;
  if (diff >= min) return `${Math.floor(diff / min)} 分钟`;
  return "即将重置";
}

/** HH:MM（中文格式），用于"重置时间"绝对时钟。 */
function resetClock(iso?: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${pad(d.getMonth() + 1)}月${pad(d.getDate())}日 ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** HH:MM（用于「实时更新」）。 */
function liveClock(iso?: string): string {
  if (!iso) return "--:--";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "--:--";
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function formatInt(n: number): string {
  return Math.round(n).toLocaleString("en-US");
}

function formatUsd(n: number | null | undefined): string {
  if (n == null) return "—";
  return n >= 1 ? `$${n.toFixed(2)}` : `$${n.toFixed(4)}`;
}

function formatCny(n: number | null | undefined): string {
  if (n == null) return "—";
  return `¥${n.toFixed(2)}`;
}

/** 速率读数：1.5K / 245.6 / 0.04（tok/min、tok/s 共用）。 */
function formatRate(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  if (n >= 1) return n.toFixed(1);
  return n.toFixed(2);
}

const cxrRate = 7.2; // USD → CNY 估算汇率（仅用于面板展示 "≈ ¥x.xx"）

/** 用于次级进度条的小段标签（周/月/账户余额…）。 */
function shortWindowLabel(w: QuotaWindowView): string {
  const label = (w.label || "").replace(/[（(].*?[)）]/g, "").replace(/\s+/g, "").trim();
  const stripped = label.endsWith("额度") ? label.slice(0, -2) : label;
  return stripped.length >= 2 ? stripped : (label || w.window_type || "额度");
}

/** 次级窗口配色：周额度→橙、月额度→紫、5小时→绿、账户余额→青（参考开源 Token Monitor 多色）。 */
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

const SWIPE_THRESHOLD = 50;
const AUTO_FLIP_SECONDS = 3;

/** 复刻截图：上方"今日 Tokens + 实时更新 + 刷新"，下方"额度大卡 + 左右箭头 + 圆点指示器" 合并面板。 */
export default function TokensQuotaMerged({
  range,
  rangeLabel,
  totalTokens,
  costUsd,
  costCny,
  updatedAt,
  refreshing,
  refreshDone,
  stale,
  onRefresh,
  onOpenDetail,
  accounts,
  heroOnly,
  cardOnly,
  rateMode,
  rate,
}: TokensQuotaMergedProps) {
  const live = liveClock(updatedAt);
  // 自适配人民币：优先使用传入的 costCny，否则用 USD × 汇率估算
  const cny = costCny != null ? costCny : (costUsd != null ? costUsd * cxrRate : null);

  // ── 账号额度卡（按"是否有真实剩余数据"过滤，额度为 0 的保留展示）──
  // 排序口径：按「卡片展示的主窗口剩余」降序（主窗口 = 最受约束的当前限制周期，即圆环百分比），
  // 而不是多窗口最大值——保证轮播顺序与展示百分比一致，额度为 0 的账号排在末尾。
  const cards = useMemo(() => {
    const list = accounts
      .map((acc) => {
        const windows = acc.windows.filter((w) => remainingOf(w) != null);
        const primary = pickPrimary(windows);
        const best =
          primary != null
            ? remainingOf(primary)
            : windows.length
              ? Math.max(...windows.map((w) => remainingOf(w)!))
              : null;
        return { account: acc, windows, best };
      })
      .filter((c) => c.windows.length > 0);
    list.sort((a, b) => (b.best ?? -1) - (a.best ?? -1) || (a.account.label || "").localeCompare(b.account.label || ""));
    return list;
  }, [accounts]);

  const count = cards.length;

  // ── 轮播状态：index / dragX / 倒计时 / 悬停暂停 ──
  const [index, setIndex] = useState(0);
  const [dragX, setDragX] = useState<number | null>(null);
  const drag = useRef({ startX: 0, startIndex: 0, active: false });
  const hover = useRef(false);
  const [paused, setPaused] = useState(false);
  const [countdown, setCountdown] = useState(AUTO_FLIP_SECONDS);
  const countdownRef = useRef(AUTO_FLIP_SECONDS);
  const [reduced, setReduced] = useState(false);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReduced(mq.matches);
    update(); mq.addEventListener?.("change", update);
    return () => mq.removeEventListener?.("change", update);
  }, []);

  useEffect(() => {
    setIndex(0); setDragX(null); setPaused(false);
    countdownRef.current = AUTO_FLIP_SECONDS; setCountdown(AUTO_FLIP_SECONDS);
    if (count <= 1) return;
    const timer = window.setInterval(() => {
      if (hover.current || drag.current.active) return;
      countdownRef.current -= 1;
      if (countdownRef.current <= 0) {
        countdownRef.current = AUTO_FLIP_SECONDS;
        setIndex((i) => (i + 1) % count);
      }
      setCountdown(countdownRef.current);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [count]);

  const goPrev = useCallback(() => {
    if (count <= 1) return;
    setIndex((i) => (i - 1 + count) % count);
    countdownRef.current = AUTO_FLIP_SECONDS; setCountdown(AUTO_FLIP_SECONDS);
  }, [count]);

  const goNext = useCallback(() => {
    if (count <= 1) return;
    setIndex((i) => (i + 1) % count);
    countdownRef.current = AUTO_FLIP_SECONDS; setCountdown(AUTO_FLIP_SECONDS);
  }, [count]);

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (count <= 1) return;
    if ((e.target as HTMLElement).closest("button")) return;
    drag.current = { startX: e.clientX, startIndex: index, active: true };
    setPaused(true);
    e.currentTarget.setPointerCapture?.(e.pointerId);
  };
  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!drag.current.active) return;
    let offset = e.clientX - drag.current.startX;
    const edge =
      (drag.current.startIndex === 0 && offset > 0) ||
      (drag.current.startIndex === count - 1 && offset < 0);
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
    countdownRef.current = AUTO_FLIP_SECONDS; setCountdown(AUTO_FLIP_SECONDS);
    if (offset <= -SWIPE_THRESHOLD) setIndex((from + 1) % count);
    else if (offset >= SWIPE_THRESHOLD) setIndex((from - 1 + count) % count);
  };

  // 当前活动卡（用于左右指示箭头 disabled）
  const dragging = drag.current.active;
  const autoText = paused ? "已暂停" : `自动轮播中 ${countdown}s`;

  const showHero = !cardOnly;
  const showCard = !heroOnly;

  return (
    <section className="pg-tm-merged" aria-label="今日 Tokens 与额度合并面板">
      {/* ────── HERO（column 布局）：
          行1：左 label + 右 实时更新 HH:MM
          行2：左 大数字 + 右 刷新按钮(垂直居中对齐本行)
          行3：左 价格（$xxx ≈ ¥xxx） ── */ }
      {showHero && (
      <div className="pg-tm-merged-hero">
        <div className="pg-tm-merged-hero-row top">
          <span className="pg-tm-merged-hero-label">{rangeLabel} Tokens</span>
          <span className="pg-tm-merged-live" aria-live="polite">
            <i className="dot" />
            <b>实时更新</b>
            <em>{live}</em>
          </span>
        </div>
        <div className="pg-tm-merged-hero-row mid">
          <strong className="pg-tm-merged-hero-total">
            <RollingNumber value={totalTokens} />
          </strong>
          <button
            type="button"
            className={`pg-tm-merged-refresh${refreshing ? " refreshing" : ""}${refreshDone ? " done" : ""}${stale && !refreshing && !refreshDone ? " stale" : ""}`}
            aria-label="立即刷新"
            title={stale ? "数据可能已陈旧，点击立即刷新" : "立即刷新（强制重扫所有工具）"}
            onClick={() => onRefresh?.()}
            disabled={refreshing}
          >
            {refreshDone ? <Check size={15} /> : <RefreshCw size={15} className={refreshing ? "spin" : ""} />}
          </button>
        </div>
        <div className="pg-tm-merged-hero-row btm">
          {rateMode && rateMode !== "cost" ? (
            <span className="pg-tm-rate" role="status" aria-live="polite">
              {(rateMode === "sec" ? rate?.tokens_per_sec : rate?.tokens_per_min) != null ? (
                <>
                  <b>{formatRate(rateMode === "sec" ? rate?.tokens_per_sec ?? 0 : rate?.tokens_per_min ?? 0)}</b>
                  <em>{rateMode === "sec" ? "tok/s" : "tok/min"}</em>
                </>
              ) : (
                <em>速率不可用</em>
              )}
            </span>
          ) : (
            <span className="pg-tm-merged-hero-cost">
              <b>{formatUsd(costUsd)}</b>
              <em>≈ {formatCny(cny)}</em>
            </span>
          )}
        </div>
      </div>
      )}

      {/* ────── QUOTA 大卡：标题行（额度 + 自动轮播倒计时 + 箭头）+ 账号轮播 ────── */}
      {showCard && (
        <>
          <button type="button" className="pg-tm-nav" onClick={onOpenDetail} aria-label="额度明细">
            <span className="pg-tm-nav-title">
              <Coins size={14} />
              <span>额度</span>
              <em className="pg-tm-nav-count">({count} 个账号)</em>
            </span>
            <span className="pg-tm-nav-right">
              {count > 1 && (
                <span className={`pg-tm-merged-auto${autoText.startsWith("已暂停") ? " paused" : ""}`} aria-live="polite">
                  <Clock size={10} />
                  {autoText}
                </span>
              )}
              <ChevronRight size={15} className="pg-tm-nav-arrow" />
            </span>
          </button>
          {count > 0 ? (
            <div
              className={`pg-tm-merged-quota-card${dragging ? " dragging" : ""}`}
              onPointerDown={onPointerDown}
              onPointerMove={onPointerMove}
              onPointerUp={endDrag}
              onPointerCancel={endDrag}
              onMouseEnter={() => { hover.current = true; setPaused(true); }}
              onMouseLeave={() => { hover.current = false; setPaused(false); }}
              aria-label={`账号额度轮播（共 ${count} 个）`}
            >
              <div className={`pg-tm-merged-track${reduced ? " no-motion" : ""}`} style={{ transform: `translateX(calc(${-index * 100}% + ${dragX ?? 0}px))` }}>
                {cards.map(({ account, windows }, i) => (
                  <MergedAccountCard key={account.account_id} account={account} windows={windows} active={i === index} />
                ))}
              </div>
            </div>
          ) : (
            <div className="pg-tm-merged-empty">
              <span>暂无实时额度数据</span>
              <em>绑定 OAuth 账号后将在此展示各账号的实时额度窗口</em>
            </div>
          )}
        </>
      )}
    </section>
  );
}

// ──────────────────── 单张合并卡（两列：账户 / 窗口 + 进度条紧凑布局） ────────────────────
function MergedAccountCard({
  account, windows, active,
}: {
  account: QuotaAccountView;
  windows: QuotaWindowView[];
  active?: boolean;
}) {
  const { mode } = useAccountDisplay();
  const primary = pickPrimary(windows) ?? windows[0];
  // 预付费余额（DeepSeek 等）：无窗口上限/重置，圆环显示金额 + 阈值色，不走百分比模型
  const isBalance = primary != null && isBalanceWindow(primary);
  const balanceAmt = isBalance ? (primary?.remaining_value ?? null) : null;
  const balCode = isBalance ? currencyOf(primary!) : "USD";
  const balTone = balanceTone(balanceAmt, balCode);
  const extras = windows.filter((w) => w.window_key !== primary?.window_key && !isBalanceWindow(w));
  const pct = !isBalance && primary ? remainingOf(primary) : null;
  const pctClamp = pct != null ? Math.max(0, Math.min(100, pct)) : null;
  const identity = accountIdentity(account, mode);
  const { main: brand } = accountBrandParts(account);
  const remainingStr = primary?.resets_at ? relativeUntil(primary.resets_at) : "—";
  const resetStr = resetClock(primary?.resets_at) || "—";

  // 多窗口徽章顺序
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

  // 圆环颜色分级：余额按阈值色；百分比窗口按剩余比例
  const ringStroke = isBalance
    ? balTone.color
    : pctClamp != null && pctClamp <= 10
      ? "#ff453a"
      : pctClamp != null && pctClamp <= 30
        ? "#ff9500"
        : "#34c759";
  const ringR = 33; // 76px 直径 / 2 - 5 stroke / 2
  const ringC = 2 * Math.PI * ringR;
  const ringDash = isBalance
    ? balanceAmt != null && balanceAmt > 0 ? ringC : 0 // 余额：金额>0 满环（状态色），耗尽空环
    : pctClamp != null ? (pctClamp / 100) * ringC : 0;

  // 会员等级胶囊：从 plan_name 中识别 Free/Plus/Pro/Max 等等级（带颜色）
  const tier = accountTier(account.plan_name);

  return (
    <div className={`pg-tm-merged-card${active ? " is-active" : " is-inactive"}`}>
      {/* 顶部状态：品牌 + 等级胶囊；右侧为脱敏账号（原自动轮播倒计时的位置） */}
      <div className="pg-tm-merged-card-status">
        <div className="pg-tm-merged-card-brand-row">
          <ProviderLogo brand={brand} size={16} />
          <strong className="pg-tm-merged-card-brand">{brand}</strong>
          {tier && (
            <span className="pg-tm-merged-card-tier" style={{ background: tier.color }}>
              {tier.label}
            </span>
          )}
        </div>
        {identity && (
          <span className="pg-tm-merged-card-ident" title={identity}>{identity}</span>
        )}
      </div>

      {/* 左列：最大化账户额度环（76px 主视觉） */}
      <div
        className="pg-tm-merged-acc"
        role="img"
        aria-label={isBalance ? `账户余额 ${formatBalance(balanceAmt, balCode)}` : `账户额度 ${pctClamp != null ? `${Math.round(pctClamp)}%` : "未知"}`}
      >
        <svg width="76" height="76" viewBox="0 0 76 76" aria-hidden>
          <circle cx="38" cy="38" r={ringR} fill="none" stroke="rgba(118,118,128,.18)" strokeWidth="5" />
          {(isBalance ? balanceAmt != null : pctClamp != null) && (
            <circle
              cx="38" cy="38" r={ringR} fill="none"
              stroke={ringStroke} strokeWidth="5" strokeLinecap="round"
              strokeDasharray={`${ringDash} ${ringC}`}
              transform="rotate(-90 38 38)"
              style={{ filter: `drop-shadow(0 0 5px ${ringStroke}66)` }}
            />
          )}
        </svg>
        <div className="pg-tm-merged-acc-center">
          {isBalance ? (
            <strong className="pg-tm-merged-acc-ring-pct is-amt" style={{ color: ringStroke }}>
              {formatBalance(balanceAmt, balCode)}
            </strong>
          ) : (
            <strong className="pg-tm-merged-acc-ring-pct" style={{ color: ringStroke }}>
              {pctClamp != null ? `${Math.round(pctClamp)}%` : "—"}
            </strong>
          )}
          <em className="pg-tm-merged-acc-ring-label">{isBalance ? "账户余额" : "剩余额度"}</em>
        </div>
      </div>

      {/* 圆环与右侧窗口之间的竖杆分割 */}
      <div className="pg-tm-merged-divider-col" aria-hidden />

      {/* 右列：主窗口 + 重置 + 次级窗口紧凑布局 */}
      <div className="pg-tm-merged-wins">
        {/* 主窗口（5小时额度）：单行头 + 全宽进度条 + 剩余/重置合并行 */}
        <div className="pg-tm-merged-win primary">
          <div className="pg-tm-merged-win-head">
            <span className="pg-tm-merged-win-icon" aria-hidden>
              <Clock size={11} />
            </span>
            <span className="pg-tm-merged-win-name">{primary?.label || "5 小时额度"}</span>
            <b className="pg-tm-merged-win-pct" style={isBalance ? { color: ringStroke } : undefined}>
              {isBalance ? formatBalance(balanceAmt, balCode) : pctClamp != null ? `${Math.round(pctClamp)}%` : "—"}
            </b>
          </div>
          <span className="pg-tm-merged-bar">
            <i style={isBalance ? { width: balanceAmt != null && balanceAmt > 0 ? "100%" : "0%", background: balTone.color } : { width: `${pctClamp ?? 0}%` }} />
          </span>
          {isBalance ? (
            <span className="pg-tm-merged-win-foot">
              <em style={{ color: balTone.color }}>
                余额 <b style={{ color: balTone.color }}>{balTone.label}</b>
              </em>
            </span>
          ) : (
            primary?.resets_at && (
              <span className="pg-tm-merged-win-foot">
                <em>
                  <ArrowLeft size={9} className="rotate" />
                  剩余 <b>{remainingStr}</b>
                </em>
                <span className="dot-sep" />
                <em className="reset">
                  重置 <b>{resetStr}</b>
                </em>
              </span>
            )
          )}
        </div>

        {/* 次级窗口（月度/周/账户余额…）：单行紧凑布局 */}
        {sortedExtras.length > 0 && <div className="pg-tm-merged-divider" />}
        {sortedExtras.slice(0, 3).map((w, i) => {
          const wp = remainingOf(w);
          const wc = wp != null ? Math.max(0, Math.min(100, wp)) : null;
          const wcColor = i === 0 ? "#bf5af2" : i === 1 ? "#ff9500" : subBarColor(w.label || "");
          return (
            <div key={w.window_key} className="pg-tm-merged-sub">
              <span className="pg-tm-merged-sub-swatch" style={{ background: wcColor }} />
              <span className="pg-tm-merged-sub-name">{shortWindowLabel(w)}额度</span>
              <span className="pg-tm-merged-sub-track">
                <i style={{ width: `${wc ?? 0}%`, background: wcColor }} />
              </span>
              <b className="pg-tm-merged-sub-pct" style={{ color: wcColor }}>
                {wc != null ? `${Math.round(wc)}%` : "—"}
              </b>
            </div>
          );
        })}
        {sortedExtras.length > 3 && (
          <em className="pg-tm-merged-more">+{sortedExtras.length - 3} 更多</em>
        )}
      </div>
    </div>
  );
}

// 复用：导出小工具（避免 TokenMonitorTrayCard 内重复实现）
export { pickPrimary, remainingOf, ringColor, relativeUntil, resetClock, formatInt, formatUsd, formatCny };
// 让 RANGE_LABEL 默认导出（保持对外一致）
export { RANGE_LABEL };
