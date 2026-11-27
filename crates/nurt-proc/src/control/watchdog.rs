// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::control::watchdog
// Responsibility:   Limit decisions of a running tree.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Limit decisions of a running tree.
//!
//! Times are active times: time since the start without the time spent
//! suspended, so suspension counts neither towards `timeout` nor towards
//! `idle_timeout`.

use std::time::Duration;

use crate::iface::{ProcessLimits, ResourceUsage};

/// Measurement interval while a processor or memory limit applies.
const LIMIT_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
/// Number of measurements within one idle window.
const IDLE_SAMPLES_PER_WINDOW: u32 = 10;
/// Shortest measurement interval for the idle limit alone.
const IDLE_SAMPLE_MIN: Duration = Duration::from_millis(100);
/// Longest measurement interval for the idle limit alone.
const IDLE_SAMPLE_MAX: Duration = Duration::from_secs(1);

/// Limit reached by a tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Measured memory reached `max_memory`.
    Memory,
    /// Measured processor time reached `max_cpu`.
    Cpu,
    /// Active time reached `timeout`.
    Timeout,
    /// No output and no processor use for `idle_timeout`.
    Idle,
}

/// Tracks activity and resource use against the limits of one tree.
#[derive(Debug, Clone)]
pub(crate) struct Watchdog {
    timeout: Option<Duration>,
    idle_timeout: Option<Duration>,
    max_cpu: Option<Duration>,
    max_memory: Option<u64>,
    last_activity: Duration,
    last_output: Option<Duration>,
    usage: ResourceUsage,
}

impl Watchdog {
    pub(crate) fn new(limits: &ProcessLimits) -> Self {
        Self {
            timeout: limits.timeout,
            idle_timeout: limits.idle_timeout,
            max_cpu: limits.max_cpu,
            max_memory: limits.max_memory,
            last_activity: Duration::ZERO,
            last_output: None,
            usage: ResourceUsage::default(),
        }
    }

    /// Interval between measurements; `None` when no limit needs them.
    pub(crate) fn sample_interval(&self) -> Option<Duration> {
        if self.max_cpu.is_some() || self.max_memory.is_some() {
            return Some(LIMIT_SAMPLE_INTERVAL);
        }
        self.idle_timeout
            .map(|idle| (idle / IDLE_SAMPLES_PER_WINDOW).clamp(IDLE_SAMPLE_MIN, IDLE_SAMPLE_MAX))
    }

    /// Records the time of the last output byte seen at active time `now`.
    pub(crate) fn observe_output(&mut self, now: Duration, last_output: Option<Duration>) {
        if last_output != self.last_output {
            self.last_output = last_output;
            self.last_activity = now;
        }
    }

    /// Records a measurement taken at active time `now`. Any growth of the
    /// processor time counts as activity.
    pub(crate) fn observe_usage(&mut self, now: Duration, usage: ResourceUsage) {
        if usage.cpu_time > self.usage.cpu_time {
            self.last_activity = now;
        }
        self.usage = ResourceUsage {
            cpu_time: self.usage.cpu_time.max(usage.cpu_time),
            max_rss: usage.max_rss,
        };
    }

    /// Limit reached at active time `now`, the most severe first.
    pub(crate) fn verdict(&self, now: Duration) -> Option<Verdict> {
        if self
            .max_memory
            .is_some_and(|limit| self.usage.max_rss >= limit)
        {
            return Some(Verdict::Memory);
        }
        if self
            .max_cpu
            .is_some_and(|limit| self.usage.cpu_time >= limit)
        {
            return Some(Verdict::Cpu);
        }
        if self.timeout.is_some_and(|limit| now >= limit) {
            return Some(Verdict::Timeout);
        }
        if self
            .idle_timeout
            .is_some_and(|limit| now.saturating_sub(self.last_activity) >= limit)
        {
            return Some(Verdict::Idle);
        }
        None
    }

    /// Active time until the next time-based limit; `None` without one.
    pub(crate) fn until_deadline(&self, now: Duration) -> Option<Duration> {
        let timeout = self.timeout.map(|limit| limit.saturating_sub(now));
        let idle = self
            .idle_timeout
            .map(|limit| self.last_activity.saturating_add(limit).saturating_sub(now));
        match (timeout, idle) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (left, right) => left.or(right),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iface::Priority;

    fn limits() -> ProcessLimits {
        ProcessLimits {
            timeout: None,
            idle_timeout: None,
            kill_grace: Duration::from_secs(5),
            max_cpu: None,
            max_memory: None,
            priority: Priority::Normal,
        }
    }

    fn secs(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    fn usage(cpu_millis: u64, rss: u64) -> ResourceUsage {
        ResourceUsage {
            cpu_time: Duration::from_millis(cpu_millis),
            max_rss: rss,
        }
    }

    #[test]
    fn timeout_is_reached_at_the_limit() {
        let watchdog = Watchdog::new(&ProcessLimits {
            timeout: Some(secs(10)),
            ..limits()
        });
        assert_eq!(watchdog.verdict(secs(9)), None);
        assert_eq!(watchdog.verdict(secs(10)), Some(Verdict::Timeout));
        assert_eq!(watchdog.until_deadline(secs(4)), Some(secs(6)));
    }

    #[test]
    fn silence_without_processor_use_is_idle() {
        let mut watchdog = Watchdog::new(&ProcessLimits {
            idle_timeout: Some(secs(60)),
            ..limits()
        });
        watchdog.observe_usage(secs(30), usage(0, 0));
        assert_eq!(watchdog.verdict(secs(59)), None);
        assert_eq!(watchdog.verdict(secs(60)), Some(Verdict::Idle));
    }

    #[test]
    fn output_restarts_the_idle_window() {
        let mut watchdog = Watchdog::new(&ProcessLimits {
            idle_timeout: Some(secs(60)),
            ..limits()
        });
        watchdog.observe_output(secs(50), Some(secs(49)));
        assert_eq!(watchdog.verdict(secs(100)), None);
        assert_eq!(watchdog.until_deadline(secs(100)), Some(secs(10)));
        watchdog.observe_output(secs(105), Some(secs(49)));
        assert_eq!(watchdog.verdict(secs(110)), Some(Verdict::Idle));
    }

    #[test]
    fn processor_use_restarts_the_idle_window() {
        let mut watchdog = Watchdog::new(&ProcessLimits {
            idle_timeout: Some(secs(60)),
            ..limits()
        });
        watchdog.observe_usage(secs(40), usage(500, 0));
        watchdog.observe_usage(secs(80), usage(900, 0));
        assert_eq!(watchdog.verdict(secs(130)), None);
        watchdog.observe_usage(secs(130), usage(900, 0));
        assert_eq!(watchdog.verdict(secs(140)), Some(Verdict::Idle));
    }

    #[test]
    fn processor_limit_is_reached_by_measurement() {
        let mut watchdog = Watchdog::new(&ProcessLimits {
            max_cpu: Some(secs(2)),
            ..limits()
        });
        watchdog.observe_usage(secs(1), usage(1999, 0));
        assert_eq!(watchdog.verdict(secs(1)), None);
        watchdog.observe_usage(secs(2), usage(2000, 0));
        assert_eq!(watchdog.verdict(secs(2)), Some(Verdict::Cpu));
    }

    #[test]
    fn memory_limit_comes_before_other_limits() {
        let mut watchdog = Watchdog::new(&ProcessLimits {
            timeout: Some(secs(1)),
            max_cpu: Some(secs(1)),
            max_memory: Some(1024),
            ..limits()
        });
        watchdog.observe_usage(secs(5), usage(5000, 4096));
        assert_eq!(watchdog.verdict(secs(5)), Some(Verdict::Memory));
    }

    #[test]
    fn processor_time_never_decreases() {
        let mut watchdog = Watchdog::new(&ProcessLimits {
            max_cpu: Some(secs(1)),
            ..limits()
        });
        watchdog.observe_usage(secs(1), usage(1200, 0));
        watchdog.observe_usage(secs(2), usage(100, 0));
        assert_eq!(watchdog.verdict(secs(2)), Some(Verdict::Cpu));
    }

    #[test]
    fn sample_interval_follows_the_limits() {
        assert_eq!(Watchdog::new(&limits()).sample_interval(), None);
        let idle = |seconds| {
            Watchdog::new(&ProcessLimits {
                idle_timeout: Some(secs(seconds)),
                ..limits()
            })
            .sample_interval()
        };
        assert_eq!(idle(60), Some(IDLE_SAMPLE_MAX));
        assert_eq!(idle(5), Some(Duration::from_millis(500)));
        let memory = Watchdog::new(&ProcessLimits {
            max_memory: Some(1),
            idle_timeout: Some(secs(60)),
            ..limits()
        });
        assert_eq!(memory.sample_interval(), Some(LIMIT_SAMPLE_INTERVAL));
    }

    #[test]
    fn without_time_limits_there_is_no_deadline() {
        assert_eq!(Watchdog::new(&limits()).until_deadline(secs(1)), None);
    }

    #[test]
    fn the_idle_deadline_does_not_overflow_for_the_largest_limit() {
        let mut watchdog = Watchdog::new(&ProcessLimits {
            idle_timeout: Some(Duration::MAX),
            ..limits()
        });
        watchdog.observe_output(secs(3), Some(secs(2)));
        assert_eq!(watchdog.verdict(secs(10)), None);
        assert_eq!(
            watchdog.until_deadline(secs(10)),
            Some(Duration::MAX - secs(10))
        );
    }

    #[test]
    fn the_nearest_of_the_two_deadlines_is_reported() {
        let mut watchdog = Watchdog::new(&ProcessLimits {
            timeout: Some(secs(100)),
            idle_timeout: Some(secs(20)),
            ..limits()
        });
        watchdog.observe_output(secs(10), Some(secs(1)));
        assert_eq!(watchdog.until_deadline(secs(15)), Some(secs(15)));
        watchdog.observe_output(secs(90), Some(secs(2)));
        assert_eq!(watchdog.until_deadline(secs(95)), Some(secs(5)));
        assert_eq!(watchdog.until_deadline(secs(500)), Some(Duration::ZERO));
    }

    #[test]
    fn a_limit_of_zero_is_reached_at_once() {
        let timeout = Watchdog::new(&ProcessLimits {
            timeout: Some(Duration::ZERO),
            ..limits()
        });
        assert_eq!(timeout.verdict(Duration::ZERO), Some(Verdict::Timeout));
        let cpu = Watchdog::new(&ProcessLimits {
            max_cpu: Some(Duration::ZERO),
            ..limits()
        });
        assert_eq!(cpu.verdict(Duration::ZERO), Some(Verdict::Cpu));
    }

    #[test]
    fn unchanged_output_time_is_not_activity() {
        let mut watchdog = Watchdog::new(&ProcessLimits {
            idle_timeout: Some(secs(10)),
            ..limits()
        });
        watchdog.observe_output(secs(1), Some(secs(1)));
        watchdog.observe_output(secs(9), Some(secs(1)));
        assert_eq!(watchdog.verdict(secs(10)), None);
        assert_eq!(watchdog.verdict(secs(11)), Some(Verdict::Idle));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
