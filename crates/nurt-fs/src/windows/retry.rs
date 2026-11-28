// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::windows::retry
// Responsibility:   Retries after sharing and lock violations.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Retries after sharing and lock violations.
//!
//! Windows locks are mandatory: a file opened without `FILE_SHARE_DELETE`
//! cannot be renamed or removed, and a byte-range lock blocks reads and
//! writes. Such conflicts often end within milliseconds (indexers, virus
//! scanners, backup agents), so the back end retries with growing intervals
//! for a bounded period before reporting the entry as in use.

use std::time::Duration;

/// Default retry period (`fs.sharing_retry_ms`).
pub const DEFAULT_SHARING_RETRY: Duration = Duration::from_millis(2_000);

/// First interval between attempts.
pub const FIRST_DELAY: Duration = Duration::from_millis(10);

/// Upper bound of a single interval.
pub const MAX_DELAY: Duration = Duration::from_millis(250);

/// Retry schedule bounded by a total waiting time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    budget: Duration,
}

impl RetryPolicy {
    /// Policy that waits no longer than `budget` in total; a zero budget makes
    /// exactly one attempt.
    #[must_use]
    pub const fn new(budget: Duration) -> Self {
        Self { budget }
    }

    /// Total waiting time.
    #[must_use]
    pub const fn budget(&self) -> Duration {
        self.budget
    }

    /// Intervals between attempts: doubling from [`FIRST_DELAY`] up to
    /// [`MAX_DELAY`], with the last interval shortened so that the sum equals
    /// the budget.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use nurt_fs::windows::retry::RetryPolicy;
    ///
    /// let delays: Vec<_> = RetryPolicy::new(Duration::from_millis(50)).delays().collect();
    /// assert_eq!(
    ///     delays,
    ///     [10, 20, 20].map(Duration::from_millis)
    /// );
    /// ```
    #[must_use]
    pub fn delays(&self) -> Delays {
        Delays {
            remaining: self.budget,
            next: FIRST_DELAY,
        }
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::new(DEFAULT_SHARING_RETRY)
    }
}

/// Iterator over the intervals of a [`RetryPolicy`].
#[derive(Debug, Clone)]
pub struct Delays {
    remaining: Duration,
    next: Duration,
}

impl Iterator for Delays {
    type Item = Duration;

    fn next(&mut self) -> Option<Duration> {
        if self.remaining.is_zero() {
            return None;
        }
        let delay = self.next.min(self.remaining);
        self.remaining -= delay;
        self.next = self.next.saturating_mul(2).min(MAX_DELAY);
        Some(delay)
    }
}

/// Runs `attempt` until it succeeds, fails with an error that `retryable`
/// rejects, or the schedule of `policy` is exhausted; `sleep` waits between
/// attempts.
///
/// # Errors
///
/// Returns the error of the last attempt.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use nurt_fs::windows::retry::{run, RetryPolicy};
///
/// let mut waited = Duration::ZERO;
/// let mut attempts = 0;
/// let result: Result<(), u32> = run(
///     &RetryPolicy::new(Duration::from_millis(30)),
///     |delay| waited += delay,
///     || {
///         attempts += 1;
///         Err(32)
///     },
///     |code| *code == 32,
/// );
/// assert_eq!(result, Err(32));
/// assert_eq!(waited, Duration::from_millis(30));
/// assert_eq!(attempts, 3);
/// ```
pub fn run<T, E>(
    policy: &RetryPolicy,
    mut sleep: impl FnMut(Duration),
    mut attempt: impl FnMut() -> Result<T, E>,
    retryable: impl Fn(&E) -> bool,
) -> Result<T, E> {
    let mut delays = policy.delays();
    loop {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(error) if retryable(&error) => match delays.next() {
                Some(delay) => sleep(delay),
                None => return Err(error),
            },
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows::error::codes::{ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION};
    use crate::windows::error::is_sharing_conflict;

    fn millis(values: &[u64]) -> Vec<Duration> {
        values.iter().copied().map(Duration::from_millis).collect()
    }

    #[test]
    fn grows_intervals_up_to_the_cap_within_the_budget() {
        let delays: Vec<_> = RetryPolicy::default().delays().collect();
        assert_eq!(
            delays,
            millis(&[10, 20, 40, 80, 160, 250, 250, 250, 250, 250, 250, 190])
        );
        assert_eq!(delays.iter().sum::<Duration>(), DEFAULT_SHARING_RETRY);
    }

    #[test]
    fn makes_a_single_attempt_with_zero_budget() {
        assert_eq!(RetryPolicy::new(Duration::ZERO).delays().next(), None);
        let mut attempts = 0;
        let result: Result<(), u32> = run(
            &RetryPolicy::new(Duration::ZERO),
            |_| panic!("no wait expected"),
            || {
                attempts += 1;
                Err(ERROR_SHARING_VIOLATION)
            },
            |code| is_sharing_conflict(*code),
        );
        assert_eq!(result, Err(ERROR_SHARING_VIOLATION));
        assert_eq!(attempts, 1);
    }

    #[test]
    fn stops_after_the_budget_with_the_last_sharing_error() {
        let mut clock = Duration::ZERO;
        let mut attempts = 0_u32;
        let result: Result<(), u32> = run(
            &RetryPolicy::default(),
            |delay| clock += delay,
            || {
                attempts += 1;
                Err(ERROR_SHARING_VIOLATION)
            },
            |code| is_sharing_conflict(*code),
        );
        assert_eq!(result, Err(ERROR_SHARING_VIOLATION));
        assert_eq!(clock, DEFAULT_SHARING_RETRY);
        assert_eq!(attempts, 13);
    }

    #[test]
    fn succeeds_once_the_conflict_ends() {
        let mut slept = Vec::new();
        let mut remaining_conflicts = 3;
        let result = run(
            &RetryPolicy::default(),
            |delay| slept.push(delay),
            || {
                if remaining_conflicts == 0 {
                    Ok("done")
                } else {
                    remaining_conflicts -= 1;
                    Err(ERROR_SHARING_VIOLATION)
                }
            },
            |code| is_sharing_conflict(*code),
        );
        assert_eq!(result, Ok("done"));
        assert_eq!(slept, millis(&[10, 20, 40]));
    }

    #[test]
    fn does_not_retry_other_errors() {
        let mut attempts = 0;
        let result: Result<(), u32> = run(
            &RetryPolicy::default(),
            |_| panic!("no wait expected"),
            || {
                attempts += 1;
                Err(ERROR_ACCESS_DENIED)
            },
            |code| is_sharing_conflict(*code),
        );
        assert_eq!(result, Err(ERROR_ACCESS_DENIED));
        assert_eq!(attempts, 1);
    }

    #[test]
    fn reports_the_configured_budget() {
        let policy = RetryPolicy::new(Duration::from_millis(5));
        assert_eq!(policy.budget(), Duration::from_millis(5));
        assert_eq!(policy.delays().collect::<Vec<_>>(), millis(&[5]));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
