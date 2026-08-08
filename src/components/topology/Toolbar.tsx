import { AlertTriangle, Eye, EyeOff, Maximize2, Minimize2, Minus, Plus, RotateCcw, Search } from "lucide-react";

type ToolbarProps = {
  protocolCount: number;
  poolCount: number;
  providerCount: number;
  activeConnections: number;
  zoom: number;
  updatedAt?: string;
  stale: boolean;
  filterAbnormal: boolean;
  abnormalCount: number;
  searching: boolean;
  showInspector: boolean;
  isFullscreen: boolean;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onToggleFilter: () => void;
  onToggleSearch: () => void;
  onToggleInspector: () => void;
  onToggleFullscreen: () => void;
  onRefresh: () => void;
};

export default function TopologyToolbar({
  protocolCount,
  poolCount,
  providerCount,
  activeConnections,
  zoom,
  updatedAt,
  stale,
  filterAbnormal,
  abnormalCount,
  searching,
  showInspector,
  isFullscreen,
  onZoomIn,
  onZoomOut,
  onToggleFilter,
  onToggleSearch,
  onToggleInspector,
  onToggleFullscreen,
  onRefresh,
}: ToolbarProps) {
  return (
    <div className="pg-tv-toolbar">
      <div className="pg-tv-layer-summary">
        <span><b>1</b>GATEWAY</span>
        <span><b>{protocolCount}</b>PROTOCOLS</span>
        <span><b>{poolCount}</b>POOLS</span>
        <span><b>{providerCount}</b>PROVIDERS</span>
        <span><b>{activeConnections}</b>ACTIVE</span>
      </div>
      <div className="pg-tv-tools">
        {stale && <span className="pg-tv-stale" title="实时数据超过 10 秒未更新"><AlertTriangle size={12} />数据可能已过期</span>}
        {updatedAt && !stale && <span className="pg-tv-updated">更新于 {updatedAt}</span>}
        <button onClick={onRefresh} title="刷新" aria-label="刷新"><RotateCcw size={13} /></button>
        <button
          onClick={onToggleInspector}
          className={`pg-tv-inspector-toggle ${showInspector ? "active" : ""}`}
          title={showInspector ? "关闭路由节点详情" : "显示路由节点详情"}
          aria-label={showInspector ? "关闭路由节点详情" : "显示路由节点详情"}
          aria-pressed={showInspector}
        >
          {showInspector ? <EyeOff size={13} /> : <Eye size={13} />}
        </button>
        <button onClick={onToggleSearch} className={searching ? "active" : ""} title="定位节点 (⌘F)" aria-label="定位节点"><Search size={13} /></button>
        <button
          onClick={onToggleFilter}
          className={`pg-tv-filter ${filterAbnormal ? "active" : ""}`}
          title="异常筛选：只显示告警/故障分支"
          aria-label="异常筛选"
          disabled={abnormalCount === 0}
        >
          <AlertTriangle size={13} />
          {abnormalCount > 0 && <span className="pg-tv-filter-count">{abnormalCount}</span>}
        </button>
        <button onClick={onZoomOut} title="缩小 (⌘-)" aria-label="缩小"><Minus size={13} /></button>
        <span className="pg-tv-zoom">{Math.round(zoom * 100)}%</span>
        <button onClick={onZoomIn} title="放大 (⌘+)" aria-label="放大"><Plus size={13} /></button>
        <span className="pg-tv-toolbar-divider" aria-hidden="true" />
        <button
          onClick={onToggleFullscreen}
          className={`pg-tv-fullscreen ${isFullscreen ? "active" : ""}`}
          title={isFullscreen ? "退出应用全屏 (Esc)" : "应用全屏展示"}
          aria-label={isFullscreen ? "退出应用全屏" : "应用全屏展示"}
          aria-pressed={isFullscreen}
        >
          {isFullscreen ? <Minimize2 size={13} /> : <Maximize2 size={13} />}
        </button>
      </div>
    </div>
  );
}
