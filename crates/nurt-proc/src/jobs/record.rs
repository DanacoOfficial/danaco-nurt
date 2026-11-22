// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Records of jobs and finished runs.

use std::path::Path;
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::record::{Exec, Job, JobState, KilledBy};
use nurt_contracts::value::{self, Size};

use super::state::final_state;
use super::table::{instant, wall_time};
use crate::api::ToolResult;
use crate::iface::{RunRequest, RunResult};

/// Path as a record value.
pub(crate) fn path_value(path: &Path) -> value::Path {
    value::Path::from(path.to_string_lossy().into_owned())
}

/// Duration as a record value, saturating at the largest value.
pub(crate) fn duration_value(duration: Duration) -> value::Duration {
    value::Duration::from_std(duration).unwrap_or(value::Duration::from_millis(i64::MAX))
}

/// Record of a job whose program has started.
pub(crate) fn started_job(id: &str, request: &RunRequest, pid: u32, started: WallTime) -> Job {
    Job {
        id: id.to_owned(),
        command: request.command.clone(),
        cwd: path_value(&request.spawn.cwd),
        pid: u64::from(pid),
        state: JobState::Running,
        started: instant(started),
        elapsed: value::Duration::ZERO,
        label: request.label.clone(),
        description: request.description.clone(),
        program: Some(path_value(&request.spawn.program)),
        session: request.session.clone(),
        retry_of: request.retry_of.clone(),
        ..Job::default()
    }
}

/// Record of a job with the elapsed time at `now`; the record of a finished
/// job keeps its elapsed time.
pub(crate) fn with_elapsed(job: &Job, now: WallTime) -> Job {
    let mut job = job.clone();
    if job.finished.is_none() {
        job.elapsed = duration_value(now.saturating_since(wall_time(job.started)));
    }
    job
}

/// Completes the record of a job with the finished run.
pub(crate) fn finish_job(job: &mut Job, result: &RunResult) {
    let end = &result.end;
    let output = &result.output;
    job.state = final_state(end);
    job.exit_code = end.exit_code.map(i64::from);
    job.signal = end.signal;
    job.killed_by = end.killed_by;
    job.finished = Some(instant(end.finished));
    job.elapsed = duration_value(end.finished.saturating_since(result.started));
    job.progress.clone_from(&output.progress);
    job.stdout_bytes = Some(output.stdout_bytes);
    job.stderr_bytes = Some(output.stderr_bytes);
    job.stdout_lines = Some(output.stdout_lines);
    job.stderr_lines = Some(output.stderr_lines);
    job.output_file = output.output_file.as_deref().map(path_value);
    job.stdout_file = output.stdout_file.as_deref().map(path_value);
    job.stderr_file = output.stderr_file.as_deref().map(path_value);
    job.output_cursor.clone_from(&output.output_cursor);
    job.op = result.op;
    job.changes = Some(result.track.changes.clone());
    job.suspended_total = Some(duration_value(end.suspended_total));
}

/// Execution record of a finished run.
pub(crate) fn exec_record(request: &RunRequest, result: &RunResult) -> Exec {
    let end = &result.end;
    let output = &result.output;
    let track = &result.track;
    Exec {
        command: request.command.clone(),
        program: path_value(&request.spawn.program),
        cwd: path_value(&request.spawn.cwd),
        pid: u64::from(result.pid),
        exit_code: end.exit_code.map_or(-1, i64::from),
        timed_out: end.killed_by == Some(KilledBy::Timeout),
        stalled: end.killed_by == Some(KilledBy::Idle),
        started: instant(result.started),
        elapsed: duration_value(end.finished.saturating_since(result.started)),
        stdout: output.stdout.clone(),
        stderr: output.stderr.clone(),
        stdout_bytes: output.stdout_bytes,
        stderr_bytes: output.stderr_bytes,
        stdout_lines: output.stdout_lines,
        stderr_lines: output.stderr_lines,
        decoded_as: output.decoded_as.clone(),
        changes: track.changes.clone(),
        tracking: track.tracking,
        watch: track.watch.iter().map(|path| path_value(path)).collect(),
        launcher: Some(request.spawn.launcher),
        signal: end.signal,
        signal_number: end.signal_number.map(i64::from),
        killed_by: end.killed_by,
        limit_hit: end.limit_hit,
        finished: Some(instant(end.finished)),
        cpu_time: Some(duration_value(end.usage.cpu_time)),
        max_rss: Some(Size::from_bytes(end.usage.max_rss)),
        shown: Some(output.shown.clone()),
        output_file: output.output_file.as_deref().map(path_value),
        stdout_file: output.stdout_file.as_deref().map(path_value),
        stderr_file: output.stderr_file.as_deref().map(path_value),
        output_cursor: output.output_cursor.clone(),
        output_sha256: output.output_sha256.clone(),
        summary_lines: (!output.summary_lines.is_empty()).then(|| output.summary_lines.clone()),
        progress: output.progress.clone(),
        preimage: Some(track.preimage),
        preimage_entries: Some(track.preimage_entries),
        preimage_bytes: Some(Size::from_bytes(track.preimage_bytes)),
        external_effect: (!request.external_effect.is_empty())
            .then(|| request.external_effect.clone()),
        batches: None,
        failed_batches: None,
        env_keys: None,
        job: result.job.clone(),
    }
}

/// Finished tool run as passed to a job finisher.
pub(crate) fn tool_result(request: &RunRequest, result: &RunResult) -> ToolResult {
    ToolResult {
        exec: exec_record(request, result),
        stdout_file: result
            .output
            .stdout_file
            .clone()
            .or_else(|| result.output.output_file.clone()),
        stderr_file: result.output.stderr_file.clone(),
        op: result.op,
    }
}
