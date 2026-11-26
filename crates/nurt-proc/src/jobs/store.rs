// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Operations on jobs used by the commands `jobs`, `stop`, `suspend`,
//! `resume`, `retry` and `remove`.

use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Job, Signal};
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::Record;

use super::control::{self, ControlAction};
use super::record::{path_value, with_elapsed};
use super::runner::POLL_INTERVAL;
use super::state::{Action, check_action, is_live};
use super::table::{JobFile, JobTable};
use crate::iface::{Components, OutputView, RenderedOutput, RunKind, RunRequest, ShowMode};

/// Number of last output lines shown by the status of a job.
pub(crate) const STATUS_TAIL_LINES: u32 = 5;

/// The jobs of the state directory of one execution context.
pub(crate) struct JobStore<'a> {
    ctx: &'a ExecCtx,
    components: &'a Components,
    table: JobTable,
}

impl<'a> JobStore<'a> {
    /// Store over `table`.
    pub(crate) fn new(ctx: &'a ExecCtx, components: &'a Components, table: JobTable) -> Self {
        Self {
            ctx,
            components,
            table,
        }
    }

    /// Store of the state directory of `ctx`.
    ///
    /// # Errors
    ///
    /// Fails with `E_CONFIG` without a state directory and with `E_IO` when
    /// the job table cannot be created.
    pub(crate) fn open(ctx: &'a ExecCtx, components: &'a Components) -> Result<Self> {
        Ok(Self::new(ctx, components, JobTable::open(ctx)?))
    }

    /// Removes finished jobs older than `output.spool_ttl` with their output.
    ///
    /// A job whose files cannot be removed keeps its record and is retried by
    /// the next sweep; it does not stop the removal of the other jobs.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the job table cannot be read.
    pub(crate) fn sweep(&self) -> Result<()> {
        let ttl = self.ctx.config().output.spool_ttl.as_std();
        for id in self.table.expired(self.ctx.clock().now(), ttl)? {
            let _ = self.remove_files(&id);
        }
        Ok(())
    }

    /// All jobs: running and suspended jobs first, each group in the order
    /// of their identifiers.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the job table cannot be read.
    pub(crate) fn list(&self) -> Result<Vec<Job>> {
        self.sweep()?;
        let mut jobs = Vec::new();
        for id in self.table.ids()? {
            if let Some(file) = self.table.read(&id)? {
                jobs.push(self.record(&file));
            }
        }
        jobs.sort_by_key(|job| !is_live(job.state));
        Ok(jobs)
    }

    /// The job `id`.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_JOB` for an unknown job.
    pub(crate) fn get(&self, id: &str) -> Result<JobFile> {
        self.table.get(id)
    }

    /// Waits up to `limit` for job `id` to end and returns its file.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_JOB` for an unknown job and with `E_CANCELLED`
    /// or `E_TIMEOUT` when the call ends first.
    pub(crate) fn wait(&self, id: &str, limit: Duration) -> Result<JobFile> {
        let deadline = self.ctx.clock().monotonic().saturating_add(limit);
        loop {
            let file = self.table.get(id)?;
            let now = self.ctx.clock().monotonic();
            if !is_live(file.record.state) || now >= deadline {
                return Ok(file);
            }
            self.ctx
                .sleep(POLL_INTERVAL.min(deadline.saturating_sub(now)))?;
        }
    }

    /// Public record of a job, with the elapsed time of a running job.
    pub(crate) fn record(&self, file: &JobFile) -> Job {
        with_elapsed(&file.record, self.ctx.clock().now())
    }

    /// State of a job with the last lines of its output.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the output cannot be read.
    pub(crate) fn status(&self, file: &JobFile) -> Result<Job> {
        let view = OutputView {
            show: ShowMode::Both,
            head: Some(0),
            tail: Some(STATUS_TAIL_LINES),
            ..OutputView::default()
        };
        let store = self.components.spool.open(self.ctx, file.id())?;
        let output = store.render(&view, exit_code(file))?;
        Ok(with_output(self.record(file), &output))
    }

    /// Output of a job written after `since`.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the output cannot be read.
    pub(crate) fn output(
        &self,
        file: &JobFile,
        since: Option<&str>,
        view: &OutputView,
    ) -> Result<Job> {
        let store = self.components.spool.open(self.ctx, file.id())?;
        let output = store.read_since(since, view)?;
        Ok(with_output(self.record(file), &output))
    }

    /// Final record of a finished job with its output selected by `view`,
    /// and the final record of a tool job.
    ///
    /// # Errors
    ///
    /// Fails with `E_JOB_STATE` while the job runs and with `E_IO` when the
    /// output cannot be read.
    pub(crate) fn result(
        &self,
        file: &JobFile,
        view: &OutputView,
    ) -> Result<(Job, Option<Record>)> {
        if is_live(file.record.state) {
            let id = file.id();
            return Err(Error::new(
                ErrorCode::JobState,
                format!(
                    "job `{id}` is {}; its result exists after the end",
                    file.record.state
                ),
            )
            .with_found(file.record.state.as_str())
            .with_fix(format!("jobs \"{id}\" view: result wait: 60s")));
        }
        let store = self.components.spool.open(self.ctx, file.id())?;
        let output = store.render(view, exit_code(file))?;
        Ok((
            with_output(self.record(file), &output),
            file.final_record.clone(),
        ))
    }

    /// Stops job `id`: sends `signal` to its process tree and kills the tree
    /// after `grace`; returns the final record.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_JOB`, with `E_JOB_STATE` for a job that does not
    /// run, and with the error of the supervising process.
    pub(crate) fn stop(&self, id: &str, signal: Signal, grace: Duration) -> Result<Job> {
        control::send(
            self.ctx,
            &self.table,
            id,
            ControlAction::Stop { signal, grace },
        )
    }

    /// Suspends job `id`.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_JOB`, with `E_JOB_STATE` for a job that does not
    /// run, and with the error of the supervising process.
    pub(crate) fn suspend(&self, id: &str) -> Result<Job> {
        control::send(self.ctx, &self.table, id, ControlAction::Suspend)
    }

    /// Resumes job `id`.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_JOB`, with `E_JOB_STATE` for a job that is not
    /// suspended, and with the error of the supervising process.
    pub(crate) fn resume(&self, id: &str) -> Result<Job> {
        control::send(self.ctx, &self.table, id, ControlAction::Resume)
    }

    /// Removes a finished job with its record and output; returns its last
    /// record.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_JOB`, with `E_JOB_STATE` for a running job and
    /// with `E_IO` when the files cannot be removed.
    pub(crate) fn remove(&self, id: &str) -> Result<Job> {
        let file = self.table.get(id)?;
        check_action(id, file.record.state, Action::Remove)?;
        self.remove_files(id)?;
        Ok(file.record)
    }

    /// Request that repeats job `id` with the same command, working
    /// directory, environment and standard input, as the job `retry_of`.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_JOB`, with `E_JOB_STATE` for a running job and
    /// with `E_IO` when the job files cannot be read or the stored environment
    /// and standard input of the job are missing: repeating the command with
    /// an empty environment would not be the same run.
    pub(crate) fn retry_request(&self, id: &str, timeout: Option<Duration>) -> Result<RunRequest> {
        let file = self.table.get(id)?;
        check_action(id, file.record.state, Action::Retry)?;
        let mut request = file.request.0;
        let Some(input) = self.table.read_input(id)? else {
            return Err(Error::new(
                ErrorCode::Io,
                format!(
                    "job `{id}` has no stored environment and standard input; start the command again instead of retrying it"
                ),
            ));
        };
        request.spawn.env = Arc::from(input.env);
        request.spawn.stdin = input.stdin;
        if let Some(timeout) = timeout {
            request.spawn.limits.timeout = Some(timeout);
        }
        if request.kind == RunKind::Run {
            request.kind = RunKind::Retry;
        }
        request.retry_of = Some(id.to_owned());
        Ok(request)
    }

    fn remove_files(&self, id: &str) -> Result<()> {
        if let Err(error) = self.components.spool.remove(self.ctx, id)
            && error.code != ErrorCode::NotFound
        {
            return Err(error);
        }
        self.table.remove(id)
    }
}

fn exit_code(file: &JobFile) -> Option<i32> {
    file.record
        .exit_code
        .and_then(|code| i32::try_from(code).ok())
}

fn with_output(mut job: Job, output: &RenderedOutput) -> Job {
    job.stdout = Some(output.stdout.clone());
    job.stderr = Some(output.stderr.clone());
    job.stdout_bytes = Some(output.stdout_bytes);
    job.stderr_bytes = Some(output.stderr_bytes);
    job.stdout_lines = Some(output.stdout_lines);
    job.stderr_lines = Some(output.stderr_lines);
    if output.output_cursor.is_some() {
        job.output_cursor.clone_from(&output.output_cursor);
    }
    if output.progress.is_some() {
        job.progress.clone_from(&output.progress);
    }
    job.output_file = output
        .output_file
        .as_deref()
        .map(path_value)
        .or(job.output_file);
    job.stdout_file = output
        .stdout_file
        .as_deref()
        .map(path_value)
        .or(job.stdout_file);
    job.stderr_file = output
        .stderr_file
        .as_deref()
        .map(path_value)
        .or(job.stderr_file);
    job
}
