// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-service/tests/scenarios.rs
// Responsibility:   End-to-end scenarios of the background service on the running system.
// Dependencies:     nurt_contracts, nurt_sched, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! End-to-end scenarios of the background service on the running system.
//!
//! Every scenario is ignored by default and checks its precondition before it
//! changes the system; a scenario whose precondition does not hold returns
//! without assertions. Scenarios that install the service register the
//! program named by the environment variable `NURT_IT_NURT`, refuse to touch
//! an existing installation of the account and remove their installation in
//! a guard, also when an assertion fails. Scenarios that need administrator
//! rights run only when `NURT_IT_PRIVILEGED` is `1`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "an integration test reports failures by panicking"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use nurt_contracts::config::{Config, ServiceMode};
use nurt_contracts::services::ProcessHost;
use nurt_service::status::{ServiceStatus, StatusRequest, status};
use nurt_service::{
    InstallReport, InstallRequest, ServicePaths, UninstallReport, UninstallRequest, install,
    uninstall,
};

/// Longest wait for the service manager to start the service.
const START_PATIENCE: Duration = Duration::from_secs(60);

/// Interval between two readings of the service status.
const POLL: Duration = Duration::from_millis(500);

/// Program registered as the service, from `NURT_IT_NURT`.
fn nurt_program() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("NURT_IT_NURT")?);
    path.is_file().then_some(path)
}

/// The run may change the configuration of the system.
#[cfg(any(target_os = "linux", windows))]
fn privileged() -> bool {
    std::env::var_os("NURT_IT_PRIVILEGED").is_some_and(|value| value == "1")
}

fn status_of(paths: &ServicePaths) -> ServiceStatus {
    status(&StatusRequest::new(paths.clone(), &Config::default())).unwrap()
}

/// Waits until the service answers; panics after [`START_PATIENCE`].
fn wait_running(paths: &ServicePaths) -> ServiceStatus {
    let deadline = Instant::now() + START_PATIENCE;
    loop {
        let current = status_of(paths);
        if current.running {
            return current;
        }
        assert!(
            Instant::now() < deadline,
            "the service did not answer within {} s",
            START_PATIENCE.as_secs()
        );
        thread::sleep(POLL);
    }
}

/// Installation of the service made by a scenario, removed on drop.
struct Installation {
    paths: ServicePaths,
    process_host: Option<Arc<dyn ProcessHost>>,
    report: InstallReport,
    removed: bool,
}

impl Installation {
    /// Installs the service in `mode`; `None` when the account already has
    /// an installation.
    fn install(
        mode: ServiceMode,
        program: PathBuf,
        account: Option<String>,
        process_host: Option<Arc<dyn ProcessHost>>,
    ) -> Option<Self> {
        let paths = ServicePaths::from_config(&Config::default()).ok()?;
        if paths.install_manifest().exists() {
            return None;
        }
        let mut request = InstallRequest::new(paths.clone());
        request.mode = Some(mode);
        request.executable = Some(program);
        request.account = account;
        request.process_host = process_host.clone();
        let report = install(&request).unwrap();
        Some(Self {
            paths,
            process_host,
            report,
            removed: false,
        })
    }

    fn request(&self) -> UninstallRequest {
        UninstallRequest {
            mode: None,
            paths: self.paths.clone(),
            process_host: self.process_host.clone(),
        }
    }

    /// Removes the installation and returns the report of the removal.
    fn remove(mut self) -> UninstallReport {
        self.removed = true;
        uninstall(&self.request()).unwrap()
    }
}

impl Drop for Installation {
    fn drop(&mut self) {
        if !self.removed {
            let _ = uninstall(&self.request());
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeSet;
    use std::fs;
    use std::ops::RangeInclusive;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::mpsc::{self, Sender};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use nurt_contracts::config::{Config, ServiceMode};
    use nurt_contracts::error::Error;
    use nurt_contracts::record::TriggerKind;
    use nurt_contracts::stage::{CancelReason, CancelToken};
    use nurt_contracts::value::{Duration as ValueDuration, Instant as ValueInstant};
    use nurt_sched::action::args_hash;
    use nurt_sched::store::{Definition, FORMAT, OnMissed, Overlap, Store, encode_definition};
    use nurt_sched::trigger::{TriggerDef, format_duration};
    use nurt_service::ipc::client::ControlClient;
    use nurt_service::module::ServiceModule;
    use nurt_service::{Backend, ScheduleService, ServicePaths, ServiceSetup, run};
    use serde_json::json;

    use super::{Installation, nurt_program, privileged, status_of, wait_running};

    /// Directory that exists when systemd runs as the service manager.
    const SYSTEMD_RUN_DIR: &str = "/run/systemd/system";

    /// Interval of the schedule of the ten-minute scenario.
    const INTERVAL: Duration = Duration::from_secs(30);

    /// Length of the ten-minute scenario.
    const RUN_TIME: Duration = Duration::from_secs(600);

    /// Time of the restart of the service in the ten-minute scenario.
    const RESTART_AT: Duration = Duration::from_secs(300);

    /// Runs expected in [`RUN_TIME`] at [`INTERVAL`], with one run time of
    /// tolerance on each side for the start and the restart.
    const EXPECTED_RUNS: RangeInclusive<usize> = 19..=21;

    /// Shortest distance of two runs; closer runs overlap or repeat a run
    /// time.
    const MIN_GAP: Duration = Duration::from_secs(25);

    /// Remote action kind answered by the test console.
    const ACTION_KIND: &str = "session_message";

    /// Delay after which a missed run time is skipped.
    const MISFIRE_MS: i64 = 120_000;

    /// Time limit of one run.
    const TIMEOUT_MS: i64 = 60_000;

    /// Longest wait for the service host to answer after a start.
    const HOST_PATIENCE: Duration = Duration::from_secs(10);

    /// Interval between two connection attempts to the service host.
    const HOST_POLL: Duration = Duration::from_millis(20);

    fn systemd_runs() -> bool {
        Path::new(SYSTEMD_RUN_DIR).is_dir()
    }

    /// Effective user identifier of the test process.
    fn effective_uid() -> Option<u32> {
        let text = fs::read_to_string("/proc/self/status").ok()?;
        let line = text.lines().find(|l| l.starts_with("Uid:"))?;
        line.split_whitespace().nth(2)?.parse().ok()
    }

    fn is_root() -> bool {
        effective_uid() == Some(0)
    }

    /// Control group of a process.
    fn cgroup_of(pid: u32) -> String {
        fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap()
    }

    #[test]
    #[ignore = "requires systemd with a user manager, lingering and the program named by NURT_IT_NURT"]
    fn e2e_linux_user_mode_linger_survives_logout() {
        let Some(program) = nurt_program() else {
            return;
        };
        if !systemd_runs() || std::env::var_os("XDG_RUNTIME_DIR").is_none() || is_root() {
            return;
        }
        let Some(installation) = Installation::install(ServiceMode::User, program, None, None)
        else {
            return;
        };
        assert_eq!(installation.report.backend, Backend::SystemdUser);
        assert_eq!(installation.report.linger, Some(true));
        let paths = installation.paths.clone();
        let running = wait_running(&paths);
        assert_eq!(running.linger, Some(true));
        let pid = running.pid.expect("a running service reports its process");
        let cgroup = cgroup_of(pid);
        assert!(
            cgroup.contains("/user@"),
            "the user manager runs the service: {cgroup}"
        );
        assert!(
            !cgroup.contains("/session-"),
            "the service runs outside every login session and survives its end: {cgroup}"
        );
        let removed = installation.remove();
        assert_eq!(removed.backend, Backend::SystemdUser);
        assert!(!status_of(&paths).installed);
    }

    #[test]
    #[ignore = "requires systemd, root with NURT_IT_PRIVILEGED=1 and the program named by NURT_IT_NURT"]
    fn e2e_linux_system_mode_service_runs() {
        let Some(program) = nurt_program() else {
            return;
        };
        if !systemd_runs() || !privileged() || !is_root() {
            return;
        }
        let account = std::env::var("SUDO_USER").ok();
        let Some(installation) = Installation::install(ServiceMode::System, program, account, None)
        else {
            return;
        };
        assert_eq!(installation.report.backend, Backend::SystemdSystem);
        let paths = installation.paths.clone();
        let running = wait_running(&paths);
        assert_eq!(running.mode, Some(ServiceMode::System));
        let pid = running.pid.expect("a running service reports its process");
        assert!(cgroup_of(pid).contains("/system.slice/"));
        let removed = installation.remove();
        assert_eq!(removed.backend, Backend::SystemdSystem);
        assert!(!status_of(&paths).installed);
    }

    /// Schedule `id` that fires the remote action every [`INTERVAL`] from
    /// now.
    fn every_interval(id: &str) -> Definition {
        let created: ValueInstant = jiff::Timestamp::now()
            .round(jiff::Unit::Second)
            .unwrap()
            .to_string()
            .parse()
            .unwrap();
        let arguments = json!({ "text": "scenario" });
        Definition {
            format: FORMAT.to_owned(),
            id: id.to_owned(),
            label: "remote action every 30 seconds".to_owned(),
            trigger: TriggerDef {
                kind: TriggerKind::Every,
                expression: format_duration(INTERVAL),
                timezone: "UTC".to_owned(),
            },
            action_kind: ACTION_KIND.to_owned(),
            pipeline: None,
            ast_hash: args_hash(ACTION_KIND, Some(&arguments)).unwrap(),
            action_args: Some(arguments),
            action: ACTION_KIND.to_owned(),
            on_missed: OnMissed::Skip,
            misfire: ValueDuration::from_millis(MISFIRE_MS),
            overlap: Overlap::Skip,
            timeout: ValueDuration::from_millis(TIMEOUT_MS),
            max_runs: None,
            expires: None,
            paused: false,
            created,
            changed: created,
            session: None,
            base: None,
            project_roots: Vec::new(),
        }
    }

    fn write_definition(store: &Store, definition: &Definition) {
        fs::create_dir_all(store.definitions_dir()).unwrap();
        fs::write(
            store.definition_path(&definition.id),
            encode_definition(definition).unwrap(),
        )
        .unwrap();
    }

    /// Service host with the schedule module, running on its own thread.
    struct Host {
        stop: CancelToken,
        thread: Option<JoinHandle<Result<(), Error>>>,
    }

    impl Host {
        /// Starts the host on `paths` and waits until the schedule module
        /// runs.
        fn start(paths: &ServicePaths) -> Self {
            let setup =
                ServiceSetup::with_paths(Config::default(), paths.clone()).without_signals();
            let stop = setup.stop();
            let modules: Vec<Arc<dyn ServiceModule>> = vec![Arc::new(ScheduleService::new())];
            let host = Self {
                stop,
                thread: Some(thread::spawn(move || run(setup, modules))),
            };
            let deadline = Instant::now() + HOST_PATIENCE;
            while !schedule_module_runs(paths) {
                assert!(Instant::now() < deadline, "the service host did not start");
                thread::sleep(HOST_POLL);
            }
            host
        }

        /// Stops the host as the service manager does at a restart.
        fn stop(mut self) {
            self.stop.cancel(CancelReason::Shutdown);
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap().unwrap();
            }
        }
    }

    impl Drop for Host {
        fn drop(&mut self) {
            self.stop.cancel(CancelReason::Shutdown);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn schedule_module_runs(paths: &ServicePaths) -> bool {
        let Ok(mut client) = ControlClient::connect(paths) else {
            return false;
        };
        client
            .call("status", json!({}))
            .ok()
            .and_then(Result::ok)
            .is_some_and(|status| status["modules"]["schedule"]["state"] == "running")
    }

    /// Console that registers [`ACTION_KIND`], acknowledges every fired run
    /// and reports its token and arrival; it ends when the host stops.
    fn console(paths: &ServicePaths, fired: Sender<(String, Instant)>) -> JoinHandle<()> {
        let mut client = ControlClient::connect(paths).unwrap();
        client
            .call(
                "schedule/register_action",
                json!({ "kind": ACTION_KIND, "description": "scenario console" }),
            )
            .unwrap()
            .unwrap();
        thread::spawn(move || {
            while let Ok((event, data)) = client.next_event() {
                if event != "schedule.fired" {
                    continue;
                }
                let Some(token) = data["ack_token"].as_str().map(str::to_owned) else {
                    continue;
                };
                let acknowledged =
                    client.call("schedule/ack", json!({ "token": token, "state": "ok" }));
                if acknowledged.is_err() || fired.send((token, Instant::now())).is_err() {
                    break;
                }
            }
        })
    }

    #[test]
    #[ignore = "requires ten minutes of wall-clock time"]
    fn e2e_linux_schedule_every_30s_10min_with_service_restart() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ServicePaths::new(dir.path().join("state"));
        let store = Store::new(paths.state_dir());
        write_definition(&store, &every_interval("s1"));
        let (sender, receiver) = mpsc::channel();
        let begin = Instant::now();

        let first = Host::start(&paths);
        let first_console = console(&paths, sender.clone());
        thread::sleep(RESTART_AT.saturating_sub(begin.elapsed()));
        first.stop();
        first_console.join().unwrap();

        let second = Host::start(&paths);
        let second_console = console(&paths, sender);
        thread::sleep(RUN_TIME.saturating_sub(begin.elapsed()));
        second.stop();
        second_console.join().unwrap();

        let fired: Vec<(String, Instant)> = receiver.try_iter().collect();
        assert!(
            EXPECTED_RUNS.contains(&fired.len()),
            "{} runs in ten minutes",
            fired.len()
        );
        let tokens: BTreeSet<&str> = fired.iter().map(|(token, _)| token.as_str()).collect();
        assert_eq!(tokens.len(), fired.len(), "no run fired twice");
        for pair in fired.windows(2) {
            assert!(
                pair[1].1.duration_since(pair[0].1) >= MIN_GAP,
                "two runs closer than {} s",
                MIN_GAP.as_secs()
            );
        }

        let state = store.read_state().unwrap();
        let count = u64::try_from(fired.len()).unwrap();
        assert_eq!(state.schedules["s1"].run_count, count);
        let runs: Vec<u64> = state.history["s1"]
            .iter()
            .filter(|record| record.reason.is_none())
            .map(|record| record.run)
            .collect();
        assert_eq!(runs.len(), fired.len(), "the history lists every run");
        assert!(
            runs.windows(2).all(|pair| pair[0] < pair[1]),
            "run numbers increase without repetition: {runs:?}"
        );
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;

    use nurt_contracts::config::ServiceMode;
    use nurt_contracts::error::{Error, ErrorCode, Result};
    use nurt_contracts::services::{
        ProcessHost, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec,
    };
    use nurt_contracts::stage::CancelToken;
    use nurt_service::Backend;

    use super::{Installation, nurt_program, status_of, wait_running};

    /// Location of `launchctl` on every macOS release.
    const LAUNCHCTL: &str = "/bin/launchctl";

    /// Process host of the scenario: runs `launchctl` for the installer.
    struct CommandHost;

    impl ProcessHost for CommandHost {
        fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
            let path = PathBuf::from(program);
            if path.is_file() {
                Ok(path)
            } else {
                Err(Error::new(
                    ErrorCode::ExternalMissing,
                    format!("{program} is not available"),
                ))
            }
        }

        fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
            let output = Command::new(&spec.program)
                .args(&spec.args)
                .current_dir(&spec.cwd)
                .output()
                .map_err(|error| Error::new(ErrorCode::ExternalMissing, error.to_string()))?;
            Ok(ProgramOutcome {
                program_path: PathBuf::from(&spec.program),
                exit_code: output.status.code(),
                stdout: output.stdout,
                stderr: output.stderr,
                ..ProgramOutcome::default()
            })
        }

        fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
            Err(Error::new(
                ErrorCode::Unsupported,
                "the scenario host starts no services",
            ))
        }
    }

    #[test]
    #[ignore = "requires macOS with a logged-in user session and the program named by NURT_IT_NURT"]
    fn e2e_macos_user_mode_launch_agent_install_status_uninstall() {
        let Some(program) = nurt_program() else {
            return;
        };
        if !Path::new(LAUNCHCTL).is_file() {
            return;
        }
        let host: Arc<dyn ProcessHost> = Arc::new(CommandHost);
        let Some(installation) =
            Installation::install(ServiceMode::User, program, None, Some(host))
        else {
            return;
        };
        assert_eq!(installation.report.backend, Backend::LaunchAgent);
        let paths = installation.paths.clone();
        let running = wait_running(&paths);
        assert!(running.installed);
        assert_eq!(running.mode, Some(ServiceMode::User));
        let removed = installation.remove();
        assert_eq!(removed.backend, Backend::LaunchAgent);
        assert!(removed.removed.iter().all(|file| !file.exists()));
        assert!(!status_of(&paths).installed);
    }
}

#[cfg(windows)]
mod windows {
    use nurt_contracts::config::ServiceMode;
    use nurt_service::{Backend, Manifest};

    use super::{Installation, nurt_program, privileged, status_of, wait_running};

    #[test]
    #[ignore = "requires administrator rights with NURT_IT_PRIVILEGED=1 and the program named by NURT_IT_NURT"]
    fn e2e_windows_system_mode_service_runs() {
        let Some(program) = nurt_program() else {
            return;
        };
        if !privileged() {
            return;
        }
        let Some(installation) = Installation::install(ServiceMode::System, program, None, None)
        else {
            return;
        };
        assert_eq!(installation.report.backend, Backend::WindowsService);
        let paths = installation.paths.clone();
        let running = wait_running(&paths);
        assert_eq!(running.mode, Some(ServiceMode::System));
        let removed = installation.remove();
        assert_eq!(removed.backend, Backend::WindowsService);
        assert!(!status_of(&paths).installed);
    }

    #[test]
    #[ignore = "requires an interactive Windows session and the program named by NURT_IT_NURT"]
    fn it_windows_user_mode_task_scheduler() {
        let Some(program) = nurt_program() else {
            return;
        };
        let Some(installation) = Installation::install(ServiceMode::User, program, None, None)
        else {
            return;
        };
        assert_eq!(installation.report.backend, Backend::TaskScheduler);
        let paths = installation.paths.clone();
        let manifest = Manifest::read(&paths.install_manifest()).unwrap().unwrap();
        assert_eq!(manifest.backend, Backend::TaskScheduler);
        assert_eq!(manifest.mode, ServiceMode::User);
        let removed = installation.remove();
        assert_eq!(removed.backend, Backend::TaskScheduler);
        assert!(Manifest::read(&paths.install_manifest()).unwrap().is_none());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
