//! Application log file utilities.
//!
//! Reads the current `tracing`-based rolling log file written by
//! [`tracing_appender`], applies display-layer sanitisation, and
//! provides a paginated tail view used by the "App Logs" UI page.

use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Maximum bytes to read from the log file.  A single daily file rarely
/// exceeds ~10 MB; for larger files only the tail portion is read to keep
/// memory bounded.
const MAX_READ_BYTES: u64 = 50 * 1024 * 1024; // 50 MiB

/// Log files older than this many days are pruned on startup.
const RETENTION_DAYS: i64 = 7;

// ─── Public types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppLogPage {
    /// Log lines in reverse order (newest first), already sanitised.
    pub lines: Vec<String>,
    /// Total number of lines after applying the keyword filter.
    pub total: usize,
    pub page: u32,
    pub page_size: u32,
    pub file_path: String,
    pub file_size: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppLogInfo {
    pub log_dir: String,
    pub file_path: Option<String>,
    pub file_size: u64,
    pub line_count: usize,
}

// ─── Log directory helpers ────────────────────────────────────────────────────

/// Return the log directory path.  This must match the path used by
/// [`super::init_logging`].
pub fn log_dir_for(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(".poolgate").join("logs")
}

/// Locate the most recently modified `app.log.*` file in the given directory.
///
/// [`tracing_appender::rolling::daily`] produces files named
/// `app.log.YYYY-MM-DD` — the date suffix is appended *after* the base name.
fn latest_log_file(log_dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    let entries = std::fs::read_dir(log_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.starts_with("app.log.") {
            continue;
        }
        let mtime = entry.metadata().ok()?.modified().ok()?;
        match &best {
            Some((best_time, _)) if *best_time >= mtime => {}
            _ => best = Some((mtime, path)),
        }
    }
    best.map(|(_, path)| path)
}

// ─── Sanitisation ────────────────────────────────────────────────────────────

/// Display-layer regex patterns.  Values are compiled once and reused.
/// The first match group is replaced with `***` (or the prefix is preserved
/// where applicable).
///
/// Order matters: Authorization / x-api-key header patterns must run before
/// the standalone Bearer pattern, otherwise the header regex replaces the
/// Bearer keyword itself as the value, producing `Authorization: *** ***`.
static SANITIZE_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        // x-api-key / Authorization / api_key style headers.
        (
            Regex::new(r"(?i)(x-api-key|authorization|api[_-]?key)\s*[:=]\s*\S+(?:\s+\S+)*")
                .unwrap(),
            "$1: ***",
        ),
        // Standalone Bearer tokens (8+ chars after the keyword) — catches
        // tokens not already covered by the Authorization header pattern.
        (
            Regex::new(r"(Bearer\s+)[A-Za-z0-9._-]{8,}").unwrap(),
            "$1***",
        ),
        // PoolGate virtual client keys.
        (Regex::new(r"pg_live_[A-Za-z0-9]+").unwrap(), "pg_live_***"),
        // OpenAI opaque access tokens.
        (Regex::new(r"at-[A-Za-z0-9]+").unwrap(), "at-***"),
    ]
});

pub fn sanitize_line(line: &str) -> String {
    let mut out = line.to_string();
    for (re, replacement) in SANITIZE_PATTERNS.iter() {
        out = re.replace_all(&out, *replacement).into_owned();
    }
    out
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Read the latest log file with optional keyword filtering and pagination.
///
/// Lines are returned in reverse order (newest first).
pub fn read_logs(
    log_dir: &Path,
    page: u32,
    page_size: u32,
    keyword: Option<&str>,
) -> Result<AppLogPage, String> {
    let file_path =
        latest_log_file(log_dir).ok_or_else(|| "日志目录下没有 app.*.log 文件".to_string())?;
    let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

    let bytes = read_file_tail(&file_path, MAX_READ_BYTES)?;
    let content = String::from_utf8_lossy(&bytes);
    let keyword_lower = keyword
        .map(|k| k.trim().to_lowercase())
        .filter(|k| !k.is_empty());

    let mut lines: Vec<String> = content
        .lines()
        .filter(|line| {
            keyword_lower
                .as_ref()
                .map(|kw| line.to_lowercase().contains(kw))
                .unwrap_or(true)
        })
        .map(|line| sanitize_line(line))
        .collect();

    lines.reverse(); // newest first
    let total = lines.len();
    let start = ((page.max(1) - 1) * page_size) as usize;
    let end = (start + page_size as usize).min(total);
    let page_lines = if start < total {
        lines[start..end].to_vec()
    } else {
        vec![]
    };

    Ok(AppLogPage {
        lines: page_lines,
        total,
        page,
        page_size,
        file_path: file_path.display().to_string(),
        file_size,
    })
}

/// Return metadata about the current log file.
pub fn get_log_info(log_dir: &Path) -> Result<AppLogInfo, String> {
    let file_path = latest_log_file(log_dir);
    let file_size = file_path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);
    let line_count = file_path
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .map(|b| String::from_utf8_lossy(&b).lines().count())
        .unwrap_or(0);
    Ok(AppLogInfo {
        log_dir: log_dir.display().to_string(),
        file_path: file_path.map(|p| p.display().to_string()),
        file_size,
        line_count,
    })
}

/// Delete rolling log files older than [`RETENTION_DAYS`].
pub fn cleanup_old_logs(log_dir: &Path) {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(RETENTION_DAYS);
    let entries = match std::fs::read_dir(log_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // tracing_appender rolling::daily produces `app.log.YYYY-MM-DD`.
        if !name.starts_with("app.log.") {
            continue;
        }
        // Extract date from name: `app.log.YYYY-MM-DD`.
        let date_str = name.strip_prefix("app.log.");
        if let Some(ds) = date_str {
            if let Ok(date) = chrono::NaiveDate::parse_from_str(ds, "%Y-%m-%d") {
                if date.and_hms_opt(0, 0, 0).map(|d| d.and_utc()) < Some(cutoff) {
                    tracing::debug!("Pruning old log file: {}", path.display());
                    std::fs::remove_file(&path).ok();
                }
            }
        }
    }
}

// ─── Private helpers ─────────────────────────────────────────────────────────

/// Read at most `max_bytes` from the tail of a file.
///
/// If the file is smaller than `max_bytes`, read it entirely.
fn read_file_tail(path: &Path, max_bytes: u64) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("读取日志文件元数据失败: {}", e))?;
    let file_size = metadata.len();

    if file_size <= max_bytes {
        return std::fs::read(path).map_err(|e| format!("读取日志文件失败: {}", e));
    }

    // Seek to (file_size - max_bytes) and read from there.
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).map_err(|e| format!("打开日志文件失败: {}", e))?;
    let skip = file_size - max_bytes;
    file.seek(SeekFrom::Start(skip))
        .map_err(|e| format!("定位日志文件失败: {}", e))?;
    let mut buf = Vec::with_capacity(max_bytes as usize);
    file.read_to_end(&mut buf)
        .map_err(|e| format!("读取日志文件失败: {}", e))?;

    // Skip the first partial line (we seeked into the middle of one).
    if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
        Ok(buf[pos + 1..].to_vec())
    } else {
        Ok(buf)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_masks_bearer_tokens() {
        // Authorization header covers the Bearer keyword itself.
        let line = r#"request completed Authorization: Bearer sk-abc1234567890extra"#;
        let sanitized = sanitize_line(line);
        assert!(sanitized.contains("Authorization: ***"));
        assert!(!sanitized.contains("sk-abc1234567890extra"));
        // Standalone Bearer (no Authorization prefix) is also caught.
        let standalone = "token Bearer at-xxxxxxxx12345678";
        assert!(sanitize_line(standalone).contains("Bearer ***"));
    }

    #[test]
    fn sanitize_masks_poolgate_keys() {
        let line = "key=pg_live_21bf222e5631ae45a72ad14e";
        let sanitized = sanitize_line(line);
        assert!(sanitized.contains("pg_live_***"));
        assert!(!sanitized.contains("21bf222e5631ae45a72ad14e"));
    }

    #[test]
    fn sanitize_masks_opaque_access_tokens() {
        let line = "got token at-abc123def456";
        let sanitized = sanitize_line(line);
        assert!(sanitized.contains("at-***"));
        assert!(!sanitized.contains("abc123def456"));
    }

    #[test]
    fn sanitize_preserves_non_sensitive_lines() {
        let line = "proxy listening on port 9800";
        let sanitized = sanitize_line(line);
        assert_eq!(sanitized, line);
    }

    #[test]
    fn read_logs_with_keyword_filter() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("app.log.2026-08-04");
        std::fs::write(
            &log_path,
            "line one request_id=abc\nline two 503 error\nline three\n",
        )
        .unwrap();

        let page = read_logs(dir.path(), 1, 100, Some("503")).unwrap();
        assert_eq!(page.total, 1);
        assert!(page.lines[0].contains("503"));
    }

    #[test]
    fn read_logs_reverses_order() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("app.log.2026-08-04");
        std::fs::write(&log_path, "first\nsecond\nthird\n").unwrap();

        let page = read_logs(dir.path(), 1, 100, None).unwrap();
        assert_eq!(page.lines, vec!["third", "second", "first"]);
    }

    #[test]
    fn read_logs_pagination() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("app.log.2026-08-04");
        let lines: Vec<String> = (0..25).map(|i| format!("line {i}")).collect();
        std::fs::write(&log_path, lines.join("\n") + "\n").unwrap();

        let page1 = read_logs(dir.path(), 1, 10, None).unwrap();
        assert_eq!(page1.total, 25);
        assert_eq!(page1.lines.len(), 10);
        // Newest first: page 1 starts at line 24.
        assert!(page1.lines[0].contains("line 24"));

        let page3 = read_logs(dir.path(), 3, 10, None).unwrap();
        assert_eq!(page3.lines.len(), 5);
        assert!(page3.lines[0].contains("line 4"));
    }
}
