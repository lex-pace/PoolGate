use crate::db::accounts::Account;
use crate::db::providers::Provider;
use crate::AppState;
use reqwest::Client;
use std::sync::Arc;
use tauri::State;

#[derive(serde::Serialize)]
pub struct ProviderTestResult {
    pub success: bool,
    pub message: String,
    pub latency_ms: u64,
    pub model_tested: Option<String>,
    pub error_details: Option<String>,
}

#[derive(serde::Serialize)]
pub struct AccountTestResult {
    pub success: bool,
    pub message: String,
    pub latency_ms: u64,
    pub model_tested: Option<String>,
    pub reply: Option<String>,
}

#[tauri::command]
pub fn list_providers(state: State<'_, Arc<AppState>>) -> Result<Vec<Provider>, String> {
    state
        .db
        .providers
        .list_all(&state.db.conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_provider(
    state: State<'_, Arc<AppState>>,
    provider: Provider,
) -> Result<Provider, String> {
    provider.custom_header_pairs()?;
    state
        .db
        .providers
        .create(&state.db.conn, &provider)
        .map_err(|e| e.to_string())?;
    Ok(provider)
}

#[tauri::command]
pub fn update_provider(state: State<'_, Arc<AppState>>, provider: Provider) -> Result<(), String> {
    provider.custom_header_pairs()?;
    state
        .db
        .providers
        .update(&state.db.conn, &provider)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_provider(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    state
        .db
        .providers
        .delete(&state.db.conn, &id)
        .map_err(|e| e.to_string())
}

/// Canonicalize protocol name for routing.
fn canonical_protocol(protocol: &str) -> &str {
    match protocol.trim().to_ascii_lowercase().as_str() {
        "openai" | "chat_completions" | "openai_chat" => "chat",
        "messages" | "anthropic_messages" | "claude" => "anthropic",
        "google" | "google_gemini" => "gemini",
        "response" | "openai_responses" | "codex" => "responses",
        _ => protocol,
    }
}

/// Pick the first model from provider's declared models, or use a safe default.
fn pick_test_model(provider: &Provider) -> String {
    if let Some(ref models_raw) = provider.models {
        if let Ok(list) = serde_json::from_str::<Vec<String>>(models_raw) {
            if let Some(first) = list.into_iter().find(|m| !m.trim().is_empty()) {
                return first;
            }
        }
    }
    // Fallback defaults per protocol
    match canonical_protocol(&provider.protocol) {
        "anthropic" => "claude-3-haiku-20240307".to_string(),
        "gemini" => "gemini-2.0-flash".to_string(),
        _ => "gpt-4o-mini".to_string(),
    }
}

/// Test an OpenAI-compatible chat endpoint with a minimal request.
async fn test_openai_chat(
    client: &Client,
    provider: &Provider,
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<(String, u64), String> {
    let base = base_url.trim_end_matches('/');
    let url = if base.ends_with("/v1") {
        format!("{}/chat/completions", base)
    } else {
        format!("{}/v1/chat/completions", base)
    };

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "Say hello in one sentence."}],
        "max_tokens": 10,
        "stream": false
    });

    let start = std::time::Instant::now();
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        provider
            .apply_custom_headers(client.post(&url).bearer_auth(api_key).json(&body))?
            .send(),
    )
    .await
    .map_err(|_| "请求超时 (15s)".to_string())?
    .map_err(|e| format!("请求失败: {}", e))?;

    let latency_ms = start.elapsed().as_millis() as u64;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status, truncate(&text, 200)));
    }

    // Try to extract the reply text from the response
    let reply =
        extract_chat_reply(&text).unwrap_or_else(|| "(响应解析成功，但未提取到文本)".to_string());
    Ok((reply, latency_ms))
}

/// Test an Anthropic Messages endpoint with a minimal request.
async fn test_anthropic_chat(
    client: &Client,
    provider: &Provider,
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<(String, u64), String> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{}/v1/messages", base);

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 10,
        "messages": [{"role": "user", "content": "Say hello in one sentence."}]
    });

    let start = std::time::Instant::now();
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        provider
            .apply_custom_headers(
                client
                    .post(&url)
                    .header("x-api-key", api_key)
                    .header("anthropic-version", "2023-06-01")
                    .json(&body),
            )?
            .send(),
    )
    .await
    .map_err(|_| "请求超时 (15s)".to_string())?
    .map_err(|e| format!("请求失败: {}", e))?;

    let latency_ms = start.elapsed().as_millis() as u64;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status, truncate(&text, 200)));
    }

    let reply = extract_anthropic_reply(&text)
        .unwrap_or_else(|| "(响应解析成功，但未提取到文本)".to_string());
    Ok((reply, latency_ms))
}

/// Test a Gemini endpoint with a minimal request.
async fn test_gemini_chat(
    client: &Client,
    provider: &Provider,
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<(String, u64), String> {
    let base = base_url.trim_end_matches('/');
    // Strip trailing /v1beta or /v1alpha to rebuild path
    let root = base
        .trim_end_matches("/v1beta")
        .trim_end_matches("/v1alpha");
    let url = format!(
        "{}/v1beta/models/{}:generateContent?key={}",
        root, model, api_key
    );

    let body = serde_json::json!({
        "contents": [{"parts": [{"text": "Say hello in one sentence."}]}],
        "generationConfig": {"maxOutputTokens": 10}
    });

    let start = std::time::Instant::now();
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        provider
            .apply_custom_headers(client.post(&url).json(&body))?
            .send(),
    )
    .await
    .map_err(|_| "请求超时 (15s)".to_string())?
    .map_err(|e| format!("请求失败: {}", e))?;

    let latency_ms = start.elapsed().as_millis() as u64;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status, truncate(&text, 200)));
    }

    let reply =
        extract_gemini_reply(&text).unwrap_or_else(|| "(响应解析成功，但未提取到文本)".to_string());
    Ok((reply, latency_ms))
}

// ── Response text extraction helpers ──

fn extract_chat_reply(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
}

fn extract_anthropic_reply(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    // Anthropic returns { content: [{type: "text", text: "..."}], ... }
    if let Some(arr) = v["content"].as_array() {
        for block in arr {
            if let Some(text) = block["text"].as_str() {
                return Some(text.trim().to_string());
            }
        }
    }
    None
}

fn extract_gemini_reply(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .map(|s| s.trim().to_string())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

#[tauri::command]
pub async fn test_provider_connection(
    state: State<'_, Arc<AppState>>,
    provider_id: String,
) -> Result<ProviderTestResult, String> {
    let provider = state
        .db
        .providers
        .get_by_id(&state.db.conn, &provider_id)?
        .ok_or_else(|| "Provider not found".to_string())?;

    // Get the first API key
    let api_key = provider.api_keys.as_ref().and_then(|keys| {
        serde_json::from_str::<Vec<String>>(keys)
            .ok()
            .and_then(|k| k.into_iter().find(|k| !k.trim().is_empty()))
    });

    let api_key = match api_key {
        Some(k) => k,
        None => {
            return Ok(ProviderTestResult {
                success: false,
                message: "无可用 API Key".to_string(),
                latency_ms: 0,
                model_tested: None,
                error_details: Some("该服务商未配置 API Key，无法测试连接".to_string()),
            });
        }
    };

    // Build HTTP client with optional proxy
    let mut builder = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("PoolGate/1.0");
    if let Some(ref proxy_url) = provider.proxy_url {
        if !proxy_url.trim().is_empty() {
            if let Ok(proxy) = reqwest::Proxy::http(proxy_url) {
                builder = builder.proxy(proxy);
            }
            if let Ok(proxy) = reqwest::Proxy::https(proxy_url) {
                builder = builder.proxy(proxy);
            }
        }
    }
    let client = builder
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let model = pick_test_model(&provider);
    let protocol = canonical_protocol(&provider.protocol);
    let base_url = provider.base_url_for_protocol(protocol);

    let result = match protocol {
        "anthropic" => test_anthropic_chat(&client, &provider, &base_url, &api_key, &model).await,
        "gemini" => test_gemini_chat(&client, &provider, &base_url, &api_key, &model).await,
        // OpenAI-compatible: chat, responses, and unknown protocols
        _ => test_openai_chat(&client, &provider, &base_url, &api_key, &model).await,
    };

    match result {
        Ok((reply, latency_ms)) => Ok(ProviderTestResult {
            success: true,
            message: format!("测试成功 ({:.0}ms)", latency_ms),
            latency_ms,
            model_tested: Some(model),
            error_details: Some(reply), // reuse field to show the model's reply
        }),
        Err(e) => Ok(ProviderTestResult {
            success: false,
            message: "测试失败".to_string(),
            latency_ms: 0,
            model_tested: Some(model),
            error_details: Some(e),
        }),
    }
}

/// Test a specific account's connection using its own credentials.
#[tauri::command]
pub async fn test_account_connection(
    state: State<'_, Arc<AppState>>,
    account_id: String,
) -> Result<ProviderTestResult, String> {
    let account = state
        .db
        .accounts
        .get_by_id(&state.db.conn, &account_id)?
        .ok_or_else(|| "账号不存在".to_string())?;

    let provider = state
        .db
        .providers
        .get_by_id(&state.db.conn, account.provider_id.as_deref().unwrap_or(""))?
        .ok_or_else(|| "未找到关联的上游连接器".to_string())?;

    // Get credential from account
    let credential = crate::services::credentials::auth_credential(&account)
        .map_err(|e| format!("获取账号凭据失败: {}", e))?;

    // Build HTTP client with optional proxy
    let mut builder = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("PoolGate/1.0");
    if let Some(ref proxy_url) = provider.proxy_url {
        if !proxy_url.trim().is_empty() {
            if let Ok(proxy) = reqwest::Proxy::http(proxy_url) {
                builder = builder.proxy(proxy);
            }
            if let Ok(proxy) = reqwest::Proxy::https(proxy_url) {
                builder = builder.proxy(proxy);
            }
        }
    }
    let client = builder
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    // Pick model: prefer account's first model, then provider's, then fallback default
    let model = pick_test_model_for_account(&account, &provider);
    let protocol = canonical_protocol(&provider.protocol);
    let base_url = provider.base_url_for_protocol(protocol);

    let api_key = match &credential {
        crate::services::credentials::AuthCredential::ApiKey(k) => k.clone(),
        crate::services::credentials::AuthCredential::Bearer(t) => t.clone(),
    };

    let result = match protocol {
        "anthropic" => test_anthropic_chat(&client, &provider, &base_url, &api_key, &model).await,
        "gemini" => test_gemini_chat(&client, &provider, &base_url, &api_key, &model).await,
        // OpenAI-compatible: chat, responses, and unknown protocols
        _ => test_openai_chat(&client, &provider, &base_url, &api_key, &model).await,
    };

    match result {
        Ok((reply, latency_ms)) => Ok(ProviderTestResult {
            success: true,
            message: format!("测试成功 ({:.0}ms)", latency_ms),
            latency_ms,
            model_tested: Some(model),
            error_details: Some(reply),
        }),
        Err(e) => Ok(ProviderTestResult {
            success: false,
            message: "测试失败".to_string(),
            latency_ms: 0,
            model_tested: Some(model),
            error_details: Some(e),
        }),
    }
}

/// Pick a model to test with, preferring account's declared models, then provider's.
fn pick_test_model_for_account(account: &Account, provider: &Provider) -> String {
    // Try account's models first
    if let Some(ref models_raw) = account.models {
        if let Ok(list) = serde_json::from_str::<Vec<String>>(models_raw) {
            if let Some(first) = list.into_iter().find(|m| !m.trim().is_empty()) {
                return first;
            }
        }
    }
    // Fall back to provider's models
    pick_test_model(provider)
}
