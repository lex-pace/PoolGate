import React, { useMemo, useState } from "react";
import { Card, CBody, CHeader, CTitle } from "@/components/ui/Card";
import ActivityHeatmap, { type HeatmapDay } from "@/components/ActivityHeatmap";
import Segmented from "@/components/ui/Segmented";
import { CalendarDays, Clock, Flame, Gauge, Layers, MessageSquare, PiggyBank, Trophy, Wrench, Zap } from "lucide-react";
import {
  useTokenMonitorSnapshot, useToolUsage, useModelUsage, useActiveSessions, useTrend, formatTokens,
} from "@/components/token-monitor/token-monitor-data";
import ToolLogo, { ProviderLogo } from "@/components/token-monitor/ToolLogo";
import type { Range } from "@/lib/token-monitor-commands";

const MODEL_COLORS = ["#d98a5c", "#5b8def", "#37b6a0", "#d56a54", "#9b7fe0", "#c2a24d", "#5cc2d9", "#d97fb0"];

const rangeOptions: Array<{ value: Range; label: string }> = [
  { value: "day", label: "今日" },
  { value: "7d", label: "近 7 天" },
  { value: "month", label: "本月" },
  { value: "total", label: "累计" },
];

const metricOptions = [
  { value: "tokens" as const, label: "用量" },
  { value: "cost" as const, label: "成本" },
];

/** 本地今天（YYYY-MM-DD），用于热力图锚定右端。 */
function todayKey(): string {
  const now = new Date();
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

function formatUsd(cost?: number): string | null {
  if (cost == null) return null;
  return cost >= 1 ? `$${cost.toFixed(2)}` : `$${cost.toFixed(4)}`;
}

function formatInt(n: number): string {
  return Math.round(n).toLocaleString("en-US");
}

function activeTimeLabel(ms?: number): string {
  if (!ms || ms <= 0) return "—";
  const totalMin = Math.round(ms / 60000);
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

export default function Overview() {
  const [range, setRange] = useState<Range>("day");
  const { data: snapshot } = useTokenMonitorSnapshot(range);
  const { data: tools } = useToolUsage(range);
  const { data: models } = useModelUsage(range);
  const { data: sessions } = useActiveSessions();
  const { data: trend, isLoading: trendLoading } = useTrend(range);
  const [metric, setMetric] = useState<"tokens" | "cost">("tokens");

  // 趋势日序列升序（后端 DESC 返回）
  const dailyAsc = useMemo(() => [...(trend?.daily ?? [])].sort((a, b) => a.date.localeCompare(b.date)), [trend]);
  const heatmapData: HeatmapDay[] = useMemo(
    () => dailyAsc.map((d) => ({ date: d.date, tokens: d.tokens, requests: d.requests, cost: d.cost_amount ?? 0 })),
    [dailyAsc],
  );

  const usage = snapshot?.usage;
  const messageCount = trend?.message_count ?? (sessions ?? []).reduce((sum, s) => sum + (s.message_count || 0), 0);
  const topModel = models?.[0]?.model;

  const overviewCards = [
    { label: "总 TOKEN", value: usage ? formatTokens(usage.total_tokens) : "—", sub: "当前范围累计", icon: Zap, tone: "var(--color-brand)" },
    { label: "总花费", value: usage ? (formatUsd(usage.cost_amount) ?? "—") : "—", sub: "USD · 估算", icon: PiggyBank, tone: "var(--color-ok)" },
    { label: "活跃天数", value: `${trend?.active_days ?? 0}`, sub: "有 Token 活动", icon: CalendarDays, tone: "var(--color-info)" },
    { label: "连续天数", value: `${trend?.streak_days ?? 0}`, sub: "当前连续记录", icon: Flame, tone: "var(--color-warn)" },
    { label: "活跃时间", value: activeTimeLabel(trend?.active_time_ms), sub: "累计会话估算", icon: Clock, tone: "var(--color-info)" },
    { label: "峰值单日", value: trend?.peak_day ? formatTokens(trend.peak_day.tokens) : "—", sub: trend?.peak_day?.date ?? "暂无峰值", icon: Trophy, tone: "var(--color-warn)" },
    { label: "常用模型", value: topModel || "—", sub: "Top 1 by tokens", icon: Gauge, tone: "var(--color-brand)" },
    { label: "消息数", value: formatInt(messageCount), sub: "活跃会话合计", icon: MessageSquare, tone: "var(--color-ok)" },
  ];

  // 与开源一致：每列展示 Top 5
  const topTools = (tools ?? []).slice(0, 5);
  const topModels = (models ?? []).slice(0, 5);

  return (
    <div className="space-y-4 animate-fade-in">
      {/* 统计概览：范围切换在总览页顶部（今日/本月/累计 只影响下方统计与拆分） */}
      <section>
        <div className="flex items-center justify-between gap-3 mb-2 px-1">
          <span className="pg-eyebrow">统计概览</span>
          <Segmented options={rangeOptions} value={range} onChange={setRange} />
        </div>
        <div className="grid grid-cols-2 md:grid-cols-4 xl:grid-cols-8 gap-3">
          {overviewCards.map((item) => {
            const Icon = item.icon;
            return (
              <div key={item.label} className="pg-panel px-3.5 py-3.5 min-w-0">
                <div className="flex items-center justify-between gap-2">
                  <span className="pg-eyebrow truncate">{item.label}</span>
                  <Icon size={14} style={{ color: item.tone }} />
                </div>
                <div className="mt-2 text-[23px] leading-7 font-semibold tracking-[-0.025em] text-[var(--text-primary)] pg-mono truncate">{item.value}</div>
                <div className="mt-1 text-[10px] text-[var(--text-dim)] truncate">{item.sub}</div>
              </div>
            );
          })}
        </div>
      </section>

      {/* 滚动一年 Token 活动热力图：今天固定在右下角，数据早于一年则延伸到首次有数据（对齐开源） */}
      <Card className="p-0 overflow-hidden">
        <CHeader className="px-4 pt-4 mb-2">
          <div>
            <CTitle className="inline-flex items-center gap-2"><Flame size={14} className="text-[var(--color-brand)]" />Token 活动</CTitle>
            <p className="mt-1 text-[10px] text-[var(--text-dim)]">最近一年每日用量热力图 · 今天在最右侧 · 可左右滑动</p>
          </div>
          <Segmented options={metricOptions} value={metric} onChange={setMetric} />
        </CHeader>
        <CBody className="px-4 pb-4">
          {/* 初始化时显示加载中；仅在数据已加载且为空时显示无数据（垂直水平居中） */}
          {trendLoading ? (
            <div className="flex min-h-[220px] items-center justify-center text-sm text-[var(--text-tertiary)]">加载中…</div>
          ) : heatmapData.length === 0 ? (
            <div className="flex min-h-[220px] items-center justify-center text-sm text-[var(--text-tertiary)]">暂无活动数据</div>
          ) : (
            // fill 无上限：按窗口宽度让所有小方格占满整行（可左右滑动）
            <ActivityHeatmap data={heatmapData} showLabels metric={metric} fill minCellSize={10} windowEnd={todayKey()} windowDays={365} />
          )}
        </CBody>
      </Card>

      {/* 按模型 / 按工具 双列（与指挥中心区块一致） */}
      <section className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <Card className="p-0 overflow-hidden">
          <CHeader className="px-4 pt-4 mb-2">
            <CTitle className="inline-flex items-center gap-2"><Layers size={14} className="text-[var(--color-brand)]" />按模型</CTitle>
          </CHeader>
          <CBody className="px-4 pb-4 space-y-2.5">
            {topModels.length === 0 && <div className="py-8 text-center text-sm text-[var(--text-tertiary)]">暂无模型用量</div>}
            {topModels.map((m, i) => (
              <div key={m.model}>
                <div className="flex items-center justify-between gap-3 text-[12px]">
                  <span className="flex items-center gap-1.5 min-w-0">
                    <ProviderLogo model={m.model} size={14} />
                    <span className="truncate font-mono text-[var(--text-secondary)]">{m.model}</span>
                  </span>
                  <span className="shrink-0 tabular-nums font-medium">
                    {formatTokens(m.total_tokens)}
                    <em className="ml-1.5 not-italic text-[var(--text-tertiary)]">{Math.round(m.share_percent)}%</em>
                  </span>
                </div>
                <div className="mt-1 h-1 rounded-full bg-[var(--bg-elevated)] overflow-hidden">
                  <div className="h-full rounded-full" style={{ width: `${Math.max(2, m.share_percent)}%`, background: MODEL_COLORS[i % MODEL_COLORS.length] }} />
                </div>
              </div>
            ))}
          </CBody>
        </Card>
        <Card className="p-0 overflow-hidden">
          <CHeader className="px-4 pt-4 mb-2">
            <CTitle className="inline-flex items-center gap-2"><Wrench size={14} className="text-[var(--color-info)]" />按工具</CTitle>
          </CHeader>
          <CBody className="px-4 pb-4 space-y-2.5">
            {topTools.length === 0 && <div className="py-8 text-center text-sm text-[var(--text-tertiary)]">暂无工具用量</div>}
            {topTools.map((t, i) => (
              <div key={t.tool_id}>
                <div className="flex items-center justify-between gap-3 text-[12px]">
                  <span className="flex items-center gap-1.5 min-w-0">
                    <ToolLogo toolId={t.tool_id} displayName={t.display_name} size={14} />
                    <span className="truncate text-[var(--text-secondary)]">{t.display_name || t.tool_id}</span>
                  </span>
                  <span className="shrink-0 tabular-nums font-medium">
                    {formatTokens(t.total_tokens)}
                    <em className="ml-1.5 not-italic text-[var(--text-tertiary)]">{Math.round(t.share_percent)}%</em>
                  </span>
                </div>
                <div className="mt-1 h-1 rounded-full bg-[var(--bg-elevated)] overflow-hidden">
                  <div className="h-full rounded-full" style={{ width: `${Math.max(2, t.share_percent)}%`, background: MODEL_COLORS[i % MODEL_COLORS.length] }} />
                </div>
              </div>
            ))}
          </CBody>
        </Card>
      </section>
    </div>
  );
}
