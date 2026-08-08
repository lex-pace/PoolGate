-- Secure credential references
-- Version 6
-- Secret values are stored in the operating-system credential vault. SQLite
-- keeps only opaque references so database backups do not expose credentials.
ALTER TABLE accounts ADD COLUMN secret_ref TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_accounts_secret_ref
    ON accounts(secret_ref)
    WHERE secret_ref IS NOT NULL;
