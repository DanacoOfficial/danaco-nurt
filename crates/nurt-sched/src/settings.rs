// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Limits and defaults of schedules (`schedule.*` configuration keys).

use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::stage::Extensions;

const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 3_600;
const SECONDS_PER_DAY: u64 = 86_400;
const MIN_INTERVAL_SECS: u64 = 30;
const MAX_INTERVAL_DAYS: u64 = 366;
const MAX_ACTIVE: u32 = 50;
const DEFAULT_TIMEOUT_HOURS: u64 = 1;
const MAX_TIMEOUT_HOURS: u64 = 24;
const MISFIRE_MINUTES: u64 = 2;
const HISTORY_KEEP: u32 = 20;
const MAX_PARALLEL: u32 = 16;
const ACTION_ACK_TIMEOUT_SECS: u64 = 30;

/// Limits and defaults of schedules.
///
/// The embedding application registers the effective values in the
/// [`Extensions`] of the execution context and of the service context as
/// `Arc<ScheduleSettings>`; without a registration the defaults apply.
///
/// ```
/// use std::time::Duration;
/// use nurt_sched::settings::ScheduleSettings;
///
/// let settings = ScheduleSettings::default();
/// assert_eq!(settings.min_interval, Duration::from_secs(30));
/// assert_eq!(settings.max_active, 50);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleSettings {
    /// Key `schedule.min_interval`: shortest interval of `every`. Default `30s`.
    pub min_interval: Duration,
    /// Key `schedule.max_interval`: longest interval of `every`. Default `366d`.
    pub max_interval: Duration,
    /// Key `schedule.max_active`: largest number of schedules that are not
    /// completed or expired, paused ones included. Default `50`.
    pub max_active: u32,
    /// Key `schedule.default_timeout`: time limit of a run when
    /// `schedule.create` gives none. Default `1h`.
    pub default_timeout: Duration,
    /// Key `schedule.max_timeout`: largest accepted time limit of a run.
    /// Default `24h`.
    pub max_timeout: Duration,
    /// Key `schedule.misfire`: default tolerance for a late start. Default `2m`.
    pub misfire: Duration,
    /// Key `schedule.history_keep`: runs kept per schedule. Default `20`.
    pub history_keep: u32,
    /// Key `schedule.max_parallel`: runs executing at the same time in the
    /// service. Default `16`.
    pub max_parallel: u32,
    /// Key `schedule.action_ack_timeout`: time a client of a remote action
    /// kind has to acknowledge a run. Default `30s`.
    pub action_ack_timeout: Duration,
}

impl Default for ScheduleSettings {
    fn default() -> Self {
        Self {
            min_interval: Duration::from_secs(MIN_INTERVAL_SECS),
            max_interval: Duration::from_secs(MAX_INTERVAL_DAYS * SECONDS_PER_DAY),
            max_active: MAX_ACTIVE,
            default_timeout: Duration::from_secs(DEFAULT_TIMEOUT_HOURS * SECONDS_PER_HOUR),
            max_timeout: Duration::from_secs(MAX_TIMEOUT_HOURS * SECONDS_PER_HOUR),
            misfire: Duration::from_secs(MISFIRE_MINUTES * SECONDS_PER_MINUTE),
            history_keep: HISTORY_KEEP,
            max_parallel: MAX_PARALLEL,
            action_ack_timeout: Duration::from_secs(ACTION_ACK_TIMEOUT_SECS),
        }
    }
}

impl ScheduleSettings {
    /// Settings registered in `extensions`, or the defaults.
    #[must_use]
    pub fn from_extensions(extensions: &Extensions) -> Self {
        extensions
            .get::<ScheduleSettings>()
            .map(|settings| Arc::unwrap_or_clone(settings))
            .unwrap_or_default()
    }
}
