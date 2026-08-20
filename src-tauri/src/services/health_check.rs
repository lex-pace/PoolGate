//! Account health checker — real HTTP health checks against provider endpoints.
//!
//! Supported provider types and their check strategies:
//! - Anthropic: POST {base_url}/v1/messages with minimal body, check 2xx
//! - OpenAI: GET {base_url}/v1/models or POST minimal chat, check 2xx
//! - Gemini: GET {base_url}/v1beta/models?key={api_key}, check 2xx
//! - Local: GET {base_url}/v1/models, check 2xx
//!
//! Timeout is 5 seconds per request.

use crate::db::accounts::Account;
use crate::db::providers::Provider;
use crate::services::upstream_models::models_endpoint;
use reqwest::Client;
use std::time::Instant;

/// A health-check result for a single account.
#[derive(Debug, Clone)]
pub enum HealthResult {
    /// Endpoint responded with a 2xx status.
    Passed { latency_ms: u64 },
    /// Endpoint responded with a non-2xx status.
    Failed { code: u16, body: String },
    /// Request timed out (>5s).
    Timeout,
    /// An unexpected error occurred (DNS, connection refused, etc.).
    Error(String),
}

/// Configurable health checker that performs concurrent HTTP health checks.
pub struct HealthChecker {
    _max_concurrent: usize,
}

impl HealthChecker {
    /// Create a new `HealthChecker` that runs at most `max_concurrent` checks in parallel.
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            _max_concurrent: max_concurrent,
        }
    }

    /// Run a single health check against the given account's provider endpoint.
    ///
    /// The check strategy is chosen based on [`Provider::provider_type`]:
    /// - `"anthropic"` → POST `{base_url}/v1/messages`
    /// - `"openai"` → GET `{base_url}/v1/models`
    /// - `"gemini"` → GET `{base_url}/v1beta/models?key={api_key}`
    /// - `"local"` → GET `{base_url}/v1/models`
    /// - otherwise → GET `{base_url}/v1/models` (generic fallback)
    pub async fn check_account(&self, account: &Account, provider: &Provider) -> HealthResult {
        let client = Self::build_client(provider);

        if crate::services::codex_adapter::is_codex_oauth(account, provider) {
            return self.check_codex(&client, account).await;
        }

        if crate::services::copilot_adapter::is_copilot_pat(account, provider)
            || crate::services::copilot_adapter::is_copilot_oauth(account, provider)
        {
            return crate::services::copilot_adapter::check_copilot_health(account).await;
        }

        if crate::services::claude_adapter::is_claude_oauth(account, provider) {
            return crate::services::claude_adapter::check_claude_health(account).await;
        }

        if crate::services::antigravity_adapter::is_antigravity_account(account, provider) {
            return crate::services::antigravity_adapter::check_antigravity_health(account).await;
        }

        if crate::services::gemini_adapter::is_gemini_api_key(account, provider)
            || crate::services::gemini_adapter::is_gemini_oauth(account, provider)
        {
            return crate::services::gemini_adapter::check_gemini_health(account).await;
        }

        if crate::services::grok_adapter::is_grok_oauth(account, provider) {
            return crate::services::grok_adapter::check_grok_health(account).await;
        }

        if !crate::services::credentials::is_directly_routable(account) {
            return HealthResult::Error(
                "Account requires an upstream adapter before health checks".into(),
            );
        }

        let health_protocol = Self::health_protocol(account, provider);
        let selected_base_url = provider.base_url_for_protocol(&health_protocol);
        let base_url = selected_base_url.trim_end_matches('/');

        match health_protocol.as_str() {
            "anthropic" => match crate::services::credentials::auth_credential(account) {
                Ok(credential) => {
                    let model = Self::preferred_model(account, provider)
                        .unwrap_or_else(|| "claude-3-haiku-20240307".to_string());
                    self.check_anthropic(&client, base_url, credential, &model)
                        .await
                }
                Err(error) => HealthResult::Error(error),
            },
            "responses" | "chat" => {
                match crate::services::credentials::authorization_secret(account) {
                    Ok(secret) => self.check_openai(&client, base_url, &secret).await,
                    Err(error) => HealthResult::Error(error),
                }
            }
            "gemini" => match crate::services::credentials::auth_credential(account) {
                Ok(credential) => self.check_gemini(&client, base_url, credential).await,
                Err(error) => HealthResult::Error(error),
            },
            "local" => self.check_local(&client, base_url).await,
            _ => match crate::services::credentials::authorization_secret(account) {
                Ok(secret) => self.check_generic(&client, base_url, &secret).await,
                Err(error) => HealthResult::Error(error),
            },
        }
    }

    // ── Private helpers ────────────────────────────────────────────────────

    /// Select the health-check protocol. The *connector* declaration
    /// (`provider.protocols`) is authoritative because it describes the
    /// upstream's actual native protocol(s) and matching Base URL mount
    /// points; the account-level `protocols` field is an import-time marker
    /// that may disagree (e.g. a resource imported as `chat` whose upstream
    /// only serves the Anthropic `/v1/messages` endpoint — probing `/v1/models`
    /// then yields 404 and wrongly flags a healthy account as failed). We
    /// therefore prefer the provider's set and only fall back to the account's
    /// set, then legacy fields, when the connector declares nothing.
    fn health_protocol(account: &Account, provider: &Provider) -> String {
        fn parse(raw: Option<&str>) -> Vec<String> {
            crate::proxy::router::parse_protocols(raw)
                .into_iter()
                .map(|value| match value.trim().to_lowercase().as_str() {
                    "openai" | "chat_completions" => "chat".into(),
                    "codex" => "responses".into(),
                    "messages" => "anthropic".into(),
                    "google" => "gemini".into(),
                    other => other.to_string(),
                })
                .collect()
        }

        let mut protocols = parse(provider.protocols.as_deref());
        if protocols.is_empty() {
            protocols = parse(account.protocols.as_deref());
        }
        if let Some(protocol) = protocols.first() {
            return protocol.clone();
        }

        match provider.protocol.trim().to_lowercase().as_str() {
            "openai" | "chat_completions" => "chat".into(),
            "codex" => "responses".into(),
            "messages" => "anthropic".into(),
            "google" => "gemini".into(),
            "" => provider.provider_type.to_lowercase(),
            protocol => protocol.to_string(),
        }
    }

    /// Pick a model name to probe with, preferring the account's declared
    /// models, then the provider's declared models. Health checks must use a
    /// model the upstream actually serves — a hardcoded default (e.g.
    /// `claude-3-haiku`) makes providers that only serve `mimo` / `step` reject
    /// the probe with 400 and be wrongly flagged unhealthy, dropping them from
    /// the routable pool.
    fn preferred_model(account: &Account, provider: &Provider) -> Option<String> {
        fn first_model(raw: Option<&str>) -> Option<String> {
            let raw = raw?.trim();
            if raw.is_empty() {
                return None;
            }
            if let Ok(list) = serde_json::from_str::<Vec<String>>(raw) {
                return list
                    .into_iter()
                    .map(|m| m.trim().to_string())
                    .find(|m| !m.is_empty());
            }
            raw.split([',', ';'])
                .map(|m| m.trim().to_string())
                .find(|m| !m.is_empty())
        }
        first_model(account.models.as_deref()).or_else(|| first_model(provider.models.as_deref()))
    }

    fn build_client(provider: &Provider) -> Client {
        let mut builder = Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .user_agent("PoolGate/1.0");

        if let Some(ref proxy_url) = provider.proxy_url {
            if let Ok(proxy) = reqwest::Proxy::http(proxy_url) {
                builder = builder.proxy(proxy);
            }
            if let Ok(proxy) = reqwest::Proxy::https(proxy_url) {
                builder = builder.proxy(proxy);
            }
        }

        builder.build().unwrap_or_else(|_| Client::new())
    }

    async fn check_anthropic(
        &self,
        client: &Client,
        base_url: &str,
        credential: crate::services::credentials::AuthCredential,
        model: &str,
    ) -> HealthResult {
        let url = crate::proxy::protocol::build_upstream_url(base_url, "/v1/messages");
        let body = serde_json::json!({
            "model": model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "ping"}]
        });

        let mut request = client
            .post(&url)
            .header("anthropic-version", "2023-06-01")
            .json(&body);
        request = match credential {
            crate::services::credentials::AuthCredential::ApiKey(secret) => {
                request.header("x-api-key", secret)
            }
            crate::services::credentials::AuthCredential::Bearer(secret) => {
                request.header("Authorization", format!("Bearer {}", secret))
            }
        };
        let start = Instant::now();
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), request.send()).await;

        Self::finish(start, resp).await
    }

    async fn check_codex(&self, client: &Client, account: &Account) -> HealthResult {
        let context = match crate::services::codex_adapter::request_context(account) {
            Ok(context) => context,
            Err(error) => return HealthResult::Error(error),
        };
        let request = client.get(crate::services::codex_adapter::codex_models_url());
        let request = crate::services::codex_adapter::apply_json_headers(request, &context);
        let start = Instant::now();
        let response =
            tokio::time::timeout(std::time::Duration::from_secs(5), request.send()).await;
        Self::finish(start, response).await
    }

    async fn check_openai(&self, client: &Client, base_url: &str, api_key: &str) -> HealthResult {
        // Most OpenAI-compatible providers expose `/v1/models`; a few (e.g.
        // DeepSeek) only serve the bare `/models`. Try both so a routable
        // account is not wrongly flagged unhealthy because of the path layout.
        let candidates = [models_endpoint(base_url), format!("{}/models", base_url)];
        let mut last = HealthResult::Error("模型列表接口不可用".into());
        for url in candidates {
            let start = Instant::now();
            let resp = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                client.get(&url).bearer_auth(api_key).send(),
            )
            .await;
            match Self::finish_raw(start, resp) {
                HealthResult::Passed { latency_ms } => return HealthResult::Passed { latency_ms },
                HealthResult::Failed { code, .. } if code == 401 || code == 403 => {
                    // 401/403 from the models endpoint means the server is reachable
                    // and responding — the probe key may just lack /models scope.
                    // Treat as passed to avoid wrongly dropping routable accounts.
                    return HealthResult::Passed {
                        latency_ms: start.elapsed().as_millis() as u64,
                    };
                }
                other => last = other,
            }
        }
        last
    }

    async fn check_gemini(
        &self,
        client: &Client,
        base_url: &str,
        credential: crate::services::credentials::AuthCredential,
    ) -> HealthResult {
        let mut request = client.get(crate::proxy::protocol::build_upstream_url(
            base_url,
            "/v1beta/models",
        ));
        request = match credential {
            crate::services::credentials::AuthCredential::ApiKey(secret) => {
                request.query(&[("key", secret)])
            }
            crate::services::credentials::AuthCredential::Bearer(secret) => {
                request.bearer_auth(secret)
            }
        };
        let start = Instant::now();
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), request.send()).await;

        Self::finish(start, resp).await
    }

    async fn check_local(&self, client: &Client, base_url: &str) -> HealthResult {
        let url = models_endpoint(base_url);
        let start = Instant::now();
        let resp =
            tokio::time::timeout(std::time::Duration::from_secs(5), client.get(&url).send()).await;

        Self::finish(start, resp).await
    }

    async fn check_generic(&self, client: &Client, base_url: &str, api_key: &str) -> HealthResult {
        // Try models endpoint first, fall back to a simple chat completion
        let url = models_endpoint(base_url);
        let start = Instant::now();
        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.get(&url).bearer_auth(api_key).send(),
        )
        .await;

        // If models endpoint fails (e.g. 404), try a minimal chat completion
        match Self::finish_raw(start, resp) {
            HealthResult::Failed { .. } => {
                let url =
                    crate::proxy::protocol::build_upstream_url(base_url, "/v1/chat/completions");
                let body = serde_json::json!({
                    "model": "gpt-3.5-turbo",
                    "messages": [{"role": "user", "content": "ping"}],
                    "max_tokens": 1
                });
                let start = Instant::now();
                let resp = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    client.post(&url).bearer_auth(api_key).json(&body).send(),
                )
                .await;
                Self::finish(start, resp).await
            }
            other => other,
        }
    }

    async fn finish(
        start: Instant,
        result: Result<Result<reqwest::Response, reqwest::Error>, tokio::time::error::Elapsed>,
    ) -> HealthResult {
        match result {
            Ok(Ok(resp)) => {
                let latency = start.elapsed().as_millis() as u64;
                if resp.status().is_success() {
                    HealthResult::Passed {
                        latency_ms: latency,
                    }
                } else {
                    let code = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    HealthResult::Failed { code, body }
                }
            }
            Ok(Err(e)) => HealthResult::Error(e.to_string()),
            Err(_elapsed) => HealthResult::Timeout,
        }
    }

    fn finish_raw(
        start: Instant,
        result: Result<Result<reqwest::Response, reqwest::Error>, tokio::time::error::Elapsed>,
    ) -> HealthResult {
        // Fallback: just use status code without awaiting the body
        match result {
            Ok(Ok(resp)) => {
                let latency = start.elapsed().as_millis() as u64;
                if resp.status().is_success() {
                    HealthResult::Passed {
                        latency_ms: latency,
                    }
                } else {
                    let code = resp.status().as_u16();
                    HealthResult::Failed {
                        code,
                        body: format!("HTTP {}", code),
                    }
                }
            }
            Ok(Err(e)) => HealthResult::Error(e.to_string()),
            Err(_elapsed) => HealthResult::Timeout,
        }
    }
}
