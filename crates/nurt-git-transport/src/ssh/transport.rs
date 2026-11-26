// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `gix-transport` implementation over SSH.

use std::any::Any;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bstr::{BStr, BString, ByteSlice};
use gix_transport::client::blocking_io::{RequestWriter, SetServiceResponse, Transport};
use gix_transport::client::git::ConnectMode;
use gix_transport::client::git::blocking_io::Connection;
use gix_transport::client::{self, MessageKind, TransportWithoutIO, WriteMode};
use gix_transport::{Protocol, Service};
use nurt_contracts::config::KnownHosts as KnownHostsSetting;
use nurt_contracts::error::{Error, Result};

use super::channel::{self, ChannelReader, ChannelWriter, Status};
use super::config::{self, ConfigSources, HostConfig, Query, StrictMode};
use super::identity;
use super::known_hosts::{HostKeyAddition, HostKeyCheck, KnownHosts, Policy};
use super::session::{self, AuthInput, Established, HopPlan, Limits, Runtime, SessionHandle};
use super::target::{self, Target};
use super::{SshNotice, SshOptions};

/// Transport of the Git protocol over an authenticated SSH session.
///
/// Each [`Transport::handshake`] opens a new `exec` channel on the same
/// session and runs `git-upload-pack '<path>'` or
/// `git-receive-pack '<path>'` on the server; the protocol version is
/// requested with the variable `GIT_PROTOCOL`, which servers that do not
/// accept it ignore. No program is started on the local host.
///
/// The value is created by [`connect`], which also verifies the host key
/// and authenticates.
pub struct SshTransport {
    connection: Option<Connection<ChannelReader, ChannelWriter>>,
    url: BString,
    path: BString,
    desired_version: Protocol,
    limits: Limits,
    diagnostics: RemoteDiagnostics,
    additions: Vec<HostKeyAddition>,
    notices: Vec<SshNotice>,
    sessions: Vec<Arc<SessionHandle>>,
    runtime: Runtime,
}

impl std::fmt::Debug for SshTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshTransport")
            .field("url", &self.url)
            .field("desired_version", &self.desired_version)
            .field("hops", &self.sessions.len())
            .finish_non_exhaustive()
    }
}

/// Outcome of the remote command of the latest channel, readable after the
/// transport was handed to `gix`.
///
/// `gix` reports a failed remote command as an input/output error; this
/// handle gives the contract error with the message of the server.
#[derive(Debug, Clone, Default)]
pub struct RemoteDiagnostics {
    current: Arc<Mutex<Option<(Status, String)>>>,
}

impl RemoteDiagnostics {
    fn replace(&self, status: Status, command: String) {
        if let Ok(mut current) = self.current.lock() {
            *current = Some((status, command));
        }
    }

    /// Error describing an abnormal end of the remote command: refused
    /// command, non-zero exit status, exit signal, cancellation or timeout.
    ///
    /// Returns `None` while the command runs and after a normal end.
    #[must_use]
    pub fn error(&self) -> Option<Error> {
        let current = self.current.lock().ok()?;
        let (status, command) = current.as_ref()?;
        status.lock().failure(command)
    }

    /// Standard error of the remote command, decoded lossily as UTF-8.
    #[must_use]
    pub fn stderr(&self) -> String {
        self.current
            .lock()
            .ok()
            .and_then(|current| {
                current
                    .as_ref()
                    .map(|(status, _)| String::from_utf8_lossy(&status.lock().stderr).into_owned())
            })
            .unwrap_or_default()
    }
}

/// Connects to the server of an `ssh://` or `[user@]host:path` URL.
///
/// The connection follows the SSH client configuration of the user and of
/// the system, including `ProxyJump` chains, checks every host key against
/// the known hosts files and authenticates with the agent and the key
/// files. Accepted new host keys are returned by
/// [`SshTransport::take_host_key_additions`] and recorded by the caller
/// with [`record_host_key`](super::record_host_key).
///
/// # Errors
///
/// Fails with `E_GIT_HOST_KEY` for an unknown, changed or revoked host key,
/// with `E_GIT_AUTH` when no key is accepted, with `E_GIT_UNSUPPORTED` for
/// `ProxyCommand` and servers without public key authentication, with
/// `E_ARG` for a malformed address and with `E_NETWORK`, `E_TIMEOUT` or
/// `E_CANCELLED` when the connection fails or stops.
///
/// # Examples
///
/// ```no_run
/// use gix_transport::Protocol;
/// use nurt_git_transport::ssh::{connect, SshOptions};
///
/// let url = gix_url::parse(bstr::BStr::new("git@git.example.com:team/repo.git")).expect("valid url");
/// let options = SshOptions::new("/home/user", "user");
/// if let Ok(mut transport) = connect(&url, Protocol::default(), &options) {
///     let additions = transport.take_host_key_additions();
///     assert!(additions.iter().all(|addition| addition.fingerprint.starts_with("SHA256:")));
/// }
/// ```
pub fn connect(
    url: &gix_url::Url,
    desired_version: Protocol,
    options: &SshOptions,
) -> Result<SshTransport> {
    let target = Target::from_url(url)?;
    let sources = ConfigSources {
        home: options.home.clone(),
        local_user: options.local_user.clone(),
        local_host: options.local_host.clone(),
        user_file: options.user_config.clone(),
        system_dir: options.system_dir.clone(),
        extra_identities: options.identity_files.clone(),
    };
    let hops = config::route(
        &sources,
        &Query {
            host: &target.host,
            user: target.user.as_deref(),
            port: target.port,
        },
    )?;
    let mut notices: Vec<SshNotice> = options
        .ignored_ssh_command
        .iter()
        .map(|source| SshNotice::SshCommandIgnored {
            source: source.clone(),
        })
        .collect();
    notices.extend(hops.iter().flat_map(|hop| {
        hop.match_exec
            .iter()
            .map(|location| SshNotice::MatchExecIgnored {
                file: location.file.clone(),
                line: location.line,
            })
    }));
    let auth = AuthInput {
        auth_sock: options.auth_sock.clone(),
        passphrase: options
            .passphrase_file
            .as_deref()
            .map(|file| identity::read_passphrase_file(file, &options.home))
            .transpose()?,
    };
    let plans = hops.into_iter().map(|hop| plan_hop(hop, options)).collect();
    let limits = Limits {
        cancel: options.cancel.clone(),
        deadline: options.timeout.map(|timeout| Instant::now() + timeout),
    };
    let runtime = Runtime::start()?;
    let established: Established = runtime.run(
        session::establish(plans, auth, options.connect_timeout),
        &limits,
    )?;
    notices.extend(established.notices);
    options.host_keys.push_all(&established.additions);
    Ok(SshTransport {
        connection: None,
        url: url.to_bstring(),
        path: target.path,
        desired_version,
        limits,
        diagnostics: RemoteDiagnostics::default(),
        additions: established.additions,
        notices,
        sessions: established.sessions,
        runtime,
    })
}

/// Checking of unknown hosts from the setting of Nurt and the
/// `StrictHostKeyChecking` of the host: the setting of Nurt that accepts new
/// hosts never weakens an explicit `StrictHostKeyChecking yes` of the user.
fn host_key_policy(setting: KnownHostsSetting, strict: Option<StrictMode>) -> Policy {
    match (setting, strict) {
        (_, Some(StrictMode::Yes)) => Policy::Strict,
        (KnownHostsSetting::AcceptNew, _) | (_, Some(StrictMode::AcceptNew)) => Policy::AcceptNew,
        (_, Some(StrictMode::No)) => Policy::Off,
        _ => Policy::Strict,
    }
}

fn plan_hop(config: HostConfig, options: &SshOptions) -> HopPlan {
    let mut files = config.user_known_hosts.clone();
    files.extend(config.global_known_hosts.iter().cloned());
    files.extend(options.nurt_known_hosts.iter().cloned());
    let policy = host_key_policy(options.known_hosts, config.strict);
    let check = HostKeyCheck {
        host: config.original_host.clone(),
        name: config.known_hosts_name(),
        known: KnownHosts::load(&files),
        files,
        policy,
        accept_fingerprint: options.accept_host_key.clone(),
        record_file: options.nurt_known_hosts.clone(),
        retry_command: options.retry_command.clone(),
    };
    HopPlan { config, check }
}

impl SshTransport {
    /// Unusual conditions met while connecting, for the report of the
    /// operation.
    #[must_use]
    pub fn notices(&self) -> &[SshNotice] {
        &self.notices
    }

    /// Host keys accepted for this connection that are not yet recorded;
    /// later calls return an empty list.
    pub fn take_host_key_additions(&mut self) -> Vec<HostKeyAddition> {
        std::mem::take(&mut self.additions)
    }

    /// Handle to the outcome of the remote command, valid after the
    /// transport was moved into `gix`.
    #[must_use]
    pub fn diagnostics(&self) -> RemoteDiagnostics {
        self.diagnostics.clone()
    }

    /// Value of `GIT_PROTOCOL` for a service. Only fetching requests a
    /// version, and only version 2: `gix` cannot parse the version 1
    /// advertisement of `git upload-pack`, so version 1 is served as version 0.
    fn protocol_variable(&self, service: Service) -> Option<String> {
        match (service, self.desired_version) {
            (Service::UploadPack, Protocol::V2) => Some("version=2".to_owned()),
            _ => None,
        }
    }

    fn open_channel(&mut self, service: Service) -> std::result::Result<(), client::Error> {
        self.connection = None;
        let session = self
            .sessions
            .last()
            .cloned()
            .ok_or(client::Error::MissingHandshake)?;
        let command = target::remote_command(service, self.path.as_bstr());
        let command = command.to_str_lossy().into_owned();
        let status = Status::default();
        self.diagnostics.replace(status.clone(), command.clone());
        let protocol = self.protocol_variable(service);
        let (reader, writer) = self
            .runtime
            .run(
                channel::open(session, command, protocol, status, self.limits.clone()),
                &self.limits,
            )
            .map_err(|error| client::Error::Io(std::io::Error::other(error.to_string())))?;
        self.connection = Some(Connection::new(
            reader,
            writer,
            self.desired_version,
            self.path.clone(),
            None::<(&str, Option<u16>)>,
            ConnectMode::Process,
            false,
        ));
        Ok(())
    }
}

impl TransportWithoutIO for SshTransport {
    fn to_url(&self) -> Cow<'_, BStr> {
        Cow::Borrowed(self.url.as_bstr())
    }

    fn connection_persists_across_multiple_requests(&self) -> bool {
        true
    }

    fn configure(&mut self, _config: &dyn Any) -> gix::Result {
        Ok(())
    }
}

impl Transport for SshTransport {
    fn handshake<'a>(
        &mut self,
        service: Service,
        extra_parameters: &'a [(&'a str, Option<&'a str>)],
    ) -> std::result::Result<SetServiceResponse<'_>, client::Error> {
        self.open_channel(service)?;
        self.connection
            .as_mut()
            .ok_or(client::Error::MissingHandshake)?
            .handshake(service, extra_parameters)
    }

    fn request(
        &mut self,
        write_mode: WriteMode,
        on_into_read: MessageKind,
        trace: bool,
    ) -> std::result::Result<RequestWriter<'_>, client::Error> {
        self.connection
            .as_mut()
            .ok_or(client::Error::MissingHandshake)?
            .request(write_mode, on_into_read, trace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_new_setting_does_not_weaken_an_explicit_strict_host() {
        assert_eq!(
            host_key_policy(KnownHostsSetting::AcceptNew, Some(StrictMode::Yes)),
            Policy::Strict
        );
        assert_eq!(
            host_key_policy(KnownHostsSetting::AcceptNew, None),
            Policy::AcceptNew
        );
        assert_eq!(
            host_key_policy(KnownHostsSetting::Strict, Some(StrictMode::AcceptNew)),
            Policy::AcceptNew
        );
        assert_eq!(
            host_key_policy(KnownHostsSetting::Strict, Some(StrictMode::No)),
            Policy::Off
        );
        assert_eq!(
            host_key_policy(KnownHostsSetting::Strict, None),
            Policy::Strict
        );
    }
}
