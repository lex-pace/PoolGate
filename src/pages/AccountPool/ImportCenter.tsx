import { useEffect, useMemo, useState, type ReactNode } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import {
  useAccounts,
  useBatchUpdateAccounts,
  useCancelOAuthLogin,
  useCompleteOAuthLogin,
  useExecuteImport,
  useFetchUpstreamModels,
  usePreviewAndCheckImport,
  useScanAgentConfigs,
  useStartOAuthLogin,
} from "@/hooks/use-tauri";
import { useToast } from "@/components/ui/Toast";
import {
  cancelOAuthLogin as cancelOAuthSession,
  type AgentConfigScanResult,
  type CheckedAccount,
  type DiscoveredAccount,
  type DiscoveredModelResource,
  type ImportCheckResult,
  type ImportOptions,
  type ImportSourceRequest,
} from "@/lib/tauri-commands";
import {
  baseUrlForProtocol,
  getResourceTemplate,
  modelResourceTemplates,
  providerGroupLabels,
  importFormatLabels,
  primaryProtocol,
  protocolsLabel,
  protocolOptions,
  modelResourceCategoryLabels,
  resourceCategoryForTemplate,
  resourceCategoryTag,
  type AuthMethod,
  type ModelResourceTemplate,
  type Protocol,
  type ProviderGroup,
} from "@/lib/model-resource-templates";
import { getProviderBrand } from "@/lib/provider-brand";
import {
  Braces,
  Boxes,
  Check,
  CheckCircle2,
  ChevronDown,
  Copy,
  ExternalLink,
  EyeOff,
  FolderOpen,
  Globe2,
  Heart,
  KeyRound,
  Lightbulb,
  Plus,
  Radar,
  RefreshCw,
  Search,
  ShieldCheck,
  Sparkles,
  Star,
  Tags,
  TriangleAlert,
  Upload,
  X,
} from "lucide-react";

/* ---------- Constants & helpers ---------- */

/** Generic provider names that must NOT be used as a custom provider's identity. */
const GENERIC_PROVIDER_NAMES = new Set([
  "custom", "自定义", "自定义供应商", "provider", "relay", "中转", "其他", "other", "unknown",
  "openai", "codex", "chatgpt", "anthropic", "claude", "google", "gemini", "antigravity", "xai", "grok",
]);

const isGenericProviderName = (name: string): boolean => {
  const normalized = name.trim().toLowerCase().replace(/[\s\-_]/g, "");
  return !normalized || GENERIC_PROVIDER_NAMES.has(normalized) || GENERIC_PROVIDER_NAMES.has(name.trim());
};

let keyEntrySeq = 0;
const createKeyEntry = (value = "", name = "") => ({
  id: `import-key-${keyEntrySeq++}`,
  name,
  value,
  reveal: false,
});

const methodMeta: Record<AuthMethod, { label: string; icon: typeof Globe2; desc: string }> = {
  oauth: { label: "OAuth 授权", icon: Globe2, desc: "浏览器登录获取 Token" },
  token: { label: "Token & JSON", icon: Braces, desc: "粘贴 JSON、access_token 或 refresh_token" },
  apikey: { label: "API Key", icon: KeyRound, desc: "输入 API Key 并选择模型" },
  batch: { label: "批量导入", icon: Upload, desc: "从文件批量导入多账号" },
};

const formatLabels: Record<string, string> = {
  ...importFormatLabels,
  mixed: "混合格式",
};

const conflictOptions: { value: "skip" | "overwrite" | "merge"; label: string; title: string }[] = [
  {
    value: "skip",
    label: "跳过重复",
    title: "已存在相同凭据的账号时保持原样（默认）",
  },
  {
    value: "overwrite",
    label: "覆盖更新",
    title: "用新配置整体替换已有账号的凭据与元数据",
  },
  {
    value: "merge",
    label: "合并缺失",
    title: "只补充已有账号缺失的字段，保留原状态",
  },
];

/** Map backend categorized errors (`[CATEGORY] message`) to clear Chinese text. */
function importErrorMessage(err: string): string {
  const match = /^\[([A-Z_]+)\]\s*(.*)$/s.exec(err.trim());
  if (!match) return err;
  const category = match[1];
  const detail = match[2].trim();
  const map: Record<string, string> = {
    UNSUPPORTED_FORMAT: "无法识别的配置格式（既不是 JSON / YAML / TOML，也不是受支持的账号格式）",
    MISSING_FIELD: "配置缺少必要字段",
    INVALID_VALUE: "配置内容非法",
    UNREADABLE_FILE: "无法读取配置文件（文件不存在或没有权限）",
    NO_SOURCE: "未提供任何导入内容或文件",
  };
  const base = map[category];
  if (!base) return err;
  return detail ? `${base}：${detail}` : base;
}

const credentialLabels: Record<string, string> = {
  api_key: "API Key",
  upstream_key: "上游网关 Key",
  oauth: "OAuth Token",
  token: "Token",
  codex_oauth: "Codex OAuth",
};

function protocolLabel(protocol: Protocol): string {
  return protocolOptions.find((option) => option.value === protocol)?.label || protocol;
}

/**
 * Provider display order for the flat cc-switch style grid. `custom` comes
 * first (matching cc-switch, where "自定义配置" is the leading tile), followed
 * by official and the rest.
 */
const providerGroupOrder: ProviderGroup[] = [
  "custom",
  "official",
  "cn_official",
  "aggregator",
  "third_party",
  "cloud",
];

/* ---------- Main component ---------- */

export function ImportCenter({ onClose }: { onClose: () => void }) {
  const previewAndCheckImport = usePreviewAndCheckImport();
  const executeImport = useExecuteImport();
  const batchUpdateAccounts = useBatchUpdateAccounts();
  const startOAuth = useStartOAuthLogin();
  const completeOAuth = useCompleteOAuthLogin();
  const cancelOAuth = useCancelOAuthLogin();
  const scanConfigs = useScanAgentConfigs();
  const { toast } = useToast();

  // Import entry mode: provider-template based flow, or one-click scan of
  // well-known Codex / Cockpit config locations.
  const [mode, setMode] = useState<"provider" | "scan">("provider");
  const [discovered, setDiscovered] = useState<AgentConfigScanResult | null>(null);
  const [selectedScanFingerprints, setSelectedScanFingerprints] = useState<Set<string>>(new Set());
  const [selectedScanModelKeys, setSelectedScanModelKeys] = useState<Set<string>>(new Set());
  // Conflict resolution for accounts that already exist.
  const [conflictMode, setConflictMode] = useState<"skip" | "overwrite" | "merge">("skip");

  // Get existing accounts to determine which fingerprints are already added
  const { data: existingAccounts = [] } = useAccounts();
  const existingFingerprints = useMemo(() => {
    // Use name + provider_id + api_key as a simple fingerprint for comparison
    return new Set(
      existingAccounts
        .map((account) => `${account.name || ""}|${account.provider_id || ""}|${account.api_key || ""}`)
        .filter((fingerprint) => fingerprint !== "||")
    );
  }, [existingAccounts]);

  const [selectedTemplateId, setSelectedTemplateId] = useState<string>("");
  const [providerSearch, setProviderSearch] = useState("");

  const template = getResourceTemplate(selectedTemplateId);
  const [method, setMethod] = useState<AuthMethod>("apikey");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (template && !template.authMethods.includes(method)) {
      setMethod(template.authMethods[0]);
    }
  }, [selectedTemplateId, template, method]);

  useEffect(() => {
    setCheckResult(null);
    setSelectedFingerprints(new Set());
    setError(null);
  }, [selectedTemplateId, method]);

  useEffect(() => {
    if (template) {
      // Custom upstreams declare no protocols; default to OpenAI-compatible so
      // the Base URL field is visible immediately instead of being hidden.
      const protocols =
        template.protocols.length > 0 ? template.protocols : (["chat"] as Protocol[]);
      setSelectedProtocols(protocols);
      setBaseUrls(
        Object.fromEntries(
          protocols.map((protocol) => [protocol, baseUrlForProtocol(template, protocol)]),
        ) as Partial<Record<Protocol, string>>,
      );
    }
  }, [selectedTemplateId, template]);

  // Credential state
  const [apiKey, setApiKey] = useState("");
  const [baseUrls, setBaseUrls] = useState<Partial<Record<Protocol, string>>>({});
  const [customModels, setCustomModels] = useState("");
  const [selectedProtocols, setSelectedProtocols] = useState<Protocol[]>([]);
  const [resourceName, setResourceName] = useState("");
  const [tokenContent, setTokenContent] = useState("");
  const [paths, setPaths] = useState<string[]>([]);
  const [showFormatHint, setShowFormatHint] = useState(false);
  // Multi-key support: each entry has an optional name and a key value.
  const [keyEntries, setKeyEntries] = useState<Array<{ id: string; name: string; value: string; reveal: boolean }>>([]);
  // Custom provider name: required when importing a custom provider to create
  // a distinct provider identity (avoid merging with other custom providers).
  const [customProviderName, setCustomProviderName] = useState("");

  // OAuth state
  const [pendingEmail, setPendingEmail] = useState("");
  const [pendingNote, setPendingNote] = useState("");
  const [callbackUrl, setCallbackUrl] = useState("");
  const [oauthLoginId, setOauthLoginId] = useState("");
  const [oauthAuthorizationUrl, setOauthAuthorizationUrl] = useState("");

  useEffect(() => () => {
    if (oauthLoginId) void cancelOAuthSession(oauthLoginId);
  }, [oauthLoginId]);

  // Import check state
  const [checkResult, setCheckResult] = useState<ImportCheckResult | null>(null);
  const [selectedFingerprints, setSelectedFingerprints] = useState<Set<string>>(new Set());
  const [batchTags, setBatchTags] = useState("");
  const [checkBeforeImport, setCheckBeforeImport] = useState(true);

  const request: ImportSourceRequest | null = useMemo(() => {
    if (mode === "scan") {
      // Scan mode: account selection is applied by full credential fingerprint
      // during execution; selected source files are reparsed so preview data is
      // never trusted as a credential source.
      if (!discovered || selectedScanFingerprints.size === 0) return null;
      const selectedPaths = new Set(
        discovered.accounts
          .filter((account) => selectedScanFingerprints.has(account.fingerprint))
          .flatMap((account) => account.source_paths),
      );
      if (selectedPaths.size === 0) return null;
      return { paths: Array.from(selectedPaths), provider_hint: undefined };
    }
    if (!template) return null;
    const protocols = selectedProtocols;
    const protocol = protocols[0] || primaryProtocol(template) || "";
    if (!protocol || protocols.length === 0) return null;
    // Store raw URL as-is; normalization only happens at routing/request time.
    const effectiveBaseUrls = Object.fromEntries(
      protocols.map((item) => [
        item,
        baseUrls[item]?.trim() || baseUrlForProtocol(template, item) || "",
      ]),
    ) as Partial<Record<Protocol, string>>;
    if (protocols.some((item) => !effectiveBaseUrls[item])) return null;
    const defaultBaseUrl = effectiveBaseUrls[protocol] || "";
    // Custom providers use user-specified name; others use template name.
    const isCustom = template.group === "custom";
    const providerName = isCustom ? customProviderName.trim() || template.name : template.name;
    const resourceCategory = resourceCategoryForTemplate(template, method);
    const resourceTags = [
      template.group,
      template.id,
      resourceCategoryTag(resourceCategory),
    ];
    const providerHint = {
      // Custom providers get a unique id derived from their name to avoid merging.
      id: isCustom && customProviderName.trim() ? `custom_${customProviderName.trim().toLowerCase().replace(/[\s\-_]+/g, "_")}` : template.id,
      name: providerName,
      protocol,
      protocols,
      base_url: defaultBaseUrl,
      base_urls: effectiveBaseUrls,
      models: customModels.split(",").map((item) => item.trim()).filter(Boolean).length
        ? customModels.split(",").map((item) => item.trim()).filter(Boolean)
        : template.models,
      tags: resourceTags,
      credential_mode: method === "oauth" ? undefined : method,
    };

    if (method === "apikey") {
      // Collect all keys: from multi-key editor first, fallback to single apiKey field.
      const allKeys = keyEntries.length > 0
        ? keyEntries.map((e) => ({ name: e.name.trim(), value: e.value.trim() })).filter((e) => e.value.length > 0)
        : apiKey.trim() ? [{ name: resourceName.trim(), value: apiKey.trim() }] : [];
      if (allKeys.length === 0) return null;
      const effectiveBaseUrl = defaultBaseUrl;
      const models = customModels
        .split(",")
        .map((m) => m.trim())
        .filter(Boolean);
      // Generate one import content per key; the backend will create one account per key.
      const contents = allKeys.map((entry, i) => {
        const accountName = entry.name || resourceName.trim() || (allKeys.length > 1 ? `${providerName} #${i + 1}` : `${providerName} resource`);
        return JSON.stringify({
          name: accountName,
          provider_name: providerName,
          provider: providerHint.id,
          protocol,
          base_url: effectiveBaseUrl,
          base_urls: effectiveBaseUrls,
          protocols,
          api_key: entry.value,
          models: models.length ? models : template.models,
          tags: resourceTags,
        });
      });
      // For single key, send as single content; for multiple keys, join with newlines.
      const content = contents.join("\n");
      return { content, source_name: "model-resource.json", provider_hint: providerHint };
    }

    if (method === "token") {
      if (!tokenContent.trim()) return null;
      return { content: tokenContent, source_name: "pasted.json", provider_hint: providerHint };
    }

    if (method === "batch") {
      if (paths.length === 0) return null;
      return { paths, provider_hint: providerHint };
    }

    return null;
  }, [
    mode,
    discovered,
    selectedScanFingerprints,
    template,
    method,
    apiKey,
    baseUrls,
    customModels,
    selectedProtocols,
    tokenContent,
    paths,
    keyEntries,
    customProviderName,
    resourceName,
  ]);

  const canPreview = request !== null;

  const scanSelectionOptions = useMemo(() => {
    if (mode !== "scan") return {};
    return {
      selected_fingerprints: Array.from(selectedScanFingerprints),
      selected_models: Object.fromEntries(
        (discovered?.accounts || [])
          .filter(
            (account) =>
              selectedScanFingerprints.has(account.fingerprint) && account.models.length > 0,
          )
          .map((account) => [
            account.fingerprint,
            account.models.filter((model) => {
              const resource = discovered?.model_resources.find(
                (item) =>
                  item.provider_name.trim().toLowerCase() ===
                    account.provider_name.trim().toLowerCase() &&
                  item.model.trim().toLowerCase() === model.trim().toLowerCase(),
              );
              return resource ? selectedScanModelKeys.has(resource.key) : false;
            }),
          ]),
      ),
    };
  }, [mode, discovered, selectedScanFingerprints, selectedScanModelKeys]);

  const handleCheck = async () => {
    if (!request) {
      setError("请先填写凭证内容或选择文件");
      return;
    }
    // Validate custom provider name before checking
    if (template?.group === "custom" && method === "apikey") {
      if (!customProviderName.trim()) {
        setError("自定义供应商需要填写供应商名称");
        return;
      }
      if (isGenericProviderName(customProviderName)) {
        setError('自定义供应商名称不能使用 "custom"、"自定义" 等通用名，请填写独立名称（如中转站名称）');
        return;
      }
    }
    setError(null);
    try {
      const result = await previewAndCheckImport.mutateAsync({
        request,
        options: {
          auto_create_providers: true,
          skip_duplicates: true,
          import_adapter_required: true,
          on_conflict: conflictMode,
          ...scanSelectionOptions,
        },
      });
      setCheckResult(result);
      // Keep every account the user selected during local scanning. OAuth
      // accounts may be intentionally marked adapter_required and cannot pass a
      // generic HTTP health check, but they still need to be persisted so the
      // dedicated adapter can use them later.
      setSelectedFingerprints(
        mode === "scan"
          ? new Set(
              result.accounts
                .filter((account) => selectedScanFingerprints.has(account.fingerprint))
                .map((account) => account.fingerprint),
            )
          : new Set(
              result.accounts
                .filter((account) => account.health_status === "healthy")
                .map((account) => account.fingerprint),
            ),
      );
    } catch (err) {
      setError(`检测失败: ${importErrorMessage(String(err))}`);
    }
  };

  const handleImportChecked = async (enableRouting: boolean) => {
    if (!request || !checkResult) return;
    if (selectedFingerprints.size === 0) {
      setError("请至少选择一个账号");
      return;
    }
    setError(null);
    try {
      const defaultTags = batchTags
        .split(/[,\s]+/)
        .map((t) => t.trim())
        .filter(Boolean);
      const result = await executeImport.mutateAsync({
        request,
        options: {
          auto_create_providers: true,
          skip_duplicates: true,
          import_adapter_required: true,
          selected_fingerprints: Array.from(selectedFingerprints),
          selected_models: mode === "scan" ? scanSelectionOptions.selected_models : undefined,
          default_tags: defaultTags.length > 0 ? defaultTags : undefined,
          on_conflict: conflictMode,
        },
      });
      const updateNote =
        result.updated > 0 ? `，更新 ${result.updated} 个已有账号` : "";
      const note = result.adapter_required
        ? `，${result.adapter_required} 个账号等待上游适配器`
        : "";
      toast("success", `已导入 ${result.imported} 个账号${updateNote}${note}`);
      if (enableRouting && result.account_ids.length > 0) {
        try {
          await batchUpdateAccounts.mutateAsync({
            ids: result.account_ids,
            status: "active",
          });
        } catch {
          toast("warning", "导入成功，但启用路由失败，请手动启用");
        }
      }
      if (result.errors.length === 0) onClose();
    } catch (err) {
      setError(`导入失败: ${importErrorMessage(String(err))}`);
    }
  };

  const handleDirectImport = async () => {
    if (!request) return;
    // Validate custom provider name
    if (template?.group === "custom" && method === "apikey") {
      if (!customProviderName.trim()) {
        setError("自定义供应商需要填写供应商名称");
        return;
      }
      if (isGenericProviderName(customProviderName)) {
        setError('自定义供应商名称不能使用通用名，请填写独立名称');
        return;
      }
    }
    setError(null);
    try {
      const result = await executeImport.mutateAsync({
        request,
        options: {
          auto_create_providers: true,
          skip_duplicates: true,
          import_adapter_required: true,
          on_conflict: conflictMode,
          ...scanSelectionOptions,
        },
      });
      const updateNote =
        result.updated > 0 ? `，更新 ${result.updated} 个已有账号` : "";
      const note = result.adapter_required
        ? `，${result.adapter_required} 个账号等待上游适配器`
        : "";
      toast("success", `已导入 ${result.imported} 个账号${updateNote}${note}`);
      if (result.errors.length === 0) onClose();
    } catch (err) {
      setError(`导入失败: ${importErrorMessage(String(err))}`);
    }
  };

  const toggleFingerprint = (fingerprint: string) => {
    setSelectedFingerprints((prev) => {
      const next = new Set(prev);
      if (next.has(fingerprint)) next.delete(fingerprint);
      else next.add(fingerprint);
      return next;
    });
  };

  const handleScan = async () => {
    setError(null);
    try {
      const result = await scanConfigs.mutateAsync();
      setDiscovered(result);
      setSelectedScanFingerprints(
        new Set(result.accounts.map((account) => account.fingerprint)),
      );
      setSelectedScanModelKeys(
        new Set(result.model_resources.map((resource) => resource.key)),
      );
      if (result.accounts.length === 0 && result.model_resources.length === 0) {
        toast("info", "未发现 Cockpit Tools、Codex Tools 或 EchoBird 账号");
      } else {
        toast(
          "success",
          `发现 ${result.accounts.length} 个去重账号、${result.model_resources.length} 个模型供应商`,
        );
      }
    } catch (err) {
      setError(`扫描失败: ${importErrorMessage(String(err))}`);
    }
  };

  const toggleScanAccount = (account: DiscoveredAccount) => {
    setSelectedScanFingerprints((prev) => {
      const next = new Set(prev);
      if (next.has(account.fingerprint)) next.delete(account.fingerprint);
      else next.add(account.fingerprint);
      return next;
    });
  };

  const toggleScanModelResource = (resource: DiscoveredModelResource) => {
    const enable = !selectedScanModelKeys.has(resource.key);
    setSelectedScanModelKeys((prev) => {
      const next = new Set(prev);
      if (enable) next.add(resource.key);
      else next.delete(resource.key);
      return next;
    });
    // A model resource needs at least one credential-bearing account. Selecting
    // a resource therefore also selects every deduplicated account that can
    // serve it; deselecting the resource leaves account choices unchanged.
    if (enable) {
      setSelectedScanFingerprints((prev) => {
        const next = new Set(prev);
        for (const fingerprint of resource.account_fingerprints) next.add(fingerprint);
        return next;
      });
    }
  };

  const chooseFiles = async () => {
    const selection = await open({
      multiple: true,
      directory: false,
      filters: [
        { name: "账号凭证", extensions: ["json", "csv", "txt"] },
        { name: "全部文件", extensions: ["*"] },
      ],
      title: "选择账号文件",
    });
    const selected = Array.isArray(selection) ? selection : selection ? [selection] : [];
    if (selected.length) {
      setPaths(selected);
      previewAndCheckImport.reset();
    }
  };

  const copyLink = async (link: string) => {
    try {
      await navigator.clipboard.writeText(link);
      toast("success", "授权链接已复制");
    } catch {
      toast("warning", "复制失败，请手动选择");
    }
  };

  const handleStartOAuth = async () => {
    if (!template) return;
    setError(null);
    try {
      const result = await startOAuth.mutateAsync({
        providerId: template.id,
        emailHint: pendingEmail.trim() || undefined,
        note: pendingNote.trim() || undefined,
      });
      setOauthLoginId(result.login_id);
      setOauthAuthorizationUrl(result.authorization_url);
      toast("success", "已打开系统浏览器，请完成授权");
    } catch (err) {
      setError(`发起授权失败: ${String(err)}`);
    }
  };

  const handleCompleteOAuth = async () => {
    if (!oauthLoginId) {
      setError("请先发起浏览器授权");
      return;
    }
    setError(null);
    try {
      await completeOAuth.mutateAsync({
        loginId: oauthLoginId,
        callbackUrl: callbackUrl.trim() || undefined,
      });
      toast("success", "OAuth 账号已接入");
      onClose();
    } catch (err) {
      setError(`完成授权失败: ${String(err)}`);
    }
  };

  const handleCancelOAuth = async () => {
    if (oauthLoginId) await cancelOAuth.mutateAsync(oauthLoginId);
    setOauthLoginId("");
    setOauthAuthorizationUrl("");
    setCallbackUrl("");
  };

  const resetFormFields = () => {
    setApiKey("");
    setBaseUrls({});
    setCustomModels("");
    setSelectedProtocols([]);
    setResourceName("");
    setTokenContent("");
    setPaths([]);
    setPendingEmail("");
    setPendingNote("");
    setCallbackUrl("");
    setOauthLoginId("");
    setOauthAuthorizationUrl("");
    setShowFormatHint(false);
    setKeyEntries([]);
    setCustomProviderName("");
  };

  // Filtered templates based on search; grouped by `group` for display.
  const filteredTemplates = useMemo(() => {
    const q = providerSearch.trim().toLowerCase();
    if (!q) return modelResourceTemplates;
    return modelResourceTemplates.filter(
      (t) => t.name.toLowerCase().includes(q) || t.description.toLowerCase().includes(q),
    );
  }, [providerSearch]);

  // Templates flattened into a single continuous list, ordered by group.
  const orderedTemplates = useMemo(() => {
    const rank = new Map(providerGroupOrder.map((g, i) => [g, i] as const));
    return [...filteredTemplates].sort(
      (a, b) => (rank.get(a.group) ?? 99) - (rank.get(b.group) ?? 99),
    );
  }, [filteredTemplates]);

  // Group templates by provider group for sectioned display.
  const groupedTemplates = useMemo(() => {
    const groups: Array<{ group: ProviderGroup; label: string; items: typeof orderedTemplates }> = [];
    const map = new Map<ProviderGroup, typeof orderedTemplates>();
    for (const item of orderedTemplates) {
      const existing = map.get(item.group) || [];
      existing.push(item);
      map.set(item.group, existing);
    }
    for (const group of providerGroupOrder) {
      const items = map.get(group);
      if (items && items.length > 0) {
        groups.push({ group, label: providerGroupLabels[group] || group, items });
      }
    }
    return groups;
  }, [orderedTemplates]);

  return (
    <div className="flex flex-col h-full min-h-0">
      {/* ═══════════════════════════════════════════════════════════════════
          MAIN VIEW: Provider Selection Grid
          Shows when no template is selected and not in scan mode
          ═══════════════════════════════════════════════════════════════════ */}
      {!template && mode === "provider" ? (
        <div className="flex-1 flex flex-col min-h-0">
          {/* ─── Header with quick actions ─── */}
          <div className="shrink-0 px-5 pt-4 pb-3 border-b" style={{ background: "var(--bg-elevated)", borderColor: "var(--border-subtle)" }}>
            <div className="flex items-center justify-between mb-3">
              <div>
                <h2 className="text-[15px] font-semibold text-[var(--text-primary)]">选择供应商</h2>
                <p className="text-[11px] text-[var(--text-dim)] mt-0.5">从预设供应商中选择，或使用快捷操作</p>
              </div>
              <div className="flex items-center gap-2">
                {/* Scan button */}
                <button
                  onClick={() => { setMode("scan"); setCheckResult(null); previewAndCheckImport.reset(); }}
                  className="flex items-center gap-1.5 h-8 px-3 rounded-lg border text-[12px] font-medium transition-all cursor-pointer hover:bg-emerald-500/10 hover:border-emerald-500/30 hover:text-emerald-500"
                  style={{ borderColor: "var(--border-default)", color: "var(--text-secondary)" }}
                >
                  <Radar size={14} />
                  扫描本机配置
                </button>
                {/* Manual button */}
                <button
                  onClick={() => { setSelectedTemplateId("custom"); resetFormFields(); }}
                  className="flex items-center gap-1.5 h-8 px-3 rounded-lg border text-[12px] font-medium transition-all cursor-pointer hover:bg-blue-500/10 hover:border-blue-500/30 hover:text-blue-500"
                  style={{ borderColor: "var(--border-default)", color: "var(--text-secondary)" }}
                >
                  <Plus size={14} />
                  手动配置
                </button>
              </div>
            </div>
            {/* Search */}
            <div className="relative">
              <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--text-dim)]" />
              <input
                value={providerSearch}
                onChange={(e) => setProviderSearch(e.target.value)}
                placeholder="搜索供应商名称..."
                className="w-full h-9 rounded-lg pl-9 pr-3 text-[13px] outline-none border bg-[var(--bg-surface)] text-[var(--text-primary)] placeholder:text-[var(--text-dim)] focus:ring-2 focus:ring-[var(--color-brand)]/25 focus:border-[var(--color-brand)] transition-all"
                style={{ borderColor: "var(--border-default)" }}
              />
            </div>
          </div>

          {/* ─── Scrollable provider grid ─── */}
          <div className="flex-1 overflow-y-auto px-5 py-4">
            {filteredTemplates.length === 0 ? (
              <div className="text-center py-12 text-[13px] text-[var(--text-dim)]">
                <Search size={22} className="mx-auto mb-2 opacity-30" />
                未找到匹配的供应商
              </div>
            ) : (
              <div className="space-y-5">
                {groupedTemplates.map((section) => (
                  <div key={section.group}>
                    <div className="flex items-center gap-2.5 mb-2.5">
                      <span className="text-[11px] font-semibold text-[var(--text-dim)] tracking-wide uppercase">
                        {section.label}
                      </span>
                      <span className="text-[10px] text-[var(--text-dim)] pg-mono">{section.items.length}</span>
                      <div className="flex-1 h-px bg-[var(--border-subtle)]" />
                    </div>
                    <div className="grid gap-2" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(232px, 1fr))" }}>
                      {section.items.map((item) => {
                        const brand = getProviderBrand(item.id, item.name);
                        const isCustom = item.group === "custom";
                        return (
                          <button
                            key={item.id}
                            onClick={() => {
                              setSelectedTemplateId(item.id);
                              resetFormFields();
                            }}
                            title={`${item.name} · ${item.description}`}
                            className={`group relative flex items-center gap-2 rounded-[10px] border pl-2 pr-6 py-2 text-left transition-all cursor-pointer ${
                              isCustom
                                ? "text-white shadow-sm"
                                : "hover:border-[var(--border-strong)] hover:bg-[var(--bg-elevated)] hover:shadow-[var(--shadow-card)]"
                            }`}
                            style={
                              isCustom
                                ? { background: "var(--color-brand)", borderColor: "var(--color-brand)" }
                                : { borderColor: "var(--border-subtle)", background: "var(--bg-surface)" }
                            }
                          >
                            <span
                              className="shrink-0 w-7 h-7 rounded-lg grid place-items-center text-[11px] font-bold leading-none tracking-tight shadow-sm"
                              style={
                                isCustom
                                  ? { background: "rgba(255,255,255,0.22)", color: "#fff" }
                                  : { background: brand.color, color: brand.fg || "#fff" }
                              }
                            >
                              {isCustom ? <Plus size={15} /> : brand.mono}
                            </span>
                            <span
                              className={`min-w-0 flex-1 text-[13px] font-medium whitespace-nowrap ${
                                isCustom ? "text-white" : "text-[var(--text-primary)]"
                              }`}
                            >
                              {item.name}
                            </span>
                            {!isCustom &&
                              (item.recommended ? (
                                <Heart
                                  size={12}
                                  className="absolute top-1.5 right-1.5 text-[var(--color-warn)] fill-[var(--color-warn)]"
                                />
                              ) : (
                                <Star
                                  size={11}
                                  className="absolute top-1.5 right-1.5 text-[var(--color-warn)] fill-[var(--color-warn)]"
                                />
                              ))}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                ))}
              </div>
            )}
            <div className="mt-5 flex items-center gap-1.5 text-[11px] text-[var(--text-dim)]">
              <Lightbulb size={12} className="text-[var(--color-warn)]" />
              自定义配置需手动填写所有必要字段
            </div>
          </div>
        </div>
      ) : mode === "scan" ? (
        /* ═══════════════════════════════════════════════════════════════════
            SCAN MODE: Auto-discover configs
            ═══════════════════════════════════════════════════════════════════ */
        <>
          {/* Back button for scan mode */}
          <div className="shrink-0 px-5 py-2.5 border-b" style={{ background: "var(--bg-elevated)", borderColor: "var(--border-subtle)" }}>
            <button
              onClick={() => { setMode("provider"); setDiscovered(null); setSelectedScanFingerprints(new Set()); setSelectedScanModelKeys(new Set()); }}
              className="flex items-center gap-1.5 text-[12px] text-[var(--text-dim)] hover:text-[var(--color-brand)] transition-colors cursor-pointer"
            >
              <ChevronDown size={14} className="rotate-90" /> 返回供应商选择
            </button>
          </div>
          <ScanConfigView
            discovered={discovered}
            selectedFingerprints={selectedScanFingerprints}
            selectedModelKeys={selectedScanModelKeys}
            existingFingerprints={existingFingerprints}
            pending={scanConfigs.isPending}
            onToggleAccount={toggleScanAccount}
            onToggleModelResource={toggleScanModelResource}
            onScan={handleScan}
            onClear={() => {
              setDiscovered(null);
              setSelectedScanFingerprints(new Set());
              setSelectedScanModelKeys(new Set());
              previewAndCheckImport.reset();
            }}
          />
        </>
      ) : template ? (
        /* ═══════════════════════════════════════════════════════════════════
            CREDENTIAL FORM: Step 2 - Fill in credentials
            Shows when a template is selected
            ═══════════════════════════════════════════════════════════════════ */
        <>
          {/* ─── Compact Header: Back + Vendor + Progress + Methods ─── */}
          <div className="shrink-0 border-b" style={{ borderColor: "var(--border-subtle)", background: "var(--bg-elevated)" }}>
            {/* Top row */}
            <div className="flex items-center gap-3 px-5 py-3">
              {/* Back button */}
              <button
                onClick={() => { setSelectedTemplateId(""); resetFormFields(); }}
                className="flex items-center justify-center w-8 h-8 rounded-lg border transition-all cursor-pointer hover:bg-[var(--bg-hover)] hover:border-[var(--border-strong)]"
                style={{ borderColor: "var(--border-default)", color: "var(--text-secondary)" }}
                title="返回供应商选择"
              >
                <ChevronDown size={16} className="rotate-90" />
              </button>

              {/* Vendor info */}
              <div className="flex items-center gap-3 flex-1 min-w-0">
                {(() => {
                  const brand = getProviderBrand(template.id, template.name);
                  return (
                    <span
                      className="shrink-0 w-9 h-9 rounded-lg grid place-items-center text-[12px] font-bold leading-none shadow-sm"
                      style={{ background: brand.color, color: brand.fg || "#fff" }}
                    >
                      {brand.mono}
                    </span>
                  );
                })()}
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-[15px] font-semibold tracking-[-0.01em] text-[var(--text-primary)] truncate">
                      {template.name}
                    </span>
                    <Badge variant={template.group === "custom" ? "mute" : "brand"} className="text-[10px]">
                      {providerGroupLabels[template.group]}
                    </Badge>
                    {template.recommended && <Badge variant="ok" className="text-[10px]">推荐</Badge>}
                    {template.codingPlan && <Badge variant="warn" className="text-[10px]">Coding Plan</Badge>}
                  </div>
                  <div className="text-[11px] text-[var(--text-dim)] flex items-center gap-1.5 mt-0.5 truncate">
                    <span className="pg-mono truncate">
                      {selectedProtocols.length > 0
                        ? selectedProtocols.map(protocolLabel).join(" · ")
                        : protocolsLabel(template) || "选择 API 格式"}
                    </span>
                    {template.models.length > 0 && (
                      <>
                        <span className="text-[var(--border-default)]">·</span>
                        <span>{template.models.length} 个模型</span>
                      </>
                    )}
                  </div>
                </div>
              </div>

              {/* Compact step indicator */}
              <div className="flex items-center gap-1 shrink-0">
                {([
                  { n: 2, label: "填写凭证", active: !checkResult },
                  { n: 3, label: "检测确认", active: !!checkResult },
                ] as const).map((step, i) => (
                  <div key={step.n} className="flex items-center gap-1">
                    <div className={`flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-medium transition-colors ${
                      step.active
                        ? "bg-[var(--color-brand)]/10 text-[var(--color-brand)]"
                        : "text-[var(--text-dim)]"
                    }`}>
                      <span className={`w-4 h-4 rounded-full flex items-center justify-center text-[9px] font-bold ${
                        step.active
                          ? "bg-[var(--color-brand)] text-white"
                          : "bg-[var(--bg-hover)]"
                      }`}>
                        {step.n}
                      </span>
                    </div>
                    {i < 1 && <div className="w-3 h-px bg-[var(--border-subtle)]" />}
                  </div>
                ))}
              </div>
            </div>

            {/* Method selector row */}
            <div className="flex items-center gap-1.5 px-5 pb-2.5">
              {template.authMethods.map((m) => {
                const meta = methodMeta[m];
                const Icon = meta.icon;
                const active = method === m;
                return (
                  <button
                    key={m}
                    onClick={() => setMethod(m)}
                    title={meta.desc}
                    className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[12px] font-medium transition-all cursor-pointer ${
                      active
                        ? "bg-[var(--color-brand)] text-white shadow-sm shadow-[var(--color-brand)]/20"
                        : "text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
                    }`}
                  >
                    <Icon size={13} />
                    {meta.label}
                  </button>
                );
              })}
              <div className="flex-1" />
              <span className="text-[10px] text-[var(--text-dim)] pg-mono">
                {modelResourceCategoryLabels[resourceCategoryForTemplate(template, method)]}
              </span>
            </div>
          </div>

          {/* ─── Form content: single column layout ─── */}
          <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
            {/* Form area */}
            <div className="flex-1 overflow-y-auto px-6 py-5 relative min-w-0 max-w-3xl mx-auto w-full">
              {/* Loading bar */}
              {(previewAndCheckImport.isPending || executeImport.isPending) && (
                <div className="absolute inset-x-0 top-0 h-0.5 bg-[var(--color-brand)] opacity-80 animate-pulse" />
              )}

              {/* Adapter warning */}
              {template.codingPlan && (
                <div
                  className="mb-5 rounded-xl border px-5 py-4 text-[13px] leading-6 flex items-start gap-3"
                  style={{
                    borderColor: "var(--color-warn)",
                    background: "color-mix(in srgb, var(--color-warn) 6%, transparent)",
                    color: "var(--text-secondary)",
                  }}
                >
                  <TriangleAlert size={16} className="mt-0.5 shrink-0 text-[var(--color-warn)]" />
                  <span>
                    {template.oauth
                      ? `${template.name} 已支持浏览器 OAuth；授权后会安全保存账号并可查询额度。`
                      : `${template.name} 提供 Coding Plan 接入，可使用 Token & JSON 保存凭证。`}
                  </span>
                </div>
              )}

              {/* Method-specific forms */}
              {method === "oauth" && template && (
                <OauthForm
                  template={template}
                  email={pendingEmail}
                  setEmail={setPendingEmail}
                  note={pendingNote}
                  setNote={setPendingNote}
                  callbackUrl={callbackUrl}
                  setCallbackUrl={setCallbackUrl}
                  loginId={oauthLoginId}
                  authorizationUrl={oauthAuthorizationUrl}
                  supported={!!template.oauth}
                  pending={startOAuth.isPending || completeOAuth.isPending}
                  onStart={handleStartOAuth}
                  onComplete={handleCompleteOAuth}
                  onCancel={handleCancelOAuth}
                  onCopy={() => copyLink(oauthAuthorizationUrl)}
                />
              )}

              {method === "token" && (
                <TokenForm
                  content={tokenContent}
                  setContent={setTokenContent}
                  showHint={showFormatHint}
                  setShowHint={setShowFormatHint}
                />
              )}

              {method === "apikey" && template && (
                <ApiKeyForm
                  template={template}
                  apiKey={apiKey}
                  setApiKey={setApiKey}
                  resourceName={resourceName}
                  setResourceName={setResourceName}
                  baseUrls={baseUrls}
                  setBaseUrls={setBaseUrls}
                  customModels={customModels}
                  setCustomModels={setCustomModels}
                  selectedProtocols={selectedProtocols}
                  setSelectedProtocols={setSelectedProtocols}
                  keyEntries={keyEntries}
                  setKeyEntries={setKeyEntries}
                  customProviderName={customProviderName}
                  setCustomProviderName={setCustomProviderName}
                />
              )}

              {method === "batch" && (
                <BatchForm
                  paths={paths}
                  onPick={chooseFiles}
                  onClear={() => {
                    setPaths([]);
                    previewAndCheckImport.reset();
                  }}
                />
              )}

              {/* Error message */}
              {error && (
                <div
                  className="mt-5 rounded-xl border px-5 py-4 text-[13px] leading-6 flex items-start gap-3"
                  style={{
                    borderColor: "var(--color-err)",
                    background: "color-mix(in srgb, var(--color-err) 6%, transparent)",
                    color: "var(--color-err)",
                  }}
                >
                  <TriangleAlert size={16} className="mt-0.5 shrink-0" />
                  <span>{error}</span>
                </div>
              )}

              {/* Loading indicator */}
              {previewAndCheckImport.isPending && (
                <div className="mt-5 flex items-center gap-2.5 text-[13px] text-[var(--text-dim)]">
                  <Spinner size={15} />
                  <span>解析中...</span>
                </div>
              )}
            </div>

            {/* Check result panel - slides up when available */}
            {checkResult && !previewAndCheckImport.isPending && (
              <div
                className="shrink-0 border-t max-h-[40vh] overflow-y-auto animate-slide-up"
                style={{ borderColor: "var(--border-subtle)", background: "var(--bg-elevated)" }}
              >
                <CheckResultPanel
                  result={checkResult}
                  selected={selectedFingerprints}
                  onToggle={toggleFingerprint}
                  onSelectAll={() => setSelectedFingerprints(new Set(checkResult.accounts.map((a) => a.fingerprint)))}
                  onSelectHealthy={() =>
                    setSelectedFingerprints(
                      new Set(
                        checkResult.accounts
                          .filter((a) => a.health_status === "healthy")
                          .map((a) => a.fingerprint),
                      ),
                    )
                  }
                  onClearSelection={() => setSelectedFingerprints(new Set())}
                />
              </div>
            )}
          </div>
        </>
      ) : null}

      {/* ─── Footer ─── */}
      <div
        className="shrink-0 px-5 py-2.5 border-t flex items-center justify-between gap-3"
        style={{ borderColor: "var(--border-subtle)", background: "var(--bg-elevated)" }}
      >
        <div className="flex items-center gap-3 min-w-0 flex-wrap">
          <label className="flex items-center gap-2 text-[11px] text-[var(--text-secondary)] cursor-pointer select-none shrink-0">
            <Toggle checked={checkBeforeImport} onChange={setCheckBeforeImport} />
            导入前检测
          </label>
          <div className="w-px h-4 bg-[var(--border-subtle)] shrink-0" />
          <div className="flex items-center gap-1 shrink-0">
            <span className="text-[10px] text-[var(--text-dim)] mr-0.5">冲突</span>
            {conflictOptions.map((opt) => {
              const active = conflictMode === opt.value;
              return (
                <button
                  key={opt.value}
                  onClick={() => setConflictMode(opt.value)}
                  title={opt.title}
                  className={`rounded-md px-1.5 py-0.5 text-[10px] font-medium transition-all cursor-pointer ${
                    active
                      ? "bg-[var(--color-brand)] text-white"
                      : "text-[var(--text-secondary)] hover:bg-[var(--bg-hover)]"
                  }`}
                >
                  {opt.label}
                </button>
              );
            })}
          </div>
          {checkResult && (
            <>
              <div className="w-px h-4 bg-[var(--border-subtle)] shrink-0" />
              <div className="flex items-center gap-1.5">
                <Tags size={12} className="text-[var(--text-dim)] shrink-0" />
                <input
                  value={batchTags}
                  onChange={(e) => setBatchTags(e.target.value)}
                  placeholder="批量标签，逗号分隔"
                  className="h-7 w-[160px] rounded-md border px-2 text-[10px] outline-none bg-[var(--bg-surface)] text-[var(--text-primary)] placeholder:text-[var(--text-dim)]"
                  style={{ borderColor: "var(--border-default)" }}
                />
              </div>
            </>
          )}
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <Button variant="ghost" size="sm" onClick={onClose}>关闭</Button>
          {/* Provider mode: check or direct import */}
          {mode === "provider" && template && method !== "oauth" && !checkResult && checkBeforeImport && (
            <Button
              size="sm"
              onClick={handleCheck}
              loading={previewAndCheckImport.isPending}
              disabled={!canPreview}
            >
              <ShieldCheck size={13} /> 检测账号
            </Button>
          )}
          {mode === "provider" && template && method !== "oauth" && !checkResult && !checkBeforeImport && (
            <Button
              size="sm"
              onClick={handleDirectImport}
              loading={executeImport.isPending}
              disabled={!canPreview}
            >
              <Upload size={13} /> 确认导入
            </Button>
          )}
          {/* Scan mode: check or direct sync */}
          {mode === "scan" && !checkResult && checkBeforeImport && (
            <Button
              size="sm"
              onClick={handleCheck}
              loading={previewAndCheckImport.isPending}
              disabled={!canPreview}
            >
              <ShieldCheck size={13} /> 检测账号
            </Button>
          )}
          {mode === "scan" && !checkResult && !checkBeforeImport && (
            <Button
              size="sm"
              onClick={handleDirectImport}
              loading={executeImport.isPending}
              disabled={!canPreview}
            >
              <Upload size={13} /> 同步已选账号
            </Button>
          )}
          {/* Post-check: import actions */}
          {checkResult && (
            <>
              <Button
                variant="outline"
                size="sm"
                onClick={() => handleImportChecked(false)}
                loading={executeImport.isPending}
                disabled={selectedFingerprints.size === 0}
              >
                <Upload size={13} /> 仅导入
              </Button>
              <Button
                size="sm"
                onClick={() => handleImportChecked(true)}
                loading={executeImport.isPending}
                disabled={selectedFingerprints.size === 0}
              >
                <CheckCircle2 size={13} /> 导入并启用路由
              </Button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/* Step indicator helpers removed — stepper is now inline */

/* ---------- Field label ---------- */

function FieldLabel({ children, required }: { children: ReactNode; required?: boolean }) {
  return (
    <div className="text-[12px] font-medium text-[var(--text-secondary)]">
      {children}
      {required && <span className="text-[var(--color-err)] ml-0.5">*</span>}
    </div>
  );
}

/* ---------- Toggle ---------- */

function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={`relative w-9 h-5 rounded-full transition-colors ${checked ? "bg-[var(--color-brand)]" : "bg-slate-300"}`}
    >
      <span
        className={`absolute top-0.5 w-4 h-4 rounded-full bg-white transition-transform ${checked ? "translate-x-[18px]" : "translate-x-0.5"}`}
      />
    </button>
  );
}

/* ---------- Info note ---------- */

function InfoNote({ tone, children }: { tone: "info" | "warn"; children: ReactNode }) {
  const palette =
    tone === "warn"
      ? {
          border: "var(--color-warn)",
          bg: "color-mix(in srgb, var(--color-warn) 10%, transparent)",
          color: "var(--text-secondary)",
        }
      : {
          border: "var(--color-brand)",
          bg: "color-mix(in srgb, var(--color-brand) 8%, transparent)",
          color: "var(--text-secondary)",
        };
  return (
    <div
      className="rounded-lg border px-3 py-2.5 text-[11px] leading-5"
      style={{ borderColor: palette.border, background: palette.bg, color: palette.color }}
    >
      {children}
    </div>
  );
}

/* ---------- Section heading (replaces nested cards) ---------- */

function SectionHeading({ icon: Icon, title, hint }: { icon: typeof KeyRound; title: string; hint?: string }) {
  return (
    <div className="flex items-center gap-2 mb-3">
      <Icon size={14} className="text-[var(--text-dim)]" />
      <span className="text-[13px] font-semibold text-[var(--text-primary)]">{title}</span>
      {hint && <span className="ml-auto text-[11px] text-[var(--text-dim)]">{hint}</span>}
    </div>
  );
}

/* ============================== */
/*   Method-specific forms        */
/* ============================== */

/* ---------- Model list section (used in two-column layout) ---------- */

function ModelListSection(props: {
  template: ModelResourceTemplate;
  customModels: string;
  setCustomModels: (v: string) => void;
  effectiveBaseUrl: string;
}) {
  const { template } = props;
  const current = props.customModels.split(/[,\n]+/).map((s) => s.trim()).filter(Boolean);
  const fetchModels = useFetchUpstreamModels();
  const { toast } = useToast();

  const handleFetchModels = async () => {
    if (!props.effectiveBaseUrl) {
      toast("warning", "请先填写 Base URL");
      return;
    }
    try {
      const discoveryUrl = template.modelDiscovery?.url || props.effectiveBaseUrl;
      const discoveryProtocol = template.modelDiscovery?.protocol || primaryProtocol(template) || "chat";
      const models = await fetchModels.mutateAsync({
        baseUrl: discoveryUrl,
        apiKey: undefined,
        protocol: discoveryProtocol,
      });
      if (models.length === 0) {
        toast("warning", "上游未返回任何模型");
        return;
      }
      props.setCustomModels(models.join(", "));
      toast("success", `已获取 ${models.length} 个模型`);
    } catch (err) {
      toast("error", `获取模型失败: ${String(err)}`);
    }
  };

  const appendModel = (model: string) => {
    if (!current.includes(model)) {
      props.setCustomModels([...current, model].join(", "));
    } else {
      props.setCustomModels(current.filter((c) => c !== model).join(", "));
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-info)]" />
        <span className="text-[13px] font-semibold text-[var(--text-primary)]">模型列表</span>
        <span className="ml-auto text-[11px] text-[var(--text-dim)] pg-mono">{current.length} 个</span>
      </div>

      {template.models.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {template.models.map((m) => {
            const selected = current.includes(m);
            return (
              <button
                key={m}
                onClick={() => appendModel(m)}
                className={`rounded-lg border px-3 py-1.5 text-[11px] font-medium transition-colors cursor-pointer ${
                  selected
                    ? "border-[var(--color-brand)] bg-[var(--color-brand-subtle)] text-[var(--color-brand)]"
                    : "text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:border-[var(--border-strong)]"
                }`}
                style={{ borderColor: selected ? "var(--color-brand)" : "var(--border-subtle)" }}
              >
                {selected ? "✓ " : "+ "}{m}
              </button>
            );
          })}
        </div>
      )}

      <div
        className="rounded-lg border overflow-hidden"
        style={{ borderColor: "var(--border-default)", background: "var(--bg-surface)" }}
      >
        <textarea
          value={current.join("\n")}
          onChange={(e) => {
            const value = e.target.value
              .split(/[,\n]+/)
              .map((s) => s.trim())
              .filter(Boolean)
              .join(", ");
            props.setCustomModels(value);
          }}
          spellCheck={false}
          placeholder="model-a\nmodel-b"
          className="w-full min-h-[140px] max-h-[240px] resize-y bg-transparent p-4 text-[12px] font-mono outline-none"
          style={{ color: "var(--text-primary)" }}
        />
        <div
          className="px-4 py-3 flex items-center justify-between border-t"
          style={{ borderColor: "var(--border-subtle)", background: "var(--bg-elevated)" }}
        >
          <span className="text-[11px] text-[var(--text-dim)]">
            上游结果仅填入草稿，可在保存前调整。
          </span>
          <Button
            size="sm"
            variant="outline"
            onClick={handleFetchModels}
            loading={fetchModels.isPending}
            disabled={!props.effectiveBaseUrl}
          >
            <RefreshCw size={12} /> 从上游获取
          </Button>
        </div>
      </div>
    </div>
  );
}

/* ---------- API Key form ---------- */

function ApiKeyForm(props: {
  template: ModelResourceTemplate;
  apiKey: string;
  setApiKey: (v: string) => void;
  resourceName: string;
  setResourceName: (v: string) => void;
  baseUrls: Partial<Record<Protocol, string>>;
  setBaseUrls: (v: Partial<Record<Protocol, string>>) => void;
  customModels: string;
  setCustomModels: (v: string) => void;
  selectedProtocols: Protocol[];
  setSelectedProtocols: (v: Protocol[]) => void;
  keyEntries: Array<{ id: string; name: string; value: string; reveal: boolean }>;
  setKeyEntries: (v: Array<{ id: string; name: string; value: string; reveal: boolean }>) => void;
  customProviderName: string;
  setCustomProviderName: (v: string) => void;
}) {
  const { template } = props;
  const isCustom = template.group === "custom";
  const selectedProtocol = props.selectedProtocols[0] || primaryProtocol(template);
  const defaultBaseUrl = baseUrlForProtocol(template, selectedProtocol);
  const fetchModels = useFetchUpstreamModels();
  const { toast } = useToast();

  // Raw URL for display; normalization happens at request time.
  const effectiveBaseUrl = props.baseUrls[selectedProtocol]?.trim() || defaultBaseUrl || "";

  // Determine effective keys: use multi-key editor if entries exist, otherwise single key.
  const effectiveKeys = props.keyEntries.length > 0
    ? props.keyEntries.map((e) => e.value.trim()).filter(Boolean)
    : props.apiKey.trim() ? [props.apiKey.trim()] : [];
  const hasMultipleKeys = props.keyEntries.length > 0;

  return (
    <div className="space-y-6">
      {/* ── Section 1: Provider Identity ── */}
      <div
        className="rounded-xl border p-5 space-y-4"
        style={{ borderColor: "var(--border-subtle)", background: "var(--bg-surface)" }}
      >
        <div className="flex items-center gap-2">
          <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-brand)]" />
          <span className="text-[12px] font-semibold text-[var(--text-primary)]">供应商信息</span>
        </div>

        {isCustom && (
          <div>
            <FieldLabel required>供应商名称</FieldLabel>
            <div className="mt-2">
              <Input
                value={props.customProviderName}
                onChange={(e) => props.setCustomProviderName(e.target.value)}
                placeholder="例如：小七中转站、Agent Router"
              />
            </div>
            <div className="mt-1.5 text-[11px] text-[var(--text-dim)] leading-4">
              自定义供应商需要填写独立名称，多个自定义供应商不能共用通用名。
            </div>
            {props.customProviderName.trim() && isGenericProviderName(props.customProviderName) && (
              <div className="mt-1.5 text-[11px] text-[var(--color-err)]">
                请使用独立名称，通用名会导致多个自定义供应商被合并。
              </div>
            )}
          </div>
        )}

        <div>
          <FieldLabel>资源名称</FieldLabel>
          <div className="mt-2">
            <Input
              value={props.resourceName}
              onChange={(e) => props.setResourceName(e.target.value)}
              placeholder={isCustom ? (props.customProviderName.trim() || "我的中转站") + " resource" : `${template.name} resource`}
            />
          </div>
          <div className="mt-1.5 text-[11px] text-[var(--text-dim)]">
            留空时使用默认资源名，导入后可修改。
          </div>
        </div>
      </div>

      {/* ── Section 2: Credentials ── */}
      <div
        className="rounded-xl border p-5 space-y-4"
        style={{ borderColor: "var(--border-subtle)", background: "var(--bg-surface)" }}
      >
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-ok)]" />
            <span className="text-[12px] font-semibold text-[var(--text-primary)]">
              凭证{props.keyEntries.length > 0 ? ` · ${props.keyEntries.length} 个 Key` : ""}
            </span>
          </div>
          <button
            type="button"
            className="text-[11px] text-[var(--color-brand)] hover:underline cursor-pointer"
            onClick={() => {
              if (props.keyEntries.length === 0 && props.apiKey.trim()) {
                props.setKeyEntries([createKeyEntry(props.apiKey.trim(), props.resourceName.trim())]);
                props.setApiKey("");
              } else {
                props.setKeyEntries([...props.keyEntries, createKeyEntry()]);
              }
            }}
          >
            {props.keyEntries.length === 0 ? "添加多个 Key" : "+ 添加 Key"}
          </button>
        </div>

        {props.keyEntries.length === 0 ? (
          <div>
            <Input
              value={props.apiKey}
              onChange={(e) => props.setApiKey(e.target.value)}
              placeholder="sk-..."
              className="font-mono"
            />
            <div className="mt-2 text-[11px] text-[var(--text-dim)] leading-4">
              每个 Key 生成一个独立账号，支持轮询调度。点击上方「添加多个 Key」可批量添加。
            </div>
          </div>
        ) : (
          <div className="space-y-3">
            {props.keyEntries.map((entry, i) => (
              <div key={entry.id} className="flex items-start gap-2.5">
                <span className="mt-2 w-6 shrink-0 text-center text-[11px] tabular-nums text-[var(--text-dim)]">{i + 1}</span>
                <div className="flex-1 space-y-2">
                  <input
                    type="text"
                    className="h-9 w-full rounded-lg border px-3 text-[12px] outline-none transition-colors focus:ring-2 focus:ring-[var(--color-brand)]/40"
                    style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-default)", color: "var(--text-primary)" }}
                    value={entry.name}
                    onChange={(e) => props.setKeyEntries(props.keyEntries.map((x) => x.id === entry.id ? { ...x, name: e.target.value } : x))}
                    placeholder="名称（可选，如：工作账号）"
                  />
                  <div className="flex items-center gap-2">
                    <input
                      type={entry.reveal ? "text" : "password"}
                      className="h-9 flex-1 rounded-lg border px-3 font-mono text-[12px] outline-none transition-colors focus:ring-2 focus:ring-[var(--color-brand)]/40"
                      style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-default)", color: "var(--text-primary)" }}
                      value={entry.value}
                      onChange={(e) => props.setKeyEntries(props.keyEntries.map((x) => x.id === entry.id ? { ...x, value: e.target.value } : x))}
                      placeholder="sk-..."
                    />
                    <button
                      type="button"
                      onClick={() => props.setKeyEntries(props.keyEntries.map((x) => x.id === entry.id ? { ...x, reveal: !x.reveal } : x))}
                      className="p-2 rounded-lg transition-colors text-[var(--text-dim)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)] cursor-pointer"
                      title={entry.reveal ? "隐藏 Key" : "明文查看"}
                    >
                      {entry.reveal ? <EyeOff size={15} /> : <KeyRound size={15} />}
                    </button>
                  </div>
                </div>
                <button
                  type="button"
                  onClick={() => props.setKeyEntries(props.keyEntries.filter((x) => x.id !== entry.id))}
                  className="mt-2 p-2 rounded-lg transition-colors text-[var(--text-dim)] hover:text-[var(--color-err)] hover:bg-[var(--color-err-bg)] cursor-pointer"
                  title="删除该 Key"
                >
                  <X size={15} />
                </button>
              </div>
            ))}
            <div className="text-[11px] text-[var(--text-dim)]">
              每个 Key 生成一个独立账号，支持轮询调度。
            </div>
          </div>
        )}
      </div>

      {/* ── Section 3: Connection ── */}
      <div
        className="rounded-xl border p-5 space-y-4"
        style={{ borderColor: "var(--border-subtle)", background: "var(--bg-surface)" }}
      >
        <div className="flex items-center gap-2">
          <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-info)]" />
          <span className="text-[12px] font-semibold text-[var(--text-primary)]">连接配置</span>
        </div>

        {/* Protocol selector */}
        <div>
          <FieldLabel required>API 格式</FieldLabel>
          <div className="mt-2 flex items-center gap-2 flex-wrap">
            {protocolOptions.map((opt) => {
              const supported = template.protocols.length === 0 || template.protocols.includes(opt.value);
              const active = props.selectedProtocols.includes(opt.value);
              return (
                <button
                  key={opt.value}
                  disabled={!supported}
                  title={supported ? `启用 ${opt.label}` : `${template.name} 模板未声明支持该协议`}
                  onClick={() => {
                    if (!supported) return;
                    const next = active
                      ? props.selectedProtocols.filter((item) => item !== opt.value)
                      : [...props.selectedProtocols, opt.value];
                    if (next.length === 0) return;
                    props.setSelectedProtocols(next);
                    if (!active && !props.baseUrls[opt.value]) {
                      props.setBaseUrls({
                        ...props.baseUrls,
                        [opt.value]: baseUrlForProtocol(template, opt.value),
                      });
                    }
                  }}
                  className={`rounded-lg border px-3.5 py-2 text-[12px] font-medium transition-all cursor-pointer ${
                    active
                      ? "bg-[var(--color-brand)] text-white border-[var(--color-brand)]"
                      : supported
                        ? "text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] border-[var(--border-subtle)]"
                        : "text-[var(--text-dim)] border-[var(--border-subtle)] opacity-35 cursor-not-allowed"
                  }`}
                >
                  {active ? "✓ " : "+ "}{opt.label}
                </button>
              );
            })}
          </div>
          <div className="mt-2 text-[11px] text-[var(--text-dim)] leading-4">
            可同时启用多个协议；同一 API Key 只接入一次，客户端调用时自动选择对应上游地址。
          </div>
        </div>

        {/* Base URLs */}
        <div className="space-y-3">
          <FieldLabel required>协议 Base URL</FieldLabel>
          {template.protocols.length === 0 ? (
            <Input
              value={props.baseUrls[props.selectedProtocols[0]] || ""}
              onChange={(e) => {
                const url = e.target.value;
                const next = { ...props.baseUrls };
                for (const p of props.selectedProtocols) next[p] = url;
                props.setBaseUrls(next);
              }}
              placeholder="https://api.example.com"
              className="font-mono"
            />
          ) : (
            props.selectedProtocols.map((protocol) => (
              <div key={protocol} className="grid grid-cols-[160px_1fr] items-center gap-3">
                <div className="text-[11px] font-medium text-[var(--text-secondary)]">
                  {protocolLabel(protocol)}
                </div>
                <Input
                  value={props.baseUrls[protocol] || ""}
                  onChange={(e) =>
                    props.setBaseUrls({ ...props.baseUrls, [protocol]: e.target.value })
                  }
                  placeholder={baseUrlForProtocol(template, protocol) || "https://api.example.com"}
                  className="font-mono"
                />
              </div>
            ))
          )}
          <div className="text-[11px] text-[var(--text-dim)] leading-4">
            {template.protocols.length === 0
              ? "所有协议共用同一个 Base URL，网关会自动拼接各协议的请求路径。"
              : "已按厂商模板自动填写，每个协议地址都可以单独修改。"}
          </div>
        </div>

        <InfoNote tone="info">
          <span className="font-medium">Base URL 填写标准：</span>只填到协议挂载点（如{" "}
          <span className="pg-mono">https://api.deepseek.com</span>），网关会自动拼接{" "}
          <span className="pg-mono">/v1/chat/completions</span> 等路径。
        </InfoNote>
      </div>

      {/* ── Section 4: Models ── */}
      <div
        className="rounded-xl border p-5 space-y-4"
        style={{ borderColor: "var(--border-subtle)", background: "var(--bg-surface)" }}
      >
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-info)]" />
            <span className="text-[12px] font-semibold text-[var(--text-primary)]">模型列表</span>
            <span className="text-[11px] text-[var(--text-dim)] pg-mono">
              {props.customModels ? props.customModels.split(",").filter(Boolean).length : template.models.length} 个
            </span>
          </div>
          <button
            type="button"
            className="text-[11px] text-[var(--color-brand)] hover:underline cursor-pointer flex items-center gap-1"
            onClick={async () => {
              if (!effectiveBaseUrl) {
                toast("warning", "请先填写 Base URL");
                return;
              }
              try {
                const discoveryUrl = template.modelDiscovery?.url || effectiveBaseUrl;
                const discoveryProtocol = template.modelDiscovery?.protocol || primaryProtocol(template) || "chat";
                const models = await fetchModels.mutateAsync({ baseUrl: discoveryUrl, apiKey: undefined, protocol: discoveryProtocol });
                if (models.length > 0) {
                  props.setCustomModels(models.join(", "));
                  toast("success", `已获取 ${models.length} 个模型`);
                } else {
                  toast("warning", "上游未返回任何模型");
                }
              } catch (err) {
                toast("error", `获取模型失败: ${String(err)}`);
              }
            }}
          >
            <RefreshCw size={11} /> 获取模型
          </button>
        </div>

        {/* Quick select chips */}
        {template.models.length > 0 && (
          <div className="flex flex-wrap gap-2">
            {template.models.map((m) => {
              const current = props.customModels ? props.customModels.split(",").map((s) => s.trim()).filter(Boolean) : [];
              const selected = current.includes(m);
              return (
                <button
                  key={m}
                  type="button"
                  onClick={() => {
                    const next = selected
                      ? current.filter((c) => c !== m)
                      : [...current, m];
                    props.setCustomModels(next.join(", "));
                  }}
                  className={`rounded-lg border px-3 py-1.5 text-[11px] font-medium transition-colors cursor-pointer ${
                    selected
                      ? "border-[var(--color-brand)] bg-[var(--color-brand-subtle)] text-[var(--color-brand)]"
                      : "text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] border-[var(--border-subtle)]"
                  }`}
                >
                  {selected ? "✓ " : "+ "}{m}
                </button>
              );
            })}
          </div>
        )}

        {/* Model input */}
        <div>
          <textarea
            value={props.customModels ? props.customModels.split(",").map((s) => s.trim()).filter(Boolean).join("\n") : ""}
            onChange={(e) => {
              const value = e.target.value
                .split(/[\n,]+/)
                .map((s) => s.trim())
                .filter(Boolean)
                .join(", ");
              props.setCustomModels(value);
            }}
            spellCheck={false}
            placeholder="每行一个模型名称，或用逗号分隔&#10;model-a&#10;model-b"
            className="w-full min-h-[100px] max-h-[200px] resize-y rounded-lg border bg-[var(--bg-elevated)] p-3 text-[12px] font-mono outline-none focus:ring-2 focus:ring-[var(--color-brand)]/25"
            style={{ borderColor: "var(--border-default)", color: "var(--text-primary)" }}
          />
          <div className="mt-2 text-[11px] text-[var(--text-dim)]">
            留空使用模板默认模型列表，或手动输入要启用的模型名称。
          </div>
        </div>
      </div>
    </div>
  );
}

/* ---------- Token & JSON form ---------- */

function TokenForm(props: {
  content: string;
  setContent: (v: string) => void;
  showHint: boolean;
  setShowHint: (v: boolean) => void;
}) {
  return (
    <div
      className="rounded-xl border p-5 space-y-4"
      style={{ borderColor: "var(--border-subtle)", background: "var(--bg-surface)" }}
    >
      <div className="flex items-center gap-2">
        <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-brand)]" />
        <span className="text-[13px] font-semibold text-[var(--text-primary)]">凭证输入</span>
      </div>

      <div className="text-[12px] leading-5 text-[var(--text-secondary)]">
        粘贴 <span className="pg-mono text-[var(--text-primary)]">auth.json</span>、Sub2API JSON、accessToken、refresh_token 或其他账号 JSON。
      </div>

      {/* Format hint toggle */}
      <button
        onClick={() => props.setShowHint(!props.showHint)}
        className="w-full flex items-center justify-between rounded-lg border px-3.5 py-2.5 text-[12px] text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] transition-colors cursor-pointer"
        style={{ borderColor: "var(--border-subtle)" }}
      >
        <span className="flex items-center gap-2">
          <TriangleAlert size={13} className="text-[var(--color-warn)]" />
          字段格式与示例
        </span>
        <ChevronDown
          size={15}
          className={`transition-transform ${props.showHint ? "rotate-180" : ""}`}
        />
      </button>
      {props.showHint && (
        <div
          className="rounded-lg border p-4 text-[11px] font-mono text-[var(--text-secondary)] leading-6"
          style={{ borderColor: "var(--border-subtle)", background: "var(--bg-elevated)" }}
        >
          <div className="text-[var(--text-dim)]"># Codex auth.json</div>
          <div>{`{ "OPENAI_API_KEY": "sk-...", "tokens": { "access_token": "...", "refresh_token": "..." } }`}</div>
          <div className="mt-3 text-[var(--text-dim)]"># Sub2API / CPA / Cockpit 账号 JSON</div>
          <div>{`{ "type": "codex", "access_token": "...", "refresh_token": "...", "account_id": "..." }`}</div>
          <div className="mt-3 text-[var(--text-dim)]"># 仅 refresh_token</div>
          <div>rt_xxx...</div>
        </div>
      )}

      {/* Textarea */}
      <textarea
        value={props.content}
        onChange={(e) => props.setContent(e.target.value)}
        spellCheck={false}
        placeholder="在此粘贴 JSON 或 Token..."
          className="w-full h-[160px] resize-none rounded-lg border p-4 text-[12px] font-mono outline-none focus:ring-2 focus:ring-[var(--color-brand)]/30 transition-colors"
        style={{
          background: "var(--bg-elevated)",
          borderColor: "var(--border-default)",
          color: "var(--text-primary)",
        }}
      />
    </div>
  );
}

/* ---------- OAuth form ---------- */

function OauthForm(props: {
  template: ModelResourceTemplate;
  email: string;
  setEmail: (v: string) => void;
  note: string;
  setNote: (v: string) => void;
  callbackUrl: string;
  setCallbackUrl: (v: string) => void;
  loginId: string;
  authorizationUrl: string;
  supported: boolean;
  pending: boolean;
  onStart: () => void;
  onComplete: () => void;
  onCancel: () => void;
  onCopy: () => void;
}) {
  const { template } = props;
  return (
    <div className="space-y-5">
      <div
        className="rounded-xl border p-5 space-y-4"
        style={{ borderColor: "var(--border-subtle)", background: "var(--bg-surface)" }}
      >
        <div className="flex items-center gap-2">
          <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-brand)]" />
          <span className="text-[13px] font-semibold text-[var(--text-primary)]">OAuth 授权</span>
        </div>

        <div className="text-[12px] leading-5 text-[var(--text-secondary)]">
          通过浏览器 OAuth 授权获取 <strong className="text-[var(--text-primary)]">{template.name}</strong> 账号 Token。
          授权完成后将回调地址粘贴到下方。
        </div>

        {/* Two-column: Email + Note */}
        <div className="grid grid-cols-2 gap-4">
          <div>
            <FieldLabel required>账号邮箱</FieldLabel>
            <div className="mt-2">
              <Input
                value={props.email}
                onChange={(e) => props.setEmail(e.target.value)}
                placeholder="输入账号邮箱"
              />
            </div>
          </div>
          <div>
            <FieldLabel>备注（可选）</FieldLabel>
            <div className="mt-2">
              <Input
                value={props.note}
                onChange={(e) => props.setNote(e.target.value)}
                placeholder="便于区分账号"
              />
            </div>
          </div>
        </div>

        {/* Authorization link */}
        <div>
          <FieldLabel>授权链接</FieldLabel>
          <div className="mt-2 flex items-center gap-2">
            <code
              className="flex-1 truncate rounded-lg border px-3.5 py-2.5 text-[11px] font-mono text-[var(--text-secondary)]"
              style={{ borderColor: "var(--border-default)", background: "var(--bg-elevated)" }}
            >
              {props.authorizationUrl || "发起授权后生成一次性 PKCE 链接"}
            </code>
            <button
              onClick={props.onCopy}
              disabled={!props.authorizationUrl}
              className="w-10 h-10 rounded-lg border flex items-center justify-center text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-40 cursor-pointer"
              style={{ borderColor: "var(--border-default)" }}
              title="复制"
            >
              <Copy size={15} />
            </button>
          </div>
        </div>

        {/* Action buttons */}
        <div className="grid grid-cols-2 gap-3">
          <Button
            fullWidth
            onClick={props.loginId ? props.onComplete : props.onStart}
            loading={props.pending}
            disabled={!props.supported}
            title={props.supported ? undefined : "该厂商尚未配置真实 OAuth 适配器"}
          >
            <ExternalLink size={14} /> {props.loginId ? "等待/完成授权" : "浏览器授权"}
          </Button>
          <Button
            fullWidth
            variant="outline"
            onClick={props.onCancel}
            disabled={!props.loginId || props.pending}
          >
            <EyeOff size={14} /> 取消会话
          </Button>
        </div>

        {/* Callback URL */}
        <div>
          <FieldLabel>回调地址</FieldLabel>
          <div className="mt-2">
            <Input
              value={props.callbackUrl}
              onChange={(e) => props.setCallbackUrl(e.target.value)}
              placeholder="http://localhost:1455/auth/callback?code=..."
              className="font-mono"
            />
          </div>
        </div>
      </div>

      <InfoNote tone={props.supported ? "info" : "warn"}>
        {props.supported
          ? "支持 PKCE、state 校验、本地回调与手动粘贴回调地址。OAuth 凭证会保存在本机数据库，不返回前端。"
          : `${template.name} 的 OAuth 适配器尚未配置，请使用 Token & JSON 方式接入。`}
      </InfoNote>
    </div>
  );
}

/* ---------- Batch import form ---------- */

function BatchForm(props: {
  paths: string[];
  onPick: () => void;
  onClear: () => void;
}) {
  return (
    <div className="space-y-5">
      <div
        className="rounded-xl border p-5 space-y-4"
        style={{ borderColor: "var(--border-subtle)", background: "var(--bg-surface)" }}
      >
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-brand)]" />
            <span className="text-[13px] font-semibold text-[var(--text-primary)]">文件导入</span>
          </div>
          {props.paths.length > 0 && (
            <span className="text-[11px] text-[var(--text-dim)] pg-mono">{props.paths.length} 个文件</span>
          )}
        </div>

        <div className="text-[12px] leading-5 text-[var(--text-secondary)]">
          选择 JSON / CSV / TXT 文件批量导入。支持 Sub2API、CPA、Cockpit、Codex auth.json、API Key 文本混合格式。
        </div>

        <button
          onClick={props.paths.length ? props.onClear : props.onPick}
          className="w-full rounded-xl border-2 border-dashed bg-[var(--bg-elevated)] px-6 py-8 text-[13px] text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:border-[var(--color-brand)]/40 transition-all cursor-pointer"
          style={{ borderColor: "var(--border-default)" }}
        >
          {props.paths.length ? (
            <span className="flex items-center justify-center gap-2.5">
              <FolderOpen size={18} /> 已选 {props.paths.length} 个文件，点击重新选择
            </span>
          ) : (
            <span className="flex items-center justify-center gap-2.5">
              <FolderOpen size={18} /> 从本地文件导入
            </span>
          )}
        </button>

        {props.paths.length > 0 && (
          <div
            className="rounded-lg border p-3 text-[11px] font-mono text-[var(--text-secondary)] max-h-[120px] overflow-auto"
            style={{ borderColor: "var(--border-subtle)", background: "var(--bg-elevated)" }}
          >
            {props.paths.map((p) => (
              <div key={p} className="truncate py-0.5">
                {p}
              </div>
            ))}
          </div>
        )}
      </div>

      <InfoNote tone="info">
        系统在生成模型供应商前会先脱敏预览，并按 SHA-256 指纹去重。
      </InfoNote>
    </div>
  );
}

/* ---------- Config scan view ---------- */

function ScanConfigView(props: {
  discovered: AgentConfigScanResult | null;
  selectedFingerprints: Set<string>;
  selectedModelKeys: Set<string>;
  existingFingerprints: Set<string>;
  pending: boolean;
  onToggleAccount: (account: DiscoveredAccount) => void;
  onToggleModelResource: (resource: DiscoveredModelResource) => void;
  onScan: () => void;
  onClear: () => void;
}) {
  // Get models for selected accounts
  const selectedModels = useMemo(() => {
    if (!props.discovered) return [];
    const modelSet = new Set<string>();
    for (const account of props.discovered.accounts) {
      if (props.selectedFingerprints.has(account.fingerprint)) {
        account.models.forEach((m) => modelSet.add(m));
      }
    }
    return Array.from(modelSet).sort();
  }, [props.discovered, props.selectedFingerprints]);

  const selectedAccountCount = props.discovered?.accounts.filter((a) => props.selectedFingerprints.has(a.fingerprint)).length ?? 0;

  return (
    <div className="flex-1 overflow-y-auto px-5 py-4 relative">
      {props.pending && (
        <div className="absolute inset-x-0 top-0 h-0.5 bg-[var(--color-brand)] opacity-80 animate-pulse" />
      )}
      <SectionHeading
        icon={Radar}
        title="扫描本机账号与模型"
        hint={
          props.discovered
            ? `${props.discovered.accounts.length} 个账号 · ${props.discovered.model_resources.length} 个模型供应商`
            : undefined
        }
      />
      <div className="text-[11px] leading-5 text-[var(--text-secondary)] mb-3">
        自动查找 Cockpit Tools、Codex Tools / Codex CLI 和 EchoBird 的 API Key、OAuth
        账号与模型配置。扫描只读取本地文件；勾选账号后才会同步到 PoolGate，
        凭据会加密保存到系统钥匙串。
      </div>

      {!props.discovered ? (
        <div
          className="rounded-md border border-dashed p-6 text-center"
          style={{ borderColor: "var(--border-default)" }}
        >
          <Radar size={20} className="mx-auto mb-2 text-[var(--text-dim)]" />
          <div className="text-[11px] text-[var(--text-dim)]">
            尚未扫描。点击下方按钮查找本机的 Cockpit Tools、Codex Tools 和 EchoBird 配置。
          </div>
          <Button size="sm" className="mt-3" onClick={props.onScan} loading={props.pending}>
            <Radar size={13} /> 开始扫描
          </Button>
        </div>
      ) : (
        <>
          {/* Batch actions */}
          {props.discovered.accounts.length > 0 && (
            <div className="flex items-center gap-2 mb-3">
              <button
                onClick={() => {
                  const allFingerprints = props.discovered!.accounts
                    .filter((a) => !props.existingFingerprints.has(a.fingerprint))
                    .map((a) => a.fingerprint);
                  // Select all non-existing accounts
                  const next = new Set(props.selectedFingerprints);
                  allFingerprints.forEach((f) => next.add(f));
                  // Use a custom event or callback to update
                  props.onToggleAccount(props.discovered!.accounts[0]); // Trigger update via parent
                }}
                className="text-[10px] px-2 py-1 rounded-md border hover:bg-[var(--bg-hover)] transition-colors"
                style={{ borderColor: "var(--border-subtle)", color: "var(--text-secondary)" }}
              >
                全选
              </button>
              <button
                onClick={() => {
                  // Clear all selections
                  const next = new Set(props.selectedFingerprints);
                  next.clear();
                  props.onToggleAccount(props.discovered!.accounts[0]); // Trigger update via parent
                }}
                className="text-[10px] px-2 py-1 rounded-md border hover:bg-[var(--bg-hover)] transition-colors"
                style={{ borderColor: "var(--border-subtle)", color: "var(--text-secondary)" }}
              >
                取消全选
              </button>
              <span className="ml-auto text-[10px] text-[var(--text-dim)]">
                已选 {selectedAccountCount} 个账号
              </span>
            </div>
          )}

          <div className="grid grid-cols-[1fr_260px] gap-3">
            {/* Account List */}
            <ScanResultList
              title="账号列表"
              count={props.discovered.accounts.length}
              emptyText="未识别到可同步账号"
            >
              {props.discovered.accounts.map((account) => {
                const selected = props.selectedFingerprints.has(account.fingerprint);
                const existing = props.existingFingerprints.has(account.fingerprint);
                return (
                  <label
                    key={account.fingerprint}
                    className={`flex items-start gap-2.5 px-3 py-2 border-t first:border-t-0 ${
                      existing
                        ? "opacity-50 cursor-not-allowed bg-[var(--bg-inset)]"
                        : selected
                          ? "bg-[var(--color-brand-subtle)] cursor-pointer"
                          : "hover:bg-[var(--bg-hover)] cursor-pointer"
                    }`}
                    style={{ borderColor: "var(--border-subtle)" }}
                  >
                    <input
                      type="checkbox"
                      className="mt-0.5 accent-[var(--color-brand)]"
                      checked={selected}
                      disabled={existing}
                      onChange={() => props.onToggleAccount(account)}
                    />
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2 flex-wrap">
                        <span className="text-[11px] font-medium text-[var(--text-primary)]">
                          {account.name}
                        </span>
                        <Badge variant={account.routable ? "ok" : "warn"} dot>
                          {credentialLabels[account.credential_type] || account.credential_type}
                        </Badge>
                        {existing && (
                          <Badge variant="mute" dot>已加入</Badge>
                        )}
                      </div>
                      <div className="mt-0.5 text-[9px] text-[var(--text-dim)]">
                        {account.provider_name} · {account.email || account.masked_credential}
                      </div>
                      <div className="mt-1 flex flex-wrap gap-1">
                        {account.source_apps.map((app) => (
                          <span key={app} className="rounded bg-[var(--bg-elevated)] px-1.5 py-0.5 text-[9px] text-[var(--text-secondary)]">
                            来源：{app}
                          </span>
                        ))}
                      </div>
                      {account.warning && (
                        <div className="mt-1 text-[9px] text-[var(--color-warn)]">{account.warning}</div>
                      )}
                    </div>
                  </label>
                );
              })}
            </ScanResultList>

            {/* Selected Models Preview */}
            <div className="rounded-md border overflow-hidden bg-[var(--bg-surface)]" style={{ borderColor: "var(--border-default)" }}>
              <div className="flex items-center justify-between px-3 py-2 bg-[var(--bg-elevated)]">
                <span className="text-[11px] font-semibold text-[var(--text-primary)]">关联模型</span>
                <span className="text-[9px] text-[var(--text-dim)]">{selectedModels.length} 个模型</span>
              </div>
              <div className="max-h-[330px] overflow-y-auto p-3">
                {selectedModels.length > 0 ? (
                  <div className="flex flex-wrap gap-1.5">
                    {selectedModels.map((model) => (
                      <span
                        key={model}
                        className="inline-flex items-center rounded border bg-[var(--color-brand-subtle)] px-2 py-0.5 text-[10px] text-[var(--text-primary)]"
                        style={{ borderColor: "var(--color-brand)" }}
                      >
                        {model}
                      </span>
                    ))}
                  </div>
                ) : (
                  <div className="p-5 text-center text-[10px] text-[var(--text-dim)]">
                    选择账号后将显示关联的模型
                  </div>
                )}
              </div>
            </div>
          </div>

          {props.discovered.accounts.length === 0 && props.discovered.model_resources.length === 0 && (
            <div className="mt-3 rounded-md border border-dashed p-5 text-center text-[11px] text-[var(--text-dim)]" style={{ borderColor: "var(--border-default)" }}>
              未发现 Cockpit Tools、Codex Tools 或 EchoBird 账号，可切换「模板导入」手动添加。
            </div>
          )}
          <div className="mt-3 flex items-center gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={props.onScan}
              loading={props.pending}
            >
              <RefreshCw size={12} /> 重新扫描
            </Button>
            <Button size="sm" variant="ghost" onClick={props.onClear}>
              <X size={12} /> 清空结果
            </Button>
          </div>
        </>
      )}
    </div>
  );
}

function ScanResultList(props: {
  title: string;
  count: number;
  emptyText: string;
  children: ReactNode;
}) {
  return (
    <section className="rounded-md border overflow-hidden bg-[var(--bg-surface)]" style={{ borderColor: "var(--border-default)" }}>
      <div className="flex items-center justify-between px-3 py-2 bg-[var(--bg-elevated)]">
        <span className="text-[11px] font-semibold text-[var(--text-primary)]">{props.title}</span>
        <span className="text-[9px] text-[var(--text-dim)]">去重后 {props.count} 项</span>
      </div>
      <div className="max-h-[330px] overflow-y-auto">
        {props.count > 0 ? props.children : (
          <div className="p-5 text-center text-[10px] text-[var(--text-dim)]">{props.emptyText}</div>
        )}
      </div>
    </section>
  );
}

/* ---------- Check result panel ---------- */

const healthToneMap: Record<string, "ok" | "warn" | "err" | "mute"> = {
  healthy: "ok",
  failed: "err",
  timeout: "warn",
  error: "err",
  duplicate: "mute",
  adapter_required: "warn",
};

const healthLabelMap: Record<string, string> = {
  healthy: "正常",
  failed: "异常",
  timeout: "超时",
  error: "错误",
  duplicate: "已存在",
  adapter_required: "待适配",
};

function CheckResultPanel(props: {
  result: ImportCheckResult;
  selected: Set<string>;
  onToggle: (fingerprint: string) => void;
  onSelectAll: () => void;
  onSelectHealthy: () => void;
  onClearSelection: () => void;
}) {
  const { result, selected } = props;
  const abnormal = result.accounts.filter((a) =>
    ["failed", "timeout", "error"].includes(a.health_status),
  );

  return (
    <div className="px-4 py-3 flex flex-col min-h-0 h-full">
      {/* Header */}
      <div className="flex items-center justify-between mb-2 shrink-0">
        <div className="text-[12px] font-semibold text-[var(--text-primary)]">检测结果</div>
        <Badge variant="brand">{selected.size}/{result.accounts.length} 已选</Badge>
      </div>

      {/* Summary cards */}
      <div className="grid grid-cols-2 gap-1.5 mb-3 shrink-0">
        <SummaryCell label="可导入" value={result.summary.ready} tone="ok" />
        <SummaryCell label="异常" value={result.summary.abnormal} tone="err" />
        <SummaryCell label="已存在" value={result.summary.duplicates} tone="mute" />
        <SummaryCell label="待适配" value={result.summary.adapter_required} tone="warn" />
      </div>

      {/* Batch actions */}
      <div className="flex items-center gap-1.5 mb-2 shrink-0 flex-wrap">
        <button
          onClick={props.onSelectAll}
          className="text-[10px] px-2 py-1 rounded-md border hover:bg-[var(--bg-hover)] transition-colors"
          style={{ borderColor: "var(--border-subtle)", color: "var(--text-secondary)" }}
        >
          全选
        </button>
        <button
          onClick={props.onSelectHealthy}
          className="text-[10px] px-2 py-1 rounded-md border hover:bg-[var(--bg-hover)] transition-colors"
          style={{ borderColor: "var(--border-subtle)", color: "var(--text-secondary)" }}
        >
          仅正常
        </button>
        <button
          onClick={props.onClearSelection}
          className="text-[10px] px-2 py-1 rounded-md border hover:bg-[var(--bg-hover)] transition-colors"
          style={{ borderColor: "var(--border-subtle)", color: "var(--text-secondary)" }}
        >
          清除
        </button>
      </div>

      {/* Account list — scrollable to fill available space */}
      <div
        className="rounded-md border overflow-hidden flex-1 min-h-0"
        style={{ borderColor: "var(--border-subtle)" }}
      >
        <div className="overflow-y-auto h-full">
          {result.accounts.map((account) => (
            <CheckResultItem
              key={account.fingerprint}
              account={account}
              selected={selected.has(account.fingerprint)}
              onToggle={() => props.onToggle(account.fingerprint)}
            />
          ))}
        </div>
      </div>

      {/* Error examples */}
      {abnormal.length > 0 && (
        <div className="mt-2 text-[10px] leading-4 text-[var(--color-err)] shrink-0">
          <span className="font-medium">{abnormal[0].name}</span>
          <span className="ml-1 truncate">— {abnormal[0].health_message}</span>
        </div>
      )}
    </div>
  );
}

function CheckResultItem(props: {
  account: CheckedAccount;
  selected: boolean;
  onToggle: () => void;
}) {
  const { account } = props;
  const tone = healthToneMap[account.health_status] || "mute";
  const label = healthLabelMap[account.health_status] || account.health_status;
  const abnormal = ["failed", "timeout", "error"].includes(account.health_status);

  return (
    <div
      className={`flex items-start gap-2 px-3 py-2 border-t transition-colors ${
        props.selected ? "bg-[var(--color-brand-subtle)]" : "hover:bg-[var(--bg-hover)]"
      }`}
      style={{ borderColor: "var(--border-subtle)" }}
    >
      <div className="pt-0.5 shrink-0">
        <input
          type="checkbox"
          className="accent-[var(--color-brand)]"
          checked={props.selected}
          onChange={props.onToggle}
        />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[11px] font-medium text-[var(--text-primary)] truncate">
            {account.name}
          </span>
          <Badge variant={tone} dot>{label}</Badge>
        </div>
        <div className="mt-0.5 text-[9px] text-[var(--text-dim)] flex items-center gap-1 flex-wrap">
          <span>{account.email || account.masked_credential}</span>
          <span>·</span>
          <span>{formatLabels[account.source_format] || account.source_format}</span>
          <span>·</span>
          <span>{credentialLabels[account.credential_type] || account.credential_type}</span>
        </div>
        {abnormal && (
          <div className="mt-1 text-[9px] leading-4 text-[var(--color-err)] break-all">
            {account.health_message}
          </div>
        )}
      </div>
    </div>
  );
}

function SummaryCell({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone: "ok" | "warn" | "err" | "mute";
}) {
  const colors: Record<string, string> = {
    ok: "var(--color-ok)",
    warn: "var(--color-warn)",
    err: "var(--color-err)",
    mute: "var(--text-dim)",
  };
  return (
    <div
      className="rounded-md border px-2.5 py-1.5"
      style={{ borderColor: "var(--border-subtle)", background: "var(--bg-surface)" }}
    >
      <div className="text-[9px] text-[var(--text-dim)]">{label}</div>
      <div className="mt-0.5 text-[15px] font-semibold leading-tight" style={{ color: colors[tone] }}>
        {value}
      </div>
    </div>
  );
}
