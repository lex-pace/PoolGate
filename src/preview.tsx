// Design preview for the Topology V2 component. Renders the real
// React Flow + ELK canvas with a representative mock snapshot so the
// layout, health states and inspector can be verified in a browser.
import React from "react";
import { createRoot } from "react-dom/client";
import TopologyView from "./components/topology/TopologyView";
import type { RouteTopology } from "./lib/tauri-commands";
import "./styles/globals.css";
import "./preview.css";

function mockTraffic(providerId: string, requests: number, successRate = 0.96, latency = 320) {
  const success = Math.round(requests * successRate);
  return {
    provider_id: providerId,
    total_requests: requests,
    success_count: success,
    error_count: requests - success,
    success_rate: successRate * 100,
    input_tokens: requests * 800,
    output_tokens: requests * 400,
    cache_tokens: requests * 120,
    total_tokens: requests * 1320,
    total_cost: requests * 0.0042,
    avg_latency_ms: latency,
  };
}

const pools = [
  { id: "pool-coding-plan", name: "Coding Plan 主池", protocol: "chat", strategy: "least_used", enabled: true, provider_ids: ["provider-openai", "provider-azure", "provider-grok"], resource_count: 12, healthy_resource_count: 10, model_count: 4, traffic: mockTraffic("preview", 1820, 0.99, 260) },
  { id: "pool-free", name: "免费模型池", protocol: "chat", strategy: "priority", enabled: true, provider_ids: ["provider-grok", "provider-gemini", "provider-custom"], resource_count: 8, healthy_resource_count: 8, model_count: 3, traffic: mockTraffic("preview", 640, 0.97, 410) },
  { id: "pool-anthropic", name: "Anthropic 直连池", protocol: "anthropic", strategy: "round_robin", enabled: true, provider_ids: ["provider-anthropic"], resource_count: 6, healthy_resource_count: 6, model_count: 2, traffic: mockTraffic("preview", 1510, 0.995, 190) },
  { id: "pool-gemini", name: "Gemini 研究池", protocol: "gemini", strategy: "cost_optimized", enabled: true, provider_ids: ["provider-gemini", "provider-vertex"], resource_count: 5, healthy_resource_count: 3, model_count: 2, traffic: mockTraffic("preview", 380, 0.9, 520) },
  { id: "pool-responses", name: "Responses 实验池", protocol: "responses", strategy: "round_robin", enabled: true, provider_ids: ["provider-openai"], resource_count: 4, healthy_resource_count: 4, model_count: 2, traffic: mockTraffic("preview", 210, 0.98, 300) },
  { id: "pool-fallback", name: "故障转移池", protocol: "chat", strategy: "priority", enabled: false, provider_ids: ["provider-fallback"], resource_count: 2, healthy_resource_count: 0, model_count: 1, traffic: mockTraffic("preview", 12, 0.5, 900) },
];

const providers = [
  { id: "provider-openai", name: "OpenAI 官方", protocol: "chat", enabled: true, account_count: 7, healthy_account_count: 6, traffic: mockTraffic("preview", 1180, 0.99, 250), host: "https://api.openai.com" },
  { id: "provider-azure", name: "Azure OpenAI", protocol: "chat", enabled: true, account_count: 4, healthy_account_count: 4, traffic: mockTraffic("preview", 430, 0.995, 230), host: "https://api.azure.com" },
  { id: "provider-grok", name: "Grok API", protocol: "chat", enabled: true, account_count: 3, healthy_account_count: 3, traffic: mockTraffic("preview", 410, 0.97, 300), host: "https://api.x.ai" },
  { id: "provider-gemini", name: "Google Gemini", protocol: "gemini", enabled: true, account_count: 5, healthy_account_count: 4, traffic: mockTraffic("preview", 280, 0.93, 480), host: "https://generativelanguage.googleapis.com" },
  { id: "provider-vertex", name: "Vertex AI", protocol: "gemini", enabled: true, account_count: 2, healthy_account_count: 1, traffic: mockTraffic("preview", 96, 0.86, 620), host: "https://aiplatform.googleapis.com" },
  { id: "provider-anthropic", name: "Anthropic", protocol: "anthropic", enabled: true, account_count: 6, healthy_account_count: 6, traffic: mockTraffic("preview", 1510, 0.995, 190), host: "https://api.anthropic.com" },
  { id: "provider-fallback", name: "免费接口池", protocol: "chat", enabled: false, account_count: 2, healthy_account_count: 0, traffic: mockTraffic("preview", 12, 0.5, 900), host: "https://free.example.com" },
  { id: "provider-custom", name: "自定义", protocol: "chat", enabled: true, account_count: 3, healthy_account_count: 2, traffic: mockTraffic("preview", 180, 0.9, 420), host: "https://api.myproxy.example.com" },
];

// Fifth layer: concrete accounts. provider-openai carries 7 accounts so the
// collapse behaviour (> 6) is exercised in the preview.
const accounts: RouteTopology["accounts"] = [];
const accountSeeds: Record<string, string[]> = {
  "provider-openai": ["team@openai.com", "eng@openai.com", "research@openai.com", "ops@openai.com", "dev@openai.com", "qa@openai.com", "sre@openai.com"],
  "provider-azure": ["azure-1", "azure-2", "azure-3", "azure-4"],
  "provider-grok": ["grok-key-1", "grok-key-2", "grok-key-3"],
  "provider-gemini": ["gemini@google.com", "gemini-2", "gemini-3", "gemini-4", "gemini-5"],
  "provider-vertex": ["vertex-1", "vertex-2"],
  "provider-anthropic": ["claude@anthropic.com", "anthropic-2", "anthropic-3", "anthropic-4", "anthropic-5", "anthropic-6"],
  "provider-fallback": ["free-1", "free-2"],
  "provider-custom": ["proxy-account-a", "proxy-account-b", "proxy-account-c"],
};
Object.entries(accountSeeds).forEach(([providerId, names]) => {
  names.forEach((name, index) => {
    const faulted = providerId === "provider-vertex" && index === 1;
    accounts.push({
      id: `account-${providerId.replace("provider-", "")}-${index + 1}`,
      provider_id: providerId,
      name,
      email_masked: name.includes("@") ? `${name.slice(0, 2)}***@${name.split("@")[1]}` : undefined,
      status: providerId === "provider-fallback" ? "disabled" : "active",
      health_status: faulted ? "error" : "healthy",
      routable: true,
      plan_type: index === 0 && name.includes("@") ? "Coding Plan" : undefined,
    });
  });
});

const protocolNames = ["openai-chat", "openai-responses", "anthropic-messages", "google-gemini"];
const protocols = protocolNames.map((id, index) => {
  const protocol = id.replace("openai-", "").replace("-messages", "").replace("google-", "").replace("-chat", "chat");
  const poolIds = pools.filter((pool) => pool.protocol === protocol || (protocol === "chat" && pool.protocol === "chat")).map((pool) => pool.id);
  const requestCount = pools.filter((pool) => poolIds.includes(pool.id)).reduce((sum, pool) => sum + pool.traffic.total_requests, 0);
  const traffic = mockTraffic(`protocol-${protocol}`, requestCount, protocol === "responses" ? 0.98 : 0.96, protocol === "responses" ? 300 : 260);
  return { id, name: id === "openai-chat" ? "OpenAI Chat" : id === "openai-responses" ? "OpenAI Responses" : id === "anthropic-messages" ? "Anthropic Messages" : "Google Gemini", protocol, enabled: true, pool_ids: poolIds, request_count: requestCount, traffic };
});

// Build edges mirroring the backend contract.
const edges: RouteTopology["edges"] = [];
pools.forEach((pool) => {
  const protocolId = protocols.find((p) => p.pool_ids.includes(pool.id))?.id || "protocol-openai-chat";
  if (!edges.some((edge) => edge.id === `gateway-${protocolId}`)) {
    edges.push({ id: `gateway-${protocolId}`, source: "gateway", target: protocolId, status: "healthy", active: false, active_requests: 0 });
  }
  edges.push({ id: `${protocolId}-pool-${pool.id}`, source: protocolId, target: `pool-${pool.id}`, status: pool.enabled ? (pool.healthy_resource_count ? "healthy" : "warning") : "disabled", active: false, active_requests: 0 });
  pool.provider_ids.forEach((providerId) => {
    edges.push({ id: `pool-${pool.id}-provider-${providerId}`, source: `pool-${pool.id}`, target: `provider-${providerId}`, status: "healthy", active: false, active_requests: 0 });
  });
});
// Provider -> account edges (fifth layer).
accounts.forEach((account) => {
  edges.push({ id: `provider-${account.provider_id}-account-${account.id}`, source: `provider-${account.provider_id}`, target: `account-${account.id}`, status: account.status === "disabled" ? "disabled" : account.health_status === "error" ? "fault" : "healthy", active: false, active_requests: 0 });
});

const data = {
  version: 2 as const,
  topology_revision: 1,
  gateway: { id: "gateway", name: "PoolGate Gateway", address: "http://127.0.0.1:9800", running: true, active_connections: 3 },
  protocols,
  pools,
  providers,
  accounts,
  edges,
  active_route: {
    request_id: "mock-1",
    protocol: "openai-chat",
    pool_id: "pool-coding-plan",
    provider_id: "provider-openai",
    account_id: "account-preview",
    status: "active" as const,
    attempt: 1,
    started_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
  },
  runtime: {
    sequence: 1,
    emitted_at: new Date().toISOString(),
    active_connections: 3,
    active_paths: [{ protocol: "openai-chat", pool_id: "pool-coding-plan", provider_id: "provider-openai", account_id: "account-openai-1", active_requests: 2, last_active_at: new Date().toISOString() }],
    latest_route: undefined,
    node_deltas: [
      { id: "gateway", active_concurrency: 3 },
      { id: "protocol-openai-chat", active_concurrency: 2 },
      { id: "pool-pool-coding-plan", active_concurrency: 2 },
      { id: "provider-provider-openai", active_concurrency: 2 },
    ],
    edge_deltas: [
      { id: "gateway-protocol-openai-chat", active_requests: 2 },
      { id: "protocol-openai-chat-pool-pool-coding-plan", active_requests: 2 },
      { id: "pool-pool-coding-plan-provider-provider-openai", active_requests: 2 },
    ],
    completed: [],
  },
  updated_at: new Date().toISOString(),
};

const refresh = () => { /* no-op in preview */ };

createRoot(document.getElementById("root")!).render(
  <div style={{ height: "100vh", background: "var(--bg-app)", display: "flex", flexDirection: "column" }}>
    <div style={{ padding: "10px 16px", borderBottom: "1px solid var(--border-default)", color: "var(--text-secondary)", fontSize: 11, display: "flex", gap: 8, alignItems: "center" }}>
      <span style={{ color: "var(--color-brand)", fontWeight: 700 }}>Topology V2 预览</span>
      <span>React Flow + ELK 五层布局 · 6 路由池 / 8 厂商 / 32 账号</span>
      <span style={{ marginLeft: "auto", color: "var(--text-dim)" }}>点击节点/连线查看检查器 · ⌘+/⌘- 缩放 · 双击空白适配</span>
    </div>
    <div style={{ flex: 1, minHeight: 0 }}>
      <TopologyView data={data} loading={false} error={false} onRefresh={refresh} />
    </div>
  </div>,
);

// Allow `?select=pool-xxx|provider-xxx|protocol-xxx|gateway` in the URL so the
// design preview can demonstrate the inspector auto-popup for screenshots.
const initialSelect = (() => {
  if (typeof window === "undefined") return undefined;
  const params = new URLSearchParams(window.location.search);
  const target = params.get("select");
  if (!target) return undefined;
  if (target === "gateway") return { id: "gateway", nodeKind: "gateway" as const };
  if (target.startsWith("pool-")) return { id: target, nodeKind: "pool" as const };
  if (target.startsWith("provider-")) return { id: target, nodeKind: "provider" as const };
  if (target.startsWith("protocol-")) return { id: target, nodeKind: "protocol" as const };
  return undefined;
})();
if (initialSelect) {
  setTimeout(() => {
    const root = document.getElementById("root");
    if (!root) return;
    const targetNode = root.querySelector<HTMLElement>(`[data-tv-node="${initialSelect.id}"]`);
    targetNode?.click();
  }, 800);
}
