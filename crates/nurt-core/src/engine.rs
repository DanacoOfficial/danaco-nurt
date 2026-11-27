// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The engine: one configured instance serving requests.
//!
//! [`Engine::open`] assembles the configuration, the state directory, the
//! stores of sessions, cursors and candidates and the read cache from an
//! [`EngineConfig`]. [`Engine::run`] handles one request envelope and always
//! returns a response: an invalid request or a failed program is reported in
//! the effect report. [`Engine::start`] runs a request on its own thread and
//! delivers its events; the returned [`ExecHandle`] waits for or cancels it.
//!
//! A request passes these steps: envelope rules and defaults, session,
//! language front end (correction and binding, or `accept` of a candidate),
//! planning against the command registry, execution by the executor of
//! `nurt-exec`, report assembly, rendering and paging. A request with
//! `cursor` reads a kept page and runs nothing. The reports of executed
//! programs and of programs that fail before execution are assembled by the
//! report rules, and every response carries the rendered result for the
//! requested format. Executing a program needs a file system in the
//! [`EngineConfig`]; without it the request fails with `E_UNSUPPORTED`. A
//! program with effects also needs a journal, and fails at its apply step
//! without one; a program that only reads runs without a journal.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_catalog::{CatalogQuery, Registry};
use nurt_contracts::ast::{AST_VERSION, AstDocument};
use nurt_contracts::bound::{
    BOUND_VERSION, BoundDocument, BoundExpr, BoundHead, BoundPipeline, BoundStatement,
};
use nurt_contracts::config::{CONFIG_FORMAT, Config, DurationSetting, Language as ConfigLanguage};
use nurt_contracts::env::{Clock, Host, Os, WallTime};
use nurt_contracts::envelope::{
    DEFAULT_TIMEOUT, ENVELOPE_VERSION, Language, Request, RequestInput, RequestOptions, Response,
    Verbosity,
};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::{AbsPath, Fs, RegistryAccess};
use nurt_contracts::journal::{HistoryQuery, JournalStore, OperationRecord, VolumeSpace};
use nurt_contracts::kinds::{EffectKind, SyntaxVariant};
use nurt_contracts::report::{
    REPORT_VERSION, RecordsPage, Report, ReportPart, ReportStatus, Timing,
};
use nurt_contracts::schema;
use nurt_contracts::services::Services;
use nurt_contracts::stage::{
    CancelReason, CancelToken, EventSink, ExecCtx, ExecStep, Extensions, Limits, StageImpl,
};
use nurt_contracts::value::{Record, RecordKind};
use nurt_exec::{ExecEnv, Executor, OperationOutcome, Outcome, ResultData};
use nurt_lspclient::provider::LspProvider;
use nurt_plan::{CheckSummary, Plan, PlanOptions};
use nurt_report::assemble::{ReportBuilder, ReportContext, TargetChange};
use nurt_report::rules::PathContext;
use nurt_report::{RenderOptions, respond};
use nurt_sys::registry::RegistryProvider;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::candidates::CandidateStore;
use crate::config::{ConfigLoader, ConfigSources, LoadedConfig};
use crate::cursor::{CursorStore, ProgramRef};
use crate::elevated::Early;
use crate::elevation::{Mark, Setup};
use crate::error::{EngineError, envelope_error, unavailable};
use crate::events::{EventListener, Sequencer};
use crate::extensions;
use crate::frontend::{Compiled, Corrected, Frontend, UnavailableFrontend};
use crate::inverse::InverseLog;
use crate::readcache::ReadCache;
use crate::registry::{ProcessComponents, registry_with};
use crate::session::{SessionId, Sessions};
use crate::state::{StateArea, StateDir, write_atomic};
use crate::system::{SystemClock, SystemHost};

/// Report kind of a request that fails the envelope rules before any program
/// is known.
const REQUEST_KIND: &str = "request";
/// Report kind of a failed request whose program is not known.
const PROGRAM_KIND: &str = "program";
/// Report kind of a failed page request.
const PAGE_KIND: &str = "page";
/// Report kind of a failed `accept` request.
const ACCEPT_KIND: &str = "accept";
/// File of the correction candidates in the state directory.
const CANDIDATES_FILE: &str = "candidates.json";
/// Name of the threads started by [`Engine::start`].
const EXEC_THREAD_NAME: &str = "nurt-exec";
/// Cargo feature compiled into this build, listed by [`Engine::info`].
const FIXTURES_FEATURE: &str = "testkit";

/// Everything an engine is built from.
///
/// The clock and the host are always given; the file system, the journal,
/// the language front end and the services of other command groups are
/// optional. Without a front end every program fails with `E_UNSUPPORTED`;
/// without a file system a program fails the same way when it is executed.
/// Without a journal programs that only read still run, programs with effects
/// fail at their apply step, and `history` and `space` fail with
/// `E_UNSUPPORTED`.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use nurt_core::config::ConfigSources;
/// use nurt_core::{Engine, EngineConfig};
/// use nurt_testkit::clock::VirtualClock;
/// use nurt_testkit::host::TestHost;
///
/// let config = EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
///     .with_sources(ConfigSources::isolated())
///     .without_state_dir();
/// let engine = Engine::open(config).unwrap();
/// assert_eq!(engine.config().journal.max_ops.to_string(), "30");
/// ```
pub struct EngineConfig {
    clock: Arc<dyn Clock>,
    host: Arc<dyn Host>,
    fs: Option<Arc<dyn Fs>>,
    journal: Option<Arc<dyn JournalStore>>,
    system_journal: bool,
    account: Option<crate::elevation::AccountJournal>,
    process_components: bool,
    system_services: bool,
    registry: Option<Arc<RegistryProvider>>,
    frontend: Arc<dyn Frontend>,
    services: Services,
    stages: Vec<Arc<dyn StageImpl>>,
    sources: ConfigSources,
    session: Option<String>,
    state_dir: Option<PathBuf>,
    use_state_dir: bool,
    elevation: Setup,
}

impl fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineConfig")
            .field("sources", &self.sources)
            .field("session", &self.session)
            .field("state_dir", &self.state_dir)
            .field("services", &self.services)
            .field("stages", &self.stages.len())
            .finish_non_exhaustive()
    }
}

impl EngineConfig {
    /// Configuration with the given clock and host, the platform
    /// configuration sources and no optional component.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>, host: Arc<dyn Host>) -> Self {
        Self {
            clock,
            host,
            fs: None,
            journal: None,
            system_journal: false,
            account: None,
            process_components: false,
            system_services: false,
            registry: None,
            frontend: Arc::new(UnavailableFrontend),
            services: Services::new(),
            stages: Vec::new(),
            sources: ConfigSources::new(),
            session: None,
            state_dir: None,
            use_state_dir: true,
            elevation: Setup::default(),
        }
    }

    /// Configuration for the running process: the system clock and host
    /// ([`SystemClock`], [`SystemHost`]), the platform configuration sources
    /// and no optional component.
    ///
    /// The language front end, the services, the file system and the journal
    /// are added with the `with_*` methods; see [`Engine::open_system`].
    #[must_use]
    pub fn system() -> Self {
        Self::new(Arc::new(SystemClock::new()), Arc::new(SystemHost::detect()))
            .with_fs(nurt_fs::system_fs())
            .with_system_journal()
            .with_process_components()
            .with_system_services()
    }

    /// The engine registers the services of the command groups of this build
    /// when it opens: documents, source code, the process host, the file
    /// listing, repository queries, the text codec and the file holders. A
    /// service given with [`with_services`](Self::with_services) is kept.
    #[must_use]
    pub fn with_system_services(mut self) -> Self {
        self.system_services = true;
        self
    }

    /// The journal of the process is built in the state directory when the
    /// engine opens, unless a journal is given with
    /// [`with_journal`](Self::with_journal) or the engine has no state
    /// directory.
    #[must_use]
    pub fn with_system_journal(mut self) -> Self {
        self.system_journal = true;
        self
    }

    /// The commands of the extensions installed for the account are added to
    /// the command registry when the engine opens, and the code commands use
    /// language servers for semantic analysis.
    #[must_use]
    pub fn with_process_components(mut self) -> Self {
        self.process_components = true;
        self
    }

    /// Read-only file system given to stages.
    #[must_use]
    pub fn with_fs(mut self, fs: Arc<dyn Fs>) -> Self {
        self.fs = Some(fs);
        self
    }

    /// Journal of changes.
    #[must_use]
    pub fn with_journal(mut self, journal: Arc<dyn JournalStore>) -> Self {
        self.journal = Some(journal);
        self
    }

    /// Registry that `registry:` paths address and that the journal changes
    /// and restores, instead of the registry of the system, which Windows
    /// engines use by default. Outside Windows an engine has no registry
    /// unless one is given here, and `registry:` paths fail with
    /// `E_OS_ONLY`.
    #[must_use]
    pub fn with_registry(mut self, registry: Arc<RegistryProvider>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Language front end.
    #[must_use]
    pub fn with_frontend(mut self, frontend: Arc<dyn Frontend>) -> Self {
        self.frontend = frontend;
        self
    }

    /// Services of other command groups, given to every stage.
    #[must_use]
    pub fn with_services(mut self, services: Services) -> Self {
        self.services = services;
        self
    }

    /// Stage implementation registered with the command catalog in addition
    /// to the implementations compiled into the build. The command named by
    /// its descriptor must be in the catalog, and no other implementation may
    /// accept the same record kinds; otherwise [`Engine::open`] fails.
    #[must_use]
    pub fn with_stage(mut self, stage: Arc<dyn StageImpl>) -> Self {
        self.stages.push(stage);
        self
    }

    /// Sources of the configuration.
    #[must_use]
    pub fn with_sources(mut self, sources: ConfigSources) -> Self {
        self.sources = sources;
        self
    }

    /// Session of requests that do not name one, for example the session
    /// given to `initialize` of the protocol server.
    #[must_use]
    pub fn with_session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    /// State directory, instead of `state.dir` and the platform default.
    #[must_use]
    pub fn with_state_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.state_dir = Some(dir.into());
        self.use_state_dir = true;
        self
    }

    /// The journal of an account that this process, running as the
    /// administrator, writes for it: the entries and the copies are created
    /// with the user and the group of the account below its state directory
    /// (Linux), while the targets are changed with the rights of the
    /// process. The engine has no other state directory.
    #[must_use]
    pub fn with_account_journal(mut self, account: crate::elevation::AccountJournal) -> Self {
        self.account = Some(account);
        self.system_journal = true;
        self.use_state_dir = false;
        self.state_dir = None;
        self
    }

    /// Executable that the elevation backends start with the subcommand
    /// `elevated-exec`, instead of the running executable.
    #[must_use]
    pub fn with_elevation_executable(mut self, executable: impl Into<PathBuf>) -> Self {
        self.elevation.executable = Some(executable.into());
        self
    }

    /// Search path of the programs of the elevation backends (`sudo`,
    /// `doas`, `run0`), instead of the path of the process.
    #[must_use]
    pub fn with_elevation_search_path(mut self, path: impl Into<std::ffi::OsString>) -> Self {
        self.elevation.search_path = Some(path.into());
        self
    }

    /// States whether the process has the identity of the administrator,
    /// instead of asking the operating system.
    #[must_use]
    pub fn with_administrator(mut self, administrator: bool) -> Self {
        self.elevation.administrator = Some(administrator);
        self
    }

    /// No state directory: cursors and candidates are kept in memory only.
    #[must_use]
    pub fn without_state_dir(mut self) -> Self {
        self.state_dir = None;
        self.use_state_dir = false;
        self
    }
}

/// Result of [`Engine::check`]: the bound program and what running it would
/// touch, without running it.
///
/// In JSON the members of the summary (`nurt-check/1`) and the member
/// `bound` form one object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckReport {
    /// The bound program.
    pub bound: BoundDocument,
    /// Hash of the syntax tree the program was bound from, as
    /// `sha256:<hex>`; absent when the tree is not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ast_hash: Option<String>,
    /// Class of the effect and reversibility, paths, network endpoints,
    /// external programs, waits, command groups and the corrections applied
    /// to the program text.
    #[serde(flatten)]
    pub summary: CheckSummary,
}

/// Versions of the formats spoken by the engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Formats {
    /// Syntax tree `nurt-ast/1`.
    pub ast: String,
    /// Bound tree `nurt-bound/1`.
    pub bound: String,
    /// Effect report `nurt-report/1`.
    pub report: String,
    /// Request and response envelope.
    pub envelope: String,
    /// Configuration.
    pub config: String,
}

/// Description of an engine for version negotiation and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EngineInfo {
    /// Version of the engine.
    pub version: String,
    /// Versions of the formats.
    pub formats: Formats,
    /// Optional features compiled into this build.
    pub features: Vec<String>,
    /// Operating system of the host.
    pub os: Os,
    /// Processor architecture of the host.
    pub architecture: String,
    /// Services of other command groups that are available.
    pub services: Vec<String>,
    /// State directory, when the engine has one.
    pub state_dir: Option<String>,
}

#[derive(Debug, Default)]
struct Completion {
    response: Mutex<Option<Response>>,
    ready: Condvar,
}

impl Completion {
    fn complete(&self, response: Response) {
        *self.response.lock().unwrap_or_else(PoisonError::into_inner) = Some(response);
        self.ready.notify_all();
    }
}

/// Handle of a request started with [`Engine::start`].
#[derive(Debug, Clone)]
pub struct ExecHandle {
    request_id: String,
    cancel: CancelToken,
    completion: Arc<Completion>,
}

impl ExecHandle {
    /// Identifier of the request, as carried by its events.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Requests cancellation; the response then reports `E_CANCELLED`.
    /// Cancelling a finished or already cancelled request has no effect.
    pub fn cancel(&self) {
        self.cancel.cancel(CancelReason::Requested);
    }

    /// Whether the response is available.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.completion
            .response
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// Waits for the response, at most `timeout` (`None`: without limit);
    /// returns `None` when the request has not finished in time.
    #[must_use]
    pub fn wait(&self, timeout: Option<Duration>) -> Option<Response> {
        let guard = self
            .completion
            .response
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let guard = match timeout {
            None => self
                .completion
                .ready
                .wait_while(guard, |response| response.is_none())
                .unwrap_or_else(PoisonError::into_inner),
            Some(timeout) => {
                self.completion
                    .ready
                    .wait_timeout_while(guard, timeout, |response| response.is_none())
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
        };
        guard.clone()
    }
}

pub(crate) struct Inner {
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) host: Arc<dyn Host>,
    pub(crate) fs: Option<Arc<dyn Fs>>,
    pub(crate) journal: Option<Arc<dyn JournalStore>>,
    pub(crate) registry_access: Option<Arc<dyn RegistryAccess>>,
    pub(crate) frontend: Arc<dyn Frontend>,
    pub(crate) services: Arc<Services>,
    pub(crate) registry: Arc<Registry>,
    pub(crate) catalog_rev: String,
    pub(crate) loader: ConfigLoader,
    pub(crate) default_config: Arc<LoadedConfig>,
    pub(crate) projects: Mutex<HashMap<Option<PathBuf>, CachedConfig>>,
    pub(crate) state: Option<StateDir>,
    pub(crate) sessions: Sessions,
    pub(crate) candidates: CandidateStore,
    pub(crate) cursors: CursorStore,
    pub(crate) read_cache: Arc<ReadCache>,
    pub(crate) default_session: Option<SessionId>,
    pub(crate) requests: AtomicU64,
    pub(crate) results: AtomicU64,
    pub(crate) elevation: Setup,
    pub(crate) inverses: Option<InverseLog>,
}

/// The registry of the system on Windows; elsewhere there is none.
#[cfg(windows)]
fn system_registry() -> Option<Arc<RegistryProvider>> {
    Some(Arc::new(RegistryProvider::system()))
}

/// The registry of the system on Windows; elsewhere there is none.
#[cfg(not(windows))]
fn system_registry() -> Option<Arc<RegistryProvider>> {
    None
}

/// Configuration of one project with the state of its file at load time.
pub(crate) struct CachedConfig {
    stamp: Option<FileStamp>,
    loaded: Arc<LoadedConfig>,
}

/// Modification time and size of a project file; a change of either reloads
/// the configuration of the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    modified: Option<std::time::SystemTime>,
    len: u64,
}

impl FileStamp {
    fn of(file: Option<&std::path::Path>) -> Option<Self> {
        let metadata = std::fs::metadata(file?).ok()?;
        Some(Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

/// Request after the envelope rules, with every option set.
#[derive(Clone)]
pub(crate) struct Prepared {
    pub(crate) config: Arc<LoadedConfig>,
    pub(crate) options: RequestOptions,
    pub(crate) session: Option<SessionId>,
    pub(crate) timeout: Duration,
    pub(crate) elevation: Option<Mark>,
    /// Values given for the parameters of a script.
    pub(crate) params: Option<serde_json::Map<String, serde_json::Value>>,
}

/// The engine of Danaco Nurt.
///
/// Cloning is cheap and clones share all state. The engine is `Send` and
/// `Sync`; requests may run on many threads at the same time.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::envelope::Request;
/// use nurt_contracts::error::ErrorCode;
/// use nurt_core::config::ConfigSources;
/// use nurt_core::{Engine, EngineConfig};
/// use nurt_testkit::clock::VirtualClock;
/// use nurt_testkit::host::TestHost;
///
/// let engine = Engine::open(
///     EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
///         .with_sources(ConfigSources::isolated())
///         .without_state_dir(),
/// )
/// .unwrap();
/// let response = engine.run(&Request::program("find \"/srv\""));
/// let error = response.report.error.as_ref().unwrap();
/// assert_eq!(error.code, ErrorCode::Unsupported);
/// assert!(response.validate().is_ok());
/// ```
#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: Arc<Inner>,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine")
            .field("state", &self.inner.state)
            .field("services", &self.inner.services)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// Opens an engine of the running process: system clock and host, the
    /// platform configuration sources, the given language front end and the
    /// given services.
    ///
    /// This is the assembly point of an executable; the front end and the
    /// services come from the crates of the language and of the commands.
    ///
    /// # Errors
    ///
    /// Same as [`Engine::open`].
    pub fn open_system(
        frontend: Arc<dyn Frontend>,
        services: Services,
    ) -> Result<Self, EngineError> {
        Self::open(
            EngineConfig::system()
                .with_frontend(frontend)
                .with_services(services),
        )
    }

    /// Opens an engine.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when the configuration is invalid, the
    /// session identifier is invalid, the state directory cannot be created
    /// or the command registry of the build cannot be assembled.
    pub fn open(config: EngineConfig) -> Result<Self, EngineError> {
        let loader = ConfigLoader::new(&config.sources, config.host.as_ref())?;
        let project = loader.project_for(None);
        let loaded = Arc::new(loader.load(project.as_deref())?);
        let components = if config.process_components {
            ProcessComponents {
                extensions: extensions::locations(config.host.as_ref(), &loaded.config().ext)
                    .map_err(|error| EngineError::registry(&error))?,
                semantic: Some(Arc::new(LspProvider::new(
                    nurt_proc::process_host(),
                    Arc::clone(&config.clock),
                    &loaded.config().code,
                ))),
            }
        } else {
            ProcessComponents::default()
        };
        let registry = registry_with(&config.stages, components)
            .map(Arc::new)
            .map_err(|error| EngineError::registry(&error))?;
        let catalog_rev = registry
            .catalog_rev()
            .map_err(|error| EngineError::registry(&error))?;
        let root = if config.use_state_dir {
            config
                .state_dir
                .clone()
                .or_else(|| loaded.config().state.dir.clone().map(PathBuf::from))
                .or_else(|| loader.default_state_dir().map(PathBuf::from))
        } else {
            None
        };
        let state = root.map(StateDir::open).transpose()?;
        let windows_registry = config.registry.or_else(system_registry);
        let journal = match config.journal {
            Some(journal) => Some(journal),
            None if config.system_journal => {
                let root = config
                    .account
                    .as_ref()
                    .map(|account| account.state_dir.clone())
                    .or_else(|| state.as_ref().map(|state| state.root().to_path_buf()));
                root.map(|root| {
                    crate::journal_setup::open_journal(
                        loader.layers(),
                        &loaded,
                        false,
                        Arc::clone(&config.clock),
                        &root,
                        windows_registry.clone(),
                        config.account.as_ref(),
                    )
                })
                .transpose()?
            }
            None => None,
        };
        let inverses = state
            .as_ref()
            .map(|state| InverseLog::in_state(state.root()));
        let default_session = config
            .session
            .map(SessionId::new)
            .transpose()
            .map_err(|error| EngineError::argument(error.message.clone()))?;
        let settings = loaded.config();
        let cursors = CursorStore::new(
            state.as_ref().map(|state| state.area(StateArea::Results)),
            settings.output.spool_ttl.as_std(),
        );
        cursors.sweep(config.clock.now());
        let mut read_cache = ReadCache::new(Arc::clone(&config.clock));
        if let Some(state) = &state {
            read_cache = read_cache.with_dir(state.area(StateArea::ReadCache));
        }
        let services = if config.system_services {
            crate::system_services::complete(
                config.services,
                config.fs.as_ref(),
                &config.clock,
                &config.host,
                settings,
            )
        } else {
            config.services
        };
        let inner = Inner {
            sessions: Sessions::new(settings.session.ttl.as_std()),
            candidates: CandidateStore::new(
                settings.correction.candidates_ttl.as_std(),
                state
                    .as_ref()
                    .map(|state| state.root().join(CANDIDATES_FILE)),
            ),
            cursors,
            read_cache: Arc::new(read_cache),
            projects: Mutex::new(HashMap::from([(
                project.clone(),
                CachedConfig {
                    stamp: FileStamp::of(project.as_deref()),
                    loaded: Arc::clone(&loaded),
                },
            )])),
            default_config: loaded,
            clock: config.clock,
            host: config.host,
            fs: config.fs,
            journal,
            registry_access: windows_registry.map(|registry| registry as Arc<dyn RegistryAccess>),
            frontend: config.frontend,
            services: Arc::new(services),
            registry,
            catalog_rev,
            loader,
            state,
            default_session,
            requests: AtomicU64::new(0),
            results: AtomicU64::new(0),
            inverses,
            elevation: config.elevation,
        };
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Configuration in effect for requests without their own base
    /// directory.
    #[must_use]
    pub fn config(&self) -> &Config {
        self.inner.default_config.config()
    }

    /// The same configuration with the level of every key, the notes and the
    /// effective journal limits.
    #[must_use]
    pub fn loaded_config(&self) -> &LoadedConfig {
        &self.inner.default_config
    }

    /// The state directory, when the engine has one.
    #[must_use]
    pub fn state_dir(&self) -> Option<&StateDir> {
        self.inner.state.as_ref()
    }

    /// Services of other command groups.
    #[must_use]
    pub fn services(&self) -> &Services {
        &self.inner.services
    }

    /// The read cache shared by all requests.
    #[must_use]
    pub fn read_cache(&self) -> &Arc<ReadCache> {
        &self.inner.read_cache
    }

    /// Sessions of the callers.
    #[must_use]
    pub fn sessions(&self) -> &Sessions {
        &self.inner.sessions
    }

    /// Handles one request and returns its response.
    ///
    /// Every failure, including a request that breaks the envelope rules, is
    /// reported in the effect report of the response.
    #[must_use]
    pub fn run(&self, request: &Request) -> Response {
        self.execute(request, &CancelToken::new(), None)
    }

    /// Starts a request on its own thread and delivers its events to
    /// `listener`, from `exec.started` to `exec.finished`.
    #[must_use]
    pub fn start(&self, request: &Request, listener: Arc<dyn EventListener>) -> ExecHandle {
        let request_id = self.next_request_id();
        let cancel = CancelToken::new();
        let completion = Arc::new(Completion::default());
        let handle = ExecHandle {
            request_id: request_id.clone(),
            cancel: cancel.clone(),
            completion: Arc::clone(&completion),
        };
        let kind = input_kind(request);
        let sequencer = Arc::new(Sequencer::new(
            listener,
            Arc::clone(&self.inner.clock),
            request_id,
        ));
        let engine = self.clone();
        let owned = request.clone();
        let finished = Arc::clone(&completion);
        let spawned = std::thread::Builder::new()
            .name(EXEC_THREAD_NAME.to_owned())
            .spawn(move || {
                let events: Arc<dyn EventSink> = sequencer.clone();
                let response = engine.execute(&owned, &cancel, Some(events));
                sequencer.close(
                    response.report.status,
                    response.report.error.as_ref().map(Error::code),
                );
                finished.complete(response);
            });
        if let Err(error) = spawned {
            let report = failed_report(
                kind,
                Error::new(
                    ErrorCode::Internal,
                    format!("cannot start the execution thread: {error}"),
                ),
            );
            completion.complete(respond_with(&report, &request.options));
        }
        handle
    }

    /// Binds and plans a request and computes what running it would touch,
    /// without running it.
    ///
    /// # Errors
    ///
    /// Returns the language error of the envelope, the front end, the planner
    /// (for example `E_ONE_EFFECT` with the split program in `next_steps`) or
    /// the static check of a stage; a request with `cursor` is `E_ARG`.
    pub fn check(&self, request: &Request) -> Result<CheckReport, Error> {
        let prepared = self.prepare(request)?;
        let compiled = match request
            .input()
            .map_err(|error| envelope_error(&error, request))?
        {
            RequestInput::Program { text, variant } => self.compile(text, variant, &prepared),
            RequestInput::Ast(document) => self.bind(document, &prepared),
            RequestInput::Accept(reference) => self.accept(reference, &prepared),
            _ => Err(Error::new(
                ErrorCode::Arg,
                "check needs `program`, `ast` or `accept`",
            )),
        }?;
        let plan = self.plan(&compiled, &prepared)?;
        let summary = nurt_plan::check(
            &plan,
            &self.inner.registry,
            self.inner.host.as_ref(),
            prepared.config.config(),
        )?;
        Ok(CheckReport {
            bound: compiled.bound,
            ast_hash: compiled.ast_hash,
            summary,
        })
    }

    /// Parses program text without correcting it.
    ///
    /// # Errors
    ///
    /// Returns the lexical or syntax error of the text.
    pub fn parse(&self, text: &str, variant: Option<SyntaxVariant>) -> Result<AstDocument, Error> {
        self.inner.frontend.parse(text, variant, self.config())
    }

    /// Prints a syntax tree as text; without `variant`, in the configured
    /// notation variant.
    ///
    /// # Errors
    ///
    /// Returns the error of the front end for a tree that cannot be printed.
    pub fn print(
        &self,
        ast: &AstDocument,
        variant: Option<SyntaxVariant>,
    ) -> Result<String, Error> {
        self.inner
            .frontend
            .print(ast, variant.unwrap_or(self.config().syntax.variant))
    }

    /// Formats program text written in `from` as canonical text of `to`,
    /// keeping its comments; a missing variant is the configured one.
    ///
    /// # Errors
    ///
    /// Returns the syntax error of the text, or the error of printing it.
    pub fn format(
        &self,
        text: &str,
        from: Option<SyntaxVariant>,
        to: Option<SyntaxVariant>,
    ) -> Result<String, Error> {
        self.inner.frontend.format(text, from, to, self.config())
    }

    /// Parses and corrects program text without running it.
    ///
    /// # Errors
    ///
    /// Returns the syntax error, or `E_AMBIGUOUS` or `E_UNCORRECTABLE` with
    /// candidates.
    pub fn correct(&self, text: &str, variant: Option<SyntaxVariant>) -> Result<Corrected, Error> {
        self.inner.frontend.correct(text, variant, self.config())
    }

    /// Reads a further page of an earlier result; nothing is run.
    #[must_use]
    pub fn page(&self, cursor: &str, options: &RequestOptions) -> Response {
        self.run(&Request::cursor(cursor).with_options(options.clone()))
    }

    /// Description of a command, a group or the language (`core`) as JSON.
    ///
    /// # Errors
    ///
    /// Returns the error of the front end, for example for an unknown
    /// subject.
    pub fn describe(&self, subject: &str, language: Option<Language>) -> Result<Json, Error> {
        let language = language.unwrap_or_else(|| language_of(self.config()));
        self.inner
            .frontend
            .describe(subject, language)
            .map_err(|error| localized(error, language))
    }

    /// Help text of a topic.
    ///
    /// # Errors
    ///
    /// Returns the error of the front end, for example for an unknown topic.
    pub fn help(&self, topic: &str, language: Option<Language>) -> Result<String, Error> {
        let language = language.unwrap_or_else(|| language_of(self.config()));
        self.inner
            .frontend
            .help(topic, language)
            .map_err(|error| localized(error, language))
    }

    /// Help text of a topic of the stated kind with a number of examples.
    ///
    /// `kind` is the name of a topic kind (`command`, `group`, `error`,
    /// `skill`, `syntax`, `corrections`); a front end that does not
    /// distinguish them answers like [`Engine::help`].
    ///
    /// # Errors
    ///
    /// Returns the error of the front end, for example for an unknown topic
    /// or a topic of another kind.
    pub fn help_query(
        &self,
        topic: &str,
        kind: Option<&str>,
        examples: Option<usize>,
        language: Option<Language>,
    ) -> Result<String, Error> {
        let language = language.unwrap_or_else(|| language_of(self.config()));
        self.inner
            .frontend
            .help_query(topic, kind, examples, language)
            .map_err(|error| localized(error, language))
    }

    /// The command catalog as JSON (`nurt-catalog-result/1`): the revision
    /// `catalog_rev`, the number of commands `total` and the groups with the
    /// number of their commands.
    ///
    /// # Errors
    ///
    /// Returns `E_INTERNAL` when the catalog cannot be serialized.
    pub fn catalog(&self) -> Result<Json, Error> {
        self.catalog_query(&CatalogQuery::Summary)
    }

    /// One query of the command catalog as JSON: the groups, the commands of
    /// one group or the full specification of one command.
    ///
    /// # Errors
    ///
    /// Returns `E_UNKNOWN_NAME` for an unknown group and `E_UNKNOWN_COMMAND`
    /// for an unknown command.
    pub fn catalog_query(&self, query: &CatalogQuery<'_>) -> Result<Json, Error> {
        self.inner.registry.catalog_json(query)
    }

    /// Revision of the command catalog: `sha256:` and the digest of its
    /// canonical form.
    #[must_use]
    pub fn catalog_rev(&self) -> &str {
        &self.inner.catalog_rev
    }

    /// Published JSON Schema `name`: `ast`, `bound`, `report`, `request`,
    /// `config`, `catalog` or `value`.
    ///
    /// # Errors
    ///
    /// Returns `E_UNKNOWN_VALUE` with the known names for another name.
    pub fn schema(&self, name: &str) -> Result<&'static str, Error> {
        schema::by_name(name).ok_or_else(|| {
            let known: Vec<&str> = schema::ALL.iter().map(|(name, _)| *name).collect();
            Error::new(
                ErrorCode::UnknownValue,
                format!("unknown schema `{name}`; known: {}", known.join(", ")),
            )
            .with_found(name)
            .with_detail("accepted", json!(known))
        })
    }

    /// Operations recorded in the journal.
    ///
    /// # Errors
    ///
    /// Returns `E_UNSUPPORTED` without a journal and the error of the
    /// journal otherwise.
    pub fn history(&self, query: &HistoryQuery) -> Result<Vec<OperationRecord>, Error> {
        self.journal()?.history(query)
    }

    /// Space used by the journal on one volume or on every volume.
    ///
    /// # Errors
    ///
    /// Returns `E_UNSUPPORTED` without a journal and the error of the
    /// journal otherwise.
    pub fn space(&self, volume: Option<&AbsPath>) -> Result<Vec<VolumeSpace>, Error> {
        self.journal()?.space(volume)
    }

    /// Description of the engine.
    #[must_use]
    pub fn info(&self) -> EngineInfo {
        let host = self.inner.host.info();
        EngineInfo {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            formats: Formats {
                ast: AST_VERSION.to_owned(),
                bound: BOUND_VERSION.to_owned(),
                report: REPORT_VERSION.to_owned(),
                envelope: ENVELOPE_VERSION.to_owned(),
                config: CONFIG_FORMAT.to_owned(),
            },
            features: if cfg!(feature = "fixtures") {
                vec![FIXTURES_FEATURE.to_owned()]
            } else {
                Vec::new()
            },
            os: host.os,
            architecture: host.architecture.clone(),
            services: self
                .inner
                .services
                .available()
                .iter()
                .map(|kind| kind.as_str().to_owned())
                .collect(),
            state_dir: self
                .inner
                .state
                .as_ref()
                .map(|state| state.root().to_string_lossy().into_owned()),
        }
    }

    /// Execution context that stages of `request` receive: clock, host, file
    /// system, journal, configuration, effective options, deadline, limits,
    /// state directory, services and the read cache (in the extensions).
    ///
    /// # Errors
    ///
    /// Returns the language error of the envelope or of the configuration of
    /// the base directory.
    pub fn context(&self, request: &Request) -> Result<ExecCtx, Error> {
        let prepared = self.prepare(request)?;
        Ok(self.build_context(&prepared, CancelToken::new()))
    }

    fn execute(
        &self,
        request: &Request,
        cancel: &CancelToken,
        events: Option<Arc<dyn EventSink>>,
    ) -> Response {
        self.route(request, cancel, events)
    }

    /// Runs a request in this process with the rights of the account, or
    /// with those of the administrator when `mark` says the process already
    /// has them.
    pub(crate) fn execute_local(
        &self,
        request: &Request,
        cancel: &CancelToken,
        events: Option<Arc<dyn EventSink>>,
        mark: Option<Mark>,
        early: Option<Early>,
    ) -> Response {
        let started = self.inner.clock.monotonic();
        let prepared = match self.prepare(request) {
            Ok(mut prepared) => {
                prepared.elevation = mark;
                prepared
            }
            Err(error) => {
                let report = failed_report(REQUEST_KIND, error);
                return respond_with(&report, &request.options);
            }
        };
        let compiled = match request.input() {
            Ok(RequestInput::Cursor(cursor)) => return self.read_page(cursor, &prepared),
            Ok(RequestInput::Accept(reference)) => {
                early.unwrap_or_else(|| self.accept(reference, &prepared))
            }
            Ok(RequestInput::Program { text, variant }) => {
                early.unwrap_or_else(|| self.compile(text, variant, &prepared))
            }
            Ok(RequestInput::Ast(document)) => {
                early.unwrap_or_else(|| self.bind(document, &prepared))
            }
            Ok(_) => Err(Error::new(ErrorCode::Arg, "unsupported request input")),
            Err(error) => Err(envelope_error(&error, request)),
        };
        let reports = self.report_context(prepared.config.config());
        let compiled = match compiled {
            Ok(compiled) => compiled,
            Err(error) => {
                let report = failure(&reports, &ProgramKind::input(request), None, error);
                return self.finish(report, &prepared, None, started);
            }
        };
        if let Some(response) =
            self.undo_by_inverse(request, &prepared, &compiled.bound, cancel, events.clone())
        {
            return response;
        }
        if prepared.elevation.is_none() && self.is_elevated_entry(&compiled.bound) {
            return self.undo_elevated(request, &prepared, cancel, events);
        }
        if prepared.elevation.is_none()
            && let Some(target) =
                self.bare_elevated_target(&compiled.bound, prepared.session.as_ref())
        {
            return self.refuse_bare(request, &prepared, target);
        }
        let kind = ProgramKind::of(&compiled.bound);
        if request.options.script != Some(true) && effect_pipelines(&compiled.bound) > 1 {
            let error = Error::new(
                ErrorCode::OneEffect,
                "the program has several pipelines that change the world; run them as separate calls or set `script: true`",
            );
            let report = failure(&reports, &kind, Some(&compiled), error);
            return self.finish(report, &prepared, None, started);
        }
        let program = ProgramRef::of(
            &compiled.bound.program,
            compiled.ran.clone().or_else(|| request.program.clone()),
        );
        let report = cancel
            .check()
            .and_then(|()| {
                let run = Run {
                    compiled: &compiled,
                    prepared: &prepared,
                    kind: &kind,
                    reports: &reports,
                    cancel,
                    events,
                    started,
                };
                self.run_program(run)
            })
            .unwrap_or_else(|error| (failure(&reports, &kind, Some(&compiled), error), Vec::new()));
        let (report, pipelines) = report;
        let mut response = self.finish(report, &prepared, Some(&program), started);
        attach_pipelines(&mut response, &pipelines, &prepared.options);
        response
    }

    /// Plans a bound program, executes the plan and assembles its report.
    fn run_program(&self, run: Run<'_>) -> Result<(Report, Vec<Report>), Error> {
        let Run {
            compiled,
            prepared,
            kind,
            reports,
            cancel,
            events,
            started,
        } = run;
        let clock = &self.inner.clock;
        let planning = clock.monotonic();
        let plan = self.plan(compiled, prepared)?;
        let planned = clock.monotonic();
        let params = self.script_params(compiled, prepared)?;
        let env = self
            .exec_env(prepared, cancel, events)?
            .with_params(params)
            .with_extension(Arc::new(compiled.bound.clone()));
        let env = match &compiled.ast_hash {
            Some(hash) => env.with_ast_hash(hash.clone()),
            None => env,
        };
        let outcome = Executor::new().run(&plan, &env);
        let timing = (prepared.options.verbosity == Some(Verbosity::Detailed)).then(|| {
            let mut timing = timing_of(&outcome);
            timing.parse_ms = Some(millis(planning.saturating_sub(started)));
            timing.plan_ms =
                Some(millis(planned.saturating_sub(planning)) + timing.plan_ms.unwrap_or(0));
            timing
        });
        report_of(reports, kind, compiled, outcome, timing)
    }

    /// Binds the values given for the parameters of the script to its
    /// `params` declaration, before anything runs. A program without
    /// declaration takes no values.
    fn script_params(
        &self,
        compiled: &Compiled,
        prepared: &Prepared,
    ) -> Result<Vec<(String, BoundExpr)>, Error> {
        let declared = compiled
            .bound
            .program
            .statements
            .iter()
            .find_map(|statement| match statement {
                BoundStatement::Params(params) => Some(params.params.as_slice()),
                _ => None,
            })
            .unwrap_or_default();
        self.inner.frontend.bind_params(
            declared,
            prepared.params.as_ref(),
            prepared.config.config(),
        )
    }

    /// Environment of the executor for one request.
    fn exec_env(
        &self,
        prepared: &Prepared,
        cancel: &CancelToken,
        events: Option<Arc<dyn EventSink>>,
    ) -> Result<ExecEnv, Error> {
        let fs = self
            .inner
            .fs
            .clone()
            .ok_or_else(|| unavailable("file system"))?;
        let registry = Arc::clone(&self.inner.registry);
        let clock = Arc::clone(&self.inner.clock);
        let host = Arc::clone(&self.inner.host);
        let base = match self.inner.journal.clone() {
            Some(journal) => ExecEnv::new(registry, fs, journal, clock, host),
            None => ExecEnv::read_only(registry, fs, clock, host),
        };
        let mut env = base
            .with_extension(Arc::clone(&self.inner.read_cache))
            .with_config(Arc::new(prepared.config.config().clone()))
            .with_options(Arc::new(prepared.options.clone()))
            .with_services(Arc::clone(&self.inner.services))
            .with_cancel(cancel.clone())
            .with_timeout(prepared.timeout);
        if let Some(access) = &self.inner.registry_access {
            env = env.with_extension(Arc::clone(access));
        }
        if let Some(mark) = &prepared.elevation {
            env = env.with_extension(Arc::new(nurt_contracts::stage::ElevatedRun {
                backend: mark.backend.as_str().to_owned(),
            }));
        }
        if let Some(events) = events {
            env = env.with_events(events);
        }
        if let Some(state) = &self.inner.state {
            env = env.with_state_dir(state.root().to_path_buf());
        }
        Ok(env)
    }

    /// Plans a bound program against the command registry. The fields
    /// requested by the caller reach the planner; the page size does not,
    /// because a kept result serves every later page.
    fn plan(&self, compiled: &Compiled, prepared: &Prepared) -> Result<Plan, Error> {
        let mut options = PlanOptions::new().with_corrections(compiled.corrections.clone());
        if let Some(fields) = &prepared.options.fields {
            options = options.with_fields(fields.iter().cloned());
        }
        nurt_plan::plan_with(&compiled.bound, &self.inner.registry, &options)
    }

    /// Context of the report rules for the configuration of one request.
    fn report_context(&self, config: &Config) -> ReportContext {
        ReportContext::new(
            config.report.clone(),
            PathContext::from_host(self.inner.host.as_ref()),
        )
        .with_scope(&config.scope)
    }

    /// Completes a report: durations, result file, the first page and the
    /// rendered result.
    fn finish(
        &self,
        mut report: Report,
        prepared: &Prepared,
        program: Option<&ProgramRef>,
        started: Duration,
    ) -> Response {
        if let Some(mark) = &prepared.elevation {
            mark.apply(&mut report);
        } else if report
            .error
            .as_ref()
            .is_some_and(|error| error.code() == ErrorCode::Privilege)
            && let Some(error) = report.error.take()
        {
            let config = prepared.config.config();
            let system = crate::elevation::system_of(self.inner.host.info().os);
            let backends =
                crate::elevation::backends_of(&config.privilege, system).unwrap_or_default();
            report.error = Some(crate::elevation::guide_privilege_error(
                error,
                &backends,
                &self.inner.elevation,
                config.privilege.allowed,
            ));
        }
        if !prepared.options.preview() {
            self.keep_inverse(&mut report);
        }
        if prepared.options.verbosity == Some(Verbosity::Detailed) {
            let elapsed = self.inner.clock.monotonic().saturating_sub(started);
            report.timing.get_or_insert_with(|| Timing {
                parse_ms: Some(millis(elapsed)),
                ..Timing::default()
            });
        }
        if has_items(&report)
            && let Some(path) = self.result_path(prepared)
        {
            match save_result(&path, &report) {
                Ok(()) => {
                    strip_items(&mut report);
                    report.result_file = Some(path);
                }
                Err(error) if report.error.is_none() => {
                    if matches!(
                        report.status,
                        ReportStatus::Ok | ReportStatus::Planned | ReportStatus::Running
                    ) {
                        report.status = ReportStatus::Failed;
                    }
                    report.error = Some(error.with_effect_applied(report.op.is_some()));
                }
                Err(_) => {}
            }
        }
        if let Some(program) = program {
            let limit = prepared.options.limit();
            report = self
                .inner
                .cursors
                .first_page(report, limit, program, self.now());
        }
        respond_with(&report, &prepared.options)
    }

    /// File that receives the full result: `save_to`, else a new file in
    /// `output.results_dir` when that key is set.
    fn result_path(&self, prepared: &Prepared) -> Option<String> {
        if let Some(path) = &prepared.options.save_to {
            return Some(path.clone());
        }
        let dir = prepared.config.config().output.results_dir.as_deref()?;
        let millis = self.now().unix_nanos() / 1_000_000;
        let serial = self.inner.results.fetch_add(1, Ordering::Relaxed) + 1;
        Some(format!(
            "{}/result-{millis}-{serial}.json",
            dir.trim_end_matches(['/', '\\'])
        ))
    }

    fn read_page(&self, cursor: &str, prepared: &Prepared) -> Response {
        let report = self
            .inner
            .cursors
            .page(cursor, prepared.options.limit(), self.now())
            .unwrap_or_else(|error| failed_report(PAGE_KIND, error));
        respond_with(&report, &prepared.options)
    }

    pub(crate) fn compile(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        prepared: &Prepared,
    ) -> Result<Compiled, Error> {
        self.inner
            .frontend
            .compile(text, variant, &request_config(prepared))
            .map_err(|mut error| {
                self.offer(&mut error, prepared);
                error
            })
    }

    pub(crate) fn bind(
        &self,
        document: &AstDocument,
        prepared: &Prepared,
    ) -> Result<Compiled, Error> {
        self.inner
            .frontend
            .bind(document, &request_config(prepared))
            .map(|bound| Compiled {
                bound,
                ast_hash: crate::frontend::ast_hash(document),
                corrections: Vec::new(),
                ran: None,
            })
    }

    pub(crate) fn accept(&self, reference: &str, prepared: &Prepared) -> Result<Compiled, Error> {
        let candidate =
            self.inner
                .candidates
                .accept(prepared.session.as_ref(), reference, self.now())?;
        let mut compiled = self.compile(&candidate.program, None, prepared)?;
        compiled.ran.get_or_insert(candidate.program);
        Ok(compiled)
    }

    /// Keeps the candidates of an ambiguous program for `accept`.
    fn offer(&self, error: &mut Error, prepared: &Prepared) {
        if let Err(failure) =
            self.inner
                .candidates
                .register(prepared.session.as_ref(), error, self.now())
        {
            error
                .details
                .insert("candidates_kept".to_owned(), json!(failure.message));
        }
    }

    pub(crate) fn prepare(&self, request: &Request) -> Result<Prepared, Error> {
        request
            .validate()
            .map_err(|error| envelope_error(&error, request))?;
        let config = self.config_for(request.options.base.as_deref())?;
        let session = self.session_for(&request.options, config.config())?;
        let options = effective_options(&request.options, config.config(), session.as_ref());
        let timeout = options
            .timeout
            .as_deref()
            .unwrap_or(DEFAULT_TIMEOUT)
            .parse::<DurationSetting>()
            .map_err(|error| Error::new(ErrorCode::Arg, error.message))?
            .as_std();
        if let Some(id) = &session {
            self.inner.sessions.touch(id, self.now());
        }
        Ok(Prepared {
            config,
            options,
            session,
            timeout,
            elevation: None,
            params: request.params.clone(),
        })
    }

    fn config_for(&self, base: Option<&str>) -> Result<Arc<LoadedConfig>, Error> {
        let project = self.inner.loader.project_for(base);
        let stamp = FileStamp::of(project.as_deref());
        if let Some(found) = self.projects().get(&project)
            && found.stamp == stamp
        {
            return Ok(Arc::clone(&found.loaded));
        }
        let loaded = Arc::new(
            self.inner
                .loader
                .load(project.as_deref())
                .map_err(|error| error.to_error())?,
        );
        self.projects().insert(
            project,
            CachedConfig {
                stamp,
                loaded: Arc::clone(&loaded),
            },
        );
        Ok(loaded)
    }

    fn session_for(
        &self,
        options: &RequestOptions,
        config: &Config,
    ) -> Result<Option<SessionId>, Error> {
        if let Some(id) = &options.session {
            return SessionId::new(id.clone()).map(Some);
        }
        if let Some(id) = &self.inner.default_session {
            return Ok(Some(id.clone()));
        }
        config.session.id.clone().map(SessionId::new).transpose()
    }

    fn build_context(&self, prepared: &Prepared, cancel: CancelToken) -> ExecCtx {
        let config = prepared.config.config();
        let mut limits = Limits::default();
        limits.max_records = config.limits.max_records;
        limits.max_file_bytes_text = config.limits.max_file_bytes_text.bytes();
        limits.threads = config.limits.threads.unwrap_or(self.inner.host.info().cpus);
        let mut extensions = Extensions::new();
        extensions.insert::<ReadCache>(Arc::clone(&self.inner.read_cache));
        if let Some(registry) = &self.inner.registry_access {
            extensions.insert::<dyn RegistryAccess>(Arc::clone(registry));
        }
        if let Some(mark) = &prepared.elevation {
            extensions.insert::<nurt_contracts::stage::ElevatedRun>(Arc::new(
                nurt_contracts::stage::ElevatedRun {
                    backend: mark.backend.as_str().to_owned(),
                },
            ));
        }
        let mut builder =
            ExecCtx::builder(Arc::clone(&self.inner.clock), Arc::clone(&self.inner.host))
                .config(Arc::new(config.clone()))
                .options(Arc::new(prepared.options.clone()))
                .cancel(cancel)
                .timeout(prepared.timeout)
                .limits(limits)
                .services(Arc::clone(&self.inner.services))
                .extensions(extensions);
        if let Some(fs) = &self.inner.fs {
            builder = builder.fs(Arc::clone(fs));
        }
        if let Some(journal) = &self.inner.journal {
            builder = builder.journal(Arc::clone(journal));
        }
        if let Some(state) = &self.inner.state {
            builder = builder.state_dir(state.root().to_path_buf());
        }
        builder.build()
    }

    fn journal(&self) -> Result<&dyn JournalStore, Error> {
        self.inner
            .journal
            .as_deref()
            .ok_or_else(|| unavailable("journal"))
    }

    fn projects(&self) -> MutexGuard<'_, HashMap<Option<PathBuf>, CachedConfig>> {
        self.inner
            .projects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn now(&self) -> WallTime {
        self.inner.clock.now()
    }

    fn next_request_id(&self) -> String {
        format!(
            "r{}",
            self.inner.requests.fetch_add(1, Ordering::Relaxed) + 1
        )
    }
}

/// Configuration that binds the program of a request.
///
/// The call parameter `base` of the request replaces `scope.base` of the
/// configuration, so relative paths of the program resolve against it.
pub(crate) fn request_config(prepared: &Prepared) -> Cow<'_, Config> {
    let config = prepared.config.config();
    match prepared.options.base.as_deref() {
        Some(base) if config.scope.base.as_deref() != Some(base) => {
            let mut copy = config.clone();
            copy.scope.base = Some(base.to_owned());
            Cow::Owned(copy)
        }
        _ => Cow::Borrowed(config),
    }
}

/// Options of a request with every default of the envelope and the
/// configuration applied.
fn effective_options(
    options: &RequestOptions,
    config: &Config,
    session: Option<&SessionId>,
) -> RequestOptions {
    RequestOptions {
        preview: Some(options.preview()),
        base: options.base.clone().or_else(|| config.scope.base.clone()),
        limit: Some(options.limit()),
        fields: options.fields.clone(),
        format: Some(options.format()),
        verbosity: Some(options.verbosity()),
        diff: Some(options.diff()),
        diff_context: Some(options.diff_context()),
        max_chars: Some(options.max_chars()),
        save_to: options.save_to.clone(),
        timeout: Some(
            options
                .timeout
                .clone()
                .unwrap_or_else(|| DEFAULT_TIMEOUT.to_owned()),
        ),
        session: session.map(|id| id.as_str().to_owned()),
        language: Some(options.language.unwrap_or_else(|| language_of(config))),
        script: Some(options.script()),
        privilege: Some(options.privilege.unwrap_or(config.privilege.default)),
        as_user: options.as_user.clone(),
    }
}

fn language_of(config: &Config) -> Language {
    match config.output.language {
        ConfigLanguage::Pl => Language::Pl,
        _ => Language::En,
    }
}

pub(crate) fn input_kind(request: &Request) -> &'static str {
    if request.cursor.is_some() && request.program.is_none() && request.ast.is_none() {
        PAGE_KIND
    } else if request.accept.is_some() && request.program.is_none() && request.ast.is_none() {
        ACCEPT_KIND
    } else {
        PROGRAM_KIND
    }
}

/// Report kind of a program and whether it changes targets.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProgramKind {
    kind: String,
    changing: bool,
}

impl ProgramKind {
    /// Kind of a request whose program is not bound: `program`, `accept` or
    /// `page`.
    fn input(request: &Request) -> Self {
        Self {
            kind: input_kind(request).to_owned(),
            changing: false,
        }
    }

    /// Kind of a bound program: its last effect command, or the source
    /// command of its first pipeline.
    fn of(bound: &BoundDocument) -> Self {
        let (kind, changing) = kind_of(bound);
        Self { kind, changing }
    }
}

/// Last effect command of a bound program, or the source command of its
/// first pipeline, and whether the program has an effect command.
fn kind_of(bound: &BoundDocument) -> (String, bool) {
    let mut pipelines = Vec::new();
    let mut construct = None;
    collect_pipelines(&bound.program.statements, &mut pipelines, &mut construct);
    let mut source = None;
    let mut effect = None;
    let mut calls = false;
    for pipeline in pipelines {
        let head = match &pipeline.head {
            BoundHead::Stage(stage) => Some(stage),
            BoundHead::Call(call) => {
                calls = true;
                source = source.or_else(|| Some(call.name.clone()));
                None
            }
            _ => None,
        };
        source = source.or_else(|| head.map(|stage| stage.name.clone()));
        if let Some(stage) = head
            .into_iter()
            .chain(&pipeline.stages)
            .rev()
            .find(|stage| stage.phase.is_effect())
        {
            effect = Some(stage.name.clone());
        }
    }
    if let Some((name, changes)) = construct {
        return (name.to_owned(), changes || effect.is_some());
    }
    let changing = effect.is_some() || (calls && statements_change(&bound.program.statements));
    let kind = effect.or(source).unwrap_or_else(|| PROGRAM_KIND.to_owned());
    (kind, changing)
}

/// The pipelines at the top of a program and the first control construct
/// with its name and whether it changes the world.
fn collect_pipelines<'b>(
    statements: &'b [BoundStatement],
    pipelines: &mut Vec<&'b BoundPipeline>,
    construct: &mut Option<(&'static str, bool)>,
) {
    for statement in statements {
        let name = match statement {
            BoundStatement::Pipeline(pipeline) => {
                pipelines.push(pipeline);
                continue;
            }
            BoundStatement::Let(binding) => {
                pipelines.push(&binding.value);
                continue;
            }
            BoundStatement::When(_) => "when",
            BoundStatement::Each(_) => "each",
            BoundStatement::Repeat(_) => "repeat",
            BoundStatement::While(_) => "while",
            BoundStatement::Steps(_) => "steps",
            _ => continue,
        };
        if construct.is_none() {
            *construct = Some((name, statements_change(std::slice::from_ref(statement))));
        }
    }
}

/// Whether a stage of the statements, nested ones included, changes the world
/// or runs a program.
fn statements_change(statements: &[BoundStatement]) -> bool {
    statements.iter().any(|statement| match statement {
        BoundStatement::Pipeline(pipeline) => pipeline_changes(pipeline),
        BoundStatement::Let(binding) => pipeline_changes(&binding.value),
        BoundStatement::When(construct) => {
            construct
                .branches
                .iter()
                .any(|branch| statements_change(&branch.body))
                || construct
                    .otherwise
                    .as_deref()
                    .is_some_and(statements_change)
        }
        BoundStatement::Each(construct) => statements_change(&construct.body),
        BoundStatement::Repeat(construct) => statements_change(&construct.body),
        BoundStatement::While(construct) => statements_change(&construct.body),
        BoundStatement::Steps(construct) => statements_change(&construct.body),
        BoundStatement::Define(construct) => statements_change(&construct.body),
        _ => false,
    })
}

fn pipeline_changes(pipeline: &BoundPipeline) -> bool {
    let head = match &pipeline.head {
        BoundHead::Stage(stage) => Some(stage),
        _ => None,
    };
    head.into_iter()
        .chain(&pipeline.stages)
        .any(|stage| stage.phase.is_effect())
}

/// Number of pipelines of a bound program that have an effect stage.
fn effect_pipelines(bound: &BoundDocument) -> usize {
    bound
        .program
        .statements
        .iter()
        .filter_map(|statement| match statement {
            BoundStatement::Pipeline(pipeline) => Some(pipeline),
            _ => None,
        })
        .filter(|pipeline| {
            let head = match &pipeline.head {
                BoundHead::Stage(stage) => Some(stage),
                _ => None,
            };
            head.into_iter()
                .chain(&pipeline.stages)
                .any(|stage| stage.phase.is_effect())
        })
        .count()
}

/// Effect kind of the last effect command of a bound program.
fn effect_kind_of(bound: &BoundDocument) -> Option<EffectKind> {
    let mut pipelines = Vec::new();
    nested_pipelines(&bound.program.statements, &mut pipelines);
    pipelines
        .into_iter()
        .filter_map(|pipeline| {
            let head = match &pipeline.head {
                BoundHead::Stage(stage) => Some(stage),
                _ => None,
            };
            head.into_iter()
                .chain(&pipeline.stages)
                .rev()
                .find(|stage| stage.phase.is_effect())
                .map(|stage| stage.effect)
        })
        .next_back()
}

/// Every pipeline of the statements in program order, those inside control
/// constructs and function bodies included.
fn nested_pipelines<'b>(statements: &'b [BoundStatement], found: &mut Vec<&'b BoundPipeline>) {
    for statement in statements {
        match statement {
            BoundStatement::Pipeline(pipeline) => found.push(pipeline),
            BoundStatement::Let(binding) => found.push(&binding.value),
            BoundStatement::When(construct) => {
                for branch in &construct.branches {
                    nested_pipelines(&branch.body, found);
                }
                if let Some(body) = &construct.otherwise {
                    nested_pipelines(body, found);
                }
            }
            BoundStatement::Each(construct) => nested_pipelines(&construct.body, found),
            BoundStatement::Repeat(construct) => nested_pipelines(&construct.body, found),
            BoundStatement::While(construct) => nested_pipelines(&construct.body, found),
            BoundStatement::Steps(construct) => nested_pipelines(&construct.body, found),
            BoundStatement::Define(construct) => nested_pipelines(&construct.body, found),
            _ => {}
        }
    }
}

/// Inputs of one program execution.
struct Run<'a> {
    compiled: &'a Compiled,
    prepared: &'a Prepared,
    kind: &'a ProgramKind,
    reports: &'a ReportContext,
    cancel: &'a CancelToken,
    events: Option<Arc<dyn EventSink>>,
    started: Duration,
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Durations of the execution steps: reading, planning and applying the
/// effect. The steps `check` and `after` count as reading and `verify` as
/// applying.
fn timing_of(outcome: &Outcome) -> Timing {
    let mut timing = Timing::default();
    for step in outcome.operations.iter().flat_map(|op| &op.timings) {
        let slot = match step.step {
            ExecStep::Read | ExecStep::Check | ExecStep::After => &mut timing.read_ms,
            ExecStep::Plan => &mut timing.plan_ms,
            ExecStep::Apply | ExecStep::Verify => &mut timing.apply_ms,
            _ => continue,
        };
        *slot = Some(slot.unwrap_or(0) + millis(step.elapsed));
    }
    timing
}

/// Page of records of a result: the records, or a single value as one `row`
/// record with the field `value`.
fn records_page(result: &ResultData, skip: &[RecordKind]) -> Option<RecordsPage> {
    let rows: Vec<Record> = match &result.value {
        Some(value) => vec![Record::new(RecordKind::Row).with("value", value.clone())],
        None => result
            .records
            .iter()
            .filter(|record| !skip.contains(&record.kind))
            .cloned()
            .collect(),
    };
    if rows.is_empty() && !skip.is_empty() && result.header.is_none() {
        return None;
    }
    let record_type = result
        .header
        .as_ref()
        .or_else(|| rows.first())
        .map_or(RecordKind::Entry, |record| record.kind);
    Some(RecordsPage {
        record_type,
        matched: Some(rows.len() as u64),
        header: result.header.clone(),
        rows,
    })
}

/// Kind and effect of every pipeline of a bound program, in program order.
fn pipeline_kinds(bound: &BoundDocument) -> Vec<ProgramKind> {
    bound
        .program
        .statements
        .iter()
        .filter_map(|statement| match statement {
            BoundStatement::Pipeline(pipeline) => {
                let head = match &pipeline.head {
                    BoundHead::Stage(stage) => Some(stage),
                    _ => None,
                };
                let effect = head
                    .into_iter()
                    .chain(&pipeline.stages)
                    .rev()
                    .find(|stage| stage.phase.is_effect())
                    .map(|stage| stage.name.clone());
                let changing = effect.is_some();
                let kind = effect
                    .or_else(|| head.map(|stage| stage.name.clone()))
                    .unwrap_or_else(|| PROGRAM_KIND.to_owned());
                Some(ProgramKind { kind, changing })
            }
            _ => None,
        })
        .collect()
}

/// Attaches the reports of the pipelines of a script to the response and
/// appends their rendered text to the text of the program.
fn attach_pipelines(response: &mut Response, pipelines: &[Report], options: &RequestOptions) {
    for report in pipelines {
        let part = respond_with(report, options);
        if let (Some(rendered), Some(extra)) = (response.rendered.as_mut(), part.rendered) {
            rendered.text.push('\n');
            rendered.text.push_str(&extra.text);
        }
        response.reports.push(part.report);
    }
}

/// Report of one pipeline of a script, with its own kind, status, error and
/// records.
fn pipeline_report(
    ctx: &ReportContext,
    program: &ProgramKind,
    operation: OperationOutcome,
) -> Result<Report, Error> {
    let status = operation.status;
    let error = operation.error.clone();
    let index = operation.index;
    let mut builder = if program.changing {
        ReportBuilder::changing(ctx, program.kind.clone(), status)
    } else {
        ReportBuilder::reading(ctx, program.kind.clone(), status)
    };
    builder = add_operation(builder, operation, Some(index));
    if let Some(error) = error {
        builder = builder.error(error);
    }
    builder.build().map_err(|violation| {
        Error::new(
            ErrorCode::Internal,
            format!("the report of a pipeline breaks a rule of the format: {violation}"),
        )
    })
}

/// Report of an executed program: the changes, notable and irreversible
/// effects of every operation, the error that ended the execution and, for
/// a program with a single operation, its records. A script with several
/// operations yields the report of the program without records and one
/// report per pipeline, in order, each with the records of its own pipeline.
fn report_of(
    ctx: &ReportContext,
    program: &ProgramKind,
    compiled: &Compiled,
    outcome: Outcome,
    timing: Option<Timing>,
) -> Result<(Report, Vec<Report>), Error> {
    let Outcome {
        status,
        error,
        operations,
        ..
    } = outcome;
    let mut pipelines = Vec::new();
    if operations.len() > 1 {
        let kinds = pipeline_kinds(&compiled.bound);
        for operation in &operations {
            let own = kinds.get(operation.index as usize).unwrap_or(program);
            pipelines.push(pipeline_report(ctx, own, operation.clone())?);
        }
    }
    let several = !pipelines.is_empty();
    let mut builder = if program.changing {
        ReportBuilder::changing(ctx, program.kind.clone(), status)
    } else {
        ReportBuilder::reading(ctx, program.kind.clone(), status)
    };
    if let Some(effect) = effect_kind_of(&compiled.bound) {
        builder = builder.effect(effect);
    }
    if !compiled.corrections.is_empty() {
        builder = builder.corrections(compiled.corrections.clone());
        if let Some(ran) = &compiled.ran {
            builder = builder.ran(ran.clone());
        }
    }
    if let Some(hash) = &compiled.ast_hash {
        builder = builder.ast_hash(hash.clone());
    }
    let last = operations
        .last()
        .map(|operation| operation.index)
        .filter(|_| !several);
    let parts: Vec<ReportPart> = operations
        .iter()
        .filter_map(|operation| operation.construct.as_ref())
        .flat_map(|construct| construct.parts.clone())
        .collect();
    for operation in operations {
        builder = add_operation(builder, operation, last);
    }
    if !parts.is_empty() {
        builder = builder.parts(parts);
    }
    if let Some(error) = error {
        builder = builder.error(error);
    }
    if let Some(timing) = timing {
        builder = builder.timing(timing);
    }
    let report = builder.build().map_err(|violation| {
        Error::new(
            ErrorCode::Internal,
            format!("the report of the program breaks a rule of the format: {violation}"),
        )
    })?;
    Ok((report, pipelines))
}

fn add_operation<'c>(
    mut builder: ReportBuilder<'c>,
    operation: OperationOutcome,
    last: Option<u32>,
) -> ReportBuilder<'c> {
    let OperationOutcome {
        index,
        result,
        effect,
        notable,
        ..
    } = operation;
    builder = builder.notes(notable);
    let is_last = last == Some(index);
    let mut skip: &[RecordKind] = &[];
    if let Some(effect) = effect {
        builder = builder
            .notes(effect.notable)
            .irreversible_entries(effect.irreversible)
            .changes(effect.changes.into_iter().map(|facts| {
                let target =
                    TargetChange::observed(facts.change, facts.entry, facts.replaced_existing);
                match facts.inside {
                    Some((files, directories)) => target.with_content(files, directories),
                    None => target,
                }
            }));
        if let Some(op) = effect.handle.as_ref().and_then(|handle| handle.op) {
            builder = builder.op(op.0);
        }
        skip = &[RecordKind::Effect, RecordKind::Exec, RecordKind::Job];
        if is_last {
            for record in &result.records {
                builder = match record.kind {
                    RecordKind::Exec => builder.exec(record.clone()),
                    RecordKind::Job => builder.job(record.clone()),
                    _ => builder,
                };
            }
        }
    }
    if is_last && let Some(page) = records_page(&result, skip) {
        builder = builder.records(page);
    }
    builder
}

/// Report of a program that failed before execution, assembled by the report
/// rules for a reading or a changing program, with the corrections applied
/// to its text.
fn failure(
    ctx: &ReportContext,
    program: &ProgramKind,
    compiled: Option<&Compiled>,
    error: Error,
) -> Report {
    let builder = if program.changing {
        ReportBuilder::changing(ctx, program.kind.clone(), ReportStatus::Failed)
    } else {
        ReportBuilder::reading(ctx, program.kind.clone(), ReportStatus::Failed)
    };
    let mut builder = builder.error(error.clone());
    if let Some(compiled) = compiled.filter(|compiled| !compiled.corrections.is_empty()) {
        builder = builder.corrections(compiled.corrections.clone());
        if let Some(ran) = &compiled.ran {
            builder = builder.ran(ran.clone());
        }
    }
    builder
        .build()
        .unwrap_or_else(|_| failed_report(&program.kind, error))
}

/// Response with the result rendered for the options of the request; the
/// messages and notes of the report are put into the requested language
/// first.
pub(crate) fn respond_with(report: &Report, options: &RequestOptions) -> Response {
    match options.language {
        Some(language @ Language::Pl) => {
            let mut localized = report.clone();
            nurt_help::translate::localize(&mut localized, language);
            respond(&localized, &RenderOptions::from_request(options))
        }
        _ => respond(report, &RenderOptions::from_request(options)),
    }
}

/// The error with its message in `language`.
fn localized(mut error: Error, language: Language) -> Error {
    nurt_help::translate::localize_error(&mut error, language);
    error
}

pub(crate) fn failed_report(kind: &str, error: Error) -> Report {
    let mut report = Report::new(ReportStatus::Failed, kind);
    report.error = Some(error);
    report
}

fn has_items(report: &Report) -> bool {
    !report.changes.is_empty()
        || report
            .records
            .as_ref()
            .is_some_and(|records| !records.rows.is_empty())
}

fn strip_items(report: &mut Report) {
    if let Some(records) = report.records.as_mut() {
        records.rows.clear();
    }
    report.changes.clear();
    report.diff.clear();
    if let Some(targets) = report.targets.as_mut()
        && targets.listed.is_some()
    {
        targets.listed = Some(0);
    }
}

fn save_result(path: &str, report: &Report) -> Result<(), Error> {
    let mut bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| Error::new(ErrorCode::Internal, error.to_string()))?;
    bytes.push(b'\n');
    write_atomic(std::path::Path::new(path), &bytes).map_err(|error| {
        Error::new(
            ErrorCode::Io,
            format!("cannot write the result to `{path}`: {error}"),
        )
        .with_found(path)
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use nurt_contracts::value::Value;
    use nurt_testkit::clock::VirtualClock;
    use nurt_testkit::host::TestHost;

    use super::*;

    fn engine(state: &std::path::Path) -> Engine {
        Engine::open(
            EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
                .with_sources(ConfigSources::isolated())
                .with_state_dir(state),
        )
        .unwrap()
    }

    fn removal(count: usize) -> Report {
        let changes: Vec<_> = (0..count)
            .map(|index| json!({"path": format!("/srv/build/f{index}.tmp"), "action": "delete"}))
            .collect();
        serde_json::from_value(json!({
            "nurt": "1.0", "status": "ok", "kind": "remove", "op": 3,
            "targets": {"count": count, "listed": count},
            "summary": {"files_deleted": count},
            "changes": changes
        }))
        .unwrap()
    }

    fn prepared(engine: &Engine, options: RequestOptions) -> Prepared {
        engine
            .prepare(&Request::program("remove").with_options(options))
            .unwrap()
    }

    #[test]
    fn reading_pages_never_runs_the_program_again() {
        let temp = tempfile::tempdir().unwrap();
        let engine = engine(&temp.path().join("state"));
        let runs = AtomicUsize::new(0);
        let run_removal = || {
            runs.fetch_add(1, Ordering::SeqCst);
            removal(120)
        };
        let options = RequestOptions {
            limit: Some(50),
            ..RequestOptions::default()
        };
        let program = ProgramRef::of(&json!(["remove"]), None);
        let first = engine.finish(
            run_removal(),
            &prepared(&engine, options.clone()),
            Some(&program),
            Duration::ZERO,
        );
        assert_eq!(first.report.changes.len(), 50);
        first.validate().unwrap();
        let cursor = first.report.next_cursor.clone().unwrap();
        let second = engine.page(&cursor, &options);
        let again = engine.page(&cursor, &options);
        assert_eq!(second, again);
        assert_eq!(second.report.changes.len(), 50);
        second.validate().unwrap();
        let third = engine.page(second.report.next_cursor.as_deref().unwrap(), &options);
        assert_eq!(third.report.changes.len(), 20);
        assert!(third.report.next_cursor.is_none());
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn saved_results_name_the_result_file() {
        let temp = tempfile::tempdir().unwrap();
        let engine = engine(&temp.path().join("state"));
        let file = temp.path().join("result.json");
        let options = RequestOptions {
            save_to: Some(file.to_string_lossy().into_owned()),
            verbosity: Some(Verbosity::Detailed),
            ..RequestOptions::default()
        };
        let response = engine.finish(
            removal(3),
            &prepared(&engine, options),
            Some(&ProgramRef::of(&json!(["remove"]), None)),
            Duration::ZERO,
        );
        let report = &response.report;
        assert_eq!(report.result_file.as_deref(), file.to_str());
        assert!(report.changes.is_empty());
        assert!(report.timing.is_some());
        response.validate().unwrap();
        let saved: Report = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(saved.changes.len(), 3);
    }

    #[test]
    fn unwritable_result_file_is_reported_in_the_report() {
        let temp = tempfile::tempdir().unwrap();
        let engine = engine(&temp.path().join("state"));
        let blocker = temp.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let options = RequestOptions {
            save_to: Some(blocker.join("result.json").to_string_lossy().into_owned()),
            ..RequestOptions::default()
        };
        let response = engine.finish(
            removal(2),
            &prepared(&engine, options),
            None,
            Duration::ZERO,
        );
        assert_eq!(response.report.error.as_ref().unwrap().code, ErrorCode::Io);
        assert_eq!(response.report.changes.len(), 2);
        assert_eq!(response.report.status, ReportStatus::Failed);
        assert!(response.report.error.as_ref().unwrap().effect_applied);
    }

    #[test]
    fn unwritable_result_file_fails_a_preview_that_changed_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let engine = engine(&temp.path().join("state"));
        let blocker = temp.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let options = RequestOptions {
            save_to: Some(blocker.join("result.json").to_string_lossy().into_owned()),
            ..RequestOptions::default()
        };
        let mut preview = removal(2);
        preview.status = ReportStatus::Planned;
        preview.op = None;
        let response = engine.finish(preview, &prepared(&engine, options), None, Duration::ZERO);
        assert_eq!(response.report.status, ReportStatus::Failed);
        let error = response.report.error.as_ref().unwrap();
        assert_eq!(error.code, ErrorCode::Io);
        assert!(!error.effect_applied);
    }

    #[test]
    fn single_value_is_listed_as_one_row() {
        let page = records_page(&ResultData::from_value(Value::Int(3)), &[]).unwrap();
        assert_eq!(page.record_type, RecordKind::Row);
        assert_eq!(page.matched, Some(1));
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].get("value"), Some(&Value::Int(3)));
    }

    #[test]
    fn records_of_the_effect_are_left_out_of_the_rows() {
        let mut result = ResultData::from_records(vec![Record::new(RecordKind::Effect)]);
        assert!(records_page(&result, &[RecordKind::Effect]).is_none());
        result.header = Some(Record::new(RecordKind::TestRun));
        let page = records_page(&result, &[RecordKind::Effect]).unwrap();
        assert_eq!(page.record_type, RecordKind::TestRun);
        assert!(page.rows.is_empty());
    }

    #[test]
    fn report_kind_of_a_request_without_a_program_follows_its_input() {
        assert_eq!(input_kind(&Request::cursor("c_1")), PAGE_KIND);
        assert_eq!(input_kind(&Request::accept("c1")), ACCEPT_KIND);
        assert_eq!(input_kind(&Request::program("find")), PROGRAM_KIND);
    }

    fn engine_with(overrides: &str, state: &std::path::Path) -> Engine {
        Engine::open(
            EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
                .with_sources(
                    ConfigSources::isolated()
                        .with_overrides(crate::config::toml_table(overrides, "test").unwrap()),
                )
                .with_state_dir(state),
        )
        .unwrap()
    }

    #[test]
    fn results_dir_receives_the_full_result() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("results");
        let overrides = format!(
            "[output]\nresults_dir = {:?}\n",
            dir.to_string_lossy().replace('\\', "/")
        );
        let engine = engine_with(&overrides, &temp.path().join("state"));
        let first = engine.finish(
            removal(3),
            &prepared(&engine, RequestOptions::default()),
            None,
            Duration::ZERO,
        );
        let second = engine.finish(
            removal(2),
            &prepared(&engine, RequestOptions::default()),
            None,
            Duration::ZERO,
        );
        let named = |response: &Response| response.report.result_file.clone().unwrap();
        assert_ne!(named(&first), named(&second));
        assert!(first.report.changes.is_empty());
        first.validate().unwrap();
        let saved: Report = serde_json::from_slice(&std::fs::read(named(&first)).unwrap()).unwrap();
        assert_eq!(saved.changes.len(), 3);
        assert!(std::path::Path::new(&named(&second)).starts_with(&dir));
    }

    #[test]
    fn save_to_takes_precedence_over_results_dir() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("results");
        let overrides = format!(
            "[output]\nresults_dir = {:?}\n",
            dir.to_string_lossy().replace('\\', "/")
        );
        let engine = engine_with(&overrides, &temp.path().join("state"));
        let file = temp.path().join("chosen.json");
        let options = RequestOptions {
            save_to: Some(file.to_string_lossy().into_owned()),
            ..RequestOptions::default()
        };
        let response = engine.finish(
            removal(2),
            &prepared(&engine, options),
            None,
            Duration::ZERO,
        );
        assert_eq!(response.report.result_file.as_deref(), file.to_str());
        assert!(!dir.exists());
    }

    #[test]
    fn results_without_items_are_not_saved_to_results_dir() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("results");
        let overrides = format!(
            "[output]\nresults_dir = {:?}\n",
            dir.to_string_lossy().replace('\\', "/")
        );
        let engine = engine_with(&overrides, &temp.path().join("state"));
        let response = engine.finish(
            removal(0),
            &prepared(&engine, RequestOptions::default()),
            None,
            Duration::ZERO,
        );
        assert!(response.report.result_file.is_none());
        assert!(!dir.exists());
    }

    #[test]
    fn changed_project_file_reloads_the_cached_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let file = project.join(nurt_contracts::config::PROJECT_FILE_NAME);
        std::fs::write(&file, "[report]\nmass_threshold = 11\n").unwrap();
        let engine = Engine::open(
            EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
                .with_sources(ConfigSources::isolated().with_project_dir(&project))
                .without_state_dir(),
        )
        .unwrap();
        let threshold = |engine: &Engine| {
            engine
                .config_for(None)
                .unwrap()
                .config()
                .report
                .mass_threshold
        };
        assert_eq!(threshold(&engine), 11);
        std::fs::write(&file, "[report]\nmass_threshold = 222\n").unwrap();
        assert_eq!(threshold(&engine), 222);
        std::fs::remove_file(&file).unwrap();
        assert_ne!(threshold(&engine), 222);
    }

    #[test]
    fn failures_before_execution_follow_the_report_rules() {
        let ctx = ReportContext::default();
        let compiled = Compiled {
            bound: nurt_testkit::fixtures::empty_program(),
            corrections: vec![
                serde_json::from_value(json!({
                    "id": "k1", "rule": "L01", "class": "F", "from": "fnd", "to": "find"
                }))
                .unwrap(),
            ],
            ran: Some("find \"/srv\"".to_owned()),
            ast_hash: None,
        };
        let changing = ProgramKind {
            kind: "remove".to_owned(),
            changing: true,
        };
        let error = Error::new(ErrorCode::OneEffect, "two effect stages");
        let report = failure(&ctx, &changing, Some(&compiled), error.clone());
        assert_eq!(report.status, ReportStatus::Failed);
        assert_eq!(report.kind, "remove");
        assert_eq!(report.error.as_ref(), Some(&error));
        assert_eq!(report.ran.as_deref(), Some("find \"/srv\""));
        assert_eq!(report.corrections.len(), 1);
        assert!(report.notable.is_empty());
        let reading = failure(&ctx, &ProgramKind::of(&compiled.bound), None, error);
        assert_eq!(reading.kind, PROGRAM_KIND);
        assert!(reading.corrections.is_empty());
    }

    #[test]
    fn polish_responses_carry_polish_messages_and_unchanged_codes() {
        let report = failed_report(
            "find",
            Error::new(ErrorCode::OneEffect, "two effect stages").with_found("remove"),
        );
        let polish = RequestOptions {
            language: Some(Language::Pl),
            ..RequestOptions::default()
        };
        let error = respond_with(&report, &polish).report.error.unwrap();
        assert_eq!(error.code(), ErrorCode::OneEffect);
        assert!(error.message.contains("etap"), "{}", error.message);
        assert_eq!(error.details["message_en"], json!("two effect stages"));
        let english = respond_with(&report, &RequestOptions::default());
        assert_eq!(english.report.error.unwrap().message, "two effect stages");
    }

    #[test]
    fn report_kind_is_the_effect_command() {
        let bound: BoundDocument = serde_json::from_value(json!({
            "nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
            "program": {"statements": [{
                "t": "pipeline",
                "head": {"t": "stage", "name": "find", "effect": "read", "phase": "source", "args": []},
                "stages": [{"t": "stage", "name": "remove", "effect": "local", "phase": "mutate", "args": []}]
            }]}
        }))
        .unwrap();
        assert_eq!(kind_of(&bound), ("remove".to_owned(), true));
        assert_eq!(
            kind_of(&nurt_testkit::fixtures::empty_program()),
            (PROGRAM_KIND.to_owned(), false)
        );
    }

    #[test]
    fn effect_pipelines_counts_the_pipelines_with_an_effect_stage() {
        let stage = |name: &str, effect: &str, phase: &str| json!({"t": "stage", "name": name, "effect": effect, "phase": phase, "args": []});
        let pipeline = |head: serde_json::Value, stages: Vec<serde_json::Value>| json!({"t": "pipeline", "head": head, "stages": stages});
        let bound = |statements: Vec<serde_json::Value>| -> BoundDocument {
            serde_json::from_value(json!({
                "nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
                "program": {"statements": statements}
            }))
            .unwrap()
        };
        let read = || pipeline(stage("find", "read", "source"), Vec::new());
        let change = || {
            pipeline(
                stage("find", "read", "source"),
                vec![stage("remove", "local", "mutate")],
            )
        };
        assert_eq!(effect_pipelines(&bound(vec![read(), read()])), 0);
        assert_eq!(effect_pipelines(&bound(vec![read(), change()])), 1);
        assert_eq!(effect_pipelines(&bound(vec![change(), change()])), 2);
    }

    #[test]
    fn the_effect_kind_is_the_one_of_the_effect_command() {
        let bound: BoundDocument = serde_json::from_value(json!({
            "nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
            "program": {"statements": [{
                "t": "pipeline",
                "head": {"t": "stage", "name": "find", "effect": "read", "phase": "source", "args": []},
                "stages": [{"t": "stage", "name": "run", "effect": "process", "phase": "run", "args": []}]
            }]}
        }))
        .unwrap();
        assert_eq!(effect_kind_of(&bound), Some(EffectKind::Process));
        assert_eq!(
            effect_kind_of(&nurt_testkit::fixtures::empty_program()),
            None
        );
    }
}
