import ELK from "elkjs/lib/elk.bundled.js";
import type { TopologyFlowEdge, TopologyFlowNode, TopologyNodeKind } from "./types";
import { NODE_SIZES } from "./types";

const elk = new ELK();

export interface LayoutInput {
  nodes: TopologyFlowNode[];
  edges: TopologyFlowEdge[];
  revision: number;
}

export interface LayoutResult {
  nodes: TopologyFlowNode[];
  revision: number;
  width: number;
  height: number;
}

/** Cache of the last layout per topology revision. Structure changes only
 *  trigger a re-layout; metric refreshes never do (V2 spec §9). */
const layoutCache = new Map<number, LayoutResult>();

const LAYER_SPACING = 64;
const NODE_SPACING = 24;
const PADDING = 34;

/**
 * Runs the layered ELK layout (LEFT_TO_RIGHT) for the four-layer topology.
 * Executed on the main thread with a revision cache; for the expected node
 * counts (≤ 300) a single run stays well under the 400ms acceptance target.
 */
export async function layoutTopology(input: LayoutInput): Promise<LayoutResult> {
  const cached = layoutCache.get(input.revision);
  if (cached) return cached;

  const children = input.nodes.map((node) => {
    const size = NODE_SIZES[node.data.kind];
    return {
      id: node.id,
      width: size.width,
      height: size.height,
      // Keep sibling branches (pools / providers) close to their parent.
      properties: {
        "org.eclipse.elk.layered.crossingMinimization.forceNodeModelOrder": "true",
      },
    };
  });

  const graphEdges = input.edges.map((edge) => ({
    id: edge.id,
    sources: [edge.source],
    targets: [edge.target],
  }));

  const graph = {
    id: "topology-root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": "RIGHT",
      "elk.spacing.nodeNode": String(NODE_SPACING),
      "elk.layered.spacing.nodeNodeBetweenLayers": String(LAYER_SPACING),
      // SPLINES + explicit edge spacing keeps every relationship on its own
      // gently-curved lane instead of merging sibling edges onto one shared
      // orthogonal channel (mockup: each available link is a distinct line).
      "elk.edgeRouting": "SPLINES",
      "elk.spacing.edgeEdge": "22",
      "elk.layered.spacing.edgeEdgeBetweenLayers": "16",
      "elk.layered.crossingMinimization.strategy": "LAYER_SWEEP",
      "elk.layered.nodePlacement.strategy": "BRANDES_KOEPF",
      "elk.padding": `[top=${PADDING},left=${PADDING},bottom=${PADDING},right=${PADDING}]`,
    },
    children,
    edges: graphEdges,
  };

  let laidOut: any;
  try {
    laidOut = await elk.layout(graph);
  } catch (error) {
    console.error("[topology] ELK layout failed, falling back to layered grid", error);
    return fallbackLayout(input);
  }

  const positionByNodeId = new Map<string, { x: number; y: number }>();
  for (const child of laidOut.children ?? []) {
    positionByNodeId.set(child.id, { x: child.x, y: child.y });
  }

  // Determine the graph extent so the viewport can be fitted properly.
  const positioned = input.nodes
    .filter((node) => positionByNodeId.has(node.id))
    .map((node) => {
      const position = positionByNodeId.get(node.id)!;
      const size = NODE_SIZES[node.data.kind];
      return { ...node, position, width: size.width, height: size.height };
    });

  // ELK treats disconnected nodes (e.g. a provider not yet attached to any
  // pool) as their own layers, collapsing them onto the gateway column and
  // creating phantom columns. Re-anchor every node into the column of its
  // kind, then stack re-anchored nodes in ELK's vertical order so nothing
  // overlaps and no phantom layer appears.
  fixPhantomColumns(positioned, input.edges);

  // Single-node layers (the gateway) would otherwise sit at the column bottom;
  // re-center them vertically over the graph extent, matching the mockup where
  // the gateway floats in the middle of the first column.
  const gatewayNode = positioned.find((node) => node.id === "gateway");
  if (gatewayNode) {
    let graphBottom = 0;
    positioned.forEach((node) => {
      graphBottom = Math.max(graphBottom, node.position.y + (node.height ?? 0));
    });
    const height = NODE_SIZES.gateway.height;
    gatewayNode.position = {
      ...gatewayNode.position,
      y: Math.max(PADDING, (graphBottom - height) / 2),
    };
  }

  // Nodes that ELK dropped (should not happen) are stacked to the right.
  const orphans = input.nodes.filter((node) => !positionByNodeId.has(node.id));
  let maxX = 0;
  let maxY = 0;
  positioned.forEach((node) => {
    maxX = Math.max(maxX, node.position.x + (node.width ?? 0));
    maxY = Math.max(maxY, node.position.y + (node.height ?? 0));
  });
  const fallbackNodes = orphans.map((node, index) => {
    const size = NODE_SIZES[node.data.kind];
    const positionedNode = {
      ...node,
      position: { x: maxX + 40, y: index * (size.height + NODE_SPACING) },
      width: size.width,
      height: size.height,
    };
    maxX = Math.max(maxX, positionedNode.position.x + size.width);
    maxY = Math.max(maxY, positionedNode.position.y + size.height);
    return positionedNode;
  });

  const result: LayoutResult = {
    nodes: [...positioned, ...fallbackNodes],
    revision: input.revision,
    width: Math.max(720, maxX + PADDING),
    height: Math.max(420, maxY + PADDING),
  };
  layoutCache.set(input.revision, result);
  return result;
}

/**
 * Re-anchor disconnected nodes into their kind's column.
 *
 * ELK layered gives every node without edges its own layer (x position), so
 * an unlinked provider/pool lands at the far left next to the gateway instead
 * of in its own column — or worse, several of them pile onto the same spot.
 * This pass:
 *   1. derives each kind's column x from its *connected* nodes (median),
 *   2. snaps any node whose x deviates from that column back onto it,
 *   3. stacks snapped nodes below the existing column, in ELK's vertical order.
 * Kinds with no connected anchor (all instances isolated) interpolate the
 * column x from their nearest neighbour columns.
 */
function fixPhantomColumns(nodes: TopologyFlowNode[], edges: TopologyFlowEdge[]): void {
  const KIND_ORDER: TopologyNodeKind[] = ["gateway", "protocol", "pool", "provider", "account"];
  const connected = new Set<string>();
  edges.forEach((edge) => {
    connected.add(edge.source);
    connected.add(edge.target);
  });

  // 1. Column x per kind from connected nodes only.
  const xsByKind = new Map<TopologyNodeKind, number[]>();
  nodes.forEach((node) => {
    if (!connected.has(node.id)) return;
    const list = xsByKind.get(node.data.kind) ?? [];
    list.push(node.position.x);
    xsByKind.set(node.data.kind, list);
  });
  const connectedX = new Map<TopologyNodeKind, number>();
  xsByKind.forEach((xs, kind) => {
    xs.sort((a, b) => a - b);
    connectedX.set(kind, xs[Math.floor(xs.length / 2)] ?? 0);
  });

  // 2. Missing kinds (all instances isolated) get a column between neighbours.
  const anchorX = new Map<TopologyNodeKind, number>();
  KIND_ORDER.forEach((kind, index) => {
    if (connectedX.has(kind)) {
      anchorX.set(kind, connectedX.get(kind)!);
      return;
    }
    const prev = KIND_ORDER.slice(0, index).reverse().find((k) => connectedX.has(k));
    const next = KIND_ORDER.slice(index + 1).find((k) => connectedX.has(k));
    if (prev && next) anchorX.set(kind, (connectedX.get(prev)! + connectedX.get(next)!) / 2);
    else if (prev) anchorX.set(kind, connectedX.get(prev)! + LAYER_SPACING);
    else if (next) anchorX.set(kind, Math.max(PADDING, connectedX.get(next)! - LAYER_SPACING));
    else anchorX.set(kind, PADDING);
  });

  // 3. Split anchored vs phantom nodes, keeping ELK's y order for re-stacking.
  const COLUMN_TOLERANCE = 150;
  const columns = new Map<TopologyNodeKind, TopologyFlowNode[]>();
  const phantom: TopologyFlowNode[] = [];
  nodes.forEach((node) => {
    const x = anchorX.get(node.data.kind);
    if (x !== undefined && Math.abs(node.position.x - x) <= COLUMN_TOLERANCE) {
      const list = columns.get(node.data.kind) ?? [];
      list.push(node);
      columns.set(node.data.kind, list);
    } else {
      phantom.push(node);
    }
  });

  // 4. Stack phantom nodes below their kind's column, in ELK's vertical order.
  phantom.sort((a, b) => a.position.y - b.position.y);
  phantom.forEach((node) => {
    const x = anchorX.get(node.data.kind) ?? node.position.x;
    const size = NODE_SIZES[node.data.kind];
    const column = columns.get(node.data.kind) ?? [];
    const lastBottom = column.reduce(
      (max, item) => Math.max(max, item.position.y + (item.height ?? 0)),
      PADDING,
    );
    node.position = { x, y: lastBottom + NODE_SPACING };
    node.width = size.width;
    node.height = size.height;
    column.push(node);
  });

  // 5. Phantom layers pushed the whole graph right, leaving an empty band on
  // the left; collapse it so the gateway column starts at the padding edge.
  let minX = Infinity;
  nodes.forEach((node) => {
    minX = Math.min(minX, node.position.x);
  });
  if (Number.isFinite(minX) && minX !== PADDING) {
    const shift = PADDING - minX;
    nodes.forEach((node) => {
      node.position = { ...node.position, x: node.position.x + shift };
    });
  }
}

function fallbackLayout(input: LayoutInput): LayoutResult {
  const kinds: TopologyNodeKind[] = ["gateway", "protocol", "pool", "provider", "account"];
  const columns = new Map<TopologyNodeKind, TopologyFlowNode[]>();
  input.nodes.forEach((node) => {
    const list = columns.get(node.data.kind) ?? [];
    list.push(node);
    columns.set(node.data.kind, list);
  });
  const nodes: TopologyFlowNode[] = [];
  let maxX = 0;
  let maxY = 0;
  let columnX = PADDING;
  kinds.forEach((kind) => {
    const column = columns.get(kind) ?? [];
    const width = NODE_SIZES[kind].width;
    const totalHeight = column.reduce((sum, node) => sum + (NODE_SIZES[node.data.kind]?.height ?? 80) + NODE_SPACING, 0) - NODE_SPACING;
    let y = PADDING + Math.max(0, (420 - totalHeight) / 2);
    column.forEach((node) => {
      const size = NODE_SIZES[node.data.kind];
      nodes.push({ ...node, position: { x: columnX, y }, width: size.width, height: size.height });
      y += size.height + NODE_SPACING;
      maxY = Math.max(maxY, y);
    });
    columnX += width + LAYER_SPACING;
    maxX = columnX;
  });
  return { nodes, revision: input.revision, width: maxX, height: Math.max(420, maxY + PADDING) };
}

export function clearLayoutCache() {
  layoutCache.clear();
}
