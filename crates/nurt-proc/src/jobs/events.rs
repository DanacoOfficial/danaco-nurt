// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Events of launched programs: `job.started`, `job.output`,
//! `job.progress` and `job.finished`.

use std::time::{Duration, Instant};

use nurt_contracts::error::Result;
use nurt_contracts::record::{Job, Progress};
use nurt_contracts::stage::{
    Event, ExecCtx, JobFinished, JobOutput, JobProgress, JobStarted, LogEvent, LogLevel,
    OutputStream,
};

use super::state::job_end;
use crate::iface::{OutputStore, OutputView, ShowMode};

/// Shortest time between two batches of output events of one job.
pub(crate) const OUTPUT_EVENT_INTERVAL: Duration = Duration::from_millis(200);

/// Event of a started job.
pub(crate) fn started(job: &Job) -> Event {
    Event::JobStarted(JobStarted {
        id: job.id.clone(),
        label: job.label.clone(),
        command: job.command.clone(),
        cwd: job.cwd.as_str().to_owned(),
        pid: u32::try_from(job.pid).unwrap_or(u32::MAX),
        started: job.started,
    })
}

/// Event of a finished job.
pub(crate) fn finished(job: &Job) -> Event {
    Event::JobFinished(JobFinished {
        id: job.id.clone(),
        state: job_end(job.state),
        exit_code: job.exit_code.and_then(|code| i32::try_from(code).ok()),
        signal: job.signal.map(|signal| signal.as_str().to_owned()),
        elapsed: job.elapsed,
        output_file: job
            .output_file
            .as_ref()
            .or(job.stdout_file.as_ref())
            .map(|path| path.as_str().to_owned()),
        changes: job.changes.as_ref().map_or(0, |changes| {
            u64::try_from(changes.len()).unwrap_or(u64::MAX)
        }),
    })
}

/// Emits the output of a running job in batches of complete lines, at most
/// once per [`OUTPUT_EVENT_INTERVAL`].
pub(crate) struct OutputEvents {
    id: String,
    view: OutputView,
    cursor: Option<String>,
    progress: Option<Progress>,
    last: Option<Instant>,
    failed: bool,
}

impl OutputEvents {
    /// Events of job `id`; `progress` is the pattern of progress lines.
    pub(crate) fn new(id: &str, progress: Option<String>) -> Self {
        Self {
            id: id.to_owned(),
            view: OutputView {
                show: ShowMode::Both,
                progress,
                ..OutputView::default()
            },
            cursor: None,
            progress: None,
            last: None,
            failed: false,
        }
    }

    /// Emits the lines written since the previous batch; with `force` also
    /// when the interval has not elapsed. After the first failure to read
    /// the output, one log event is emitted and no further output events.
    pub(crate) fn pump(&mut self, ctx: &ExecCtx, store: &dyn OutputStore, force: bool) {
        if self.failed {
            return;
        }
        let now = Instant::now();
        if !force
            && self
                .last
                .is_some_and(|last| now.duration_since(last) < OUTPUT_EVENT_INTERVAL)
        {
            return;
        }
        self.last = Some(now);
        if let Err(error) = self.emit_new(ctx, store) {
            self.failed = true;
            ctx.emit(Event::Log(LogEvent {
                level: LogLevel::Warn,
                message: format!(
                    "output events of job `{}` stopped: {}",
                    self.id, error.message
                ),
                source: Some("jobs".to_owned()),
            }));
        }
    }

    fn emit_new(&mut self, ctx: &ExecCtx, store: &dyn OutputStore) -> Result<()> {
        let output = store.read_since(self.cursor.as_deref(), &self.view)?;
        let Some(cursor) = output.output_cursor.clone() else {
            return Ok(());
        };
        if self.cursor.as_deref() != Some(cursor.as_str()) {
            for (stream, text) in [
                (OutputStream::Stdout, &output.stdout),
                (OutputStream::Stderr, &output.stderr),
            ] {
                if text.is_empty() {
                    continue;
                }
                let lines: Vec<String> = text.lines().map(str::to_owned).collect();
                let bytes = lines
                    .iter()
                    .map(|line| {
                        u64::try_from(line.len())
                            .unwrap_or(u64::MAX)
                            .saturating_add(1)
                    })
                    .sum();
                ctx.emit(Event::JobOutput(JobOutput {
                    id: self.id.clone(),
                    stream,
                    cursor: cursor.clone(),
                    lines,
                    bytes,
                }));
            }
            self.cursor = Some(cursor);
        }
        if output.progress.is_some() && output.progress != self.progress {
            self.progress.clone_from(&output.progress);
            if let Some(progress) = &output.progress {
                ctx.emit(Event::JobProgress(JobProgress {
                    id: self.id.clone(),
                    percent: progress.percent,
                    line: Some(progress.line.clone()),
                }));
            }
        }
        Ok(())
    }
}
