// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-difftest::ssh
// Responsibility:   Test server of the SSH transport.
// Dependencies:     russh
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Test server of the SSH transport.
//!
//! The server runs `russh` on its own thread, listens on the loopback
//! interface, accepts one client key and answers `exec` requests for
//! `git-upload-pack`, `git-receive-pack` and `git-upload-archive` by running
//! the `git` program on a repository below the case directory; the path of
//! the request is relative to that directory. A `GIT_PROTOCOL` variable sent
//! with an `env` request reaches the program.
//!
//! The `git` program needs an `ssh` client for such addresses. The `git`
//! side of a scenario uses [`local_ssh_command`] instead: a shell command
//! that runs the requested program on the same repository directly, so both
//! sides reach the same repositories under the same addresses, and only the
//! Nurt side passes through the SSH protocol.

use std::collections::HashMap;
use std::ffi::OsString;
use std::net::{SocketAddr, TcpListener};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId, MethodKind, MethodSet};

use crate::error::{DifftestError, Result, io_error};
use crate::git::{GitEnv, GitProgram};

/// Address the server listens on; the port is chosen by the system.
const LOOPBACK: &str = "127.0.0.1:0";

/// User name in the addresses of the server.
pub const SSH_USER: &str = "nurt";

/// Seed of the host key of the server.
const HOST_SEED: u8 = 0x48;

/// Seed of the key of the client.
const CLIENT_SEED: u8 = 0x43;

/// Name of the private key file of the client in `<home>/.ssh`.
const IDENTITY_FILE: &str = "id_ed25519";

/// Time after which a silent connection is closed.
const INACTIVITY: Duration = Duration::from_secs(60);

/// Extended data type of standard error (RFC 4254, section 5.2).
const STDERR_DATA: u32 = 1;

/// Exit status reported when the program cannot be started.
const EXIT_NOT_STARTED: u32 = 127;

/// Exit status reported when the program ended by a signal.
const EXIT_SIGNALED: u32 = 255;

/// Programs that an `exec` request may run.
const SERVICES: &[&str] = &["git-upload-pack", "git-receive-pack", "git-upload-archive"];

/// SSH server for the repositories below a directory.
///
/// The server stops when the value is dropped.
#[derive(Debug)]
pub struct SshServer {
    address: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl SshServer {
    /// Starts a server for the repositories below `root` and writes the
    /// client files to `<home>/.ssh`: the private key `id_ed25519`, its
    /// public key and a `known_hosts` line for the host key of the server.
    ///
    /// The repository `<root>/<path>` has the address `<base_url>/<path>`;
    /// `env` is the environment of the programs run by the server, so its
    /// identity and dates apply to reflog entries written by pushes.
    ///
    /// # Errors
    ///
    /// Fails with [`DifftestError::Io`] when the listening socket or the
    /// client files cannot be created.
    pub fn start(git: &GitProgram, env: &GitEnv, root: &Path, home: &Path) -> Result<Self> {
        let listen = Path::new(LOOPBACK);
        let listener = TcpListener::bind(LOOPBACK).map_err(io_error("listen on", listen))?;
        listener
            .set_nonblocking(true)
            .map_err(io_error("listen on", listen))?;
        let address = listener
            .local_addr()
            .map_err(io_error("listen on", listen))?;
        let host_key = key(HOST_SEED);
        write_client_files(home, address, host_key.public_key())?;
        let settings = Arc::new(Settings {
            program: git.program().to_path_buf(),
            env: env
                .vars()
                .iter()
                .map(|(name, value)| (OsString::from(name), value.clone()))
                .collect(),
            root: root.to_path_buf(),
            client: key(CLIENT_SEED).public_key().clone(),
        });
        let config = Arc::new(server::Config {
            keys: vec![host_key],
            methods: MethodSet::from(&[MethodKind::PublicKey][..]),
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            inactivity_timeout: Some(INACTIVITY),
            ..server::Config::default()
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(io_error("start the server runtime for", listen))?;
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let thread = std::thread::spawn(move || {
            runtime.block_on(serve(listener, config, settings, stopped));
        });
        Ok(Self {
            address,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    /// Address of the root directory, `ssh://nurt@127.0.0.1:<port>`, without
    /// a trailing slash.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("ssh://{SSH_USER}@{}", self.address)
    }
}

impl Drop for SshServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Deterministic Ed25519 key.
fn key(seed: u8) -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
}

fn key_error(path: &Path, error: impl std::fmt::Display) -> DifftestError {
    DifftestError::Io {
        action: "encode a key for",
        path: path.to_path_buf(),
        source: std::io::Error::other(error.to_string()),
    }
}

/// Writes the identity of the client and the `known_hosts` entry of the
/// server below `<home>/.ssh`.
fn write_client_files(home: &Path, address: SocketAddr, host: &PublicKey) -> Result<()> {
    let dir = home.join(".ssh");
    std::fs::create_dir_all(&dir).map_err(io_error("create directory", &dir))?;
    let client = key(CLIENT_SEED);
    let identity = dir.join(IDENTITY_FILE);
    let private = client
        .to_openssh(LineEnding::LF)
        .map_err(|error| key_error(&identity, error))?;
    std::fs::write(&identity, private.as_bytes()).map_err(io_error("write", &identity))?;
    restrict_to_owner(&identity)?;
    let public_path = dir.join(format!("{IDENTITY_FILE}.pub"));
    let public = client
        .public_key()
        .to_openssh()
        .map_err(|error| key_error(&public_path, error))?;
    std::fs::write(&public_path, format!("{public}\n")).map_err(io_error("write", &public_path))?;
    let known_hosts = dir.join("known_hosts");
    let host_line = host
        .to_openssh()
        .map_err(|error| key_error(&known_hosts, error))?;
    let line = format!("[{}]:{} {host_line}\n", address.ip(), address.port());
    std::fs::write(&known_hosts, line).map_err(io_error("write", &known_hosts))
}

/// SSH clients refuse private keys that other users can read.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    const OWNER_READ_WRITE: u32 = 0o600;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(OWNER_READ_WRITE))
        .map_err(io_error("set permissions of", path))
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> Result<()> {
    Ok(())
}

/// Fixed data of every connection.
#[derive(Debug)]
struct Settings {
    program: PathBuf,
    env: Vec<(OsString, OsString)>,
    root: PathBuf,
    client: PublicKey,
}

async fn serve(
    listener: TcpListener,
    config: Arc<server::Config>,
    settings: Arc<Settings>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) {
    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
        return;
    };
    loop {
        tokio::select! {
            _ = &mut stop => return,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else {
                    return;
                };
                let handler = Connection {
                    settings: Arc::clone(&settings),
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

/// State of one client connection.
struct Connection {
    settings: Arc<Settings>,
    protocol: HashMap<ChannelId, String>,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl server::Handler for Connection {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        key: server::VerifiedPublicKey<'_>,
    ) -> std::result::Result<Auth, Self::Error> {
        if key.public_key.key_data() == self.settings.client.key_data() {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> std::result::Result<(), Self::Error> {
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
    ) -> std::result::Result<(), Self::Error> {
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
    ) -> std::result::Result<(), Self::Error> {
        let request = ExecRequest::parse(&String::from_utf8_lossy(data), &self.settings.root);
        let (Some(request), Some(open)) = (request, self.channels.remove(&channel)) else {
            session.channel_failure(channel)?;
            session.close(channel)?;
            return Ok(());
        };
        session.channel_success(channel)?;
        let mut env = self.settings.env.clone();
        if let Some(protocol) = self.protocol.remove(&channel) {
            env.push(("GIT_PROTOCOL".into(), protocol.into()));
        }
        let program = self.settings.program.clone();
        tokio::spawn(run_exec(open, program, env, request));
        Ok(())
    }
}

/// A validated `exec` request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecRequest {
    /// Subcommand of `git`, for example `upload-pack`.
    subcommand: String,
    /// Repository directory.
    repository: PathBuf,
}

impl ExecRequest {
    /// Reads `git-<service> '<path>'`; the path is relative to `root` and
    /// may not leave it.
    fn parse(command: &str, root: &Path) -> Option<Self> {
        let (service, argument) = command.trim().split_once(' ')?;
        if !SERVICES.contains(&service) {
            return None;
        }
        let path = unquote_shell_word(argument.trim())?;
        let relative = Path::new(path.trim_start_matches('/'));
        let plain = relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        if !plain || relative.as_os_str().is_empty() {
            return None;
        }
        Some(Self {
            subcommand: service.trim_start_matches("git-").to_owned(),
            repository: root.join(relative),
        })
    }
}

/// Value of one shell word written plainly or in single quotes, where `'`
/// is written `'\''`, the quoting used by the `git` program.
fn unquote_shell_word(word: &str) -> Option<String> {
    if !word.starts_with('\'') {
        return (!word.contains(['\'', '"', '\\', ' '])).then(|| word.to_owned());
    }
    let mut out = String::new();
    let mut rest = word;
    while !rest.is_empty() {
        let inner = rest.strip_prefix('\'')?;
        let end = inner.find('\'')?;
        out.push_str(&inner[..end]);
        rest = &inner[end + 1..];
        if let Some(after) = rest.strip_prefix("\\'") {
            out.push('\'');
            rest = after;
        }
    }
    Some(out)
}

/// Runs the program of a request with the channel as its standard streams.
async fn run_exec(
    channel: Channel<Msg>,
    program: PathBuf,
    env: Vec<(OsString, OsString)>,
    request: ExecRequest,
) {
    let (mut read, write) = channel.split();
    let spawned = tokio::process::Command::new(&program)
        .arg(&request.subcommand)
        .arg(&request.repository)
        .env_clear()
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let status = match spawned {
        Ok(mut child) => {
            let stdin = child.stdin.take();
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let input = async {
                if let Some(mut stdin) = stdin {
                    let mut reader = read.make_reader();
                    let _ = tokio::io::copy(&mut reader, &mut stdin).await;
                }
            };
            let output = async {
                let out = async {
                    if let Some(stdout) = stdout {
                        let _ = write.data(stdout).await;
                    }
                };
                let err = async {
                    if let Some(stderr) = stderr {
                        let _ = write.extended_data(STDERR_DATA, stderr).await;
                    }
                };
                tokio::join!(out, err);
                child.wait().await
            };
            tokio::pin!(input, output);
            let mut input_done = false;
            // The program may end before the client closes its side, so the
            // copy of the input does not decide the end of the request.
            let finished = loop {
                tokio::select! {
                    finished = &mut output => break finished,
                    () = &mut input, if !input_done => input_done = true,
                }
            };
            match finished {
                Ok(status) => status
                    .code()
                    .and_then(|code| u32::try_from(code).ok())
                    .unwrap_or(EXIT_SIGNALED),
                Err(_) => EXIT_SIGNALED,
            }
        }
        Err(_) => EXIT_NOT_STARTED,
    };
    let _ = write.exit_status(status).await;
    let _ = write.eof().await;
    let _ = write.close().await;
}

/// Value of `GIT_SSH_COMMAND` for the `git` side: a POSIX shell command that
/// runs the program named by the last argument on the repository below
/// `root`, without a connection. Options and the host before it are ignored.
#[must_use]
pub fn local_ssh_command(root: &Path) -> String {
    let root = root.to_string_lossy().replace('\\', "/");
    format!(
        "nurt_ssh() {{ for request; do :; done; cd {} && eval \"set -- $request\" && exec git \"${{1#git-}}\" \".$2\"; }}; nurt_ssh",
        shell_quote(&root)
    )
}

/// `text` as one single-quoted shell word.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_and_plain_requests_below_the_root() {
        let root = Path::new("/case");
        let request = ExecRequest::parse("git-upload-pack '/b/remote.git'", root).unwrap();
        assert_eq!(request.subcommand, "upload-pack");
        assert_eq!(request.repository, Path::new("/case/b/remote.git"));
        let plain = ExecRequest::parse("git-receive-pack b/up.git", root).unwrap();
        assert_eq!(plain.subcommand, "receive-pack");
        assert_eq!(plain.repository, Path::new("/case/b/up.git"));
        let quoted = ExecRequest::parse("git-upload-pack '/b/it'\\''s.git'", root).unwrap();
        assert_eq!(quoted.repository, Path::new("/case/b/it's.git"));
    }

    #[test]
    fn rejects_other_programs_and_paths_outside_the_root() {
        let root = Path::new("/case");
        for command in [
            "rm -rf '/b'",
            "git-upload-pack '/b/../../etc'",
            "git-upload-pack '/'",
            "git-upload-pack",
            "git-upload-pack '/b",
            "git-upload-pack /b/a b",
        ] {
            assert!(ExecRequest::parse(command, root).is_none(), "{command}");
        }
    }

    #[test]
    fn local_command_quotes_the_root() {
        let command = local_ssh_command(Path::new("/tmp/it's"));
        assert!(command.contains("cd '/tmp/it'\\''s' &&"), "{command}");
        assert!(command.ends_with("; nurt_ssh"));
    }

    #[test]
    fn keys_are_deterministic() {
        assert_eq!(key(HOST_SEED).public_key(), key(HOST_SEED).public_key());
        assert_ne!(key(HOST_SEED).public_key(), key(CLIENT_SEED).public_key());
    }

    #[test]
    fn client_files_hold_the_identity_and_the_host_key() {
        let dir = tempfile::tempdir().unwrap();
        let address: SocketAddr = "127.0.0.1:2222".parse().unwrap();
        write_client_files(dir.path(), address, key(HOST_SEED).public_key()).unwrap();
        let ssh = dir.path().join(".ssh");
        let identity = std::fs::read_to_string(ssh.join(IDENTITY_FILE)).unwrap();
        assert!(identity.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
        let parsed = PrivateKey::from_openssh(&identity).unwrap();
        assert_eq!(parsed.public_key(), key(CLIENT_SEED).public_key());
        let known = std::fs::read_to_string(ssh.join("known_hosts")).unwrap();
        assert!(
            known.starts_with("[127.0.0.1]:2222 ssh-ed25519 "),
            "{known}"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
