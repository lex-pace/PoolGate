-- Version 5: normalize legacy protocol values to the canonical set.
-- Canonical values are: responses, chat, anthropic, gemini.

-- Codex is a Responses-native resource even when legacy rows called it openai.
UPDATE providers
SET protocol = 'responses', protocols = '["responses"]'
WHERE lower(type) = 'codex'
   OR lower(name) LIKE '%codex%'
   OR lower(protocol) = 'codex';

UPDATE accounts
SET protocols = '["responses"]'
WHERE provider_id IN (
    SELECT id FROM providers
    WHERE lower(type) = 'codex' OR lower(name) LIKE '%codex%'
)
   OR lower(COALESCE(credential_type, '')) = 'codex_oauth'
   OR lower(COALESCE(source_format, '')) = 'codex_auth';

-- Normalize the legacy single protocol columns for all remaining resources.
UPDATE providers
SET protocol = CASE lower(protocol)
    WHEN 'openai' THEN 'chat'
    WHEN 'chat_completions' THEN 'chat'
    WHEN 'messages' THEN 'anthropic'
    WHEN 'anthropic_messages' THEN 'anthropic'
    WHEN 'google' THEN 'gemini'
    ELSE lower(protocol)
END
WHERE lower(protocol) NOT IN ('responses', 'chat', 'anthropic', 'gemini');

-- Version 4 populated protocols from the legacy single protocol. Replace known
-- legacy tokens in JSON or delimiter-separated values without discarding
-- already-canonical multi-protocol selections.
UPDATE providers
SET protocols = CASE
    WHEN protocols IS NULL OR trim(protocols) = '' THEN json_array(protocol)
    ELSE replace(replace(replace(replace(protocols,
        '"openai"', '"chat"'),
        '"codex"', '"responses"'),
        '"messages"', '"anthropic"'),
        '"google"', '"gemini"')
END;

UPDATE accounts
SET protocols = CASE
    WHEN protocols IS NULL OR trim(protocols) = '' THEN json_array('chat')
    ELSE replace(replace(replace(replace(protocols,
        '"openai"', '"chat"'),
        '"codex"', '"responses"'),
        '"messages"', '"anthropic"'),
        '"google"', '"gemini"')
END
WHERE NOT (
    lower(COALESCE(credential_type, '')) = 'codex_oauth'
    OR lower(COALESCE(source_format, '')) = 'codex_auth'
);
