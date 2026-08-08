-- Initial schema for PoolGate
-- Version 1
-- Providers table
CREATE TABLE IF NOT EXISTS providers (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    type        TEXT NOT NULL,       -- 'official' | 'relay' | 'local'
    base_url    TEXT NOT NULL,
    protocol    TEXT NOT NULL,       -- 'openai' | 'anthropic' | 'gemini' | 'custom'
    api_keys    TEXT,                -- JSON array of keys
    models      TEXT,                -- JSON array of supported models
    proxy_url   TEXT,                -- upstream proxy
    timeout_ms  INTEGER DEFAULT 30000,
    priority    INTEGER DEFAULT 0,
    enabled     BOOLEAN DEFAULT 1,
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Accounts table
CREATE TABLE IF NOT EXISTS accounts (
    id              TEXT PRIMARY KEY,
    provider_id     TEXT REFERENCES providers(id),
    name            TEXT,
    api_key         TEXT NOT NULL,
    models          TEXT,             -- JSON: available models for this account
    quota_limit     REAL,             -- quota upper limit (tokens/dollars)
    quota_used      REAL DEFAULT 0,
    status          TEXT DEFAULT 'active',  -- active|limited|exhausted|disabled
    health_status   TEXT DEFAULT 'unchecked', -- unchecked|healthy|failed|error
    health_code     INTEGER,          -- last health check HTTP status
    health_msg      TEXT,             -- failure reason
    health_latency  INTEGER,          -- last health check latency (ms)
    health_check_at DATETIME,         -- last health check time
    priority        INTEGER DEFAULT 0,
    tags            TEXT,             -- JSON array
    last_used_at    DATETIME,
    created_at      DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Agent groups table
CREATE TABLE IF NOT EXISTS agent_groups (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT,
    protocol    TEXT NOT NULL,       -- 'openai' | 'anthropic' (determines routing)
    strategy    TEXT DEFAULT 'round_robin',  -- round_robin|least_used|priority|random|cost_optimized
    api_key     TEXT,                -- optional group-level access key
    enabled     BOOLEAN DEFAULT 1,
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Group-Account association
CREATE TABLE IF NOT EXISTS group_accounts (
    group_id   TEXT REFERENCES agent_groups(id),
    account_id TEXT REFERENCES accounts(id),
    weight     INTEGER DEFAULT 1,
    PRIMARY KEY (group_id, account_id)
);

-- Request logs (master table, monthly sub-tables will be created dynamically)
CREATE TABLE IF NOT EXISTS request_logs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id      TEXT,
    source        TEXT,
    provider_id   TEXT,
    account_id    TEXT,
    model         TEXT,
    endpoint      TEXT,
    status        TEXT,        -- success|error|rate_limited|timeout
    status_code   INTEGER,
    input_tokens  INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cache_tokens  INTEGER DEFAULT 0,
    cost          REAL DEFAULT 0,
    latency_ms    INTEGER,
    ttft_ms       INTEGER,
    is_stream     BOOLEAN DEFAULT 1,
    error_message TEXT,
    request_at    DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Pricing configuration
CREATE TABLE IF NOT EXISTS pricing (
    model         TEXT PRIMARY KEY,
    input_per_m   REAL,   -- price per 1M input tokens
    output_per_m  REAL,   -- price per 1M output tokens
    currency      TEXT DEFAULT 'USD'
);

-- Alert events table
CREATE TABLE IF NOT EXISTS alerts (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    level       TEXT,       -- 'info' | 'warning' | 'error' | 'critical'
    category    TEXT,       -- 'quota' | 'token' | 'health' | 'gateway'
    title       TEXT,
    message     TEXT,
    dismissed   BOOLEAN DEFAULT 0,
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Gateway runtime logs
CREATE TABLE IF NOT EXISTS gateway_logs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    level       TEXT,         -- 'info' | 'warn' | 'error'
    event       TEXT,         -- 'startup' | 'shutdown' | 'port_conflict' | 'tls_error'
    message     TEXT,
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Settings table
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
