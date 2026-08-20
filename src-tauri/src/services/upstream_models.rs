//! Fetch and persist model catalogs from upstream providers.

use crate::db::accounts::Account;
use crate::db::providers::Provider;
use crate::services::credentials::{auth_credential, AuthCredential};
use crate::AppState;
use futures::stream::{self, StreamExt};
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelRefreshResult {
    pub account_id: String,
    pub success: bool,
    pub message: String,
    pub models: Vec<String>,
}

/// Build the model-list endpoint for a Base URL.
///
/// Most OpenAI-compatible providers expose `/v1/models`, while some official
/// endpoints use `/models`. A complete model-list URL supplied by a connector
/// template is preserved here.
pub fn models_endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/models") {
        base.to_string()
    } else if base.ends_with("/v1") || base.ends_with("/v1beta") || base.ends_with("/v1alpha") {
        format!("{}/models", base)
    } else if base.ends_with("/responses") {
        format!("{}/models", base.trim_end_matches("/responses"))
    } else {
        format!("{}/v1/models", base)
    }
}

#[derive(Debug, Deserialize)]
struct ModelItem {
    id: Option<String>,
    name: Option<String>,
    slug: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelList {
    data: Option<Vec<ModelItem>>,
    models: Option<Vec<ModelItem>>,
}

fn canonical_protocol(protocol: &str) -> &str {
    match protocol.trim().to_ascii_lowercase().as_str() {
        "openai" | "chat_completions" | "openai_chat" => "chat",
        "messages" | "anthropic_messages" | "claude" => "anthropic",
        "google" | "google_gemini" => "gemini",
        "response" | "openai_responses" | "codex" => "responses",
        _ => protocol,
    }
}

fn authorize_request(
    request: RequestBuilder,
    credential: &AuthCredential,
    protocol: Option<&str>,
) -> RequestBuilder {
    let protocol = canonical_protocol(protocol.unwrap_or("chat"));
    match credential {
        AuthCredential::Bearer(token) => {
            let request = request.bearer_auth(token);
            if protocol == "anthropic" {
                request.header("anthropic-version", "2023-06-01")
            } else {
                request
            }
        }
        AuthCredential::ApiKey(key) => match protocol {
            "anthropic" => request
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01"),
            "gemini" => request.query(&[("key", key)]),
            _ => request.bearer_auth(key),
        },
    }
}

fn normalize_model_id(raw: String) -> Option<String> {
    let model = raw.trim().trim_start_matches("models/").trim().to_string();
    (!model.is_empty()).then_some(model)
}

fn parse_model_list(value: Value) -> Result<Vec<String>, String> {
    let list: ModelList =
        serde_json::from_value(value).map_err(|e| format!("解析响应失败: {}", e))?;
    let models = list.data.or(list.models).unwrap_or_default();
    let mut unique = BTreeSet::new();
    for item in models {
        if let Some(model) = item
            .slug
            .or(item.id)
            .or(item.name)
            .and_then(normalize_model_id)
        {
            unique.insert(model);
        }
    }
    Ok(unique.into_iter().collect())
}

async fn send_model_request(request: RequestBuilder) -> Result<Vec<String>, String> {
    let resp = request
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("上游返回 {}: {}", status, body));
    }
    let value: Value = serde_json::from_str(&body).map_err(|e| format!("解析响应失败: {}", e))?;
    parse_model_list(value)
}

fn client() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("PoolGate/1.0")
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// Fetch model IDs from an upstream model-list endpoint for import preview.
pub async fn fetch_upstream_models(
    base_url: &str,
    api_key: Option<&str>,
    protocol: Option<&str>,
) -> Result<Vec<String>, String> {
    if base_url.trim().is_empty() {
        return Err("Base URL 不能为空".into());
    }
    let mut request = client().get(models_endpoint(base_url));
    if let Some(key) = api_key.filter(|key| !key.trim().is_empty()) {
        request = authorize_request(request, &AuthCredential::ApiKey(key.to_string()), protocol);
    }
    send_model_request(request).await
}

async fn fetch_models_for_account(
    account: &Account,
    provider: &Provider,
) -> Result<Vec<String>, String> {
    let credential = auth_credential(account)?;
    if crate::services::codex_adapter::is_codex_oauth(account, provider) {
        let context = crate::services::codex_adapter::request_context(account)?;
        let request = client().get(crate::services::codex_adapter::codex_models_url());
        let request = crate::services::codex_adapter::apply_json_headers(request, &context);
        return send_model_request(request).await;
    }

    // Antigravity (Cloud Code) has no /v1/models; the catalog comes from the
    // private v1internal `fetchAvailableModels` endpoint.
    if crate::services::antigravity_adapter::is_antigravity_account(account, provider) {
        let payload = crate::services::credentials::payload_for_account(account)?;
        let access_token = payload
            .access_token
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| "Antigravity account has no access token".to_string())?;
        let project_id = payload
            .metadata
            .as_ref()
            .and_then(|meta| meta.get("project_id"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let models = crate::services::antigravity_adapter::fetch_models(
            &access_token,
            project_id.as_deref(),
        )
        .await?;
        let ids = models
            .into_iter()
            .filter_map(|model| model.get("id").and_then(|v| v.as_str()).map(String::from))
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Err("fetchAvailableModels 未返回可用模型".into());
        }
        return Ok(ids);
    }

    let protocol = canonical_protocol(&provider.protocol);
    let base_url = provider.base_url_for_protocol(protocol);
    if base_url.trim().is_empty() {
        return Err("上游连接器 Base URL 为空".into());
    }
    let request = authorize_request(
        client().get(models_endpoint(&base_url)),
        &credential,
        Some(protocol),
    );
    send_model_request(request).await
}

pub async fn refresh_account_models(
    state: Arc<AppState>,
    account_id: String,
) -> ModelRefreshResult {
    let result = refresh_account_models_inner(&state, &account_id).await;
    match result {
        Ok(models) => ModelRefreshResult {
            account_id,
            success: true,
            message: format!("已发现 {} 个模型", models.len()),
            models,
        },
        Err(message) => ModelRefreshResult {
            account_id,
            success: false,
            message,
            models: Vec::new(),
        },
    }
}

async fn refresh_account_models_inner(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<Vec<String>, String> {
    let account = state
        .db
        .accounts
        .get_by_id(&state.db.conn, account_id)?
        .ok_or_else(|| "账号不存在".to_string())?;
    let provider_id = account
        .provider_id
        .as_deref()
        .ok_or_else(|| "账号没有关联上游连接器".to_string())?;
    let provider = state
        .db
        .providers
        .get_by_id(&state.db.conn, provider_id)?
        .ok_or_else(|| "上游连接器不存在".to_string())?;
    let models = fetch_models_for_account(&account, &provider).await?;
    if models.is_empty() {
        return Err("上游未返回可用模型，请检查账号权限或客户端版本".into());
    }
    state
        .db
        .accounts
        .update_models(&state.db.conn, account_id, &models)?;
    Ok(models)
}

pub async fn batch_refresh_account_models(
    state: Arc<AppState>,
    account_ids: Vec<String>,
) -> Vec<ModelRefreshResult> {
    stream::iter(account_ids)
        .map(|account_id| {
            let state = state.clone();
            async move { refresh_account_models(state, account_id).await }
        })
        .buffer_unordered(5)
        .collect()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_models_endpoint() {
        assert_eq!(
            models_endpoint("https://api.openai.com"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            models_endpoint("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            models_endpoint("https://chatgpt.com/backend-api/codex/responses"),
            "https://chatgpt.com/backend-api/codex/models"
        );
        assert_eq!(
            models_endpoint("https://generativelanguage.googleapis.com/v1beta"),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
        assert_eq!(
            models_endpoint("https://api.deepseek.com/models"),
            "https://api.deepseek.com/models"
        );
    }

    #[test]
    fn parses_openai_gemini_and_codex_model_lists() {
        let openai = serde_json::json!({
            "data": [{"id":"deepseek-v4-pro"}, {"id":"deepseek-v4-flash"}]
        });
        assert_eq!(parse_model_list(openai).unwrap().len(), 2);

        let gemini = serde_json::json!({"models":[{"name":"models/gemini-2.5-pro"}]});
        assert_eq!(
            parse_model_list(gemini).unwrap(),
            vec!["gemini-2.5-pro".to_string()]
        );

        let codex = serde_json::json!({
            "models": [{"slug":"gpt-5.3-codex"}, {"slug":"gpt-5.2-codex"}]
        });
        assert_eq!(
            parse_model_list(codex).unwrap(),
            vec!["gpt-5.2-codex".to_string(), "gpt-5.3-codex".to_string()]
        );
    }
}
