import React, { useState, useMemo, useCallback } from "react";
import {
  useProviders,
  useCreateProvider,
  useUpdateProvider,
  useDeleteProvider,
  useTestProviderConnection,
  useAccounts,
  useCreateAccount,
  useUpdateAccount,
  useDeleteAccount,
} from "@/hooks/use-tauri";
import { getProviderApiKeys } from "@/lib/tauri-commands";
import type { Provider, ProviderTestResult, Account } from "@/lib/tauri-commands";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { Modal } from "@/components/ui/Modal";
import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import { Spinner, PageSpinner } from "@/components/ui/Spinner";
import {
  Search, Plus, Edit3, Trash2, Power, PowerOff, Globe, Zap,
  CheckCircle, XCircle, Braces, List, KeyRound, Eye, EyeOff,
  FileJson, Scan, ArrowRight, Sparkles, Shield, Clock, Layers,
  ExternalLink, ChevronDown, ChevronRight, Copy, Terminal, Settings2,
  Download, Upload, Wand2, Star, Server, Wifi, WifiOff, AlertCircle,
  CheckCircle2, RefreshCw, Filter, Grid3X3, LayoutList,
} from "lucide-react";
import {
  getProviderBrand as getBrand,
  type ProviderBrand,
} from "@/lib/provider-brand";
import {
  modelResourceTemplates,
  providerGroupLabels,
  type ModelResourceTemplate,
  type ProviderGroup,
  type AuthMethod,
  type Protocol,
} from "@/lib/model-resource-templates";

// ─── Brand tile component ────────────────────────────────────────────────────
function BrandTile({ id, name, size = 36 }: { id: string; name: string; size?: number }) {
  const brand = getBrand(id, name);
  return (
    <div
      className="flex items-center justify-center rounded-lg font-bold shrink-0 select-none"
      style={{
        width: size,
        height: size,
        backgroundColor: brand.color,
        color: brand.fg || "#fff",
        fontSize: size * 0.32,
        letterSpacing: "-0.02em",
      }}
    >
      {brand.mono}
    </div>
  );
}

// ─── Types ────────────────────────────────────────────────────────────────────
const typeBadge: Record<string, "ok" | "warn" | "info"> = {
  official: "ok",
  relay: "warn",
  local: "info",
};

const typeLabel: Record<string, string> = {
  official: "官方",
  relay: "中转",
  local: "本地",
};

const protocolOptions = [
  { value: "openai", label: "OpenAI" },
  { value: "anthropic", label: "Anthropic" },
  { value: "google", label: "Google" },
  { value: "aws", label: "AWS Bedrock" },
  { value: "azure", label: "Azure OpenAI" },
  { value: "custom", label: "自定义" },
];

const typeOptions = [
  { value: "official", label: "官方" },
  { value: "relay", label: "中转" },
  { value: "local", label: "本地" },
];

/**
 * SQLite stores provider timestamps as `YYYY-MM-DD HH:mm:ss`; newer clients
 * may return an ISO timestamp with either `Z` or an explicit timezone offset.
 */
function providerCreatedTimestamp(value?: string): number {
  const raw = value?.trim();
  if (!raw) return 0;

  // SQLite CURRENT_TIMESTAMP is UTC in `YYYY-MM-DD HH:mm:ss` form. Treat
  // timezone-less ISO values as UTC as well, while preserving explicit offsets.
  const normalized = raw.includes("T") ? raw : raw.replace(" ", "T");
  const hasTimezone = /(?:Z|[+-]\d{2}:?\d{2})$/i.test(normalized);
  const timestamp = Date.parse(hasTimezone ? normalized : `${normalized}Z`);
  return Number.isFinite(timestamp) ? timestamp : 0;
}

/**
 * Provider display order: enabled providers first, then newest first. Disabled
 * providers are the UI's unavailable state and always stay at the bottom.
 */
function compareProvidersForDisplay(left: Provider, right: Provider): number {
  const leftUnavailable = left.enabled === false ? 1 : 0;
  const rightUnavailable = right.enabled === false ? 1 : 0;
  if (leftUnavailable !== rightUnavailable) return leftUnavailable - rightUnavailable;

  const byCreatedAt = providerCreatedTimestamp(right.created_at) - providerCreatedTimestamp(left.created_at);
  if (byCreatedAt !== 0) return byCreatedAt;

  // Keep ordering deterministic for legacy rows without created_at.
  return right.id.localeCompare(left.id);
}

interface ProviderForm {
  name: string;
  type: string;
  base_url: string;
  protocol: string;
  api_keys: string;
  proxy_url: string;
  custom_headers: string;
  timeout_ms: number;
  priority: number;
}

interface HeaderEntry {
  id: string;
  name: string;
  value: string;
}

const blockedHeaderNames = new Set([
  "authorization", "proxy-authorization", "host",
  "content-length", "transfer-encoding", "connection",
]);

let headerEntrySequence = 0;

const createHeaderEntry = (name = "", value = ""): HeaderEntry => ({
  id: `header-${headerEntrySequence++}`,
  name,
  value,
});

const parseHeaderEntries = (raw: string): HeaderEntry[] => {
  if (!raw.trim()) return [];
  const parsed = JSON.parse(raw);
  if (!parsed || Array.isArray(parsed) || typeof parsed !== "object") {
    throw new Error("自定义请求头必须是 JSON 对象");
  }
  return Object.entries(parsed).map(([name, value]) => {
    if (typeof value !== "string") {
      throw new Error(`请求头 ${name} 的值必须是字符串`);
    }
    return createHeaderEntry(name, value);
  });
};

const emptyForm: ProviderForm = {
  name: "",
  type: "official",
  base_url: "",
  protocol: "openai",
  api_keys: "",
  proxy_url: "",
  custom_headers: "",
  timeout_ms: 30000,
  priority: 0,
};

const GENERIC_PROVIDER_NAMES = new Set([
  "custom", "自定义", "自定义供应商", "provider", "relay", "中转", "其他", "other", "unknown",
  "openai", "codex", "chatgpt", "anthropic", "claude", "google", "gemini", "antigravity", "xai", "grok",
]);

const isGenericProviderName = (name: string): boolean => {
  const normalized = name.trim().toLowerCase().replace(/[\s\-_]/g, "");
  return !normalized || GENERIC_PROVIDER_NAMES.has(normalized) || GENERIC_PROVIDER_NAMES.has(name.trim());
};

function newProviderId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return `prov_${crypto.randomUUID().replace(/-/g, "")}`;
  }
  return `prov_${Date.now().toString(16)}${Math.random().toString(16).slice(2, 18)}`;
}

function newAccountId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return `acct_${crypto.randomUUID().replace(/-/g, "")}`;
  }
  return `acct_${Date.now().toString(16)}${Math.random().toString(16).slice(2, 18)}`;
}

function sameBaseUrl(a: string, b: string): boolean {
  const na = a.trim().trimEnd().replace(/\/+$/, "").toLowerCase();
  const nb = b.trim().trimEnd().replace(/\/+$/, "").toLowerCase();
  return na.length > 0 && na === nb;
}

function parseApiKeys(raw?: string | null): string[] {
  const s = (raw ?? "").trim();
  if (!s) return [];
  if (s.startsWith("[")) {
    try {
      const parsed = JSON.parse(s);
      if (Array.isArray(parsed)) {
        return parsed
          .map((v) => (typeof v === "string" ? v.trim() : ""))
          .filter((v) => v.length > 0);
      }
    } catch {
      // fall through
    }
  }
  return s
    .split(/[\n,]+/)
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
}

function stringifyApiKeys(keys: string[]): string {
  const clean = keys.map((k) => k.trim()).filter((k) => k.length > 0);
  return JSON.stringify(clean);
}

interface KeyEntry {
  id: string;
  name: string;
  value: string;
  reveal: boolean;
}

let keyEntrySequence = 0;
const createKeyEntry = (value = "", name = ""): KeyEntry => ({
  id: `key-${keyEntrySequence++}`,
  name,
  value,
  reveal: false,
});

// ─── Provider group icon mapping ─────────────────────────────────────────────
const groupIcons: Record<ProviderGroup, React.ReactNode> = {
  official: <Star size={16} />,
  cn_official: <Star size={16} />,
  aggregator: <Layers size={16} />,
  third_party: <Server size={16} />,
  cloud: <Globe size={16} />,
  custom: <Settings2 size={16} />,
};

// ─── Scan config detection patterns ──────────────────────────────────────────
interface ScanResult {
  source: string;
  path: string;
  provider: string;
  keyPreview: string;
  detected: boolean;
}

// ─── Main Component ──────────────────────────────────────────────────────────
export default function ProvidersPage() {
  const { data: providers, isLoading } = useProviders();
  const createProvider = useCreateProvider();
  const updateProvider = useUpdateProvider();
  const deleteProvider = useDeleteProvider();
  const testProviderConnection = useTestProviderConnection();
  const { data: accounts } = useAccounts();
  const createAccount = useCreateAccount();
  const updateAccount = useUpdateAccount();
  const deleteAccount = useDeleteAccount();

  // Page state
  const [view, setView] = useState<"grid" | "list">("grid");
  const [search, setSearch] = useState("");
  const [filterType, setFilterType] = useState("");
  const [filterProtocol, setFilterProtocol] = useState("");
  const [activeSection, setActiveSection] = useState<"overview" | "templates" | "scan">("overview");

  // Modal state
  const [modalOpen, setModalOpen] = useState(false);
  const [modalMode, setModalMode] = useState<"template" | "manual" | "scan">("manual");
  const [editing, setEditing] = useState<Provider | null>(null);
  const [form, setForm] = useState<ProviderForm>(emptyForm);
  const [saving, setSaving] = useState(false);
  const [testingProviderId, setTestingProviderId] = useState<string | null>(null);
  const [testResults, setTestResults] = useState<Record<string, ProviderTestResult>>({});
  const [deletingProvider, setDeletingProvider] = useState<Provider | null>(null);
  const [deleteLoading, setDeleteLoading] = useState(false);
  const [formError, setFormError] = useState("");
  const [headerEntries, setHeaderEntries] = useState<HeaderEntry[]>([]);
  const [headersJsonMode, setHeadersJsonMode] = useState(false);
  const [keyEntries, setKeyEntries] = useState<KeyEntry[]>([]);
  const [keysLoading, setKeysLoading] = useState(false);
  const [existingPrompt, setExistingPrompt] = useState<Provider | null>(null);

  // Template picker state
  const [selectedTemplate, setSelectedTemplate] = useState<ModelResourceTemplate | null>(null);
  const [templateStep, setTemplateStep] = useState<"browse" | "configure">("browse");
  const [templateKeyInput, setTemplateKeyInput] = useState("");
  const [templateKeyEntries, setTemplateKeyEntries] = useState<KeyEntry[]>([]);

  // Scan state
  const [scanning, setScanning] = useState(false);
  const [scanResults, setScanResults] = useState<ScanResult[]>([]);
  const [selectedScanResults, setSelectedScanResults] = useState<Set<number>>(new Set());

  // Group filter for template picker
  const [templateGroupFilter, setTemplateGroupFilter] = useState<ProviderGroup | "all">("all");
  const [templateSearch, setTemplateSearch] = useState("");

  // Stats
  const providerStats = useMemo(() => {
    const list = providers ?? [];
    return {
      total: list.length,
      enabled: list.filter((p) => p.enabled !== false).length,
      official: list.filter((p) => p.type === "official").length,
      relay: list.filter((p) => p.type === "relay").length,
    };
  }, [providers]);

  // ─── Template functions ──────────────────────────────────────────────────
  const filteredTemplates = useMemo(() => {
    const q = templateSearch.trim().toLowerCase();
    return modelResourceTemplates.filter((t) => {
      if (templateGroupFilter !== "all" && t.group !== templateGroupFilter) return false;
      if (q && !t.name.toLowerCase().includes(q) && !t.description.toLowerCase().includes(q) && !t.baseUrl.toLowerCase().includes(q)) {
        return false;
      }
      return true;
    });
  }, [templateGroupFilter, templateSearch]);

  const templateGroups = useMemo(() => {
    const groups = new Map<ProviderGroup, number>();
    modelResourceTemplates.forEach((t) => {
      groups.set(t.group, (groups.get(t.group) || 0) + 1);
    });
    return groups;
  }, []);

  // 注意：必须位于所有 hooks 之后，否则 isLoading 翻转时会触发
  // "Rendered more hooks than during the previous render"
  if (isLoading) return <PageSpinner />;

  const filtered = [...(providers ?? [])]
    .sort(compareProvidersForDisplay)
    .filter((p) => {
      if (search && !p.name.toLowerCase().includes(search.toLowerCase())) return false;
      if (filterType && p.type !== filterType) return false;
      if (filterProtocol && p.protocol !== filterProtocol) return false;
      return true;
    });

  const openTemplatePicker = () => {
    setModalMode("template");
    setTemplateStep("browse");
    setSelectedTemplate(null);
    setTemplateKeyEntries([]);
    setTemplateGroupFilter("all");
    setTemplateSearch("");
    setFormError("");
    setModalOpen(true);
  };

  const selectTemplate = (template: ModelResourceTemplate) => {
    setSelectedTemplate(template);
    setTemplateStep("configure");
    setTemplateKeyEntries([createKeyEntry()]);
    setFormError("");
  };

  const handleTemplateSave = async () => {
    if (!selectedTemplate) return;
    const keys = templateKeyEntries
      .map((e) => e.value.trim())
      .filter((v) => v.length > 0);
    if (keys.length === 0) {
      setFormError("请至少填写一个 API Key");
      return;
    }

    setSaving(true);
    try {
      const existing = providers?.find((p) => p.name === selectedTemplate.name || sameBaseUrl(p.base_url, selectedTemplate.baseUrl));
      if (existing) {
        // Update existing provider with new keys
        const existingKeys = parseApiKeys(existing.api_keys);
        const mergedKeys = [...new Set([...existingKeys, ...keys])];
        await updateProvider.mutateAsync({
          ...existing,
          api_keys: stringifyApiKeys(mergedKeys),
        } as any);
        // Sync accounts
        const entries = mergedKeys.map((v, i) => ({
          name: templateKeyEntries[i]?.name || `${selectedTemplate.name} #${i + 1}`,
          value: v,
        }));
        await syncKeyAccounts(existing, entries);
      } else {
        // Create new provider
        const payload = {
          id: newProviderId(),
          name: selectedTemplate.name,
          type: selectedTemplate.group === "official" || selectedTemplate.group === "cn_official" ? "official" : "relay",
          base_url: selectedTemplate.baseUrl,
          protocol: selectedTemplate.protocols[0] === "anthropic" ? "anthropic" : "openai",
          api_keys: stringifyApiKeys(keys),
          proxy_url: "",
          custom_headers: "",
          timeout_ms: 30000,
          priority: 0,
        };
        const created = await createProvider.mutateAsync(payload as any);
        const entries = templateKeyEntries.map((e, i) => ({
          name: e.name.trim() || `${selectedTemplate.name} #${i + 1}`,
          value: e.value.trim(),
        }));
        await syncKeyAccounts(created as Provider, entries);
      }
      setModalOpen(false);
      setSelectedTemplate(null);
      setTemplateStep("browse");
    } catch (err) {
      console.error("Failed to save template provider", err);
      setFormError("保存失败，请重试");
    } finally {
      setSaving(false);
    }
  };

  // ─── Scan functions ──────────────────────────────────────────────────────
  const startScan = async () => {
    setScanning(true);
    setScanResults([]);
    setSelectedScanResults(new Set());

    // Simulate scanning common config locations
    await new Promise((resolve) => setTimeout(resolve, 1500));

    const results: ScanResult[] = [
      { source: "环境变量", path: "ANTHROPIC_API_KEY", provider: "Anthropic", keyPreview: "sk-ant-***...***", detected: true },
      { source: "环境变量", path: "OPENAI_API_KEY", provider: "OpenAI", keyPreview: "sk-***...***", detected: true },
      { source: "环境变量", path: "GEMINI_API_KEY", provider: "Google Gemini", keyPreview: "AI***...***", detected: false },
      { source: "配置文件", path: "~/.claude/credentials", provider: "Claude", keyPreview: "***...***", detected: true },
      { source: "配置文件", path: "~/.config/openai/auth.json", provider: "OpenAI Codex", keyPreview: "***...***", detected: true },
      { source: "环境变量", path: "DEEPSEEK_API_KEY", provider: "DeepSeek", keyPreview: "sk-***...***", detected: false },
    ];

    setScanResults(results.filter((r) => r.detected));
    setScanning(false);
  };

  const importScanResults = async () => {
    // This would import selected scan results
    setModalOpen(false);
  };

  // ─── Manual form functions ───────────────────────────────────────────────
  const resetForm = () => {
    setForm(emptyForm);
    setFormError("");
    setHeaderEntries([]);
    setHeadersJsonMode(false);
    setKeyEntries([]);
    setKeysLoading(false);
    setExistingPrompt(null);
    setEditing(null);
  };

  const openCreate = () => {
    resetForm();
    setModalMode("manual");
    setModalOpen(true);
  };

  const openEdit = (p: Provider) => {
    let parsedHeaders: HeaderEntry[] = [];
    let useJsonMode = false;
    try {
      parsedHeaders = parseHeaderEntries(p.custom_headers || "");
    } catch {
      useJsonMode = true;
    }
    const providerKeys = parseApiKeys(p.api_keys);
    const initialEntries = buildKeyEntriesFromProvider(p);
    setKeyEntries(initialEntries);
    setForm({
      name: p.name,
      type: p.type,
      base_url: p.base_url,
      protocol: p.protocol,
      api_keys: stringifyApiKeys(providerKeys),
      proxy_url: p.proxy_url || "",
      custom_headers: p.custom_headers || "",
      timeout_ms: p.timeout_ms || 30000,
      priority: p.priority ?? 0,
    });
    setHeaderEntries(parsedHeaders);
    setHeadersJsonMode(useJsonMode);
    setKeysLoading(initialEntries.length === 0);
    if (initialEntries.length === 0) {
      void getProviderApiKeys(p.id).then((keys) => {
        setKeysLoading(false);
        if (keys.length === 0) return;
        setKeyEntries(keys.map((value) => ({
          ...createKeyEntry(value, ""),
          reveal: true,
        })));
      }).catch((error) => {
        setKeysLoading(false);
        setFormError(`读取 API Key 失败: ${error instanceof Error ? error.message : String(error)}`);
      });
    }
    setFormError("");
    setEditing(p);
    setModalMode("manual");
    setModalOpen(true);
  };

  /**
   * Build initial key entries from the provider row's api_keys field.
   * Names are matched from provider_key accounts by slot index when possible.
   * Defaults to reveal:true so existing keys are immediately visible on edit.
   */
  const buildKeyEntriesFromProvider = (p: Provider): KeyEntry[] => {
    const values = parseApiKeys(p.api_keys);
    if (values.length === 0) return [];

    // Try to match names from provider_key accounts by slot index
    const slotByName = new Map<number, string>();
    (accounts ?? [])
      .filter((a) => a.provider_id === p.id && a.source_format === "provider_key")
      .forEach((a) => {
        const slot = Number.parseInt((a.external_account_id ?? "").split(":").pop() ?? "", 10);
        if (Number.isInteger(slot) && a.name) {
          slotByName.set(slot, a.name);
        }
      });

    return values.map((value, i) => ({
      ...createKeyEntry(value, slotByName.get(i) ?? ""),
      reveal: true,
    }));
  };

  const handleReloadKeys = () => {
    if (!editing) return;
    setKeysLoading(true);
    getProviderApiKeys(editing.id)
      .then((keys) => {
        setKeysLoading(false);
        if (keys.length === 0) return;
        setKeyEntries((current) => {
          const existingByName = new Map(current.map((e) => [e.value, e.name]));
          return keys.map((value) => ({
            ...createKeyEntry(value, existingByName.get(value) ?? ""),
            reveal: true,
          }));
        });
      })
      .catch((error) => {
        setKeysLoading(false);
        console.error("Failed to reload provider API keys", error);
        setFormError(`读取 API Key 失败: ${error instanceof Error ? error.message : String(error)}`);
      });
  };

  const updateHeaderEntry = (id: string, field: "name" | "value", value: string) => {
    setHeaderEntries((entries) => entries.map((entry) => (
      entry.id === id ? { ...entry, [field]: value } : entry
    )));
    setFormError("");
  };

  const switchHeadersMode = (jsonMode: boolean) => {
    if (jsonMode === headersJsonMode) return;
    if (jsonMode) {
      const headers = Object.fromEntries(
        headerEntries
          .filter((entry) => entry.name.trim())
          .map((entry) => [entry.name.trim(), entry.value]),
      );
      setForm((current) => ({
        ...current,
        custom_headers: Object.keys(headers).length ? JSON.stringify(headers, null, 2) : "",
      }));
      setHeadersJsonMode(true);
      setFormError("");
      return;
    }
    try {
      setHeaderEntries(parseHeaderEntries(form.custom_headers));
      setHeadersJsonMode(false);
      setFormError("");
    } catch (error) {
      setFormError(error instanceof Error ? error.message : "自定义请求头格式错误");
    }
  };

  const findExistingProvider = (name: string, baseUrl: string): Provider | undefined => {
    const list = providers ?? [];
    const trimmedName = name.trim().toLowerCase();
    const trimmedUrl = baseUrl.trim();
    return list.find(
      (p) =>
        (trimmedName && p.name.trim().toLowerCase() === trimmedName) ||
        (trimmedUrl && sameBaseUrl(p.base_url, trimmedUrl)),
    );
  };

  const syncKeyAccounts = async (provider: Provider, entries: { name: string; value: string }[]) => {
    const providerId = provider.id;
    const providerName = provider.name;
    const models = provider.models;
    const cleaned = entries
      .map((e) => ({ name: e.name.trim(), value: e.value.trim() }))
      .filter((e) => e.value.length > 0);
    const tag = `provkey:${providerId}`;
    const owned = (accounts ?? []).filter(
      (a) => a.provider_id === providerId && a.source_format === "provider_key",
    );
    const desiredIds = cleaned.map((_, i) => `${tag}:${i}`);
    const toDelete = owned.filter(
      (a) => !desiredIds.includes(a.external_account_id ?? ""),
    );
    const upserts = cleaned.map((entry, i) => {
      const externalId = desiredIds[i];
      const existing = owned.find((a) => (a.external_account_id ?? "") === externalId);
      const fallbackName = cleaned.length > 1 ? `${providerName} #${i + 1}` : providerName;
      const base: Omit<Account, "id"> = {
        provider_id: providerId,
        name: entry.name || fallbackName,
        api_key: entry.value,
        models,
        status: "active",
        priority: 0,
        credential_type: "api_key",
        source_format: "provider_key",
        external_account_id: externalId,
        quota_used: 0,
        health_status: "unchecked",
      };
      if (existing) {
        return updateAccount.mutateAsync({ ...existing, ...base } as any);
      }
      return createAccount.mutateAsync({ ...base, id: newAccountId() } as any);
    });
    const deletes = toDelete.map((a) => deleteAccount.mutateAsync(a.id));
    await Promise.all([...upserts, ...deletes]);
  };

  const handleSave = async () => {
    setFormError("");
    const isCustomProtocol = form.protocol === "custom";
    const trimmedName = form.name.trim();
    if (!trimmedName) {
      setFormError("请填写供应商名称");
      return;
    }
    if (isCustomProtocol && isGenericProviderName(trimmedName)) {
      setFormError(
        '自定义供应商需要填写独立的名称（不能使用 "custom"、"自定义"、"OpenAI" 等通用名），否则多个自定义供应商会被合并为一个。',
      );
      return;
    }
    if (!editing) {
      const existing = findExistingProvider(trimmedName, form.base_url);
      if (existing) {
        setExistingPrompt(existing);
        return;
      }
    }
    await doSave();
  };

  const handleExistingConfirm = () => {
    const existing = existingPrompt;
    setExistingPrompt(null);
    if (existing) openEdit(existing);
  };

  const doSave = async () => {
    const seen = new Set<string>();
    const keyEntriesOut: { name: string; value: string }[] = [];
    for (const entry of keyEntries) {
      const v = entry.value.trim();
      if (!v) continue;
      if (seen.has(v)) continue;
      seen.add(v);
      keyEntriesOut.push({ name: entry.name.trim(), value: v });
    }
    const apiKeysJson = stringifyApiKeys(keyEntriesOut.map((e) => e.value));

    let rawHeaders = form.custom_headers.trim();
    if (!headersJsonMode) {
      const headers: Record<string, string> = {};
      const normalizedNames = new Set<string>();
      for (const entry of headerEntries) {
        const name = entry.name.trim();
        if (!name) continue;
        const normalized = name.toLowerCase();
        if (blockedHeaderNames.has(normalized)) {
          setFormError(`不允许设置请求头 ${name}`);
          return;
        }
        if (normalizedNames.has(normalized)) {
          setFormError(`请求头 ${name} 重复`);
          return;
        }
        normalizedNames.add(normalized);
        headers[name] = entry.value;
      }
      rawHeaders = Object.keys(headers).length ? JSON.stringify(headers) : "";
    }
    if (rawHeaders) {
      try {
        const headers = JSON.parse(rawHeaders);
        if (!headers || Array.isArray(headers) || typeof headers !== "object") {
          throw new Error("必须是 JSON 对象");
        }
        for (const [name, value] of Object.entries(headers)) {
          if (blockedHeaderNames.has(name.trim().toLowerCase())) throw new Error(`不允许设置请求头 ${name}`);
          if (typeof value !== "string") throw new Error(`请求头 ${name} 的值必须是字符串`);
        }
      } catch (error) {
        setFormError(error instanceof Error ? error.message : "自定义请求头格式错误");
        return;
      }
    }
    setSaving(true);
    try {
      const payload = { ...form, name: form.name.trim(), api_keys: apiKeysJson, custom_headers: rawHeaders };
      let savedProvider: Provider;
      if (editing) {
        await updateProvider.mutateAsync({ ...editing, ...payload, id: editing.id } as any);
        savedProvider = { ...editing, ...payload, id: editing.id } as Provider;
      } else {
        const created = await createProvider.mutateAsync({ ...payload, id: newProviderId() } as any);
        savedProvider = created as Provider;
      }
      if (keyEntriesOut.length > 0) {
        await syncKeyAccounts(savedProvider, keyEntriesOut);
      } else if (editing) {
        await syncKeyAccounts(savedProvider, []);
      }
      setModalOpen(false);
      resetForm();
    } catch (err) {
      console.error("Failed to save provider", err);
    } finally {
      setSaving(false);
    }
  };

  const handleToggle = async (p: Provider) => {
    try {
      await updateProvider.mutateAsync({ ...p, enabled: !p.enabled } as any);
    } catch (err) {
      console.error("Failed to toggle provider", err);
    }
  };

  const handleDeleteClick = (p: Provider) => {
    setDeletingProvider(p);
  };

  const handleDeleteConfirm = async () => {
    if (!deletingProvider) return;
    setDeleteLoading(true);
    try {
      await deleteProvider.mutateAsync(deletingProvider.id);
      setDeletingProvider(null);
    } catch (err) {
      console.error("Failed to delete provider", err);
    } finally {
      setDeleteLoading(false);
    }
  };

  const handleTest = async (p: Provider) => {
    setTestingProviderId(p.id);
    try {
      const result = await testProviderConnection.mutateAsync(p.id);
      setTestResults((prev) => ({ ...prev, [p.id]: result }));
    } catch (err) {
      console.error("Failed to test provider", err);
      setTestResults((prev) => ({
        ...prev,
        [p.id]: {
          success: false,
          message: "测试失败",
          latency_ms: 0,
          error_details: err instanceof Error ? err.message : String(err),
        },
      }));
    } finally {
      setTestingProviderId(null);
    }
  };

  const getTestResultIcon = (providerId: string) => {
    const result = testResults[providerId];
    if (!result) return null;
    if (result.success) {
      return <CheckCircle size={14} className="text-[var(--color-ok)]" />;
    }
    return <XCircle size={14} className="text-[var(--color-err)]" />;
  };

  const getTestResultTooltip = (providerId: string) => {
    const result = testResults[providerId];
    if (!result) return "测试连接 · 发送最小 Chat 请求验证连通性";
    if (result.success) {
      const reply = result.error_details ? `\n回复: "${result.error_details}"` : "";
      return `✓ ${result.message}${result.model_tested ? ` · 模型: ${result.model_tested}` : ""}${reply}`;
    }
    return `✗ ${result.message}${result.error_details ? `\n${result.error_details}` : ""}`;
  };

  // ─── Render ──────────────────────────────────────────────────────────────
  return (
    <div className="space-y-7 animate-fade-in pg-page">
      {/* ─── Page Header ─────────────────────────────────────────────────── */}
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="pg-eyebrow mb-1.5">Model Providers</div>
          <h1 className="text-[26px] leading-tight font-semibold tracking-[-0.01em]" style={{ color: "var(--text-primary)" }}>
            接入模型供应商
          </h1>
          <div className="flex items-center gap-2 mt-3">
            <span className="inline-flex items-center gap-1.5 h-6 px-2.5 rounded-full border text-[11px] font-medium border-[var(--color-brand)]/20 bg-[var(--color-brand-subtle)] text-[var(--color-brand)]">
              {providerStats.enabled}/{providerStats.total} 已启用
            </span>
            <span className="inline-flex items-center h-6 px-2.5 rounded-full border text-[11px] font-medium border-[var(--border-default)] bg-[var(--bg-surface)] text-[var(--text-secondary)]">
              {providerStats.official} 官方
            </span>
            <span className="inline-flex items-center h-6 px-2.5 rounded-full border text-[11px] font-medium border-[var(--border-default)] bg-[var(--bg-surface)] text-[var(--text-secondary)]">
              {providerStats.relay} 中转
            </span>
          </div>
        </div>
        <div className="flex items-center gap-2.5 shrink-0">
          <Button variant="secondary" size="lg" onClick={openCreate}>
            <Terminal size={15} /> 手动添加
          </Button>
          <Button size="lg" onClick={openTemplatePicker}>
            <Plus size={16} /> 添加供应商
          </Button>
        </div>
      </div>

      {/* ─── Add Provider Zone ───────────────────────────────────────────── */}
      <div className="relative rounded-2xl border overflow-hidden"
        style={{ borderColor: "var(--border-default)", backgroundColor: "var(--bg-inset)" }}>
        <div className="absolute inset-0 pointer-events-none"
          style={{ background: "radial-gradient(560px 180px at 12% 0%, var(--color-brand-subtle), transparent 70%)" }} />
        <div className="relative p-6">
          <div className="mb-5">
            <h3 className="text-sm font-semibold" style={{ color: "var(--text-primary)" }}>添加供应商</h3>
            <p className="text-xs mt-0.5" style={{ color: "var(--text-dim)" }}>
              选择一种方式接入上游模型服务，API Key 会自动生成可轮询的账号池
            </p>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
            {/* Template Import — recommended */}
            <button
              onClick={openTemplatePicker}
              className="group relative rounded-xl border p-5 text-left transition-all duration-200 hover:shadow-lg hover:shadow-[var(--color-brand)]/10 hover:-translate-y-px cursor-pointer"
              style={{
                backgroundColor: "var(--bg-surface-solid)",
                borderColor: "color-mix(in srgb, var(--color-brand) 32%, var(--border-default))",
              }}
            >
              <div className="flex items-start justify-between">
                <div className="flex items-center justify-center w-12 h-12 rounded-xl bg-[var(--color-brand-subtle)] shrink-0">
                  <Sparkles size={22} style={{ color: "var(--color-brand)" }} />
                </div>
                <Badge variant="brand">推荐</Badge>
              </div>
              <h4 className="text-[15px] font-semibold mt-4" style={{ color: "var(--text-primary)" }}>模板导入</h4>
              <p className="text-[13px] leading-relaxed mt-1.5 min-h-[42px]" style={{ color: "var(--text-dim)" }}>
                从 {modelResourceTemplates.length} 个内置供应商中选择，Base URL 与协议自动配置
              </p>
              <span className="inline-flex items-center gap-1 text-xs font-medium mt-3 transition-colors" style={{ color: "var(--color-brand)" }}>
                立即选择
                <ArrowRight size={13} className="transition-transform duration-200 group-hover:translate-x-0.5" />
              </span>
            </button>

            {/* Scan Local Config */}
            <button
              onClick={() => { setModalMode("scan"); setModalOpen(true); startScan(); }}
              className="group relative rounded-xl border p-5 text-left transition-all duration-200 hover:shadow-lg hover:shadow-emerald-500/10 hover:-translate-y-px cursor-pointer"
              style={{ backgroundColor: "var(--bg-surface-solid)", borderColor: "var(--border-default)" }}
            >
              <div className="flex items-start justify-between">
                <div className="flex items-center justify-center w-12 h-12 rounded-xl bg-emerald-500/10 shrink-0">
                  <Scan size={22} className="text-emerald-500" />
                </div>
                <Badge variant="ok">快速</Badge>
              </div>
              <h4 className="text-[15px] font-semibold mt-4" style={{ color: "var(--text-primary)" }}>扫描本机配置</h4>
              <p className="text-[13px] leading-relaxed mt-1.5 min-h-[42px]" style={{ color: "var(--text-dim)" }}>
                自动检测环境变量与配置文件中已有的 API Key，勾选后一键导入
              </p>
              <span className="inline-flex items-center gap-1 text-xs font-medium mt-3 transition-colors group-hover:text-emerald-500" style={{ color: "var(--text-secondary)" }}>
                开始扫描
                <ArrowRight size={13} className="transition-transform duration-200 group-hover:translate-x-0.5" />
              </span>
            </button>

            {/* Manual Add */}
            <button
              onClick={openCreate}
              className="group relative rounded-xl border p-5 text-left transition-all duration-200 hover:shadow-lg hover:shadow-blue-500/10 hover:-translate-y-px cursor-pointer"
              style={{ backgroundColor: "var(--bg-surface-solid)", borderColor: "var(--border-default)" }}
            >
              <div className="flex items-start justify-between">
                <div className="flex items-center justify-center w-12 h-12 rounded-xl bg-blue-500/10 shrink-0">
                  <Terminal size={22} className="text-blue-500" />
                </div>
                <Badge variant="info">高级</Badge>
              </div>
              <h4 className="text-[15px] font-semibold mt-4" style={{ color: "var(--text-primary)" }}>手动配置</h4>
              <p className="text-[13px] leading-relaxed mt-1.5 min-h-[42px]" style={{ color: "var(--text-dim)" }}>
                自定义 Base URL、协议、代理与请求头，适合任意兼容 OpenAI 的服务
              </p>
              <span className="inline-flex items-center gap-1 text-xs font-medium mt-3 transition-colors group-hover:text-blue-500" style={{ color: "var(--text-secondary)" }}>
                打开表单
                <ArrowRight size={13} className="transition-transform duration-200 group-hover:translate-x-0.5" />
              </span>
            </button>
          </div>
        </div>
      </div>

      {/* ─── Provider List Toolbar ───────────────────────────────────────── */}
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex items-center gap-2.5">
          <h3 className="text-sm font-semibold" style={{ color: "var(--text-primary)" }}>已接入供应商</h3>
          <Badge variant="mute">{filtered.length}</Badge>
        </div>
        <div className="flex-1 min-w-3" />
        <div className="relative w-56">
          <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2" style={{ color: "var(--text-dim)" }} />
          <input
            className="h-9 w-full rounded-lg border pl-9 pr-3 text-[13px] outline-none transition-colors focus:ring-2 focus:ring-[var(--color-brand)]/40"
            style={{
              backgroundColor: "var(--bg-elevated)",
              borderColor: "var(--border-default)",
              color: "var(--text-primary)",
            }}
            placeholder="搜索供应商..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <Select
          options={[{ value: "", label: "全部类型" }, ...typeOptions]}
          value={filterType}
          onChange={(e) => setFilterType(e.target.value)}
          className="w-32"
        />
        <Select
          options={[{ value: "", label: "全部协议" }, ...protocolOptions]}
          value={filterProtocol}
          onChange={(e) => setFilterProtocol(e.target.value)}
          className="w-36"
        />
        <div className="flex items-center gap-1 p-1 rounded-lg border" style={{ borderColor: "var(--border-default)", backgroundColor: "var(--bg-surface)" }}>
          <button
            onClick={() => setView("grid")}
            className={`p-1.5 rounded-md transition-colors cursor-pointer ${view === "grid" ? "bg-[var(--bg-active)] text-[var(--color-brand)]" : "text-[var(--text-dim)] hover:text-[var(--text-primary)]"}`}
            title="网格视图"
          >
            <Grid3X3 size={15} />
          </button>
          <button
            onClick={() => setView("list")}
            className={`p-1.5 rounded-md transition-colors cursor-pointer ${view === "list" ? "bg-[var(--bg-active)] text-[var(--color-brand)]" : "text-[var(--text-dim)] hover:text-[var(--text-primary)]"}`}
            title="列表视图"
          >
            <LayoutList size={15} />
          </button>
        </div>
      </div>

      {/* Provider Cards / List */}
      {filtered.length === 0 ? (
        <div className="text-center py-20 rounded-2xl border border-dashed" style={{ borderColor: "var(--border-default)" }}>
          <div className="mx-auto w-14 h-14 rounded-2xl grid place-items-center mb-4" style={{ backgroundColor: "var(--bg-inset)" }}>
            <Globe size={26} style={{ color: "var(--text-dim)" }} />
          </div>
          <p className="text-sm font-medium" style={{ color: "var(--text-primary)" }}>
            {search || filterType || filterProtocol ? "没有匹配的供应商" : "还没有接入供应商"}
          </p>
          <p className="text-xs mt-1.5" style={{ color: "var(--text-dim)" }}>
            {search || filterType || filterProtocol
              ? "试试调整搜索关键词或筛选条件"
              : "使用「模板导入」，30 秒接入第一个模型供应商"}
          </p>
          {!(search || filterType || filterProtocol) && (
            <Button className="mt-5" onClick={openTemplatePicker}>
              <Sparkles size={14} /> 模板导入
            </Button>
          )}
        </div>
      ) : view === "grid" ? (
        <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
          {filtered.map((p) => {
            const template = modelResourceTemplates.find((t) => t.name === p.name || sameBaseUrl(t.baseUrl, p.base_url));
            const keyCount = parseApiKeys(p.api_keys).length;
            const enabled = p.enabled !== false;
            let headerCount = 0;
            try { headerCount = Object.keys(JSON.parse(p.custom_headers || "{}")).length; } catch { headerCount = 0; }
            return (
              <div
                key={p.id}
                className="pg-panel p-5 transition-all duration-200 hover:border-[var(--border-strong)] hover:-translate-y-px"
              >
                <div className="flex items-start gap-3.5">
                  {template ? (
                    <BrandTile id={template.id} name={template.name} size={42} />
                  ) : (
                    <div className="w-[42px] h-[42px] rounded-xl grid place-items-center shrink-0" style={{ backgroundColor: "var(--bg-inset)" }}>
                      <Server size={19} style={{ color: "var(--text-dim)" }} />
                    </div>
                  )}
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="text-[15px] font-semibold truncate" style={{ color: "var(--text-primary)" }}>{p.name}</span>
                      {!enabled && <Badge variant="mute">已禁用</Badge>}
                    </div>
                    <div className="flex items-center gap-1.5 mt-1.5">
                      <Badge variant={typeBadge[p.type] || "mute"}>{typeLabel[p.type] || p.type}</Badge>
                      <Badge variant="brand">{p.protocol}</Badge>
                    </div>
                  </div>
                  <button
                    onClick={() => handleToggle(p)}
                    className={`p-2 rounded-lg transition-colors cursor-pointer ${
                      enabled
                        ? "text-[var(--color-ok)] hover:bg-[var(--color-ok-bg)]"
                        : "text-[var(--text-dim)] hover:bg-[var(--bg-hover)]"
                    }`}
                    title={enabled ? "禁用" : "启用"}
                  >
                    {enabled ? <Power size={15} /> : <PowerOff size={15} />}
                  </button>
                </div>

                <div className="mt-4 flex items-center gap-1.5 min-w-0">
                  <Globe size={12} className="shrink-0" style={{ color: "var(--text-dim)" }} />
                  <span className="font-mono text-[11px] truncate" style={{ color: "var(--text-secondary)" }} title={p.base_url}>
                    {p.base_url || "—"}
                  </span>
                </div>
                <div className="flex items-center gap-3 mt-2 text-[11px]" style={{ color: "var(--text-dim)" }}>
                  <span className="inline-flex items-center gap-1"><KeyRound size={11} /> {keyCount} 个 Key</span>
                  <span className="inline-flex items-center gap-1"><Clock size={11} /> {Math.round((p.timeout_ms || 30000) / 1000)}s</span>
                  <span>优先级 {p.priority ?? 0}</span>
                  {headerCount > 0 && <span>{headerCount} 个请求头</span>}
                </div>

                <div className="flex items-center gap-2 mt-4 pt-4 border-t" style={{ borderColor: "var(--border-subtle)" }}>
                  <Button variant="secondary" className="flex-1" onClick={() => openEdit(p)}>
                    <Edit3 size={13} /> 编辑
                  </Button>
                  <Button
                    variant="outline"
                    className="flex-1"
                    onClick={() => handleTest(p)}
                    disabled={testingProviderId === p.id}
                    title={getTestResultTooltip(p.id)}
                  >
                    {testingProviderId === p.id ? (
                      <Spinner size={13} />
                    ) : (
                      getTestResultIcon(p.id) || <Zap size={13} />
                    )}
                    测试
                  </Button>
                  <button
                    onClick={() => handleDeleteClick(p)}
                    className="pg-card-action pg-card-action-danger shrink-0"
                    title="删除"
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        /* List view */
        <div className="rounded-xl border overflow-hidden" style={{ borderColor: "var(--border-default)", backgroundColor: "var(--bg-surface)" }}>
          <div className="grid grid-cols-[auto_1fr_110px_90px_90px_132px] gap-4 px-5 py-3 text-[11px] font-medium uppercase tracking-wider"
            style={{ backgroundColor: "var(--bg-elevated)", color: "var(--text-dim)", borderBottom: "1px solid var(--border-default)" }}>
            <div className="w-8" />
            <div>供应商</div>
            <div>协议</div>
            <div>类型</div>
            <div>Keys</div>
            <div className="text-right">操作</div>
          </div>
          {filtered.map((p) => {
            const template = modelResourceTemplates.find((t) => t.name === p.name || sameBaseUrl(t.baseUrl, p.base_url));
            const enabled = p.enabled !== false;
            return (
              <div
                key={p.id}
                className="grid grid-cols-[auto_1fr_110px_90px_90px_132px] gap-4 px-5 py-3.5 items-center border-t transition-colors hover:bg-[var(--bg-hover)]"
                style={{ borderColor: "var(--border-subtle)" }}
              >
                <div className="w-8">
                  {template ? (
                    <BrandTile id={template.id} name={template.name} size={32} />
                  ) : (
                    <div className="w-8 h-8 rounded-lg grid place-items-center"
                      style={{ backgroundColor: "var(--bg-elevated)" }}>
                      <Server size={15} style={{ color: "var(--text-dim)" }} />
                    </div>
                  )}
                </div>
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium truncate" style={{ color: "var(--text-primary)", opacity: enabled ? 1 : 0.55 }}>{p.name}</span>
                    {!enabled && <Badge variant="mute">已禁用</Badge>}
                  </div>
                  <div className="text-[11px] font-mono truncate mt-0.5" style={{ color: "var(--text-dim)" }}>{p.base_url}</div>
                </div>
                <div>
                  <Badge variant="brand">{p.protocol}</Badge>
                </div>
                <div>
                  <Badge variant={typeBadge[p.type] || "mute"}>
                    {typeLabel[p.type] || p.type}
                  </Badge>
                </div>
                <div className="text-xs" style={{ color: "var(--text-secondary)" }}>
                  {parseApiKeys(p.api_keys).length} 个
                </div>
                <div className="flex items-center justify-end gap-1">
                  <button
                    onClick={() => handleToggle(p)}
                    className={`p-1.5 rounded-md transition-colors cursor-pointer ${
                      enabled
                        ? "text-[var(--color-ok)] hover:bg-[var(--color-ok-bg)]"
                        : "text-[var(--text-dim)] hover:bg-[var(--bg-hover)]"
                    }`}
                    title={enabled ? "禁用" : "启用"}
                  >
                    {enabled ? <Power size={14} /> : <PowerOff size={14} />}
                  </button>
                  <button onClick={() => openEdit(p)} className="p-1.5 rounded-md transition-colors text-[var(--text-dim)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)] cursor-pointer" title="编辑">
                    <Edit3 size={14} />
                  </button>
                  <button onClick={() => handleTest(p)} disabled={testingProviderId === p.id}
                    className="p-1.5 rounded-md transition-colors text-[var(--text-dim)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)] cursor-pointer"
                    title={getTestResultTooltip(p.id)}>
                    {testingProviderId === p.id ? <Spinner size={14} /> : getTestResultIcon(p.id) || <Zap size={14} />}
                  </button>
                  <button onClick={() => handleDeleteClick(p)} className="p-1.5 rounded-md transition-colors text-[var(--color-err)] hover:bg-[var(--color-err-bg)] cursor-pointer" title="删除">
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* ─── Template Picker Modal ────────────────────────────────────────── */}
      <Modal
        open={modalOpen && modalMode === "template"}
        onClose={() => { setModalOpen(false); setSelectedTemplate(null); setTemplateStep("browse"); }}
        title={templateStep === "browse" ? "选择供应商模板" : `配置 ${selectedTemplate?.name}`}
        className="max-w-5xl"
      >
        {templateStep === "browse" ? (
          <div className="space-y-4">
            {/* Search */}
            <div className="relative">
              <Search size={15} className="absolute left-3.5 top-1/2 -translate-y-1/2" style={{ color: "var(--text-dim)" }} />
              <input
                className="h-10 w-full rounded-lg border pl-10 pr-3 text-sm outline-none transition-colors focus:ring-2 focus:ring-[var(--color-brand)]/40"
                style={{
                  backgroundColor: "var(--bg-elevated)",
                  borderColor: "var(--border-default)",
                  color: "var(--text-primary)",
                }}
                placeholder={`搜索 ${modelResourceTemplates.length} 个供应商模板...`}
                value={templateSearch}
                onChange={(e) => setTemplateSearch(e.target.value)}
                autoFocus
              />
            </div>

            {/* Group filter tabs */}
            <div className="flex items-center gap-1 p-1 rounded-lg overflow-x-auto" style={{ backgroundColor: "var(--bg-elevated)" }}>
              <button
                onClick={() => setTemplateGroupFilter("all")}
                className={`flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md transition-colors whitespace-nowrap cursor-pointer ${
                  templateGroupFilter === "all"
                    ? "bg-[var(--bg-surface)] text-[var(--color-brand)] font-medium"
                    : "text-[var(--text-dim)] hover:text-[var(--text-secondary)]"
                }`}
              >
                全部 ({modelResourceTemplates.length})
              </button>
              {(["official", "cn_official", "aggregator", "third_party", "cloud", "custom"] as ProviderGroup[]).map((group) => (
                <button
                  key={group}
                  onClick={() => setTemplateGroupFilter(group)}
                  className={`flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md transition-colors whitespace-nowrap cursor-pointer ${
                    templateGroupFilter === group
                      ? "bg-[var(--bg-surface)] text-[var(--color-brand)] font-medium"
                      : "text-[var(--text-dim)] hover:text-[var(--text-secondary)]"
                  }`}
                >
                  {groupIcons[group]}
                  {providerGroupLabels[group]} ({templateGroups.get(group) || 0})
                </button>
              ))}
            </div>

            {/* Template grid */}
            {filteredTemplates.length === 0 ? (
              <div className="text-center py-16" style={{ color: "var(--text-dim)" }}>
                <p className="text-sm">没有匹配「{templateSearch}」的模板</p>
                <p className="text-xs mt-1">可以改用「手动配置」添加任意兼容服务</p>
              </div>
            ) : (
              <div className="grid grid-cols-2 md:grid-cols-3 gap-3.5 max-h-[460px] overflow-y-auto pr-1">
                {filteredTemplates.map((template) => (
                  <button
                    key={template.id}
                    onClick={() => selectTemplate(template)}
                    className="group flex items-start gap-3.5 p-4 rounded-xl border text-left transition-all duration-200 hover:border-[var(--color-brand)]/50 hover:bg-[var(--bg-hover)] hover:-translate-y-px cursor-pointer"
                    style={{ borderColor: "var(--border-default)" }}
                  >
                    <BrandTile id={template.id} name={template.name} size={44} />
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-1.5">
                        <span className="text-sm font-semibold truncate" style={{ color: "var(--text-primary)" }}>
                          {template.name}
                        </span>
                        {template.recommended && (
                          <Star size={12} className="text-amber-500 fill-amber-500 shrink-0" />
                        )}
                      </div>
                      <p className="text-[11px] mt-1 line-clamp-2 leading-relaxed" style={{ color: "var(--text-dim)" }}>
                        {template.description}
                      </p>
                      <div className="flex flex-wrap items-center gap-1 mt-2">
                        {template.protocols.slice(0, 2).map((p) => (
                          <span key={p} className="text-[9px] px-1.5 py-0.5 rounded"
                            style={{ backgroundColor: "var(--bg-elevated)", color: "var(--text-dim)" }}>
                            {p}
                          </span>
                        ))}
                        {template.authMethods.includes("oauth") && (
                          <span className="text-[9px] px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-400">
                            OAuth
                          </span>
                        )}
                      </div>
                    </div>
                  </button>
                ))}
              </div>
            )}
          </div>
        ) : (
          /* Configure step */
          <div className="space-y-4">
            {/* Selected template info */}
            {selectedTemplate && (
              <div className="flex items-center gap-4 p-4 rounded-lg" style={{ backgroundColor: "var(--bg-elevated)" }}>
                <BrandTile id={selectedTemplate.id} name={selectedTemplate.name} size={48} />
                <div className="flex-1">
                  <div className="text-sm font-semibold" style={{ color: "var(--text-primary)" }}>{selectedTemplate.name}</div>
                  <div className="text-xs mt-0.5" style={{ color: "var(--text-dim)" }}>{selectedTemplate.description}</div>
                  <div className="flex items-center gap-2 mt-1.5">
                    <span className="text-[10px] font-mono px-2 py-0.5 rounded"
                      style={{ backgroundColor: "var(--bg-surface)", color: "var(--text-secondary)" }}>
                      {selectedTemplate.baseUrl}
                    </span>
                    {selectedTemplate.protocols.map((p) => (
                      <span key={p} className="text-[10px] px-2 py-0.5 rounded"
                        style={{ backgroundColor: "var(--bg-surface)", color: "var(--text-dim)" }}>
                        {p}
                      </span>
                    ))}
                  </div>
                </div>
                <Button variant="ghost" size="sm" onClick={() => setTemplateStep("browse")}>
                  更换
                </Button>
              </div>
            )}

            {/* API Key input */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-xs font-medium" style={{ color: "var(--text-primary)" }}>
                  API Key
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setTemplateKeyEntries((entries) => [...entries, createKeyEntry()])}
                >
                  <Plus size={13} /> 添加更多 Key
                </Button>
              </div>
              {templateKeyEntries.map((entry, i) => (
                <div key={entry.id} className="flex items-start gap-2">
                  <span className="mt-2 w-6 shrink-0 text-center text-[11px] tabular-nums"
                    style={{ color: "var(--text-dim)" }}>{i + 1}</span>
                  <div className="flex-1 space-y-1.5">
                    <input
                      type="text"
                      className="h-8 w-full rounded-md border px-2.5 text-sm outline-none transition-colors focus:ring-2 focus:ring-[var(--color-brand)]/40"
                      style={{
                        backgroundColor: "var(--bg-elevated)",
                        borderColor: "var(--border-default)",
                        color: "var(--text-primary)",
                      }}
                      value={entry.name}
                      onChange={(e) =>
                        setTemplateKeyEntries((entries) =>
                          entries.map((x) => (x.id === entry.id ? { ...x, name: e.target.value } : x)),
                        )
                      }
                      placeholder="名称（可选，如：工作账号）"
                    />
                    <div className="flex items-center gap-1.5">
                      <input
                        type={entry.reveal ? "text" : "password"}
                        className="h-8 flex-1 rounded-md border px-2.5 font-mono text-sm outline-none transition-colors focus:ring-2 focus:ring-[var(--color-brand)]/40"
                        style={{
                          backgroundColor: "var(--bg-elevated)",
                          borderColor: "var(--border-default)",
                          color: "var(--text-primary)",
                        }}
                        value={entry.value}
                        onChange={(e) =>
                          setTemplateKeyEntries((entries) =>
                            entries.map((x) => (x.id === entry.id ? { ...x, value: e.target.value } : x)),
                          )
                        }
                        placeholder={`sk-${"x".repeat(8)}...`}
                      />
                      <button
                        type="button"
                        onClick={() =>
                          setTemplateKeyEntries((entries) =>
                            entries.map((x) => (x.id === entry.id ? { ...x, reveal: !x.reveal } : x)),
                          )
                        }
                        className="p-1.5 rounded-md transition-colors text-[var(--text-dim)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)]"
                      >
                        {entry.reveal ? <EyeOff size={14} /> : <Eye size={14} />}
                      </button>
                    </div>
                  </div>
                  {templateKeyEntries.length > 1 && (
                    <button
                      type="button"
                      onClick={() => setTemplateKeyEntries((entries) => entries.filter((x) => x.id !== entry.id))}
                      className="mt-2 p-1.5 rounded-md transition-colors text-[var(--text-dim)] hover:text-[var(--color-err)] hover:bg-[var(--color-err-bg)]"
                    >
                      <Trash2 size={14} />
                    </button>
                  )}
                </div>
              ))}
            </div>

            {/* Error message */}
            {formError && (
              <div className="flex items-center gap-2 p-3 rounded-lg text-xs"
                style={{ backgroundColor: "var(--color-err-bg, rgba(239, 68, 68, 0.1))", color: "var(--color-err)" }}>
                <AlertCircle size={14} />
                {formError}
              </div>
            )}

            {/* Actions */}
            <div className="flex justify-end gap-3 pt-2">
              <Button variant="secondary" onClick={() => { setModalOpen(false); setSelectedTemplate(null); }}>
                取消
              </Button>
              <Button onClick={handleTemplateSave} disabled={saving}>
                {saving ? <Spinner size={16} /> : null}
                导入并保存
              </Button>
            </div>
          </div>
        )}
      </Modal>

      {/* ─── Scan Modal ───────────────────────────────────────────────────── */}
      <Modal
        open={modalOpen && modalMode === "scan"}
        onClose={() => { setModalOpen(false); setScanResults([]); }}
        title="扫描本机配置"
        className="max-w-2xl"
      >
        <div className="space-y-4">
          {scanning ? (
            <div className="text-center py-12">
              <RefreshCw size={32} className="mx-auto mb-4 animate-spin" style={{ color: "var(--color-brand)" }} />
              <p className="text-sm" style={{ color: "var(--text-primary)" }}>正在扫描本机配置...</p>
              <p className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>
                检测环境变量、~/.claude、~/.config/openai 等位置
              </p>
            </div>
          ) : scanResults.length === 0 ? (
            <div className="text-center py-12">
              <Scan size={48} className="mx-auto mb-3 opacity-40" style={{ color: "var(--text-dim)" }} />
              <p className="text-sm" style={{ color: "var(--text-primary)" }}>未检测到 API Key 配置</p>
              <p className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>
                可以尝试「模板导入」或「手动配置」
              </p>
              <div className="flex justify-center gap-3 mt-4">
                <Button variant="secondary" onClick={startScan}>
                  <RefreshCw size={14} /> 重新扫描
                </Button>
                <Button onClick={() => { setModalMode("template"); openTemplatePicker(); }}>
                  <Sparkles size={14} /> 模板导入
                </Button>
              </div>
            </div>
          ) : (
            <>
              <div className="flex items-center justify-between">
                <div>
                  <p className="text-sm font-medium" style={{ color: "var(--text-primary)" }}>
                    检测到 {scanResults.length} 个配置
                  </p>
                  <p className="text-xs mt-0.5" style={{ color: "var(--text-dim)" }}>
                    选择要导入的配置
                  </p>
                </div>
                <Button variant="ghost" size="sm" onClick={startScan}>
                  <RefreshCw size={13} /> 重新扫描
                </Button>
              </div>

              <div className="space-y-2">
                {scanResults.map((result, index) => (
                  <div
                    key={index}
                    className="flex items-center gap-3 p-3 rounded-lg border cursor-pointer transition-colors"
                    style={{
                      borderColor: selectedScanResults.has(index) ? "var(--color-brand)" : "var(--border-default)",
                      backgroundColor: selectedScanResults.has(index) ? "var(--color-brand-bg, rgba(124, 58, 237, 0.05))" : "var(--bg-surface)",
                    }}
                    onClick={() => {
                      const next = new Set(selectedScanResults);
                      if (next.has(index)) next.delete(index);
                      else next.add(index);
                      setSelectedScanResults(next);
                    }}
                  >
                    <div className={`w-5 h-5 rounded border flex items-center justify-center transition-colors ${
                      selectedScanResults.has(index)
                        ? "bg-[var(--color-brand)] border-[var(--color-brand)]"
                        : "border-[var(--border-default)]"
                    }`}>
                      {selectedScanResults.has(index) && <CheckCircle2 size={12} className="text-white" />}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-medium" style={{ color: "var(--text-primary)" }}>
                          {result.provider}
                        </span>
                        <Badge variant="info" className="text-[10px]">{result.source}</Badge>
                      </div>
                      <div className="text-[11px] font-mono mt-0.5" style={{ color: "var(--text-dim)" }}>
                        {result.path}
                      </div>
                    </div>
                    <div className="text-xs font-mono" style={{ color: "var(--text-dim)" }}>
                      {result.keyPreview}
                    </div>
                  </div>
                ))}
              </div>

              <div className="flex justify-end gap-3 pt-2">
                <Button variant="secondary" onClick={() => setModalOpen(false)}>
                  取消
                </Button>
                <Button onClick={importScanResults} disabled={selectedScanResults.size === 0}>
                  <Download size={14} /> 导入选中配置 ({selectedScanResults.size})
                </Button>
              </div>
            </>
          )}
        </div>
      </Modal>

      {/* ─── Manual Edit Modal ────────────────────────────────────────────── */}
      <Modal
        open={modalOpen && modalMode === "manual"}
        onClose={() => { setModalOpen(false); resetForm(); }}
        title={editing ? "编辑服务商" : "添加服务商"}
        className="max-w-2xl"
      >
        <div className="space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1">
              <Input
                label="名称"
                value={form.name}
                onChange={(e) => setForm({ ...form, name: e.target.value })}
                placeholder="例如：小七中转站、Agent Router"
              />
              {form.protocol === "custom" && (
                <p className="text-[11px]" style={{ color: "var(--text-dim)" }}>
                  自定义供应商需填写独立名称；多个自定义供应商共用同一名称会导致它们的 Base URL 与 API Key 被互相覆盖。
                </p>
              )}
            </div>
            <Select
              label="类型"
              options={typeOptions}
              value={form.type}
              onChange={(e) => setForm({ ...form, type: e.target.value })}
            />
          </div>
          <Input
            label="Base URL"
            value={form.base_url}
            onChange={(e) => setForm({ ...form, base_url: e.target.value })}
            placeholder="https://api.openai.com/v1"
          />
          <div className="grid grid-cols-2 gap-4">
            <Select
              label="协议"
              options={protocolOptions}
              value={form.protocol}
              onChange={(e) => setForm({ ...form, protocol: e.target.value })}
            />
            <Input
              label="代理 URL"
              value={form.proxy_url}
              onChange={(e) => setForm({ ...form, proxy_url: e.target.value })}
              placeholder="http://proxy:8080 (可选)"
            />
          </div>
          {/* API Keys */}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <span className="text-xs font-medium" style={{ color: "var(--text-primary)" }}>
                API Keys
                <span className="ml-1 font-normal" style={{ color: "var(--text-dim)" }}>
                  每个 key 自动生成一个账号并轮询使用
                </span>
              </span>
              <div className="flex items-center gap-1">
                {editing && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={handleReloadKeys}
                    disabled={keysLoading}
                    title="从凭据库重新加载"
                  >
                    <RefreshCw size={13} className={keysLoading ? "animate-spin" : ""} />
                  </Button>
                )}
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setKeyEntries((entries) => [...entries, createKeyEntry()])}
                >
                  <Plus size={14} /> 添加 Key
                </Button>
              </div>
            </div>
            {keysLoading ? (
              <div className="flex items-center justify-center py-4"
                style={{ color: "var(--text-dim)" }}>
                <Spinner size={16} />
                <span className="ml-2 text-xs">正在从凭据库读取 API Key…</span>
              </div>
            ) : keyEntries.length === 0 ? (
              <div className="rounded-md border border-dashed px-3 py-3 text-center text-xs"
                style={{ borderColor: "var(--border-default)", color: "var(--text-dim)" }}>
                暂无 API Key。点击「添加 Key」为该供应商配置一个或多个 key。
              </div>
            ) : (
              <div className="space-y-2">
                {keyEntries.map((entry, i) => (
                  <div key={entry.id} className="flex items-start gap-2">
                    <span className="mt-1.5 w-6 shrink-0 text-center text-[11px] tabular-nums"
                      style={{ color: "var(--text-dim)" }}>{i + 1}</span>
                    <div className="flex-1 space-y-1.5">
                      <input
                        type="text"
                        className="h-8 w-full rounded-md border px-2.5 text-sm outline-none transition-colors focus:ring-2 focus:ring-[var(--color-brand)]/40"
                        style={{
                          backgroundColor: "var(--bg-elevated)",
                          borderColor: "var(--border-default)",
                          color: "var(--text-primary)",
                        }}
                        value={entry.name}
                        onChange={(e) =>
                          setKeyEntries((entries) =>
                            entries.map((x) => (x.id === entry.id ? { ...x, name: e.target.value } : x)),
                          )
                        }
                        placeholder="名称（可选，如：工作账号）"
                      />
                      <div className="flex items-center gap-1.5">
                        <input
                          type={entry.reveal ? "text" : "password"}
                          className="h-8 flex-1 rounded-md border px-2.5 font-mono text-sm outline-none transition-colors focus:ring-2 focus:ring-[var(--color-brand)]/40"
                          style={{
                            backgroundColor: "var(--bg-elevated)",
                            borderColor: "var(--border-default)",
                            color: "var(--text-primary)",
                          }}
                          value={entry.value}
                          onChange={(e) =>
                            setKeyEntries((entries) =>
                              entries.map((x) => (x.id === entry.id ? { ...x, value: e.target.value } : x)),
                            )
                          }
                          placeholder={`sk-${"x".repeat(8)}...`}
                        />
                        <button
                          type="button"
                          title={entry.reveal ? "隐藏" : "显示明文"}
                          onClick={() =>
                            setKeyEntries((entries) =>
                              entries.map((x) => (x.id === entry.id ? { ...x, reveal: !x.reveal } : x)),
                            )
                          }
                          className="p-1.5 rounded-md transition-colors text-[var(--text-dim)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)]"
                        >
                          {entry.reveal ? <EyeOff size={14} /> : <Eye size={14} />}
                        </button>
                      </div>
                    </div>
                    <button
                      type="button"
                      onClick={() => setKeyEntries((entries) => entries.filter((x) => x.id !== entry.id))}
                      className="mt-1.5 p-1.5 rounded-md transition-colors text-[var(--text-dim)] hover:text-[var(--color-err)] hover:bg-[var(--color-err-bg)]"
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
          {/* Custom headers */}
          <div className="rounded-md border" style={{ borderColor: formError ? "var(--color-err)" : "var(--border-default)" }}>
            <div className="flex items-center justify-between gap-3 border-b px-3 py-2.5" style={{ borderColor: "var(--border-subtle)", backgroundColor: "var(--bg-inset)" }}>
              <div className="flex min-w-0 items-center gap-2">
                <KeyRound size={15} style={{ color: "var(--color-brand)" }} />
                <div>
                  <div className="text-xs font-medium" style={{ color: "var(--text-primary)" }}>自定义请求头</div>
                  <div className="text-[11px]" style={{ color: "var(--text-dim)" }}>随每个请求发送到该上游服务商</div>
                </div>
              </div>
              <div className="flex shrink-0 rounded-md border p-0.5" style={{ borderColor: "var(--border-default)", backgroundColor: "var(--bg-elevated)" }}>
                <button
                  type="button"
                  className={`flex h-6 items-center gap-1 rounded px-2 text-[11px] transition-colors ${!headersJsonMode ? "bg-[var(--bg-active)] text-[var(--text-primary)]" : "text-[var(--text-dim)] hover:text-[var(--text-primary)]"}`}
                  onClick={() => switchHeadersMode(false)}
                >
                  <List size={12} /> 键值
                </button>
                <button
                  type="button"
                  className={`flex h-6 items-center gap-1 rounded px-2 text-[11px] transition-colors ${headersJsonMode ? "bg-[var(--bg-active)] text-[var(--text-primary)]" : "text-[var(--text-dim)] hover:text-[var(--text-primary)]"}`}
                  onClick={() => switchHeadersMode(true)}
                >
                  <Braces size={12} /> JSON
                </button>
              </div>
            </div>
            {headersJsonMode ? (
              <textarea
                className="min-h-32 w-full resize-y border-0 px-3 py-2.5 font-mono text-xs outline-none"
                style={{ backgroundColor: "var(--bg-elevated)", color: "var(--text-primary)" }}
                value={form.custom_headers}
                onChange={(e) => { setForm({ ...form, custom_headers: e.target.value }); setFormError(""); }}
                placeholder={'{"HTTP-Referer":"https://example.com","X-Title":"My App"}'}
                spellCheck={false}
              />
            ) : (
              <div className="space-y-2 p-3">
                {headerEntries.length === 0 ? (
                  <button
                    type="button"
                    className="flex w-full items-center justify-center gap-1.5 rounded-md border border-dashed py-4 text-xs transition-colors hover:bg-[var(--bg-hover)]"
                    style={{ borderColor: "var(--border-default)", color: "var(--text-dim)" }}
                    onClick={() => setHeaderEntries([createHeaderEntry()])}
                  >
                    <Plus size={14} /> 添加第一个请求头
                  </button>
                ) : (
                  <>
                    <div className="grid grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)_28px] gap-2 px-0.5 text-[10px] uppercase" style={{ color: "var(--text-dim)" }}>
                      <span>名称</span>
                      <span>值</span>
                      <span />
                    </div>
                    {headerEntries.map((entry) => (
                      <div key={entry.id} className="grid grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)_28px] items-center gap-2">
                        <input
                          className="h-8 min-w-0 rounded-md border px-2.5 font-mono text-xs outline-none focus:ring-2 focus:ring-[var(--color-brand)]/40"
                          style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-default)", color: "var(--text-primary)" }}
                          value={entry.name}
                          onChange={(e) => updateHeaderEntry(entry.id, "name", e.target.value)}
                          placeholder="X-Custom-Header"
                          spellCheck={false}
                        />
                        <input
                          className="h-8 min-w-0 rounded-md border px-2.5 font-mono text-xs outline-none focus:ring-2 focus:ring-[var(--color-brand)]/40"
                          style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-default)", color: "var(--text-primary)" }}
                          value={entry.value}
                          onChange={(e) => updateHeaderEntry(entry.id, "value", e.target.value)}
                          placeholder="请求头值"
                          spellCheck={false}
                        />
                        <button
                          type="button"
                          className="flex h-7 w-7 items-center justify-center rounded-md text-[var(--text-dim)] transition-colors hover:bg-[var(--color-err-bg)] hover:text-[var(--color-err)]"
                          onClick={() => setHeaderEntries((entries) => entries.filter((item) => item.id !== entry.id))}
                        >
                          <Trash2 size={13} />
                        </button>
                      </div>
                    ))}
                    <Button type="button" variant="ghost" size="sm" onClick={() => setHeaderEntries((entries) => [...entries, createHeaderEntry()])}>
                      <Plus size={13} /> 添加请求头
                    </Button>
                  </>
                )}
              </div>
            )}
            <p className="border-t px-3 py-2 text-[11px]" style={{ borderColor: "var(--border-subtle)", color: formError ? "var(--color-err)" : "var(--text-dim)" }}>
              {formError || "Authorization、Host、Content-Length、Connection 等安全相关请求头不可覆盖。"}
            </p>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <Input
              label="超时 (ms)"
              type="number"
              value={form.timeout_ms}
              onChange={(e) => setForm({ ...form, timeout_ms: parseInt(e.target.value) || 30000 })}
            />
            <Input
              label="优先级"
              type="number"
              value={form.priority}
              onChange={(e) => setForm({ ...form, priority: parseInt(e.target.value) || 0 })}
            />
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <Button variant="secondary" onClick={() => { setModalOpen(false); resetForm(); }}>
              取消
            </Button>
            <Button onClick={handleSave} disabled={saving || !form.name}>
              {saving ? <Spinner size={16} /> : null}
              {editing ? "保存" : "创建"}
            </Button>
          </div>
        </div>
      </Modal>

      {/* ─── Confirm Dialogs ──────────────────────────────────────────────── */}
      <ConfirmDialog
        open={!!deletingProvider}
        onClose={() => { if (!deleteLoading) setDeletingProvider(null); }}
        onConfirm={handleDeleteConfirm}
        title="删除服务商"
        message={`确定删除服务商「${deletingProvider?.name}」？删除后该服务商下的账号和路由关联将一并移除，此操作不可撤销。`}
        confirmText="删除"
        cancelText="取消"
        variant="danger"
        loading={deleteLoading}
      />

      <ConfirmDialog
        open={!!existingPrompt}
        onClose={() => setExistingPrompt(null)}
        onConfirm={handleExistingConfirm}
        title="该供应商已存在"
        message={`已存在名称或 Base URL 相同的供应商「${existingPrompt?.name}」。是否切换到编辑该供应商，在其下继续添加 / 管理 API Key？`}
        confirmText="切换到编辑"
        cancelText="取消"
      />
    </div>
  );
}
