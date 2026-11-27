// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport::ssh
// Responsibility:   SSH transport.
// Dependencies:     known_hosts, nurt_contracts, nurt_testkit, transport
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! SSH transport.
//!
//! [`connect`] opens an SSH session with the built-in client and returns an
//! [`SshTransport`] for `gix`. No external program is started: settings
//! that name a program (`core.sshCommand`, `GIT_SSH_COMMAND`, `GIT_SSH`,
//! `ProxyCommand`, `Match exec`) are not run and are reported.
//!
//! Host keys are checked against the known hosts files of OpenSSH and a
//! known hosts file of Nurt. A changed key is always refused. An unknown key
//! is refused unless `git.ssh.known_hosts` is `accept_new`, the SSH
//! configuration sets `StrictHostKeyChecking accept-new` or `no`, or its
//! fingerprint was accepted explicitly. Accepted keys are recorded only in
//! the file of Nurt, through the journal, by [`record_host_key`].

mod channel;
mod config;
mod identity;
mod known_hosts;
mod pattern;
mod session;
mod target;
mod transport;

#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::config::{GitConfig, KnownHosts};
use nurt_contracts::env::Os;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{CancelToken, ExecCtx};

pub use known_hosts::{
    HostKeyAddition, HostKeyReservation, host_keys_plan, record_host_key, reserve_host_key_file,
};
pub use transport::{RemoteDiagnostics, SshTransport, connect};

/// Time limit of the TCP connection and key exchange of one hop when
/// `ConnectTimeout` is not configured.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Name of the variable that names the passphrase file of encrypted keys.
pub const PASSPHRASE_FILE_VAR: &str = "NURT_SSH_KEY_PASSPHRASE_FILE";

/// Name of the known hosts file of Nurt in its configuration directory.
const KNOWN_HOSTS_FILE: &str = "known_hosts";

/// Settings of an SSH connection that do not come from the SSH
/// configuration files.
///
/// [`SshOptions::from_ctx`] reads them from the execution context;
/// [`SshOptions::new`] gives the defaults for a home directory.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SshOptions {
    /// Home directory of the local account.
    pub home: PathBuf,
    /// Name of the local account, the default remote account.
    pub local_user: String,
    /// Name of the local host, for `%l` expansions.
    pub local_host: String,
    /// SSH client configuration of the user; `None` reads no user file.
    pub user_config: Option<PathBuf>,
    /// System SSH directory with `ssh_config` and `ssh_known_hosts`.
    pub system_dir: Option<PathBuf>,
    /// Socket or named pipe of the agent (`SSH_AUTH_SOCK`).
    pub auth_sock: Option<PathBuf>,
    /// File holding the passphrase of encrypted keys.
    pub passphrase_file: Option<PathBuf>,
    /// Key files added to those of the SSH configuration.
    pub identity_files: Vec<PathBuf>,
    /// Handling of unknown hosts (`git.ssh.known_hosts`).
    pub known_hosts: KnownHosts,
    /// Known hosts file of Nurt, read with the OpenSSH files and receiving
    /// accepted keys.
    pub nurt_known_hosts: Option<PathBuf>,
    /// Fingerprint (`SHA256:…`) accepted for an unknown host.
    pub accept_host_key: Option<String>,
    /// Settings naming an SSH program that is not run, for example
    /// `core.sshCommand`.
    pub ignored_ssh_command: Vec<String>,
    /// Time limit of the TCP connection and key exchange of one hop.
    pub connect_timeout: Duration,
    /// Time limit of the whole operation.
    pub timeout: Option<Duration>,
    /// Cancellation of the operation.
    pub cancel: CancelToken,
    /// Call to repeat with an accepted fingerprint, used in the fix of
    /// unknown host errors, for example `git.fetch "/home/user/repo"`.
    pub retry_command: Option<String>,
    /// Keys accepted for new hosts by the connections made with these
    /// options; the caller records them in its transaction.
    pub host_keys: HostKeyLog,
}

/// Keys accepted for new hosts, shared by the clones of one [`SshOptions`].
///
/// A transport is boxed before the transfer, so the keys it accepts reach the
/// command that owns the transaction through this handle.
#[derive(Debug, Clone, Default)]
pub struct HostKeyLog(std::sync::Arc<std::sync::Mutex<Vec<HostKeyAddition>>>);

impl HostKeyLog {
    pub(crate) fn push_all(&self, additions: &[HostKeyAddition]) {
        let mut keys = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for addition in additions {
            if !keys.contains(addition) {
                keys.push(addition.clone());
            }
        }
    }

    /// Removes and returns the keys accepted so far.
    #[must_use]
    pub fn take(&self) -> Vec<HostKeyAddition> {
        std::mem::take(
            &mut *self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

impl SshOptions {
    /// Defaults for a local account, with the file locations of the host
    /// operating system.
    #[must_use]
    pub fn new(home: impl Into<PathBuf>, local_user: impl Into<String>) -> Self {
        let os = if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        };
        Self::for_host(home.into(), local_user.into(), String::new(), os, &|_| None)
    }

    /// Settings of the execution context: host facts, the variables
    /// `SSH_AUTH_SOCK`, `NURT_SSH_KEY_PASSPHRASE_FILE`, `GIT_SSH_COMMAND`,
    /// `GIT_SSH`, `XDG_CONFIG_HOME`, `APPDATA` and `PROGRAMDATA`, the keys
    /// `git.ssh.*` and `git.network.timeout`, the deadline and the
    /// cancellation of the context.
    #[must_use]
    pub fn from_ctx(ctx: &ExecCtx) -> Self {
        let info = ctx.host().info();
        let env = |name: &str| ctx.host().env_var(name);
        let mut options = Self::for_host(
            info.home.clone(),
            info.user.clone(),
            info.hostname.clone(),
            info.os,
            &env,
        );
        options.apply_git_config(&ctx.config().git);
        options.timeout = match (options.timeout, ctx.remaining()) {
            (Some(configured), Some(remaining)) => Some(configured.min(remaining)),
            (configured, remaining) => configured.or(remaining),
        };
        options.cancel = ctx.cancel().clone();
        options
    }

    fn for_host(
        home: PathBuf,
        local_user: String,
        local_host: String,
        os: Os,
        env: &dyn Fn(&str) -> Option<OsString>,
    ) -> Self {
        let path = |name: &str| {
            env(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let ignored_ssh_command = ["GIT_SSH_COMMAND", "GIT_SSH"]
            .into_iter()
            .filter(|name| path(name).is_some())
            .map(str::to_owned)
            .collect();
        Self {
            user_config: Some(home.join(".ssh").join("config")),
            system_dir: Some(system_dir(os, path("PROGRAMDATA"))),
            auth_sock: path("SSH_AUTH_SOCK"),
            passphrase_file: path(PASSPHRASE_FILE_VAR),
            identity_files: Vec::new(),
            known_hosts: KnownHosts::Strict,
            nurt_known_hosts: Some(product_config_dir(os, &home, &path).join(KNOWN_HOSTS_FILE)),
            accept_host_key: None,
            ignored_ssh_command,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            timeout: None,
            cancel: CancelToken::new(),
            retry_command: None,
            host_keys: HostKeyLog::default(),
            home,
            local_user,
            local_host,
        }
    }

    fn apply_git_config(&mut self, git: &GitConfig) {
        self.known_hosts = git.ssh.known_hosts;
        self.identity_files = git
            .ssh
            .identity_files
            .iter()
            .map(|file| expand_home(file, &self.home))
            .collect();
        self.timeout = Some(git.network.timeout.as_std());
    }
}

fn expand_home(file: &str, home: &Path) -> PathBuf {
    match file.strip_prefix("~/").or_else(|| file.strip_prefix("~\\")) {
        Some(rest) => home.join(rest),
        None => PathBuf::from(file),
    }
}

fn system_dir(os: Os, program_data: Option<PathBuf>) -> PathBuf {
    match os {
        Os::Windows => program_data
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
            .join("ssh"),
        _ => PathBuf::from("/etc/ssh"),
    }
}

/// Configuration directory of Nurt, the directory of the user file
/// `nurt.toml`.
fn product_config_dir(os: Os, home: &Path, path: &dyn Fn(&str) -> Option<PathBuf>) -> PathBuf {
    match os {
        Os::Windows => path("APPDATA")
            .unwrap_or_else(|| home.join("AppData").join("Roaming"))
            .join("Nurt"),
        Os::Macos => home
            .join("Library")
            .join("Application Support")
            .join("Nurt"),
        _ => path("XDG_CONFIG_HOME")
            .filter(|dir| dir.is_absolute())
            .unwrap_or_else(|| home.join(".config"))
            .join("nurt"),
    }
}

/// An unusual condition met while connecting.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SshNotice {
    /// A setting names an SSH program that was not run; the built-in client
    /// was used.
    SshCommandIgnored {
        /// Name of the setting, for example `core.sshCommand`.
        source: String,
    },
    /// A `Match exec` criterion was not evaluated, since it runs a program;
    /// the block was treated as not matching.
    MatchExecIgnored {
        /// Configuration file.
        file: PathBuf,
        /// Line number.
        line: usize,
    },
    /// An unknown host key was accepted without verification because the
    /// SSH configuration sets `StrictHostKeyChecking no`.
    HostKeyUnverified {
        /// Host as requested.
        host: String,
        /// Fingerprint of the key.
        fingerprint: String,
    },
}

impl SshNotice {
    /// Entry of the effect report.
    #[must_use]
    pub fn notable(&self) -> NotableEntry {
        let (code, path, detail) = match self {
            Self::SshCommandIgnored { source } => (
                Notable::SshCommandIgnored,
                None,
                format!("{source} is not run; the built-in SSH client is used"),
            ),
            Self::MatchExecIgnored { file, line } => (
                Notable::MatchExecIgnored,
                Some(file.display().to_string()),
                format!(
                    "line {line}: the program of `Match exec` is not run; the block does not apply"
                ),
            ),
            Self::HostKeyUnverified { host, fingerprint } => (
                Notable::HostKeyUnverified,
                None,
                format!(
                    "host key {fingerprint} of {host} accepted without verification by StrictHostKeyChecking no"
                ),
            ),
        };
        let mut entry = NotableEntry::new(code);
        entry.path = path;
        entry.detail = Some(detail);
        entry
    }
}

#[cfg(test)]
mod options_tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn linux_defaults_follow_xdg_and_etc() {
        let sock = nurt_testkit::paths::abs("/run/user/1000/agent.sock");
        let config_home = nurt_testkit::paths::abs("/home/user/.cfg");
        let env = env_of(&[
            ("SSH_AUTH_SOCK", sock.as_str()),
            ("XDG_CONFIG_HOME", config_home.as_str()),
            ("GIT_SSH_COMMAND", "ssh -i key"),
            ("GIT_SSH", ""),
        ]);
        let options = SshOptions::for_host(
            nurt_testkit::paths::abs("/home/user").as_str().into(),
            "user".into(),
            "box".into(),
            Os::Linux,
            &env,
        );
        assert_eq!(
            options.auth_sock,
            Some(nurt_testkit::paths::abs_path("/run/user/1000/agent.sock"))
        );
        assert_eq!(
            options.nurt_known_hosts,
            Some(nurt_testkit::paths::abs_path(
                "/home/user/.cfg/nurt/known_hosts"
            ))
        );
        assert_eq!(options.system_dir, Some(PathBuf::from("/etc/ssh")));
        assert_eq!(
            options.user_config,
            Some(nurt_testkit::paths::abs_path("/home/user/.ssh/config"))
        );
        assert_eq!(options.ignored_ssh_command, ["GIT_SSH_COMMAND"]);
        assert_eq!(options.connect_timeout, DEFAULT_CONNECT_TIMEOUT);
    }

    #[test]
    fn macos_and_windows_use_their_directories() {
        let options = SshOptions::for_host(
            "/Users/user".into(),
            "user".into(),
            String::new(),
            Os::Macos,
            &|_| None,
        );
        assert_eq!(
            options.nurt_known_hosts,
            Some(PathBuf::from(
                "/Users/user/Library/Application Support/Nurt/known_hosts"
            ))
        );
        let env = env_of(&[
            ("APPDATA", "C:/Users/user/AppData/Roaming"),
            ("PROGRAMDATA", "D:/Data"),
        ]);
        let options = SshOptions::for_host(
            "C:/Users/user".into(),
            "user".into(),
            String::new(),
            Os::Windows,
            &env,
        );
        assert_eq!(
            options.nurt_known_hosts,
            Some(PathBuf::from(
                "C:/Users/user/AppData/Roaming/Nurt/known_hosts"
            ))
        );
        assert_eq!(options.system_dir, Some(PathBuf::from("D:/Data/ssh")));
    }

    #[test]
    fn relative_xdg_directory_is_ignored() {
        let env = env_of(&[("XDG_CONFIG_HOME", "relative")]);
        let options = SshOptions::for_host(
            "/home/user".into(),
            "user".into(),
            String::new(),
            Os::Linux,
            &env,
        );
        assert_eq!(
            options.nurt_known_hosts,
            Some(PathBuf::from("/home/user/.config/nurt/known_hosts"))
        );
    }

    #[test]
    fn git_config_sets_policy_keys_and_timeout() {
        let mut options = SshOptions::new("/home/user", "user");
        let mut git = GitConfig::default();
        git.ssh.known_hosts = KnownHosts::AcceptNew;
        git.ssh.identity_files = vec!["~/.ssh/deploy".into(), "/keys/ci".into()];
        options.apply_git_config(&git);
        assert_eq!(options.known_hosts, KnownHosts::AcceptNew);
        assert_eq!(
            options.identity_files,
            [
                PathBuf::from("/home/user/.ssh/deploy"),
                PathBuf::from("/keys/ci")
            ]
        );
        assert_eq!(options.timeout, Some(git.network.timeout.as_std()));
    }

    #[test]
    fn every_notice_has_its_report_code() {
        let entry = SshNotice::SshCommandIgnored {
            source: "GIT_SSH".into(),
        }
        .notable();
        assert_eq!(entry.code, Notable::SshCommandIgnored);
        assert!(entry.detail.unwrap().contains("GIT_SSH"));

        let entry = SshNotice::MatchExecIgnored {
            file: PathBuf::from("/home/user/.ssh/config"),
            line: 4,
        }
        .notable();
        assert_eq!(entry.code, Notable::MatchExecIgnored);
        assert_eq!(entry.path.as_deref(), Some("/home/user/.ssh/config"));
        assert!(entry.detail.unwrap().starts_with("line 4:"));

        let entry = SshNotice::HostKeyUnverified {
            host: "git.example.com".into(),
            fingerprint: "SHA256:x".into(),
        }
        .notable();
        assert_eq!(entry.code, Notable::HostKeyUnverified);
        assert_eq!(
            entry.severity,
            Notable::HostKeyUnverified.default_severity()
        );
        assert!(entry.detail.unwrap().contains("SHA256:x"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
