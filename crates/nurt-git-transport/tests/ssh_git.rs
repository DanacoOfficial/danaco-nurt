// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Fetch and push over SSH against the `git` program.
//!
//! A `russh` server on the loopback interface answers every `exec` request
//! with `git upload-pack` or `git receive-pack` and relays the channel to
//! the pipes of that process. The tests need the `git` program and are
//! ignored by default:
//! `cargo test -p nurt-git-transport --test ssh_git -- --ignored`.

#![cfg(feature = "git-ssh")]
// Test code: a failed `unwrap` fails the test, and the reference `git` program
// is started directly because the tests compare against it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bstr::BStr;
use gix::ObjectId;
use gix_transport::Protocol;
use gix_transport::client::blocking_io::Transport;
use nurt_contracts::error::ErrorCode;
use nurt_git_transport::connect::{self, Direction, TransportOptions};
use nurt_git_transport::http::HttpSettings;
use nurt_git_transport::send_pack::{self, RefCommand, SendOptions};
use nurt_git_transport::ssh::{self, SshOptions, SshTransport};
use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, ChannelMsg};
use tokio::sync::mpsc;

/// Seed of the host key of the server.
const HOST_SEED: u8 = 200;
/// Seed of the key of the client.
const USER_SEED: u8 = 1;
/// Extended data type of standard error (RFC 4254, section 5.2).
const STDERR: u32 = 1;
/// Exit status sent when `git` cannot be waited for or ends on a signal.
const EXIT_FAILURE: u32 = 255;
/// Bytes read from a pipe of `git` at once.
const CHUNK: usize = 32 * 1024;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(60);
const AUTHOR: &str = "Nurt Test";
const EMAIL: &str = "test@nurt.invalid";

fn key(seed: u8) -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `git` with an isolated home directory, without system configuration and
/// with a fixed identity.
fn git_command(home: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", AUTHOR)
        .env("GIT_AUTHOR_EMAIL", EMAIL)
        .env("GIT_COMMITTER_NAME", AUTHOR)
        .env("GIT_COMMITTER_EMAIL", EMAIL)
        .env_remove("GIT_DIR")
        .env_remove("GIT_PROTOCOL");
    command
}

/// Requests seen by the server.
#[derive(Debug, Default)]
struct ServerLog {
    users: Vec<String>,
    commands: Vec<String>,
}

struct Handler {
    authorized: PublicKey,
    home: PathBuf,
    log: Arc<Mutex<ServerLog>>,
    protocol: HashMap<ChannelId, String>,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl Handler {
    /// Starts `git upload-pack` or `git receive-pack` for an `exec` command
    /// in the form sent by Git clients.
    fn spawn_git(&self, command: &str, protocol: Option<&String>) -> Option<Child> {
        let (program, quoted) = command.split_once(' ')?;
        let service = match program {
            "git-upload-pack" => "upload-pack",
            "git-receive-pack" => "receive-pack",
            _ => return None,
        };
        let path = quoted.strip_prefix('\'')?.strip_suffix('\'')?;
        let mut git = git_command(&self.home);
        git.arg(service)
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(value) = protocol {
            git.env("GIT_PROTOCOL", value);
        }
        git.spawn().ok()
    }
}

impl server::Handler for Handler {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        if self.authorized.key_data() != key.public_key.key_data() {
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

    async fn env_request(
        &mut self,
        channel: ChannelId,
        variable_name: &str,
        variable_value: &str,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if variable_name == "GIT_PROTOCOL" {
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
        let child = self.spawn_git(&command, self.protocol.get(&channel));
        lock(&self.log).commands.push(command);
        match (child, self.channels.remove(&channel)) {
            (Some(child), Some(handle)) => {
                session.channel_success(channel)?;
                tokio::spawn(relay(handle, child));
            }
            _ => {
                session.channel_failure(channel)?;
                session.close(channel)?;
            }
        }
        Ok(())
    }
}

/// Output of the `git` process, in the order it is sent to the client.
enum Output {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit(u32),
}

fn drain(
    pipe: Option<impl Read>,
    output: &mpsc::UnboundedSender<Output>,
    wrap: fn(Vec<u8>) -> Output,
) {
    let Some(mut pipe) = pipe else {
        return;
    };
    let mut buffer = vec![0; CHUNK];
    while let Ok(count) = pipe.read(&mut buffer) {
        if count == 0 || output.send(wrap(buffer[..count].to_vec())).is_err() {
            return;
        }
    }
}

fn feed(stdin: Option<ChildStdin>, input: &std::sync::mpsc::Receiver<Vec<u8>>) {
    let Some(mut stdin) = stdin else {
        return;
    };
    for chunk in input {
        if stdin.write_all(&chunk).is_err() {
            return;
        }
    }
}

/// Moves the channel data to the standard input of `git` and its standard
/// output, standard error and exit status to the channel. End of file from
/// the client closes the standard input.
async fn relay(channel: Channel<Msg>, mut child: Child) {
    let (mut reader, writer) = channel.split();
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let stdin = child.stdin.take();
    std::thread::spawn(move || feed(stdin, &input_rx));
    let (output_tx, mut output_rx) = mpsc::unbounded_channel();
    let stderr_tx = output_tx.clone();
    let stderr = child.stderr.take();
    let stderr_thread = std::thread::spawn(move || drain(stderr, &stderr_tx, Output::Stderr));
    let stdout = child.stdout.take();
    std::thread::spawn(move || {
        drain(stdout, &output_tx, Output::Stdout);
        let _ = stderr_thread.join();
        let status = child
            .wait()
            .ok()
            .and_then(|status| status.code())
            .and_then(|code| u32::try_from(code).ok())
            .unwrap_or(EXIT_FAILURE);
        let _ = output_tx.send(Output::Exit(status));
    });
    tokio::spawn(async move {
        while let Some(message) = reader.wait().await {
            match message {
                ChannelMsg::Data { data } => {
                    if input_tx.send(data.to_vec()).is_err() {
                        return;
                    }
                }
                ChannelMsg::Eof | ChannelMsg::Close => return,
                _ => {}
            }
        }
    });
    while let Some(output) = output_rx.recv().await {
        let sent = match output {
            Output::Stdout(bytes) => writer.data_bytes(bytes).await,
            Output::Stderr(bytes) => writer.extended_data_bytes(STDERR, bytes).await,
            Output::Exit(status) => {
                let _ = writer.exit_status(status).await;
                let _ = writer.eof().await;
                let _ = writer.close().await;
                return;
            }
        };
        if sent.is_err() {
            return;
        }
    }
}

/// SSH server on a loopback port, stopped when dropped.
struct GitServer {
    port: u16,
    host_key: PublicKey,
    log: Arc<Mutex<ServerLog>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl GitServer {
    fn start(home: &Path, authorized: PublicKey) -> Self {
        let host_key = key(HOST_SEED);
        let public = host_key.public_key().clone();
        let log = Arc::new(Mutex::new(ServerLog::default()));
        let (port_tx, port_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let shared = Arc::clone(&log);
        let home = home.to_path_buf();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(serve(host_key, authorized, home, shared, port_tx, stop_rx));
        });
        let port = port_rx.recv().unwrap();
        Self {
            port,
            host_key: public,
            log,
            stop: Some(stop_tx),
            thread: Some(thread),
        }
    }

    fn log(&self) -> MutexGuard<'_, ServerLog> {
        lock(&self.log)
    }
}

impl Drop for GitServer {
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
    host_key: PrivateKey,
    authorized: PublicKey,
    home: PathBuf,
    log: Arc<Mutex<ServerLog>>,
    port: std::sync::mpsc::Sender<u16>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    port.send(listener.local_addr().unwrap().port()).unwrap();
    let config = Arc::new(server::Config {
        keys: vec![host_key],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        inactivity_timeout: Some(OPERATION_TIMEOUT),
        ..server::Config::default()
    });
    loop {
        tokio::select! {
            _ = &mut stop => return,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else {
                    return;
                };
                let handler = Handler {
                    authorized: authorized.clone(),
                    home: home.clone(),
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

/// Temporary directory with a home directory, the running server and the
/// client options trusting it.
struct Fixture {
    dir: tempfile::TempDir,
    server: GitServer,
    options: SshOptions,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let ssh_dir = home.join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let user_key = key(USER_SEED);
        std::fs::write(
            ssh_dir.join("id_ed25519"),
            user_key.to_openssh(LineEnding::LF).unwrap().as_bytes(),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                ssh_dir.join("id_ed25519"),
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        let server = GitServer::start(&home, user_key.public_key().clone());
        std::fs::write(
            ssh_dir.join("known_hosts"),
            format!(
                "[127.0.0.1]:{} {}\n",
                server.port,
                server.host_key.to_openssh().unwrap()
            ),
        )
        .unwrap();
        let mut options = SshOptions::new(&home, "user");
        options.system_dir = Some(dir.path().join("etc-ssh"));
        options.nurt_known_hosts = Some(dir.path().join("nurt").join("known_hosts"));
        options.timeout = Some(OPERATION_TIMEOUT);
        Self {
            dir,
            server,
            options,
        }
    }

    fn home(&self) -> PathBuf {
        self.options.home.clone()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Runs `git` in `cwd` and returns its trimmed standard output.
    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let output = git_command(&self.home())
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// Creates a repository with branch `main` beside the home directory.
    fn init(&self, name: &str, bare: bool) -> PathBuf {
        let mut args = vec!["init", "-q", "-b", "main"];
        if bare {
            args.push("--bare");
        }
        args.push(name);
        self.git(self.dir.path(), &args);
        let repo = self.path(name);
        self.git(&repo, &["config", "user.name", "Test User"]);
        self.git(&repo, &["config", "user.email", "test@example.com"]);
        repo
    }

    /// Writes a file, commits it and returns the commit identifier.
    fn commit(&self, repo: &Path, file: &str, text: &str) -> ObjectId {
        std::fs::write(repo.join(file), text).unwrap();
        self.git(repo, &["add", file]);
        self.git(repo, &["commit", "-q", "-m", &format!("write {file}")]);
        self.rev_parse(repo, "HEAD").unwrap()
    }

    fn rev_parse(&self, repo: &Path, revision: &str) -> Option<ObjectId> {
        let output = git_command(&self.home())
            .args(["rev-parse", "--verify", "--quiet", revision])
            .current_dir(repo)
            .output()
            .unwrap();
        output.status.success().then(|| {
            ObjectId::from_hex(String::from_utf8_lossy(&output.stdout).trim().as_bytes()).unwrap()
        })
    }

    fn url(&self, repo: &Path) -> String {
        format!("ssh://git@127.0.0.1:{}{}", self.server.port, repo.display())
    }

    fn connect(&self, repo: &Path, protocol: Protocol) -> SshTransport {
        let url = gix_url::parse(BStr::new(&self.url(repo))).unwrap();
        ssh::connect(&url, protocol, &self.options).unwrap()
    }
}

/// Fetches every branch of the remote at `url` into `refs/remotes/origin/`.
fn fetch(repo: &Path, url: &str, transport: impl Transport) -> gix::remote::fetch::Outcome {
    let repo = gix::open(repo).unwrap();
    let url = gix_url::parse(BStr::new(url)).unwrap();
    let remote = repo
        .remote_at(url)
        .unwrap()
        .with_refspecs(
            ["+refs/heads/*:refs/remotes/origin/*"],
            gix::remote::Direction::Fetch,
        )
        .unwrap();
    remote
        .to_connection_with_transport(transport)
        .prepare_fetch(gix::progress::Discard, Default::default())
        .unwrap()
        .receive(gix::progress::Discard, &AtomicBool::new(false))
        .unwrap()
}

fn push(transport: &mut SshTransport, repo: &Path, commands: &[RefCommand]) {
    let repo = gix::open(repo).unwrap();
    let advertisement = send_pack::discover(transport).unwrap();
    let outcome = send_pack::send(
        transport,
        &repo,
        &advertisement,
        commands,
        &[],
        &SendOptions::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(outcome.report.is_some());
}

fn null() -> ObjectId {
    ObjectId::null(gix::hash::Kind::Sha1)
}

#[test]
#[ignore = "needs the git program"]
fn fetch_over_protocol_v2_receives_every_branch() {
    let fixture = Fixture::new();
    let source = fixture.init("source", false);
    let first = fixture.commit(&source, "a.txt", "one\n");
    fixture.git(&source, &["branch", "topic"]);
    let second = fixture.commit(&source, "b.txt", "two\n");
    let client = fixture.init("client", false);

    let transport = fixture.connect(&source, Protocol::V2);
    fetch(&client, &fixture.url(&source), transport);

    assert_eq!(
        fixture.rev_parse(&client, "refs/remotes/origin/main"),
        Some(second)
    );
    assert_eq!(
        fixture.rev_parse(&client, "refs/remotes/origin/topic"),
        Some(first)
    );
    fixture.git(&client, &["fsck", "--strict"]);
    let log = fixture.server.log();
    assert_eq!(log.users, ["git"]);
    assert_eq!(
        log.commands,
        [format!("git-upload-pack '{}'", source.display())]
    );
}

#[test]
#[ignore = "needs the git program"]
fn fetch_with_protocol_version_0_reads_the_reference_advertisement() {
    let fixture = Fixture::new();
    let source = fixture.init("source", false);
    let head = fixture.commit(&source, "a.txt", "one\n");
    let client = fixture.init("client", false);

    let mut transport = fixture.connect(&source, Protocol::V0);
    fetch(&client, &fixture.url(&source), &mut transport);

    assert_eq!(
        fixture.rev_parse(&client, "refs/remotes/origin/main"),
        Some(head)
    );
    assert!(transport.diagnostics().error().is_none());
}

#[test]
#[ignore = "needs the git program"]
fn one_session_serves_consecutive_fetches() {
    let fixture = Fixture::new();
    let source = fixture.init("source", false);
    fixture.commit(&source, "a.txt", "one\n");
    let client = fixture.init("client", false);
    let url = fixture.url(&source);

    let mut transport = fixture.connect(&source, Protocol::V2);
    fetch(&client, &url, &mut transport);
    let second = fixture.commit(&source, "a.txt", "two\n");
    fetch(&client, &url, &mut transport);

    assert_eq!(
        fixture.rev_parse(&client, "refs/remotes/origin/main"),
        Some(second)
    );
    let log = fixture.server.log();
    assert_eq!(log.users.len(), 1);
    assert_eq!(log.commands.len(), 2);
}

#[test]
#[ignore = "needs the git program"]
fn push_creates_updates_and_deletes_branches() {
    let fixture = Fixture::new();
    let remote = fixture.init("remote.git", true);
    let local = fixture.init("local", false);
    let first = fixture.commit(&local, "a.txt", "one\n");
    let mut transport = fixture.connect(&remote, Protocol::V2);

    push(
        &mut transport,
        &local,
        &[
            RefCommand {
                old: null(),
                new: first,
                name: "refs/heads/main".into(),
            },
            RefCommand {
                old: null(),
                new: first,
                name: "refs/heads/topic".into(),
            },
        ],
    );
    assert_eq!(fixture.rev_parse(&remote, "refs/heads/main"), Some(first));

    let second = fixture.commit(&local, "a.txt", "two\n");
    push(
        &mut transport,
        &local,
        &[
            RefCommand {
                old: first,
                new: second,
                name: "refs/heads/main".into(),
            },
            RefCommand {
                old: first,
                new: null(),
                name: "refs/heads/topic".into(),
            },
        ],
    );

    assert_eq!(fixture.rev_parse(&remote, "refs/heads/main"), Some(second));
    assert_eq!(fixture.rev_parse(&remote, "refs/heads/topic"), None);
    fixture.git(&remote, &["fsck", "--strict"]);
    let log = fixture.server.log();
    assert_eq!(
        log.commands,
        [
            format!("git-receive-pack '{}'", remote.display()),
            format!("git-receive-pack '{}'", remote.display())
        ]
    );
}

#[test]
#[ignore = "needs the git program"]
fn missing_repository_reports_the_message_of_git() {
    let fixture = Fixture::new();
    let mut transport = fixture.connect(&fixture.path("missing.git"), Protocol::V2);
    let diagnostics = transport.diagnostics();

    assert!(
        transport
            .handshake(gix_transport::Service::UploadPack, &[])
            .is_err()
    );

    let error = diagnostics.error().unwrap();
    assert_eq!(error.code(), ErrorCode::GitProtocol);
    assert_eq!(
        error.data().problem.as_deref(),
        Some("repository_not_found")
    );
    assert!(
        diagnostics
            .stderr()
            .contains("does not appear to be a git repository")
    );
}

#[test]
#[ignore = "needs the git program"]
fn configured_alias_in_scp_form_goes_through_the_transport_dispatcher() {
    let fixture = Fixture::new();
    let source = fixture.init("source", false);
    let head = fixture.commit(&source, "a.txt", "one\n");
    let client = fixture.init("client", false);
    std::fs::write(
        fixture.home().join(".ssh").join("config"),
        format!(
            "Host repo-host\n  HostName 127.0.0.1\n  Port {}\n  User deploy\n",
            fixture.server.port
        ),
    )
    .unwrap();
    let url = format!("repo-host:{}", source.display());
    let options = TransportOptions::new(HttpSettings::default()).with_ssh(fixture.options.clone());

    let transport = connect::connect(&url, Direction::Fetch, &options).unwrap();
    fetch(&client, &url, transport);

    assert_eq!(
        fixture.rev_parse(&client, "refs/remotes/origin/main"),
        Some(head)
    );
    assert_eq!(fixture.server.log().users, ["deploy"]);
}
