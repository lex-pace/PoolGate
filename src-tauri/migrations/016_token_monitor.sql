-- ============================================================
-- 016: PoolGate Token Monitor
--  工具注册表 / 本地用量事件 / 会话摘要 / 项目归属 / 额度账号 / 额度快照 / 按日聚合
-- 仅追加元数据，不持久化 Prompt/Response/源代码/文件正文。
-- 时间列存 TEXT ISO8601 UTC（与现有迁移一致，不使用 STRICT）。
-- ============================================================

-- 3.1 tool_definition — 工具注册表与采集状态
CREATE TABLE IF NOT EXISTS tool_definition (
    tool_id            TEXT PRIMARY KEY,          -- claude_code / codex / cursor ...
    display_name       TEXT NOT NULL,
    vendor             TEXT,
    kind               TEXT NOT NULL DEFAULT 'usage', -- usage | quota_only | gateway
    adapter_version    INTEGER NOT NULL DEFAULT 1,
    capabilities_json  TEXT NOT NULL DEFAULT '{}',  -- AdapterCapabilities 序列化
    supported_os_json  TEXT NOT NULL DEFAULT '[]',  -- ["macos","windows","linux"]
    support_level      TEXT NOT NULL DEFAULT 'basic', -- full|standard|basic|quota_only|experimental
    enabled            INTEGER NOT NULL DEFAULT 1,
    custom_paths_json  TEXT,                        -- 用户自定义路径覆盖
    last_collected_at  TEXT,
    collector_status   TEXT NOT NULL DEFAULT 'idle', -- idle|active|waiting|permission|path_missing|format_changed|partial|error
    collector_error    TEXT,
    created_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    updated_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

-- 3.2 usage_event — 本地工具用量事件（含导入）
CREATE TABLE IF NOT EXISTS usage_event (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    source_type        TEXT NOT NULL,             -- local_discovered | imported
    tool_id            TEXT NOT NULL REFERENCES tool_definition(tool_id) ON DELETE CASCADE,
    device_id          TEXT NOT NULL DEFAULT 'local',
    model_raw          TEXT,
    model_normalized   TEXT,
    session_id         TEXT,                       -- 关联 session.session_id（工具内部 id 归一后）
    project_id         TEXT,                       -- 关联 project.project_id
    account_id         TEXT,                       -- 可空：本地日志未必含账号
    input_tokens       INTEGER,
    output_tokens      INTEGER,
    cache_read_tokens  INTEGER,
    cache_write_tokens INTEGER,
    reasoning_tokens   INTEGER,
    total_tokens       INTEGER,                    -- 按 provider usage 语义标准化，不重复累加 cache
    cost_amount        REAL,
    cost_currency      TEXT,
    usage_accuracy     TEXT NOT NULL DEFAULT 'unavailable', -- exact|provider_reported|derived|unavailable
    occurred_at        TEXT NOT NULL,              -- 事件发生时间（UTC ISO8601）
    source_fingerprint TEXT NOT NULL,              -- 跨面/幂等去重键（见 02 §5）
    source_locator_hash TEXT,                      -- 来源文件+offset 的不可逆 hash（诊断用）
    created_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    UNIQUE(source_fingerprint)                     -- 幂等：同一事件重复解析不重复入库
);
CREATE INDEX IF NOT EXISTS idx_usage_event_tool_time  ON usage_event(tool_id, occurred_at);
CREATE INDEX IF NOT EXISTS idx_usage_event_time        ON usage_event(occurred_at);
CREATE INDEX IF NOT EXISTS idx_usage_event_session     ON usage_event(session_id);
CREATE INDEX IF NOT EXISTS idx_usage_event_project     ON usage_event(project_id);
CREATE INDEX IF NOT EXISTS idx_usage_event_model       ON usage_event(model_normalized);

-- 3.3 session — 会话摘要（只存摘要，不存正文）
CREATE TABLE IF NOT EXISTS tm_session (
    session_id          TEXT PRIMARY KEY,          -- 归一后的内部 id（tool_id + external hash）
    tool_id             TEXT NOT NULL REFERENCES tool_definition(tool_id) ON DELETE CASCADE,
    device_id           TEXT NOT NULL DEFAULT 'local',
    external_session_id TEXT,                       -- 工具原始 session id
    project_id          TEXT,
    title_redacted      TEXT,                       -- 默认 项目名+时间 生成，非首条 Prompt
    model_set_json      TEXT,                       -- 本会话出现过的模型集合
    started_at          TEXT,
    last_active_at      TEXT,
    input_tokens        INTEGER,
    output_tokens       INTEGER,
    cache_tokens        INTEGER,
    total_tokens        INTEGER,
    message_count       INTEGER,
    status              TEXT,                        -- active | idle | closed
    created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    updated_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS idx_tm_session_tool ON tm_session(tool_id, last_active_at);
CREATE INDEX IF NOT EXISTS idx_tm_session_proj ON tm_session(project_id);

-- 3.4 project — 项目归属（默认只存 Hash）
CREATE TABLE IF NOT EXISTS tm_project (
    project_id          TEXT PRIMARY KEY,           -- canonical_path_hash 或显式 id
    device_id           TEXT NOT NULL DEFAULT 'local',
    canonical_path_hash TEXT NOT NULL,              -- 不可逆 hash（sha256）
    display_name        TEXT,                        -- 目录名（basename），非完整路径
    vcs_root_hash       TEXT,
    source              TEXT NOT NULL DEFAULT 'inferred', -- explicit|cwd|workspace|inferred
    created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS idx_tm_project_hash ON tm_project(canonical_path_hash);

-- 3.5 quota_account — 额度账号（监控绑定）
CREATE TABLE IF NOT EXISTS quota_account (
    account_id       TEXT PRIMARY KEY,             -- tm_ 前缀，区别于 accounts 表路由账号
    provider_id      TEXT NOT NULL,                -- claude|codex|cursor|copilot|deepseek|openrouter|...
    label            TEXT,
    identity_masked  TEXT,                          -- 脱敏身份（邮箱掩码等）
    plan_name        TEXT,
    auth_method      TEXT NOT NULL,                 -- oauth|api_key|local_auth_import|dashboard_cookie|custom_endpoint
    credential_ref   TEXT,                          -- 仅 Keychain 引用，绝不存明文
    linked_route_account_id TEXT,                   -- 可选：关联 accounts 表已有路由账号，复用其凭证
    display_window_key TEXT,                         -- 托盘展示哪个窗口
    alert_thresholds_json TEXT,                      -- 覆盖默认告警阈值
    enabled          INTEGER NOT NULL DEFAULT 1,
    last_success_at  TEXT,
    status           TEXT NOT NULL DEFAULT 'active', -- active|auth_expired|rate_limited|unavailable|error
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS idx_quota_account_provider ON quota_account(provider_id);

-- 3.6 quota_window_snapshot — 额度窗口快照（短期）
CREATE TABLE IF NOT EXISTS quota_window_snapshot (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id         TEXT NOT NULL REFERENCES quota_account(account_id) ON DELETE CASCADE,
    window_key         TEXT NOT NULL,               -- primary/secondary/weekly/monthly/model:opus ...
    window_type        TEXT NOT NULL,               -- rolling_5h|weekly|monthly|billing|credits|prepaid_balance|requests
    unit               TEXT NOT NULL,               -- tokens|requests|credits|currency|percent
    used_value         REAL,
    limit_value        REAL,
    remaining_value    REAL,
    remaining_percent  REAL,
    period_started_at  TEXT,
    resets_at          TEXT,
    source             TEXT NOT NULL,               -- official_api|local_auth|dashboard_session|custom_endpoint
    confidence         TEXT NOT NULL DEFAULT 'reported', -- reported|derived|stale
    error_code         TEXT,
    fetched_at         TEXT NOT NULL,
    expires_at         TEXT,
    UNIQUE(account_id, window_key)                  -- 每账号每窗口只保留最新（upsert）
);
CREATE INDEX IF NOT EXISTS idx_quota_snapshot_acc ON quota_window_snapshot(account_id);

-- 3.7 tm_daily_rollup — 按日聚合摘要（MONTH/TOTAL/热力图/连续天数）
CREATE TABLE IF NOT EXISTS tm_daily_rollup (
    day               TEXT NOT NULL,                -- 本地日历 YYYY-MM-DD
    tool_id           TEXT NOT NULL,
    model_normalized  TEXT NOT NULL DEFAULT '',
    input_tokens      INTEGER NOT NULL DEFAULT 0,
    output_tokens     INTEGER NOT NULL DEFAULT 0,
    cache_tokens      INTEGER NOT NULL DEFAULT 0,
    total_tokens      INTEGER NOT NULL DEFAULT 0,
    cost_amount       REAL NOT NULL DEFAULT 0,
    request_count     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, tool_id, model_normalized)
);
CREATE INDEX IF NOT EXISTS idx_tm_rollup_day ON tm_daily_rollup(day);
