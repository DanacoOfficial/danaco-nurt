// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::host
// Responsibility:   The service process: instance lock, control channel, heartbeat and module supervision.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The service process: instance lock, control channel, heartbeat and module
//! supervision.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use nurt_contracts::config::Config;
use nurt_contracts::env::{Clock, WallTime};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::stage::{CancelReason, CancelToken, Extensions};
use serde_json::{Map, Value, json};

use crate::clock::SystemClock;
use crate::heartbeat::{self, HeartbeatWriter, ModuleState};
use crate::hub::Hub;
use crate::ipc::{self, Control, RouteError, Server, protocol};
use crate::lock::{InstanceLock, LockOutcome};
use crate::module::{ModuleRequest, ServiceModule};
use crate::paths::ServicePaths;
use crate::supervisor::{RestartPolicy, Shared, Supervisor};

/// Source of the configuration, read again on `reload` and `SIGHUP`.
pub trait ConfigSource: Send + Sync {
    /// Reads the configuration.
    ///
    /// # Errors
    ///
    /// Returns `E_CONFIG` when a configuration file is not valid.
    fn load(&self) -> Result<Config, Error>;
}

impl ConfigSource for Config {
    fn load(&self) -> Result<Config, Error> {
        Ok(self.clone())
    }
}

/// Settings of one run of the service.
#[derive(Clone)]
pub struct ServiceSetup {
    config: Config,
    config_source: Option<Arc<dyn ConfigSource>>,
    paths: ServicePaths,
    clock: Arc<dyn Clock>,
    extensions: Arc<Extensions>,
    stop: CancelToken,
    signals: bool,
}

impl ServiceSetup {
    /// Setup for the configuration: state directory from `state.dir` or the
    /// operating system, the system clock, termination signals handled.
    ///
    /// # Errors
    ///
    /// `E_IO` when the state directory cannot be determined.
    pub fn new(config: Config) -> Result<Self, Error> {
        let paths = ServicePaths::from_config(&config).map_err(|error| {
            Error::new(
                ErrorCode::Io,
                format!("cannot determine the state directory: {error}; set `state.dir`"),
            )
        })?;
        Ok(Self::with_paths(config, paths))
    }

    /// Setup with an explicit state directory.
    #[must_use]
    pub fn with_paths(config: Config, paths: ServicePaths) -> Self {
        Self {
            config,
            config_source: None,
            paths,
            clock: Arc::new(SystemClock::new()),
            extensions: Arc::new(Extensions::new()),
            stop: CancelToken::new(),
            signals: true,
        }
    }

    /// Replaces the clock.
    #[must_use]
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Sets the source read on `reload`; without one, `reload` passes the
    /// configuration given to [`new`](Self::new) again.
    #[must_use]
    pub fn config_source(mut self, source: Arc<dyn ConfigSource>) -> Self {
        self.config_source = Some(source);
        self
    }

    /// Sets the facilities handed to modules, for example the factory of
    /// execution contexts for schedule actions.
    #[must_use]
    pub fn extensions(mut self, extensions: Arc<Extensions>) -> Self {
        self.extensions = extensions;
        self
    }

    /// Sets the token whose cancellation stops the service.
    #[must_use]
    pub fn stop_token(mut self, stop: CancelToken) -> Self {
        self.stop = stop;
        self
    }

    /// Leaves `SIGTERM`, `SIGINT` and `SIGHUP` to the caller (Unix).
    #[must_use]
    pub fn without_signals(mut self) -> Self {
        self.signals = false;
        self
    }

    /// Paths of the service files.
    #[must_use]
    pub fn paths(&self) -> &ServicePaths {
        &self.paths
    }

    /// Token whose cancellation stops the service.
    #[must_use]
    pub fn stop(&self) -> CancelToken {
        self.stop.clone()
    }
}

impl fmt::Debug for ServiceSetup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceSetup")
            .field("paths", &self.paths)
            .field("signals", &self.signals)
            .finish_non_exhaustive()
    }
}

/// Runs the service until its stop token is cancelled, `SIGTERM` or
/// `SIGINT` arrives (Unix), or the service manager sends `Stop` or
/// `Shutdown` (Windows, when started by the service control manager).
///
/// The instance lock is taken before anything else. Then a socket left by a
/// previous service is removed, the control channel is opened, every module
/// starts on its own thread and the heartbeat is written. On the way out
/// every module is asked to stop and given ten seconds, the control channel
/// and the heartbeat file are removed and the lock is released.
///
/// # Errors
///
/// `E_EXISTS` when the service already runs for this state directory (the
/// `pid` of the running service is in `details`), `E_INVALID_VALUE` when two
/// modules have the same name, `E_IO` when the state directory or the
/// control channel cannot be created.
pub fn run(setup: ServiceSetup, modules: Vec<Arc<dyn ServiceModule>>) -> Result<(), Error> {
    #[cfg(windows)]
    let (setup, modules) = match crate::scm::dispatch(setup, modules) {
        crate::scm::Dispatch::Done(result) => return result,
        crate::scm::Dispatch::Console(setup, modules) => (*setup, modules),
    };
    serve(setup, modules)
}

pub(crate) fn serve(
    setup: ServiceSetup,
    modules: Vec<Arc<dyn ServiceModule>>,
) -> Result<(), Error> {
    let mut names = HashSet::new();
    if let Some(module) = modules
        .iter()
        .find(|module| !names.insert(module.name().to_owned()))
    {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!(
                "two modules are named `{}`; give every module its own name",
                module.name()
            ),
        ));
    }
    #[cfg(unix)]
    let signals = if setup.signals {
        Some(crate::signals::SignalGuard::block().map_err(io_error)?)
    } else {
        None
    };
    let paths = setup.paths.clone();
    let lock = match InstanceLock::try_acquire(&paths.lock_file()).map_err(io_error)? {
        LockOutcome::Acquired(lock) => lock,
        LockOutcome::Busy => return Err(already_running(&paths)),
    };
    let outcome = serve_locked(
        &setup,
        modules,
        #[cfg(unix)]
        signals,
    );
    lock.release();
    outcome
}

fn serve_locked(
    setup: &ServiceSetup,
    modules: Vec<Arc<dyn ServiceModule>>,
    #[cfg(unix)] signals: Option<crate::signals::SignalGuard>,
) -> Result<(), Error> {
    let listener = ipc::bind(&setup.paths).map_err(io_error)?;
    let hub = Arc::new(Hub::default());
    let shared = Shared {
        clock: Arc::clone(&setup.clock),
        config: Arc::new(Mutex::new(Arc::new(setup.config.clone()))),
        state_dir: setup.paths.state_dir().to_owned(),
        hub: Arc::clone(&hub),
        extensions: Arc::clone(&setup.extensions),
        policy: RestartPolicy::default(),
    };
    let supervisors: Vec<Arc<Supervisor>> = modules
        .into_iter()
        .map(|module| Arc::new(Supervisor::new(module)))
        .collect();
    let core = Arc::new(HostCore {
        pid: std::process::id(),
        started: setup.clock.now(),
        shared: shared.clone(),
        source: setup.config_source.clone(),
        supervisors: supervisors.clone(),
    });
    let server = Server::start(
        listener,
        Arc::clone(&hub),
        Arc::clone(&core) as Arc<dyn Control>,
    )
    .map_err(io_error)?;
    let module_stop = CancelToken::new();
    let mut threads: Vec<JoinHandle<()>> = Vec::new();
    let mut startup_failure: Option<Error> = None;
    for supervisor in &supervisors {
        match supervisor.spawn(shared.clone(), module_stop.clone()) {
            Ok(thread) => threads.push(thread),
            Err(error) => {
                supervisor.mark_failed(format!("cannot start the module thread: {error}"));
            }
        }
    }
    let heartbeat_stop = CancelToken::new();
    let heartbeat = spawn_heartbeat(setup, &core, heartbeat_stop.clone());
    if let Err(error) = &heartbeat {
        startup_failure = Some(thread_error("heartbeat", error));
    }
    #[cfg(unix)]
    let signal_thread = signals.map(|guard| {
        let stop = setup.stop.clone();
        let reload_core = Arc::clone(&core);
        guard.spawn(
            move || stop.cancel(CancelReason::Shutdown),
            move || {
                let _ = reload_core.reload();
            },
        )
    });
    #[cfg(unix)]
    if let Some(Err(error)) = &signal_thread {
        startup_failure = Some(thread_error("signal", error));
    }
    if startup_failure.is_some() {
        setup.stop.cancel(CancelReason::Shutdown);
    }
    let _ = setup.stop.wait_timeout(std::time::Duration::MAX);
    module_stop.cancel(CancelReason::Shutdown);
    for thread in threads {
        let _ = thread.join();
    }
    server.shutdown();
    heartbeat_stop.cancel(CancelReason::Shutdown);
    if let Ok(thread) = heartbeat {
        let _ = thread.join();
    }
    #[cfg(unix)]
    if let Some(Ok(thread)) = signal_thread {
        thread.finish();
    }
    startup_failure.map_or(Ok(()), Err)
}

/// `E_IO` for a thread of the service that could not be started; the service
/// stops instead of running without it.
fn thread_error(name: &str, error: &std::io::Error) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot start the {name} thread of the service: {error}"),
    )
}

fn spawn_heartbeat(
    setup: &ServiceSetup,
    core: &Arc<HostCore>,
    stop: CancelToken,
) -> std::io::Result<JoinHandle<()>> {
    let writer = HeartbeatWriter::new(
        setup.paths.heartbeat_file(),
        core.pid,
        core.started,
        u32::try_from(protocol::PROTOCOL_VERSION).unwrap_or(u32::MAX),
    );
    let interval = heartbeat::interval(setup.config.space.heartbeat_ms);
    let clock = Arc::clone(&setup.clock);
    let core = Arc::clone(core);
    thread::Builder::new()
        .name("nurt-service-heartbeat".to_owned())
        .spawn(move || writer.run(&*clock, interval, &stop, || core.module_states()))
}

fn io_error(error: std::io::Error) -> Error {
    Error::new(ErrorCode::Io, error.to_string())
}

fn already_running(paths: &ServicePaths) -> Error {
    let pid = heartbeat::read(&paths.heartbeat_file())
        .ok()
        .flatten()
        .map(|heartbeat| heartbeat.pid);
    let message = match pid {
        Some(pid) => {
            format!("the service is already running (pid {pid}); stop it before starting another")
        }
        None => "the service is already running; stop it before starting another".to_owned(),
    };
    let error = Error::new(ErrorCode::Exists, message);
    match pid {
        Some(pid) => error.with_detail("pid", json!(pid)),
        None => error,
    }
}

/// State of the running service shared with the control channel.
struct HostCore {
    pid: u32,
    started: WallTime,
    shared: Shared,
    source: Option<Arc<dyn ConfigSource>>,
    supervisors: Vec<Arc<Supervisor>>,
}

impl HostCore {
    fn module_states(&self) -> BTreeMap<String, ModuleState> {
        self.supervisors
            .iter()
            .map(|supervisor| (supervisor.name().to_owned(), supervisor.state()))
            .collect()
    }

    fn supervisor(&self, name: &str) -> Option<&Arc<Supervisor>> {
        self.supervisors
            .iter()
            .find(|supervisor| supervisor.name() == name)
    }
}

impl Control for HostCore {
    fn ping(&self) -> Value {
        json!({ "pid": self.pid })
    }

    fn status(&self) -> Value {
        let modules: Map<String, Value> = self
            .supervisors
            .iter()
            .map(|supervisor| (supervisor.name().to_owned(), supervisor.describe()))
            .collect();
        json!({
            "pid": self.pid,
            "started": heartbeat::format_instant(self.started),
            "version": env!("CARGO_PKG_VERSION"),
            "protocol": protocol::PROTOCOL_VERSION,
            "modules": modules,
            "subscribers": self.shared.hub.subscribers(),
        })
    }

    fn reload(&self) -> Result<Value, Error> {
        let config = match &self.source {
            Some(source) => Arc::new(source.load()?),
            None => self.shared.config(),
        };
        *self
            .shared
            .config
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Arc::clone(&config);
        let mut modules = Map::new();
        for supervisor in &self.supervisors {
            let outcome = match supervisor.reload(&config) {
                Ok(outcome) => json!(outcome),
                Err(error) => json!({ "error": protocol::language_error(&error) }),
            };
            modules.insert(supervisor.name().to_owned(), outcome);
        }
        Ok(json!({ "modules": modules }))
    }

    fn hosts(&self, module: &str) -> bool {
        self.supervisor(module).is_some()
    }

    fn module_request(&self, module: &str, request: ModuleRequest) -> Result<Value, RouteError> {
        let Some(supervisor) = self.supervisor(module) else {
            return Err(RouteError::Unavailable(format!(
                "the service does not host the module `{module}`"
            )));
        };
        supervisor.request(request).map_err(|error| match error {
            Some(error) => RouteError::Module(error),
            None => RouteError::Unavailable(format!(
                "the module `{module}` is {}; send `reload` to start it again",
                supervisor.state().as_str()
            )),
        })
    }

    fn client_closed(&self, client: &str) {
        for supervisor in &self.supervisors {
            supervisor.client_closed(client);
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
