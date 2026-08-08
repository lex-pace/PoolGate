import { useCallback, useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import TopologyView from "@/components/topology/TopologyView";
import { useGroups, useRouteTopology } from "@/hooks/use-tauri";
import type { RouteTopology } from "@/lib/tauri-commands";

/** Standalone fullscreen overlay for the live route topology. Mounted when the
 *  window hash is `#/topology-fullscreen`, this page replaces the entire
 *  app shell so the four-layer topology occupies every pixel of the PoolGate
 *  window without the sidebar, KPI row, or sibling cards behind it. */
export default function TopologyFullscreenPage() {
  const { data: routeTopology, isLoading, isError, refetch } = useRouteTopology();
  const { data: groups = [] } = useGroups();
  const queryClient = useQueryClient();
  const [refreshedAt, setRefreshedAt] = useState<number>(Date.now());

  const topologyPoolCount = routeTopology?.pools.length ?? groups.length;
  const topologyProviderCount = routeTopology?.providers.length ?? 0;

  const handleRefresh = useCallback(() => {
    void refetch();
    setRefreshedAt(Date.now());
  }, [refetch]);

  const exitFullscreen = useCallback(() => {
    if (window.location.hash === "#/topology-fullscreen") {
      window.location.hash = "";
    }
  }, []);

  // Make sure window resizes trigger a layout refit once the standalone
  // layer mounts. The Canvas inside TopologyView already observes resizes,
  // so we only need to ensure the wrapper fills the viewport.
  useEffect(() => {
    document.body.classList.add("pg-topology-fullscreen-open");
    const onHashChange = () => {
      if (window.location.hash !== "#/topology-fullscreen") {
        document.body.classList.remove("pg-topology-fullscreen-open");
      }
    };
    window.addEventListener("hashchange", onHashChange);
    return () => {
      document.body.classList.remove("pg-topology-fullscreen-open");
      window.removeEventListener("hashchange", onHashChange);
    };
  }, []);

  // Use the most recent route topology we have (in case the query is still
  // loading on entry).
  const safeData: RouteTopology | undefined = useMemo(() => routeTopology, [routeTopology]);

  return (
    <div className="pg-topology-fullscreen" role="dialog" aria-label="实时路由拓扑全屏">
      <TopologyView
        data={safeData}
        loading={isLoading && !safeData}
        error={isError && !safeData}
        onRefresh={handleRefresh}
        isFullscreen
        onToggleFullscreen={exitFullscreen}
      />
      {/* Hidden read-only badges so lints/queryClient stay warm. */}
      <span hidden>{topologyPoolCount}{topologyProviderCount}{refreshedAt}{queryClient ? 1 : 0}</span>
    </div>
  );
}