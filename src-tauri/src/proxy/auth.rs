//! Gateway authentication context & route-pool resolution.
//!
//! The formal authentication identity is a *virtual client key* (see
//! `db::client_keys`). This module resolves the presented key on each request
//! and decides which route pool the request may use:
//!
//! Precedence (highest first):
//!   1. Explicit `X-Group-Id` header — debugging / admin advanced usage.
//!      - Admin (gateway access key) may target any pool.
//!      - A virtual client key may only target a pool it is bound to,
//!        otherwise the request is rejected (403) — no privilege escalation.
//!   2. The first pool bound to the virtual client key (the normal flow).
//!   3. `default` — falls back to the full account pool (legacy import-just-use).
//!
//! Extension points (reserved for later capabilities, already stored on the
//! `client_keys` row): `rpm_limit`/`tpm_limit` (rate limiting),
//! `allowed_protocols`/`allowed_models` (permission scope), `last_used_at`
//! (usage statistics), `client_key_id` in the request log (audit).

use crate::proxy::server::ProxyState;
use axum::async_trait;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

/// Authentication context attached to the request extensions by the auth
/// middleware. Handlers use it for routing + audit.
#[derive(Clone, Debug)]
pub struct ClientKeyAuth {
    pub key_id: String,
    pub name: String,
    pub key_last_four: String,
    /// Route pools this key is bound to (from `client_key_pools`).
    pub pool_ids: Vec<String>,
    /// True when the request authenticated with the gateway access key
    /// (admin) — allowed to use the explicit X-Group-Id header freely.
    pub is_admin: bool,
    /// Rate-limit extension fields (0/None = unlimited).
    pub rpm_limit: Option<i64>,
    pub tpm_limit: Option<i64>,
    /// Permission-scope extension fields (JSON arrays or None = allow all).
    pub allowed_protocols: Option<String>,
    pub allowed_models: Option<String>,
}

impl ClientKeyAuth {
    /// Display fingerprint used in logs (audit identity, no secrets).
    pub fn fingerprint(&self) -> String {
        format!("{}…{}", &self.name, self.key_last_four)
    }
}

/// Extension hook — rate limiting. Returns `Some(message)` when the request
/// must be rejected (429). Currently the limits are stored on the key; the
/// actual sliding-window counters live in a future stats module that can be
/// mounted here without changing the request path.
pub fn rate_limit_check(key: &ClientKeyAuth, rpm: u64, tpm: u64) -> Option<String> {
    if let Some(limit) = key.rpm_limit {
        if rpm >= limit as u64 {
            return Some(format!(
                "Client key '{}' exceeded RPM limit ({})",
                key.fingerprint(),
                limit
            ));
        }
    }
    if let Some(limit) = key.tpm_limit {
        if tpm >= limit as u64 {
            return Some(format!(
                "Client key '{}' exceeded TPM limit ({})",
                key.fingerprint(),
                limit
            ));
        }
    }
    None
}

/// Extension hook — permission scope. Returns `Some(message)` when the key is
/// not allowed to use the requested protocol or model.
pub fn permission_scope_check(
    key: &ClientKeyAuth,
    protocol: Option<&str>,
    model: Option<&str>,
) -> Option<String> {
    if let Some(raw) = &key.allowed_protocols {
        if let Ok(list) = serde_json::from_str::<Vec<String>>(raw) {
            if !list.is_empty() {
                if let Some(p) = protocol {
                    if !list.iter().any(|item| item == p) {
                        return Some(format!(
                            "Client key '{}' is not allowed to use protocol '{}'",
                            key.fingerprint(),
                            p
                        ));
                    }
                }
            }
        }
    }
    if let Some(raw) = &key.allowed_models {
        if let Ok(list) = serde_json::from_str::<Vec<String>>(raw) {
            if !list.is_empty() {
                if let Some(m) = model {
                    if !list.iter().any(|item| item == m) {
                        return Some(format!(
                            "Client key '{}' is not allowed to use model '{}'",
                            key.fingerprint(),
                            m
                        ));
                    }
                }
            }
        }
    }
    None
}

impl ClientKeyAuth {
    /// Admin identity (authenticated with the gateway access key).
    pub fn admin(_state: &ProxyState) -> ClientKeyAuth {
        ClientKeyAuth {
            key_id: "admin".to_string(),
            name: "gateway-admin".to_string(),
            key_last_four: "****".to_string(),
            pool_ids: Vec::new(),
            is_admin: true,
            rpm_limit: None,
            tpm_limit: None,
            allowed_protocols: None,
            allowed_models: None,
        }
    }
}

/// Handler extractor: reads the auth context injected by `auth_middleware`.
/// Use as the first argument of a handler alongside `State`:
/// `auth: AuthContext, headers: HeaderMap, body: Bytes`.
#[derive(Clone, Debug, Default)]
pub struct AuthContext(pub Option<ClientKeyAuth>);

impl AuthContext {
    pub fn client_key(&self) -> Option<&ClientKeyAuth> {
        self.0.as_ref()
    }
}

#[async_trait]
impl<S> axum::extract::FromRequestParts<S> for AuthContext
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        Ok(AuthContext(
            parts.extensions.get::<ClientKeyAuth>().cloned(),
        ))
    }
}

/// Decide which route pool a request should use.
///
/// * `explicit_group` — from `X-Group-Id` / `X-Pool-Group` headers (may be
///   `None` when the client did not send one).
/// * `client_key` — resolved virtual key context, if any.
///
/// Returns `Err(Response)` for a clear authorization error (403) when a
/// non-admin client key tries to target a pool it is not bound to.
pub fn resolve_routing_group(
    explicit_group: Option<String>,
    client_key: Option<&ClientKeyAuth>,
) -> Result<String, Response> {
    // 1. Explicit header — admin may use any pool; a client key is restricted
    //    to its bound pools (X-Group-Id is debugging-only, never an escape).
    if let Some(group) = explicit_group {
        if !group.is_empty() {
            match client_key {
                Some(key) if key.is_admin => return Ok(group),
                Some(key) => {
                    if key.pool_ids.iter().any(|id| id == &group) {
                        return Ok(group);
                    }
                    return Err(auth_error(
                        StatusCode::FORBIDDEN,
                        format!(
                            "Client key '{}' is not bound to route pool '{}'",
                            key.fingerprint(),
                            group
                        ),
                    ));
                }
                None => return Ok(group),
            }
        }
    }

    // 2. Normal flow: first pool bound to the virtual client key.
    if let Some(key) = client_key {
        if let Some(first) = key.pool_ids.first() {
            return Ok(first.clone());
        }
        // Bound to no pool → default (full account pool), legacy behavior.
        return Ok("default".to_string());
    }

    // 3. No key / no header → default (full account pool).
    Ok("default".to_string())
}

/// Read the optional X-Group-Id / X-Pool-Group header value.
pub fn explicit_group_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-group-id")
        .or_else(|| headers.get("x-pool-group"))
        .and_then(|val| val.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Standard JSON error response for auth/routing failures.
pub fn auth_error(status: StatusCode, message: impl std::fmt::Display) -> Response {
    let message = message.to_string();
    let body = serde_json::json!({
        "error": {
            "message": message.clone(),
            "type": "authentication_error",
            "code": status.as_u16(),
        }
    });
    let mut response = Json(body).into_response();
    *response.status_mut() = status;
    response
        .extensions_mut()
        .insert(crate::proxy::server::GatewayErrorDetail(message));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(pool_ids: Vec<String>, is_admin: bool) -> ClientKeyAuth {
        ClientKeyAuth {
            key_id: "ck-1".into(),
            name: "test".into(),
            key_last_four: "abcd".into(),
            pool_ids,
            is_admin,
            rpm_limit: None,
            tpm_limit: None,
            allowed_protocols: None,
            allowed_models: None,
        }
    }

    #[test]
    fn admin_can_target_any_explicit_group() {
        let explicit = Some("pool-x".to_string());
        let r = resolve_routing_group(explicit, Some(&key(vec!["pool-a".into()], true)));
        assert_eq!(r.unwrap(), "pool-x");
    }

    #[test]
    fn client_key_restricted_to_bound_pools() {
        let k = key(vec!["pool-a".into(), "pool-b".into()], false);
        assert_eq!(
            resolve_routing_group(Some("pool-b".into()), Some(&k)).unwrap(),
            "pool-b"
        );
        assert!(resolve_routing_group(Some("pool-x".into()), Some(&k)).is_err());
    }

    #[test]
    fn client_key_without_header_uses_first_bound_pool() {
        let k = key(vec!["pool-a".into()], false);
        assert_eq!(resolve_routing_group(None, Some(&k)).unwrap(), "pool-a");
    }

    #[test]
    fn no_key_falls_back_to_default() {
        assert_eq!(resolve_routing_group(None, None).unwrap(), "default");
        assert_eq!(
            resolve_routing_group(Some("pool-z".into()), None).unwrap(),
            "pool-z"
        );
    }

    #[test]
    fn empty_header_ignored() {
        let k = key(vec!["pool-a".into()], false);
        assert_eq!(
            resolve_routing_group(Some(String::new()), Some(&k)).unwrap(),
            "pool-a"
        );
    }

    #[test]
    fn rate_limit_hook_blocks_at_limit() {
        let mut k = key(vec![], false);
        k.rpm_limit = Some(5);
        assert!(rate_limit_check(&k, 4, 0).is_none());
        assert!(rate_limit_check(&k, 5, 0).is_some());
    }

    #[test]
    fn permission_scope_hook_filters_protocol_and_model() {
        let mut k = key(vec![], false);
        k.allowed_protocols = Some(r#"["anthropic"]"#.into());
        assert!(permission_scope_check(&k, Some("anthropic"), None).is_none());
        assert!(permission_scope_check(&k, Some("chat"), None).is_some());

        k.allowed_protocols = None;
        k.allowed_models = Some(r#"["claude-sonnet-4"]"#.into());
        assert!(permission_scope_check(&k, None, Some("claude-sonnet-4")).is_none());
        assert!(permission_scope_check(&k, None, Some("gpt-4o")).is_some());
    }
}
