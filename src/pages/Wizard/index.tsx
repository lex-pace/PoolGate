import React, { useCallback, useEffect, useState } from "react";
import { Card, CBody } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import { useToast } from "@/components/ui/Toast";
import { useProxyStatus, useStartProxy } from "@/hooks/use-tauri";
import * as api from "@/lib/tauri-commands";
import {
  Check, Copy, KeyRound, FolderKanban, Terminal, Play, Square,
  RotateCcw, ArrowLeft, ArrowRight, PartyPopper, Loader2,
} from "lucide-react";

interface Props {
  onComplete: () => void;
}

type ProviderType = "chat" | "anthropic" | "gemini";

const PRESETS: Record<ProviderType, {
  label: string;
  defaultBase: string;
  hint: string;
}> = {
  chat: {
    label: "OpenAI 兼容",
    defaultBase: "https://api.openai.com/v1",
    hint: "覆盖 OpenAI 官方、Azure、DeepSeek、以及各类中转站（sub2api / new-api 等）",
  },
  anthropic: {
    label: "Anthropic",
    defaultBase: "https://api.anthropic.com",
    hint: "Claude 官方 API Key",
  },
  gemini: {
    label: "Google Gemini",
    defaultBase: "https://generativelanguage.googleapis.com/v1beta",
    hint: "Google Gemini API Key",
  },
};

function poolProtocolFor(providerProtocol: string): string {
  const p = providerProtocol.trim().toLowerCase();
  if (p === "anthropic" || p === "messages" || p === "claude") return "anthropic";
  if (p === "gemini" || p === "google" || p === "google_gemini") return "gemini";
  if (p === "both" || p === "multi" || p === "unified") return "both";
  return "openai"; // chat / openai / responses / codex / custom …
}

function poolLabel(protocol: string): string {
  switch (poolProtocolFor(protocol)) {
    case "anthropic": return "Anthropic";
    case "gemini": return "Gemini";
    case "both": return "多协议";
    default: return "OpenAI";
  }
}

function configFor(pool: { protocol: string; rawKey: string }, origin: string): string {
  const openai = `export OPENAI_BASE_URL=${origin}/v1\nexport OPENAI_API_KEY=${pool.rawKey}`;
  const anthropic = `export ANTHROPIC_BASE_URL=${origin}\nexport ANTHROPIC_AUTH_TOKEN=${pool.rawKey}`;
  switch (poolProtocolFor(pool.protocol)) {
    case "anthropic": return anthropic;
    case "gemini": return `export GEMINI_BASE_URL=${origin}\nexport GEMINI_API_KEY=${pool.rawKey}`;
    case "both": return `${openai}\n${anthropic}`;
    default: return openai;
  }
}

export default function Wizard({ onComplete }: Props) {
  const { toast } = useToast();
  const { data: proxyStatus } = useProxyStatus();
  const startProxy = useStartProxy();

  const [step, setStep] = useState(1);

  // Step 1 — import
  const [providerType, setProviderType] = useState<ProviderType>("chat");
  const [providerName, setProviderName] = useState("");
  const [baseUrl, setBaseUrl] = useState(PRESETS.chat.defaultBase);
  const [modelsText, setModelsText] = useState("");
  const [keysText, setKeysText] = useState("");
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<api.ImportResult | null>(null);
  const [importError, setImportError] = useState("");

  // Step 2 — build pools
  const [building, setBuilding] = useState(false);
  const [buildError, setBuildError] = useState("");
  const [poolsBuilt, setPoolsBuilt] = useState(false);
  const [pools, setPools] = useState<Array<{ pool: api.AgentGroup; rawKey: string; resourceCount: number }>>([]);

  // Step 3 — copy
  const [copied, setCopied] = useState<string | null>(null);

  const origin = `http://127.0.0.1:${proxyStatus?.port || 9800}`;

  const switchProviderType = (type: ProviderType) => {
    const previousDefault = PRESETS[providerType].defaultBase;
    setProviderType(type);
    // 只有用户没改过 Base URL（仍是上一个类型的默认值）时才换用新类型的默认地址
    setBaseUrl((current) => (current === previousDefault ? PRESETS[type].defaultBase : current));
  };

  const handleImport = async () => {
    if (!keysText.trim()) { toast("warning", "请先粘贴 API Key（每行一个，支持「名称=密钥」）"); return; }
    if (!providerName.trim()) { toast("warning", "请填写服务商名称"); return; }
    if (!baseUrl.trim()) { toast("warning", "请填写 Base URL"); return; }
    setImporting(true);
    setImportError("");
    try {
      const protocol = providerType;
      const models = modelsText.split(/[,，]/).map((m) => m.trim()).filter(Boolean);
      const providerId = `wizard_${providerName.trim().toLowerCase().replace(/[\s\-_]+/g, "_")}`;
      const lines = keysText.split("\n").map((l) => l.trim()).filter((l) => l && !l.startsWith("#"));
      const contents = lines.map((line, i) => {
        const eq = line.indexOf("=");
        const name = eq > 0 ? line.slice(0, eq).trim() : "";
        const key = eq > 0 ? line.slice(eq + 1).trim() : line;
        return JSON.stringify({
          name: name || `${providerName.trim()} #${i + 1}`,
          provider_name: providerName.trim(),
          provider: providerId,
          protocol,
          base_url: baseUrl.trim(),
          base_urls: { [protocol]: baseUrl.trim() },
          protocols: [protocol],
          api_key: key,
          models,
          tags: ["wizard"],
        });
      });
      const result = await api.executeImport(
        {
          content: contents.join("\n"),
          source_name: "model-resource.json",
          provider_hint: {
            id: providerId,
            name: providerName.trim(),
            protocol,
            protocols: [protocol],
            base_url: baseUrl.trim(),
            base_urls: { [protocol]: baseUrl.trim() },
            models,
            tags: ["wizard"],
          },
        },
        {
          auto_create_providers: true,
          skip_duplicates: true,
          import_adapter_required: true,
          on_conflict: "skip",
        },
      );
      setImportResult(result);
      if (result.errors.length) {
        setImportError(result.errors.join("；"));
      }
      toast("success", `已导入 ${result.imported} 个账号${result.updated ? `，更新 ${result.updated} 个` : ""}`);
    } catch (e) {
      setImportError(String(e));
      toast("error", `导入失败: ${String(e)}`);
    } finally {
      setImporting(false);
    }
  };

  const buildPools = useCallback(async () => {
    setBuilding(true);
    setBuildError("");
    const created: Array<{ pool: api.AgentGroup; rawKey: string; resourceCount: number }> = [];
    try {
      // 1) 空模型的账号尝试从上游拉取模型（OpenAI 兼容 / Gemini 有 /v1/models；Anthropic 失败则提示手动填写）
      const accounts = await api.listAccounts();
      const noModelIds = accounts
        .filter((a) => !(a.models || "").trim() && a.id)
        .map((a) => a.id as string);
      if (noModelIds.length) {
        const results = await api.batchRefreshAccountModels(noModelIds);
        const ok = results.filter((r) => r.success).length;
        if (ok > 0) toast("info", `已从上游获取 ${ok} 个账号的模型`);
      }

      // 2) 按协议族分组，每个协议族建一个路由池
      const providers = await api.listProviders();
      const byProtocol = new Map<string, typeof providers>();
      for (const p of providers) {
        if (p.enabled === false) continue;
        const proto = poolProtocolFor(p.protocol || "chat");
        byProtocol.set(proto, [...(byProtocol.get(proto) || []), p]);
      }

      if (byProtocol.size === 0) {
        setBuildError("没有可用的服务商，无法创建路由池。请先回到第 1 步导入账号。");
        return;
      }

      for (const [proto, provs] of byProtocol) {
        const label = poolLabel(proto);
        const createdPool = await api.createGroup({
          id: crypto.randomUUID(),
          name: `默认池 · ${label}`,
          description: `首启向导自动创建（${provs.map((p) => p.name).join(" / ")}）`,
          protocol: proto,
          strategy: "round_robin",
          enabled: true,
        });
        // 3) 把该池兼容服务商的所有可用模型资源加入池
        const available = await api.listAvailableGroupModelResources(createdPool.group.id);
        const resources = available.map((r) => ({ provider_id: r.provider_id, model: r.model }));
        let resourceCount = 0;
        if (resources.length) {
          resourceCount = await api.addGroupModelResources(createdPool.group.id, resources);
        }
        created.push({ pool: createdPool.group, rawKey: createdPool.raw_key, resourceCount });
      }

      if (created.length === 0) {
        setBuildError("没有可用账号/模型，无法创建路由池。请到「模型供应商」页检查账号状态。");
        return;
      }
      setPools(created);
      setPoolsBuilt(true);
      toast("success", `已创建 ${created.length} 个路由池（含专属接入密钥）`);
    } catch (e) {
      setBuildError(String(e));
      toast("error", `创建路由池失败: ${String(e)}`);
    } finally {
      setBuilding(false);
    }
  }, [toast]);

  // 进入第 2 步且尚未建池时自动执行（一键），失败可重试
  useEffect(() => {
    if (step === 2 && !poolsBuilt && !building) {
      void buildPools();
    }
  }, [step, poolsBuilt, building, buildPools]);

  const handleCopy = async (tag: string, text: string) => {
    await navigator.clipboard.writeText(text);
    setCopied(tag);
    setTimeout(() => setCopied(null), 1600);
    toast("success", "已复制到剪贴板");
  };

  const finish = () => onComplete();

  const stepIndicator = (
    <div className="flex items-center justify-center gap-2">
      {[1, 2, 3].map((s) => {
        const done = s < step;
        const active = s === step;
        return (
          <div key={s} className="flex items-center gap-2">
            <div
              className="w-8 h-8 rounded-full flex items-center justify-center text-sm font-medium border transition-colors"
              style={{
                backgroundColor: done ? "var(--color-ok)" : active ? "var(--color-brand)" : "var(--bg-hover)",
                borderColor: done ? "var(--color-ok)" : active ? "var(--color-brand)" : "var(--border-default)",
                color: done || active ? "#fff" : "var(--text-dim)",
              }}
            >
              {done ? <Check size={14} /> : s}
            </div>
            {s < 3 && (
              <div className="w-12 h-0.5 rounded" style={{ backgroundColor: done ? "var(--color-ok)" : "var(--bg-hover)" }} />
            )}
          </div>
        );
      })}
    </div>
  );

  const nav = (
    <div className="flex items-center justify-between">
      <Button variant="ghost" size="sm" disabled={step === 1} onClick={() => setStep(step - 1)}>
        <ArrowLeft size={13} /> 上一步
      </Button>
      <div className="flex gap-2">
        <Button variant="ghost" size="sm" onClick={finish}>跳过</Button>
        {step < 3 ? (
          <Button size="sm" variant="primary" onClick={() => setStep(step + 1)}>
            下一步 <ArrowRight size={13} />
          </Button>
        ) : (
          <Button size="sm" variant="success" onClick={finish}>
            <PartyPopper size={14} /> 完成，开始使用
          </Button>
        )}
      </div>
    </div>
  );

  return (
    <div className="flex items-center justify-center min-h-full p-6">
      <div className="w-full max-w-2xl space-y-6">
        <div className="text-center space-y-1">
          <div className="flex items-center justify-center gap-2">
            <img src="/poolgate-icon.png" alt="PoolGate" className="w-9 h-9" />
            <h1 className="text-xl font-bold" style={{ color: "var(--text-primary)" }}>欢迎使用 PoolGate</h1>
          </div>
          <p className="text-xs" style={{ color: "var(--text-dim)" }}>三步让 Agent 工具走通你的账号池 —— 无需 Docker，一键启用</p>
        </div>

        {stepIndicator}

        {/* ── Step 1: 导入账号 ── */}
        {step === 1 && (
          <Card>
            <CBody className="space-y-4 pt-2">
              <div className="text-center">
                <h2 className="text-base font-bold" style={{ color: "var(--text-primary)" }}>
                  <KeyRound size={15} className="inline mr-1.5 -mt-0.5" style={{ color: "var(--color-brand)" }} />
                  导入账号
                </h2>
                <p className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>粘贴 API Key，自动创建服务商与账号</p>
              </div>

              <div>
                <div className="text-xs mb-1.5" style={{ color: "var(--text-dim)" }}>服务商类型</div>
                <div className="grid grid-cols-3 gap-2">
                  {(Object.keys(PRESETS) as ProviderType[]).map((type) => {
                    const preset = PRESETS[type];
                    const active = providerType === type;
                    return (
                      <button
                        key={type}
                        onClick={() => switchProviderType(type)}
                        className="px-2 py-2.5 rounded-lg border text-left transition-colors"
                        style={{
                          backgroundColor: active ? "var(--color-brand-subtle)" : "var(--bg-elevated)",
                          borderColor: active ? "var(--color-brand)" : "var(--border-default)",
                        }}
                      >
                        <div className="text-sm font-medium" style={{ color: active ? "var(--color-brand)" : "var(--text-primary)" }}>{preset.label}</div>
                        <div className="text-[10px] mt-0.5 leading-4" style={{ color: "var(--text-dim)" }}>{preset.hint}</div>
                      </button>
                    );
                  })}
                </div>
              </div>

              <Input label="服务商名称" value={providerName} onChange={(e) => setProviderName(e.target.value)} placeholder="如: 我的中转站 / OpenAI 官方" />
              <Input label="Base URL" value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} />
              <Input
                label="模型列表（可选，逗号分隔）"
                value={modelsText}
                onChange={(e) => setModelsText(e.target.value)}
                placeholder="如: gpt-4o, claude-sonnet-4-5（留空则下一步自动从上游获取）"
              />

              <div>
                <div className="text-xs mb-1.5" style={{ color: "var(--text-dim)" }}>API Key（每行一个，支持「名称=密钥」）</div>
                <textarea
                  value={keysText}
                  onChange={(e) => setKeysText(e.target.value)}
                  rows={5}
                  placeholder={"account1=sk-xxx\naccount2=sk-yyy"}
                  className="w-full px-3 py-2 rounded-md border text-sm font-mono outline-none resize-y"
                  style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-default)", color: "var(--text-primary)" }}
                />
              </div>

              <Button variant="primary" className="w-full" disabled={importing} onClick={handleImport}>
                {importing ? <><Loader2 size={14} className="animate-spin" /> 导入中...</> : "导入账号"}
              </Button>

              {importResult && (
                <div className="p-3 rounded-md border text-xs" style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-subtle)" }}>
                  <div className="flex items-center gap-2 font-medium" style={{ color: "var(--color-ok)" }}>
                    <Check size={13} /> 导入完成
                  </div>
                  <div className="mt-1" style={{ color: "var(--text-secondary)" }}>
                    成功 {importResult.imported} · 跳过 {importResult.skipped} · 重复 {importResult.duplicates}
                    {importResult.adapter_required ? ` · 待适配 ${importResult.adapter_required}` : ""}
                  </div>
                  {importError && <div className="mt-1" style={{ color: "var(--color-err)" }}>{importError}</div>}
                </div>
              )}
            </CBody>
          </Card>
        )}

        {/* ── Step 2: 建池 ── */}
        {step === 2 && (
          <Card>
            <CBody className="space-y-4 pt-2">
              <div className="text-center">
                <h2 className="text-base font-bold" style={{ color: "var(--text-primary)" }}>
                  <FolderKanban size={15} className="inline mr-1.5 -mt-0.5" style={{ color: "var(--color-brand)" }} />
                  创建路由池
                </h2>
                <p className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>按服务商协议自动建池，并生成每池专属接入密钥</p>
              </div>

              {building ? (
                <div className="flex flex-col items-center gap-3 py-8">
                  <Spinner size={26} />
                  <span className="text-xs" style={{ color: "var(--text-dim)" }}>正在拉取模型、创建路由池与专属密钥...</span>
                </div>
              ) : pools.length > 0 ? (
                <div className="space-y-2.5">
                  {pools.map((item) => (
                    <div key={item.pool.id} className="flex items-center gap-3 p-3 rounded-md border" style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-subtle)" }}>
                      <div className="w-9 h-9 grid place-items-center rounded-md" style={{ backgroundColor: "var(--color-brand-subtle)", color: "var(--color-brand)" }}>
                        <FolderKanban size={16} />
                      </div>
                      <div className="flex-1 min-w-0">
                        <div className="text-sm font-medium" style={{ color: "var(--text-primary)" }}>{item.pool.name}</div>
                        <div className="text-[11px]" style={{ color: "var(--text-dim)" }}>
                          {poolLabel(item.pool.protocol)} · {item.resourceCount} 个模型资源 · 专属密钥 {item.rawKey.slice(0, 12)}…
                        </div>
                      </div>
                      <Badge variant="ok" dot>已创建</Badge>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="p-4 rounded-md border text-center" style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-default)" }}>
                  <div className="text-sm" style={{ color: "var(--text-secondary)" }}>还没有可用的路由池</div>
                  <div className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>
                    {buildError || "请确认第 1 步已成功导入账号，或到「模型供应商」页补充账号。"}
                  </div>
                  <Button size="sm" variant="secondary" className="mt-3" onClick={() => void buildPools()}>
                    <RotateCcw size={13} /> 重新创建
                  </Button>
                </div>
              )}
            </CBody>
          </Card>
        )}

        {/* ── Step 3: 复制配置 ── */}
        {step === 3 && (
          <Card>
            <CBody className="space-y-4 pt-2">
              <div className="text-center">
                <h2 className="text-base font-bold" style={{ color: "var(--text-primary)" }}>
                  <Terminal size={15} className="inline mr-1.5 -mt-0.5" style={{ color: "var(--color-brand)" }} />
                  复制配置，开始使用
                </h2>
                <p className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>
                  所有工具统一连接 <span className="font-mono" style={{ color: "var(--color-brand)" }}>{origin}</span>
                </p>
              </div>

              {pools.length === 0 ? (
                <div className="p-4 rounded-md border text-center text-xs" style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-default)", color: "var(--text-dim)" }}>
                  尚未创建路由池，请回到第 2 步。也可以稍后在「路由池」页手动创建。
                </div>
              ) : (
                <div className="space-y-3">
                  {pools.map((item) => {
                    const tag = `pool-${item.pool.id}`;
                    const config = configFor({ protocol: item.pool.protocol, rawKey: item.rawKey }, origin);
                    return (
                      <div key={item.pool.id} className="p-3 rounded-md border" style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-subtle)" }}>
                        <div className="flex items-center justify-between mb-2">
                          <div className="text-sm font-medium" style={{ color: "var(--text-primary)" }}>{item.pool.name}</div>
                          <Badge variant="brand">{poolLabel(item.pool.protocol)}</Badge>
                        </div>
                        <pre className="p-2.5 rounded text-xs font-mono overflow-x-auto whitespace-pre-wrap" style={{ backgroundColor: "var(--bg-canvas)", color: "var(--text-secondary)" }}>
                          {config}
                        </pre>
                        <Button size="sm" variant="secondary" className="w-full mt-2" onClick={() => handleCopy(tag, config)}>
                          {copied === tag ? <><Check size={13} /> 已复制</> : <><Copy size={13} /> 复制配置（含专属密钥）</>}
                        </Button>
                      </div>
                    );
                  })}
                  <p className="text-[11px]" style={{ color: "var(--text-dim)" }}>
                    每个路由池自带一把专属接入密钥，已随配置复制；粘贴到 Claude Code / Codex / OpenCode 等工具的环境变量即可。
                  </p>
                </div>
              )}

              <div className="flex items-center justify-between p-3 rounded-md border" style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-subtle)" }}>
                <div className="flex items-center gap-2">
                  <span className={`status-dot ${proxyStatus?.running ? "ok pulse-ok" : "err"}`} />
                  <span className="text-sm" style={{ color: "var(--text-primary)" }}>
                    {proxyStatus?.running ? "网关运行中" : "网关未启动"}
                  </span>
                </div>
                <Button
                  size="sm"
                  variant={proxyStatus?.running ? "secondary" : "primary"}
                  disabled={startProxy.isPending}
                  onClick={() => void startProxy.mutateAsync()}
                >
                  {proxyStatus?.running ? <><Square size={11} fill="currentColor" /> 已启动</> : <><Play size={12} fill="currentColor" /> 启动网关</>}
                </Button>
              </div>
            </CBody>
          </Card>
        )}

        {nav}
      </div>
    </div>
  );
}
