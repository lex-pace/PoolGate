import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Activity, Boxes, ExternalLink, LogOut, Network, Power, RefreshCw, ServerCog, Settings2, Waypoints } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import {
  getTraySnapshot,
  openPoolGateFromTray,
  quitPoolGateFromTray,
  resizeTrayWindow,
  startProxy,
  stopProxy,
  type TopologyRuntimeDelta,
  type TraySnapshot,
} from "@/lib/tauri-commands";
import ActivityHeatmap from "@/components/ActivityHeatmap";
import { formatTokensZh } from "@/lib/utils";

const REFRESH_INTERVAL_MS = 10_000;
const MAX_TRAY_HEIGHT = 760;
const MIN_TRAY_HEIGHT = 360;
const MAX_TRAY_WIDTH = 430;
const MIN_TRAY_WIDTH = 386;
type Range = "today" | "7d" | "month" | "all";

const rangeOptions: Array<{ key: Range; label: string }> = [
  { key: "today", label: "今日" },
  { key: "7d", label: "近 7 天" },
  { key: "month", label: "本月" },
  { key: "all", label: "累计" },
];

function Sparkline({ values, area = false }: { values: number[]; area?: boolean }) {
  const safe = values.length ? values : Array.from({ length: 24 }, () => 0);
  const max = Math.max(1, ...safe);
  const points = safe.map((value, index) => {
    const x = safe.length === 1 ? 0 : (index / (safe.length - 1)) * 100;
    const y = 38 - (value / max) * 31;
    return [x, y] as const;
  });
  const line = points.map(([x, y], index) => `${index ? "L" : "M"}${x.toFixed(2)} ${y.toFixed(2)}`).join(" ");
  const areaPath = `${line} L100 40 L0 40 Z`;

  return (
    <svg className="pg-tray-sparkline" viewBox="0 0 100 42" preserveAspectRatio="none" aria-hidden="true">
      <line x1="0" y1="9" x2="100" y2="9" className="pg-tray-grid-line" />
      <line x1="0" y1="24" x2="100" y2="24" className="pg-tray-grid-line" />
      <line x1="0" y1="39" x2="100" y2="39" className="pg-tray-grid-line" />
      {area && <path d={areaPath} className="pg-tray-chart-area" />}
      <path d={line} className="pg-tray-chart-line" />
    </svg>
  );
}

const emptySnapshot: TraySnapshot = {
  gateway_running: false,
  port: 9800,
  active_connections: 0,
  topology: { protocol_count: 0, pool_count: 0, provider_count: 0, warning_count: 0, fault_count: 0 },
  resources_total: 0,
  resources_available: 0,
  resources_limited: 0,
  enabled_pool_count: 0,
  today_tokens: 0,
  seven_day_tokens: 0,
  month_tokens: 0,
  cumulative_tokens: 0,
  total_requests: 0,
  success_rate: 0,
  current_tps: 0,
  activity: [],
  tokens_heatmap: [],
  pools: [],
  updated_at: "--:--:--",
};

export default function TrayCard() {
  const cardRef = useRef<HTMLElement>(null);
  const lastWindowSize = useRef({ width: 0, height: 0 });
  const [snapshot, setSnapshot] = useState<TraySnapshot>(emptySnapshot);
  const [range, setRange] = useState<Range>("today");
  const [activityView, setActivityView] = useState<"requests" | "tokens">("requests");
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [toggling, setToggling] = useState(false);
  const [actionFeedback, setActionFeedback] = useState("");

  const refresh = useCallback(async (showBusy = false) => {
    if (showBusy) setRefreshing(true);
    try {
      setSnapshot(await getTraySnapshot());
      if (showBusy) setActionFeedback("已刷新");
    } finally {
      setLoading(false);
      if (showBusy) setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), REFRESH_INTERVAL_MS);
    const unlistenPromise = listen<TopologyRuntimeDelta>("topology:runtime-delta", ({ payload }) => {
      setSnapshot((current) => ({
        ...current,
        active_connections: payload.active_connections,
        active_route: payload.latest_route ? {
          protocol: payload.latest_route.protocol,
          pool_id: payload.latest_route.pool_id,
          pool_name: current.pools.find((pool) => pool.id === payload.latest_route?.pool_id)?.name || payload.latest_route.pool_id,
          provider_id: payload.latest_route.provider_id,
          provider_name: current.pools.flatMap((pool) => pool.providers).find((provider) => provider.id === payload.latest_route?.provider_id)?.name || payload.latest_route.provider_id,
          status: payload.latest_route.status,
          attempt: payload.latest_route.attempt,
          latency_ms: payload.latest_route.latency_ms,
          active_requests: payload.active_paths[0]?.active_requests || 0,
          updated_at: payload.latest_route.updated_at,
        } : current.active_route,
      }));
    });
    return () => {
      window.clearInterval(timer);
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [refresh]);

  useEffect(() => {
    if (!actionFeedback) return;
    const timer = window.setTimeout(() => setActionFeedback(""), 1600);
    return () => window.clearTimeout(timer);
  }, [actionFeedback]);

  useEffect(() => {
    const card = cardRef.current;
    if (!card) return;
    const syncWindowSize = () => {
      const rect = card.getBoundingClientRect();
      const width = Math.max(MIN_TRAY_WIDTH, Math.min(MAX_TRAY_WIDTH, Math.ceil(Math.max(rect.width, card.scrollWidth) + 2)));
      const height = Math.max(MIN_TRAY_HEIGHT, Math.min(MAX_TRAY_HEIGHT, Math.ceil(Math.max(rect.height, card.scrollHeight) + 2)));
      const previous = lastWindowSize.current;
      if (Math.abs(width - previous.width) < 2 && Math.abs(height - previous.height) < 2) return;
      lastWindowSize.current = { width, height };
      void resizeTrayWindow(width, height).catch(() => undefined);
    };
    const observer = new ResizeObserver(syncWindowSize);
    observer.observe(card);
    window.requestAnimationFrame(syncWindowSize);
    return () => observer.disconnect();
  }, []);

  const availability = snapshot.resources_total
    ? Math.round((snapshot.resources_available / snapshot.resources_total) * 100)
    : 0;
  const unavailable = Math.max(0, snapshot.resources_total - snapshot.resources_available);
  const selectedTokens = {
    today: snapshot.today_tokens,
    "7d": snapshot.seven_day_tokens,
    month: snapshot.month_tokens,
    all: snapshot.cumulative_tokens,
  }[range];
  const requestSeries = useMemo(() => snapshot.activity.map((point) => point.requests), [snapshot.activity]);
  const maxHour = useMemo(() => {
    if (!snapshot.activity.length) return null;
    return snapshot.activity.reduce((best, point) => point.requests > best.requests ? point : best, snapshot.activity[0]);
  }, [snapshot.activity]);
  const topology = useMemo(() => {
    const enabledPools = snapshot.pools.filter((pool) => pool.enabled);
    const protocolNames = Array.from(new Set(enabledPools.map((pool) => pool.protocol)));
    const providers = Array.from(
      new Map(enabledPools.flatMap((pool) => pool.providers).map((provider) => [provider.id, provider])).values(),
    );
    const activePool = snapshot.active_route
      ? snapshot.pools.find((pool) => pool.id === snapshot.active_route?.pool_id)
      : enabledPools[0];
    const activeProvider = snapshot.active_route
      ? providers.find((provider) => provider.id === snapshot.active_route?.provider_id)
      : activePool?.providers[0];
    return { enabledPools, protocolNames, providers, activePool, activeProvider };
  }, [snapshot.pools, snapshot.active_route]);

  const toggleGateway = async () => {
    if (toggling) return;
    setToggling(true);
    try {
      if (snapshot.gateway_running) await stopProxy();
      else await startProxy();
      await refresh();
      setActionFeedback(snapshot.gateway_running ? "网关已停止" : "网关已启动");
    } finally {
      setToggling(false);
    }
  };

  // Deep link: clicking the active path opens the command center with the
  // provider (or the pool when no provider is routed yet) focused.
  const openPathInDashboard = () => {
    const providerId = snapshot.active_route?.provider_id || topology.activeProvider?.id;
    if (providerId) void openPoolGateFromTray("dashboard", `provider-${providerId}`);
    else void openPoolGateFromTray("dashboard");
  };

  return (
    <main ref={cardRef} className={`pg-tray-card pg-tray-card-light ${loading ? "is-loading" : ""}`}>
      <div className="pg-tray-scroll-content">
        <header className="pg-tray-light-header">
        <div className="pg-tray-brand">
          <img src="/poolgate-icon.png" alt="" />
          <div className="pg-tray-title-group">
            <h1>PoolGate</h1>
            <span>本地 Agent 网关</span>
          </div>
          <span className={`pg-tray-gateway-badge ${snapshot.gateway_running ? "online" : "offline"}`}>
            <i />{snapshot.gateway_running ? "运行中" : "已停止"}
          </span>
        </div>
        <div className="pg-tray-segmented">
          {rangeOptions.map((option) => (
            <button key={option.key} className={range === option.key ? "active" : ""} onClick={() => setRange(option.key)}>
              {option.label}
            </button>
          ))}
        </div>
      </header>

      <section className="pg-tray-topology-panel" aria-label="实时路由拓扑">
        <div className="pg-tray-topology-head">
          <div>
            <strong>实时路由拓扑</strong>
            <span className={snapshot.gateway_running ? "online" : "offline"}>
              <i />{snapshot.gateway_running ? "链路可用" : "网关离线"}
            </span>
          </div>
          <code>http://127.0.0.1:{snapshot.port}</code>
        </div>
        <button className="pg-tray-route-path" onClick={openPathInDashboard} title="打开指挥中心并定位当前路径" aria-label="打开指挥中心并定位当前路径">
          <span className={`pg-tray-path-step ${snapshot.gateway_running ? "healthy" : "offline"}`}><Waypoints size={15} /><span><small>Gateway</small><strong>127.0.0.1:{snapshot.port}</strong></span></span>
          <i className={snapshot.active_route?.status === "active" ? "active" : ""} />
          <span className="pg-tray-path-step"><Network size={15} /><span><small>Protocol · {snapshot.topology.protocol_count}</small><strong title={snapshot.active_route?.protocol || topology.protocolNames[0] || "未配置"}>{snapshot.active_route?.protocol || topology.protocolNames[0] || "未配置"}</strong></span></span>
          <i className={snapshot.active_route?.status === "active" ? "active" : ""} />
          <span className="pg-tray-path-step"><Boxes size={15} /><span><small>Route Pool · {snapshot.topology.pool_count}</small><strong title={snapshot.active_route?.pool_name || topology.activePool?.name || "暂无路由池"}>{snapshot.active_route?.pool_name || topology.activePool?.name || "暂无路由池"}</strong></span></span>
          <i className={snapshot.active_route?.status === "active" ? "active" : ""} />
          <span className="pg-tray-path-step"><ServerCog size={15} /><span><small>Provider · {snapshot.topology.provider_count}</small><strong>{snapshot.active_route?.provider_name || topology.activeProvider?.name || "暂无厂商"}</strong></span></span>
        </button>
        <div className="pg-tray-current-route">
          <span><Activity size={12} />当前最活跃路径</span>
          <b>{snapshot.active_route?.active_requests || 0} 个路径请求 · {snapshot.active_connections} 个网关连接</b>
          {(snapshot.topology.warning_count + snapshot.topology.fault_count) > 0 && <em>{snapshot.topology.warning_count + snapshot.topology.fault_count} 项异常</em>}
        </div>
      </section>

      <section className="pg-tray-overview-strip" aria-label="运行统计">
        <div><span>资源可用率</span><strong>{availability}%</strong><small>{snapshot.resources_available}/{snapshot.resources_total} 可用</small></div>
        <div><span>可用路由池</span><strong>{snapshot.enabled_pool_count}</strong><small>共 {snapshot.pools.length} 个</small></div>
        <div><span>{rangeOptions.find((item) => item.key === range)?.label} Tokens</span><strong>{formatTokensZh(selectedTokens)}</strong><small>{snapshot.total_requests.toLocaleString("zh-CN")} 次请求</small></div>
      </section>

      <section className="pg-tray-activity-section">
        <div className="pg-tray-activity-title">
          <div className="pg-tray-title-left">
            <strong>{activityView === "requests" ? "今日请求活跃度" : "Tokens 活跃热力图"}</strong>
            <span>
              {activityView === "requests"
                ? `成功率 ${snapshot.success_rate.toFixed(1)}% · 本小时 ${snapshot.current_tps.toFixed(2)} req/s`
                : `最近 26 周 · ${snapshot.tokens_heatmap.filter((day) => day.tokens > 0).length} 天活跃`}
            </span>
          </div>
          <div className="pg-tray-view-toggle">
            <button className={activityView === "requests" ? "active" : ""} onClick={() => setActivityView("requests")} title="今日请求活跃度">请求</button>
            <button className={activityView === "tokens" ? "active" : ""} onClick={() => setActivityView("tokens")} title="Tokens 活跃热力图">Tokens</button>
          </div>
        </div>
        {activityView === "requests" ? (
          <div className="pg-tray-activity-chart compact">
            <Sparkline values={requestSeries} area />
            <div className="pg-tray-axis"><span>0 时</span><span>6 时</span><span>12 时</span><span>18 时</span><span>23 时</span></div>
          </div>
        ) : (
          <div className="pg-tray-activity-chart compact pg-tray-heatmap-wrap">
            {snapshot.tokens_heatmap.length ? (
              <>
                <ActivityHeatmap
                  data={snapshot.tokens_heatmap}
                  cellSize={7}
                  gap={2}
                  rounded={2}
                  maxWeeks={26}
                  align="end"
                  compact
                  fill
                  showLegend={false}
                />
                <div className="pg-tray-heatmap-foot">
                  <span>近 26 周</span>
                  <span>{snapshot.tokens_heatmap.filter((day) => day.tokens > 0).length} 天活跃</span>
                </div>
              </>
            ) : (
              <span className="pg-tray-heatmap-empty">暂无活跃记录</span>
            )}
          </div>
        )}
      </section>
      </div>

      <footer className="pg-tray-light-footer pg-tray-action-footer">
        <div className="pg-tray-footer-status">
          <span>{actionFeedback || `本机 · ${snapshot.updated_at} 更新`}</span>
          <small>{unavailable ? `${unavailable} 个资源需关注` : `127.0.0.1:${snapshot.port}`}{maxHour?.requests ? ` · 峰值 ${maxHour.hour} 时` : ""}</small>
        </div>
        <div className="pg-tray-icon-actions">
          <button className="pg-tray-icon-button" onClick={() => void refresh(true)} disabled={refreshing} title="刷新运行数据" aria-label="刷新运行数据">
            <RefreshCw size={16} className={refreshing ? "spinning" : ""} /><span>刷新</span>
          </button>
          <button className="pg-tray-icon-button" onClick={() => void openPoolGateFromTray("dashboard")} title="打开 PoolGate" aria-label="打开 PoolGate">
            <ExternalLink size={16} /><span>打开</span>
          </button>
          <button className="pg-tray-icon-button" onClick={() => void openPoolGateFromTray("settings")} title="打开网关配置" aria-label="打开网关配置">
            <Settings2 size={16} /><span>配置</span>
          </button>
          <button className={`pg-tray-icon-button power ${snapshot.gateway_running ? "running" : ""}`} onClick={() => void toggleGateway()} disabled={toggling} title={snapshot.gateway_running ? "停止网关" : "启动网关"} aria-label={snapshot.gateway_running ? "停止网关" : "启动网关"}>
            <Power size={16} /><span>{snapshot.gateway_running ? "停止" : "启动"}</span>
          </button>
          <button className="pg-tray-icon-button danger" onClick={() => void quitPoolGateFromTray()} title="退出 PoolGate" aria-label="退出 PoolGate">
            <LogOut size={16} /><span>退出</span>
          </button>
        </div>
      </footer>
    </main>
  );
}
