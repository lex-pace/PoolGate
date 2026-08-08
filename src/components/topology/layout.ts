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
      "elk.edgeRouting": "ORTHOGONAL",
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
