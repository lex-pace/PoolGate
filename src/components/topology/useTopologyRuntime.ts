import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { TopologyRuntimeDelta } from "@/lib/tauri-commands";

export interface RuntimeState {
  sequence: number;
  activeConnections: number;
  /** Edge id -> active_requests, accumulated between snapshots. */
  edgeRequests: Map<string, number>;
  /** Node id -> active concurrency. */
  nodeConcurrency: Map<string, number>;
  /** Recently completed requests used to flash edges briefly. */
  flashes: Array<{ edgeId: string; status: "success" | "failed"; at: number }>;
  updatedAt: number;
  stale: boolean;
}

const EMPTY: RuntimeState = {
  sequence: 0,
  activeConnections: 0,
  edgeRequests: new Map(),
  nodeConcurrency: new Map(),
  flashes: [],
  updatedAt: 0,
  stale: false,
};

/**
 * Subscribes to `topology:runtime-delta` events and merges them into a compact
 * runtime state. The backend merges events every 250ms; here we keep the latest
 * per-edge/node counts and a short-lived flash list for completed requests.
 * Delays are applied on the UI layer, not here.
 */
export function useTopologyRuntime(enabled: boolean) {
  const [state, setState] = useState<RuntimeState>(EMPTY);
  const sequenceRef = useRef(0);
  const flashesRef = useRef<Array<{ edgeId: string; status: "success" | "failed"; at: number }>>([]);

  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    const unlistenPromise = listen<TopologyRuntimeDelta>("topology:runtime-delta", ({ payload }) => {
      if (disposed || payload.sequence <= sequenceRef.current) return;
      sequenceRef.current = payload.sequence;

      const edgeRequests = new Map<string, number>();
      payload.edge_deltas.forEach((delta) => {
        if (delta.active_requests > 0) edgeRequests.set(delta.id, delta.active_requests);
      });
      const nodeConcurrency = new Map<string, number>();
      payload.node_deltas.forEach((delta) => {
        if (delta.active_concurrency > 0) nodeConcurrency.set(delta.id, delta.active_concurrency);
      });

      const flashes: Array<{ edgeId: string; status: "success" | "failed"; at: number }> = [];
      payload.completed.forEach((completed) => {
        (completed.path_edge_ids || []).forEach((edgeId) => {
          flashes.push({ edgeId, status: completed.status === "success" ? "success" : "failed", at: Date.now() });
        });
      });
      // Keep a bounded history for the 1s fade-out on the UI side.
      flashesRef.current = [...flashesRef.current.filter((f) => Date.now() - f.at < 1500), ...flashes].slice(-16);

      setState({
        sequence: payload.sequence,
        activeConnections: payload.active_connections,
        edgeRequests,
        nodeConcurrency,
        flashes: [...flashesRef.current],
        updatedAt: Date.now(),
        stale: false,
      });
    });
    unlistenPromise.catch(() => {
      // Running outside the Tauri runtime (e.g. design preview): no events.
    });
    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [enabled]);

  // Stale detection: 10s without updates shows "stale", 30s upgrades to warning.
  useEffect(() => {
    if (!enabled || state.sequence === 0) return;
    const timer = window.setInterval(() => {
      const elapsed = Date.now() - state.updatedAt;
      if (elapsed > 10_000) {
        setState((current) => (current.stale ? current : { ...current, stale: true }));
      }
    }, 5_000);
    return () => window.clearInterval(timer);
  }, [enabled, state.updatedAt, state.sequence]);

  return state;
}

/** Route an edge flash into the graph model (called by the canvas during render). */
export function applyFlashesToEdges<T extends { id: string; data?: { flash?: "success" | "failed" } }>(
  edges: T[],
  flashes: Array<{ edgeId: string; status: "success" | "failed"; at: number }>,
  now = Date.now(),
): T[] {
  if (!flashes.length) return edges;
  const flashByEdge = new Map<string, "success" | "failed">();
  flashes.forEach((flash) => {
    if (now - flash.at < 1000) flashByEdge.set(flash.edgeId, flash.status);
  });
  if (!flashByEdge.size) return edges;
  return edges.map((edge) => {
    const flash = flashByEdge.get(edge.id);
    return flash ? { ...edge, data: { ...edge.data, flash } } : edge;
  });
}
