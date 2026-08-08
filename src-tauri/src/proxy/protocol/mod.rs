pub mod anthropic;
pub mod gemini;
pub mod openai;
pub mod responses;

#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_tokens: i64,
    pub available: bool,
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
