//! Routing strategies, model cache, and account selection logic.
//! Picks an account from a group using the configured routing strategy,
//! respecting health status, concurrency limits, and account priority.

use crate::db::{accounts::Account, providers::Provider};
use crate::AppState;
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

/// Supported routing strategies for account selection.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum RoutingStrategy {
    RoundRobin,
    LeastUsed,
    Priority,
    Random,
    CostOptimized,
}

impl RoutingStrategy {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "round_robin" | "roundrobin" => Self::RoundRobin,
            "least_used" | "leastused" => Self::LeastUsed,
            "priority" => Self::Priority,
            "random" => Self::Random,
            "cost_optimized" | "costoptimized" => Self::CostOptimized,
            _ => Self::RoundRobin,
        }
    }
}

/// Cache for aggregated model lists from all providers/accounts.
pub struct ModelCache {
    inner: RwLock<Option<(Instant, Vec<serde_json::Value>)>>,
    ttl: Duration,
}

impl ModelCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: RwLock::new(None),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Get cached models if still fresh.
    pub async fn get(&self) -> Option<Vec<serde_json::Value>> {
        let guard = self.inner.read().await;
        match &*guard {
            Some((timestamp, models)) if timestamp.elapsed() < self.ttl => Some(models.clone()),
            _ => None,
        }
    }

    /// Set the cached model list.
    pub async fn set(&self, models: Vec<serde_json::Value>) {
        let mut guard = self.inner.write().await;
        *guard = Some((Instant::now(), models));
    }

    /// Invalidate the cache.
    pub async fn invalidate(&self) {
        let mut guard = self.inner.write().await;
        *guard = None;
    }
}

impl Default for ModelCache {
    fn default() -> Self {
        Self::new(30)
    }
}

/// Round-robin counter (per-group) using a global Mutex.
mod rr_counter {
    use std::collections::HashMap;
    use std::sync::Mutex;

    static COUNTERS: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

    pub fn next(group_id: &str, max: usize) -> usize {
        let mut guard = COUNTERS.lock().unwrap();
        let map = guard.get_or_insert_with(HashMap::new);
        let entry = map.entry(group_id.to_string()).or_insert(0);
        let current = *entry;
        *entry = (*entry + 1) % max;
        current
    }
}

/// Sticky account per routing key (group|entry|model).
///
/// Codex OAuth 串行使用：同一模型固定一个本人账号发起，直到该账号额度耗尽或
/// 请求失败被排除出候选，才切换到下一个账号；绝不按请求轮询。
mod sticky_counter {
    use std::collections::HashMap;
    use std::sync::Mutex;

    static STICKY: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

    pub fn get(key: &str) -> Option<String> {
        STICKY
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|map| map.get(key).cloned())
    }

    pub fn set(key: &str, account_id: &str) {
        let mut guard = STICKY.lock().unwrap();
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(key.to_string(), account_id.to_string());
    }
}

/// Account selection logic.
pub struct AccountSelector {
    strategy: RoutingStrategy,
}

impl AccountSelector {
    pub fn new(strategy: RoutingStrategy) -> Self {
        Self { strategy }
    }

    /// Select an account from the provided list based on the strategy.
    /// Only considers accounts with health_status == "healthy", "unchecked", or
    /// unset. Explicitly failed/error/timeout accounts stay out of the pool until
    /// a later health check marks them usable again.
    async fn select_index(&self, routing_key: &str, accounts: &[Account]) -> Option<usize> {
        if accounts.is_empty() {
            return None;
        }

        let candidates: Vec<usize> = accounts
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                matches!(
                    a.health_status.as_deref(),
                    None | Some("healthy") | Some("unchecked")
                )
            })
            .map(|(i, _)| i)
            .collect();

        if candidates.is_empty() {
            return None;
        }

        // Codex OAuth 串行使用：只要候选里存在 Codex 账号，就固定使用当前
        // sticky 账号（同模型），直到它因额度耗尽 / 请求失败 / 健康异常被
        // 排除出候选，才按剩余额度优先切到下一个 Codex 账号。绝不轮询。
        let has_codex = candidates
            .iter()
            .any(|&i| accounts[i].credential_type.as_deref() == Some("codex_oauth"));
        if has_codex {
            if let Some(sticky_id) = sticky_counter::get(routing_key) {
                if let Some(&idx) = candidates
                    .iter()
                    .find(|&&i| accounts[i].id == sticky_id)
                {
                    return Some(idx);
                }
            }
            // 首次使用或当前账号已离开候选 → 选剩余额度最多的 Codex 账号并记住。
            let pick = candidates
                .iter()
                .copied()
                .filter(|&i| accounts[i].credential_type.as_deref() == Some("codex_oauth"))
                .max_by(|&a, &b| {
                    codex_remaining(&accounts[a])
                        .partial_cmp(&codex_remaining(&accounts[b]))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })?;
            sticky_counter::set(routing_key, &accounts[pick].id);
            return Some(pick);
        }

        self.pick_index(routing_key, accounts, candidates).await
    }

    async fn pick_index(
        &self,
        routing_key: &str,
        accounts: &[Account],
        candidates: Vec<usize>,
    ) -> Option<usize> {
        if candidates.is_empty() {
            return None;
        }

        match self.strategy {
            RoutingStrategy::RoundRobin => {
                let idx = rr_counter::next(routing_key, candidates.len());
                Some(candidates[idx])
            }
            RoutingStrategy::Random => {
                let idx = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as usize
                    % candidates.len();
                Some(candidates[idx])
            }
            RoutingStrategy::Priority => {
                // Pick the highest priority (lowest number = higher priority)
                candidates
                    .into_iter()
                    .min_by_key(|&i| accounts[i].priority.unwrap_or(0))
            }
            RoutingStrategy::LeastUsed => {
                // Pick the account with the lowest quota_used / quota_limit ratio
                candidates.into_iter().min_by(|&a, &b| {
                    let ratio_a = accounts[a]
                        .quota_used
                        .zip(accounts[a].quota_limit)
                        .map(|(u, l)| if l > 0.0 { u / l } else { 1.0 })
                        .unwrap_or(0.0);
                    let ratio_b = accounts[b]
                        .quota_used
                        .zip(accounts[b].quota_limit)
                        .map(|(u, l)| if l > 0.0 { u / l } else { 1.0 })
                        .unwrap_or(0.0);
                    ratio_a
                        .partial_cmp(&ratio_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            }
            RoutingStrategy::CostOptimized => {
                // Pick account with lowest health_latency
                candidates.into_iter().min_by(|&a, &b| {
                    let cost_a = accounts[a].health_latency.unwrap_or(1000);
                    let cost_b = accounts[b].health_latency.unwrap_or(1000);
                    cost_a.cmp(&cost_b)
                })
            }
        }
    }
}

/// Parse a JSON array (or comma-separated legacy) model list.
fn parse_model_list(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else { return Vec::new() };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if let Ok(list) = serde_json::from_str::<Vec<String>>(trimmed) {
        return list
            .into_iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
    }
    trimmed
        .split([',', ';'])
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .collect()
}

/// Whether an account (backed by its provider) can serve the requested model.
///
/// Precedence: account-declared models > provider-declared models > wildcard.
/// An account with no declared models is treated as capable of any model, so
/// freshly imported API keys still route before health/model discovery runs.
fn quota_exhausted(account: &Account) -> bool {
    // Legacy quota columns (quota_limit / quota_used).
    if matches!(
        (account.quota_limit, account.quota_used),
        (Some(limit), Some(used)) if limit > 0.0 && used >= limit
    ) {
        return true;
    }
    // Codex OAuth 额度存储在 quota_windows（primary = 5 小时滚动窗口，
    // used_percent 达到 100 即视为本轮额度耗尽，串行策略据此切换账号）。
    if account.credential_type.as_deref() == Some("codex_oauth") {
        if let Some(raw) = account.quota_windows.as_deref() {
            if let Ok(windows) = serde_json::from_str::<Vec<serde_json::Value>>(raw) {
                if let Some(primary) = windows.iter().find(|window| {
                    window.get("key").and_then(|key| key.as_str()) == Some("primary")
                }) {
                    let used = primary
                        .get("used_percent")
                        .and_then(|value| value.as_f64())
                        .unwrap_or(0.0);
                    if used >= 100.0 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Codex 账号剩余额度比例（primary 5 小时滚动窗口）。无额度数据视为满额 1.0，
/// 供串行选择在账号间切换时优先挑剩余额度最多的账号。
fn codex_remaining(account: &Account) -> f64 {
    if let Some(raw) = account.quota_windows.as_deref() {
        if let Ok(windows) = serde_json::from_str::<Vec<serde_json::Value>>(raw) {
            if let Some(primary) = windows.iter().find(|window| {
                window.get("key").and_then(|key| key.as_str()) == Some("primary")
            }) {
                let used = primary
                    .get("used_percent")
                    .and_then(|value| value.as_f64())
                    .unwrap_or(0.0);
                return (100.0 - used).max(0.0) / 100.0;
            }
        }
    }
    1.0
}

fn account_supports_model(account: &Account, provider: &Provider, model: &str) -> bool {
    let account_models = parse_model_list(account.models.as_deref());
    if !account_models.is_empty() {
        return account_models.iter().any(|m| m == model);
    }
    let provider_models = parse_model_list(provider.models.as_deref());
    if !provider_models.is_empty() {
        return provider_models.iter().any(|m| m == model);
    }
    true
}

/// Parse a JSON array (or comma/semicolon-separated legacy) protocol list.
pub fn parse_protocols(raw: Option<&str>) -> Vec<String> {
    fn canonical(value: &str) -> String {
        match value.trim().to_lowercase().as_str() {
            "openai" | "chat_completions" | "openai_chat" => "chat".into(),
            "codex" | "openai_responses" => "responses".into(),
            "messages" | "anthropic_messages" | "claude" => "anthropic".into(),
            "google" | "google_gemini" => "gemini".into(),
            value => value.to_string(),
        }
    }

    let Some(raw) = raw else { return Vec::new() };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let values = if let Ok(list) = serde_json::from_str::<Vec<String>>(trimmed) {
        list
    } else {
        trimmed.split([',', ';']).map(ToString::to_string).collect()
    };
    let mut protocols = Vec::new();
    for value in values {
        let value = canonical(&value);
        if !value.is_empty() && !protocols.contains(&value) {
            protocols.push(value);
        }
    }
    protocols
}

/// Whether the account's protocol set can serve the gateway entry protocol.
///
/// Entry semantics:
/// * `responses` — unified entry. A native `responses` speaker is always OK;
///   `chat`/`anthropic` speakers are OK only when route takeover is on. Gemini
///   remains native-only and is never selected for Responses conversion.
/// * `chat` / `anthropic` / `gemini` — legacy direct-passthrough entries, matched
///   only by their exact native protocol.
fn protocol_matches(
    protocols: &[String],
    entry_protocol: &str,
    route_takeover: bool,
    pool_protocol: Option<&str>,
) -> bool {
    // If pool is unified, apply the actual protocol conversion matrix instead
    // of allowing any protocol unconditionally. This prevents mismatched
    // routing (e.g., Codex OAuth accounts on chat entry without conversion).
    if let Some(pool_proto) = pool_protocol {
        if pool_proto.to_lowercase() == "unified" {
            return unified_protocol_matches(protocols, entry_protocol, route_takeover);
        }
    }

    let entry = entry_protocol.to_lowercase();
    protocols.iter().any(|p| {
        let pr = p.to_lowercase();
        match entry.as_str() {
            "responses" => {
                pr == "responses" || (route_takeover && (pr == "chat" || pr == "anthropic"))
            }
            "chat" => pr == "chat",
            "anthropic" => pr == "anthropic",
            "gemini" => pr == "gemini",
            _ => false,
        }
    })
}

/// Check if a provider's protocols are compatible with the given entry protocol
/// in a unified pool, based on the actual conversion matrix supported by PoolGate.
///
/// Conversion matrix (from responses.rs, anthropic.rs, gemini.rs, openai.rs):
/// - responses entry → supports: responses (native), chat (via conversion),
///   anthropic (via conversion), gemini (via conversion)
/// - chat entry → supports: chat (native), responses (via conversion),
///   anthropic (via conversion), gemini (via conversion)
/// - anthropic entry → supports: anthropic (native), responses (via conversion),
///   chat (via conversion), gemini (via conversion)
/// - gemini entry → supports: gemini (native), responses (via conversion),
///   chat (via conversion), anthropic (via conversion)
fn unified_protocol_matches(
    protocols: &[String],
    entry_protocol: &str,
    _route_takeover: bool,
) -> bool {
    // In unified mode, any provider protocol can serve any entry because
    // PoolGate performs the conversion. However, we still require the provider
    // to have at least one protocol declared.
    if protocols.is_empty() {
        return false;
    }

    // All entry protocols are supported in unified mode as long as the provider
    // has at least one protocol (responses, chat, anthropic, gemini).
    // The actual conversion is handled by the protocol converters.
    let entry = entry_protocol.to_lowercase();
    match entry.as_str() {
        "responses" | "chat" | "anthropic" | "gemini" => {
            // Any provider with a declared protocol can serve any entry in unified mode.
            true
        }
        _ => false,
    }
}

/// Select an account for a request using the given strategy and application state.
///
/// Routing model:
/// * When `group_id` resolves to an existing group with members, candidates are
///   drawn from that group. Otherwise (missing group, the synthetic `default`
///   group, or an empty group) the entire account pool is used — this makes the
///   "import credentials, then just use it" flow work without manual grouping.
/// * Candidates are filtered by: not disabled, directly routable, matching the
///   entry protocol (when provided), and capable of the requested model (when
///   provided). Health filtering happens inside the selector.
///
/// Returns the selected `Account` and its `Provider`, or a human-readable error
/// describing why no account could serve the request.
pub async fn select_account(
    group_id: &str,
    strategy: &RoutingStrategy,
    model: Option<&str>,
    entry_protocol: Option<&str>,
    state: &AppState,
) -> Result<(Account, Provider), String> {
    select_account_excluding(
        group_id,
        strategy,
        model,
        entry_protocol,
        &std::collections::HashSet::new(),
        state,
    )
    .await
}

/// Cooldown (seconds) before an unhealthy account is eligible for auto-recovery.
/// Prevents hammering a genuinely down provider on every request.
const HEALTH_RECOVERY_COOLDOWN_SECS: i64 = 60;

/// Attempt to auto-recover unhealthy accounts by re-running health checks on
/// those whose last check is older than [`HEALTH_RECOVERY_COOLDOWN_SECS`].
///
/// Modifies `candidates` in place: accounts that pass the re-check have their
/// `health_status` updated to `"healthy"` and will be picked up by the next
/// `select_index` call.  Returns `true` if at least one account recovered.
async fn attempt_health_recovery(
    candidates: &mut [Account],
    provider_map: &std::collections::HashMap<String, Provider>,
    state: &AppState,
) -> bool {
    use chrono::{NaiveDateTime, Utc};

    let checker = crate::services::health_check::HealthChecker::new(10);
    let now = Utc::now().naive_utc();
    let mut recovered = false;

    for account in candidates.iter_mut() {
        // Only attempt recovery on explicitly unhealthy statuses.
        if !matches!(
            account.health_status.as_deref(),
            Some("failed") | Some("timeout") | Some("error")
        ) {
            continue;
        }

        // Respect cooldown — skip if last check is recent enough.
        if let Some(ref last_check) = account.health_check_at {
            if let Ok(last) = NaiveDateTime::parse_from_str(last_check, "%Y-%m-%d %H:%M:%S") {
                if (now - last).num_seconds() < HEALTH_RECOVERY_COOLDOWN_SECS {
                    continue;
                }
            }
        }

        // Need the provider to run the check.
        let provider = match account
            .provider_id
            .as_deref()
            .and_then(|pid| provider_map.get(pid))
        {
            Some(p) => p,
            None => continue,
        };

        tracing::info!(
            "Auto-recovery: re-checking account '{}' (was {})",
            account.id,
            account.health_status.as_deref().unwrap_or("unset"),
        );

        let result = checker.check_account(account, provider).await;
        match &result {
            crate::services::health_check::HealthResult::Passed { latency_ms } => {
                // Persist recovery.
                state
                    .db
                    .accounts
                    .update_health(
                        &state.db.conn,
                        &account.id,
                        "healthy",
                        200,
                        "OK",
                        *latency_ms as i64,
                    )
                    .ok();
                // Also recover terminal status fields.
                state
                    .db
                    .accounts
                    .recover_status(&state.db.conn, &account.id, account.status.as_deref())
                    .ok();
                // Update in-memory so select_index can see it.
                account.health_status = Some("healthy".into());
                account.health_code = Some(200);
                account.health_msg = Some("OK".into());
                account.health_latency = Some(*latency_ms as i64);
                recovered = true;
                tracing::info!(
                    "Auto-recovery: account '{}' recovered ({}ms)",
                    account.id,
                    latency_ms,
                );
            }
            other => {
                // Still unhealthy — update timestamp so we don't re-check
                // again within the cooldown window.
                let (status, code, msg) = match other {
                    crate::services::health_check::HealthResult::Failed {
                        code,
                        body,
                    } => ("failed", *code, body.clone()),
                    crate::services::health_check::HealthResult::Timeout => {
                        ("timeout", 0u16, "Request timed out".into())
                    }
                    crate::services::health_check::HealthResult::Error(m) => {
                        ("error", 0u16, m.clone())
                    }
                    _ => unreachable!(),
                };
                state
                    .db
                    .accounts
                    .update_health(&state.db.conn, &account.id, status, code, &msg, 0)
                    .ok();
                tracing::debug!(
                    "Auto-recovery: account '{}' still {} (code={})",
                    account.id,
                    status,
                    code,
                );
            }
        }
    }

    recovered
}

/// Select an account while excluding accounts already attempted for the same
/// gateway request. This is the foundation for pool failover on 401/429/5xx.
pub async fn select_account_excluding(
    group_id: &str,
    strategy: &RoutingStrategy,
    model: Option<&str>,
    entry_protocol: Option<&str>,
    excluded_account_ids: &std::collections::HashSet<String>,
    state: &AppState,
) -> Result<(Account, Provider), String> {
    let db = &state.db;

    // Provider lookup map (single lock, released immediately).
    let providers = db.providers.list_all(&db.conn)?;
    let provider_map: std::collections::HashMap<String, Provider> =
        providers.into_iter().map(|p| (p.id.clone(), p)).collect();

    // Only the synthetic `default` pool may use all accounts. Named pools are
    // strict security boundaries: they must be enabled and explicitly contain
    // model resources or legacy account members.
    let group = if group_id == "default" {
        None
    } else {
        Some(
            db.groups
                .get_by_id(&db.conn, group_id)?
                .ok_or_else(|| "POOL_NOT_FOUND: 路由池不存在".to_string())?,
        )
    };
    if matches!(group.as_ref().and_then(|value| value.enabled), Some(false)) {
        return Err("POOL_DISABLED: 路由池已停用".into());
    }
    let model_resources = if let Some(group) = &group {
        db.groups.get_model_resources(&db.conn, &group.id)?
    } else {
        Vec::new()
    };
    if let (Some(requested), Some(_)) = (model, group.as_ref()) {
        if !model_resources.is_empty()
            && !model_resources
                .iter()
                .any(|resource| resource.model == requested)
        {
            return Err(format!(
                "POOL_MODEL_FORBIDDEN: 路由池未允许模型 '{}'",
                requested
            ));
        }
    }
    let legacy_ids = if let Some(group) = &group {
        db.groups.get_account_ids(&db.conn, &group.id)?
    } else {
        Vec::new()
    };
    // A non-empty exact mapping is authoritative for the requested resource.
    // Empty mapping means legacy resources retain their all-supporting-account behavior.
    let exact_model_account_ids: std::collections::HashMap<
        String,
        std::collections::HashSet<String>,
    > = if let (Some(group), Some(requested)) = (group.as_ref(), model) {
        let mut selected = std::collections::HashMap::new();
        for resource in model_resources
            .iter()
            .filter(|item| item.model == requested)
        {
            let ids = db.groups.get_model_account_ids(
                &db.conn,
                &group.id,
                &resource.provider_id,
                &resource.model,
            )?;
            if !ids.is_empty() {
                selected.insert(resource.provider_id.clone(), ids.into_iter().collect());
            }
        }
        selected
    } else {
        std::collections::HashMap::new()
    };
    if group.is_some() && model_resources.is_empty() && legacy_ids.is_empty() {
        return Err("POOL_EMPTY: 路由池尚未添加模型资源".into());
    }

    let all_accounts: Vec<Account> = if group.is_none() || !model_resources.is_empty() {
        db.accounts.list_all(&db.conn)?
    } else {
        let mut out = Vec::new();
        for id in &legacy_ids {
            if let Some(acct) = db.accounts.get_by_id(&db.conn, id)? {
                out.push(acct);
            }
        }
        out
    };

    if all_accounts.is_empty() {
        return Err("账号池为空，请先导入模型资源".to_string());
    }

    // Filter candidates by routability, provider protocol, and model capability.
    let mut disabled = 0usize;
    let mut protocol_mismatch = 0usize;
    let mut model_mismatch = 0usize;
    let mut candidates: Vec<Account> = Vec::new();

    for account in all_accounts {
        if let Some(provider_id) = account.provider_id.as_ref() {
            if let Some(selected_ids) = exact_model_account_ids.get(provider_id) {
                if !selected_ids.contains(&account.id) {
                    model_mismatch += 1;
                    continue;
                }
            }
        }
        if excluded_account_ids.contains(&account.id) {
            continue;
        }
        if !crate::services::credentials::is_available_for_routing(&account) {
            disabled += 1;
            continue;
        }
        if quota_exhausted(&account) {
            disabled += 1;
            continue;
        }
        let provider = match account
            .provider_id
            .as_deref()
            .and_then(|pid| provider_map.get(pid))
        {
            Some(p) => p,
            None => {
                disabled += 1;
                continue;
            }
        };
        let is_codex_oauth = crate::services::codex_adapter::is_codex_oauth(&account, provider);
        // ChatGPT subscription credentials are deliberately excluded from the
        // catch-all default pool and may serve only the native Responses entry.
        // This prevents Unified pools from accidentally routing Chat/Messages
        // traffic to the Codex backend and prevents implicit account rotation.
        if is_codex_oauth && (group.is_none() || entry_protocol != Some("responses")) {
            protocol_mismatch += 1;
            continue;
        }
        if !model_resources.is_empty() {
            let requested_model = model.unwrap_or_default();
            if !model_resources.iter().any(|resource| {
                resource.provider_id == provider.id
                    && (requested_model.is_empty() || resource.model == requested_model)
            }) {
                model_mismatch += 1;
                continue;
            }
        }
        if let Some(entry) = entry_protocol {
            let acct_protocols = parse_protocols(account.protocols.as_deref());
            let protocols = if acct_protocols.is_empty() {
                parse_protocols(provider.protocols.as_deref())
            } else {
                acct_protocols
            };
            let takeover = account
                .route_takeover
                .unwrap_or_else(|| provider.route_takeover.unwrap_or(1))
                != 0;
            if !protocol_matches(
                &protocols,
                entry,
                takeover,
                group.as_ref().map(|g| g.protocol.as_str()),
            ) {
                protocol_mismatch += 1;
                continue;
            }
        }
        if let Some(model) = model {
            if !account_supports_model(&account, provider, model) {
                model_mismatch += 1;
                continue;
            }
        }
        candidates.push(account);
    }

    // Codex OAuth 账号不再强制单账号：多个本人账号共存时由 AccountSelector 的
    // 串行（sticky）策略保证同一模型固定一个账号发起，额度耗尽或请求失败后才
    // 切换到下一个，绝不轮询（详见 select_index 的 codex 分支）。

    if candidates.is_empty() {
        let mut reasons = Vec::new();
        if disabled > 0 {
            reasons.push(format!("{} 个已停用/不可直接路由", disabled));
        }
        if protocol_mismatch > 0 {
            reasons.push(format!(
                "{} 个协议不匹配（入口 {}）",
                protocol_mismatch,
                entry_protocol.unwrap_or("-")
            ));
        }
        if model_mismatch > 0 {
            reasons.push(format!(
                "{} 个不支持模型 '{}'",
                model_mismatch,
                model.unwrap_or("-")
            ));
        }
        let detail = if reasons.is_empty() {
            "没有可用账号".to_string()
        } else {
            reasons.join("，")
        };
        return Err(format!("没有可路由的账号：{}", detail));
    }

    // Select an account based on the routing strategy.
    let selector = AccountSelector::new(strategy.clone());
    let routing_key = format!(
        "{}|{}|{}",
        group_id,
        entry_protocol.unwrap_or("*"),
        model.unwrap_or("*")
    );
    let idx = selector
        .select_index(&routing_key, &candidates)
        .await;
    let idx = match idx {
        Some(index) => index,
        None => {
            // All candidates rejected by health gate — attempt auto-recovery:
            // re-check accounts whose last health check is older than the
            // cooldown period.  If any recover, retry selection.
            let recovered = attempt_health_recovery(
                &mut candidates,
                &provider_map,
                state,
            )
            .await;
            if recovered {
                selector
                    .select_index(&routing_key, &candidates)
                    .await
                    .unwrap_or(usize::MAX) // sentinel, handled below
            } else {
                usize::MAX
            }
        }
    };
    if idx == usize::MAX {
        // Either no recovery attempted or recovery didn't help — report
        // the actual health distribution.
        let mut failed = 0usize;
        let mut error = 0usize;
        let mut other = 0usize;
        for candidate in &candidates {
            match candidate.health_status.as_deref() {
                Some("failed") => failed += 1,
                Some("error") => error += 1,
                Some(_) => other += 1,
                None => {}
            }
        }
        return Err(format!(
            "候选账号当前均不可路由：健康异常 {} 个（failed {} / error {} / 其他 {}）",
            failed + error + other,
            failed,
            error,
            other
        ));
    }
    let account = candidates[idx].clone();
    if let Err(error) = db.accounts.mark_used(&db.conn, &account.id) {
        tracing::warn!(
            "Failed to mark routed account '{}' as used: {}",
            account.id,
            error
        );
    }

    let provider = account
        .provider_id
        .as_deref()
        .and_then(|pid| provider_map.get(pid))
        .cloned()
        .ok_or_else(|| "选中账号缺少上游连接器".to_string())?;

    Ok((account, provider))
}

/// Generate aggregated model list from all healthy accounts.
pub async fn get_aggregated_models(state: &AppState, cache: &ModelCache) -> Vec<serde_json::Value> {
    // Try cache first
    if let Some(cached) = cache.get().await {
        return cached;
    }

    let db = &state.db;

    let all_accounts = match db.accounts.list_all(&db.conn) {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };
    let all_providers = match db.providers.list_all(&db.conn) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let mut models: Vec<serde_json::Value> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let provider_map: std::collections::HashMap<String, &Provider> =
        all_providers.iter().map(|p| (p.id.clone(), p)).collect();

    // Only surface models that a real, routable, enabled account can serve.
    // This keeps GET /v1/models honest: what appears here can actually be used.
    for account in &all_accounts {
        if account.status.as_deref() == Some("disabled")
            || !crate::services::credentials::is_directly_routable(account)
        {
            continue;
        }
        let Some(provider) = account
            .provider_id
            .as_deref()
            .and_then(|pid| provider_map.get(pid))
        else {
            continue;
        };

        // Prefer account-declared models; fall back to provider-declared models.
        let mut account_models = parse_model_list(account.models.as_deref());
        if account_models.is_empty() {
            account_models = parse_model_list(provider.models.as_deref());
        }

        for model_name in account_models {
            let resource_key = format!("{}::{}", provider.id, model_name);
            if seen.insert(resource_key) {
                models.push(serde_json::json!({
                    "id": model_name,
                    "object": "model",
                    "provider": provider.name,
                    "provider_id": provider.id,
                    "protocol": parse_protocols(provider.protocols.as_deref()).join(" · "),
                    "owned_by": provider.name,
                }));
            }
        }
    }

    // Cache the result
    cache.set(models.clone()).await;
    models
}

/// Filter the honest global model inventory by a named pool and optional
/// client-key model scope. Provider identity is part of the pool resource key,
/// so same-named models from different upstreams cannot escape the pool.
pub async fn get_models_for_scope(
    state: &AppState,
    cache: &ModelCache,
    group_id: &str,
    allowed_models: Option<&str>,
) -> Vec<serde_json::Value> {
    let models = get_aggregated_models(state, cache).await;
    let allowed_by_key = allowed_models
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .filter(|items| !items.is_empty());

    let pool_resources = if group_id == "default" {
        None
    } else {
        match state
            .db
            .groups
            .get_model_resources(&state.db.conn, group_id)
        {
            Ok(resources) => Some(resources),
            Err(_) => return Vec::new(),
        }
    };

    models
        .into_iter()
        .filter(|item| {
            let model = item
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let provider_id = item
                .get("provider_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let key_allows = allowed_by_key
                .as_ref()
                .map(|items| items.iter().any(|allowed| allowed == model))
                .unwrap_or(true);
            let pool_allows = pool_resources
                .as_ref()
                .map(|resources| {
                    resources.iter().any(|resource| {
                        resource.provider_id == provider_id && resource.model == model
                    })
                })
                .unwrap_or(true);
            key_allows && pool_allows
        })
        .collect()
}

/// Build a stats response from the database.
pub async fn get_stats_response(state: &AppState) -> serde_json::Value {
    let db = &state.db;

    let stats = db.logs.get_stats(&db.conn).ok();
    let analytics = db
        .logs
        .get_analytics(&db.conn, None, None)
        .ok();

    serde_json::json!({
        "stats": stats,
        "analytics": analytics,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::accounts::Account;
    use crate::db::providers::Provider;

    fn provider(protocol: &str, models: Option<&str>) -> Provider {
        Provider {
            id: "prov_1".into(),
            name: "Test".into(),
            provider_type: protocol.into(),
            base_url: "https://api.example.com".into(),
            base_urls: None,
            protocol: protocol.into(),
            protocols: None,
            route_takeover: Some(1),
            api_keys: None,
            models: models.map(|m| m.to_string()),
            proxy_url: None,
            custom_headers: None,
            timeout_ms: Some(30000),
            priority: Some(0),
            enabled: Some(true),
            created_at: None,
            auth_mode: None,
            oauth_config: None,
        }
    }

    fn account(models: Option<&str>) -> Account {
        Account {
            id: "acct_1".into(),
            provider_id: Some("prov_1".into()),
            name: Some("acct".into()),
            api_key: "sk-test".into(),
            models: models.map(|m| m.to_string()),
            quota_limit: None,
            quota_used: None,
            status: Some("active".into()),
            health_status: Some("healthy".into()),
            health_code: None,
            health_msg: None,
            health_latency: None,
            health_check_at: None,
            priority: Some(0),
            tags: None,
            last_used_at: None,
            created_at: None,
            credential_type: Some("api_key".into()),
            credential_data: None,
            source_format: None,
            external_account_id: None,
            email: None,
            expires_at: None,
            metadata: None,
            credential_fingerprint: None,
            protocols: None,
            route_takeover: Some(1),
            plan_type: None,
            quota_windows: None,
            quota_refreshed_at: None,
            quota_error: None,
            token_refreshed_at: None,
            secret_ref: None,
        }
    }

    #[tokio::test]
    async fn failed_health_accounts_are_not_selected() {
        let selector = AccountSelector::new(RoutingStrategy::RoundRobin);
        let mut failed = account(None);
        failed.health_status = Some("failed".into());
        let mut error = account(None);
        error.id = "acct_2".into();
        error.health_status = Some("error".into());
        assert!(selector
            .select_index("default|chat|model", &[failed, error])
            .await
            .is_none());
    }

    #[test]
    fn parse_model_list_handles_json_and_csv() {
        assert_eq!(
            parse_model_list(Some(r#"["gpt-4o","gpt-4o-mini"]"#)),
            vec!["gpt-4o", "gpt-4o-mini"]
        );
        assert_eq!(
            parse_model_list(Some("gpt-4o, gpt-4o-mini")),
            vec!["gpt-4o", "gpt-4o-mini"]
        );
        assert!(parse_model_list(Some("  ")).is_empty());
        assert!(parse_model_list(None).is_empty());
    }

    #[test]
    fn account_without_models_serves_any_model() {
        let acct = account(None);
        let prov = provider("openai", None);
        assert!(account_supports_model(&acct, &prov, "any-model"));
    }

    #[test]
    fn account_models_take_precedence_over_provider() {
        let acct = account(Some(r#"["gpt-4o"]"#));
        let prov = provider("openai", Some(r#"["claude-3"]"#));
        assert!(account_supports_model(&acct, &prov, "gpt-4o"));
        assert!(!account_supports_model(&acct, &prov, "claude-3"));
    }

    #[test]
    fn quota_exhaustion_is_detected() {
        let mut acct = account(None);
        acct.quota_limit = Some(100.0);
        acct.quota_used = Some(100.0);
        assert!(quota_exhausted(&acct));
        acct.quota_used = Some(99.9);
        assert!(!quota_exhausted(&acct));
        acct.quota_limit = None;
        assert!(!quota_exhausted(&acct));
    }

    #[test]
    fn codex_quota_exhausted_from_primary_window() {
        let mut acct = account(None);
        acct.credential_type = Some("codex_oauth".into());
        acct.quota_windows = Some(r#"[{"key":"primary","used_percent":100.0}]"#.into());
        assert!(quota_exhausted(&acct));
        acct.quota_windows = Some(r#"[{"key":"primary","used_percent":99.9}]"#.into());
        assert!(!quota_exhausted(&acct));
    }

    #[test]
    fn codex_serial_uses_same_account_until_exhausted() {
        // 即便池配置为轮询，Codex 候选也必须串行：同一模型固定一个账号。
        let selector = AccountSelector::new(RoutingStrategy::RoundRobin);
        let mut a1 = account(None);
        a1.id = "acct-codex-1".into();
        a1.credential_type = Some("codex_oauth".into());
        a1.quota_windows = Some(r#"[{"key":"primary","used_percent":10.0}]"#.into());
        let mut a2 = account(None);
        a2.id = "acct-codex-2".into();
        a2.credential_type = Some("codex_oauth".into());
        a2.quota_windows = Some(r#"[{"key":"primary","used_percent":90.0}]"#.into());
        let key = "serial-test|responses|gpt-5.5";

        let accounts = vec![a1.clone(), a2.clone()];
        // 首次：剩余额度最多的 acct-codex-1（used 10%）。
        let i = futures::executor::block_on(selector.select_index(key, &accounts)).unwrap();
        assert_eq!(accounts[i].id, "acct-codex-1");
        // 连续请求保持 sticky，绝不轮询。
        for _ in 0..5 {
            let j = futures::executor::block_on(selector.select_index(key, &accounts)).unwrap();
            assert_eq!(accounts[j].id, "acct-codex-1");
        }
        // acct-codex-1 额度耗尽离开候选（quota_exhausted 已过滤）→ 切到
        // acct-codex-2 并 sticky。
        let remaining = vec![a2.clone()];
        let k = futures::executor::block_on(selector.select_index(key, &remaining)).unwrap();
        assert_eq!(remaining[k].id, "acct-codex-2");
        let m = futures::executor::block_on(selector.select_index(key, &remaining)).unwrap();
        assert_eq!(remaining[m].id, "acct-codex-2");
    }

    #[test]
    fn non_codex_candidates_keep_configured_strategy() {
        let selector = AccountSelector::new(RoutingStrategy::RoundRobin);
        let mut a1 = account(None);
        a1.id = "acct-chat-1".into();
        let mut a2 = account(None);
        a2.id = "acct-chat-2".into();
        let key = "noncodex-test|chat|gpt-4.1";
        let accounts = vec![a1, a2];
        let i = futures::executor::block_on(selector.select_index(key, &accounts)).unwrap();
        let j = futures::executor::block_on(selector.select_index(key, &accounts)).unwrap();
        // 非 Codex 场景仍按池配置轮询。
        assert_ne!(accounts[i].id, accounts[j].id);
    }

    #[test]
    fn provider_models_used_when_account_has_none() {
        let acct = account(None);
        let prov = provider("openai", Some(r#"["gpt-4o"]"#));
        assert!(account_supports_model(&acct, &prov, "gpt-4o"));
        assert!(!account_supports_model(&acct, &prov, "gpt-3.5"));
    }

    #[test]
    fn parses_legacy_protocol_values_as_canonical() {
        assert_eq!(
            parse_protocols(Some(r#"["openai","messages","google"]"#)),
            vec!["chat", "anthropic", "gemini"]
        );
        assert_eq!(
            parse_protocols(Some("codex;openai")),
            vec!["responses", "chat"]
        );
    }

    #[test]
    fn protocol_matches_multi() {
        assert!(protocol_matches(&["chat".into()], "chat", true, None));
        assert!(protocol_matches(
            &["anthropic".into()],
            "anthropic",
            true,
            None
        ));
        assert!(!protocol_matches(&["anthropic".into()], "chat", true, None));
        // responses-native always routes the unified entry
        assert!(protocol_matches(
            &["responses".into()],
            "responses",
            true,
            None
        ));
        // chat/anthropic only route the unified entry with route takeover on
        assert!(protocol_matches(&["chat".into()], "responses", true, None));
        assert!(!protocol_matches(
            &["chat".into()],
            "responses",
            false,
            None
        ));
        assert!(protocol_matches(
            &["anthropic".into()],
            "responses",
            true,
            None
        ));
        // gemini matches only its own entry
        assert!(protocol_matches(&["gemini".into()], "gemini", true, None));
        assert!(!protocol_matches(
            &["gemini".into()],
            "responses",
            true,
            None
        ));
    }

    #[test]
    fn unified_pool_matches_any_entry() {
        // Unified pool with route takeover should match any entry protocol
        assert!(protocol_matches(
            &["chat".into()],
            "chat",
            true,
            Some("unified")
        ));
        assert!(protocol_matches(
            &["anthropic".into()],
            "anthropic",
            true,
            Some("unified")
        ));
        assert!(protocol_matches(
            &["gemini".into()],
            "gemini",
            true,
            Some("unified")
        ));
        assert!(protocol_matches(
            &["responses".into()],
            "responses",
            true,
            Some("unified")
        ));
        // Even with route takeover off, unified pool should match
        assert!(protocol_matches(
            &["chat".into()],
            "responses",
            false,
            Some("unified")
        ));
        assert!(protocol_matches(
            &["anthropic".into()],
            "responses",
            false,
            Some("unified")
        ));
    }
}
