import React, { useMemo, useState } from "react";
import { Card, CBody } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { StatusDot } from "@/components/ui/StatusDot";
import { Input } from "@/components/ui/Input";
import { Modal } from "@/components/ui/Modal";
import { Spinner } from "@/components/ui/Spinner";
import {
  useAccounts,
  useClientKeys,
  useCreateClientKey,
  useCreateGroup,
  useDeleteClientKey,
  useDeleteGroup,
  useGroups,
  useProviders,
  useProxyStatus,
  useSetClientKeyPools,
  useUpdateClientKey,
  useRotateGroupClientKey,
  useEnsureGroupClientKey,
  useGroupModelResources,
  useAvailableGroupModelResources,
  useAddGroupModelResources,
  useSetGroupModelAccountIds,
  useRemoveGroupModelResource,
  useGroupDashboard,
  useAgentApps,
  usePreviewAgentAppConfig,
  useConfigureAndLaunchAgentApp,
  useRestoreAgentAppConfig,
  useUpdateGroup,
} from "@/hooks/use-tauri";
import { useToast } from "@/components/ui/Toast";
import type { Account, AgentAppInfo, AgentGroup, ClientKeyView } from "@/lib/tauri-commands";
import {
  Check,
  Copy,
  ExternalLink,
  Gauge,
  Bot,
  Box,
  Edit3,
  Pencil,
  PanelRightOpen,
  KeyRound,
  Layers3,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Route,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";

const protocolLabel: Record<string, string> = {
  openai: "OpenAI compatible",
  anthropic: "Anthropic Messages",
  both: "OpenAI + Anthropic",
  gemini: "Gemini",
  unified: "Unified Responses",
};

const strategyLabel: Record<string, string> = {
  round_robin: "轮询",
  weighted: "权重",
  least_latency: "最低延迟",
  quota_aware: "配额感知",
  failover: "优先级故障转移",
};

type PoolAddressBlock = {
  id: string;
  label: string;
  rows: Array<{ id: string; label: string; url: string }>;
};

function poolAddressBlocks(protocol: string, port: number): PoolAddressBlock[] {
  const origin = `http://127.0.0.1:${port}`;
  const openai: PoolAddressBlock = {
    id: "openai",
    label: "OpenAI compatible",
    rows: [
      { id: "base", label: "Base URL", url: `${origin}/v1` },
      { id: "chat", label: "Chat Completions", url: `${origin}/v1/chat/completions` },
      { id: "responses", label: "Responses", url: `${origin}/v1/responses` },
    ],
  };
  const anthropic: PoolAddressBlock = {
    id: "anthropic",
    label: "Anthropic Messages",
    rows: [
      { id: "base", label: "Base URL", url: origin },
      { id: "messages", label: "Messages", url: `${origin}/v1/messages` },
    ],
  };
  const gemini: PoolAddressBlock = {
    id: "gemini",
    label: "Gemini",
    rows: [
      { id: "base", label: "Base URL", url: origin },
      { id: "generate", label: "GenerateContent", url: `${origin}/v1beta/models/{model}:generateContent` },
    ],
  };

  switch (protocol.toLowerCase()) {
    case "openai":
    case "openai_compatible":
      return [openai];
    case "chat":
    case "chat_completions":
    case "openai_chat":
      return [{ ...openai, rows: openai.rows.slice(0, 2) }];
    case "responses":
    case "response":
    case "openai_responses":
    case "codex":
      return [{ ...openai, rows: [openai.rows[0], openai.rows[2]] }];
    case "anthropic":
    case "messages":
      return [anthropic];
    case "both":
      return [openai, anthropic];
    case "gemini":
      return [gemini];
    case "unified":
      return [openai, anthropic, gemini];
    default:
      return [{ id: "models", label: protocolLabel[protocol] || protocol, rows: [
        { id: "base", label: "Base URL", url: `${origin}/v1` },
        { id: "models", label: "Models", url: `${origin}/v1/models` },
      ] }];
  }
}

function parseModels(account: Account) {
  if (!account.models) return [];
  try {
    const value = JSON.parse(account.models);
    if (Array.isArray(value)) return value.map(String);
  } catch {
    // Legacy comma-separated value.
  }
  return account.models.split(",").map((item) => item.trim()).filter(Boolean);
}

function AgentAppIcon({ app }: { app: AgentAppInfo }) {
  const label = app.name.toLowerCase();
  if (label.includes("claude")) return <span className="text-[10px] font-bold">C</span>;
  if (label.includes("codex")) return <span className="text-[10px] font-bold">X</span>;
  if (label.includes("open")) return <span className="text-[10px] font-bold">O</span>;
  return <Bot size={14} />;
}

function PoolCard({
  group,
  providers,
  clientKeys,
  agentApps,
  lastLaunch,
  busyApp,
  removingModel,
  proxyPort,
  copied,
  onCopyText,
  onToggle,
  onEdit,
  onDelete,
  onCopyConfig,
  onManageModels,
  onCopyAllModels,
  onRotateKey,
  onRemoveModel,
  onOpenApp,
  onRestore,
}: {
  group: AgentGroup;
  providers: Array<{ id: string; name: string }>;
  clientKeys: ClientKeyView[];
  agentApps: AgentAppInfo[];
  lastLaunch: { appName: string; snapshotId: string; groupId: string } | null;
  busyApp: boolean;
  removingModel: boolean;
  proxyPort: number;
  copied: string | null;
  onCopyText: (text: string, tag: string) => void;
  onToggle: (group: AgentGroup) => void;
  onEdit: (group: AgentGroup) => void;
  onDelete: (group: AgentGroup) => void;
  onCopyConfig: (group: AgentGroup) => void;
  onManageModels: (groupId: string) => void;
  onCopyAllModels: (groupId: string, models: string[]) => void;
  onRotateKey: (groupId: string, hasKey: boolean) => void;
  onRemoveModel: (groupId: string, providerId: string, model: string) => void;
  onOpenApp: (appId: string, groupId: string) => void;
  onRestore: () => void;
}) {
  const { data: models = [], isLoading: modelsLoading, isError: modelsError } = useGroupModelResources(group.id);
  const { data: dashboard, isLoading: dashboardLoading } = useGroupDashboard(group.id);
  const managedKey = clientKeys.find((key) => key.managed_pool_id === group.id);
  const addressBlocks = useMemo(() => poolAddressBlocks(group.protocol, proxyPort), [group.protocol, proxyPort]);
  const providerMap = useMemo(() => new Map(providers.map((provider) => [provider.id, provider.name])), [providers]);
  const sortedModels = useMemo(() => {
    // 按模型名去重，保留第一个出现的（不同 provider），tooltip 中展示所有 provider
    const seen = new Map<string, { resource: typeof models[number]; providers: string[] }>();
    for (const resource of models) {
      const existing = seen.get(resource.model);
      if (existing) {
        existing.providers.push(providerMap.get(resource.provider_id) || resource.provider_id);
      } else {
        seen.set(resource.model, {
          resource,
          providers: [providerMap.get(resource.provider_id) || resource.provider_id],
        });
      }
    }
    return Array.from(seen.values())
      .map(({ resource, providers }) => ({ ...resource, _providers: providers }))
      .sort((left, right) => left.model.localeCompare(right.model));
  }, [models, providerMap]);

  const disabled = group.enabled === false;
  return (
    <Card className={`flex h-full flex-col overflow-hidden p-0 ${disabled ? "opacity-60" : ""}`}>
      <CBody className="flex h-full flex-col">
        {/* Header: Name + Status + Actions */}
        <div className="flex items-start justify-between gap-2 border-b px-3 py-2.5" style={{ borderColor: "var(--border-subtle)" }}>
          <div className="flex min-w-0 items-start gap-2.5">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-[var(--bg-inset)]"><StatusDot status={group.enabled ? "ok" : "mute"} pulse={group.enabled} /></div>
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="truncate text-[13px] font-semibold text-[var(--text-primary)]">{group.name}</span>
                <Badge variant="brand">{protocolLabel[group.protocol] || group.protocol}</Badge>
                {!group.enabled && <Badge variant="mute" dot>已停用</Badge>}
              </div>
              <p className="mt-0.5 truncate text-[9px] leading-4 text-[var(--text-dim)]" title={group.description || "按模型能力组织的资源池"}>{group.description || "按模型能力组织的资源池"}</p>
              <div className="flex flex-wrap gap-x-2.5 text-[9px] text-[var(--text-secondary)]">
                <span>{strategyLabel[group.strategy || "round_robin"] || group.strategy}</span>
                <span>{group.created_at?.slice(0, 10) || "--"}</span>
              </div>
            </div>
          </div>
          <div className="flex shrink-0 items-center">
            <button onClick={() => onEdit(group)} className="rounded-md p-1.5 text-[var(--text-dim)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]" title="编辑号池" aria-label={`编辑 ${group.name}`}><Pencil size={13} /></button>
            <button onClick={() => onToggle(group)} className={`rounded-md p-1.5 ${group.enabled ? "text-[var(--color-ok)] hover:bg-[var(--color-ok-bg)]" : "text-[var(--text-dim)] hover:bg-[var(--bg-hover)]"}`} title={group.enabled ? "停用号池" : "启用号池"}>{group.enabled ? <Power size={13} /> : <PowerOff size={13} />}</button>
            <button onClick={() => onCopyConfig(group)} className="rounded-md p-1.5 text-[var(--text-dim)] hover:bg-[var(--bg-hover)]" title="复制 Agent 接入配置"><Copy size={13} /></button>
            <button onClick={() => onDelete(group)} className="rounded-md p-1.5 text-[var(--text-dim)] hover:bg-[var(--color-err-bg)] hover:text-[var(--color-err)]" title="删除号池"><Trash2 size={13} /></button>
          </div>
        </div>

        {/* Main Content Area */}
        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-3 py-2.5">
          {/* Top Section: API Key + Stats + Provider Quota */}
          <section className="rounded-lg border" style={{ borderColor: "var(--border-subtle)" }}>
            {/* API Key */}
            <div className="flex items-center justify-between gap-2 border-b px-2.5 py-2" style={{ borderColor: "var(--border-subtle)" }}>
              <div className="flex min-w-0 items-center gap-2">
                <KeyRound size={12} className="shrink-0 text-[var(--color-brand)]" />
                <div className="min-w-0">
                  <code className="max-w-xs truncate text-[10px] text-[var(--text-secondary)] pg-mono" title={managedKey ? `${managedKey.key_prefix}••••••••${managedKey.key_last_four}` : "尚未生成"}>{managedKey ? `${managedKey.key_prefix}••••••••${managedKey.key_last_four}` : "尚未生成"}</code>
                </div>
              </div>
              <button onClick={() => onRotateKey(group.id, !!managedKey)} className="rounded bg-[var(--color-brand)] px-2 py-1 text-[9px] font-medium text-white transition-colors hover:bg-[var(--color-brand)]/80" title={managedKey ? "刷新替换号池专属 Key" : "生成号池专属 Key"}>{managedKey ? "刷新" : "生成"}</button>
            </div>
            {/* Stats Grid */}
            <div className="grid grid-cols-5 divide-x" style={{ borderColor: "var(--border-subtle)" }}>
              {[
                ["账号", dashboard?.resource_count],
                ["模型", dashboard?.model_count],
                ["请求", dashboard?.traffic.total_requests],
                ["Tokens", dashboard?.traffic.total_tokens],
                ["成功率", dashboard ? `${dashboard.traffic.success_rate.toFixed(1)}%` : undefined],
              ].map(([label, value]) => (
                <div key={String(label)} className="min-w-0 px-2 py-2 text-center" style={{ borderColor: "var(--border-subtle)" }}>
                  <div className="truncate text-[8px] uppercase tracking-wide text-[var(--text-dim)]">{label}</div>
                  <div className="truncate text-[11px] font-semibold leading-4 text-[var(--text-primary)] pg-mono">{dashboardLoading ? "…" : value ?? 0}</div>
                </div>
              ))}
            </div>
            {/* Provider Quota */}
            {dashboard?.quota_by_provider.length ? (
              <div className="border-t px-2.5 py-2" style={{ borderColor: "var(--border-subtle)" }}>
                <div className="mb-1.5 text-[9px] font-medium text-[var(--text-dim)]">PROVIDER 额度</div>
                <div className="space-y-1.5">
                  {dashboard.quota_by_provider.map((quota) => (
                    <div key={quota.provider_id} className="flex items-center gap-2">
                      <span className="w-20 shrink-0 truncate text-[9px] text-[var(--text-primary)]" title={quota.provider_name}>{quota.provider_name}</span>
                      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-[var(--bg-hover)]"><div className="h-full rounded-full" style={{ width: `${Math.min(100, Math.max(0, quota.average_used_percent))}%`, background: quota.average_used_percent >= 90 ? "var(--color-err)" : quota.average_used_percent >= 70 ? "var(--color-warn)" : "var(--color-ok)" }} /></div>
                      <span className={`shrink-0 text-[8px] ${quota.abnormal_accounts ? "text-[var(--color-warn)]" : "text-[var(--text-dim)]"}`}>{quota.account_count}账号</span>
                    </div>
                  ))}
                </div>
              </div>
            ) : null}
          </section>

          {/* Address Section */}
          <section>
            <div className="mb-1.5 flex items-center justify-between gap-2">
              <div className="pg-eyebrow">接入地址</div>
              <code className="text-[8px] text-[var(--text-dim)] pg-mono">127.0.0.1:{proxyPort}</code>
            </div>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              {addressBlocks.map((block) => (
                <div key={block.id} className="rounded-lg border bg-[var(--bg-inset)] p-2" style={{ borderColor: "var(--border-subtle)" }}>
                  <div className="mb-1.5 text-[10px] font-semibold text-[var(--text-primary)]">{block.label}</div>
                  <div className="space-y-1">
                    {block.rows.filter((row) => row.id !== "base").map((row) => {
                      const tag = `pool-url-${group.id}-${block.id}-${row.id}`;
                      return (
                        <div key={row.id} className="flex min-w-0 items-center gap-1.5">
                          <span className="w-[76px] shrink-0 text-[9px] text-[var(--text-dim)]">{row.label}</span>
                          <code className="min-w-0 flex-1 truncate text-[10px] text-[var(--text-primary)] pg-mono" title={row.url}>{row.url}</code>
                          <button type="button" onClick={() => onCopyText(row.url, tag)} className="shrink-0 rounded p-0.5 text-[var(--text-dim)] hover:bg-[var(--bg-hover)] hover:text-[var(--color-brand)]" title={`复制 ${row.label}`} aria-label={`复制 ${row.label}`}>{copied === tag ? <Check size={10} /> : <Copy size={10} />}</button>
                        </div>
                      );
                    })}
                  </div>
                </div>
              ))}
            </div>
          </section>

              {/* Models Section */}
          <section>
            <div className="flex items-center justify-between gap-2">
              <div className="pg-eyebrow">模型供应商 · {sortedModels.length}</div>
              <div className="flex items-center gap-0.5">
                <button disabled={!sortedModels.length} onClick={() => onCopyAllModels(group.id, sortedModels.map((resource) => resource.model))} className="inline-flex items-center gap-1 rounded-md px-1.5 py-1 text-[9px] font-medium text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--color-brand)] disabled:cursor-not-allowed disabled:opacity-40" title="选择分隔符并复制全部模型"><Copy size={10} /> 复制全部</button>
                <button onClick={() => onManageModels(group.id)} className="inline-flex items-center gap-1 rounded-md px-1.5 py-1 text-[9px] font-medium text-[var(--color-brand)] hover:bg-[var(--color-brand-subtle)]"><Plus size={11} /> 添加</button>
              </div>
            </div>
            <div className="mt-1 min-h-14 rounded-md border p-1.5" style={{ borderColor: "var(--border-subtle)" }}>
              {modelsLoading ? <div className="flex items-center justify-center py-3"><Spinner size={14} /></div> : modelsError ? (
                <div className="py-3 text-center text-[9px] text-[var(--color-err)]">模型供应商加载失败</div>
              ) : sortedModels.length ? (
                <div className="max-h-[92px] overflow-y-auto pr-0.5">
                  <div className="flex flex-wrap gap-1">
                    {sortedModels.map((resource) => {
                      const tag = `pool-model-${group.id}-${resource.model}`;
                      const providerNames = (resource as any)._providers?.join(", ") || providerMap.get(resource.provider_id) || "Provider";
                      return (
                        <span key={resource.model} className="inline-flex max-w-full items-center gap-0.5 rounded border bg-[var(--color-brand-subtle)] px-1.5 py-0.5 text-[9px] leading-4 text-[var(--text-primary)]" style={{ borderColor: "var(--color-brand)" }}>
                          <button type="button" onDoubleClick={() => onCopyText(resource.model, tag)} className="max-w-40 truncate text-left" title={`${providerNames} · ${resource.model}；双击复制模型名称`} aria-label={`双击复制模型 ${resource.model}`}>{copied === tag ? <span className="inline-flex items-center gap-0.5 text-[var(--color-ok)]"><Check size={9} />已复制</span> : resource.model}</button>
                          <button disabled={removingModel} onClick={() => onRemoveModel(group.id, resource.provider_id, resource.model)} className="rounded p-0.5 text-[var(--text-dim)] hover:bg-[var(--color-err-bg)] hover:text-[var(--color-err)] disabled:opacity-40" title={`从号池移除 ${resource.model}`} aria-label={`从号池移除 ${resource.model}`}><X size={9} /></button>
                        </span>
                      );
                    })}
                  </div>
                  <div className="mt-1 text-[8px] text-[var(--text-dim)]">双击模型名称可复制</div>
                </div>
              ) : (
                <div className="flex h-12 items-center justify-center gap-1.5 text-center"><Box size={14} className="text-[var(--color-warn)]" /><div><div className="text-[9px] text-[var(--color-warn)]">尚无模型供应商</div><div className="text-[8px] text-[var(--text-dim)]">空池不会回退全量账号</div></div></div>
              )}
            </div>
          </section>
        </div>

        {/* Footer: Agent Apps */}
        <div className="mt-auto flex flex-wrap items-center justify-between gap-x-3 gap-y-1.5 border-t px-3 py-2" style={{ borderColor: "var(--border-subtle)", background: "var(--bg-inset)" }}>
          <div className="flex items-center gap-1.5">
            {agentApps.map((app) => (
              <button key={app.app_id} disabled={!app.installed || busyApp || models.length === 0} onClick={() => onOpenApp(app.app_id, group.id)} className="flex h-6 w-6 items-center justify-center rounded-md border text-[var(--text-secondary)] transition-all enabled:hover:border-[var(--color-brand)] enabled:hover:bg-[var(--color-brand-subtle)] enabled:hover:text-[var(--color-brand)] disabled:cursor-not-allowed disabled:opacity-35" style={{ borderColor: "var(--border-default)" }} aria-label={`使用 ${group.name} 打开 ${app.name}`} title={app.installed ? models.length ? `切换到 ${group.name} 并打开 ${app.name}` : "请先添加模型供应商" : `${app.name} 未安装`}><AgentAppIcon app={app} /></button>
            ))}
            {lastLaunch?.groupId === group.id && <button className="ml-1 text-[9px] text-[var(--color-warn)] hover:underline" onClick={onRestore}>恢复 {lastLaunch.appName}</button>}
          </div>
        </div>
      </CBody>
    </Card>
  );
}

function CompactPoolCard({
  group,
  onOpen,
  onToggle,
  onEdit,
}: {
  group: AgentGroup;
  onOpen: () => void;
  onToggle: (group: AgentGroup) => void;
  onEdit: (group: AgentGroup) => void;
}) {
  const { data: dashboard, isLoading } = useGroupDashboard(group.id, "today");
  const { data: models = [] } = useGroupModelResources(group.id);
  const healthy = dashboard?.healthy_resource_count || 0;
  const total = dashboard?.resource_count || 0;
  const protocol = (group.protocol || "openai").toLowerCase();
  const protocolAccent: Record<string, string> = {
    openai: "from-emerald-400/60 via-emerald-500/30 to-transparent",
    anthropic: "from-orange-400/60 via-orange-500/30 to-transparent",
    both: "from-violet-400/60 via-violet-500/30 to-transparent",
    gemini: "from-sky-400/60 via-sky-500/30 to-transparent",
    unified: "from-blue-400/60 via-blue-500/30 to-transparent",
  };
  const accent = protocolAccent[protocol] || protocolAccent.openai;
  const disabled = group.enabled === false;
  return (
    <article
      className={`pg-pool-card group relative cursor-pointer overflow-hidden transition-all hover:-translate-y-0.5 hover:shadow-lg ${disabled ? "opacity-50 grayscale-[0.5]" : ""}`}
      onClick={onOpen}
      role="button"
      tabIndex={0}
      onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") onOpen(); }}
    >
      {disabled && (
        <div className="absolute inset-0 z-10 flex items-center justify-center pointer-events-none">
          <span className="px-3 py-1 rounded-full bg-[var(--text-dim)]/80 text-[var(--bg-primary)] text-[10px] font-semibold tracking-wide">已停用</span>
        </div>
      )}
      <div className={`absolute inset-x-0 top-0 h-1 bg-gradient-to-r ${accent}`} aria-hidden="true" />
      <div className="flex items-start justify-between gap-3 px-4 pb-3 pt-4">
        <div className="flex min-w-0 items-start gap-3">
          <div className="pg-pool-card-icon flex h-10 w-10 shrink-0 items-center justify-center rounded-xl"><Route size={18} /></div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2"><h3 className="truncate text-sm font-semibold text-[var(--text-primary)]">{group.name}</h3><Badge variant="brand">{protocolLabel[protocol] || group.protocol}</Badge></div>
            <p className="mt-1 truncate text-[10px] text-[var(--text-dim)]">{group.description || "按模型能力组织的资源池"}</p>
            <div className="mt-1.5 flex items-center gap-2 text-[9px] text-[var(--text-secondary)]"><span>{strategyLabel[group.strategy || "round_robin"]}</span><span>·</span><span>{models.length} 个模型</span></div>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button type="button" className="pg-card-action" title={group.enabled ? "停用路由池" : "启用路由池"} onClick={(event) => { event.stopPropagation(); onToggle(group); }}>{group.enabled ? <Power size={14} /> : <PowerOff size={14} />}</button>
          <button type="button" className="pg-card-action" title="编辑路由池" onClick={(event) => { event.stopPropagation(); onEdit(group); }}><Edit3 size={14} /></button>
          <button type="button" className="pg-card-action" title="查看路由池详情" onClick={(event) => { event.stopPropagation(); onOpen(); }}><PanelRightOpen size={14} /></button>
        </div>
      </div>
      <div className="grid grid-cols-4 border-t bg-[var(--bg-inset)]/60" style={{ borderColor: "var(--border-subtle)" }}>
        {[
          ["资源健康", isLoading ? "…" : `${healthy}/${total}`],
          ["今日请求", isLoading ? "…" : dashboard?.traffic.total_requests || 0],
          ["成功率", isLoading ? "…" : dashboard?.traffic.total_requests ? `${dashboard.traffic.success_rate.toFixed(1)}%` : "--"],
          ["今日 Tokens", isLoading ? "…" : dashboard?.traffic.total_tokens || 0],
        ].map(([label, value]) => <div key={String(label)} className="min-w-0 px-3 py-2.5"><div className="truncate text-[8px] uppercase tracking-wide text-[var(--text-dim)]">{label}</div><div className="mt-0.5 truncate text-[12px] font-semibold text-[var(--text-primary)] pg-mono">{value}</div></div>)}
      </div>
    </article>
  );
}

export default function RoutePools() {
  const { data: groups = [], isLoading, refetch: refetchGroups, isFetching: groupsFetching } = useGroups();
  const { data: accounts = [] } = useAccounts();
  const { data: providers = [] } = useProviders();
  const { data: proxyStatus } = useProxyStatus();
  const createGroup = useCreateGroup();
  const deleteGroup = useDeleteGroup();
  const updateGroup = useUpdateGroup();
  const { toast } = useToast();

  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [inspectorOpen, setInspectorOpen] = useState(false);

  // Virtual client keys → route pools.
  const { data: clientKeys = [], isLoading: keysLoading } = useClientKeys();
  const createKey = useCreateClientKey();
  const deleteKey = useDeleteClientKey();
  const setKeyPools = useSetClientKeyPools();
  const updateKey = useUpdateClientKey();
  const rotatePoolKey = useRotateGroupClientKey();
  const ensurePoolKey = useEnsureGroupClientKey();
  const addGroupModels = useAddGroupModelResources();
  const setGroupModelAccounts = useSetGroupModelAccountIds();
  const removeGroupModel = useRemoveGroupModelResource();
  const { data: groupModels = [] } = useGroupModelResources(selected);
  const { data: availableGroupModels = [], isLoading: availableModelsLoading, isError: availableModelsError, refetch: refetchAvailableModels } = useAvailableGroupModelResources(selected);
  const { data: agentApps = [] } = useAgentApps();
  const previewAgentApp = usePreviewAgentAppConfig();
  const launchAgentApp = useConfigureAndLaunchAgentApp();
  const restoreAgentApp = useRestoreAgentAppConfig();
  const [showCreate, setShowCreate] = useState(false);
  const [showMembers, setShowMembers] = useState(false);
  const [newName, setNewName] = useState("");
  const [newProtocol, setNewProtocol] = useState("openai");
  const [newStrategy, setNewStrategy] = useState("quota_aware");
  const [newDescription, setNewDescription] = useState("");
  // Access-key management state.
  const [showKeys, setShowKeys] = useState(false);
  const [keyName, setKeyName] = useState("");
  const [keyPoolIds, setKeyPoolIds] = useState<string[]>([]);
  const [createdRawKey, setCreatedRawKey] = useState<string | null>(null);
  const [createdPoolName, setCreatedPoolName] = useState<string | null>(null);
  const [providerDraft, setProviderDraft] = useState<string[]>([]);
  const [accountDraft, setAccountDraft] = useState<Record<string, string[]>>({});
  const [editingGroup, setEditingGroup] = useState<any | null>(null);
  const [editName, setEditName] = useState("");
  const [editDescription, setEditDescription] = useState("");
  const [editProtocol, setEditProtocol] = useState("openai");
  const [editStrategy, setEditStrategy] = useState("quota_aware");
  const [appPreview, setAppPreview] = useState<{ appId: string; appName: string; groupId: string; groupName: string; paths: string[]; backupRoot: string; warnings: string[] } | null>(null);
  const [lastLaunch, setLastLaunch] = useState<{ appName: string; snapshotId: string; groupId: string } | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [copyModelsDialog, setCopyModelsDialog] = useState<{ groupId: string; models: string[] } | null>(null);
  const [modelSeparator, setModelSeparator] = useState(",");
  const [customSeparator, setCustomSeparator] = useState("");

  const selectedGroup = groups.find((group) => group.id === selected);
  const list = groups.filter((group) => !search || `${group.name} ${group.description || ""} ${group.protocol}`.toLowerCase().includes(search.toLowerCase()));

  const modelResourcesByProvider = useMemo(() => {
    const grouped = new Map<string, typeof availableGroupModels>();
    availableGroupModels.forEach((resource) => {
      const current = grouped.get(resource.provider_id) || [];
      current.push(resource);
      grouped.set(resource.provider_id, current);
    });
    return Array.from(grouped.entries());
  }, [availableGroupModels]);

  const accountsByProvider = useMemo(() => {
    const grouped = new Map<string, {
      providerName: string;
      accounts: Map<string, { account: (typeof availableGroupModels)[number]["accounts"][number]; models: string[] }>;
    }>();
    availableGroupModels.forEach((resource) => {
      const entry = grouped.get(resource.provider_id) || { providerName: resource.provider_name, accounts: new Map() };
      resource.accounts.forEach((account) => {
        const acc = entry.accounts.get(account.id) || { account, models: [] };
        if (resource.already_added) acc.models.push(`${resource.model} · 已入池`);
        else acc.models.push(resource.model);
        entry.accounts.set(account.id, acc);
      });
      grouped.set(resource.provider_id, entry);
    });
    return Array.from(grouped.entries()).map(([providerId, value]) => ({
      providerId,
      providerName: value.providerName,
      // 对账号内的模型列表进行去重
      accounts: Array.from(value.accounts.values()).map((acc) => ({
        ...acc,
        models: Array.from(new Set(acc.models)),
      })),
    }));
  }, [availableGroupModels]);

  const providerAccountSelection = useMemo(() => {
    const map: Record<string, { allRoutable: string[]; healthy: string[]; selected: Set<string> }> = {};
    accountsByProvider.forEach(({ providerId, accounts }) => {
      const allRoutable = accounts.filter((item) => item.account.routable).map((item) => item.account.id);
      const healthy = accounts
        .filter((item) => item.account.routable && !["error", "exhausted", "token_expired"].includes(item.account.status))
        .map((item) => item.account.id);
      const selected = new Set<string>();
      accounts.forEach((item) => {
        if (item.account.selected) selected.add(item.account.id);
      });
      map[providerId] = { allRoutable, healthy, selected };
    });
    return map;
  }, [accountsByProvider]);

  React.useEffect(() => {
    setProviderDraft([]);
    setAccountDraft({});
  }, [selected]);

  React.useEffect(() => {
    setProviderDraft((current) => current.filter((providerId) =>
      availableGroupModels.some((resource) =>
        resource.provider_id === providerId && !resource.already_added,
      ),
    ));
  }, [availableGroupModels]);

  const handleCreate = async () => {
    if (!newName.trim()) {
      toast("warning", "请输入路由池名称");
      return;
    }
    try {
      const created = await createGroup.mutateAsync({
        id: crypto.randomUUID(),
        name: newName,
        description: newDescription || "按模型能力组织的资源池",
        protocol: newProtocol,
        strategy: newStrategy,
        enabled: true,
      });
      setCreatedRawKey(created.raw_key);
      setCreatedPoolName(created.group.name);
      setSelected(created.group.id);
      toast("success", `路由池「${newName}」与专属 Key 已创建`);
      setShowCreate(false);
      setNewName("");
      setNewDescription("");
    } catch (error) {
      toast("error", `创建失败：${String(error)}`);
    }
  };

  const handleToggle = async (group: any) => {
    try {
      await updateGroup.mutateAsync({ ...group, enabled: !group.enabled });
      toast("success", `路由池「${group.name}」已${group.enabled ? "停用" : "启用"}`);
    } catch (error) {
      toast("error", `操作失败：${String(error)}`);
    }
  };

  const handleDelete = async (group: any) => {
    if (!window.confirm(`确定删除路由池「${group.name}」？模型供应商本身不会被删除。`)) return;
    try {
      await deleteGroup.mutateAsync(group.id);
      if (selected === group.id) setSelected(null);
      toast("success", `路由池「${group.name}」已删除`);
    } catch (error) {
      toast("error", `删除失败：${String(error)}`);
    }
  };

  const handleCopyConfig = (group: AgentGroup) => {
    const origin = `http://127.0.0.1:${proxyStatus?.port || 9800}`;
    const config = group.protocol === "anthropic"
      ? `export ANTHROPIC_BASE_URL=${origin}`
      : group.protocol === "gemini"
        ? `export GEMINI_BASE_URL=${origin}`
        : group.protocol === "openai"
          ? `export OPENAI_BASE_URL=${origin}/v1`
          : `export OPENAI_BASE_URL=${origin}/v1\nexport ANTHROPIC_BASE_URL=${origin}`;
    navigator.clipboard.writeText(config).then(() => toast("success", "Agent 网关配置已复制"));
  };

  const copyText = async (text: string, tag: string) => {
    await navigator.clipboard.writeText(text);
    setCopied(tag);
    setTimeout(() => setCopied(null), 1500);
  };

  const openCopyModelsDialog = (groupId: string, models: string[]) => {
    setCopyModelsDialog({ groupId, models: Array.from(new Set(models)) });
    setModelSeparator(",");
    setCustomSeparator("");
  };

  const handleCopyAllModels = async () => {
    if (!copyModelsDialog) return;
    const separator = modelSeparator === "custom" ? customSeparator : modelSeparator;
    if (!separator) {
      toast("warning", "请输入自定义分隔符");
      return;
    }
    await copyText(copyModelsDialog.models.join(separator), `pool-models-${copyModelsDialog.groupId}`);
    toast("success", `已复制 ${copyModelsDialog.models.length} 个模型名称`);
    setCopyModelsDialog(null);
  };

  const handleCreateKey = async () => {
    if (!keyName.trim()) {
      toast("warning", "请输入密钥名称");
      return;
    }
    try {
      const result = await createKey.mutateAsync({ name: keyName, poolIds: keyPoolIds });
      setCreatedRawKey(result.raw_key);
      setKeyName("");
      setKeyPoolIds([]);
      toast("success", "接入密钥已创建，请立即复制保存");
    } catch (error) {
      toast("error", `创建失败：${String(error)}`);
    }
  };

  const handleRotatePoolKey = async (groupId: string, hasKey: boolean) => {
    const groupName = groups.find((group) => group.id === groupId)?.name || "路由池";
    const message = hasKey
      ? "刷新后旧 Key 将立即失效，正在使用它的客户端会立刻断开。确定继续？"
      : "将为这个历史路由池生成专属 Key。Key 只显示一次，确定继续？";
    if (!window.confirm(message)) return;
    try {
      const result = hasKey
        ? await rotatePoolKey.mutateAsync(groupId)
        : await ensurePoolKey.mutateAsync(groupId);
      setCreatedRawKey(result.raw_key);
      setCreatedPoolName(groupName);
      toast("success", hasKey ? "专属 Key 已刷新，旧 Key 已失效" : "专属 Key 已生成");
    } catch (error) {
      toast("error", `Key 操作失败：${String(error)}`);
    }
  };

  const handleToggleProvider = (providerId: string, availableCount: number) => {
    if (!availableCount) return;
    setProviderDraft((current) => current.includes(providerId)
      ? current.filter((item) => item !== providerId)
      : [...current, providerId]);
  };

  const toggleAccountSelection = (providerId: string, accountId: string) => {
    setAccountDraft((current) => {
      const key = providerId;
      const providerInfo = providerAccountSelection[providerId];
      const previous = current[key] !== undefined ? new Set(current[key]) : new Set(providerInfo.selected);
      if (previous.has(accountId)) previous.delete(accountId);
      else previous.add(accountId);
      return { ...current, [key]: Array.from(previous) };
    });
  };

  const selectAllHealthyAccounts = (providerId: string) => {
    const info = providerAccountSelection[providerId];
    if (!info) return;
    setAccountDraft((current) => ({ ...current, [providerId]: info.healthy }));
  };

  const clearAccountSelection = (providerId: string) => {
    setAccountDraft((current) => ({ ...current, [providerId]: [] }));
  };

  const resolveAccountSelection = (providerId: string): string[] | undefined => {
    const draft = accountDraft[providerId];
    if (draft !== undefined) return draft;
    const info = providerAccountSelection[providerId];
    if (!info) return undefined;
    if (info.selected.size === info.allRoutable.length) return undefined;
    return Array.from(info.selected);
  };

  const selectedProviderResources = useMemo(() => availableGroupModels.filter((resource) =>
    providerDraft.includes(resource.provider_id) && !resource.already_added,
  ), [availableGroupModels, providerDraft]);

  const resourceKey = (providerId: string, model: string) => `${providerId}:${model}`;

  const toggleDraftAccount = (resource: (typeof availableGroupModels)[number], accountId: string) => {
    const key = resourceKey(resource.provider_id, resource.model);
    setAccountDraft((current) => {
      const existing = current[key] || resource.accounts.filter((account) => account.selected).map((account) => account.id);
      const next = existing.includes(accountId)
        ? existing.filter((id) => id !== accountId)
        : [...existing, accountId];
      return { ...current, [key]: next };
    });
  };

  const toggleHealthyAccounts = (resource: (typeof availableGroupModels)[number]) => {
    const key = resourceKey(resource.provider_id, resource.model);
    const healthy = resource.accounts
      .filter((account) => account.routable && !["error", "exhausted", "token_expired"].includes(account.status))
      .map((account) => account.id);
    setAccountDraft((current) => ({
      ...current,
      [key]: healthy,
    }));
  };

  const handleAddProviders = async () => {
    if (!selected) return;
    const resources = selectedProviderResources.map(({ provider_id, model }) => ({ provider_id, model }));
    if (!resources.length) {
      toast("warning", "请至少选择一个有可用模型的厂商");
      return;
    }
    try {
      const added = await addGroupModels.mutateAsync({ groupId: selected, resources });
      for (const providerId of providerDraft) {
        const ids = resolveAccountSelection(providerId);
        if (!ids || ids.length === 0) continue;
        const providerResources = selectedProviderResources.filter((resource) => resource.provider_id === providerId);
        for (const resource of providerResources) {
          await setGroupModelAccounts.mutateAsync({
            groupId: selected,
            providerId: resource.provider_id,
            model: resource.model,
            accountIds: ids,
          });
        }
      }
      toast(added === 0 ? "info" : "success", added === 0 ? "所选资源已在路由池内，账号约束已更新" : `已加入 ${added} 个模型供应商，并保存账号选择`);
      setProviderDraft([]);
      setAccountDraft({});
      setShowMembers(false);
    } catch (error) {
      toast("error", `添加厂商模型供应商失败：${String(error)}`);
    }
  };

  const handleRemoveModel = async (groupId: string, providerId: string, model: string) => {
    if (!window.confirm(`确定从号池移除模型「${model}」？模型供应商本身不会被删除。`)) return;
    try {
      const removed = await removeGroupModel.mutateAsync({ groupId, providerId, model });
      toast(removed ? "success" : "info", removed ? `模型「${model}」已从号池移除` : "该模型供应商已不在号池内");
    } catch (error) {
      toast("error", `移除模型供应商失败：${String(error)}`);
    }
  };

  const openEditGroup = (group: any) => {
    setEditingGroup(group);
    setEditName(group.name);
    setEditDescription(group.description || "");
    setEditProtocol(group.protocol);
    setEditStrategy(group.strategy || "round_robin");
  };

  const handleEditGroup = async () => {
    if (!editingGroup || !editName.trim()) {
      toast("warning", "请输入号池名称");
      return;
    }
    try {
      await updateGroup.mutateAsync({
        ...editingGroup,
        name: editName.trim(),
        description: editDescription.trim(),
        protocol: editProtocol,
        strategy: editStrategy,
      });
      toast("success", `号池「${editName.trim()}」已更新`);
      setEditingGroup(null);
    } catch (error) {
      toast("error", `编辑失败：${String(error)}`);
    }
  };

  const handlePreviewApp = async (appId: string, groupId = selected) => {
    if (!groupId) return;
    const groupName = groups.find((group) => group.id === groupId)?.name || "当前号池";
    try {
      const preview = await previewAgentApp.mutateAsync({ appId, groupId });
      setAppPreview({
        appId,
        appName: preview.app.name,
        groupId,
        groupName,
        paths: preview.affected_paths,
        backupRoot: preview.backup_root,
        warnings: preview.warnings,
      });
    } catch (error) {
      toast("error", `无法预览应用配置：${String(error)}`);
    }
  };

  const handleLaunchApp = async () => {
    if (!appPreview) return;
    try {
      const result = await launchAgentApp.mutateAsync({ appId: appPreview.appId, groupId: appPreview.groupId, confirmed: true });
      setLastLaunch({ appName: appPreview.appName, snapshotId: result.snapshot_id, groupId: appPreview.groupId });
      toast(result.launched ? "success" : "warning", result.launched ? "配置已切换并启动应用" : "配置已切换，但应用启动失败，可从备份恢复");
      setAppPreview(null);
    } catch (error) {
      toast("error", `打开应用失败：${String(error)}`);
    }
  };

  const handleRestoreLastLaunch = async () => {
    if (!lastLaunch) return;
    try {
      await restoreAgentApp.mutateAsync({ snapshotId: lastLaunch.snapshotId });
      toast("success", `${lastLaunch.appName} 原配置已恢复`);
      setLastLaunch(null);
    } catch (error) {
      if (String(error).includes("APP_CONFIG_CHANGED")) {
        const force = window.confirm("检测到应用配置在 PoolGate 改写后又被修改。强制恢复会覆盖这些新改动，是否继续？");
        if (!force) return;
        try {
          await restoreAgentApp.mutateAsync({ snapshotId: lastLaunch.snapshotId, force: true });
          toast("success", `${lastLaunch.appName} 原配置已强制恢复`);
          setLastLaunch(null);
        } catch (forceError) {
          toast("error", `恢复失败：${String(forceError)}`);
        }
        return;
      }
      toast("error", `恢复失败：${String(error)}`);
    }
  };

  const closeCreatedKey = () => {
    setCreatedRawKey(null);
    setCreatedPoolName(null);
  };

  const handleToggleKey = async (key: ClientKeyView) => {
    try {
      await updateKey.mutateAsync({ id: key.id, enabled: !key.enabled });
      toast("success", `密钥「${key.name}」已${key.enabled ? "停用" : "启用"}`);
    } catch (error) {
      toast("error", `操作失败：${String(error)}`);
    }
  };

  const handleDeleteKey = async (key: ClientKeyView) => {
    if (!window.confirm(`确定删除接入密钥「${key.name}」？使用该密钥的客户端将立即失去访问。`)) return;
    try {
      await deleteKey.mutateAsync(key.id);
      toast("success", "接入密钥已删除");
    } catch (error) {
      toast("error", `删除失败：${String(error)}`);
    }
  };

  const handleBindPool = async (key: ClientKeyView, poolId: string) => {
    try {
      const next = key.pool_ids.includes(poolId)
        ? key.pool_ids.filter((id) => id !== poolId)
        : [...key.pool_ids, poolId];
      await setKeyPools.mutateAsync({ clientKeyId: key.id, poolIds: next });
      toast("success", `密钥「${key.name}」的绑定池已更新`);
    } catch (error) {
      toast("error", `绑定失败：${String(error)}`);
    }
  };

  const totalActiveResources = accounts.filter((account) => account.status !== "disabled" && ["api_key", "upstream_key", "oauth", "token", "codex_oauth"].includes(account.credential_type || "api_key")).length;

  return (
    <div className="space-y-4 animate-fade-in pg-page">
      <section className="pg-panel px-5 py-4">
        <div className="flex items-center justify-between gap-5">
          <div>
            <div className="pg-eyebrow mb-1">Capability routing pools</div>
            <h2 className="text-[20px] leading-7 font-semibold tracking-[-0.02em] text-[var(--text-primary)]">路由池</h2>
            <p className="mt-1 text-[11px] text-[var(--text-dim)]">按协议和模型能力组织资源；路由器根据健康、配额、延迟与权重选择上游。</p>
          </div>
          <div className="flex gap-2">
            <Button size="sm" variant="outline" onClick={() => setShowKeys(true)}><KeyRound size={14} /> 接入密钥</Button>
            <Button size="sm" onClick={() => setShowCreate(true)}><Plus size={14} /> 新建路由池</Button>
            <Button size="sm" variant="outline" onClick={() => refetchGroups()} disabled={groupsFetching}><RefreshCw size={14} className={groupsFetching ? "animate-spin" : ""} /> 刷新</Button>
          </div>
        </div>
      </section>

      <section className="grid grid-cols-4 gap-3">
        {[
          { label: "路由池", value: groups.length, sub: "能力池数量", icon: Route, tone: "var(--color-brand)" },
          { label: "可用资源", value: totalActiveResources, sub: "可参与调度", icon: Layers3, tone: "var(--color-ok)" },
          { label: "模型能力", value: new Set(accounts.flatMap(parseModels)).size || "--", sub: "跨来源模型", icon: ShieldCheck, tone: "var(--color-info)" },
          { label: "调度策略", value: new Set(groups.map((group) => group.strategy || "round_robin")).size, sub: "按池独立配置", icon: Gauge, tone: "var(--color-warn)" },
        ].map((item) => {
          const Icon = item.icon;
          return (
            <div key={item.label} className="pg-panel px-3.5 py-3">
              <div className="flex items-center justify-between"><span className="pg-eyebrow">{item.label}</span><Icon size={14} style={{ color: item.tone }} /></div>
              <div className="mt-2 text-[22px] pg-mono font-semibold text-[var(--text-primary)]">{item.value}</div>
              <div className="mt-1 text-[9px] text-[var(--text-dim)]">{item.sub}</div>
            </div>
          );
        })}
      </section>

      <div className="relative max-w-xs"><Input placeholder="搜索路由池、协议或能力..." value={search} onChange={(event) => setSearch(event.target.value)} /></div>

      {isLoading ? (
        <div className="flex items-center justify-center py-16"><Spinner size={24} /><span className="ml-2 text-sm text-[var(--text-dim)]">加载中...</span></div>
      ) : list.length === 0 ? (
        <div className="pg-panel text-center py-16 text-sm text-[var(--text-dim)]"><Route size={26} className="mx-auto mb-2" />暂无路由池，创建后可按模型能力加入资源</div>
      ) : (
        <div className="grid grid-cols-1 items-stretch gap-3 md:grid-cols-2 xl:grid-cols-3">
          {list.map((group) => (
            <CompactPoolCard
              key={group.id}
              group={group}
              onToggle={handleToggle}
              onOpen={() => { setSelected(group.id); setInspectorOpen(true); }}
              onEdit={() => openEditGroup(group)}
            />
          ))}
        </div>
      )}

      {inspectorOpen && selectedGroup && (
        <div className="pg-pool-detail" onClick={(e) => { if (e.target === e.currentTarget) setInspectorOpen(false); }}>
          <div className="pg-pool-detail-body">
            <PoolCard
              group={selectedGroup}
              providers={providers}
              clientKeys={clientKeys}
              agentApps={agentApps}
              lastLaunch={lastLaunch}
              busyApp={previewAgentApp.isPending || launchAgentApp.isPending}
              removingModel={removeGroupModel.isPending}
              proxyPort={proxyStatus?.port || 9800}
              copied={copied}
              onCopyText={copyText}
              onToggle={handleToggle}
              onEdit={openEditGroup}
              onDelete={handleDelete}
              onCopyConfig={handleCopyConfig}
              onManageModels={(groupId) => { setSelected(groupId); setShowMembers(true); }}
              onCopyAllModels={openCopyModelsDialog}
              onRotateKey={handleRotatePoolKey}
              onRemoveModel={handleRemoveModel}
              onOpenApp={handlePreviewApp}
              onRestore={handleRestoreLastLaunch}
            />
          </div>
        </div>
      )}

      <Modal open={!!copyModelsDialog} onClose={() => setCopyModelsDialog(null)} title="复制全部模型" className="max-w-md">
        <div className="space-y-4">
          <div className="rounded-lg bg-[var(--bg-inset)] px-3 py-2.5 text-[11px] leading-5 text-[var(--text-secondary)]">
            将复制 {copyModelsDialog?.models.length || 0} 个去重后的模型名称。推荐使用英文逗号，兼容多数模型配置格式。
          </div>
          <div>
            <div className="mb-2 text-xs font-medium text-[var(--text-primary)]">分隔符</div>
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              {[
                [",", "英文逗号", ","],
                [", ", "逗号+空格", ", + 空格"],
                ["\n", "换行", "换行"],
                ["custom", "自定义", "自定义"],
              ].map(([value, label, preview]) => (
                <button key={value} type="button" onClick={() => setModelSeparator(value)} className={`rounded-lg border px-2 py-2 text-left transition-colors ${modelSeparator === value ? "border-[var(--color-brand)] bg-[var(--color-brand-subtle)]" : "border-[var(--border-default)] bg-[var(--bg-surface)] hover:bg-[var(--bg-hover)]"}`}>
                  <div className="text-[10px] font-medium text-[var(--text-primary)]">{label}</div>
                  <code className="mt-0.5 block text-[9px] text-[var(--text-dim)] pg-mono">{preview}</code>
                </button>
              ))}
            </div>
          </div>
          {modelSeparator === "custom" && <Input label="自定义分隔符" placeholder="例如： | " value={customSeparator} onChange={(event) => setCustomSeparator(event.target.value)} />}
          <div>
            <div className="mb-1.5 text-[10px] text-[var(--text-dim)]">复制预览</div>
            <code className="block max-h-24 overflow-y-auto break-all rounded-lg bg-[var(--bg-inset)] px-3 py-2 text-[10px] leading-5 text-[var(--text-primary)] pg-mono">
              {copyModelsDialog?.models.join(modelSeparator === "custom" ? customSeparator || "…" : modelSeparator)}
            </code>
          </div>
          <div className="flex justify-end gap-2"><Button variant="ghost" onClick={() => setCopyModelsDialog(null)}>取消</Button><Button onClick={handleCopyAllModels}><Copy size={13} /> 复制全部</Button></div>
        </div>
      </Modal>

      <Modal open={showCreate} onClose={() => setShowCreate(false)} title="新建路由池">
        <div className="space-y-4">
          <Input label="路由池名称" placeholder="例如：代码高质量池" value={newName} onChange={(event) => setNewName(event.target.value)} />
          <Input label="说明" placeholder="例如：优先使用 Coding Plan，失败时切换 API Key" value={newDescription} onChange={(event) => setNewDescription(event.target.value)} />
          <div className="grid grid-cols-2 gap-3">
            <label className="text-xs text-[var(--text-dim)]">对外协议
              <select className="mt-1 w-full h-9 rounded-md px-3 text-sm border bg-[var(--bg-elevated)] text-[var(--text-primary)]" style={{ borderColor: "var(--border-default)" }} value={newProtocol} onChange={(event) => setNewProtocol(event.target.value)}>
                <option value="openai">OpenAI compatible</option>
                <option value="anthropic">Anthropic Messages</option>
                <option value="both">OpenAI + Anthropic</option>
                <option value="gemini">Gemini</option>
                <option value="unified">Unified Responses（统一入口）</option>
              </select>
            </label>
            <label className="text-xs text-[var(--text-dim)]">调度策略
              <select className="mt-1 w-full h-9 rounded-md px-3 text-sm border bg-[var(--bg-elevated)] text-[var(--text-primary)]" style={{ borderColor: "var(--border-default)" }} value={newStrategy} onChange={(event) => setNewStrategy(event.target.value)}>
                {Object.entries(strategyLabel).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
              </select>
            </label>
          </div>
          <div className="rounded-lg border px-3 py-2.5 text-[11px] leading-5 text-[var(--text-secondary)]" style={{ borderColor: "var(--color-brand)", background: "var(--color-brand-subtle)" }}>
            创建后可加入来自不同 Coding Plan、模型大厂、免费模型和聚合网关的资源。PoolGate 将按能力和策略统一调度。
          </div>
          <div className="flex justify-end gap-2 pt-2"><Button variant="ghost" onClick={() => setShowCreate(false)}>取消</Button><Button onClick={handleCreate} loading={createGroup.isPending}>创建路由池</Button></div>
        </div>
      </Modal>

      <Modal open={showMembers && !!selectedGroup} onClose={() => { setShowMembers(false); setProviderDraft([]); }} title={`按厂商添加账号 · ${selectedGroup?.name || ""}`} className="max-w-3xl">
        <div className="space-y-3">
          <div className="flex flex-wrap items-center justify-between gap-3 text-[11px] text-[var(--text-dim)]">
            <span>选择模型厂商后，将加入该厂商全部未入池的模型；下方可单独挑选账号。</span>
            <div className="flex gap-1.5"><Badge variant="mute">池内 {groupModels.length}</Badge><Badge variant="brand">已选 {providerDraft.length} 厂商 / {selectedProviderResources.length} 模型</Badge></div>
          </div>
          <div className="max-h-[440px] space-y-2 overflow-y-auto rounded-lg border p-2" style={{ borderColor: "var(--border-subtle)" }}>
            {availableModelsLoading ? (
              <div className="flex items-center justify-center gap-2 py-12 text-sm text-[var(--text-dim)]"><Spinner size={18} /> 正在加载可用厂商...</div>
            ) : availableModelsError ? (
              <div className="flex flex-col items-center justify-center py-12 text-center"><div className="text-sm text-[var(--color-err)]">可用厂商加载失败</div><Button className="mt-3" size="sm" variant="outline" onClick={() => refetchAvailableModels()}><RefreshCw size={12} /> 重试</Button></div>
            ) : accountsByProvider.length ? accountsByProvider.map(({ providerId, providerName, accounts }) => {
              const info = providerAccountSelection[providerId];
              const availableModels = modelResourcesByProvider.find(([id]) => id === providerId)?.[1] || [];
              const availableCount = availableModels.filter((resource) => !resource.already_added).length;
              const addedCount = availableModels.length - availableCount;
              const totalAccounts = accounts.length;
              const healthyCount = info.healthy.length;
              const checked = providerDraft.includes(providerId);
              const draftIds = accountDraft[providerId];
              const selectedIds = draftIds !== undefined ? new Set(draftIds) : info.selected;
              const selectedCount = selectedIds.size;
              const hasSelection = draftIds !== undefined;
              return (
                <div
                  key={providerId}
                  className={`rounded-xl border p-3 transition-all ${checked ? "border-[var(--color-brand)] bg-[var(--color-brand-subtle)]/40" : "border-[var(--border-subtle)] bg-[var(--bg-elevated)]"}`}
                >
                  <button
                    type="button"
                    disabled={!availableCount}
                    onClick={() => handleToggleProvider(providerId, availableCount)}
                    className="flex w-full items-start gap-3 text-left disabled:cursor-default disabled:opacity-65"
                  >
                    <span className={`mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-md border ${!availableCount ? "border-[var(--color-ok)] bg-[var(--color-ok-subtle)] text-[var(--color-ok)]" : checked ? "border-[var(--color-brand)] bg-[var(--color-brand)] text-white" : "border-[var(--border-strong)]"}`}>{(!availableCount || checked) && <Check size={12} />}</span>
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <div><div className="text-[12px] font-semibold text-[var(--text-primary)]">{providerName}</div><div className="mt-0.5 text-[9px] text-[var(--text-dim)]">{totalAccounts} 个账号 · {healthyCount} 个健康 · {availableModels.length} 个模型</div></div>
                        <div className="flex gap-1.5"><Badge variant={availableCount ? "brand" : "ok"}>{availableCount ? `${availableCount} 个待加入` : "已全部加入"}</Badge>{addedCount > 0 && availableCount > 0 && <Badge variant="mute">已加入 {addedCount}</Badge>}</div>
                      </div>
                    </div>
                  </button>
                  <div className="mt-3 space-y-2">
                    <div className="flex items-center justify-between gap-2 text-[9px]">
                      <span className="text-[var(--text-dim)]">{hasSelection ? `已选 ${selectedCount}/${totalAccounts} 个账号` : "未调整账号选择，加入该厂商全部可路由账号"}</span>
                      <div className="flex gap-2">
                        <button type="button" className="text-[var(--color-brand)] hover:underline" onClick={() => selectAllHealthyAccounts(providerId)}>全选健康</button>
                        <button type="button" className="text-[var(--text-dim)] hover:underline" onClick={() => clearAccountSelection(providerId)}>清空选择</button>
                      </div>
                    </div>
                    <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
                      {accounts.map(({ account, models }) => {
                        const isChecked = selectedIds.has(account.id);
                        const isHealthy = account.routable && !["error", "exhausted", "token_expired"].includes(account.status);
                        return (
                          <label key={account.id} className={`flex cursor-pointer flex-col gap-1.5 rounded-md border px-2.5 py-2 transition-colors ${isChecked ? "border-[var(--color-brand)] bg-[var(--color-brand-subtle)]/60" : "border-[var(--border-subtle)] bg-[var(--bg-elevated)]"} ${!account.routable ? "opacity-60" : ""}`}>
                            <div className="flex items-center gap-2">
                              <input type="checkbox" checked={isChecked} disabled={!account.routable} onChange={() => toggleAccountSelection(providerId, account.id)} />
                              <span className="min-w-0 flex-1"><span className="block truncate text-[10px] font-medium text-[var(--text-primary)]">{account.name}</span><span className="block truncate text-[8px] text-[var(--text-dim)]">{account.email || account.id}</span></span>
                              <Badge variant={isHealthy ? "ok" : "err"}>{account.health_status === "error" ? "健康异常" : account.status}</Badge>
                            </div>
                            <div className="flex flex-wrap gap-1 pl-6">
                              {models.slice(0, 3).map((model) => <span key={model} className="rounded bg-[var(--bg-inset)] px-1.5 py-0.5 text-[8px] text-[var(--text-secondary)]">{model}</span>)}
                              {models.length > 3 && <span className="rounded bg-[var(--bg-inset)] px-1.5 py-0.5 text-[8px] text-[var(--text-dim)]">+{models.length - 3}</span>}
                            </div>
                          </label>
                        );
                      })}
                    </div>
                  </div>
                </div>
              );
            }) : (
              <div className="flex flex-col items-center justify-center py-12 text-center"><Box size={22} className="mb-2 text-[var(--text-dim)]" /><div className="text-sm text-[var(--text-secondary)]">没有与当前协议兼容的可路由厂商</div><div className="mt-1 text-[10px] text-[var(--text-dim)]">OpenAI compatible 号池支持 Chat Completions 与 Responses 厂商。</div></div>
            )}
          </div>
          <div className="rounded-lg bg-[var(--color-warn-bg)] px-3 py-2 text-[10px] text-[var(--color-warn)]">部分模型已入池时，仅补齐该厂商剩余模型。空号池会返回 POOL_EMPTY，不会回退全量账号。</div>
          <div className="flex justify-end gap-2"><Button variant="ghost" onClick={() => { setShowMembers(false); setProviderDraft([]); setAccountDraft({}); }}><X size={13} /> 取消</Button><Button onClick={handleAddProviders} loading={addGroupModels.isPending || setGroupModelAccounts.isPending} disabled={!selectedProviderResources.length}><Plus size={13} /> 加入模型并保存账号</Button></div>
        </div>
      </Modal>

      <Modal open={!!editingGroup} onClose={() => setEditingGroup(null)} title={`编辑号池 · ${editingGroup?.name || ""}`}>
        <div className="space-y-4">
          <Input label="号池名称" value={editName} onChange={(event) => setEditName(event.target.value)} />
          <Input label="说明" placeholder="按模型能力组织的资源池" value={editDescription} onChange={(event) => setEditDescription(event.target.value)} />
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <label className="text-xs text-[var(--text-dim)]">对外协议
              <select className="mt-1 h-9 w-full rounded-md border bg-[var(--bg-elevated)] px-3 text-sm text-[var(--text-primary)]" style={{ borderColor: "var(--border-default)" }} value={editProtocol} onChange={(event) => setEditProtocol(event.target.value)}>
                <option value="openai">OpenAI compatible</option>
                <option value="anthropic">Anthropic Messages</option>
                <option value="both">OpenAI + Anthropic</option>
                <option value="gemini">Gemini</option>
                <option value="unified">Unified Responses（统一入口）</option>
              </select>
            </label>
            <label className="text-xs text-[var(--text-dim)]">调度策略
              <select className="mt-1 h-9 w-full rounded-md border bg-[var(--bg-elevated)] px-3 text-sm text-[var(--text-primary)]" style={{ borderColor: "var(--border-default)" }} value={editStrategy} onChange={(event) => setEditStrategy(event.target.value)}>
                {Object.entries(strategyLabel).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
              </select>
            </label>
          </div>
          {editingGroup && editProtocol !== editingGroup.protocol && <div className="rounded-lg bg-[var(--color-warn-bg)] px-3 py-2 text-[10px] leading-5 text-[var(--color-warn)]">修改协议不会自动删除现有模型供应商。保存后请检查模型列表，移除与新协议不兼容的资源。</div>}
          <div className="flex justify-end gap-2"><Button variant="ghost" onClick={() => setEditingGroup(null)}>取消</Button><Button onClick={handleEditGroup} loading={updateGroup.isPending}><Check size={13} /> 保存修改</Button></div>
        </div>
      </Modal>

      <Modal open={!!createdRawKey && !!createdPoolName} onClose={() => {}} title={`${createdPoolName || "路由池"} · 专属 API Key`} className="max-w-xl">
        <div className="space-y-4">
          <div className="rounded-lg border p-3" style={{ borderColor: "var(--color-ok)", background: "var(--color-ok-subtle)" }}>
            <div className="pg-eyebrow mb-2">仅显示一次</div>
            <div className="flex items-start gap-2"><code className="flex-1 pg-mono text-[12px] leading-5 break-all text-[var(--text-primary)]">{createdRawKey}</code><button className="p-1 text-[var(--color-ok)]" onClick={() => createdRawKey && copyText(createdRawKey, "pool-raw")} title="复制 API Key">{copied === "pool-raw" ? <Check size={15} /> : <Copy size={15} />}</button></div>
          </div>
          <div className="rounded-lg border px-3 py-2.5 text-[11px] leading-5 text-[var(--text-secondary)]" style={{ borderColor: "var(--color-warn)", background: "var(--color-warn-bg)" }}>
            PoolGate 数据库不保存明文。关闭后无法再次查看；如遗失只能刷新替换，刷新后旧 Key 会立即失效。
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" onClick={() => createdRawKey && copyText(`export OPENAI_BASE_URL=http://127.0.0.1:9800\nexport OPENAI_API_KEY=${createdRawKey}\nexport ANTHROPIC_BASE_URL=http://127.0.0.1:9800\nexport ANTHROPIC_AUTH_TOKEN=${createdRawKey}`, "pool-config")}><Copy size={13} /> {copied === "pool-config" ? "配置已复制" : "复制 Agent 配置"}</Button>
            <Button className="ml-auto" onClick={closeCreatedKey}><ShieldCheck size={13} /> 我已安全保存</Button>
          </div>
        </div>
      </Modal>

      <Modal open={!!appPreview} onClose={() => setAppPreview(null)} title={`切换配置并打开 ${appPreview?.appName || "Agent"}`} className="max-w-xl">
        <div className="space-y-4">
          <div className="text-[11px] leading-5 text-[var(--text-secondary)]">将把应用持久配置切换到本地 PoolGate 网关，并使用号池「{appPreview?.groupName}」的专属 Key。操作前会创建可恢复备份。</div>
          <div><div className="pg-eyebrow mb-2">将改写的配置</div><div className="space-y-1.5">{appPreview?.paths.map((path) => <code key={path} className="block rounded-md bg-[var(--bg-inset)] px-2.5 py-2 pg-mono text-[10px] break-all text-[var(--text-primary)]">{path}</code>)}</div></div>
          <div><div className="pg-eyebrow mb-2">备份目录</div><code className="block rounded-md bg-[var(--bg-inset)] px-2.5 py-2 pg-mono text-[10px] break-all text-[var(--text-primary)]">{appPreview?.backupRoot}</code></div>
          <div className="rounded-lg border px-3 py-2 text-[10px] leading-5 text-[var(--color-warn)]" style={{ borderColor: "var(--color-warn)", background: "var(--color-warn-bg)" }}>{appPreview?.warnings.map((warning) => <div key={warning}>· {warning}</div>)}</div>
          <div className="flex justify-end gap-2"><Button variant="ghost" onClick={() => setAppPreview(null)}>取消</Button><Button onClick={handleLaunchApp} loading={launchAgentApp.isPending}><ExternalLink size={13} /> 确认切换并打开</Button></div>
        </div>
      </Modal>

      <Modal open={showKeys} onClose={() => { setShowKeys(false); if (!createdPoolName) setCreatedRawKey(null); }} title="接入密钥 · 虚拟客户端 Key" className="max-w-3xl">
        <div className="space-y-4">
          <div className="rounded-lg border px-3 py-2.5 text-[11px] leading-5 text-[var(--text-secondary)]" style={{ borderColor: "var(--color-brand)", background: "var(--color-brand-subtle)" }}>
            每个接入密钥绑定一个或多个路由池。把密钥填进 Agent 工具的 <code className="pg-mono">API_KEY</code>，网关会自动把请求路由到对应池，无需再配置 <code className="pg-mono">X-Group-Id</code>。不绑定任何池时走默认（全量）账号池。
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <Input label="密钥名称" placeholder="例如：Cursor 主工作流" value={keyName} onChange={(event) => setKeyName(event.target.value)} />
            <label className="text-xs text-[var(--text-dim)]">绑定路由池
              <select
                multiple
                className="mt-1 w-full h-9 rounded-md px-3 text-sm border bg-[var(--bg-elevated)] text-[var(--text-primary)]"
                style={{ borderColor: "var(--border-default)" }}
                value={keyPoolIds}
                onChange={(event) => setKeyPoolIds(Array.from(event.target.selectedOptions).map((option) => option.value))}
              >
                {groups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}
              </select>
            </label>
          </div>
          <div className="flex items-center gap-2">
            <Button size="sm" onClick={handleCreateKey} loading={createKey.isPending}><KeyRound size={13} /> 生成接入密钥</Button>
            {createdRawKey && (
              <div className="flex-1 flex items-center gap-2 rounded-lg border px-3 py-2" style={{ borderColor: "var(--color-ok)", background: "var(--color-ok-subtle)" }}>
                <code className="pg-mono text-[11px] break-all text-[var(--text-primary)]">{createdRawKey}</code>
                <button className="text-[var(--color-ok)] hover:opacity-80" onClick={() => copyText(createdRawKey, "raw")} title="复制密钥">
                  {copied === "raw" ? <Check size={14} /> : <Copy size={14} />}
                </button>
              </div>
            )}
          </div>
          {createdRawKey && <div className="text-[11px] text-[var(--color-warn)]">密钥仅显示这一次，关闭弹窗后将无法再次查看，请立即复制保存。</div>}

          <div className="border-t pt-3" style={{ borderColor: "var(--border-subtle)" }}>
            {keysLoading ? <Spinner size={18} /> : clientKeys.length === 0 ? (
              <div className="py-8 text-center text-sm text-[var(--text-dim)]">还没有接入密钥，生成一个即可让 Agent 工具连入指定路由池</div>
            ) : (
              <div className="space-y-2">
                {clientKeys.map((key) => (
                  <div key={key.id} className="rounded-lg border px-3 py-2.5" style={{ borderColor: "var(--border-default)" }}>
                    <div className="flex items-center justify-between gap-3">
                      <div className="flex items-center gap-2 min-w-0">
                        <KeyRound size={13} className="shrink-0" style={{ color: key.enabled ? "var(--color-ok)" : "var(--text-dim)" }} />
                        <span className="text-[12px] font-medium text-[var(--text-primary)] truncate">{key.name}</span>
                        <code className="pg-mono text-[10px] text-[var(--text-dim)]">{key.key_prefix}••••{key.key_last_four}</code>
                        {!key.enabled && <Badge variant="mute" dot>已停用</Badge>}
                      </div>
                      <div className="flex items-center gap-1 shrink-0">
                        <button
                          className="p-1.5 rounded-md text-[var(--text-dim)] hover:bg-[var(--bg-hover)]"
                          title="复制 Agent 配置"
                          onClick={() => copyText(`export OPENAI_BASE_URL=http://127.0.0.1:9800\nexport OPENAI_API_KEY=${"pg_live_••••" + key.key_last_four}\n# 用上方真实密钥替换占位`, `cfg-${key.id}`)}
                        >
                          {copied === `cfg-${key.id}` ? <Check size={13} /> : <Copy size={13} />}
                        </button>
                        <button
                          className={`p-1.5 rounded-md ${key.enabled ? "text-[var(--color-ok)] hover:bg-[var(--color-ok-bg)]" : "text-[var(--text-dim)] hover:bg-[var(--bg-hover)]"}`}
                          title={key.enabled ? "停用" : "启用"}
                          onClick={() => handleToggleKey(key)}
                        >
                          {key.enabled ? <Power size={13} /> : <PowerOff size={13} />}
                        </button>
                        <button className="p-1.5 rounded-md text-[var(--text-dim)] hover:bg-[var(--color-err-bg)] hover:text-[var(--color-err)]" title="删除" onClick={() => handleDeleteKey(key)}><Trash2 size={13} /></button>
                      </div>
                    </div>
                    <div className="mt-2 flex flex-wrap items-center gap-1.5">
                      <span className="text-[10px] text-[var(--text-dim)]">绑定池：</span>
                      {groups.map((group) => {
                        const bound = key.pool_ids.includes(group.id);
                        return (
                          <button
                            key={group.id}
                            className={`px-2 py-0.5 rounded-full text-[10px] border transition-colors ${bound ? "text-[var(--color-ok)]" : "text-[var(--text-dim)] hover:text-[var(--text-secondary)]"}`}
                            style={{ borderColor: bound ? "var(--color-ok)" : "var(--border-default)", background: bound ? "var(--color-ok-subtle)" : "transparent" }}
                            onClick={() => handleBindPool(key, group.id)}
                            title={bound ? "点击解绑" : "点击绑定"}
                          >
                            {group.name}
                          </button>
                        );
                      })}
                      {key.pool_ids.length === 0 && <span className="text-[10px] text-[var(--text-dim)]">（默认池 · 全量账号）</span>}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </Modal>
    </div>
  );
}
