import { AlertTriangle, ChevronDown, Eye, EyeOff, Maximize2, Minimize2, Minus, Plus, RotateCcw, Search } from "lucide-react";

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
        <span><b>{activeConnections}</b>ACTIVE</span>
        <span><b>{protocolCount}</b>PROTOCOLS</span>
        <span><b>{poolCount}</b>POOLS</span>
        <span><b>{providerCount}</b>PROVIDERS</span>
      </div>
      <div className="pg-tv-tools">
        {stale && <span className="pg-tv-stale" title="实时数据超过 10 秒未更新"><AlertTriangle size={12} />数据可能已过期</span>}
        {updatedAt && !stale && <span className="pg-tv-updated">更新于 {updatedAt}</span>}
        <button
          onClick={onToggleFilter}
          className={`pg-tv-view-filter ${filterAbnormal ? "active" : ""}`}
          data-tip={filterAbnormal ? "显示全部节点" : "只显示异常分支"}
          title={filterAbnormal ? "显示全部节点" : "只显示异常分支"}
          aria-label="显示节点范围"
          disabled={abnormalCount === 0}
        >
          显示：{filterAbnormal ? "异常分支" : "全部节点"}<ChevronDown size={12} />
        </button>
        <button
          onClick={onToggleFilter}
          className={`pg-tv-alert-pill ${abnormalCount > 0 ? "has-alerts" : ""}`}
          data-tip={`${abnormalCount} 个告警，点击筛选异常分支`}
          title={`${abnormalCount} 个告警，点击筛选异常分支`}
          aria-label="告警筛选"
          disabled={abnormalCount === 0}
        >
          <i />{abnormalCount} 个告警
        </button>
        <span className="pg-tv-toolbar-divider" aria-hidden="true" />
        <button onClick={onRefresh} data-tip="刷新数据" title="刷新" aria-label="刷新"><RotateCcw size={13} /></button>
        <button onClick={onToggleSearch} className={searching ? "active" : ""} data-tip="定位节点 (⌘F)" title="定位节点 (⌘F)" aria-label="定位节点"><Search size={13} /></button>
        <button
          onClick={onToggleInspector}
          className={`pg-tv-inspector-toggle ${showInspector ? "active" : ""}`}
          data-tip={showInspector ? "关闭路由节点详情" : "显示路由节点详情"}
          title={showInspector ? "关闭路由节点详情" : "显示路由节点详情"}
          aria-label={showInspector ? "关闭路由节点详情" : "显示路由节点详情"}
          aria-pressed={showInspector}
        >
          {showInspector ? <EyeOff size={13} /> : <Eye size={13} />}
        </button>
        <button onClick={onZoomOut} data-tip="缩小 (⌘-)" title="缩小 (⌘-)" aria-label="缩小"><Minus size={13} /></button>
        <span className="pg-tv-zoom">{Math.round(zoom * 100)}%</span>
        <button onClick={onZoomIn} data-tip="放大 (⌘+)" title="放大 (⌘+)" aria-label="放大"><Plus size={13} /></button>
        <span className="pg-tv-toolbar-divider" aria-hidden="true" />
        <button
          onClick={onToggleFullscreen}
          className={`pg-tv-fullscreen ${isFullscreen ? "active" : ""}`}
          data-tip={isFullscreen ? "退出应用全屏 (Esc)" : "应用全屏展示"}
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
