use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const DEFAULT_ACCOUNT_CONCURRENCY: usize = 4;
pub const DEFAULT_QUEUE_TIMEOUT: Duration = Duration::from_secs(15);

/// Minimum spacing between upstream dispatches on one subscription (OAuth)
/// account. Subscription backends flag machine-perfect cadences; a floor
/// spacing keeps the request rate human-plausible.
const SUBSCRIPTION_MIN_SPACING_MS: u64 = 200;
/// Random jitter added to the spacing so the interval is never constant.
const SUBSCRIPTION_JITTER_MS: u64 = 120;

/// Whether an account's credential is a subscription/OAuth credential (as
/// opposed to a metered API key). Only subscription accounts get behavioral
/// throttling — API-key traffic is billed per token and needs no pacing.
pub fn is_subscription_credential(credential_type: Option<&str>) -> bool {
    matches!(
        credential_type,
        Some("oauth" | "token" | "codex_oauth" | "claude_oauth" | "gemini_oauth" | "grok_oauth" | "copilot_pat")
    )
}

/// Per-account request pacing for subscription upstreams.
///
/// Unlike the semaphore (concurrency cap), this enforces a minimum spacing
/// between request *starts* with random jitter, shaping the traffic into a
/// human-like cadence that provider risk models tolerate.
#[derive(Clone, Default)]
pub struct AccountThrottle {
    next_slot: Arc<Mutex<HashMap<String, std::time::Instant>>>,
}

impl AccountThrottle {
    /// Wait until this account may dispatch its next upstream request.
    /// `subscription` should come from [`is_subscription_credential`]; metered
    /// API-key accounts pass through untouched.
    pub async fn wait_turn(&self, account_id: &str, subscription: bool) {
        if !subscription {
            return;
        }
        let jitter = rand::random::<u64>() % SUBSCRIPTION_JITTER_MS;
        let spacing = Duration::from_millis(SUBSCRIPTION_MIN_SPACING_MS + jitter);
        let slot = {
            let Ok(mut slots) = self.next_slot.lock() else {
                return;
            };
            let now = std::time::Instant::now();
            // Reserve the next free slot: at least `spacing` after the
            // previous reservation (back-to-back dispatches queue up), or
            // immediately when the account has been idle long enough.
            let slot = slots
                .get(account_id)
                .copied()
                .map_or(now, |previous| (previous + spacing).max(now));
            slots.insert(account_id.to_string(), slot);
            slot
        };
        let now = std::time::Instant::now();
        if slot > now {
            tokio::time::sleep(slot - now).await;
        }
    }
}

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

    #[tokio::test]
    async fn throttle_paces_subscription_accounts_only() {
        let throttle = AccountThrottle::default();
        // API-key accounts pass through with no pacing at all.
        throttle.wait_turn("acct-key", false).await;

        // Two back-to-back subscription dispatches must be spaced apart
        // (>= the minimum spacing). Allow generous slack for slow CI machines.
        let start = std::time::Instant::now();
        throttle.wait_turn("acct-oauth", true).await;
        throttle.wait_turn("acct-oauth", true).await;
        assert!(
            start.elapsed() >= Duration::from_millis(SUBSCRIPTION_MIN_SPACING_MS),
            "second dispatch must wait for the pacing slot"
        );

        // Different accounts are paced independently.
        let start = std::time::Instant::now();
        throttle.wait_turn("acct-oauth-2", true).await;
        assert!(start.elapsed() < Duration::from_millis(SUBSCRIPTION_MIN_SPACING_MS));
    }

    #[test]
    fn subscription_credential_classification() {
        assert!(is_subscription_credential(Some("codex_oauth")));
        assert!(is_subscription_credential(Some("claude_oauth")));
        assert!(is_subscription_credential(Some("copilot_pat")));
        assert!(is_subscription_credential(Some("oauth")));
        assert!(!is_subscription_credential(Some("api_key")));
        assert!(!is_subscription_credential(Some("upstream_key")));
        assert!(!is_subscription_credential(None));
    }
}
