// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The cycle of one run, executed by a job thread.
//!
//! Every run is a job from its start. Its thread opens the journal
//! transaction, prepares change detection, starts the program, supervises it
//! until it ends, records its changes, commits and writes the final record.
//! The thread alone decides when the run moves to the background: when the
//! waiting time of the call has elapsed. A caller in the same process learns
//! the outcome through a [`Waiter`]; callers in other processes read the job
//! file.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nurt_contracts::bound::BoundDocument;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{MutationTx, RollbackReason};
use nurt_contracts::record::{Job, JobState, KilledBy, Signal};
use nurt_contracts::stage::{CancelReason, CancelToken, Event, ExecCtx, LogEvent, LogLevel};

use super::control::{self, ControlAction};
use super::events::{self, OutputEvents};
use super::journal::{begin, bound_program};
use super::record::{external_effect, finish_job, started_job, tool_result, with_elapsed};
use super::state::{Action, check_action};
use super::table::{JobFile, JobTable, PROTOCOL, io_error};
use super::wire::{StoredInput, StoredRequest, StoredResult};
use crate::api::{JobFinishers, ToolKind};
use crate::iface::{
    Components, OutputSink, OutputStore, PreparedScan, ProcessEnd, ResourceUsage, RunKind,
    RunRequest, RunResult, RunWait, RunningProcess, StdinSource, TrackOutcome,
};
use crate::spawn::Platform;

/// Interval at which a job thread waits for its program before it looks for
/// control requests, its waiting time and new output.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// One run as handed to a job thread.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Launch {
    /// Command that started the run, for example `run` or `test.run`.
    pub(crate) command_name: String,
    /// Bound program recorded in the journal entry.
    pub(crate) program: BoundDocument,
    /// The run.
    pub(crate) request: RunRequest,
    /// Time the run may take before it moves to the background; `None`
    /// keeps it in the foreground until it ends.
    pub(crate) wait: Option<Duration>,
}

impl Launch {
    /// Launch of `request` by the command running in `ctx`.
    pub(crate) fn new(ctx: &ExecCtx, request: RunRequest, wait: Option<Duration>) -> Self {
        let command_name = match ctx.frame().command.as_str() {
            "" => default_command(request.kind).to_owned(),
            name => name.to_owned(),
        };
        let program = bound_program(ctx, &command_name, &request);
        Self {
            command_name,
            program,
            request,
            wait,
        }
    }
}

fn default_command(kind: RunKind) -> &'static str {
    match kind {
        RunKind::Run => "run",
        RunKind::Retry => "retry",
        RunKind::Test => "test.run",
        RunKind::Analyze => "analyze.check",
    }
}

#[derive(Default)]
struct WaitState {
    started: Option<Job>,
    outcome: Option<Result<RunWait>>,
}

/// Outcome of a run as seen by a caller in the same process.
#[derive(Default)]
pub(crate) struct Waiter {
    state: Mutex<WaitState>,
    changed: Condvar,
}

impl Waiter {
    fn lock(&self) -> MutexGuard<'_, WaitState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn started(&self, job: Job) {
        self.lock().started = Some(job);
        self.changed.notify_all();
    }

    fn settle(&self, outcome: Result<RunWait>) {
        let mut state = self.lock();
        if state.outcome.is_none() {
            state.outcome = Some(outcome);
        }
        drop(state);
        self.changed.notify_all();
    }

    /// Blocks until the program has started and returns the job record.
    ///
    /// # Errors
    ///
    /// Fails with the error that prevented the start.
    pub(crate) fn wait_started(&self) -> Result<Job> {
        let mut state = self.lock();
        loop {
            if let Some(job) = &state.started {
                return Ok(job.clone());
            }
            if let Some(outcome) = &state.outcome {
                return match outcome {
                    Err(error) => Err(error.clone()),
                    Ok(_) => Err(thread_ended()),
                };
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Blocks until the run has ended or moved to the background.
    ///
    /// # Errors
    ///
    /// Fails with the error that prevented the start.
    pub(crate) fn wait_outcome(&self) -> Result<RunWait> {
        let mut state = self.lock();
        loop {
            if let Some(outcome) = &state.outcome {
                return outcome.clone();
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

fn thread_ended() -> Error {
    Error::new(
        ErrorCode::StageFailed,
        "the job thread ended before the program started",
    )
}

/// Settles the waiter when the job thread ends without an outcome,
/// including by a panic.
struct SettleOnExit(Arc<Waiter>);

impl Drop for SettleOnExit {
    fn drop(&mut self) {
        self.0.settle(Err(thread_ended()));
    }
}

/// Starts a job thread for `launch`.
///
/// # Errors
///
/// Fails with `E_IO` when no job identifier can be allocated or the thread
/// cannot be started.
pub(crate) fn start(
    ctx: &ExecCtx,
    components: &Components,
    table: &JobTable,
    launch: Launch,
) -> Result<(Arc<Waiter>, JoinHandle<()>)> {
    let id = table.allocate()?;
    let waiter = Arc::new(Waiter::default());
    let job = JobRun {
        ctx: ctx.clone(),
        components: components.clone(),
        table: table.clone(),
        id: id.clone(),
        launch,
        waiter: Arc::clone(&waiter),
    };
    let handle = std::thread::Builder::new()
        .name(format!("nurt-job-{id}"))
        .spawn(move || job.execute())
        .map_err(|error| io_error("start the job thread", &error))?;
    Ok((waiter, handle))
}

struct JobRun {
    ctx: ExecCtx,
    components: Components,
    table: JobTable,
    id: String,
    launch: Launch,
    waiter: Arc<Waiter>,
}

/// State of a job while its program runs.
struct Supervised {
    file: JobFile,
    begun: Instant,
    stop_requests: Vec<String>,
}

impl JobRun {
    fn execute(self) {
        let _settle = SettleOnExit(Arc::clone(&self.waiter));
        if let Err(error) = self.run() {
            self.waiter.settle(Err(error));
        }
    }

    fn run(&self) -> Result<()> {
        let ctx = &self.ctx;
        let request = &self.launch.request;
        let begun = Instant::now();
        let _lock = self.table.lock_job(&self.id)?;
        let mut tx = begin(
            ctx,
            &self.launch.command_name,
            self.launch.program.clone(),
            request,
        );
        if let Some(entry) = external_effect(request) {
            tx.mark_irreversible(entry);
        }
        let scan = match self
            .components
            .tracker
            .prepare(ctx, &request.track, tx.as_mut())
        {
            Ok(scan) => scan,
            Err(error) => return Err(rolled_back(tx, error)),
        };
        let store = match self
            .components
            .spool
            .create(ctx, &self.id, request.spawn.merge_stderr)
        {
            Ok(store) => store,
            Err(error) => return Err(rolled_back(tx, error)),
        };
        // A call cancelled during the preparation must not start the program.
        if let Err(error) = ctx.cancel().check() {
            let _ = self.components.spool.remove(ctx, &self.id);
            return Err(rolled_back(tx, error));
        }
        let cancel = CancelToken::new();
        let sink: Arc<dyn OutputSink> = store.clone();
        // The time limit covers the preparation of the run as well.
        let mut spawn = request.spawn.clone();
        spawn.limits.timeout = spawn
            .limits
            .timeout
            .map(|timeout| timeout.saturating_sub(begun.elapsed()));
        let mut process = match self.components.control.spawn(spawn, sink, cancel.clone()) {
            Ok(process) => process,
            Err(error) => {
                let _ = self.components.spool.remove(ctx, &self.id);
                return Err(rolled_back(tx, error));
            }
        };
        let file = self.started_file(process.as_ref());
        if let Err(error) = self.record_start(&file) {
            let _ = process.stop(Signal::Kill, Duration::ZERO);
            return Err(rolled_back(tx, error));
        }
        // The cancellation is a state of the call, not an event: one that was
        // set while the program started ends the group at once.
        if let Some(reason) = ctx.cancel().reason() {
            cancel.cancel(reason);
        }
        ctx.emit(events::started(&file.record));
        self.waiter.started(file.record.clone());
        let mut supervised = Supervised {
            file,
            begun: Instant::now(),
            stop_requests: Vec::new(),
        };
        let mut output = OutputEvents::new(&self.id, request.view.progress.clone());
        let end = self.supervise(
            &mut supervised,
            process.as_mut(),
            store.as_ref(),
            &mut output,
            &cancel,
        );
        output.pump(ctx, store.as_ref(), true);
        self.finish(supervised, process.as_ref(), end, scan, tx, store.as_ref());
        Ok(())
    }

    fn started_file(&self, process: &dyn RunningProcess) -> JobFile {
        let mut request = self.launch.request.clone();
        request.spawn.env = Arc::from([]);
        request.spawn.stdin = StdinSource::Null;
        JobFile {
            protocol: PROTOCOL,
            record: started_job(&self.id, &request, process.pid(), process.started()),
            command_name: self.launch.command_name.clone(),
            request: StoredRequest(request),
            wait: self.launch.wait,
            supervisor_pid: std::process::id(),
            detached: false,
            result: None,
            final_record: None,
        }
    }

    fn record_start(&self, file: &JobFile) -> Result<()> {
        let spawn = &self.launch.request.spawn;
        self.table.write_input(
            &self.id,
            &StoredInput {
                env: spawn.env.to_vec(),
                stdin: spawn.stdin.clone(),
            },
        )?;
        self.table.write(file)
    }

    fn supervise(
        &self,
        job: &mut Supervised,
        process: &mut dyn RunningProcess,
        store: &dyn OutputStore,
        output: &mut OutputEvents,
        cancel: &CancelToken,
    ) -> ProcessEnd {
        loop {
            // The cancellation goes first: a call cancelled in the cycle in
            // which its wait elapses must stop the run, not leave it in the
            // background.
            if !job.file.detached
                && let Some(reason) = self.ctx.cancel().reason()
            {
                cancel.cancel(reason);
            } else {
                self.check_wait(job);
            }
            let mut stopped = None;
            for (name, action) in control::pending(&self.table, &self.id) {
                if stopped.is_some() {
                    after_stop(&self.table, &self.id, job, &name, action);
                } else {
                    stopped = self.control(job, process, cancel, &name, action);
                }
            }
            if let Some(end) = stopped {
                return end;
            }
            match process.wait(Some(POLL_INTERVAL)) {
                Ok(Some(end)) => return end,
                Ok(None) => output.pump(&self.ctx, store, false),
                Err(error) => {
                    self.log(&format!(
                        "waiting for job `{}` failed: {}",
                        self.id, error.message
                    ));
                    return process
                        .stop(Signal::Kill, Duration::ZERO)
                        .unwrap_or_else(|_| self.unknown_end());
                }
            }
        }
    }

    fn check_wait(&self, job: &mut Supervised) {
        if job.file.detached
            || !self
                .launch
                .wait
                .is_some_and(|wait| job.begun.elapsed() >= wait)
        {
            return;
        }
        job.file.detached = true;
        if let Err(error) = self.table.write(&job.file) {
            self.log(&error.message);
        }
        let record = with_elapsed(&job.file.record, self.ctx.clock().now());
        self.waiter
            .settle(Ok(RunWait::Background(Box::new(record))));
    }

    fn control(
        &self,
        job: &mut Supervised,
        process: &mut dyn RunningProcess,
        cancel: &CancelToken,
        name: &str,
        action: ControlAction,
    ) -> Option<ProcessEnd> {
        let state = job.file.record.state;
        let result = match action {
            ControlAction::Stop { signal, grace } => {
                match check_action(&self.id, state, Action::Stop)
                    .and_then(|()| process.stop(signal, grace))
                {
                    Ok(mut end) => {
                        end.killed_by.get_or_insert(KilledBy::Stop);
                        job.stop_requests.push(name.to_owned());
                        return Some(end);
                    }
                    Err(error) => Err(error),
                }
            }
            ControlAction::Cancel => {
                cancel.cancel(CancelReason::Requested);
                return None;
            }
            ControlAction::Suspend => check_action(&self.id, state, Action::Suspend)
                .and_then(|()| process.suspend())
                .and_then(|()| self.set_state(job, JobState::Suspended)),
            ControlAction::Resume => check_action(&self.id, state, Action::Resume)
                .and_then(|()| process.resume())
                .and_then(|()| self.set_state(job, JobState::Running)),
        };
        control::reply(&self.table, &self.id, name, result);
        None
    }

    fn set_state(&self, job: &mut Supervised, state: JobState) -> Result<Job> {
        job.file.record.state = state;
        self.table.write(&job.file)?;
        Ok(with_elapsed(&job.file.record, self.ctx.clock().now()))
    }

    fn unknown_end(&self) -> ProcessEnd {
        ProcessEnd {
            exit_code: None,
            signal: None,
            signal_number: None,
            killed_by: None,
            limit_hit: None,
            finished: self.ctx.clock().now(),
            usage: ResourceUsage::default(),
            suspended_total: Duration::ZERO,
            stdin_closed_early: false,
        }
    }

    fn finish(
        &self,
        job: Supervised,
        process: &dyn RunningProcess,
        end: ProcessEnd,
        scan: Box<dyn PreparedScan>,
        mut tx: Box<dyn MutationTx + '_>,
        store: &dyn OutputStore,
    ) {
        let ctx = &self.ctx;
        let request = &self.launch.request;
        let track = scan.finish(ctx, tx.as_mut()).unwrap_or_else(|error| {
            self.log(&format!(
                "changes of job `{}` were not detected: {}",
                self.id, error.message
            ));
            TrackOutcome::default()
        });
        let op = match tx.commit() {
            Ok(handle) => handle.op.map(|op| op.0),
            Err(error) => {
                self.log(&format!(
                    "changes of job `{}` were not journaled: {}",
                    self.id, error.message
                ));
                None
            }
        };
        let output = store
            .render(&request.view, end.exit_code)
            .unwrap_or_else(|error| {
                self.log(&format!(
                    "output of job `{}` cannot be read: {}",
                    self.id, error.message
                ));
                crate::iface::RenderedOutput::default()
            });
        let Supervised {
            mut file,
            stop_requests,
            ..
        } = job;
        let result = RunResult {
            pid: process.pid(),
            started: process.started(),
            end,
            output,
            track,
            op,
            job: file.detached.then(|| self.id.clone()),
        };
        finish_job(&mut file.record, &result);
        file.result = Some(StoredResult(result.clone()));
        if file.detached {
            file.final_record = self.final_record(&result);
        }
        if let Err(error) = self.table.write(&file) {
            self.log(&error.message);
        }
        ctx.emit(events::finished(&file.record));
        for name in stop_requests {
            control::reply(&self.table, &self.id, &name, Ok(file.record.clone()));
        }
        self.waiter.settle(Ok(RunWait::Finished(Box::new(result))));
    }

    fn final_record(&self, result: &RunResult) -> Option<nurt_contracts::value::Record> {
        let request = &self.launch.request;
        let kind = match request.kind {
            RunKind::Test => ToolKind::Test,
            RunKind::Analyze => ToolKind::Analyze,
            RunKind::Run | RunKind::Retry => return None,
        };
        let finisher = self.ctx.extensions().get::<JobFinishers>()?.get(kind)?;
        let tool = tool_result(request, result, Platform::of(self.ctx.host().info().os));
        match finisher.finish(&self.ctx, &tool, request.finish_data.as_ref()) {
            Ok(record) => Some(record),
            Err(error) => {
                self.log(&format!(
                    "result of job `{}` failed: {}",
                    self.id, error.message
                ));
                None
            }
        }
    }

    fn log(&self, message: &str) {
        self.ctx.emit(Event::Log(LogEvent {
            level: LogLevel::Warn,
            message: message.to_owned(),
            source: Some("jobs".to_owned()),
        }));
    }
}

/// Handles a request taken in the same poll as a stop request that already
/// ended the program: a further stop is answered with the final record, a
/// suspension or resumption is refused.
fn after_stop(table: &JobTable, id: &str, job: &mut Supervised, name: &str, action: ControlAction) {
    match action {
        ControlAction::Stop { .. } => job.stop_requests.push(name.to_owned()),
        ControlAction::Cancel => {}
        ControlAction::Suspend | ControlAction::Resume => {
            let refused = check_action(id, JobState::Stopped, action.action())
                .map(|()| job.file.record.clone());
            control::reply(table, id, name, refused);
        }
    }
}

fn rolled_back(tx: Box<dyn MutationTx + '_>, error: Error) -> Error {
    // The journal records why the entry was undone: a run that did not start
    // because its call was cancelled is `cancelled`, not a failed target.
    let reason = if error.code() == ErrorCode::Cancelled {
        RollbackReason::Cancelled
    } else {
        RollbackReason::TargetFailed
    };
    match tx.rollback(reason) {
        Ok(report) if report.failed.is_empty() => error,
        Ok(report) => Error::new(
            ErrorCode::RollbackFailed,
            "the run did not start and its preparation could not be undone completely",
        )
        .with_detail(
            "failed",
            serde_json::to_value(&report.failed).unwrap_or_default(),
        )
        .with_cause(error),
        Err(rollback) => rollback.with_cause(error),
    }
}
