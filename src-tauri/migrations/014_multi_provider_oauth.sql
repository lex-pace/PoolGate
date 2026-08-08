-- Add auth_mode and oauth_config to providers for multi-provider OAuth support.
-- auth_mode: how the provider authenticates with its upstream (api_key, oauth_device_flow, oauth_pkce, pat_to_token, api_key_or_oauth).
-- oauth_config: JSON blob with OAuth endpoints, client IDs, scopes, etc.

ALTER TABLE providers ADD COLUMN auth_mode TEXT DEFAULT 'api_key';
ALTER TABLE providers ADD COLUMN oauth_config TEXT;

-- Update existing Codex provider to mark it as OAuth-enabled.
-- Other providers will be updated when their OAuth flows are integrated.
-- Note: column name is 'type' in the database (mapped to provider_type in Rust).
UPDATE providers SET auth_mode = 'oauth_pkce' WHERE type = 'codex';
