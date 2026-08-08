import React, { useState } from "react";
import { Card, CTitle, CBody } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { Spinner } from "@/components/ui/Spinner";
import { useProxyStatus, useGatewaySettings, useSetGatewayAccessKey, useSetCloseButtonBehavior } from "@/hooks/use-tauri";
import { useToast } from "@/components/ui/Toast";
import {
  Settings as SettingsIcon, Server, FileText, Wrench, Info, Terminal,
  Copy, Check, ExternalLink, KeyRound, Eye, EyeOff,
} from "lucide-react";

const shortcuts = [
  { keys: "⌘ K", desc: "全局搜索" },
  { keys: "⌘ 1-6", desc: "快速切换页面" },
  { keys: "⌘ ,", desc: "打开设置" },
  { keys: "⌘ R", desc: "刷新页面" },
  { keys: "Esc", desc: "关闭搜索/面板" },
];

const settingsTabs = [
  { id: "general", label: "通用", icon: SettingsIcon },
  { id: "proxy", label: "代理", icon: Server },
  { id: "logs", label: "日志 & 存储", icon: FileText },
  { id: "tools", label: "工具配置", icon: Wrench },
  { id: "about", label: "关于", icon: Info },
];

const toolConfigs = [
  {
    name: "Claude Code",
    description: "ANTHROPIC_BASE_URL 环境变量",
    config: "export ANTHROPIC_BASE_URL=http://127.0.0.1:9800",
  },
  {
    name: "Codex CLI",
    description: "OPENAI_BASE_URL 环境变量",
    config: "export OPENAI_BASE_URL=http://127.0.0.1:9800",
  },
  {
    name: "OpenCode",
    description: "JSON 配置文件格式",
    config: `{
  "providers": {
    "default": {
      "baseURL": "http://127.0.0.1:9800"
    }
  }
}`,
  },
];

export default function Settings() {
  const { data: proxy, isLoading } = useProxyStatus();
  const { data: gateway } = useGatewaySettings();
  const setAccessKey = useSetGatewayAccessKey();
  const setCloseButtonBehavior = useSetCloseButtonBehavior();
  const { toast } = useToast();
  const [activeTab, setActiveTab] = useState("general");
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);
  const [keyInput, setKeyInput] = useState("");
  const [keyDirty, setKeyDirty] = useState(false);
  const [showKey, setShowKey] = useState(false);

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-20">
        <Spinner size={24} />
        <span className="ml-2 text-sm" style={{ color: "var(--text-dim)" }}>加载中...</span>
      </div>
    );
  }

  const endpoint = `http://127.0.0.1:${proxy?.port || 9800}`;

  const handleCopy = async (config: string, index: number) => {
    await navigator.clipboard.writeText(config);
    setCopiedIndex(index);
    toast("success", "已复制到剪贴板");
    setTimeout(() => setCopiedIndex(null), 1500);
  };

  const handleSaveKey = async () => {
    try {
      await setAccessKey.mutateAsync(keyInput.trim());
      setKeyDirty(false);
      setKeyInput("");
      toast("success", "访问密钥已安全保存并立即生效");
    } catch (e) {
      toast("error", `保存失败: ${String(e)}`);
    }
  };

  const handleClearKey = async () => {
    try {
      await setAccessKey.mutateAsync("");
      setKeyDirty(false);
      setKeyInput("");
      toast("success", "访问密钥已清除并立即生效");
    } catch (e) {
      toast("error", `清除失败: ${String(e)}`);
    }
  };

  const handleCloseBehaviorChange = async (behavior: string) => {
    try {
      await setCloseButtonBehavior.mutateAsync(behavior);
      toast("success", `关闭按钮行为已设置为"${behavior === "hide" ? "隐藏到托盘" : "退出程序"}"`);
    } catch (e) {
      toast("error", `设置失败: ${String(e)}`);
    }
  };

  return (
    <div className="space-y-5 animate-fade-in max-w-4xl pg-page">
      <div className="pg-page-header">
        <div className="pg-eyebrow mb-1">System Configuration</div>
        <h2 style={{ color: "var(--text-primary)" }}>系统设置</h2>
        <p className="text-xs mt-0.5" style={{ color: "var(--text-dim)" }}>PoolGate 代理服务状态与系统信息</p>
      </div>

      {/* Tab navigation */}
      <div className="flex items-center gap-1 p-1 rounded-lg" style={{ backgroundColor: "var(--bg-elevated)" }}>
        {settingsTabs.map((tab) => {
          const Icon = tab.icon;
          return (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`flex items-center gap-2 px-3 py-2 text-sm rounded-md transition-all duration-150 cursor-pointer flex-1 justify-center ${
                activeTab === tab.id
                  ? "bg-[var(--bg-surface)] text-[var(--color-brand)] font-medium shadow-sm"
                  : "text-[var(--text-dim)] hover:text-[var(--text-secondary)]"
              }`}
            >
              <Icon size={14} />
              <span className="hidden sm:inline">{tab.label}</span>
            </button>
          );
        })}
      </div>

      {/* General Tab */}
      {activeTab === "general" && (
        <Card>
          <CTitle>通用设置</CTitle>
          <CBody className="space-y-4 mt-2">
            <div className="grid grid-cols-2 gap-4 text-sm">
              <div>
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>代理端口</div>
                <div className="mt-1 font-mono" style={{ color: "var(--text-primary)" }}>{proxy?.port || 9800}</div>
                <div className="text-xs mt-0.5" style={{ color: "var(--text-dim)" }}>修改后需重启</div>
              </div>
              <div>
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>开机自启</div>
                <div className="mt-1">
                  <Badge variant="ok" dot>已启用</Badge>
                </div>
              </div>
              <div className="col-span-2">
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>关闭按钮行为</div>
                <div className="mt-2 flex items-center gap-3">
                  <label className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="radio"
                      name="closeBehavior"
                      value="hide"
                      checked={gateway?.close_button_behavior === "hide"}
                      onChange={() => handleCloseBehaviorChange("hide")}
                      className="w-4 h-4"
                    />
                    <span className="text-sm" style={{ color: "var(--text-primary)" }}>隐藏到托盘</span>
                  </label>
                  <label className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="radio"
                      name="closeBehavior"
                      value="quit"
                      checked={gateway?.close_button_behavior === "quit"}
                      onChange={() => handleCloseBehaviorChange("quit")}
                      className="w-4 h-4"
                    />
                    <span className="text-sm" style={{ color: "var(--text-primary)" }}>退出程序</span>
                  </label>
                </div>
                <div className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>
                  选择点击窗口关闭按钮时的行为
                </div>
              </div>
            </div>
          </CBody>
        </Card>
      )}

      {/* Proxy Tab */}
      {activeTab === "proxy" && (
        <Card>
          <CTitle>代理服务器</CTitle>
          <CBody className="space-y-4 mt-2">
            <div className="grid grid-cols-2 gap-4 text-sm">
              <div>
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>运行状态</div>
                <div className="flex items-center gap-2 mt-1">
                  <Badge variant={proxy?.running ? "ok" : "err"} dot>{proxy?.running ? "运行中" : "已停止"}</Badge>
                </div>
              </div>
              <div>
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>监听端口</div>
                <div className="mt-1 font-mono" style={{ color: "var(--text-primary)" }}>{proxy?.port || "--"}</div>
              </div>
              <div>
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>活跃连接数</div>
                <div className="mt-1" style={{ color: "var(--text-primary)" }}>{proxy?.active_connections || 0}</div>
              </div>
              <div>
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>服务地址</div>
                <div className="flex items-center gap-2 mt-1">
                  <span className="font-mono text-sm" style={{ color: "var(--text-primary)" }}>{endpoint}</span>
                  <Button size="sm" variant="ghost" onClick={() => { navigator.clipboard.writeText(endpoint); toast("success", "已复制"); }}>
                    <Copy size={12} />
                  </Button>
                </div>
              </div>
            </div>

            {/* Gateway access key */}
            <div className="pt-4 mt-2 border-t" style={{ borderColor: "var(--border-subtle)" }}>
              <div className="flex items-center gap-2 mb-1">
                <KeyRound size={14} style={{ color: "var(--text-dim)" }} />
                <div className="text-sm font-medium" style={{ color: "var(--text-primary)" }}>网关访问密钥</div>
                <Badge variant={gateway?.access_key_set ? "ok" : "mute"} dot>
                  {gateway?.access_key_set ? "已启用" : "未设置"}
                </Badge>
              </div>
              <p className="text-xs mb-3" style={{ color: "var(--text-dim)" }}>
                设置后，调用网关的 Agent 工具须携带此密钥（<span className="font-mono">Authorization: Bearer</span>、
                <span className="font-mono"> x-api-key</span> 或 <span className="font-mono">x-goog-api-key</span>）。
                密钥仅写入系统凭证库，保存后不会再次回显；留空可清除鉴权。<span className="font-mono">/health</span> 探针始终免鉴权，修改立即生效。
              </p>
              <div className="flex items-center gap-2">
                <div className="relative flex-1">
                  <input
                    type={showKey ? "text" : "password"}
                    value={keyInput}
                    onChange={(e) => { setKeyInput(e.target.value); setKeyDirty(true); }}
                    placeholder="输入新的访问密钥"
                    className="w-full px-3 py-2 pr-9 rounded-md border text-sm font-mono outline-none"
                    style={{
                      backgroundColor: "var(--bg-elevated)",
                      borderColor: "var(--border-default)",
                      color: "var(--text-primary)",
                    }}
                  />
                  <button
                    type="button"
                    onClick={() => setShowKey((v) => !v)}
                    className="absolute right-2 top-1/2 -translate-y-1/2 cursor-pointer"
                    style={{ color: "var(--text-dim)" }}
                  >
                    {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
                  </button>
                </div>
                {gateway?.access_key_set && (
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={setAccessKey.isPending}
                    onClick={handleClearKey}
                  >
                    清除
                  </Button>
                )}
                <Button
                  size="sm"
                  variant="primary"
                  disabled={!keyDirty || !keyInput.trim() || setAccessKey.isPending}
                  onClick={handleSaveKey}
                >
                  {setAccessKey.isPending ? "保存中..." : "保存"}
                </Button>
              </div>
            </div>
          </CBody>
        </Card>
      )}

      {/* Logs & Storage Tab */}
      {activeTab === "logs" && (
        <Card>
          <CTitle>日志 & 存储</CTitle>
          <CBody className="space-y-4 mt-2">
            <div className="grid grid-cols-2 gap-4 text-sm">
              <div>
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>数据库</div>
                <div className="mt-1 font-mono text-xs" style={{ color: "var(--text-primary)" }}>SQLite (WAL 模式)</div>
              </div>
              <div>
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>日志保留</div>
                <div className="mt-1" style={{ color: "var(--text-primary)" }}>最近 7 天</div>
              </div>
            </div>
            <div
              className="flex items-center gap-3 p-3 rounded-md border cursor-pointer transition-colors"
              style={{
                borderColor: "var(--border-subtle)",
                backgroundColor: "var(--bg-elevated)",
              }}
              onClick={() => window.dispatchEvent(new CustomEvent("poolgate:navigate", { detail: "applog" }))}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter") window.dispatchEvent(new CustomEvent("poolgate:navigate", { detail: "applog" }));
              }}
            >
              <div
                className="w-9 h-9 grid place-items-center rounded-md"
                style={{ backgroundColor: "var(--bg-canvas)", color: "var(--color-brand)" }}
              >
                <Terminal size={16} />
              </div>
              <div className="flex-1 min-w-0">
                <div className="text-sm font-medium" style={{ color: "var(--text-primary)" }}>查看程序日志</div>
                <div className="text-xs truncate" style={{ color: "var(--text-dim)" }}>
                  网关运行日志 · 终端实时输出 · ERROR / WARN / INFO 分级着色
                </div>
              </div>
              <ExternalLink size={14} style={{ color: "var(--text-dim)" }} />
            </div>
          </CBody>
        </Card>
      )}

      {/* Tool Config Tab */}
      {activeTab === "tools" && (
        <Card>
          <CTitle>工具配置 (一键复制)</CTitle>
          <CBody className="space-y-3 mt-2">
            <p className="text-xs" style={{ color: "var(--text-dim)" }}>
              将以下配置粘贴到对应的工具中，即可通过 PoolGate 代理访问 API。
            </p>
            {toolConfigs.map((tc, i) => (
              <div key={tc.name} className="p-3 rounded-md border" style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-subtle)" }}>
                <div className="flex items-center justify-between mb-2">
                  <div>
                    <div className="text-sm font-medium" style={{ color: "var(--text-primary)" }}>{tc.name}</div>
                    <div className="text-xs" style={{ color: "var(--text-dim)" }}>{tc.description}</div>
                  </div>
                  <Button size="sm" variant="ghost" onClick={() => handleCopy(tc.config, i)}>
                    {copiedIndex === i ? <Check size={14} className="text-[var(--color-ok)]" /> : <Copy size={14} />}
                    {copiedIndex === i ? "已复制" : "复制"}
                  </Button>
                </div>
                <pre className="p-2 rounded text-xs font-mono overflow-x-auto" style={{ backgroundColor: "var(--bg-canvas)", color: "var(--text-secondary)" }}>
                  {tc.config}
                </pre>
              </div>
            ))}
          </CBody>
        </Card>
      )}

      {/* About Tab */}
      {activeTab === "about" && (
        <>
          <Card>
            <CTitle>快捷键</CTitle>
            <CBody className="mt-2">
              <div className="space-y-2">
                {shortcuts.map((s) => (
                  <div key={s.keys} className="flex items-center justify-between py-1.5">
                    <span className="text-sm" style={{ color: "var(--text-secondary)" }}>{s.desc}</span>
                    <kbd className="text-xs px-2 py-1 rounded border font-mono" style={{ borderColor: "var(--border-default)", color: "var(--text-dim)", backgroundColor: "var(--bg-elevated)" }}>
                      {s.keys}
                    </kbd>
                  </div>
                ))}
              </div>
            </CBody>
          </Card>

          <Card>
            <CTitle>系统信息</CTitle>
            <CBody className="mt-2">
              <div className="grid grid-cols-2 gap-4 text-sm">
                <div>
                  <div className="text-xs" style={{ color: "var(--text-dim)" }}>版本</div>
                  <div className="mt-1" style={{ color: "var(--text-primary)" }}>PoolGate v0.1.0</div>
                </div>
                <div>
                  <div className="text-xs" style={{ color: "var(--text-dim)" }}>运行环境</div>
                  <div className="mt-1" style={{ color: "var(--text-primary)" }}>Tauri 2.x + SQLite</div>
                </div>
                <div>
                  <div className="text-xs" style={{ color: "var(--text-dim)" }}>前端框架</div>
                  <div className="mt-1" style={{ color: "var(--text-primary)" }}>React 18 + TypeScript + TailwindCSS</div>
                </div>
                <div>
                  <div className="text-xs" style={{ color: "var(--text-dim)" }}>GitHub</div>
                  <div className="mt-1">
                    <a href="#" className="inline-flex items-center gap-1 text-sm text-[var(--color-brand)] hover:underline cursor-pointer">
                      查看源码 <ExternalLink size={12} />
                    </a>
                  </div>
                </div>
              </div>
            </CBody>
          </Card>
        </>
      )}
    </div>
  );
}
