import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  AlertTriangle,
  Boxes,
  ChevronRight,
  CircleGauge,
  KeyRound,
  Maximize2,
  Minus,
  Network,
  Plus,
  RefreshCw,
  RotateCcw,
  ServerCog,
  Waypoints,
  X,
} from "lucide-react";
import { getProviderTopologyDetail, type ProviderTopologyDetail, type RouteTopology } from "@/lib/tauri-commands";

type Point = { x: number; y: number };
type MeasuredEdge = RouteTopology["edges"][number] & { d: string };

type Props = {
  data?: RouteTopology;
  loading: boolean;
  error: boolean;
  onRefresh: () => void;
};

const layerLabels = ["Gateway", "Protocol", "Route Pool", "Upstream Provider"];

function protocolLabel(protocol: string) {
  switch (protocol.toLowerCase()) {
    case "responses": return "OpenAI Responses";
    case "anthropic": return "Anthropic Messages";
    case "gemini": return "Google Gemini";
    case "both": return "Multi Protocol";
    case "chat": return "OpenAI Chat";
    default: return protocol || "Compatible";
  }
}

function strategyLabel(strategy?: string) {
  switch (strategy) {
    case "least_used": return "最少使用";
    case "priority": return "优先级";
    case "random": return "随机";
    case "cost_optimized": return "成本优先";
    default: return "轮询";
  }
}

function metric(value = 0) {
  return Math.max(0, value).toLocaleString("zh-CN");
}

function pathBetween(from: Point, to: Point) {
  const bend = Math.max(42, Math.abs(to.x - from.x) * 0.42);
  return `M ${from.x} ${from.y} C ${from.x + bend} ${from.y}, ${to.x - bend} ${to.y}, ${to.x} ${to.y}`;
}

function statusForProvider(provider: RouteTopology["providers"][number]) {
  if (!provider.enabled) return "disabled";
  if (provider.healthy_account_count === 0) return "warning";
  return "healthy";
}

function RouteNode({
  id,
  kind,
  title,
  subtitle,
  stats,
  detail,
  status,
  selected,
  active,
  onClick,
}: {
  id: string;
  kind: "gateway" | "protocol" | "pool" | "provider";
  title: string;
  subtitle: string;
  stats: string;
  detail: string;
  status: string;
  selected: boolean;
  active: boolean;
  onClick: () => void;
}) {
  const Icon = kind === "gateway" ? Waypoints : kind === "protocol" ? Network : kind === "pool" ? Boxes : ServerCog;
  return (
    <button
      data-route-node={id}
      className={`pg-rt-node ${kind} status-${status} ${selected ? "selected" : ""} ${active ? "active-route" : ""}`}
      onClick={onClick}
      title={`${title} · ${subtitle}`}
    >
      <span className="pg-rt-port in" data-route-port={`${id}:in`} />
      <span className="pg-rt-node-head">
        <span className="pg-rt-node-icon"><Icon size={15} /></span>
        <span className="pg-rt-node-title"><strong>{title}</strong><small>{subtitle}</small></span>
        <i className="pg-rt-health" />
      </span>
      <span className="pg-rt-node-stats"><b>{stats}</b><span>{detail}</span></span>
      <span className="pg-rt-port out" data-route-port={`${id}:out`} />
    </button>
  );
}

function EmptyInspector() {
  return (
    <div className="pg-rt-inspector-empty">
      <Waypoints size={26} />
      <strong>实时路由总览</strong>
      <span>点击网关、协议、路由池或上游厂商节点查看详情。账号仅在厂商详情中展示。</span>
    </div>
  );
}

function ProviderDetail({ detail, loading }: { detail?: ProviderTopologyDetail; loading: boolean }) {
  if (loading) return <div className="pg-rt-detail-loading"><RefreshCw size={16} className="animate-spin" />正在加载厂商账号</div>;
  if (!detail) return <EmptyInspector />;
  const total = detail.traffic.total_requests;
  const successRate = total ? detail.traffic.success_count / total * 100 : 0;
  return (
    <div className="pg-rt-provider-detail">
      <div className="pg-rt-detail-hero">
        <div className="pg-rt-detail-icon"><ServerCog size={18} /></div>
        <div><span>Upstream Provider</span><strong>{detail.name}</strong><small>{detail.base_url_masked}</small></div>
      </div>
      <div className="pg-rt-detail-grid">
        <div><span>请求</span><strong>{metric(total)}</strong></div>
        <div><span>成功率</span><strong>{total ? `${successRate.toFixed(1)}%` : "--"}</strong></div>
        <div><span>Tokens</span><strong>{metric(detail.traffic.total_tokens)}</strong></div>
        <div><span>账号</span><strong>{detail.accounts.length}</strong></div>
      </div>
      <div className="pg-rt-detail-meta"><span>{protocolLabel(detail.protocol)}</span><span>{detail.models.length} models</span><span>{detail.enabled ? "已启用" : "已停用"}</span></div>
      <div className="pg-rt-account-head"><strong>关联账号</strong><span>{detail.accounts.length}</span></div>
      <div className="pg-rt-account-list">
        {detail.accounts.map((account) => (
          <div key={account.id} className="pg-rt-account-row">
            <div className="pg-rt-account-main"><i className={account.health_status} /><span><strong>{account.name}</strong><small>{account.email_masked || account.plan_type || account.credential_type}{!account.routable ? " · 待适配" : ""}</small></span></div>
            <div className="pg-rt-account-quota"><b>{account.quota_remaining_percent == null ? "未提供" : `${Math.round(account.quota_remaining_percent)}%`}</b><small>配额剩余</small></div>
            <div className="pg-rt-account-concurrency"><b>{account.concurrency_active}/{account.concurrency_limit}</b><small>{account.queued_requests ? `排队 ${account.queued_requests}` : "并发"}</small></div>
          </div>
        ))}
        {!detail.accounts.length && <div className="pg-rt-account-empty"><KeyRound size={18} /><strong>暂无关联账号</strong><span>请在模型供应商中为该厂商导入可路由账号。</span></div>}
      </div>
    </div>
  );
}

export default function RealtimeRouteTopology({ data, loading, error, onRefresh }: Props) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLDivElement>(null);
  const [selected, setSelected] = useState("gateway");
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [drag, setDrag] = useState<{ x: number; y: number; panX: number; panY: number } | null>(null);
  const [edges, setEdges] = useState<MeasuredEdge[]>([]);
  const [providerDetail, setProviderDetail] = useState<ProviderTopologyDetail>();
  const [detailLoading, setDetailLoading] = useState(false);

  const poolsByProtocol = useMemo(() => {
    const map = new Map<string, RouteTopology["pools"]>();
    data?.protocols.forEach((protocol) => map.set(protocol.id, data.pools.filter((pool) => protocol.pool_ids.includes(pool.id))));
    return map;
  }, [data]);
  const runtimePath = data?.runtime.active_paths[0];
  const active = runtimePath ? {
    protocol: runtimePath.protocol,
    pool_id: runtimePath.pool_id,
    provider_id: runtimePath.provider_id,
    active_requests: runtimePath.active_requests,
  } : undefined;
  const displayRoute = active || data?.active_route;
  const activeNodeCounts = useMemo(
    () => new Map(data?.runtime.node_deltas.map((node) => [node.id, node.active_concurrency]) || []),
    [data?.runtime.node_deltas],
  );

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const command = event.metaKey || event.ctrlKey;
      if (command && event.key === "+") { event.preventDefault(); setZoom((value) => Math.min(1.8, value + .1)); }
      if (command && event.key === "-") { event.preventDefault(); setZoom((value) => Math.max(.2, value - .1)); }
      if (command && event.key === "0") { event.preventDefault(); setZoom(1); setPan({ x: 0, y: 0 }); }
      if (event.key === "Escape") void selectNode("gateway");
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  });

  const measure = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || !data) return;
    const canvasRect = canvas.getBoundingClientRect();
    const endpoint = (id: string, side: "in" | "out") => {
      const port = canvas.querySelector<HTMLElement>(`[data-route-port="${id}:${side}"]`);
      if (!port) return null;
      const rect = port.getBoundingClientRect();
      return { x: (rect.left - canvasRect.left + rect.width / 2) / zoom, y: (rect.top - canvasRect.top + rect.height / 2) / zoom };
    };
    setEdges(data.edges.flatMap((edge) => {
      const from = endpoint(edge.source, "out");
      const to = endpoint(edge.target, "in");
      return from && to ? [{ ...edge, d: pathBetween(from, to) }] : [];
    }));
  }, [data, zoom]);

  useLayoutEffect(() => {
    measure();
    const canvas = canvasRef.current;
    if (!canvas) return;
    const observer = new ResizeObserver(measure);
    observer.observe(canvas);
    canvas.querySelectorAll<HTMLElement>("[data-route-node]").forEach((node) => observer.observe(node));
    return () => observer.disconnect();
  }, [measure]);

  const selectNode = async (id: string) => {
    setSelected(id);
    if (!id.startsWith("provider-")) {
      setProviderDetail(undefined);
      return;
    }
    setDetailLoading(true);
    try {
      setProviderDetail(await getProviderTopologyDetail(id.replace("provider-", "")));
    } finally {
      setDetailLoading(false);
    }
  };

  const fitView = () => {
    const viewport = viewportRef.current;
    const canvas = canvasRef.current;
    if (!viewport || !canvas) return;
    const next = Math.min(1, (viewport.clientWidth - 32) / canvas.scrollWidth, (viewport.clientHeight - 32) / canvas.scrollHeight);
    setZoom(Math.max(.58, next));
    setPan({ x: 0, y: 0 });
  };

  if (loading) return <div className="pg-rt-state"><RefreshCw size={20} className="animate-spin" /><strong>正在构建四层路由拓扑</strong><span>加载网关、协议、路由池与上游厂商</span></div>;
  if (error || !data) return <div className="pg-rt-state error"><AlertTriangle size={21} /><strong>实时路由拓扑加载失败</strong><button onClick={onRefresh}>重新加载</button></div>;

  const providerById = new Map(data.providers.map((provider) => [provider.id, provider]));
  return (
    <div className="pg-rt-shell">
      <div className="pg-rt-toolbar">
        <div className="pg-rt-layer-summary">{layerLabels.map((label, index) => <span key={label}><b>{index === 0 ? 1 : index === 1 ? data.protocols.length : index === 2 ? data.pools.length : data.providers.length}</b>{label}</span>)}</div>
        <div className="pg-rt-tools">
          <button onClick={() => setZoom((value) => Math.min(1.8, value + .1))} title="放大"><Plus size={14} /></button>
          <span>{Math.round(zoom * 100)}%</span>
          <button onClick={() => setZoom((value) => Math.max(.2, value - .1))} title="缩小"><Minus size={14} /></button>
          <button onClick={fitView} title="适配画布"><Maximize2 size={14} /></button>
          <button onClick={() => { setZoom(1); setPan({ x: 0, y: 0 }); }} title="重置视图"><RotateCcw size={14} /></button>
        </div>
      </div>
      <div className="pg-rt-content">
        <div
          ref={viewportRef}
          className={`pg-rt-viewport ${drag ? "dragging" : ""}`}
          onPointerDown={(event) => {
            if ((event.target as HTMLElement).closest("button")) return;
            setDrag({ x: event.clientX, y: event.clientY, panX: pan.x, panY: pan.y });
            event.currentTarget.setPointerCapture(event.pointerId);
          }}
          onPointerMove={(event) => drag && setPan({ x: drag.panX + event.clientX - drag.x, y: drag.panY + event.clientY - drag.y })}
          onPointerUp={() => setDrag(null)}
          onDoubleClick={(event) => { if (!(event.target as HTMLElement).closest("button")) fitView(); }}
          onWheel={(event) => {
            if (!(event.metaKey || event.ctrlKey)) return;
            event.preventDefault();
            setZoom((value) => Math.max(.2, Math.min(1.8, value + (event.deltaY < 0 ? .08 : -.08))));
          }}
        >
          <div ref={canvasRef} className="pg-rt-canvas" style={{ transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})` }}>
            <svg className="pg-rt-edges" aria-hidden="true">
              {edges.map((edge) => <path id={edge.id} key={`${edge.id}-base`} d={edge.d} className={`pg-rt-edge-base status-${edge.status}`} />)}
              {edges.filter((edge) => edge.active).map((edge) => <path key={`${edge.id}-active`} d={edge.d} className="pg-rt-edge-active" style={{ strokeWidth: Math.min(5, 2.4 + edge.active_requests * .35) }} />)}
              {edges.filter((edge) => edge.active_requests > 0).map((edge) => <text key={`${edge.id}-count`} className="pg-rt-edge-count"><textPath href={`#${edge.id}`}>{edge.active_requests} active</textPath></text>)}
            </svg>
            <section className="pg-rt-layer gateway-layer"><span className="pg-rt-layer-label">Gateway</span><RouteNode id="gateway" kind="gateway" title={data.gateway.name} subtitle={data.gateway.address} stats={`${data.gateway.active_connections} active`} detail={data.gateway.running ? "RUNNING" : "STOPPED"} status={data.gateway.running ? "healthy" : "offline"} selected={selected === "gateway"} active={(activeNodeCounts.get("gateway") || 0) > 0} onClick={() => void selectNode("gateway")} /></section>
            <section className="pg-rt-layer"><span className="pg-rt-layer-label">Protocol · {data.protocols.length}</span><div className="pg-rt-node-list">{data.protocols.map((protocol) => <RouteNode key={protocol.id} id={protocol.id} kind="protocol" title={protocol.name} subtitle={protocolLabel(protocol.protocol)} stats={`${protocol.pool_ids.length} pools`} detail={`${metric(protocol.request_count)} req`} status={protocol.enabled ? "healthy" : "disabled"} selected={selected === protocol.id} active={(activeNodeCounts.get(protocol.id) || 0) > 0} onClick={() => void selectNode(protocol.id)} />)}</div></section>
            <section className="pg-rt-layer"><span className="pg-rt-layer-label">Route Pools · {data.pools.length}</span><div className="pg-rt-node-list">{data.pools.map((pool) => <RouteNode key={pool.id} id={`pool-${pool.id}`} kind="pool" title={pool.name} subtitle={`${protocolLabel(pool.protocol)} · ${strategyLabel(pool.strategy)}`} stats={`${pool.healthy_resource_count}/${pool.resource_count} resources`} detail={`${metric(pool.traffic.total_requests)} req · ${pool.model_count} models`} status={pool.enabled ? (pool.healthy_resource_count ? "healthy" : "warning") : "disabled"} selected={selected === `pool-${pool.id}`} active={(activeNodeCounts.get(`pool-${pool.id}`) || 0) > 0} onClick={() => void selectNode(`pool-${pool.id}`)} />)}</div></section>
            <section className="pg-rt-layer"><span className="pg-rt-layer-label">Upstream Providers · {data.providers.length}</span><div className="pg-rt-node-list">{data.providers.map((provider) => <RouteNode key={provider.id} id={`provider-${provider.id}`} kind="provider" title={provider.name} subtitle={protocolLabel(provider.protocol)} stats={`${provider.healthy_account_count}/${provider.account_count} healthy`} detail={`${metric(provider.traffic.total_requests)} req · ${metric(provider.traffic.total_tokens)} tok`} status={statusForProvider(provider)} selected={selected === `provider-${provider.id}`} active={(activeNodeCounts.get(`provider-${provider.id}`) || 0) > 0} onClick={() => void selectNode(`provider-${provider.id}`)} />)}</div></section>
          </div>
        </div>
        <aside className="pg-rt-inspector">
          <div className="pg-rt-inspector-head"><div><span>Node Inspector</span><strong>{selected === "gateway" ? "网关总览" : selected.startsWith("provider-") ? providerById.get(selected.replace("provider-", ""))?.name : "路由节点详情"}</strong></div>{selected !== "gateway" && <button onClick={() => void selectNode("gateway")} title="返回总览"><X size={14} /></button>}</div>
          {selected.startsWith("provider-") ? <ProviderDetail detail={providerDetail} loading={detailLoading} /> : selected === "gateway" ? <div className="pg-rt-gateway-detail"><div className="pg-rt-detail-hero"><div className="pg-rt-detail-icon"><CircleGauge size={18} /></div><div><span>Gateway Runtime</span><strong>{data.gateway.running ? "网关运行中" : "网关已停止"}</strong><small>{data.gateway.address}</small></div></div><div className="pg-rt-detail-grid"><div><span>协议</span><strong>{data.protocols.length}</strong></div><div><span>路由池</span><strong>{data.pools.length}</strong></div><div><span>厂商</span><strong>{data.providers.length}</strong></div><div><span>连接</span><strong>{data.gateway.active_connections}</strong></div></div>{displayRoute ? <div className="pg-rt-current-path"><div><Activity size={13} /><strong>当前最活跃路径</strong><span className={active ? "active" : data.active_route?.status}>{active ? `${active.active_requests} active` : data.active_route?.status}</span></div><p>Gateway <ChevronRight size={11} /> {protocolLabel(displayRoute.protocol)} <ChevronRight size={11} /> {data.pools.find((pool) => pool.id === displayRoute.pool_id)?.name || displayRoute.pool_id} <ChevronRight size={11} /> {providerById.get(displayRoute.provider_id)?.name || displayRoute.provider_id}</p></div> : <div className="pg-rt-current-path empty"><Network size={16} /><span>等待新的 Agent 请求，路径将在选路后实时高亮。</span></div>}</div> : <EmptyInspector />}
          <div className="pg-rt-legend"><span><i className="healthy" />正常</span><span><i className="warning" />告警</span><span><i className="failed" />故障</span><span><i className="active" />当前路径</span></div>
        </aside>
      </div>
    </div>
  );
}
