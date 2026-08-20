import { memo } from "react";
import { BaseEdge, EdgeLabelRenderer, getBezierPath } from "@xyflow/react";
import type { EdgeProps } from "@xyflow/react";
import type { TopologyFlowEdge } from "./types";

/**
 * Flow animation budget: every active edge animates (the dash flows from the
 * source — the gateway side — toward the target). The cap only guards against
 * pathological graphs with hundreds of concurrent edges.
 */
const MAX_ANIMATED_EDGES = 120;

/** Arrow glyph that travels along the active path (SMIL animateMotion). */
function FlowArrow({ id, path, durationMs, tint }: { id: string; path: string; durationMs: number; tint: string }) {
  return (
    <g className="pg-tv-edge-motion">
      <path id={`${id}-motion-path`} d={path} fill="none" stroke="none" />
      <polygon points="0,-4 8.5,0 0,4" fill={tint} opacity={0.95}>
        <animateMotion dur={`${durationMs}ms`} repeatCount="indefinite" rotate="auto">
          <mpath href={`#${id}-motion-path`} />
        </animateMotion>
      </polygon>
    </g>
  );
}

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
  // Bezier lanes (mockup: gentle S-curves) so sibling edges fan out instead of
  // merging onto a single shared orthogonal channel.
  const [path, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
    curvature: 0.28,
  });

  const status = data?.status ?? "healthy";
  const active = (data?.activeRequests ?? 0) > 0;
  const flash = data?.flash;
  const animated = active && (data?.activeRequests ?? 0) <= MAX_ANIMATED_EDGES;
  const reducedMotion = typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

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
          id={`${id}-under`}
          path={path}
          className="pg-tv-edge-under"
          markerEnd="none"
          style={{ strokeWidth: Math.min(9, 5.5 + (data?.activeRequests ?? 0) * 0.5) }}
        />
      )}
      {active && (
        <BaseEdge
          id={`${id}-flow`}
          path={path}
          className={`pg-tv-edge-flow ${animated ? "animated" : "static"}`}
          markerEnd="none"
          style={{ strokeWidth: Math.min(3.2, 1.8 + (data?.activeRequests ?? 0) * 0.22) }}
        />
      )}
      {active && animated && !reducedMotion && (
        <FlowArrow id={id} path={path} durationMs={1100} tint="#0a84ff" />
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
