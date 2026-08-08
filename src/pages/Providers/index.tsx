import React, { useState } from "react";
import {
  useProviders,
  useCreateProvider,
  useUpdateProvider,
  useDeleteProvider,
  useTestProviderConnection,
} from "@/hooks/use-tauri";
import type { Provider, ProviderTestResult } from "@/lib/tauri-commands";
import { Card, CBody as CardContent, CHeader as CardHeader, CTitle as CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { Modal } from "@/components/ui/Modal";
import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import { Spinner, PageSpinner } from "@/components/ui/Spinner";
import {
  Search,
  Plus,
  Edit3,
  Trash2,
  Power,
  PowerOff,
  Globe,
  Zap,
  CheckCircle,
  XCircle,
  Braces,
  List,
  KeyRound,
} from "lucide-react";

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
  "authorization",
  "proxy-authorization",
  "host",
  "content-length",
  "transfer-encoding",
  "connection",
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

export default function ProvidersPage() {
  const { data: providers, isLoading } = useProviders();
  const createProvider = useCreateProvider();
  const updateProvider = useUpdateProvider();
  const deleteProvider = useDeleteProvider();
  const testProviderConnection = useTestProviderConnection();

  const [search, setSearch] = useState("");
  const [filterType, setFilterType] = useState("");
  const [filterProtocol, setFilterProtocol] = useState("");
  const [modalOpen, setModalOpen] = useState(false);
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

  if (isLoading) return <PageSpinner />;

  const filtered = (providers ?? []).filter((p) => {
    if (search && !p.name.toLowerCase().includes(search.toLowerCase())) return false;
    if (filterType && p.type !== filterType) return false;
    if (filterProtocol && p.protocol !== filterProtocol) return false;
    return true;
  });

  const resetForm = () => {
    setForm(emptyForm);
    setFormError("");
    setHeaderEntries([]);
    setHeadersJsonMode(false);
    setEditing(null);
  };

  const openCreate = () => {
    resetForm();
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
    setForm({
      name: p.name,
      type: p.type,
      base_url: p.base_url,
      protocol: p.protocol,
      api_keys: p.api_keys || "",
      proxy_url: p.proxy_url || "",
      custom_headers: p.custom_headers || "",
      timeout_ms: p.timeout_ms || 30000,
      priority: p.priority ?? 0,
    });
    setHeaderEntries(parsedHeaders);
    setHeadersJsonMode(useJsonMode);
    setFormError("");
    setEditing(p);
    setModalOpen(true);
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

  const handleSave = async () => {
    setFormError("");
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
      const payload = { ...form, custom_headers: rawHeaders };
      if (editing) {
        // Preserve protocol collections and protocol-specific URL mappings that
        // are not yet exposed by this legacy maintenance form.
        await updateProvider.mutateAsync({ ...editing, ...payload, id: editing.id } as any);
      } else {
        await createProvider.mutateAsync(payload as any);
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
      // error_details holds the model reply on success
      const reply = result.error_details ? `\n回复: "${result.error_details}"` : "";
      return `✓ ${result.message}${result.model_tested ? ` · 模型: ${result.model_tested}` : ""}${reply}`;
    }
    return `✗ ${result.message}${result.error_details ? `\n${result.error_details}` : ""}`;
  };

  return (
    <div className="space-y-5 animate-fade-in pg-page">
      <div className="pg-page-header flex items-center justify-between">
        <div>
          <div className="pg-eyebrow mb-1">Upstream Providers</div>
          <h2 style={{ color: "var(--text-primary)" }}>服务商管理</h2>
          <p className="text-xs mt-0.5" style={{ color: "var(--text-dim)" }}>
            管理 API 服务商配置 · 共 {filtered.length} 个
          </p>
        </div>
        <Button onClick={openCreate}>
          <Plus size={16} /> 添加服务商
        </Button>
      </div>

      {/* Filters */}
      <div className="flex flex-wrap gap-3">
        <div className="relative flex-1 max-w-xs">
          <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2" style={{ color: "var(--text-dim)" }} />
          <input
            className="h-9 w-full rounded-md border pl-9 pr-3 text-sm outline-none transition-colors focus:ring-2 focus:ring-[var(--color-brand)]/40"
            style={{
              backgroundColor: "var(--bg-elevated)",
              borderColor: "var(--border-default)",
              color: "var(--text-primary)",
            }}
            placeholder="搜索服务商..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <Select
          options={[{ value: "", label: "全部类型" }, ...typeOptions]}
          value={filterType}
          onChange={(e) => setFilterType(e.target.value)}
          className="w-36"
        />
        <Select
          options={[{ value: "", label: "全部协议" }, ...protocolOptions]}
          value={filterProtocol}
          onChange={(e) => setFilterProtocol(e.target.value)}
          className="w-40"
        />
      </div>

      {/* Cards */}
      {filtered.length === 0 ? (
        <div className="text-center py-16" style={{ color: "var(--text-dim)" }}>
          <Globe size={48} className="mx-auto mb-3 opacity-40" />
          <p>暂无服务商</p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
          {filtered.map((p) => (
            <Card key={p.id} hover>
              <CardHeader>
                <div className="flex items-center gap-2">
                  <CardTitle className="text-base">{p.name}</CardTitle>
                  <Badge variant={typeBadge[p.type] || "mute"}>
                    {typeLabel[p.type] || p.type}
                  </Badge>
                </div>
                <div className="flex items-center gap-1">
                  <button
                    onClick={() => handleToggle(p)}
                    className={`p-1.5 rounded-md transition-colors cursor-pointer ${
                      p.enabled !== false
                        ? "text-[var(--color-ok)] hover:bg-[var(--color-ok-bg)]"
                        : "text-[var(--text-dim)] hover:bg-[var(--bg-hover)]"
                    }`}
                    title={p.enabled !== false ? "禁用" : "启用"}
                  >
                    {p.enabled !== false ? <Power size={14} /> : <PowerOff size={14} />}
                  </button>
                </div>
              </CardHeader>
              <CardContent>
                <div className="space-y-2 text-sm">
                  <div className="flex justify-between">
                    <span style={{ color: "var(--text-dim)" }}>协议</span>
                    <Badge variant="brand">{p.protocol}</Badge>
                  </div>
                  <div className="flex justify-between">
                    <span style={{ color: "var(--text-dim)" }}>Base URL</span>
                    <span className="truncate max-w-[180px]" style={{ color: "var(--text-primary)" }} title={p.base_url}>
                      {p.base_url}
                    </span>
                  </div>
                  <div className="flex justify-between">
                    <span style={{ color: "var(--text-dim)" }}>优先级</span>
                    <span style={{ color: "var(--text-primary)" }}>{p.priority ?? 0}</span>
                  </div>
                  {p.custom_headers && p.custom_headers !== "{}" && (
                    <div className="flex justify-between">
                      <span style={{ color: "var(--text-dim)" }}>自定义请求头</span>
                      <span style={{ color: "var(--text-primary)" }}>
                        {Object.keys((() => { try { return JSON.parse(p.custom_headers || "{}"); } catch { return {}; } })()).length} 项
                      </span>
                    </div>
                  )}
                  {p.timeout_ms && (
                    <div className="flex justify-between">
                      <span style={{ color: "var(--text-dim)" }}>超时</span>
                      <span style={{ color: "var(--text-primary)" }}>{p.timeout_ms}ms</span>
                    </div>
                  )}
                </div>
                <div className="flex items-center gap-2 mt-4 pt-3 border-t" style={{ borderColor: "var(--border-subtle)" }}>
                  <Button variant="ghost" size="sm" onClick={() => openEdit(p)}>
                    <Edit3 size={14} /> 编辑
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => handleTest(p)}
                    disabled={testingProviderId === p.id}
                    title={getTestResultTooltip(p.id)}
                  >
                    {testingProviderId === p.id ? (
                      <Spinner size={14} />
                    ) : (
                      getTestResultIcon(p.id) || <Zap size={14} />
                    )}
                    测试
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => handleDeleteClick(p)} className="text-[var(--color-err)] hover:text-[var(--color-err)]">
                    <Trash2 size={14} /> 删除
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      {/* Modal */}
      <Modal
        open={modalOpen}
        onClose={() => { setModalOpen(false); resetForm(); }}
        title={editing ? "编辑服务商" : "添加服务商"}
        className="max-w-2xl"
      >
        <div className="space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <Input
              label="名称"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder="我的服务商"
            />
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
          <Input
            label="API Keys (逗号分隔)"
            value={form.api_keys}
            onChange={(e) => setForm({ ...form, api_keys: e.target.value })}
            placeholder="sk-xxx, sk-yyy"
          />
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
                  title="按名称和值逐项编辑"
                >
                  <List size={12} /> 键值
                </button>
                <button
                  type="button"
                  className={`flex h-6 items-center gap-1 rounded px-2 text-[11px] transition-colors ${headersJsonMode ? "bg-[var(--bg-active)] text-[var(--text-primary)]" : "text-[var(--text-dim)] hover:text-[var(--text-primary)]"}`}
                  onClick={() => switchHeadersMode(true)}
                  title="批量编辑 JSON"
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
                          title="删除请求头"
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
    </div>
  );
}
