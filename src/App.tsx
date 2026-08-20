import React, { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { LucideIcon } from "lucide-react";
import { ToastProvider } from "@/components/ui/Toast";
import TokenMonitorAlerts from "@/components/token-monitor/TokenMonitorAlerts";
import { GlobalSearch } from "@/components/ui/GlobalSearch";
import {
  useLogStats,
  useProxyStatus,
  useStartProxy,
  useStopProxy,
  useOnboardingState,
  useSetOnboardingCompleted,
  useAppMode,
} from "@/hooks/use-tauri";
import Wizard from "@/pages/Wizard";
import { ModeSelectionScreen } from "@/components/AppMode";
import {
  Activity,
  BarChart3,
  Bell,
  ChevronLeft,
  Command,
  FileText,
  FolderKanban,
  Layers3,
  LayoutDashboard,
  PanelLeftClose,
  PanelLeftOpen,
  Play,
  Search,
  Settings,
  Square,
} from "lucide-react";
import { lazy, Suspense, type ComponentType } from "react";
// Route-level code splitting keeps the topology engine (React Flow + ELK) out
// of the initial bundle (V2 §9: Topology page lazy-loaded).
const Dashboard = lazy(() => import("@/pages/Dashboard"));
const AccountPool = lazy(() => import("@/pages/AccountPool"));
const AgentGroups = lazy(() => import("@/pages/AgentGroups"));
const Logs = lazy(() => import("@/pages/Logs"));
const Analytics = lazy(() => import("@/pages/Analytics"));
const SettingsPage = lazy(() => import("@/pages/Settings"));
const AppLogs = lazy(() => import("@/pages/AppLogs"));
const TopologyFullscreenPage = lazy(() => import("@/pages/TopologyFullscreen"));
const TokenMonitorPage = lazy(() => import("@/pages/TokenMonitor"));

type Page = "dashboard" | "resources" | "groups" | "logs" | "analytics" | "tokenmonitor" | "settings" | "applog";

interface NavItem {
  id: Page;
  label: string;
  shortLabel: string;
  description: string;
  icon: LucideIcon;
  section: "command" | "resources" | "observability";
}

const navItems: NavItem[] = [
  { id: "dashboard", label: "指挥中心", shortLabel: "总览", description: "实时路由与网关态势", icon: LayoutDashboard, section: "command" },
  { id: "resources", label: "模型供应商", shortLabel: "供应商", description: "Coding Plan、免费模型与自定义接口", icon: Layers3, section: "resources" },
  { id: "groups", label: "路由池", shortLabel: "路由池", description: "按模型能力组池、调度与故障转移", icon: FolderKanban, section: "resources" },
  { id: "logs", label: "请求日志", shortLabel: "日志", description: "实时请求与错误追踪", icon: FileText, section: "observability" },
  { id: "analytics", label: "用量分析", shortLabel: "分析", description: "网关吞吐、延迟与成本趋势", icon: BarChart3, section: "observability" },
];

const sectionLabels: Record<NavItem["section"], string> = {
  command: "Gateway",
  resources: "Routing",
  observability: "Observability",
};

function AppShell() {
  const [page, setPage] = useState<Page>("dashboard");
  const [collapsed, setCollapsed] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [topologyFocusNode, setTopologyFocusNode] = useState<string | undefined>();
  const { data: proxyStatus } = useProxyStatus();
  const { data: logStats } = useLogStats("today");
  const startProxy = useStartProxy();
  const stopProxy = useStopProxy();

  const current = useMemo(
    () => navItems.find((item) => item.id === page) ?? {
      id: "settings" as Page,
      label: "设置",
      shortLabel: "设置",
      description: "网关、存储与开发工具配置",
      icon: Settings,
      section: "command" as const,
    },
    [page],
  );

  const handleNavigate = useCallback((nextPage: string) => {
    setPage(nextPage as Page);
  }, []);

  const handleToggleProxy = useCallback(async () => {
    if (startProxy.isPending || stopProxy.isPending) return;
    if (proxyStatus?.running) await stopProxy.mutateAsync();
    else await startProxy.mutateAsync();
  }, [proxyStatus?.running, startProxy, stopProxy]);

  useEffect(() => {
    const unlistenPromise = listen<string>("tray:navigate", ({ payload }) => {
      // Payload may carry a topology node deep link: "dashboard?node=provider-xxx".
      const [pagePart, query] = payload.split("?", 2);
      if (["dashboard", "resources", "groups", "logs", "analytics", "tokenmonitor", "settings", "applog"].includes(pagePart)) {
        setPage(pagePart as Page);
        if (pagePart === "dashboard") {
          const node = query?.startsWith("node=") ? query.slice("node=".length) : undefined;
          setTopologyFocusNode(node || undefined);
        }
      }
    });
    // In-app navigation from topology empty states and settings entries.
    const handleCustomNavigate = (event: Event) => {
      const page = (event as CustomEvent<string>).detail;
      if (["dashboard", "resources", "groups", "logs", "analytics", "tokenmonitor", "settings", "applog"].includes(page)) {
        setPage(page as Page);
      }
    };
    window.addEventListener("poolgate:navigate", handleCustomNavigate);
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
      window.removeEventListener("poolgate:navigate", handleCustomNavigate);
    };
  }, []);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const commandKey = event.metaKey || event.ctrlKey;
      if (commandKey && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setSearchOpen((value) => !value);
      }
      if (commandKey && event.key === ",") {
        event.preventDefault();
        setPage("settings");
      }
      if (commandKey && event.key.toLowerCase() === "r") {
        event.preventDefault();
        window.location.reload();
      }
      if (commandKey && event.shiftKey && event.key.toLowerCase() === "p") {
        event.preventDefault();
        void handleToggleProxy();
      }
      const index = Number.parseInt(event.key, 10);
      if (commandKey && index >= 1 && index <= 6) {
        event.preventDefault();
        const pages: Page[] = ["dashboard", "resources", "groups", "logs", "analytics", "settings"];
        setPage(pages[index - 1]);
      }
      if (event.key === "Escape") setSearchOpen(false);
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [handleToggleProxy]);

  const renderPage = () => {
    const pages: Record<Page, ComponentType<{ focusNodeId?: string; initialTab?: string; onInitialTabConsumed?: () => void } | Record<string, never>>> = {
      dashboard: Dashboard,
      resources: AccountPool,
      groups: AgentGroups,
      logs: Logs,
      applog: AppLogs,
      analytics: Analytics,
      tokenmonitor: TokenMonitorPage,
      settings: SettingsPage,
    };
    const Page = pages[page];
    return (
      <Suspense fallback={<div className="pg-page-loading"><span className="pg-spinner" /></div>}>
        <Page
          {...(page === "dashboard" ? { focusNodeId: topologyFocusNode } : {})}
        />
      </Suspense>
    );
  };

  const successRate = logStats?.total_requests
    ? `${((logStats.success_count / logStats.total_requests) * 100).toFixed(1)}%`
    : "--";

  const sections: NavItem["section"][] = ["command", "resources", "observability"];

  return (
    <div className="pg-app h-screen flex flex-col overflow-hidden select-none">
      <div className="flex flex-1 min-h-0 overflow-hidden">
        <aside
          className="pg-sidebar flex-shrink-0 flex flex-col border-r transition-[width] duration-200"
          style={{ width: collapsed ? 70 : 230, borderColor: "var(--border-default)" }}
        >
          <div
            className="h-[54px] flex items-center border-b px-4 shrink-0"
            style={{ borderColor: "var(--border-subtle)" }}
            data-tauri-drag-region
          >
            <div className={`flex items-center min-w-0 ${collapsed ? "w-full justify-center" : "gap-2"}`}>
              <img src="/poolgate-icon.png" alt="PoolGate" className="w-7 h-7 shrink-0" />
              {!collapsed && (
                <div className="leading-tight">
                  <div className="font-semibold tracking-[-0.02em] text-[13px] text-[var(--text-primary)]">PoolGate</div>
                  <div className="text-[9px] tracking-[0.12em] uppercase text-[var(--text-dim)]">Local Gateway</div>
                </div>
              )}
            </div>
          </div>

          <div className={`px-2.5 pt-3 ${collapsed ? "text-center" : ""}`}>
            <div
              className="flex items-center gap-2.5 rounded-[9px] border px-2.5 py-2"
              style={{ background: "var(--bg-inset)", borderColor: "var(--border-subtle)" }}
            >
              <span className={`status-dot ${proxyStatus?.running ? "ok pulse-ok" : "err"}`} />
              {!collapsed && (
                <div className="min-w-0 flex-1">
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-[11px] font-medium text-[var(--text-primary)]">
                      {proxyStatus?.running ? "网关在线" : "网关离线"}
                    </span>
                    <span className="pg-mono text-[9px] text-[var(--text-dim)]">:{proxyStatus?.port || 9800}</span>
                  </div>
                  <div className="text-[9px] text-[var(--text-dim)] mt-0.5">
                    {proxyStatus?.active_connections || 0} 个活跃连接
                  </div>
                </div>
              )}
            </div>
          </div>

          <nav className="flex-1 min-h-0 overflow-y-auto px-2.5 py-3">
            {sections.map((section) => {
              const items = navItems.filter((item) => item.section === section);
              return (
                <div key={section} className="mb-4">
                  {!collapsed && <div className="pg-eyebrow px-2 mb-1.5">{sectionLabels[section]}</div>}
                  <div className="space-y-0.5">
                    {items.map((item) => {
                      const Icon = item.icon;
                      const active = page === item.id;
                      return (
                        <button
                          key={item.id}
                          onClick={() => setPage(item.id)}
                          className={`w-full h-9 flex items-center rounded-[7px] transition-colors ${collapsed ? "justify-center" : "gap-2.5 px-2.5"}`}
                          style={{
                            background: active ? "var(--bg-active)" : "transparent",
                            color: active ? "var(--color-brand)" : "var(--text-secondary)",
                            fontWeight: active ? 600 : 450,
                          }}
                          title={collapsed ? item.label : undefined}
                        >
                          <Icon size={16} strokeWidth={active ? 2.2 : 1.8} />
                          {!collapsed && <span className="text-[12px] flex-1 text-left">{item.label}</span>}
                          {!collapsed && active && <ChevronLeft size={12} className="rotate-180 opacity-70" />}
                        </button>
                      );
                    })}
                  </div>
                </div>
              );
            })}
          </nav>

          <div className="px-2.5 pb-3">
            <button
              onClick={() => setPage("settings")}
              className={`w-full h-9 flex items-center rounded-[7px] transition-colors ${collapsed ? "justify-center" : "gap-2.5 px-2.5"}`}
              style={{
                background: page === "settings" ? "var(--bg-active)" : "transparent",
                color: page === "settings" ? "var(--color-brand)" : "var(--text-secondary)",
              }}
              title={collapsed ? "设置" : undefined}
            >
              <Settings size={16} strokeWidth={1.8} />
              {!collapsed && <span className="text-[12px]">设置</span>}
            </button>
          </div>
        </aside>

        <section className="flex-1 min-w-0 flex flex-col overflow-hidden">
          <header
            className="pg-toolbar pg-desktop-toolbar h-[54px] flex items-center justify-between gap-4 px-4 border-b shrink-0"
            style={{ borderColor: "var(--border-default)" }}
            data-tauri-drag-region
          >
            <div className="flex items-center gap-3 min-w-0">
              <button
                onClick={() => setCollapsed((value) => !value)}
                className="w-7 h-7 flex items-center justify-center rounded-md text-[var(--text-dim)] hover:bg-[var(--bg-hover)]"
                title={collapsed ? "展开边栏" : "收起边栏"}
              >
                {collapsed ? <PanelLeftOpen size={15} /> : <PanelLeftClose size={15} />}
              </button>
              <div className="w-px h-4 bg-[var(--border-default)]" />
              <div className="min-w-0">
                <h1 className="text-[13px] leading-4 font-semibold tracking-[-0.01em] text-[var(--text-primary)]">{current.label}</h1>
                <p className="text-[9px] leading-3 text-[var(--text-dim)] truncate">{current.description}</p>
              </div>
            </div>

            <div className="flex items-center gap-2">
              <button
                onClick={() => {
                  window.location.hash = "#/token-monitor";
                }}
                className={`h-7 flex items-center gap-1.5 px-2.5 rounded-[7px] border text-[11px] font-medium transition-colors ${
                  typeof window !== "undefined" && window.location.hash.startsWith("#/token-monitor")
                    ? "border-[var(--color-brand)] text-[var(--color-brand)] bg-[var(--color-brand)]/10"
                    : "border-[var(--color-brand)]/40 text-[var(--color-brand)] hover:bg-[var(--color-brand)]/10"
                }`}
                title="切换 PoolGate 桌面为 Token Monitor 仪表盘"
              >
                <Activity size={13} />
                <span>Token Monitor</span>
              </button>
              <button
                onClick={() => setSearchOpen(true)}
                className="h-7 w-[220px] flex items-center gap-2 px-2.5 rounded-[7px] border text-[11px] transition-colors hover:border-[var(--border-strong)]"
                style={{ background: "var(--bg-inset)", borderColor: "var(--border-default)", color: "var(--text-dim)" }}
              >
                <Search size={13} />
                <span className="flex-1 text-left">搜索模型资源和路由池</span>
                <span className="pg-command-key">⌘K</span>
              </button>
              <button className="relative w-7 h-7 flex items-center justify-center rounded-md text-[var(--text-dim)] hover:bg-[var(--bg-hover)]" title="告警中心">
                <Bell size={14} />
                {(logStats?.error_count || 0) > 0 && <span className="absolute top-1 right-1 w-1.5 h-1.5 rounded-full bg-[var(--color-err)]" />}
              </button>
              <button
                onClick={() => void handleToggleProxy()}
                disabled={startProxy.isPending || stopProxy.isPending}
                className="h-7 flex items-center gap-1.5 px-2.5 rounded-[7px] border text-[11px] font-medium disabled:opacity-50"
                style={{
                  color: proxyStatus?.running ? "var(--color-err)" : "var(--color-ok)",
                  borderColor: proxyStatus?.running ? "rgba(255,69,58,.25)" : "rgba(48,209,88,.25)",
                  background: proxyStatus?.running ? "var(--color-err-bg)" : "var(--color-ok-bg)",
                }}
                title="⌘⇧P"
              >
                {proxyStatus?.running ? <Square size={11} fill="currentColor" /> : <Play size={12} fill="currentColor" />}
                {proxyStatus?.running ? "停止网关" : "启动网关"}
              </button>
            </div>
          </header>

          <main className="pg-content flex-1 overflow-auto px-5 py-5 select-text">
            <div key={page} className="pg-page">{renderPage()}</div>
          </main>
        </section>
      </div>

      <footer
        className="pg-desktop-statusbar h-7 flex items-center justify-between px-3 border-t text-[10px] shrink-0"
        style={{ background: "var(--bg-page-solid)", borderColor: "var(--border-default)", color: "var(--text-dim)" }}
      >
        <div className="flex items-center gap-2.5">
          <span className="inline-flex items-center gap-1.5">
            <Activity size={11} className={proxyStatus?.running ? "text-[var(--color-ok)]" : "text-[var(--color-err)]"} />
            {proxyStatus?.running ? "Gateway operational" : "Gateway stopped"}
          </span>
          <span className="w-px h-3 bg-[var(--border-subtle)]" />
          <span className="pg-mono">127.0.0.1:{proxyStatus?.port || 9800}</span>
        </div>
        <div className="flex items-center gap-3.5">
          <span>连接 <strong className="font-medium text-[var(--text-secondary)]">{proxyStatus?.active_connections || 0}</strong></span>
          <span>请求 <strong className="font-medium text-[var(--text-secondary)]">{(logStats?.total_requests || 0).toLocaleString()}</strong></span>
          <span>成功率 <strong className="font-medium text-[var(--text-secondary)]">{successRate}</strong></span>
          <span className="inline-flex items-center gap-1"><Command size={10} /> 系统外观</span>
        </div>
      </footer>

      <GlobalSearch open={searchOpen} onClose={() => setSearchOpen(false)} onNavigate={handleNavigate} />
    </div>
  );
}

export default function App() {
  // 首次启动引导：未完成（且没有账号+池）时全屏展示三步向导。
  const { data: appMode, isLoading: appModeLoading } = useAppMode();
  const { data: onboarding, isLoading: onboardingLoading } = useOnboardingState();
  const setOnboardingCompleted = useSetOnboardingCompleted();
  const [onboardingDismissed, setOnboardingDismissed] = useState(false);

  // 已经建好账号+路由池的老用户不弹向导，静默标记完成。
  useEffect(() => {
    if (onboarding && !onboarding.completed && onboarding.account_count > 0 && onboarding.pool_count > 0) {
      void setOnboardingCompleted.mutate(true);
    }
  }, [onboarding, setOnboardingCompleted]);

  const showOnboarding =
    appMode?.mode === "gateway" && !onboardingLoading && !!onboarding && !onboarding.completed && !onboardingDismissed;

  // 两种全屏形态（通过 hash 切换，均不进侧边栏菜单）：
  //  - `#/topology-fullscreen`：四层拓扑占满整窗（既有）
  //  - `#/token-monitor`：Token Monitor 仪表盘 —— 切换 PoolGate 桌面的形态，
  //    不含侧边栏/工具栏，顶部提供「返回 PoolGate」入口（用户决策）。
  const [topologyFullscreen, setTopologyFullscreen] = useState<boolean>(
    typeof window !== "undefined" && window.location.hash === "#/topology-fullscreen",
  );
  const isMonitorMode = appMode?.mode === "monitor";
  const [tmFullscreen, setTmFullscreen] = useState<boolean>(
    isMonitorMode || (typeof window !== "undefined" && window.location.hash.startsWith("#/token-monitor")),
  );
  const [tmInitialTab, setTmInitialTab] = useState<string | undefined>(
    typeof window !== "undefined"
      ? /^#\/token-monitor\?tab=(.+)$/.exec(window.location.hash)?.[1] || undefined
      : undefined,
  );

  useEffect(() => {
    const sync = () => {
      setTopologyFullscreen(window.location.hash === "#/topology-fullscreen");
      const isTm = isMonitorMode || window.location.hash.startsWith("#/token-monitor");
      setTmFullscreen(isTm);
      if (isTm) {
        const match = /^#\/token-monitor\?tab=(.+)$/.exec(window.location.hash);
        setTmInitialTab(match?.[1] || undefined);
      }
    };
    window.addEventListener("hashchange", sync);
    sync();
    return () => window.removeEventListener("hashchange", sync);
  }, [isMonitorMode]);

  // 托盘深链：`tokenmonitor`（或 `tokenmonitor?tab=settings`）→ 切换为仪表盘形态。
  // 放在 App 层保证两种形态下都生效（AppShell 卸载后其监听不失效）。
  useEffect(() => {
    const unlistenPromise = listen<string>("tray:navigate", ({ payload }) => {
      const [pagePart, query] = payload.split("?", 2);
      if (pagePart === "tokenmonitor") {
        const tab = query?.startsWith("tab=") ? query.slice("tab=".length) : undefined;
        window.location.hash = tab ? `#/token-monitor?tab=${tab}` : "#/token-monitor";
      }
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  return (
    <ToastProvider>
      {/* 额度/采集告警桥接：监听 token-monitor:alert → 系统通知（跨窗口去重）+ 应用内 Toast */}
      <TokenMonitorAlerts />
      {appModeLoading ? (
        <div className="pg-app h-screen grid place-items-center"><span className="pg-spinner" /></div>
      ) : !appMode?.selected ? (
        <ModeSelectionScreen />
      ) : showOnboarding ? (
        <div className="pg-app h-screen flex flex-col overflow-hidden">
          <div className="flex-1 min-h-0 overflow-auto">
            <Wizard
              onComplete={() => {
                setOnboardingDismissed(true);
                void setOnboardingCompleted.mutate(true);
              }}
            />
          </div>
        </div>
      ) : tmFullscreen ? (
        <Suspense fallback={<div className="pg-page-loading"><span className="pg-spinner" /></div>}>
          <div className="pg-app h-screen flex flex-col overflow-hidden">
            <TokenMonitorPage
              initialTab={tmInitialTab}
              onInitialTabConsumed={() => setTmInitialTab(undefined)}
              onExit={isMonitorMode ? undefined : () => { window.location.hash = ""; }}
            />
          </div>
        </Suspense>
      ) : topologyFullscreen ? (
        <Suspense fallback={<div className="pg-page-loading"><span className="pg-spinner" /></div>}>
          <TopologyFullscreenPage />
        </Suspense>
      ) : (
        <AppShell />
      )}
    </ToastProvider>
  );
}
