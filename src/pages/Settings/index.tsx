import React, { useState } from "react";
import { Card, CTitle, CBody } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { Spinner } from "@/components/ui/Spinner";
import { useProxyStatus, useGatewaySettings, useSetGatewayAccessKey, useSetCloseButtonBehavior, useSetListenAddr, useLanAddresses, useMenuBarMainText, useSetMenuBarMainText } from "@/hooks/use-tauri";
import { useTheme } from "@/components/ui/ThemeProvider";
import { useGlassSettings } from "@/components/ui/GlassSettings";
import { useAccountDisplay } from "@/components/ui/AccountDisplay";
import { useToast } from "@/components/ui/Toast";
import { ModeSettings } from "@/components/AppMode";
import {
  Settings as SettingsIcon, Server, FileText, Wrench, Info, Terminal, Layers3, Activity,
  Copy, Check, ExternalLink, KeyRound, Eye, EyeOff, Globe, Network, ShieldAlert,
  Archive, LogOut, Monitor, Sun, Moon,
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

const choiceIcons: Record<string, React.ElementType> = {
  hide: Archive,
  quit: LogOut,
  system: Monitor,
  light: Sun,
  dark: Moon,
  tokens: Activity,
  top1_tool: Wrench,
  top1_model: Layers3,
  mask: EyeOff,
  full: Eye,
  localhost: Globe,
  lan: Network,
};

function ChoiceIcon({ kind }: { kind: string }) {
  const Icon = choiceIcons[kind] ?? SettingsIcon;
  return <Icon size={16} strokeWidth={1.8} aria-hidden="true" />;
}

const buildToolConfigs = (endpoint: string) => [
  {
    name: "Claude Code",
    description: "ANTHROPIC_BASE_URL 环境变量",
    config: `export ANTHROPIC_BASE_URL=${endpoint}`,
  },
  {
    name: "Codex CLI",
    description: "OPENAI_BASE_URL 环境变量",
    config: `export OPENAI_BASE_URL=${endpoint}`,
  },
  {
    name: "OpenCode",
    description: "JSON 配置文件格式",
    config: `{
  "providers": {
    "default": {
      "baseURL": "${endpoint}"
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
  const setListenAddr = useSetListenAddr();
  const { data: lanAddresses, isLoading: lanLoading } = useLanAddresses();
  const { preference: themePreference, setPreference: setThemePreference } = useTheme();
  const { glassOpacity, glassBlur, chromeFollows, setGlassSettings, setChromeFollows, resetGlassSettings } = useGlassSettings();
  const { mode: accountDisplayMode, setMode: setAccountDisplayMode } = useAccountDisplay();
  const { data: menuBarMainText } = useMenuBarMainText();
  const setMenuBarMainText = useSetMenuBarMainText();
  const { toast } = useToast();

  const handleGlassChange = (opacity: number, blur: number) => {
    setGlassSettings(opacity, blur);
  };
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

  const isLan = gateway?.listen_addr === "lan";
  const lanHost = isLan && lanAddresses && lanAddresses.length > 0 ? lanAddresses[0] : null;
  const serviceHost = isLan ? (lanHost || "127.0.0.1") : "127.0.0.1";
  const endpoint = `http://${serviceHost}:${proxy?.port || 9800}`;
  const toolConfigs = buildToolConfigs(endpoint);

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

  const handleListenAddrChange = async (mode: "localhost" | "lan") => {
    try {
      await setListenAddr.mutateAsync(mode);
      toast("success", mode === "lan" ? "已切换为局域网监听（若网关运行中已自动重启）" : "已切回仅本机监听（若网关运行中已自动重启）");
    } catch (e) {
      toast("error", `切换失败: ${String(e)}`);
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
    <div className="space-y-5 animate-fade-in max-w-4xl pg-page pg-settings-page">
      <div className="pg-page-header pg-settings-page-header">
        <div className="pg-eyebrow mb-1">System Configuration</div>
        <h2 style={{ color: "var(--text-primary)" }}>系统设置</h2>
        <p className="text-xs mt-0.5" style={{ color: "var(--text-dim)" }}>PoolGate 代理服务状态与系统信息</p>
      </div>

      {/* Tab navigation */}
      <div className="pg-settings-tabs" role="tablist" aria-label="设置分类">
        {settingsTabs.map((tab) => {
          const Icon = tab.icon;
          return (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              role="tab"
              aria-selected={activeTab === tab.id}
              className={`pg-settings-tab ${activeTab === tab.id ? "pg-settings-tab-active" : "pg-settings-tab-idle"}`}
            >
              <Icon size={14} />
              <span className="hidden sm:inline">{tab.label}</span>
            </button>
          );
        })}
      </div>

      {/* General Tab */}
      {activeTab === "general" && (
        <>
        <Card className="pg-settings-mode-card">
          <CTitle>产品模式</CTitle>
          <CBody className="mt-2">
            <ModeSettings />
          </CBody>
        </Card>
        <Card className="pg-settings-general-card">
          <div className="pg-settings-card-heading">
            <div className="pg-settings-card-heading-main">
              <span className="pg-settings-card-icon"><SettingsIcon size={16} /></span>
              <div>
                <div className="pg-settings-card-eyebrow">PREFERENCES</div>
                <h3>通用设置</h3>
                <p>控制 PoolGate 的外观、行为与菜单栏信息</p>
              </div>
            </div>
            <span className="pg-settings-live"><i /> 实时生效</span>
          </div>
          <CBody className="space-y-4 mt-2 pg-settings-body">
            <div className="grid grid-cols-2 gap-4 text-sm pg-settings-grid">
              <div className="pg-settings-summary">
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>代理端口</div>
                <div className="mt-1 font-mono" style={{ color: "var(--text-primary)" }}>{proxy?.port || 9800}</div>
                <div className="text-xs mt-0.5" style={{ color: "var(--text-dim)" }}>修改后需重启</div>
              </div>
              <div className="pg-settings-summary">
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>开机自启</div>
                <div className="mt-1">
                  <Badge variant="ok" dot>已启用</Badge>
                </div>
              </div>
              <div className="col-span-2 pg-settings-section">
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>关闭按钮行为</div>
                <div className="mt-2 flex items-center gap-3">
                  <label className="pg-radio-option" data-choice="hide">
                    <input
                      type="radio"
                      name="closeBehavior"
                      value="hide"
                      checked={gateway?.close_button_behavior === "hide"}
                      onChange={() => handleCloseBehaviorChange("hide")}
                      className="w-4 h-4"
                    />
                    <ChoiceIcon kind="hide" />
                    <span className="text-sm" style={{ color: "var(--text-primary)" }}>隐藏到托盘</span>
                  </label>
                  <label className="pg-radio-option" data-choice="quit">
                    <input
                      type="radio"
                      name="closeBehavior"
                      value="quit"
                      checked={gateway?.close_button_behavior === "quit"}
                      onChange={() => handleCloseBehaviorChange("quit")}
                      className="w-4 h-4"
                    />
                    <ChoiceIcon kind="quit" />
                    <span className="text-sm" style={{ color: "var(--text-primary)" }}>退出程序</span>
                  </label>
                </div>
                <div className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>
                  选择点击窗口关闭按钮时的行为
                </div>
              </div>
              <div className="pg-settings-section">
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>界面主题（含托盘）</div>
                <div className="mt-2 flex items-center gap-3">
                  {(["system", "light", "dark"] as const).map((option) => (
                    <label key={option} className="pg-radio-option" data-choice={option}>
                      <input
                        type="radio"
                        name="themePreference"
                        value={option}
                        checked={themePreference === option}
                        onChange={() => { setThemePreference(option); toast("success", `主题已切换为「${option === "system" ? "随系统" : option === "light" ? "浅色" : "深色"}」`); }}
                        className="w-4 h-4"
                      />
                      <ChoiceIcon kind={option} />
                      <span className="text-sm" style={{ color: "var(--text-primary)" }}>
                        {option === "system" ? "随系统" : option === "light" ? "浅色" : "深色"}
                      </span>
                    </label>
                  ))}
                </div>
                <div className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>
                  应用于主界面与托盘卡片，保存后立即生效
                </div>
              </div>
              <div className="col-span-2 pg-settings-section">
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>菜单栏主文本（macOS 菜单栏 / Windows 托盘）</div>
                <div className="mt-2 flex items-center gap-3">
                  {([["tokens", "今日 Tokens"], ["top1_tool", "Top1 工具"], ["top1_model", "Top1 模型"]] as const).map(([value, label]) => (
                    <label key={value} className="pg-radio-option" data-choice={value}>
                      <input
                        type="radio"
                        name="menuBarMainText"
                        value={value}
                        checked={(menuBarMainText ?? "tokens") === value}
                        onChange={() => {
                          setMenuBarMainText.mutate(value, {
                            onSuccess: () => toast("success", `菜单栏主文本已切换为「${label}」`),
                            onError: (e) => toast("error", `设置失败: ${String(e)}`),
                          });
                        }}
                        className="w-4 h-4"
                      />
                      <ChoiceIcon kind={value} />
                      <span className="text-sm" style={{ color: "var(--text-primary)" }}>{label}</span>
                    </label>
                  ))}
                </div>
                <div className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>
                  选择菜单栏主文本，带含义标签：今日（今日 Tokens，如「今日 240K」）/ 工具（今日用量第一的工具，如「工具 Claude Code」）/ 模型（今日用量第一的模型，如「模型 claude-sonnet-4…」，超长自动截断）。三选一固定展示、不轮播。菜单栏为单一图标（Logo + 状态点 + 主文本），状态点按综合状态着色：绿=正常、蓝=流量活跃、橙=额度告警、红=离线；额度剩余百分比与完整信息（状态 · 今日 Tokens · Top1 工具/模型）显示在鼠标悬浮提示中，保存后立即生效
                </div>
              </div>
              <div className="col-span-2 pg-settings-section">
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>账号身份展示（桌面端 + 托盘）</div>
                <div className="mt-2 flex items-center gap-3">
                  {([["mask", "脱密显示"], ["full", "完整显示"]] as const).map(([value, label]) => (
                    <label key={value} className="pg-radio-option" data-choice={value}>
                      <input
                        type="radio"
                        name="accountDisplay"
                        value={value}
                        checked={accountDisplayMode === value}
                        onChange={() => { setAccountDisplayMode(value); toast("success", `账号已切换为「${label}」`); }}
                        className="w-4 h-4"
                      />
                      <ChoiceIcon kind={value} />
                      <span className="text-sm" style={{ color: "var(--text-primary)" }}>{label}</span>
                    </label>
                  ))}
                </div>
                <div className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>
                  脱密显示只展示掩码身份（u***@example.com）；完整显示按原样展示账号名称与邮箱，立即生效
                </div>
              </div>
              <div className="col-span-2 pg-settings-section pg-settings-glass-section">
                <div className="text-xs" style={{ color: "var(--text-dim)" }}>玻璃效果（托盘 / 侧栏工具栏 / Token 仪表盘卡片）</div>
                <div className="mt-2 space-y-2.5">
                  <div className="flex items-center gap-3">
                    <span className="w-16 shrink-0 text-xs" style={{ color: "var(--text-primary)" }}>不透明度</span>
                    <input
                      type="range"
                      min={0}
                      max={100}
                      value={glassOpacity}
                      style={{ background: `linear-gradient(to right, var(--color-brand) ${glassOpacity}%, var(--bg-elevated) ${glassOpacity}%)` }}
                      onChange={(e) => handleGlassChange(Number(e.target.value), glassBlur)}
                      className="flex-1 accent-[var(--color-brand)]"
                    />
                    <span className="w-10 shrink-0 text-right font-mono text-xs" style={{ color: "var(--text-dim)" }}>{glassOpacity}%</span>
                  </div>
                  <div className="flex items-center gap-3">
                    <span className="w-16 shrink-0 text-xs" style={{ color: "var(--text-primary)" }}>模糊</span>
                    <input
                      type="range"
                      min={0}
                      max={100}
                      value={glassBlur}
                      style={{ background: `linear-gradient(to right, var(--color-brand) ${glassBlur}%, var(--bg-elevated) ${glassBlur}%)` }}
                      onChange={(e) => handleGlassChange(glassOpacity, Number(e.target.value))}
                      className="flex-1 accent-[var(--color-brand)]"
                    />
                    <span className="w-10 shrink-0 text-right font-mono text-xs" style={{ color: "var(--text-dim)" }}>{glassBlur}px</span>
                  </div>
                  <div className="flex items-center gap-2.5">
                    <input
                      type="checkbox"
                      id="glassChromeFollows"
                      checked={chromeFollows}
                      onChange={(e) => {
                        setChromeFollows(e.target.checked);
                        toast("success", `主窗口侧栏/工具栏${e.target.checked ? "已跟随玻璃滑块" : "已恢复固定透明度"}`);
                      }}
                      className="pg-settings-checkbox"
                    />
                    <label htmlFor="glassChromeFollows" className="text-xs cursor-pointer" style={{ color: "var(--text-primary)" }}>
                      主窗口侧栏/工具栏跟随玻璃滑块
                    </label>
                    <span className="text-xs" style={{ color: "var(--text-dim)" }}>（默认开启：托盘与桌面端统一使用玻璃参数）</span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-xs" style={{ color: "var(--text-dim)" }}>
                      作用于托盘外壳、主窗口侧栏/工具栏与各页卡片（Token 仪表盘 / 指挥中心 / 供应商 / 日志 / 分析）的透明度与毛玻璃模糊；浅色与深色均采用低透明度 Liquid Glass，默认透明度 32%、模糊 64px；默认全局联动，取消勾选后侧栏/工具栏使用固定透明度
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="shrink-0"
                      onClick={() => { resetGlassSettings(); toast("success", "已恢复默认玻璃效果"); }}
                    >
                      恢复默认
                    </Button>
                  </div>
                </div>
              </div>
            </div>
          </CBody>
        </Card>
        </>
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

            {/* Listen address */}
            <div className="pt-4 mt-2 border-t" style={{ borderColor: "var(--border-subtle)" }}>
              <div className="flex items-center gap-2 mb-1">
                <Globe size={14} style={{ color: "var(--text-dim)" }} />
                <div className="text-sm font-medium" style={{ color: "var(--text-primary)" }}>监听地址</div>
                <Badge variant={isLan ? "warn" : "ok"} dot>
                  {isLan ? "局域网" : "仅本机"}
                </Badge>
              </div>
              <div className="mt-2 flex items-center gap-3">
                <label className="pg-radio-option" data-choice="localhost">
                  <input
                    type="radio"
                    name="listenAddr"
                    value="localhost"
                    checked={!isLan}
                    onChange={() => handleListenAddrChange("localhost")}
                    className="w-4 h-4"
                  />
                  <ChoiceIcon kind="localhost" />
                  <span className="text-sm" style={{ color: "var(--text-primary)" }}>仅本机</span>
                </label>
                <label className="pg-radio-option" data-choice="lan">
                  <input
                    type="radio"
                    name="listenAddr"
                    value="lan"
                    checked={isLan}
                    onChange={() => handleListenAddrChange("lan")}
                    className="w-4 h-4"
                  />
                  <ChoiceIcon kind="lan" />
                  <span className="text-sm" style={{ color: "var(--text-primary)" }}>局域网共享</span>
                </label>
              </div>
              <p className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>
                仅本机：网关只监听 127.0.0.1，仅本机可访问（个人使用默认）。局域网共享：网关监听 0.0.0.0，同一网络内的同事可访问，切换后立即生效（若网关运行中会自动重启）。
              </p>
              {isLan && (
                <div className="mt-3 p-3 rounded-md border" style={{ backgroundColor: "var(--bg-elevated)", borderColor: "var(--border-subtle)" }}>
                  {gateway?.access_key_set ? (
                    <>
                      <div className="flex items-center gap-2 mb-2">
                        <Network size={13} style={{ color: "var(--color-ok)" }} />
                        <span className="text-xs font-medium" style={{ color: "var(--text-primary)" }}>同事访问地址</span>
                      </div>
                      {lanLoading ? (
                        <span className="text-xs" style={{ color: "var(--text-dim)" }}>正在探测局域网地址...</span>
                      ) : lanAddresses && lanAddresses.length > 0 ? (
                        <div className="space-y-1.5">
                          {lanAddresses.map((ip) => {
                            const url = `http://${ip}:${proxy?.port || 9800}`;
                            return (
                              <div key={ip} className="flex items-center gap-2">
                                <span className="font-mono text-xs" style={{ color: "var(--text-primary)" }}>{url}</span>
                                <Button size="sm" variant="ghost" onClick={() => { navigator.clipboard.writeText(url); toast("success", "已复制"); }}>
                                  <Copy size={12} />
                                </Button>
                              </div>
                            );
                          })}
                          <p className="text-xs mt-1" style={{ color: "var(--text-dim)" }}>
                            让同事在各自的 Agent 工具里把 Base URL 指向上面的地址，并携带网关访问密钥（<span className="font-mono">Authorization: Bearer</span>）。
                          </p>
                        </div>
                      ) : (
                        <p className="text-xs" style={{ color: "var(--text-dim)" }}>未检测到局域网地址，请确认已连接到网络。</p>
                      )}
                    </>
                  ) : (
                    <div className="flex items-start gap-2">
                      <ShieldAlert size={14} className="mt-0.5 shrink-0" style={{ color: "var(--color-warn)" }} />
                      <p className="text-xs" style={{ color: "var(--text-secondary)" }}>
                        局域网共享必须先设置<span className="font-medium">网关访问密钥</span>，否则局域网内任何设备都能无鉴权调用网关。请先在下方保存访问密钥后再切换。
                      </p>
                    </div>
                  )}
                </div>
              )}
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
