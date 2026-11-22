// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Pool of language servers kept between commands.
//!
//! The pool holds one server per pair of server program and project root;
//! TypeScript and JavaScript use the same program and therefore share a
//! server. A server is started in the background and initialized within the
//! largest command time limit, so a command that stops waiting leaves the
//! server warming up for the next command; concurrent commands share one
//! start. A server unused for the idle time is stopped, the least recently
//! used server is stopped when the pool reaches its limit, and after a failed
//! start the same server is not started again before the retry period ends.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Sender, bounded};
use nurt_contracts::env::Clock;
use nurt_contracts::services::ProcessHost;
use nurt_contracts::stage::{CancelReason, CancelToken};

use crate::config::{self, ClientSettings, ResolvedCommand};
use crate::server::{LanguageServer, LspError, ServerSpec};

/// Longest single wait between checks of the cancellation token.
const CANCEL_POLL: Duration = Duration::from_millis(25);

/// Interval of the background check for idle servers.
const REAP_INTERVAL: Duration = Duration::from_secs(15);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PoolKey {
    program: String,
    root: PathBuf,
}

enum SlotState {
    Starting,
    Ready(LanguageServer),
    Failed(LspError),
}

struct Slot {
    name: String,
    root: PathBuf,
    state: Mutex<SlotState>,
    changed: Condvar,
    languages: Mutex<BTreeSet<String>>,
    last_used: Mutex<Duration>,
    requests: AtomicU64,
    start_cancel: CancelToken,
}

impl Slot {
    fn server(&self) -> Option<LanguageServer> {
        match &*lock(&self.state) {
            SlotState::Ready(server) => Some(server.clone()),
            SlotState::Starting | SlotState::Failed(_) => None,
        }
    }

    fn is_dead(&self) -> bool {
        match &*lock(&self.state) {
            SlotState::Ready(server) => server.is_closed(),
            SlotState::Failed(_) => true,
            SlotState::Starting => false,
        }
    }

    fn stop(&self) {
        self.start_cancel.cancel(CancelReason::Shutdown);
        if let Some(server) = self.server() {
            server.shutdown();
        }
    }
}

/// State of one server of the pool, for `code.languages`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStatus {
    /// File name of the server program.
    pub server: String,
    /// Project root.
    pub root: PathBuf,
    /// Languages served so far.
    pub languages: Vec<String>,
    /// Process identifier, once started.
    pub pid: Option<u32>,
    /// The server answered `initialize`.
    pub ready: bool,
    /// The server reports work in progress (project loading, indexing).
    pub loading: bool,
    /// Number of commands that used the server.
    pub requests: u64,
    /// Time since the last use.
    pub idle: Duration,
}

/// What a command asks of the pool.
#[derive(Debug, Clone, Copy)]
pub struct AcquireRequest<'a> {
    /// Language of the file.
    pub language: &'a str,
    /// Project root.
    pub root: &'a Path,
    /// Longest wait for a server that is starting.
    pub timeout: Duration,
    /// Ends the wait early.
    pub cancel: &'a CancelToken,
}

#[derive(Default)]
struct PoolState {
    entries: HashMap<PoolKey, Arc<Slot>>,
    failures: HashMap<PoolKey, (Duration, LspError)>,
}

struct PoolInner {
    host: Arc<dyn ProcessHost>,
    clock: Arc<dyn Clock>,
    settings: ClientSettings,
    state: Mutex<PoolState>,
}

impl Drop for PoolInner {
    fn drop(&mut self) {
        let slots: Vec<Arc<Slot>> = lock(&self.state)
            .entries
            .drain()
            .map(|(_, slot)| slot)
            .collect();
        for slot in slots {
            slot.stop();
        }
    }
}

/// Pool of running language servers.
///
/// The pool stops its servers when it is dropped.
pub struct ServerPool {
    inner: Arc<PoolInner>,
    _reaper: Option<Sender<()>>,
}

impl std::fmt::Debug for ServerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerPool")
            .field("servers", &lock(&self.inner.state).entries.len())
            .finish_non_exhaustive()
    }
}

impl ServerPool {
    /// Pool that starts servers through `host` and measures idle and retry
    /// periods on `clock`. A background thread stops idle servers.
    #[must_use]
    pub fn new(
        host: Arc<dyn ProcessHost>,
        clock: Arc<dyn Clock>,
        settings: ClientSettings,
    ) -> Self {
        let inner = Arc::new(PoolInner {
            host,
            clock,
            settings,
            state: Mutex::new(PoolState::default()),
        });
        let reaper = spawn_reaper(Arc::downgrade(&inner));
        Self {
            inner,
            _reaper: reaper,
        }
    }

    /// Settings of the pool.
    #[must_use]
    pub fn settings(&self) -> &ClientSettings {
        &self.inner.settings
    }

    /// Process host used to start servers.
    #[must_use]
    pub fn host(&self) -> &dyn ProcessHost {
        self.inner.host.as_ref()
    }

    /// Resolved command of the server of `language`, without starting it.
    ///
    /// # Errors
    ///
    /// Fails with [`LspError::Missing`] when no server is configured or
    /// allowed for the language or its program is not found.
    pub fn command_for(&self, language: &str, root: &Path) -> Result<ResolvedCommand, LspError> {
        let settings = &self.inner.settings;
        let command = settings
            .command(language)
            .ok_or_else(|| LspError::Missing {
                server: String::new(),
                message: format!("no language server is configured for {language}"),
            })?;
        let program = command.first().map(String::as_str).unwrap_or_default();
        if !settings.allows(language, program) {
            return Err(LspError::Missing {
                server: config::program_name(program),
                message: format!(
                    "the language server of {language} is not allowed by code.servers_allow"
                ),
            });
        }
        config::resolve_command(self.inner.host.as_ref(), command, root)
    }

    /// Returns a running server for the language and root, starting one
    /// when needed, and waits for its initialization.
    ///
    /// # Errors
    ///
    /// Fails with [`LspError::Missing`] (no server), [`LspError::Unavailable`]
    /// (start failed now or within the retry period), [`LspError::Timeout`]
    /// (initialization not finished within `request.timeout`; the server
    /// keeps starting), [`LspError::Cancelled`] or the error of a server that
    /// ended during initialization.
    pub fn acquire(&self, request: &AcquireRequest<'_>) -> Result<LanguageServer, LspError> {
        let command = self.command_for(request.language, request.root)?;
        let key = PoolKey {
            program: command.program.clone(),
            root: request.root.to_path_buf(),
        };
        let slot = self.slot(&key, &command, request)?;
        lock(&slot.languages).insert(config::normalize_language(request.language));
        wait_ready(&slot, request.timeout, request.cancel)
    }

    fn slot(
        &self,
        key: &PoolKey,
        command: &ResolvedCommand,
        request: &AcquireRequest<'_>,
    ) -> Result<Arc<Slot>, LspError> {
        let inner = &self.inner;
        let now = inner.clock.monotonic();
        let mut stopped = reap(inner, now);
        let mut state = lock(&inner.state);
        if let Some(slot) = state
            .entries
            .get(key)
            .filter(|slot| !slot.is_dead())
            .cloned()
        {
            *lock(&slot.last_used) = now;
            slot.requests.fetch_add(1, Ordering::Relaxed);
            drop(state);
            stop_all(stopped);
            return Ok(slot);
        }
        state.entries.remove(key);
        if let Some((until, error)) = state.failures.get(key)
            && *until > now
        {
            let error = error.clone();
            drop(state);
            stop_all(stopped);
            return Err(error);
        }
        state.failures.remove(key);
        stopped.extend(evict(
            &mut state,
            inner.settings.max_servers.saturating_sub(1),
        ));
        let slot = Arc::new(Slot {
            name: command.name.clone(),
            root: request.root.to_path_buf(),
            state: Mutex::new(SlotState::Starting),
            changed: Condvar::new(),
            languages: Mutex::new(BTreeSet::new()),
            last_used: Mutex::new(now),
            requests: AtomicU64::new(1),
            start_cancel: CancelToken::new(),
        });
        state.entries.insert(key.clone(), Arc::clone(&slot));
        drop(state);
        stop_all(stopped);
        self.start(key.clone(), command, request.language, &slot);
        Ok(slot)
    }

    fn start(&self, key: PoolKey, command: &ResolvedCommand, language: &str, slot: &Arc<Slot>) {
        let inner = Arc::clone(&self.inner);
        let init_options = config::init_options(
            &inner.settings,
            language,
            &command.name,
            &key.root,
            inner.host.as_ref(),
        );
        let spec = ServerSpec {
            name: command.name.clone(),
            program: command.program.clone(),
            args: command.args.clone(),
            root: key.root.clone(),
            init_options,
            env: Vec::new(),
        };
        let thread_slot = Arc::clone(slot);
        let started = thread::Builder::new()
            .name(format!("lsp-{}-start", command.name))
            .spawn(move || run_start(&inner, &key, &spec, &thread_slot));
        if let Err(error) = started {
            set_state(
                slot,
                SlotState::Failed(LspError::Unavailable {
                    server: command.name.clone(),
                    message: format!("cannot start language server {}: {error}", command.name),
                }),
            );
        }
    }

    /// Stops servers unused for the idle time; returns how many stopped.
    pub fn reap_idle(&self) -> usize {
        let stopped = reap(&self.inner, self.inner.clock.monotonic());
        let count = stopped.len();
        stop_all(stopped);
        count
    }

    /// State of every server, ordered by server name and root.
    #[must_use]
    pub fn status(&self) -> Vec<ServerStatus> {
        let now = self.inner.clock.monotonic();
        let slots: Vec<Arc<Slot>> = lock(&self.inner.state).entries.values().cloned().collect();
        let mut statuses: Vec<ServerStatus> = slots
            .iter()
            .filter(|slot| !slot.is_dead())
            .map(|slot| slot_status(slot, now))
            .collect();
        statuses
            .sort_by(|left, right| (&left.server, &left.root).cmp(&(&right.server, &right.root)));
        statuses
    }

    /// Stops the servers whose program name or root equals `filter`, or all
    /// servers (and forgets failed starts) without a filter. Returns how
    /// many stopped.
    pub fn stop(&self, filter: Option<&str>) -> usize {
        let mut state = lock(&self.inner.state);
        let keys: Vec<PoolKey> = state
            .entries
            .iter()
            .filter(|(_, slot)| {
                filter.is_none_or(|filter| slot.name == filter || slot.root == Path::new(filter))
            })
            .map(|(key, _)| key.clone())
            .collect();
        let slots: Vec<Arc<Slot>> = keys
            .iter()
            .filter_map(|key| state.entries.remove(key))
            .collect();
        if filter.is_none() {
            state.failures.clear();
        }
        drop(state);
        let count = slots.len();
        stop_all(slots);
        count
    }
}

fn slot_status(slot: &Slot, now: Duration) -> ServerStatus {
    let server = slot.server();
    ServerStatus {
        server: slot.name.clone(),
        root: slot.root.clone(),
        languages: lock(&slot.languages).iter().cloned().collect(),
        pid: server.as_ref().map(LanguageServer::pid),
        ready: server.is_some(),
        loading: server
            .as_ref()
            .is_some_and(|server| server.progress().busy()),
        requests: slot.requests.load(Ordering::Relaxed),
        idle: now.saturating_sub(*lock(&slot.last_used)),
    }
}

fn spawn_reaper(inner: Weak<PoolInner>) -> Option<Sender<()>> {
    let (stop, stopped) = bounded::<()>(0);
    thread::Builder::new()
        .name("lsp-pool-reaper".to_owned())
        .spawn(move || {
            while stopped
                .recv_timeout(REAP_INTERVAL)
                .is_err_and(|error| error.is_timeout())
            {
                let Some(inner) = inner.upgrade() else {
                    return;
                };
                stop_all(reap(&inner, inner.clock.monotonic()));
            }
        })
        .ok()
        .map(|_| stop)
}

/// Removes idle and ended servers from the pool and returns the slots to
/// stop outside the lock.
fn reap(inner: &PoolInner, now: Duration) -> Vec<Arc<Slot>> {
    let idle = inner.settings.idle;
    let mut state = lock(&inner.state);
    let expired: Vec<PoolKey> = state
        .entries
        .iter()
        .filter(|(_, slot)| slot.is_dead() || now.saturating_sub(*lock(&slot.last_used)) >= idle)
        .map(|(key, _)| key.clone())
        .collect();
    expired
        .iter()
        .filter_map(|key| state.entries.remove(key))
        .collect()
}

/// Removes the least recently used servers above `keep` and returns them.
fn evict(state: &mut PoolState, keep: usize) -> Vec<Arc<Slot>> {
    let mut ordered: Vec<(Duration, PoolKey)> = state
        .entries
        .iter()
        .map(|(key, slot)| (*lock(&slot.last_used), key.clone()))
        .collect();
    ordered.sort_by_key(|entry| entry.0);
    let excess = ordered.len().saturating_sub(keep);
    ordered
        .into_iter()
        .take(excess)
        .filter_map(|(_, key)| state.entries.remove(&key))
        .collect()
}

fn stop_all(slots: Vec<Arc<Slot>>) {
    for slot in slots {
        slot.stop();
    }
}

fn set_state(slot: &Slot, state: SlotState) {
    *lock(&slot.state) = state;
    slot.changed.notify_all();
}

fn run_start(inner: &PoolInner, key: &PoolKey, spec: &ServerSpec, slot: &Arc<Slot>) {
    let started = LanguageServer::spawn(inner.host.as_ref(), spec).and_then(|server| match server
        .initialize(
            spec.init_options.as_ref(),
            inner.settings.timeout_max,
            &slot.start_cancel,
        ) {
        Ok(()) => Ok(server),
        Err(error) => {
            server.shutdown();
            Err(match error {
                LspError::Closed { .. } => error,
                other => LspError::Unavailable {
                    server: spec.name.clone(),
                    message: format!("language server {} did not start: {other}", spec.name),
                },
            })
        }
    });
    match started {
        Ok(server) => {
            if slot.start_cancel.is_cancelled() {
                server.shutdown();
            }
            set_state(slot, SlotState::Ready(server));
        }
        Err(error) => {
            let until = inner.clock.monotonic().saturating_add(inner.settings.retry);
            let mut state = lock(&inner.state);
            if state
                .entries
                .get(key)
                .is_some_and(|known| Arc::ptr_eq(known, slot))
            {
                state.entries.remove(key);
            }
            if !slot.start_cancel.is_cancelled() {
                state.failures.insert(key.clone(), (until, error.clone()));
            }
            drop(state);
            set_state(slot, SlotState::Failed(error));
        }
    }
}

fn wait_ready(
    slot: &Slot,
    timeout: Duration,
    cancel: &CancelToken,
) -> Result<LanguageServer, LspError> {
    let started = Instant::now();
    let mut state = lock(&slot.state);
    loop {
        match &*state {
            SlotState::Ready(server) if server.is_closed() => return Err(server.closed_error()),
            SlotState::Ready(server) => return Ok(server.clone()),
            SlotState::Failed(error) => return Err(error.clone()),
            SlotState::Starting => {}
        }
        if let Some(reason) = cancel.reason() {
            return Err(LspError::Cancelled(reason));
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return Err(LspError::Timeout {
                server: slot.name.clone(),
            });
        }
        state = slot
            .changed
            .wait_timeout(state, (timeout - elapsed).min(CANCEL_POLL))
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}
