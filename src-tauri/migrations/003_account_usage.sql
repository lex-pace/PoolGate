-- Non-sensitive account profile and quota snapshots
-- Version 3
CREATE TABLE IF NOT EXISTS account_usage (
    account_id          TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    provider            TEXT NOT NULL,
    plan_type           TEXT,
    quota_windows       TEXT NOT NULL DEFAULT '[]',
    last_refreshed_at   TEXT,
    last_error          TEXT,
    token_refreshed_at  TEXT
);

CREATE INDEX IF NOT EXISTS idx_account_usage_last_refreshed
    ON account_usage(last_refreshed_at);
