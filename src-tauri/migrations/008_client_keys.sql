-- Virtual client keys: the formal authentication & routing identity for
-- PoolGate gateway access. A client key binds to zero or more route pools
-- (agent_groups). Rate limiting, statistics, audit and permission features
-- hang off this table in later migrations.
--
-- Security model:
--   * The raw key is generated once (pg_live_...) and shown to the user at
--     creation time only. It is never persisted.
--   * Only key_hash (SHA-256 hex), key_prefix (for fast lookup) and
--     key_last_four (for display) are stored.
CREATE TABLE IF NOT EXISTS client_keys (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    key_prefix      TEXT NOT NULL,              -- e.g. 'pg_live_' constant prefix
    key_hash        TEXT NOT NULL UNIQUE,       -- SHA-256 hex of the raw key
    key_last_four   TEXT NOT NULL,              -- display-only suffix
    enabled         BOOLEAN DEFAULT 1,
    -- Extension hooks (reserved for later capabilities):
    --   rpm_limit / tpm_limit: rate limiting
    --   allowed_protocols:     protocol permission scope
    --   allowed_models:        model permission scope (JSON array)
    rpm_limit       INTEGER,                    -- requests per minute (0 = unlimited)
    tpm_limit       INTEGER,                    -- tokens per minute (0 = unlimited)
    allowed_protocols TEXT,                     -- JSON array or NULL (all)
    allowed_models    TEXT,                     -- JSON array or NULL (all)
    expires_at      DATETIME,
    last_used_at    DATETIME,
    created_at      DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Many-to-many: client key -> route pools (agent_groups).
-- A key can route to multiple pools; the pool used per request is decided by
-- the routing precedence (explicit X-Group-Id > first bound pool > default).
CREATE TABLE IF NOT EXISTS client_key_pools (
    client_key_id TEXT NOT NULL REFERENCES client_keys(id) ON DELETE CASCADE,
    pool_id       TEXT NOT NULL REFERENCES agent_groups(id) ON DELETE CASCADE,
    PRIMARY KEY (client_key_id, pool_id)
);

CREATE INDEX IF NOT EXISTS idx_client_key_pools_key ON client_key_pools(client_key_id);
