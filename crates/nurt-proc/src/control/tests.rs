// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of process control with the `proc_fixture` test program.
//!
//! Real-time limits are at least three times shorter than the time the
//! fixture would run without them, and every test ends within five seconds.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::record::{KilledBy, Launcher, LimitHit, Signal};
use nurt_contracts::stage::{CancelReason, CancelToken, OutputStream};

use super::supervisor::Supervised;
use super::system;
use crate::iface::{
    OutputSink, Priority, ProcessEnd, ProcessLimits, RunningProcess, SpawnRequest, StdinSource,
};

/// Bound on any single wait in these tests.
const TEST_BOUND: Duration = Duration::from_secs(5);

mod tree;
#[cfg(unix)]
mod targets;

fn fixture() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let target = exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(PathBuf::from)
        .unwrap_or_default();
    target
        .join("examples")
        .join(format!("proc_fixture{}", std::env::consts::EXE_SUFFIX))
}

fn limits() -> ProcessLimits {
    ProcessLimits {
        timeout: None,
        idle_timeout: None,
        kill_grace: Duration::from_secs(1),
        max_cpu: None,
        max_memory: None,
        priority: Priority::Normal,
    }
}

fn request(args: &[&str], limits: ProcessLimits) -> SpawnRequest {
    SpawnRequest {
        program: fixture(),
        args: args.iter().map(Into::into).collect(),
        launcher: Launcher::Direct,
        raw_command_line: None,
        cwd: std::env::temp_dir(),
        env: std::env::vars_os().collect(),
        stdin: StdinSource::Null,
        merge_stderr: false,
        limits,
    }
}

/// Sink that keeps the output in memory.
#[derive(Default)]
struct Collect {
    inner: Mutex<Collected>,
}

#[derive(Default)]
struct Collected {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    finished: Vec<OutputStream>,
    last: Option<Duration>,
    origin: Option<Instant>,
}

impl Collect {
    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.lock().stdout).into_owned()
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.lock().stderr).into_owned()
    }

    fn pids(&self) -> Vec<u32> {
        self.stdout()
            .lines()
            .filter_map(|line| line.strip_prefix("pid ")?.parse().ok())
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Collected> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl OutputSink for Collect {
    fn write(&self, stream: OutputStream, bytes: &[u8]) -> Result<()> {
        let mut inner = self.lock();
        let origin = *inner.origin.get_or_insert_with(Instant::now);
        inner.last = Some(origin.elapsed());
        match stream {
            OutputStream::Stderr => inner.stderr.extend_from_slice(bytes),
            OutputStream::Stdout => inner.stdout.extend_from_slice(bytes),
        }
        Ok(())
    }

    fn finish(&self, stream: OutputStream) -> Result<()> {
        self.lock().finished.push(stream);
        Ok(())
    }

    fn last_activity(&self) -> Option<Duration> {
        self.lock().last
    }
}

struct Started {
    process: Supervised,
    sink: Arc<Collect>,
    cancel: CancelToken,
}

fn start(request: SpawnRequest) -> Started {
    let sink = Arc::new(Collect::default());
    let cancel = CancelToken::new();
    let process = super::start(request, sink.clone(), cancel.clone())
        .unwrap_or_else(|error| panic!("the fixture did not start: {error:?}"));
    Started {
        process,
        sink,
        cancel,
    }
}

fn finish(started: &mut Started) -> ProcessEnd {
    let pgid = started.process.pid();
    let end = started
        .process
        .wait(Some(TEST_BOUND))
        .unwrap_or_else(|error| panic!("waiting failed: {error:?}"))
        .unwrap_or_else(|| panic!("the fixture still runs after {TEST_BOUND:?}"));
    assert_group_gone(pgid);
    end
}

/// Waits until the fixture printed `marker`.
#[cfg(unix)]
fn wait_for_output(started: &Started, marker: &str) {
    let deadline = Instant::now() + TEST_BOUND;
    while !started.sink.stdout().contains(marker) {
        assert!(Instant::now() < deadline, "no {marker:?} in the output");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// No process of group `pgid` runs any longer; collected and ended
/// processes waiting for their parent do not count.
#[cfg(target_os = "linux")]
fn assert_group_gone(pgid: u32) {
    let deadline = Instant::now() + TEST_BOUND;
    loop {
        let alive: Vec<u32> = std::fs::read_dir("/proc")
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
                    .filter(|pid| group_member(*pid, pgid))
                    .collect()
            })
            .unwrap_or_default();
        if alive.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "processes {alive:?} of group {pgid} still run"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "linux")]
fn group_member(pid: u32, pgid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some((_, rest)) = stat.rsplit_once(')') else {
        return false;
    };
    let fields: Vec<&str> = rest.split_whitespace().collect();
    fields.first() != Some(&"Z") && fields.get(2) == Some(&pgid.to_string().as_str())
}

#[cfg(not(target_os = "linux"))]
fn assert_group_gone(_pgid: u32) {}

/// A process ended or was collected.
#[cfg(target_os = "linux")]
fn assert_process_gone(pid: u32) {
    let deadline = Instant::now() + TEST_BOUND;
    loop {
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                let (_, rest) = stat.rsplit_once(')')?;
                rest.split_whitespace().next().map(str::to_owned)
            });
        if state.is_none_or(|state| state == "Z") {
            return;
        }
        assert!(Instant::now() < deadline, "process {pid} still runs");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(not(target_os = "linux"))]
fn assert_process_gone(_pid: u32) {}

#[test]
fn program_exit_code_and_output_are_reported() {
    let mut started = start(request(&["print-both"], limits()));
    let end = finish(&mut started);
    assert_eq!(end.exit_code, Some(0));
    assert_eq!(end.killed_by, None);
    assert_eq!(started.sink.stdout(), "out\n");
    assert_eq!(started.sink.stderr(), "err\n");
    assert_eq!(started.sink.lock().finished.len(), 2);
    assert!(end.finished.unix_nanos() >= started.process.started().unix_nanos());
}

#[test]
fn merged_streams_arrive_as_standard_output() {
    let mut spawn = request(&["print-both"], limits());
    spawn.merge_stderr = true;
    let mut started = start(spawn);
    finish(&mut started);
    let stdout = started.sink.stdout();
    assert!(
        stdout.contains("out") && stdout.contains("err"),
        "{stdout:?}"
    );
    assert_eq!(started.sink.stderr(), "");
    assert_eq!(started.sink.lock().finished, vec![OutputStream::Stdout]);
}

#[test]
fn input_bytes_reach_the_program() {
    let mut spawn = request(&["copy-input"], limits());
    spawn.stdin = StdinSource::Bytes(b"first\nsecond\n".to_vec());
    let mut started = start(spawn);
    let end = finish(&mut started);
    assert_eq!(end.exit_code, Some(0));
    assert_eq!(started.sink.stdout(), "first\nsecond\n");
}

#[test]
fn input_file_reaches_the_program() {
    let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let path = dir.path().join("input.txt");
    std::fs::write(&path, "from a file\n").unwrap_or_else(|error| panic!("{error}"));
    let mut spawn = request(&["copy-input"], limits());
    spawn.stdin = StdinSource::File(path);
    let mut started = start(spawn);
    finish(&mut started);
    assert_eq!(started.sink.stdout(), "from a file\n");
}

#[test]
fn default_input_is_empty() {
    let mut started = start(request(&["copy-input"], limits()));
    let end = finish(&mut started);
    assert_eq!(end.exit_code, Some(0));
    assert_eq!(started.sink.stdout(), "");
}

#[test]
fn missing_program_is_a_spawn_error() {
    let mut spawn = request(&[], limits());
    spawn.program = std::env::temp_dir().join("nurt-proc-missing-program");
    let result = system().spawn(spawn, Arc::new(Collect::default()), CancelToken::new());
    assert_eq!(
        result.err().map(|error| error.code()),
        Some(ErrorCode::Spawn)
    );
}

#[test]
fn missing_working_directory_is_a_spawn_error() {
    let mut spawn = request(&["print-both"], limits());
    spawn.cwd = std::env::temp_dir().join("nurt-proc-missing-directory");
    let result = system().spawn(spawn, Arc::new(Collect::default()), CancelToken::new());
    let error = result.err();
    assert_eq!(
        error.as_ref().map(|error| error.code()),
        Some(ErrorCode::Spawn)
    );
    assert!(
        error.is_some_and(|error| error.data().message.contains("working directory")),
        "the message names the working directory"
    );
}

#[test]
fn wait_with_zero_time_returns_while_the_program_runs() {
    let mut started = start(request(&["silent-sleep", "3000"], limits()));
    let polled = started.process.wait(Some(Duration::ZERO));
    assert!(matches!(polled, Ok(None)));
    let end = started
        .process
        .stop(Signal::Kill, Duration::ZERO)
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(end.killed_by, Some(KilledBy::Stop));
    assert_eq!(end.signal, Some(Signal::Kill));
    assert_group_gone(started.process.pid());
}

#[test]
fn timeout_ends_the_whole_tree() {
    let mut started = start(request(
        &["spawn-tree", "2", "15000"],
        ProcessLimits {
            timeout: Some(Duration::from_millis(500)),
            ..limits()
        },
    ));
    let begin = Instant::now();
    let end = finish(&mut started);
    assert_eq!(end.killed_by, Some(KilledBy::Timeout));
    assert_eq!(end.signal, Some(Signal::Terminate));
    assert!(begin.elapsed() < TEST_BOUND);
    let pids = started.sink.pids();
    assert_eq!(pids.len(), 3, "the tree has three levels: {pids:?}");
    for pid in pids {
        assert_process_gone(pid);
    }
}

#[cfg(unix)]
#[test]
fn ignored_termination_is_followed_by_kill_after_the_grace_time() {
    let mut started = start(request(
        &["ignore-term", "15000"],
        ProcessLimits {
            timeout: Some(Duration::from_millis(300)),
            kill_grace: Duration::from_millis(700),
            ..limits()
        },
    ));
    wait_for_output(&started, "ready");
    let begin = Instant::now();
    let end = finish(&mut started);
    assert_eq!(end.killed_by, Some(KilledBy::Timeout));
    assert_eq!(end.signal, Some(Signal::Kill));
    assert!(begin.elapsed() >= Duration::from_millis(600));
}

#[test]
fn silent_program_without_processor_use_is_stalled() {
    let mut started = start(request(
        &["silent-sleep", "15000"],
        ProcessLimits {
            idle_timeout: Some(Duration::from_millis(500)),
            ..limits()
        },
    ));
    let end = finish(&mut started);
    assert_eq!(end.killed_by, Some(KilledBy::Idle));
    assert_eq!(end.limit_hit, None);
}

#[test]
fn busy_program_without_output_is_not_stalled() {
    let mut started = start(request(
        &["burn-cpu", "1500"],
        ProcessLimits {
            idle_timeout: Some(Duration::from_millis(500)),
            ..limits()
        },
    ));
    let end = finish(&mut started);
    assert_eq!(end.killed_by, None);
    assert_eq!(end.exit_code, Some(0));
    assert!(end.usage.cpu_time >= Duration::from_millis(500));
}

#[test]
fn regular_output_keeps_the_program_active() {
    let mut started = start(request(
        &["print-every", "50", "30"],
        ProcessLimits {
            idle_timeout: Some(Duration::from_millis(500)),
            ..limits()
        },
    ));
    let end = finish(&mut started);
    assert_eq!(end.killed_by, None);
    assert_eq!(end.exit_code, Some(0));
}

#[test]
fn processor_limit_ends_the_tree() {
    let mut started = start(request(
        &["burn-cpu", "15000"],
        ProcessLimits {
            max_cpu: Some(Duration::from_millis(300)),
            ..limits()
        },
    ));
    let end = finish(&mut started);
    assert_eq!(end.killed_by, Some(KilledBy::Limit));
    assert_eq!(end.limit_hit, Some(LimitHit::Cpu));
    assert!(end.usage.cpu_time >= Duration::from_millis(300));
}

#[test]
fn memory_limit_ends_the_tree() {
    let mut started = start(request(
        &["alloc", "192", "15000"],
        ProcessLimits {
            max_memory: Some(64 * 1024 * 1024),
            ..limits()
        },
    ));
    let end = finish(&mut started);
    assert_eq!(end.killed_by, Some(KilledBy::Limit));
    assert_eq!(end.limit_hit, Some(LimitHit::Memory));
}

#[test]
fn memory_below_the_limit_is_measured() {
    let mut started = start(request(
        &["alloc", "32", "300"],
        ProcessLimits {
            max_memory: Some(512 * 1024 * 1024),
            ..limits()
        },
    ));
    let end = finish(&mut started);
    assert_eq!(end.exit_code, Some(0));
    assert_eq!(end.limit_hit, None);
    assert!(end.usage.max_rss >= 32 * 1024 * 1024, "{:?}", end.usage);
}

#[cfg(unix)]
#[test]
fn signal_names_follow_the_ending_signal() {
    for (number, signal) in [
        (libc::SIGHUP, Signal::Hangup),
        (libc::SIGTERM, Signal::Terminate),
        (libc::SIGUSR1, Signal::Other),
    ] {
        let mut started = start(request(&["raise", &number.to_string()], limits()));
        let end = finish(&mut started);
        assert_eq!(end.signal, Some(signal));
        assert_eq!(end.signal_number, Some(number));
        assert_eq!(end.exit_code, None);
        assert_eq!(end.killed_by, None);
    }
}

#[cfg(unix)]
#[test]
fn signal_reaches_the_whole_tree() {
    let mut started = start(request(&["spawn-tree", "1", "15000"], limits()));
    let deadline = Instant::now() + TEST_BOUND;
    while started.sink.pids().len() < 2 {
        assert!(Instant::now() < deadline, "the tree did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    started
        .process
        .signal(Signal::Interrupt)
        .unwrap_or_else(|error| panic!("{error:?}"));
    let end = finish(&mut started);
    assert_eq!(end.signal, Some(Signal::Interrupt));
    for pid in started.sink.pids() {
        assert_process_gone(pid);
    }
}

#[test]
fn suspended_tree_uses_no_processor_time() {
    let mut started = start(request(&["burn-cpu", "15000"], limits()));
    std::thread::sleep(Duration::from_millis(200));
    started
        .process
        .suspend()
        .unwrap_or_else(|error| panic!("{error:?}"));
    std::thread::sleep(Duration::from_millis(100));
    let before = cpu(&started);
    std::thread::sleep(Duration::from_millis(500));
    let after = cpu(&started);
    assert!(
        after.saturating_sub(before) < Duration::from_millis(50),
        "{before:?} -> {after:?}"
    );
    started
        .process
        .resume()
        .unwrap_or_else(|error| panic!("{error:?}"));
    std::thread::sleep(Duration::from_millis(500));
    assert!(cpu(&started) > after, "the tree continues after resume");
    let end = started
        .process
        .stop(Signal::Terminate, Duration::from_secs(1))
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert!(end.suspended_total >= Duration::from_millis(600));
    assert_group_gone(started.process.pid());
}

fn cpu(started: &Started) -> Duration {
    started
        .process
        .usage()
        .unwrap_or_else(|error| panic!("{error:?}"))
        .cpu_time
}

#[test]
fn suspension_does_not_count_towards_the_timeout() {
    let mut started = start(request(
        &["silent-sleep", "15000"],
        ProcessLimits {
            timeout: Some(Duration::from_millis(600)),
            ..limits()
        },
    ));
    started
        .process
        .suspend()
        .unwrap_or_else(|error| panic!("{error:?}"));
    let still = started.process.wait(Some(Duration::from_millis(1000)));
    assert!(matches!(still, Ok(None)), "the timeout ran while suspended");
    started
        .process
        .resume()
        .unwrap_or_else(|error| panic!("{error:?}"));
    let end = finish(&mut started);
    assert_eq!(end.killed_by, Some(KilledBy::Timeout));
    assert!(end.suspended_total >= Duration::from_millis(1000));
}

#[test]
fn suspend_and_resume_follow_the_state() {
    let mut started = start(request(&["silent-sleep", "3000"], limits()));
    let code = |result: Result<()>| result.err().map(|error| error.code());
    assert_eq!(code(started.process.resume()), Some(ErrorCode::JobState));
    assert_eq!(code(started.process.suspend()), None);
    assert_eq!(code(started.process.suspend()), Some(ErrorCode::JobState));
    let end = started
        .process
        .stop(Signal::Terminate, Duration::from_secs(1))
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(end.killed_by, Some(KilledBy::Stop));
    assert_eq!(code(started.process.suspend()), Some(ErrorCode::JobState));
    assert_eq!(
        code(started.process.signal(Signal::Kill)),
        Some(ErrorCode::JobState)
    );
}

#[test]
fn cancellation_ends_the_tree() {
    let mut started = start(request(&["spawn-tree", "1", "15000"], limits()));
    let cancel = started.cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        cancel.cancel(CancelReason::Requested);
    });
    let end = finish(&mut started);
    assert_eq!(end.killed_by, Some(KilledBy::Cancel));
    for pid in started.sink.pids() {
        assert_process_gone(pid);
    }
}

#[test]
fn background_process_holding_the_output_belongs_to_the_run() {
    let mut started = start(request(&["detach", "700", "keep"], limits()));
    let begin = Instant::now();
    let end = finish(&mut started);
    assert_eq!(end.exit_code, Some(0));
    assert!(
        begin.elapsed() >= Duration::from_millis(500),
        "the run ended before the output"
    );
}

#[test]
fn background_process_is_ended_with_the_run() {
    let mut started = start(request(&["detach", "15000", "close"], limits()));
    let begin = Instant::now();
    let end = finish(&mut started);
    assert_eq!(end.exit_code, Some(0));
    assert!(begin.elapsed() < Duration::from_secs(3));
    let pids = started.sink.pids();
    assert_eq!(pids.len(), 1);
    for pid in pids {
        assert_process_gone(pid);
    }
}

#[test]
fn timeout_applies_while_a_background_process_holds_the_output() {
    let mut started = start(request(
        &["detach", "15000", "keep"],
        ProcessLimits {
            timeout: Some(Duration::from_millis(500)),
            ..limits()
        },
    ));
    let end = finish(&mut started);
    assert_eq!(end.exit_code, Some(0));
    assert_eq!(end.killed_by, Some(KilledBy::Timeout));
}

#[test]
fn dropped_process_is_killed() {
    let started = start(request(&["spawn-tree", "1", "15000"], limits()));
    let deadline = Instant::now() + TEST_BOUND;
    while started.sink.pids().len() < 2 {
        assert!(Instant::now() < deadline, "the tree did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    let pgid = started.process.pid();
    let pids = started.sink.pids();
    drop(started);
    assert_group_gone(pgid);
    for pid in pids {
        assert_process_gone(pid);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn low_priority_sets_the_niceness() {
    let mut spawn = request(&["silent-sleep", "3000"], limits());
    spawn.limits.priority = Priority::Low;
    let mut started = start(spawn);
    let pid = started.process.pid();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let niceness = stat
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().nth(16).map(str::to_owned));
    assert_eq!(niceness.as_deref(), Some("10"));
    started
        .process
        .stop(Signal::Kill, Duration::ZERO)
        .unwrap_or_else(|error| panic!("{error:?}"));
}

#[cfg(unix)]
#[test]
fn program_runs_in_a_new_session_without_a_terminal() {
    let mut started = start(request(&["silent-sleep", "3000"], limits()));
    let pid = i32::try_from(started.process.pid()).unwrap_or_default();
    let session = nix::unistd::getsid(Some(nix::unistd::Pid::from_raw(pid)))
        .map(nix::unistd::Pid::as_raw)
        .unwrap_or_default();
    let group = nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(pid)))
        .map(nix::unistd::Pid::as_raw)
        .unwrap_or_default();
    assert_eq!(session, pid);
    assert_eq!(group, pid);
    started
        .process
        .stop(Signal::Kill, Duration::ZERO)
        .unwrap_or_else(|error| panic!("{error:?}"));
}

#[cfg(windows)]
#[test]
fn windows_quit_is_unsupported() {
    let mut started = start(request(&["silent-sleep", "3000"], limits()));
    let result = started.process.signal(Signal::Quit);
    assert_eq!(
        result.err().map(|error| error.code()),
        Some(ErrorCode::Unsupported)
    );
    started
        .process
        .stop(Signal::Kill, Duration::ZERO)
        .unwrap_or_else(|error| panic!("{error:?}"));
}

#[cfg(windows)]
#[test]
fn windows_hangup_ends_the_tree_like_terminate() {
    let mut started = start(request(&["silent-sleep", "15000"], limits()));
    let end = started
        .process
        .stop(Signal::Hangup, Duration::from_secs(1))
        .unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(end.killed_by, Some(KilledBy::Stop));
    assert_group_gone(started.process.pid());
}

#[cfg(windows)]
#[test]
fn windows_job_memory_limit_is_reported() {
    let mut started = start(request(
        &["alloc", "192", "15000"],
        ProcessLimits {
            max_memory: Some(64 * 1024 * 1024),
            ..limits()
        },
    ));
    let end = finish(&mut started);
    assert_eq!(end.limit_hit, Some(LimitHit::Memory));
}
