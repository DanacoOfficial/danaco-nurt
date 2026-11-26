// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Records and report entries of a finished run.

use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::{Batch, Exec, KilledBy, Launcher, LimitHit};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::value::{self, Instant, Size};

use crate::iface::{ProcessEnd, RunRequest, RunResult};
use crate::jobs::{end_notes, exec_record};
use crate::spawn::{Platform, record_path};

/// What the result of a run reports besides the run itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LaunchInfo {
    /// Command as given, program first.
    pub(super) command: Vec<String>,
    /// Program found in the search path.
    pub(super) program: PathBuf,
    /// Working directory.
    pub(super) cwd: PathBuf,
    /// How the program was started.
    pub(super) launcher: Launcher,
    /// Names of the variables given in `env`.
    pub(super) env_keys: Vec<String>,
    /// Path notation of the host.
    pub(super) platform: Platform,
    /// Time limit.
    pub(super) timeout: Option<Duration>,
    /// Limit of time without activity.
    pub(super) idle_timeout: Option<Duration>,
}

/// Exit code of a record: the code of the program, or `-1` when it ended by
/// a signal or was stopped.
pub(super) fn exit_code(end: &ProcessEnd) -> i64 {
    end.exit_code.map_or(-1, i64::from)
}

/// Instant of a record.
pub(super) fn instant(time: WallTime) -> Instant {
    let nanos = time
        .unix_nanos()
        .clamp(i128::from(i64::MIN), i128::from(i64::MAX));
    Instant::from_unix_nanos(i64::try_from(nanos).unwrap_or(i64::MAX))
}

/// Duration of a record, in whole milliseconds.
pub(super) fn duration(length: Duration) -> value::Duration {
    value::Duration::from_millis(i64::try_from(length.as_millis()).unwrap_or(i64::MAX))
}

/// One finished batch of a run over a record stream.
pub(super) struct BatchRun<'a> {
    /// Batch number, from 1.
    pub(super) no: u64,
    /// Arguments taken from the input records.
    pub(super) args: Vec<String>,
    /// The process of the batch.
    pub(super) info: &'a LaunchInfo,
    /// Its request.
    pub(super) request: &'a RunRequest,
    /// Its result.
    pub(super) result: &'a RunResult,
}

/// The single `exec` record of a run over a record stream.
///
/// The exit code is 0 when every batch exited with 0, otherwise the code of
/// the first failed batch in batch order; the output of the batches is
/// joined in batch order.
pub(super) fn combine(command: &[String], runs: &[BatchRun<'_>]) -> Option<Exec> {
    let first = runs.first()?;
    let mut combined = exec_record(first.request, first.result, first.info.platform);
    combined.command = command.to_vec();
    combined.output_file = None;
    combined.stdout_file = None;
    combined.stderr_file = None;
    combined.output_cursor = None;
    combined.output_sha256 = None;
    combined.job = None;
    let failed: Vec<&BatchRun<'_>> = runs
        .iter()
        .filter(|run| exit_code(&run.result.end) != 0)
        .collect();
    let deciding = failed.first().copied().unwrap_or(first);
    let end = &deciding.result.end;
    combined.exit_code = exit_code(end);
    combined.signal = end.signal;
    combined.signal_number = end.signal_number.map(i64::from);
    combined.killed_by = end.killed_by;
    combined.limit_hit = end.limit_hit;
    let started = runs
        .iter()
        .map(|run| run.result.started)
        .min()
        .unwrap_or(first.result.started);
    let finished = runs
        .iter()
        .map(|run| run.result.end.finished)
        .max()
        .unwrap_or(started);
    combined.started = instant(started);
    combined.finished = Some(instant(finished));
    combined.elapsed = duration(finished.saturating_since(started));
    combined.timed_out = runs
        .iter()
        .any(|run| run.result.end.killed_by == Some(KilledBy::Timeout));
    combined.stalled = runs
        .iter()
        .any(|run| run.result.end.killed_by == Some(KilledBy::Idle));
    combined.stdout = runs
        .iter()
        .map(|run| run.result.output.stdout.as_str())
        .collect();
    combined.stderr = runs
        .iter()
        .map(|run| run.result.output.stderr.as_str())
        .collect();
    combined.stdout_bytes = runs.iter().map(|run| run.result.output.stdout_bytes).sum();
    combined.stderr_bytes = runs.iter().map(|run| run.result.output.stderr_bytes).sum();
    combined.stdout_lines = runs.iter().map(|run| run.result.output.stdout_lines).sum();
    combined.stderr_lines = runs.iter().map(|run| run.result.output.stderr_lines).sum();
    combined.changes = runs
        .iter()
        .flat_map(|run| run.result.track.changes.iter().cloned())
        .collect();
    combined.cpu_time = Some(duration(
        runs.iter().map(|run| run.result.end.usage.cpu_time).sum(),
    ));
    combined.max_rss = Some(Size::from_bytes(
        runs.iter()
            .map(|run| run.result.end.usage.max_rss)
            .max()
            .unwrap_or(0),
    ));
    combined.preimage_entries = Some(
        runs.iter()
            .map(|run| run.result.track.preimage_entries)
            .sum(),
    );
    combined.preimage_bytes = Some(Size::from_bytes(
        runs.iter().map(|run| run.result.track.preimage_bytes).sum(),
    ));
    combined.batches = Some(
        runs.iter()
            .map(|run| Batch {
                no: run.no,
                args: run.args.clone(),
                exit_code: exit_code(&run.result.end),
                elapsed: duration(run.result.end.finished.saturating_since(run.result.started)),
            })
            .collect(),
    );
    combined.failed_batches = Some(failed.len() as u64);
    Some(combined)
}

/// Note for an exit code other than zero; `count` holds a non-negative code.
pub(super) fn exit_note(code: i64) -> Option<NotableEntry> {
    if code == 0 {
        return None;
    }
    let mut note = NotableEntry::new(Notable::ExitNonzero);
    note.count = u64::try_from(code).ok();
    note.detail = Some(if code < 0 {
        "the program ended by a signal or was stopped".to_owned()
    } else {
        format!("exit code {code}")
    });
    Some(note)
}

/// Notes of one run: the exit code, the input, the output and the change
/// detection.
pub(super) fn notes(result: &RunResult) -> Vec<NotableEntry> {
    exit_note(exit_code(&result.end))
        .into_iter()
        .chain(end_notes(result))
        .chain(result.output.notes.iter().cloned())
        .chain(result.track.notes.iter().cloned())
        .collect()
}

/// Error of a run the console ended because of a limit or cancellation; the
/// effect of the program stays and the error carries the `exec` record.
pub(super) fn termination_error(info: &LaunchInfo, end: &ProcessEnd, exec: &Exec) -> Option<Error> {
    let (code, reason, message) = match end.killed_by? {
        KilledBy::Timeout => (
            ErrorCode::Timeout,
            "timeout",
            format!(
                "the program reached its time limit{}; raise `timeout` or start it with `background: true`",
                limit_text(info.timeout)
            ),
        ),
        KilledBy::Idle => (
            ErrorCode::Timeout,
            "idle",
            format!(
                "the program wrote no output and used no processor time{}; raise `idle_timeout`",
                limit_text(info.idle_timeout)
            ),
        ),
        KilledBy::Limit => {
            let (reason, parameter) = match end.limit_hit {
                Some(LimitHit::Memory) => ("memory", "max_memory"),
                _ => ("cpu", "max_cpu"),
            };
            (
                ErrorCode::Limit,
                reason,
                format!("the program exceeded its {reason} limit; raise `{parameter}`"),
            )
        }
        KilledBy::Cancel => (
            ErrorCode::Cancelled,
            "cancel",
            "the program was stopped because the call was cancelled".to_owned(),
        ),
        _ => return None,
    };
    Some(
        Error::new(code, message)
            .with_effect_applied(true)
            .with_detail("reason", serde_json::Value::from(reason))
            .with_detail(
                "exec",
                serde_json::to_value(exec).unwrap_or(serde_json::Value::Null),
            ),
    )
}

fn limit_text(limit: Option<Duration>) -> String {
    limit.map_or_else(String::new, |limit| {
        if limit.subsec_millis() == 0 {
            format!(" of {}s", limit.as_secs())
        } else {
            format!(" of {}ms", limit.as_millis())
        }
    })
}

/// Program path of a preview, in the canonical form.
pub(super) fn preview_path(path: &Path, platform: Platform) -> value::Value {
    value::Value::Path(record_path(path, platform).into_string())
}

#[cfg(test)]
mod tests {
    use nurt_contracts::env::WallTime;

    use super::*;
    use crate::iface::ResourceUsage;

    fn ended(exit_code: Option<i32>, killed_by: Option<KilledBy>) -> ProcessEnd {
        ProcessEnd {
            exit_code,
            signal: None,
            signal_number: None,
            killed_by,
            limit_hit: None,
            finished: WallTime::from_unix_nanos(0),
            usage: ResourceUsage::default(),
            suspended_total: Duration::ZERO,
            stdin_closed_early: false,
        }
    }

    fn info(timeout: Option<Duration>, idle_timeout: Option<Duration>) -> LaunchInfo {
        LaunchInfo {
            command: vec!["tool".to_owned()],
            program: PathBuf::from("tool"),
            cwd: PathBuf::from("work"),
            launcher: Launcher::Direct,
            env_keys: Vec::new(),
            platform: Platform::Unix,
            timeout,
            idle_timeout,
        }
    }

    #[test]
    fn a_program_ended_by_a_signal_has_the_exit_code_minus_one() {
        assert_eq!(exit_code(&ended(Some(3), None)), 3);
        assert_eq!(exit_code(&ended(Some(0), None)), 0);
        assert_eq!(exit_code(&ended(None, None)), -1);
    }

    #[test]
    fn only_a_nonzero_exit_code_is_noted() {
        assert!(exit_note(0).is_none());
        let failed = exit_note(3).unwrap();
        assert_eq!(failed.code, Notable::ExitNonzero);
        assert_eq!(failed.count, Some(3));
        assert_eq!(failed.detail.as_deref(), Some("exit code 3"));
        let signalled = exit_note(-1).unwrap();
        assert_eq!(signalled.count, None);
        assert!(signalled.detail.unwrap().contains("signal"));
    }

    #[test]
    fn durations_and_instants_saturate_instead_of_overflowing() {
        assert_eq!(
            duration(Duration::from_millis(1_500)),
            value::Duration::from_millis(1_500)
        );
        assert_eq!(duration(Duration::ZERO), value::Duration::from_millis(0));
        assert_eq!(
            duration(Duration::MAX),
            value::Duration::from_millis(i64::MAX)
        );
        assert_eq!(
            instant(WallTime::from_unix_nanos(i128::MAX)).as_unix_nanos(),
            i64::MAX
        );
        assert_eq!(
            instant(WallTime::from_unix_nanos(i128::MIN)).as_unix_nanos(),
            i64::MIN
        );
        assert_eq!(
            instant(WallTime::from_unix_nanos(1_500_000_000)).as_unix_nanos(),
            1_500_000_000
        );
    }

    #[test]
    fn the_limit_text_names_whole_seconds_or_milliseconds() {
        assert_eq!(limit_text(None), "");
        assert_eq!(limit_text(Some(Duration::from_secs(2))), " of 2s");
        assert_eq!(limit_text(Some(Duration::from_millis(2_500))), " of 2500ms");
        assert_eq!(limit_text(Some(Duration::ZERO)), " of 0s");
    }

    #[test]
    fn a_run_the_console_ended_becomes_an_error_with_the_record() {
        let exec = Exec::default();
        let launch = info(Some(Duration::from_secs(2)), Some(Duration::from_secs(30)));
        let timeout =
            termination_error(&launch, &ended(None, Some(KilledBy::Timeout)), &exec).unwrap();
        assert_eq!(timeout.code(), ErrorCode::Timeout);
        assert!(timeout.data().effect_applied);
        assert!(timeout.data().message.contains("of 2s"));
        let idle = termination_error(&launch, &ended(None, Some(KilledBy::Idle)), &exec).unwrap();
        assert_eq!(idle.code(), ErrorCode::Timeout);
        assert!(idle.data().message.contains("of 30s"));
        let cancelled =
            termination_error(&launch, &ended(None, Some(KilledBy::Cancel)), &exec).unwrap();
        assert_eq!(cancelled.code(), ErrorCode::Cancelled);
    }

    #[test]
    fn a_memory_limit_names_its_parameter_and_other_limits_the_processor() {
        let launch = info(None, None);
        let mut memory = ended(None, Some(KilledBy::Limit));
        memory.limit_hit = Some(LimitHit::Memory);
        let error = termination_error(&launch, &memory, &Exec::default()).unwrap();
        assert_eq!(error.code(), ErrorCode::Limit);
        assert!(error.data().message.contains("max_memory"));
        let processor = ended(None, Some(KilledBy::Limit));
        let error = termination_error(&launch, &processor, &Exec::default()).unwrap();
        assert!(error.data().message.contains("max_cpu"));
    }

    #[test]
    fn a_program_that_exited_or_was_stopped_is_not_an_error() {
        let launch = info(None, None);
        let exec = Exec::default();
        assert!(termination_error(&launch, &ended(Some(1), None), &exec).is_none());
        assert!(termination_error(&launch, &ended(None, Some(KilledBy::Stop)), &exec).is_none());
    }

    #[test]
    fn program_paths_of_previews_use_the_canonical_form() {
        let path = preview_path(Path::new("/usr/bin/tool"), Platform::Unix);
        assert_eq!(path, value::Value::Path("/usr/bin/tool".to_owned()));
    }
}
