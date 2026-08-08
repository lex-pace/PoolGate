import React, { useEffect, useMemo, useRef, useState } from "react";
import { Card } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { StatusDot } from "@/components/ui/StatusDot";
import { Input, Tabs } from "@/components/ui/Input";
import { Spinner } from "@/components/ui/Spinner";
import { ImportCenter } from "./ImportCenter";
import {
  useAccounts,
  useAccountRequestCounts,
  useBatchCheckHealth,
  useBatchRefreshAccountModels,
  useBatchRefreshQuotas,
  useBatchRefreshTokens,
  useBatchUpdateAccounts,
  useBatchDeleteAccounts,
  useCheckAccountHealth,
  useDeleteAccount,
  useProviders,
  useProxyStatus,
  useRefreshAccountModels,
  useRefreshAccountQuota,
  useRefreshAccountToken,
  useStartProxy,
  useStopProxy,
  useUpdateAccount,
  useTestAccountConnection,
  useCreateProvider,
  useUpdateProvider,
  useDeleteProvider,
} from "@/hooks/use-tauri";
import { useToast } from "@/components/ui/Toast";
import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import { getTimeAgo } from "@/lib/utils";
import {
  getResourceTemplate,
  modelResourceCategoryLabels,
  modelResourceTemplates,
  protocolOptions,
  resourceCategoryForTemplate,
  type ModelResourceCategory,
} from "@/lib/model-resource-templates";
import { exportAccount, type Account, type Provider, type QuotaWindow } from "@/lib/tauri-commands";
import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "@tauri-apps/plugin-fs";
import {
  Boxes,
  CheckCircle,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  CircleGauge,
  CloudCog,
  Copy,
  Download,
  Eye,
  EyeOff,
  LayoutGrid,
  Layers3,
  List,
  Pencil,
  Play,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Rows3,
  Route,
  Search,
  Sparkles,
  Square,
  Trash2,
  Waypoints,
  X,
  XCircle,
  Zap,
} from "lucide-react";

const statusBadge: Record<string, { v: "ok" | "warn" | "err" | "mute"; label: string }> = {
  active: { v: "ok", label: "可用" },
  limited: { v: "warn", label: "受限" },
  exhausted: { v: "err", label: "耗尽" },
  unchecked: { v: "mute", label: "未检查" },
  error: { v: "err", label: "异常" },
  disabled: { v: "mute", label: "已停用" },
  token_expired: { v: "err", label: "Token 过期" },
};

const credentialLabels: Record<string, string> = {
  api_key: "API Key",
  upstream_key: "上游网关",
  oauth: "OAuth",
  token: "Token",
  codex_oauth: "Coding Plan",
};

const sourceLabels: Record<string, string> = {
  api_key_text: "手动接入",
  codex_auth: "Codex auth.json",
  sub2api: "Sub2API",
  cpa: "CPA",
  cockpit: "Cockpit",
  json: "JSON",
  csv: "CSV",
  oauth: "OAuth 浏览器授权",
};

type ResourceCategory = "all" | ModelResourceCategory;

const categoryLabels: Record<ResourceCategory, string> = {
  all: "全部资源",
  ...modelResourceCategoryLabels,
};

const PAGE_SIZE = 15;

type ViewMode = "card" | "list" | "compact";

const VIEW_STORAGE_KEY = "poolgate.modelResources.viewMode";

const viewOptions: { id: ViewMode; label: string; icon: typeof LayoutGrid }[] = [
  { id: "card", label: "卡片", icon: LayoutGrid },
  { id: "list", label: "列表", icon: List },
  { id: "compact", label: "紧凑", icon: Rows3 },
];

function loadViewMode(): ViewMode {
  if (typeof window === "undefined") return "card";
  const stored = window.localStorage.getItem(VIEW_STORAGE_KEY);
  return stored === "card" || stored === "list" || stored === "compact" ? stored : "card";
}

function parseModels(account: Account, provider?: Provider) {
  const raw = account.models || provider?.models || "";
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) return parsed.filter(Boolean).map(String);
  } catch {
    // Support comma-separated legacy values.
  }
  return raw.split(",").map((item) => item.trim()).filter(Boolean);
}

function parseTags(account: Account): string[] {
  if (!account.tags) return [];
  try {
    const parsed = JSON.parse(account.tags);
    if (Array.isArray(parsed)) return parsed.filter(Boolean).map(String);
  } catch {
    // Support comma-separated legacy values.
  }
  return account.tags.split(",").map((tag) => tag.trim()).filter(Boolean);
}

function findResourceTemplate(account: Account, provider?: Provider) {
  const tags = parseTags(account);
  const taggedTemplate = tags
    .map((tag) => getResourceTemplate(tag))
    .find(Boolean);
  if (taggedTemplate) return taggedTemplate;

  const signature = `${provider?.name || ""} ${provider?.base_url || ""}`.toLowerCase();
  return modelResourceTemplates.find((template) => {
    if (template.id === "custom") return false;
    const name = template.name.toLowerCase();
    return signature.includes(template.id.toLowerCase()) || signature.includes(name);
  });
}

function resourceCategory(account: Account, provider?: Provider): ResourceCategory {
  const tags = parseTags(account);
  const explicit = tags.find((tag) => tag.startsWith("resource_category:"))?.split(":")[1];
  if (explicit && explicit in modelResourceCategoryLabels) return explicit as ModelResourceCategory;

  if (account.credential_type === "codex_oauth" || account.credential_type === "oauth") return "coding_plan";
  if (["sub2api", "cpa", "cockpit"].includes(account.source_format || "") || account.credential_type === "upstream_key") return "gateway";

  const template = findResourceTemplate(account, provider);
  if (template) return resourceCategoryForTemplate(template);

  const signature = `${provider?.name || ""} ${provider?.base_url || ""}`.toLowerCase();
  if (signature.includes("free") || signature.includes("免费")) return "free";
  if (!provider || provider.type === "local" || signature.includes("custom") || signature.includes("自定义")) return "custom";
  return "api_key";
}

function supportsQuota(account: Account, provider?: Provider): boolean {
  return account.credential_type === "codex_oauth"
    || account.source_format === "codex_auth"
    || provider?.type?.toLowerCase() === "codex"
    || provider?.type?.toLowerCase() === "antigravity"
    || provider?.name?.toLowerCase().includes("antigravity") === true;
}

function quotaUnsupported(account: Account, provider?: Provider) {
  return !supportsQuota(account, provider)
    || account.quota_error?.includes("尚未配置在线额度适配器") === true;
}

function sourceDescription(account: Account, provider?: Provider) {
  if (account.credential_type === "codex_oauth") return "OpenAI Codex Coding Plan";
  if (account.credential_type === "oauth") return `${provider?.name || "Agent"} Coding Plan`;
  if (["sub2api", "cpa", "cockpit"].includes(account.source_format || "")) {
    return sourceLabels[account.source_format || ""];
  }
  return provider?.name || sourceLabels[account.source_format || ""] || "自定义模型供应商";
}

function isDirectlyRoutable(account: Account) {
  const credentialType = account.credential_type || "api_key";
  return ["api_key", "upstream_key", "oauth", "token", "codex_oauth"].includes(credentialType);
}

function isUnavailableAccount(account: Account) {
  return account.status === "exhausted"
    || account.status === "error"
    || account.status === "token_expired"
    || account.health_status === "error";
}

function effectiveStatusBadge(account: Account) {
  if (account.health_status === "error" && !["exhausted", "token_expired"].includes(account.status || "")) {
    return { v: "err" as const, label: "健康异常" };
  }
  return statusBadge[account.status || "unchecked"] || statusBadge.unchecked;
}

function parseAccountProtocols(value: Account["protocols"] | string[] | null | undefined): string[] {
  if (Array.isArray(value)) return value.filter(Boolean).map(String);
  if (!value) return [];
  try {
    const parsed = JSON.parse(value);
    if (Array.isArray(parsed)) return parsed.filter(Boolean).map(String);
  } catch {
    // Fall back to comma-separated legacy data.
  }
  return value.split(",").map((protocol) => protocol.trim()).filter(Boolean);
}

function parseQuotaWindows(account: Account): QuotaWindow[] {
  if (!account.quota_windows) return [];
  try {
    const parsed = JSON.parse(account.quota_windows);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

function resetLabel(window: QuotaWindow) {
  if (window.reset_after_seconds != null) {
    const minutes = Math.max(0, Math.ceil(window.reset_after_seconds / 60));
    if (minutes < 60) return `${minutes} 分钟后重置`;
    return `${Math.ceil(minutes / 60)} 小时后重置`;
  }
  if (window.reset_at) return new Date(window.reset_at * 1000).toLocaleString("zh-CN");
  return "重置时间未知";
}

/** Generate a fresh provider id matching the `prov_<32hex>` convention. */
function newProviderId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return `prov_${crypto.randomUUID().replace(/-/g, "")}`;
  }
  return `prov_${Date.now().toString(16)}${Math.random().toString(16).slice(2, 18)}`;
}

/**
 * Apply a new connector Base URL while keeping the protocol-specific
 * `base_urls` mapping coherent: every protocol entry that pointed at the old
 * URL follows the change, while distinct per-protocol URLs are preserved
 * (otherwise routing would still use the stale `base_urls` entry).
 */
function buildProviderWithBaseUrl(provider: Provider, nextUrl: string): Provider {
  const original = provider.base_url || "";
  let base_urls = provider.base_urls;
  if (provider.base_urls) {
    try {
      const map = JSON.parse(provider.base_urls) as Record<string, string>;
      let changed = false;
      for (const protocol of Object.keys(map)) {
        if (map[protocol] === original) {
          map[protocol] = nextUrl;
          changed = true;
        }
      }
      if (changed) base_urls = JSON.stringify(map);
    } catch {
      // Non-JSON legacy value: leave untouched.
    }
  }
  return { ...provider, base_url: nextUrl, base_urls };
}

export default function ModelResources() {
  const { data: rawAccounts = [], isLoading } = useAccounts();
  const { data: providers = [] } = useProviders();
  const { data: proxy } = useProxyStatus();
  const startProxy = useStartProxy();
  const stopProxy = useStopProxy();
  const deleteAccount = useDeleteAccount();
  const checkHealth = useCheckAccountHealth();
  const batchHealth = useBatchCheckHealth();
  const refreshToken = useRefreshAccountToken();
  const batchRefreshTokens = useBatchRefreshTokens();
  const refreshQuota = useRefreshAccountQuota();
  const batchRefreshQuotas = useBatchRefreshQuotas();
  const refreshModels = useRefreshAccountModels();
  const batchRefreshModels = useBatchRefreshAccountModels();
  const batchUpdate = useBatchUpdateAccounts();
  const batchDelete = useBatchDeleteAccounts();
  const { toast } = useToast();
  const { data: requestCounts = {} } = useAccountRequestCounts(7);

  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<ResourceCategory>("all");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [expanded, setExpanded] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);
  const [draft, setDraft] = useState<Account | null>(null);
  const [showImport, setShowImport] = useState(false);
  const [showConnector, setShowConnector] = useState(false);
  const [page, setPage] = useState(1);
  const [refreshingAccountIds, setRefreshingAccountIds] = useState<Set<string>>(new Set());
  const [isRefreshingAll, setIsRefreshingAll] = useState(false);
  const updateAccount = useUpdateAccount();
  const createProvider = useCreateProvider();
  const updateProvider = useUpdateProvider();
  const deleteProvider = useDeleteProvider();
  const [draftBaseUrl, setDraftBaseUrl] = useState<string | null>(null);
  const [deletingAccount, setDeletingAccount] = useState<Account | null>(null);
  const [batchDeleteConfirmOpen, setBatchDeleteConfirmOpen] = useState(false);
  const handleSave = async () => {
    const base = resources.find((r) => r.account.id === expanded);
    if (!base) return;
    const merged = {
      ...base.account,
      ...(draft ?? {}),
    } as Account;
    // models: 用户输入的是逗号分隔的模型名，转为 JSON 数组持久化
    const modelsStr = merged.models ?? "";
    let normalizedModels = modelsStr;
    try {
      // 如果已经是 JSON 数组则保留
      const parsed = JSON.parse(modelsStr);
      if (!Array.isArray(parsed)) throw new Error();
    } catch {
      normalizedModels = JSON.stringify(
        modelsStr.split(",").map((m) => m.trim()).filter(Boolean)
      );
    }
    await updateAccount.mutateAsync({
      ...merged,
      models: normalizedModels,
      protocols: JSON.stringify(parseAccountProtocols(merged.protocols)),
    });
    // If base_url was changed and a connector exists, apply it. Base URL is a
    // CONNECTOR-level property: when several accounts share one connector,
    // rewriting it would silently change the upstream of every other account
    // too (e.g. 「小七中转站」跟着「Agent Router」一起被改). In that case the
    // account is detached into its own dedicated connector instead.
    if (draftBaseUrl !== null && base.connector) {
      const connector = base.connector;
      const originalUrl = connector.base_url || "";
      const nextUrl = draftBaseUrl.trim();
      if (nextUrl && nextUrl !== originalUrl) {
        const sharedByOthers = resources.some(
          (resource) => resource.connector?.id === connector.id && resource.account.id !== merged.id,
        );
        let createdProviderId: string | null = null;
        try {
          if (sharedByOthers) {
            // 拆分为独立连接器：只影响当前账号，其他账号继续用原连接器。
            const newId = newProviderId();
            createdProviderId = newId;
            await createProvider.mutateAsync(
              buildProviderWithBaseUrl(
                { ...connector, id: newId, name: merged.name || connector.name } as Provider,
                nextUrl,
              ),
            );
            await updateAccount.mutateAsync({
              ...merged,
              provider_id: newId,
              models: normalizedModels,
              protocols: JSON.stringify(parseAccountProtocols(merged.protocols)),
            } as Account);
            toast("success", `已为「${merged.name}」创建独立连接器，其他资源不受影响`);
          } else {
            await updateProvider.mutateAsync(buildProviderWithBaseUrl(connector, nextUrl));
          }
        } catch (err) {
          // 拆分中途失败时回收刚创建的孤儿连接器，避免残留无用数据。
          if (createdProviderId) {
            try {
              await deleteProvider.mutateAsync(createdProviderId);
            } catch {
              // 回收失败仅影响清理，不掩盖原始错误。
            }
          }
          toast("warning", `账号已更新，但 Base URL 更新失败：${String(err)}`);
        }
      }
    }
    setEditing(false);
    setShowApiKey(false);
    setDraftBaseUrl(null);
    toast("success", "模型供应商已更新");
  };
  const [viewMode, setViewMode] = useState<ViewMode>(loadViewMode);
  const testAccountConnection = useTestAccountConnection();
  const [testingAccountId, setTestingAccountId] = useState<string | null>(null);
  const [testResults, setTestResults] = useState<Record<string, { success: boolean; message: string; latency_ms: number; model_tested?: string; error_details?: string }>>({});
  const checkRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (typeof window !== "undefined") window.localStorage.setItem(VIEW_STORAGE_KEY, viewMode);
  }, [viewMode]);

  const providerMap = useMemo(
    () => new Map(providers.map((provider) => [provider.id, provider])),
    [providers],
  );

  const resources = useMemo(() => rawAccounts.map((account) => {
    const connector = account.provider_id ? providerMap.get(account.provider_id) : undefined;
    const quotaWindows = parseQuotaWindows(account);
    const primaryQuota = quotaWindows[0];
    const remainingPercent = Math.round(
      primaryQuota?.remaining_percent ??
      (account.quota_limit ? Math.min(100, ((account.quota_limit - (account.quota_used || 0)) / account.quota_limit) * 100) : 100)
    );
    return {
      account,
      connector,
      category: resourceCategory(account, connector),
      models: parseModels(account, connector),
      source: sourceDescription(account, connector),
      routable: isDirectlyRoutable(account),
      unavailable: isUnavailableAccount(account),
      remainingPercent,
      createdAt: account.created_at || "9999",
      callCount: requestCounts[account.id] || 0,
    };
  }).sort((left, right) => {
    // 1. Unavailable accounts go to the bottom
    if (left.unavailable !== right.unavailable) return left.unavailable ? 1 : -1;

    // 2. By remaining quota percent (lower remaining = higher priority)
    if (left.remainingPercent !== right.remainingPercent) {
      return left.remainingPercent - right.remainingPercent;
    }

    // 3. By call count (more calls = higher priority)
    if (left.callCount !== right.callCount) {
      return right.callCount - left.callCount;
    }

    // 4. By import order (older created_at = higher priority)
    return left.createdAt.localeCompare(right.createdAt);
  }), [rawAccounts, providerMap, requestCounts]);

  const list = resources.filter(({ account, connector, category, models, source }) => {
    const q = search.toLowerCase();
    const matchSearch = !search || [
      account.name,
      account.email,
      account.external_account_id,
      connector?.name,
      connector?.protocol,
      source,
      ...models,
    ].some((value) => value?.toLowerCase().includes(q));
    return matchSearch && (filter === "all" || category === filter);
  });

  const totalPages = Math.max(1, Math.ceil(list.length / PAGE_SIZE));
  const pagedList = list.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);
  const isPartial = selected.size > 0 && selected.size < list.length;

  useEffect(() => {
    if (checkRef.current) checkRef.current.indeterminate = isPartial;
  }, [isPartial]);

  useEffect(() => {
    setPage(1);
  }, [search, filter]);

  const unavailableResources = resources.filter(({ account }) => isUnavailableAccount(account));

  const summary = {
    total: resources.length,
    ready: resources.filter(({ account, routable }) => routable && account.status !== "disabled" && !isUnavailableAccount(account)).length,
    models: new Set(resources.flatMap((resource) => resource.models)).size,
    codingPlans: resources.filter((resource) => resource.category === "coding_plan").length,
    connectorCount: new Set(resources.map((resource) => resource.account.provider_id).filter(Boolean)).size,
  };

  const categoryTabs = (Object.keys(categoryLabels) as ResourceCategory[]).map((id) => ({
    id,
    label: categoryLabels[id],
    count: id === "all" ? resources.length : resources.filter((resource) => resource.category === id).length,
  }));

  const toggle = (id: string) => {
    const next = new Set(selected);
    next.has(id) ? next.delete(id) : next.add(id);
    setSelected(next);
  };

  const handleBatchHealth = async () => {
    const ids = selected.size ? Array.from(selected) : resources.map((resource) => resource.account.id);
    if (!ids.length) return;
    try {
      await batchHealth.mutateAsync(ids);
      toast("success", `资源测活完成：${ids.length} 个`);
      setSelected(new Set());
    } catch (error) {
      toast("error", `测活失败：${String(error)}`);
    }
  };

  const handleBatchRefresh = async (kind: "token" | "quota") => {
    const targets = selected.size
      ? resources.filter((resource) => selected.has(resource.account.id))
      : list;
    const ids = targets.map((resource) => resource.account.id);
    if (!ids.length) {
      toast("warning", "当前筛选范围没有可刷新的资源");
      return;
    }
    try {
      const results = kind === "token"
        ? await batchRefreshTokens.mutateAsync(ids)
        : await batchRefreshQuotas.mutateAsync(ids);
      const success = results.filter((result) => result.success).length;
      const unsupported = kind === "quota"
        ? results.filter((result) => result.message.includes("尚未配置在线额度适配器")).length
        : 0;
      const failed = results.length - success - unsupported;
      const details = [
        `成功 ${success}`,
        unsupported ? `不支持 ${unsupported}` : "",
        failed ? `失败 ${failed}` : "",
      ].filter(Boolean).join("，");
      toast(failed || unsupported ? "warning" : "success", `${kind === "token" ? "Token" : "额度"}刷新完成：${details}`);
      if (selected.size) setSelected(new Set());
    } catch (error) {
      toast("error", `刷新失败：${String(error)}`);
    }
  };

  const handleRefreshAll = async () => {
    const targets = selected.size
      ? resources.filter((resource) => selected.has(resource.account.id))
      : list;
    const ids = targets.map((resource) => resource.account.id);
    if (!ids.length) {
      toast("warning", "当前筛选范围没有可刷新的资源");
      return;
    }

    setIsRefreshingAll(true);
    setRefreshingAccountIds((current) => new Set([...current, ...ids]));
    try {
      const quotaIds = targets
        .filter(({ account, connector }) => supportsQuota(account, connector) && !quotaUnsupported(account, connector))
        .map(({ account }) => account.id);
      const [healthResults, modelResults, quotaResults] = await Promise.all([
        batchHealth.mutateAsync(ids),
        batchRefreshModels.mutateAsync(ids),
        quotaIds.length ? batchRefreshQuotas.mutateAsync(quotaIds) : Promise.resolve([]),
      ]);
      const healthSuccess = healthResults.filter((result) => result.status === "healthy").length;
      const modelSuccess = modelResults.filter((result) => result.success).length;
      const quotaSuccess = quotaResults.filter((result) => result.success).length;
      const hasFailure = healthSuccess < ids.length || modelSuccess < ids.length || quotaSuccess < quotaIds.length;
      toast(
        hasFailure ? "warning" : "success",
        `全部刷新完成：状态 ${healthSuccess}/${ids.length}，模型 ${modelSuccess}/${ids.length}${quotaIds.length ? `，额度 ${quotaSuccess}/${quotaIds.length}` : ""}`,
      );
      if (selected.size) setSelected(new Set());
    } catch (error) {
      toast("error", `全部刷新失败：${String(error)}`);
    } finally {
      setRefreshingAccountIds((current) => {
        const next = new Set(current);
        ids.forEach((id) => next.delete(id));
        return next;
      });
      setIsRefreshingAll(false);
    }
  };

  const handleCardRefresh = async (account: Account, quotaSupported: boolean) => {
    if (isRefreshingAll || refreshingAccountIds.has(account.id)) return;
    setRefreshingAccountIds((current) => new Set(current).add(account.id));
    try {
      const [healthResult, modelResult, quotaResult] = await Promise.all([
        checkHealth.mutateAsync(account.id),
        refreshModels.mutateAsync(account.id),
        quotaSupported ? refreshQuota.mutateAsync(account.id) : Promise.resolve(null),
      ]);
      const healthOk = healthResult.status === "healthy";
      const quotaOk = !quotaResult || quotaResult.success;
      const success = healthOk && modelResult.success && quotaOk;
      const details = [
        healthOk ? "资源可用" : healthResult.message,
        modelResult.message,
        quotaResult?.message,
      ].filter(Boolean).join("；");
      toast(success ? "success" : "warning", details || "资源已刷新");
    } catch (error) {
      toast("error", `刷新失败：${String(error)}`);
    } finally {
      setRefreshingAccountIds((current) => {
        const next = new Set(current);
        next.delete(account.id);
        return next;
      });
    }
  };

  const handleTestAccount = async (account: Account) => {
    setTestingAccountId(account.id);
    try {
      const result = await testAccountConnection.mutateAsync(account.id);
      setTestResults((prev) => ({ ...prev, [account.id]: result }));
      toast(result.success ? "success" : "error", result.success ? `${result.message}${result.model_tested ? ` · 模型: ${result.model_tested}` : ""}` : `测试失败: ${result.error_details || result.message}`);
    } catch (error) {
      setTestResults((prev) => ({ ...prev, [account.id]: { success: false, message: "测试失败", latency_ms: 0, error_details: String(error) } }));
      toast("error", `测试失败: ${String(error)}`);
    } finally {
      setTestingAccountId(null);
    }
  };

  const getTestResultIcon = (accountId: string) => {
    const result = testResults[accountId];
    if (!result) return null;
    return result.success ? <CheckCircle size={12} className="text-[var(--color-ok)]" /> : <XCircle size={12} className="text-[var(--color-err)]" />;
  };

  const getTestResultTooltip = (accountId: string) => {
    const result = testResults[accountId];
    if (!result) return "测试连接 · 发送最小 Chat 请求验证连通性";
    if (result.success) {
      const reply = result.error_details ? `\n回复: "${result.error_details}"` : "";
      return `✓ ${result.message}${result.model_tested ? ` · 模型: ${result.model_tested}` : ""}${reply}`;
    }
    return `✗ ${result.message}${result.error_details ? `\n${result.error_details}` : ""}`;
  };

  const handleExportAccount = async (account: Account) => {
    try {
      const json = await exportAccount(account.id, "cockpit", true);
      const safeName = (account.name || account.id).replace(/[^a-zA-Z0-9\u4e00-\u9fff._-]+/g, "-");
      const path = await save({
        defaultPath: `${safeName}.json`,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path) return;
      await writeTextFile(path, json);
      toast("success", "资源已安全导出，凭证默认脱敏");
    } catch (error) {
      toast("error", `导出失败：${String(error)}`);
    }
  };

  const handleDeleteAccount = (account: Account) => {
    // Secondary confirmation: only the in-app dialog's confirm button
    // actually deletes the provider resource.
    setDeletingAccount(account);
  };

  const confirmDeleteAccount = async () => {
    if (!deletingAccount) return;
    const account = deletingAccount;
    try {
      await deleteAccount.mutateAsync(account.id);
      if (expanded === account.id) setExpanded(null);
      setSelected((current) => {
        const next = new Set(current);
        next.delete(account.id);
        return next;
      });
      toast("success", "模型供应商已删除，路由池关联已清理");
    } catch (error) {
      toast("error", `删除失败：${String(error)}`);
    } finally {
      setDeletingAccount(null);
    }
  };

  const handleBatchStatus = async (status: string) => {
    const ids = Array.from(selected);
    try {
      await batchUpdate.mutateAsync({ ids, status });
      toast("success", `已更新 ${ids.length} 个模型供应商`);
      setSelected(new Set());
    } catch (error) {
      toast("error", `更新失败：${String(error)}`);
    }
  };

  const handleDeleteUnavailable = () => {
    if (!unavailableResources.length) {
      toast("info", "当前没有不可用模型供应商");
      return;
    }
    setBatchDeleteConfirmOpen(true);
  };

  const confirmDeleteUnavailable = async () => {
    const ids = unavailableResources.map(({ account }) => account.id);
    if (!ids.length) {
      setBatchDeleteConfirmOpen(false);
      return;
    }
    try {
      const result = await batchDelete.mutateAsync(ids);
      setSelected((current) => new Set([...current].filter((id) => !result.deleted_ids.includes(id))));
      if (result.failures.length) toast("warning", `已删除 ${result.deleted_ids.length} 个，${result.failures.length} 个删除失败`);
      else toast("success", `已删除 ${result.deleted_ids.length} 个不可用模型供应商`);
    } catch (error) {
      toast("error", `批量删除不可用资源失败：${String(error)}`);
    } finally {
      setBatchDeleteConfirmOpen(false);
    }
  };

  const activeResource = resources.find(({ account }) => account.id === expanded);

  const handleToggleProxy = async () => {
    try {
      if (proxy?.running) {
        await stopProxy.mutateAsync();
        toast("success", "代理池已停止");
      } else {
        await startProxy.mutateAsync();
        toast("success", "代理池已启动");
      }
    } catch (error: unknown) {
      toast("error", `操作失败: ${String(error)}`);
    }
  };

  const handleCopyEndpoint = () => {
    const endpoint = `http://127.0.0.1:${proxy?.port || 9800}`;
    void navigator.clipboard.writeText(endpoint);
    toast("success", "代理地址已复制");
  };

  return (
    <div className="space-y-4 animate-fade-in pg-page">
      {/* Proxy pool status banner */}
      <section
        className="pg-panel relative overflow-hidden px-5 py-4"
        style={{
          background: "linear-gradient(120deg, var(--bg-surface) 0%, var(--bg-surface) 64%, var(--color-brand-subtle) 100%)",
        }}
      >
        <div className="absolute -right-16 -top-20 w-56 h-56 rounded-full bg-[var(--color-brand-subtle)] blur-3xl pointer-events-none" />
        <div className="relative flex flex-col lg:flex-row lg:items-center justify-between gap-4 lg:gap-6">
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
                <span className="pg-eyebrow">Proxy Resource Pool</span>
                <Badge variant={proxy?.running ? "ok" : "err"}>{proxy?.running ? "代理运行中" : "代理未启动"}</Badge>
              </div>
              <h2 className="mt-1.5 text-[21px] leading-7 font-semibold tracking-[-0.025em] text-[var(--text-primary)]">
                {proxy?.running ? "代理池已就绪，Agent 可连接使用" : "启动代理池以使用模型供应商"}
              </h2>
              <div className="mt-1.5 flex flex-wrap items-center gap-2 text-[11px] text-[var(--text-dim)]">
                <button onClick={handleCopyEndpoint} className="pg-mono hover:text-[var(--color-brand)] transition-colors inline-flex items-center gap-1">
                  127.0.0.1:{proxy?.port || 9800} <Copy size={10} />
                </button>
                <span>·</span>
                <span>{summary.ready}/{summary.total} 个资源可用</span>
                <span>·</span>
                <span>OpenAI / Anthropic / Gemini compatible</span>
              </div>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2 shrink-0">
            <Button size="sm" onClick={() => setShowImport(true)}>
              <Plus size={14} /> 接入模型供应商
            </Button>
            <Button size="sm" variant="secondary" onClick={handleBatchHealth} loading={batchHealth.isPending}>
              <Zap size={14} /> 测活
            </Button>
            <Button
              variant={proxy?.running ? "danger" : "success"}
              size="sm"
              onClick={handleToggleProxy}
              loading={startProxy.isPending || stopProxy.isPending}
            >
              {proxy?.running ? <Square size={11} fill="currentColor" /> : <Play size={12} fill="currentColor" />}
              {proxy?.running ? "停止代理" : "启动代理"}
            </Button>
          </div>
        </div>
      </section>

      <section className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-3 min-w-0">
        {[
          { label: "模型供应商", value: summary.total, sub: "全部可管理凭证", icon: Layers3, tone: "var(--color-brand)" },
          { label: "可直接路由", value: summary.ready, sub: "已进入本地网关", icon: Route, tone: "var(--color-ok)" },
          { label: "模型能力", value: summary.models || "--", sub: "已发现模型数量", icon: Boxes, tone: "var(--color-info)" },
          { label: "Coding Plan", value: summary.codingPlans, sub: "Agent 登录资源", icon: Sparkles, tone: "var(--topology-violet)" },
          { label: "隐藏连接器", value: summary.connectorCount, sub: "由 PoolGate 自动维护", icon: CloudCog, tone: "var(--color-warn)" },
        ].map((item) => {
          const Icon = item.icon;
          return (
            <div key={item.label} className="pg-panel px-3.5 py-3">
              <div className="flex items-center justify-between gap-2">
                <span className="pg-eyebrow truncate">{item.label}</span>
                <Icon size={14} style={{ color: item.tone }} />
              </div>
              <div className="mt-2 text-[22px] leading-7 font-semibold pg-mono text-[var(--text-primary)]">{item.value}</div>
              <div className="mt-1 text-[9px] text-[var(--text-dim)] truncate">{item.sub}</div>
            </div>
          );
        })}
      </section>

      <div className="flex flex-wrap items-center gap-2.5 min-w-0">
        <div className="relative w-full sm:w-[260px] lg:w-[310px] shrink-0">
          <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--text-dim)]" />
          <Input className="pl-9 w-full" placeholder="搜索资源、模型或来源..." value={search} onChange={(event) => setSearch(event.target.value)} />
        </div>
        <div className="pg-horizontal-scroll flex-1 basis-[420px]">
          <Tabs tabs={categoryTabs} active={filter} onChange={(id) => setFilter(id as ResourceCategory)} className="min-w-max" />
        </div>
        <button
          type="button"
          onClick={() => void handleRefreshAll()}
          disabled={isRefreshingAll || refreshingAccountIds.size > 0 || list.length === 0}
          className="h-8 px-2.5 rounded-lg border shrink-0 inline-flex items-center gap-1.5 text-[11px] font-medium text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] disabled:opacity-50"
          style={{ borderColor: "var(--border-default)", background: "var(--bg-surface)" }}
          title={selected.size ? `刷新已选 ${selected.size} 个资源的状态、模型与额度` : `刷新当前筛选的 ${list.length} 个资源`}
        >
          {isRefreshingAll ? <Spinner className="h-3.5 w-3.5" /> : <RefreshCw size={13} />}
          <span className="hidden xl:inline">全部刷新</span>
          <span className="pg-mono text-[9px] text-[var(--text-dim)]">{selected.size || list.length}</span>
        </button>
        <div className="flex items-center gap-0.5 p-0.5 rounded-lg border shrink-0" style={{ borderColor: "var(--border-default)", background: "var(--bg-inset)" }}>
          {viewOptions.map((option) => {
            const Icon = option.icon;
            const isActive = viewMode === option.id;
            return (
              <button
                key={option.id}
                type="button"
                title={`${option.label}视图`}
                onClick={() => setViewMode(option.id)}
                className="w-7 h-7 rounded-md flex items-center justify-center transition-colors"
                style={{
                  background: isActive ? "var(--bg-surface-solid)" : "transparent",
                  color: isActive ? "var(--color-brand)" : "var(--text-dim)",
                  boxShadow: isActive ? "var(--shadow-card)" : "none",
                }}
              >
                <Icon size={14} strokeWidth={2} />
              </button>
            );
          })}
        </div>
        <label className="flex items-center gap-1.5 text-[11px] text-[var(--text-dim)] cursor-pointer shrink-0 select-none">
          <input ref={checkRef} type="checkbox" className="accent-[var(--color-brand)]" checked={selected.size === list.length && list.length > 0} onChange={() => setSelected(selected.size === list.length ? new Set() : new Set(list.map(({ account }) => account.id)))} />
          全选
        </label>
        {unavailableResources.length > 0 && (
          <Button
            size="sm"
            variant="danger"
            className="h-7 px-2.5 text-[10px] shrink-0"
            loading={batchDelete.isPending}
            onClick={() => handleDeleteUnavailable()}
          >
            <Trash2 size={12} /> 删除不可用资源 ({unavailableResources.length})
          </Button>
        )}
      </div>

      <div className="min-w-0">
        {isLoading ? (
          <Card className="p-8 text-center"><Spinner /><span className="ml-2 text-sm text-[var(--text-dim)]">加载中...</span></Card>
        ) : pagedList.length === 0 ? (
          <Card className="p-14 text-center">
            <Layers3 size={25} className="mx-auto text-[var(--text-dim)]" />
            <div className="mt-2 text-sm text-[var(--text-primary)]">暂无匹配的模型供应商</div>
            <div className="mt-1 text-[11px] text-[var(--text-dim)]">接入 Coding Plan、API Key、免费模型或批量账号文件</div>
            <Button size="sm" className="mt-3" onClick={() => setShowImport(true)}><Plus size={13} /> 接入模型供应商</Button>
          </Card>
        ) : (
          <div className={
            viewMode === "card"
              ? "grid gap-3 grid-cols-1 md:grid-cols-2 lg:grid-cols-3 auto-rows-fr min-w-0"
              : viewMode === "list"
                ? "flex flex-col gap-2"
                : "pg-panel divide-y overflow-hidden"
          } style={viewMode === "compact" ? { borderColor: "var(--border-subtle)" } : undefined}>
            {pagedList.map((resource) => {
              const { account, connector, models, source, routable } = resource;
              const badge = effectiveStatusBadge(account);
              const health = account.health_status || "unchecked";
              const quotaWindows = parseQuotaWindows(account);
              const primaryQuota = quotaWindows[0];
              const quotaSupported = supportsQuota(account, connector);
              const quotaIsUnsupported = quotaUnsupported(account, connector);
              const isRefreshing = refreshingAccountIds.has(account.id);
              const quotaPercent = primaryQuota?.used_percent ?? (account.quota_limit ? Math.min(100, ((account.quota_used || 0) / account.quota_limit) * 100) : 0);
              const remainingPercent = Math.round(primaryQuota?.remaining_percent ?? (100 - quotaPercent));
              const active = expanded === account.id;
              const dotStatus = health === "healthy" ? "ok" : health === "error" ? "err" : "mute";
              const onOpen = () => {
                setExpanded(active ? null : account.id);
                setShowConnector(false);
                // 切换供应商时重置编辑状态，避免旧 draft 混入新面板
                if (!active) {
                  setEditing(false);
                  setShowApiKey(false);
                  setDraft(null);
                  setDraftBaseUrl(null);
                }
              };

              // Compact view: single dense row
              if (viewMode === "compact") {
                return (
                  <div
                    key={account.id}
                    className={`flex items-center gap-3 px-3.5 py-2 cursor-pointer transition-colors ${active ? "bg-[var(--color-brand-subtle)]" : "hover:bg-[var(--bg-hover)]"}`}
                    onClick={onOpen}
                  >
                    <input type="checkbox" className="accent-[var(--color-brand)] shrink-0" checked={selected.has(account.id)} onClick={(event) => event.stopPropagation()} onChange={() => toggle(account.id)} />
                    <StatusDot status={dotStatus} pulse={health === "healthy"} />
                    <span className="text-[12.5px] font-medium text-[var(--text-primary)] truncate min-w-[120px] max-w-[220px]">{account.name || source}</span>
                    <span className="text-[11px] text-[var(--text-dim)] truncate hidden md:block flex-1">{source} · {categoryLabels[resource.category]}</span>
                    <span className="text-[11px] text-[var(--text-dim)] pg-mono hidden lg:block shrink-0">{models.length ? `${models.length} 模型` : (connector?.protocol || "待发现")}</span>
                    <span className="text-[11px] text-[var(--text-dim)] pg-mono shrink-0 w-20 text-right hidden sm:block">
                      {primaryQuota || account.quota_limit
                        ? `${remainingPercent}% 剩余`
                        : quotaIsUnsupported
                          ? "额度不支持"
                          : quotaSupported
                            ? "额度待刷新"
                            : "额度不支持"}
                    </span>
                    <span className="text-[10px] text-[var(--text-dim)] whitespace-nowrap shrink-0 w-16 text-right hidden xl:block">{getTimeAgo(account.last_used_at)}</span>
                    <div className="shrink-0">
                      {routable ? <Badge variant={badge.v} dot>{badge.label}</Badge> : <Badge variant="warn" dot>待适配</Badge>}
                    </div>
                  </div>
                );
              }

              // List view: horizontal row with more detail
              if (viewMode === "list") {
                return (
                  <div
                    key={account.id}
                    className={`pg-panel overflow-hidden cursor-pointer transition-all duration-200 ${active ? "ring-2 ring-[var(--color-brand)]" : "hover:border-[var(--border-strong)]"}`}
                    onClick={onOpen}
                  >
                    <div className="flex items-center gap-3 px-4 py-3">
                      <input type="checkbox" className="accent-[var(--color-brand)] shrink-0" checked={selected.has(account.id)} onClick={(event) => event.stopPropagation()} onChange={() => toggle(account.id)} />
                      <StatusDot status={dotStatus} pulse={health === "healthy"} />
                      <div className="min-w-0 flex-1 md:flex-none md:basis-[210px]">
                        <div className="text-[13px] font-semibold text-[var(--text-primary)] truncate">{account.name || source}</div>
                        <div className="mt-0.5 text-[11px] text-[var(--text-dim)] truncate">{source} · {categoryLabels[resource.category]}</div>
                      </div>
                      <div className="flex-1 min-w-0 hidden md:flex flex-wrap gap-1">
                        {(models.length ? models.slice(0, 4) : [connector?.protocol || "待发现"]).map((model) => <Badge key={model} variant="brand">{model}</Badge>)}
                        {models.length > 4 && <Badge variant="mute">+{models.length - 4}</Badge>}
                      </div>
                      {(primaryQuota || account.quota_limit) ? (
                        <div className="w-28 shrink-0 hidden lg:block">
                          <div className="flex justify-between text-[9px] text-[var(--text-dim)]">
                            <span className="truncate">{primaryQuota?.label || "额度"}</span>
                            <span>{remainingPercent}%</span>
                          </div>
                          <div className="mt-1 h-1 rounded-full bg-[var(--bg-inset)] overflow-hidden">
                            <div className="h-full bg-[var(--color-brand)] rounded-full" style={{ width: `${remainingPercent}%` }} />
                          </div>
                        </div>
                      ) : (
                        <span className="w-28 shrink-0 text-[10px] text-[var(--text-dim)] hidden lg:block text-right">
                          {quotaIsUnsupported ? "额度不支持" : quotaSupported ? "额度待刷新" : "额度不支持"}
                        </span>
                      )}
                      <span className="text-[10px] text-[var(--text-dim)] whitespace-nowrap shrink-0 w-20 text-right hidden xl:block">{getTimeAgo(account.last_used_at)}</span>
                      <div className="shrink-0">
                        {routable ? <Badge variant={badge.v} dot>{badge.label}</Badge> : <Badge variant="warn" dot>待适配</Badge>}
                      </div>
                    </div>
                  </div>
                );
              }

              // Card view (default): Cockpit-style vertical resource card.
              return (
                <article
                  key={account.id}
                  className={`pg-panel min-w-0 min-h-[310px] overflow-hidden cursor-pointer transition-all duration-200 flex flex-col ${
                    active ? "ring-2 ring-[var(--color-brand)]" : "hover:border-[var(--border-strong)] hover:-translate-y-px"
                  }`}
                  onClick={onOpen}
                >
                  <div className="px-3.5 pt-3.5 pb-2.5 flex items-start gap-2.5">
                    <input type="checkbox" className="mt-0.5 accent-[var(--color-brand)] shrink-0" checked={selected.has(account.id)} onClick={(event) => event.stopPropagation()} onChange={() => toggle(account.id)} />
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2 min-w-0">
                        <h3 className="text-[13px] font-semibold text-[var(--text-primary)] truncate flex-1">{account.name || source}</h3>
                        {routable ? <Badge variant={badge.v} dot>{badge.label}</Badge> : <Badge variant="warn" dot>待适配</Badge>}
                      </div>
                      <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                        <Badge variant="brand">{categoryLabels[resource.category]}</Badge>
                        <Badge variant="mute">{connector?.protocol || "协议待发现"}</Badge>
                        {account.plan_type && <Badge variant="ok">{account.plan_type}</Badge>}
                      </div>
                    </div>
                  </div>

                  <div className="px-3.5 pb-2 space-y-2 text-[10.5px] text-[var(--text-secondary)]">
                    <div className="flex gap-2 min-w-0"><span className="text-[var(--text-dim)] shrink-0">来源</span><span className="truncate">{source}</span></div>
                    <div className="flex gap-2 min-w-0"><span className="text-[var(--text-dim)] shrink-0">协议</span><span className="truncate pg-mono">{parseAccountProtocols(account.protocols).join(" · ") || connector?.protocol || "待发现"}</span></div>
                    {connector?.base_url && <div className="flex gap-2 min-w-0"><span className="text-[var(--text-dim)] shrink-0">地址</span><span className="truncate pg-mono">{connector.base_url}</span></div>}
                  </div>

                  <div className="px-3.5 pb-2.5 min-h-[30px]">
                    <div className="flex flex-wrap gap-1">
                      {(models.length ? models.slice(0, 3) : ["模型待发现"]).map((model) => <Badge key={model} variant="brand">{model}</Badge>)}
                      {models.length > 3 && <Badge variant="mute">+{models.length - 3}</Badge>}
                    </div>
                  </div>

                  <div className="px-3.5 py-2.5 border-t mt-auto" style={{ borderColor: "var(--border-subtle)" }}>
                    {primaryQuota || account.quota_limit ? (
                      <div>
                        <div className="flex items-center justify-between text-[10px]">
                          <span className="inline-flex items-center gap-1 text-[var(--text-secondary)]"><CircleGauge size={12} />{primaryQuota?.label || "资源额度"}</span>
                          <span className="pg-mono font-semibold text-[var(--text-primary)]">{remainingPercent}%</span>
                        </div>
                        <div className="mt-1.5 h-1.5 rounded-full bg-[var(--bg-inset)] overflow-hidden">
                          <div className={`h-full rounded-full ${remainingPercent <= 20 ? "bg-[var(--color-err)]" : remainingPercent <= 50 ? "bg-[var(--color-warn)]" : "bg-[var(--color-brand)]"}`} style={{ width: `${remainingPercent}%` }} />
                        </div>
                        {primaryQuota && <div className="mt-1 text-right text-[9px] text-[var(--text-dim)]">{resetLabel(primaryQuota)}</div>}
                      </div>
                    ) : (
                      <div className="h-9 flex items-center justify-center text-[10px] text-[var(--text-dim)]">
                        <CircleGauge size={12} className="mr-1.5" />
                        {quotaIsUnsupported ? "资源额度不支持" : quotaSupported ? "等待查询资源额度" : "资源额度不支持"}
                      </div>
                    )}
                  </div>

                  <div className="px-3.5 py-2 border-t flex items-center justify-between gap-2" style={{ borderColor: "var(--border-subtle)", background: "var(--bg-elevated)" }}>
                    <span className="text-[9.5px] text-[var(--text-dim)] truncate">{account.last_used_at ? `最近使用 ${getTimeAgo(account.last_used_at)}` : `创建于 ${getTimeAgo(account.created_at)}`}</span>
                    <div className="flex items-center gap-1 shrink-0">
                      <button type="button" className="pg-card-action" title={getTestResultTooltip(account.id)} disabled={testingAccountId === account.id} onClick={(event) => { event.stopPropagation(); void handleTestAccount(account); }}>
                        {testingAccountId === account.id ? <Spinner className="h-3.5 w-3.5" /> : getTestResultIcon(account.id) || <Zap size={14} />}
                      </button>
                      <button type="button" className="pg-card-action" title="编辑资源" onClick={(event) => { event.stopPropagation(); setDraft({ ...account }); setDraftBaseUrl(connector?.base_url ?? null); setExpanded(account.id); setEditing(true); }}><Pencil size={14} /></button>
                      <button type="button" className="pg-card-action" title="刷新 Token" disabled={refreshToken.isPending} onClick={(event) => { event.stopPropagation(); void refreshToken.mutateAsync(account.id); }}>
                        {refreshToken.isPending ? <Spinner className="h-3.5 w-3.5" /> : <Sparkles size={14} />}
                      </button>
                      <button type="button" className="pg-card-action" title="导出当前资源（凭证脱敏）" onClick={(event) => { event.stopPropagation(); void handleExportAccount(account); }}><Download size={14} /></button>
                      <button type="button" className="pg-card-action pg-card-action-danger" title="删除当前资源" disabled={deleteAccount.isPending} onClick={(event) => { event.stopPropagation(); handleDeleteAccount(account); }}><Trash2 size={14} /></button>
                    </div>
                  </div>
                </article>
              );
            })}
          </div>
        )}

        {activeResource && (() => {
          const { account, connector, category, models, source, routable } = activeResource;
          const badge = effectiveStatusBadge(account);
          const quotaWindows = parseQuotaWindows(account);
          const quotaSupported = supportsQuota(account, connector);
          const quotaIsUnsupported = quotaUnsupported(account, connector);
          const sharedConnectorCount = connector
            ? resources.filter((resource) => resource.connector?.id === connector.id).length - 1
            : 0;
          return (
            <aside className="pg-panel p-0 overflow-hidden animate-slide-in-right fixed right-4 top-[70px] bottom-4 z-50 flex flex-col" style={{ width: 420 }}>
              <div className="px-4 py-3.5 border-b" style={{ borderColor: "var(--border-subtle)" }}>
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="pg-eyebrow">Model resource inspector</div>
                    <div className="mt-1.5 flex items-center gap-2">
                      <StatusDot status={account.health_status === "healthy" ? "ok" : account.health_status === "error" ? "err" : "mute"} pulse={account.health_status === "healthy"} />
                      <h3 className="text-sm font-semibold text-[var(--text-primary)] truncate">{account.name || source}</h3>
                      {(account.status === "error" || account.status === "exhausted" || account.status === "token_expired" || account.health_status === "error") && (
                        <Badge variant="err" dot>异常</Badge>
                      )}
                    </div>
                    <div className="mt-1 text-[10px] text-[var(--text-dim)]">{source} · {categoryLabels[category]}</div>
                  </div>
                  <button onClick={() => setExpanded(null)} className="w-6 h-6 rounded-md hover:bg-[var(--bg-hover)] text-[var(--text-dim)]">×</button>
                </div>
              </div>

              <div className="flex-1 min-h-0 overflow-y-auto p-3.5 space-y-3 text-[11px]">
                <div className="grid grid-cols-2 gap-2">
                  <div className="pg-panel-inset p-2.5"><div className="pg-eyebrow">资源状态</div><div className="mt-1.5"><Badge variant={routable ? badge.v : "warn"} dot>{routable ? badge.label : "等待适配器"}</Badge></div></div>
                  <div className="pg-panel-inset p-2.5"><div className="pg-eyebrow">健康延迟</div><div className="mt-1 pg-mono text-sm font-semibold text-[var(--text-primary)]">{account.health_latency ? `${account.health_latency} ms` : "--"}</div></div>
                </div>

                {account.health_msg && (
                  <div className="rounded-lg border px-3 py-2.5 leading-5" style={{ borderColor: account.health_status === "error" ? "var(--color-err)" : "var(--color-warn)", background: account.health_status === "error" ? "var(--color-err-bg)" : "var(--color-warn-bg)", color: "var(--text-secondary)" }}>
                    {account.health_msg}
                  </div>
                )}

                {!routable && (
                  <div className="rounded-lg border px-3 py-2.5 leading-5" style={{ borderColor: "var(--color-warn)", background: "var(--color-warn-bg)", color: "var(--text-secondary)" }}>
                    资源已安全保存，但当前连接器尚未完成专用协议适配，因此不会进入普通 API 路由。
                  </div>
                )}

                <div>
                  <div className="flex items-center justify-between"><span className="pg-eyebrow">模型能力</span><span className="text-[10px] text-[var(--text-dim)]">{models.length || 0} 个模型</span></div>
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    {(models.length ? models : [connector?.protocol || "等待模型发现"]).map((model) => <Badge key={model} variant="brand">{model}</Badge>)}
                  </div>
                </div>

                <div>
                  <div className="flex items-center justify-between">
                    <span className="pg-eyebrow">Quota usage</span>
                    <span className="pg-mono text-[10px] text-[var(--text-dim)]">{account.plan_type || "计划待识别"}</span>
                  </div>
                  {quotaWindows.length ? (
                    <div className="mt-2 space-y-2.5">
                      {quotaWindows.map((window) => (
                        <div key={window.key}>
                          <div className="flex justify-between gap-2 text-[10px] text-[var(--text-secondary)]">
                            <span>{window.label}</span>
                            <span>{Math.round(window.remaining_percent)}% 剩余 · {resetLabel(window)}</span>
                          </div>
                          <div className="mt-1 h-1.5 rounded-full overflow-hidden bg-[var(--bg-elevated)]">
                            <div className="h-full rounded-full bg-[var(--color-brand)]" style={{ width: `${window.remaining_percent}%` }} />
                          </div>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <div className="mt-2 rounded-md border px-2.5 py-2 text-[10px] leading-4 text-[var(--text-dim)]" style={{ borderColor: "var(--border-subtle)", background: "var(--bg-inset)" }}>
                      {quotaIsUnsupported
                        ? "该资源暂不支持在线额度查询"
                        : quotaSupported
                          ? "尚未获取在线额度，可点击下方刷新"
                          : "该资源暂不支持在线额度查询"}
                    </div>
                  )}
                  {account.quota_error && !quotaIsUnsupported && <div className="mt-2 text-[10px] leading-4 text-[var(--color-err)]">{account.quota_error}</div>}
                </div>

                {editing ? (
                  <div className="space-y-2.5 pt-1">
                    <div className="flex items-center justify-between gap-3">
                      <span className="text-[var(--text-dim)] shrink-0">名称</span>
                      <input
                        className="flex-1 rounded-md border px-2 py-1 text-[11px]"
                        style={{ borderColor: "var(--border-subtle)" }}
                        value={draft?.name ?? account.name ?? ""}
                        onChange={(e) => setDraft({ ...(draft ?? ({} as Account)), name: e.target.value } as Account)}
                      />
                    </div>
                    <div className="flex items-center justify-between gap-3">
                      <span className="text-[var(--text-dim)] shrink-0">API Key</span>
                      <div className="flex-1 flex items-center gap-1">
                        <input
                          className="flex-1 rounded-md border px-2 py-1 text-[11px] font-mono"
                          style={{ borderColor: "var(--border-subtle)" }}
                          type={showApiKey ? "text" : "password"}
                          value={draft?.api_key ?? account.api_key ?? ""}
                          onChange={(e) => setDraft({ ...(draft ?? ({} as Account)), api_key: e.target.value } as Account)}
                          placeholder="sk-..."
                        />
                        <button
                          type="button"
                          onClick={() => setShowApiKey(!showApiKey)}
                          className="p-1 rounded hover:bg-[var(--bg-hover)] text-[var(--text-dim)]"
                          title={showApiKey ? "隐藏" : "显示"}
                        >
                          {showApiKey ? <EyeOff size={14} /> : <Eye size={14} />}
                        </button>
                      </div>
                    </div>
                    {(connector?.base_url || draftBaseUrl !== null) && (
                      <div className="flex items-center justify-between gap-3">
                        <span className="text-[var(--text-dim)] shrink-0">Base URL</span>
                        <input
                          className="flex-1 rounded-md border px-2 py-1 text-[11px] font-mono"
                          style={{ borderColor: "var(--border-subtle)" }}
                          value={draftBaseUrl ?? connector?.base_url ?? ""}
                          onChange={(e) => setDraftBaseUrl(e.target.value)}
                          placeholder="https://api.example.com"
                        />
                      </div>
                    )}
                    {sharedConnectorCount > 0 && (
                      <div className="pl-[58px] text-[9.5px] leading-4 text-[var(--color-warn)]">
                        该连接器共被 {sharedConnectorCount + 1} 个资源共用，修改 Base URL 将自动拆分为独立连接器，不影响其他资源
                      </div>
                    )}
                    <div className="flex items-center justify-between gap-3">
                      <span className="text-[var(--text-dim)] shrink-0">模型 (逗号分隔)</span>
                      <input
                        className="flex-1 rounded-md border px-2 py-1 text-[11px]"
                        style={{ borderColor: "var(--border-subtle)" }}
                        value={(() => {
                          const raw = draft?.models ?? account.models ?? "";
                          // 显示为逗号分隔，而非原始 JSON 数组
                          try {
                            const parsed = JSON.parse(raw);
                            if (Array.isArray(parsed)) return parsed.join(", ");
                          } catch {}
                          return raw;
                        })()}
                        onChange={(e) => setDraft({ ...(draft ?? ({} as Account)), models: e.target.value } as Account)}
                      />
                    </div>
                    <div className="flex items-center justify-between gap-3">
                      <span className="text-[var(--text-dim)] shrink-0">优先级</span>
                      <input
                        type="number"
                        className="flex-1 rounded-md border px-2 py-1 text-[11px]"
                        style={{ borderColor: "var(--border-subtle)" }}
                        value={draft?.priority ?? account.priority ?? 1}
                        onChange={(e) => setDraft({ ...(draft ?? ({} as Account)), priority: Number(e.target.value) } as Account)}
                      />
                    </div>
                    <div>
                      <span className="text-[var(--text-dim)]">API 协议（多选）</span>
                      <div className="flex flex-wrap gap-1.5 mt-1">
                      {protocolOptions.map((opt) => {
                        const current = parseAccountProtocols(draft?.protocols ?? account.protocols);
                        const active = current.includes(opt.value);
                        return (
                          <button
                            key={opt.value}
                            type="button"
                            onClick={() => {
                              const base = draft ?? ({} as Account);
                              const next = current.includes(opt.value)
                                ? current.filter((p) => p !== opt.value)
                                : [...current, opt.value];
                              setDraft({ ...base, protocols: JSON.stringify(next) } as Account);
                            }}
                              className="px-2 py-1 rounded-md border text-[11px]"
                              style={{
                                borderColor: "var(--border-subtle)",
                                background: active ? "var(--color-brand-soft)" : "transparent",
                                color: active ? "var(--color-brand)" : "var(--text-secondary)",
                              }}
                            >
                              {opt.label}
                            </button>
                        );
                      })}
                      </div>
                    </div>
                    <div className="flex items-center justify-between gap-3">
                      <span className="text-[var(--text-dim)] shrink-0">路由接管</span>
                      <button
                        type="button"
                        onClick={() => setDraft({ ...(draft ?? ({} as Account)), route_takeover: (draft?.route_takeover ?? account.route_takeover ?? 1) === 1 ? 0 : 1 } as Account)}
                        className="px-2 py-1 rounded-md border text-[11px]"
                        style={{ borderColor: "var(--border-subtle)", color: "var(--text-secondary)" }}
                      >
                        {(draft?.route_takeover ?? account.route_takeover ?? 1) === 1 ? "开启（由 /v1/responses 统一入口调度）" : "关闭"}
                      </button>
                    </div>
                    <div className="flex gap-2 pt-1">
                      <Button size="sm" variant="primary" className="flex-1" loading={updateAccount.isPending} onClick={handleSave}>保存修改</Button>
                      <Button size="sm" variant="outline" className="flex-1" disabled={testingAccountId === account.id} onClick={async () => {
                        // Save first, then test
                        await handleSave();
                        void handleTestAccount(account);
                      }}>
                        {testingAccountId === account.id ? <Spinner size={12} className="mr-1" /> : <Zap size={12} className="mr-1" />}
                        保存并测试
                      </Button>
                    </div>
                  </div>
                ) : (
                  <div className="space-y-2.5 pt-1">
                    {[
                      ["资源类型", categoryLabels[category]],
                      ["凭证方式", credentialLabels[account.credential_type || "api_key"] || account.credential_type || "API Key"],
                      ["导入来源", sourceLabels[account.source_format || ""] || account.source_format || "手动接入"],
                      ["外部账号", account.external_account_id || account.email || "--"],
                      ["订阅计划", account.plan_type || "--"],
                      ["Token 到期", account.expires_at ? new Date(account.expires_at).toLocaleString("zh-CN") : "--"],
                      ["Token 刷新", getTimeAgo(account.token_refreshed_at)],
                      ["额度刷新", getTimeAgo(account.quota_refreshed_at)],
                      ["路由能力", routable ? "可直接加入路由池" : "等待连接器适配"],
                      ["优先级", String(account.priority || 1)],
                      ["最近使用", getTimeAgo(account.last_used_at)],
                    ].map(([label, value]) => (
                      <div key={label} className="flex items-start justify-between gap-3"><span className="text-[var(--text-dim)] shrink-0">{label}</span><span className="text-[var(--text-primary)] text-right break-all">{value}</span></div>
                    ))}
                  </div>
                )}

                <button onClick={() => setShowConnector((value) => !value)} className="w-full flex items-center justify-between rounded-lg border px-3 py-2 text-[11px] text-[var(--text-secondary)] hover:bg-[var(--bg-hover)]" style={{ borderColor: "var(--border-subtle)" }}>
                  <span className="inline-flex items-center gap-1.5"><CloudCog size={12} /> 上游连接器（高级）</span>
                  <ChevronDown size={13} className={`transition-transform ${showConnector ? "rotate-180" : ""}`} />
                </button>
                {showConnector && (
                  <div className="pg-panel-inset p-3 space-y-2.5 text-[10px]">
                    <div className="space-y-1">
                      <div className="text-[var(--text-dim)]">连接器</div>
                      <div className="text-[var(--text-primary)] break-words">{connector?.name || "系统自动识别"}</div>
                    </div>
                    <div className="space-y-1">
                      <div className="text-[var(--text-dim)]">协议</div>
                      <div className="pg-mono text-[var(--text-primary)] break-words">{connector?.protocol || "--"}</div>
                    </div>
                    <div className="space-y-1">
                      <div className="text-[var(--text-dim)]">Base URL</div>
                      <div className="pg-mono text-[var(--text-primary)] break-all">{connector?.base_url || "--"}</div>
                    </div>
                    <div className="text-[var(--text-dim)] leading-4 pt-1 border-t" style={{ borderColor: "var(--border-subtle)" }}>连接器由 PoolGate 自动创建和复用，普通使用无需维护。</div>
                  </div>
                )}

              </div>

              {/* Bottom action bar */}
              <div className="border-t px-3 py-2 flex items-center justify-center gap-2" style={{ borderColor: "var(--border-subtle)", background: "var(--bg-elevated)" }}>
                <button
                  className="flex items-center justify-center w-8 h-8 rounded-md text-[var(--color-brand)] hover:bg-[var(--color-brand)]/10 transition-colors"
                  title="编辑"
                  onClick={() => { setDraft({ ...account }); setDraftBaseUrl(connector?.base_url ?? null); setEditing(true); }}
                ><Pencil size={15} /></button>
                <button
                  className="flex items-center justify-center w-8 h-8 rounded-md text-[var(--color-ok)] hover:bg-[var(--color-ok)]/10 transition-colors disabled:opacity-40"
                  title={getTestResultTooltip(account.id)}
                  disabled={testingAccountId === account.id}
                  onClick={() => void handleTestAccount(account)}
                >{testingAccountId === account.id ? <Spinner size={15} /> : <Zap size={15} />}</button>
                <button
                  className="flex items-center justify-center w-8 h-8 rounded-md text-[var(--color-warn)] hover:bg-[var(--color-warn)]/10 transition-colors"
                  title="刷新状态与模型"
                  disabled={refreshingAccountIds.has(account.id)}
                  onClick={() => void handleCardRefresh(account, (quotaSupported ?? false) && !quotaIsUnsupported)}
                >{refreshingAccountIds.has(account.id) ? <Spinner size={15} /> : <RefreshCw size={15} />}</button>
                <button
                  className="flex items-center justify-center w-8 h-8 rounded-md text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-40"
                  title="刷新 Token"
                  disabled={refreshToken.isPending || (!account.credential_type?.includes("oauth") && account.credential_type !== "token")}
                  onClick={async () => {
                    const result = await refreshToken.mutateAsync(account.id);
                    toast(result.success ? "success" : "warning", result.message);
                  }}
                >{refreshToken.isPending ? <Spinner size={15} /> : <Sparkles size={15} />}</button>
                <button
                  className="flex items-center justify-center w-8 h-8 rounded-md text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-40"
                  title={quotaIsUnsupported ? "该资源尚无在线额度适配器" : "刷新额度"}
                  disabled={refreshQuota.isPending || quotaIsUnsupported}
                  onClick={async () => {
                    const result = await refreshQuota.mutateAsync(account.id);
                    toast(result.success ? "success" : "warning", result.message);
                  }}
                >{refreshQuota.isPending ? <Spinner size={15} /> : <CircleGauge size={15} />}</button>
                <button
                  className="flex items-center justify-center w-8 h-8 rounded-md text-[var(--text-dim)] hover:bg-[var(--bg-hover)] transition-colors"
                  title="导出"
                  onClick={() => void handleExportAccount(account)}
                ><Download size={15} /></button>
              </div>
            </aside>
          );
        })()}
      </div>

      {activeResource && (
        <div
          role="presentation"
          className="fixed inset-0 z-40 bg-black/10 backdrop-blur-[1px]"
          onMouseDown={() => setExpanded(null)}
        />
      )}

      {totalPages > 1 && (
        <div className="flex items-center justify-center gap-2">
          <Button size="sm" variant="outline" disabled={page <= 1} onClick={() => setPage((value) => value - 1)}><ChevronLeft size={14} /> 上一页</Button>
          <span className="text-sm px-3 text-[var(--text-dim)]">第 {page}/{totalPages} 页</span>
          <Button size="sm" variant="outline" disabled={page >= totalPages} onClick={() => setPage((value) => value + 1)}>下一页 <ChevronRight size={14} /></Button>
        </div>
      )}

      {selected.size > 0 && (
        <div className="fixed bottom-8 left-1/2 -translate-x-1/2 z-40 animate-batch-bar">
          <div
            className="flex items-center gap-1 rounded-xl border pl-2 pr-1.5 py-1.5"
            style={{
              background: "var(--bg-surface-solid)",
              borderColor: "var(--border-strong)",
              boxShadow: "var(--shadow-elevated)",
            }}
          >
            <div className="flex items-center gap-2 pl-2 pr-1">
              <span className="flex items-center justify-center min-w-[22px] h-[22px] px-1.5 rounded-md text-[11px] font-semibold pg-mono" style={{ background: "var(--color-brand)", color: "#fff" }}>{selected.size}</span>
              <span className="text-[12px] font-medium text-[var(--text-primary)] whitespace-nowrap">已选资源</span>
            </div>
            <div className="w-px h-5 mx-1 bg-[var(--border-default)]" />

            {/* Group: checks / refresh */}
            <div className="flex items-center gap-0.5">
              <button type="button" onClick={handleBatchHealth} disabled={batchHealth.isPending} className="pg-batch-btn">
                {batchHealth.isPending ? <Spinner className="h-3.5 w-3.5" /> : <Zap size={13} />} 测活
              </button>
              <button type="button" onClick={() => handleBatchRefresh("token")} disabled={batchRefreshTokens.isPending} className="pg-batch-btn">
                {batchRefreshTokens.isPending ? <Spinner className="h-3.5 w-3.5" /> : <RefreshCw size={13} />} Token
              </button>
              <button type="button" onClick={() => handleBatchRefresh("quota")} disabled={batchRefreshQuotas.isPending} className="pg-batch-btn">
                {batchRefreshQuotas.isPending ? <Spinner className="h-3.5 w-3.5" /> : <CircleGauge size={13} />} 额度
              </button>
            </div>
            <div className="w-px h-5 mx-1 bg-[var(--border-default)]" />

            {/* Group: enable / disable */}
            <div className="flex items-center gap-0.5">
              <button type="button" onClick={() => handleBatchStatus("active")} className="pg-batch-btn" style={{ color: "var(--color-ok)" }}>
                <Power size={13} /> 启用
              </button>
              <button type="button" onClick={() => handleBatchStatus("disabled")} className="pg-batch-btn" style={{ color: "var(--color-warn)" }}>
                <PowerOff size={13} /> 停用
              </button>
            </div>
            <div className="w-px h-5 mx-1 bg-[var(--border-default)]" />

            <button type="button" onClick={() => setSelected(new Set())} className="pg-batch-btn" title="取消选择">
              <X size={13} /> 取消
            </button>
          </div>
        </div>
      )}

      <ConfirmDialog
        open={!!deletingAccount}
        onClose={() => { if (!deleteAccount.isPending) setDeletingAccount(null); }}
        onConfirm={() => void confirmDeleteAccount()}
        title="删除模型供应商"
        message={`确认删除模型供应商「${deletingAccount?.name || deletingAccount?.id || ""}」？此操作将同时移除该供应商在所有路由池中的关联，且无法撤销。`}
        confirmText="永久删除"
        cancelText="取消"
        variant="danger"
        loading={deleteAccount.isPending}
      />

      <ConfirmDialog
        open={batchDeleteConfirmOpen}
        onClose={() => { if (!batchDelete.isPending) setBatchDeleteConfirmOpen(false); }}
        onConfirm={() => void confirmDeleteUnavailable()}
        title="删除不可用模型供应商"
        message={`确认删除全部 ${unavailableResources.length} 个不可用模型供应商？凭据及路由池关联将一并移除，此操作无法撤销。`}
        confirmText="永久删除"
        cancelText="取消"
        variant="danger"
        loading={batchDelete.isPending}
      />

      {showImport && (
        <ImportFullScreen onClose={() => setShowImport(false)} />
      )}
    </div>
  );
}

/**
 * Full-screen page for the model-resource import flow. Replaces the previous
 * modal dialog so the multi-step content (provider grid → credential form →
 * check results) has the full viewport to breathe. Rendered as a solid
 * `fixed inset-0` overlay with its own toolbar + back navigation, matching the
 * app shell's toolbar styling.
 */
function ImportFullScreen({ onClose }: { onClose: () => void }) {
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-[var(--bg-canvas)] animate-fade-in">
      {/* Top navigation bar — mirrors the app toolbar for visual consistency */}
      <header
        className="pg-toolbar shrink-0 h-[54px] flex items-center gap-3 px-4 border-b"
        style={{ borderColor: "var(--border-default)" }}
        data-tauri-drag-region
      >
        <button
          onClick={onClose}
          className="h-8 flex items-center gap-1.5 pl-1.5 pr-2.5 rounded-[7px] text-[12px] font-medium text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] transition-colors"
          title="返回模型供应商（Esc）"
        >
          <ChevronLeft size={16} /> 返回
        </button>
        <div className="w-px h-4 bg-[var(--border-default)]" />
        <span
          className="w-7 h-7 rounded-lg bg-[var(--color-brand)] text-white flex items-center justify-center shadow-sm shrink-0"
        >
          <Plus size={15} strokeWidth={2.2} />
        </span>
        <div className="min-w-0">
          <h1 className="text-[13px] leading-4 font-semibold tracking-[-0.01em] text-[var(--text-primary)]">接入模型供应商</h1>
          <p className="text-[9px] leading-3 text-[var(--text-dim)] truncate">选择厂商、填写凭证并检测后加入本地网关</p>
        </div>
        <button
          onClick={onClose}
          className="ml-auto w-8 h-8 flex items-center justify-center rounded-md text-[var(--text-dim)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)] transition-colors"
          title="关闭（Esc）"
        >
          <X size={16} />
        </button>
      </header>

      {/* Content — ImportCenter fills the remaining space via h-full */}
      <main className="flex-1 min-h-0 overflow-hidden">
        <ImportCenter onClose={onClose} />
      </main>
    </div>
  );
}
