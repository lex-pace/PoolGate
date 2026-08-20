import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Background,
  BackgroundVariant,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  useStore,
  type NodeMouseHandler,
  type EdgeMouseHandler,
  type OnSelectionChangeParams,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { AlertTriangle, RefreshCw } from "lucide-react";
import type { RouteTopology } from "@/lib/tauri-commands";
import TopologyToolbar from "./Toolbar";
import TopologyInspector from "./Inspector";
import Legend from "./Legend";
import { TopologyNode } from "./node";
import { TopologyEdge } from "./edge";
import { layoutTopology } from "./layout";
import { buildTopologyGraph, NODE_ID_GATEWAY, accountNodeId, poolNodeId, providerNodeId } from "./graph";
import { useTopologyRuntime } from "./useTopologyRuntime";
import type { TopologyFlowEdge, TopologyFlowNode, TopologyProps, TopologySelection } from "./types";

const nodeTypes = { topology: TopologyNode };
const edgeTypes = { topology: TopologyEdge };

type SearchResult = {
  id: string;
  entityId: string;
  nodeKind: "gateway" | "protocol" | "pool" | "provider" | "account";
  label: string;
  kind: string;
};

type SearchState = {
  open: boolean;
  query: string;
  results: SearchResult[];
};

function buildSearchIndex(data: RouteTopology): SearchResult[] {
  const results: SearchResult[] = [];
  results.push({ id: NODE_ID_GATEWAY, entityId: NODE_ID_GATEWAY, nodeKind: "gateway", label: data.gateway.name, kind: "网关" });
  data.protocols.forEach((protocol) => results.push({ id: protocol.id, entityId: protocol.id, nodeKind: "protocol", label: protocol.name, kind: "协议" }));
  data.pools.forEach((pool) => results.push({ id: poolNodeId(pool.id), entityId: pool.id, nodeKind: "pool", label: pool.name, kind: "路由池" }));
  data.providers.forEach((provider) => results.push({ id: providerNodeId(provider.id), entityId: provider.id, nodeKind: "provider", label: provider.name, kind: "厂商" }));
  data.accounts.forEach((account) => results.push({ id: accountNodeId(account.id), entityId: account.id, nodeKind: "account", label: account.name, kind: "账号" }));
  return results;
}

type CanvasProps = {
  data: RouteTopology;
  selection: TopologySelection;
  onSelect: (selection: TopologySelection) => void;
  filterAbnormal: boolean;
  liveEdges: Map<string, number>;
  liveNodes: Map<string, number>;
  flashEdges: Array<{ edgeId: string; status: "success" | "failed"; at: number }>;
  search: SearchState;
  onSearchResult: (id: string) => void;
  focusNodeId?: string;
  onZoomChange: (zoom: number) => void;
  /** Triggers a fitView whenever the value changes (used for inspector toggle). */
  fitSignal: number;
};

const COLUMN_LABELS: Record<string, string> = { gateway: "网关", protocol: "协议", pool: "路由池", provider: "上游厂商" };
const COLUMN_ORDER = ["gateway", "protocol", "pool", "provider"] as const;

type ColumnHeading = { kind: string; x: number; y: number; label: string };

function Canvas({
  data,
  selection,
  onSelect,
  filterAbnormal,
  liveEdges,
  liveNodes,
  flashEdges,
  search,
  onSearchResult,
  focusNodeId,
  onZoomChange,
  fitSignal,
}: CanvasProps) {
  const { fitView, setCenter, getNodes } = useReactFlow<TopologyFlowNode, TopologyFlowEdge>();
  // Viewport transform (flow space → screen space) so the per-layer heading
  // pills stay glued to their columns while panning/zooming.
  const viewportTransform = useStore((state) => state.transform);
  const [layout, setLayout] = useState<{ nodes: TopologyFlowNode[]; revision: number } | null>(null);
  const layoutRevision = data.topology_revision;
  const focusHandled = useRef<string | null>(null);

  // Build the graph model (pure, cheap).
  const graph = useMemo(
    () => buildTopologyGraph({
      data,
      selection,
      filterAbnormal,
      liveEdges,
      liveNodes,
      activeRoute: data.active_route,
    }),
    [data, selection, filterAbnormal, liveEdges, liveNodes],
  );

  // Apply transient flash to edges.
  const edgesWithFlash = useMemo(() => {
    if (!flashEdges.length) return graph.edges;
    const now = Date.now();
    const byId = new Map<string, "success" | "failed">();
    flashEdges.forEach((flash) => {
      if (now - flash.at < 1000) byId.set(flash.edgeId, flash.status);
    });
    if (!byId.size) return graph.edges;
    return graph.edges.map((edge) => {
      const flash = byId.get(edge.id);
      return flash ? { ...edge, data: { ...edge.data!, flash } } : edge;
    });
  }, [graph.edges, flashEdges]);

  // ELK layout only when the structure revision changes.
  useEffect(() => {
    let cancelled = false;
    if (layoutRevision === layout?.revision) return;
    void layoutTopology({ nodes: graph.nodes, edges: graph.edges, revision: layoutRevision }).then((result) => {
      if (cancelled) return;
      setLayout(result);
      // Refit the viewport after the layout engine has positioned the nodes;
      // the initial fitView runs before ELK resolves (async), so this picks up
      // any width changes and re-centers the four-layer graph.
      requestAnimationFrame(() => { void fitView({ padding: 0.12, duration: 300 }); });
    });
    return () => { cancelled = true; };
  }, [layoutRevision, graph.nodes, graph.edges, layout?.revision, fitView]);

  // Refit the canvas when the host triggers it (inspector toggle, abnormal
  // filter, etc.) so the graph auto-scales to the new viewport width.
  useEffect(() => {
    if (layoutRevision === layout?.revision && fitSignal > 0) {
      requestAnimationFrame(() => { void fitView({ padding: 0.12, duration: 250 }); });
    }
  }, [fitSignal, layout?.revision, layoutRevision, fitView]);

  // Refit when the canvas wrapper resizes (window resize, sidebar collapse,
  // etc.) so the four-layer graph always re-centers to the new viewport.
  const wrapperRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!wrapperRef.current) return;
    const observer = new ResizeObserver(() => {
      requestAnimationFrame(() => { void fitView({ padding: 0.12, duration: 200 }); });
    });
    observer.observe(wrapperRef.current);
    return () => observer.disconnect();
  }, [fitView, layout?.revision]);

  const positionedNodes = useMemo(() => {
    if (!layout || layout.revision !== data.topology_revision) return graph.nodes;
    const byId = new Map(layout.nodes.map((node) => [node.id, node]));
    return graph.nodes.map((node) => {
      const positioned = byId.get(node.id);
      return positioned ? { ...node, position: positioned.position, width: positioned.width, height: positioned.height } : node;
    });
  }, [layout, graph.nodes, data.topology_revision]);

  // Column heading pills: one row at the top of the graph canvas, each pill
  // centered over its layer's x-extent (mockup: 网关·1 / 协议·3 / …).
  const columnHeadings = useMemo<ColumnHeading[]>(() => {
    if (!layout || layout.revision !== data.topology_revision) return [];
    const counts: Record<string, number> = { gateway: 1, protocol: graph.protocolCount, pool: graph.poolCount, provider: graph.providerCount };
    const extent = new Map<string, { minX: number; maxX: number }>();
    let top = Infinity;
    positionedNodes.forEach((node) => {
      const kind = node.data.kind;
      top = Math.min(top, node.position.y);
      if (!(kind in COLUMN_LABELS)) return;
      const box = extent.get(kind) ?? { minX: Infinity, maxX: -Infinity };
      box.minX = Math.min(box.minX, node.position.x);
      box.maxX = Math.max(box.maxX, node.position.x + (node.width ?? 0));
      extent.set(kind, box);
    });
    if (!Number.isFinite(top)) return [];
    return COLUMN_ORDER.filter((kind) => extent.has(kind)).map((kind) => {
      const box = extent.get(kind)!;
      return { kind, x: (box.minX + box.maxX) / 2, y: top - 36, label: `${COLUMN_LABELS[kind]} · ${counts[kind]}` };
    });
  }, [layout, positionedNodes, graph.protocolCount, graph.poolCount, graph.providerCount, data.topology_revision]);

  const selectNode = useCallback((node: TopologyFlowNode) => {
    onSelect({ kind: "node", id: node.id, entityId: node.data.id, nodeKind: node.data.kind });
  }, [onSelect]);

  const onNodeClick: NodeMouseHandler<TopologyFlowNode> = useCallback((_, node) => {
    selectNode(node);
  }, [selectNode]);

  const onEdgeClick: EdgeMouseHandler<TopologyFlowEdge> = useCallback((_, edge) => {
    onSelect({ kind: "edge", edgeId: edge.id });
  }, [onSelect]);

  const onPaneClick = useCallback(() => {
    // Empty-space clicks only clear React Flow's visual selection. The details
    // panel is opened by a node/edge click and closed explicitly from its header.
  }, []);

  const onSelectionChange = useCallback(({ nodes, edges }: OnSelectionChangeParams<TopologyFlowNode, TopologyFlowEdge>) => {
    if (nodes.length) {
      selectNode(nodes[0]);
    } else if (edges.length) {
      onSelect({ kind: "edge", edgeId: edges[0].id });
    }
  }, [onSelect, selectNode]);

  const onMove = useCallback((_: unknown, viewport: { zoom: number }) => {
    onZoomChange(viewport.zoom);
  }, [onZoomChange]);

  // Focus a node from a deep link (tray jump).
  useEffect(() => {
    if (!focusNodeId || focusHandled.current === focusNodeId) return;
    const target = graph.nodes.find((node) => node.id === focusNodeId);
    if (!target) return;
    focusHandled.current = focusNodeId;
    const node = getNodes().find((item) => item.id === focusNodeId);
    if (node) {
      const { x, y } = node.position;
      void setCenter(x + (node.width ?? 0) / 2, y + (node.height ?? 0) / 2, { zoom: 1, duration: 400 });
      selectNode(node);
    }
  }, [focusNodeId, graph.nodes, getNodes, setCenter, selectNode]);

  return (
    <div className="pg-tv-canvas-wrap" ref={wrapperRef}>
      <div
        className="pg-tv-columns"
        aria-hidden="true"
        style={{ transform: `translate(${viewportTransform[0]}px, ${viewportTransform[1]}px) scale(${viewportTransform[2]})` }}
      >
        {columnHeadings.map((heading) => (
          <span key={heading.kind} className={`pg-tv-column-label layer-${heading.kind}`} style={{ left: heading.x, top: heading.y }}>
            {heading.label}
          </span>
        ))}
      </div>
      <ReactFlow
        nodes={positionedNodes}
        edges={edgesWithFlash}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodeClick={onNodeClick}
        onEdgeClick={onEdgeClick}
        onPaneClick={onPaneClick}
        onDoubleClick={(event) => {
          if ((event.target as HTMLElement).closest(".react-flow__node, .react-flow__edge")) return;
          void fitView({ padding: 0.15, duration: 300 });
        }}
        onSelectionChange={onSelectionChange}
        onMove={onMove}
        fitView
        fitViewOptions={{ padding: 0.12, duration: 300 }}
        minZoom={0.2}
        maxZoom={1.8}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable
        deleteKeyCode={null}
        panOnDrag
        selectionOnDrag={false}
        zoomOnScroll={true}
        zoomOnDoubleClick={false}
        proOptions={{ hideAttribution: true }}
        className="pg-tv-flow"
      >
        <Background variant={BackgroundVariant.Lines} gap={50} size={1} color="var(--grid-line)" />
      </ReactFlow>
      {search.open && (
        <div className="pg-tv-search" onPointerDown={(event) => event.stopPropagation()}>
          <input
            autoFocus
            placeholder="搜索网关 / 协议 / 路由池 / 厂商…"
            value={search.query}
            onChange={(event) => onSearchResult(event.target.value)}
          />
          {search.results.length > 0 && (
            <div className="pg-tv-search-results">
              {search.results.slice(0, 8).map((result) => (
                <button key={result.id} onClick={() => onSearchResult(result.id)}>
                  <span>{result.kind}</span>
                  <strong>{result.label}</strong>
                </button>
              ))}
            </div>
          )}
          {search.query && !search.results.length && <div className="pg-tv-search-empty">未找到匹配节点</div>}
        </div>
      )}
    </div>
  );
}

/** Inner shell rendered inside ReactFlowProvider so toolbar controls can drive
 *  the live React Flow instance directly. */
function TopologyInner(props: TopologyProps & {
  selection: TopologySelection;
  onSelect: (selection: TopologySelection) => void;
  filterAbnormal: boolean;
  onToggleFilter: () => void;
  search: SearchState;
  onSearchResult: (id: string) => void;
  onToggleSearch: () => void;
  zoom: number;
  onZoomChange: (zoom: number) => void;
  showInspector: boolean;
  onToggleInspector: () => void;
  onCloseInspector: () => void;
  isFullscreen: boolean;
  onToggleFullscreen: () => void;
  fitSignal: number;
}) {
  const { data, showInspector } = props;
  const { fitView, setViewport, zoomIn, zoomOut } = useReactFlow<TopologyFlowNode, TopologyFlowEdge>();
  const runtime = useTopologyRuntime(!!data);

  // Keyboard: ⌘+/⌘- zoom, ⌘0 reset (driven by the live instance).
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const command = event.metaKey || event.ctrlKey;
      if (command && event.key === "+") { event.preventDefault(); void zoomIn({ duration: 150 }); }
      if (command && event.key === "-") { event.preventDefault(); void zoomOut({ duration: 150 }); }
      if (command && event.key === "0") { event.preventDefault(); setViewport({ x: 0, y: 0, zoom: 1 }, { duration: 200 }); }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [zoomIn, zoomOut, setViewport]);

  const updatedAtLabel = useMemo(() => {
    if (!data?.updated_at) return undefined;
    return new Date(data.updated_at).toLocaleTimeString("zh-CN", { hour12: false });
  }, [data?.updated_at]);

  const abnormalCount = useMemo(() => {
    if (!data) return 0;
    return data.providers.filter((p) => !p.enabled || p.healthy_account_count === 0).length;
  }, [data]);

  if (!data) return null;
  const emptyPool = data.pools.length === 0;
  const emptyProvider = data.pools.length > 0 && data.providers.length === 0;

  return (
    <div className={`pg-tv-shell ${showInspector ? "inspector-open" : "inspector-closed"}`}>
      <TopologyToolbar
        protocolCount={data.protocols.length}
        poolCount={data.pools.length}
        providerCount={data.providers.length}
        activeConnections={data.gateway.active_connections}
        zoom={props.zoom}
        updatedAt={updatedAtLabel}
        stale={runtime.stale}
        filterAbnormal={props.filterAbnormal}
        abnormalCount={abnormalCount}
        searching={props.search.open}
        showInspector={showInspector}
        isFullscreen={props.isFullscreen}
        onZoomIn={() => { void zoomIn({ duration: 150 }); }}
        onZoomOut={() => { void zoomOut({ duration: 150 }); }}
        onToggleFilter={props.onToggleFilter}
        onToggleSearch={props.onToggleSearch}
        onToggleInspector={props.onToggleInspector}
        onToggleFullscreen={props.onToggleFullscreen}
        onRefresh={props.onRefresh}
      />
      <div className="pg-tv-content">
        <Canvas
          data={data}
          selection={props.selection}
          onSelect={props.onSelect}
          filterAbnormal={props.filterAbnormal}
          liveEdges={runtime.edgeRequests}
          liveNodes={runtime.nodeConcurrency}
          flashEdges={runtime.flashes}
          search={props.search}
          onSearchResult={props.onSearchResult}
          focusNodeId={props.focusNodeId}
          onZoomChange={props.onZoomChange}
          fitSignal={props.fitSignal}
        />
        {showInspector && (
          <TopologyInspector
            data={data}
            selection={props.selection}
            onSelect={props.onSelect}
            onClose={props.onCloseInspector}
            onOpenDashboard={() => undefined}
          />
        )}
      </div>
      {(emptyPool || emptyProvider) && (
        <div className="pg-tv-empty-banner">
          {emptyPool ? (
            <>
              <span>暂无路由池，创建后自动生成下游路由</span>
              <button onClick={() => window.dispatchEvent(new CustomEvent("poolgate:navigate", { detail: "groups" }))}>创建路由池</button>
            </>
          ) : (
            <>
              <span>路由池尚未关联上游厂商</span>
              <button onClick={() => window.dispatchEvent(new CustomEvent("poolgate:navigate", { detail: "resources" }))}>配置模型供应商</button>
            </>
          )}
        </div>
      )}
      <Legend />
    </div>
  );
}

export default function TopologyView(props: TopologyProps) {
  const { data, loading, error, onRefresh, focusNodeId } = props;
  const [selection, setSelection] = useState<TopologySelection>({ kind: "overview" });
  const [filterAbnormal, setFilterAbnormal] = useState(false);
  const [search, setSearch] = useState<SearchState>({ open: false, query: "", results: [] });
  const [zoom, setZoom] = useState(1);
  // The default view is the topology canvas only. Selecting a node or edge
  // opens its details on the right; the canvas re-fits to the remaining space.
  const [showInspector, setShowInspector] = useState(false);
  // Monotonic counter that forces a fitView inside the canvas (used when the
  // inspector is opened/closed or the abnormal filter toggles).
  const [fitSignal, setFitSignal] = useState(0);
  const searchIndex = useMemo(() => (data ? buildSearchIndex(data) : []), [data]);

  // Keyboard: ⌘F search toggle, Esc back to overview (zoom handled in provider).
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const command = event.metaKey || event.ctrlKey;
      if (command && event.key.toLowerCase() === "f") { event.preventDefault(); setSearch((value) => ({ ...value, open: !value.open })); }
      if (event.key === "Escape") {
        setSearch((value) => (value.open ? { ...value, open: false } : value));
        setSelection({ kind: "overview" });
        setShowInspector(false);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const handleSelect = useCallback((next: TopologySelection) => {
    setSelection(next);
    if (next.kind !== "overview") {
      setShowInspector(true);
      // The detail card floats above the canvas (mockup), so the canvas keeps
      // its full width — no re-fit needed.
    }
  }, []);

  const handleToggleInspector = useCallback(() => {
    setShowInspector((visible) => !visible);
  }, []);

  const handleCloseInspector = useCallback(() => {
    setShowInspector(false);
  }, []);

  const handleToggleFilter = useCallback(() => {
    setFilterAbnormal((value) => !value);
    setFitSignal((signal) => signal + 1);
  }, []);

  const handleSearchResult = useCallback((idOrQuery: string) => {
    // A graph node id was picked from the results. Resolve it from the search
    // index instead of reverse-parsing a prefixed id; entity ids may already
    // begin with "pool-" or "provider-".
    const selectedResult = searchIndex.find((item) => item.id === idOrQuery);
    if (selectedResult) {
      setSearch({ open: false, query: "", results: [] });
      setSelection({
        kind: "node",
        id: selectedResult.id,
        entityId: selectedResult.entityId,
        nodeKind: selectedResult.nodeKind,
      });
      setShowInspector(true);
      return;
    }
    const normalized = idOrQuery.trim().toLowerCase();
    if (!normalized) {
      setSearch((value) => ({ ...value, query: idOrQuery, results: [] }));
      return;
    }
    const results = searchIndex
      .filter((item) => item.label.toLowerCase().includes(normalized) || item.kind.includes(normalized))
      .slice(0, 8);
    setSearch((value) => ({ ...value, query: idOrQuery, results }));
  }, [searchIndex]);

  // Loading / error / empty states (V2 spec §8).
  if (loading) {
    return (
      <div className="pg-tv-state" role="status">
        <RefreshCw size={20} className="animate-spin" />
        <strong>正在构建四层路由拓扑</strong>
        <span>加载网关、协议、路由池与上游厂商</span>
      </div>
    );
  }
  if (error || !data) {
    return (
      <div className="pg-tv-state error" role="alert">
        <AlertTriangle size={21} />
        <strong>实时路由拓扑加载失败</strong>
        <span>数据源暂时不可用，请检查网关与数据库状态</span>
        <button onClick={onRefresh}>重新加载</button>
      </div>
    );
  }

  return (
    <ReactFlowProvider>
      <TopologyInner
        {...props}
        isFullscreen={props.isFullscreen ?? false}
        onToggleFullscreen={props.onToggleFullscreen ?? (() => undefined)}
        selection={selection}
        onSelect={handleSelect}
        filterAbnormal={filterAbnormal}
        onToggleFilter={handleToggleFilter}
        search={search}
        onSearchResult={handleSearchResult}
        onToggleSearch={() => setSearch((value) => ({ ...value, open: !value.open }))}
        zoom={zoom}
        onZoomChange={setZoom}
        showInspector={showInspector}
        onToggleInspector={handleToggleInspector}
        onCloseInspector={handleCloseInspector}
        fitSignal={fitSignal}
      />
    </ReactFlowProvider>
  );
}
