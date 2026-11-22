// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Heartbeat of the space service (`service.hb`) and liveness of the service
//! as seen by callers.
//!
//! The service writes a heartbeat into the journal directory of every
//! recorded volume. A caller treats the service as alive only when one
//! heartbeat is intact (checksum), comes from the current boot, reports a
//! running service, is not older than `space.heartbeat_max_age_ms` by the
//! suspend-inclusive monotonic clock, names a live process, and the service
//! answers a ping within [`PING_TIMEOUT`]. Otherwise the journal keeps no new
//! data.
//!
//! The file is one line of JSON followed by one line with the CRC-32 of the
//! JSON line in eight lower-case hexadecimal digits. It is replaced by
//! renaming a temporary file. It is not flushed: after a crash a heartbeat is
//! stale or torn, and both are reported as a stopped service.

use std::io::{Read, Write};
use std::time::Duration;

use nurt_contracts::config::SpaceConfig;
use nurt_contracts::env::{Clock, ClockStamp};
use nurt_contracts::fs::{
    AbsPath, Fs, FsError, FsErrorKind, FsResult, FsWrite, RenameMode, VolumeId,
};
use nurt_contracts::journal::{ServiceLiveness, VolumeState};
use nurt_journal::layout::HEARTBEAT_FILE;
use serde::{Deserialize, Serialize};

use crate::reclaim::maintenance_scope;

/// Version of the heartbeat format.
const FORMAT_VERSION: u32 = 1;

/// Longest wait for the answer of the service to a ping.
pub const PING_TIMEOUT: Duration = Duration::from_millis(50);

/// Largest heartbeat file that is read; a larger file is not a heartbeat.
const MAX_HEARTBEAT_BYTES: u64 = 4096;

/// Whether the service that wrote a heartbeat was running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunState {
    /// The service runs and keeps writing heartbeats.
    Running,
    /// The service stopped; the heartbeat is its last one.
    Stopped,
}

/// Content of a heartbeat file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Heartbeat {
    /// Format version.
    pub(crate) version: u32,
    /// Process id of the service.
    pub(crate) pid: u32,
    /// Number of the heartbeat round, from 1 at service start.
    pub(crate) seq: u64,
    /// Boot identifier of the computer.
    pub(crate) boot: String,
    /// Suspend-inclusive monotonic time since boot, nanoseconds.
    pub(crate) monotonic_ns: u64,
    /// State of the service.
    pub(crate) state: RunState,
}

impl Heartbeat {
    /// Heartbeat of process `pid` for round `seq` at the instant `stamp`.
    pub(crate) fn new(pid: u32, seq: u64, stamp: &ClockStamp, state: RunState) -> Self {
        Self {
            version: FORMAT_VERSION,
            pid,
            seq,
            boot: stamp.boot.0.clone(),
            monotonic_ns: nanos(stamp.since_boot_with_suspend),
            state,
        }
    }

    /// File content: the JSON line and the checksum line.
    pub(crate) fn encode(&self) -> FsResult<Vec<u8>> {
        let json = serde_json::to_string(self)
            .map_err(|_| FsError::new(FsErrorKind::Io, "encode", None))?;
        let crc = crc32fast::hash(json.as_bytes());
        Ok(format!("{json}\n{crc:08x}\n").into_bytes())
    }

    /// Heartbeat read from file content; `None` for content that is torn,
    /// fails the checksum or has another format version.
    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        let (json, crc) = text.strip_suffix('\n')?.split_once('\n')?;
        let crc = u32::from_str_radix(crc, 16).ok()?;
        if crc32fast::hash(json.as_bytes()) != crc {
            return None;
        }
        let heartbeat: Self = serde_json::from_str(json).ok()?;
        (heartbeat.version == FORMAT_VERSION).then_some(heartbeat)
    }

    /// Whether the heartbeat proves a running service at the instant `stamp`.
    fn is_fresh(&self, stamp: &ClockStamp, max_age: Duration) -> bool {
        let now = nanos(stamp.since_boot_with_suspend);
        self.state == RunState::Running
            && self.boot == stamp.boot.0
            && self.monotonic_ns <= now
            && now - self.monotonic_ns <= nanos(max_age)
    }
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Replaces the heartbeat in the journal directory `dir`.
pub(crate) fn write(fs: &dyn FsWrite, dir: &AbsPath, heartbeat: &Heartbeat) -> FsResult<()> {
    let scope = maintenance_scope();
    let bytes = heartbeat.encode()?;
    let target = dir.join_name(HEARTBEAT_FILE)?;
    let mut handle = fs.create_temp_in(&scope, dir, HEARTBEAT_FILE)?;
    let temp = handle.path().clone();
    let written = handle.write_all(&bytes).map_err(|error| {
        let mut failure = FsError::new(FsErrorKind::Io, "write", Some(temp.as_path()));
        failure.os_code = error.raw_os_error();
        failure
    });
    drop(handle);
    let result = written.and_then(|()| fs.rename(&scope, &temp, &target, RenameMode::Replace));
    if result.is_err() {
        // The temporary file carries no state; the write error explains the
        // outcome, so a failed removal is not reported.
        let _ = fs.remove_file(&scope, &temp);
    }
    result
}

/// Heartbeat in the journal directory `dir`; `None` when it is missing,
/// unreadable or invalid.
pub(crate) fn read(fs: &dyn Fs, dir: &AbsPath) -> Option<Heartbeat> {
    let path = dir.join_name(HEARTBEAT_FILE).ok()?;
    let handle = fs.open_read(&path).ok()?;
    let mut bytes = Vec::new();
    handle
        .take(MAX_HEARTBEAT_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_HEARTBEAT_BYTES {
        return None;
    }
    Heartbeat::decode(&bytes)
}

/// Transport between callers and the service process, provided by the
/// service host.
///
/// Production implementations check processes with
/// `nurt_fs::clock::process_alive` and reach the service through its local
/// socket or named pipe.
pub trait ServiceProbe: Send + Sync {
    /// Whether a process with this id exists.
    fn process_alive(&self, pid: u32) -> bool;

    /// Sends a ping to the service and waits no longer than `timeout`; true
    /// when the service answered that it is alive.
    fn ping(&self, timeout: Duration) -> bool;

    /// Asks the service to reclaim on a volume now, because an operation left
    /// it in `state`. Delivery is best effort.
    fn wake(&self, volume: &VolumeId, state: VolumeState);
}

/// Liveness of the space service as seen by a caller.
///
/// The service is alive when a heartbeat in one of `journal_dirs` is intact,
/// comes from the current boot, reports a running service and is not older
/// than `space.heartbeat_max_age_ms`, its process exists and the service
/// answers a ping within [`PING_TIMEOUT`]. Without journal directories the
/// service counts as down.
pub fn liveness(
    fs: &dyn Fs,
    clock: &dyn Clock,
    journal_dirs: &[AbsPath],
    config: &SpaceConfig,
    probe: &dyn ServiceProbe,
) -> ServiceLiveness {
    let stamp = clock.stamp();
    let max_age = Duration::from_millis(u64::from(config.heartbeat_max_age_ms));
    let fresh = journal_dirs
        .iter()
        .filter_map(|dir| read(fs, dir))
        .find(|heartbeat| heartbeat.is_fresh(&stamp, max_age));
    match fresh {
        Some(heartbeat) if probe.process_alive(heartbeat.pid) && probe.ping(PING_TIMEOUT) => {
            ServiceLiveness::Alive
        }
        _ => ServiceLiveness::Down,
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use nurt_contracts::fs::VolumeId;
    use nurt_contracts::journal::VolumeState;

    use super::ServiceProbe;

    /// Probe with switchable answers that records wake requests.
    pub(crate) struct TestProbe {
        pub(crate) process: AtomicBool,
        pub(crate) answers: AtomicBool,
        pub(crate) wakes: Mutex<Vec<(VolumeId, VolumeState)>>,
    }

    impl TestProbe {
        /// Probe of a live, answering service.
        pub(crate) fn alive() -> Self {
            Self {
                process: AtomicBool::new(true),
                answers: AtomicBool::new(true),
                wakes: Mutex::new(Vec::new()),
            }
        }
    }

    impl ServiceProbe for TestProbe {
        fn process_alive(&self, _pid: u32) -> bool {
            self.process.load(Ordering::SeqCst)
        }

        fn ping(&self, timeout: Duration) -> bool {
            assert_eq!(timeout, super::PING_TIMEOUT);
            self.answers.load(Ordering::SeqCst)
        }

        fn wake(&self, volume: &VolumeId, state: VolumeState) {
            self.wakes.lock().unwrap().push((volume.clone(), state));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use nurt_contracts::config::SpaceConfig;
    use nurt_contracts::env::Clock;
    use nurt_contracts::fs::{AbsPath, FsErrorKind};
    use nurt_contracts::journal::ServiceLiveness;
    use nurt_fs::clock::VirtualClock;
    use nurt_fs::mem::{MemFault, MemFs, VolumeSpec};

    use super::testing::TestProbe;
    use super::{Heartbeat, RunState, liveness, read, write};
    use crate::reclaim::maintenance_scope;

    fn abs(path: &str) -> AbsPath {
        AbsPath::new(path).unwrap()
    }

    /// File system with the journal directory `/data/.nurt-journal` and a
    /// virtual clock shared with it.
    fn setup() -> (MemFs, Arc<VirtualClock>, AbsPath) {
        let clock = Arc::new(VirtualClock::default());
        let fs = MemFs::builder()
            .clock(clock.clone())
            .volume(VolumeSpec::posix(abs("/data")))
            .build()
            .unwrap();
        let dir = abs("/data/.nurt-journal");
        fs.seed_dir(&dir).unwrap();
        (fs, clock, dir)
    }

    fn beat(clock: &VirtualClock, state: RunState) -> Heartbeat {
        Heartbeat::new(4242, 1, &clock.stamp(), state)
    }

    fn check(
        fs: &MemFs,
        clock: &VirtualClock,
        dir: &AbsPath,
        probe: &TestProbe,
    ) -> ServiceLiveness {
        liveness(
            fs,
            clock,
            std::slice::from_ref(dir),
            &SpaceConfig::default(),
            probe,
        )
    }

    #[test]
    fn encodes_and_decodes_heartbeat_with_checksum() {
        let (_, clock, _) = setup();
        clock.advance(Duration::from_millis(1500));
        let heartbeat = beat(&clock, RunState::Running);
        let bytes = heartbeat.encode().unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.starts_with("{\"version\":1,\"pid\":4242,\"seq\":1,"));
        assert!(text.ends_with('\n'));
        assert_eq!(Heartbeat::decode(&bytes), Some(heartbeat.clone()));

        let mut corrupted = bytes.clone();
        corrupted[10] ^= 0x01;
        assert_eq!(Heartbeat::decode(&corrupted), None);
        assert_eq!(Heartbeat::decode(&bytes[..bytes.len() - 4]), None);
        assert_eq!(Heartbeat::decode(b"\xff\xfe"), None);
        assert_eq!(Heartbeat::decode(b"{}\nzzzzzzzz\n"), None);

        let mut other = heartbeat;
        other.version = 2;
        let json = serde_json::to_string(&other).unwrap();
        let foreign = format!("{json}\n{:08x}\n", crc32fast::hash(json.as_bytes()));
        assert_eq!(Heartbeat::decode(foreign.as_bytes()), None);
    }

    #[test]
    fn writes_heartbeat_by_replacing_the_file() {
        let (fs, clock, dir) = setup();
        write(&fs, &dir, &beat(&clock, RunState::Running)).unwrap();
        clock.advance(Duration::from_millis(500));
        let second = Heartbeat::new(4242, 2, &clock.stamp(), RunState::Running);
        write(&fs, &dir, &second).unwrap();
        assert_eq!(read(&fs, &dir), Some(second));
        assert_eq!(names(&fs, &dir), ["service.hb"]);
    }

    fn names(fs: &MemFs, dir: &AbsPath) -> Vec<String> {
        nurt_contracts::fs::Fs::read_dir(fs, dir)
            .unwrap()
            .map(|entry| entry.unwrap().name.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn removes_temporary_file_when_rename_fails() {
        let (fs, clock, dir) = setup();
        fs.inject(MemFault::persistent(
            dir.join_name("service.hb").unwrap(),
            Some("rename"),
            FsErrorKind::Io,
        ))
        .unwrap();
        let error = write(&fs, &dir, &beat(&clock, RunState::Running)).unwrap_err();
        assert_eq!(error.kind, FsErrorKind::Io);
        fs.clear_faults();
        assert!(names(&fs, &dir).is_empty());
        assert_eq!(read(&fs, &dir), None);
    }

    #[test]
    fn rejects_oversized_heartbeat_file() {
        let (fs, _, dir) = setup();
        fs.seed_file(&dir.join_name("service.hb").unwrap(), &[b'x'; 5000])
            .unwrap();
        assert_eq!(read(&fs, &dir), None);
    }

    #[test]
    fn heartbeat_within_max_age_from_live_answering_process_is_alive() {
        let (fs, clock, dir) = setup();
        let probe = TestProbe::alive();
        write(&fs, &dir, &beat(&clock, RunState::Running)).unwrap();
        clock.advance(Duration::from_millis(2000));
        assert_eq!(check(&fs, &clock, &dir, &probe), ServiceLiveness::Alive);
        clock.advance(Duration::from_millis(1));
        assert_eq!(check(&fs, &clock, &dir, &probe), ServiceLiveness::Down);
    }

    #[test]
    fn missing_process_or_unanswered_ping_is_down() {
        let (fs, clock, dir) = setup();
        let probe = TestProbe::alive();
        write(&fs, &dir, &beat(&clock, RunState::Running)).unwrap();
        probe.process.store(false, Ordering::SeqCst);
        assert_eq!(check(&fs, &clock, &dir, &probe), ServiceLiveness::Down);
        probe.process.store(true, Ordering::SeqCst);
        probe.answers.store(false, Ordering::SeqCst);
        assert_eq!(check(&fs, &clock, &dir, &probe), ServiceLiveness::Down);
    }

    #[test]
    fn heartbeat_of_another_boot_stopped_service_or_corrupt_file_is_down() {
        let (fs, clock, dir) = setup();
        let probe = TestProbe::alive();
        write(&fs, &dir, &beat(&clock, RunState::Stopped)).unwrap();
        assert_eq!(check(&fs, &clock, &dir, &probe), ServiceLiveness::Down);

        write(&fs, &dir, &beat(&clock, RunState::Running)).unwrap();
        clock.reboot(Duration::from_secs(1));
        assert_eq!(check(&fs, &clock, &dir, &probe), ServiceLiveness::Down);

        write(&fs, &dir, &beat(&clock, RunState::Running)).unwrap();
        let path = dir.join_name("service.hb").unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut nurt_contracts::fs::Fs::open_read(&fs, &path).unwrap(),
            &mut bytes,
        )
        .unwrap();
        bytes[2] ^= 0x20;
        nurt_contracts::fs::FsWrite::remove_file(&fs, &maintenance_scope(), &path).unwrap();
        fs.seed_file(&path, &bytes).unwrap();
        assert_eq!(check(&fs, &clock, &dir, &probe), ServiceLiveness::Down);
    }

    #[test]
    fn heartbeat_from_the_future_or_no_directory_is_down() {
        let (fs, clock, dir) = setup();
        let probe = TestProbe::alive();
        clock.advance(Duration::from_secs(10));
        write(&fs, &dir, &beat(&clock, RunState::Running)).unwrap();
        let mut future = read(&fs, &dir).unwrap();
        future.monotonic_ns += 1_000_000_000;
        write(&fs, &dir, &future).unwrap();
        assert_eq!(check(&fs, &clock, &dir, &probe), ServiceLiveness::Down);
        assert_eq!(
            liveness(&fs, &*clock, &[], &SpaceConfig::default(), &probe),
            ServiceLiveness::Down
        );
    }
}
