import React, { useState } from "react";
import { Card, CTitle, CBody } from "@/components/ui/Card";
import { Spinner } from "@/components/ui/Spinner";
import { useAnalytics } from "@/hooks/use-tauri";
import { formatCost, formatTokensZh } from "@/lib/utils";
import ActivityHeatmap, { HeatmapLegend } from "@/components/ActivityHeatmap";
import {
  AreaChart, Area, BarChart, Bar, PieChart, Pie, Cell,
  XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer, Legend,
} from "recharts";
import { Activity, CalendarDays, CheckCircle2, Flame, Zap, DollarSign } from "lucide-react";

const COLORS = ["#0A84FF", "#64D2FF", "#30D158", "#FF9F0A", "#FF453A", "#BF5AF2", "#5E5CE6", "#FF375F"];

/** Helper: format a Date as YYYY-MM-DDThh:mm for <input type="datetime-local">. */
function toLocalDatetime(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export default function Analytics() {
  const [startDate, setStartDate] = useState<string>(() => {
    const d = new Date(); d.setDate(d.getDate() - 7); return toLocalDatetime(d);
  });
  const [endDate, setEndDate] = useState<string>(() => toLocalDatetime(new Date()));
  const { data: analytics, isLoading, isError } = useAnalytics(startDate, endDate);

  const daily = analytics?.daily || [];
  const modelDist = analytics?.model_distribution || [];
  const ranking = analytics?.account_ranking || [];
  const summary = analytics?.summary;
  const heatmap = analytics?.heatmap || [];

  const activeDays = daily.filter((day) => day.total_requests > 0).length;
  const topModel = modelDist[0];
  const topModelPct = topModel && summary?.total_requests
    ? Math.round((topModel.count / summary.total_requests) * 100)
    : 0;

  const usageCards = [
    {
      label: "Tokens 总量",
      value: formatTokensZh(summary?.total_tokens || 0),
      icon: <Flame size={16} />,
      sub: daily.length ? `${daily.length} 天窗口` : undefined,
    },
    {
      label: "活跃天数",
      value: String(activeDays),
      icon: <CalendarDays size={16} />,
      sub: daily.length ? `共 ${daily.length} 天有请求` : undefined,
    },
    {
      label: "最常用模型",
      value: topModel?.model || "--",
      icon: <Activity size={16} />,
      sub: topModel ? `占比 ${topModelPct}%` : "暂无使用",
    },
  ];

  const kpis = [
    { label: "总请求", value: (summary?.total_requests || 0).toLocaleString(), icon: <Activity size={16} /> },
    { label: "成功率", value: summary?.total_requests ? `${((summary.success_count / summary.total_requests) * 100).toFixed(1)}%` : "--", icon: <CheckCircle2 size={16} /> },
    { label: "平均延迟", value: summary?.avg_latency_ms ? `${(summary.avg_latency_ms / 1000).toFixed(1)}s` : "--", icon: <Zap size={16} /> },
    { label: "总费用", value: formatCost(summary?.total_cost || 0), icon: <DollarSign size={16} /> },
  ];

  return (
    <div className="space-y-5 animate-fade-in pg-page">
      <div className="pg-page-header flex items-center justify-between">
        <div>
          <div className="pg-eyebrow mb-1">Observability</div>
          <h2 style={{ color: "var(--text-primary)" }}>数据分析</h2>
          <p className="text-xs mt-0.5" style={{ color: "var(--text-dim)" }}>深度分析 API 使用趋势与账号健康度</p>
        </div>
        <div className="flex items-center gap-3">
          {/* Datetime range selector */}
          <div className="flex items-center gap-1.5">
            <input
              type="datetime-local"
              className="h-8 rounded-md px-2 text-xs border outline-none transition-colors focus:ring-2 focus:ring-[var(--color-brand)]/40"
              style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-default)", color: "var(--text-primary)" }}
              value={startDate}
              max={endDate}
              onChange={(e) => setStartDate(e.target.value)}
            />
            <span className="text-xs" style={{ color: "var(--text-dim)" }}>至</span>
            <input
              type="datetime-local"
              className="h-8 rounded-md px-2 text-xs border outline-none transition-colors focus:ring-2 focus:ring-[var(--color-brand)]/40"
              style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-default)", color: "var(--text-primary)" }}
              value={endDate}
              min={startDate}
              onChange={(e) => setEndDate(e.target.value)}
            />
          </div>
        </div>
      </div>

      {isError && (
        <div className="rounded-lg border px-4 py-3 text-sm" style={{ borderColor: "var(--color-err)", background: "var(--color-err-bg)", color: "var(--color-err)" }}>
          用量分析加载失败。请检查本地数据库状态后重试；页面不会使用历史缓存或模拟数据替代。
        </div>
      )}

      {/* Summary KPIs */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        {isLoading ? (
          <div className="col-span-4 flex items-center justify-center py-8"><Spinner /><span className="ml-2 text-sm" style={{ color: "var(--text-dim)" }}>加载中...</span></div>
        ) : kpis.map((k) => (
          <div key={k.label} className="p-4 rounded-lg border transition-all duration-200" style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-subtle)" }}>
            <div className="flex items-center gap-2 mb-2">
              <span className="flex items-center justify-center w-7 h-7 rounded-md bg-[var(--bg-surface)]" style={{ color: "var(--text-dim)" }}>
                {k.icon}
              </span>
              <span className="text-xs" style={{ color: "var(--text-dim)" }}>{k.label}</span>
            </div>
            <div className="text-xl font-bold" style={{ color: "var(--text-primary)" }}>{k.value}</div>
          </div>
        ))}
      </div>

      {/* 使用概览（截图效果：Tokens 总量 / 活跃天数 / 最常用模型） */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
        {isLoading ? (
          <div className="col-span-3 flex items-center justify-center py-8"><Spinner /><span className="ml-2 text-sm" style={{ color: "var(--text-dim)" }}>加载中...</span></div>
        ) : usageCards.map((card) => (
          <div key={card.label} className="p-4 rounded-lg border transition-all duration-200" style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-subtle)" }}>
            <div className="flex items-center gap-2 mb-2">
              <span className="flex items-center justify-center w-7 h-7 rounded-md bg-[var(--bg-surface)]" style={{ color: "var(--color-brand)" }}>
                {card.icon}
              </span>
              <span className="text-xs" style={{ color: "var(--text-dim)" }}>{card.label}</span>
            </div>
            <div className="text-xl font-bold truncate" style={{ color: "var(--text-primary)" }} title={card.value}>{card.value}</div>
            {card.sub && <div className="mt-0.5 text-[11px]" style={{ color: "var(--text-dim)" }}>{card.sub}</div>}
          </div>
        ))}
      </div>

      {/* 活跃热力图：按日 Tokens 用量着色，悬停或键盘聚焦查看当天消耗 */}
      <Card className="pg-analytics-heatmap-card">
        <div className="pg-analytics-heatmap-head">
          <div>
            <CTitle>活跃热力图</CTitle>
            <p>每日 Tokens 使用强度 · 最近 {Math.min(heatmap.length, 364)} 天</p>
          </div>
          <HeatmapLegend />
        </div>
        <CBody className="mt-4">
          {isLoading ? (
            <div className="flex items-center justify-center py-14"><Spinner /><span className="ml-2 text-sm" style={{ color: "var(--text-dim)" }}>加载中...</span></div>
          ) : heatmap.length === 0 ? (
            <div className="pg-analytics-heatmap-empty">
              <CalendarDays size={20} />
              <span>所选时间范围内暂无活跃记录</span>
            </div>
          ) : (
            <ActivityHeatmap
              data={heatmap}
              cellSize={13}
              gap={4}
              rounded={3}
              maxWeeks={52}
              showLabels
              fill
              showLegend={false}
              align="center"
            />
          )}
        </CBody>
      </Card>

      {/* Charts row */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        {/* Request trend */}
        <Card className="lg:col-span-2">
          <CTitle>请求趋势</CTitle>
          <CBody className="mt-2">
            {daily.length === 0 ? (
              <div className="flex items-center justify-center py-16 text-sm" style={{ color: "var(--text-dim)" }}>暂无数据</div>
            ) : (
              <div className="h-56">
                <ResponsiveContainer width="100%" height="100%">
                  <AreaChart data={daily}>
                    <defs>
                      <linearGradient id="gradSuccess" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="5%" stopColor="#22C55E" stopOpacity={0.3} />
                        <stop offset="95%" stopColor="#22C55E" stopOpacity={0} />
                      </linearGradient>
                      <linearGradient id="gradFail" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="5%" stopColor="#EF4444" stopOpacity={0.3} />
                        <stop offset="95%" stopColor="#EF4444" stopOpacity={0} />
                      </linearGradient>
                    </defs>
                    <CartesianGrid strokeDasharray="3 3" stroke="var(--border-subtle)" />
                    <XAxis dataKey="date" tick={{ fontSize: 10, fill: "var(--text-dim)" }} tickFormatter={(v) => v.slice(5)} />
                    <YAxis tick={{ fontSize: 10, fill: "var(--text-dim)" }} />
                    <Tooltip contentStyle={{ backgroundColor: "var(--bg-elevated)", border: "1px solid var(--border-default)", borderRadius: 6, fontSize: 12 }} />
                    <Legend />
                    <Area type="monotone" dataKey="success_count" stroke="#22C55E" fill="url(#gradSuccess)" strokeWidth={2} name="成功" />
                    <Area type="monotone" dataKey="error_count" stroke="#EF4444" fill="url(#gradFail)" strokeWidth={2} name="失败" />
                  </AreaChart>
                </ResponsiveContainer>
              </div>
            )}
          </CBody>
        </Card>

        {/* Model distribution */}
        <Card>
          <CTitle>模型分布</CTitle>
          <CBody className="mt-2">
            {modelDist.length === 0 ? (
              <div className="text-center py-12 text-xs" style={{ color: "var(--text-dim)" }}>暂无数据</div>
            ) : (
              <div className="h-52">
                <ResponsiveContainer width="100%" height="100%">
                  <PieChart>
                    <Pie data={modelDist} cx="50%" cy="50%" innerRadius={40} outerRadius={65} paddingAngle={3} dataKey="count" nameKey="model">
                      {modelDist.map((_: any, i: number) => <Cell key={i} fill={COLORS[i % COLORS.length]} />)}
                    </Pie>
                    <Tooltip contentStyle={{ backgroundColor: "var(--bg-elevated)", border: "1px solid var(--border-default)", borderRadius: 6, fontSize: 12 }} />
                  </PieChart>
                </ResponsiveContainer>
              </div>
            )}
            <div className="flex flex-wrap gap-2 mt-2">
              {modelDist.map((m: any, i: number) => (
                <div key={m.model} className="flex items-center gap-1.5 text-xs" style={{ color: "var(--text-secondary)" }}>
                  <span className="w-2 h-2 rounded-full" style={{ backgroundColor: COLORS[i % COLORS.length] }} />
                  {m.model}
                </div>
              ))}
            </div>
          </CBody>
        </Card>
      </div>

      {/* Account ranking + daily cost */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <Card>
          <CTitle>账号请求排行</CTitle>
          <CBody className="mt-2">
            {ranking.length === 0 ? (
              <div className="text-center py-12 text-sm" style={{ color: "var(--text-dim)" }}>暂无数据</div>
            ) : (
              <div className="h-64">
                <ResponsiveContainer width="100%" height="100%">
                  <BarChart data={ranking.slice(0, 8)} layout="vertical">
                    <CartesianGrid strokeDasharray="3 3" stroke="var(--border-subtle)" />
                    <XAxis type="number" tick={{ fontSize: 10, fill: "var(--text-dim)" }} />
                    <YAxis type="category" dataKey="name" width={100} tick={{ fontSize: 10, fill: "var(--text-dim)" }} />
                    <Tooltip contentStyle={{ backgroundColor: "var(--bg-elevated)", border: "1px solid var(--border-default)", borderRadius: 6, fontSize: 12 }} />
                    <Bar dataKey="count" fill="#0A84FF" radius={4} name="请求数" />
                  </BarChart>
                </ResponsiveContainer>
              </div>
            )}
          </CBody>
        </Card>

        <Card>
          <CTitle>每日费用</CTitle>
          <CBody className="mt-2">
            {daily.length === 0 ? (
              <div className="text-center py-12 text-sm" style={{ color: "var(--text-dim)" }}>暂无数据</div>
            ) : (
              <div className="h-64">
                <ResponsiveContainer width="100%" height="100%">
                  <BarChart data={daily}>
                    <CartesianGrid strokeDasharray="3 3" stroke="var(--border-subtle)" />
                    <XAxis dataKey="date" tick={{ fontSize: 10, fill: "var(--text-dim)" }} tickFormatter={(v) => v.slice(5)} />
                    <YAxis tick={{ fontSize: 10, fill: "var(--text-dim)" }} />
                    <Tooltip contentStyle={{ backgroundColor: "var(--bg-elevated)", border: "1px solid var(--border-default)", borderRadius: 6, fontSize: 12 }} />
                    <Bar dataKey="total_cost" fill="#3B82F6" radius={4} name="费用 ($)" />
                  </BarChart>
                </ResponsiveContainer>
              </div>
            )}
          </CBody>
        </Card>
      </div>
    </div>
  );
}
