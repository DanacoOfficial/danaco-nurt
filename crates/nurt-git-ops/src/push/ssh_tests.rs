// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Pushes over SSH to a loopback `russh` server that runs `git receive-pack`.
//!
//! The tests need the `git` program and are ignored by default:
//! `cargo test -p nurt-git-ops push::ssh_tests -- --ignored`.

// The server starts the reference `git` program directly: the tests compare
// the result of a push with the repository that program maintains.
#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_git_transport::send_pack::testing::{Fixture, RecordingTx};
use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, ChannelMsg};
use tokio::sync::mpsc;

use super::*;

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
/// Time after which an idle connection is closed.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(60);
/// Account name expected by the server.
const SSH_USER: &str = "git";

fn key(seed: u8) -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn identity() -> Identity {
    Identity {
        name: "Pusher".into(),
        email: "pusher@example.com".into(),
        seconds: 1_700_000_500,
        offset: 0,
    }
}

/// `git` with an isolated home directory and without system configuration.
fn git_command(home: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_PROTOCOL");
    command
}

struct Handler {
    authorized: PublicKey,
    home: PathBuf,
    commands: Arc<Mutex<Vec<String>>>,
    protocol: HashMap<ChannelId, String>,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl Handler {
    /// Starts `git receive-pack` for an `exec` command in the form sent by
    /// Git clients.
    fn spawn_git(&self, command: &str, protocol: Option<&String>) -> Option<Child> {
        let (program, quoted) = command.split_once(' ')?;
        if program != "git-receive-pack" {
            return None;
        }
        let path = quoted.strip_prefix('\'')?.strip_suffix('\'')?;
        let mut git = git_command(&self.home);
        git.arg("receive-pack")
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
        if user != SSH_USER || self.authorized.key_data() != key.public_key.key_data() {
            return Ok(Auth::reject());
        }
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
        lock(&self.commands).push(command);
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
    commands: Arc<Mutex<Vec<String>>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl GitServer {
    fn start(home: &Path, authorized: PublicKey) -> Self {
        let host_key = key(HOST_SEED);
        let public = host_key.public_key().clone();
        let commands = Arc::new(Mutex::new(Vec::new()));
        let (port_tx, port_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let shared = Arc::clone(&commands);
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
            commands,
            stop: Some(stop_tx),
            thread: Some(thread),
        }
    }

    fn commands(&self) -> Vec<String> {
        lock(&self.commands).clone()
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
    commands: Arc<Mutex<Vec<String>>>,
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
                    commands: Arc::clone(&commands),
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

/// Home directory of the client, the running server and the client settings
/// trusting it.
struct Environment {
    _home: tempfile::TempDir,
    server: GitServer,
    network: NetworkOptions,
}

impl Environment {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let ssh_dir = home.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let user_key = key(USER_SEED);
        std::fs::write(
            ssh_dir.join("id_ed25519"),
            user_key.to_openssh(LineEnding::LF).unwrap().as_bytes(),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                ssh_dir.join("id_ed25519"),
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        let server = GitServer::start(home.path(), user_key.public_key().clone());
        std::fs::write(
            ssh_dir.join("known_hosts"),
            format!(
                "[127.0.0.1]:{} {}\n",
                server.port,
                server.host_key.to_openssh().unwrap()
            ),
        )
        .unwrap();
        let mut ssh = SshOptions::new(home.path(), SSH_USER);
        ssh.system_dir = Some(home.path().join("etc-ssh"));
        ssh.nurt_known_hosts = Some(home.path().join("nurt").join("known_hosts"));
        ssh.timeout = Some(OPERATION_TIMEOUT);
        Self {
            _home: home,
            server,
            network: NetworkOptions {
                env: BTreeMap::new(),
                ssh: Some(ssh),
                git_env: None,
            },
        }
    }

    fn url(&self, remote: &Fixture) -> String {
        format!(
            "ssh://{SSH_USER}@127.0.0.1:{}{}",
            self.server.port,
            remote.git_dir().display()
        )
    }
}

fn linked(url: &str) -> Fixture {
    let local = Fixture::new();
    local.config(&format!(
        "[remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n"
    ));
    local
}

fn push(
    local: &Fixture,
    request: &PushRequest,
    network: &NetworkOptions,
) -> (Result<PushOutcome>, RecordingTx) {
    let cancel = CancelToken::new();
    let context = PushContext {
        identity: identity(),
        host: None,
        cancel: &cancel,
    };
    let mut tx = RecordingTx::new();
    let outcome = plan_push(local.root(), request, network)
        .and_then(|planned| planned.apply(&mut tx, &context));
    (outcome, tx)
}

#[test]
#[ignore = "needs the git program"]
fn pushes_a_new_branch_over_ssh_and_updates_the_tracking_ref() {
    let remote = Fixture::bare();
    let environment = Environment::new();
    let local = linked(&environment.url(&remote));
    let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
    let (outcome, tx) = push(&local, &PushRequest::new("main"), &environment.network);
    let outcome = outcome.unwrap();
    assert_eq!(outcome.updates[0].status, GitRefStatus::New);
    assert_eq!(remote.ref_value("refs/heads/main"), Some(tip));
    assert_eq!(local.ref_value("refs/remotes/origin/main"), Some(tip));
    assert_eq!(tx.irreversible.len(), 1);
    let commands = environment.server.commands();
    assert!(!commands.is_empty());
    assert!(
        commands
            .iter()
            .all(|command| command.starts_with("git-receive-pack '"))
    );
}

#[test]
#[ignore = "needs the git program"]
fn records_a_host_key_accepted_as_new_in_the_transaction() {
    let remote = Fixture::bare();
    let mut environment = Environment::new();
    let home = environment._home.path().to_path_buf();
    std::fs::remove_file(home.join(".ssh").join("known_hosts")).unwrap();
    environment.network.ssh.as_mut().unwrap().known_hosts =
        nurt_contracts::config::KnownHosts::AcceptNew;
    let local = linked(&environment.url(&remote));
    local.commit("main", &[("a.txt", "one\n")], &[]);
    let (outcome, _) = push(&local, &PushRequest::new("main"), &environment.network);
    outcome.unwrap();
    let recorded = std::fs::read_to_string(home.join("nurt").join("known_hosts")).unwrap();
    assert!(recorded.starts_with(&format!("[127.0.0.1]:{} ", environment.server.port)));
}

#[test]
#[ignore = "needs the git program"]
fn fast_forwards_over_ssh_and_rejects_a_diverged_branch() {
    let remote = Fixture::bare();
    let environment = Environment::new();
    let local = linked(&environment.url(&remote));
    let first = local.commit("main", &[("a.txt", "one\n")], &[]);
    push(&local, &PushRequest::new("main"), &environment.network)
        .0
        .unwrap();
    let second = local.commit("main", &[("a.txt", "two\n")], &[first]);
    let (outcome, _) = push(&local, &PushRequest::new("main"), &environment.network);
    assert_eq!(outcome.unwrap().updates[0].status, GitRefStatus::Updated);
    assert_eq!(remote.ref_value("refs/heads/main"), Some(second));

    let diverged = local.commit("main", &[("a.txt", "three\n")], &[first]);
    let (outcome, _) = push(&local, &PushRequest::new("main"), &environment.network);
    let outcome = outcome.unwrap();
    assert_eq!(outcome.updates[0].status, GitRefStatus::Rejected);
    assert_eq!(
        outcome.check().unwrap_err().code(),
        ErrorCode::GitPushRejected
    );
    assert_eq!(remote.ref_value("refs/heads/main"), Some(second));

    let forced = PushRequest {
        force: ForceMode::Yes,
        ..PushRequest::new("main")
    };
    let (outcome, _) = push(&local, &forced, &environment.network);
    assert_eq!(outcome.unwrap().updates[0].status, GitRefStatus::Forced);
    assert_eq!(remote.ref_value("refs/heads/main"), Some(diverged));
}

#[test]
#[ignore = "needs the git program"]
fn deletes_a_branch_over_ssh() {
    let remote = Fixture::bare();
    let environment = Environment::new();
    let local = linked(&environment.url(&remote));
    local.commit("topic", &[("a.txt", "one\n")], &[]);
    push(&local, &PushRequest::new("topic"), &environment.network)
        .0
        .unwrap();
    let delete = PushRequest {
        delete: true,
        ..PushRequest::new("topic")
    };
    let (outcome, _) = push(&local, &delete, &environment.network);
    assert_eq!(outcome.unwrap().updates[0].status, GitRefStatus::Deleted);
    assert_eq!(remote.ref_value("refs/heads/topic"), None);
    assert_eq!(local.ref_value("refs/remotes/origin/topic"), None);
}

#[test]
#[ignore = "needs the git program"]
fn refuses_an_unknown_ssh_account_without_changing_the_remote() {
    let remote = Fixture::bare();
    let environment = Environment::new();
    let url = environment.url(&remote).replacen(SSH_USER, "intruder", 1);
    let local = linked(&url);
    local.commit("main", &[("a.txt", "one\n")], &[]);
    let (outcome, tx) = push(&local, &PushRequest::new("main"), &environment.network);
    assert_eq!(outcome.unwrap_err().code(), ErrorCode::GitAuth);
    assert_eq!(remote.ref_value("refs/heads/main"), None);
    assert!(tx.irreversible.is_empty());
}
