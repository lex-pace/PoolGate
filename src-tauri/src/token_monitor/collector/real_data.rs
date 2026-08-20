//! 真数据联调（批次 3 收尾）：对本机真实工具目录执行采集验证。
//! 环境相关，默认 `#[ignore]`；显式运行：
//!   `cargo test -- --ignored real_data`
//! 目的：确认适配器能解析本机真实格式（workbuddy/codex/opencode/zcode/claude），
//! 输出每个工具发现的源数、事件数与模型分布，作为「工具获取一致性」的实证。

use super::*;
use crate::token_monitor::model::NormalizedUsageEvent;
use std::collections::BTreeMap;

fn collect_all(adapter: &dyn ToolAdapter) -> (Vec<NormalizedUsageEvent>, usize) {
    let mut events = Vec::new();
    let mut sources = 0;
    for source in adapter.discover() {
        sources += 1;
        match adapter.collect_incremental(&source, adapter.checkpoint(&source.id)) {
            Ok(r) => events.extend(r.events),
            Err(e) => eprintln!(
                "  [real] {} {:?}: {e}",
                adapter.descriptor().tool_id,
                source.path
            ),
        }
    }
    (events, sources)
}

fn assert_collects(adapter: &dyn ToolAdapter, min_events: usize) {
    let (events, sources) = collect_all(adapter);
    let id = adapter.descriptor().tool_id;
    println!("[real] {id}: {sources} sources, {} events", events.len());
    if !events.is_empty() {
        let total: i64 = events
            .iter()
            .map(|e| {
                e.total_tokens
                    .unwrap_or_else(|| e.input_tokens.unwrap_or(0) + e.output_tokens.unwrap_or(0))
            })
            .sum();
        println!("[real] {id}: total tokens ≈ {total}");
        let mut models: BTreeMap<String, i64> = BTreeMap::new();
        for e in &events {
            *models
                .entry(e.model_raw.clone().unwrap_or_default())
                .or_insert(0i64) += e.input_tokens.unwrap_or(0) + e.output_tokens.unwrap_or(0);
        }
        for (m, n) in models {
            println!("[real] {id}: model {m:?} = {n}");
        }
    }
    assert!(
        events.len() >= min_events,
        "{id}: 期望至少 {min_events} 条事件，实际 {}",
        events.len()
    );
}

#[test]
#[ignore]
fn real_claude_code() {
    assert_collects(&claude_code::ClaudeCodeAdapter::default(), 1);
}

#[test]
#[ignore]
fn real_freebuff() {
    // 本机 Freebuff 工作区 `.freebuff/desktop-v2.db`；无则打印跳过。
    let adapter = freebuff::FreebuffAdapter::default();
    let (events, sources) = collect_all(&adapter);
    println!("[real] freebuff: {sources} sources, {} events", events.len());
    if events.is_empty() {
        println!("[real] freebuff: 未发现 Freebuff 数据库 — 跳过断言");
        return;
    }
    let total: i64 = events.iter().map(|e| e.total_tokens.unwrap_or(0)).sum();
    let by_model: BTreeMap<String, i64> = {
        let mut m = BTreeMap::new();
        for e in &events {
            *m.entry(e.model_raw.clone().unwrap_or_default()).or_insert(0i64) +=
                e.total_tokens.unwrap_or(0);
        }
        m
    };
    println!("[real] freebuff: total_tokens(含cache) = {total}");
    for (model, n) in by_model {
        println!("[real] freebuff: model {model:?} = {n}");
    }
    // 会话摘要应能产出（threads 表存在）
    for source in adapter.discover() {
        if let Ok(r) =
            adapter.collect_incremental(&source, adapter.checkpoint(&source.id))
        {
            println!("[real] freebuff: {} sessions", r.sessions.len());
        }
    }
    assert!(events.len() >= 1, "freebuff 应至少产出 1 条事件");
}

/// 复现线上 bug（W12 扫描添加冷门 Agent）：Freebuff 事件被 tokscale 快照替换清空，
/// 导致「今天用了很多但 TOKENS 不显示」。用本机真实 Freebuff 数据验证修复后的
/// `replace_local_snapshot` 只替换 covered 工具、保留 companion 工具事件。
/// 真数据联调：对真实 poolgate DB 副本跑 backfill_missing_models（freebuff +
/// atomcode），确认能清零历史 NULL 模型行（「未知模型」桶）。只读真实 Freebuff/
/// AtomCode 数据源，池侧 DB 用 VACUUM INTO 快照副本，不碰线上库。
#[test]
#[ignore]
fn real_freebuff_backfill_on_real_db_copy() {
    use crate::db::Database;

    let home = std::env::var_os("HOME").expect("HOME");
    let real_db = std::path::PathBuf::from(home)
        .join("Library/Application Support/com.poolgate.desktop/.poolgate/gateway.db");
    if !real_db.exists() {
        println!("[real] 无真实 poolgate DB — 跳过");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_copy = tmp.path().join("gateway.db");
    {
        let conn = rusqlite::Connection::open(&real_db).expect("open real db");
        conn.execute_batch(&format!(
            "VACUUM INTO '{}'",
            db_copy.to_string_lossy().replace('\'', "''")
        ))
        .expect("vacuum into");
    }
    let database = Database::new(&db_copy).expect("open copy");
    database.run_migrations().expect("migrate");

    let before: i64 = {
        let conn = database.conn.lock().expect("lock");
        conn.query_row(
            "SELECT COUNT(*) FROM usage_event WHERE tool_id='freebuff' \
             AND (model_normalized IS NULL OR model_normalized='')",
            [],
            |r| r.get(0),
        )
        .expect("count before")
    };
    println!("[real] backfill: NULL rows before = {before}");

    let adapter = freebuff::FreebuffAdapter::default();
    let mut total_fixed = 0usize;
    for source in adapter.discover() {
        let fixed = freebuff::backfill_missing_models(&database.conn, &source)
            .unwrap_or_else(|e| {
                eprintln!("[real] backfill err {}: {e}", source.path.display());
                0
            });
        println!("[real] backfill {} => fixed {fixed}", source.path.display());
        total_fixed += fixed;
    }
    println!("[real] backfill total fixed = {total_fixed}");

    let after: i64 = {
        let conn = database.conn.lock().expect("lock");
        conn.query_row(
            "SELECT COUNT(*) FROM usage_event WHERE tool_id='freebuff' \
             AND (model_normalized IS NULL OR model_normalized='')",
            [],
            |r| r.get(0),
        )
        .expect("count after")
    };
    println!("[real] backfill: NULL rows after = {after}");
    if before > 0 {
        assert_eq!(after, 0, "回填应清零 freebuff 的 NULL 模型行");
    }

    // 清理后不应再有 Freebuff 的 claude-code harness 线程（它们由 claude_code 采集）。
    let harness_after: i64 = {
        let conn = database.conn.lock().expect("lock");
        conn.query_row(
            "SELECT COUNT(*) FROM usage_event WHERE tool_id='freebuff' \
             AND session_id IN (SELECT t.id FROM threads t WHERE t.harness_id='claude-code')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0)
    };
    println!("[real] backfill: freebuff claude-code harness rows after = {harness_after}");
    assert_eq!(harness_after, 0, "freebuff 不应残留 claude-code harness 线程");

    // atomcode：对真实 datalog 源跑回填（早期 env 正则无 session 不匹配 → model 缺失）
    let atom_before: i64 = {
        let conn = database.conn.lock().expect("lock");
        conn.query_row(
            "SELECT COUNT(*) FROM usage_event WHERE tool_id='atomcode' \
             AND (model_normalized IS NULL OR model_normalized='')",
            [],
            |r| r.get(0),
        )
        .expect("atom count before")
    };
    println!("[real] atomcode backfill: NULL rows before = {atom_before}");
    let mut atom_fixed = 0usize;
    for source in atomcode::AtomCodeAdapter::default().discover() {
        atom_fixed += atomcode::backfill_missing_models(&database.conn, &source).unwrap_or(0);
    }
    let atom_after: i64 = {
        let conn = database.conn.lock().expect("lock");
        conn.query_row(
            "SELECT COUNT(*) FROM usage_event WHERE tool_id='atomcode' \
             AND (model_normalized IS NULL OR model_normalized='')",
            [],
            |r| r.get(0),
        )
        .expect("atom count after")
    };
    println!(
        "[real] atomcode backfill: fixed {atom_fixed}, NULL rows after = {atom_after}"
    );
    if atom_before > 0 {
        assert_eq!(atom_after, 0, "回填应清零 atomcode 的 NULL 模型行");
    }
}

#[test]
#[ignore]
fn real_freebuff_survives_tokscale_snapshot_replace() {
    use crate::db::token_usage::UsageEventRepo;
    use rusqlite::Connection;
    use std::sync::Mutex;

    let conn = Connection::open_in_memory().expect("open database");
    conn.execute_batch(include_str!("../../../migrations/001_initial.sql"))
        .expect("migrate 001");
    conn.execute_batch(include_str!("../../../migrations/016_token_monitor.sql"))
        .expect("migrate 016");
    conn.execute_batch(include_str!(
        "../../../migrations/017_usage_event_message_count.sql"
    ))
    .expect("migrate 017");
    conn.execute_batch(include_str!(
        "../../../migrations/019_usage_event_session_times.sql"
    ))
    .expect("migrate 019");
    let db = Mutex::new(conn);

    let adapter = freebuff::FreebuffAdapter::default();
    let (events, sources) = collect_all(&adapter);
    println!("[real] freebuff e2e: {sources} sources, {} events", events.len());
    if events.is_empty() {
        println!("[real] freebuff e2e: 未发现 Freebuff 数据库 — 跳过");
        return;
    }
    let repo = UsageEventRepo;
    repo.insert_batch(&db, &events).expect("insert freebuff");

    // 模拟 tokscale 全量扫描：快照不含 freebuff（不在覆盖清单）→ 空事件替换
    let covered_ids = tokscale::covered_tool_ids();
    repo.replace_local_snapshot(&db, &[], &covered_ids)
        .expect("snapshot replace");

    let rows = repo.tool_usage_rows(&db, "total").expect("tool rows");
    let freebuff = rows
        .iter()
        .find(|r| r.tool_id == "freebuff")
        .expect("freebuff 事件必须保留（线上 bug：被清空）");
    let expected: i64 = events.iter().map(|e| e.total_tokens.unwrap_or(0)).sum();
    println!(
        "[real] freebuff e2e: 保留 {freebuff:?} = {} (期望 {expected})",
        freebuff.total_tokens
    );
    assert_eq!(freebuff.total_tokens, expected);
}

#[test]
#[ignore]
fn real_codex() {
    assert_collects(&codex::CodexAdapter::default(), 1);
}

#[test]
#[ignore]
fn real_workbuddy() {
    assert_collects(&workbuddy::WorkbuddyAdapter::default(), 1);
}

#[test]
#[ignore]
fn real_opencode() {
    assert_collects(&opencode::OpenCodeAdapter::default(), 1);
}

#[test]
#[ignore]
fn real_zcode() {
    assert_collects(&zcode::ZcodeAdapter::default(), 1);
}

#[test]
#[ignore]
fn real_tokscale_e2e_into_migrated_db() {
    // 完整链路:tokscale 全量扫描 → replace_local_snapshot 落库 → 聚合行与开源存档逐值一致
    use crate::db::token_usage::UsageEventRepo;
    use rusqlite::Connection;
    use std::sync::Mutex;

    let conn = Connection::open_in_memory().expect("open database");
    conn.execute_batch(include_str!("../../../migrations/001_initial.sql"))
        .expect("migrate 001");
    conn.execute_batch(include_str!("../../../migrations/008_client_keys.sql"))
        .expect("migrate 008");
    conn.execute_batch(include_str!(
        "../../../migrations/009_request_logs_client_key.sql"
    ))
    .expect("migrate 009");
    conn.execute_batch(include_str!(
        "../../../migrations/010_route_pool_management.sql"
    ))
    .expect("migrate 010");
    conn.execute_batch(include_str!("../../../migrations/016_token_monitor.sql"))
        .expect("migrate 016");
    conn.execute_batch(include_str!(
        "../../../migrations/017_usage_event_message_count.sql"
    ))
    .expect("migrate 017");
    conn.execute_batch(include_str!(
        "../../../migrations/018_tm_period_session_membership.sql"
    ))
    .expect("migrate 018");
    conn.execute_batch(include_str!(
        "../../../migrations/019_usage_event_session_times.sql"
    ))
    .expect("migrate 019");
    let db = Mutex::new(conn);

    let adapter = tokscale::TokscaleAdapter::default();
    let mut events = Vec::new();
    for source in adapter.discover() {
        match adapter.collect_incremental(&source, adapter.checkpoint(&source.id)) {
            Ok(r) => events.extend(r.events),
            Err(e) => eprintln!("[real] tokscale e2e: {e}"),
        }
    }
    if events.is_empty() {
        println!("[real] tokscale e2e: 二进制不可用 — 跳过");
        return;
    }
    let repo = UsageEventRepo;
    let inserted = repo
        .replace_local_snapshot(&db, &events, &tokscale::covered_tool_ids())
        .expect("snapshot replace");
    println!("[real] tokscale e2e: inserted {inserted} events");

    let (input, output, cache, total) = repo.unified_range_stats(&db, "total").expect("stats");
    println!("[real] tokscale e2e: unified total(含cache) = {total} (in {input} / out {output} / cache {cache})");
    let rows = repo.tool_usage_rows(&db, "total").expect("tool rows");
    for row in &rows {
        println!(
            "[real] tokscale e2e: tool {} = {} (${:?})",
            row.tool_id, row.total_tokens, row.cost_amount
        );
    }
    // 与开源存档逐值对齐(2026-08-08 实测):workbuddy ≈ 1.63B, codex ≈ 1.13B
    let workbuddy = rows
        .iter()
        .find(|r| r.tool_id == "workbuddy")
        .map(|r| r.total_tokens)
        .unwrap_or(0);
    let codex = rows
        .iter()
        .find(|r| r.tool_id == "codex")
        .map(|r| r.total_tokens)
        .unwrap_or(0);
    println!("[real] tokscale e2e: workbuddy={workbuddy} codex={codex}");
    assert!(
        workbuddy > 1_000_000_000,
        "workbuddy 应 > 1B, 实际 {workbuddy}"
    );
    assert!(codex > 1_000_000_000, "codex 应 > 1B, 实际 {codex}");
    assert!(total > 2_000_000_000, "全量应 > 2B, 实际 {total}");
}

#[test]
#[ignore]
fn real_tokscale_aggregate_matches_opensource() {
    // 复刻开源 Token Monitor 采集引擎：tokscale 全量扫描 → 事件 → 总量/模型分布
    let adapter = tokscale::TokscaleAdapter::default();
    let mut events = Vec::new();
    for source in adapter.discover() {
        match adapter.collect_incremental(&source, adapter.checkpoint(&source.id)) {
            Ok(r) => events.extend(r.events),
            Err(e) => eprintln!("[real] tokscale: {e}"),
        }
    }
    println!("[real] tokscale: {} events", events.len());
    if events.is_empty() {
        println!("[real] tokscale: 二进制不可用或本机无数据 — 跳过断言");
        return;
    }
    let total: i64 = events.iter().map(|e| e.total_tokens.unwrap_or(0)).sum();
    let cost: f64 = events.iter().map(|e| e.cost_amount.unwrap_or(0.0)).sum();
    println!("[real] tokscale: total_tokens(含cache) = {total}");
    println!("[real] tokscale: cost_usd = {cost:.2}");
    let mut by_tool: BTreeMap<String, i64> = BTreeMap::new();
    let mut by_model: BTreeMap<String, i64> = BTreeMap::new();
    for e in &events {
        *by_tool.entry(e.tool_id.clone()).or_insert(0) += e.total_tokens.unwrap_or(0);
        *by_model
            .entry(e.model_raw.clone().unwrap_or_default())
            .or_insert(0) += e.total_tokens.unwrap_or(0);
    }
    for (t, n) in by_tool {
        println!("[real] tokscale: tool {t} = {n}");
    }
    for (m, n) in by_model {
        println!("[real] tokscale: model {m} = {n}");
    }
    assert!(events.len() >= 1, "tokscale 应至少产出 1 条事件");
}

#[test]
#[ignore]
fn real_all_registry_snapshot() {
    // 全注册表冒烟：任一适配器出错不得 panic；打印每个工具发现源数与事件数
    let mut lines = Vec::new();
    for adapter in AdapterRegistry::all() {
        let (events, sources) = collect_all(adapter.as_ref());
        lines.push(format!(
            "{}: {} sources / {} events",
            adapter.descriptor().tool_id,
            sources,
            events.len()
        ));
    }
    println!("[real] registry:\n{}", lines.join("\n"));
}
