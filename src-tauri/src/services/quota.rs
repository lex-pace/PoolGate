//! Quota monitoring service.
//!
//! Reads `quota_limit` and `quota_used` from each account and calculates
//! usage percentages. Returns alerts when thresholds are exceeded:
//! - **Warning** (80%)
//! - **Critical** (95%)
//! - **Exhausted** (100%)

use crate::db::accounts::Account;
use rusqlite::Connection;
use std::sync::Mutex;

/// Severity of a quota alert.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum AlertLevel {
    /// Usage >= 80%
    Warning,
    /// Usage >= 95%
    Critical,
    /// Usage >= 100% (quota exhausted)
    Exhausted,
}

/// An alert generated when an account's quota usage crosses a threshold.
#[derive(Debug, Clone, serde::Serialize)]
pub struct QuotaAlert {
    pub account_id: String,
    pub account_name: Option<String>,
    pub usage_pct: f64,
    pub level: AlertLevel,
    pub quota_limit: f64,
    pub quota_used: f64,
}

/// Monitors account quotas and produces alerts when thresholds are crossed.
pub struct QuotaMonitor {
    /// Warning threshold (fraction, default 0.80)
    warning_threshold: f64,
    /// Critical threshold (fraction, default 0.95)
    critical_threshold: f64,
}

impl Default for QuotaMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl QuotaMonitor {
    /// Create a new `QuotaMonitor` with default thresholds (Warning at 80%,
    /// Critical at 95%).
    pub fn new() -> Self {
        Self {
            warning_threshold: 0.80,
            critical_threshold: 0.95,
        }
    }

    /// Create a `QuotaMonitor` with custom thresholds.
    ///
    /// # Panics
    /// Panics if `warning > critical` or either value is not in (0, 1].
    pub fn with_thresholds(warning: f64, critical: f64) -> Self {
        assert!(
            warning > 0.0 && warning <= 1.0,
            "warning threshold must be in (0, 1]"
        );
        assert!(
            critical > 0.0 && critical <= 1.0,
            "critical threshold must be in (0, 1]"
        );
        assert!(warning <= critical, "warning threshold must be <= critical");
        Self {
            warning_threshold: warning,
            critical_threshold: critical,
        }
    }

    /// Check quotas for all accounts in the database and return alerts.
    ///
    /// Only accounts with both `quota_limit` and `quota_used` set are
    /// evaluated. Accounts where `quota_limit <= 0` are skipped.
    pub fn check_quotas(&self, conn: &Mutex<Connection>) -> Vec<QuotaAlert> {
        let accounts = match crate::db::accounts::AccountRepo.list_all(conn) {
            Ok(accs) => accs,
            Err(e) => {
                tracing::warn!("QuotaMonitor: failed to list accounts: {}", e);
                return vec![];
            }
        };

        let mut alerts = Vec::new();

        for account in &accounts {
            let limit = match account.quota_limit {
                Some(l) if l > 0.0 => l,
                _ => continue, // No limit set or zero — skip
            };

            let used = account.quota_used.unwrap_or(0.0);
            let usage_pct = used / limit;

            let level = if usage_pct >= 1.0 {
                AlertLevel::Exhausted
            } else if usage_pct >= self.critical_threshold {
                AlertLevel::Critical
            } else if usage_pct >= self.warning_threshold {
                AlertLevel::Warning
            } else {
                continue; // Below all thresholds
            };

            alerts.push(QuotaAlert {
                account_id: account.id.clone(),
                account_name: account.name.clone(),
                usage_pct: (usage_pct * 10000.0).round() / 100.0, // To 2 decimal places
                level,
                quota_limit: limit,
                quota_used: used,
            });
        }

        alerts
    }

    /// Convenience: return only Critical and Exhausted alerts.
    pub fn check_critical_quotas(&self, conn: &Mutex<Connection>) -> Vec<QuotaAlert> {
        self.check_quotas(conn)
            .into_iter()
            .filter(|a| matches!(a.level, AlertLevel::Critical | AlertLevel::Exhausted))
            .collect()
    }

    /// Check an individual account's quota (without hitting the DB).
    pub fn check_account_quota(&self, account: &Account) -> Option<QuotaAlert> {
        let limit = account.quota_limit?;
        if limit <= 0.0 {
            return None;
        }
        let used = account.quota_used.unwrap_or(0.0);
        let usage_pct = used / limit;

        let level = if usage_pct >= 1.0 {
            AlertLevel::Exhausted
        } else if usage_pct >= self.critical_threshold {
            AlertLevel::Critical
        } else if usage_pct >= self.warning_threshold {
            AlertLevel::Warning
        } else {
            return None;
        };

        Some(QuotaAlert {
            account_id: account.id.clone(),
            account_name: account.name.clone(),
            usage_pct: (usage_pct * 10000.0).round() / 100.0,
            level,
            quota_limit: limit,
            quota_used: used,
        })
    }
}
