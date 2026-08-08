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

    Ok(())
}
