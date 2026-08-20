pub mod anthropic;
pub mod gemini;
pub mod openai;
pub mod responses;

use serde_json::Value;

/// Canonical token usage shared by every protocol path (the mixed-pool
/// "alignment" contract).
///
/// Caliber rule: `input_tokens` always counts **fresh (non-cached) input**
/// tokens. Anthropic reports that natively; OpenAI (`prompt_tokens`) and
/// Gemini (`promptTokenCount`) report the cache-inclusive total, so the
/// decoders subtract the cached part. With this rule the numbers can be summed
/// across providers without double counting, and [`Usage::to_*_json`] can
/// re-encode into any protocol's vocabulary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Fresh (non-cached) input tokens.
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Prompt tokens served from the upstream cache (Anthropic
    /// `cache_read_input_tokens`, OpenAI `cached_tokens`, Gemini
    /// `cachedContentTokenCount`).
    pub cache_read_tokens: i64,
    /// Tokens written into the cache this request (Anthropic prompt caching
    /// only; billed at a premium, so it must stay separate from reads).
    pub cache_write_tokens: i64,
    pub available: bool,
}

impl Usage {
    /// Aggregated cache count (read + write) for the legacy `cache_tokens`
    /// column and UI. Prefer the split fields for cost calculations.
    pub fn cache_tokens(&self) -> i64 {
        self.cache_read_tokens + self.cache_write_tokens
    }

    /// Cache-inclusive input total, comparable across providers.
    pub fn total_input_tokens(&self) -> i64 {
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens
    }

    /// Decode Anthropic Messages usage (`input_tokens` already excludes cache).
    pub fn from_anthropic_usage(usage: &serde_json::Value) -> Usage {
        Usage {
            input_tokens: usage
                .get("input_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            output_tokens: usage
                .get("output_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            cache_read_tokens: usage
                .get("cache_read_input_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            cache_write_tokens: usage
                .get("cache_creation_input_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            available: true,
        }
    }

    /// Decode OpenAI usage. Accepts Chat (`prompt_tokens`,
    /// `prompt_tokens_details.cached_tokens`) and Responses (`input_tokens`,
    /// `input_tokens_details.cached_tokens`) shapes; both report a
    /// cache-inclusive input total, so the cached part is subtracted to keep
    /// the canonical fresh-input caliber.
    pub fn from_openai_usage(usage: &serde_json::Value) -> Usage {
        let reported_input = usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let output_tokens = usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let cached = usage
            .get("prompt_tokens_details")
            .or_else(|| usage.get("input_tokens_details"))
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        Usage {
            input_tokens: (reported_input - cached).max(0),
            output_tokens,
            cache_read_tokens: cached,
            cache_write_tokens: 0,
            available: true,
        }
    }

    /// Decode Gemini `usageMetadata` (`promptTokenCount` includes cached).
    pub fn from_gemini_metadata(usage: &serde_json::Value) -> Usage {
        let reported_input = usage
            .get("promptTokenCount")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let cached = usage
            .get("cachedContentTokenCount")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        Usage {
            input_tokens: (reported_input - cached).max(0),
            output_tokens: usage
                .get("candidatesTokenCount")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            cache_read_tokens: cached,
            cache_write_tokens: 0,
            available: true,
        }
    }

    /// Re-encode into the Anthropic Messages usage vocabulary (fresh input +
    /// explicit cache read/write fields).
    pub fn to_anthropic_json(self) -> serde_json::Value {
        serde_json::json!({
            "input_tokens": self.input_tokens,
            "cache_read_input_tokens": self.cache_read_tokens,
            "cache_creation_input_tokens": self.cache_write_tokens,
            "output_tokens": self.output_tokens,
        })
    }

    /// Re-encode into the OpenAI Responses usage vocabulary (input total +
    /// `input_tokens_details.cached_tokens`).
    pub fn to_responses_usage_json(self) -> serde_json::Value {
        let total_input = self.total_input_tokens();
        serde_json::json!({
            "input_tokens": total_input,
            "input_tokens_details": { "cached_tokens": self.cache_read_tokens },
            "output_tokens": self.output_tokens,
            "total_tokens": total_input + self.output_tokens,
        })
    }
}

/// Whether a usage JSON node speaks the OpenAI vocabulary (as opposed to
/// Anthropic Messages). Responses bodies reuse Anthropic's `input_tokens` key
/// name but carry OpenAI semantics; they are told apart by the details node.
fn is_openai_usage_shape(usage: &serde_json::Value) -> bool {
    usage.get("prompt_tokens").is_some()
        || usage.get("completion_tokens").is_some()
        || usage.get("prompt_tokens_details").is_some()
        || usage.get("input_tokens_details").is_some()
}

/// Decode usage from any protocol's complete JSON response body (Chat /
/// Responses / Anthropic / Gemini, including Antigravity's `response` wrapper).
pub fn usage_from_response_body(body: &[u8]) -> Usage {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Usage::default();
    };
    let usage = value
        .get("usage")
        .or_else(|| value.get("response").and_then(|node| node.get("usage")));
    if let Some(usage) = usage {
        return if is_openai_usage_shape(usage) {
            Usage::from_openai_usage(usage)
        } else {
            Usage::from_anthropic_usage(usage)
        };
    }
    let metadata = value.get("usageMetadata").or_else(|| {
        value
            .get("response")
            .and_then(|node| node.get("usageMetadata"))
    });
    if let Some(metadata) = metadata {
        return Usage::from_gemini_metadata(metadata);
    }
    Usage::default()
}

/// Join a provider Base URL with a protocol path without duplicating API version
/// segments. Providers may store either a host root (`https://api.example.com`)
/// or a versioned root (`https://api.example.com/v1`).
pub fn build_upstream_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path = path.trim_matches('/');

    if path.is_empty() || base.ends_with(path) {
        return base.to_string();
    }

    for version in ["v1beta", "v1"] {
        if base.ends_with(&format!("/{version}")) {
            if let Some(rest) = path.strip_prefix(&format!("{version}/")) {
                return format!("{base}/{rest}");
            }
        }
    }

    format!("{base}/{path}")
}

/// Extract a concise, human-readable error message from an upstream (provider)
/// error body. Handles the common OpenAI / Anthropic / Gemini JSON error shapes
/// and falls back to a truncated plain-text excerpt. Sensitive values are
/// redacted and the result is capped so huge bodies never bloat the request log.
pub fn upstream_error_message(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed = serde_json::from_str::<serde_json::Value>(trimmed).ok();
    let message = parsed.as_ref().and_then(|value| {
        value
            .get("error")
            .and_then(|error| {
                error
                    .get("message")
                    .or_else(|| error.get("detail"))
                    .or_else(|| error.get("error"))
            })
            .or_else(|| value.get("message"))
            .or_else(|| value.get("detail"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(str::to_string)
            .or_else(|| {
                value
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|message| !message.is_empty())
                    .map(str::to_string)
            })
    });
    let candidate = message.or_else(|| {
        // Valid JSON without a recognizable message, or a plain-text body:
        // keep the first line only, capped.
        trimmed.lines().next().map(|line| line.trim().to_string())
    });
    candidate.map(|message| {
        let message = crate::services::redaction::redact_sensitive(&message);
        let message = message.trim();
        let mut result = String::with_capacity(message.len());
        for character in message.chars().take(1000) {
            result.push(character);
        }
        result
    })
}

/// Detect an upstream error carried in a complete SSE event block (Anthropic
/// `event: error`, OpenAI Chat/Responses error data, Gemini error data).
/// Returns the extracted error message, or `None` for normal events.
pub fn sse_event_error(event_block: &str) -> Option<String> {
    let mut event_name = None;
    let mut data_lines = Vec::new();
    for line in event_block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start());
        }
    }
    let data = data_lines.join("\n");
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) else {
        return None;
    };
    let has_error_event = event_name == Some("error");
    let failed_type = value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| matches!(kind, "error" | "failed" | "response.failed"));
    let error_node = value.get("error").or_else(|| {
        value
            .get("response")
            .and_then(|response| response.get("error"))
    });
    let has_error_node = error_node.is_some_and(|node| node.is_object() || node.is_string());
    if !has_error_event && !has_error_node && !failed_type {
        return None;
    }
    error_node
        .and_then(|node| {
            node.get("message")
                .or_else(|| node.get("detail"))
                .or_else(|| node.get("error"))
        })
        .or_else(|| value.get("message"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
        .or_else(|| {
            error_node
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| upstream_error_message(data.as_bytes()))
}

/// Normalize a user-supplied Base URL to the canonical mount-point form.
///
/// The gateway owns the API version segment (`/v1`, `/v1beta`) and the endpoint
/// path (`/chat/completions`, `/messages`, `/models`); a stored Base URL must be
/// the origin plus any provider-specific mount prefix (e.g. `/anthropic`).
/// This strips trailing slashes and any endpoint/version suffix the gateway
/// will re-append, so pasting a "complete" endpoint URL still resolves to the
/// same upstream host that health checks and routing target.
pub fn normalize_base_url(protocol: &str, raw: &str) -> String {
    let mut url = raw.trim().to_string();
    if url.is_empty() {
        return url;
    }
    while url.ends_with('/') {
        url.pop();
    }

    let tails: &[&str] = match protocol {
        "anthropic" => &["/v1/messages", "/messages", "/v1"],
        "gemini" => &["/v1beta/models", "/v1beta"],
        "chat" | "responses" | "openai" => &[
            "/v1/chat/completions",
            "/chat/completions",
            "/v1/models",
            "/v1",
            "/models",
        ],
        "local" => &["/v1/models", "/models", "/v1"],
        _ => &[
            "/v1/chat/completions",
            "/chat/completions",
            "/v1/messages",
            "/messages",
            "/v1beta/models",
            "/v1/models",
            "/models",
            "/v1",
            "/v1beta",
        ],
    };

    for tail in tails {
        if url.ends_with(tail) {
            url.truncate(url.len() - tail.len());
            break;
        }
    }

    url
}

#[cfg(test)]
mod tests {
    use super::build_upstream_url;
    use super::normalize_base_url;
    use super::{usage_from_response_body, Usage};

    #[test]
    fn canonical_usage_decodes_each_protocol_caliber() {
        // OpenAI Chat: prompt_tokens INCLUDES cached tokens.
        let openai = Usage::from_openai_usage(&serde_json::json!({
            "prompt_tokens": 12,
            "completion_tokens": 7,
            "prompt_tokens_details": { "cached_tokens": 3 }
        }));
        assert_eq!(openai.input_tokens, 9, "fresh input = 12 - 3 cached");
        assert_eq!(openai.cache_read_tokens, 3);
        assert_eq!(openai.cache_write_tokens, 0);

        // Anthropic: input_tokens already excludes cache; read/write split kept.
        let anthropic = Usage::from_anthropic_usage(&serde_json::json!({
            "input_tokens": 20,
            "output_tokens": 8,
            "cache_read_input_tokens": 5,
            "cache_creation_input_tokens": 2
        }));
        assert_eq!(anthropic.input_tokens, 20);
        assert_eq!(anthropic.cache_read_tokens, 5);
        assert_eq!(anthropic.cache_write_tokens, 2);
        assert_eq!(anthropic.total_input_tokens(), 27);

        // Gemini: promptTokenCount includes cached content.
        let gemini = Usage::from_gemini_metadata(&serde_json::json!({
            "promptTokenCount": 11,
            "candidatesTokenCount": 6,
            "cachedContentTokenCount": 1
        }));
        assert_eq!(gemini.input_tokens, 10);
        assert_eq!(gemini.cache_read_tokens, 1);

        // Mixed-pool sums are now safe: totals are cache-inclusive everywhere.
        let mixed = openai.total_input_tokens() + anthropic.total_input_tokens();
        assert_eq!(
            mixed,
            (9 + 3) + (20 + 5 + 2),
            "no provider double counts its cache"
        );
    }

    #[test]
    fn usage_re_encodes_into_target_vocabularies() {
        // OpenAI → Anthropic: split fresh/cached back out.
        let openai = Usage::from_openai_usage(&serde_json::json!({
            "prompt_tokens": 12,
            "completion_tokens": 7,
            "prompt_tokens_details": { "cached_tokens": 3 }
        }));
        let anthropic_json = openai.to_anthropic_json();
        assert_eq!(anthropic_json["input_tokens"], 9);
        assert_eq!(anthropic_json["cache_read_input_tokens"], 3);
        assert_eq!(anthropic_json["cache_creation_input_tokens"], 0);

        // Anthropic → Responses: fold cache back into the input total.
        let anthropic = Usage::from_anthropic_usage(&serde_json::json!({
            "input_tokens": 20,
            "output_tokens": 8,
            "cache_read_input_tokens": 5,
            "cache_creation_input_tokens": 2
        }));
        let responses_json = anthropic.to_responses_usage_json();
        assert_eq!(responses_json["input_tokens"], 27);
        assert_eq!(responses_json["input_tokens_details"]["cached_tokens"], 5);
        assert_eq!(responses_json["total_tokens"], 35);
    }

    #[test]
    fn usage_from_response_body_detects_every_protocol() {
        let chat = usage_from_response_body(
            br#"{"usage":{"prompt_tokens":12,"completion_tokens":7,"prompt_tokens_details":{"cached_tokens":3}}}"#,
        );
        assert_eq!((chat.input_tokens, chat.cache_read_tokens), (9, 3));

        let responses = usage_from_response_body(
            br#"{"response":{"usage":{"input_tokens":9,"output_tokens":4,"input_tokens_details":{"cached_tokens":2}}}}"#,
        );
        assert_eq!(
            (responses.input_tokens, responses.cache_read_tokens),
            (7, 2)
        );

        let anthropic = usage_from_response_body(
            br#"{"usage":{"input_tokens":20,"output_tokens":8,"cache_read_input_tokens":5}}"#,
        );
        assert_eq!(
            (anthropic.input_tokens, anthropic.cache_read_tokens),
            (20, 5)
        );

        let gemini = usage_from_response_body(
            br#"{"usageMetadata":{"promptTokenCount":11,"candidatesTokenCount":6,"cachedContentTokenCount":1}}"#,
        );
        assert_eq!((gemini.input_tokens, gemini.cache_read_tokens), (10, 1));

        assert!(!usage_from_response_body(b"not json").available);
    }

    #[test]
    fn joins_versioned_and_root_base_urls() {
        assert_eq!(
            build_upstream_url("https://api.openai.com/v1", "/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            build_upstream_url("https://api.anthropic.com", "/v1/messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            build_upstream_url(
                "https://generativelanguage.googleapis.com/v1beta",
                "/v1beta/models/m:generateContent"
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/m:generateContent"
        );
    }

    #[test]
    fn normalizes_complete_urls_to_mount_point() {
        // OpenAI-compatible: keep the origin, drop version + endpoint.
        assert_eq!(
            normalize_base_url("chat", "https://api.deepseek.com/v1/chat/completions"),
            "https://api.deepseek.com"
        );
        assert_eq!(
            normalize_base_url("chat", "https://api.openai.com/v1"),
            "https://api.openai.com"
        );
        assert_eq!(
            normalize_base_url("chat", "https://api.deepseek.com/v1/models"),
            "https://api.deepseek.com"
        );
        // Origin with provider mount prefix is preserved as-is.
        assert_eq!(
            normalize_base_url("anthropic", "https://api.deepseek.com/anthropic"),
            "https://api.deepseek.com/anthropic"
        );
        // A full Anthropic endpoint pasted by the user collapses to the mount.
        assert_eq!(
            normalize_base_url(
                "anthropic",
                "https://api.deepseek.com/anthropic/v1/messages"
            ),
            "https://api.deepseek.com/anthropic"
        );
        // Gemini version segment is stripped.
        assert_eq!(
            normalize_base_url(
                "gemini",
                "https://generativelanguage.googleapis.com/v1beta/models"
            ),
            "https://generativelanguage.googleapis.com"
        );
        // Unknown protocol strips every well-known suffix.
        assert_eq!(
            normalize_base_url("unknown", "https://host.example.com/v1/chat/completions"),
            "https://host.example.com"
        );
        // Already-canonical URLs are untouched.
        assert_eq!(
            normalize_base_url("chat", "https://api.deepseek.com"),
            "https://api.deepseek.com"
        );
    }
}
