import type { Node, Edge } from "@xyflow/react";
import type { RouteTopology, TopologyRuntimeDelta } from "@/lib/tauri-commands";

/** Health state shared by nodes and edges (V2 spec §7). */
export type TopologyHealth = "healthy" | "warning" | "fault" | "disabled" | "unknown";

/** Node kinds in the five-layer topology (V2 spec §3 + fifth layer for accounts). */
export type TopologyNodeKind = "gateway" | "protocol" | "pool" | "provider" | "account";

export interface TopologyNodeData {
  kind: TopologyNodeKind;
  id: string;
  label: string;
  subtitle: string;
  stats: string;
  detail: string;
  health: TopologyHealth;
  enabled: boolean;
  active: boolean;
  activeRequests: number;
  selected: boolean;
  ancestorOfSelection: boolean;
  [key: string]: unknown;
}

export type TopologyFlowNode = Node<TopologyNodeData, "topology">;

export interface TopologyEdgeData {
  id: string;
  sourceLabel: string;
  targetLabel: string;
  status: TopologyHealth;
  active: boolean;
  activeRequests: number;
  /** Transient flash shown briefly after a completed request on this edge. */
  flash?: "success" | "failed";
  [key: string]: unknown;
}

export type TopologyFlowEdge = Edge<TopologyEdgeData, "topology">;

/** Node size presets from the V2 layout spec (§4). */
export const NODE_SIZES: Record<TopologyNodeKind, { width: number; height: number }> = {
  gateway: { width: 188, height: 92 },
  protocol: { width: 176, height: 76 },
  pool: { width: 196, height: 94 },
  provider: { width: 198, height: 82 },
  account: { width: 176, height: 62 },
};

export const HEALTH_META: Record<TopologyHealth, { label: string }> = {
  healthy: { label: "正常" },
  warning: { label: "告警" },
  fault: { label: "故障" },
  disabled: { label: "禁用" },
  unknown: { label: "未知" },
};

export type TopologySelection =
  | { kind: "overview" }
  | {
      kind: "node";
      /** React Flow graph id used for canvas selection and ancestor highlighting. */
      id: string;
      /** Stable topology/database id used for detail lookup and backend commands. */
      entityId: string;
      nodeKind: TopologyNodeKind;
    }
  | { kind: "edge"; edgeId: string };

export type TopologyProps = {
  data?: RouteTopology;
  loading: boolean;
  error: boolean;
  onRefresh: () => void;
  /** Optional node id to focus on mount (tray deep-link). */
  focusNodeId?: string;
  /** Whether the canvas is currently rendered in the standalone fullscreen
   *  layer (hides the app chrome). Controls the toolbar fullscreen icon. */
  isFullscreen?: boolean;
  /** Toggle handler driven by the host (typically switches the window hash
   *  between '' and '#/topology-fullscreen'). */
  onToggleFullscreen?: () => void;
};

export type { TopologyRuntimeDelta };
