// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! SSH sessions: the runtime thread, host key checking, `ProxyJump` hops
//! and public key authentication.
//!
//! `russh` is asynchronous. Every SSH session of a transport runs on one
//! dedicated thread with a single-threaded `tokio` runtime; the blocking
//! callers hand futures to that thread and wait for their results while
//! watching cancellation and the deadline.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::CancelToken;
use russh::client::{self, AuthResult};
use russh::keys::{Algorithm, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{MethodKind, Preferred};

use super::SshNotice;
use super::config::HostConfig;
use super::identity::{self, Agent, Credential, Identities};
use super::known_hosts::{Decision, HostKeyAddition, HostKeyCheck};
use super::pattern::wildcard_match;

/// Interval at which blocked callers check cancellation and the deadline.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Unanswered keepalive messages tolerated when `ServerAliveCountMax` is not
/// set; the OpenSSH default.
const DEFAULT_ALIVE_MAX: usize = 3;

/// Originator address reported for `direct-tcpip` channels.
const ORIGINATOR_ADDRESS: &str = "127.0.0.1";

/// Session of one hop.
pub(crate) type SessionHandle = client::Handle<ClientHandler>;

/// Cancellation and deadline of an operation.
#[derive(Clone)]
pub(crate) struct Limits {
    pub(crate) cancel: CancelToken,
    pub(crate) deadline: Option<Instant>,
}

impl Limits {
    /// The error to report when the operation must stop now.
    pub(crate) fn exceeded(&self) -> Option<Error> {
        if self.cancel.is_cancelled() {
            return Some(Error::new(
                ErrorCode::Cancelled,
                "ssh operation was cancelled",
            ));
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Some(
                Error::new(ErrorCode::Timeout, "ssh operation exceeded its time limit")
                    .with_help("git.network"),
            );
        }
        None
    }

    /// Completes when the operation must stop.
    pub(crate) async fn watch(&self) -> Error {
        loop {
            if let Some(error) = self.exceeded() {
                return error;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

/// Thread with the runtime that drives every SSH session of a transport.
pub(crate) struct Runtime {
    handle: tokio::runtime::Handle,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Runtime {
    /// Starts the runtime thread.
    pub(crate) fn start() -> Result<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("nurt-ssh".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(runtime.handle().clone()));
                runtime.block_on(async {
                    let _ = stop_rx.await;
                });
            })
            .map_err(|err| runtime_error(&err))?;
        let handle = ready_rx
            .recv()
            .map_err(|_| Error::new(ErrorCode::Internal, "ssh runtime thread ended before start"))?
            .map_err(|err| runtime_error(&err))?;
        Ok(Self {
            handle,
            stop: Some(stop_tx),
            thread: Some(thread),
        })
    }

    /// Runs `future` on the runtime thread and waits for its result.
    ///
    /// The future is aborted when the operation is cancelled or its deadline
    /// passes.
    pub(crate) fn run<F, T>(&self, future: F, limits: &Limits) -> Result<T>
    where
        F: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let task = self.handle.spawn(async move {
            let _ = tx.send(future.await);
        });
        loop {
            match rx.recv_timeout(POLL_INTERVAL) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(error) = limits.exceeded() {
                        task.abort();
                        return Err(error);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::new(
                        ErrorCode::Internal,
                        "ssh runtime stopped during an operation",
                    ));
                }
            }
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn runtime_error(err: &std::io::Error) -> Error {
    Error::new(
        ErrorCode::Internal,
        format!("cannot start the ssh runtime: {err}"),
    )
}

/// Client callbacks of `russh`: the host key check.
pub(crate) struct ClientHandler {
    check: HostKeyCheck,
    decision: Arc<Mutex<Option<Decision>>>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let decision = self.check.decide(&key.public_key());
        let accepted = matches!(decision, Decision::Accept { .. });
        if let Ok(mut slot) = self.decision.lock() {
            *slot = Some(decision);
        }
        Ok(accepted)
    }
}

/// One hop of the route with its host key check.
#[derive(Debug, Clone)]
pub(crate) struct HopPlan {
    pub(crate) config: HostConfig,
    pub(crate) check: HostKeyCheck,
}

/// Inputs of authentication shared by all hops.
#[derive(Debug, Clone, Default)]
pub(crate) struct AuthInput {
    pub(crate) auth_sock: Option<PathBuf>,
    pub(crate) passphrase: Option<String>,
}

/// Authenticated sessions of a route, jump hosts first.
pub(crate) struct Established {
    pub(crate) sessions: Vec<Arc<SessionHandle>>,
    pub(crate) additions: Vec<HostKeyAddition>,
    pub(crate) notices: Vec<SshNotice>,
}

/// Connects and authenticates every hop of the route. Fails with `E_GIT_HOST_KEY` for a refused host key, with `E_GIT_AUTH`
/// or `E_GIT_UNSUPPORTED` when authentication fails, with `E_NETWORK` for
/// connection failures and with `E_TIMEOUT` when a hop does not answer in
/// time.
pub(crate) async fn establish(
    hops: Vec<HopPlan>,
    auth: AuthInput,
    connect_timeout: Duration,
) -> Result<Established> {
    let mut established = Established {
        sessions: Vec::with_capacity(hops.len()),
        additions: Vec::new(),
        notices: Vec::new(),
    };
    for hop in hops {
        let timeout = hop.config.connect_timeout.unwrap_or(connect_timeout);
        let decision = Arc::new(Mutex::new(None));
        let handler = ClientHandler {
            check: hop.check.clone(),
            decision: Arc::clone(&decision),
        };
        let previous = established.sessions.last().cloned();
        let opened =
            tokio::time::timeout(timeout, open_hop(&hop, previous.as_deref(), handler)).await;
        let decision = decision.lock().ok().and_then(|mut slot| slot.take());
        let mut handle = match (opened, decision) {
            (_, Some(Decision::Reject(error))) => return Err(error),
            (Err(_), _) => return Err(connect_timeout_error(&hop.config, timeout)),
            (Ok(Err(error)), _) => return Err(error),
            (Ok(Ok(handle)), Some(Decision::Accept { addition, notice })) => {
                established.additions.extend(addition);
                established.notices.extend(notice);
                handle
            }
            (Ok(Ok(handle)), None) => handle,
        };
        authenticate(&mut handle, &hop.config, &auth).await?;
        established.sessions.push(Arc::new(handle));
    }
    Ok(established)
}

async fn open_hop(
    hop: &HopPlan,
    previous: Option<&SessionHandle>,
    handler: ClientHandler,
) -> Result<SessionHandle> {
    let host = hop.config.host_name.as_str();
    let port = hop.config.port;
    let config = client_config(hop);
    let connected = match previous {
        None => {
            let stream = tokio::net::TcpStream::connect((host, port))
                .await
                .map_err(|err| network_error(&hop.config, &err.to_string()))?;
            let _ = stream.set_nodelay(true);
            client::connect_stream(config, stream, handler).await
        }
        Some(jump) => {
            let channel = jump
                .channel_open_direct_tcpip(host, u32::from(port), ORIGINATOR_ADDRESS, 0)
                .await
                .map_err(|err| {
                    network_error(
                        &hop.config,
                        &format!("jump host refused the forwarding: {err}"),
                    )
                })?;
            client::connect_stream(config, channel.into_stream(), handler).await
        }
    };
    connected.map_err(|err| network_error(&hop.config, &err.to_string()))
}

fn client_config(hop: &HopPlan) -> Arc<client::Config> {
    let known = hop.check.known.algorithms_for(&hop.check.name);
    let preferred = Preferred {
        key: Cow::Owned(host_key_algorithms(
            hop.config.host_key_algorithms.as_deref(),
            &known,
        )),
        ..Preferred::default()
    };
    Arc::new(client::Config {
        preferred,
        keepalive_interval: hop.config.server_alive_interval,
        keepalive_max: hop
            .config
            .server_alive_count_max
            .unwrap_or(DEFAULT_ALIVE_MAX),
        nodelay: true,
        ..client::Config::default()
    })
}

/// Host key algorithms to offer: the defaults without `ssh-rsa` (SHA-1),
/// changed by a `HostKeyAlgorithms` value (`+` appends, `-` removes, `^`
/// prepends, otherwise replaces) and reordered so that algorithms already
/// recorded for the host come first, as OpenSSH does.
pub(crate) fn host_key_algorithms(spec: Option<&str>, known: &[Algorithm]) -> Vec<Algorithm> {
    let defaults: Vec<Algorithm> = Preferred::DEFAULT
        .key
        .iter()
        .filter(|algorithm| **algorithm != Algorithm::Rsa { hash: None })
        .cloned()
        .collect();
    let mut list = match spec {
        Some(spec) => apply_algorithm_filter(defaults.clone(), spec),
        None => defaults.clone(),
    };
    if list.is_empty() {
        list = defaults;
    }
    let is_known = |algorithm: &Algorithm| {
        known
            .iter()
            .any(|known| known == algorithm || (is_rsa(known) && is_rsa(algorithm)))
    };
    let (mut first, rest): (Vec<Algorithm>, Vec<Algorithm>) = list.into_iter().partition(is_known);
    first.extend(rest);
    first
}

fn is_rsa(algorithm: &Algorithm) -> bool {
    matches!(algorithm, Algorithm::Rsa { .. })
}

fn apply_algorithm_filter(defaults: Vec<Algorithm>, spec: &str) -> Vec<Algorithm> {
    let names = |text: &str| -> Vec<Algorithm> {
        text.split(',')
            .filter_map(|name| Algorithm::new(name.trim()).ok())
            .collect()
    };
    let merge = |mut list: Vec<Algorithm>, extra: Vec<Algorithm>| {
        for algorithm in extra {
            if !list.contains(&algorithm) {
                list.push(algorithm);
            }
        }
        list
    };
    if let Some(added) = spec.strip_prefix('+') {
        merge(defaults, names(added))
    } else if let Some(removed) = spec.strip_prefix('-') {
        let patterns: Vec<&str> = removed.split(',').map(str::trim).collect();
        defaults
            .into_iter()
            .filter(|algorithm| {
                !patterns
                    .iter()
                    .any(|pattern| wildcard_match(pattern, algorithm.as_str()))
            })
            .collect()
    } else if let Some(prepended) = spec.strip_prefix('^') {
        merge(names(prepended), defaults)
    } else {
        names(spec)
    }
}

/// Signature hash for RSA keys; `None` when RSA keys cannot be used.
async fn rsa_hash(
    handle: &SessionHandle,
    identities: &Identities,
    config: &HostConfig,
) -> Option<Option<HashAlg>> {
    let has_rsa = identities
        .credentials
        .iter()
        .any(|credential| credential.public_key().algorithm().is_rsa());
    if !has_rsa {
        return Some(Some(HashAlg::Sha512));
    }
    match handle.best_supported_rsa_hash().await {
        Ok(Some(Some(hash))) => Some(Some(hash)),
        Ok(Some(None)) if allows_sha1_rsa(config) => Some(None),
        Ok(Some(None)) => None,
        _ => Some(Some(HashAlg::Sha512)),
    }
}

fn allows_sha1_rsa(config: &HostConfig) -> bool {
    config.pubkey_algorithms.as_deref().is_some_and(|spec| {
        spec.trim_start_matches(['+', '^'])
            .split(',')
            .any(|name| name.trim() == "ssh-rsa")
    })
}

#[derive(Debug, Default)]
struct Attempts {
    tried: Vec<String>,
    publickey_refused: bool,
}

async fn authenticate(
    handle: &mut SessionHandle,
    config: &HostConfig,
    auth: &AuthInput,
) -> Result<()> {
    let mut agent = identity::connect_agent(&config.agent, auth.auth_sock.as_deref()).await;
    let agent_keys = match agent.as_mut() {
        Some(agent) => identity::agent_keys(agent).await,
        None => Vec::new(),
    };
    let files = config
        .identity_files
        .iter()
        .filter_map(|path| identity::load_key_file(path, auth.passphrase.as_deref()))
        .collect();
    let identities = identity::plan(files, agent_keys, config.identities_only);
    let rsa = rsa_hash(handle, &identities, config).await;
    let mut attempts = Attempts::default();
    for credential in &identities.credentials {
        let is_rsa = credential.public_key().algorithm().is_rsa();
        let hash = match (is_rsa, rsa) {
            (false, _) => None,
            (true, Some(hash)) => hash,
            (true, None) => {
                attempts.tried.push(format!(
                    "{} (server accepts only ssh-rsa signatures)",
                    credential.describe()
                ));
                continue;
            }
        };
        match offer(handle, &config.user, credential, hash, agent.as_mut()).await {
            Ok(AuthResult::Success) => return Ok(()),
            Ok(AuthResult::Failure {
                remaining_methods, ..
            }) => {
                attempts.tried.push(credential.describe());
                if !remaining_methods.contains(&MethodKind::PublicKey) {
                    attempts.publickey_refused = true;
                    break;
                }
            }
            Err(reason) => attempts
                .tried
                .push(format!("{} ({reason})", credential.describe())),
        }
    }
    Err(auth_error(config, &identities, &attempts))
}

async fn offer(
    handle: &mut SessionHandle,
    user: &str,
    credential: &Credential,
    hash: Option<HashAlg>,
    agent: Option<&mut Agent>,
) -> std::result::Result<AuthResult, String> {
    match credential {
        Credential::File { key, .. } => handle
            .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::clone(key), hash))
            .await
            .map_err(|err| err.to_string()),
        Credential::Agent { key, .. } => match agent {
            Some(agent) => handle
                .authenticate_publickey_with(user, key.clone(), hash, agent)
                .await
                .map_err(|err| err.to_string()),
            None => Err("the agent is not reachable".to_owned()),
        },
    }
}

fn auth_error(config: &HostConfig, identities: &Identities, attempts: &Attempts) -> Error {
    let account = format!("{}@{}", config.user, config.original_host);
    let mut error = if attempts.publickey_refused {
        Error::new(
            ErrorCode::GitUnsupported,
            format!("`{account}` does not accept public key authentication; password and keyboard-interactive authentication need a terminal"),
        )
        .with_problem("interactive_auth")
    } else if !identities.locked.is_empty() {
        let mut error = Error::new(
            ErrorCode::GitAuth,
            format!(
                "ssh key for `{account}` is protected by a passphrase; add it to the agent or name a passphrase file in NURT_SSH_KEY_PASSPHRASE_FILE"
            ),
        )
        .with_problem("key_locked");
        for path in &identities.locked {
            error = error.with_fix(format!("run [\"ssh-add\", \"{}\"]", path.display()));
        }
        error
    } else if attempts.tried.is_empty() {
        Error::new(
            ErrorCode::GitAuth,
            format!("no ssh key is available for `{account}`; start an agent with a key or set IdentityFile in ~/.ssh/config"),
        )
        .with_problem("missing_credentials")
        .with_next_step("run [\"ssh-keygen\", \"-t\", \"ed25519\"]")
    } else {
        Error::new(
            ErrorCode::GitAuth,
            format!(
                "`{account}` rejected every offered ssh key; register one of them with the server"
            ),
        )
        .with_problem("rejected")
    };
    for tried in &attempts.tried {
        error = error.with_sample(tried.clone());
    }
    for (path, reason) in &identities.unsupported {
        error = error.with_sample(format!("{} ({reason})", path.display()));
    }
    error.with_help("git.ssh")
}

fn network_error(config: &HostConfig, reason: &str) -> Error {
    Error::new(
        ErrorCode::Network,
        format!(
            "cannot connect to `{}:{}` over ssh: {reason}",
            config.host_name, config.port
        ),
    )
    .with_problem("ssh_connect")
}

fn connect_timeout_error(config: &HostConfig, timeout: Duration) -> Error {
    Error::new(
        ErrorCode::Timeout,
        format!(
            "ssh connection to `{}:{}` did not complete within {} s; check the host name and the network or raise ConnectTimeout",
            config.host_name,
            config.port,
            timeout.as_secs()
        ),
    )
    .with_problem("ssh_connect")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_exclude_sha1_rsa_host_keys() {
        let list = host_key_algorithms(None, &[]);
        assert_eq!(list[0], Algorithm::Ed25519);
        assert!(!list.contains(&Algorithm::Rsa { hash: None }));
        assert!(list.contains(&Algorithm::Rsa {
            hash: Some(HashAlg::Sha512)
        }));
    }

    #[test]
    fn known_algorithms_are_preferred() {
        let list = host_key_algorithms(None, &[Algorithm::Rsa { hash: None }]);
        assert!(is_rsa(&list[0]));
        assert!(is_rsa(&list[1]));
        assert_eq!(list[2], Algorithm::Ed25519);
    }

    #[test]
    fn filter_appends_removes_prepends_and_replaces() {
        let appended = host_key_algorithms(Some("+ssh-rsa"), &[]);
        assert_eq!(appended.last(), Some(&Algorithm::Rsa { hash: None }));
        let removed = host_key_algorithms(Some("-ecdsa-*,rsa-sha2-256"), &[]);
        assert!(
            removed
                .iter()
                .all(|algorithm| !algorithm.as_str().starts_with("ecdsa"))
        );
        assert!(!removed.contains(&Algorithm::Rsa {
            hash: Some(HashAlg::Sha256)
        }));
        let prepended = host_key_algorithms(Some("^rsa-sha2-512"), &[]);
        assert_eq!(
            prepended[0],
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha512)
            }
        );
        assert_eq!(
            host_key_algorithms(Some("ssh-ed25519"), &[]),
            [Algorithm::Ed25519]
        );
        assert_eq!(
            host_key_algorithms(Some("no-such-algorithm"), &[])[0],
            Algorithm::Ed25519
        );
    }

    #[test]
    fn runtime_runs_futures_and_honours_cancellation() {
        let runtime = Runtime::start().unwrap();
        let limits = Limits {
            cancel: CancelToken::new(),
            deadline: None,
        };
        assert_eq!(runtime.run(async { Ok(7) }, &limits).unwrap(), 7);
        limits
            .cancel
            .cancel(nurt_contracts::stage::CancelReason::Requested);
        let error = runtime
            .run(
                async {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    Ok(())
                },
                &limits,
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Cancelled);
    }

    #[test]
    fn runtime_stops_at_the_deadline() {
        let runtime = Runtime::start().unwrap();
        let limits = Limits {
            cancel: CancelToken::new(),
            deadline: Some(Instant::now() + Duration::from_millis(50)),
        };
        let error = runtime
            .run(
                async {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    Ok(())
                },
                &limits,
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Timeout);
    }

    #[test]
    fn sha1_rsa_needs_explicit_algorithm() {
        let mut config = crate::ssh::config::tests_support::host_config("example.com");
        assert!(!allows_sha1_rsa(&config));
        config.pubkey_algorithms = Some("+ssh-rsa".into());
        assert!(allows_sha1_rsa(&config));
    }
}
