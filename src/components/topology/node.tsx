import { memo } from "react";
import { Handle, Position } from "@xyflow/react";
import { Boxes, KeyRound, Network, ServerCog, Waypoints } from "lucide-react";
import type { NodeProps } from "@xyflow/react";
import type { TopologyFlowNode } from "./types";
import { HEALTH_META } from "./types";

const KIND_ICONS = {
  gateway: Waypoints,
  protocol: Network,
  pool: Boxes,
  provider: ServerCog,
  account: KeyRound,
} as const;

const KIND_LABELS = {
  gateway: "Gateway",
  protocol: "Protocol",
  pool: "Route Pool",
  provider: "Upstream Provider",
  account: "Account",
} as const;

function TopologyNodeComponent({ id, data }: NodeProps<TopologyFlowNode>) {
  const Icon = KIND_ICONS[data.kind];
  const healthMeta = HEALTH_META[data.health];
  const showWarningIcon = data.health === "warning";
  const showFaultIcon = data.health === "fault";

  return (
    <div
      className={`pg-tv-node pg-tv-${data.kind} health-${data.health} ${data.selected ? "selected" : ""} ${data.active ? "active-route" : ""} ${data.ancestorOfSelection ? "ancestor" : ""}`}
      data-tv-node={id}
      title={`${data.label} · ${healthMeta.label}`}
    >
      <Handle type="target" position={Position.Left} className="pg-tv-handle pg-tv-handle-in" />
      <div className="pg-tv-node-head">
        <span className="pg-tv-node-icon"><Icon size={14} /></span>
        <span className="pg-tv-node-title" title={`${data.label}\n${data.subtitle}\n状态：${healthMeta.label}`}>
          <strong>{data.label}</strong>
          <small>{data.subtitle}</small>
        </span>
        <i className={`pg-tv-health health-${data.health}`} aria-hidden="true" />
        {showWarningIcon && <AlertMark tone="warning" />}
        {showFaultIcon && <AlertMark tone="fault" />}
      </div>
      <div className="pg-tv-node-stats"><b>{data.stats}</b><span>{data.detail}</span></div>
      {data.activeRequests > 0 && <span className="pg-tv-active-count">{data.activeRequests} active</span>}
      <Handle type="source" position={Position.Right} className="pg-tv-handle pg-tv-handle-out" />
    </div>
  );
}

function AlertMark({ tone }: { tone: "warning" | "fault" }) {
  return (
    <span className={`pg-tv-alert-mark ${tone}`} title={tone === "warning" ? "告警" : "故障"} aria-label={tone === "warning" ? "告警" : "故障"}>
      {tone === "warning" ? "!" : "×"}
    </span>
  );
}

export const TopologyNode = memo(TopologyNodeComponent);
