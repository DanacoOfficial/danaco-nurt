// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The detached supervisor process.
//!
//! One supervisor serves one state directory; its lock `host.lock` excludes
//! a second one. It starts on demand, accepts start requests from the inbox,
//! runs each job in a thread of its own process, so that the process group
//! or job object of every program belongs to the supervisor and outlives the
//! command line that started it, and ends when no job has run and no request
//! has arrived for [`IDLE_EXIT`].

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nurt_contracts::error::Result;
use nurt_contracts::stage::{CancelReason, CancelToken, ExecCtx};

use super::control::withdraw;
use super::host::{
    StartReply, StartRequest, cancel_notice_path, inbox_requests, remove_cancel_notices,
    write_start_reply,
};
use super::runner::{self, POLL_INTERVAL};
use super::store::JobStore;
use super::table::{JobTable, PROTOCOL, read_json};
use crate::iface::Components;

/// Pause between two attempts to take the host lock.
const LOCK_RETRY: Duration = Duration::from_millis(10);

/// Time without jobs and requests after which the supervisor ends.
pub(crate) const IDLE_EXIT: Duration = Duration::from_secs(60);

/// Serves the state directory of `ctx` until it is idle; returns at once
/// when another supervisor serves it.
///
/// # Errors
///
/// Fails with `E_CONFIG` without a state directory and with `E_IO` when the
/// job table cannot be read.
pub(crate) fn serve(ctx: &ExecCtx, components: &Components) -> Result<()> {
    let table = JobTable::open(ctx)?;
    let Some(lock) = lock_host(&table)? else {
        return Ok(());
    };
    let mut lock: Option<File> = Some(lock);
    remove_cancel_notices(&table);
    JobStore::new(ctx, components, table.clone()).sweep()?;
    let mut jobs: Vec<JoinHandle<()>> = Vec::new();
    let mut idle_since = Instant::now();
    loop {
        let accepted = accept(ctx, components, &table, &mut jobs)?;
        jobs.retain(|job| !job.is_finished());
        if accepted > 0 || !jobs.is_empty() {
            idle_since = Instant::now();
        } else if idle_since.elapsed() >= IDLE_EXIT {
            drop(lock.take());
            if inbox_requests(&table)?.is_empty() {
                return Ok(());
            }
            match table.try_lock_host()? {
                Some(again) => {
                    lock.replace(again);
                    idle_since = Instant::now();
                }
                None => return Ok(()),
            }
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Number of attempts to take the host lock before another supervisor is
/// assumed to hold it.
const LOCK_ATTEMPTS: u32 = 10;

/// Takes the host lock. A client that asks whether a supervisor runs takes
/// the lock itself for an instant, so a free lock that is refused is tried
/// again before the supervisor gives up; a supervisor that gave up at that
/// moment would leave the client waiting for one that never starts.
fn lock_host(table: &JobTable) -> Result<Option<File>> {
    for attempt in 0..LOCK_ATTEMPTS {
        if let Some(lock) = table.try_lock_host()? {
            return Ok(Some(lock));
        }
        if attempt + 1 < LOCK_ATTEMPTS {
            std::thread::sleep(LOCK_RETRY);
        }
    }
    Ok(None)
}

fn accept(
    ctx: &ExecCtx,
    components: &Components,
    table: &JobTable,
    jobs: &mut Vec<JoinHandle<()>>,
) -> Result<usize> {
    let requests = inbox_requests(table)?;
    let count = requests.len();
    for (name, path) in requests {
        let request = read_json::<StartRequest>(&path);
        // The request belongs to whoever removes its file; a client that
        // withdrew it first has stopped waiting for the run.
        if !withdraw(&path)? {
            continue;
        }
        let reply = match request {
            Ok(Some(request)) if request.protocol == PROTOCOL => {
                // The run has a token of its own so that a cancel notice of
                // the client reaches the preparation of the run, which
                // ends before the program and the job exist.
                let token = ctx.cancel().child();
                let watch = watch_notice(cancel_notice_path(table, &name), token.clone());
                let started = match runner::start(
                    &ctx.with_cancel(token),
                    components,
                    table,
                    request.into_launch(),
                ) {
                    Ok((waiter, job)) => {
                        jobs.push(job);
                        match waiter.wait_started() {
                            Ok(record) => StartReply::Started(record.id),
                            Err(error) => StartReply::Failed(error),
                        }
                    }
                    Err(error) => StartReply::Failed(error),
                };
                watch.end();
                started
            }
            Ok(None) => continue,
            Ok(Some(_)) => StartReply::Failed(nurt_contracts::error::Error::new(
                nurt_contracts::error::ErrorCode::Unsupported,
                "the job supervisor speaks another protocol version; end it and run the command again",
            )),
            Err(error) => StartReply::Failed(error),
        };
        write_start_reply(table, &name, &reply)?;
    }
    Ok(count)
}

/// Thread that cancels `token` when the cancel notice at `path` appears.
struct NoticeWatch {
    done: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl NoticeWatch {
    /// Stops the watch.
    fn end(mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn watch_notice(path: PathBuf, token: CancelToken) -> NoticeWatch {
    let done = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&done);
    let thread = std::thread::Builder::new()
        .name("nurt-cancel-notice".to_owned())
        .spawn(move || {
            while !flag.load(Ordering::Acquire) {
                if path.exists() {
                    token.cancel(CancelReason::Requested);
                    let _ = withdraw(&path);
                    return;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            // A notice written after the preparation ended is the client's
            // to remove; the client cancels the started job by a control
            // request.
        })
        .ok();
    NoticeWatch { done, thread }
}
