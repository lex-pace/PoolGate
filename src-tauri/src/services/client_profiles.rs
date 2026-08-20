//! Official client fingerprints for OAuth / subscription upstreams.
//!
//! Subscription backends (ChatGPT Codex, Claude Code, GitHub Copilot, Gemini
//! CLI) profile the HTTP clients that talk to them. A gateway that mixes
//! signals — e.g. an official `originator` header with `User-Agent:
//! PoolGate/1.0` — is an immediately recognizable third-party-tool signature.
//! These profiles keep every header set PoolGate sends internally consistent
//! with one real official client.
//!
//! Version strings must track the official clients: vendors occasionally
//! reject stale versions. Bump the constants here when updating; everything
//! else (adapters, health checks, OAuth refresh) picks them up automatically.

use reqwest::RequestBuilder;

// ── OpenAI Codex CLI ─────────────────────────────────────────────────────────

/// Codex CLI version we present. Keep in sync with [`CODEX_CLI_USER_AGENT`].
pub const CODEX_CLI_VERSION: &str = "0.48.0";

/// User-Agent of the official Codex CLI (codex-rs). The `originator` header
/// must match this client family or ChatGPT's backend flags the request.
pub const CODEX_CLI_USER_AGENT: &str = "codex_cli_rs/0.48.0 (Mac OS 15.5.0; arm64) Mac";

/// `originator` value sent by the official Codex CLI.
pub const CODEX_ORIGINATOR: &str = "codex_cli_rs";

// ── Anthropic Claude Code ────────────────────────────────────────────────────

/// User-Agent of the official Claude Code CLI. Claude Code always presents
/// `(external, cli)` in its UA; Anthropic's backend keys risk signals off it.
pub const CLAUDE_CODE_USER_AGENT: &str = "claude-cli/2.1.2 (external, cli)";

/// Beta flags the official CLI sends: the Claude Code feature flag plus the
/// OAuth bearer-auth flag required by subscription (OAuth) accounts.
pub const CLAUDE_CODE_BETA_FLAGS: &str = "claude-code-20250219,oauth-2025-04-20";

// ── GitHub Copilot ───────────────────────────────────────────────────────────

/// User-Agent of Copilot Chat as embedded in VS Code.
pub const COPILOT_CHAT_USER_AGENT: &str = "GitHubCopilotChat/0.28.0";

/// Editor identity Copilot requests are expected to carry.
pub const COPILOT_EDITOR_VERSION: &str = "vscode/1.100.0";

// ── Google Gemini CLI ────────────────────────────────────────────────────────

/// User-Agent of the official Gemini CLI (`glh` = gemini-long-haul client id).
pub const GEMINI_CLI_USER_AGENT: &str = "GeminiCLI/v19.1.0 (macos; arm64) glh/0.0.0";

/// `x-goog-api-client` value sent by the official Gemini CLI.
pub const GEMINI_CLI_API_CLIENT: &str = "glh/0.0.0";

// ── Profile application ──────────────────────────────────────────────────────

/// Apply the Codex CLI fingerprint (User-Agent; caller adds `originator`,
/// auth and Accept so streaming/JSON variants stay explicit).
pub fn apply_codex_profile(request: RequestBuilder) -> RequestBuilder {
    request.header("User-Agent", CODEX_CLI_USER_AGENT)
}

/// Apply the Claude Code fingerprint: CLI User-Agent, beta flags and app id.
/// Used by the proxy path and health checks for Claude OAuth accounts.
pub fn apply_claude_code_profile(request: RequestBuilder) -> RequestBuilder {
    request
        .header("User-Agent", CLAUDE_CODE_USER_AGENT)
        .header("anthropic-beta", CLAUDE_CODE_BETA_FLAGS)
        .header("x-app", "cli")
}

/// Apply the GitHub Copilot Chat fingerprint (VS Code editor identity).
pub fn apply_copilot_profile(request: RequestBuilder) -> RequestBuilder {
    request
        .header("User-Agent", COPILOT_CHAT_USER_AGENT)
        .header("Editor-Version", COPILOT_EDITOR_VERSION)
}

/// Apply the Gemini CLI fingerprint (User-Agent + api-client id).
pub fn apply_gemini_cli_profile(request: RequestBuilder) -> RequestBuilder {
    request
        .header("User-Agent", GEMINI_CLI_USER_AGENT)
        .header("x-goog-api-client", GEMINI_CLI_API_CLIENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_stay_internally_consistent() {
        // The Codex UA must belong to the same client family as its originator.
        assert!(CODEX_CLI_USER_AGENT.starts_with("codex_cli_rs/"));
        assert_eq!(CODEX_ORIGINATOR, "codex_cli_rs");
        // Claude Code always presents itself as the external CLI.
        assert!(CLAUDE_CODE_USER_AGENT.contains("claude-cli/"));
        assert!(CLAUDE_CODE_USER_AGENT.contains("(external, cli)"));
        assert!(CLAUDE_CODE_BETA_FLAGS.contains("claude-code"));
        // Gemini CLI identifies itself in both UA and api-client.
        assert!(GEMINI_CLI_USER_AGENT.contains("glh/"));
        assert_eq!(GEMINI_CLI_API_CLIENT, "glh/0.0.0");
    }
}
