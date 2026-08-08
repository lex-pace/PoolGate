import React, { useEffect, useState } from "react";
import { Card, CTitle, CBody } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Input } from "@/components/ui/Input";
import { Tabs } from "@/components/ui/Input";
import { Spinner } from "@/components/ui/Spinner";
import { useLogs, useLogStats } from "@/hooks/use-tauri";
import { formatCost } from "@/lib/utils";
import type { RequestLog } from "@/lib/tauri-commands";
import { Search, Download, ChevronLeft, ChevronRight, RefreshCw } from "lucide-react";

const timeRanges = [
  { id: "today", label: "今日" },
  { id: "6h", label: "6小时" },
  { id: "24h", label: "24小时" },
  { id: "7d", label: "7天" },
];

// 自动刷新间隔选项（秒），0 表示关闭。
const refreshIntervals = [
  { value: 0, label: "关闭" },
  { value: 3, label: "3秒" },
  { value: 5, label: "5秒" },
  { value: 10, label: "10秒" },
  { value: 30, label: "30秒" },
  { value: 60, label: "60秒" },
];

const AUTO_REFRESH_KEY = "pg_logs_auto_refresh";

export default function Logs() {
  const [page, setPage] = useState(1);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [timeRange, setTimeRange] = useState("today");
  const [expanded, setExpanded] = useState<string | null>(null);
  const [autoInterval, setAutoInterval] = useState<number>(() => {
    const saved = localStorage.getItem(AUTO_REFRESH_KEY);
    const parsed = saved ? Number(saved) : 0;
    return refreshIntervals.some((item) => item.value === parsed) ? parsed : 0;
  });

  useEffect(() => {
    localStorage.setItem(AUTO_REFRESH_KEY, String(autoInterval));
  }, [autoInterval]);

  const { data: logs = [], isLoading, refetch } = useLogs(
    {
      page,
      page_size: 10,
      range: timeRange as "today" | "6h" | "24h" | "7d",
      keyword: search || undefined,
      status: filter === "success" ? "success" : filter === "error" ? "failed" : undefined,
    },
    autoInterval > 0 ? autoInterval * 1000 : false,
  );

  const { data: stats, refetch: refetchStats } = useLogStats(timeRange as "today" | "6h" | "24h" | "7d");

  const totalLogs = logs.length;
  const hasMore = totalLogs === 10;

  const tabs = [
    { id: "all", label: "全部", count: stats?.total_requests || 0 },
    { id: "success", label: "成功", count: stats?.success_count || 0 },
    { id: "error", label: "失败", count: stats?.error_count || 0 },
  ];

  return (
    <div className="space-y-4 animate-fade-in pg-page">
      <div className="pg-page-header flex items-center justify-between">
        <div>
          <div className="pg-eyebrow mb-1">Request Stream</div>
          <h2 style={{ color: "var(--text-primary)" }}>请求日志</h2>
          <p className="text-xs mt-0.5" style={{ color: "var(--text-dim)" }}>
            实时记录所有 API 代理请求 · 总计 {totalLogs} 条
          </p>
        </div>
        <div className="flex items-center gap-2">
          {/* 自动刷新控件 */}
          <div
            className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md border"
            style={{ borderColor: "var(--border-default)", backgroundColor: "var(--bg-elevated)" }}
          >
            <span
              className="w-1.5 h-1.5 rounded-full"
              style={{ backgroundColor: autoInterval > 0 ? "var(--color-ok)" : "var(--text-dim)", boxShadow: autoInterval > 0 ? "0 0 6px var(--color-ok)" : "none" }}
            />
            <span className="text-[11px] whitespace-nowrap" style={{ color: "var(--text-dim)" }}>
              自动刷新
            </span>
            <select
              value={autoInterval}
              onChange={(e) => setAutoInterval(Number(e.target.value))}
              className="bg-transparent text-[11px] font-medium outline-none cursor-pointer"
              style={{ color: "var(--text-primary)" }}
              title="设置自动刷新间隔"
            >
              {refreshIntervals.map((item) => (
                <option key={item.value} value={item.value}>{item.label}</option>
              ))}
            </select>
          </div>
          <Button
            size="sm"
            variant="outline"
            disabled={isLoading}
            onClick={() => { void refetch(); void refetchStats(); }}
            title="刷新日志列表与统计"
          >
            <RefreshCw size={13} className={isLoading ? "animate-spin" : ""} /> 刷新
          </Button>
          <Button size="sm" variant="outline">
            <Download size={14} /> 导出日志
          </Button>
        </div>
      </div>

      {/* KPI row */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        {[
          { label: "总请求", value: stats?.total_requests?.toLocaleString() || "0", color: "var(--text-primary)" },
          { label: "成功率", value: stats?.total_requests ? `${((stats.success_count / stats.total_requests) * 100).toFixed(1)}%` : "--", color: "var(--color-ok)" },
          { label: "平均延迟", value: stats?.avg_latency_ms ? `${(stats.avg_latency_ms / 1000).toFixed(1)}s` : "--", color: "var(--text-secondary)" },
          { label: "总费用", value: formatCost(stats?.total_cost || 0), color: "var(--color-brand)" },
        ].map((k) => (
          <div key={k.label} className="p-3 rounded-md" style={{ backgroundColor: "var(--bg-elevated)" }}>
            <div className="text-xs" style={{ color: "var(--text-dim)" }}>{k.label}</div>
            <div className="text-lg font-bold mt-0.5" style={{ color: k.color }}>{k.value}</div>
          </div>
        ))}
      </div>

      {/* Filters */}
      <div className="flex items-center gap-3 flex-wrap">
        {/* Time range presets */}
        <div className="flex items-center gap-1 p-0.5 rounded-md" style={{ backgroundColor: "var(--bg-elevated)" }}>
          {timeRanges.map((tr) => (
            <button
              key={tr.id}
              onClick={() => { setTimeRange(tr.id); setPage(1); }}
              className={`px-3 py-1.5 text-xs rounded transition-all duration-150 cursor-pointer ${
                timeRange === tr.id
                  ? "bg-[var(--color-brand)] text-white"
                  : "text-[var(--text-dim)] hover:text-[var(--text-secondary)]"
              }`}
            >
              {tr.label}
            </button>
          ))}
        </div>

        <div className="relative flex-1 min-w-[180px] max-w-xs">
          <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2" style={{ color: "var(--text-dim)" }} />
          <Input className="pl-9" placeholder="搜索模型、端点..." value={search} onChange={(e) => { setSearch(e.target.value); setPage(1); }} />
        </div>
        <Tabs tabs={tabs} active={filter} onChange={(f) => { setFilter(f); setPage(1); }} className="flex-1" />
      </div>

      {/* Log table */}
      <Card className="p-0 overflow-hidden">
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr style={{ backgroundColor: "var(--bg-elevated)" }}>
                <th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider" style={{ color: "var(--text-dim)" }}>状态</th>
                <th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider" style={{ color: "var(--text-dim)" }}>时间</th>
                <th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider" style={{ color: "var(--text-dim)" }}>模型</th>
                <th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider" style={{ color: "var(--text-dim)" }}>路由</th>
                <th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider" style={{ color: "var(--text-dim)" }}>延迟</th>
                <th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider" style={{ color: "var(--text-dim)" }}>Token</th>
                <th className="px-4 py-3 text-left text-xs font-medium uppercase tracking-wider" style={{ color: "var(--text-dim)" }}>费用</th>
              </tr>
            </thead>
            <tbody>
              {isLoading ? (
                <tr><td colSpan={7} className="px-4 py-12 text-center"><Spinner /><span className="ml-2 text-sm" style={{ color: "var(--text-dim)" }}>加载中...</span></td></tr>
              ) : logs.length === 0 ? (
                <tr><td colSpan={7} className="px-4 py-12 text-center text-sm" style={{ color: "var(--text-dim)" }}>暂无日志</td></tr>
              ) : (
                logs.map((l: RequestLog) => {
                  const reqId = l.id?.toString() || l.request_at || "";
                  const isSuccess = l.status === "success" || (l.status_code && l.status_code >= 200 && l.status_code < 400);
                  return (
                    <React.Fragment key={reqId}>
                      <tr
                        className="border-t transition-colors cursor-pointer hover:bg-[var(--bg-hover)]"
                        style={{ borderColor: "var(--border-subtle)", backgroundColor: expanded === reqId ? "var(--bg-active)" : "transparent" }}
                        onClick={() => setExpanded(expanded === reqId ? null : reqId)}
                      >
                        <td className="px-4 py-2.5">
                          <Badge variant={isSuccess ? "ok" : "err"} dot>{isSuccess ? "200" : `${l.status_code || "ERR"}`}</Badge>
                        </td>
                        <td className="px-4 py-2.5 text-xs whitespace-nowrap" style={{ color: "var(--text-dim)" }}>{l.request_at ? new Date(l.request_at).toLocaleString("zh-CN", { hour12: false }) : "--"}</td>
                        <td className="px-4 py-2.5"><Badge variant="brand" className="text-xs">{l.model || "--"}</Badge></td>
                        <td className="px-4 py-2.5">
                          <div className="flex flex-col gap-0.5">
                            <span className="text-xs font-medium" style={{ color: "var(--text-primary)" }}>
                              {l.provider_name || (l.provider_id || "").slice(0, 8) || "--"}
                            </span>
                            <span className="text-xs font-mono" style={{ color: "var(--text-dim)" }}>
                              {l.account_name || (l.account_id || "").slice(0, 8)}
                            </span>
                          </div>
                        </td>
                        <td className="px-4 py-2.5 text-xs" style={{ color: (l.latency_ms || 0) > 3000 ? "var(--color-warn)" : "var(--text-secondary)" }}>
                          {l.latency_ms ? `${(l.latency_ms / 1000).toFixed(2)}s` : "--"}
                        </td>
                        <td className="px-4 py-2.5 text-xs" style={{ color: "var(--text-secondary)" }}>
                          {l.input_tokens || l.output_tokens ? `${((l.input_tokens || 0) + (l.output_tokens || 0)).toLocaleString()}` : "--"}
                        </td>
                        <td className="px-4 py-2.5 text-xs" style={{ color: "var(--text-secondary)" }}>
                          {l.cost ? `$${l.cost.toFixed(4)}` : "--"}
                        </td>
                      </tr>
                      {expanded === reqId && (
                        <tr>
                          <td colSpan={7} className="p-0">
                            <div className="animate-slide-down border-t" style={{ borderColor: "var(--border-subtle)" }}>
                              <div className="p-4 grid grid-cols-2 md:grid-cols-4 gap-4 text-xs" style={{ backgroundColor: "var(--bg-elevated)" }}>
                                <div className="space-y-1">
                                  <div style={{ color: "var(--text-dim)" }}>请求 ID</div>
                                  <div className="font-mono" style={{ color: "var(--text-primary)" }}>{l.id}</div>
                                </div>
                                <div className="space-y-1">
                                  <div style={{ color: "var(--text-dim)" }}>路由路径</div>
                                  <div style={{ color: "var(--text-primary)" }}>
                                    <span>{l.provider_name || l.provider_id || "--"}</span>
                                    <span style={{ color: "var(--text-dim)" }}> → </span>
                                    <span className="font-mono">{l.account_name || l.account_id || "--"}</span>
                                  </div>
                                </div>
                                <div className="space-y-1">
                                  <div style={{ color: "var(--text-dim)" }}>路由池</div>
                                  <div className="font-mono" style={{ color: "var(--text-primary)" }}>{l.group_name || l.group_id || "--"}</div>
                                </div>
                                <div className="space-y-1">
                                  <div style={{ color: "var(--text-dim)" }}>错误信息</div>
                                  <div style={{ color: l.error_message ? "var(--color-err)" : "var(--text-primary)" }}>{l.error_message || "无"}</div>
                                </div>
                                <div className="space-y-1">
                                  <div style={{ color: "var(--text-dim)" }}>Token 明细</div>
                                  <div style={{ color: "var(--text-primary)" }}>输入: {(l.input_tokens || 0).toLocaleString()} / 输出: {(l.output_tokens || 0).toLocaleString()}</div>
                                </div>
                                <div className="space-y-1">
                                  <div style={{ color: "var(--text-dim)" }}>费用</div>
                                  <div style={{ color: "var(--text-primary)" }}>${(l.cost || 0).toFixed(4)}</div>
                                </div>
                              </div>
                            </div>
                          </td>
                        </tr>
                      )}
                    </React.Fragment>
                  );
                })
              )}
            </tbody>
          </table>
        </div>
      </Card>

      {/* Pagination */}
      {(page > 1 || hasMore) && (
        <div className="flex items-center justify-center gap-2">
          <Button size="sm" variant="outline" disabled={page <= 1} onClick={() => setPage((p) => p - 1)}>
            <ChevronLeft size={14} /> 上一页
          </Button>
          <span className="text-sm px-3" style={{ color: "var(--text-dim)" }}>第 {page} 页</span>
          <Button size="sm" variant="outline" disabled={!hasMore} onClick={() => setPage((p) => p + 1)}>
            下一页 <ChevronRight size={14} />
          </Button>
        </div>
      )}
    </div>
  );
}
