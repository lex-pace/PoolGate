use crate::db::groups::{AgentGroup, GroupModelResource};
use crate::AppState;
use std::sync::Arc;
use tauri::State;

pub(crate) fn parse_models(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_else(|_| {
        raw.split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()
    })
}

#[derive(serde::Serialize)]
pub struct GroupDashboard {
    pub resource_count: usize,
    pub healthy_resource_count: usize,
    pub model_count: usize,
    pub provider_count: usize,
    pub traffic: crate::db::logs::GroupTrafficStats,
    pub quota_by_provider: Vec<ProviderQuotaSummary>,
}

#[derive(serde::Serialize)]
pub struct RouteTopology {
    pub version: u8,
    pub topology_revision: u64,
    pub gateway: RouteTopologyGateway,
    pub protocols: Vec<RouteTopologyProtocol>,
    pub pools: Vec<RouteTopologyPool>,
    pub providers: Vec<RouteTopologyProvider>,
    /// Fifth layer: concrete upstream accounts under each provider. The canvas
    /// renders one node per account (or a collapsed summary when a provider
    /// has more than the collapse threshold).
    pub accounts: Vec<RouteTopologyAccount>,
    pub edges: Vec<RouteTopologyEdge>,
    pub active_route: Option<crate::proxy::runtime::RuntimeRoutePath>,
    pub runtime: crate::proxy::runtime::TopologyRuntimeDelta,
    pub updated_at: String,
}

#[derive(serde::Serialize)]
pub struct RouteTopologyGateway {
    pub id: String,
    pub name: String,
    pub address: String,
    pub running: bool,
    pub active_connections: u32,
}

#[derive(serde::Serialize)]
pub struct RouteTopologyProtocol {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub enabled: bool,
    pub pool_ids: Vec<String>,
    pub request_count: i64,
    pub traffic: crate::db::logs::GroupTrafficStats,
}

#[derive(serde::Serialize)]
pub struct RouteTopologyEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub status: String,
    pub active: bool,
    pub active_requests: u32,
}

#[derive(serde::Serialize)]
pub struct RouteTopologyPool {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub strategy: Option<String>,
    pub enabled: bool,
    pub provider_ids: Vec<String>,
    pub resource_count: usize,
    pub healthy_resource_count: usize,
    pub model_count: usize,
    pub traffic: crate::db::logs::GroupTrafficStats,
}

#[derive(serde::Serialize)]
pub struct RouteTopologyProvider {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub enabled: bool,
    pub account_count: usize,
    pub healthy_account_count: usize,
    pub traffic: crate::db::logs::ProviderTrafficStats,
    /// Upstream host (scheme://host) resolved from the base URL. Lets the
    /// canvas disambiguate generic template names (e.g. "自定义") so a node
    /// stays identifiable without leaking the full URL.
    pub host: Option<String>,
}

#[derive(serde::Serialize)]
pub struct RouteTopologyAccount {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub email_masked: Option<String>,
    pub status: String,
    pub health_status: String,
    pub routable: bool,
    pub plan_type: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ProviderTopologyDetail {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub protocol: String,
    pub protocols: Vec<String>,
    pub base_url_masked: String,
    pub enabled: bool,
    pub models: Vec<String>,
    pub traffic: crate::db::logs::ProviderTrafficStats,
    pub accounts: Vec<ProviderTopologyAccount>,
}

#[derive(serde::Serialize)]
pub struct ProviderTopologyAccount {
    pub id: String,
    pub name: String,
    pub email_masked: Option<String>,
    pub credential_type: String,
    pub source_format: Option<String>,
    pub routable: bool,
    pub status: String,
    pub health_status: String,
    pub health_latency_ms: Option<i64>,
    pub last_used_at: Option<String>,
    pub plan_type: Option<String>,
    pub models: Vec<String>,
    pub quota_remaining_percent: Option<f64>,
    pub concurrency_limit: usize,
    pub concurrency_active: usize,
    pub concurrency_available: usize,
    pub queued_requests: usize,
    pub expires_at: Option<String>,
    pub last_error: Option<String>,
    pub traffic: crate::db::logs::AccountTrafficStats,
}

#[derive(serde::Serialize, Clone)]
pub struct AvailableGroupModelAccount {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub status: String,
    pub health_status: String,
    pub routable: bool,
    pub selected: bool,
}

#[derive(serde::Serialize)]
pub struct AvailableGroupModelResource {
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub account_count: usize,
    pub healthy_count: usize,
    pub already_added: bool,
    pub accounts: Vec<AvailableGroupModelAccount>,
}

fn protocol_display_name(protocol: &str) -> String {
    match canonical_protocol(protocol).as_str() {
        "chat" => "OpenAI Chat".into(),
        "responses" => "OpenAI Responses".into(),
        "anthropic" => "Anthropic Messages".into(),
        "gemini" => "Google Gemini".into(),
        "both" => "Multi Protocol".into(),
        value => value.to_string(),
    }
}

fn mask_email(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    let (local, domain) = value.split_once('@')?;
    let visible: String = local.chars().take(2).collect();
    Some(format!("{}***@{}", visible, domain))
}

fn mask_base_url(value: &str) -> String {
    url::Url::parse(value)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| format!("{}://{}", url.scheme(), host))
        })
        .unwrap_or_else(|| "已配置（已脱敏）".into())
}

/// scheme://host resolved from a provider's base URL. Used on topology nodes
/// so a generic template name (e.g. "自定义") stays identifiable.
fn provider_host(base_url: &str) -> Option<String> {
    url::Url::parse(base_url)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| format!("{}://{}", url.scheme(), host))
        })
}

/// Display name for the fifth-layer account node: the account name, falling
/// back to a masked email, then a generic label.
fn account_display_name(account: &crate::db::accounts::Account) -> String {
    account
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| account.email.as_deref().and_then(|email| mask_email(Some(email))))
        .unwrap_or_else(|| "未命名账号".into())
}

/// Health mapping for the provider->account edge (fifth layer).
fn account_topology_status(account: &RouteTopologyAccount) -> &'static str {
    if account.status == "disabled" {
        "disabled"
    } else if !account.routable {
        "warning"
    } else if matches!(account.health_status.as_str(), "error" | "unhealthy") {
        "fault"
    } else {
        "healthy"
    }
}

fn topology_revision(
    protocols: &[RouteTopologyProtocol],
    pools: &[RouteTopologyPool],
    providers: &[RouteTopologyProvider],
    accounts: &[RouteTopologyAccount],
    edges: &[RouteTopologyEdge],
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for protocol in protocols {
        protocol.id.hash(&mut hasher);
        protocol.enabled.hash(&mut hasher);
        protocol.pool_ids.hash(&mut hasher);
    }
    for pool in pools {
        pool.id.hash(&mut hasher);
        pool.enabled.hash(&mut hasher);
        pool.provider_ids.hash(&mut hasher);
    }
    for provider in providers {
        provider.id.hash(&mut hasher);
        provider.enabled.hash(&mut hasher);
    }
    for account in accounts {
        account.id.hash(&mut hasher);
        account.provider_id.hash(&mut hasher);
        account.routable.hash(&mut hasher);
    }
    for edge in edges {
        edge.id.hash(&mut hasher);
        edge.source.hash(&mut hasher);
        edge.target.hash(&mut hasher);
    }
    hasher.finish()
}

fn account_quota_remaining(account: &crate::db::accounts::Account) -> Option<f64> {
    account
        .quota_windows
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<serde_json::Value>>(raw).ok())
        .and_then(|windows| {
            windows
                .iter()
                .filter_map(|window| {
                    window
                        .get("remaining_percent")
                        .and_then(|value| value.as_f64())
                })
                .reduce(f64::min)
        })
        .or_else(|| {
            account
                .quota_limit
                .filter(|limit| *limit > 0.0)
                .map(|limit| {
                    ((limit - account.quota_used.unwrap_or(0.0)) / limit * 100.0).clamp(0.0, 100.0)
                })
        })
}

fn canonical_protocol(raw: &str) -> String {
    let normalized = raw.trim().to_lowercase().replace('-', "_");
    match normalized.as_str() {
        "openai" | "chat" | "chat_completions" | "openai_chat" => "chat".into(),
        "response" | "responses" | "openai_responses" | "codex" => "responses".into(),
        "messages" | "anthropic" | "anthropic_messages" | "claude" => "anthropic".into(),
        "google" | "gemini" | "google_gemini" => "gemini".into(),
        "multi" | "both" => "both".into(),
        "unified" | "unified_responses" | "unified_entry" => "unified".into(),
        value => value.into(),
    }
}

pub fn pool_entry_protocols(raw: &str) -> Vec<String> {
    match canonical_protocol(raw).as_str() {
        "chat" => vec!["chat".into(), "responses".into()],
        "responses" => vec!["responses".into()],
        "anthropic" => vec!["anthropic".into()],
        "gemini" => vec!["gemini".into()],
        "both" => vec!["chat".into(), "responses".into(), "anthropic".into()],
        "unified" => vec![
            "chat".into(),
            "responses".into(),
            "anthropic".into(),
            "gemini".into(),
        ],
        value => vec![value.into()],
    }
}

fn provider_protocols(
    provider: &crate::db::providers::Provider,
) -> std::collections::HashSet<String> {
    let mut protocols = std::collections::HashSet::from([canonical_protocol(&provider.protocol)]);
    if let Some(raw) = provider
        .protocols
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let declared = serde_json::from_str::<Vec<String>>(raw).unwrap_or_else(|_| {
            raw.split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        });
        protocols.extend(declared.iter().map(|value| canonical_protocol(value)));
    }
    protocols
}

fn provider_protocol_compatible(
    group_protocol: &str,
    provider: &crate::db::providers::Provider,
) -> bool {
    let group_protocol = canonical_protocol(group_protocol);
    let supported = provider_protocols(provider);
    match group_protocol.as_str() {
        // OpenAI-compatible pools expose both Chat Completions and Responses endpoints.
        "chat" => {
            supported.contains("chat")
                || supported.contains("responses")
                || supported.contains("both")
        }
        // The combined pool intentionally means OpenAI-compatible + Anthropic, not Gemini.
        "both" => supported
            .iter()
            .any(|value| matches!(value.as_str(), "chat" | "responses" | "anthropic" | "both")),
        // Unified pool supports all protocols through protocol conversion.
        "unified" => true,
        "anthropic" => supported.contains("anthropic") || supported.contains("both"),
        value => supported.contains(value),
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::{pool_entry_protocols, provider_protocol_compatible};
    use crate::db::providers::Provider;

    fn provider(protocol: &str, protocols: Option<&str>) -> Provider {
        Provider {
            id: "provider-test".into(),
            name: "Test Provider".into(),
            provider_type: "custom".into(),
            base_url: "https://example.com".into(),
            base_urls: None,
            protocol: protocol.into(),
            protocols: protocols.map(str::to_string),
            route_takeover: None,
            api_keys: None,
            models: None,
            proxy_url: None,
            custom_headers: None,
            timeout_ms: None,
            priority: None,
            enabled: Some(true),
            created_at: None,
            auth_mode: None,
            oauth_config: None,
        }
    }

    #[test]
    fn pool_protocols_expand_to_real_gateway_adapters() {
        assert_eq!(pool_entry_protocols("openai"), vec!["chat", "responses"]);
        assert_eq!(
            pool_entry_protocols("both"),
            vec!["chat", "responses", "anthropic"]
        );
        assert_eq!(pool_entry_protocols("gemini"), vec!["gemini"]);
    }

    #[test]
    fn openai_pool_accepts_chat_and_responses_aliases() {
        assert!(provider_protocol_compatible(
            "openai",
            &provider("chat", None)
        ));
        assert!(provider_protocol_compatible(
            "openai",
            &provider("openai_responses", None)
        ));
        assert!(provider_protocol_compatible(
            "openai",
            &provider("codex", None)
        ));
    }

    #[test]
    fn provider_multi_protocol_declaration_is_respected() {
        let provider = provider("chat", Some(r#"["chat","responses"]"#));
        assert!(provider_protocol_compatible("openai", &provider));
        assert!(provider_protocol_compatible("responses", &provider));
    }

    #[test]
    fn protocols_remain_isolated() {
        assert!(!provider_protocol_compatible(
            "openai",
            &provider("anthropic", None)
        ));
        assert!(!provider_protocol_compatible(
            "openai",
            &provider("gemini", None)
        ));
        assert!(!provider_protocol_compatible(
            "anthropic",
            &provider("responses", None)
        ));
    }

    #[test]
    fn unified_pool_accepts_all_protocols() {
        // Unified pool should accept any provider protocol
        assert!(provider_protocol_compatible(
            "unified",
            &provider("chat", None)
        ));
        assert!(provider_protocol_compatible(
            "unified",
            &provider("anthropic", None)
        ));
        assert!(provider_protocol_compatible(
            "unified",
            &provider("gemini", None)
        ));
        assert!(provider_protocol_compatible(
            "unified",
            &provider("responses", None)
        ));
        // Also test with aliases
        assert!(provider_protocol_compatible(
            "unified",
            &provider("openai", None)
        ));
        assert!(provider_protocol_compatible(
            "unified",
            &provider("claude", None)
        ));
    }
}

#[derive(serde::Serialize)]
pub struct ProviderQuotaSummary {
    pub provider_id: String,
    pub provider_name: String,
    pub account_count: usize,
    pub average_used_percent: f64,
    pub max_used_percent: f64,
    pub min_remaining_percent: f64,
    pub abnormal_accounts: usize,
}

#[tauri::command]
pub fn list_groups(state: State<'_, Arc<AppState>>) -> Result<Vec<AgentGroup>, String> {
    state
        .db
        .groups
        .list_all(&state.db.conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_group(
    state: State<'_, Arc<AppState>>,
    group: AgentGroup,
) -> Result<crate::services::pool_management::RoutePoolCreated, String> {
    crate::services::pool_management::create_pool_with_key(state.inner().as_ref(), group)
}

#[tauri::command]
pub fn ensure_group_client_key(
    state: State<'_, Arc<AppState>>,
    group_id: String,
) -> Result<crate::services::pool_management::ManagedPoolKeyCreated, String> {
    crate::services::pool_management::ensure_pool_key(state.inner().as_ref(), &group_id)
}

#[tauri::command]
pub fn rotate_group_client_key(
    state: State<'_, Arc<AppState>>,
    group_id: String,
) -> Result<crate::services::pool_management::ManagedPoolKeyCreated, String> {
    crate::services::pool_management::rotate_pool_key(state.inner().as_ref(), &group_id)
}

#[tauri::command]
pub fn update_group(state: State<'_, Arc<AppState>>, group: AgentGroup) -> Result<(), String> {
    state
        .db
        .groups
        .update(&state.db.conn, &group)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_group(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    state
        .db
        .groups
        .delete(&state.db.conn, &id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_group_accounts(
    state: State<'_, Arc<AppState>>,
    group_id: String,
) -> Result<Vec<String>, String> {
    state
        .db
        .groups
        .get_account_ids(&state.db.conn, &group_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_group_model_resources(
    state: State<'_, Arc<AppState>>,
    group_id: String,
) -> Result<Vec<GroupModelResource>, String> {
    state
        .db
        .groups
        .get_model_resources(&state.db.conn, &group_id)
}

#[tauri::command]
pub fn list_available_group_model_resources(
    state: State<'_, Arc<AppState>>,
    group_id: String,
) -> Result<Vec<AvailableGroupModelResource>, String> {
    let group = state
        .db
        .groups
        .get_by_id(&state.db.conn, &group_id)?
        .ok_or_else(|| "POOL_NOT_FOUND: 路由池不存在".to_string())?;
    let providers = state.db.providers.list_all(&state.db.conn)?;
    let accounts = state.db.accounts.list_all(&state.db.conn)?;
    let existing: std::collections::HashSet<_> = state
        .db
        .groups
        .get_model_resources(&state.db.conn, &group_id)?
        .into_iter()
        .collect();
    let mut output = Vec::new();

    for provider in providers.iter().filter(|provider| {
        provider.enabled != Some(false) && provider_protocol_compatible(&group.protocol, provider)
    }) {
        let provider_accounts: Vec<_> = accounts
            .iter()
            .filter(|account| {
                account.provider_id.as_deref() == Some(provider.id.as_str())
                    && crate::services::credentials::is_available_for_routing(account)
            })
            .collect();
        let mut models: std::collections::BTreeSet<String> =
            parse_models(provider.models.as_deref())
                .into_iter()
                .collect();
        for account in &provider_accounts {
            models.extend(parse_models(account.models.as_deref()));
        }
        for model in models {
            let supporting: Vec<_> = provider_accounts
                .iter()
                .filter(|account| {
                    let declared = parse_models(account.models.as_deref());
                    declared.is_empty() || declared.iter().any(|item| item == &model)
                })
                .collect();
            if supporting.is_empty() {
                continue;
            }
            let resource = GroupModelResource {
                provider_id: provider.id.clone(),
                model: model.clone(),
            };
            let already_added = existing.contains(&resource);
            let selected_ids = if already_added {
                state
                    .db
                    .groups
                    .get_model_account_ids(&state.db.conn, &group_id, &provider.id, &model)?
                    .into_iter()
                    .collect::<std::collections::HashSet<_>>()
            } else {
                std::collections::HashSet::new()
            };
            let accounts = supporting
                .iter()
                .map(|account| AvailableGroupModelAccount {
                    id: account.id.clone(),
                    name: account.name.clone().unwrap_or_else(|| account.id.clone()),
                    email: account.email.clone(),
                    status: account.status.clone().unwrap_or_else(|| "unchecked".into()),
                    health_status: account
                        .health_status
                        .clone()
                        .unwrap_or_else(|| "unchecked".into()),
                    routable: crate::services::credentials::is_available_for_routing(account),
                    selected: !already_added || selected_ids.contains(&account.id),
                })
                .collect::<Vec<_>>();
            output.push(AvailableGroupModelResource {
                provider_id: provider.id.clone(),
                provider_name: provider.name.clone(),
                model,
                account_count: supporting.len(),
                healthy_count: supporting
                    .iter()
                    .filter(|account| {
                        matches!(
                            account.health_status.as_deref(),
                            Some("healthy") | Some("unchecked") | None
                        )
                    })
                    .count(),
                already_added,
                accounts,
            });
        }
    }
    output.sort_by(|left, right| {
        left.provider_name
            .cmp(&right.provider_name)
            .then_with(|| left.model.cmp(&right.model))
    });
    Ok(output)
}

fn validate_group_model_resources(
    state: &AppState,
    group_id: &str,
    resources: &[GroupModelResource],
) -> Result<Vec<GroupModelResource>, String> {
    let group = state
        .db
        .groups
        .get_by_id(&state.db.conn, group_id)?
        .ok_or_else(|| "POOL_NOT_FOUND: 路由池不存在".to_string())?;
    let providers = state.db.providers.list_all(&state.db.conn)?;
    let accounts = state.db.accounts.list_all(&state.db.conn)?;
    let mut unique = std::collections::HashSet::new();
    let mut normalized = Vec::new();

    for resource in resources {
        let model = resource.model.trim();
        if model.is_empty() {
            return Err("MODEL_RESOURCE_INVALID: 模型名称不能为空".into());
        }
        let provider = providers
            .iter()
            .find(|provider| provider.id == resource.provider_id)
            .ok_or_else(|| {
                format!(
                    "MODEL_PROVIDER_NOT_FOUND: Provider '{}' 不存在",
                    resource.provider_id
                )
            })?;
        if provider.enabled == Some(false) {
            return Err(format!(
                "MODEL_PROVIDER_DISABLED: Provider '{}' 已停用",
                provider.name
            ));
        }
        if !provider_protocol_compatible(&group.protocol, provider) {
            return Err(format!(
                "MODEL_PROTOCOL_MISMATCH: Provider '{}' 与路由池协议不兼容",
                provider.name
            ));
        }

        let provider_declared = parse_models(provider.models.as_deref());
        let routable_accounts: Vec<_> = accounts
            .iter()
            .filter(|account| {
                account.provider_id.as_deref() == Some(provider.id.as_str())
                    && crate::services::credentials::is_available_for_routing(account)
            })
            .collect();
        let supported = routable_accounts.iter().any(|account| {
            let account_declared = parse_models(account.models.as_deref());
            if account_declared.is_empty() {
                provider_declared.is_empty() || provider_declared.iter().any(|item| item == model)
            } else {
                account_declared.iter().any(|item| item == model)
            }
        });
        if !supported {
            return Err(format!(
                "MODEL_RESOURCE_NOT_FOUND: Provider '{}' 没有可路由账号承载模型 '{}'",
                provider.name, model
            ));
        }

        let item = GroupModelResource {
            provider_id: resource.provider_id.clone(),
            model: model.to_string(),
        };
        if unique.insert(item.clone()) {
            normalized.push(item);
        }
    }
    Ok(normalized)
}

#[tauri::command]
pub fn add_group_model_resources(
    state: State<'_, Arc<AppState>>,
    group_id: String,
    resources: Vec<GroupModelResource>,
) -> Result<usize, String> {
    let resources = validate_group_model_resources(state.inner().as_ref(), &group_id, &resources)?;
    state
        .db
        .groups
        .add_model_resources(&state.db.conn, &group_id, &resources)
}

#[tauri::command]
pub fn set_group_model_account_ids(
    state: State<'_, Arc<AppState>>,
    group_id: String,
    provider_id: String,
    model: String,
    account_ids: Vec<String>,
) -> Result<(), String> {
    let resources = state
        .db
        .groups
        .get_model_resources(&state.db.conn, &group_id)?;
    if !resources
        .iter()
        .any(|resource| resource.provider_id == provider_id && resource.model == model)
    {
        return Err("MODEL_RESOURCE_NOT_FOUND: 模型资源尚未加入路由池".into());
    }
    let provider = state
        .db
        .providers
        .list_all(&state.db.conn)?
        .into_iter()
        .find(|item| item.id == provider_id)
        .ok_or_else(|| "MODEL_PROVIDER_NOT_FOUND: Provider 不存在".to_string())?;
    let accounts = state.db.accounts.list_all(&state.db.conn)?;
    let allowed: std::collections::HashSet<String> = accounts
        .iter()
        .filter(|account| {
            account.provider_id.as_deref() == Some(provider_id.as_str())
                && crate::services::credentials::is_available_for_routing(account)
                && {
                    let declared = parse_models(account.models.as_deref());
                    declared.is_empty() || declared.iter().any(|item| item == &model)
                }
        })
        .map(|account| account.id.clone())
        .collect();
    let mut selected = account_ids;
    selected.sort();
    selected.dedup();
    if selected.is_empty() {
        return Err("MODEL_ACCOUNTS_EMPTY: 至少保留一个可路由账号".into());
    }
    if let Some(invalid) = selected.iter().find(|id| !allowed.contains(*id)) {
        return Err(format!(
            "MODEL_ACCOUNT_INVALID: 账号 '{}' 不支持模型 '{}'，或状态不可用（已停用/Token 过期/健康异常），不能加入路由池",
            invalid, model
        ));
    }
    state.db.groups.set_model_account_ids(
        &state.db.conn,
        &group_id,
        &provider.id,
        &model,
        &selected,
    )
}

#[tauri::command]
pub fn remove_group_model_resource(
    state: State<'_, Arc<AppState>>,
    group_id: String,
    provider_id: String,
    model: String,
) -> Result<bool, String> {
    if state
        .db
        .groups
        .get_by_id(&state.db.conn, &group_id)?
        .is_none()
    {
        return Err("POOL_NOT_FOUND: 路由池不存在".into());
    }
    state
        .db
        .groups
        .remove_model_resource(&state.db.conn, &group_id, &provider_id, &model)
}

#[tauri::command]
pub fn set_group_model_resources(
    state: State<'_, Arc<AppState>>,
    group_id: String,
    resources: Vec<GroupModelResource>,
) -> Result<(), String> {
    let resources = validate_group_model_resources(state.inner().as_ref(), &group_id, &resources)?;
    state
        .db
        .groups
        .set_model_resources(&state.db.conn, &group_id, &resources)
}

#[tauri::command]
pub fn get_group_dashboard(
    state: State<'_, Arc<AppState>>,
    group_id: String,
    range: Option<String>,
) -> Result<GroupDashboard, String> {
    build_group_dashboard(state.inner().as_ref(), &group_id, range.as_deref())
}

pub(crate) fn build_group_dashboard(
    state: &AppState,
    group_id: &str,
    range: Option<&str>,
) -> Result<GroupDashboard, String> {
    let resources = state
        .db
        .groups
        .get_model_resources(&state.db.conn, group_id)?;
    let accounts = state.db.accounts.list_all(&state.db.conn)?;
    let providers = state.db.providers.list_all(&state.db.conn)?;
    let provider_ids: std::collections::HashSet<_> = resources
        .iter()
        .map(|resource| resource.provider_id.as_str())
        .collect();
    let effective: Vec<_> = accounts
        .iter()
        .filter(|account| {
            let Some(provider_id) = account.provider_id.as_deref() else {
                return false;
            };
            if !provider_ids.contains(provider_id)
                || account.status.as_deref() == Some("disabled")
                || !crate::services::credentials::is_directly_routable(account)
            {
                return false;
            }
            let provider_models = providers
                .iter()
                .find(|provider| provider.id == provider_id)
                .map(|provider| parse_models(provider.models.as_deref()))
                .unwrap_or_default();
            let account_models = parse_models(account.models.as_deref());
            let declared = if account_models.is_empty() {
                provider_models
            } else {
                account_models
            };
            resources.iter().any(|resource| {
                resource.provider_id == provider_id
                    && (declared.is_empty()
                        || declared.iter().any(|model| model == &resource.model))
            })
        })
        .collect();
    let mut quota_by_provider = Vec::new();
    for provider in providers
        .iter()
        .filter(|provider| provider_ids.contains(provider.id.as_str()))
    {
        let scoped: Vec<_> = effective
            .iter()
            .filter(|account| account.provider_id.as_deref() == Some(provider.id.as_str()))
            .collect();
        let mut used_values = Vec::new();
        let mut remaining_values = Vec::new();
        let mut abnormal = 0usize;
        for account in &scoped {
            if account.quota_error.is_some() {
                abnormal += 1;
            }
            if let Some(raw) = &account.quota_windows {
                if let Ok(windows) = serde_json::from_str::<Vec<serde_json::Value>>(raw) {
                    for window in windows {
                        if let Some(value) =
                            window.get("used_percent").and_then(|value| value.as_f64())
                        {
                            used_values.push(value);
                        }
                        if let Some(value) = window
                            .get("remaining_percent")
                            .and_then(|value| value.as_f64())
                        {
                            remaining_values.push(value);
                        }
                    }
                }
            }
        }
        quota_by_provider.push(ProviderQuotaSummary {
            provider_id: provider.id.clone(),
            provider_name: provider.name.clone(),
            account_count: scoped.len(),
            average_used_percent: if used_values.is_empty() {
                0.0
            } else {
                used_values.iter().sum::<f64>() / used_values.len() as f64
            },
            max_used_percent: used_values.iter().copied().fold(0.0, f64::max),
            min_remaining_percent: remaining_values.iter().copied().fold(100.0, f64::min),
            abnormal_accounts: abnormal,
        });
    }
    Ok(GroupDashboard {
        resource_count: effective.len(),
        healthy_resource_count: effective
            .iter()
            .filter(|account| {
                matches!(
                    account.health_status.as_deref(),
                    Some("healthy") | Some("unchecked") | None
                )
            })
            .count(),
        model_count: resources
            .iter()
            .map(|resource| resource.model.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        provider_count: provider_ids.len(),
        traffic: state
            .db
            .logs
            .get_group_stats(&state.db.conn, group_id, range)?,
        quota_by_provider,
    })
}

#[tauri::command]
pub fn get_route_topology(state: State<'_, Arc<AppState>>) -> Result<RouteTopology, String> {
    let groups = state.db.groups.list_all(&state.db.conn)?;
    let providers = state.db.providers.list_all(&state.db.conn)?;
    let accounts = state.db.accounts.list_all(&state.db.conn)?;
    let provider_traffic: std::collections::HashMap<_, _> = state
        .db
        .logs
        .get_provider_stats(&state.db.conn)?
        .into_iter()
        .map(|traffic| (traffic.provider_id.clone(), traffic))
        .collect();

    let mut topology_pools = Vec::with_capacity(groups.len());
    let mut referenced_provider_ids = std::collections::HashSet::new();
    for group in groups {
        let resources = state
            .db
            .groups
            .get_model_resources(&state.db.conn, &group.id)?;
        let mut provider_ids: Vec<_> = resources
            .iter()
            .map(|resource| resource.provider_id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        provider_ids.sort_by(|left, right| {
            let left_name = providers
                .iter()
                .find(|provider| provider.id == *left)
                .map(|provider| provider.name.as_str())
                .unwrap_or(left.as_str());
            let right_name = providers
                .iter()
                .find(|provider| provider.id == *right)
                .map(|provider| provider.name.as_str())
                .unwrap_or(right.as_str());
            left_name.cmp(right_name)
        });
        referenced_provider_ids.extend(provider_ids.iter().cloned());
        let dashboard = build_group_dashboard(state.inner().as_ref(), &group.id, Some("all"))?;
        topology_pools.push(RouteTopologyPool {
            id: group.id,
            name: group.name,
            protocol: group.protocol,
            strategy: group.strategy,
            enabled: group.enabled.unwrap_or(true),
            provider_ids,
            resource_count: dashboard.resource_count,
            healthy_resource_count: dashboard.healthy_resource_count,
            model_count: dashboard.model_count,
            traffic: dashboard.traffic,
        });
    }

    // Keep the raw provider rows around: the protocol layer must be derived
    // from the protocols each provider actually speaks (not the pool's static
    // declaration), so unrelated protocol nodes never appear in the graph.
    let providers_by_id: std::collections::HashMap<_, _> = providers
        .iter()
        .map(|provider| (provider.id.clone(), provider.clone()))
        .collect();
    let topology_providers: Vec<RouteTopologyProvider> = providers
        .into_iter()
        .filter(|provider| referenced_provider_ids.contains(&provider.id))
        .map(|provider| {
            let scoped: Vec<_> = accounts
                .iter()
                .filter(|account| account.provider_id.as_deref() == Some(provider.id.as_str()))
                .collect();
            let healthy_account_count = scoped
                .iter()
                .filter(|account| {
                    account.status.as_deref() != Some("disabled")
                        && !matches!(
                            account.health_status.as_deref(),
                            Some("error") | Some("unhealthy")
                        )
                        && crate::services::credentials::is_directly_routable(account)
                })
                .count();
            let traffic = provider_traffic
                .get(&provider.id)
                .cloned()
                .unwrap_or_else(|| crate::db::logs::ProviderTrafficStats {
                    provider_id: provider.id.clone(),
                    ..Default::default()
                });
            let host = provider_host(&provider.base_url);
            RouteTopologyProvider {
                id: provider.id,
                name: provider.name,
                protocol: provider.protocol,
                enabled: provider.enabled.unwrap_or(true),
                account_count: scoped.len(),
                healthy_account_count,
                traffic,
                host,
            }
        })
        .collect();

    // Fifth layer: concrete accounts reachable under the referenced providers.
    // The canvas decides whether to expand them or render a collapsed summary.
    let topology_accounts: Vec<RouteTopologyAccount> = accounts
        .iter()
        .filter(|account| {
            account
                .provider_id
                .as_deref()
                .map(|id| referenced_provider_ids.contains(id))
                .unwrap_or(false)
        })
        .map(|account| RouteTopologyAccount {
            id: account.id.clone(),
            provider_id: account.provider_id.clone().unwrap_or_default(),
            name: account_display_name(account),
            email_masked: account.email.as_deref().and_then(|email| mask_email(Some(email))),
            status: account.status.clone().unwrap_or_else(|| "unchecked".into()),
            health_status: account.health_status.clone().unwrap_or_default(),
            routable: crate::services::credentials::is_directly_routable(account),
            plan_type: account.plan_type.clone(),
        })
        .collect();

    let active_route = state.gateway_runtime.latest_route();
    let active_protocol = active_route.as_ref().map(|route| route.protocol.as_str());
    let active_pool = active_route.as_ref().map(|route| route.pool_id.as_str());
    let active_provider = active_route
        .as_ref()
        .map(|route| route.provider_id.as_str());
    // Protocol nodes and protocol->pool edges are derived from the protocols
    // the pool's providers actually speak. A pool declaring "unified"/"both"
    // only renders the protocol nodes that have at least one real upstream
    // behind them, so the graph never connects unrelated layers.
    let mut protocols_by_id: std::collections::BTreeMap<String, RouteTopologyProtocol> =
        std::collections::BTreeMap::new();
    let mut gateway_protocol_seen: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut edges = Vec::new();
    for pool in &topology_pools {
        for provider_id in &pool.provider_ids {
            let provider = providers_by_id.get(provider_id);
            let supported = provider
                .map(provider_protocols)
                .unwrap_or_default();
            let provider_enabled = provider
                .map(|provider| provider.enabled.unwrap_or(true))
                .unwrap_or(true);
            for protocol in supported {
                let protocol_id = format!("protocol-{}", protocol);
                let entry = protocols_by_id
                    .entry(protocol_id.clone())
                    .or_insert_with(|| RouteTopologyProtocol {
                        id: protocol_id.clone(),
                        name: protocol_display_name(&protocol),
                        protocol: protocol.clone(),
                        enabled: false,
                        pool_ids: Vec::new(),
                        request_count: 0,
                        traffic: crate::db::logs::GroupTrafficStats::default(),
                    });
                entry.enabled |= pool.enabled;
                if !entry.pool_ids.contains(&pool.id) {
                    entry.pool_ids.push(pool.id.clone());
                }
                if !edges
                    .iter()
                    .any(|edge: &RouteTopologyEdge| {
                        edge.id == format!("{}-pool-{}", protocol_id, pool.id)
                    })
                {
                    edges.push(RouteTopologyEdge {
                        id: format!("{}-pool-{}", protocol_id, pool.id),
                        source: protocol_id.clone(),
                        target: format!("pool-{}", pool.id),
                        status: if pool.enabled { "healthy" } else { "disabled" }.into(),
                        active: active_protocol == Some(protocol.as_str())
                            && active_pool == Some(pool.id.as_str()),
                        active_requests: 0,
                    });
                }
                if gateway_protocol_seen.insert(protocol_id.clone()) {
                    edges.push(RouteTopologyEdge {
                        id: format!("gateway-{}", protocol_id),
                        source: "gateway".into(),
                        target: protocol_id.clone(),
                        status: if pool.enabled && provider_enabled {
                            "healthy"
                        } else {
                            "disabled"
                        }
                        .into(),
                        active: active_protocol == Some(protocol.as_str()),
                        active_requests: 0,
                    });
                }
            }
        }
        for provider_id in &pool.provider_ids {
            let provider = topology_providers
                .iter()
                .find(|provider| provider.id == *provider_id);
            let status = provider
                .map(|provider| {
                    if !provider.enabled {
                        "disabled"
                    } else if provider.healthy_account_count == 0 {
                        "warning"
                    } else {
                        "healthy"
                    }
                })
                .unwrap_or("warning");
            edges.push(RouteTopologyEdge {
                id: format!("pool-{}-provider-{}", pool.id, provider_id),
                source: format!("pool-{}", pool.id),
                target: format!("provider-{}", provider_id),
                status: status.into(),
                active: active_pool == Some(pool.id.as_str())
                    && active_provider == Some(provider_id.as_str()),
                active_requests: 0,
            });
        }
    }
    // Provider -> account edges (fifth layer).
    for account in &topology_accounts {
        edges.push(RouteTopologyEdge {
            id: format!("provider-{}-account-{}", account.provider_id, account.id),
            source: format!("provider-{}", account.provider_id),
            target: format!("account-{}", account.id),
            status: account_topology_status(&account).into(),
            active: false,
            active_requests: 0,
        });
    }
    let mut protocols: Vec<_> = protocols_by_id.into_values().collect();
    for protocol in &mut protocols {
        protocol.traffic = state.db.logs.get_protocol_stats(
            &state.db.conn,
            &protocol.protocol,
            &protocol.pool_ids,
        )?;
        protocol.request_count = protocol.traffic.total_requests;
    }
    protocols.sort_by(|left, right| left.name.cmp(&right.name));
    let (running, port) = state
        .proxy
        .lock()
        .map(|proxy| {
            proxy
                .as_ref()
                .map(|handle| (true, handle.port))
                .unwrap_or((false, 9800))
        })
        .unwrap_or((false, 9800));

    let runtime = state.gateway_runtime.snapshot();
    for edge in &mut edges {
        edge.active_requests = runtime
            .edge_deltas
            .iter()
            .find(|delta| delta.id == edge.id)
            .map(|delta| delta.active_requests)
            .unwrap_or(0);
        edge.active = edge.active_requests > 0;
    }
    let topology_revision = topology_revision(
        &protocols,
        &topology_pools,
        &topology_providers,
        &topology_accounts,
        &edges,
    );

    Ok(RouteTopology {
        version: 2,
        topology_revision,
        gateway: RouteTopologyGateway {
            id: "gateway".into(),
            name: "PoolGate Gateway".into(),
            address: format!("http://127.0.0.1:{}", port),
            running,
            active_connections: state.gateway_runtime.active_connections(),
        },
        protocols,
        pools: topology_pools,
        providers: topology_providers,
        accounts: topology_accounts,
        edges,
        active_route,
        runtime,
        updated_at: chrono::Local::now().to_rfc3339(),
    })
}

#[tauri::command]
pub fn get_provider_topology_detail(
    state: State<'_, Arc<AppState>>,
    provider_id: String,
) -> Result<ProviderTopologyDetail, String> {
    let provider = state
        .db
        .providers
        .get_by_id(&state.db.conn, &provider_id)?
        .ok_or_else(|| "PROVIDER_NOT_FOUND: 上游厂商不存在".to_string())?;
    let traffic = state
        .db
        .logs
        .get_provider_stats(&state.db.conn)?
        .into_iter()
        .find(|item| item.provider_id == provider_id)
        .unwrap_or_else(|| crate::db::logs::ProviderTrafficStats {
            provider_id: provider_id.clone(),
            ..Default::default()
        });
    let mut accounts: Vec<_> = state
        .db
        .accounts
        .list_all(&state.db.conn)?
        .into_iter()
        .filter(|account| account.provider_id.as_deref() == Some(provider_id.as_str()))
        .map(|account| {
            let concurrency = state.account_concurrency.snapshot(&account.id);
            let email_masked = mask_email(account.email.as_deref());
            ProviderTopologyAccount {
                id: account.id.clone(),
                name: account
                    .name
                    .clone()
                    .filter(|name| !name.trim().is_empty())
                    .or_else(|| email_masked.clone())
                    .unwrap_or_else(|| "未命名账号".into()),
                email_masked,
                credential_type: account
                    .credential_type
                    .clone()
                    .unwrap_or_else(|| "api_key".into()),
                source_format: account.source_format.clone(),
                routable: crate::services::credentials::is_directly_routable(&account),
                status: account.status.clone().unwrap_or_else(|| "enabled".into()),
                health_status: account
                    .health_status
                    .clone()
                    .unwrap_or_else(|| "unchecked".into()),
                health_latency_ms: account.health_latency,
                last_used_at: account.last_used_at.clone(),
                plan_type: account.plan_type.clone(),
                models: parse_models(account.models.as_deref()),
                quota_remaining_percent: account_quota_remaining(&account),
                concurrency_limit: concurrency.limit,
                concurrency_active: concurrency.active,
                concurrency_available: concurrency.available,
                queued_requests: concurrency.queued,
                expires_at: account.expires_at.clone(),
                last_error: account.health_msg.clone(),
                traffic: crate::db::logs::AccountTrafficStats {
                    account_id: account.id.clone(),
                    provider_id: provider_id.clone(),
                    ..Default::default()
                },
            }
        })
        .collect();
    accounts.sort_by(|left, right| left.name.cmp(&right.name));
    // Merge real per-account traffic (from actual upstream attempts) into the
    // account rows. Metrics that have no source stay "未提供" on the client;
    // they must never be synthesized.
    let account_stats = state
        .db
        .logs
        .get_account_stats(&state.db.conn, std::slice::from_ref(&provider_id))?;
    let stats_by_account: std::collections::HashMap<_, _> = account_stats
        .into_iter()
        .map(|stats| (stats.account_id.clone(), stats))
        .collect();
    for account in &mut accounts {
        if let Some(stats) = stats_by_account.get(&account.id) {
            account.traffic = stats.clone();
        }
    }
    let protocols = provider_protocols(&provider).into_iter().collect();
    let base_url_masked = mask_base_url(&provider.base_url);
    Ok(ProviderTopologyDetail {
        id: provider.id,
        name: provider.name,
        provider_type: provider.provider_type,
        protocol: provider.protocol,
        protocols,
        base_url_masked,
        enabled: provider.enabled.unwrap_or(true),
        models: parse_models(provider.models.as_deref()),
        traffic,
        accounts,
    })
}

#[tauri::command]
pub fn add_account_to_group(
    state: State<'_, Arc<AppState>>,
    group_id: String,
    account_id: String,
    weight: Option<i64>,
) -> Result<(), String> {
    // 无效账号（已停用 / Token 过期 / 健康异常 / 耗尽）不允许加入路由池。
    let account = state
        .db
        .accounts
        .get_by_id(&state.db.conn, &account_id)?
        .ok_or_else(|| "ACCOUNT_NOT_FOUND: 账号不存在".to_string())?;
    if !crate::services::credentials::is_available_for_routing(&account) {
        return Err(
            "ACCOUNT_NOT_ROUTABLE: 账号状态不可用（已停用 / Token 过期 / 健康异常 / 耗尽），不能加入路由池"
                .into(),
        );
    }
    state
        .db
        .groups
        .add_account(&state.db.conn, &group_id, &account_id, weight.unwrap_or(1))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_account_from_group(
    state: State<'_, Arc<AppState>>,
    group_id: String,
    account_id: String,
) -> Result<(), String> {
    state
        .db
        .groups
        .remove_account(&state.db.conn, &group_id, &account_id)
        .map_err(|e| e.to_string())
}
