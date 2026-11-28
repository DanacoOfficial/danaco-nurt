// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/core/app.rs
// Responsibility:   Process level of the setup: startup facts, elevation, relaunch, background runs and the elevated worker.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Process level of the setup.
//!
//! The wizard runs as the user. The runs of the scope for all users go to a copy of this
//! program that the system starts elevated (`--worker <job>`): it reads the job, runs the
//! engine and appends the events as JSON lines to a file that the wizard follows. The
//! window and the splash never run elevated.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use crate::cli::{self, Cli};
use crate::engine::{self, Env, UndoEnv};
use crate::error::{SetupError, SetupResult};
use crate::model::{
    Existing, FinishEvent, Job, JobKind, Options, ProgressEvent, Scope, UserPaths, WorkerEvent,
};
use crate::shortcuts::CREATE_NO_WINDOW;
use crate::{logfile, paths, payload, registry, sys};

/// Version of the program that this setup installs.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Rust target of this setup.
pub(crate) const TARGET: &str = if cfg!(target_arch = "aarch64") {
    "aarch64-pc-windows-msvc"
} else {
    "x86_64-pc-windows-msvc"
};

/// Flag of `CreateProcess`: detached from the console of the parent.
const DETACHED_PROCESS: u32 = 0x0000_0008;

/// Facts that the process establishes before any screen opens.
#[derive(Debug, Clone)]
pub(crate) struct Startup {
    /// The command line.
    pub(crate) cli: Cli,
    /// Directories of the user.
    pub(crate) user: UserPaths,
    /// The process has administrator rights.
    pub(crate) elevated: bool,
    /// An installation of the setup that is on this machine.
    pub(crate) existing: Option<Existing>,
    /// State of a wizard that elevated itself: the choices made so far.
    pub(crate) resume: Option<Options>,
    /// Animations are enabled in Windows.
    pub(crate) animations: bool,
}

/// Prepares the startup facts from the command line.
pub(crate) fn startup(cli: Cli) -> Startup {
    let resume = cli.resume.as_ref().and_then(|path| {
        let text = fs::read_to_string(path).ok();
        let _ = fs::remove_file(path);
        text.and_then(|text| serde_json::from_str::<Options>(&text).ok())
    });
    Startup {
        user: UserPaths::current(),
        elevated: sys::is_elevated(),
        existing: find_existing(),
        resume,
        animations: sys::animations_enabled(),
        cli,
    }
}

/// The installation that the setup made, found through "Installed apps" and its journal.
pub(crate) fn find_existing() -> Option<Existing> {
    let (scope, root, version) = registry::find_installation()?;
    let (entries, complete) = engine::inspect(&root);
    Some(Existing {
        version,
        root,
        scope,
        entries,
        journal_ok: entries > 0 && complete,
    })
}

/// Arguments of this process after the program name.
pub(crate) fn own_arguments() -> Vec<String> {
    std::env::args().skip(1).collect()
}

/// Joins arguments into a command line.
pub(crate) fn command_line(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| cli::quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Starts this program again with the elevation prompt; `wait` returns its exit code.
pub(crate) fn relaunch_elevated(arguments: &[String], wait: bool) -> SetupResult<Option<u32>> {
    let exe = std::env::current_exe()
        .map_err(|error| SetupError::new("elevation-failed", error.to_string()))?;
    let mut full = arguments.to_vec();
    if !full.iter().any(|argument| argument == "--elevated") {
        full.push("--elevated".to_string());
    }
    sys::run_elevated(&exe, &command_line(&full), wait).map_err(|code| {
        if code == 1223 {
            SetupError::new("elevation-declined", "")
        } else {
            SetupError::new("elevation-failed", format!("error {code}"))
        }
    })
}

/// This program runs from the root of an installation, so it cannot replace or delete itself.
pub(crate) fn runs_from_installation(existing: &Existing) -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .is_some_and(|parent| {
            parent
                .to_string_lossy()
                .eq_ignore_ascii_case(&existing.root.to_string_lossy())
        })
}

/// Starts a copy of this program in the temporary directory, so that the original can be changed.
/// The copy works in the temporary directory: a shortcut opens the original in the root of the
/// installation, and a process whose working directory is that root keeps Windows from deleting it.
pub(crate) fn relaunch_from_temp(arguments: &[String], wait: bool) -> SetupResult<Option<i32>> {
    let directory = paths::temp().join(format!("nurt-setup-copy-{}", std::process::id()));
    fs::create_dir_all(&directory).map_err(|error| SetupError::io(&directory, &error))?;
    let copy = directory.join("uninstall.exe");
    payload::copy_self(&copy).map_err(|error| SetupError::io(&copy, &error))?;
    let mut full = arguments.to_vec();
    full.push("--from-temp".to_string());
    full.push(directory.display().to_string());
    let mut child = Command::new(&copy)
        .args(&full)
        .current_dir(paths::temp())
        .spawn()
        .map_err(|error| SetupError::io(&copy, &error))?;
    if wait {
        let status = child
            .wait()
            .map_err(|error| SetupError::io(&copy, &error))?;
        return Ok(status.code());
    }
    Ok(None)
}

/// Runs a command line of `cmd.exe` in the background after a pause. The line is passed as it is: the
/// quoting of the arguments of Rust would put a backslash before every quote and `cmd.exe` would not
/// read the paths that follow. `cmd.exe` works in the temporary directory, outside the directories
/// that it deletes.
fn run_after_pause(seconds: u32, commands: &str) {
    let line = format!("/D /C \"ping -n {seconds} 127.0.0.1 >nul & {commands}\"");
    let _ = Command::new("cmd.exe")
        .raw_arg(line)
        .current_dir(paths::temp())
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn();
}

/// Deletes directories after this process has ended.
pub(crate) fn delete_after_exit(directories: &[PathBuf]) {
    let commands: Vec<String> = directories
        .iter()
        .map(|directory| format!("rmdir /S /Q \"{}\"", directory.display()))
        .collect();
    run_after_pause(3, &commands.join(" & "));
}

/// Deletes a file and then its folder (when it is empty) after the program that holds the file has ended.
pub(crate) fn delete_file_after_exit(file: &Path) {
    let mut commands = format!("del /F /Q \"{}\"", file.display());
    if let Some(folder) = file.parent() {
        commands.push_str(&format!(" & rmdir \"{}\"", folder.display()));
    }
    run_after_pause(5, &commands);
}

/// Starts a program with the rights of the user, also when this process is elevated: the shell does it.
pub(crate) fn launch_with_shell(target: &Path) {
    let _ = Command::new("explorer.exe").arg(target).spawn();
}

// ------------------------------------------------------------------------------------------
// Runs in the background

/// A run in the background that the interface follows.
pub(crate) struct Runner {
    /// Events of the run.
    pub(crate) events: Receiver<WorkerEvent>,
    cancel: Arc<AtomicBool>,
    cancel_file: Option<PathBuf>,
}

impl Runner {
    /// Asks the run to stop; the engine undoes what it did.
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(file) = &self.cancel_file {
            let _ = fs::write(file, b"cancel");
        }
    }
}

fn spawn_local(job: Job, user: UserPaths, repaint: impl Fn() + Send + 'static) -> Runner {
    let (sender, receiver) = mpsc::channel::<WorkerEvent>();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    thread::spawn(move || {
        let finish = execute(
            &job,
            &user,
            true,
            &|event| {
                let _ = sender.send(WorkerEvent::Progress(event));
                repaint();
            },
            &|| flag.load(Ordering::SeqCst),
        );
        let _ = sender.send(WorkerEvent::Finish(finish));
        repaint();
    });
    Runner {
        events: receiver,
        cancel,
        cancel_file: None,
    }
}

/// Runs a job in this process; returns the event that ends it.
pub(crate) fn execute(
    job: &Job,
    user: &UserPaths,
    pace: bool,
    emit: &dyn Fn(ProgressEvent),
    cancel: &dyn Fn() -> bool,
) -> FinishEvent {
    match job.kind {
        JobKind::Install => {
            let env = Env {
                options: &job.options,
                user,
                version: VERSION,
                emit,
                cancel,
                pace,
                profile_runs: &crate::terminal::profile_runs,
            };
            match engine::install(&env) {
                Ok(summary) => FinishEvent {
                    ok: true,
                    cancelled: false,
                    code: String::new(),
                    detail: String::new(),
                    stage: String::new(),
                    rolled_back: false,
                    undone: 0,
                    summary: Some(summary),
                },
                Err(failure) => failure.event(),
            }
        }
        JobKind::Undo => {
            let env = UndoEnv {
                root: &job.root,
                user,
                remove_config: job.options.write_config,
                emit,
                pace,
            };
            match engine::undo(&env) {
                Ok(summary) => FinishEvent {
                    ok: true,
                    cancelled: false,
                    code: String::new(),
                    detail: String::new(),
                    stage: String::new(),
                    rolled_back: false,
                    undone: summary.undone.len() as u64,
                    summary: Some(summary),
                },
                Err(error) => FinishEvent {
                    ok: false,
                    cancelled: false,
                    code: error.code.to_string(),
                    detail: error.detail,
                    stage: "read".to_string(),
                    rolled_back: false,
                    undone: 0,
                    summary: None,
                },
            }
        }
    }
}

fn job_files() -> (PathBuf, PathBuf, PathBuf) {
    let directory = paths::temp().join(format!("nurt-setup-run-{}", std::process::id()));
    let _ = fs::create_dir_all(&directory);
    (
        directory.join("job.json"),
        directory.join("events.jsonl"),
        directory.join("cancel"),
    )
}

fn spawn_elevated(job: Job, repaint: impl Fn() + Send + 'static) -> Runner {
    let (sender, receiver) = mpsc::channel::<WorkerEvent>();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_file = job.cancel.clone();
    let events_file = job.events.clone();
    let job_file = events_file.with_file_name("job.json");
    thread::spawn(move || {
        let text = serde_json::to_string(&job).unwrap_or_default();
        let _ = fs::write(&events_file, b"");
        if let Err(error) = fs::write(&job_file, text) {
            let _ = sender.send(WorkerEvent::Finish(failure(
                "elevation-failed",
                error.to_string(),
            )));
            repaint();
            return;
        }
        let exited = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&exited);
        let arguments = vec!["--worker".to_string(), job_file.display().to_string()];
        let waiter = thread::spawn(move || {
            let result = relaunch_elevated(&arguments, true);
            flag.store(true, Ordering::SeqCst);
            result
        });
        let mut offset = 0usize;
        let mut finished = false;
        loop {
            let text = fs::read_to_string(&events_file).unwrap_or_default();
            let complete = &text[..text.rfind('\n').map_or(0, |index| index + 1)];
            if complete.len() > offset {
                for line in complete[offset..].lines() {
                    if let Ok(event) = serde_json::from_str::<WorkerEvent>(line) {
                        finished |= matches!(event, WorkerEvent::Finish(_));
                        let _ = sender.send(event);
                        repaint();
                    }
                }
                offset = complete.len();
            }
            if finished {
                break;
            }
            if exited.load(Ordering::SeqCst) {
                let again = fs::read_to_string(&events_file).unwrap_or_default();
                if again.len() == text.len() {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(60));
        }
        if !finished {
            let reason = match waiter.join() {
                Ok(Err(error)) => error,
                _ => SetupError::new(
                    "worker-exited",
                    "the elevated process ended before it reported a result",
                ),
            };
            let _ = sender.send(WorkerEvent::Finish(failure(reason.code, reason.detail)));
            repaint();
        }
        if let Some(directory) = events_file.parent() {
            let _ = fs::remove_dir_all(directory);
        }
    });
    Runner {
        events: receiver,
        cancel,
        cancel_file: Some(cancel_file),
    }
}

fn failure(code: &str, detail: String) -> FinishEvent {
    FinishEvent {
        ok: false,
        cancelled: false,
        code: code.to_string(),
        detail,
        stage: String::new(),
        rolled_back: true,
        undone: 0,
        summary: None,
    }
}

/// Starts an installation (or a repair) in the background.
pub(crate) fn start_install(
    options: Options,
    user: UserPaths,
    repaint: impl Fn() + Send + 'static,
) -> Runner {
    start(JobKind::Install, options, user, PathBuf::new(), repaint)
}

/// Starts the undo of an installation in the background.
pub(crate) fn start_undo(
    existing: &Existing,
    remove_config: bool,
    user: UserPaths,
    repaint: impl Fn() + Send + 'static,
) -> Runner {
    let options = Options {
        scope: existing.scope,
        write_config: remove_config,
        ..Options::default()
    };
    start(JobKind::Undo, options, user, existing.root.clone(), repaint)
}

fn start(
    kind: JobKind,
    options: Options,
    user: UserPaths,
    root: PathBuf,
    repaint: impl Fn() + Send + 'static,
) -> Runner {
    let (_, events, cancel) = job_files();
    let scope = options.scope;
    let job = Job {
        kind,
        options,
        user: user.clone(),
        root,
        scope,
        events,
        cancel,
    };
    // NURT_SETUP_FORCE_WORKER=1 sends the run to the elevated worker also from an elevated process (used to test that path).
    let force_worker = std::env::var("NURT_SETUP_FORCE_WORKER").is_ok_and(|value| value == "1");
    if scope == Scope::Machine && (force_worker || !sys::is_elevated()) {
        spawn_elevated(job, repaint)
    } else {
        spawn_local(job, user, repaint)
    }
}

// ------------------------------------------------------------------------------------------
// The elevated worker

/// Runs a job that the wizard handed over and writes its events to the file named in the job.
pub(crate) fn worker(job_file: &Path) -> i32 {
    let Ok(text) = fs::read_to_string(job_file) else {
        return 1;
    };
    let Ok(job) = serde_json::from_str::<Job>(&text) else {
        return 1;
    };
    let append = |event: &WorkerEvent| {
        if let Ok(line) = serde_json::to_string(event)
            && let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&job.events)
        {
            let _ = writeln!(file, "{line}");
        }
    };
    let cancel_file = job.cancel.clone();
    let finish = execute(
        &job,
        &job.user,
        true,
        &|event| append(&WorkerEvent::Progress(event)),
        &|| cancel_file.exists(),
    );
    let code = i32::from(!finish.ok);
    append(&WorkerEvent::Finish(finish));
    code
}

/// Logs a failure of the run.
pub(crate) fn log_finish(finish: &FinishEvent) {
    if finish.ok {
        logfile::line("run finished");
    } else {
        logfile::line(&format!(
            "run failed in {}: {} {}",
            finish.stage, finish.code, finish.detail
        ));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
