-- Enable stored Codex OAuth accounts now that the dedicated Responses adapter
-- supplies the ChatGPT account context and token refresh retry path.
UPDATE accounts
SET status = 'active',
    health_status = 'unchecked',
    health_code = NULL,
    health_msg = 'Codex Responses adapter enabled; awaiting connectivity check'
WHERE credential_type = 'codex_oauth'
  AND status = 'disabled'
  AND COALESCE(health_status, '') = 'adapter_required';
