import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { MarkerType } from "@xyflow/react";
import type { RouteTopology, TopologyRuntimeDelta } from "@/lib/tauri-commands";
import type {
  TopologyFlowEdge,
  TopologyFlowNode,
  TopologyHealth,
  TopologyNodeKind,
  TopologySelection,
} from "./types";
import { NODE_SIZES } from "./types";

export const NODE_ID_GATEWAY = "gateway";
export const NODE_ID_PREFIX_PROTOCOL = "protocol-";
export const NODE_ID_PREFIX_POOL = "pool-";
export const NODE_ID_PREFIX_PROVIDER = "provider-";
export const NODE_ID_PREFIX_ACCOUNT = "account-";

/** When a provider has more accounts than this, the fifth layer collapses
 *  them into a single summary node to keep the canvas readable. */
export const ACCOUNT_COLLAPSE_THRESHOLD = 6;

/** Mockup design (2026-08-02): accounts live in the provider inspector only
 *  and never enter the canvas — the canvas stays a clean four-layer topology
 *  (网关 → 协议 → 路由池 → 上游厂商). Flip to `true` to restore the fifth
 *  account layer. */
export const SHOW_ACCOUNT_LAYER = false;

export function protocolNodeId(protocol: string) {
  return `${NODE_ID_PREFIX_PROTOCOL}${protocol}`;
}
export function poolNodeId(poolId: string) {
  return `${NODE_ID_PREFIX_POOL}${poolId}`;
}
export function providerNodeId(providerId: string) {
  return `${NODE_ID_PREFIX_PROVIDER}${providerId}`;
}
export function accountNodeId(accountId: string) {
  return `${NODE_ID_PREFIX_ACCOUNT}${accountId}`;
}
export function accountSummaryNodeId(providerId: string) {
  return `${NODE_ID_PREFIX_ACCOUNT}summary-${providerId}`;
}

function healthForPool(pool: RouteTopology["pools"][number]): TopologyHealth {
  if (!pool.enabled) return "disabled";
  if (pool.healthy_resource_count === 0) return "warning";
  return "healthy";
}

function healthForProvider(provider: RouteTopology["providers"][number]): TopologyHealth {
  if (!provider.enabled) return "disabled";
  if (provider.healthy_account_count === 0) return "warning";
  return "healthy";
}

function healthForAccount(account: RouteTopology["accounts"][number]): TopologyHealth {
  if (account.status === "disabled") return "disabled";
  if (!account.routable) return "warning";
  if (account.health_status === "error" || account.health_status === "unhealthy") return "fault";
  return "healthy";
}

/** Template names that carry no vendor identity (the canvas then shows host). */
const GENERIC_PROVIDER_NAME = /自定义|上游|custom/i;

function providerSubtitle(provider: RouteTopology["providers"][number]) {
  if (GENERIC_PROVIDER_NAME.test(provider.name) && provider.host) return provider.host;
  return protocolLabel(provider.protocol);
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
    case "least_used": return "最少用";
    case "priority": return "优先级";
    case "random": return "随机";
    case "cost_optimized": return "成本优先";
    default: return "轮询";
  }
}

function providerAccountsLine(provider: RouteTopology["providers"][number]) {
  const accounts = `${provider.healthy_account_count}/${provider.account_count} 账号`;
  const latency = latencyMs(provider.traffic.avg_latency_ms);
  const success = successRate(provider.traffic.success_rate);
  return [accounts, latency, success].filter(Boolean).join(" · ");
}

/** Compact count matching the mockup nodes: 3,842 → "3.8k". */
function compact(value: number) {
  const n = Math.max(0, value);
  if (n >= 1000) {
    const k = n / 1000;
    return `${k >= 100 ? Math.round(k) : k.toFixed(1)}k`;
  }
  return String(Math.round(n));
}

/** Success-rate percent, e.g. 99.6. */
function successRate(rate?: number) {
  return typeof rate === "number" && Number.isFinite(rate) ? `${rate.toFixed(1)}%` : "—";
}

function latencyMs(ms?: number) {
  return typeof ms === "number" && Number.isFinite(ms) ? `${Math.round(ms)}ms` : undefined;
}

export type EdgeDeltaMap = Map<string, number>;
export type NodeDeltaMap = Map<string, number>;

/** Applies runtime edge deltas to a topology edge list (only affected edges). */
export function applyEdgeDeltas(
  edges: TopologyFlowEdge[],
  deltas: Array<{ id: string; active_requests: number }>,
): TopologyFlowEdge[] {
  if (!deltas.length) return edges;
  const byId = new Map(deltas.map((d) => [d.id, d.active_requests]));
  return edges.map((edge) => {
    const activeRequests = byId.get(edge.id);
    if (activeRequests === undefined) return edge;
    return {
      ...edge,
      data: {
        ...edge.data!,
        activeRequests,
        active: activeRequests > 0,
        flash: undefined,
      },
    };
  });
}

export interface BuildTopologyInput {
  data: RouteTopology;
  selection: TopologySelection;
  /** When set, only nodes in fault/warning branches are shown. */
  filterAbnormal: boolean;
  /** Runtime deltas accumulated since the last snapshot (edge level). */
  liveEdges: Map<string, number>;
  /** Runtime deltas (node level). */
  liveNodes: Map<string, number>;
  /** Id of the active route path (highlight ancestor path). */
  activeRoute?: RouteTopology["active_route"];
}

/**
 * Maps the V2 snapshot into React Flow nodes/edges. Pure and memoized; the
 * returned nodes keep their positions stable unless structure (revision)
 * changed — positions are set by the layout engine, not by this hook.
 */
export function buildTopologyGraph(input: BuildTopologyInput): {
  nodes: TopologyFlowNode[];
  edges: TopologyFlowEdge[];
  protocolCount: number;
  poolCount: number;
  providerCount: number;
} {
  const { data } = input;
  const nodes: TopologyFlowNode[] = [];
  const edges: TopologyFlowEdge[] = [];

  const providerById = new Map(data.providers.map((p) => [p.id, p]));
  const poolById = new Map(data.pools.map((p) => [p.id, p]));

  // --- Gateway node ---
  const gatewayHealth: TopologyHealth = data.gateway.running ? "healthy" : "fault";
  nodes.push({
    id: NODE_ID_GATEWAY,
    type: "topology",
    position: { x: 0, y: 0 },
    data: {
      kind: "gateway",
      id: NODE_ID_GATEWAY,
      label: data.gateway.name,
      subtitle: data.gateway.address,
      stats: `${data.gateway.active_connections} 并发`,
      detail: data.gateway.running ? "网关运行中" : "已停止",
      health: gatewayHealth,
      enabled: data.gateway.running,
      active: (input.liveNodes.get(NODE_ID_GATEWAY) ?? 0) > 0 || data.gateway.active_connections > 0,
      activeRequests: data.gateway.active_connections,
      selected: input.selection.kind === "node" && input.selection.id === NODE_ID_GATEWAY,
      ancestorOfSelection: false,
    },
  });

  // --- Protocol nodes ---
  for (const protocol of data.protocols) {
    nodes.push({
      id: protocol.id,
      type: "topology",
      position: { x: 0, y: 0 },
      data: {
        kind: "protocol",
        id: protocol.id,
        label: protocol.name,
        subtitle: protocolLabel(protocol.protocol),
        stats: `${compact(protocol.traffic.total_requests)} 请求 · ${successRate(protocol.traffic.success_rate)}`,
        detail: (input.liveNodes.get(protocol.id) ?? 0) > 0
          ? `${input.liveNodes.get(protocol.id) ?? 0} 条活跃路径`
          : `${protocol.pool_ids.length} 条路由路径`,
        health: protocol.enabled ? "healthy" : "disabled",
        enabled: protocol.enabled,
        active: (input.liveNodes.get(protocol.id) ?? 0) > 0,
        activeRequests: input.liveNodes.get(protocol.id) ?? 0,
        selected: input.selection.kind === "node" && input.selection.id === protocol.id,
        ancestorOfSelection: false,
      },
    });
  }

  // --- Pool nodes ---
  for (const pool of data.pools) {
    nodes.push({
      id: poolNodeId(pool.id),
      type: "topology",
      position: { x: 0, y: 0 },
      data: {
        kind: "pool",
        id: pool.id,
        label: pool.name,
        subtitle: protocolLabel(pool.protocol),
        badge: strategyLabel(pool.strategy),
        stats: `${pool.healthy_resource_count}/${pool.resource_count} 资源 · ${pool.model_count} 模型`,
        detail: `${compact(pool.traffic.total_requests)} 请求 · ${successRate(pool.traffic.success_rate)}`,
        health: healthForPool(pool),
        enabled: pool.enabled,
        active: (input.liveNodes.get(poolNodeId(pool.id)) ?? 0) > 0,
        activeRequests: input.liveNodes.get(poolNodeId(pool.id)) ?? 0,
        selected: input.selection.kind === "node" && input.selection.id === poolNodeId(pool.id),
        ancestorOfSelection: false,
      },
    });
  }

  // --- Provider nodes ---
  for (const provider of data.providers) {
    nodes.push({
      id: providerNodeId(provider.id),
      type: "topology",
      position: { x: 0, y: 0 },
      data: {
        kind: "provider",
        id: provider.id,
        label: provider.name,
        subtitle: providerSubtitle(provider),
        stats: providerAccountsLine(provider),
        detail: `${compact(provider.traffic.total_requests)} 请求`,
        health: healthForProvider(provider),
        enabled: provider.enabled,
        active: (input.liveNodes.get(providerNodeId(provider.id)) ?? 0) > 0,
        activeRequests: input.liveNodes.get(providerNodeId(provider.id)) ?? 0,
        selected: input.selection.kind === "node" && input.selection.id === providerNodeId(provider.id),
        ancestorOfSelection: false,
      },
    });
  }

  // --- Fifth layer: account nodes (collapsed per provider when too many).
  // Hidden from the canvas per the mockup design (accounts live in the
  // provider inspector); kept buildable behind SHOW_ACCOUNT_LAYER. ---
  const collapsedProviders = new Set<string>();
  if (SHOW_ACCOUNT_LAYER) {
    const accountsByProvider = new Map<string, RouteTopology["accounts"]>();
    for (const account of data.accounts) {
      const list = accountsByProvider.get(account.provider_id) ?? [];
      list.push(account);
      accountsByProvider.set(account.provider_id, list);
    }
    for (const [providerId, list] of accountsByProvider) {
      if (list.length > ACCOUNT_COLLAPSE_THRESHOLD) collapsedProviders.add(providerId);
      const provider = data.providers.find((item) => item.id === providerId);
      const providerName = provider?.name ?? providerId;
      if (collapsedProviders.has(providerId)) {
        const healthy = list.filter((account) => healthForAccount(account) === "healthy").length;
        const hasFault = list.some((account) => healthForAccount(account) === "fault");
        nodes.push({
          id: accountSummaryNodeId(providerId),
          type: "topology",
          position: { x: 0, y: 0 },
          data: {
            kind: "account",
            id: accountSummaryNodeId(providerId),
            label: `${list.length} 个账号`,
            subtitle: providerName,
            stats: `${healthy}/${list.length} 健康`,
            detail: "已折叠 · 点击查看详情",
            health: list.length === healthy ? "healthy" : hasFault ? "fault" : "warning",
            enabled: true,
            active: false,
            activeRequests: 0,
            selected: input.selection.kind === "node" && input.selection.id === accountSummaryNodeId(providerId),
            ancestorOfSelection: false,
          },
        });
        continue;
      }
      for (const account of list) {
        nodes.push({
          id: accountNodeId(account.id),
          type: "topology",
          position: { x: 0, y: 0 },
          data: {
            kind: "account",
            id: account.id,
            label: account.name,
            subtitle: account.plan_type ?? account.email_masked ?? "Account",
            stats: account.routable ? "可路由" : "待适配",
            detail: account.status === "disabled" ? "已禁用" : account.health_status ? account.health_status : "",
            health: healthForAccount(account),
            enabled: account.status !== "disabled",
            active: (input.liveNodes.get(accountNodeId(account.id)) ?? 0) > 0,
            activeRequests: input.liveNodes.get(accountNodeId(account.id)) ?? 0,
            selected: input.selection.kind === "node" && input.selection.id === accountNodeId(account.id),
            ancestorOfSelection: false,
          },
        });
      }
    }
  }

  // --- Edges: gateway -> protocol -> pool -> provider -> account ---
  const seen = new Set<string>();
  for (const edge of data.edges) {
    if (seen.has(edge.id)) continue;
    seen.add(edge.id);
    // Accounts never enter the canvas (mockup design): drop provider→account
    // edges entirely unless the fifth layer is explicitly enabled.
    if (!SHOW_ACCOUNT_LAYER && edge.target.startsWith(NODE_ID_PREFIX_ACCOUNT)) continue;
    // Collapsed providers replace their account edges with a single summary edge.
    if (
      SHOW_ACCOUNT_LAYER &&
      edge.source.startsWith(NODE_ID_PREFIX_PROVIDER) &&
      edge.target.startsWith(NODE_ID_PREFIX_ACCOUNT) &&
      collapsedProviders.has(edge.source.slice(NODE_ID_PREFIX_PROVIDER.length))
    ) {
      const providerId = edge.source.slice(NODE_ID_PREFIX_PROVIDER.length);
      const summaryEdgeId = `provider-${providerId}-account-summary`;
      if (seen.has(summaryEdgeId)) continue;
      seen.add(summaryEdgeId);
      edges.push({
        ...edge,
        id: summaryEdgeId,
        target: accountSummaryNodeId(providerId),
        data: {
          id: summaryEdgeId,
          sourceLabel: nodeLabelFor(edge.source, data, providerById, poolById),
          targetLabel: nodeLabelFor(accountSummaryNodeId(providerId), data, providerById, poolById),
          status: edge.status as TopologyHealth,
          active: false,
          activeRequests: 0,
        },
      });
      continue;
    }
    const activeRequests = Math.max(edge.active_requests, input.liveEdges.get(edge.id) ?? 0);
    const status = edge.status as TopologyHealth;
    // Arrow tint follows the edge state (mockup: gray available / blue active
    // route / amber warning) — concrete colors because SVG marker fill
    // attributes do not resolve CSS variables.
    const markerColor = status === "warning" ? "#f59e0b"
      : status === "fault" ? "#ff453a"
      : activeRequests > 0 ? "#0a84ff"
      : "#94a3b8";
    edges.push({
      id: edge.id,
      source: edge.source,
      target: edge.target,
      type: "topology",
      data: {
        id: edge.id,
        sourceLabel: nodeLabelFor(edge.source, data, providerById, poolById),
        targetLabel: nodeLabelFor(edge.target, data, providerById, poolById),
        status,
        active: activeRequests > 0,
        activeRequests,
      },
      markerEnd: { type: MarkerType.ArrowClosed, width: 12, height: 12, color: markerColor },
      selectable: true,
    });
  }

  // --- Abnormal filter: keep only fault/warning branches + ancestors ---
  if (input.filterAbnormal) {
    const keepNodes = new Set<string>();
    const abnormalNodes = nodes.filter((node) => node.data.health === "fault" || node.data.health === "warning");
    const walk = (id: string) => {
      if (keepNodes.has(id)) return;
      keepNodes.add(id);
      edges
        .filter((edge) => edge.target === id)
        .forEach((edge) => walk(edge.source));
    };
    abnormalNodes.forEach((node) => walk(node.id));
    const keepEdges = edges.filter((edge) => keepNodes.has(edge.source) && keepNodes.has(edge.target));
    const kept = nodes.filter((node) => keepNodes.has(node.id));
    kept.forEach((node) => {
      const parentEdges = keepEdges.filter((edge) => edge.target === node.id);
      node.data.ancestorOfSelection = parentEdges.length > 0 && node.data.health === "healthy";
    });
    return { nodes: kept, edges: keepEdges, protocolCount: data.protocols.length, poolCount: data.pools.length, providerCount: data.providers.length };
  }

  // --- Ancestor highlight for selection ---
  if (input.selection.kind === "node") {
    const selectedId = input.selection.id;
    const stack: string[] = [];
    let current = selectedId;
    while (true) {
      const parentEdge = edges.find((edge) => edge.target === current);
      if (!parentEdge) break;
      stack.push(parentEdge.source);
      current = parentEdge.source;
    }
    stack.forEach((id) => {
      const node = nodes.find((n) => n.id === id);
      if (node && node.id !== selectedId) node.data.ancestorOfSelection = true;
    });
  }

  return { nodes, edges, protocolCount: data.protocols.length, poolCount: data.pools.length, providerCount: data.providers.length };
}

function nodeLabelFor(
  id: string,
  data: RouteTopology,
  providerById: Map<string, RouteTopology["providers"][number]>,
  poolById: Map<string, RouteTopology["pools"][number]>,
) {
  if (id === NODE_ID_GATEWAY) return data.gateway.name;
  if (id.startsWith(NODE_ID_PREFIX_PROTOCOL)) {
    const protocol = data.protocols.find((p) => p.id === id);
    return protocol?.name ?? id;
  }
  if (id.startsWith(NODE_ID_PREFIX_POOL)) {
    const pool = poolById.get(id.slice(NODE_ID_PREFIX_POOL.length));
    return pool?.name ?? id;
  }
  if (id.startsWith(NODE_ID_PREFIX_PROVIDER)) {
    const provider = providerById.get(id.slice(NODE_ID_PREFIX_PROVIDER.length));
    return provider?.name ?? id;
  }
  if (id.startsWith(NODE_ID_PREFIX_ACCOUNT)) {
    const account = data.accounts.find((item) => accountNodeId(item.id) === id);
    if (account) return account.name;
    const summaryProviderId = id.slice(NODE_ID_PREFIX_ACCOUNT.length + "summary-".length);
    if (id.startsWith(`${NODE_ID_PREFIX_ACCOUNT}summary-`)) {
      return providerById.get(summaryProviderId)?.name ?? id;
    }
    return id;
  }
  return id;
}

/** Placeholder node sizing for the fallback layout. */
export function nodeSize(kind: TopologyNodeKind) {
  return NODE_SIZES[kind];
}
