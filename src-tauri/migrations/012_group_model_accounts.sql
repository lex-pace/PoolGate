-- Exact account membership for each route-pool model resource.
-- Version 12
CREATE TABLE IF NOT EXISTS group_model_accounts (
    group_id    TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    model       TEXT NOT NULL,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (group_id, provider_id, model, account_id),
    FOREIGN KEY (group_id, provider_id, model)
        REFERENCES group_model_resources(group_id, provider_id, model)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_group_model_accounts_resource
    ON group_model_accounts(group_id, provider_id, model);
CREATE INDEX IF NOT EXISTS idx_group_model_accounts_account
    ON group_model_accounts(account_id);
