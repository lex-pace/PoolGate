import { useEffect, useState } from "react";
import {
  Activity,
  AlertTriangle,
  Boxes,
  ChevronRight,
  CircleGauge,
  KeyRound,
  Network,
  RefreshCw,
  ServerCog,
  Waypoints,
  X,
} from "lucide-react";
import {
  getProviderTopologyDetail,
  type ProviderTopologyDetail,
  type RouteTopology,
} from "@/lib/tauri-commands";
import type { TopologySelection } from "./types";
import { HEALTH_META } from "./types";
import { NODE_ID_PREFIX_POOL, NODE_ID_PREFIX_PROVIDER, poolNodeId, providerNodeId } from "./graph";

type Props = {
  data: RouteTopology;
  selection: TopologySelection;
  onSelect: (selection: TopologySelection) => void;
  onOpenDashboard: () => void;
  onClose?: () => void;
};

function metric(value: number | undefined, fallback = "未提供") {
  return typeof value === "number" && Number.isFinite(value) ? value.toLocaleString("zh-CN") : fallback;
}

function latency(value: number | undefined, unit = "ms") {
  return typeof value === "number" && Number.isFinite(value) ? `${Math.round(value)} ${unit}` : "未提供";
}

function pct(value: number | undefined) {
  return typeof value === "number" && Number.isFinite(value) ? `${value.toFixed(1)}%` : "未提供";
}

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

function DetailHero({ icon, kind, name, subtitle, health }: {
  icon: React.ReactNode;
  kind: string;
  name: string;
  subtitle?: string;
  health?: string;
}) {
  return (
    <div className="pg-tv-hero">
      <div className="pg-tv-hero-icon">{icon}</div>
      <div className="pg-tv-hero-text">
        <span>{kind}{health ? ` · ${HEALTH_META[health as keyof typeof HEALTH_META]?.label ?? health}` : ""}</span>
        <strong>{name}</strong>
        {subtitle && <small>{subtitle}</small>}
      </div>
    </div>
  );
}

function MetricGrid({ items }: { items: Array<{ label: string; value: string; tone?: string }> }) {
  return (
    <div className="pg-tv-metric-grid">
      {items.map((item) => (
        <div key={item.label}>
          <span>{item.label}</span>
          <strong style={item.tone ? { color: item.tone } : undefined}>{item.value}</strong>
        </div>
      ))}
    </div>
  );
}

function Overview({ data, onOpenDashboard }: Props) {
  const abnormal = data.providers.filter((p) => !p.enabled || p.healthy_account_count === 0).length
    + data.pools.filter((p) => !p.enabled || p.healthy_resource_count === 0).length;
  const totalRequests = data.pools.reduce((sum, pool) => sum + (pool.traffic.total_requests || 0), 0);
  return (
    <div className="pg-tv-detail-body">
      <div className="pg-tv-hero">
        <div className="pg-tv-hero-icon"><Waypoints size={18} /></div>
        <div className="pg-tv-hero-text">
          <span>路由总览</span>
          <strong>{data.gateway.running ? "网关运行中" : "网关已停止"}</strong>
          <small>{data.gateway.address}</small>
        </div>
      </div>
      <MetricGrid items={[
        { label: "协议", value: String(data.protocols.length) },
        { label: "路由池", value: String(data.pools.length) },
        { label: "上游厂商", value: String(data.providers.length) },
        { label: "活跃连接", value: metric(data.gateway.active_connections, "0") },
      ]} />
      <MetricGrid items={[
        { label: "总请求", value: metric(totalRequests, "0") },
        { label: "成功率", value: totalRequests ? `${((data.pools.reduce((s, p) => s + p.traffic.success_count, 0) / Math.max(1, totalRequests)) * 100).toFixed(1)}%` : "未提供" },
        { label: "异常节点", value: String(abnormal), tone: abnormal ? "var(--color-warn)" : "var(--color-ok)" },
        { label: "最近更新", value: data.updated_at ? new Date(data.updated_at).toLocaleTimeString("zh-CN", { hour12: false }) : "未提供" },
      ]} />
      {data.active_route && (
        <div className="pg-tv-current-path">
          <div><Activity size={13} /><strong>当前最活跃路径</strong><span className={data.active_route.status}>{data.active_route.status}</span></div>
          <p>
            {protocolLabel(data.active_route.protocol)}
            <ChevronRight size={11} />
            {data.pools.find((pool) => pool.id === data.active_route?.pool_id)?.name || data.active_route.pool_id}
            <ChevronRight size={11} />
            {data.providers.find((provider) => provider.id === data.active_route?.provider_id)?.name || data.active_route.provider_id}
          </p>
        </div>
      )}
      <div className="pg-tv-hint">
        <Network size={15} />
        <span>五层链路：网关 → 协议 → 路由池 → 上游厂商 → 账号。点击节点或连线查看实时详情；账号较多时自动折叠为汇总节点。</span>
      </div>
    </div>
  );
}

function GatewayDetail({ data }: { data: RouteTopology }) {
  const totalRequests = data.pools.reduce((sum, pool) => sum + (pool.traffic.total_requests || 0), 0);
  const successCount = data.pools.reduce((sum, pool) => sum + (pool.traffic.success_count || 0), 0);
  const running = data.gateway.running;
  return (
    <div className="pg-tv-detail-body">
      <DetailHero
        icon={<CircleGauge size={18} />}
        kind="Gateway Runtime"
        name={running ? "网关运行中" : "网关已停止"}
        subtitle={data.gateway.address}
        health={running ? "healthy" : "fault"}
      />
      <MetricGrid items={[
        { label: "协议", value: String(data.protocols.length) },
        { label: "路由池", value: String(data.pools.length) },
        { label: "上游厂商", value: String(data.providers.length) },
        { label: "活跃连接", value: metric(data.gateway.active_connections, "0") },
      ]} />
      <MetricGrid items={[
        { label: "总请求", value: metric(totalRequests, "0") },
        { label: "成功率", value: totalRequests ? `${((successCount / totalRequests) * 100).toFixed(1)}%` : "未提供" },
        { label: "错误", value: metric(data.pools.reduce((s, p) => s + p.traffic.error_count, 0), "0") },
        { label: "平均延迟", value: latency(avgLatency(data)) },
      ]} />
      <div className="pg-tv-hint"><Activity size={14} /><span>网关统计按最终客户端请求聚合；厂商与账号按真实上游 attempt 统计。</span></div>
    </div>
  );
}

function avgLatency(data: RouteTopology) {
  const pools = data.pools.filter((pool) => pool.traffic.total_requests > 0);
  if (!pools.length) return undefined;
  const weighted = pools.reduce((sum, pool) => sum + (pool.traffic.avg_latency_ms || 0) * pool.traffic.total_requests, 0);
  const total = pools.reduce((sum, pool) => sum + pool.traffic.total_requests, 0);
  return total ? weighted / total : undefined;
}

function ProtocolDetail({ data, id, onSelect }: { data: RouteTopology; id: string; onSelect: (selection: TopologySelection) => void }) {
  const protocol = data.protocols.find((item) => item.id === id);
  if (!protocol) return <div className="pg-tv-detail-body pg-tv-empty">协议不存在</div>;
  const pools = data.pools.filter((pool) => protocol.pool_ids.includes(pool.id));
  const traffic = protocol.traffic;
  return (
    <div className="pg-tv-detail-body">
      <DetailHero
        icon={<Network size={18} />}
        kind="Protocol"
        name={protocol.name}
        subtitle={protocolLabel(protocol.protocol)}
        health={protocol.enabled ? "healthy" : "disabled"}
      />
      <MetricGrid items={[
        { label: "关联路由池", value: String(pools.length) },
        { label: "请求", value: metric(traffic.total_requests, "0") },
        { label: "成功率", value: traffic.total_requests ? pct(traffic.success_rate) : "未提供" },
        { label: "平均延迟", value: latency(traffic.avg_latency_ms) },
        { label: "状态", value: protocol.enabled ? "已启用" : "已禁用" },
      ]} />
      <MetricGrid items={[
        { label: "Tokens", value: metric(traffic.total_tokens, "0") },
        { label: "成本", value: typeof traffic.total_cost === "number" ? `$${traffic.total_cost.toFixed(4)}` : "未提供" },
        { label: "错误", value: metric(traffic.error_count, "0") },
        { label: "输入 Tokens", value: metric(traffic.input_tokens, "0") },
      ]} />
      <div className="pg-tv-list-head"><strong>关联路由池</strong><span>{pools.length}</span></div>
      <div className="pg-tv-list">
        {pools.map((pool) => (
          <button key={pool.id} className="pg-tv-list-row" onClick={() => onSelect({ kind: "node", id: poolNodeId(pool.id), entityId: pool.id, nodeKind: "pool" })}>
            <span className="pg-tv-list-main"><i className={`health-${pool.enabled ? (pool.healthy_resource_count ? "healthy" : "warning") : "disabled"}`} /><span><strong>{pool.name}</strong><small>{strategyLabel(pool.strategy)} · {pool.healthy_resource_count}/{pool.resource_count} 资源</small></span></span>
            <span className="pg-tv-list-meta"><b>{metric(pool.traffic.total_requests, "0")} req</b><small>{pool.model_count} models</small></span>
            <ChevronRight size={13} />
          </button>
        ))}
        {!pools.length && <div className="pg-tv-list-empty">暂无关联路由池</div>}
      </div>
    </div>
  );
}

function PoolDetail({ data, id, onSelect }: { data: RouteTopology; id: string; onSelect: (selection: TopologySelection) => void }) {
  const pool = data.pools.find((item) => item.id === id);
  if (!pool) return <div className="pg-tv-detail-body pg-tv-empty">路由池不存在</div>;
  const providers = data.providers.filter((provider) => pool.provider_ids.includes(provider.id));
  const traffic = pool.traffic;
  const health = !pool.enabled ? "disabled" : pool.healthy_resource_count === 0 ? "warning" : "healthy";
  return (
    <div className="pg-tv-detail-body">
      <DetailHero
        icon={<Boxes size={18} />}
        kind="Route Pool"
        name={pool.name}
        subtitle={`${protocolLabel(pool.protocol)} · ${strategyLabel(pool.strategy)}`}
        health={health}
      />
      <MetricGrid items={[
        { label: "资源", value: `${pool.healthy_resource_count}/${pool.resource_count}` },
        { label: "模型", value: String(pool.model_count) },
        { label: "厂商", value: String(providers.length) },
        { label: "请求", value: metric(traffic.total_requests, "0") },
      ]} />
      <MetricGrid items={[
        { label: "成功率", value: traffic.total_requests ? pct(traffic.success_rate) : "未提供" },
        { label: "平均延迟", value: latency(traffic.avg_latency_ms) },
        { label: "Tokens", value: metric(traffic.total_tokens, "0") },
        { label: "成本", value: typeof traffic.total_cost === "number" ? `$${traffic.total_cost.toFixed(4)}` : "未提供" },
      ]} />
      <div className="pg-tv-list-head"><strong>上游厂商</strong><span>{providers.length}</span></div>
      <div className="pg-tv-list">
        {providers.map((provider) => (
          <button key={provider.id} className="pg-tv-list-row" onClick={() => onSelect({ kind: "node", id: providerNodeId(provider.id), entityId: provider.id, nodeKind: "provider" })}>
            <span className="pg-tv-list-main"><i className={`health-${!provider.enabled ? "disabled" : provider.healthy_account_count ? "healthy" : "warning"}`} /><span><strong>{provider.name}</strong><small>{protocolLabel(provider.protocol)} · {provider.healthy_account_count}/{provider.account_count} 健康账号</small></span></span>
            <span className="pg-tv-list-meta"><b>{metric(provider.traffic.total_requests, "0")} req</b><small>{metric(provider.traffic.total_tokens, "0")} tok</small></span>
            <ChevronRight size={13} />
          </button>
        ))}
        {!providers.length && <div className="pg-tv-list-empty">该路由池尚未关联厂商</div>}
      </div>
    </div>
  );
}

function ProviderDetail({ detail, loading }: { detail?: ProviderTopologyDetail; loading: boolean }) {
  if (loading) return <div className="pg-tv-detail-body pg-tv-loading"><RefreshCw size={17} className="animate-spin" />正在加载厂商账号</div>;
  if (!detail) return <div className="pg-tv-detail-body pg-tv-empty">暂无厂商详情</div>;
  const traffic = detail.traffic;
  const accounts = detail.accounts || [];
  const healthy = accounts.filter((account) => account.health_status === "healthy" || (account.routable && account.health_status !== "error")).length;
  return (
    <div className="pg-tv-detail-body">
      <DetailHero
        icon={<ServerCog size={18} />}
        kind="Upstream Provider"
        name={detail.name}
        subtitle={detail.base_url_masked}
        health={detail.enabled ? "healthy" : "disabled"}
      />
      <MetricGrid items={[
        { label: "请求(attempt)", value: metric(traffic.total_requests, "0") },
        { label: "成功率", value: traffic.total_requests ? pct(traffic.success_rate) : "未提供" },
        { label: "平均延迟", value: latency(traffic.avg_latency_ms) },
        { label: "P95", value: latency(traffic.p95_latency_ms) },
      ]} />
      <MetricGrid items={[
        { label: "Tokens", value: metric(traffic.total_tokens, "0") },
        { label: "成本", value: typeof traffic.total_cost === "number" ? `$${traffic.total_cost.toFixed(4)}` : "未提供" },
        { label: "健康账号", value: `${healthy}/${accounts.length}` },
        { label: "TTFT", value: latency(traffic.avg_ttft_ms) },
      ]} />
      <div className="pg-tv-account-head"><strong>关联账号</strong><span>{accounts.length}</span></div>
      <div className="pg-tv-account-list">
        {accounts.map((account) => {
          const healthStatus = !account.routable ? "待适配" : account.health_status === "error" || account.status === "exhausted" || account.status === "token_expired" ? "故障" : account.quota_remaining_percent != null && account.quota_remaining_percent < 15 ? "告警" : "正常";
          const healthTone = !account.routable ? "warning" : account.health_status === "error" || account.status === "exhausted" || account.status === "token_expired" ? "fault" : account.quota_remaining_percent != null && account.quota_remaining_percent < 15 ? "warning" : "healthy";
          return (
            <div key={account.id} className="pg-tv-account-row">
              <div className="pg-tv-account-main"><i className={`health-${healthTone}`} /><span><strong>{account.name}</strong><small>{account.email_masked || account.plan_type || account.credential_type}{!account.routable ? " · 待适配" : ""}</small></span></div>
              <div className="pg-tv-account-cell"><b>{account.quota_remaining_percent == null ? "未提供" : `${Math.round(account.quota_remaining_percent)}%`}</b><small>配额</small></div>
              <div className="pg-tv-account-cell"><b>{account.concurrency_active}/{account.concurrency_limit}</b><small>{account.queued_requests ? `排队 ${account.queued_requests}` : "并发"}</small></div>
              <div className="pg-tv-account-cell"><b>{metric(account.traffic?.total_requests, "0")}</b><small>{account.traffic?.success_rate != null && account.traffic?.total_requests ? `${account.traffic.success_rate.toFixed(0)}% 成功` : "请求"}</small></div>
            </div>
          );
        })}
        {!accounts.length && <div className="pg-tv-list-empty"><KeyRound size={16} />暂无关联账号</div>}
      </div>
    </div>
  );
}

function AccountDetail({ data, accountId, onSelect }: { data: RouteTopology; accountId: string; onSelect: Props["onSelect"] }) {
  const account = data.accounts.find((item) => item.id === accountId);
  if (!account) return <div className="pg-tv-detail-body pg-tv-empty">账号不存在</div>;
  const provider = data.providers.find((p) => p.id === account.provider_id);
  const health = !account.routable ? "warning" : account.status === "disabled" ? "disabled" : account.health_status === "error" || account.health_status === "unhealthy" ? "fault" : "healthy";
  return (
    <div className="pg-tv-detail-body">
      <DetailHero
        icon={<KeyRound size={18} />}
        kind="Account"
        name={account.name}
        subtitle={account.email_masked || account.plan_type || undefined}
        health={health}
      />
      <MetricGrid items={[
        { label: "所属厂商", value: provider?.name ?? "未关联" },
        { label: "状态", value: account.status || "unchecked" },
        { label: "健康", value: account.health_status || "未检查" },
        { label: "路由", value: account.routable ? "可路由" : "待适配" },
      ]} />
      {provider && (
        <div className="pg-tv-list-head"><strong>所属厂商</strong><span>1</span></div>
      )}
      {provider && (
        <div className="pg-tv-list">
          <button className="pg-tv-list-row" onClick={() => onSelect({ kind: "node", id: providerNodeId(provider.id), entityId: provider.id, nodeKind: "provider" })}>
            <span className="pg-tv-list-main"><i className={`health-${!provider.enabled ? "disabled" : provider.healthy_account_count ? "healthy" : "warning"}`} /><span><strong>{provider.name}</strong><small>{protocolLabel(provider.protocol)} · {provider.healthy_account_count}/{provider.account_count} 健康账号</small></span></span>
            <ChevronRight size={13} />
          </button>
        </div>
      )}
    </div>
  );
}

function EdgeDetail({ data, edgeId, onSelect }: { data: RouteTopology; edgeId: string; onSelect: Props["onSelect"] }) {
  const edge = data.edges.find((item) => item.id === edgeId);
  if (!edge) return <div className="pg-tv-detail-body pg-tv-empty">连线不存在</div>;
  const sourceLabel = labelFor(data, edge.source);
  const targetLabel = labelFor(data, edge.target);
  const status = edge.status as keyof typeof HEALTH_META;
  return (
    <div className="pg-tv-detail-body">
      <DetailHero
        icon={<Activity size={18} />}
        kind="Edge"
        name={`${sourceLabel} → ${targetLabel}`}
        subtitle={edge.id}
        health={status}
      />
      <MetricGrid items={[
        { label: "状态", value: HEALTH_META[status]?.label ?? status },
        { label: "活跃请求", value: String(edge.active_requests || 0) },
        { label: "协议", value: sourceLabel },
        { label: "目标", value: targetLabel },
      ]} />
      <div className="pg-tv-hint"><Activity size={14} /><span>点击相邻节点查看详情，或在画布中选择其他连线。</span></div>
    </div>
  );
}

function labelFor(data: RouteTopology, id: string) {
  if (id === "gateway") return data.gateway.name;
  if (id.startsWith("protocol-")) return data.protocols.find((p) => p.id === id)?.name || id;
  if (id.startsWith(NODE_ID_PREFIX_POOL)) return data.pools.find((p) => poolNodeId(p.id) === id)?.name || id;
  if (id.startsWith(NODE_ID_PREFIX_PROVIDER)) return data.providers.find((p) => providerNodeId(p.id) === id)?.name || id;
  if (id.startsWith("account-")) {
    const account = data.accounts.find((item) => `account-${item.id}` === id);
    if (account) return account.name;
    const summary = data.accounts.filter((item) => id === `account-summary-${item.provider_id}`);
    if (summary.length) return `${summary.length} 个账号`;
  }
  return id;
}

export default function TopologyInspector({ data, selection, onSelect, onOpenDashboard, onClose }: Props) {
  const [providerDetail, setProviderDetail] = useState<ProviderTopologyDetail>();
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<string>();

  useEffect(() => {
    if (selection.kind !== "node" || selection.nodeKind !== "provider") {
      setProviderDetail(undefined);
      setDetailError(undefined);
      setDetailLoading(false);
      return;
    }
    let cancelled = false;
    setProviderDetail(undefined);
    setDetailLoading(true);
    setDetailError(undefined);
    getProviderTopologyDetail(selection.entityId)
      .then((detail) => { if (!cancelled) setProviderDetail(detail); })
      .catch((error) => {
        console.error("Failed to load provider topology detail", {
          providerId: selection.entityId,
          graphNodeId: selection.id,
          error,
        });
        if (!cancelled) {
          const message = typeof error === "string"
            ? error
            : error instanceof Error
              ? error.message
              : "未知错误";
          setDetailError(message);
        }
      })
      .finally(() => { if (!cancelled) setDetailLoading(false); });
    return () => { cancelled = true; };
  }, [selection]);

  const isNode = selection.kind === "node";
  const nodeKind = isNode ? selection.nodeKind : undefined;
  const title = !isNode
    ? (selection.kind === "edge" ? "连线详情" : "路由总览")
    : nodeKind === "gateway" ? "网关总览"
    : nodeKind === "provider" ? data.providers.find((p) => p.id === selection.entityId)?.name ?? "厂商详情"
    : nodeKind === "pool" ? data.pools.find((p) => p.id === selection.entityId)?.name ?? "路由池详情"
    : nodeKind === "protocol" ? data.protocols.find((p) => p.id === selection.entityId)?.name ?? "协议详情"
    : nodeKind === "account" ? data.accounts.find((p) => p.id === selection.entityId)?.name ?? "账号详情"
    : "节点详情";

  return (
    <aside className="pg-tv-inspector">
      <div className="pg-tv-inspector-head">
        <div>
          <span>Node Inspector</span>
          <strong title={title}>{title}</strong>
        </div>
        <div className="pg-tv-inspector-head-actions">
          {onClose && (
            <button
              onClick={onClose}
              title="关闭详情面板"
              aria-label="关闭详情面板"
              className="pg-tv-inspector-close"
            >
              <X size={14} />
            </button>
          )}
        </div>
      </div>
      {selection.kind === "overview" && <Overview data={data} selection={selection} onSelect={onSelect} onOpenDashboard={onOpenDashboard} />}
      {selection.kind === "node" && nodeKind === "gateway" && <GatewayDetail data={data} />}
      {selection.kind === "node" && nodeKind === "protocol" && <ProtocolDetail data={data} id={selection.entityId} onSelect={onSelect} />}
      {selection.kind === "node" && nodeKind === "pool" && <PoolDetail data={data} id={selection.entityId} onSelect={onSelect} />}
      {selection.kind === "node" && nodeKind === "provider" && (detailError
        ? <div className="pg-tv-detail-body pg-tv-empty"><AlertTriangle size={17} /><span>厂商详情加载失败<small title={detailError}>{detailError}</small></span></div>
        : <ProviderDetail detail={providerDetail} loading={detailLoading} />)}
      {selection.kind === "node" && nodeKind === "account" && <AccountDetail data={data} accountId={selection.entityId} onSelect={onSelect} />}
      {selection.kind === "edge" && <EdgeDetail data={data} edgeId={selection.edgeId} onSelect={onSelect} />}
    </aside>
  );
}
