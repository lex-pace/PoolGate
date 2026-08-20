use rusqlite::Connection;

/// Run database migrations based on PRAGMA user_version
pub fn run_migrations(
    conn: &std::sync::Mutex<Connection>,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = conn.lock().unwrap();
    let current_version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0);

    if current_version < 1 {
        conn.execute_batch(include_str!("../../migrations/001_initial.sql"))?;
        conn.pragma_update(None, "user_version", 1)?;
        tracing::info!("Database migrated to version 1");
    }

    if current_version < 2 {
        conn.execute_batch(include_str!("../../migrations/002_account_credentials.sql"))?;
        conn.pragma_update(None, "user_version", 2)?;
        tracing::info!("Database migrated to version 2");
    }

    if current_version < 3 {
        conn.execute_batch(include_str!("../../migrations/003_account_usage.sql"))?;
        conn.pragma_update(None, "user_version", 3)?;
        tracing::info!("Database migrated to version 3");
    }

    if current_version < 4 {
        conn.execute_batch(include_str!("../../migrations/004_protocol_multi.sql"))?;
        conn.pragma_update(None, "user_version", 4)?;
        tracing::info!("Database migrated to version 4");
    }

    if current_version < 5 {
        conn.execute_batch(include_str!("../../migrations/005_protocol_canonical.sql"))?;
        conn.pragma_update(None, "user_version", 5)?;
        tracing::info!("Database migrated to version 5");
    }

    if current_version < 6 {
        conn.execute_batch(include_str!("../../migrations/006_secure_credentials.sql"))?;
        conn.pragma_update(None, "user_version", 6)?;
        tracing::info!("Database migrated to version 6");
    }

    if current_version < 7 {
        conn.execute_batch(include_str!(
            "../../migrations/007_provider_protocol_base_urls.sql"
        ))?;
        conn.pragma_update(None, "user_version", 7)?;
        tracing::info!("Database migrated to version 7");
    }

    if current_version < 8 {
        conn.execute_batch(include_str!("../../migrations/008_client_keys.sql"))?;
        conn.pragma_update(None, "user_version", 8)?;
        tracing::info!("Database migrated to version 8");
    }

    if current_version < 9 {
        conn.execute_batch(include_str!(
            "../../migrations/009_request_logs_client_key.sql"
        ))?;
        conn.pragma_update(None, "user_version", 9)?;
        tracing::info!("Database migrated to version 9");
    }

    if current_version < 10 {
        conn.execute_batch(include_str!(
            "../../migrations/010_route_pool_management.sql"
        ))?;
        conn.pragma_update(None, "user_version", 10)?;
        tracing::info!("Database migrated to version 10");
    }

    if current_version < 11 {
        conn.execute_batch(include_str!(
            "../../migrations/011_topology_stats_indexes.sql"
        ))?;
        conn.pragma_update(None, "user_version", 11)?;
        tracing::info!("Database migrated to version 11");
    }

    if current_version < 12 {
        conn.execute_batch(include_str!(
            "../../migrations/012_group_model_accounts.sql"
        ))?;
        conn.pragma_update(None, "user_version", 12)?;
        tracing::info!("Database migrated to version 12");
    }

    if current_version < 13 {
        conn.execute_batch(include_str!("../../migrations/013_codex_oauth_adapter.sql"))?;
        conn.pragma_update(None, "user_version", 13)?;
        tracing::info!("Database migrated to version 13");
    }

    if current_version < 14 {
        conn.execute_batch(include_str!(
            "../../migrations/014_multi_provider_oauth.sql"
        ))?;
        conn.pragma_update(None, "user_version", 14)?;
        tracing::info!("Database migrated to version 14");
    }

    if current_version < 15 {
        conn.execute_batch(include_str!(
            "../../migrations/015_provider_custom_headers.sql"
        ))?;
        conn.pragma_update(None, "user_version", 15)?;
        tracing::info!("Database migrated to version 15");
    }

    // ==== token_monitor migrations ====
    if current_version < 16 {
        conn.execute_batch(include_str!("../../migrations/016_token_monitor.sql"))?;
        conn.pragma_update(None, "user_version", 16)?;
        tracing::info!("Database migrated to version 16 (token_monitor)");
    }

    if current_version < 17 {
        conn.execute_batch(include_str!(
            "../../migrations/017_usage_event_message_count.sql"
        ))?;
        conn.pragma_update(None, "user_version", 17)?;
        tracing::info!("Database migrated to version 17 (usage_event.message_count)");
    }

    if current_version < 18 {
        conn.execute_batch(include_str!(
            "../../migrations/018_tm_period_session_membership.sql"
        ))?;
        conn.pragma_update(None, "user_version", 18)?;
        tracing::info!("Database migrated to version 18 (tm_period_session)");
    }

    if current_version < 19 {
        conn.execute_batch(include_str!(
            "../../migrations/019_usage_event_session_times.sql"
        ))?;
        conn.pragma_update(None, "user_version", 19)?;
        tracing::info!("Database migrated to version 19 (usage_event session times)");
    }

    if current_version < 20 {
        conn.execute_batch(include_str!("../../migrations/020_custom_apps.sql"))?;
        conn.pragma_update(None, "user_version", 20)?;
        tracing::info!("Database migrated to version 20 (custom app fields)");
    }

    if current_version < 21 {
        conn.execute_batch(include_str!(
            "../../migrations/021_request_log_cache_split.sql"
        ))?;
        conn.pragma_update(None, "user_version", 21)?;
        tracing::info!("Database migrated to version 21 (request_logs cache split)");
    }

    Ok(())
}

#[cfg(test)]
mod token_monitor_migrations {
    use super::*;

    /// 空库依次执行 001→021，断言 user_version=21 且 token_monitor 表存在、列齐（02 §8 勾项 1/2）。
    #[test]
    fn migrations_run_to_021_and_create_token_monitor_tables() {
        let conn = Connection::open_in_memory().expect("open database");
        let db = std::sync::Mutex::new(conn);
        run_migrations(&db).expect("run migrations 001..021");

        let conn = db.lock().expect("lock database");
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("user_version");
        assert_eq!(version, 21, "user_version should reach 21");

        // 021: request_logs cache split columns exist.
        let cache_read: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('request_logs') \
                 WHERE name IN ('cache_read_tokens','cache_write_tokens')",
                [],
                |row| row.get(0),
            )
            .expect("check request_logs cache columns");
        assert_eq!(cache_read, 2, "cache split columns must exist");

        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' \
                 AND name IN ('tool_definition','usage_event','tm_session','tm_project',\
                              'quota_account','quota_window_snapshot','tm_daily_rollup',\
                              'tm_period_session')",
            )
            .expect("prepare table query")
            .query_map([], |row| row.get(0))
            .expect("query tables")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect tables");
        assert_eq!(
            tables.len(),
            8,
            "all eight token_monitor tables should exist: {tables:?}"
        );

        // usage_event 列齐
        let usage_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(usage_event)")
            .expect("prepare pragma")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query pragma")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect pragma");
        for col in [
            "source_type",
            "tool_id",
            "model_raw",
            "model_normalized",
            "session_id",
            "project_id",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_write_tokens",
            "total_tokens",
            "usage_accuracy",
            "occurred_at",
            "source_fingerprint",
            "source_locator_hash",
            "message_count",
            "session_started_at",
            "session_last_active_at",
        ] {
            assert!(
                usage_cols.contains(&col.to_string()),
                "missing column {col}"
            );
        }

        // tool_definition 列齐（020：custom_fields_json 供自定义应用字段映射）
        let tool_cols: Vec<String> = conn
            .prepare("PRAGMA table_info(tool_definition)")
            .expect("prepare pragma")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query pragma")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect pragma");
        for col in ["tool_id", "display_name", "custom_paths_json", "custom_fields_json"] {
            assert!(
                tool_cols.contains(&col.to_string()),
                "missing tool_definition column {col}"
            );
        }
    }

    /// usage_event 的 UNIQUE(source_fingerprint) 生效：重复 INSERT OR IGNORE 不增行（02 §8 勾项 3）。
    #[test]
    fn usage_event_source_fingerprint_dedupes() {
        let conn = Connection::open_in_memory().expect("open database");
        let db = std::sync::Mutex::new(conn);
        run_migrations(&db).expect("run migrations");
        let conn = db.lock().expect("lock database");

        conn.execute_batch(
            "INSERT INTO tool_definition (tool_id, display_name) VALUES ('claude_code', 'Claude Code');",
        )
        .expect("seed tool_definition");
        let row_sql = "INSERT OR IGNORE INTO usage_event \
            (source_type, tool_id, occurred_at, source_fingerprint, usage_accuracy) \
            VALUES ('local_discovered', 'claude_code', '2026-08-08T00:00:00Z', 'fp-a', 'exact')";
        conn.execute_batch(row_sql).expect("first insert");
        conn.execute_batch(row_sql)
            .expect("duplicate insert ignored");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM usage_event WHERE source_fingerprint='fp-a'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(count, 1, "duplicate fingerprint must not insert twice");
    }

    /// quota_window_snapshot 的 UNIQUE(account_id, window_key) upsert 覆盖（02 §8 勾项 4）。
    #[test]
    fn quota_window_snapshot_upsert_keeps_latest() {
        let conn = Connection::open_in_memory().expect("open database");
        let db = std::sync::Mutex::new(conn);
        run_migrations(&db).expect("run migrations");
        let conn = db.lock().expect("lock database");

        conn.execute_batch(
            "INSERT INTO quota_account (account_id, provider_id, auth_method) \
             VALUES ('tm_claude_1', 'claude', 'oauth');",
        )
        .expect("seed quota_account");
        conn.execute_batch(
            "INSERT INTO quota_window_snapshot \
                (account_id, window_key, window_type, unit, used_value, limit_value, source, fetched_at) \
             VALUES ('tm_claude_1', 'primary', 'rolling_5h', 'tokens', 100, 1000, 'official_api', '2026-08-08T00:00:00Z') \
             ON CONFLICT(account_id, window_key) DO UPDATE SET \
                used_value=excluded.used_value, limit_value=excluded.limit_value, \
                fetched_at=excluded.fetched_at;",
        )
        .expect("first upsert");
        conn.execute_batch(
            "INSERT INTO quota_window_snapshot \
                (account_id, window_key, window_type, unit, used_value, limit_value, source, fetched_at) \
             VALUES ('tm_claude_1', 'primary', 'rolling_5h', 'tokens', 300, 1000, 'official_api', '2026-08-08T01:00:00Z') \
             ON CONFLICT(account_id, window_key) DO UPDATE SET \
                used_value=excluded.used_value, limit_value=excluded.limit_value, \
                fetched_at=excluded.fetched_at;",
        )
        .expect("second upsert");
        let (count, used): (i64, f64) = conn
            .query_row(
                "SELECT COUNT(*), MAX(used_value) FROM quota_window_snapshot WHERE account_id='tm_claude_1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("query snapshot");
        assert_eq!(count, 1, "one row per (account_id, window_key)");
        assert_eq!(used, 300.0, "latest upsert wins");
    }

    /// 独立口径（用户决策）：Token Monitor 只统计 usage_event（本地工具），
    /// 网关流量（request_logs）由网关仪表盘独立统计。无论 request_logs 有无数据，
    /// TM 查询结果都只反映 usage_event —— 两个功能数据源分离、不重复。
    #[test]
    fn token_monitor_query_counts_local_events_only() {
        let gateway_rows = |conn: &Connection| {
            conn.execute_batch(
                "INSERT INTO request_logs \
                    (request_id, attempt_count, source, status, model, input_tokens, output_tokens, cache_tokens, request_at) \
                 VALUES
                    ('req-a', 1, 'proxy', 'error', 'gpt-4o', 99, 99, 99, datetime('now')),
                    ('req-a', 2, 'proxy', 'success', 'gpt-4o', 12, 8, 2, datetime('now'));",
            )
            .expect("insert gateway logs");
        };
        let local_rows = |conn: &Connection| {
            conn.execute_batch(
                "INSERT INTO tool_definition (tool_id, display_name) VALUES ('claude_code', 'Claude Code');",
            )
            .expect("seed tool_definition");
            conn.execute_batch(
                "INSERT INTO usage_event \
                    (source_type, tool_id, occurred_at, source_fingerprint, usage_accuracy, \
                     input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, total_tokens) \
                 VALUES ('local_discovered', 'claude_code', '2026-08-08T00:00:00Z', 'fp-1', 'exact', \
                         30, 20, 5, 0, 50);",
            )
            .expect("insert local event");
        };
        let tm_query = |conn: &Connection| -> (i64, i64, i64, i64) {
            conn.query_row(
                "SELECT COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                        COALESCE(SUM(COALESCE(cache_read_tokens,0)+COALESCE(cache_write_tokens,0)),0),
                        COALESCE(SUM(total_tokens),0)
                 FROM usage_event",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("tm query")
        };

        // 仅 gateway：request_logs 不进 TM → 全 0
        {
            let conn = Connection::open_in_memory().expect("open database");
            let db = std::sync::Mutex::new(conn);
            run_migrations(&db).expect("run migrations");
            let conn = db.lock().expect("lock database");
            gateway_rows(&conn);
            let (input, output, cache, total) = tm_query(&conn);
            assert_eq!((input, output, cache, total), (0, 0, 0, 0));
        }
        // 仅本地：usage_event 全量计（30+20, cache 5, total 50）
        {
            let conn = Connection::open_in_memory().expect("open database");
            let db = std::sync::Mutex::new(conn);
            run_migrations(&db).expect("run migrations");
            let conn = db.lock().expect("lock database");
            local_rows(&conn);
            let (input, output, cache, total) = tm_query(&conn);
            assert_eq!((input, output, cache, total), (30, 20, 5, 50));
        }
        // 两者皆有：仍只计 usage_event（网关行不影响 TM）
        {
            let conn = Connection::open_in_memory().expect("open database");
            let db = std::sync::Mutex::new(conn);
            run_migrations(&db).expect("run migrations");
            let conn = db.lock().expect("lock database");
            gateway_rows(&conn);
            local_rows(&conn);
            let (input, output, cache, total) = tm_query(&conn);
            assert_eq!((input, output, cache, total), (30, 20, 5, 50));
        }
    }
}
