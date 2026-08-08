use chrono::Utc;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::broadcast;

#[derive(Clone, Debug, serde::Serialize)]
pub struct RuntimeRoutePath {
    pub request_id: String,
    pub protocol: String,
    pub pool_id: String,
    pub provider_id: String,
    #[serde(skip_serializing)]
    pub account_id: String,
    pub status: String,
    pub attempt: usize,
    pub latency_ms: Option<i64>,
    pub started_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RuntimeActivePath {
    pub protocol: String,
    pub pool_id: String,
    pub provider_id: String,
    pub account_id: String,
    pub active_requests: u32,
    pub last_active_at: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RuntimeNodeDelta {
    pub id: String,
    pub active_concurrency: u32,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RuntimeEdgeDelta {
    pub id: String,
    pub active_requests: u32,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RuntimeCompletedRequest {
    pub request_id: String,
    pub status: String,
    pub latency_ms: i64,
    /// Edge ids the completed request traversed (gateway-protocol, protocol-pool,
    /// pool-provider). Lets the command center flash exactly those edges.
    pub path_edge_ids: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TopologyRuntimeDelta {
    pub sequence: u64,
    pub emitted_at: String,
    pub active_connections: u32,
    pub active_paths: Vec<RuntimeActivePath>,
    pub latest_route: Option<RuntimeRoutePath>,
    pub node_deltas: Vec<RuntimeNodeDelta>,
    pub edge_deltas: Vec<RuntimeEdgeDelta>,
    pub completed: Vec<RuntimeCompletedRequest>,
}

#[derive(Default)]
struct RuntimeState {
    active_routes: HashMap<String, RuntimeRoutePath>,
    latest_route: Option<RuntimeRoutePath>,
}

pub struct GatewayRuntime {
    active_connections: AtomicU32,
    sequence: AtomicU64,
    state: Mutex<RuntimeState>,
    events: broadcast::Sender<TopologyRuntimeDelta>,
}

impl Default for GatewayRuntime {
    fn default() -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            active_connections: AtomicU32::new(0),
            sequence: AtomicU64::new(0),
            state: Mutex::new(RuntimeState::default()),
            events,
        }
    }
}

pub struct ActiveConnectionGuard<'a> {
    runtime: &'a GatewayRuntime,
}

impl Drop for ActiveConnectionGuard<'_> {
    fn drop(&mut self) {
        let _ = self.runtime.active_connections.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |value| Some(value.saturating_sub(1)),
        );
        self.runtime.publish_delta(Vec::new());
    }
}

impl GatewayRuntime {
    /// Record a request connection being opened (manual counter, no RAII
    /// guard). Used by the proxy middleware for SSE streaming requests whose
    /// body keeps streaming after the response head is sent — the matching
    /// `connection_closed()` is called when the body finishes or drops.
    pub fn connection_opened(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
        self.publish_delta(Vec::new());
    }

    /// Record a request connection being closed. Matches `connection_opened()`
    /// for manually-managed (streaming) connections.
    pub fn connection_closed(&self) {
        let _ = self.active_connections.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |value| Some(value.saturating_sub(1)),
        );
        self.publish_delta(Vec::new());
    }

    pub fn begin_connection(&self) -> ActiveConnectionGuard<'_> {
        self.connection_opened();
        ActiveConnectionGuard { runtime: self }
    }

    pub fn active_connections(&self) -> u32 {
        self.active_connections.load(Ordering::Relaxed)
    }

    pub fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Relaxed)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<TopologyRuntimeDelta> {
        self.events.subscribe()
    }

    pub fn select_route(
        &self,
        request_id: &str,
        protocol: &str,
        pool_id: &str,
        provider_id: &str,
        account_id: &str,
        attempt: usize,
    ) {
        let now = Utc::now().to_rfc3339();
        let route = RuntimeRoutePath {
            request_id: request_id.to_string(),
            protocol: protocol.to_string(),
            pool_id: pool_id.to_string(),
            provider_id: provider_id.to_string(),
            account_id: account_id.to_string(),
            status: "active".to_string(),
            attempt,
            latency_ms: None,
            started_at: now.clone(),
            updated_at: now,
        };
        if let Ok(mut state) = self.state.lock() {
            state
                .active_routes
                .insert(request_id.to_string(), route.clone());
            state.latest_route = Some(route);
        }
        self.publish_delta(Vec::new());
    }

    pub fn complete_route(&self, request_id: &str, status: &str, latency_ms: i64) {
        let mut completed = Vec::new();
        if let Ok(mut state) = self.state.lock() {
            if let Some(mut route) = state.active_routes.remove(request_id) {
                route.status = status.to_string();
                route.latency_ms = Some(latency_ms);
                route.updated_at = Utc::now().to_rfc3339();
                let path_edge_ids = route_edge_ids(&route);
                state.latest_route = Some(route);
                completed.push(RuntimeCompletedRequest {
                    request_id: request_id.to_string(),
                    status: status.to_string(),
                    latency_ms,
                    path_edge_ids,
                });
            }
        }
        self.publish_delta(completed);
    }

    pub fn latest_route(&self) -> Option<RuntimeRoutePath> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.latest_route.clone())
    }

    pub fn active_paths(&self) -> Vec<RuntimeActivePath> {
        self.state
            .lock()
            .map(|state| aggregate_active_paths(&state.active_routes))
            .unwrap_or_default()
    }

    pub fn snapshot(&self) -> TopologyRuntimeDelta {
        self.build_delta(self.sequence(), Vec::new())
    }

    pub fn reset(&self) {
        self.active_connections.store(0, Ordering::Relaxed);
        if let Ok(mut state) = self.state.lock() {
            *state = RuntimeState::default();
        }
        self.publish_delta(Vec::new());
    }

    fn publish_delta(&self, completed: Vec<RuntimeCompletedRequest>) {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.events.send(self.build_delta(sequence, completed));
    }

    fn build_delta(
        &self,
        sequence: u64,
        completed: Vec<RuntimeCompletedRequest>,
    ) -> TopologyRuntimeDelta {
        let (active_paths, latest_route) = self
            .state
            .lock()
            .map(|state| {
                (
                    aggregate_active_paths(&state.active_routes),
                    state.latest_route.clone(),
                )
            })
            .unwrap_or_default();
        let active_connections = self.active_connections();
        let mut node_counts: BTreeMap<String, u32> = BTreeMap::new();
        let mut edge_counts: BTreeMap<String, u32> = BTreeMap::new();
        node_counts.insert("gateway".into(), active_connections);
        for path in &active_paths {
            let protocol_id = format!("protocol-{}", path.protocol);
            let pool_id = format!("pool-{}", path.pool_id);
            let provider_id = format!("provider-{}", path.provider_id);
            let account_id = format!("account-{}", path.account_id);
            let ids: Vec<&String> = if path.account_id.is_empty() {
                vec![&protocol_id, &pool_id, &provider_id]
            } else {
                vec![&protocol_id, &pool_id, &provider_id, &account_id]
            };
            for id in ids {
                *node_counts.entry(id.clone()).or_default() += path.active_requests;
            }
            *edge_counts
                .entry(format!("gateway-{}", protocol_id))
                .or_default() += path.active_requests;
            *edge_counts
                .entry(format!("{}-pool-{}", protocol_id, path.pool_id))
                .or_default() += path.active_requests;
            *edge_counts
                .entry(format!(
                    "pool-{}-provider-{}",
                    path.pool_id, path.provider_id
                ))
                .or_default() += path.active_requests;
            if !path.account_id.is_empty() {
                *edge_counts
                    .entry(format!(
                        "provider-{}-account-{}",
                        path.provider_id, path.account_id
                    ))
                    .or_default() += path.active_requests;
            }
        }
        TopologyRuntimeDelta {
            sequence,
            emitted_at: Utc::now().to_rfc3339(),
            active_connections,
            active_paths,
            latest_route,
            node_deltas: node_counts
                .into_iter()
                .map(|(id, active_concurrency)| RuntimeNodeDelta {
                    id,
                    active_concurrency,
                })
                .collect(),
            edge_deltas: edge_counts
                .into_iter()
                .map(|(id, active_requests)| RuntimeEdgeDelta {
                    id,
                    active_requests,
                })
                .collect(),
            completed,
        }
    }
}

/// Edge ids a route traverses: gateway->protocol, protocol->pool, pool->provider,
/// and (when an account is resolved) provider->account.
fn route_edge_ids(route: &RuntimeRoutePath) -> Vec<String> {
    let protocol_id = format!("protocol-{}", route.protocol);
    let mut ids = vec![
        format!("gateway-{}", protocol_id),
        format!("{}-pool-{}", protocol_id, route.pool_id),
        format!("pool-{}-provider-{}", route.pool_id, route.provider_id),
    ];
    if !route.account_id.is_empty() {
        ids.push(format!(
            "provider-{}-account-{}",
            route.provider_id, route.account_id
        ));
    }
    ids
}

fn aggregate_active_paths(routes: &HashMap<String, RuntimeRoutePath>) -> Vec<RuntimeActivePath> {
    let mut paths: BTreeMap<(String, String, String, String), RuntimeActivePath> = BTreeMap::new();
    for route in routes.values() {
        let key = (
            route.protocol.clone(),
            route.pool_id.clone(),
            route.provider_id.clone(),
            route.account_id.clone(),
        );
        let path = paths.entry(key).or_insert_with(|| RuntimeActivePath {
            protocol: route.protocol.clone(),
            pool_id: route.pool_id.clone(),
            provider_id: route.provider_id.clone(),
            account_id: route.account_id.clone(),
            active_requests: 0,
            last_active_at: route.updated_at.clone(),
        });
        path.active_requests += 1;
        if route.updated_at > path.last_active_at {
            path.last_active_at.clone_from(&route.updated_at);
        }
    }
    let mut paths: Vec<_> = paths.into_values().collect();
    paths.sort_by(|left, right| {
        right
            .active_requests
            .cmp(&left.active_requests)
            .then_with(|| right.last_active_at.cmp(&left.last_active_at))
    });
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_guard_tracks_active_count() {
        let runtime = GatewayRuntime::default();
        assert_eq!(runtime.active_connections(), 0);
        {
            let _guard = runtime.begin_connection();
            assert_eq!(runtime.active_connections(), 1);
        }
        assert_eq!(runtime.active_connections(), 0);
    }

    #[test]
    fn active_routes_are_aggregated_and_completed() {
        let runtime = GatewayRuntime::default();
        runtime.select_route("req-1", "chat", "pool-1", "provider-1", "account-1", 1);
        runtime.select_route("req-2", "chat", "pool-1", "provider-1", "account-2", 1);
        assert_eq!(runtime.active_paths()[0].active_requests, 2);
        runtime.complete_route("req-1", "success", 42);
        assert_eq!(runtime.active_paths()[0].active_requests, 1);
        let route = runtime.latest_route().unwrap();
        assert_eq!(route.status, "success");
        assert_eq!(route.latency_ms, Some(42));
    }

    #[test]
    fn reset_does_not_underflow_connection_guard() {
        let runtime = GatewayRuntime::default();
        let guard = runtime.begin_connection();
        runtime.reset();
        drop(guard);
        assert_eq!(runtime.active_connections(), 0);
    }
}
