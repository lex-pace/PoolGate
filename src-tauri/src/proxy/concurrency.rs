use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const DEFAULT_ACCOUNT_CONCURRENCY: usize = 4;
pub const DEFAULT_QUEUE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, serde::Serialize)]
pub struct AccountConcurrencySnapshot {
    pub limit: usize,
    pub active: usize,
    pub available: usize,
    pub queued: usize,
}

/// Per-account capacity registry. Semaphores are created lazily and shared by
/// all protocols, so one upstream account cannot be overloaded by concurrent
/// Agent traffic through different compatible endpoints.
#[derive(Clone)]
pub struct AccountConcurrency {
    semaphores: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
    queued: Arc<Mutex<HashMap<String, usize>>>,
    permits_per_account: usize,
    queue_timeout: Duration,
}

impl Default for AccountConcurrency {
    fn default() -> Self {
        Self::new(DEFAULT_ACCOUNT_CONCURRENCY, DEFAULT_QUEUE_TIMEOUT)
    }
}

impl AccountConcurrency {
    pub fn new(permits_per_account: usize, queue_timeout: Duration) -> Self {
        Self {
            semaphores: Arc::new(Mutex::new(HashMap::new())),
            queued: Arc::new(Mutex::new(HashMap::new())),
            permits_per_account: permits_per_account.max(1),
            queue_timeout,
        }
    }

    pub async fn acquire(&self, account_id: &str) -> Result<OwnedSemaphorePermit, String> {
        let semaphore = {
            let mut registry = self
                .semaphores
                .lock()
                .map_err(|error| format!("Account capacity registry is unavailable: {}", error))?;
            registry
                .entry(account_id.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(self.permits_per_account)))
                .clone()
        };
        let is_queued = semaphore.available_permits() == 0;
        if is_queued {
            if let Ok(mut queued) = self.queued.lock() {
                *queued.entry(account_id.to_string()).or_default() += 1;
            }
        }
        let result = tokio::time::timeout(self.queue_timeout, semaphore.acquire_owned())
            .await
            .map_err(|_| {
                format!(
                    "Account capacity queue timed out after {}ms",
                    self.queue_timeout.as_millis()
                )
            })?
            .map_err(|_| "Account capacity gate was closed".to_string());
        if is_queued {
            if let Ok(mut queued) = self.queued.lock() {
                let count = queued.entry(account_id.to_string()).or_default();
                *count = count.saturating_sub(1);
            }
        }
        result
    }

    pub fn snapshot(&self, account_id: &str) -> AccountConcurrencySnapshot {
        let available = self
            .semaphores
            .lock()
            .ok()
            .and_then(|registry| registry.get(account_id).cloned())
            .map(|semaphore| semaphore.available_permits())
            .unwrap_or(self.permits_per_account);
        let queued = self
            .queued
            .lock()
            .ok()
            .and_then(|registry| registry.get(account_id).copied())
            .unwrap_or(0);
        AccountConcurrencySnapshot {
            limit: self.permits_per_account,
            active: self.permits_per_account.saturating_sub(available),
            available,
            queued,
        }
    }

    #[cfg(test)]
    pub async fn available_permits(&self, account_id: &str) -> usize {
        self.snapshot(account_id).available
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn capacity_is_scoped_per_account_and_released_on_drop() {
        let gate = AccountConcurrency::new(1, Duration::from_millis(20));
        let permit = gate.acquire("acct-a").await.unwrap();
        assert_eq!(gate.available_permits("acct-a").await, 0);
        assert!(gate.acquire("acct-a").await.is_err());
        assert!(gate.acquire("acct-b").await.is_ok());
        drop(permit);
        assert!(gate.acquire("acct-a").await.is_ok());
    }
}
