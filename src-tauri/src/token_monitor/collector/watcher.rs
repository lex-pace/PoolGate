//! Native file watcher for local Token Monitor data sources.
//! The service layer owns debounce and collection scheduling.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use notify::Watcher as _;

use crate::token_monitor::model::DataSource;

#[derive(Debug, Clone)]
pub struct PendingSource {
    pub tool_id: String,
    pub source_id: String,
}

pub struct FileWatch {
    // Keeping this field alive keeps the native registration active.
    _watcher: notify::RecommendedWatcher,
    rx: mpsc::Receiver<PendingSource>,
}

// Directory watches report the changed child path, not the watch root. Matching
// descendants prevents new JSONL files and session rotations from waiting for a
// fallback polling cycle.
#[derive(Debug, Clone)]
struct WatchTarget {
    source_path: PathBuf,
    tool_id: String,
    source_id: String,
}

impl WatchTarget {
    fn matches(&self, event_path: &Path) -> bool {
        event_path.starts_with(&self.source_path)
    }
}

impl FileWatch {
    // File sources use their parent because direct file watching is unreliable
    // on some platforms; directory sources are watched recursively for sessions.
    pub fn watch(sources: &[(String, DataSource)]) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<PendingSource>();
        let targets: std::sync::Arc<Vec<WatchTarget>> = std::sync::Arc::new(
            sources
                .iter()
                .filter(|(_, source)| source.watch)
                .map(|(tool_id, source)| WatchTarget {
                    source_path: source.path.clone(),
                    tool_id: tool_id.clone(),
                    source_id: source.id.clone(),
                })
                .collect(),
        );
        let handler_tx = tx;
        let handler_targets = targets.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                let Ok(event) = result else {
                    return;
                };
                // Reading a JSONL/SQLite source may emit an access event. Ignore it
                // to prevent the collector from triggering another collection pass.
                if matches!(event.kind, notify::EventKind::Access(_)) {
                    return;
                }
                for path in &event.paths {
                    for target in handler_targets.iter().filter(|target| target.matches(path)) {
                        let _ = handler_tx.send(PendingSource {
                            tool_id: target.tool_id.clone(),
                            source_id: target.source_id.clone(),
                        });
                    }
                }
            })
            .map_err(|error| format!("notify watcher init: {error}"))?;

        let mut watched_count = 0usize;
        for (_, source) in sources {
            if !source.watch || !source.path.exists() {
                continue;
            }
            let target = if source.path.is_file() {
                source
                    .path
                    .parent()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| source.path.clone())
            } else {
                source.path.clone()
            };
            let mode = if source.path.is_dir() {
                notify::RecursiveMode::Recursive
            } else {
                notify::RecursiveMode::NonRecursive
            };
            watcher
                .watch(&target, mode)
                .map_err(|error| format!("watch {}: {error}", target.display()))?;
            watched_count += 1;
        }
        if watched_count == 0 {
            return Err("no watchable Token Monitor source paths".into());
        }
        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }

    pub fn recv_blocking(&self) -> Option<PendingSource> {
        self.rx.recv().ok()
    }

    /// 带超时接收：超时返回 None（调用方借此周期重检目标集合，
    /// 让新增/删除自定义应用等路径变更及时重建 watcher）。
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<PendingSource> {
        self.rx.recv_timeout(timeout).ok()
    }

    pub fn drain(&self) -> Vec<PendingSource> {
        let mut pending = Vec::new();
        while let Ok(pending_source) = self.rx.try_recv() {
            pending.push(pending_source);
        }
        pending
    }
}
