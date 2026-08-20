import React, { useMemo, useState } from "react";
import { Card, CBody, CHeader, CTitle } from "@/components/ui/Card";
import Segmented from "@/components/ui/Segmented";
import { TrendingUp } from "lucide-react";
import { useTrend, formatTokens } from "@/components/token-monitor/token-monitor-data";
import type { TrendDay } from "@/lib/token-monitor-commands";

const rangeOptions = [
  { value: 7, label: "7 天" },
  { value: 30, label: "30 天" },
  { value: 90, label: "90 天" },
  { value: 365, label: "1 年" },
  { value: 0, label: "全部" },
] as const;

const modeOptions = [
  { value: "bars" as const, label: "堆叠柱" },
  { value: "cumulative" as const, label: "累计增长" },
];

const stackOptions = [
  { value: "tool" as const, label: "按工具" },
  { value: "model" as const, label: "按模型" },
];

// 与总览按工具/按模型一致的调色板
const PALETTE = ["#37b6a0", "#5b8def", "#6b7280", "#d98a5c", "#d56a54", "#9b7fe0", "#c2a24d", "#5cc2d9", "#d97fb0", "#4ade80"];

function shortDate(key: string): string {
  return key.slice(5).replace("-", "/");
}

function localKey(date: Date): string {
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

function todayKey(): string {
  return localKey(new Date());
}

interface Tooltip {
  x: number;
  y: number;
  title: string;
  rows: { label: string; value: number }[];
}

// Y 轴"整齐"刻度：取 1/2/2.5/5 × 10^n 步进，向上取整到 ≥ 数据最大值，保证任何峰值都不被截断。
function niceScale(maxValue: number, targetTicks = 5): { yMax: number; step: number } {
  if (maxValue <= 0) return { yMax: 1, step: 0.25 };
  const raw = maxValue / targetTicks;
  const exp = Math.floor(Math.log10(raw));
  const base = Math.pow(10, exp);
  let step = base * 10;
  for (const m of [1, 2, 2.5, 5, 10]) {
    if (m * base >= raw) {
      step = m * base;
      break;
    }
  }
  const yMax = Math.ceil(maxValue / step) * step;
  return { yMax, step };
}

// 从 0 到 yMax 的刻度值序列（含两端），最后一项钳到 yMax 避免浮点误差。
function scaleTicks(yMax: number, step: number): number[] {
  const n = Math.max(1, Math.round(yMax / step));
  return Array.from({ length: n + 1 }, (_, i) => (i === n ? yMax : i * step));
}

// 刻度标签：整数去掉小数尾（400M / 1.5B），大数用 B/M/K。
function fmtTick(v: number): string {
  if (v >= 1_000_000_000) return `${(v / 1_000_000_000).toFixed(2).replace(/\.?0+$/, "")}B`;
  if (v >= 1_000_000) return `${(v / 1_000_000).toFixed(2).replace(/\.?0+$/, "")}M`;
  if (v >= 1_000) return `${(v / 1_000).toFixed(1).replace(/\.?0+$/, "")}K`;
  return String(v);
}

export default function Trend() {
  const { data: trend, isLoading: trendLoading } = useTrend("total");
  const [days, setDays] = useState<number>(30);
  const [mode, setMode] = useState<"bars" | "cumulative">("bars");
  const [stackBy, setStackBy] = useState<"tool" | "model">("tool");
  const [hover, setHover] = useState<Tooltip | null>(null);

  const dailyAsc = useMemo(() => [...(trend?.daily ?? [])].sort((a, b) => a.date.localeCompare(b.date)), [trend]);
  // 按自然日开窗：30 天 = 最近 30 个自然日（稀疏数据缺的天用零占位，避免「30 天显示 60 天」）
  const window = useMemo(() => {
    if (days === 0) return dailyAsc;
    // 只保留最近 N 个自然日内的数据点（缺数据的日不占位，避免横向空白）
    const end = new Date();
    const start = new Date(end);
    start.setDate(start.getDate() - (days - 1));
    const startKey = localKey(start);
    return dailyAsc.filter((d) => d.date >= startKey);
  }, [dailyAsc, days]);

  // 拆分字段：按工具 → per_client，按模型 → per_model
  const splitField = stackBy === "model" ? "per_model" : "per_client";
  const splitsOf = (d: TrendDay) => (d[splitField] ?? []) as { key: string; tokens: number }[];

  // 窗口内各 key 合计（降序）→ 图例与柱段顺序
  const keys = useMemo(() => {
    const totals: Record<string, number> = {};
    for (const d of window) for (const s of splitsOf(d)) totals[s.key] = (totals[s.key] ?? 0) + s.tokens;
    return Object.keys(totals).sort((a, b) => totals[b] - totals[a] || a.localeCompare(b));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [window, splitField]);

  const colorOf = (key: string) => PALETTE[Math.max(0, keys.indexOf(key)) % PALETTE.length];

  const legend = useMemo(() => {
    const totals: Record<string, number> = {};
    let grand = 0;
    for (const d of window) for (const s of splitsOf(d)) {
      totals[s.key] = (totals[s.key] ?? 0) + s.tokens;
      grand += s.tokens;
    }
    return Object.entries(totals)
      .map(([key, value]) => ({ key, value, pct: grand > 0 ? (value / grand) * 100 : 0 }))
      .sort((a, b) => b.value - a.value)
      .filter((r) => r.value > 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [window, splitField]);

  // 图表几何（加高，让柱子更饱满）
  const W = 760;
  const H = 300;
  const PAD_L = 48;
  const PAD_R = 8;
  const PAD_T = 12;
  const PAD_B = 30;
  const innerW = W - PAD_L - PAD_R;
  const innerH = H - PAD_T - PAD_B;

  // Y 轴刻度：以窗口内最高单日用量为基准向上取整（1/2/2.5/5 × 10^n 步进），
  // yMax 恒 ≥ 真实最大值，任何峰值都不会被截断。
  const { yMax, step: yStep } = useMemo(() => {
    const maxDaily = window.reduce((max, d) => {
      const splitTotal = splitsOf(d).reduce((sum, s) => sum + s.tokens, 0);
      return Math.max(max, d.tokens, splitTotal);
    }, 0);
    return niceScale(maxDaily);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [window, splitField]);

  // —— 堆叠柱 ——
  // 柱高以 d.tokens（当日权威总量，与总览「Token 活动」热力图同源）为基准：
  // 拆分项之和与总量有偏差时按比例缩放，保证每天柱顶 = 每日用量，不被拆分加和带偏。
  const bars = useMemo(() => {
    const slot = window.length ? innerW / window.length : innerW;
    const barW = slot * 0.86;
    return window.map((d, i) => {
      const x = PAD_L + i * slot + (slot - barW) / 2;
      const source = new Map(splitsOf(d).map((s) => [s.key, s.tokens]));
      const splitTotal = [...source.values()].reduce((sum, v) => sum + v, 0);
      const scale = splitTotal > 0 ? d.tokens / splitTotal : 0;
      let cum = 0;
      const segments: { key: string; tokens: number; y: number; h: number }[] = [];
      for (const k of keys) {
        const v = source.get(k) ?? 0;
        if (v <= 0) continue;
        const h = ((v * scale) / yMax) * innerH;
        segments.push({ key: k, tokens: v, y: PAD_T + innerH - cum - h, h });
        cum += h;
      }
      // 兜底：当日有用量但无任何可拆分项时画一个整柱（避免柱子凭空消失）
      if (segments.length === 0 && d.tokens > 0) {
        segments.push({ key: "其他", tokens: d.tokens, y: PAD_T, h: (d.tokens / yMax) * innerH });
      }
      return { date: d.date, x, width: barW, total: d.tokens, requests: d.requests, segments };
    });
  }, [window, keys, yMax, splitField]);

  // —— 累计增长（总量增长曲线）——
  const cumulative = useMemo(() => {
    let acc = 0;
    return window.map((d) => {
      acc += d.tokens;
      return { date: d.date, cum: acc, delta: d.tokens };
    });
  }, [window]);
  const grandTotal = cumulative.length ? cumulative[cumulative.length - 1].cum : 0;
  const { yMax: cumulativeYMax, step: cumulativeStep } = niceScale(grandTotal);

  const fmtTokens = (v: number) => formatTokens(v);

  const onMove = (e: React.MouseEvent<SVGElement>, title: string, rows: { label: string; value: number }[]) => {
    const svg = (e.currentTarget as SVGElement).ownerSVGElement?.getBoundingClientRect();
    if (!svg) return;
    setHover({ x: e.clientX - svg.left, y: e.clientY - svg.top, title, rows });
  };

  const axisTicks = useMemo(() => {
    const every = Math.max(1, Math.ceil(window.length / 8));
    return window.map((d, i) => (i % every === 0 ? { x: PAD_L + (i + 0.5) * (innerW / Math.max(1, window.length)), label: shortDate(d.date) } : null)).filter(Boolean) as { x: number; label: string }[];
  }, [window, innerW]);

  return (
    <div className="flex flex-col gap-4 animate-fade-in">
      <Card className="p-0 overflow-hidden">
        <CHeader className="px-4 pt-4 mb-2">
          <div>
            <CTitle className="inline-flex items-center gap-2"><TrendingUp size={14} className="text-[var(--color-brand)]" />趋势 · 每日用量</CTitle>
            <p className="mt-1 text-[10px] text-[var(--text-dim)]">
              {mode === "bars" ? "按时间范围的每日 Token 用量堆叠" : "窗口内累计 Token 总量增长曲线"}
            </p>
          </div>
          <div className="flex flex-wrap items-center justify-end gap-2">
            {mode === "bars" && <Segmented options={stackOptions} value={stackBy} onChange={setStackBy} />}
            <Segmented options={modeOptions} value={mode} onChange={setMode} />
            <Segmented options={rangeOptions as unknown as Array<{ value: number; label: string }>} value={days} onChange={(v) => setDays(v)} />
          </div>
        </CHeader>
        <CBody className="px-4 pb-4">
          {/* 初始化时显示加载中；仅在数据已加载且为空时显示无数据（垂直水平居中） */}
          {trendLoading ? (
            <div className="flex min-h-[280px] items-center justify-center text-sm text-[var(--text-tertiary)]">加载中…</div>
          ) : window.length === 0 ? (
            <div className="flex min-h-[280px] items-center justify-center text-sm text-[var(--text-tertiary)]">暂无趋势数据</div>
          ) : (
            <div className="overflow-x-auto relative">
              <svg viewBox={`0 0 ${W} ${H}`} className="w-full min-w-[560px] block" role="img" aria-label={mode === "bars" ? "趋势堆叠柱状图" : "累计 Token 总量曲线"}>
                <defs>
                  <linearGradient id="tm-cum-fill" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor="var(--color-brand)" stopOpacity="0.32" />
                    <stop offset="100%" stopColor="var(--color-brand)" stopOpacity="0.03" />
                  </linearGradient>
                </defs>

                {/* 横向网格线 + Y 轴刻度（整齐取整刻度，上限不低于真实最大值） */}
                {scaleTicks(mode === "bars" ? yMax : cumulativeYMax, mode === "bars" ? yStep : cumulativeStep).map((v) => {
                  const f = v / (mode === "bars" ? yMax : cumulativeYMax);
                  const y = PAD_T + innerH * (1 - f);
                  return (
                    <g key={v}>
                      <line x1={PAD_L} y1={y} x2={W - PAD_R} y2={y} stroke="var(--border)" strokeWidth={0.5} strokeDasharray={v === 0 ? "0" : "3 3"} />
                      <text x={PAD_L - 6} y={y + 3} textAnchor="end" fontSize={9} fill="var(--text-tertiary)">
                        {v === 0 ? "0" : fmtTick(v)}
                      </text>
                    </g>
                  );
                })}

                {/* 累计增长：面积 + 曲线 */}
                {mode === "cumulative" && cumulative.length > 0 && (() => {
                  const xOf = (i: number) => (cumulative.length === 1 ? PAD_L + innerW / 2 : PAD_L + (i * innerW) / (cumulative.length - 1));
                  const yOf = (v: number) => PAD_T + innerH - (innerH * v) / Math.max(1, cumulativeYMax);
                  const line = cumulative.map((c, i) => `${i === 0 ? "M" : "L"}${xOf(i).toFixed(2)},${yOf(c.cum).toFixed(2)}`).join(" ");
                  const area = `${line} L${xOf(cumulative.length - 1).toFixed(2)},${(PAD_T + innerH).toFixed(2)} L${xOf(0).toFixed(2)},${(PAD_T + innerH).toFixed(2)} Z`;
                  const slotW = innerW / Math.max(1, window.length);
                  return (
                    <g>
                      <path d={area} fill="url(#tm-cum-fill)" />
                      <path d={line} fill="none" stroke="var(--color-brand)" strokeWidth={1.6} strokeLinejoin="round" strokeLinecap="round" />
                      {/* 末端点高亮 */}
                      <circle cx={xOf(cumulative.length - 1)} cy={yOf(cumulative[cumulative.length - 1].cum)} r={3} fill="var(--color-brand)" stroke="var(--bg-surface)" strokeWidth={1.5} />
                      {/* 悬停热区（每日一列透明条） */}
                      {cumulative.map((c, i) => (
                        <rect
                          key={c.date}
                          x={PAD_L + i * slotW}
                          y={PAD_T}
                          width={slotW}
                          height={innerH}
                          fill="transparent"
                          onMouseEnter={(e) => onMove(e, c.date, [
                            { label: "累计", value: c.cum },
                            { label: "当日增量", value: c.delta },
                          ])}
                          onMouseMove={(e) => onMove(e, c.date, [
                            { label: "累计", value: c.cum },
                            { label: "当日增量", value: c.delta },
                          ])}
                          onMouseLeave={() => setHover(null)}
                        />
                      ))}
                    </g>
                  );
                })()}

                {/* 堆叠柱 */}
                {mode === "bars" && bars.map((bar) => (
                  <g key={bar.date}>
                    {bar.segments.map((s) => (
                      <rect
                        key={`${bar.date}-${s.key}`}
                        x={bar.x}
                        y={s.y}
                        width={bar.width}
                        height={Math.max(0.5, s.h)}
                        rx={Math.min(2, bar.width / 2)}
                        fill={colorOf(s.key)}
                        opacity={0.9}
                        onMouseEnter={(e) => onMove(e, bar.date, [
                          { label: "当日总量", value: bar.total },
                          { label: "请求数", value: bar.requests },
                          ...bar.segments.map((sg) => ({ label: sg.key, value: sg.tokens })),
                        ])}
                        onMouseMove={(e) => onMove(e, bar.date, [
                          { label: "当日总量", value: bar.total },
                          { label: "请求数", value: bar.requests },
                          ...bar.segments.map((sg) => ({ label: sg.key, value: sg.tokens })),
                        ])}
                        onMouseLeave={() => setHover(null)}
                      />
                    ))}
                  </g>
                ))}

                {/* X 轴日期刻度 */}
                {axisTicks.map((t) => (
                  <text key={t.label} x={t.x} y={H - 6} textAnchor="middle" fontSize={9} fill="var(--text-tertiary)">
                    {t.label}
                  </text>
                ))}
              </svg>

              {hover && (
                <div
                  className="pointer-events-none absolute z-20 min-w-[158px] rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)]/95 backdrop-blur-md px-3 py-2 text-[11px] shadow-[0_8px_28px_rgba(0,0,0,0.18)]"
                  style={{ left: Math.min(hover.x + 14, W - 168), top: Math.max(4, hover.y - 64) }}
                >
                  <div className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)] mb-1.5">
                    <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-brand)]" />
                    {hover.title}
                  </div>
                  <div className="space-y-1">
                    {hover.rows.map((r, i) => (
                      <div key={i} className="flex items-center justify-between gap-4 tabular-nums">
                        <span className="text-[var(--text-tertiary)]">{r.label}</span>
                        <span className="font-semibold text-[var(--text-primary)]">{fmtTokens(r.value)}</span>
                      </div>
                    ))}
                  </div>
                  <span className="absolute -bottom-1 left-6 w-2 h-2 rotate-45 border-b border-r border-[var(--border-subtle)] bg-[var(--bg-surface)]/95" />
                </div>
              )}
            </div>
          )}

          {/* 柱状图摘要：过去 N 天 · 活跃天数 · 峰值 */}
          {mode === "bars" && window.length > 0 && (
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-[var(--text-secondary)] tabular-nums border-t border-[var(--border)] pt-3">
              <span>过去 {days === 0 ? "全部" : `${days} 天`} · 活跃 <b className="font-semibold text-[var(--text-primary)]">{window.length}</b> 天</span>
              <span>峰值 <b className="font-semibold text-[var(--text-primary)]">{fmtTokens(Math.max(...window.map((d) => d.tokens), 0))}</b></span>

            </div>
          )}

          {/* 图例：色块 + 名称 + 总量 + 占比（仅堆叠柱模式） */}
          {mode === "bars" && legend.length > 0 && (
            <div className="mt-3 grid grid-cols-2 md:grid-cols-3 gap-x-6 gap-y-1.5 border-t border-[var(--border)] pt-3">
              {legend.map((r) => (
                <div key={r.key} className="flex items-center gap-2 text-[11px] min-w-0">
                  <span className="w-2.5 h-2.5 rounded-[3px] shrink-0" style={{ background: colorOf(r.key) }} />
                  <span className="truncate text-[var(--text-secondary)]">{r.key}</span>
                  <span className="ml-auto shrink-0 tabular-nums font-medium text-[var(--text-primary)]">{fmtTokens(r.value)}</span>
                  <span className="shrink-0 tabular-nums text-[var(--text-tertiary)] w-11 text-right">{r.pct.toFixed(1)}%</span>
                </div>
              ))}
            </div>
          )}

          {/* 累计模式摘要 */}
          {mode === "cumulative" && grandTotal > 0 && (
            <div className="mt-3 flex gap-4 text-[11px] text-[var(--text-secondary)] tabular-nums border-t border-[var(--border)] pt-3">
              <span>窗口累计 <b className="font-semibold text-[var(--text-primary)]">{fmtTokens(grandTotal)}</b></span>
              <span>{window.length} 天</span>
            </div>
          )}
        </CBody>
      </Card>
    </div>
  );
}
