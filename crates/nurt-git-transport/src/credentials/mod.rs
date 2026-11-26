// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Credential sources for HTTPS remotes.
//!
//! [`Credentials`] answers the credential requests of the transport without
//! a terminal, in a fixed order: environment variables (see
//! [`bearer_token`] for Bearer authentication), then the configured
//! `credential.helper` entries with `credential.username` as the user name
//! when the address has none. The helpers `store`, `cache`, `osxkeychain`
//! and `libsecret` are served by built-in code reading the stores of the
//! corresponding programs of `git`; any other helper runs as a program of
//! the user through the [`ProcessHost`] of a [`ProgramRunner`], and is
//! skipped with a `credential_helper_ignored` entry when no runner is given.
//!
//! Approved and rejected credentials are stored to and erased from the
//! configured helpers as `git` does. Changes of `store` files are not
//! written here: they are returned as [`StoreUpdate`] values for the caller
//! to write inside its transaction. Secrets never appear in `Debug` output,
//! errors or reports.

#[cfg(unix)]
mod cache;
mod config;
mod environment;
#[cfg(target_os = "macos")]
mod keychain;
mod program;
#[cfg(all(unix, not(target_os = "macos")))]
mod secret_service;
mod store;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gix_credentials::helper::{Action, NextAction};
use gix_credentials::protocol::{Context, ContextOptions, Outcome};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::ProcessHost;
use nurt_contracts::stage::CancelToken;

use self::config::{CredentialConfig, Helper, HelperKind, ProgramHelper};
use crate::http::BearerToken;

/// Reads a variable of the environment of the operation.
pub type EnvLookup = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Permissions of a `store` file created by the caller, as by `git`.
pub const STORE_FILE_MODE: u32 = 0o600;

/// A password or token; never shown by `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a secret value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value itself, for the request that needs it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Where credentials came from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialSource {
    /// An environment variable.
    Environment {
        /// Name of the variable.
        variable: String,
    },
    /// A file of the `store` helper.
    Store {
        /// Path of the file.
        file: PathBuf,
    },
    /// The daemon of the `cache` helper.
    Cache,
    /// The macOS keychain (`osxkeychain`).
    Keychain,
    /// The Secret Service of the desktop session (`libsecret`).
    SecretService,
    /// A helper run as a program.
    Program {
        /// Name of the program.
        name: String,
    },
}

impl CredentialSource {
    /// Short name for reports, without secrets, for example `env:GH_TOKEN`.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Environment { variable } => format!("env:{variable}"),
            Self::Store { file } => format!("store:{}", file.display()),
            Self::Cache => "cache".to_owned(),
            Self::Keychain => "osxkeychain".to_owned(),
            Self::SecretService => "libsecret".to_owned(),
            Self::Program { name } => format!("helper:{name}"),
        }
    }
}

/// Runs credential helpers that are programs of the user.
#[derive(Clone)]
pub struct ProgramRunner {
    /// Process host of the console.
    pub host: Arc<dyn ProcessHost>,
    /// Cancellation of the operation.
    pub cancel: CancelToken,
    /// Working directory of the helpers.
    pub cwd: PathBuf,
}

impl fmt::Debug for ProgramRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProgramRunner")
            .field("cwd", &self.cwd)
            .finish_non_exhaustive()
    }
}

/// Settings of the credential lookup.
#[derive(Clone)]
pub struct CredentialOptions {
    /// Environment of the operation; also gives `HOME`, `XDG_CONFIG_HOME`
    /// and `XDG_CACHE_HOME`.
    pub env: EnvLookup,
    /// Use the token variables of hosting providers (`GH_TOKEN`,
    /// `GITHUB_TOKEN`, `GITLAB_TOKEN`, `CI_JOB_TOKEN`).
    pub provider_variables: bool,
    /// Serve `osxkeychain`; reading items created by another program may
    /// show a consent dialog of the system.
    pub keychain: bool,
    /// Serve `libsecret`.
    pub secret_service: bool,
    /// Runner for other helpers; `None` skips them.
    pub programs: Option<ProgramRunner>,
}

impl CredentialOptions {
    /// Default settings: provider variables and `libsecret` enabled, the
    /// keychain disabled, other helpers skipped.
    #[must_use]
    pub fn new(env: EnvLookup) -> Self {
        Self {
            env,
            provider_variables: true,
            keychain: false,
            secret_service: true,
            programs: None,
        }
    }
}

impl fmt::Debug for CredentialOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialOptions")
            .field("provider_variables", &self.provider_variables)
            .field("keychain", &self.keychain)
            .field("secret_service", &self.secret_service)
            .field("programs", &self.programs)
            .finish_non_exhaustive()
    }
}

/// New content of a `store` file, to be written by the caller with mode
/// [`STORE_FILE_MODE`].
#[derive(Clone, PartialEq, Eq)]
pub struct StoreUpdate {
    /// Path of the file.
    pub path: PathBuf,
    /// The file existed before the update.
    pub existed: bool,
    /// Complete new content; contains secrets.
    pub contents: Vec<u8>,
}

impl fmt::Debug for StoreUpdate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreUpdate")
            .field("path", &self.path)
            .field("existed", &self.existed)
            .field("len", &self.contents.len())
            .finish()
    }
}

/// What the lookup did, for the report of the operation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CredentialReport {
    /// Source of the credentials sent to the server.
    pub source: Option<CredentialSource>,
    /// Sources consulted, in order, without values.
    pub consulted: Vec<String>,
    /// Unusual conditions, such as skipped helpers.
    pub notable: Vec<NotableEntry>,
    /// Pending changes of `store` files.
    pub store_updates: Vec<StoreUpdate>,
    /// Helper programs that were run.
    pub programs_run: Vec<String>,
    /// The server rejected the credentials.
    pub rejected: bool,
    /// No source had credentials.
    pub missing: bool,
    /// Credentials exist in a locked Secret Service collection.
    pub store_locked: bool,
}

/// A credential found by a source.
#[derive(Clone, PartialEq, Eq)]
struct Found {
    username: String,
    password: Secret,
    source: CredentialSource,
}

/// Request of the transport for one address.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Query {
    /// Scheme, for example `https`.
    pub(crate) protocol: String,
    /// Host with the port when it is not the default port.
    pub(crate) host: String,
    /// Path, present with `credential.useHttpPath`.
    pub(crate) path: Option<String>,
    /// User name, when known.
    pub(crate) username: Option<String>,
}

impl Query {
    fn from_context(context: &Context) -> Option<Self> {
        Some(Self {
            protocol: context.protocol.clone()?,
            host: context.host.clone()?,
            path: context.path.as_ref().map(|p| p.to_string()),
            username: context.username.clone(),
        })
    }

    /// Host without the port.
    pub(crate) fn host_name(&self) -> &str {
        match self.host.rsplit_once(':') {
            Some((name, port)) if !port.contains(']') && port.parse::<u16>().is_ok() => name,
            _ => &self.host,
        }
    }

    /// Port written in the address.
    pub(crate) fn port(&self) -> Option<u16> {
        self.host
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse().ok())
    }

    fn context(&self, username: Option<&str>, password: Option<&str>) -> Context {
        Context {
            protocol: Some(self.protocol.clone()),
            host: Some(self.host.clone()),
            path: self.path.as_deref().map(Into::into),
            username: username.or(self.username.as_deref()).map(str::to_owned),
            password: password.map(str::to_owned),
            ..Context::default()
        }
    }
}

/// Error returned to the transport when no source has credentials; the
/// details are in the [`CredentialReport`].
#[derive(Debug)]
struct NoCredentials {
    host: String,
}

impl fmt::Display for NoCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no credentials for `{}`", self.host)
    }
}

impl std::error::Error for NoCredentials {}

/// Outcome of asking one helper.
enum HelperAnswer {
    Found(Found),
    Nothing,
    Quit,
}

/// Expands a leading `~/` with `home`.
pub(crate) fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// Bearer token for `url` from `NURT_GIT_BEARER_TOKEN_<HOST>` or
/// `NURT_GIT_BEARER_TOKEN`; `None` for addresses other than `https://`.
#[must_use]
pub fn bearer_token(env: &dyn Fn(&str) -> Option<String>, url: &str) -> Option<BearerToken> {
    let parsed = url::Url::parse(url)
        .ok()
        .filter(|u| u.scheme() == "https")?;
    environment::bearer(env, parsed.host_str()?).map(|(_, token)| BearerToken::new(token))
}

/// Credential lookup for one remote address.
///
/// The value is cheap to clone; clones share the [`CredentialReport`].
#[derive(Clone)]
pub struct Credentials {
    options: Arc<CredentialOptions>,
    config: Arc<CredentialConfig>,
    host: String,
    state: Arc<Mutex<CredentialReport>>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("host", &self.host)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl Credentials {
    /// Prepares the lookup for `url` with the `credential.*` settings of
    /// `config`.
    #[must_use]
    pub fn new(config: &gix::config::File, url: &str, options: CredentialOptions) -> Self {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        Self {
            options: Arc::new(options),
            config: Arc::new(CredentialConfig::resolve(config, url)),
            host,
            state: Arc::new(Mutex::new(CredentialReport::default())),
        }
    }

    /// The function given to the connection as its credential source.
    ///
    /// It answers `get` from the sources and handles `store` and `erase`
    /// after the server accepted or rejected the credentials.
    pub fn authenticate(
        &self,
    ) -> impl FnMut(Action) -> gix::error::Result<Option<Outcome>> + Send + 'static {
        let this = self.clone();
        move |action| this.handle(action)
    }

    /// What the lookup did so far.
    #[must_use]
    pub fn report(&self) -> CredentialReport {
        self.lock().clone()
    }

    /// Error for a failed authentication with the HTTP `status` of the last
    /// response (401 or 403).
    #[must_use]
    pub fn error(&self, status: Option<u16>) -> Error {
        let state = self.lock();
        let host = &self.host;
        let variable = format!("NURT_GIT_TOKEN_{}", environment::host_suffix(host));
        let error = if status == Some(FORBIDDEN) {
            Error::new(
                ErrorCode::GitAuth,
                format!("`{host}` refused access to the repository; use the credentials of an account with read access"),
            )
            .with_problem("forbidden")
        } else if state.store_locked {
            Error::new(
                ErrorCode::GitAuth,
                format!("the secret service collection with the credentials for `{host}` is locked; unlock it in the desktop session or set {variable}"),
            )
            .with_problem("store_locked")
        } else if state.rejected {
            let source = state
                .source
                .as_ref()
                .map_or_else(|| "the address".to_owned(), CredentialSource::label);
            Error::new(
                ErrorCode::GitAuth,
                format!("`{host}` rejected the credentials from {source}; replace them or set {variable}"),
            )
            .with_problem("rejected")
        } else {
            Error::new(
                ErrorCode::GitAuth,
                format!("no credentials for `{host}`; set {variable} or add them to ~/.git-credentials with credential.helper=store"),
            )
            .with_problem("missing_credentials")
        };
        let error = state
            .consulted
            .iter()
            .fold(error, |e, source| e.with_sample(source.clone()));
        error
            .with_detail("host", serde_json::Value::from(host.as_str()))
            .with_detail("variable", serde_json::Value::from(variable))
            .with_help("git.credentials")
    }

    fn lock(&self) -> MutexGuard<'_, CredentialReport> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn env(&self, name: &str) -> Option<String> {
        (self.options.env)(name).filter(|value| !value.is_empty())
    }

    fn home(&self) -> Option<PathBuf> {
        let home = self.env("HOME");
        #[cfg(windows)]
        let home = home.or_else(|| self.env("USERPROFILE"));
        home.map(PathBuf::from)
    }

    fn handle(&self, action: Action) -> gix::error::Result<Option<Outcome>> {
        match action {
            Action::Get(mut context) => {
                context.destructure_url_in_place(self.config.use_http_path)?;
                self.get(&context)
            }
            Action::Store(payload) => {
                if let Ok(context) = Context::from_bytes(&payload, ContextOptions::default()) {
                    self.approve(&context);
                }
                Ok(None)
            }
            Action::Erase(payload) => {
                if let Ok(context) = Context::from_bytes(&payload, ContextOptions::default()) {
                    self.reject(&context);
                }
                Ok(None)
            }
        }
    }

    fn get(&self, context: &Context) -> gix::error::Result<Option<Outcome>> {
        let missing = || {
            gix::error::Error::from_error(NoCredentials {
                host: self.host.clone(),
            })
        };
        let Some(mut query) = Query::from_context(context) else {
            return Err(missing());
        };
        if query.username.is_none() {
            query.username.clone_from(&self.config.username);
        }
        let mut state = self.lock();
        state.missing = false;
        let Some(found) = self.lookup(&mut state, &query) else {
            state.missing = true;
            return Err(missing());
        };
        let next = NextAction::from(
            query.context(Some(found.username.as_str()), Some(found.password.expose())),
        );
        let credentials = gix_sec::identity::Credentials::UsernamePassword {
            username: found.username,
            password: found.password.expose().to_owned(),
            oauth_refresh_token: None,
        };
        state.source = Some(found.source);
        Ok(Some(Outcome { credentials, next }))
    }

    fn lookup(&self, state: &mut CredentialReport, query: &Query) -> Option<Found> {
        state.consulted.push("environment".to_owned());
        let env = |name: &str| self.env(name);
        let host = query.host_name();
        if let Some(token) = environment::token(
            &env,
            host,
            query.username.as_deref(),
            self.options.provider_variables,
        ) {
            return Some(Found {
                username: token.username,
                password: Secret::new(token.token),
                source: CredentialSource::Environment {
                    variable: token.variable,
                },
            });
        }
        for helper in &self.config.helpers {
            match self.ask(state, helper, query) {
                HelperAnswer::Found(found) => return Some(found),
                HelperAnswer::Quit => return None,
                HelperAnswer::Nothing => {}
            }
        }
        None
    }

    fn note(state: &mut CredentialReport, detail: String) {
        if state
            .notable
            .iter()
            .any(|n| n.detail.as_deref() == Some(detail.as_str()))
        {
            return;
        }
        let mut entry = NotableEntry::new(Notable::CredentialHelperIgnored);
        entry.detail = Some(detail);
        state.notable.push(entry);
    }

    fn ask(&self, state: &mut CredentialReport, helper: &Helper, query: &Query) -> HelperAnswer {
        state.consulted.push(helper.label());
        match &helper.kind {
            HelperKind::Store { file } => self.ask_store(file.as_deref(), query),
            HelperKind::Cache { socket } => self.ask_cache(state, socket.as_deref(), query),
            HelperKind::Keychain => self.ask_keychain(state, query),
            HelperKind::SecretService => self.ask_secret_service(state, query),
            HelperKind::CredentialManager => self.ask_credential_manager(state, helper, query),
            HelperKind::Program(program) => self.ask_program(state, helper, program, query),
        }
    }

    fn store_files(&self, file: Option<&str>) -> Vec<PathBuf> {
        let home = self.home();
        if let Some(file) = file {
            return vec![expand_home(file, home.as_deref())];
        }
        let Some(home) = home else {
            return Vec::new();
        };
        let config_home = self
            .env("XDG_CONFIG_HOME")
            .map_or_else(|| home.join(".config"), PathBuf::from);
        vec![
            home.join(".git-credentials"),
            config_home.join("git").join("credentials"),
        ]
    }

    fn ask_store(&self, file: Option<&str>, query: &Query) -> HelperAnswer {
        for path in self.store_files(file) {
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(entry) = store::find(&content, query) {
                return HelperAnswer::Found(Found {
                    username: entry.username,
                    password: Secret::new(entry.password),
                    source: CredentialSource::Store { file: path },
                });
            }
        }
        HelperAnswer::Nothing
    }

    #[cfg(unix)]
    fn cache_socket(&self, socket: Option<&str>) -> Option<PathBuf> {
        let cache_home = self.env("XDG_CACHE_HOME").map(PathBuf::from);
        cache::socket_path(socket, self.home().as_deref(), cache_home.as_deref())
    }

    #[cfg(unix)]
    fn ask_cache(
        &self,
        _state: &mut CredentialReport,
        socket: Option<&str>,
        query: &Query,
    ) -> HelperAnswer {
        let Some(path) = self.cache_socket(socket) else {
            return HelperAnswer::Nothing;
        };
        let Ok(answer) = cache::request(&path, "get", &query.context(None, None)) else {
            return HelperAnswer::Nothing;
        };
        match Context::from_bytes(&answer, ContextOptions::default()) {
            Ok(context) => answer_from(context, CredentialSource::Cache),
            Err(_) => HelperAnswer::Nothing,
        }
    }

    #[cfg(not(unix))]
    fn ask_cache(
        &self,
        state: &mut CredentialReport,
        _socket: Option<&str>,
        _query: &Query,
    ) -> HelperAnswer {
        Self::note(
            state,
            "cache: the credential cache daemon is available on Unix systems only".to_owned(),
        );
        HelperAnswer::Nothing
    }

    #[cfg(target_os = "macos")]
    fn ask_keychain(&self, state: &mut CredentialReport, query: &Query) -> HelperAnswer {
        if !self.options.keychain {
            Self::note(
                state,
                "osxkeychain: reading the keychain is disabled in the git credential settings"
                    .to_owned(),
            );
            return HelperAnswer::Nothing;
        }
        let Some(username) = query.username.as_deref() else {
            Self::note(state, "osxkeychain: the keychain is searched only when the user name is known; set credential.username".to_owned());
            return HelperAnswer::Nothing;
        };
        match keychain::get(query, username) {
            Ok(Some(password)) => HelperAnswer::Found(Found {
                username: username.to_owned(),
                password: Secret::new(password),
                source: CredentialSource::Keychain,
            }),
            Ok(None) => HelperAnswer::Nothing,
            Err(reason) => {
                Self::note(state, format!("osxkeychain: {reason}"));
                HelperAnswer::Nothing
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn ask_keychain(&self, state: &mut CredentialReport, _query: &Query) -> HelperAnswer {
        Self::note(
            state,
            "osxkeychain: the macOS keychain is not available on this system".to_owned(),
        );
        HelperAnswer::Nothing
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn ask_secret_service(&self, state: &mut CredentialReport, query: &Query) -> HelperAnswer {
        if !self.options.secret_service {
            Self::note(
                state,
                "libsecret: reading the secret service is disabled in the git credential settings"
                    .to_owned(),
            );
            return HelperAnswer::Nothing;
        }
        match secret_service::get(query) {
            Ok(secret_service::Lookup::Found { username, password }) => {
                HelperAnswer::Found(Found {
                    username,
                    password: Secret::new(password),
                    source: CredentialSource::SecretService,
                })
            }
            Ok(secret_service::Lookup::NotFound) => HelperAnswer::Nothing,
            Ok(secret_service::Lookup::Locked) => {
                state.store_locked = true;
                HelperAnswer::Nothing
            }
            Err(reason) => {
                Self::note(state, format!("libsecret: {reason}"));
                HelperAnswer::Nothing
            }
        }
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    fn ask_secret_service(&self, state: &mut CredentialReport, _query: &Query) -> HelperAnswer {
        Self::note(
            state,
            "libsecret: the secret service is not available on this system".to_owned(),
        );
        HelperAnswer::Nothing
    }

    /// The Windows credential manager is reached only through system calls,
    /// which this crate does not make; elsewhere `manager` names the program
    /// of Git Credential Manager.
    fn ask_credential_manager(
        &self,
        state: &mut CredentialReport,
        helper: &Helper,
        query: &Query,
    ) -> HelperAnswer {
        if cfg!(windows) {
            Self::note(
                state,
                format!(
                    "{}: the Windows credential manager is not supported; set NURT_GIT_TOKEN_{}",
                    helper.label(),
                    environment::host_suffix(query.host_name())
                ),
            );
            return HelperAnswer::Nothing;
        }
        let program = manager_program(helper);
        self.ask_program(state, helper, &program, query)
    }

    fn ask_program(
        &self,
        state: &mut CredentialReport,
        helper: &Helper,
        program: &ProgramHelper,
        query: &Query,
    ) -> HelperAnswer {
        let Some(runner) = &self.options.programs else {
            Self::note(
                state,
                format!(
                    "{}: credential helper programs are not run by this operation",
                    helper.label()
                ),
            );
            return HelperAnswer::Nothing;
        };
        state.programs_run.push(program.name().to_owned());
        match program::run(runner, program, "get", &query.context(None, None)) {
            Ok(Some(context)) => answer_from(
                context,
                CredentialSource::Program {
                    name: program.name().to_owned(),
                },
            ),
            Ok(None) => HelperAnswer::Nothing,
            Err(reason) => {
                Self::note(state, format!("{}: {reason}", helper.label()));
                HelperAnswer::Nothing
            }
        }
    }

    fn approve(&self, context: &Context) {
        let (Some(query), Some(username), Some(password)) = (
            Query::from_context(context),
            context.username.as_deref(),
            context.password.as_deref(),
        ) else {
            return;
        };
        let mut state = self.lock();
        if matches!(
            state.source,
            None | Some(CredentialSource::Environment { .. })
        ) {
            return;
        }
        for helper in &self.config.helpers {
            self.change(&mut state, helper, &query, username, Some(password));
        }
    }

    fn reject(&self, context: &Context) {
        let mut state = self.lock();
        state.rejected = true;
        let (Some(query), Some(username)) =
            (Query::from_context(context), context.username.as_deref())
        else {
            return;
        };
        if matches!(
            state.source,
            None | Some(CredentialSource::Environment { .. })
        ) {
            return;
        }
        let password = context.password.clone();
        for helper in &self.config.helpers {
            self.change(&mut state, helper, &query, username, None);
            if let HelperKind::Store { file } = &helper.kind {
                self.erase_from_files(
                    &mut state,
                    file.as_deref(),
                    &query,
                    username,
                    password.as_deref(),
                );
            }
        }
    }

    /// Stores (`password` given) or erases (`None`) the credentials in one
    /// helper other than `store` files, which are erased separately.
    fn change(
        &self,
        state: &mut CredentialReport,
        helper: &Helper,
        query: &Query,
        username: &str,
        password: Option<&str>,
    ) {
        let action = if password.is_some() { "store" } else { "erase" };
        match &helper.kind {
            HelperKind::Store { file } => {
                if let Some(password) = password {
                    self.store_in_files(state, file.as_deref(), query, username, password);
                }
            }
            HelperKind::Cache { socket } => {
                self.change_cache(socket.as_deref(), query, username, password, action)
            }
            HelperKind::Keychain => self.change_keychain(state, query, username, password),
            HelperKind::SecretService => {
                self.change_secret_service(state, query, username, password)
            }
            HelperKind::CredentialManager if !cfg!(windows) => {
                self.change_program(
                    state,
                    &manager_program(helper),
                    query,
                    username,
                    password,
                    action,
                );
            }
            HelperKind::CredentialManager => {}
            HelperKind::Program(program) => {
                self.change_program(state, program, query, username, password, action)
            }
        }
    }

    fn current_content(state: &CredentialReport, path: &Path) -> Option<String> {
        if let Some(update) = state.store_updates.iter().rev().find(|u| u.path == path) {
            return Some(String::from_utf8_lossy(&update.contents).into_owned());
        }
        std::fs::read_to_string(path).ok()
    }

    fn push_update(
        state: &mut CredentialReport,
        path: PathBuf,
        before: Option<String>,
        after: String,
    ) {
        if before.as_deref() == Some(after.as_str()) {
            return;
        }
        let existed = path.exists() || state.store_updates.iter().any(|u| u.path == path);
        state.store_updates.retain(|u| u.path != path);
        state.store_updates.push(StoreUpdate {
            path,
            existed,
            contents: after.into_bytes(),
        });
    }

    fn store_in_files(
        &self,
        state: &mut CredentialReport,
        file: Option<&str>,
        query: &Query,
        username: &str,
        password: &str,
    ) {
        let files = self.store_files(file);
        let target = files
            .iter()
            .find(|path| Self::current_content(state, path).is_some())
            .or_else(|| files.first())
            .cloned();
        let Some(path) = target else {
            return;
        };
        let entry = store::Entry {
            protocol: query.protocol.clone(),
            host: query.host.clone(),
            path: query.path.clone(),
            username: username.to_owned(),
            password: password.to_owned(),
        };
        let before = Self::current_content(state, &path);
        let after = store::store(before.as_deref().unwrap_or_default(), &entry);
        Self::push_update(state, path, before, after);
    }

    fn erase_from_files(
        &self,
        state: &mut CredentialReport,
        file: Option<&str>,
        query: &Query,
        username: &str,
        password: Option<&str>,
    ) {
        let query = Query {
            username: Some(username.to_owned()),
            ..query.clone()
        };
        for path in self.store_files(file) {
            let Some(before) = Self::current_content(state, &path) else {
                continue;
            };
            let after = store::erase(&before, &query, password);
            Self::push_update(state, path, Some(before), after);
        }
    }

    #[cfg(unix)]
    fn change_cache(
        &self,
        socket: Option<&str>,
        query: &Query,
        username: &str,
        password: Option<&str>,
        action: &str,
    ) {
        if let Some(path) = self.cache_socket(socket) {
            cache::request(&path, action, &query.context(Some(username), password)).ok();
        }
    }

    #[cfg(not(unix))]
    fn change_cache(
        &self,
        _socket: Option<&str>,
        _query: &Query,
        _username: &str,
        _password: Option<&str>,
        _action: &str,
    ) {
    }

    #[cfg(target_os = "macos")]
    fn change_keychain(
        &self,
        state: &mut CredentialReport,
        query: &Query,
        username: &str,
        password: Option<&str>,
    ) {
        if !self.options.keychain
            || (state.source == Some(CredentialSource::Keychain) && password.is_some())
        {
            return;
        }
        let result = match password {
            Some(password) => keychain::store(query, username, password),
            None => keychain::erase(query, username),
        };
        if let Err(reason) = result {
            Self::note(state, format!("osxkeychain: {reason}"));
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn change_keychain(
        &self,
        _state: &mut CredentialReport,
        _query: &Query,
        _username: &str,
        _password: Option<&str>,
    ) {
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn change_secret_service(
        &self,
        state: &mut CredentialReport,
        query: &Query,
        username: &str,
        password: Option<&str>,
    ) {
        if !self.options.secret_service
            || (state.source == Some(CredentialSource::SecretService) && password.is_some())
        {
            return;
        }
        let result = match password {
            Some(password) => secret_service::store(query, username, password).map(|_| ()),
            None => secret_service::erase(query, username),
        };
        if let Err(reason) = result {
            Self::note(state, format!("libsecret: {reason}"));
        }
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    fn change_secret_service(
        &self,
        _state: &mut CredentialReport,
        _query: &Query,
        _username: &str,
        _password: Option<&str>,
    ) {
    }

    fn change_program(
        &self,
        state: &mut CredentialReport,
        program: &ProgramHelper,
        query: &Query,
        username: &str,
        password: Option<&str>,
        action: &str,
    ) {
        let Some(runner) = &self.options.programs else {
            return;
        };
        state.programs_run.push(program.name().to_owned());
        if let Err(reason) = program::run(
            runner,
            program,
            action,
            &query.context(Some(username), password),
        ) {
            Self::note(state, format!("{}: {reason}", program.name()));
        }
    }
}

/// HTTP status of a response refusing access to an authenticated user.
const FORBIDDEN: u16 = 403;

/// Program of Git Credential Manager for a `manager` entry.
fn manager_program(helper: &Helper) -> ProgramHelper {
    let mut words = config::split_words(&helper.definition).unwrap_or_default();
    let args = if words.is_empty() {
        Vec::new()
    } else {
        words.split_off(1)
    };
    let first = words.into_iter().next().unwrap_or_default();
    let program = if Path::new(&first).is_absolute() {
        first
    } else {
        format!("git-credential-{first}")
    };
    ProgramHelper::Command { program, args }
}

fn answer_from(context: Context, source: CredentialSource) -> HelperAnswer {
    if context.quit == Some(true) {
        return HelperAnswer::Quit;
    }
    match (context.username, context.password) {
        (Some(username), Some(password)) => HelperAnswer::Found(Found {
            username,
            password: Secret::new(password),
            source,
        }),
        _ => HelperAnswer::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::services::{
        ProgramInput, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec,
    };
    use std::collections::HashMap;

    fn env_of(vars: &[(&str, &str)]) -> EnvLookup {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Arc::new(move |name: &str| vars.get(name).cloned())
    }

    fn credentials(config: &str, url: &str, options: CredentialOptions) -> Credentials {
        let config: gix::config::File = config.parse().expect("valid configuration");
        Credentials::new(&config, url, options)
    }

    fn get(credentials: &Credentials, url: &str) -> gix::error::Result<Option<Outcome>> {
        let mut authenticate = credentials.authenticate();
        authenticate(Action::get_for_url(url))
    }

    fn identity(outcome: gix::error::Result<Option<Outcome>>) -> (String, String) {
        let outcome = outcome.expect("credentials").expect("outcome");
        match outcome.credentials {
            gix_sec::identity::Credentials::UsernamePassword {
                username, password, ..
            } => (username, password),
            gix_sec::identity::Credentials::Encoded { .. } => {
                panic!("expected a username and a password")
            }
        }
    }

    #[derive(Default)]
    struct FakeHost {
        stdout: Vec<u8>,
        exit_code: Option<i32>,
        specs: Mutex<Vec<ProgramSpec>>,
    }

    impl ProcessHost for FakeHost {
        fn resolve_program(
            &self,
            program: &str,
            _cwd: &Path,
        ) -> nurt_contracts::error::Result<PathBuf> {
            Ok(PathBuf::from(program))
        }

        fn run_program(
            &self,
            spec: &ProgramSpec,
            _cancel: &CancelToken,
        ) -> nurt_contracts::error::Result<ProgramOutcome> {
            self.specs.lock().expect("lock").push(spec.clone());
            Ok(ProgramOutcome {
                exit_code: self.exit_code,
                stdout: self.stdout.clone(),
                ..ProgramOutcome::default()
            })
        }

        fn spawn_service(
            &self,
            _spec: &ServiceSpawnSpec,
        ) -> nurt_contracts::error::Result<Box<dyn ServiceChild>> {
            Err(Error::new(
                ErrorCode::Internal,
                "services are not available in this test",
            ))
        }
    }

    fn runner(host: &Arc<FakeHost>) -> ProgramRunner {
        ProgramRunner {
            host: Arc::clone(host) as Arc<dyn ProcessHost>,
            cancel: CancelToken::new(),
            cwd: PathBuf::from("/"),
        }
    }

    #[test]
    fn environment_token_answers_first() {
        let options = CredentialOptions::new(env_of(&[("GH_TOKEN", "gh-secret")]));
        let creds = credentials(
            "[credential]\n\thelper = store\n",
            "https://github.com/o/r.git",
            options,
        );
        let (user, password) = identity(get(&creds, "https://github.com/o/r.git"));
        assert_eq!(
            (user.as_str(), password.as_str()),
            ("x-access-token", "gh-secret")
        );
        let report = creds.report();
        assert_eq!(
            report.source.as_ref().map(CredentialSource::label),
            Some("env:GH_TOKEN".to_owned())
        );
        assert_eq!(report.consulted, ["environment"]);
        assert!(!format!("{report:?}").contains("gh-secret"));
    }

    #[test]
    fn store_file_answers_with_the_configured_user_name() {
        let home = tempfile::tempdir().expect("temporary directory");
        std::fs::write(
            home.path().join(".git-credentials"),
            "https://alice:a1@example.com\nhttps://bob:b1@example.com\n",
        )
        .expect("store written");
        let home_path = home.path().to_string_lossy().into_owned();
        let options = CredentialOptions::new(env_of(&[("HOME", home_path.as_str())]));
        let config = "[credential]\n\thelper = store\n\tusername = bob\n";
        let creds = credentials(config, "https://example.com/r.git", options);
        assert_eq!(
            identity(get(&creds, "https://example.com/r.git")),
            ("bob".to_owned(), "b1".to_owned())
        );
        let source = creds.report().source.expect("source");
        assert_eq!(
            source,
            CredentialSource::Store {
                file: home.path().join(".git-credentials")
            }
        );
    }

    #[test]
    fn approval_and_rejection_produce_store_updates() {
        let home = tempfile::tempdir().expect("temporary directory");
        let file = home.path().join(".git-credentials");
        std::fs::write(
            &file,
            "https://other:o@other.com\nhttps://bob:b1@example.com\n",
        )
        .expect("store written");
        let home_path = home.path().to_string_lossy().into_owned();
        let options = CredentialOptions::new(env_of(&[("HOME", home_path.as_str())]));
        let creds = credentials(
            "[credential]\n\thelper = store\n",
            "https://example.com/r.git",
            options,
        );
        let outcome = get(&creds, "https://example.com/r.git")
            .expect("credentials")
            .expect("outcome");
        let mut authenticate = creds.authenticate();
        authenticate(outcome.next.store()).expect("stored");
        let report = creds.report();
        assert_eq!(report.store_updates.len(), 1);
        assert_eq!(
            String::from_utf8_lossy(&report.store_updates[0].contents),
            "https://bob:b1@example.com\nhttps://other:o@other.com\n"
        );
        assert!(!format!("{:?}", report.store_updates).contains("b1"));
        let outcome = get(&creds, "https://example.com/r.git")
            .expect("credentials")
            .expect("outcome");
        authenticate(outcome.next.erase()).expect("erased");
        let report = creds.report();
        assert!(report.rejected);
        assert_eq!(
            String::from_utf8_lossy(&report.store_updates[0].contents),
            "https://other:o@other.com\n"
        );
        assert_eq!(
            creds.error(Some(401)).data().problem.as_deref(),
            Some("rejected")
        );
    }

    #[test]
    fn missing_credentials_list_consulted_sources() {
        let options = CredentialOptions::new(env_of(&[]));
        let creds = credentials(
            "[credential]\n\thelper = store\n",
            "https://example.com/r.git",
            options,
        );
        assert!(get(&creds, "https://example.com/r.git").is_err());
        let report = creds.report();
        assert!(report.missing);
        let error = creds.error(Some(401));
        assert_eq!(error.code(), ErrorCode::GitAuth);
        assert_eq!(error.data().problem.as_deref(), Some("missing_credentials"));
        assert_eq!(error.data().sample, ["environment", "store"]);
        assert!(error.data().message.contains("NURT_GIT_TOKEN_EXAMPLE_COM"));
        assert_eq!(
            creds.error(Some(403)).data().problem.as_deref(),
            Some("forbidden")
        );
    }

    #[test]
    fn program_helpers_run_through_the_process_host() {
        let host = Arc::new(FakeHost {
            stdout: b"username=carol\npassword=c1\n".to_vec(),
            exit_code: Some(0),
            ..FakeHost::default()
        });
        let mut options = CredentialOptions::new(env_of(&[]));
        options.programs = Some(runner(&host));
        let creds = credentials(
            "[credential]\n\thelper = netrc -v\n",
            "https://example.com/r.git",
            options,
        );
        assert_eq!(
            identity(get(&creds, "https://example.com/r.git")),
            ("carol".to_owned(), "c1".to_owned())
        );
        let specs = host.specs.lock().expect("lock");
        assert_eq!(specs[0].program, "git-credential-netrc");
        assert_eq!(specs[0].args, ["-v", "get"]);
        let ProgramInput::Bytes(input) = &specs[0].stdin else {
            panic!("context expected on standard input");
        };
        assert!(String::from_utf8_lossy(input).contains("host=example.com\n"));
        assert_eq!(creds.report().programs_run, ["git-credential-netrc"]);
    }

    #[test]
    fn failing_program_helper_is_reported_and_skipped() {
        let host = Arc::new(FakeHost {
            exit_code: Some(1),
            ..FakeHost::default()
        });
        let mut options = CredentialOptions::new(env_of(&[]));
        options.programs = Some(runner(&host));
        let creds = credentials(
            "[credential]\n\thelper = !false\n",
            "https://example.com/r.git",
            options,
        );
        assert!(get(&creds, "https://example.com/r.git").is_err());
        let notable = creds.report().notable;
        assert_eq!(notable.len(), 1);
        assert_eq!(notable[0].detail.as_deref(), Some("!: exited with code 1"));
    }

    #[test]
    fn program_helpers_without_runner_are_ignored() {
        let creds = credentials(
            "[credential]\n\thelper = !gh auth git-credential\n",
            "https://example.com/r.git",
            CredentialOptions::new(env_of(&[])),
        );
        assert!(get(&creds, "https://example.com/r.git").is_err());
        let report = creds.report();
        assert_eq!(report.notable[0].code, Notable::CredentialHelperIgnored);
        assert!(report.programs_run.is_empty());
    }

    #[test]
    fn quit_from_a_helper_stops_the_lookup() {
        let host = Arc::new(FakeHost {
            stdout: b"quit=1\n".to_vec(),
            exit_code: Some(0),
            ..FakeHost::default()
        });
        let mut options = CredentialOptions::new(env_of(&[]));
        options.programs = Some(runner(&host));
        let config = "[credential]\n\thelper = first\n\thelper = second\n";
        let creds = credentials(config, "https://example.com/r.git", options);
        assert!(get(&creds, "https://example.com/r.git").is_err());
        assert_eq!(host.specs.lock().expect("lock").len(), 1);
    }

    #[test]
    fn finds_bearer_tokens_for_https_only() {
        let env = env_of(&[("NURT_GIT_BEARER_TOKEN_EXAMPLE_COM", "bt")]);
        let token = bearer_token(&*env, "https://example.com/r.git").expect("token");
        assert_eq!(token.expose(), "bt");
        assert!(bearer_token(&*env, "http://example.com/r.git").is_none());
        assert!(bearer_token(&*env, "https://other.com/r.git").is_none());
    }

    #[test]
    fn query_splits_host_and_port() {
        let query = Query {
            host: "example.com:8443".to_owned(),
            ..Query::default()
        };
        assert_eq!(
            (query.host_name(), query.port()),
            ("example.com", Some(8443))
        );
        let plain = Query {
            host: "example.com".to_owned(),
            ..Query::default()
        };
        assert_eq!((plain.host_name(), plain.port()), ("example.com", None));
    }

    #[test]
    fn secret_is_hidden_from_debug() {
        let secret = Secret::new("hidden-value");
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
        assert_eq!(secret.expose(), "hidden-value");
    }

    #[test]
    fn manager_helper_names_the_program_of_git_credential_manager() {
        let helper = |definition: &str| Helper {
            definition: definition.to_owned(),
            kind: config::classify(definition),
        };
        assert_eq!(
            manager_program(&helper("manager")),
            ProgramHelper::Command {
                program: "git-credential-manager".to_owned(),
                args: Vec::new(),
            }
        );
        assert_eq!(
            manager_program(&helper("manager-core --verbose")),
            ProgramHelper::Command {
                program: "git-credential-manager-core".to_owned(),
                args: vec!["--verbose".to_owned()],
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn manager_helper_keeps_an_absolute_program_path() {
        let definition = "/opt/gcm/git-credential-manager --x";
        let helper = Helper {
            definition: definition.to_owned(),
            kind: config::classify(definition),
        };
        assert_eq!(
            manager_program(&helper),
            ProgramHelper::Command {
                program: "/opt/gcm/git-credential-manager".to_owned(),
                args: vec!["--x".to_owned()],
            }
        );
    }

    #[test]
    fn expands_home_prefix() {
        assert_eq!(
            expand_home("~/x", Some(Path::new("/h"))),
            PathBuf::from("/h/x")
        );
        assert_eq!(
            expand_home("/abs", Some(Path::new("/h"))),
            PathBuf::from("/abs")
        );
    }
}
