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
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nurt_contracts::error::Result;
use nurt_contracts::stage::ExecCtx;

use super::host::{StartReply, StartRequest, inbox_requests, write_start_reply};
use super::runner::{self, POLL_INTERVAL};
use super::store::JobStore;
use super::table::{JobTable, PROTOCOL, read_json, remove_file};
use crate::iface::Components;

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
    let Some(lock) = table.try_lock_host()? else {
        return Ok(());
    };
    let mut lock: Option<File> = Some(lock);
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
        remove_file(&path)?;
        let reply = match request {
            Ok(Some(request)) if request.protocol == PROTOCOL => {
                match runner::start(ctx, components, table, request.into_launch()) {
                    Ok((waiter, job)) => {
                        jobs.push(job);
                        match waiter.wait_started() {
                            Ok(record) => StartReply::Started(record.id),
                            Err(error) => StartReply::Failed(error),
                        }
                    }
                    Err(error) => StartReply::Failed(error),
                }
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
