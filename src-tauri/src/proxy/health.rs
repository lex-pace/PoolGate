//! Circuit breaker for account health.
//! Tracks consecutive failures per account and opens the circuit
//! when a threshold is exceeded, with automatic half-open recovery.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct BreakerState {
    pub consecutive_failures: u32,
    pub last_failure: Option<Instant>,
    pub is_open: bool,
    /// True after the recovery probe has been admitted and is still in flight.
    /// This prevents concurrent requests from bypassing the half-open probe.
    pub half_open: bool,
}

pub struct CircuitBreaker {
    failure_threshold: u32,
    recovery_interval: Duration,
    state: Arc<RwLock<HashMap<String, BreakerState>>>,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, recovery_interval_secs: u64) -> Self {
        Self {
            failure_threshold,
            recovery_interval: Duration::from_secs(recovery_interval_secs),
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn record_failure(&self, account_id: &str) {
        let mut state = self.state.write().await;
        let entry = state.entry(account_id.to_string()).or_insert(BreakerState {
            consecutive_failures: 0,
            last_failure: None,
            is_open: false,
            half_open: false,
        });
        entry.consecutive_failures += 1;
        entry.last_failure = Some(Instant::now());

        if entry.consecutive_failures >= self.failure_threshold {
            entry.is_open = true;
            entry.half_open = false;
            tracing::warn!(
                "Circuit breaker OPEN for account {} after {} consecutive failures",
                account_id,
                entry.consecutive_failures
            );
        }
    }

    pub async fn record_success(&self, account_id: &str) {
        let mut state = self.state.write().await;
        let entry = state.entry(account_id.to_string()).or_insert(BreakerState {
            consecutive_failures: 0,
            last_failure: None,
            is_open: false,
            half_open: false,
        });
        // Reset on success
        entry.consecutive_failures = 0;
        entry.is_open = false;
        entry.half_open = false;
    }

    pub async fn is_open(&self, account_id: &str) -> bool {
        let state = self.state.read().await;
        match state.get(account_id) {
            None => false,
            Some(entry) => {
                if !entry.is_open {
                    return false;
                }
                // Check if recovery interval has elapsed -> half-open
                if let Some(last_fail) = entry.last_failure {
                    if last_fail.elapsed() >= self.recovery_interval {
                        // Transition to half-open (will be tried next)
                        // We don't mutate here; the caller should call allow_request
                        return false; // allow through for half-open
                    }
                }
                true
            }
        }
    }

    /// Returns true if the request is allowed. For half-open state,
    /// allows a single probe request.
    pub async fn allow_request(&self, account_id: &str) -> bool {
        let mut state = self.state.write().await;
        match state.get_mut(account_id) {
            None => true,
            Some(entry) => {
                // A half-open probe is already running. Keep every other request
                // out until that probe records success or failure.
                if entry.half_open {
                    return false;
                }
                if !entry.is_open {
                    return true;
                }
                // Check recovery. Keep `is_open` true while the single probe is
                // running so concurrent requests cannot treat the circuit as closed.
                if let Some(last_fail) = entry.last_failure {
                    if last_fail.elapsed() >= self.recovery_interval {
                        entry.half_open = true;
                        tracing::info!(
                            "Circuit breaker HALF-OPEN for account {} — allowing one probe",
                            account_id
                        );
                        return true;
                    }
                }
                false
            }
        }
    }

    pub async fn get_status(&self, account_id: &str) -> String {
        let state = self.state.read().await;
        match state.get(account_id) {
            None => "closed".to_string(),
            Some(entry) => {
                if entry.half_open {
                    "half_open".to_string()
                } else if entry.is_open {
                    "open".to_string()
                } else {
                    "closed".to_string()
                }
            }
        }
    }

    /// Reset all breaker state for a given account
    pub async fn reset(&self, account_id: &str) {
        let mut state = self.state.write().await;
        state.remove(account_id);
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(3, 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_closed_by_default() {
        let cb = CircuitBreaker::default();
        assert!(!cb.is_open("test-account").await);
        assert_eq!(cb.get_status("test-account").await, "closed");
    }

    #[tokio::test]
    async fn test_opens_after_threshold() {
        let cb = CircuitBreaker::new(2, 60);
        assert!(!cb.is_open("a1").await);
        cb.record_failure("a1").await;
        assert!(!cb.is_open("a1").await);
        cb.record_failure("a1").await;
        assert!(cb.is_open("a1").await);
        assert_eq!(cb.get_status("a1").await, "open");
    }

    #[tokio::test]
    async fn test_success_resets() {
        let cb = CircuitBreaker::new(2, 60);
        cb.record_failure("a1").await;
        cb.record_failure("a1").await;
        assert!(cb.is_open("a1").await);
        cb.record_success("a1").await;
        assert!(!cb.is_open("a1").await);
    }

    #[tokio::test]
    async fn test_half_open_allows_only_one_probe() {
        let cb = CircuitBreaker::new(1, 0);
        cb.record_failure("a1").await;
        assert!(cb.allow_request("a1").await);
        assert!(!cb.allow_request("a1").await);
        assert_eq!(cb.get_status("a1").await, "half_open");
        cb.record_success("a1").await;
        assert!(cb.allow_request("a1").await);
    }
}
