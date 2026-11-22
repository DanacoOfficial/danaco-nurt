// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Supervision of one started tree.
//!
//! Each tree has one thread per output stream, which copies the output to
//! the sink, one thread that observes the end of the main process, and one
//! thread that writes the given input. Limits are enforced by the caller of
//! [`RunningProcess::wait`]: the thread that waits for a run measures the
//! tree and ends it, so no thread runs for a tree nobody waits for and
//! supervision needs no shared timer.
//!
//! A run ends when the main process ended and both output streams reached
//! their end: a process the program started in the background and that
//! keeps the output open belongs to the run until it closes the output or a
//! limit ends the tree. The remaining members of the tree are ended when
//! the run ends.

use std::io::{ErrorKind, Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{KilledBy, LimitHit, Signal};
use nurt_contracts::stage::{CancelToken, OutputStream};

use super::watchdog::{Verdict, Watchdog};
use crate::iface::{OutputSink, ProcessEnd, ProcessLimits, ResourceUsage, RunningProcess};
use crate::os::{Exit, Finished, Streams, Tree};

/// Size of the buffer of each output thread.
const READ_BUFFER: usize = 64 * 1024;
/// Longest wait between two checks of the cancellation token.
const CANCEL_POLL: Duration = Duration::from_millis(50);
/// Time to wait for the output to end after the tree was killed. A process
/// that left the tree may keep the output open; the run ends regardless.
const OUTPUT_DRAIN_AFTER_KILL: Duration = Duration::from_secs(1);

/// State shared with the threads of a tree.
#[derive(Default)]
struct State {
    exited: bool,
    open_streams: u8,
    collected: bool,
    abandoned: bool,
    error: Option<Error>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record_error(&self, error: Error) {
        self.lock().error.get_or_insert(error);
    }

    fn stream_closed(&self) {
        let mut state = self.lock();
        state.open_streams = state.open_streams.saturating_sub(1);
        drop(state);
        self.changed.notify_all();
    }

    /// Waits until `done` holds or `deadline` passes; returns whether
    /// `done` holds.
    fn wait_for(&self, deadline: Instant, done: impl Fn(&State) -> bool) -> bool {
        let mut state = self.lock();
        loop {
            if done(&state) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            state = self
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// Why the console ended a tree.
#[derive(Debug, Clone, Copy)]
struct Reason {
    killed_by: KilledBy,
    limit_hit: Option<LimitHit>,
}

/// Started tree under supervision.
pub(crate) struct Supervised {
    tree: Arc<Tree>,
    shared: Arc<Shared>,
    sink: Arc<dyn OutputSink>,
    cancel: CancelToken,
    kill_grace: Duration,
    watchdog: Watchdog,
    started: WallTime,
    started_at: Instant,
    suspended_since: Option<Instant>,
    suspended_total: Duration,
    next_sample: Option<Instant>,
    peak: ResourceUsage,
    waiter_started: bool,
    end: Option<ProcessEnd>,
}

impl Supervised {
    /// Supervises `tree` and starts the threads of its streams.
    pub(crate) fn start(
        tree: Tree,
        streams: Streams,
        limits: &ProcessLimits,
        sink: Arc<dyn OutputSink>,
        cancel: CancelToken,
        stdin: Option<Vec<u8>>,
    ) -> Result<Self> {
        let open_streams = if streams.stderr.is_some() { 2 } else { 1 };
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                open_streams,
                ..State::default()
            }),
            changed: Condvar::new(),
        });
        let watchdog = Watchdog::new(limits);
        let now = Instant::now();
        let mut supervised = Self {
            tree: Arc::new(tree),
            shared,
            sink,
            cancel,
            kill_grace: limits.kill_grace,
            next_sample: watchdog.sample_interval().map(|interval| now + interval),
            watchdog,
            started: wall_now(),
            started_at: now,
            suspended_since: None,
            suspended_total: Duration::ZERO,
            peak: ResourceUsage::default(),
            waiter_started: false,
            end: None,
        };
        supervised.start_waiter()?;
        supervised.waiter_started = true;
        supervised.start_reader(streams.stdout, OutputStream::Stdout)?;
        if let Some(stderr) = streams.stderr {
            supervised.start_reader(stderr, OutputStream::Stderr)?;
        }
        if let (Some(writer), Some(bytes)) = (streams.stdin, stdin) {
            start_thread("nurt-proc-input", move || write_input(writer, &bytes))?;
        }
        Ok(supervised)
    }

    fn start_waiter(&self) -> Result<()> {
        let tree = Arc::clone(&self.tree);
        let shared = Arc::clone(&self.shared);
        start_thread("nurt-proc-wait", move || {
            let observed = tree.wait_exit();
            let mut state = shared.lock();
            if let Err(error) = observed {
                state.error.get_or_insert(error);
            }
            state.exited = true;
            let collect = state.abandoned && !state.collected;
            state.collected |= collect;
            drop(state);
            shared.changed.notify_all();
            if collect {
                tree.finish().ok();
            }
        })
    }

    fn start_reader(&self, reader: Box<dyn Read + Send>, stream: OutputStream) -> Result<()> {
        let sink = Arc::clone(&self.sink);
        let shared = Arc::clone(&self.shared);
        start_thread("nurt-proc-output", move || {
            copy_output(reader, stream, sink.as_ref(), &shared);
        })
    }

    /// Active time: time since the start without suspension.
    fn active(&self, now: Instant) -> Duration {
        let current = self
            .suspended_since
            .map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
        now.saturating_duration_since(self.started_at)
            .saturating_sub(self.suspended_total)
            .saturating_sub(current)
    }

    fn ended(&self) -> bool {
        let state = self.shared.lock();
        state.exited && state.open_streams == 0
    }

    /// Measures the tree when a measurement is due and returns the limit
    /// the tree reached, if any.
    fn check(&mut self, now: Instant) -> Result<Option<Reason>> {
        if self.cancel.is_cancelled() {
            return Ok(Some(Reason {
                killed_by: KilledBy::Cancel,
                limit_hit: None,
            }));
        }
        if let Some(limit_hit) = self.tree.limit_hit() {
            return Ok(Some(Reason {
                killed_by: KilledBy::Limit,
                limit_hit: Some(limit_hit),
            }));
        }
        if self.suspended_since.is_some() {
            return Ok(None);
        }
        let active = self.active(now);
        if self.next_sample.is_some_and(|due| now >= due) {
            let usage = self.tree.sample()?;
            self.record_peak(usage);
            self.watchdog.observe_usage(active, usage);
            self.next_sample = self
                .watchdog
                .sample_interval()
                .map(|interval| now + interval);
        }
        self.watchdog
            .observe_output(active, self.sink.last_activity());
        Ok(self.watchdog.verdict(active).map(|verdict| match verdict {
            Verdict::Memory => Reason {
                killed_by: KilledBy::Limit,
                limit_hit: Some(LimitHit::Memory),
            },
            Verdict::Cpu => Reason {
                killed_by: KilledBy::Limit,
                limit_hit: Some(LimitHit::Cpu),
            },
            Verdict::Timeout => Reason {
                killed_by: KilledBy::Timeout,
                limit_hit: None,
            },
            Verdict::Idle => Reason {
                killed_by: KilledBy::Idle,
                limit_hit: None,
            },
        }))
    }

    fn record_peak(&mut self, usage: ResourceUsage) {
        self.peak.cpu_time = self.peak.cpu_time.max(usage.cpu_time);
        self.peak.max_rss = self.peak.max_rss.max(usage.max_rss);
    }

    /// Next time the wait loop has to look at the tree.
    fn next_wake(&self, now: Instant) -> Instant {
        let mut wake = now + CANCEL_POLL;
        if self.suspended_since.is_none() {
            if let Some(due) = self.next_sample {
                wake = wake.min(due);
            }
            if let Some(left) = self.watchdog.until_deadline(self.active(now)) {
                wake = wake.min(now + left);
            }
        }
        wake
    }

    /// Ends the tree for `reason`: a memory limit kills at once, every other
    /// reason requests termination and kills after the grace time.
    fn enforce(&mut self, reason: Reason) -> Result<ProcessEnd> {
        let signal = if reason.limit_hit == Some(LimitHit::Memory) {
            Signal::Kill
        } else {
            Signal::Terminate
        };
        self.shut_down(signal, self.kill_grace)?;
        self.complete(Some(reason))
    }

    fn shut_down(&mut self, signal: Signal, grace: Duration) -> Result<()> {
        self.tree.signal(signal)?;
        if self.suspended_since.is_some() {
            // A stopped process receives the termination request once it
            // continues.
            self.tree.resume()?;
            self.end_suspension(Instant::now());
        }
        let ended = |state: &State| state.exited && state.open_streams == 0;
        if signal != Signal::Kill && !self.shared.wait_for(Instant::now() + grace, ended) {
            self.tree.kill()?;
        }
        let mut state = self.shared.lock();
        while !state.exited {
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        drop(state);
        self.shared
            .wait_for(Instant::now() + OUTPUT_DRAIN_AFTER_KILL, ended);
        Ok(())
    }

    fn end_suspension(&mut self, now: Instant) {
        if let Some(since) = self.suspended_since.take() {
            self.suspended_total = self
                .suspended_total
                .saturating_add(now.saturating_duration_since(since));
        }
    }

    /// Collects the main process after its end and builds the result.
    fn complete(&mut self, reason: Option<Reason>) -> Result<ProcessEnd> {
        let mut state = self.shared.lock();
        state.collected = true;
        let error = state.error.take();
        drop(state);
        let finished = self.tree.finish()?;
        let end = self.process_end(&finished, reason);
        self.end = Some(end.clone());
        match error {
            Some(error) => Err(error),
            None => Ok(end),
        }
    }

    fn process_end(&self, finished: &Finished, reason: Option<Reason>) -> ProcessEnd {
        let (exit_code, signal, signal_number) = match finished.exit {
            Exit::Code(code) => (Some(code), None, None),
            Exit::Signal { signal, number } => (None, Some(signal), number),
        };
        let limit_hit = reason
            .and_then(|reason| reason.limit_hit)
            .or(finished.limit_hit);
        let killed_by = reason
            .map(|reason| reason.killed_by)
            .or(finished.limit_hit.map(|_| KilledBy::Limit));
        ProcessEnd {
            exit_code,
            signal,
            signal_number,
            killed_by,
            limit_hit,
            finished: wall_now(),
            usage: ResourceUsage {
                cpu_time: self.peak.cpu_time.max(finished.usage.cpu_time),
                max_rss: self.peak.max_rss.max(finished.usage.max_rss),
            },
            suspended_total: self.suspended_total,
        }
    }

    fn ensure_running(&self) -> Result<()> {
        if self.end.is_some() {
            return Err(Error::new(
                ErrorCode::JobState,
                "the process has already ended",
            ));
        }
        Ok(())
    }
}

impl RunningProcess for Supervised {
    fn pid(&self) -> u32 {
        self.tree.pid()
    }

    fn started(&self) -> WallTime {
        self.started
    }

    /// Waits for the end of the run while enforcing the limits.
    ///
    /// # Errors
    ///
    /// A failed system call, or the first error of the output sink; the
    /// latter is returned once, after the end of the run, and later calls
    /// return the end.
    fn wait(&mut self, timeout: Option<Duration>) -> Result<Option<ProcessEnd>> {
        if let Some(end) = &self.end {
            return Ok(Some(end.clone()));
        }
        let deadline = timeout.and_then(|timeout| Instant::now().checked_add(timeout));
        loop {
            if self.ended() {
                return self.complete(None).map(Some);
            }
            let now = Instant::now();
            if let Some(reason) = self.check(now)? {
                return self.enforce(reason).map(Some);
            }
            if deadline.is_some_and(|deadline| now >= deadline) {
                return Ok(None);
            }
            let wake = deadline.map_or(self.next_wake(now), |deadline| {
                deadline.min(self.next_wake(now))
            });
            let ended = |state: &State| state.exited && state.open_streams == 0;
            self.shared.wait_for(wake, ended);
        }
    }

    fn signal(&mut self, signal: Signal) -> Result<()> {
        self.ensure_running()?;
        self.tree.signal(signal)
    }

    fn stop(&mut self, signal: Signal, grace: Duration) -> Result<ProcessEnd> {
        if let Some(end) = &self.end {
            return Ok(end.clone());
        }
        self.shut_down(signal, grace)?;
        self.complete(Some(Reason {
            killed_by: KilledBy::Stop,
            limit_hit: None,
        }))
    }

    fn suspend(&mut self) -> Result<()> {
        self.ensure_running()?;
        if self.suspended_since.is_some() {
            return Err(Error::new(
                ErrorCode::JobState,
                "the process is already suspended",
            ));
        }
        self.tree.suspend()?;
        self.suspended_since = Some(Instant::now());
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        self.ensure_running()?;
        if self.suspended_since.is_none() {
            return Err(Error::new(
                ErrorCode::JobState,
                "the process is not suspended",
            ));
        }
        self.tree.resume()?;
        self.end_suspension(Instant::now());
        Ok(())
    }

    fn usage(&self) -> Result<ResourceUsage> {
        if let Some(end) = &self.end {
            return Ok(end.usage);
        }
        let usage = self.tree.sample()?;
        Ok(ResourceUsage {
            cpu_time: self.peak.cpu_time.max(usage.cpu_time),
            max_rss: self.peak.max_rss.max(usage.max_rss),
        })
    }
}

impl Drop for Supervised {
    /// A tree dropped before its end is killed. The main process is
    /// collected here when it already ended, otherwise by the thread that
    /// observes its end.
    fn drop(&mut self) {
        if self.end.is_some() {
            return;
        }
        self.tree.kill().ok();
        if !self.waiter_started {
            if self.tree.wait_exit().is_ok() {
                self.tree.finish().ok();
            }
            return;
        }
        let mut state = self.shared.lock();
        let collect = state.exited && !state.collected;
        state.collected |= collect;
        state.abandoned = true;
        drop(state);
        if collect {
            self.tree.finish().ok();
        }
    }
}

fn start_thread(name: &str, body: impl FnOnce() + Send + 'static) -> Result<()> {
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(body)
        .map(drop)
        .map_err(|error| Error::new(ErrorCode::Io, format!("cannot start a thread: {error}")))
}

fn copy_output(
    mut reader: Box<dyn Read + Send>,
    stream: OutputStream,
    sink: &dyn OutputSink,
    shared: &Shared,
) {
    let mut buffer = vec![0u8; READ_BUFFER];
    // After a sink error the output is still read to its end, so the
    // program never blocks on a full pipe.
    let mut accepting = true;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if accepting && let Err(error) = sink.write(stream, &buffer[..count]) {
                    accepting = false;
                    shared.record_error(error);
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => {
                shared.record_error(Error::new(
                    ErrorCode::Io,
                    format!("cannot read the output of the program: {error}"),
                ));
                break;
            }
        }
    }
    if accepting && let Err(error) = sink.finish(stream) {
        shared.record_error(error);
    }
    shared.stream_closed();
}

fn write_input(mut writer: Box<dyn Write + Send>, bytes: &[u8]) {
    // A program may end or close its input before reading all of it; the
    // rest of the input is then discarded.
    writer.write_all(bytes).ok();
}

fn wall_now() -> WallTime {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    WallTime::from_unix_nanos(i128::try_from(nanos).unwrap_or(i128::MAX))
}
