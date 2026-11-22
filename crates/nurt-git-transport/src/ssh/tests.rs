// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! End-to-end tests of the SSH transport against an in-process server.
//!
//! The server runs `russh` on its own thread, listens on the loopback
//! interface and answers `git-upload-pack` and `git-receive-pack` with fixed
//! advertisements, so the tests need neither the network nor the `git`
//! program.

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bstr::BStr;
use gix_transport::client::blocking_io::Transport as _;
use gix_transport::{Protocol, Service};
use nurt_contracts::config::KnownHosts as KnownHostsSetting;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::stage::CancelReason;
use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, ChannelOpenFailure, MethodKind, MethodSet};

use super::known_hosts::{fingerprint, host_line};
use super::{HostKeyAddition, SshNotice, SshOptions, SshTransport, connect, record_host_key};

/// Seed of the host key of the test server.
const HOST_SEED: u8 = 200;
/// Seed of the key the client offers.
const USER_SEED: u8 = 1;
/// Seed of a key the server does not know.
const STRANGER_SEED: u8 = 2;
/// Object id used in reference advertisements.
const OID: &str = "1111111111111111111111111111111111111111";
const REPOSITORY: &str = "/srv/repo.git";
/// Path for which the server reports a missing repository.
const MISSING: &str = "/srv/missing.git";
/// Path for which the server refuses the `exec` request.
const FORBIDDEN: &str = "/srv/forbidden.git";
const PASSPHRASE: &str = "correct horse battery staple";
const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const SHORT_TIMEOUT: Duration = Duration::from_millis(300);
const CANCEL_DELAY: Duration = Duration::from_millis(200);
/// Extended data type of standard error (RFC 4254, section 5.2).
const STDERR: u32 = 1;
const EXIT_NOT_A_REPOSITORY: u32 = 128;

fn key(seed: u8) -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn parse_url(text: &str) -> gix_url::Url {
    gix_url::parse(BStr::new(text)).unwrap()
}

fn problem(error: &Error) -> Option<&str> {
    error.data().problem.as_deref()
}

fn pkt(line: &str) -> String {
    format!("{:04x}{line}", line.len() + 4)
}

/// Capability advertisement of protocol version 2.
fn v2_advertisement() -> Vec<u8> {
    let lines = [
        "version 2\n",
        "agent=git/2.53.0\n",
        "ls-refs=unborn\n",
        "fetch=shallow wait-for-done\n",
        "server-option\n",
        "object-format=sha1\n",
    ];
    let mut out: String = lines.iter().map(|line| pkt(line)).collect();
    out.push_str("0000");
    out.into_bytes()
}

/// Reference advertisement of protocol version 0.
fn v0_advertisement(service: &str) -> Vec<u8> {
    let mut out = if service == "git-receive-pack" {
        pkt(&format!(
            "{OID} refs/heads/main\0report-status delete-refs ofs-delta agent=git/2.53.0\n"
        ))
    } else {
        let mut out = pkt(&format!(
            "{OID} HEAD\0multi_ack thin-pack side-band side-band-64k ofs-delta shallow no-progress include-tag symref=HEAD:refs/heads/main agent=git/2.53.0\n"
        ));
        out.push_str(&pkt(&format!("{OID} refs/heads/main\n")));
        out
    };
    out.push_str("0000");
    out.into_bytes()
}

/// Answer of the test server to an `exec` request.
enum Reply {
    Run {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        status: u32,
    },
    Refuse,
}

fn reply(command: &str, protocol: Option<&str>) -> Reply {
    let Some((service, quoted)) = command.split_once(' ') else {
        return Reply::Refuse;
    };
    let path = quoted.trim_matches('\'');
    if !matches!(service, "git-upload-pack" | "git-receive-pack") || path == FORBIDDEN {
        return Reply::Refuse;
    }
    if path == MISSING {
        return Reply::Run {
            stdout: Vec::new(),
            stderr: format!("fatal: '{path}' does not appear to be a git repository\n")
                .into_bytes(),
            status: EXIT_NOT_A_REPOSITORY,
        };
    }
    let stdout = match protocol {
        Some("version=2") if service == "git-upload-pack" => v2_advertisement(),
        _ => v0_advertisement(service),
    };
    Reply::Run {
        stdout,
        stderr: Vec::new(),
        status: 0,
    }
}

/// Requests seen by the test server.
#[derive(Debug, Default)]
struct ServerLog {
    users: Vec<String>,
    env: Vec<(String, String)>,
    commands: Vec<String>,
    forwards: Vec<(String, u32)>,
}

#[derive(Clone)]
struct ServerSettings {
    host_key: PrivateKey,
    authorized: Vec<PublicKey>,
    methods: MethodSet,
    honour_protocol: bool,
}

impl ServerSettings {
    fn new(authorized: &[&PrivateKey]) -> Self {
        Self {
            host_key: key(HOST_SEED),
            authorized: authorized
                .iter()
                .map(|key| key.public_key().clone())
                .collect(),
            methods: MethodSet::from(&[MethodKind::PublicKey][..]),
            honour_protocol: true,
        }
    }
}

struct Connection {
    settings: Arc<ServerSettings>,
    log: Arc<Mutex<ServerLog>>,
    protocol: HashMap<ChannelId, String>,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl server::Handler for Connection {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        // The server library advertises `Config::methods` but still calls
        // this handler for a public key request; the test server enforces
        // the advertised set itself.
        if !self.settings.methods.contains(&MethodKind::PublicKey) {
            return Ok(Auth::Reject {
                proceed_with_methods: Some(self.settings.methods.clone()),
                partial_success: false,
            });
        }
        let known = self
            .settings
            .authorized
            .iter()
            .any(|authorized| authorized.key_data() == key.public_key.key_data());
        if !known {
            return Ok(Auth::reject());
        }
        lock(&self.log).users.push(user.to_owned());
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        lock(&self.log)
            .forwards
            .push((host_to_connect.to_owned(), port_to_connect));
        let port = u16::try_from(port_to_connect).unwrap_or_default();
        match tokio::net::TcpStream::connect((host_to_connect, port)).await {
            Ok(mut tcp) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
                });
            }
            Err(_) => reply.reject(ChannelOpenFailure::ConnectFailed).await,
        }
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        variable_name: &str,
        variable_value: &str,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        lock(&self.log)
            .env
            .push((variable_name.to_owned(), variable_value.to_owned()));
        if variable_name == "GIT_PROTOCOL" && self.settings.honour_protocol {
            self.protocol.insert(channel, variable_value.to_owned());
        }
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).into_owned();
        let answer = reply(&command, self.protocol.get(&channel).map(String::as_str));
        lock(&self.log).commands.push(command);
        match answer {
            Reply::Refuse => session.channel_failure(channel)?,
            Reply::Run {
                stdout,
                stderr,
                status,
            } => {
                session.channel_success(channel)?;
                if !stdout.is_empty() {
                    session.data(channel, stdout)?;
                }
                if !stderr.is_empty() {
                    session.extended_data(channel, STDERR, stderr)?;
                }
                session.exit_status_request(channel, status)?;
                session.eof(channel)?;
            }
        }
        session.close(channel)?;
        Ok(())
    }
}

/// SSH server on a loopback port, stopped when dropped.
struct TestServer {
    port: u16,
    host_key: PublicKey,
    log: Arc<Mutex<ServerLog>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TestServer {
    fn start(settings: ServerSettings) -> Self {
        let log = Arc::new(Mutex::new(ServerLog::default()));
        let host_key = settings.host_key.public_key().clone();
        let (port_tx, port_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let shared = Arc::clone(&log);
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(serve(settings, shared, port_tx, stop_rx));
        });
        let port = port_rx.recv().unwrap();
        Self {
            port,
            host_key,
            log,
            stop: Some(stop_tx),
            thread: Some(thread),
        }
    }

    fn log(&self) -> MutexGuard<'_, ServerLog> {
        lock(&self.log)
    }

    fn url(&self, path: &str) -> gix_url::Url {
        parse_url(&format!("ssh://git@127.0.0.1:{}{path}", self.port))
    }

    fn known_hosts_name(&self) -> String {
        format!("[127.0.0.1]:{}", self.port)
    }

    fn known_hosts_line(&self) -> String {
        host_line(&self.known_hosts_name(), &self.host_key).unwrap()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn serve(
    settings: ServerSettings,
    log: Arc<Mutex<ServerLog>>,
    port: std::sync::mpsc::Sender<u16>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    port.send(listener.local_addr().unwrap().port()).unwrap();
    let config = Arc::new(server::Config {
        keys: vec![settings.host_key.clone()],
        methods: settings.methods.clone(),
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        inactivity_timeout: Some(OPERATION_TIMEOUT),
        ..server::Config::default()
    });
    let settings = Arc::new(settings);
    loop {
        tokio::select! {
            _ = &mut stop => return,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else {
                    return;
                };
                let handler = Connection {
                    settings: Arc::clone(&settings),
                    log: Arc::clone(&log),
                    protocol: HashMap::new(),
                    channels: HashMap::new(),
                };
                let config = Arc::clone(&config);
                tokio::spawn(async move {
                    if let Ok(session) = server::run_stream(config, stream, handler).await {
                        let _ = session.await;
                    }
                });
            }
        }
    }
}

/// Home directory, SSH files and options of the client side.
struct Client {
    _dir: tempfile::TempDir,
    options: SshOptions,
}

impl Client {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        let mut options = SshOptions::new(&home, "user");
        options.system_dir = Some(dir.path().join("etc-ssh"));
        options.nurt_known_hosts = Some(dir.path().join("nurt").join("known_hosts"));
        options.timeout = Some(OPERATION_TIMEOUT);
        options.retry_command = Some("git.fetch \"/home/user/repo\"".to_owned());
        Self { _dir: dir, options }
    }

    fn ssh_dir(&self) -> PathBuf {
        self.options.home.join(".ssh")
    }

    fn write_identity(&self, name: &str, key: &PrivateKey, passphrase: Option<&str>) -> PathBuf {
        let stored = match passphrase {
            Some(passphrase) => key
                .encrypt(&mut russh::keys::key::safe_rng(), passphrase)
                .unwrap(),
            None => key.clone(),
        };
        let path = self.ssh_dir().join(name);
        std::fs::write(&path, stored.to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
        let public = self.ssh_dir().join(format!("{name}.pub"));
        std::fs::write(public, key.public_key().to_openssh().unwrap()).unwrap();
        path
    }

    fn trust(&self, line: &str) {
        let path = self.ssh_dir().join("known_hosts");
        let mut text = std::fs::read_to_string(&path).unwrap_or_default();
        text.push_str(line);
        text.push('\n');
        std::fs::write(path, text).unwrap();
    }

    fn write_config(&self, text: &str) {
        std::fs::write(self.ssh_dir().join("config"), text).unwrap();
    }

    fn connect(&self, url: &gix_url::Url) -> Result<SshTransport, Error> {
        connect(url, Protocol::V2, &self.options)
    }
}

/// Client with a trusted server and an authorized key file.
fn trusted_pair() -> (Client, TestServer) {
    let client = Client::new();
    let user_key = key(USER_SEED);
    client.write_identity("id_ed25519", &user_key, None);
    let server = TestServer::start(ServerSettings::new(&[&user_key]));
    client.trust(&server.known_hosts_line());
    (client, server)
}

fn handshake(transport: &mut SshTransport, service: Service) -> (Protocol, bool) {
    let response = transport.handshake(service, &[]).unwrap();
    (response.actual_protocol, response.refs.is_some())
}

#[test]
fn fetch_requests_protocol_version_2_through_the_environment() {
    let (client, server) = trusted_pair();
    let mut transport = client.connect(&server.url(REPOSITORY)).unwrap();
    assert_eq!(
        handshake(&mut transport, Service::UploadPack),
        (Protocol::V2, false)
    );
    assert!(transport.take_host_key_additions().is_empty());
    let log = server.log();
    assert_eq!(log.users, ["git"]);
    assert_eq!(
        log.env,
        [("GIT_PROTOCOL".to_owned(), "version=2".to_owned())]
    );
    assert_eq!(log.commands, [format!("git-upload-pack '{REPOSITORY}'")]);
}

#[test]
fn server_without_protocol_v2_answers_with_references() {
    let client = Client::new();
    let user_key = key(USER_SEED);
    client.write_identity("id_ed25519", &user_key, None);
    let mut settings = ServerSettings::new(&[&user_key]);
    settings.honour_protocol = false;
    let server = TestServer::start(settings);
    client.trust(&server.known_hosts_line());
    let mut transport = client.connect(&server.url(REPOSITORY)).unwrap();
    let (protocol, has_refs) = handshake(&mut transport, Service::UploadPack);
    assert_ne!(protocol, Protocol::V2);
    assert!(has_refs);
}

#[test]
fn push_runs_receive_pack_without_protocol_variable() {
    let (client, server) = trusted_pair();
    let mut transport = client.connect(&server.url(REPOSITORY)).unwrap();
    let (_, has_refs) = handshake(&mut transport, Service::ReceivePack);
    assert!(has_refs);
    let log = server.log();
    assert!(log.env.is_empty());
    assert_eq!(log.commands, [format!("git-receive-pack '{REPOSITORY}'")]);
}

#[test]
fn one_session_serves_several_channels() {
    let (client, server) = trusted_pair();
    let mut transport = client.connect(&server.url(REPOSITORY)).unwrap();
    handshake(&mut transport, Service::UploadPack);
    handshake(&mut transport, Service::ReceivePack);
    let log = server.log();
    assert_eq!(log.users.len(), 1);
    assert_eq!(log.commands.len(), 2);
}

#[test]
fn missing_repository_is_reported_with_the_server_message() {
    let (client, server) = trusted_pair();
    let mut transport = client.connect(&server.url(MISSING)).unwrap();
    let diagnostics = transport.diagnostics();
    assert!(transport.handshake(Service::UploadPack, &[]).is_err());
    let error = diagnostics.error().unwrap();
    assert_eq!(error.code(), ErrorCode::GitProtocol);
    assert_eq!(problem(&error), Some("repository_not_found"));
    assert!(error.to_string().contains("exit status 128"));
    assert!(
        diagnostics
            .stderr()
            .contains("does not appear to be a git repository")
    );
}

#[test]
fn refused_command_is_reported() {
    let (client, server) = trusted_pair();
    let mut transport = client.connect(&server.url(FORBIDDEN)).unwrap();
    let diagnostics = transport.diagnostics();
    assert!(transport.handshake(Service::UploadPack, &[]).is_err());
    let error = diagnostics.error().unwrap();
    assert!(error.to_string().contains("server refused to run"));
}

#[test]
fn unknown_host_is_refused_with_its_fingerprint() {
    let client = Client::new();
    let user_key = key(USER_SEED);
    client.write_identity("id_ed25519", &user_key, None);
    let server = TestServer::start(ServerSettings::new(&[&user_key]));
    let error = client.connect(&server.url(REPOSITORY)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitHostKey);
    assert_eq!(problem(&error), Some("unknown"));
    let accepted = format!("accept_host_key: \"{}\"", fingerprint(&server.host_key));
    assert!(error.data().fix.iter().any(|fix| fix.ends_with(&accepted)));
    assert!(server.log().users.is_empty());
}

#[test]
fn changed_host_key_is_refused() {
    let client = Client::new();
    let user_key = key(USER_SEED);
    client.write_identity("id_ed25519", &user_key, None);
    let server = TestServer::start(ServerSettings::new(&[&user_key]));
    let other = key(STRANGER_SEED);
    client.trust(&host_line(&server.known_hosts_name(), other.public_key()).unwrap());
    let mut client = client;
    client.options.known_hosts = KnownHostsSetting::AcceptNew;
    let error = client.connect(&server.url(REPOSITORY)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitHostKey);
    assert_eq!(problem(&error), Some("changed"));
}

#[test]
fn accept_new_returns_the_key_to_record() {
    let mut client = Client::new();
    let user_key = key(USER_SEED);
    client.write_identity("id_ed25519", &user_key, None);
    let server = TestServer::start(ServerSettings::new(&[&user_key]));
    client.options.known_hosts = KnownHostsSetting::AcceptNew;
    let mut transport = client.connect(&server.url(REPOSITORY)).unwrap();
    let additions = transport.take_host_key_additions();
    assert_eq!(additions.len(), 1);
    assert_eq!(
        Some(&additions[0].file),
        client.options.nurt_known_hosts.as_ref()
    );
    assert_eq!(additions[0].line, server.known_hosts_line());
    assert_eq!(additions[0].fingerprint, fingerprint(&server.host_key));
    assert!(transport.take_host_key_additions().is_empty());
    drop(transport);

    let file = &additions[0].file;
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, format!("{}\n", additions[0].line)).unwrap();
    client.options.known_hosts = KnownHostsSetting::Strict;
    let mut transport = client.connect(&server.url(REPOSITORY)).unwrap();
    assert!(transport.take_host_key_additions().is_empty());
}

#[test]
fn accepted_fingerprint_admits_only_that_key() {
    let mut client = Client::new();
    let user_key = key(USER_SEED);
    client.write_identity("id_ed25519", &user_key, None);
    let server = TestServer::start(ServerSettings::new(&[&user_key]));
    client.options.accept_host_key = Some(fingerprint(key(STRANGER_SEED).public_key()));
    let error = client.connect(&server.url(REPOSITORY)).unwrap_err();
    assert_eq!(problem(&error), Some("fingerprint_mismatch"));
    client.options.accept_host_key = Some(fingerprint(&server.host_key));
    let mut transport = client.connect(&server.url(REPOSITORY)).unwrap();
    assert_eq!(transport.take_host_key_additions().len(), 1);
}

#[test]
fn strict_checking_off_accepts_with_a_notice() {
    let client = Client::new();
    let user_key = key(USER_SEED);
    client.write_identity("id_ed25519", &user_key, None);
    client.write_config("Host 127.0.0.1\n  StrictHostKeyChecking no\n");
    let server = TestServer::start(ServerSettings::new(&[&user_key]));
    let mut transport = client.connect(&server.url(REPOSITORY)).unwrap();
    assert!(transport.take_host_key_additions().is_empty());
    assert!(
        transport
            .notices()
            .iter()
            .any(|notice| matches!(notice, SshNotice::HostKeyUnverified { .. }))
    );
}

#[test]
fn host_key_alias_names_the_known_hosts_entry() {
    let client = Client::new();
    let user_key = key(USER_SEED);
    client.write_identity("id_ed25519", &user_key, None);
    client.write_config("Host 127.0.0.1\n  HostKeyAlias build-box\n");
    let server = TestServer::start(ServerSettings::new(&[&user_key]));
    client.trust(&host_line("build-box", &server.host_key).unwrap());
    let mut transport = client.connect(&server.url(REPOSITORY)).unwrap();
    assert!(transport.take_host_key_additions().is_empty());
}

#[test]
fn configured_alias_selects_host_port_account_and_key() {
    let client = Client::new();
    let user_key = key(USER_SEED);
    client.write_identity("deploy_key", &user_key, None);
    let server = TestServer::start(ServerSettings::new(&[&user_key]));
    client.trust(&server.known_hosts_line());
    client.write_config(&format!(
        "Host repo-host\n  HostName 127.0.0.1\n  Port {}\n  User deploy\n  IdentityFile ~/.ssh/deploy_key\n  IdentitiesOnly yes\n",
        server.port
    ));
    let mut transport = client
        .connect(&parse_url("repo-host:srv/repo.git"))
        .unwrap();
    handshake(&mut transport, Service::UploadPack);
    let log = server.log();
    assert_eq!(log.users, ["deploy"]);
    assert_eq!(log.commands, ["git-upload-pack 'srv/repo.git'"]);
}

#[test]
fn proxy_jump_reaches_the_target_through_forwarding() {
    let (client, server) = trusted_pair();
    client.write_config(&format!(
        "Host target\n  HostName 127.0.0.1\n  Port {port}\n  ProxyJump jumper@127.0.0.1:{port}\n",
        port = server.port
    ));
    let mut transport = client
        .connect(&parse_url("ssh://git@target/srv/repo.git"))
        .unwrap();
    handshake(&mut transport, Service::UploadPack);
    let log = server.log();
    assert_eq!(
        log.forwards,
        [("127.0.0.1".to_owned(), u32::from(server.port))]
    );
    assert_eq!(log.users, ["jumper", "git"]);
}

#[test]
fn proxy_command_is_not_run() {
    let client = Client::new();
    client.write_config("Host target\n  ProxyCommand nc %h %p\n");
    let error = client
        .connect(&parse_url("ssh://git@target/repo.git"))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitUnsupported);
    assert_eq!(problem(&error), Some("proxy_command"));
}

#[test]
fn ignored_programs_are_reported_as_notices() {
    let (mut client, server) = trusted_pair();
    client.write_config("Match exec \"test -f /etc/hosts\"\n  User nobody\n");
    client.options.ignored_ssh_command = vec!["core.sshCommand".to_owned()];
    let transport = client.connect(&server.url(REPOSITORY)).unwrap();
    let notices = transport.notices();
    assert!(notices.contains(&SshNotice::SshCommandIgnored {
        source: "core.sshCommand".to_owned()
    }));
    assert!(
        notices
            .iter()
            .any(|notice| matches!(notice, SshNotice::MatchExecIgnored { line: 1, .. }))
    );
    assert_eq!(server.log().users, ["git"]);
}

#[test]
fn rejected_keys_are_an_auth_error() {
    let client = Client::new();
    client.write_identity("id_ed25519", &key(USER_SEED), None);
    let server = TestServer::start(ServerSettings::new(&[&key(STRANGER_SEED)]));
    client.trust(&server.known_hosts_line());
    let error = client.connect(&server.url(REPOSITORY)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitAuth);
    assert_eq!(problem(&error), Some("rejected"));
}

#[test]
fn missing_keys_are_an_auth_error() {
    let client = Client::new();
    let server = TestServer::start(ServerSettings::new(&[&key(USER_SEED)]));
    client.trust(&server.known_hosts_line());
    let error = client.connect(&server.url(REPOSITORY)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitAuth);
    assert_eq!(problem(&error), Some("missing_credentials"));
}

#[test]
fn encrypted_key_without_agent_needs_ssh_add() {
    let client = Client::new();
    let user_key = key(USER_SEED);
    client.write_identity("id_ed25519", &user_key, Some(PASSPHRASE));
    let server = TestServer::start(ServerSettings::new(&[&user_key]));
    client.trust(&server.known_hosts_line());
    let error = client.connect(&server.url(REPOSITORY)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitAuth);
    assert_eq!(problem(&error), Some("key_locked"));
    assert!(error.data().fix.iter().any(|fix| fix.contains("ssh-add")));
}

#[test]
fn server_without_public_key_authentication_is_unsupported() {
    let client = Client::new();
    let user_key = key(USER_SEED);
    client.write_identity("id_ed25519", &user_key, None);
    let mut settings = ServerSettings::new(&[&user_key]);
    settings.methods = MethodSet::from(&[MethodKind::Password][..]);
    let server = TestServer::start(settings);
    client.trust(&server.known_hosts_line());
    let error = client.connect(&server.url(REPOSITORY)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitUnsupported);
    assert_eq!(problem(&error), Some("interactive_auth"));
}

#[test]
fn silent_server_hits_the_connect_timeout() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut client = Client::new();
    client.options.connect_timeout = SHORT_TIMEOUT;
    let error = client
        .connect(&parse_url(&format!(
            "ssh://git@127.0.0.1:{port}/srv/repo.git"
        )))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Timeout);
    drop(listener);
}

#[test]
fn cancellation_stops_a_pending_connection() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let client = Client::new();
    let cancel = client.options.cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(CANCEL_DELAY);
        cancel.cancel(CancelReason::Requested);
    });
    let error = client
        .connect(&parse_url(&format!(
            "ssh://git@127.0.0.1:{port}/srv/repo.git"
        )))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Cancelled);
    canceller.join().unwrap();
    drop(listener);
}

#[test]
fn record_host_key_creates_the_file_and_its_directory() {
    use nurt_contracts::fs::AbsPath;
    use nurt_contracts::journal::JournalStore as _;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::journal::RecordingJournal;
    use nurt_testkit::vfs::VirtualFs;

    let dir = tempfile::tempdir().unwrap();
    let vfs = Arc::new(VirtualFs::default());
    vfs.create_dir_all(&AbsPath::new(dir.path()).unwrap())
        .unwrap();
    let journal = RecordingJournal::new(vfs.clone());
    let file = dir.path().join("nurt").join("known_hosts");
    let addition = HostKeyAddition {
        file: file.clone(),
        line: "[git.example.com]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA".to_owned(),
        host: "[git.example.com]:2222".to_owned(),
        fingerprint: "SHA256:test".to_owned(),
    };
    let mut tx = journal.begin(begin_request("git.fetch"));
    record_host_key(tx.as_mut(), &addition).unwrap();
    tx.commit().unwrap();
    let written = vfs.read_file(&AbsPath::new(&file).unwrap()).unwrap();
    assert_eq!(written, format!("{}\n", addition.line).into_bytes());
}

#[test]
fn record_host_key_appends_after_an_unterminated_line() {
    use nurt_contracts::fs::AbsPath;
    use nurt_contracts::journal::JournalStore as _;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::journal::RecordingJournal;
    use nurt_testkit::vfs::VirtualFs;

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("known_hosts");
    std::fs::write(&file, "old.example.com ssh-ed25519 AAAA").unwrap();
    let vfs = Arc::new(VirtualFs::default());
    vfs.create_dir_all(&AbsPath::new(dir.path()).unwrap())
        .unwrap();
    vfs.write_file(
        &AbsPath::new(&file).unwrap(),
        b"old.example.com ssh-ed25519 AAAA",
    )
    .unwrap();
    let journal = RecordingJournal::new(vfs.clone());
    let addition = HostKeyAddition {
        file: file.clone(),
        line: "new.example.com ssh-ed25519 BBBB".to_owned(),
        host: "new.example.com".to_owned(),
        fingerprint: "SHA256:test".to_owned(),
    };
    let mut tx = journal.begin(begin_request("git.fetch"));
    record_host_key(tx.as_mut(), &addition).unwrap();
    tx.commit().unwrap();
    let written = vfs.read_file(&AbsPath::new(&file).unwrap()).unwrap();
    assert_eq!(
        written,
        b"old.example.com ssh-ed25519 AAAA\nnew.example.com ssh-ed25519 BBBB\n".to_vec()
    );
}

#[cfg(unix)]
mod unix {
    use std::os::unix::net::{UnixListener, UnixStream};

    use russh::keys::signature::Signer as _;
    use russh::keys::ssh_encoding::Encode as _;

    use super::*;

    /// Message numbers of the SSH agent protocol (draft-miller-ssh-agent).
    const AGENT_FAILURE: u8 = 5;
    const AGENTC_REQUEST_IDENTITIES: u8 = 11;
    const AGENT_IDENTITIES_ANSWER: u8 = 12;
    const AGENTC_SIGN_REQUEST: u8 = 13;
    const AGENT_SIGN_RESPONSE: u8 = 14;

    fn put_string(out: &mut Vec<u8>, value: &[u8]) {
        out.extend_from_slice(&u32::try_from(value.len()).unwrap().to_be_bytes());
        out.extend_from_slice(value);
    }

    fn take_string<'a>(bytes: &mut &'a [u8]) -> Option<&'a [u8]> {
        let (length, rest) = bytes.split_first_chunk::<4>()?;
        let length = usize::try_from(u32::from_be_bytes(*length)).ok()?;
        if rest.len() < length {
            return None;
        }
        let (value, rest) = rest.split_at(length);
        *bytes = rest;
        Some(value)
    }

    fn agent_answer(message: &[u8], key: &PrivateKey) -> Vec<u8> {
        let blob = key.public_key().key_data().encode_vec().unwrap();
        let Some((&kind, mut body)) = message.split_first() else {
            return vec![AGENT_FAILURE];
        };
        match kind {
            AGENTC_REQUEST_IDENTITIES => {
                let mut out = vec![AGENT_IDENTITIES_ANSWER];
                out.extend_from_slice(&1u32.to_be_bytes());
                put_string(&mut out, &blob);
                put_string(&mut out, b"test agent key");
                out
            }
            AGENTC_SIGN_REQUEST => {
                let (Some(requested), Some(data)) =
                    (take_string(&mut body), take_string(&mut body))
                else {
                    return vec![AGENT_FAILURE];
                };
                if requested != blob.as_slice() {
                    return vec![AGENT_FAILURE];
                }
                let signature: russh::keys::ssh_key::Signature = key.try_sign(data).unwrap();
                let mut out = vec![AGENT_SIGN_RESPONSE];
                put_string(&mut out, &signature.encode_vec().unwrap());
                out
            }
            _ => vec![AGENT_FAILURE],
        }
    }

    fn serve_agent(mut stream: UnixStream, key: &PrivateKey) {
        loop {
            let mut length = [0u8; 4];
            if stream.read_exact(&mut length).is_err() {
                return;
            }
            let mut message = vec![0u8; usize::try_from(u32::from_be_bytes(length)).unwrap()];
            if stream.read_exact(&mut message).is_err() {
                return;
            }
            let answer = agent_answer(&message, key);
            let length = u32::try_from(answer.len()).unwrap().to_be_bytes();
            if stream
                .write_all(&length)
                .and_then(|()| stream.write_all(&answer))
                .is_err()
            {
                return;
            }
        }
    }

    /// Starts an agent holding `key` on a socket in `dir`.
    fn start_agent(dir: &Path, key: PrivateKey) -> PathBuf {
        let path = dir.join("agent.sock");
        let listener = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    return;
                };
                let key = key.clone();
                std::thread::spawn(move || serve_agent(stream, &key));
            }
        });
        path
    }

    #[test]
    fn agent_key_authenticates_without_key_files() {
        let agent_dir = tempfile::tempdir().unwrap();
        let user_key = key(USER_SEED);
        let mut client = Client::new();
        client.options.auth_sock = Some(start_agent(agent_dir.path(), user_key.clone()));
        let server = TestServer::start(ServerSettings::new(&[&user_key]));
        client.trust(&server.known_hosts_line());
        let mut transport = client.connect(&server.url(REPOSITORY)).unwrap();
        handshake(&mut transport, Service::UploadPack);
        assert_eq!(server.log().users, ["git"]);
    }

    #[test]
    fn agent_holds_the_encrypted_key_file() {
        let agent_dir = tempfile::tempdir().unwrap();
        let user_key = key(USER_SEED);
        let mut client = Client::new();
        client.write_identity("id_ed25519", &user_key, Some(PASSPHRASE));
        client.write_config("Host *\n  IdentitiesOnly yes\n");
        client.options.auth_sock = Some(start_agent(agent_dir.path(), user_key.clone()));
        let server = TestServer::start(ServerSettings::new(&[&user_key]));
        client.trust(&server.known_hosts_line());
        assert!(client.connect(&server.url(REPOSITORY)).is_ok());
    }

    #[test]
    fn passphrase_file_unlocks_the_encrypted_key() {
        use std::os::unix::fs::PermissionsExt;

        let mut client = Client::new();
        let user_key = key(USER_SEED);
        client.write_identity("id_ed25519", &user_key, Some(PASSPHRASE));
        let passphrase_file = client.ssh_dir().join("passphrase");
        std::fs::write(&passphrase_file, format!("{PASSPHRASE}\n")).unwrap();
        std::fs::set_permissions(&passphrase_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        client.options.passphrase_file = Some(passphrase_file);
        let server = TestServer::start(ServerSettings::new(&[&user_key]));
        client.trust(&server.known_hosts_line());
        assert!(client.connect(&server.url(REPOSITORY)).is_ok());
    }
}
