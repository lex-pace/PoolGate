//! 采集适配器共享工具（W3 内部共用，非契约文件，由集成负责人维护）。
//!
//! 提供：JSONL 增量读取（轮转感知）、通用 usage 字段提取、时间归一化、
//! 路径不可逆 hash、Claude Code 路径解码。其余 21 个 Adapter 复用本模块，
//! 各自只实现 discover + 字段映射。

use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use sha2::Digest;

use crate::token_monitor::collector::CollectResult;
use crate::token_monitor::model::{
    CollectorCheckpoint, CollectorError, DataSource, NormalizedUsageEvent, SessionSummary,
    SourceType, UsageAccuracy,
};

/// 路径不可逆 hash（隐私红线：DB 只存 hash，不存明文路径）。前 12 字节 hex。
pub(crate) fn hash_short(input: &str) -> String {
    let digest = sha2::Sha256::digest(input.as_bytes());
    hex::encode(&digest[..12])
}

// ---------------------------------------------------------------------------
// JSONL 增量读取（样板① 范式，全部 JSONL Adapter 复用）
// ---------------------------------------------------------------------------

/// 轮转感知的 JSONL 增量读取：
/// - `cp.inode` 与当前 inode 不同 → 从 0 重读（文件轮转/重建）。
/// - 否则从 `cp.byte_offset` seek 续读。
/// - 不完整的尾行丢弃，offset 停在最后一个完整换行处。
///
/// 返回 (完整行, 新 byte_offset, 当前 inode)。
#[cfg(unix)]
pub(crate) fn read_jsonl_incremental(
    path: &Path,
    cp: &CollectorCheckpoint,
) -> Result<(Vec<String>, u64, Option<u64>), CollectorError> {
    use std::os::unix::fs::MetadataExt;

    let file = std::fs::File::open(path)
        .map_err(|e| CollectorError::PathMissing(format!("{path:?}: {e}")))?;
    let metadata = file.metadata().ok();
    let inode = metadata.as_ref().map(|m| m.ino());
    let mut start = if cp.inode.is_some() && cp.inode == inode {
        cp.byte_offset.unwrap_or(0)
    } else {
        0
    };
    // 未轮转但被截断（offset 越界）→ 从 0 重读，避免在文件中间续读产生脏解析
    if let Some(len) = metadata.map(|m| m.len()) {
        if start > len {
            start = 0;
        }
    }
    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(start))
        .map_err(|e| CollectorError::Io(format!("seek {path:?}: {e}")))?;

    let mut lines = Vec::new();
    let mut cursor = start;
    loop {
        let mut buf = Vec::new();
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| CollectorError::Io(format!("read {path:?}: {e}")))?;
        if n == 0 {
            break;
        }
        if buf.last() == Some(&b'\n') {
            // 只留完整行；行内容可能含换行转义，直接截掉结尾 \n
            let line = String::from_utf8_lossy(&buf[..buf.len() - 1]).to_string();
            lines.push(line);
            cursor += buf.len() as u64;
        } else {
            // 不完整尾行（文件正在写入）→ 跳过，不推进 offset
            break;
        }
    }
    Ok((lines, cursor, inode))
}

#[cfg(not(unix))]
pub(crate) fn read_jsonl_incremental(
    path: &Path,
    cp: &CollectorCheckpoint,
) -> Result<(Vec<String>, u64, Option<u64>), CollectorError> {
    let file = std::fs::File::open(path)
        .map_err(|e| CollectorError::PathMissing(format!("{path:?}: {e}")))?;
    let start = cp.byte_offset.unwrap_or(0);
    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(start))
        .map_err(|e| CollectorError::Io(format!("seek {path:?}: {e}")))?;
    let mut lines = Vec::new();
    let mut cursor = start;
    loop {
        let mut buf = Vec::new();
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| CollectorError::Io(format!("read {path:?}: {e}")))?;
        if n == 0 {
            break;
        }
        if buf.last() == Some(&b'\n') {
            lines.push(String::from_utf8_lossy(&buf[..buf.len() - 1]).to_string());
            cursor += buf.len() as u64;
        } else {
            break;
        }
    }
    Ok((lines, cursor, None))
}

// ---------------------------------------------------------------------------
// 时间归一化
// ---------------------------------------------------------------------------

/// 转 UTC ISO8601（秒精度）。支持 epoch 秒/毫秒、RFC3339、常见本地格式。
/// 无法识别时返回 None（事件丢弃该行，不 panic）。
pub(crate) fn to_utc_iso(value: &serde_json::Value) -> Option<String> {
    if let Some(n) = value.as_i64() {
        // 单位启发：纳秒(>1e15) → 毫秒(>1e12) → 秒
        let secs = if n.abs() > 1_000_000_000_000_000 {
            n / 1_000_000_000
        } else if n.abs() > 1_000_000_000_000 {
            n / 1_000
        } else {
            n
        };
        let dt = Utc.timestamp_opt(secs, 0).single()?;
        return Some(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    if let Some(n) = value.as_f64() {
        let secs = if n.abs() > 1_000_000_000_000_000.0 {
            (n / 1_000_000_000.0) as i64
        } else if n.abs() > 1_000_000_000_000.0 {
            (n / 1_000.0) as i64
        } else {
            n as i64
        };
        let dt = Utc.timestamp_opt(secs, 0).single()?;
        return Some(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    let raw = value.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(
            dt.with_timezone(&Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(
                naive
                    .and_utc()
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            );
        }
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Some(
            date.and_hms_opt(0, 0, 0)?
                .and_utc()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );
    }
    // 已是 ISO 形态（无时区后缀等）→ 原样透传，替换空格
    Some(raw.replace(' ', "T"))
}

// ---------------------------------------------------------------------------
// 通用 usage 字段提取（按路径列表依次尝试）
// ---------------------------------------------------------------------------

/// 某工具一行 JSON 的 usage 字段路径表。路径以 `/` 开头 = JSON Pointer，
/// 否则为顶层键名。
pub(crate) struct GenericFields {
    pub input: &'static [&'static str],
    pub output: &'static [&'static str],
    pub cache: &'static [&'static str],
    pub model: &'static [&'static str],
    pub ts: &'static [&'static str],
    pub cost: &'static [&'static str],
}

pub(crate) fn pick<'a, S: AsRef<str>>(
    value: &'a serde_json::Value,
    paths: &[S],
) -> Option<&'a serde_json::Value> {
    for path in paths {
        let path = path.as_ref();
        let hit = if let Some(pointer) = path.strip_prefix('/') {
            value.pointer(&format!("/{pointer}"))
        } else {
            value.get(path)
        };
        if hit.is_some() {
            return hit;
        }
    }
    None
}

/// 从一行 JSON 提取 usage 事件；无任何 token 字段则返回 None（非 usage 行）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn parse_generic_event(
    value: &serde_json::Value,
    fields: &GenericFields,
    tool_id: &str,
    device_id: &str,
    session_id: Option<String>,
    project_id: Option<String>,
) -> Option<NormalizedUsageEvent> {
    let input = pick(value, fields.input).and_then(|v| v.as_i64());
    let output = pick(value, fields.output).and_then(|v| v.as_i64());
    if input.is_none() && output.is_none() {
        return None; // 非 usage 行（元数据/事件行），跳过
    }
    let occurred_at = pick(value, fields.ts)
        .and_then(to_utc_iso)
        .unwrap_or_else(|| Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    let model_raw = pick(value, fields.model)
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let cache = pick(value, fields.cache).and_then(|v| v.as_i64());
    let cost = pick(value, fields.cost)
        .and_then(|v| v.as_f64())
        .or_else(|| {
            pick(value, fields.cost)
                .and_then(|v| v.as_i64())
                .map(|n| n as f64)
        });
    Some(NormalizedUsageEvent {
        source_type: SourceType::LocalDiscovered,
        tool_id: tool_id.into(),
        device_id: device_id.into(),
        model_raw,
        model_normalized: None,
        session_id,
        project_id,
        account_id: None,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache,
        cache_write_tokens: None,
        reasoning_tokens: None,
        message_count: None,
        session_started_at: None,
        session_last_active_at: None,
        total_tokens: None,
        cost_amount: cost,
        cost_currency: None,
        usage_accuracy: UsageAccuracy::Exact,
        occurred_at,
        source_locator_hash: None,
    })
}

// ---------------------------------------------------------------------------
// 通用 JSONL 增量采集（W3b/W3c 多数 Adapter 直接复用）
// ---------------------------------------------------------------------------

/// 通用 JSONL 增量采集：
/// - 每个文件 = 一个 session（`external_session_id` = 文件名去扩展名）。
/// - 项目 = 父目录名（hash 入库，display 名取还原 basename）。
/// - `project_namer` 可对父目录名做解码（如 Claude Code 编码）。
pub(crate) fn collect_jsonl_incremental(
    source: &DataSource,
    cp: &CollectorCheckpoint,
    fields: &GenericFields,
    tool_id: &str,
    device_id: &str,
    session_enabled: bool,
    project_enabled: bool,
    project_namer: fn(&str) -> String,
) -> Result<CollectResult, CollectorError> {
    let (lines, new_offset, inode) = read_jsonl_incremental(&source.path, cp)?;
    if lines.is_empty() {
        // 无新行且启用会话摘要：若文件内容自上次会话落库后变化（或首次升级），全量重读
        // 重建会话摘要（迁移场景：session=false 时消费过的文件，升级后首扫即补上会话）。
        // 用 mtime 守卫：空闲轮询不再重读/重发（防事件风暴）；空文件（offset=0）不重建（防死循环）。
        if session_enabled && new_offset > 0 {
            let mtime = std::fs::metadata(&source.path)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0)
                });
            if mtime != cp.mtime_ms {
                let full = collect_jsonl_incremental(
                    source,
                    &CollectorCheckpoint::default(),
                    fields,
                    tool_id,
                    device_id,
                    session_enabled,
                    project_enabled,
                    project_namer,
                )?;
                if !full.sessions.is_empty() {
                    let mut next = cp.clone();
                    next.mtime_ms = mtime;
                    return Ok(CollectResult {
                        events: Vec::new(),
                        sessions: full.sessions,
                        next_checkpoint: next,
                    });
                }
            }
        }
        return Ok(CollectResult {
            events: Vec::new(),
            sessions: Vec::new(),
            next_checkpoint: CollectorCheckpoint {
                source_id: cp.source_id.clone(),
                byte_offset: Some(new_offset),
                inode,
                ..Default::default()
            },
        });
    }

    let file_stem = source
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string();
    let session_external = if session_enabled {
        Some(file_stem.clone())
    } else {
        None
    };
    let project_name = project_enabled
        .then(|| {
            source
                .path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(ToString::to_string)
                .unwrap_or_default()
        })
        .filter(|s| !s.is_empty());
    let project_id = project_name.as_ref().map(|n| hash_short(n));

    let mut events = Vec::new();
    let mut input_sum = 0i64;
    let mut output_sum = 0i64;
    let mut cache_sum = 0i64;
    let mut first_ts: Option<String> = None;
    let mut last_ts: Option<String> = None;

    for line in lines {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue; // 非 JSON / 元数据行，隔离跳过
        };
        let mut event = match parse_generic_event(
            &value,
            fields,
            tool_id,
            device_id,
            session_external.clone(),
            project_id.clone(),
        ) {
            Some(event) => event,
            None => continue,
        };
        // 行定位 hash（用于 fingerprint 兜底）
        event.source_locator_hash = Some(hash_short(&format!("{new_offset}:{line}")));
        input_sum += event.input_tokens.unwrap_or(0);
        output_sum += event.output_tokens.unwrap_or(0);
        cache_sum += event.cache_read_tokens.unwrap_or(0);
        if first_ts.is_none() {
            first_ts = Some(event.occurred_at.clone());
        }
        last_ts = Some(event.occurred_at.clone());
        events.push(event);
    }

    let sessions = if session_enabled && !events.is_empty() {
        let session_id = hash_short(&format!("{}:{tool_id}:{file_stem}", source.id));
        vec![SessionSummary {
            session_id: session_id.clone(),
            tool_id: tool_id.into(),
            external_session_id: session_external,
            project_id,
            title_redacted: Some(format!(
                "{} · {}",
                project_name
                    .as_deref()
                    .map(project_namer)
                    .unwrap_or_else(|| "会话".into()),
                first_ts
                    .as_deref()
                    .and_then(|ts| ts.get(..16))
                    .unwrap_or("")
            )),
            model_set: Vec::new(),
            started_at: first_ts,
            last_active_at: last_ts,
            input_tokens: input_sum,
            output_tokens: output_sum,
            cache_tokens: cache_sum,
            total_tokens: input_sum + output_sum + cache_sum,
            message_count: events.len() as i64,
            status: Some("active".into()),
            cost_amount: None,
        }]
    } else {
        Vec::new()
    };

    Ok(CollectResult {
        events,
        sessions,
        next_checkpoint: CollectorCheckpoint {
            source_id: cp.source_id.clone(),
            byte_offset: Some(new_offset),
            inode,
            ..Default::default()
        },
    })
}

/// Claude Code 目录名解码（`-2f` → `/`，多字节 UTF-8 以逐字节 `-e4-b8-ad` 形式解码）。
/// 最佳努力：`-` 后跟两位 hex 视为一个字节；解不开就原样返回（仅影响显示名）。
pub(crate) fn claude_path_decode(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut utf8_buf: Vec<u8> = Vec::new();
    let mut i = 0;
    let is_hex = |b: u8| b.is_ascii_hexdigit();
    while i < bytes.len() {
        if bytes[i] == b'-' && i + 2 < bytes.len() && is_hex(bytes[i + 1]) && is_hex(bytes[i + 2]) {
            let hex_part = &bytes[i + 1..i + 3];
            if let Ok(hex_str) = std::str::from_utf8(hex_part) {
                if let Ok(byte) = u8::from_str_radix(hex_str, 16) {
                    utf8_buf.push(byte);
                    if let Ok(s) = std::str::from_utf8(&utf8_buf) {
                        out.push_str(s);
                        utf8_buf.clear();
                    }
                    i += 3;
                    continue;
                }
            }
        }
        if !utf8_buf.is_empty() {
            out.push_str(&String::from_utf8_lossy(&utf8_buf));
            utf8_buf.clear();
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    if !utf8_buf.is_empty() {
        out.push_str(&String::from_utf8_lossy(&utf8_buf));
    }
    out
}

/// 递归收集目录下指定扩展名的文件（有界深度，防止误入巨型目录）。
pub(crate) fn collect_files(root: &Path, ext: &str, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > max_depth {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push((path, depth + 1));
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case(ext))
                .unwrap_or(false)
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// 只读打开 SQLite（兼容 WAL；不 PRAGMA journal_mode）。
pub(crate) fn open_readonly(path: &Path) -> Result<rusqlite::Connection, CollectorError> {
    if !path.exists() {
        return Err(CollectorError::PathMissing(format!("{:?}", path)));
    }
    rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| CollectorError::Io(format!("open sqlite {:?}: {e}", path)))
}

/// 通用 SQLite 增量采集（W3c 的 hermes/zed/mimo 等复用）。
/// 防御式：在候选表中找 id + tokens 列；列名不符 → `FormatChanged`（保留旧数据）。
/// tokens 可为数字或 JSON 字符串（{"input":..,"output":..,"cache":..}）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn collect_sqlite_generic(
    source: &DataSource,
    cp: &CollectorCheckpoint,
    tool_id: &str,
    table_candidates: &[&str],
    model_col: Option<&str>,
    time_col: Option<&str>,
) -> Result<CollectResult, CollectorError> {
    let conn = open_readonly(&source.path)?;
    let last_id: i64 = cp
        .last_record_id
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .map_err(|e| CollectorError::Io(format!("schema: {e}")))?;
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| CollectorError::Io(format!("schema: {e}")))?
        .collect::<Result<_, _>>()
        .map_err(|e| CollectorError::Io(format!("schema: {e}")))?;
    drop(stmt);

    let table = table_candidates
        .iter()
        .find(|candidate| tables.iter().any(|t| t == *candidate))
        .ok_or_else(|| {
            CollectorError::FormatChanged(format!("{tool_id}: 无候选表（{table_candidates:?}）"))
        })?;

    let has_col = |conn: &rusqlite::Connection, name: &str| -> bool {
        let cols: Vec<String> = {
            let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
                return false;
            };
            let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
                return false;
            };
            rows.flatten().collect()
        };
        cols.iter().any(|c| c == name)
    };
    let tokens_col = ["tokens", "token_count", "usage", "input_tokens"]
        .iter()
        .copied()
        .find(|c| has_col(&conn, c));
    let Some(tokens_col) = tokens_col else {
        return Err(CollectorError::FormatChanged(format!(
            "{tool_id}: 表 {table} 无 tokens 列"
        )));
    };
    let model_col = model_col.filter(|c| has_col(&conn, c)).unwrap_or("''");
    let time_col = time_col.filter(|c| has_col(&conn, c)).unwrap_or("NULL");

    let sql = format!(
        "SELECT id, {tokens_col}, {model_col}, {time_col} FROM {table} WHERE id > ?1 ORDER BY id LIMIT 500"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| CollectorError::Io(format!("query: {e}")))?;
    let rows = stmt
        .query_map([last_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| CollectorError::Io(format!("query: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| CollectorError::Io(format!("query: {e}")))?;
    drop(stmt);

    let mut events = Vec::new();
    let mut max_id = last_id;
    for (id, tokens_raw, model, time) in rows {
        max_id = max_id.max(id);
        let (input, output, cache) = if let Ok(n) = tokens_raw.trim().parse::<i64>() {
            // 单个数字：视为 output（部分源只有 completion tokens）
            (None, Some(n), None)
        } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(&tokens_raw) {
            let input = value.get("input").and_then(|v| v.as_i64());
            let output = value
                .get("output")
                .and_then(|v| v.as_i64())
                .or_else(|| value.get("completion").and_then(|v| v.as_i64()));
            let cache = value
                .pointer("/cache/read")
                .and_then(|v| v.as_i64())
                .or_else(|| value.get("cache").and_then(|v| v.as_i64()));
            (input, output, cache)
        } else {
            (None, None, None)
        };
        if input.is_none() && output.is_none() {
            continue;
        }
        let occurred_at = time
            .as_deref()
            .and_then(|t| to_utc_iso(&serde_json::Value::String(t.to_string())))
            .unwrap_or_else(|| "1970-01-01T00:00:00Z".into());
        events.push(NormalizedUsageEvent {
            source_type: SourceType::LocalDiscovered,
            tool_id: tool_id.into(),
            device_id: "local".into(),
            model_raw: model,
            model_normalized: None,
            session_id: None,
            project_id: None,
            account_id: None,
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache,
            cache_write_tokens: None,
            reasoning_tokens: None,
            message_count: None,
            session_started_at: None,
            session_last_active_at: None,
            total_tokens: None,
            cost_amount: None,
            cost_currency: None,
            usage_accuracy: UsageAccuracy::Exact,
            occurred_at,
            source_locator_hash: Some(hash_short(&format!("{tool_id}:{id}"))),
        });
    }

    Ok(CollectResult {
        events,
        sessions: Vec::new(),
        next_checkpoint: CollectorCheckpoint {
            source_id: cp.source_id.clone(),
            last_record_id: Some(max_id.to_string()),
            ..Default::default()
        },
    })
}

/// 整文件 JSON 增量采集（IDE storage 类：sessions.json / tasks 等）。
/// - 顶层为数组 → 逐元素提取 usage；顶层为对象 → 尝试常见嵌套路径。
/// - 增量：mtime 未变则跳过（`IncrementalMode::Mtime`）；变了全量重读 + fingerprint 去重。
pub(crate) fn collect_json_array(
    source: &DataSource,
    cp: &CollectorCheckpoint,
    fields: &GenericFields,
    tool_id: &str,
) -> Result<CollectResult, CollectorError> {
    let metadata = std::fs::metadata(&source.path)
        .map_err(|e| CollectorError::PathMissing(format!("{:?}: {e}", source.path)))?;
    let mtime_ms = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    if let (Some(prev), Some(current)) = (cp.mtime_ms, mtime_ms) {
        if prev == current {
            return Ok(CollectResult {
                events: Vec::new(),
                sessions: Vec::new(),
                next_checkpoint: cp.clone(),
            });
        }
    }

    let raw = std::fs::read_to_string(&source.path)
        .map_err(|e| CollectorError::Io(format!("read {:?}: {e}", source.path)))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CollectorError::Parse(format!("json {:?}: {e}", source.path)))?;

    let entries: Vec<&serde_json::Value> = match &value {
        serde_json::Value::Array(items) => items.iter().collect(),
        _ => vec![&value],
    };
    let mut events = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        // 常见嵌套：entry.tokens / entry.usage / entry.message.usage / entry.extra
        let probe = if entry.get("tokens").is_some() || entry.get("usage").is_some() {
            entry
        } else if let Some(message) = entry.get("message") {
            message
        } else {
            entry
        };
        let mut event = match parse_generic_event(probe, fields, tool_id, "local", None, None) {
            Some(event) => event,
            None => continue,
        };
        event.source_locator_hash = Some(hash_short(&format!("{}:{index}", source.id)));
        events.push(event);
    }

    Ok(CollectResult {
        events,
        sessions: Vec::new(),
        next_checkpoint: CollectorCheckpoint {
            source_id: cp.source_id.clone(),
            mtime_ms,
            ..Default::default()
        },
    })
}

// ---------------------------------------------------------------------------
// JSONL 适配器生成宏（W3b/W3c 的 JSONL 工具共用；样板语义见 claude_code/codex）
// ---------------------------------------------------------------------------

/// 展开为一个完整的 `ToolAdapter` 实现（JSONL / offset 增量）。
/// `dirs` 为候选根目录表达式（求值为 `Vec<PathBuf>`）；`fields` 为 `GenericFields` 常量。
macro_rules! jsonl_adapter {
    (
        $adapter:ident;
        tool_id = $tool_id:literal;
        display_name = $display:literal;
        vendor = $vendor:literal;
        level = $level:path;
        session = $session:expr;
        project = $project:expr;
        cache = $cache:expr;
        cost = $cost:expr;
        accuracy = $accuracy:path;
        dirs = $dirs:expr;
        ext = $ext:literal;
        fields = $fields:expr;
    ) => {
        #[derive(Default)]
        pub struct $adapter;

        impl crate::token_monitor::collector::ToolAdapter for $adapter {
            fn descriptor(&self) -> crate::token_monitor::model::ToolDescriptor {
                crate::token_monitor::model::ToolDescriptor {
                    tool_id: $tool_id.into(),
                    display_name: $display.into(),
                    vendor: Some($vendor.into()),
                    kind: crate::token_monitor::model::ToolKind::Usage,
                    support_level: $level,
                    supported_os: vec!["macos".into(), "linux".into()],
                    adapter_version: 1,
                    privacy_note: concat!("读取 ", $display,
                        " 会话元数据（usage/model/时间），不读取 Prompt/Response 正文；路径仅 hash 入库")
                        .into(),
                }
            }

            fn capabilities(&self) -> crate::token_monitor::model::AdapterCapabilities {
                crate::token_monitor::model::AdapterCapabilities {
                    token: true,
                    model: true,
                    session: $session,
                    project: $project,
                    cache_tokens: $cache,
                    cost: $cost,
                    accuracy: $accuracy,
                    incremental: crate::token_monitor::model::IncrementalMode::FileOffset,
                }
            }

            fn discover(&self) -> Vec<crate::token_monitor::model::DataSource> {
                let roots: Vec<PathBuf> = $dirs;
                let mut sources = Vec::new();
                for root in roots {
                    if !root.exists() {
                        continue;
                    }
                    for path in collect_files(&root, $ext, 3) {
                        sources.push(crate::token_monitor::model::DataSource {
                            id: format!("{}:{}", $tool_id, hash_short(&path.to_string_lossy())),
                            path,
                            format: crate::token_monitor::model::DataFormat::Jsonl,
                            watch: true,
                        });
                    }
                }
                sources
            }

            fn checkpoint(&self, source_id: &str) -> crate::token_monitor::model::CollectorCheckpoint {
                crate::token_monitor::model::CollectorCheckpoint {
                    source_id: source_id.into(),
                    ..Default::default()
                }
            }

            fn collect_incremental(
                &self,
                source: &crate::token_monitor::model::DataSource,
                checkpoint: crate::token_monitor::model::CollectorCheckpoint,
            ) -> Result<crate::token_monitor::collector::CollectResult, crate::token_monitor::model::CollectorError> {
                collect_jsonl_incremental(
                    source,
                    &checkpoint,
                    &$fields,
                    $tool_id,
                    "local",
                    $session,
                    $project,
                    |name| name.to_string(),
                )
            }

            fn list_sessions(
                &self,
                _query: &crate::token_monitor::model::SessionQuery,
            ) -> Result<Vec<crate::token_monitor::model::SessionSummary>, crate::token_monitor::model::CollectorError> {
                Ok(Vec::new())
            }
        }
    };
}
pub(crate) use jsonl_adapter;

/// 展开为一个完整的 `ToolAdapter` 实现（整文件 JSON / mtime 增量，IDE storage 类）。
macro_rules! json_adapter {
    (
        $adapter:ident;
        tool_id = $tool_id:literal;
        display_name = $display:literal;
        vendor = $vendor:literal;
        level = $level:path;
        session = $session:expr;
        project = $project:expr;
        cache = $cache:expr;
        cost = $cost:expr;
        accuracy = $accuracy:path;
        dirs = $dirs:expr;
        ext = $ext:literal;
        fields = $fields:expr;
    ) => {
        #[derive(Default)]
        pub struct $adapter;

        impl crate::token_monitor::collector::ToolAdapter for $adapter {
            fn descriptor(&self) -> crate::token_monitor::model::ToolDescriptor {
                crate::token_monitor::model::ToolDescriptor {
                    tool_id: $tool_id.into(),
                    display_name: $display.into(),
                    vendor: Some($vendor.into()),
                    kind: crate::token_monitor::model::ToolKind::Usage,
                    support_level: $level,
                    supported_os: vec!["macos".into(), "linux".into()],
                    adapter_version: 1,
                    privacy_note: concat!("读取 ", $display,
                        " 会话元数据（usage/model/时间），不读取 Prompt/Response 正文；路径仅 hash 入库")
                        .into(),
                }
            }

            fn capabilities(&self) -> crate::token_monitor::model::AdapterCapabilities {
                crate::token_monitor::model::AdapterCapabilities {
                    token: true,
                    model: true,
                    session: $session,
                    project: $project,
                    cache_tokens: $cache,
                    cost: $cost,
                    accuracy: $accuracy,
                    incremental: crate::token_monitor::model::IncrementalMode::Mtime,
                }
            }

            fn discover(&self) -> Vec<crate::token_monitor::model::DataSource> {
                let roots: Vec<PathBuf> = $dirs;
                let mut sources = Vec::new();
                for root in roots {
                    if !root.exists() {
                        continue;
                    }
                    for path in collect_files(&root, $ext, 4) {
                        sources.push(crate::token_monitor::model::DataSource {
                            id: format!("{}:{}", $tool_id, hash_short(&path.to_string_lossy())),
                            path,
                            format: crate::token_monitor::model::DataFormat::Json,
                            watch: true,
                        });
                    }
                }
                sources
            }

            fn checkpoint(&self, source_id: &str) -> crate::token_monitor::model::CollectorCheckpoint {
                crate::token_monitor::model::CollectorCheckpoint {
                    source_id: source_id.into(),
                    ..Default::default()
                }
            }

            fn collect_incremental(
                &self,
                source: &crate::token_monitor::model::DataSource,
                checkpoint: crate::token_monitor::model::CollectorCheckpoint,
            ) -> Result<crate::token_monitor::collector::CollectResult, crate::token_monitor::model::CollectorError> {
                collect_json_array(source, &checkpoint, &$fields, $tool_id)
            }

            fn list_sessions(
                &self,
                _query: &crate::token_monitor::model::SessionQuery,
            ) -> Result<Vec<crate::token_monitor::model::SessionSummary>, crate::token_monitor::model::CollectorError> {
                Ok(Vec::new())
            }
        }
    };
}
pub(crate) use json_adapter;
