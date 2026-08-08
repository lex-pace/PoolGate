//! Automatic pool onboarding for OAuth accounts.
//!
//! When a user completes an OAuth login (Codex, Claude, Copilot, Gemini, Grok),
//! the account should be automatically added to a routing pool so it's immediately
//! available for gateway requests.
//!
//! Strategy:
//! 1. Find or create a pool matching the provider type (e.g., "codex" → "Codex Pool").
//! 2. Add the account to that pool with weight=1 (round-robin default).
//! 3. Add the provider to the pool's model resources if not already present.

use crate::db::groups::{AgentGroup, GroupModelResource};
use crate::AppState;

/// Ensure an OAuth account is available in a routing pool immediately after login.
///
/// If no pool exists for this provider type, one is created with the appropriate
/// protocol configuration. The account is added to the pool with default weight.
pub fn ensure_account_in_pool(
    state: &AppState,
    account_id: &str,
    provider_id: &str,
    models: &[String],
    pool_label: &str,
    pool_protocol: &str,
) -> Result<String, String> {
    let db = &state.db;

    // 1. Find existing pool by name, or create one.
    let pool_name = format!("{} Pool", pool_label);
    let groups = db.groups.list_all(&db.conn)?;
    let pool_id = if let Some(existing) = groups.iter().find(|g| g.name == pool_name) {
        existing.id.clone()
    } else {
        // Create a new pool.
        let pool_id = format!("pool_{}", uuid::Uuid::new_v4().simple());
        let group = AgentGroup {
            id: pool_id.clone(),
            name: pool_name.clone(),
            description: Some(format!("Auto-created for {} OAuth accounts", pool_label)),
            protocol: pool_protocol.to_string(),
            strategy: Some("round_robin".into()),
            api_key: None,
            enabled: Some(true),
            created_at: None,
        };
        db.groups.create(&db.conn, &group)?;
        tracing::info!("Auto-created routing pool '{}' ({})", pool_name, pool_id);
        pool_id
    };

    // 2. Add account to pool (idempotent via INSERT OR REPLACE).
    db.groups.add_account(&db.conn, &pool_id, account_id, 1)?;

    // 3. Add provider's models to pool's model resources.
    if !models.is_empty() {
        let resources: Vec<GroupModelResource> = models
            .iter()
            .map(|model| GroupModelResource {
                provider_id: provider_id.to_string(),
                model: model.clone(),
            })
            .collect();
        db.groups.add_model_resources(&db.conn, &pool_id, &resources)?;
    }

    tracing::info!(
        "Account '{}' added to pool '{}' ({}) with provider '{}'",
        account_id,
        pool_name,
        pool_id,
        provider_id
    );

    Ok(pool_id)
}
