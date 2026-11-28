// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::jobs::record
// Responsibility:   Records of jobs and finished runs.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Records of jobs and finished runs.

use std::path::Path;
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::record::{Exec, Job, JobState, KilledBy};
use nurt_contracts::report::{Irreversible, IrreversibleEntry, Notable, NotableEntry};
use nurt_contracts::value::{self, Size};

use super::state::final_state;
use super::table::{instant, wall_time};
use crate::api::ToolResult;
use crate::iface::{RunRequest, RunResult, ShowMode};
use crate::spawn::{Platform, record_path};

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
        program: Some(path_value(&request.program)),
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

/// Execution record of a finished run, with paths in the canonical form of
/// `platform`.
pub(crate) fn exec_record(request: &RunRequest, result: &RunResult, platform: Platform) -> Exec {
    let end = &result.end;
    let output = &result.output;
    let track = &result.track;
    let path = |path: &Path| record_path(path, platform);
    Exec {
        command: request.command.clone(),
        program: path(&request.program),
        cwd: path(&request.spawn.cwd),
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
        watch: track.watch.iter().map(|watched| path(watched)).collect(),
        launcher: Some(request.spawn.launcher),
        signal: end.signal,
        signal_number: end.signal_number.map(i64::from),
        killed_by: end.killed_by,
        limit_hit: end.limit_hit,
        finished: Some(instant(end.finished)),
        cpu_time: Some(duration_value(end.usage.cpu_time)),
        max_rss: Some(Size::from_bytes(end.usage.max_rss)),
        shown: Some(output.shown.clone()),
        output_file: output.output_file.as_deref().map(path),
        stdout_file: output.stdout_file.as_deref().map(path),
        stderr_file: output.stderr_file.as_deref().map(path),
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
        env_keys: (!request.env_keys.is_empty()).then(|| request.env_keys.clone()),
        job: result.job.clone(),
        diagnostics: output
            .diagnostics
            .clone()
            .filter(|found| !found.first.is_empty()),
        digest: digest(request, result),
    }
}

/// One line about a successful run whose response shows only the last lines
/// of its output: exit code, time, lines and bytes of output and, for a
/// recognised tool, the numbers of errors and warnings.
fn digest(request: &RunRequest, result: &RunResult) -> Option<String> {
    let view = &request.view;
    if !view.compact || view.show != ShowMode::Auto || result.end.exit_code != Some(0) {
        return None;
    }
    let output = &result.output;
    let elapsed = result.end.finished.saturating_since(result.started);
    let elapsed = if elapsed < Duration::from_secs(1) {
        format!("{} ms", elapsed.as_millis())
    } else {
        format!("{:.1} s", elapsed.as_secs_f64())
    };
    let mut line = format!(
        "exit 0 in {elapsed}; {} lines, {} bytes of output",
        output.stdout_lines + output.stderr_lines,
        output.stdout_bytes + output.stderr_bytes,
    );
    if let Some(found) = &output.diagnostics {
        line.push_str(&format!(
            "; {}: {} errors, {} warnings",
            found.tool, found.errors, found.warnings
        ));
    }
    Some(line)
}

/// Irreversible entry of a run whose command line matched the external
/// effect classifier.
pub(crate) fn external_effect(request: &RunRequest) -> Option<IrreversibleEntry> {
    (!request.external_effect.is_empty()).then(|| IrreversibleEntry {
        detail: Some(request.external_effect.join(", ")),
        ..IrreversibleEntry::new(Irreversible::ExternalEffect)
    })
}

/// Notes of how a run ended that do not depend on its output.
pub(crate) fn end_notes(result: &RunResult) -> Option<NotableEntry> {
    result
        .end
        .stdin_closed_early
        .then(|| NotableEntry::new(Notable::StdinClosedEarly))
}

/// Finished tool run as passed to a job finisher.
pub(crate) fn tool_result(
    request: &RunRequest,
    result: &RunResult,
    platform: Platform,
) -> ToolResult {
    ToolResult {
        exec: exec_record(request, result, platform),
        stdout_file: result
            .output
            .stdout_file
            .clone()
            .or_else(|| result.output.output_file.clone()),
        stderr_file: result.output.stderr_file.clone(),
        op: result.op,
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
