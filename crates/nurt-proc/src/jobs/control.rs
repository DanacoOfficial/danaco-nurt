// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Control requests to the process that supervises a job.
//!
//! A requester writes `<name>.request.json` into the control directory of
//! the job; the job thread of the supervising process reads it at its next
//! poll, removes it, acts and writes `<name>.reply.json`. The same exchange
//! serves jobs supervised by this process, by another long-lived process and
//! by the detached supervisor.
//!
//! Whoever removes a request file owns the request: the supervising process
//! acts only on requests it removed itself, and a requester that cannot
//! remove its request any more waits for the reply, which the supervising
//! process writes after the final state of a stopped job.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Job, JobState, Signal};
use nurt_contracts::stage::ExecCtx;
use serde::{Deserialize, Serialize};

use super::state::{Action, check_action, is_live};
use super::table::{
    JobTable, PROTOCOL, create_private_dir, io_error, read_json, remove_file, write_json,
};

/// Interval at which requesters look for replies.
pub(crate) const CONTROL_POLL: Duration = Duration::from_millis(20);

/// Time a stopped job may need after its grace period to record its
/// changes and write its final record.
pub(crate) const STOP_FINISH_MARGIN: Duration = Duration::from_secs(60);

/// Time allowed for a reply to a request other than `stop`.
pub(crate) const CONTROL_TIMEOUT: Duration = Duration::from_secs(10);

const REQUEST_SUFFIX: &str = ".request.json";
const REPLY_SUFFIX: &str = ".reply.json";

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Action requested on a running job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum ControlAction {
    /// Send `signal` to the process tree, then kill it after `grace`.
    Stop {
        /// Signal sent first.
        signal: Signal,
        /// Time before the tree is killed.
        grace: Duration,
    },
    /// Suspend the process tree.
    Suspend,
    /// Resume the process tree.
    Resume,
    /// Cancel the run started by a call that is being cancelled.
    Cancel,
}

impl ControlAction {
    /// Action checked against the state of the job.
    pub(crate) fn action(self) -> Action {
        match self {
            Self::Stop { .. } | Self::Cancel => Action::Stop,
            Self::Suspend => Action::Suspend,
            Self::Resume => Action::Resume,
        }
    }

    fn timeout(self) -> Duration {
        match self {
            Self::Stop { grace, .. } => grace.saturating_add(STOP_FINISH_MARGIN),
            _ => CONTROL_TIMEOUT,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ControlRequest {
    protocol: u32,
    action: ControlAction,
}

/// Reply of the supervising process.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ControlReply {
    /// The action was performed; the record of the job after it.
    Done(Box<Job>),
    /// The action failed.
    Failed(Error),
}

/// Unique name of a request file written by this process.
pub(crate) fn request_name() -> String {
    let counter = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{counter}", std::process::id())
}

/// Sort key of a request name `<process>-<counter>`; counters compare as numbers.
pub(crate) fn request_order(name: &str) -> (u64, u64) {
    name.split_once('-')
        .and_then(|(process, counter)| Some((process.parse().ok()?, counter.parse().ok()?)))
        .unwrap_or((u64::MAX, u64::MAX))
}

/// Sends `action` to the process supervising job `id` and waits for its
/// reply.
///
/// # Errors
///
/// Fails with `E_JOB_STATE` when the job is not in a state that allows the
/// action or ends before it is performed, with `E_TIMEOUT` when the
/// supervising process does not answer, with `E_CANCELLED` when the call is
/// cancelled, and with the error of the supervising process.
pub(crate) fn send(
    ctx: &ExecCtx,
    table: &JobTable,
    id: &str,
    action: ControlAction,
) -> Result<Job> {
    let file = table.get(id)?;
    check_action(id, file.record.state, action.action())?;
    let dir = table.control_dir(id);
    create_private_dir(&dir)?;
    let name = request_name();
    let request_path = dir.join(format!("{name}{REQUEST_SUFFIX}"));
    let reply_path = dir.join(format!("{name}{REPLY_SUFFIX}"));
    write_json(
        &request_path,
        &ControlRequest {
            protocol: PROTOCOL,
            action,
        },
        true,
    )?;
    let deadline = Instant::now().checked_add(action.timeout());
    loop {
        if let Some(reply) = take_reply(&reply_path)? {
            return reply;
        }
        let ended = table.read(id)?.filter(|file| !is_live(file.record.state));
        let cancelled = ctx.cancel().check();
        let expired = deadline.is_some_and(|deadline| Instant::now() >= deadline);
        if ended.is_some() || cancelled.is_err() || expired {
            let withdrawn = withdraw(&request_path)?;
            if let Some(reply) = take_reply(&reply_path)? {
                return reply;
            }
            let answer_follows = !withdrawn
                && ended
                    .as_ref()
                    .is_some_and(|file| file.record.state != JobState::Lost);
            if answer_follows && cancelled.is_ok() && !expired {
                std::thread::sleep(CONTROL_POLL);
                continue;
            }
            cancelled?;
            if let Some(file) = ended {
                check_action(id, file.record.state, action.action())?;
            }
            return Err(Error::new(
                ErrorCode::Timeout,
                format!(
                    "the supervisor of job `{id}` did not answer; read its state with `jobs \"{id}\"`"
                ),
            ));
        }
        std::thread::sleep(CONTROL_POLL);
    }
}

/// Reads and removes the reply at `path`, when one was written.
fn take_reply(path: &Path) -> Result<Option<Result<Job>>> {
    let Some(reply) = read_json::<ControlReply>(path)? else {
        return Ok(None);
    };
    remove_file(path)?;
    Ok(Some(match reply {
        ControlReply::Done(job) => Ok(*job),
        ControlReply::Failed(error) => Err(error),
    }))
}

/// Removes a request file; `false` when the other side removed it first.
pub(crate) fn withdraw(path: &Path) -> Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(&format!("remove {}", path.display()), &error)),
    }
}

/// Writes a request to job `id` that needs no reply.
///
/// # Errors
///
/// Fails with `E_IO` when the request cannot be written.
pub(crate) fn post(table: &JobTable, id: &str, action: ControlAction) -> Result<()> {
    let dir = table.control_dir(id);
    create_private_dir(&dir)?;
    let path = dir.join(format!("{}{REQUEST_SUFFIX}", request_name()));
    write_json(
        &path,
        &ControlRequest {
            protocol: PROTOCOL,
            action,
        },
        true,
    )
}

/// Takes the pending requests of job `id`, oldest first. A request withdrawn
/// by its requester before it was removed here is not taken.
pub(crate) fn pending(table: &JobTable, id: &str) -> Vec<(String, ControlAction)> {
    let Ok(entries) = fs::read_dir(table.control_dir(id)) else {
        return Vec::new();
    };
    let mut requests: Vec<(String, ControlAction)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_name = entry.file_name().into_string().ok()?;
            let name = file_name.strip_suffix(REQUEST_SUFFIX)?.to_owned();
            let request = read_json::<ControlRequest>(&entry.path()).ok().flatten()?;
            fs::remove_file(entry.path()).ok()?;
            (request.protocol == PROTOCOL).then_some((name, request.action))
        })
        .collect();
    requests.sort_by_cached_key(|(name, _)| (request_order(name), name.clone()));
    requests
}

/// Answers request `name` of job `id`.
pub(crate) fn reply(table: &JobTable, id: &str, name: &str, result: Result<Job>) {
    let reply = match result {
        Ok(job) => ControlReply::Done(Box::new(job)),
        Err(error) => ControlReply::Failed(error),
    };
    let path = table.control_dir(id).join(format!("{name}{REPLY_SUFFIX}"));
    let _ = write_json(&path, &reply, true);
}
