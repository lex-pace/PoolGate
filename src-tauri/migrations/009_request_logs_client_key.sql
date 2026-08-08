-- Audit: associate request logs with the virtual client key that authenticated
-- the request, so per-key statistics / audit can be derived without parsing
-- free-text fields. NULL/absent for admin (gateway access key) or open mode.
ALTER TABLE request_logs ADD COLUMN client_key_id TEXT;
CREATE INDEX IF NOT EXISTS idx_request_logs_client_key ON request_logs(client_key_id);
