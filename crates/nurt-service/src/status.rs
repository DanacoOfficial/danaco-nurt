// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Liveness rule and `nurt service status`.
//!
//! The service counts as running when all of the following hold:
//!
//! 1. `service.hb` exists and holds a heartbeat;
//! 2. the heartbeat is at most `space.heartbeat_max_age_ms` old (2000 ms by
//!    default);
//! 3. the process `pid` of the heartbeat exists;
//! 4. `ping` on the control channel answers within 50 ms.
//!
//! The journal applies the same rule: without a running service it does not
//! keep data (`journal_inactive`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::config::{Config, ServiceMode};
use nurt_contracts::env::{Clock, WallTime};
use nurt_contracts::error::Error;
use serde::Serialize;

use crate::clock::SystemClock;
use crate::heartbeat::{self, Heartbeat, ModuleState};
use crate::install::Manifest;
use crate::ipc::client;
use crate::paths::ServicePaths;

/// Time limit of `ping` in the liveness rule.
pub const PING_TIMEOUT: Duration = Duration::from_millis(50);

/// Why the service does not count as running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InactiveReason {
    /// There is no heartbeat file or it does not hold a heartbeat.
    NoHeartbeat,
    /// The heartbeat is older than the limit.
    StaleHeartbeat,
    /// The process of the heartbeat does not exist.
    ProcessGone,
    /// The control channel does not answer `ping` in time.
    NoResponse,
}

/// Outcome of the liveness rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Liveness {
    /// The service runs.
    Active {
        /// Heartbeat read.
        heartbeat: Heartbeat,
    },
    /// The service does not count as running.
    Inactive {
        /// Rule that failed.
        reason: InactiveReason,
        /// Heartbeat read, when there was one.
        heartbeat: Option<Heartbeat>,
    },
}

/// Current wall time of the system clock.
#[must_use]
pub fn wall_now() -> WallTime {
    SystemClock::new().now()
}

/// Applies the liveness rule to a heartbeat without contacting the service.
#[must_use]
pub fn heartbeat_liveness(
    heartbeat: Option<Heartbeat>,
    now: WallTime,
    max_age: Duration,
    pid_alive: impl Fn(u32) -> bool,
) -> Liveness {
    let Some(heartbeat) = heartbeat else {
        return Liveness::Inactive {
            reason: InactiveReason::NoHeartbeat,
            heartbeat: None,
        };
    };
    let reason = match heartbeat.age(now) {
        None => Some(InactiveReason::NoHeartbeat),
        Some(age) if age > max_age => Some(InactiveReason::StaleHeartbeat),
        Some(_) if !pid_alive(heartbeat.pid) => Some(InactiveReason::ProcessGone),
        Some(_) => None,
    };
    match reason {
        Some(reason) => Liveness::Inactive {
            reason,
            heartbeat: Some(heartbeat),
        },
        None => Liveness::Active { heartbeat },
    }
}

/// Applies the full liveness rule, including `ping`.
#[must_use]
pub fn liveness(
    paths: &ServicePaths,
    now: WallTime,
    max_age: Duration,
    ping_timeout: Duration,
) -> Liveness {
    let heartbeat = heartbeat::read(&paths.heartbeat_file()).ok().flatten();
    match heartbeat_liveness(heartbeat, now, max_age, crate::process::pid_alive) {
        Liveness::Active { heartbeat } => match client::ping(paths, ping_timeout) {
            Ok((pid, _)) if pid == heartbeat.pid => Liveness::Active { heartbeat },
            _ => Liveness::Inactive {
                reason: InactiveReason::NoResponse,
                heartbeat: Some(heartbeat),
            },
        },
        inactive => inactive,
    }
}

/// Parameters of [`status`].
#[derive(Clone)]
pub struct StatusRequest {
    /// Paths of the service files.
    pub paths: ServicePaths,
    /// Clock used to compute the age of the heartbeat.
    pub clock: Arc<dyn Clock>,
    /// Largest heartbeat age of a running service.
    pub max_age: Duration,
    /// Time limit of `ping`.
    pub ping_timeout: Duration,
}

impl StatusRequest {
    /// Request with the limits of the configuration and the system clock.
    #[must_use]
    pub fn new(paths: ServicePaths, config: &Config) -> Self {
        Self {
            paths,
            clock: Arc::new(SystemClock::new()),
            max_age: Duration::from_millis(u64::from(config.space.heartbeat_max_age_ms)),
            ping_timeout: PING_TIMEOUT,
        }
    }
}

impl std::fmt::Debug for StatusRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusRequest")
            .field("paths", &self.paths)
            .field("max_age", &self.max_age)
            .field("ping_timeout", &self.ping_timeout)
            .finish_non_exhaustive()
    }
}

/// Result of `nurt service status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServiceStatus {
    /// An installation manifest exists.
    pub installed: bool,
    /// Installation mode from the manifest.
    pub mode: Option<ServiceMode>,
    /// The service counts as running.
    pub running: bool,
    /// Why the service does not count as running.
    pub reason: Option<InactiveReason>,
    /// Process identifier from the heartbeat.
    pub pid: Option<u32>,
    /// Start of the service, RFC 3339.
    pub started: Option<String>,
    /// Version of the running service, or of the installed one when it does
    /// not run.
    pub version: Option<String>,
    /// Lingering of the account (Linux, mode `user`) recorded at
    /// installation.
    pub linger: Option<bool>,
    /// Age of the heartbeat in milliseconds.
    pub heartbeat_age_ms: Option<u64>,
    /// State of every hosted module; empty when the service does not run.
    pub modules: BTreeMap<String, ModuleState>,
}

/// Assembles the state of the service from the installation manifest, the
/// heartbeat and a `ping`.
///
/// # Errors
///
/// `E_IO` when the installation manifest exists but cannot be read.
pub fn status(request: &StatusRequest) -> Result<ServiceStatus, Error> {
    let manifest = Manifest::read(&request.paths.install_manifest())?;
    let now = request.clock.now();
    let liveness = liveness(&request.paths, now, request.max_age, request.ping_timeout);
    let (running, reason, heartbeat) = match liveness {
        Liveness::Active { heartbeat } => (true, None, Some(heartbeat)),
        Liveness::Inactive { reason, heartbeat } => (false, Some(reason), heartbeat),
    };
    let heartbeat_age_ms = heartbeat
        .as_ref()
        .and_then(|heartbeat| heartbeat.age(now))
        .map(|age| u64::try_from(age.as_millis()).unwrap_or(u64::MAX));
    let live = heartbeat.filter(|_| running);
    Ok(ServiceStatus {
        installed: manifest.is_some(),
        mode: manifest.as_ref().map(|manifest| manifest.mode),
        running,
        reason,
        pid: live.as_ref().map(|heartbeat| heartbeat.pid),
        started: live.as_ref().map(|heartbeat| heartbeat.started.clone()),
        version: live
            .as_ref()
            .map(|heartbeat| heartbeat.version.clone())
            .or_else(|| manifest.as_ref().map(|manifest| manifest.version.clone())),
        linger: manifest.as_ref().and_then(|manifest| manifest.linger),
        heartbeat_age_ms,
        modules: live.map(|heartbeat| heartbeat.modules).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beat(at: WallTime, pid: u32) -> Heartbeat {
        Heartbeat {
            pid,
            started: heartbeat::format_instant(at),
            beat: heartbeat::format_instant(at),
            version: "0.1.0".to_owned(),
            protocol: 1,
            modules: BTreeMap::new(),
        }
    }

    #[test]
    fn heartbeat_rules_in_order() {
        let at = WallTime::from_unix_nanos(1_795_000_000_000_000_000);
        let limit = Duration::from_millis(2_000);
        let alive = |_| true;
        let inactive = |liveness: Liveness| match liveness {
            Liveness::Inactive { reason, .. } => Some(reason),
            Liveness::Active { .. } => None,
        };
        assert_eq!(
            inactive(heartbeat_liveness(None, at, limit, alive)),
            Some(InactiveReason::NoHeartbeat)
        );
        let fresh = at.saturating_add(Duration::from_millis(2_000));
        assert_eq!(
            inactive(heartbeat_liveness(Some(beat(at, 7)), fresh, limit, alive)),
            None
        );
        let stale = at.saturating_add(Duration::from_millis(2_001));
        assert_eq!(
            inactive(heartbeat_liveness(Some(beat(at, 7)), stale, limit, alive)),
            Some(InactiveReason::StaleHeartbeat)
        );
        assert_eq!(
            inactive(heartbeat_liveness(Some(beat(at, 7)), fresh, limit, |_| {
                false
            })),
            Some(InactiveReason::ProcessGone)
        );
        let mut garbled = beat(at, 7);
        garbled.beat = "later".to_owned();
        assert_eq!(
            inactive(heartbeat_liveness(Some(garbled), fresh, limit, alive)),
            Some(InactiveReason::NoHeartbeat)
        );
    }

    #[test]
    fn status_without_installation_or_service() {
        let dir = tempfile::tempdir().unwrap();
        let request = StatusRequest::new(ServicePaths::new(dir.path()), &Config::default());
        let status = status(&request).unwrap();
        assert!(!status.installed && !status.running);
        assert_eq!(status.reason, Some(InactiveReason::NoHeartbeat));
        assert!(status.modules.is_empty());
    }
}
