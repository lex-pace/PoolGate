import { memo } from "react";
import { Handle, Position } from "@xyflow/react";
import { Plus } from "lucide-react";
import type { NodeProps } from "@xyflow/react";
import type { TopologyFlowNode } from "./types";
import { HEALTH_META } from "./types";

function TopologyNodeComponent({ id, data }: NodeProps<TopologyFlowNode>) {
  const healthMeta = HEALTH_META[data.health];
  const showWarningIcon = data.health === "warning";
  const showFaultIcon = data.health === "fault";
  const badge = data.badge;

  return (
    <div
      className={`pg-tv-node pg-tv-${data.kind} health-${data.health} ${data.selected ? "selected" : ""} ${data.active ? "active-route" : ""} ${data.ancestorOfSelection ? "ancestor" : ""}`}
      data-tv-node={id}
      title={`${data.label}\n${data.subtitle}\n${data.stats} · ${data.detail}\n状态：${healthMeta.label}`}
    >
      <Handle type="target" position={Position.Left} className="pg-tv-handle pg-tv-handle-in" />

      {data.kind === "gateway" && (
        <>
          <div className="pg-tv-node-head">
            <span className="pg-tv-node-icon pg-tv-gateway-logo"><Plus size={16} strokeWidth={2.6} /></span>
            <span className="pg-tv-node-title">
              <strong>{data.label}</strong>
              <small>{data.subtitle}</small>
            </span>
            <i className={`pg-tv-node-pulse ${data.active ? "on" : ""}`} aria-hidden="true" />
          </div>
          <div className="pg-tv-node-stats"><b>{data.stats}</b><span className="pg-tv-node-detail-accent">{data.detail}</span></div>
        </>
      )}

      {data.kind === "protocol" && (
        <>
          <div className="pg-tv-node-head">
            <i className={`pg-tv-health health-${data.health}`} aria-hidden="true" />
            <span className="pg-tv-node-title">
              <strong>{data.label}</strong>
              <small>{data.subtitle}</small>
            </span>
            {showWarningIcon && <AlertMark tone="warning" />}
            {showFaultIcon && <AlertMark tone="fault" />}
          </div>
          <div className="pg-tv-node-stats"><b>{data.stats}</b></div>
          <div className={`pg-tv-node-line ${data.active ? "accent" : ""}`}>{data.detail}</div>
        </>
      )}

      {data.kind === "pool" && (
        <>
          <div className="pg-tv-node-head">
            <span className="pg-tv-node-title">
              <strong>{data.label}</strong>
              {data.subtitle && <small>{data.subtitle}</small>}
            </span>
            {badge && <span className="pg-tv-node-badge">{badge}</span>}
            {showWarningIcon && <AlertMark tone="warning" />}
            {showFaultIcon && <AlertMark tone="fault" />}
          </div>
          <div className="pg-tv-node-stats"><b>{data.stats}</b></div>
          <div className={`pg-tv-node-line ${data.active ? "accent" : ""}`}>{data.detail}</div>
        </>
      )}

      {data.kind === "provider" && (
        <>
          <div className="pg-tv-node-head">
            <i className={`pg-tv-health health-${data.health}`} aria-hidden="true" />
            <span className="pg-tv-node-title">
              <strong>{data.label}</strong>
              {data.subtitle && <small>{data.subtitle}</small>}
            </span>
            {showWarningIcon && <AlertMark tone="warning" />}
            {showFaultIcon && <AlertMark tone="fault" />}
            <i className={`pg-tv-node-pulse ${data.active ? "on" : ""}`} aria-hidden="true" />
          </div>
          <div className="pg-tv-node-stats"><b>{data.stats}</b></div>
        </>
      )}

      {data.kind === "account" && (
        <>
          <div className="pg-tv-node-head">
            <i className={`pg-tv-health health-${data.health}`} aria-hidden="true" />
            <span className="pg-tv-node-title">
              <strong>{data.label}</strong>
              {data.subtitle && <small>{data.subtitle}</small>}
            </span>
          </div>
          <div className="pg-tv-node-stats"><b>{data.stats}</b><span>{data.detail}</span></div>
        </>
      )}

      {data.activeRequests > 0 && data.kind !== "gateway" && <span className="pg-tv-active-count">{data.activeRequests}</span>}
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
