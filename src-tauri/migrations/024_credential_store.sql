CREATE TABLE IF NOT EXISTS credential_store (
    secret_ref TEXT PRIMARY KEY NOT NULL,
    credential BLOB NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
