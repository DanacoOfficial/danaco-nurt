// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::jobs::host
// Responsibility:   Run hosts: runs supervised by this process and runs supervised by the.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Run hosts: runs supervised by this process and runs supervised by the
//! detached supervisor process.

use std::fs;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nurt_contracts::bound::BoundDocument;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::JobState;
use nurt_contracts::stage::ExecCtx;
use serde::{Deserialize, Serialize};

use super::SupervisorCommand;
use super::control::{self, request_name, request_order, withdraw};
use super::events::{self, OutputEvents};
use super::record::with_elapsed;
use super::runner::{self, Launch, POLL_INTERVAL};
use super::table::{JobTable, PROTOCOL, read_json, remove_file, write_json};
use super::wire::StoredRequest;
use crate::iface::{Components, OutputStore, RunHost, RunRequest, RunWait};

/// Time a client waits for the detached supervisor to accept a run.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// Time after which a supervisor that has not taken its lock is started
/// again.
const RELAUNCH_AFTER: Duration = Duration::from_secs(3);

const REQUEST_SUFFIX: &str = ".request.json";
const REPLY_SUFFIX: &str = ".reply.json";
const CANCEL_SUFFIX: &str = ".cancel.json";

/// Notice of a client that cancelled its call after the supervisor took the
/// start request and before the program started. The supervisor cancels the
/// preparation of the run named by the file.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CancelNotice {
    /// Format version.
    pub(crate) protocol: u32,
}

/// Removes the cancel notices left in the inbox by clients that ended before
/// they removed them. A supervisor that has taken the lock serves no
/// request yet, so every notice it finds belongs to a request that was
/// answered or lost.
pub(crate) fn remove_cancel_notices(table: &JobTable) {
    let Ok(entries) = fs::read_dir(table.inbox()) else {
        return;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        if entry.file_name().to_string_lossy().ends_with(CANCEL_SUFFIX) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Path of the cancel notice of start request `name`.
pub(crate) fn cancel_notice_path(table: &JobTable, name: &str) -> std::path::PathBuf {
    table.inbox().join(format!("{name}{CANCEL_SUFFIX}"))
}

/// Runs supervised by threads of this process.
pub(crate) struct LocalHost {
    components: Components,
    background: bool,
}

impl LocalHost {
    /// Host whose runs may move to the background when `background` is set;
    /// otherwise every run is waited for until it ends.
    pub(crate) fn new(components: Components, background: bool) -> Self {
        Self {
            components,
            background,
        }
    }
}

impl RunHost for LocalHost {
    fn run(&self, ctx: &ExecCtx, request: RunRequest, wait: Option<Duration>) -> Result<RunWait> {
        let wait = if self.background {
            wait
        } else if wait == Some(Duration::ZERO) {
            return Err(no_supervisor());
        } else {
            None
        };
        let table = JobTable::open(ctx)?;
        let launch = Launch::new(ctx, request, wait);
        let (waiter, _thread) = runner::start(ctx, &self.components, &table, launch)?;
        waiter.wait_outcome()
    }
}

fn no_supervisor() -> Error {
    Error::new(
        ErrorCode::Unsupported,
        "background runs need a supervisor; set `run.supervisor` to `auto` or run inside a long-lived process",
    )
    .with_help("run")
}

/// Request to start a run, read by the detached supervisor.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StartRequest {
    /// Format version.
    pub(crate) protocol: u32,
    /// Command that started the run.
    pub(crate) command_name: String,
    /// Bound program recorded in the journal entry.
    pub(crate) program: BoundDocument,
    /// The run, with its environment and standard input.
    pub(crate) request: StoredRequest,
    /// Time the run may take before it moves to the background.
    pub(crate) wait: Option<Duration>,
}

impl StartRequest {
    /// Launch of the run.
    pub(crate) fn into_launch(self) -> Launch {
        Launch {
            command_name: self.command_name,
            program: self.program,
            request: self.request.0,
            wait: self.wait,
        }
    }
}

/// Reply of the detached supervisor to a start request.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartReply {
    /// The program started as the job with this identifier.
    Started(String),
    /// The program did not start.
    Failed(Error),
}

/// Start requests waiting in the inbox: request name and path, oldest first.
pub(crate) fn inbox_requests(table: &JobTable) -> Result<Vec<(String, std::path::PathBuf)>> {
    let entries = fs::read_dir(table.inbox())
        .map_err(|error| super::table::io_error("read the supervisor inbox", &error))?;
    let mut requests: Vec<(String, std::path::PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_name = entry.file_name().into_string().ok()?;
            let name = file_name.strip_suffix(REQUEST_SUFFIX)?.to_owned();
            Some((name, entry.path()))
        })
        .collect();
    requests.sort_by_cached_key(|(name, _)| (request_order(name), name.clone()));
    Ok(requests)
}

/// Writes the reply to start request `name`.
///
/// # Errors
///
/// Fails with `E_IO` when the reply cannot be written.
pub(crate) fn write_start_reply(table: &JobTable, name: &str, reply: &StartReply) -> Result<()> {
    write_json(
        &table.inbox().join(format!("{name}{REPLY_SUFFIX}")),
        reply,
        true,
    )
}

/// Runs supervised by the detached supervisor process, which is started on
/// demand with `command`. A run waited for until its end needs no
/// supervisor and runs in this process.
pub(crate) struct DetachedHost {
    components: Components,
    command: SupervisorCommand,
}

impl DetachedHost {
    /// Host that starts the supervisor with `command`.
    pub(crate) fn new(components: Components, command: SupervisorCommand) -> Self {
        Self {
            components,
            command,
        }
    }

    fn submit(&self, ctx: &ExecCtx, table: &JobTable, launch: Launch) -> Result<String> {
        let name = request_name();
        let request_path = table.inbox().join(format!("{name}{REQUEST_SUFFIX}"));
        let reply_path = table.inbox().join(format!("{name}{REPLY_SUFFIX}"));
        let notice_path = cancel_notice_path(table, &name);
        let mut notified = false;
        let request = StartRequest {
            protocol: PROTOCOL,
            command_name: launch.command_name,
            program: launch.program,
            request: StoredRequest(launch.request),
            wait: launch.wait,
        };
        write_json(&request_path, &request, true)?;
        let begun = Instant::now();
        let mut launched: Option<Instant> = None;
        let mut answer_by: Option<Instant> = None;
        let mut unanswered = false;
        loop {
            if let Some(reply) = read_json::<StartReply>(&reply_path)? {
                remove_file(&reply_path)?;
                if notified {
                    remove_file(&notice_path)?;
                }
                return match reply {
                    StartReply::Started(id) => Ok(id),
                    StartReply::Failed(error) => Err(error),
                };
            }
            if unanswered {
                if notified {
                    remove_file(&notice_path)?;
                }
                return Err(Error::new(
                    ErrorCode::Spawn,
                    "the job supervisor took the run and did not answer; read the jobs with `jobs`",
                ));
            }
            let cancelled = ctx.cancel().check();
            if answer_by.is_none() && (cancelled.is_err() || begun.elapsed() >= START_TIMEOUT) {
                if withdraw(&request_path)? {
                    cancelled?;
                    return Err(Error::new(
                        ErrorCode::Spawn,
                        "the job supervisor did not accept the run; check the supervisor command of the engine",
                    ));
                }
                // The supervisor took the request before it was withdrawn
                // and has started the program or is starting it. The
                // program is not abandoned: `follow` cancels it when the
                // call is cancelled.
                answer_by = Some(Instant::now() + START_TIMEOUT);
            }
            if answer_by.is_some() && cancelled.is_err() && !notified {
                // The supervisor owns the request, so the withdrawal cannot
                // stop it; the notice reaches the preparation of the run
                // before the program exists.
                write_json(&notice_path, &CancelNotice { protocol: PROTOCOL }, true)?;
                notified = true;
            }
            if let Some(deadline) = answer_by {
                unanswered = Instant::now() >= deadline || !table.host_running()?;
            } else if launched.is_none_or(|at| at.elapsed() >= RELAUNCH_AFTER)
                && !table.host_running()?
            {
                self.launch_supervisor()?;
                launched = Some(Instant::now());
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    fn launch_supervisor(&self) -> Result<()> {
        let mut command = Command::new(&self.command.program);
        command
            .args(&self.command.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        detach(&mut command);
        let mut child = command.spawn().map_err(|error| {
            Error::new(
                ErrorCode::Spawn,
                format!(
                    "cannot start the job supervisor {}: {error}",
                    self.command.program.display()
                ),
            )
        })?;
        std::thread::spawn(move || child.wait());
        Ok(())
    }

    fn follow(
        &self,
        ctx: &ExecCtx,
        table: &JobTable,
        id: &str,
        progress: Option<String>,
    ) -> Result<RunWait> {
        let store: Option<std::sync::Arc<dyn OutputStore>> =
            self.components.spool.open(ctx, id).ok();
        let mut output = OutputEvents::new(id, progress);
        let mut announced = false;
        let mut cancel_sent = false;
        loop {
            let file = table.get(id)?;
            if !announced {
                ctx.emit(events::started(&file.record));
                announced = true;
            }
            if let Some(store) = &store {
                output.pump(ctx, store.as_ref(), file.result.is_some());
            }
            if file.detached || file.record.state == JobState::Lost {
                let record = with_elapsed(&file.record, ctx.clock().now());
                return Ok(RunWait::Background(Box::new(record)));
            }
            if let Some(result) = file.result {
                ctx.emit(events::finished(&file.record));
                return Ok(RunWait::Finished(Box::new(result.0)));
            }
            if !cancel_sent && ctx.cancel().is_cancelled() {
                cancel_sent = true;
                control::cancel(table, id)?;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

impl RunHost for DetachedHost {
    fn run(&self, ctx: &ExecCtx, request: RunRequest, wait: Option<Duration>) -> Result<RunWait> {
        if wait.is_none() {
            return LocalHost::new(self.components.clone(), false).run(ctx, request, None);
        }
        let table = JobTable::open(ctx)?;
        let progress = request.view.progress.clone();
        let id = self.submit(ctx, &table, Launch::new(ctx, request, wait))?;
        self.follow(ctx, &table, &id, progress)
    }
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    std::os::unix::process::CommandExt::process_group(command, 0);
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::os::windows::process::CommandExt::creation_flags(
        command,
        DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW,
    );
}

#[cfg(not(any(unix, windows)))]
fn detach(_command: &mut Command) {}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
