import { memo } from "react";
import { BaseEdge, EdgeLabelRenderer, getSmoothStepPath } from "@xyflow/react";
import type { EdgeProps } from "@xyflow/react";
import type { TopologyFlowEdge } from "./types";

/**
 * Flow animation budget: every active edge animates (the dash flows from the
 * source — the gateway side — toward the target). The cap only guards against
 * pathological graphs with hundreds of concurrent edges.
 */
const MAX_ANIMATED_EDGES = 120;

function TopologyEdgeComponent({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
  selected,
}: EdgeProps<TopologyFlowEdge>) {
  const [path, labelX, labelY] = getSmoothStepPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
    borderRadius: 12,
  });

  const status = data?.status ?? "healthy";
  const active = (data?.activeRequests ?? 0) > 0;
  const flash = data?.flash;
  const animated = active && (data?.activeRequests ?? 0) <= MAX_ANIMATED_EDGES;

  return (
    <>
      <BaseEdge
        id={id}
        path={path}
        className={`pg-tv-edge-base status-${status} ${active ? "active" : ""}`}
        style={active ? { strokeWidth: Math.min(5, 2.2 + (data?.activeRequests ?? 0) * 0.3) } : undefined}
      />
      {active && (
        <BaseEdge
          id={`${id}-flow`}
          path={path}
          className={`pg-tv-edge-flow ${animated ? "animated" : "static"}`}
          markerEnd="none"
          style={{ strokeWidth: Math.min(3.2, 1.8 + (data?.activeRequests ?? 0) * 0.22) }}
        />
      )}
      {flash && (
        <BaseEdge
          id={`${id}-flash`}
          path={path}
          className={`pg-tv-edge-flash ${flash}`}
          markerEnd="none"
        />
      )}
      {active && (
        <EdgeLabelRenderer>
          <span
            className={`pg-tv-edge-tag ${selected ? "selected" : ""}`}
            style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}
          >
            {data?.activeRequests}
          </span>
        </EdgeLabelRenderer>
      )}
    </>
  );
}

export const TopologyEdge = memo(TopologyEdgeComponent);
