//! Async request logging middleware.
//! Uses a tokio mpsc channel to decouple log writes from the proxy request path.
//! A background task batches writes to SQLite every 1s or every 100 events.

use crate::db::logs::RequestLog;
use crate::AppState;
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

/// Maximum events buffered before a forced flush.
const MAX_BATCH_SIZE: usize = 100;
/// Flush interval.
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// Event sent through the logging channel.
#[derive(Clone, Debug)]
pub struct LogEvent {
    pub group_id: Option<String>,
    /// Audit identity: the virtual client key that authenticated the request.
    pub client_key_id: Option<String>,
    pub request_id: Option<String>,
    pub attempt_count: i64,
    pub usage_available: bool,
    pub source: Option<String>,
    pub provider_id: Option<String>,
    pub account_id: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub status: String,
    pub status_code: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_tokens: Option<i64>,
    pub cost: Option<f64>,
    pub latency_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub is_stream: Option<bool>,
    pub error_message: Option<String>,
}

/// Log writer that sends events to a background writer task.
pub struct LogWriter {
    tx: mpsc::Sender<LogEvent>,
}

impl LogWriter {
    /// Create a new LogWriter with shared AppState.
    /// The background task accesses the database through the shared Arc<AppState>.
    pub fn new_with_app_state(state: &Arc<AppState>) -> Self {
        let (tx, rx) = mpsc::channel::<LogEvent>(1024);
        let state = Arc::clone(state);
        tokio::spawn(writer_task(rx, state));
        Self { tx }
    }

    /// Queue a log event for writing.
    pub async fn write_log(&self, event: LogEvent) -> Result<(), String> {
        self.tx
            .send(event)
            .await
            .map_err(|e| format!("Failed to send log event: {}", e))
    }
}

/// Background task that batches LogEvents and writes them to SQLite.
async fn writer_task(mut rx: mpsc::Receiver<LogEvent>, state: Arc<AppState>) {
    let mut batch: Vec<LogEvent> = Vec::with_capacity(MAX_BATCH_SIZE);
    let mut ticker = interval(FLUSH_INTERVAL);

    loop {
        tokio::select! {
            Some(event) = rx.recv() => {
                batch.push(event);
                if batch.len() >= MAX_BATCH_SIZE {
                    flush_batch(&batch, &state);
                    batch.clear();
                }
            }
            _ = ticker.tick() => {
                if !batch.is_empty() {
                    flush_batch(&batch, &state);
                    batch.clear();
                }
            }
        }
    }
}

/// Flush a batch of log events to the database.
fn flush_batch(events: &[LogEvent], state: &Arc<AppState>) {
    let db = &state.db;
    let conn = match db.conn.lock() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to acquire connection lock for logging: {}", e);
            return;
        }
    };

    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    for event in events {
        let sql = "INSERT INTO request_logs \
            (group_id, client_key_id, request_id, attempt_count, usage_available, source, \
             provider_id, account_id, model, endpoint, status, status_code, input_tokens, \
             output_tokens, cache_tokens, cost, latency_ms, ttft_ms, is_stream, error_message, request_at) \
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)";

        if let Err(e) = conn.execute(
            sql,
            rusqlite::params![
                event.group_id,
                event.client_key_id,
                event.request_id,
                event.attempt_count,
                event.usage_available,
                event.source,
                event.provider_id,
                event.account_id,
                event.model,
                event.endpoint,
                event.status,
                event.status_code,
                event.input_tokens,
                event.output_tokens,
                event.cache_tokens,
                event.cost,
                event.latency_ms,
                event.ttft_ms,
                event.is_stream,
                event
                    .error_message
                    .as_deref()
                    .map(crate::services::redaction::redact_sensitive),
                now,
            ],
        ) {
            tracing::error!("Failed to write request log: {}", e);
        }
    }
}

impl LogEvent {
    /// Create a LogEvent from a RequestLog row (used for re-logging).
    pub fn from_request_log(log: &RequestLog) -> Self {
        Self {
            group_id: log.group_id.clone(),
            client_key_id: log.client_key_id.clone(),
            request_id: log.request_id.clone(),
            attempt_count: log.attempt_count.unwrap_or(1),
            usage_available: log.usage_available.unwrap_or(false),
            source: log.source.clone(),
            provider_id: log.provider_id.clone(),
            account_id: log.account_id.clone(),
            model: log.model.clone(),
            endpoint: log.endpoint.clone(),
            status: log.status.clone().unwrap_or_else(|| "unknown".to_string()),
            status_code: log.status_code,
            input_tokens: log.input_tokens,
            output_tokens: log.output_tokens,
            cache_tokens: log.cache_tokens,
            cost: log.cost,
            latency_ms: log.latency_ms,
            ttft_ms: log.ttft_ms,
            is_stream: log.is_stream,
            error_message: log.error_message.clone(),
        }
    }
}
