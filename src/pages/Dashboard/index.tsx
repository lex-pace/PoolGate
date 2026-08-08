import { useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  useAccounts,
  useBatchDeleteAccounts,
  useGroups,
  useLogStats,
  useLogs,
  useProviders,
  useProxyStatus,
  useRouteTopology,
  useStartProxy,
  useStopProxy,
  useTraySnapshot,
} from "@/hooks/use-tauri";
import { useToast } from "@/components/ui/Toast";
import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import { Card, CBody, CHeader, CTitle } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { StatusDot } from "@/components/ui/StatusDot";
import TopologyView from "@/components/topology/TopologyView";
import { formatCost, getTimeAgo } from "@/lib/utils";
import type { Account, RequestLog } from "@/lib/tauri-commands";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  CircleDollarSign,
  Copy,
  KeyRound,
  Network,
  Play,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Square,
  TimerReset,
  Trash2,
  Waypoints,
} from "lucide-react";

function metric(value: number | undefined, fallback = "0") {
  return typeof value === "number" && Number.isFinite(value) ? value.toLocaleString() : fallback;
}

function logSucceeded(log: RequestLog) {
  return log.status === "success" || (!!log.status_code && log.status_code >= 200 && log.status_code < 400);
}

const accountSourceLabels: Record<string, string> = {
  api_key_text: "手动接入",
  codex_auth: "Codex / Codex Tools",
  codex_tools: "Codex Tools",
  sub2api: "Sub2API",
  cpa: "CPA / CLIProxyAPI",
  cockpit: "Cockpit Tools",
  echobird: "EchoBird",
  json: "JSON 导入",
  csv: "CSV 导入",
  oauth: "OAuth 浏览器授权",
};

function isUnavailableAccount(account: Account) {
  return account.status === "exhausted"
    || account.status === "error"
    || account.status === "token_expired"
    || account.health_status === "error";
}

function accountSource(account: Account) {
  return accountSourceLabels[account.source_format || ""]
    || account.source_format
    || (account.credential_type === "oauth" || account.credential_type === "codex_oauth" ? "OAuth 导入" : "手动接入");
}

export default function Dashboard({ focusNodeId }: { focusNodeId?: string }) {
  const { data: stats } = useLogStats("today");
  const { data: accounts = [] } = useAccounts();
  const { data: providers = [] } = useProviders();
  const { data: groups = [] } = useGroups();
  const { data: proxy } = useProxyStatus();
  const {
    data: routeTopology,
    isLoading: topologyLoading,
    isError: topologyError,
    dataUpdatedAt: topologyUpdatedAt,
  } = useRouteTopology();
  const { data: recentLogs = [] } = useLogs({ page: 1, page_size: 8, range: "today" });
  const { data: traySnapshot } = useTraySnapshot();
  const startProxy = useStartProxy();
  const stopProxy = useStopProxy();
  const batchDeleteAccounts = useBatchDeleteAccounts();
  const queryClient = useQueryClient();
  const { toast } = useToast();
  const [selectedUnavailableIds, setSelectedUnavailableIds] = useState<Set<string>>(new Set());
  const [deletingUnavailableMode, setDeletingUnavailableMode] = useState<"selected" | "all" | null>(null);
  const [pendingDelete, setPendingDelete] = useState<{ ids: string[]; mode: "selected" | "all" } | null>(null);

  const accountSummary = useMemo(() => {
    const active = accounts.filter((account) => account.status === "active" || account.health_status === "healthy").length;
    const limited = accounts.filter((account) => account.status === "limited").length;
    const errors = accounts.filter(isUnavailableAccount).length;
    return { total: accounts.length, active, limited, errors };
  }, [accounts]);

  const unavailableAccounts = useMemo(() => accounts.filter(isUnavailableAccount), [accounts]);
  const providersById = useMemo(() => new Map(providers.map((provider) => [provider.id, provider])), [providers]);

  useEffect(() => {
    const validIds = new Set(unavailableAccounts.map((account) => account.id));
    setSelectedUnavailableIds((current) => new Set([...current].filter((id) => validIds.has(id))));
  }, [unavailableAccounts]);

  const successRate = stats?.total_requests
    ? (stats.success_count / stats.total_requests) * 100
    : 0;
  const unhealthyCount = accountSummary.errors + accountSummary.limited;
  const topologyPoolCount = routeTopology?.pools.length ?? groups.length;
  const topologyProviderCount = routeTopology?.providers.length ?? 0;
  const routeAccounts = accounts.slice(0, 4);

  const trendData = useMemo(() => {
    const activity = traySnapshot?.activity || [];
    if (!activity.some((point) => point.requests > 0 || point.tokens > 0)) return [];
    return activity.map((point) => ({
      time: `${String(point.hour).padStart(2, "0")}:00`,
      throughput: point.requests,
      latency: point.avg_latency_ms,
    }));
  }, [traySnapshot?.activity]);

  const handleToggleProxy = async () => {
    try {
      if (proxy?.running) {
        await stopProxy.mutateAsync();
        toast("success", "网关已停止");
      } else {
        await startProxy.mutateAsync();
        toast("success", "网关已启动");
      }
    } catch (error: unknown) {
      toast("error", `操作失败: ${String(error)}`);
    }
  };

  const handleRestart = async () => {
    if (!proxy?.running) return;
    try {
      await stopProxy.mutateAsync();
      await startProxy.mutateAsync();
      toast("success", "网关已重启");
    } catch (error: unknown) {
      toast("error", `重启失败: ${String(error)}`);
    }
  };

  const handleRefresh = () => {
    void queryClient.invalidateQueries();
    toast("info", "监控数据已刷新");
  };

  const handleCopyEndpoint = () => {
    const endpoint = `http://127.0.0.1:${proxy?.port || 9800}`;
    void navigator.clipboard.writeText(endpoint);
    toast("success", "网关地址已复制");
  };

  const toggleUnavailableAccount = (accountId: string) => {
    setSelectedUnavailableIds((current) => {
      const next = new Set(current);
      if (next.has(accountId)) next.delete(accountId);
      else next.add(accountId);
      return next;
    });
  };

  const handleDeleteUnavailable = (ids: string[], mode: "selected" | "all") => {
    if (!ids.length) {
      toast("info", "请先勾选需要删除的不可用账号");
      return;
    }
    // Secondary confirmation: the user must explicitly confirm in the
    // in-app dialog before any account is permanently deleted.
    setPendingDelete({ ids, mode });
  };

  const confirmDeleteUnavailable = async () => {
    if (!pendingDelete) return;
    const { ids, mode } = pendingDelete;
    setDeletingUnavailableMode(mode);
    try {
      const result = await batchDeleteAccounts.mutateAsync(ids);
      setSelectedUnavailableIds((current) => {
        const next = new Set(current);
        result.deleted_ids.forEach((id) => next.delete(id));
        return next;
      });
      if (result.failures.length === 0) toast("success", `已删除 ${result.deleted_ids.length} 个不可用账号`);
      else if (result.deleted_ids.length > 0) toast("warning", `已删除 ${result.deleted_ids.length} 个，${result.failures.length} 个删除失败`);
      else toast("error", `${result.failures.length} 个账号删除失败：${result.failures[0]?.message || "未知错误"}`);
    } catch (error: unknown) {
      toast("error", `删除不可用账号失败：${String(error)}`);
    } finally {
      setDeletingUnavailableMode(null);
      setPendingDelete(null);
    }
  };

  const allUnavailableSelected = unavailableAccounts.length > 0
    && unavailableAccounts.every((account) => selectedUnavailableIds.has(account.id));

  return (
    <div className="space-y-4 animate-fade-in">
      <section
        className="pg-panel relative overflow-hidden px-5 py-4"
        style={{ background: "linear-gradient(120deg, var(--bg-surface) 0%, var(--bg-surface) 64%, var(--color-brand-subtle) 100%)" }}
      >
        <div className="absolute -right-20 -top-28 w-64 h-64 rounded-full bg-[var(--color-brand-subtle)] blur-3xl pointer-events-none" />
        <div className="relative flex items-center justify-between gap-6">
          <div className="flex items-center gap-4 min-w-0">
            <div
              className="w-12 h-12 rounded-[12px] flex items-center justify-center border shrink-0"
              style={{
                background: proxy?.running ? "var(--color-ok-bg)" : "var(--color-err-bg)",
                borderColor: proxy?.running ? "rgba(48,209,88,.24)" : "rgba(255,69,58,.24)",
                color: proxy?.running ? "var(--color-ok)" : "var(--color-err)",
              }}
            >
              <Waypoints size={24} strokeWidth={1.8} />
            </div>
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <span className={`status-dot ${proxy?.running ? "ok pulse-ok" : "err"}`} />
                <span className="pg-eyebrow">Gateway Runtime</span>
                <Badge variant={proxy?.running ? "ok" : "err"}>{proxy?.running ? "Operational" : "Stopped"}</Badge>
              </div>
              <h2 className="mt-1.5 text-[21px] leading-7 font-semibold tracking-[-0.025em] text-[var(--text-primary)]">
                {proxy?.running ? "所有路由正常工作" : "本地网关当前未运行"}
              </h2>
              <div className="mt-1.5 flex flex-wrap items-center gap-2 text-[11px] text-[var(--text-dim)]">
                <button onClick={handleCopyEndpoint} className="pg-mono hover:text-[var(--color-brand)] transition-colors inline-flex items-center gap-1">
                  127.0.0.1:{proxy?.port || 9800} <Copy size={10} />
                </button>
                <span>·</span>
                <span>OpenAI / Anthropic / Gemini compatible</span>
                <span>·</span>
                <span>{accountSummary.active}/{accountSummary.total} 个模型供应商可用</span>
              </div>
            </div>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <Button variant="outline" size="sm" onClick={handleRefresh}><RefreshCw size={12} />刷新</Button>
            <Button variant="secondary" size="sm" onClick={handleRestart} disabled={!proxy?.running}><RotateCcw size={12} />重启</Button>
            <Button
              variant={proxy?.running ? "danger" : "success"}
              size="sm"
              onClick={handleToggleProxy}
              loading={startProxy.isPending || stopProxy.isPending}
            >
              {proxy?.running ? <Square size={11} fill="currentColor" /> : <Play size={12} fill="currentColor" />}
              {proxy?.running ? "停止" : "启动"}
            </Button>
          </div>
        </div>
      </section>

      <section className="grid grid-cols-5 gap-3">
        {[
          { label: "今日请求", value: metric(stats?.total_requests), sub: `${stats?.error_count || 0} 个错误`, icon: Activity, tone: "var(--color-brand)" },
          { label: "成功率", value: stats?.total_requests ? `${successRate.toFixed(1)}%` : "--", sub: successRate >= 99 ? "SLO 正常" : "需要关注", icon: ShieldCheck, tone: successRate >= 99 ? "var(--color-ok)" : "var(--color-warn)" },
          { label: "活跃连接", value: metric(proxy?.active_connections), sub: `端口 ${proxy?.port || 9800}`, icon: Network, tone: "var(--color-info)" },
          { label: "平均延迟", value: stats?.avg_latency_ms ? `${Math.round(stats.avg_latency_ms)} ms` : "--", sub: `${stats?.timeout_count || 0} 次超时`, icon: TimerReset, tone: "var(--color-warn)" },
          { label: "预估费用", value: formatCost(stats?.total_cost || 0), sub: `${metric(stats?.total_tokens)} tokens`, icon: CircleDollarSign, tone: "var(--color-ok)" },
        ].map((item) => {
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
      </section>

      <section className="grid grid-cols-1 2xl:grid-cols-[minmax(0,1.65fr)_minmax(340px,.85fr)] gap-4">
        <Card className="p-0 overflow-hidden min-h-[430px]">
          <CHeader className="pg-topology-head mb-0">
            <div>
              <CTitle>实时路由拓扑</CTitle>
              <p className="mt-1 text-[10px] text-[var(--text-dim)]">Gateway → Protocol → Route Pool → Upstream Provider</p>
            </div>
            <span className="pg-topology-summary">
              1 GATEWAY · {routeTopology?.protocols.length || 0} PROTOCOLS · {topologyPoolCount} POOLS · {topologyProviderCount} PROVIDERS · {routeTopology?.gateway.active_connections || 0} ACTIVE · {topologyUpdatedAt ? new Date(topologyUpdatedAt).toLocaleTimeString("zh-CN", { hour12: false }) : "--:--:--"}
            </span>
          </CHeader>
          <CBody className="pg-topology pg-topology-v2">
            <TopologyView
              data={routeTopology}
              loading={topologyLoading}
              error={topologyError}
              onRefresh={handleRefresh}
              focusNodeId={focusNodeId}
              onToggleFullscreen={() => {
                if (window.location.hash !== "#/topology-fullscreen") {
                  window.location.hash = "#/topology-fullscreen";
                }
              }}
            />
          </CBody>
        </Card>

        <Card className="p-0 overflow-hidden min-h-[314px]">
          <CHeader className="px-4 pt-4 mb-0">
            <div>
              <CTitle className="inline-flex items-center gap-2"><Activity size={14} className="text-[var(--color-info)]" />吞吐与延迟</CTitle>
              <p className="mt-1 text-[10px] text-[var(--text-dim)]">过去 24 小时真实请求活跃度</p>
            </div>
            <Badge variant="info">24H</Badge>
          </CHeader>
          <CBody className="px-2 pt-2 pb-1">
            <div className="h-[236px]">
              {trendData.length > 0 ? (
                <ResponsiveContainer width="100%" height="100%">
                  <AreaChart data={trendData} margin={{ top: 10, right: 12, left: -22, bottom: 0 }}>
                    <defs>
                      <linearGradient id="throughputFill" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" stopColor="var(--color-brand)" stopOpacity={0.28} />
                        <stop offset="100%" stopColor="var(--color-brand)" stopOpacity={0} />
                      </linearGradient>
                      <linearGradient id="latencyFill" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" stopColor="var(--color-info)" stopOpacity={0.17} />
                        <stop offset="100%" stopColor="var(--color-info)" stopOpacity={0} />
                      </linearGradient>
                    </defs>
                    <CartesianGrid stroke="var(--border-subtle)" strokeDasharray="2 5" vertical={false} />
                    <XAxis dataKey="time" tick={{ fontSize: 9, fill: "var(--text-dim)" }} axisLine={false} tickLine={false} interval={5} />
                    <YAxis tick={{ fontSize: 9, fill: "var(--text-dim)" }} axisLine={false} tickLine={false} />
                    <Tooltip
                      contentStyle={{ background: "var(--bg-surface-solid)", border: "1px solid var(--border-default)", borderRadius: 9, fontSize: 10, boxShadow: "var(--shadow-elevated)" }}
                      labelStyle={{ color: "var(--text-primary)" }}
                    />
                    <Area type="monotone" dataKey="throughput" name="请求" stroke="var(--color-brand)" fill="url(#throughputFill)" strokeWidth={1.8} />
                    <Area type="monotone" dataKey="latency" name="延迟 ms" stroke="var(--color-info)" fill="url(#latencyFill)" strokeWidth={1.3} />
                  </AreaChart>
                </ResponsiveContainer>
              ) : (
                <div className="h-full flex flex-col items-center justify-center gap-1 text-center text-[var(--text-dim)]">
                  <Activity size={20} />
                  <span className="text-[10px]">过去 24 小时暂无请求数据</span>
                </div>
              )}
            </div>
          </CBody>
        </Card>
      </section>

      <section className="grid grid-cols-1 xl:grid-cols-[1fr_1.15fr_1.25fr] gap-4">
        <Card className="p-0 overflow-hidden min-h-[254px]">
          <CHeader className="px-4 pt-4 mb-2">
            <CTitle className="inline-flex items-center gap-2"><AlertTriangle size={14} className={unhealthyCount ? "text-[var(--color-warn)]" : "text-[var(--color-ok)]"} />路由告警</CTitle>
            <Badge variant={unhealthyCount ? "warn" : "ok"}>{unhealthyCount || "Clear"}</Badge>
          </CHeader>
          <CBody className="px-3 pb-3 space-y-1.5">
            {unhealthyCount === 0 ? (
              <div className="h-[178px] flex flex-col items-center justify-center text-center">
                <CheckCircle2 size={25} className="text-[var(--color-ok)]" />
                <div className="mt-2 text-[11px] font-medium text-[var(--text-primary)]">没有需要处理的告警</div>
                <div className="mt-1 text-[10px] text-[var(--text-dim)]">账号健康度与配额状态正常</div>
              </div>
            ) : (
              <>
                {accountSummary.errors > 0 && (
                  <div className="pg-panel-inset overflow-hidden">
                    <div className="flex items-center gap-2 border-b border-[var(--border-subtle)] px-3 py-2.5">
                      <StatusDot status="err" pulse />
                      <div className="min-w-0 flex-1">
                        <div className="text-[11px] font-medium text-[var(--text-primary)]">{accountSummary.errors} 个账号不可用</div>
                        <div className="mt-0.5 text-[9px] text-[var(--text-dim)]">已进入熔断、Token 过期或健康检查失败</div>
                      </div>
                      <Badge variant="err">P1</Badge>
                    </div>
                    <div className="flex items-center justify-between gap-2 border-b border-[var(--border-subtle)] bg-[var(--bg-inset)] px-3 py-1.5">
                      <label className="flex cursor-pointer select-none items-center gap-1.5 text-[9px] text-[var(--text-secondary)]">
                        <input
                          type="checkbox"
                          className="accent-[var(--color-brand)]"
                          checked={allUnavailableSelected}
                          onChange={() => setSelectedUnavailableIds(allUnavailableSelected ? new Set() : new Set(unavailableAccounts.map((account) => account.id)))}
                        />
                        全选 {unavailableAccounts.length}
                      </label>
                      <div className="flex items-center gap-1.5">
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-6 px-2 text-[9px]"
                          disabled={selectedUnavailableIds.size === 0}
                          loading={deletingUnavailableMode === "selected"}
                          onClick={() => handleDeleteUnavailable([...selectedUnavailableIds], "selected")}
                        >
                          <Trash2 size={10} />删除已选 {selectedUnavailableIds.size || ""}
                        </Button>
                        <Button
                          size="sm"
                          variant="danger"
                          className="h-6 px-2 text-[9px]"
                          loading={deletingUnavailableMode === "all"}
                          onClick={() => handleDeleteUnavailable(unavailableAccounts.map((account) => account.id), "all")}
                        >
                          一键删除全部
                        </Button>
                      </div>
                    </div>
                    <div className="max-h-[148px] overflow-y-auto">
                      {unavailableAccounts.map((account) => {
                        const providerName = account.provider_id ? providersById.get(account.provider_id)?.name || "未知厂商" : "未知厂商";
                        return (
                          <label key={account.id} className="flex cursor-pointer items-start gap-2 border-b border-[var(--border-subtle)] px-3 py-2 last:border-b-0 hover:bg-[var(--bg-hover)]">
                            <input
                              type="checkbox"
                              className="mt-0.5 shrink-0 accent-[var(--color-brand)]"
                              checked={selectedUnavailableIds.has(account.id)}
                              onChange={() => toggleUnavailableAccount(account.id)}
                            />
                            <div className="min-w-0 flex-1">
                              <div className="flex items-center gap-1.5">
                                <span className="truncate text-[10px] font-medium text-[var(--text-primary)]">{account.name || account.email || account.external_account_id || account.id.slice(0, 8)}</span>
                                <Badge variant="err">{account.status === "token_expired" ? "Token 过期" : account.status === "exhausted" ? "已耗尽" : "不可用"}</Badge>
                              </div>
                              <div className="mt-0.5 truncate text-[9px] text-[var(--text-dim)]">厂商：{providerName} · 来源：{accountSource(account)}</div>
                              {account.health_msg && <div className="mt-0.5 truncate text-[8px] text-[var(--color-err)]" title={account.health_msg}>{account.health_msg}</div>}
                            </div>
                          </label>
                        );
                      })}
                    </div>
                  </div>
                )}
                {accountSummary.limited > 0 && (
                  <div className="pg-panel-inset px-3 py-2.5 flex items-start gap-2.5">
                    <StatusDot status="warn" pulse />
                    <div className="min-w-0 flex-1">
                      <div className="text-[11px] font-medium text-[var(--text-primary)]">{accountSummary.limited} 个账号配额受限</div>
                      <div className="mt-0.5 text-[9px] text-[var(--text-dim)]">路由器将降低其分配优先级</div>
                    </div>
                    <Badge variant="warn">P2</Badge>
                  </div>
                )}
              </>
            )}
          </CBody>
        </Card>

        <Card className="p-0 overflow-hidden min-h-[254px]">
          <CHeader className="px-4 pt-4 mb-2">
            <CTitle className="inline-flex items-center gap-2"><KeyRound size={14} className="text-[var(--color-brand)]" />模型供应商负载</CTitle>
            <span className="text-[10px] text-[var(--text-dim)]">{accountSummary.active}/{accountSummary.total} available</span>
          </CHeader>
          <CBody className="px-3 pb-3 space-y-1.5">
            {routeAccounts.map((account: Account) => {
              const quota = account.quota_limit ? Math.min(100, ((account.quota_used || 0) / account.quota_limit) * 100) : null;
              const healthy = !isUnavailableAccount(account);
              return (
                <div key={account.id} className="pg-panel-inset px-3 py-2">
                  <div className="flex items-center gap-2">
                    <StatusDot status={healthy ? "ok" : "err"} />
                    <span className="text-[10px] font-medium text-[var(--text-primary)] truncate flex-1">{account.name || account.email || account.source_format || account.id.slice(0, 8)}</span>
                    <span className="pg-mono text-[9px] text-[var(--text-dim)]">{quota == null ? "未提供" : `${Math.round(quota)}% 已用`}</span>
                  </div>
                  {quota != null && (
                    <div className="mt-2 h-1 rounded-full overflow-hidden bg-[var(--bg-elevated)]">
                      <div className="h-full rounded-full" style={{ width: `${quota}%`, background: quota > 80 ? "var(--color-warn)" : "var(--color-brand)" }} />
                    </div>
                  )}
                </div>
              );
            })}
            {routeAccounts.length === 0 && (
              <div className="h-[170px] flex flex-col items-center justify-center text-center text-[var(--text-dim)]">
                <KeyRound size={23} />
                <span className="mt-2 text-[10px]">暂无模型供应商</span>
              </div>
            )}
          </CBody>
        </Card>

        <Card className="p-0 overflow-hidden min-h-[254px]">
          <CHeader className="px-4 pt-4 mb-2">
            <CTitle className="inline-flex items-center gap-2"><Activity size={14} className="text-[var(--color-ok)]" />实时请求流</CTitle>
            <span className="inline-flex items-center gap-1.5 text-[9px] text-[var(--text-dim)]"><span className="status-dot ok pulse-ok" />Live</span>
          </CHeader>
          <CBody className="px-3 pb-3">
            <div className="space-y-0.5">
              {recentLogs.slice(0, 6).map((log: RequestLog) => {
                const success = logSucceeded(log);
                return (
                  <div key={log.id ?? log.request_at} className="grid grid-cols-[50px_minmax(0,1fr)_50px_46px] gap-2 items-center px-2 py-1.5 rounded-md hover:bg-[var(--bg-hover)]">
                    <Badge variant={success ? "ok" : "err"}>{success ? log.status_code || 200 : log.status_code || "ERR"}</Badge>
                    <div className="min-w-0">
                      <div className="text-[10px] text-[var(--text-primary)] truncate">{log.model || log.endpoint || "Unknown model"}</div>
                      <div className="text-[8px] text-[var(--text-dim)] truncate">{log.source || log.group_id || "local"}</div>
                    </div>
                    <span className="pg-mono text-[9px] text-right text-[var(--text-secondary)]">{log.latency_ms ? `${log.latency_ms}ms` : "--"}</span>
                    <span className="text-[8px] text-right text-[var(--text-dim)]">{getTimeAgo(log.request_at)}</span>
                  </div>
                );
              })}
              {recentLogs.length === 0 && (
                <div className="h-[172px] flex flex-col items-center justify-center text-center text-[var(--text-dim)]">
                  <Activity size={23} />
                  <span className="mt-2 text-[10px]">等待新的网关请求</span>
                </div>
              )}
            </div>
          </CBody>
        </Card>
      </section>

      <ConfirmDialog
        open={!!pendingDelete}
        onClose={() => { if (!deletingUnavailableMode) setPendingDelete(null); }}
        onConfirm={() => void confirmDeleteUnavailable()}
        title="删除不可用账号"
        message={`确认永久删除${pendingDelete
          ? (pendingDelete.mode === "all" ? `全部 ${pendingDelete.ids.length} 个不可用账号` : `已勾选的 ${pendingDelete.ids.length} 个不可用账号`)
          : ""}？账号凭据及其路由池关联将一并移除，此操作无法撤销。`}
        confirmText="永久删除"
        cancelText="取消"
        variant="danger"
        loading={!!deletingUnavailableMode}
      />
    </div>
  );
}
