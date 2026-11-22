// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Environment of one execution.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nurt_catalog::Registry;
use nurt_contracts::config::Config;
use nurt_contracts::env::{Clock, Host};
use nurt_contracts::envelope::{DEFAULT_TIMEOUT, RequestOptions};
use nurt_contracts::error::{Error, ErrorCode, Result, Stage};
use nurt_contracts::fs::Fs;
use nurt_contracts::journal::JournalStore;
use nurt_contracts::services::Services;
use nurt_contracts::stage::{CancelToken, DiscardEvents, EventSink, Extensions, Limits};
use rayon::ThreadPool;

/// Adds one object to the extensions of a stage context.
type ExtensionInstaller = Arc<dyn Fn(&mut Extensions) + Send + Sync>;

/// Everything an execution may use: the registry of commands, the world
/// (file system, journal, clock, host, services), the request (configuration,
/// options, cancellation, time limit) and the receiver of events.
///
/// The stage contexts of the execution are built from this environment with
/// [`nurt_contracts::stage::ExecCtxBuilder`]; stages never see the
/// environment itself.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use std::time::Duration;
/// use nurt_catalog::Registry;
/// use nurt_exec::ExecEnv;
/// use nurt_testkit::{clock::VirtualClock, host::TestHost, journal::NoopJournal, vfs::VirtualFs};
///
/// let clock = Arc::new(VirtualClock::new());
/// let fs = Arc::new(VirtualFs::new(clock.clone()));
/// let env = ExecEnv::new(
///     Arc::new(Registry::load_embedded()?),
///     fs.clone(),
///     Arc::new(NoopJournal::new(fs)),
///     clock,
///     Arc::new(TestHost::linux()),
/// )
/// .with_timeout(Duration::from_secs(30));
/// assert_eq!(env.resolved_timeout()?, Some(Duration::from_secs(30)));
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[non_exhaustive]
pub struct ExecEnv {
    /// Command registry with the stage implementations.
    pub registry: Arc<Registry>,
    /// Read-only file system.
    pub fs: Arc<dyn Fs>,
    /// Operation journal; the only path for changes. Absent in an
    /// environment for programs that only read.
    pub journal: Option<Arc<dyn JournalStore>>,
    /// Clock.
    pub clock: Arc<dyn Clock>,
    /// Host facts.
    pub host: Arc<dyn Host>,
    /// Effective configuration.
    pub config: Arc<Config>,
    /// Options of the request.
    pub options: Arc<RequestOptions>,
    /// Services of other command groups.
    pub services: Arc<Services>,
    /// Cancellation of the execution.
    pub cancel: CancelToken,
    /// Time limit of the program; when absent, the `timeout` option of the
    /// request, and without it the default time limit of requests.
    pub timeout: Option<Duration>,
    /// Receiver of the events of the execution.
    pub events: Arc<dyn EventSink>,
    /// State directory of the engine.
    pub state_dir: Option<PathBuf>,
    /// Thread pool shared by executions; when absent and the configuration
    /// allows more than one thread, the execution creates its own.
    pub pool: Option<Arc<ThreadPool>>,
    /// Hash of the executed program, reported in [`nurt_contracts::stage::Event::ExecStarted`].
    pub ast_hash: Option<String>,
    /// Seed of `random()` and `uuid()`; entropy of the process when absent.
    pub seed: Option<u64>,
    extensions: Vec<ExtensionInstaller>,
}

impl ExecEnv {
    /// Environment with the default configuration and request options, no
    /// services, a new cancellation token and an event sink that drops
    /// events.
    #[must_use]
    pub fn new(
        registry: Arc<Registry>,
        fs: Arc<dyn Fs>,
        journal: Arc<dyn JournalStore>,
        clock: Arc<dyn Clock>,
        host: Arc<dyn Host>,
    ) -> Self {
        Self::build(registry, fs, Some(journal), clock, host)
    }

    /// An environment without a journal, for programs that only read.
    ///
    /// An effect stage fails with `E_STAGE_FAILED` at the apply step; a
    /// preview, which ends after the plan of the effect, runs.
    #[must_use]
    pub fn read_only(
        registry: Arc<Registry>,
        fs: Arc<dyn Fs>,
        clock: Arc<dyn Clock>,
        host: Arc<dyn Host>,
    ) -> Self {
        Self::build(registry, fs, None, clock, host)
    }

    fn build(
        registry: Arc<Registry>,
        fs: Arc<dyn Fs>,
        journal: Option<Arc<dyn JournalStore>>,
        clock: Arc<dyn Clock>,
        host: Arc<dyn Host>,
    ) -> Self {
        Self {
            registry,
            fs,
            journal,
            clock,
            host,
            config: Arc::default(),
            options: Arc::default(),
            services: Arc::default(),
            cancel: CancelToken::new(),
            timeout: None,
            events: Arc::new(DiscardEvents),
            state_dir: None,
            pool: None,
            ast_hash: None,
            seed: None,
            extensions: Vec::new(),
        }
    }

    /// Sets the configuration.
    #[must_use]
    pub fn with_config(mut self, config: Arc<Config>) -> Self {
        self.config = config;
        self
    }

    /// Sets the request options.
    #[must_use]
    pub fn with_options(mut self, options: Arc<RequestOptions>) -> Self {
        self.options = options;
        self
    }

    /// Sets the services of other command groups.
    #[must_use]
    pub fn with_services(mut self, services: Arc<Services>) -> Self {
        self.services = services;
        self
    }

    /// Sets the cancellation token.
    #[must_use]
    pub fn with_cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// Sets the time limit of the program.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the receiver of events.
    #[must_use]
    pub fn with_events(mut self, events: Arc<dyn EventSink>) -> Self {
        self.events = events;
        self
    }

    /// Sets the state directory.
    #[must_use]
    pub fn with_state_dir(mut self, dir: PathBuf) -> Self {
        self.state_dir = Some(dir);
        self
    }

    /// Sets the shared thread pool.
    #[must_use]
    pub fn with_pool(mut self, pool: Arc<ThreadPool>) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Adds an object to the extensions of the stage contexts, for example a
    /// cache that stages of other command groups read with
    /// [`nurt_contracts::stage::ExecCtx::extensions`].
    ///
    /// An object of a type that was already added replaces the earlier one.
    #[must_use]
    pub fn with_extension<T: ?Sized + Send + Sync + 'static>(mut self, object: Arc<T>) -> Self {
        self.extensions
            .push(Arc::new(move |extensions: &mut Extensions| {
                extensions.insert(Arc::clone(&object));
            }));
        self
    }

    /// Registers the objects added with [`ExecEnv::with_extension`].
    pub(crate) fn fill_extensions(&self, extensions: &mut Extensions) {
        for add in &self.extensions {
            add(extensions);
        }
    }

    /// Sets the hash of the executed program: `sha256:` and 64 lowercase
    /// hexadecimal digits, computed by the caller from the bound tree
    /// (`nurt_contracts::ast::strip_for_hash`); the executor reports it in
    /// the first event and does not compute it.
    #[must_use]
    pub fn with_ast_hash(mut self, hash: impl Into<String>) -> Self {
        self.ast_hash = Some(hash.into());
        self
    }

    /// Sets the seed of `random()` and `uuid()`.
    #[must_use]
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Returns `true` when the request asks for a preview.
    #[must_use]
    pub fn preview(&self) -> bool {
        self.options.preview == Some(true)
    }

    /// Time limit of the program: [`ExecEnv::timeout`], else the `timeout`
    /// option of the request, else the default of requests.
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` in stage `1` when the `timeout` option is
    /// not a duration.
    pub fn resolved_timeout(&self) -> Result<Option<Duration>> {
        if let Some(timeout) = self.timeout {
            return Ok(Some(timeout));
        }
        let text = self.options.timeout.as_deref().unwrap_or(DEFAULT_TIMEOUT);
        let duration = nurt_types::parse_duration(text).map_err(|error| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("the timeout `{text}` is not a duration: {error}"),
            )
            .with_stage(Stage::Bind)
            .with_found(text)
        })?;
        Ok(duration.to_std())
    }

    /// Resource limits of the stages from the configuration: the record
    /// limit, the largest text file and the number of threads (one when the
    /// configuration leaves it open).
    #[must_use]
    pub fn limits(&self) -> Limits {
        let config = &self.config.limits;
        Limits {
            max_records: config.max_records,
            max_file_bytes_text: config.max_file_bytes_text.bytes(),
            threads: config.threads.unwrap_or(Limits::default().threads).max(1),
        }
    }
}

impl fmt::Debug for ExecEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecEnv")
            .field("registry", &self.registry.len())
            .field("options", &self.options)
            .field("cancel", &self.cancel)
            .field("timeout", &self.timeout)
            .field("state_dir", &self.state_dir)
            .field("pool", &self.pool.is_some())
            .field("ast_hash", &self.ast_hash)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use nurt_testkit::clock::VirtualClock;
    use nurt_testkit::host::TestHost;
    use nurt_testkit::journal::NoopJournal;
    use nurt_testkit::vfs::VirtualFs;

    use super::*;

    fn env() -> ExecEnv {
        let clock = Arc::new(VirtualClock::new());
        let fs = Arc::new(VirtualFs::new(clock.clone()));
        ExecEnv::new(
            Arc::new(Registry::load_embedded().unwrap()),
            fs.clone(),
            Arc::new(NoopJournal::new(fs)),
            clock,
            Arc::new(TestHost::linux()),
        )
    }

    #[test]
    fn timeout_falls_back_to_the_request_option_and_the_default() {
        let default = env().resolved_timeout().unwrap();
        assert_eq!(default, Some(Duration::from_secs(120)));
        let options = RequestOptions {
            timeout: Some("2s".to_owned()),
            ..RequestOptions::default()
        };
        let env = env().with_options(Arc::new(options));
        assert_eq!(
            env.resolved_timeout().unwrap(),
            Some(Duration::from_secs(2))
        );
    }

    #[test]
    fn malformed_timeout_is_an_invalid_value() {
        let options = RequestOptions {
            timeout: Some("soon".to_owned()),
            ..RequestOptions::default()
        };
        let error = env()
            .with_options(Arc::new(options))
            .resolved_timeout()
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.stage(), Some(Stage::Bind));
    }

    #[test]
    fn limits_follow_the_configuration() {
        let mut config = Config::default();
        config.limits.max_records = 7;
        config.limits.threads = Some(4);
        let env = env()
            .with_config(Arc::new(config))
            .with_seed(3)
            .with_ast_hash("sha256:00")
            .with_state_dir(PathBuf::from("/state"));
        let limits = env.limits();
        assert_eq!(limits.max_records, 7);
        assert_eq!(limits.threads, 4);
        assert!(!env.preview());
        assert!(format!("{env:?}").contains("sha256:00"));
    }
}
