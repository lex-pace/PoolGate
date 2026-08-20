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
  useSetGroupModelResources,
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
  ChevronDown,
  ChevronRight,
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
  Search,
  ShieldCheck,
  Trash2,
  Users,
  X,
} from "lucide-react";
import Segmented from "@/components/ui/Segmented";

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

/// 自定义供应商判定：本地类型、名称/Base URL 含 custom/自定义，或账号显式打上
/// resource_category:custom 标签（与模型资源页「自定义」分类口径一致）。
function providerLooksCustom(provider?: { type?: string; name?: string; base_url?: string }) {
  if (!provider) return true;
  if (provider.type === "local") return true;
  const signature = `${provider.name || ""} ${provider.base_url || ""}`.toLowerCase();
  return signature.includes("custom") || signature.includes("自定义");
}

function accountHasCustomCategory(account: Account) {
  if (!account.tags) return false;
  try {
    const parsed = JSON.parse(account.tags);
    if (Array.isArray(parsed)) return parsed.some((tag) => tag === "resource_category:custom");
  } catch {
    // Legacy comma-separated tags.
  }
  return account.tags.split(",").map((tag) => tag.trim()).includes("resource_category:custom");
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
        <div className="flex items-start justify-between gap-3 border-b px-4 py-3" style={{ borderColor: "var(--border-subtle)" }}>
          <div className="flex min-w-0 items-start gap-2.5">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-[var(--bg-inset)]"><StatusDot status={group.enabled ? "ok" : "mute"} pulse={group.enabled} /></div>
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="truncate text-[13px] font-semibold text-[var(--text-primary)]">{group.name}</span>
                <Badge variant="brand">{protocolLabel[group.protocol] || group.protocol}</Badge>
                {!group.enabled && <Badge variant="mute" dot>已停用</Badge>}
              </div>
              <p className="mt-1 truncate text-[10px] leading-5 text-[var(--text-dim)]" title={group.description || "按模型能力组织的资源池"}>{group.description || "按模型能力组织的资源池"}</p>
              <div className="mt-0.5 flex flex-wrap gap-x-3 text-[10px] text-[var(--text-secondary)]">
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
        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 py-3">
          {/* Top Section: API Key + Stats + Provider Quota */}
          <section className="rounded-lg border" style={{ borderColor: "var(--border-subtle)" }}>
            {/* API Key */}
            <div className="flex items-center justify-between gap-2 border-b px-3 py-2.5" style={{ borderColor: "var(--border-subtle)" }}>
              <div className="flex min-w-0 items-center gap-2">
                <KeyRound size={13} className="shrink-0 text-[var(--color-brand)]" />
                <div className="min-w-0">
                  <code className="max-w-xs truncate text-[11px] text-[var(--text-secondary)] pg-mono" title={managedKey ? `${managedKey.key_prefix}••••••••${managedKey.key_last_four}` : "尚未生成"}>{managedKey ? `${managedKey.key_prefix}••••••••${managedKey.key_last_four}` : "尚未生成"}</code>
                </div>
              </div>
              <button onClick={() => onRotateKey(group.id, !!managedKey)} className="rounded bg-[var(--color-brand)] px-2.5 py-1 text-[10px] font-medium text-white transition-colors hover:bg-[var(--color-brand)]/80" title={managedKey ? "刷新替换号池专属 Key" : "生成号池专属 Key"}>{managedKey ? "刷新" : "生成"}</button>
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
                <div key={String(label)} className="min-w-0 px-2 py-2.5 text-center" style={{ borderColor: "var(--border-subtle)" }}>
                  <div className="truncate text-[9px] uppercase tracking-wide text-[var(--text-dim)]">{label}</div>
                  <div className="truncate text-[12px] font-semibold leading-5 text-[var(--text-primary)] pg-mono">{dashboardLoading ? "…" : value ?? 0}</div>
                </div>
              ))}
            </div>
            {/* Provider Quota */}
            {dashboard?.quota_by_provider.length ? (
              <div className="border-t px-3 py-2.5" style={{ borderColor: "var(--border-subtle)" }}>
                <div className="mb-2 text-[10px] font-medium text-[var(--text-dim)]">PROVIDER 额度</div>
                <div className="space-y-2">
                  {dashboard.quota_by_provider.map((quota) => (
                    <div key={quota.provider_id} className="flex items-center gap-2">
                      <span className="w-24 shrink-0 truncate text-[10px] text-[var(--text-primary)]" title={quota.provider_name}>{quota.provider_name}</span>
                      <div className="h-2 flex-1 overflow-hidden rounded-full bg-[var(--bg-hover)]"><div className="h-full rounded-full" style={{ width: `${Math.min(100, Math.max(0, quota.average_used_percent))}%`, background: quota.average_used_percent >= 90 ? "var(--color-err)" : quota.average_used_percent >= 70 ? "var(--color-warn)" : "var(--color-ok)" }} /></div>
                      <span className={`shrink-0 text-[9px] ${quota.abnormal_accounts ? "text-[var(--color-warn)]" : "text-[var(--text-dim)]"}`}>{quota.account_count}账号</span>
                    </div>
                  ))}
                </div>
              </div>
            ) : null}
          </section>

          {/* Address Section */}
          <section>
            <div className="mb-2 flex items-center justify-between gap-2">
              <div className="pg-eyebrow" style={{ fontSize: "11px" }}>接入地址</div>
              <code className="text-[8px] text-[var(--text-dim)] pg-mono">127.0.0.1:{proxyPort}</code>
            </div>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              {addressBlocks.map((block) => (
                <div key={block.id} className="rounded-lg border bg-[var(--bg-inset)] p-2.5" style={{ borderColor: "var(--border-subtle)" }}>
                  <div className="mb-2 text-[11px] font-semibold text-[var(--text-primary)]">{block.label}</div>
                  <div className="space-y-1.5">
                    {block.rows.filter((row) => row.id !== "base").map((row) => {
                      const tag = `pool-url-${group.id}-${block.id}-${row.id}`;
                      return (
                        <div key={row.id} className="flex min-w-0 items-center gap-2">
                          <span className="w-[80px] shrink-0 text-[10px] text-[var(--text-dim)]">{row.label}</span>
                          <code className="min-w-0 flex-1 truncate text-[11px] text-[var(--text-primary)] pg-mono" title={row.url}>{row.url}</code>
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
              <div className="pg-eyebrow" style={{ fontSize: "11px" }}>模型供应商 · {sortedModels.length}</div>
              <div className="flex items-center gap-0.5">
                <button disabled={!sortedModels.length} onClick={() => onCopyAllModels(group.id, sortedModels.map((resource) => resource.model))} className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[10px] font-medium text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--color-brand)] disabled:cursor-not-allowed disabled:opacity-40" title="选择分隔符并复制全部模型"><Copy size={10} /> 复制全部</button>
                <button onClick={() => onManageModels(group.id)} className="inline-flex items-center gap-1 rounded-md border border-[var(--color-brand)]/40 px-2.5 py-1 text-[10px] font-medium text-[var(--color-brand)] hover:bg-[var(--color-brand-subtle)] cursor-pointer"><Plus size={11} /> 添加供应商</button>
              </div>
            </div>
            <div className="mt-2 min-h-14 rounded-md border p-2" style={{ borderColor: "var(--border-subtle)" }}>
              {modelsLoading ? <div className="flex items-center justify-center py-3"><Spinner size={14} /></div> : modelsError ? (
                <div className="py-3 text-center text-[9px] text-[var(--color-err)]">模型供应商加载失败</div>
              ) : sortedModels.length ? (
                <div className="max-h-[160px] overflow-y-auto pr-0.5">
                  {/* Compact table layout */}
                  <div className="space-y-1">
                    {sortedModels.slice(0, 8).map((resource) => {
                      const tag = `pool-model-${group.id}-${resource.model}`;
                      const providerNames = (resource as any)._providers?.join(", ") || providerMap.get(resource.provider_id) || "Provider";
                      return (
                        <div key={resource.model} className="flex items-center gap-2 rounded px-2 py-1.5 hover:bg-[var(--bg-hover)] group">
                          <span className="flex-1 min-w-0 text-[11px] text-[var(--text-primary)] truncate" title={`${providerNames} · ${resource.model}`}>
                            {resource.model}
                          </span>
                          <span className="shrink-0 text-[9px] text-[var(--text-dim)] truncate max-w-[100px]" title={providerNames}>
                            {providerNames}
                          </span>
                          <div className="flex items-center gap-0.5 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
                            <button type="button" onClick={() => onCopyText(resource.model, tag)} className="rounded p-0.5 text-[var(--text-dim)] hover:text-[var(--color-brand)]" title="复制模型名称">
                              {copied === tag ? <Check size={9} /> : <Copy size={9} />}
                            </button>
                            <button disabled={removingModel} onClick={() => onRemoveModel(group.id, resource.provider_id, resource.model)} className="rounded p-0.5 text-[var(--text-dim)] hover:text-[var(--color-err)] disabled:opacity-40" title={`从号池移除 ${resource.model}`}>
                              <X size={9} />
                            </button>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                  {sortedModels.length > 8 && (
                    <button
                      onClick={() => onManageModels(group.id)}
                      className="w-full mt-1 py-1 text-[9px] text-[var(--color-brand)] hover:underline text-center"
                    >
                      查看全部 {sortedModels.length} 个模型 →
                    </button>
                  )}
                </div>
              ) : (
                <div className="flex h-12 items-center justify-center gap-2 text-center"><Box size={14} className="text-[var(--color-warn)]" /><div><div className="text-[10px] text-[var(--color-warn)]">尚无模型供应商</div><div className="text-[9px] text-[var(--text-dim)]">空池不会回退全量账号</div></div></div>
              )}
            </div>
          </section>
        </div>

        {/* Footer: Agent Apps */}
        <div className="mt-auto flex flex-wrap items-center justify-between gap-x-3 gap-y-2 border-t px-4 py-2.5" style={{ borderColor: "var(--border-subtle)", background: "var(--bg-inset)" }}>
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
  const clearGroupModels = useSetGroupModelResources();
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
  const [providerSearch, setProviderSearch] = useState("");
  // 弹层左栏的分段筛选与聚焦的供应商（右栏详情）
  const [providerFilter, setProviderFilter] = useState<"all" | "in_pool" | "custom">("all");
  const [focusedProviderId, setFocusedProviderId] = useState<string | null>(null);
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

  // 供应商列表（按 provider 聚合，含账号详情，用于弹窗展示）
  const providerEntries = useMemo(() => {
    type AccountInfo = {
      id: string;
      name: string;
      email?: string;
      status: string;
      healthStatus: string;
      routable: boolean;
      selected: boolean;
    };
    const grouped = new Map<string, {
      providerName: string;
      modelCount: number;
      accounts: Map<string, AccountInfo>;
      allAdded: boolean;
    }>();
    availableGroupModels.forEach((resource) => {
      const entry = grouped.get(resource.provider_id) || {
        providerName: resource.provider_name,
        modelCount: 0,
        accounts: new Map<string, AccountInfo>(),
        allAdded: true,
      };
      entry.modelCount += 1;
      if (!resource.already_added) entry.allAdded = false;
      resource.accounts.forEach((a) => {
        const existing = entry.accounts.get(a.id);
        // selected=true 只要任一资源中标记为选中即可
        entry.accounts.set(a.id, {
          id: a.id,
          name: a.name,
          email: a.email,
          status: a.status,
          healthStatus: a.health_status,
          routable: a.routable,
          selected: existing ? (existing.selected || a.selected) : a.selected,
        });
      });
      grouped.set(resource.provider_id, entry);
    });
    return Array.from(grouped.entries()).map(([providerId, { providerName, modelCount, accounts, allAdded }]) => {
      const accountList = Array.from(accounts.values());
      return {
        providerId,
        providerName,
        modelCount,
        accountCount: accountList.filter((a) => a.routable).length,
        healthyCount: accountList.filter((a) => a.routable && !["error", "exhausted", "token_expired"].includes(a.status)).length,
        allAdded,
        accounts: accountList,
      };
    });
  }, [availableGroupModels]);

  // 自定义供应商分组：满足自定义口径的供应商归入「自定义」组，可整组选择。
  const customProviderIds = useMemo(() => {
    const ids = new Set<string>();
    providers.forEach((provider) => {
      if (providerLooksCustom(provider)) ids.add(provider.id);
    });
    accounts.forEach((account) => {
      if (account.provider_id && accountHasCustomCategory(account)) ids.add(account.provider_id);
    });
    return ids;
  }, [providers, accounts]);

  const regularProviderEntries = providerEntries.filter((entry) => !customProviderIds.has(entry.providerId));
  const customProviderEntries = providerEntries.filter((entry) => customProviderIds.has(entry.providerId));

  React.useEffect(() => {
    setProviderDraft([]);
    setAccountDraft({});
    setProviderSearch("");
    setProviderFilter("all");
    setFocusedProviderId(null);
  }, [selected]);

  // 当可用供应商数据加载后，初始化草稿为已入池的供应商集合，账号草稿设为已选中的账号
  React.useEffect(() => {
    if (!showMembers) return;
    const alreadyInPool = new Set<string>();
    const draft: Record<string, string[]> = {};
    availableGroupModels.forEach((resource) => {
      if (resource.already_added) {
        alreadyInPool.add(resource.provider_id);
      }
    });
    // 为已入池供应商初始化账号草稿（从资源中收集已选中的账号）
    alreadyInPool.forEach((providerId) => {
      const selectedAccountIds = new Set<string>();
      availableGroupModels
        .filter((r) => r.provider_id === providerId)
        .forEach((r) => r.accounts.filter((a) => a.selected).forEach((a) => selectedAccountIds.add(a.id)));
      if (selectedAccountIds.size > 0) {
        draft[providerId] = Array.from(selectedAccountIds);
      }
    });
    setProviderDraft(Array.from(alreadyInPool));
    setAccountDraft(draft);
  }, [showMembers, availableGroupModels]);

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

  // 切换供应商选择状态：选中 = 该供应商将入池（继承全部模型 + 全部可路由账号），取消 = 该供应商将从池中移除。
  const handleToggleProvider = (providerId: string) => {
    const turningOn = !providerDraft.includes(providerId);
    setProviderDraft((current) => current.includes(providerId)
      ? current.filter((item) => item !== providerId)
      : [...current, providerId]);
    if (turningOn) {
      // 选中供应商时，初始化账号草稿为全部可路由账号
      const entry = providerEntries.find((e) => e.providerId === providerId);
      if (entry) {
        const routableIds = entry.accounts.filter((a) => a.routable).map((a) => a.id);
        setAccountDraft((current) => ({ ...current, [providerId]: routableIds }));
      }
    } else {
      // 取消选中时，清除该供应商的账号草稿
      setAccountDraft((current) => {
        const next = { ...current };
        delete next[providerId];
        return next;
      });
    }
  };

  // 切换单个账号的选中状态
  const handleToggleAccount = (providerId: string, accountId: string) => {
    setAccountDraft((current) => {
      const entry = providerEntries.find((e) => e.providerId === providerId);
      const allRoutable = entry?.accounts.filter((a) => a.routable).map((a) => a.id) || [];
      const base = current[providerId] !== undefined
        ? new Set(current[providerId])
        : new Set(allRoutable);
      if (base.has(accountId)) {
        base.delete(accountId);
      } else {
        base.add(accountId);
      }
      return { ...current, [providerId]: Array.from(base) };
    });
  };

  // 全选当前供应商的所有账号
  const selectAllAccounts = (providerId: string) => {
    const entry = providerEntries.find((e) => e.providerId === providerId);
    if (!entry) return;
    const routableIds = entry.accounts.filter((a) => a.routable).map((a) => a.id);
    setAccountDraft((current) => ({ ...current, [providerId]: routableIds }));
  };

  // 全选健康账号
  const selectAllHealthyAccounts = (providerId: string) => {
    const entry = providerEntries.find((e) => e.providerId === providerId);
    if (!entry) return;
    const healthyIds = entry.accounts
      .filter((a) => a.routable && !["error", "exhausted", "token_expired"].includes(a.status))
      .map((a) => a.id);
    setAccountDraft((current) => ({ ...current, [providerId]: healthyIds }));
  };

  // 恢复默认（清除账号草稿，使用全部可路由账号）
  const restoreAccountSelection = (providerId: string) => {
    setAccountDraft((current) => {
      const next = { ...current };
      delete next[providerId];
      return next;
    });
  };

  // 全选当前筛选结果中的可见供应商（同步初始化账号草稿为全部可路由账号）。
  const selectVisibleProviders = () => {
    const newlyChecked = visibleProviderEntries
      .map((entry) => entry.providerId)
      .filter((id) => !providerDraft.includes(id));
    if (!newlyChecked.length) return;
    setProviderDraft((current) => Array.from(new Set([...current, ...newlyChecked])));
    setAccountDraft((current) => {
      const next = { ...current };
      newlyChecked.forEach((id) => {
        const entry = providerEntries.find((e) => e.providerId === id);
        if (entry) next[id] = entry.accounts.filter((a) => a.routable).map((a) => a.id);
      });
      return next;
    });
  };

  // 当前池内供应商集合（去重）
  const currentPoolProviders = useMemo(() => {
    const ids = new Set<string>();
    groupModels.forEach((resource) => ids.add(resource.provider_id));
    return ids;
  }, [groupModels]);

  // 弹层左栏列表：分段筛选（全部 / 池内 / 自定义）叠加搜索。
  const providerFilterBase = providerFilter === "in_pool"
    ? providerEntries.filter((entry) => currentPoolProviders.has(entry.providerId))
    : providerFilter === "custom"
      ? customProviderEntries
      : providerEntries;
  const visibleProviderEntries = providerSearch.trim()
    ? providerFilterBase.filter((entry) => entry.providerName.toLowerCase().includes(providerSearch.toLowerCase()))
    : providerFilterBase;
  const visibleUncheckedCount = visibleProviderEntries.filter((entry) => !providerDraft.includes(entry.providerId)).length;

  // 右栏聚焦的供应商详情
  const focusedEntry = providerEntries.find((entry) => entry.providerId === focusedProviderId) || null;

  // 弹层打开时自动聚焦第一个可见供应商，右栏不空白。
  React.useEffect(() => {
    if (showMembers && !focusedProviderId && visibleProviderEntries.length > 0) {
      setFocusedProviderId(visibleProviderEntries[0].providerId);
    }
  }, [showMembers, visibleProviderEntries, focusedProviderId]);

  // 草稿相对池内的变更数量（用于底部摘要）
  const pendingAddCount = providerDraft.filter((id) => !currentPoolProviders.has(id)).length;
  const pendingRemoveCount = Array.from(currentPoolProviders).filter((id) => !providerDraft.includes(id)).length;

  // 保存：按供应商粒度做 diff，新增走 addGroupModelResources，移除走 removeGroupModelResource。
  // 对于保留在池中的供应商，如果账号草稿有变更则更新账号绑定。
  const handleAddProviders = async () => {
    if (!selected) return;
    const desiredSet = new Set(providerDraft);
    const toAdd = providerDraft.filter((id) => !currentPoolProviders.has(id));
    const toRemove = Array.from(currentPoolProviders).filter((id) => !desiredSet.has(id));
    // 需要更新账号绑定的供应商：在池中且在草稿中有账号选择
    const toUpdateAccounts = providerDraft.filter((id) =>
      currentPoolProviders.has(id) && accountDraft[id] !== undefined,
    );

    if (!toAdd.length && !toRemove.length && !toUpdateAccounts.length) {
      toast("info", "未发生变更");
      setShowMembers(false);
      return;
    }

    try {
      // 新增供应商：自动继承其全部模型
      if (toAdd.length) {
        const resources = toAdd.flatMap((providerId) => {
          const models = availableGroupModels
            .filter((r) => r.provider_id === providerId)
            .map((r) => ({ provider_id: r.provider_id, model: r.model }));
          return models;
        });
        if (resources.length) {
          await addGroupModels.mutateAsync({ groupId: selected, resources });
        }
      }
      // 移除供应商：删除其在池中的全部模型
      if (toRemove.length) {
        for (const providerId of toRemove) {
          const models = groupModels.filter((r) => r.provider_id === providerId);
          for (const resource of models) {
            await removeGroupModel.mutateAsync({ groupId: selected, providerId: resource.provider_id, model: resource.model });
          }
        }
      }
      // 更新账号绑定：为新增供应商和已有供应商设置账号约束
      const providersNeedingAccountBinding = [
        ...toAdd, // 新增供应商也需要设置账号绑定
        ...toUpdateAccounts,
      ];
      for (const providerId of providersNeedingAccountBinding) {
        const ids = accountDraft[providerId];
        if (ids === undefined) continue; // 未调整该供应商的账号
        // 获取该供应商在池中的所有模型资源（包括刚新增的）
        const providerResources = [
          ...availableGroupModels.filter((r) => r.provider_id === providerId),
        ];
        for (const resource of providerResources) {
          const supported = new Set(resource.accounts.filter((a) => a.routable).map((a) => a.id));
          const valid = ids.filter((id) => supported.has(id));
          if (valid.length > 0) {
            await setGroupModelAccounts.mutateAsync({
              groupId: selected,
              providerId: resource.provider_id,
              model: resource.model,
              accountIds: valid,
            });
          }
        }
      }
      const parts: string[] = [];
      if (toAdd.length) parts.push(`加入 ${toAdd.length} 个供应商`);
      if (toRemove.length) parts.push(`移除 ${toRemove.length} 个供应商`);
      if (providersNeedingAccountBinding.length) parts.push(`更新 ${providersNeedingAccountBinding.length} 个供应商的账号绑定`);
      toast("success", parts.join("，") || "已保存");
      setProviderDraft([]);
      setAccountDraft({});
      setShowMembers(false);
    } catch (error) {
      toast("error", `保存失败：${String(error)}`);
    }
  };

  const handleClearPool = async () => {
    if (!selected) return;
    if (!window.confirm("确定清空路由池中所有已绑定的模型供应商？此操作不可撤销。")) return;
    try {
      await clearGroupModels.mutateAsync({ groupId: selected, resources: [] });
      toast("success", "路由池已清空");
      setProviderDraft([]);
      setAccountDraft({});
    } catch (error) {
      toast("error", `清空失败：${String(error)}`);
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

  // 左栏供应商行：勾选（checkbox）与聚焦（行主体）分离，行高固定，状态 chip 固定在行尾。
  const renderProviderRow = (entry: (typeof providerEntries)[number]) => {
    const { providerId, providerName, modelCount, accountCount, healthyCount } = entry;
    const checked = providerDraft.includes(providerId);
    const isCustom = customProviderIds.has(providerId);
    const inPool = currentPoolProviders.has(providerId);
    const focused = focusedProviderId === providerId;
    const allHealthy = accountCount > 0 && healthyCount === accountCount;
    const healthTone = accountCount === 0
      ? "text-[var(--text-dim)]"
      : allHealthy ? "text-[var(--color-ok)]" : "text-[var(--color-warn)]";
    return (
      <div
        key={providerId}
        className={`group relative flex items-center rounded-lg transition-colors duration-150 ${focused ? "bg-[var(--bg-hover)]" : "hover:bg-[var(--bg-hover)]"}`}
      >
        {focused && <span className="absolute left-0 top-1.5 bottom-1.5 w-[2.5px] rounded-full bg-[var(--color-brand)]" />}
        <button
          type="button"
          aria-label={checked ? `移除 ${providerName}` : `加入 ${providerName}`}
          onClick={() => handleToggleProvider(providerId)}
          className="flex h-[52px] w-9 shrink-0 cursor-pointer items-center justify-center"
        >
          <span className={`flex h-[16px] w-[16px] items-center justify-center rounded-[5px] border-[1.5px] transition-colors duration-150 ${checked ? "border-[var(--color-brand)] bg-[var(--color-brand)] text-white" : "border-[var(--border-strong)] group-hover:border-[var(--color-brand)]/70"}`}>
            {checked && <Check size={11} strokeWidth={3} />}
          </span>
        </button>
        <button
          type="button"
          onClick={() => setFocusedProviderId(providerId)}
          className="flex h-[52px] min-w-0 flex-1 cursor-pointer items-center gap-2 pr-2.5 text-left"
        >
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5">
              <span className={`truncate text-[13px] font-medium ${checked ? "text-[var(--color-brand)]" : "text-[var(--text-primary)]"}`}>{providerName}</span>
              {inPool && <span className="shrink-0 rounded border border-[var(--border-default)] px-1 text-[10px] leading-4 text-[var(--text-dim)]">池内</span>}
              {isCustom && !inPool && <span className="shrink-0 text-[10px] leading-4 text-[var(--text-dim)]">自定义</span>}
            </div>
            <div className="mt-0.5 flex items-center gap-1.5 text-[11px] leading-4 text-[var(--text-dim)]">
              <span className="pg-mono">{modelCount}</span><span>模型</span>
              <span className="opacity-50">·</span>
              <span className={`pg-mono ${healthTone}`}>{healthyCount}/{accountCount}</span><span>健康</span>
            </div>
          </div>
          {inPool && !checked
            ? <span className="shrink-0 rounded bg-[var(--color-warn)]/12 px-1.5 py-0.5 text-[10px] font-medium text-[var(--color-warn)]">将移除</span>
            : !inPool && checked
              ? <span className="shrink-0 rounded bg-[var(--color-brand)]/12 px-1.5 py-0.5 text-[10px] font-medium text-[var(--color-brand)]">将加入</span>
              : <ChevronRight size={13} className={`shrink-0 text-[var(--text-dim)] transition-opacity duration-150 ${focused ? "opacity-100 text-[var(--color-brand)]" : "opacity-0 group-hover:opacity-100"}`} />}
        </button>
      </div>
    );
  };

  // 右栏：聚焦供应商 — 标题 + 状态 Pill + 三张 KPI 卡片 + 账号列表。
  // 「是否入池」是单一信号源：左侧多选框 + 右侧 Pill，颜色和文案一起表达「现状 / 将要发生」。
  // 顶部不再放重复的"加入/保留在池"按钮，避免与左侧 checkbox 形成两条路径。
  const renderFocusedDetail = () => {
    if (!focusedEntry) {
      return (
        <div className="flex h-full flex-col items-center justify-center py-16 text-center">
          <PanelRightOpen size={24} className="mb-3 text-[var(--text-dim)]" />
          <div className="text-[13px] text-[var(--text-secondary)]">在左侧选择一个供应商</div>
          <div className="mt-1 text-[11px] text-[var(--text-dim)]">查看模型、账号健康状态，并配置参与调度的账号</div>
        </div>
      );
    }
    const { providerId, providerName, modelCount, accountCount, healthyCount, accounts } = focusedEntry;
    const checked = providerDraft.includes(providerId);
    const inPool = currentPoolProviders.has(providerId);
    const isCustom = customProviderIds.has(providerId);
    const draftIds = accountDraft[providerId];
    const allRoutableIds = accounts.filter((a) => a.routable).map((a) => a.id);
    const selectedAccountIds = draftIds !== undefined ? new Set(draftIds) : new Set(allRoutableIds);
    const healthyRate = accountCount === 0 ? 0 : Math.round((healthyCount / accountCount) * 100);
    const rateTone = accountCount === 0
      ? "var(--text-dim)"
      : healthyRate >= 100 ? "var(--color-ok)"
      : healthyRate >= 60  ? "var(--color-warn)"
      : "var(--color-err)";
    const poolState = !checked && inPool
      ? { variant: "warn" as const, text: "将移除" }
      : checked && !inPool
        ? { variant: "brand" as const, text: "将加入" }
        : checked
          ? { variant: "ok" as const, text: "已在池内" }
          : { variant: "mute" as const, text: "未入池" };
    return (
      <div className="flex h-full min-h-0 flex-col">
        {/* 标题区：名称 + 自定义 + 状态 Pill */}
        <div className="flex items-center justify-between gap-3 pb-2">
          <div className="flex min-w-0 items-center gap-1.5">
            <h3 className="truncate text-[15px] font-semibold tracking-[-0.01em] text-[var(--text-primary)]">{providerName}</h3>
            {isCustom && <Badge variant="info" className="!text-[10px]">自定义</Badge>}
          </div>
          <Badge variant={poolState.variant} dot className="!text-[10.5px]">{poolState.text}</Badge>
        </div>

        {/* 1 行 KPI：模型 · 账号 · 健康率 */}
        <div
          className="flex items-center divide-x rounded-md border"
          style={{ borderColor: "var(--border-default)", background: "var(--bg-surface)" }}
        >
          <div className="flex flex-1 items-center gap-2 px-3 py-1.5">
            <Layers3 size={12} className="text-[var(--text-dim)]" />
            <span className="pg-eyebrow">模型</span>
            <span className="ml-auto pg-mono text-[15px] font-semibold leading-none text-[var(--text-primary)]">{modelCount}</span>
          </div>
          <div className="flex flex-1 items-center gap-2 px-3 py-1.5">
            <Users size={12} className="text-[var(--text-dim)]" />
            <span className="pg-eyebrow">账号</span>
            <span className="ml-auto pg-mono text-[15px] font-semibold leading-none text-[var(--text-primary)]">
              {accountCount === 0 ? "—" : `${selectedAccountIds.size}/${accountCount}`}
            </span>
          </div>
          <div className="flex flex-1 items-center gap-2 px-3 py-1.5">
            <ShieldCheck size={12} style={{ color: rateTone }} />
            <span className="pg-eyebrow">健康率</span>
            <span className="ml-auto pg-mono text-[15px] font-semibold leading-none" style={{ color: rateTone }}>
              {accountCount === 0 ? "—" : `${healthyRate}%`}
            </span>
          </div>
        </div>

        {/* 账号配置 */}
        {accounts.length === 0 ? (
          <div className="mt-4 flex flex-1 flex-col items-center justify-center rounded-md border border-dashed py-8 text-center" style={{ borderColor: "var(--border-default)" }}>
            <Users size={20} className="mb-2 text-[var(--text-dim)]" />
            <div className="text-[12px] text-[var(--text-secondary)]">该供应商暂无可路由账号</div>
            <div className="mt-1 text-[11px] text-[var(--text-dim)]">请先在「模型供应商」页面对其配置账号</div>
          </div>
        ) : (
          <div className="mt-3 flex min-h-0 flex-1 flex-col">
            <div className="mb-1.5 flex shrink-0 items-center justify-between gap-2">
              <div className="flex items-center gap-1.5">
                <span className="text-[12px] font-medium text-[var(--text-secondary)]">参与调度的账号</span>
                <span className="pg-mono text-[10.5px] text-[var(--text-dim)]">{selectedAccountIds.size}/{allRoutableIds.length}</span>
              </div>
              {checked ? (
                <div className="flex items-center gap-1">
                  {(["全选", "仅健康", "重置"] as const).map((label) => (
                    <button
                      key={label}
                      type="button"
                      className="h-6 cursor-pointer rounded-md border border-[var(--border-default)] px-2 text-[10.5px] text-[var(--text-secondary)] transition-colors duration-150 hover:border-[var(--color-brand)]/50 hover:bg-[var(--color-brand-subtle)] hover:text-[var(--color-brand)]"
                      onClick={() => {
                        if (label === "全选") selectAllAccounts(providerId);
                        else if (label === "仅健康") selectAllHealthyAccounts(providerId);
                        else restoreAccountSelection(providerId);
                      }}
                    >
                      {label}
                    </button>
                  ))}
                </div>
              ) : (
                <span className="text-[10.5px] text-[var(--text-dim)]">勾选供应商后可调整选择范围</span>
              )}
            </div>
            <div className={`min-h-0 flex-1 space-y-1 overflow-y-auto pr-1 ${checked ? "" : "pointer-events-none opacity-50"}`}>
              {accounts.map((account) => {
                const isChecked = selectedAccountIds.has(account.id);
                const isHealthy = account.routable && !["error", "exhausted", "token_expired"].includes(account.status);
                return (
                  <label
                    key={account.id}
                    title={account.email || account.name}
                    className={`flex h-[36px] cursor-pointer items-center gap-2.5 rounded-md border px-2.5 transition-colors duration-150 ${isChecked ? "border-[var(--color-brand)]/40 bg-[var(--color-brand-subtle)]/40" : "border-transparent hover:bg-[var(--bg-hover)]"} ${!account.routable ? "cursor-not-allowed opacity-40" : ""}`}
                  >
                    <span className={`flex h-[13px] w-[13px] shrink-0 items-center justify-center rounded-[4px] border-[1.5px] transition-colors ${isChecked ? "border-[var(--color-brand)] bg-[var(--color-brand)] text-white" : "border-[var(--border-strong)]"}`}>
                      {isChecked && <Check size={9} strokeWidth={3.5} />}
                    </span>
                    <input type="checkbox" className="sr-only" checked={isChecked} disabled={!account.routable || !checked} onChange={() => handleToggleAccount(providerId, account.id)} />
                    <div className="min-w-0 flex-1 truncate text-[12px] text-[var(--text-primary)]">{account.name}</div>
                    <Badge variant={isHealthy ? "ok" : "err"} dot className="!text-[10px]">
                      {isHealthy ? "健康" : "异常"}
                    </Badge>
                  </label>
                );
              })}
            </div>
          </div>
        )}
      </div>
    );
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

      <Modal
        open={showMembers && !!selectedGroup}
        onClose={() => {
          setShowMembers(false);
          setProviderDraft([]);
          setAccountDraft({});
          setProviderSearch("");
          setProviderFilter("all");
          setFocusedProviderId(null);
        }}
        title={`管理模型供应商 · ${selectedGroup?.name || ""}`}
        className="!max-w-[1180px] !w-[calc(100vw-48px)]"
        style={{ maxWidth: "min(1180px, calc(100vw - 48px))" }}
        contentClassName="!p-0 flex flex-col overflow-hidden"
      >
        {/* 工具栏：一行说明 + 搜索 + 分段 + 全选 */}
        <div className="shrink-0 px-5 pt-3.5 pb-3 border-b" style={{ borderColor: "var(--border-default)" }}>
          <p className="text-[11.5px] leading-5 text-[var(--text-dim)] flex items-start gap-1.5">
            <Layers3 size={12} className="mt-[2px] shrink-0 text-[var(--color-brand)]" />
            勾选供应商加入「{selectedGroup?.name}」后可自动继承其全部模型，右侧可调整参与调度的账号。
          </p>
          <div className="mt-2.5 flex items-center gap-2">
            <div className="relative min-w-0 flex-1">
              <Search size={13} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--text-dim)]" />
              <input
                type="text"
                placeholder="搜索供应商名称..."
                value={providerSearch}
                onChange={(e) => setProviderSearch(e.target.value)}
                className="h-8 w-full rounded-md border bg-[var(--bg-inset)] pr-3 text-[12px] text-[var(--text-primary)] outline-none transition-colors placeholder:text-[var(--text-dim)] focus:border-[var(--color-brand)] focus:ring-2 focus:ring-[var(--color-brand)]/25"
                style={{ borderColor: "var(--border-default)", paddingLeft: "30px" }}
              />
            </div>
            <Segmented
              value={providerFilter}
              onChange={setProviderFilter}
              options={[
                { value: "all", label: `全部 ${providerEntries.length}` },
                { value: "in_pool", label: `池内 ${currentPoolProviders.size}` },
                { value: "custom", label: `自定义 ${customProviderEntries.length}` },
              ]}
            />
            <button
              type="button"
              onClick={selectVisibleProviders}
              disabled={!visibleUncheckedCount}
              className="flex h-8 shrink-0 cursor-pointer items-center gap-1 rounded-md border border-[var(--border-default)] px-2.5 text-[11px] font-medium text-[var(--text-secondary)] transition-colors duration-150 hover:border-[var(--color-brand)]/50 hover:bg-[var(--color-brand-subtle)] hover:text-[var(--color-brand)] disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-[var(--text-secondary)]"
            >
              <Check size={11} />
              全选
            </button>
          </div>
        </div>
        {/* 主体：左栏供应商列表 + 右栏聚焦详情 */}
        <div className="flex min-h-0 flex-1 overflow-hidden">
          <div className="w-[280px] shrink-0 overflow-y-auto overflow-x-hidden border-r py-1.5" style={{ borderColor: "var(--border-default)" }}>
            {availableModelsLoading ? (
              <div className="flex items-center justify-center gap-2.5 py-20 text-[12px] text-[var(--text-dim)]"><Spinner size={18} /> 正在加载供应商...</div>
            ) : availableModelsError ? (
              <div className="flex flex-col items-center justify-center py-20 text-center px-4">
                <div className="text-[12px] text-[var(--color-err)]">可用供应商加载失败</div>
                <Button className="mt-3" size="sm" variant="outline" onClick={() => refetchAvailableModels()}><RefreshCw size={12} /> 重试</Button>
              </div>
            ) : visibleProviderEntries.length ? (
              <>
                <div className="px-2 pb-1 pt-0.5 text-[10px] tracking-wide text-[var(--text-dim)] flex items-center justify-between">
                  <span className="pg-eyebrow">供应商 · {visibleProviderEntries.length}</span>
                  {providerSearch.trim() && <span className="text-[var(--color-brand)]">搜索匹配</span>}
                </div>
                <div className="space-y-0.5">
                  {visibleProviderEntries.map(renderProviderRow)}
                </div>
              </>
            ) : providerEntries.length ? (
              <div className="flex flex-col items-center py-16 text-center px-4">
                <Search size={20} className="mb-3 text-[var(--text-dim)]" />
                <div className="text-[12px] text-[var(--text-secondary)]">未找到匹配「{providerSearch}」的供应商</div>
                <div className="mt-1 text-[11px] text-[var(--text-dim)]">试试其他关键词或切换筛选</div>
              </div>
            ) : (
              <div className="flex flex-col items-center justify-center py-20 text-center px-4">
                <Box size={24} className="mb-3 text-[var(--text-dim)]" />
                <div className="text-[12px] text-[var(--text-secondary)]">没有与当前协议兼容的供应商</div>
                <div className="mt-1 text-[11px] text-[var(--text-dim)]">请先在「模型供应商」页面接入并配置 API Key 或账号。</div>
              </div>
            )}
          </div>
          <div className="min-w-0 flex-1 overflow-x-hidden overflow-y-auto p-4">
            {renderFocusedDetail()}
          </div>
        </div>
        {/* 底部：变更摘要 + 清空 + 取消 + 保存 */}
        <div className="flex shrink-0 items-center justify-between gap-3 border-t px-5 py-3" style={{ borderColor: "var(--border-default)", background: "var(--bg-elevated)" }}>
          <div className="flex min-w-0 items-center gap-2.5 text-[11px] text-[var(--text-dim)]">
            {providerDraft.length > 0 ? (
              <>
                <span>已选 <span className="font-semibold text-[var(--text-primary)]">{providerDraft.length}</span> 个供应商</span>
                {pendingAddCount > 0 && (
                  <span className="inline-flex items-center gap-1 text-[var(--color-ok)]">
                    <span className="h-1 w-1 rounded-full bg-[var(--color-ok)]" />
                    加入 {pendingAddCount}
                  </span>
                )}
                {pendingRemoveCount > 0 && (
                  <span className="inline-flex items-center gap-1 text-[var(--color-warn)]">
                    <span className="h-1 w-1 rounded-full bg-[var(--color-warn)]" />
                    移除 {pendingRemoveCount}
                  </span>
                )}
                <button
                  type="button"
                  onClick={handleClearPool}
                  disabled={!groupModels.length}
                  className="ml-1 inline-flex h-7 cursor-pointer items-center gap-1 rounded-md px-2 text-[11px] text-[var(--color-err)]/80 transition-colors duration-150 hover:bg-[var(--color-err-bg)] hover:text-[var(--color-err)] disabled:cursor-not-allowed disabled:opacity-40"
                >
                  <Trash2 size={11} />
                  清空池
                </button>
              </>
            ) : currentPoolProviders.size > 0 ? (
              <>
                <span>池内 {currentPoolProviders.size} 个供应商，未勾选任何供应商</span>
                <button
                  type="button"
                  onClick={handleClearPool}
                  disabled={!groupModels.length}
                  className="ml-1 inline-flex h-7 cursor-pointer items-center gap-1 rounded-md px-2 text-[11px] text-[var(--color-err)]/80 transition-colors duration-150 hover:bg-[var(--color-err-bg)] hover:text-[var(--color-err)] disabled:cursor-not-allowed disabled:opacity-40"
                >
                  <Trash2 size={11} />
                  清空池
                </button>
              </>
            ) : (
              <span>未选择任何供应商</span>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            <Button variant="secondary" onClick={() => {
              setShowMembers(false);
              setProviderDraft([]);
              setAccountDraft({});
              setProviderSearch("");
              setProviderFilter("all");
              setFocusedProviderId(null);
            }}>取消</Button>
            <Button size="lg" onClick={handleAddProviders} loading={addGroupModels.isPending || removeGroupModel.isPending || setGroupModelAccounts.isPending} disabled={!providerDraft.length && !currentPoolProviders.size}><Check size={14} /> 保存更改</Button>
          </div>
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
