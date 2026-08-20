//! AtomCode 采集适配器（自建；tokscale 未覆盖，作为 companion 在 tokscale 模式并行采集）。
//!
//! 数据源：`~/.atomcode/datalog/<project>/<ts>.md` —— 每文件一个会话，逐轮用量元数据
//! 以固定格式内嵌。已在本机 60 文件 / 752 轮上验证，格式：
//! ```text
//! # Turn 2026-06-17 11:10:55 [build:867bc77]
//! **env:** model=GLM-5.1, ctx_window=64000, session=cec4e083-..., cwd=/Users/.../ai-hub
//! ### Turn 1
//!   _[tokens: prompt=47257+completion=64, cache=3200tok]_   （cache 可选）
//! ```
//! 隐私红线：逐行流式读取，仅用正则提取上述元数据行；消息/工具输出正文只在内存短暂
//! 经过、绝不持久化；`cwd` 路径与 session uuid 一律以不可逆 hash 入库。
//! 增量：文件 mtime 变化 → 重读全文件重发全部轮次（`source_locator_hash` 幂等去重）。

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use regex::Regex;

use crate::token_monitor::collector::common::{collect_files, hash_short};
use crate::token_monitor::collector::CollectResult;
use crate::token_monitor::collector::ToolAdapter;
use crate::token_monitor::model::{
    AdapterCapabilities, CollectorCheckpoint, CollectorError, DataFormat, DataSource,
    IncrementalMode, NormalizedUsageEvent, SessionQuery, SessionSummary, SourceType, SupportLevel,
    ToolDescriptor, ToolKind, UsageAccuracy,
};

#[derive(Default)]
pub struct AtomCodeAdapter;

fn mtime_ms(path: &Path) -> Option<i64> {
    path.metadata()
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

/// 解析单个 datalog 文件 → (事件, 会话摘要)。逐行正则，正文不落库。
fn parse_datalog(
    path: &Path,
) -> Result<(Vec<NormalizedUsageEvent>, Option<SessionSummary>), CollectorError> {
    let file = std::fs::File::open(path)
        .map_err(|e| CollectorError::PathMissing(format!("{path:?}: {e}")))?;
    // session 字段可选：部分文件的 env 行无 session（`model=…, ctx_window=…, cwd=…`）。
    // 早期正则强制要求 session → 整行不匹配 → model 没提取 → 模型视图「未知模型」。
    let re_env = Regex::new(
        r"^\*\*env:\*\* model=([^,]+), ctx_window=\d+(?:, session=([0-9a-fA-F-]+))?, cwd=(\S+)",
    )
    .expect("valid env regex");
    let re_header =
        Regex::new(r"^# Turn (\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})").expect("valid header regex");
    let re_tokens = Regex::new(r"\[tokens: prompt=(\d+)\+completion=(\d+)(?:, cache=(\d+)tok)?\]")
        .expect("valid tokens regex");
    // 每轮耗时 `_(N.Ns)_`：用于把会话内逐轮时间戳从文件头推导出来（真实派生，
    // 非伪造）——既让指纹天然互异（避免同值轮次误去重），也让会话时间线更准确。
    let re_dur = Regex::new(r"\(([0-9.]+)s\)").expect("valid duration regex");

    let file_id = hash_short(&path.to_string_lossy());
    let project_display = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    let mut model: Option<String> = None;
    let mut session_uuid: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut first_ts: Option<String> = None;
    let mut turn = 0usize;
    let mut events = Vec::new();
    let mut sum_in = 0i64;
    let mut sum_out = 0i64;
    let mut sum_cache = 0i64;
    // 已过耗时的累计（毫秒）：前一轮耗时累加后作为本轮发生时刻的偏移
    let mut elapsed_ms: i64 = 0;

    for line in BufReader::new(file).lines() {
        // 文件可能正被写入：单行读取失败跳过该行，不中断整个文件解析
        let Ok(line) = line else { continue };
        if let Some(c) = re_env.captures(&line) {
            model = Some(c[1].trim().to_string());
            session_uuid = c.get(2).map(|m| m.as_str().to_string());
            cwd = Some(c[3].to_string());
            continue;
        }
        if first_ts.is_none() {
            if let Some(c) = re_header.captures(&line) {
                first_ts = Some(c[1].to_string());
                continue;
            }
        }
        let Some(c) = re_tokens.captures(&line) else {
            // 耗时行：累加到已过耗时（供下一轮定位），本身不产生事件
            if let Some(d) = re_dur.captures(&line) {
                if let Ok(secs) = d[1].parse::<f64>() {
                    elapsed_ms += (secs * 1000.0) as i64;
                }
            }
            continue;
        };
        // 仅元数据行（usage/model），正文行不含该模式 → 自然跳过
        let prompt: i64 = c[1].parse().unwrap_or(0);
        let completion: i64 = c[2].parse().unwrap_or(0);
        let cache: i64 = c.get(3).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        turn += 1;
        sum_in += prompt;
        sum_out += completion;
        sum_cache += cache;
        let occurred_at = first_ts
            .as_deref()
            .map(|ts| header_plus_ms(ts, elapsed_ms))
            .unwrap_or_else(|| {
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            });
        events.push(NormalizedUsageEvent {
            source_type: SourceType::LocalDiscovered,
            tool_id: "atomcode".into(),
            device_id: "local".into(),
            model_raw: model.clone(),
            model_normalized: None,
            session_id: session_uuid.as_ref().map(|u| hash_short(u)),
            project_id: cwd.as_ref().map(|p| hash_short(p)),
            account_id: None,
            input_tokens: Some(prompt),
            output_tokens: Some(completion),
            cache_read_tokens: Some(cache),
            cache_write_tokens: None,
            reasoning_tokens: None,
            message_count: None,
            session_started_at: None,
            session_last_active_at: None,
            total_tokens: Some(prompt + completion + cache),
            cost_amount: None,
            cost_currency: None,
            usage_accuracy: UsageAccuracy::ProviderReported,
            occurred_at,
            source_locator_hash: Some(hash_short(&format!("atomcode:{file_id}:{turn}"))),
        });
    }

    let session = if events.is_empty() {
        None
    } else {
        Some(SessionSummary {
            session_id: session_uuid
                .as_ref()
                .map(|u| hash_short(u))
                .unwrap_or_else(|| format!("atomcode:{file_id}")),
            tool_id: "atomcode".into(),
            external_session_id: session_uuid,
            project_id: cwd.as_ref().map(|p| hash_short(p)),
            title_redacted: Some(format!(
                "{} · {}",
                if project_display.is_empty() {
                    "会话".into()
                } else {
                    project_display
                },
                first_ts
                    .as_deref()
                    .and_then(|ts| ts.get(..10))
                    .unwrap_or("")
            )),
            model_set: model.into_iter().collect(),
            started_at: first_ts.as_deref().map(|ts| header_plus_ms(ts, 0)),
            last_active_at: first_ts.as_deref().map(|ts| header_plus_ms(ts, elapsed_ms)),
            input_tokens: sum_in,
            output_tokens: sum_out,
            cache_tokens: sum_cache,
            total_tokens: sum_in + sum_out + sum_cache,
            message_count: turn as i64,
            status: Some("active".into()),
            cost_amount: None,
        })
    };

    Ok((events, session))
}

/// 本地 naive 时间（datalog 头）+ 已过耗时偏移 → UTC ISO8601。
/// 与 common::to_utc_iso 对本地格式的处理一致（按 UTC 解释；DAY 归因由 SQLite
/// localtime 统一，不引入双重时区偏移）。解析失败回退当前时刻（不 panic）。
fn header_plus_ms(ts: &str, ms: i64) -> String {
    match chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S") {
        Ok(naive) => (naive.and_utc() + chrono::Duration::milliseconds(ms))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        Err(_) => chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }
}

impl ToolAdapter for AtomCodeAdapter {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            tool_id: "atomcode".into(),
            display_name: "AtomCode".into(),
            vendor: Some("AtomCode".into()),
            kind: ToolKind::Usage,
            support_level: SupportLevel::Basic,
            supported_os: vec!["macos".into(), "linux".into()],
            adapter_version: 1,
            privacy_note: "读取 ~/.atomcode/datalog 会话文件的用量元数据行（逐轮 prompt/completion/cache/model/时间）；不读取消息正文；cwd 路径与 session 仅 hash 入库"
                .into(),
        }
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            token: true,
            model: true,
            session: true,
            project: true,
            cache_tokens: true,
            cost: false,
            accuracy: UsageAccuracy::ProviderReported,
            incremental: IncrementalMode::Mtime,
        }
    }

    fn discover(&self) -> Vec<DataSource> {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return Vec::new();
        };
        let root = home.join(".atomcode").join("datalog");
        if !root.exists() {
            return Vec::new();
        }
        collect_files(&root, "md", 3)
            .into_iter()
            .map(|path| DataSource {
                id: format!("atomcode:{}", hash_short(&path.to_string_lossy())),
                path,
                format: DataFormat::Jsonl,
                watch: true,
            })
            .collect()
    }

    fn checkpoint(&self, source_id: &str) -> CollectorCheckpoint {
        CollectorCheckpoint {
            source_id: source_id.into(),
            ..Default::default()
        }
    }

    fn collect_incremental(
        &self,
        source: &DataSource,
        checkpoint: CollectorCheckpoint,
    ) -> Result<CollectResult, CollectorError> {
        let current = mtime_ms(&source.path);
        if let (Some(prev), Some(cur)) = (checkpoint.mtime_ms, current) {
            if prev == cur {
                return Ok(CollectResult {
                    events: Vec::new(),
                    sessions: Vec::new(),
                    next_checkpoint: checkpoint,
                });
            }
        }
        let (events, session) = parse_datalog(&source.path)?;
        Ok(CollectResult {
            events,
            sessions: session.into_iter().collect(),
            next_checkpoint: CollectorCheckpoint {
                source_id: checkpoint.source_id,
                mtime_ms: current,
                ..Default::default()
            },
        })
    }

    fn list_sessions(&self, _query: &SessionQuery) -> Result<Vec<SessionSummary>, CollectorError> {
        Ok(Vec::new())
    }
}

/// 修复 AtomCode 历史事件（自愈）：早期 env 正则强制要求 `session` 字段，部分文件
/// （env 行无 session）的 model 没被提取 → `model_normalized=NULL`（模型视图「未知
/// 模型」）。重解析数据源重建 `locator_hash → model` 映射，交给通用修复模块处理
/// （去重残留重复 + 回填 NULL + 重建 rollup）。幂等，无待修时快速短路返回 0。
pub(crate) fn backfill_missing_models(
    conn: &Mutex<rusqlite::Connection>,
    source: &DataSource,
) -> Result<usize, String> {
    if !super::repair::has_null_model_rows(conn, "atomcode") {
        return Ok(0);
    }
    let (events, _) = parse_datalog(&source.path).map_err(|e| e.to_string())?;
    let mut models: HashMap<String, String> = HashMap::new();
    for e in &events {
        if let (Some(model), Some(hash)) =
            (e.model_raw.as_deref(), e.source_locator_hash.as_deref())
        {
            models
                .entry(hash.to_string())
                .or_insert_with(|| model.to_string());
        }
    }
    super::repair::repair_missing_models(conn, "atomcode", &models)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在 `<temp>/ai-hub-362a5e37/<ts>.md` 下建样例文件（模拟真实 datalog 目录结构）。
    fn sample_file() -> (tempfile::TempDir, std::path::PathBuf) {
        use std::io::Write;
        let dir = tempfile::tempdir().expect("tempdir");
        let proj = dir.path().join("ai-hub-362a5e37");
        std::fs::create_dir_all(&proj).unwrap();
        let path = proj.join("2026-06-17_11-10-55.md");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "# Turn 2026-06-17 11:10:55 [build:867bc77]").unwrap();
        writeln!(f, "**env:** model=GLM-5.1, ctx_window=64000, session=cec4e083-fcb4-4d33-a3ed-18f557823a05, cwd=/Users/dev/ai-hub").unwrap();
        writeln!(f, "## User").unwrap();
        writeln!(f, "```").unwrap();
        writeln!(f, "请重构这个组件").unwrap();
        writeln!(f, "```").unwrap();
        writeln!(f, "## Agent").unwrap();
        writeln!(f, "### Turn 1").unwrap();
        writeln!(f, "  _[request: 84msgs · 39973tok · 13tools]_").unwrap();
        writeln!(f, "  _[tokens: prompt=47257+completion=64, cache=3200tok]_").unwrap();
        writeln!(f, "  _(30.7s)_").unwrap();
        writeln!(f, "### Turn 2").unwrap();
        writeln!(f, "  _[tokens: prompt=131550+completion=1361]_").unwrap();
        writeln!(f, "  _(5.0s)_").unwrap();
        (dir, path)
    }

    #[test]
    fn parses_turns_into_usage_events() {
        let (_dir, path) = sample_file();
        let (events, session) = parse_datalog(&path).expect("parse");
        assert_eq!(events.len(), 2);
        let e0 = &events[0];
        assert_eq!(e0.tool_id, "atomcode");
        assert_eq!(e0.input_tokens, Some(47257));
        assert_eq!(e0.output_tokens, Some(64));
        assert_eq!(e0.cache_read_tokens, Some(3200));
        assert_eq!(e0.total_tokens, Some(47257 + 64 + 3200));
        assert_eq!(e0.model_raw.as_deref(), Some("GLM-5.1"));
        assert!(e0.occurred_at.starts_with("2026-06-17"));
        // 无 cache 行的兜底
        assert_eq!(events[1].cache_read_tokens, Some(0));
        assert_eq!(events[1].total_tokens, Some(131550 + 1361));
        // 逐轮时间由耗时推导：第二轮 = 文件头 11:10:55 + 30.7s，天然互异（指纹不冲突）
        assert_eq!(e0.occurred_at, "2026-06-17T11:10:55Z");
        assert_eq!(events[1].occurred_at, "2026-06-17T11:11:25Z");
        assert_ne!(e0.occurred_at, events[1].occurred_at);
        // 隐私：正文内容绝不进事件
        for e in &events {
            assert!(e.source_locator_hash.is_some());
        }
        let s = session.expect("session");
        assert_eq!(s.message_count, 2);
        assert_eq!(s.total_tokens, 47257 + 64 + 3200 + 131550 + 1361);
        assert_eq!(s.input_tokens, 47257 + 131550);
        // cwd 仅 hash
        assert!(s.project_id.is_some());
        assert_ne!(s.project_id.as_deref(), Some("/Users/dev/ai-hub"));
        assert!(s.title_redacted.as_deref().unwrap_or("").contains("ai-hub"));
    }

    #[test]
    fn parses_env_without_session_field() {
        // 复现线上 bug：部分文件 env 行无 session（`model=…, ctx_window=…, cwd=…`），
        // 早期正则强制要求 session → 整行不匹配 → model 没提取 → 「未知模型」。
        use std::io::Write;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("2026-06-17_11-10-55.md");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "# Turn 2026-06-17 11:10:55 [build:867bc77]").unwrap();
        writeln!(
            f,
            "**env:** model=GLM-5.2, ctx_window=200000, cwd=/Users/dev/wxbuddy"
        )
        .unwrap();
        writeln!(f, "## Agent").unwrap();
        writeln!(f, "### Turn 1").unwrap();
        writeln!(f, "  _[tokens: prompt=100+completion=20]_").unwrap();
        let (events, session) = parse_datalog(&path).expect("parse");
        assert_eq!(events.len(), 1);
        // 无 session 也能提取 model（不再显示「未知模型」）
        assert_eq!(events[0].model_raw.as_deref(), Some("GLM-5.2"));
        // 无 session → 会话回退到文件维度 id
        let s = session.expect("session");
        assert!(s.external_session_id.is_none());
        assert!(s.session_id.starts_with("atomcode:"));
        assert_eq!(s.model_set, vec!["GLM-5.2"]);
    }

    #[test]
    fn backfill_fills_null_model_rows() {
        use std::io::Write;
        // AtomCode 数据源：单轮、env 无 session（旧正则下 model 提取失败）
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("2026-06-17_11-10-55.md");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "# Turn 2026-06-17 11:10:55 [build:867bc77]").unwrap();
        writeln!(
            f,
            "**env:** model=GLM-5.2, ctx_window=200000, cwd=/Users/dev/wxbuddy"
        )
        .unwrap();
        writeln!(f, "### Turn 1").unwrap();
        writeln!(f, "  _[tokens: prompt=100+completion=20]_").unwrap();

        // 池侧 DB + 手工插入旧适配器采集的 NULL 模型行
        let db_dir = tempfile::tempdir().expect("tempdir");
        let database = crate::db::Database::new(&db_dir.path().join("gateway.db")).expect("open");
        database.run_migrations().expect("migrate");
        // 与 parse_datalog 一致地推导 locator hash
        let file_id = hash_short(&path.to_string_lossy());
        let locator = hash_short(&format!("atomcode:{file_id}:1"));
        {
            let conn = database.conn.lock().expect("lock");
            conn.execute(
                "INSERT INTO tool_definition (tool_id, display_name, kind, support_level) \
                 VALUES ('atomcode','AtomCode','usage','basic')",
                [],
            )
            .expect("tool def");
            conn.execute(
                "INSERT INTO usage_event (source_type, tool_id, device_id, model_raw, model_normalized, \
                 session_id, input_tokens, output_tokens, total_tokens, usage_accuracy, \
                 occurred_at, source_fingerprint, source_locator_hash) \
                 VALUES ('local_discovered','atomcode','local',NULL,NULL,NULL,100,20,120, \
                 'provider_reported','2026-06-17T11:10:55Z','stale-fingerprint',?1)",
                rusqlite::params![locator],
            )
            .expect("insert null row");
        }
        let source = DataSource {
            id: "atomcode:test".into(),
            path: path.clone(),
            format: DataFormat::Jsonl,
            watch: true,
        };
        let fixed = backfill_missing_models(&database.conn, &source).expect("backfill");
        assert_eq!(fixed, 1);
        let conn = database.conn.lock().expect("lock");
        let model: Option<String> = conn
            .query_row(
                "SELECT model_normalized FROM usage_event WHERE source_locator_hash=?1",
                rusqlite::params![locator],
                |r| r.get(0),
            )
            .expect("read back");
        assert_eq!(model.as_deref(), Some("GLM-5.2"));
    }

    #[test]
    fn mtime_unchanged_skips_reexport() {
        let (_dir, path) = sample_file();
        let source = DataSource {
            id: "atomcode:t".into(),
            path,
            format: DataFormat::Jsonl,
            watch: true,
        };
        let adapter = AtomCodeAdapter;
        let cp = CollectorCheckpoint {
            source_id: "atomcode:t".into(),
            mtime_ms: mtime_ms(&source.path),
            ..Default::default()
        };
        let r = adapter.collect_incremental(&source, cp).expect("collect");
        assert!(r.events.is_empty());
        assert!(r.sessions.is_empty());
    }
}
