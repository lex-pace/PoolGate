//! 每日历史归档 —— 对齐开源 Token Monitor 的 `daily-history-archive.json`。
//!
//! 背景：`tokscale graph` 只返回「源会话文件仍存在」的活跃日；工具一旦清理了历史
//! 会话文件，那些活跃日就会从 graph 中消失，导致「活跃天数 / 连续天数」随时间缩水
//! （PoolGate 95 vs 开源 96）。开源版用 `daily-history-archive.json` 持久化**每一个
//! 观察到的活跃日**，源文件被清理后仍不丢。
//!
//! 这里在 PoolGate 的 `settings` 表用等价结构实现，并把自定义工具（`custom:*`）与
//! companion 工具（freebuff / atomcode / dsh）作为「工具维度」一并归档——它们也走
//! 同一份逐日历史，不会因 usage_event 快照替换 / 源文件清理而丢失。

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::db::settings::SettingsRepo;
use crate::token_monitor::model::{SeriesSplit, TrendDay, TrendSeries};

/// 归档 JSON 存储键（settings 表）。
const ARCHIVE_KEY: &str = "tm.daily_history_archive";
/// 开源 Token Monitor 归档播种标记（只做一次，幂等）。
const SEEDED_KEY: &str = "tm.daily_history.seeded_from_token_monitor";

/// 读取归档（date → TrendDay）。缺失 / 损坏返回空，绝不 panic。
fn read_archive(conn: &Mutex<Connection>) -> BTreeMap<String, TrendDay> {
    let json = match SettingsRepo.get(conn, ARCHIVE_KEY) {
        Ok(Some(json)) => json,
        _ => return BTreeMap::new(),
    };
    serde_json::from_str::<BTreeMap<String, TrendDay>>(&json).unwrap_or_default()
}

/// 写回归档。序列化失败时静默跳过（归档是尽力而为的耐久层，不阻塞趋势查询）。
fn write_archive(conn: &Mutex<Connection>, days: &BTreeMap<String, TrendDay>) {
    if let Ok(json) = serde_json::to_string(days) {
        let _ = SettingsRepo.set(conn, ARCHIVE_KEY, &json);
    }
}

/// 把当前 live 日并入归档（按日保留 tokens 更大的观测），返回是否有变化。
fn capture_into_archive(archive: &mut BTreeMap<String, TrendDay>, series: &TrendSeries) -> bool {
    let mut changed = false;
    for day in &series.daily {
        if day.tokens <= 0 {
            continue;
        }
        match archive.get(&day.date) {
            Some(prev) if prev.tokens >= day.tokens => {}
            _ => {
                archive.insert(day.date.clone(), day.clone());
                changed = true;
            }
        }
    }
    changed
}

/// 合并归档 + 捕获 live + 重算汇总（`trend_series` 的唯一入口）。
///
/// 顺序：
///   1. 一次性播种开源 Token Monitor 的历史归档（对齐历史活跃天数）；
///   2. 把当前 live 日并入归档（不缩水）；
///   3. 把归档中「live 已丢失」的日补回 `series.daily`；
///   4. 用合并后的日序列重算 active_days / streak_days / peak_day / monthly / active_time。
pub(crate) fn merge_and_capture(
    conn: &Mutex<Connection>,
    series: &mut TrendSeries,
) -> Result<(), String> {
    let _ = seed_from_token_monitor_once(conn);

    let mut archive = read_archive(conn);
    let changed = capture_into_archive(&mut archive, series);
    if changed {
        write_archive(conn, &archive);
    }

    // 归档里「live 已丢失」的日补回（源文件清理后活跃天数不缩水）。
    let live: HashSet<String> = series.daily.iter().map(|d| d.date.clone()).collect();
    let mut added_messages = 0i64;
    for (date, day) in &archive {
        if day.tokens <= 0 || live.contains(date) {
            continue;
        }
        added_messages += day.requests.max(0);
        series.daily.push(day.clone());
    }
    series.daily.sort_by(|a, b| a.date.cmp(&b.date));

    // 活跃天数 / 连续天数 / 峰值单日 / 月度：全历史口径（对齐开源 history.js）。
    let active_days = series.daily.iter().filter(|d| d.tokens > 0).count() as i64;
    let peak_day = series
        .daily
        .iter()
        .filter(|d| d.tokens > 0)
        .max_by_key(|d| d.tokens)
        .cloned();

    let active_set: HashSet<String> = series
        .daily
        .iter()
        .filter(|d| d.tokens > 0)
        .map(|d| d.date.clone())
        .collect();
    let mut streak_days = 0i64;
    let mut cursor = local_today();
    loop {
        if active_set.contains(&cursor) {
            streak_days += 1;
            cursor = prev_local_day(&cursor);
        } else {
            break;
        }
    }

    let mut months: BTreeMap<String, i64> = BTreeMap::new();
    for day in series.daily.iter().filter(|d| d.tokens > 0) {
        if day.date.len() >= 7 {
            *months.entry(day.date[..7].to_string()).or_default() += day.tokens;
        }
    }

    // 活跃时间：timeMetrics 总时长 vs 合并后逐日之和，取较大者（对齐开源 graphTimeMetrics）。
    let sum_active = series
        .daily
        .iter()
        .map(|d| d.active_time_ms.max(0))
        .sum::<i64>();
    series.active_days = active_days;
    series.streak_days = streak_days;
    series.peak_day = peak_day;
    series.monthly = months
        .into_iter()
        .map(|(month, tokens)| crate::token_monitor::model::TrendMonth { month, tokens })
        .collect();
    series.active_time_ms = series.active_time_ms.max(sum_active);
    // 归档补回的日携带的消息数计入累计（live 部分已在 build/companion 阶段计入）。
    series.message_count = series.message_count.max(0) + added_messages;
    Ok(())
}

/// 本地日历今天（YYYY-MM-DD）。
fn local_today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// 前一天的本地日历（YYYY-MM-DD）。
fn prev_local_day(day: &str) -> String {
    if let Ok(date) = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d") {
        return (date - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
    }
    day.to_string()
}

// ---------------------------------------------------------------------------
// 开源 Token Monitor 归档播种（对齐历史活跃天数）
// ---------------------------------------------------------------------------

/// 测试不读真实文件系统（避免本机开源归档污染断言）。
#[cfg(test)]
fn token_monitor_shared_dir() -> Option<PathBuf> {
    None
}

/// 开源 Token Monitor 的共享数据目录（与 `shared/config.js::sharedDataDir` 一致）。
#[cfg(not(test))]
fn token_monitor_shared_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("TOKEN_MONITOR_SHARED_DIR") {
        if !dir.trim().is_empty() {
            return Some(PathBuf::from(dir.trim()));
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    #[cfg(target_os = "macos")]
    {
        Some(
            home.join("Library")
                .join("Application Support")
                .join("Token Monitor"),
        )
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        let root = if appdata.trim().is_empty() {
            home.join("AppData").join("Roaming")
        } else {
            PathBuf::from(appdata.trim())
        };
        Some(root.join("Token Monitor"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let xdg = std::env::var("XDG_CONFIG_HOME").unwrap_or_default();
        let root = if xdg.trim().is_empty() {
            home.join(".config")
        } else {
            PathBuf::from(xdg.trim())
        };
        Some(root.join("Token Monitor"))
    }
}

/// 把开源 Token Monitor 归档的一个「日」转成 `TrendDay`（纯函数，便于单测）。
///
/// 开源结构：`{ activeTimeMs, observations: { key: { client, modelId, tokens, cost, messages } } }`。
/// 客户端 id → tool_id、模型别名归一与 live graph 口径一致，保证按工具/按模型拆分可对齐。
fn convert_token_monitor_day(date: &str, day: &serde_json::Value) -> Option<TrendDay> {
    let active_time_ms = day
        .get("activeTimeMs")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        .max(0);
    let observations = day.get("observations").and_then(|v| v.as_object())?;
    if observations.is_empty() {
        return None;
    }

    let mut tokens = 0i64;
    let mut cost = 0.0f64;
    let mut messages = 0i64;
    let mut per_client: BTreeMap<String, i64> = BTreeMap::new();
    let mut per_model: BTreeMap<String, i64> = BTreeMap::new();

    for obs in observations.values() {
        let t = obs
            .get("tokens")
            .and_then(|v| v.as_f64())
            .map(|v| v.round() as i64)
            .unwrap_or(0)
            .max(0);
        if t == 0 {
            continue;
        }
        tokens += t;
        cost += obs.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);
        messages += obs
            .get("messages")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);

        if let Some(client) = obs.get("client").and_then(|v| v.as_str()) {
            if !client.is_empty() {
                let tool_id = crate::token_monitor::collector::tokscale::tool_id_for_client(client);
                *per_client.entry(tool_id).or_default() += t;
            }
        }
        if let Some(model) = obs.get("modelId").and_then(|v| v.as_str()) {
            if !model.is_empty() {
                let canonical =
                    crate::token_monitor::normalization::alias_model(model).unwrap_or(model);
                *per_model.entry(canonical.to_string()).or_default() += t;
            }
        }
    }
    if tokens <= 0 {
        return None;
    }

    let split = |map: BTreeMap<String, i64>| {
        let mut list: Vec<SeriesSplit> = map
            .into_iter()
            .map(|(key, tokens)| SeriesSplit { key, tokens })
            .collect();
        list.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.key.cmp(&b.key)));
        list
    };

    Some(TrendDay {
        date: date.to_string(),
        tokens,
        requests: messages,
        cost_amount: if cost > 0.0 { Some(cost) } else { None },
        active_time_ms,
        per_client: Some(split(per_client)),
        per_model: Some(split(per_model)),
    })
}

/// 一次性把开源 Token Monitor 的 `daily-history-archive.json` 播种进 PoolGate 归档。
/// 幂等：`SEEDED_KEY` 存在即跳过；文件缺失 / 解析失败也标记为已播种（不反复读文件）。
fn seed_from_token_monitor_once(conn: &Mutex<Connection>) -> Result<(), String> {
    if SettingsRepo.get(conn, SEEDED_KEY)?.is_some() {
        return Ok(());
    }
    let Some(dir) = token_monitor_shared_dir() else {
        let _ = SettingsRepo.set(conn, SEEDED_KEY, "1");
        return Ok(());
    };
    let path = dir.join("daily-history-archive.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        let _ = SettingsRepo.set(conn, SEEDED_KEY, "1");
        return Ok(());
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        let _ = SettingsRepo.set(conn, SEEDED_KEY, "1");
        return Ok(());
    };

    let mut archive = read_archive(conn);
    let mut changed = false;
    if let Some(days) = json.get("days").and_then(|v| v.as_object()) {
        for (date, day) in days {
            let Some(converted) = convert_token_monitor_day(date, day) else {
                continue;
            };
            match archive.get(date) {
                Some(prev) if prev.tokens >= converted.tokens => {}
                _ => {
                    archive.insert(date.clone(), converted);
                    changed = true;
                }
            }
        }
    }
    if changed {
        write_archive(conn, &archive);
    }
    let _ = SettingsRepo.set(conn, SEEDED_KEY, "1");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("open memory db");
        conn.execute_batch(include_str!("../../migrations/001_initial.sql"))
            .expect("create settings table");
        Mutex::new(conn)
    }

    fn day(date: &str, tokens: i64, active_time_ms: i64) -> TrendDay {
        TrendDay {
            date: date.to_string(),
            tokens,
            requests: 0,
            cost_amount: None,
            active_time_ms,
            per_client: None,
            per_model: None,
        }
    }

    #[test]
    fn archive_retains_days_when_live_series_shrinks() {
        let conn = test_conn();
        let today = local_today();
        let yesterday = prev_local_day(&today);
        let mut series = TrendSeries {
            daily: vec![day(&yesterday, 100, 60_000), day(&today, 200, 120_000)],
            active_days: 2,
            streak_days: 2,
            peak_day: Some(day(&today, 200, 120_000)),
            monthly: vec![],
            active_time_ms: 180_000,
            message_count: 0,
        };
        merge_and_capture(&conn, &mut series).expect("merge");

        // 首轮捕获后归档包含两天。
        let archived = read_archive(&conn);
        assert_eq!(archived.len(), 2);

        // 模拟源文件清理：live graph 少了一天（昨天消失）。
        let mut shrunk = TrendSeries {
            daily: vec![day(&today, 200, 120_000)],
            ..TrendSeries::default()
        };
        shrunk.active_time_ms = 120_000;
        merge_and_capture(&conn, &mut shrunk).expect("merge again");

        // 活跃天数 / 连续天数不缩水。
        assert_eq!(shrunk.active_days, 2);
        assert_eq!(shrunk.streak_days, 2);
        assert_eq!(shrunk.daily.len(), 2);
        // 活跃时间回退为 max(live, 归档逐日之和)。
        assert_eq!(shrunk.active_time_ms, 180_000);
    }

    #[test]
    fn archive_keeps_richer_day_observation() {
        let conn = test_conn();
        let mut series = TrendSeries {
            daily: vec![day("2026-08-17", 100, 60_000)],
            ..TrendSeries::default()
        };
        merge_and_capture(&conn, &mut series).expect("merge");

        // 更小的 live 观测不应把归档里的更大值覆盖掉。
        let mut smaller = TrendSeries {
            daily: vec![day("2026-08-17", 50, 30_000)],
            ..TrendSeries::default()
        };
        merge_and_capture(&conn, &mut smaller).expect("merge smaller");
        let archived = read_archive(&conn);
        assert_eq!(archived.get("2026-08-17").map(|d| d.tokens), Some(100));

        // 当天从 live 完全消失后，归档保留的更大观测被补回。
        let mut gone = TrendSeries::default();
        merge_and_capture(&conn, &mut gone).expect("merge gone");
        assert_eq!(gone.daily.len(), 1);
        assert_eq!(gone.daily[0].tokens, 100);
        assert_eq!(gone.active_days, 1);
    }

    #[test]
    fn converts_token_monitor_archive_day_to_trend_day() {
        let json = serde_json::json!({
            "activeTimeMs": 722705,
            "observations": {
                "[\"zcode\",\"mimo-v2.5\"]": {
                    "client": "zcode",
                    "modelId": "mimo-v2.5",
                    "providerId": "zhipu",
                    "tokens": 11162859.0,
                    "cost": 0.110791268,
                    "messages": 68
                },
                "[\"zcode\",\"deepseek-v4-flash\"]": {
                    "client": "zcode",
                    "modelId": "deepseek-v4-flash",
                    "providerId": "zhipu",
                    "tokens": 448706.0,
                    "cost": 0.0173187784,
                    "messages": 22
                }
            }
        });
        let day = convert_token_monitor_day("2026-07-04", &json).expect("convert");
        assert_eq!(day.date, "2026-07-04");
        assert_eq!(day.tokens, 11_162_859 + 448_706);
        assert_eq!(day.requests, 90);
        assert_eq!(day.active_time_ms, 722_705);
        let clients = day.per_client.as_ref().expect("per_client");
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].key, "zcode");
        assert_eq!(clients[0].tokens, day.tokens);
        let models = day.per_model.as_ref().expect("per_model");
        assert_eq!(models.len(), 2);
    }

    #[test]
    fn ignores_zero_token_archive_day() {
        let json = serde_json::json!({
            "activeTimeMs": 0,
            "observations": {
                "[\"zcode\",\"mimo-v2.5\"]": {
                    "client": "zcode",
                    "modelId": "mimo-v2.5",
                    "tokens": 0,
                    "cost": 0,
                    "messages": 0
                }
            }
        });
        assert!(convert_token_monitor_day("2026-07-04", &json).is_none());
    }
}
