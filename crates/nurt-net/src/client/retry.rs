// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-net::client::retry
// Responsibility:   Retries after transient failures.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Retries after transient failures.

use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode};

/// Pause before the first retry.
const DEFAULT_BASE: Duration = Duration::from_millis(500);
/// Longest computed pause between attempts.
const DEFAULT_MAX_PAUSE: Duration = Duration::from_secs(2);
/// Longest pause requested by a `Retry-After` header that is honoured.
const DEFAULT_RETRY_AFTER_MAX: Duration = Duration::from_secs(5);
/// Status `429 Too Many Requests`.
const TOO_MANY_REQUESTS: u16 = 429;

/// Pauses between attempts of one request.
///
/// The pause before retry `n` (from 0) is `base · 2ⁿ`, at most `max_pause`.
/// A `Retry-After` header given in seconds replaces it, at most
/// `retry_after_max`.
///
/// ```
/// use std::time::Duration;
/// use nurt_net::client::RetryPolicy;
///
/// let policy = RetryPolicy::default();
/// assert_eq!(policy.pause(0, None), Duration::from_millis(500));
/// assert_eq!(policy.pause(5, None), Duration::from_secs(2));
/// assert_eq!(policy.pause(0, Some("3")), Duration::from_secs(3));
/// assert_eq!(policy.pause(0, Some("60")), Duration::from_secs(5));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Pause before the first retry.
    pub base: Duration,
    /// Longest computed pause.
    pub max_pause: Duration,
    /// Longest honoured `Retry-After` pause.
    pub retry_after_max: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base: DEFAULT_BASE,
            max_pause: DEFAULT_MAX_PAUSE,
            retry_after_max: DEFAULT_RETRY_AFTER_MAX,
        }
    }
}

impl RetryPolicy {
    /// Pause before retry `retry`, counted from 0, with the value of the
    /// `Retry-After` header of the failed response.
    #[must_use]
    pub fn pause(&self, retry: u32, retry_after: Option<&str>) -> Duration {
        if let Some(seconds) = retry_after.and_then(|text| text.trim().parse::<u64>().ok()) {
            return Duration::from_secs(seconds).min(self.retry_after_max);
        }
        let factor = 1_u32.checked_shl(retry).unwrap_or(u32::MAX);
        self.base.saturating_mul(factor).min(self.max_pause)
    }
}

/// Whether a response status is retried: 429 and every 5xx status.
#[must_use]
pub(crate) fn retryable_status(status: u16) -> bool {
    status == TOO_MANY_REQUESTS || (500..600).contains(&status)
}

/// Whether a failed attempt is retried: connection failures and broken
/// connections, but not TLS, proxy, redirect or decoding errors and not the
/// time limit of the request.
#[must_use]
pub(crate) fn retryable_error(error: &Error) -> bool {
    error.code() == ErrorCode::Network
        && matches!(
            error.data().problem.as_deref(),
            Some("connect" | "reset" | "dns")
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_statuses() {
        assert!(retryable_status(429));
        assert!(retryable_status(503));
        assert!(!retryable_status(404));
        assert!(!retryable_status(200));
    }

    #[test]
    fn classifies_errors() {
        let reset = Error::new(ErrorCode::Network, "reset").with_problem("reset");
        let tls = Error::new(ErrorCode::Network, "tls").with_problem("tls");
        let timeout = Error::new(ErrorCode::Timeout, "slow");
        assert!(retryable_error(&reset));
        assert!(!retryable_error(&tls));
        assert!(!retryable_error(&timeout));
    }

    #[test]
    fn grows_exponentially_without_overflow() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.pause(1, None), Duration::from_secs(1));
        assert_eq!(policy.pause(40, None), Duration::from_secs(2));
        assert_eq!(policy.pause(1, Some("soon")), Duration::from_secs(1));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
