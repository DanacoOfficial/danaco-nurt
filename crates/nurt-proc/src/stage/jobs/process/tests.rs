// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of `stop`, `suspend` and `resume` on process records, with child
//! processes of the test program.

use nurt_contracts::config::Config;
use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::record::{Effect, Process};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, EffectStage, ExecCtx, RecordStream, StageData, StageImpl, StageInput,
};
use nurt_contracts::value::{Record, RecordKind, Value};

use super::system::same_start;
use super::{Kind, ProcessControl};
use crate::spawn::testing::{NullTx, TestHost, context};

fn ctx() -> ExecCtx {
    context(
        TestHost::current(&[]),
        Config::default(),
        RequestOptions::default(),
    )
}

fn input(processes: Vec<Process>) -> StageInput<'static> {
    StageInput::records(RecordStream::from_records(
        processes.into_iter().map(Record::from).collect(),
    ))
}

fn args(kind: Kind, extra: Vec<(&str, Value)>) -> BoundArgs {
    let mut args = BoundArgs::new(kind.name());
    for (name, value) in extra {
        args.insert(name, ArgValue::Value(value), false);
    }
    args
}

/// Plans and applies the command; returns its effect records.
fn execute(kind: Kind, processes: Vec<Process>, extra: Vec<(&str, Value)>) -> Result<Vec<Effect>> {
    let stage = ProcessControl::new(kind);
    let ctx = ctx();
    let args = args(kind, extra);
    let plan = stage.plan(&args, input(processes), &ctx)?;
    let mut tx = NullTx::default();
    let outcome = stage.apply(&args, plan, &mut tx, &ctx)?;
    Ok(match outcome.output.data {
        StageData::Records(stream) => stream
            .map(|record| Effect::try_from(record.unwrap()).unwrap())
            .collect(),
        _ => Vec::new(),
    })
}

fn word(value: &str) -> Value {
    Value::Enum(value.to_owned())
}

#[test]
fn the_commands_on_processes_accept_only_process_records() {
    for kind in [Kind::Stop, Kind::Suspend, Kind::Resume] {
        let stage = ProcessControl::new(kind);
        let descriptor = stage.descriptor();
        assert_eq!(descriptor.command, kind.name());
        assert!(!descriptor.source);
        assert!(descriptor.input.accepts(RecordKind::Process));
        assert!(!descriptor.input.accepts(RecordKind::Job));
        assert!(stage.as_effect().is_some());
    }
}

#[test]
fn the_job_stages_include_the_commands_on_processes() {
    let stages = crate::stage::jobs::stages();
    for command in ["stop", "suspend", "resume"] {
        let descriptors: Vec<_> = stages
            .iter()
            .map(|stage| stage.descriptor())
            .filter(|descriptor| descriptor.command == command)
            .collect();
        assert_eq!(descriptors.len(), 2, "{command}");
        assert!(
            descriptors
                .iter()
                .any(|descriptor| descriptor.input.accepts(RecordKind::Job)),
            "{command}"
        );
        assert!(
            descriptors
                .iter()
                .any(|descriptor| descriptor.input.accepts(RecordKind::Process)),
            "{command}"
        );
    }
}

#[test]
fn start_times_within_the_tolerance_name_the_same_process() {
    assert!(same_start(Some(1_000), Some(1_000)));
    assert!(same_start(Some(1_000_000_000), Some(2_500_000_000)));
    assert!(!same_start(Some(0), Some(3_000_000_000)));
    assert!(!same_start(Some(i64::MIN), Some(i64::MAX)));
    assert!(same_start(None, Some(1)));
    assert!(same_start(Some(1), None));
}

#[test]
fn an_identifier_that_names_no_single_process_is_refused() {
    for pid in [0, u64::from(u32::MAX), u64::MAX] {
        let process = Process {
            pid,
            name: "none".to_owned(),
            ..Process::default()
        };
        let error = execute(Kind::Stop, vec![process], Vec::new()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue, "{pid}");
    }
}

#[cfg(unix)]
mod unix {
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Child, Command, Stdio};

    use nurt_contracts::value::Instant;

    use super::*;
    use crate::spawn::testing::fixture_path;
    use crate::stage::jobs::process::system::Handle;

    /// Test program that sleeps for thirty seconds unless it is stopped.
    struct Sleeper(Child);

    impl Sleeper {
        fn start() -> Self {
            let child = Command::new(fixture_path())
                .args(["exit-with", "0", "30000"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            Self(child)
        }

        fn pid(&self) -> u32 {
            self.0.id()
        }

        /// Record of the process as `processes` lists it.
        fn record(&self) -> Process {
            let handle = Handle::open(self.pid()).unwrap().unwrap();
            Process {
                pid: u64::from(self.pid()),
                name: "proc_fixture".to_owned(),
                started: Instant::from_unix_nanos(handle.started().unwrap_or(0)),
                ..Process::default()
            }
        }

        /// Waits for the state change that `flags` asks for and returns the
        /// raw wait status.
        fn wait_status(&self, flags: libc::c_int) -> libc::c_int {
            let mut status: libc::c_int = 0;
            let pid = libc::pid_t::try_from(self.pid()).unwrap();
            // SAFETY: `status` is a writable `c_int` and the identifier names
            // a child of this process.
            let collected = unsafe { libc::waitpid(pid, &raw mut status, flags) };
            assert_eq!(collected, pid);
            status
        }
    }

    impl Drop for Sleeper {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn stop_ends_a_listed_process() {
        let mut sleeper = Sleeper::start();
        let effects = execute(Kind::Stop, vec![sleeper.record()], Vec::new()).unwrap();
        assert_eq!(effects.len(), 1);
        let effect = &effects[0];
        assert_eq!(effect.kind, "stop");
        assert_eq!(effect.action.as_deref(), Some("stop"));
        assert_eq!(
            effect.target.as_deref(),
            Some(format!("process:{}", sleeper.pid()).as_str())
        );
        assert_eq!(effect.before, Some(word("running")));
        assert_eq!(effect.after, Some(word("ended")));
        assert_eq!(effect.irreversible.as_ref().map(Vec::len), Some(1));
        let status = sleeper.0.wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGTERM));
    }

    #[test]
    fn stop_with_the_kill_signal_ends_the_process_at_once() {
        let mut sleeper = Sleeper::start();
        let extra = vec![("signal", word("kill"))];
        execute(Kind::Stop, vec![sleeper.record()], extra).unwrap();
        assert_eq!(sleeper.0.wait().unwrap().signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn stop_ends_several_processes_in_one_call() {
        let mut first = Sleeper::start();
        let mut second = Sleeper::start();
        let stage = ProcessControl::new(Kind::Stop);
        let ctx = ctx();
        let args = args(Kind::Stop, Vec::new());
        let plan = stage
            .plan(&args, input(vec![first.record(), second.record()]), &ctx)
            .unwrap();
        assert!(plan.bulk);
        assert_eq!(plan.preview.len(), 2);
        assert_eq!(plan.irreversible.len(), 2);
        let mut tx = NullTx::default();
        let outcome = stage.apply(&args, plan, &mut tx, &ctx).unwrap();
        assert_eq!(outcome.irreversible.len(), 2);
        assert_eq!(tx.markers.len(), 2);
        assert_eq!(first.0.wait().unwrap().signal(), Some(libc::SIGTERM));
        assert_eq!(second.0.wait().unwrap().signal(), Some(libc::SIGTERM));
    }

    #[test]
    fn suspend_and_resume_change_the_state_of_the_process() {
        let sleeper = Sleeper::start();
        let suspended = execute(Kind::Suspend, vec![sleeper.record()], Vec::new()).unwrap();
        assert_eq!(suspended[0].action.as_deref(), Some("suspend"));
        assert_eq!(suspended[0].before, Some(word("running")));
        assert_eq!(suspended[0].after, Some(word("stopped")));
        assert!(libc::WIFSTOPPED(sleeper.wait_status(libc::WUNTRACED)));
        let resumed = execute(Kind::Resume, vec![sleeper.record()], Vec::new()).unwrap();
        assert_eq!(resumed[0].action.as_deref(), Some("resume"));
        assert_eq!(resumed[0].before, Some(word("stopped")));
        assert_eq!(resumed[0].after, Some(word("running")));
        assert!(libc::WIFCONTINUED(sleeper.wait_status(libc::WCONTINUED)));
    }

    #[test]
    fn stop_ends_a_suspended_process_without_waiting_for_the_grace_period() {
        let mut sleeper = Sleeper::start();
        execute(Kind::Suspend, vec![sleeper.record()], Vec::new()).unwrap();
        assert!(libc::WIFSTOPPED(sleeper.wait_status(libc::WUNTRACED)));
        let effects = execute(Kind::Stop, vec![sleeper.record()], Vec::new()).unwrap();
        assert_eq!(effects[0].before, Some(word("stopped")));
        assert_eq!(sleeper.0.wait().unwrap().signal(), Some(libc::SIGTERM));
    }

    #[test]
    fn an_ended_process_is_reported_and_nothing_is_signalled() {
        let mut sleeper = Sleeper::start();
        let record = sleeper.record();
        sleeper.0.kill().unwrap();
        sleeper.0.wait().unwrap();
        let error = execute(Kind::Stop, vec![record.clone()], Vec::new()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        let error = execute(Kind::Suspend, vec![record], Vec::new()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn a_process_with_another_start_time_is_not_signalled() {
        const HOUR_NANOS: i64 = 3_600_000_000_000;
        let mut sleeper = Sleeper::start();
        let mut record = sleeper.record();
        record.started = Instant::from_unix_nanos(record.started.as_unix_nanos() - HOUR_NANOS);
        for kind in [Kind::Stop, Kind::Suspend, Kind::Resume] {
            let error = execute(kind, vec![record.clone()], Vec::new()).unwrap_err();
            assert_eq!(error.code(), ErrorCode::NotFound, "{}", kind.name());
            assert_eq!(
                error.data().problem.as_deref(),
                Some("identifier_reused"),
                "{}",
                kind.name()
            );
        }
        assert!(sleeper.0.try_wait().unwrap().is_none());
    }

    #[test]
    fn a_signal_the_system_cannot_send_is_refused() {
        let sleeper = Sleeper::start();
        let extra = vec![("signal", word("quit"))];
        let error = execute(Kind::Stop, vec![sleeper.record()], extra).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Unsupported);
    }
}

#[cfg(windows)]
mod windows {
    use std::process::{Command, Stdio};
    use std::thread::sleep;
    use std::time::{Duration, Instant as Clock};

    use nurt_contracts::value::Instant;

    use super::*;
    use crate::spawn::testing::fixture_path;
    use crate::stage::jobs::process::system::Handle;

    #[test]
    fn suspend_holds_the_process_and_resume_lets_it_end() {
        let mut child = Command::new(fixture_path())
            .args(["exit-with", "0", "1500"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let handle = Handle::open(pid).unwrap().unwrap();
        let record = Process {
            pid: u64::from(pid),
            name: "proc_fixture".to_owned(),
            started: Instant::from_unix_nanos(handle.started().unwrap_or(0)),
            ..Process::default()
        };
        let suspended = execute(Kind::Suspend, vec![record.clone()], Vec::new()).unwrap();
        assert_eq!(suspended[0].action.as_deref(), Some("suspend"));
        assert_eq!(suspended[0].after, Some(word("stopped")));
        // The program ends by itself after 1.5 seconds unless it is held.
        sleep(Duration::from_secs(3));
        assert!(
            child.try_wait().unwrap().is_none(),
            "the suspended process ended"
        );
        let resumed = execute(Kind::Resume, vec![record], Vec::new()).unwrap();
        assert_eq!(resumed[0].action.as_deref(), Some("resume"));
        assert_eq!(resumed[0].after, Some(word("running")));
        let deadline = Clock::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            assert!(Clock::now() < deadline, "the resumed process did not end");
            sleep(Duration::from_millis(50));
        }
    }
}
