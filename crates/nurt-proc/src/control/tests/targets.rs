// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Signals to single processes named by identifier and start time, started
//! by the tests and signalled through the `process` functions.

use std::io::{BufRead, BufReader};
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};

use nurt_contracts::value::Instant as StartTime;

use super::super::{ProcessRef, StopEnd, resume_process, stop_process, suspend_process};
use super::*;

/// One hour in nanoseconds.
const HOUR_NANOS: i64 = 3_600_000_000_000;

/// Fixture process started by a test and ended with it.
struct Fixture {
    child: Child,
}

impl Fixture {
    fn start(args: &[&str]) -> Self {
        let child = Command::new(fixture())
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self { child }
    }

    fn pid(&self) -> u64 {
        u64::from(self.child.id())
    }

    /// The process as a `process` record names it.
    fn reference(&self) -> ProcessRef {
        let started = crate::os::start_time(self.pid()).unwrap();
        ProcessRef {
            pid: self.pid(),
            started: StartTime::from_unix_nanos(i64::try_from(started.unix_nanos()).unwrap()),
        }
    }

    /// First line the fixture writes to its standard output.
    fn line(&mut self) -> String {
        let mut line = String::new();
        let stdout = self.child.stdout.as_mut().unwrap();
        BufReader::new(stdout).read_line(&mut line).unwrap();
        line
    }

    fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TEST_BOUND;
    while !condition() {
        assert!(Instant::now() < deadline, "the condition was not reached");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// State letter of a process in `/proc/<pid>/stat`.
#[cfg(target_os = "linux")]
fn state_of(pid: u64) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(')')?;
    rest.trim_start().chars().next()
}

/// The reference of `fixture` with a start time one hour earlier.
fn forged(fixture: &Fixture) -> ProcessRef {
    let real = fixture.reference();
    ProcessRef {
        pid: real.pid,
        started: StartTime::from_unix_nanos(real.started.as_unix_nanos() - HOUR_NANOS),
    }
}

fn code<T: std::fmt::Debug>(result: Result<T>) -> ErrorCode {
    result.unwrap_err().code()
}

#[test]
fn stops_the_process_named_by_identifier_and_start_time() {
    let mut sleeper = Fixture::start(&["silent-sleep", "30000"]);
    let end = stop_process(
        sleeper.reference(),
        Signal::Terminate,
        TEST_BOUND,
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(end, StopEnd::Signalled);
    assert_eq!(sleeper.child.wait().unwrap().signal(), Some(libc::SIGTERM));
}

#[test]
fn kills_a_process_that_ignores_the_signal_after_the_grace_time() {
    let mut stubborn = Fixture::start(&["ignore-term", "30000"]);
    assert_eq!(stubborn.line().trim(), "ready");
    let end = stop_process(
        stubborn.reference(),
        Signal::Terminate,
        Duration::from_millis(200),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(end, StopEnd::Killed);
    assert_eq!(stubborn.child.wait().unwrap().signal(), Some(libc::SIGKILL));
}

#[test]
fn kill_does_not_wait_for_the_grace_time() {
    let mut sleeper = Fixture::start(&["silent-sleep", "30000"]);
    let end = stop_process(
        sleeper.reference(),
        Signal::Kill,
        Duration::from_secs(3600),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(end, StopEnd::Signalled);
    assert_eq!(sleeper.child.wait().unwrap().signal(), Some(libc::SIGKILL));
}

#[test]
fn refuses_a_process_with_another_start_time() {
    let mut sleeper = Fixture::start(&["silent-sleep", "30000"]);
    let reference = forged(&sleeper);
    let cancel = CancelToken::new();
    assert_eq!(
        code(stop_process(reference, Signal::Kill, Duration::ZERO, &cancel)),
        ErrorCode::Stale
    );
    assert_eq!(code(suspend_process(reference)), ErrorCode::Stale);
    assert_eq!(code(resume_process(reference)), ErrorCode::Stale);
    assert!(sleeper.running());
}

#[test]
fn reports_a_process_that_was_collected() {
    let mut sleeper = Fixture::start(&["silent-sleep", "30000"]);
    let reference = sleeper.reference();
    sleeper.child.kill().unwrap();
    sleeper.child.wait().unwrap();
    let cancel = CancelToken::new();
    assert_eq!(
        code(stop_process(
            reference,
            Signal::Terminate,
            Duration::ZERO,
            &cancel
        )),
        ErrorCode::NotFound
    );
    assert_eq!(code(suspend_process(reference)), ErrorCode::NotFound);
    assert_eq!(code(resume_process(reference)), ErrorCode::NotFound);
}

#[test]
fn a_process_that_only_waits_for_its_parent_has_ended() {
    let mut sleeper = Fixture::start(&["silent-sleep", "30000"]);
    let reference = sleeper.reference();
    sleeper.child.kill().unwrap();
    wait_for(|| crate::os::start_time(reference.pid).is_none());
    let cancel = CancelToken::new();
    assert_eq!(
        code(stop_process(
            reference,
            Signal::Kill,
            Duration::ZERO,
            &cancel
        )),
        ErrorCode::NotFound
    );
}

#[test]
fn an_identifier_that_names_a_group_or_nothing_is_never_signalled() {
    let cancel = CancelToken::new();
    for pid in [0, u64::from(u32::MAX), u64::MAX] {
        let reference = ProcessRef {
            pid,
            started: StartTime::UNIX_EPOCH,
        };
        assert_eq!(
            code(stop_process(
                reference,
                Signal::Kill,
                Duration::ZERO,
                &cancel
            )),
            ErrorCode::NotFound,
            "{pid}"
        );
        assert_eq!(code(suspend_process(reference)), ErrorCode::NotFound);
    }
}

#[test]
fn a_signal_without_a_name_is_unsupported_and_leaves_the_process_alone() {
    let mut sleeper = Fixture::start(&["silent-sleep", "30000"]);
    let cancel = CancelToken::new();
    let result = stop_process(sleeper.reference(), Signal::Other, TEST_BOUND, &cancel);
    assert_eq!(code(result), ErrorCode::Unsupported);
    assert!(sleeper.running());
}

#[test]
fn a_cancelled_call_signals_nothing() {
    let mut sleeper = Fixture::start(&["silent-sleep", "30000"]);
    let cancel = CancelToken::new();
    cancel.cancel(CancelReason::Requested);
    let result = stop_process(sleeper.reference(), Signal::Terminate, TEST_BOUND, &cancel);
    assert_eq!(code(result), ErrorCode::Cancelled);
    assert!(sleeper.running());
}

#[test]
fn cancellation_ends_the_wait_for_a_process_that_ignores_the_signal() {
    let mut stubborn = Fixture::start(&["ignore-term", "30000"]);
    assert_eq!(stubborn.line().trim(), "ready");
    let cancel = CancelToken::new();
    let canceller = {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            cancel.cancel(CancelReason::Requested);
        })
    };
    let result = stop_process(
        stubborn.reference(),
        Signal::Terminate,
        Duration::from_secs(3600),
        &cancel,
    );
    canceller.join().unwrap();
    assert_eq!(code(result), ErrorCode::Cancelled);
    assert!(stubborn.running());
}

#[cfg(target_os = "linux")]
#[test]
fn suspends_and_resumes_the_process() {
    let mut worker = Fixture::start(&["burn-cpu", "30000"]);
    let reference = worker.reference();
    suspend_process(reference).unwrap();
    wait_for(|| state_of(reference.pid) == Some('T'));
    resume_process(reference).unwrap();
    wait_for(|| state_of(reference.pid) != Some('T'));
    assert!(worker.running());
}
