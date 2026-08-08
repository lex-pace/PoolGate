-- Route-pool managed keys, model constraints, per-request usage metadata,
-- and managed Agent application configuration snapshots.
-- Version 10
ALTER TABLE client_keys ADD COLUMN managed_pool_id TEXT REFERENCES agent_groups(id) ON DELETE CASCADE;
ALTER TABLE client_keys ADD COLUMN secret_ref TEXT;
ALTER TABLE client_keys ADD COLUMN rotated_at DATETIME;

CREATE UNIQUE INDEX IF NOT EXISTS idx_client_keys_managed_pool
    ON client_keys(managed_pool_id)
    WHERE managed_pool_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS group_model_resources (
    group_id   TEXT NOT NULL REFERENCES agent_groups(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    model      TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (group_id, provider_id, model)
);

CREATE INDEX IF NOT EXISTS idx_group_model_resources_group
    ON group_model_resources(group_id);
CREATE INDEX IF NOT EXISTS idx_group_model_resources_provider
    ON group_model_resources(provider_id);

ALTER TABLE request_logs ADD COLUMN request_id TEXT;
ALTER TABLE request_logs ADD COLUMN attempt_count INTEGER DEFAULT 1;
ALTER TABLE request_logs ADD COLUMN usage_available BOOLEAN DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_request_logs_request_id
    ON request_logs(request_id);

CREATE TABLE IF NOT EXISTS agent_app_snapshots (
    id            TEXT PRIMARY KEY,
    app_id        TEXT NOT NULL,
    group_id      TEXT NOT NULL REFERENCES agent_groups(id) ON DELETE CASCADE,
    config_path   TEXT NOT NULL,
    backup_path   TEXT NOT NULL,
    original_hash TEXT NOT NULL,
    managed_hash  TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'active',
    created_at    DATETIME DEFAULT CURRENT_TIMESTAMP,
    restored_at   DATETIME
);

CREATE INDEX IF NOT EXISTS idx_agent_app_snapshots_app_status
    ON agent_app_snapshots(app_id, status);
