//! Provider CRUD operations

use rusqlite::Connection;
use std::sync::Mutex;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Provider {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub provider_type: String,
    pub base_url: String,
    /// JSON object mapping canonical protocols to their upstream Base URL.
    /// Legacy rows keep this empty and transparently fall back to `base_url`.
    pub base_urls: Option<String>,
    pub protocol: String,
    pub protocols: Option<String>,
    pub route_takeover: Option<i64>,
    pub api_keys: Option<String>,
    pub models: Option<String>,
    pub proxy_url: Option<String>,
    /// JSON object containing additional HTTP headers for this upstream.
    pub custom_headers: Option<String>,
    pub timeout_ms: Option<i64>,
    pub priority: Option<i64>,
    pub enabled: Option<bool>,
    pub created_at: Option<String>,
    /// How the provider authenticates with its upstream.
    /// Values: api_key, oauth_pkce, oauth_device_flow, pat_to_token, api_key_or_oauth.
    pub auth_mode: Option<String>,
    /// JSON blob with OAuth configuration (authorize_url, token_url, client_id,
    /// scope, redirect_port, etc.). Stored as raw JSON string.
    pub oauth_config: Option<String>,
}

impl Provider {
    /// Resolve the upstream Base URL for a concrete canonical protocol.
    /// Invalid/missing mappings never block legacy providers: `base_url` remains
    /// the compatibility fallback until all existing rows have been edited.
    pub fn base_url_for_protocol(&self, protocol: &str) -> String {
        let normalized = protocol.trim().to_lowercase();
        let canonical = match normalized.as_str() {
            "openai" | "chat_completions" | "openai_chat" => "chat",
            "messages" | "anthropic_messages" | "claude" => "anthropic",
            "google" | "google_gemini" => "gemini",
            "response" | "openai_responses" | "codex" => "responses",
            value => value,
        };
        self.base_urls
            .as_deref()
            .and_then(|raw| {
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(raw).ok()
            })
            .and_then(|urls| {
                urls.get(canonical)
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| self.base_url.clone())
    }

    pub fn custom_header_pairs(&self) -> Result<Vec<(String, String)>, String> {
        let Some(raw) = self.custom_headers.as_deref().map(str::trim).filter(|v| !v.is_empty()) else {
            return Ok(Vec::new());
        };
        let object = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(raw)
            .map_err(|e| format!("自定义请求头不是有效 JSON 对象: {}", e))?;
        let blocked = [
            "authorization",
            "proxy-authorization",
            "host",
            "content-length",
            "transfer-encoding",
            "connection",
        ];
        let mut headers = Vec::with_capacity(object.len());
        for (name, value) in object {
            let normalized = name.trim().to_ascii_lowercase();
            if normalized.is_empty() || blocked.contains(&normalized.as_str()) {
                return Err(format!("不允许设置请求头: {}", name));
            }
            let value = value
                .as_str()
                .ok_or_else(|| format!("请求头 {} 的值必须是字符串", name))?
                .trim();
            reqwest::header::HeaderName::from_bytes(name.trim().as_bytes())
                .map_err(|_| format!("无效的请求头名称: {}", name))?;
            reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| format!("请求头 {} 包含无效字符", name))?;
            headers.push((name.trim().to_string(), value.to_string()));
        }
        Ok(headers)
    }

    pub fn apply_custom_headers(
        &self,
        mut request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, String> {
        for (name, value) in self.custom_header_pairs()? {
            request = request.header(name, value);
        }
        Ok(request)
    }
}

pub struct ProviderRepo;

impl ProviderRepo {
    pub fn list_all(&self, conn: &Mutex<Connection>) -> Result<Vec<Provider>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, type, base_url, base_urls, protocol, protocols, route_takeover, api_keys, models, proxy_url, custom_headers, timeout_ms, priority, enabled, created_at, auth_mode, oauth_config FROM providers ORDER BY priority, name",
            )
            .map_err(|e| e.to_string())?;

        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        let mut providers = Vec::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            providers.push(Provider {
                id: row.get(0).map_err(|e| e.to_string())?,
                name: row.get(1).map_err(|e| e.to_string())?,
                provider_type: row.get(2).map_err(|e| e.to_string())?,
                base_url: row.get(3).map_err(|e| e.to_string())?,
                base_urls: row.get(4).map_err(|e| e.to_string())?,
                protocol: row.get(5).map_err(|e| e.to_string())?,
                protocols: row.get(6).map_err(|e| e.to_string())?,
                route_takeover: row.get(7).map_err(|e| e.to_string())?,
                api_keys: row.get(8).map_err(|e| e.to_string())?,
                models: row.get(9).map_err(|e| e.to_string())?,
                proxy_url: row.get(10).map_err(|e| e.to_string())?,
                custom_headers: row.get(11).map_err(|e| e.to_string())?,
                timeout_ms: row.get(12).map_err(|e| e.to_string())?,
                priority: row.get(13).map_err(|e| e.to_string())?,
                enabled: row.get(14).map_err(|e| e.to_string())?,
                created_at: row.get(15).map_err(|e| e.to_string())?,
                auth_mode: row.get(16).map_err(|e| e.to_string())?,
                oauth_config: row.get(17).map_err(|e| e.to_string())?,
            });
        }
        Ok(providers)
    }

    pub fn get_by_id(
        &self,
        conn: &Mutex<Connection>,
        id: &str,
    ) -> Result<Option<Provider>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, type, base_url, base_urls, protocol, protocols, route_takeover, api_keys, models, proxy_url, custom_headers, timeout_ms, priority, enabled, created_at, auth_mode, oauth_config FROM providers WHERE id=?1",
            )
            .map_err(|e| e.to_string())?;

        let mut rows = stmt
            .query(rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        match rows.next().map_err(|e| e.to_string())? {
            Some(row) => Ok(Some(Provider {
                id: row.get(0).map_err(|e| e.to_string())?,
                name: row.get(1).map_err(|e| e.to_string())?,
                provider_type: row.get(2).map_err(|e| e.to_string())?,
                base_url: row.get(3).map_err(|e| e.to_string())?,
                base_urls: row.get(4).map_err(|e| e.to_string())?,
                protocol: row.get(5).map_err(|e| e.to_string())?,
                protocols: row.get(6).map_err(|e| e.to_string())?,
                route_takeover: row.get(7).map_err(|e| e.to_string())?,
                api_keys: row.get(8).map_err(|e| e.to_string())?,
                models: row.get(9).map_err(|e| e.to_string())?,
                proxy_url: row.get(10).map_err(|e| e.to_string())?,
                custom_headers: row.get(11).map_err(|e| e.to_string())?,
                timeout_ms: row.get(12).map_err(|e| e.to_string())?,
                priority: row.get(13).map_err(|e| e.to_string())?,
                enabled: row.get(14).map_err(|e| e.to_string())?,
                created_at: row.get(15).map_err(|e| e.to_string())?,
                auth_mode: row.get(16).map_err(|e| e.to_string())?,
                oauth_config: row.get(17).map_err(|e| e.to_string())?,
            })),
            None => Ok(None),
        }
    }

    pub fn create(&self, conn: &Mutex<Connection>, provider: &Provider) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO providers (id, name, type, base_url, base_urls, protocol, protocols, route_takeover, api_keys, models, proxy_url, custom_headers, timeout_ms, priority, enabled, auth_mode, oauth_config) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            rusqlite::params![
                provider.id, provider.name, provider.provider_type, provider.base_url,
                provider.base_urls, provider.protocol, provider.protocols, provider.route_takeover,
                provider.api_keys, provider.models, provider.proxy_url, provider.custom_headers,
                provider.timeout_ms, provider.priority, provider.enabled, provider.auth_mode,
                provider.oauth_config
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update(&self, conn: &Mutex<Connection>, provider: &Provider) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE providers SET name=?1, type=?2, base_url=?3, base_urls=?4, protocol=?5, protocols=?6, route_takeover=?7, \
             api_keys=?8, models=?9, proxy_url=?10, custom_headers=?11, timeout_ms=?12, priority=?13, enabled=?14, auth_mode=?15, oauth_config=?16 WHERE id=?17",
            rusqlite::params![
                provider.name,
                provider.provider_type,
                provider.base_url,
                provider.base_urls,
                provider.protocol,
                provider.protocols,
                provider.route_takeover,
                provider.api_keys,
                provider.models,
                provider.proxy_url,
                provider.custom_headers,
                provider.timeout_ms,
                provider.priority,
                provider.enabled,
                provider.auth_mode,
                provider.oauth_config,
                provider.id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete(&self, conn: &Mutex<Connection>, id: &str) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM providers WHERE id=?1", rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Provider;

    fn provider(base_urls: Option<&str>) -> Provider {
        Provider {
            id: "prov_test".into(),
            name: "Test".into(),
            provider_type: "chat".into(),
            base_url: "https://legacy.example.com".into(),
            base_urls: base_urls.map(str::to_string),
            protocol: "chat".into(),
            protocols: Some("[\"chat\",\"anthropic\"]".into()),
            route_takeover: Some(1),
            api_keys: None,
            models: None,
            proxy_url: None,
            custom_headers: None,
            timeout_ms: Some(30_000),
            priority: Some(0),
            enabled: Some(true),
            created_at: None,
            auth_mode: None,
            oauth_config: None,
        }
    }

    #[test]
    fn resolves_protocol_specific_url_and_canonical_alias() {
        let provider = provider(Some(
            r#"{"chat":"https://chat.example.com/v1","anthropic":"https://anthropic.example.com"}"#,
        ));
        assert_eq!(
            provider.base_url_for_protocol("openai"),
            "https://chat.example.com/v1"
        );
        assert_eq!(
            provider.base_url_for_protocol("anthropic"),
            "https://anthropic.example.com"
        );
    }

    #[test]
    fn validates_custom_headers_and_blocks_security_headers() {
        let mut value = provider(None);
        value.custom_headers = Some(r#"{"X-Title":"PoolGate","HTTP-Referer":"https://example.com"}"#.into());
        assert_eq!(value.custom_header_pairs().unwrap().len(), 2);

        value.custom_headers = Some(r#"{"Authorization":"Bearer override"}"#.into());
        assert!(value.custom_header_pairs().is_err());

        value.custom_headers = Some(r#"{"X-Count":1}"#.into());
        assert!(value.custom_header_pairs().is_err());
    }

    #[test]
    fn falls_back_to_legacy_url_for_missing_or_invalid_mapping() {
        assert_eq!(
            provider(Some("not-json")).base_url_for_protocol("chat"),
            "https://legacy.example.com"
        );
        assert_eq!(
            provider(Some(r#"{"chat":""}"#)).base_url_for_protocol("chat"),
            "https://legacy.example.com"
        );
    }
}
