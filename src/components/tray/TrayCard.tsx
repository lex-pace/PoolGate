import { useCallback, useEffect, useMemo, useState}

 from "react";
import { Activity, Boxes, LogOut, Network, Power, ServerCog, Settings2, Waypoints, Coins}

 from "lucide-react";
import { listen}

 from "@tauri-apps/api/event";
import {
  getTraySnapshot,
  openPoolGateFromTray,
  quitPoolGateFromTray,
  startProxy,
  stopProxy,
  type TopologyRuntimeDelta,
  type TraySnapshot,
} from "@/lib/tauri-commands";
import ActivityHeatmap from "@/components/ActivityHeatmap";
import Sparkline from "@/components/tray/Sparkline";
import RingProgress from "@/components/tray/RingProgress";
import useCountUp from "@/hooks/useCountUp";
import RollingNumber from "@/components/tray/RollingNumber";
import { formatTokensZh}

 from "@/lib/utils";

const REFRESH_INTERVAL_MS = 10_000;
type Range = "today" | "7d" | "month" | "all";

const rangeOptions: Array<{ key: Range; label: string}

> = [
  { key: "today", label: "今日"}

,
  { key: "7d", label: "近 7 天"}

,
  { key: "month", label: "本月"}

,
  { key: "all", label: "累计"}

,
];

const emptySnapshot: TraySnapshot = {
  gateway_running: false,
  port: 9800,
  active_connections: 0,
  topology: { protocol_count: 0, pool_count: 0, provider_count: 0, warning_count: 0, fault_count: 0}

,
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

/** 本地今天（YYYY-MM-DD），热力图锚定窗口右端（与 Token 托盘一致）。 */
function todayKey(): string {
  const now = new Date();
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

export default function TrayCard() {
  const [snapshot, setSnapshot] = useState<TraySnapshot>(emptySnapshot);
  const [range, setRange] = useState<Range>("today");
  const [activityView, setActivityView] = useState<"requests" | "tokens">("requests");
  const [loading, setLoading] = useState(true);
  const [toggling, setToggling] = useState(false);
  const [actionFeedback, setActionFeedback] = useState("");

  const refresh = useCallback(async () => {
    try {
      setSnapshot(await getTraySnapshot()  );
}


 finally {
      setLoading(false  );
}



 }

, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), REFRESH_INTERVAL_MS);
    const unlistenPromise = listen<TopologyRuntimeDelta>("topology:runtime-delta", ({ payload}

) => {
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
       }

 : current.active_route,
     }

)  );
}


);
    return () => {
      window.clearInterval(timer);
      void unlistenPromise.then((unlisten) => unlisten()  );
}


;
 }

, [refresh]);

  useEffect(() => {
    if (!actionFeedback) return;
    const timer = window.setTimeout(() => setActionFeedback(""), 1600);
    return () => window.clearTimeout(timer);
 }

, [actionFeedback]);

  const availability = snapshot.resources_total
    ? Math.round((snapshot.resources_available / snapshot.resources_total) * 100)
    : 0;
  const unavailable = Math.max(0, snapshot.resources_total - snapshot.resources_available);
  const poolPercent = snapshot.pools.length
    ? Math.round((snapshot.enabled_pool_count / snapshot.pools.length) * 100)
    : 0;
  const selectedTokens = {
    today: snapshot.today_tokens,
    "7d": snapshot.seven_day_tokens,
    month: snapshot.month_tokens,
    all: snapshot.cumulative_tokens,
 }

[range];
  const tokenPercent = snapshot.cumulative_tokens > 0
    ? Math.min(100, Math.round((selectedTokens / snapshot.cumulative_tokens) * 100))
    : 0;
  // 指标数字滚动（与 Token 页 Hero 一致）：可用率 / 路由池（Tokens 用里程表翻牌）
  const animatedAvailability = useCountUp(availability);
  const animatedPoolCount = useCountUp(snapshot.enabled_pool_count);
  const requestSeries = useMemo(() => snapshot.activity.map((point) => point.requests), [snapshot.activity]);
  const maxHour = useMemo(() => {
    if (!snapshot.activity.length) return null;
    return snapshot.activity.reduce((best, point) => point.requests > best.requests ? point : best, snapshot.activity[0]);
 }

, [snapshot.activity]);
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
    return { enabledPools, protocolNames, providers, activePool, activeProvider}

;
 }

, [snapshot.pools, snapshot.active_route]);

  const toggleGateway = async () => {
    if (toggling) return;
    setToggling(true);
    try {
      if (snapshot.gateway_running) await stopProxy();
      else await startProxy();
      await refresh();
      setActionFeedback(snapshot.gateway_running ? "网关已停止" : "网关已启动"  );
}


 finally {
      setToggling(false  );
}



 }

;

  // Deep link: clicking the active path opens the command center with the
  // provider (or the pool when no provider is routed yet) focused.
  const openPathInDashboard = () => {
    const providerId = snapshot.active_route?.provider_id || topology.activeProvider?.id;
    if (providerId) void openPoolGateFromTray("dashboard", `provider-${providerId}`);
    else void openPoolGateFromTray("dashboard");
 }

;

  const protocolValue = snapshot.active_route?.protocol || topology.protocolNames[0] || "未配置";
  const poolValue = snapshot.active_route?.pool_name || topology.activePool?.name || "暂无路由池";
  const providerValue = snapshot.active_route?.provider_name || topology.activeProvider?.name || "暂无厂商";
  const routeActive = snapshot.active_route?.status === "active";

  return (
    <main className={`pg-tray-card pg-tray-glass ${loading ? "is-loading" : ""}`}>
      <header className="pg-tg-header">
        <div className="pg-tg-brand">
          <img src="/poolgate-icon.png" alt="" />
          <div className="pg-tg-title-group">
            <div className="pg-tg-title-row">
              <h1>PoolGate</h1>
              <span
                className={`pg-tg-online ${snapshot.gateway_running ? "on" : ""}`}
                title={snapshot.gateway_running ? "网关在线" : "网关离线"}
              >
                <i />{snapshot.gateway_running ? "在线" : "离线"}
              </span>
            </div>

          </div>
        </div>
        <div className="pg-tg-segmented" role="tablist">
          {rangeOptions.map((option) => (
            <button key={option.key} role="tab" className={range === option.key ? "active" : ""} onClick={() => setRange(option.key)}>
              {option.label}
            </button>
          ))}
        </div>
      </header>

      <div className="pg-tg-scroll">
        <section className="pg-tg-card pg-tg-topology" aria-label="实时路由拓扑">
          <div className="pg-tg-card-head">
            <strong>实时路由拓扑</strong>
            <span className={`pg-tg-head-status ${snapshot.gateway_running ? "on" : ""}`}>
              <i />{snapshot.gateway_running ? "网关在线" : "网关离线"}
            </span>
            <code>127.0.0.1:{snapshot.port}</code>
          </div>
          <button className="pg-tg-route-list" onClick={openPathInDashboard} title="打开指挥中心并定位当前路径" aria-label="打开指挥中心并定位当前路径">
            <span className="pg-tg-route-item">
              <i className="pg-tg-route-icon"><Waypoints size={16} /></i>
              <span className="pg-tg-route-label">Gateway</span>
              <span className="pg-tg-route-value">127.0.0.1:{snapshot.port}</span>
              <i className={`pg-tg-route-dot ${snapshot.gateway_running ? "on" : ""}`} />
            </span>
            <span className="pg-tg-route-item">
              <i className="pg-tg-route-icon"><Network size={16} /></i>
              <span className="pg-tg-route-label">Protocol · {snapshot.topology.protocol_count}</span>
              <span className="pg-tg-route-value" title={protocolValue}>{protocolValue}</span>
              <i className={`pg-tg-route-dot ${routeActive ? "on" : ""}`} />
            </span>
            <span className="pg-tg-route-item">
              <i className="pg-tg-route-icon"><Boxes size={16} /></i>
              <span className="pg-tg-route-label">Route Pool · {snapshot.topology.pool_count}</span>
              <span className="pg-tg-route-value" title={poolValue}>{poolValue}</span>
              <i className={`pg-tg-route-dot ${routeActive ? "on" : ""}`} />
            </span>
            <span className="pg-tg-route-item">
              <i className="pg-tg-route-icon"><ServerCog size={16} /></i>
              <span className="pg-tg-route-label">Provider · {snapshot.topology.provider_count}</span>
              <span className="pg-tg-route-value" title={providerValue}>{providerValue}</span>
              <i className={`pg-tg-route-dot ${routeActive ? "on" : ""}`} />
            </span>
          </button>
          <div className="pg-tg-route-foot">
            <span><Activity size={12} />当前活动路由</span>
            <b>{snapshot.active_route?.active_requests || 0} 个请求 · {snapshot.active_connections} 个网关连接</b>
          </div>
        </section>

        <section className="pg-tg-metrics" aria-label="资源使用">
          <div className="pg-tg-metrics-title">资源使用</div>
          <div className="pg-tg-metric-grid">
            <div className="pg-tg-metric-card">
              <div className="pg-tg-metric-info">
                <span>资源可用率</span>
                <strong>{animatedAvailability}%</strong>
                <small>{snapshot.resources_available}/{snapshot.resources_total} 可用</small>
              </div>
              <RingProgress value={availability} size={34} stroke={4} color="var(--tg-primary)" />
            </div>
            <div className="pg-tg-metric-card">
              <div className="pg-tg-metric-info">
                <span>可用路由池</span>
                <strong>{animatedPoolCount}</strong>
                <small>共 {snapshot.pools.length} 个</small>
              </div>
              <RingProgress value={poolPercent} size={34} stroke={4} color="var(--tg-success)" />
            </div>
            <div className="pg-tg-metric-card">
              <div className="pg-tg-metric-info">
                <span>{rangeOptions.find((item) => item.key === range)?.label} Tokens</span>
                <strong><RollingNumber value={selectedTokens} format={formatTokensZh} /></strong>
                <small>{snapshot.total_requests.toLocaleString("zh-CN")} 次请求</small>
              </div>
              <RingProgress value={tokenPercent} size={34} stroke={4} color="var(--tg-primary)" />
            </div>
          </div>
        </section>

        <section className="pg-tg-card pg-tg-activity" aria-label="今日请求活跃度">
          <div className="pg-tg-card-head">
            <strong>{activityView === "requests" ? "网关请求趋势" : "网关活动"}</strong>
            <span className="pg-tg-activity-meta">
              {activityView === "requests"
                ? `成功率 ${snapshot.success_rate.toFixed(1)}% · 本小时 ${snapshot.current_tps.toFixed(2)} req/s`
                : `活跃 ${snapshot.tokens_heatmap.filter((day) => day.tokens > 0).length} 天`}
            </span>
            <div className="pg-tg-toggle">
              <button className={activityView === "requests" ? "active" : ""} onClick={() => setActivityView("requests")}>请求</button>
              <button className={activityView === "tokens" ? "active" : ""} onClick={() => setActivityView("tokens")}>Tokens</button>
            </div>
          </div>
          {activityView === "requests" ? (
            <div className="pg-tg-chart">
              <Sparkline values={requestSeries} area />
              <div className="pg-tg-axis"><span>0 时</span><span>6 时</span><span>12 时</span><span>18 时</span><span>23 时</span></div>
            </div>
          ) : (
            <div className="pg-tg-chart pg-tg-heatmap">
              {/* 小方块样式与 Token 托盘活动一致：365 天窗口锚定今天、14px 格子、默认停靠最右 */}
              {snapshot.tokens_heatmap.length ? (
                <ActivityHeatmap
                  data={snapshot.tokens_heatmap}
                  cellSize={10}
                  gap={2}
                  rounded={2}
                  align="end"
                  windowEnd={todayKey()}
                  windowDays={365}
                  compact
                  fill
                  minCellSize={14}
                  maxCellSize={15}
                  showLegend={false}
                />
              ) : (
                <span className="pg-tg-empty-inline">暂无活跃记录</span>
              )}
            </div>
          )}
        </section>
      </div>

      <footer className="pg-tg-footer">
        <div className="pg-tg-footer-status">
          <span>{actionFeedback || `${snapshot.updated_at} 更新`}</span>
          <small>{unavailable ? `${unavailable} 个资源需关注` : `127.0.0.1:${snapshot.port}`}{maxHour?.requests ? ` · 峰值 ${maxHour.hour} 时` : ""}</small>
        </div>
        <div className="pg-tg-actions">
          <button className="pg-tg-icon-btn" onClick={() => { window.location.search = "view=token-monitor";}

} title="切换到 Token Monitor 托盘" aria-label="切换到 Token Monitor 托盘">
            <Coins size={17} /><span>Token</span>
          </button>
          <button className="pg-tg-icon-btn" onClick={() => void openPoolGateFromTray("settings")} title="打开网关配置" aria-label="打开网关配置">
            <Settings2 size={17} /><span>配置</span>
          </button>
          <button className={`pg-tg-icon-btn power ${snapshot.gateway_running ? "running" : ""}`} onClick={() => void toggleGateway()} disabled={toggling} title={snapshot.gateway_running ? "停止网关" : "启动网关"} aria-label={snapshot.gateway_running ? "停止网关" : "启动网关"}>
            <Power size={17} /><span>{snapshot.gateway_running ? "停止" : "启动"}</span>
          </button>
          <button className="pg-tg-icon-btn danger" onClick={() => void quitPoolGateFromTray()} title="退出 PoolGate" aria-label="退出 PoolGate">
            <LogOut size={17} /><span>退出</span></button>
        </div>
      </footer>
    </main>
  );
}