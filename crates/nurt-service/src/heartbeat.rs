// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::heartbeat
// Responsibility:   Heartbeat file of the running service.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Heartbeat file of the running service.
//!
//! The service replaces `service.hb` every `space.heartbeat_ms` (never less
//! often than once per second). Readers decide whether the service is alive
//! from the age of `beat` and the existence of `pid`; together with a `ping`
//! on the control channel this is the liveness rule of the journal.
//!
//! The file is one JSON object:
//!
//! ```json
//! {"pid":4242,"started":"2026-11-22T08:00:00Z","beat":"2026-11-22T08:15:30.5Z",
//!  "version":"0.1.0","protocol":1,"modules":{"schedule":"running","space":"running"}}
//! ```
//!
//! `started` and `beat` are RFC 3339 instants in UTC with millisecond
//! precision. `modules` maps every hosted module to its [`ModuleState`].

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::env::{Clock, WallTime};
use nurt_contracts::stage::CancelToken;
use serde::{Deserialize, Serialize};

const NANOS_PER_MILLI: i128 = 1_000_000;
const MIN_INTERVAL: Duration = Duration::from_millis(50);
const MAX_INTERVAL: Duration = Duration::from_secs(1);
const TEMP_SUFFIX: &str = "tmp";

/// State of a hosted module as published in the heartbeat and in `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ModuleState {
    /// The module thread is starting.
    Starting,
    /// The module works.
    Running,
    /// The module works with reduced function.
    Degraded,
    /// The module failed and waits for its next restart.
    Restarting,
    /// The module failed too often; it stays stopped until `reload`.
    Failed,
    /// The module stopped with the service.
    Stopped,
    /// A state written by a newer version of the service.
    #[serde(other)]
    Unknown,
}

impl ModuleState {
    /// Name used in the heartbeat and in `status`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Degraded => "degraded",
            Self::Restarting => "restarting",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
            Self::Unknown => "unknown",
        }
    }
}

/// Content of the heartbeat file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    /// Process identifier of the service.
    pub pid: u32,
    /// Start of the service, RFC 3339 in UTC.
    pub started: String,
    /// Time of this heartbeat, RFC 3339 in UTC.
    pub beat: String,
    /// Version of Danaco Nurt.
    pub version: String,
    /// Version of the control protocol.
    pub protocol: u32,
    /// State of every hosted module, by module name.
    pub modules: BTreeMap<String, ModuleState>,
}

impl Heartbeat {
    /// Time of the heartbeat; `None` when `beat` is not an RFC 3339 instant.
    #[must_use]
    pub fn beat_time(&self) -> Option<WallTime> {
        parse_instant(&self.beat)
    }

    /// Age of the heartbeat at `now`; zero when the beat lies in the future.
    #[must_use]
    pub fn age(&self, now: WallTime) -> Option<Duration> {
        self.beat_time().map(|beat| now.saturating_since(beat))
    }
}

/// Formats an instant as RFC 3339 in UTC with millisecond precision.
///
/// ```
/// use nurt_contracts::env::WallTime;
/// use nurt_service::heartbeat::{format_instant, parse_instant};
///
/// let instant = WallTime::from_unix_nanos(1_795_000_000_500_000_000);
/// let text = format_instant(instant);
/// assert_eq!(text, "2026-11-18T11:06:40.5Z");
/// assert_eq!(parse_instant(&text), Some(instant));
/// ```
#[must_use]
pub fn format_instant(instant: WallTime) -> String {
    let millis = i64::try_from(instant.unix_nanos().div_euclid(NANOS_PER_MILLI)).unwrap_or(0);
    jiff::Timestamp::from_millisecond(millis)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .to_string()
}

/// Parses an RFC 3339 instant; `None` when the text is not one.
#[must_use]
pub fn parse_instant(text: &str) -> Option<WallTime> {
    text.parse::<jiff::Timestamp>()
        .ok()
        .map(|timestamp| WallTime::from_unix_nanos(timestamp.as_nanosecond()))
}

/// Reads the heartbeat file.
///
/// Returns `Ok(None)` when the file does not exist or does not hold a
/// heartbeat; a reader treats both as "no service".
///
/// # Errors
///
/// Fails on other input and output errors.
pub fn read(path: &Path) -> io::Result<Option<Heartbeat>> {
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Heartbeat period for the configured `space.heartbeat_ms`, kept between
/// 50 ms and one second.
#[must_use]
pub fn interval(heartbeat_ms: u32) -> Duration {
    Duration::from_millis(u64::from(heartbeat_ms)).clamp(MIN_INTERVAL, MAX_INTERVAL)
}

/// Writer of the heartbeat file of one service process.
#[derive(Debug, Clone)]
pub(crate) struct HeartbeatWriter {
    path: PathBuf,
    temp: PathBuf,
    pid: u32,
    started: String,
    version: String,
    protocol: u32,
}

impl HeartbeatWriter {
    pub(crate) fn new(path: PathBuf, pid: u32, started: WallTime, protocol: u32) -> Self {
        let mut temp = path.clone().into_os_string();
        temp.push(format!(".{pid}.{TEMP_SUFFIX}"));
        Self {
            path,
            temp: PathBuf::from(temp),
            pid,
            started: format_instant(started),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol,
        }
    }

    /// Writes one heartbeat: the content goes to a temporary file in the
    /// same directory, which then replaces the heartbeat file, so a reader
    /// sees either the previous or the new content and never a part.
    pub(crate) fn write(
        &self,
        now: WallTime,
        modules: BTreeMap<String, ModuleState>,
    ) -> io::Result<()> {
        let heartbeat = Heartbeat {
            pid: self.pid,
            started: self.started.clone(),
            beat: format_instant(now),
            version: self.version.clone(),
            protocol: self.protocol,
            modules,
        };
        let bytes = serde_json::to_vec(&heartbeat).map_err(io::Error::other)?;
        let mut file = fs::File::create(&self.temp)?;
        file.write_all(&bytes)?;
        drop(file);
        fs::rename(&self.temp, &self.path)
    }

    /// Writes heartbeats every `interval` until `stop` is cancelled, then
    /// removes the heartbeat file.
    ///
    /// A failed write is retried at the next period; the readers see the
    /// heartbeat age grow meanwhile.
    pub(crate) fn run<F>(
        &self,
        clock: &dyn Clock,
        interval: Duration,
        stop: &CancelToken,
        modules: F,
    ) where
        F: Fn() -> BTreeMap<String, ModuleState>,
    {
        loop {
            let _retried_next_period = self.write(clock.now(), modules());
            if clock.sleep(interval, stop).is_err() {
                break;
            }
        }
        self.remove();
    }

    pub(crate) fn remove(&self) {
        let _absent_is_fine = fs::remove_file(&self.path);
        let _absent_is_fine = fs::remove_file(&self.temp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modules() -> BTreeMap<String, ModuleState> {
        BTreeMap::from([
            ("schedule".to_owned(), ModuleState::Running),
            ("space".to_owned(), ModuleState::Failed),
        ])
    }

    #[test]
    fn interval_is_clamped_to_one_second() {
        assert_eq!(interval(500), Duration::from_millis(500));
        assert_eq!(interval(5_000), Duration::from_secs(1));
        assert_eq!(interval(0), Duration::from_millis(50));
    }

    #[test]
    fn written_heartbeat_reads_back_with_age() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("service.hb");
        let started = WallTime::from_unix_nanos(1_795_000_000_000_000_000);
        let writer = HeartbeatWriter::new(path.clone(), 77, started, 1);
        let now = started.saturating_add(Duration::from_millis(1_500));
        writer.write(now, modules()).unwrap();
        let read_back = read(&path).unwrap().unwrap();
        assert_eq!(read_back.pid, 77);
        assert_eq!(read_back.started, "2026-11-18T11:06:40Z");
        assert_eq!(read_back.modules["space"], ModuleState::Failed);
        let later = now.saturating_add(Duration::from_millis(700));
        assert_eq!(read_back.age(later), Some(Duration::from_millis(700)));
        assert_eq!(read_back.age(started), Some(Duration::ZERO));
        writer.remove();
        assert_eq!(read(&path).unwrap(), None);
    }

    #[test]
    fn unknown_module_state_and_garbage_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("service.hb");
        fs::write(
            &path,
            r#"{"pid":1,"started":"2026-01-01T00:00:00Z","beat":"2026-01-01T00:00:01Z","version":"9.0.0","protocol":1,"modules":{"space":"hibernating"}}"#,
        )
        .unwrap();
        let heartbeat = read(&path).unwrap().unwrap();
        assert_eq!(heartbeat.modules["space"], ModuleState::Unknown);
        assert_eq!(ModuleState::Unknown.as_str(), "unknown");
        fs::write(&path, b"{\"pid\":").unwrap();
        assert_eq!(read(&path).unwrap(), None);
        assert_eq!(parse_instant("yesterday"), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
