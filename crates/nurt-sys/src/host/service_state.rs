// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! State of the background service of the instance, for `host.service`.
//!
//! The service is `running` when its heartbeat file `service.hb` in the state
//! directory is fresh, the process it names exists and the service answers
//! `ping` on its control channel within 50 ms. Otherwise it is `stopped` when
//! it is installed (a systemd unit, a launchd property list or a Windows
//! service), and `not_installed` when it is not.
//!
//! The heartbeat is a JSON object with at least `pid` (process identifier)
//! and `beat` (RFC 3339 time of the last beat). A control message is one line
//! of JSON: the request `{"v":1,"op":"ping"}` is answered by `{"v":1,"ok":true}`.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::record::ServiceAvailability;
use nurt_contracts::stage::ExecCtx;
use serde::{Deserialize, Serialize};

use super::SysRoot;

/// Version of the control protocol carried in every message.
pub(crate) const PROTOCOL_VERSION: u64 = 1;
/// Operation that tests liveness.
pub(crate) const PING: &str = "ping";
/// Heartbeat file in the state directory.
pub(crate) const HEARTBEAT_FILE: &str = "service.hb";
/// Control socket in the state directory (Linux and macOS).
#[cfg(unix)]
pub(crate) const SOCKET_FILE: &str = "service.sock";
/// Longest wait for the answer to `ping`.
pub(crate) const PING_TIMEOUT: Duration = Duration::from_millis(50);
/// Name of the systemd unit.
#[cfg(any(target_os = "linux", test))]
pub(crate) const SYSTEMD_UNIT: &str = "nurt-service.service";
/// File name of the launchd property list.
#[cfg(target_os = "macos")]
pub(crate) const LAUNCHD_PLIST: &str = "nurt.service.plist";
/// Name of the Windows service.
#[cfg(windows)]
pub(crate) const WINDOWS_SERVICE: &str = "NurtService";
/// Prefix of the named pipe of the control channel; the security identifier
/// of the account follows.
#[cfg(windows)]
pub(crate) const PIPE_PREFIX: &str = r"\\.\pipe\nurt-service-";

/// Directories of system-wide systemd units, relative to the system root.
#[cfg(any(target_os = "linux", test))]
const SYSTEM_UNIT_DIRS: [&str; 3] = [
    "/etc/systemd/system",
    "/usr/lib/systemd/system",
    "/lib/systemd/system",
];

/// Request of the control protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Request {
    /// Protocol version.
    pub(crate) v: u64,
    /// Operation.
    pub(crate) op: String,
}

/// Reply of the control protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Reply {
    /// Protocol version.
    pub(crate) v: u64,
    /// The operation succeeded.
    pub(crate) ok: bool,
}

/// Fields of the heartbeat file read by clients.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct Heartbeat {
    /// Process identifier of the service.
    pub(crate) pid: u32,
    /// Time of the last beat, RFC 3339.
    pub(crate) beat: String,
}

/// Inputs of the liveness test.
#[derive(Debug, Clone)]
pub(crate) struct Probe<'a> {
    /// Root of the system files.
    pub(crate) root: &'a SysRoot,
    /// State directory of the instance, when known.
    pub(crate) state_dir: Option<PathBuf>,
    /// Home directory of the account.
    pub(crate) home: PathBuf,
    /// Configuration directory of the account (`XDG_CONFIG_HOME`), when set.
    pub(crate) config_home: Option<PathBuf>,
    /// Current wall time.
    pub(crate) now: WallTime,
    /// Age after which a heartbeat is stale.
    pub(crate) max_age: Duration,
}

impl<'a> Probe<'a> {
    /// Inputs taken from the execution context.
    pub(crate) fn from_ctx(root: &'a SysRoot, ctx: &ExecCtx) -> Self {
        let state_dir = ctx
            .state_dir()
            .map(Path::to_path_buf)
            .or_else(|| ctx.config().state.dir.as_ref().map(PathBuf::from));
        Self {
            root,
            state_dir,
            home: ctx.host().info().home.clone(),
            config_home: ctx
                .host()
                .env_var("XDG_CONFIG_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            now: ctx.clock().now(),
            max_age: Duration::from_millis(u64::from(ctx.config().space.heartbeat_max_age_ms)),
        }
    }
}

/// State of the service.
pub(crate) fn availability(probe: &Probe<'_>) -> ServiceAvailability {
    if is_alive(probe) {
        ServiceAvailability::Running
    } else if is_installed(probe) {
        ServiceAvailability::Stopped
    } else {
        ServiceAvailability::NotInstalled
    }
}

/// Text of the `ping` request line, without the line break.
pub(crate) fn ping_request() -> String {
    let request = Request {
        v: PROTOCOL_VERSION,
        op: PING.to_owned(),
    };
    serde_json::to_string(&request).unwrap_or_default()
}

/// Returns `true` when `line` is a successful reply of this protocol version.
pub(crate) fn is_pong(line: &str) -> bool {
    serde_json::from_str::<Reply>(line.trim())
        .is_ok_and(|reply| reply.v == PROTOCOL_VERSION && reply.ok)
}

/// Process identifier of the heartbeat `text` when the beat lies no further
/// than `max_age` from `now`.
pub(crate) fn heartbeat_pid_if_fresh(text: &str, now: WallTime, max_age: Duration) -> Option<u32> {
    let heartbeat: Heartbeat = serde_json::from_str(text).ok()?;
    let beat: jiff::Timestamp = heartbeat.beat.parse().ok()?;
    let beat = WallTime::from_unix_nanos(beat.as_nanosecond());
    let age = now.saturating_since(beat).max(beat.saturating_since(now));
    (age <= max_age).then_some(heartbeat.pid)
}

fn is_alive(probe: &Probe<'_>) -> bool {
    let Some(state_dir) = probe.state_dir.as_deref() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(state_dir.join(HEARTBEAT_FILE)) else {
        return false;
    };
    heartbeat_pid_if_fresh(&text, probe.now, probe.max_age)
        .is_some_and(|pid| process_exists(probe.root, pid) && ping(state_dir))
}

/// Exchanges `ping` with the service over `stream` within [`PING_TIMEOUT`].
fn exchange<S: std::io::Read + Write>(mut stream: S) -> bool {
    let mut line = ping_request();
    line.push('\n');
    if stream.write_all(line.as_bytes()).is_err() || stream.flush().is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).is_ok() && is_pong(&reply)
}

#[cfg(unix)]
fn ping(state_dir: &Path) -> bool {
    let Ok(stream) = std::os::unix::net::UnixStream::connect(state_dir.join(SOCKET_FILE)) else {
        return false;
    };
    let limits = stream
        .set_read_timeout(Some(PING_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(PING_TIMEOUT)));
    limits.is_ok() && exchange(stream)
}

#[cfg(windows)]
fn ping(_state_dir: &Path) -> bool {
    let Some(sid) = crate::accounts::windows::current_sid() else {
        return false;
    };
    let name = format!("{PIPE_PREFIX}{sid}");
    let (sender, receiver) = std::sync::mpsc::channel();
    // The pipe has no read timeout; the exchange runs on its own thread and
    // the answer is awaited for the protocol limit only.
    std::thread::spawn(move || {
        let answered = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(name)
            .is_ok_and(exchange);
        let _ignored = sender.send(answered);
    });
    receiver.recv_timeout(PING_TIMEOUT).unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn process_exists(root: &SysRoot, pid: u32) -> bool {
    root.path(format!("/proc/{pid}")).is_dir()
}

#[cfg(target_os = "macos")]
fn process_exists(_root: &SysRoot, pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    rustix::process::Pid::from_raw(pid).is_some_and(|pid| match rustix::process::test_kill_process(
        pid,
    ) {
        Ok(()) => true,
        Err(error) => error == rustix::io::Errno::PERM,
    })
}

#[cfg(windows)]
fn process_exists(_root: &SysRoot, pid: u32) -> bool {
    super::windows::process_alive(pid)
}

#[cfg(target_os = "linux")]
fn is_installed(probe: &Probe<'_>) -> bool {
    let user_dir = probe
        .config_home
        .clone()
        .unwrap_or_else(|| probe.home.join(".config"))
        .join("systemd/user");
    user_dir.join(SYSTEMD_UNIT).is_file()
        || SYSTEM_UNIT_DIRS
            .iter()
            .any(|dir| probe.root.path(dir).join(SYSTEMD_UNIT).is_file())
}

#[cfg(target_os = "macos")]
fn is_installed(probe: &Probe<'_>) -> bool {
    probe
        .home
        .join("Library/LaunchAgents")
        .join(LAUNCHD_PLIST)
        .is_file()
        || probe
            .root
            .path("/Library/LaunchDaemons")
            .join(LAUNCHD_PLIST)
            .is_file()
}

#[cfg(windows)]
fn is_installed(_probe: &Probe<'_>) -> bool {
    super::windows::service_exists(WINDOWS_SERVICE)
}
