//! Token Monitor 服务运行时（W1：聚合/快照/事件广播；W2：采集调度经 `service_collect` 接入）。
//!
//! 生命周期挂 `AppState`，不依赖 `ProxyHandle`（Gateway 关闭时 Token Monitor 继续工作）。
//! 事件走 `token-monitor:usage-delta`，由 lib.rs 的 250ms 合并窗口广播（复刻 topology 范式）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast;

use crate::token_monitor::model::{SessionSummary, ToolUsageRow, UsageDelta};
use crate::AppState;

/// Token Monitor 运行时。
///
/// 持有事件 broadcast 发送端与序列号；`publish_usage_delta` 在采集落库后由采集循环
/// （W2 `service_collect`）调用，快速聚合今日数据并广播。快照本体由命令层经 Repo 查询
/// 生成（`get_token_monitor_snapshot`），不常驻全量缓存，保证托盘 150ms 目标。
/// 会话变更走独立通道 `session_events`（`token-monitor:session-changed`，W7 新增）。
pub struct TokenMonitorRuntime {
    sequence: AtomicU64,
    events: broadcast::Sender<UsageDelta>,
    session_events: broadcast::Sender<SessionSummary>,
}

impl Default for TokenMonitorRuntime {
    fn default() -> Self {
        let (events, _) = broadcast::channel(64);
        let (session_events, _) = broadcast::channel(64);
        Self {
            sequence: AtomicU64::new(0),
            events,
            session_events,
        }
    }
}

impl TokenMonitorRuntime {
    pub fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Relaxed)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<UsageDelta> {
        self.events.subscribe()
    }

    /// 订阅会话变更（`token-monitor:session-changed` 的 broadcast 源）。
    pub fn subscribe_sessions(&self) -> broadcast::Receiver<SessionSummary> {
        self.session_events.subscribe()
    }

    /// 采集落库会话摘要后调用：广播 `SessionSummary`（新增/更新），
    /// 前端据此实时刷新会话列表。无接收者时静默丢弃（测试环境）。
    pub fn publish_session_changed(&self, session: SessionSummary) {
        let _ = self.session_events.send(session);
    }

    /// 采集落库后调用：聚合今日（day 口径）总量与 Top 工具并广播 `UsageDelta`。
    /// 失败静默降级：不广播零值 delta（避免 UI 增量叠加把托盘清零），不 panic 全局。
    pub fn publish_usage_delta(&self, state: &Arc<AppState>) {
        let repo = &state.db.usage_events;
        let Ok((_input, _output, _cache, total_tokens)) =
            repo.unified_range_stats(&state.db.conn, "day")
        else {
            return;
        };
        let top_tools = repo
            .tool_usage_rows(&state.db.conn, "day")
            .unwrap_or_default();
        let delta = UsageDelta {
            sequence: self.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            emitted_at: chrono::Utc::now().to_rfc3339(),
            range: "day".to_string(),
            total_tokens,
            cost_amount: None,
            active_tools: top_tools.len() as u32,
            top_tools: top_tools.into_iter().take(5).collect::<Vec<ToolUsageRow>>(),
        };
        let _ = self.events.send(delta);
    }
}

/// 进程级 AppHandle（采集告警等后台路径需要 emit 事件/发系统通知；setup 时注册一次）。
static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// 取已注册的 AppHandle（未注册（如单元测试）返回 None，告警路径静默跳过）。
pub fn app_handle() -> Option<&'static tauri::AppHandle> {
    APP_HANDLE.get()
}

fn register_app_handle(app: &tauri::AppHandle) {
    let _ = APP_HANDLE.set(app.clone());
}

/// 启动 Token Monitor 服务（在 `lib.rs` `.setup()` 中调用）。
///
/// W1：事件转发（`token-monitor:usage-delta`，250ms 合并窗口）+ 低频维护循环
/// （每日一次清理 30 天前原始事件；批量、离热路径）。
/// W2 的采集调度（watcher/轮询）由集成负责人在此追加一行 `service_collect::start(...)`。
pub fn init(app: &tauri::AppHandle, state: Arc<AppState>) -> Result<(), String> {
    register_app_handle(app);
    tracing::info!("token_monitor::init: service started");

    // W2 采集调度（watcher + 降级轮询）：注册表为空时静默无操作，W3 注册后自动生效。
    if let Err(error) = crate::token_monitor::service_collect::start(app, state.clone()) {
        tracing::warn!("token_monitor: collector start failed: {}", error);
    }

    // W4 额度后台刷新（5 分钟；剩余 <20% 缩到 2 分钟；401/403 熔断）。
    // W5：每轮刷新后评估告警 + 更新托盘 tooltip 主指标（需 AppHandle）。
    crate::token_monitor::quota::spawn_refresh_loop(app.clone(), state.clone());

    // 事件转发：订阅运行时广播，250ms 合并窗口后 emit `token-monitor:usage-delta`
    // （复刻 lib.rs 的 topology:runtime-delta 范式，避免 UI 收到每条日志的全量刷新）。
    {
        use tauri::Emitter;
        let app = app.clone();
        let mut events = state.token_monitor.subscribe();
        tauri::async_runtime::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(delta) => {
                        let mut latest = delta;
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        // 合并窗口内丢弃中间态，只发最新一版
                        while let Ok(delta) = events.try_recv() {
                            latest = delta;
                        }
                        let _ = app.emit("token-monitor:usage-delta", latest);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // 追不上：跳过本次，等下一版（避免用旧快照误导 UI）
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // 会话变更转发：同样的 250ms 合并窗口 → emit `token-monitor:session-changed`
    // （W7，契约 03 §6；首扫/批量采集的会话突发合并成一条，前端只需失效重取）。
    {
        use tauri::Emitter;
        let app = app.clone();
        let mut sessions = state.token_monitor.subscribe_sessions();
        tauri::async_runtime::spawn(async move {
            loop {
                match sessions.recv().await {
                    Ok(session) => {
                        let mut latest = session;
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        while let Ok(session) = sessions.try_recv() {
                            latest = session;
                        }
                        let _ = app.emit("token-monitor:session-changed", latest);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // 追不上：跳过本次，等下一版
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // 每日清理：usage_event 保留 30 天（02 §6），批量删除超期行。
    let cleanup_state = state.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)).await;
            if let Ok(conn) = cleanup_state.db.conn.lock() {
                let cutoff = (chrono::Utc::now() - chrono::Duration::days(30))
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string();
                let _ = conn.execute(
                    "DELETE FROM usage_event WHERE occurred_at < ?1",
                    rusqlite::params![cutoff],
                );
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str) -> SessionSummary {
        SessionSummary {
            session_id: id.into(),
            tool_id: "claude_code".into(),
            external_session_id: None,
            project_id: None,
            title_redacted: None,
            model_set: vec!["claude-3.5-sonnet".into()],
            started_at: None,
            last_active_at: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_tokens: 0,
            total_tokens: 0,
            message_count: 0,
            status: None,
            cost_amount: None,
        }
    }

    #[test]
    fn session_changed_broadcast_delivers_summary() {
        let rt = TokenMonitorRuntime::default();
        let mut rx = rt.subscribe_sessions();
        rt.publish_session_changed(session("s1"));
        let got = rx.try_recv().expect("session event");
        assert_eq!(got.session_id, "s1");
        assert_eq!(got.tool_id, "claude_code");
    }

    #[test]
    fn session_changed_broadcast_no_receiver_is_noop() {
        // 采集路径在无订阅者（如单元测试）时调用不应 panic
        let rt = TokenMonitorRuntime::default();
        rt.publish_session_changed(session("s2"));
        // 无接收者 → send 返回 Err，静默忽略；此处到达即通过
    }
}
