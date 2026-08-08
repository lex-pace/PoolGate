-- PoolGate credential metadata and upstream adapter fields
-- Version 2
ALTER TABLE accounts ADD COLUMN credential_type TEXT NOT NULL DEFAULT 'api_key';
ALTER TABLE accounts ADD COLUMN credential_data TEXT;
ALTER TABLE accounts ADD COLUMN source_format TEXT;
ALTER TABLE accounts ADD COLUMN external_account_id TEXT;
ALTER TABLE accounts ADD COLUMN email TEXT;
ALTER TABLE accounts ADD COLUMN expires_at TEXT;
ALTER TABLE accounts ADD COLUMN metadata TEXT;
ALTER TABLE accounts ADD COLUMN credential_fingerprint TEXT;

CREATE INDEX IF NOT EXISTS idx_accounts_credential_fingerprint
    ON accounts(credential_fingerprint);
CREATE INDEX IF NOT EXISTS idx_accounts_external_account_id
    ON accounts(external_account_id);
