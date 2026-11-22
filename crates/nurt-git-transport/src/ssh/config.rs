// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Subset of the OpenSSH client configuration (`ssh_config(5)`).
//!
//! Supported: `Host`, `Match` with the criteria `all`, `canonical`, `final`,
//! `host`, `originalhost`, `user` and `localuser`, `Include`, `HostName`,
//! `User`, `Port`, `IdentityFile`, `IdentitiesOnly`, `IdentityAgent`,
//! `UserKnownHostsFile`, `GlobalKnownHostsFile`, `StrictHostKeyChecking`,
//! `HostKeyAlias`, `ConnectTimeout`, `ServerAliveInterval`,
//! `ServerAliveCountMax`, `HostKeyAlgorithms`, `PubkeyAcceptedAlgorithms`,
//! `ProxyJump` and `ProxyCommand`. Other keywords are read and ignored.
//!
//! The first value obtained for a keyword is used, except for
//! `IdentityFile`, whose values accumulate. The user file is read before the
//! system file.

use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};

use super::pattern::{match_list, wildcard_match};

/// Default SSH port.
pub(crate) const DEFAULT_PORT: u16 = 22;

/// Nesting limit of `Include`, as in OpenSSH.
const MAX_INCLUDE_DEPTH: usize = 16;

/// Hops allowed in a `ProxyJump` chain.
const MAX_JUMPS: usize = 8;

/// Default private key files, in the order they are tried.
const DEFAULT_IDENTITIES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

/// Locations of the configuration and facts used by `%` expansions.
#[derive(Debug, Clone)]
pub(crate) struct ConfigSources {
    /// Home directory of the local account.
    pub(crate) home: PathBuf,
    /// Name of the local account.
    pub(crate) local_user: String,
    /// Name of the local host.
    pub(crate) local_host: String,
    /// User file, normally `~/.ssh/config`.
    pub(crate) user_file: Option<PathBuf>,
    /// System directory, normally `/etc/ssh` or `%ProgramData%\ssh`.
    pub(crate) system_dir: Option<PathBuf>,
    /// Key files configured outside `ssh_config`.
    pub(crate) extra_identities: Vec<PathBuf>,
}

impl ConfigSources {
    fn user_dir(&self) -> PathBuf {
        self.home.join(".ssh")
    }
}

/// Host key checking requested by `StrictHostKeyChecking`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StrictMode {
    /// `yes` or `ask`: only known hosts.
    Yes,
    /// `accept-new`: unknown hosts are recorded.
    AcceptNew,
    /// `no` or `off`: unknown hosts are accepted without recording.
    No,
}

/// Agent selected by `IdentityAgent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSetting {
    /// The agent named by `SSH_AUTH_SOCK` or the platform default.
    Default,
    /// No agent.
    Disabled,
    /// Socket or named pipe at this path.
    Path(PathBuf),
}

/// One host of a `ProxyJump` chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Jump {
    pub(crate) user: Option<String>,
    pub(crate) host: String,
    pub(crate) port: Option<u16>,
}

/// Place of a directive in a configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Location {
    pub(crate) file: PathBuf,
    pub(crate) line: usize,
}

/// Effective configuration of one host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostConfig {
    /// Host as requested, before `HostName`.
    pub(crate) original_host: String,
    /// Host to connect to.
    pub(crate) host_name: String,
    /// Remote account.
    pub(crate) user: String,
    /// Remote port.
    pub(crate) port: u16,
    /// Private key files in the order they are tried.
    pub(crate) identity_files: Vec<PathBuf>,
    /// Use only agent keys that correspond to `identity_files`.
    pub(crate) identities_only: bool,
    /// Agent to use.
    pub(crate) agent: AgentSetting,
    /// Known hosts files of the user.
    pub(crate) user_known_hosts: Vec<PathBuf>,
    /// Known hosts files of the system.
    pub(crate) global_known_hosts: Vec<PathBuf>,
    /// Requested host key checking, when configured.
    pub(crate) strict: Option<StrictMode>,
    /// Name used in place of the host in `known_hosts`.
    pub(crate) host_key_alias: Option<String>,
    /// Time limit of the TCP connection.
    pub(crate) connect_timeout: Option<Duration>,
    /// Interval of keepalive messages.
    pub(crate) server_alive_interval: Option<Duration>,
    /// Unanswered keepalive messages before the connection is closed.
    pub(crate) server_alive_count_max: Option<usize>,
    /// `HostKeyAlgorithms` value.
    pub(crate) host_key_algorithms: Option<String>,
    /// `PubkeyAcceptedAlgorithms` value.
    pub(crate) pubkey_algorithms: Option<String>,
    /// `ProxyJump` chain.
    pub(crate) proxy_jump: Vec<Jump>,
    /// `ProxyCommand` value other than `none`.
    pub(crate) proxy_command: Option<String>,
    /// `Match exec` criteria that were not evaluated.
    pub(crate) match_exec: Vec<Location>,
}

impl HostConfig {
    /// Name of the host in `known_hosts`: the alias, `host` for port 22 and
    /// `[host]:port` otherwise.
    pub(crate) fn known_hosts_name(&self) -> String {
        match &self.host_key_alias {
            Some(alias) => alias.clone(),
            None if self.port == DEFAULT_PORT => self.host_name.to_ascii_lowercase(),
            None => format!("[{}]:{}", self.host_name.to_ascii_lowercase(), self.port),
        }
    }
}

/// Request for one host.
#[derive(Debug, Clone)]
pub(crate) struct Query<'a> {
    pub(crate) host: &'a str,
    pub(crate) user: Option<&'a str>,
    pub(crate) port: Option<u16>,
}

/// Returns the configuration of every hop, jump hosts first and the target
/// last. Fails with `E_GIT_UNSUPPORTED` when the target uses `ProxyCommand`,
/// with `E_ARG` for a malformed `ProxyJump` and with `E_IO` when a
/// configuration file cannot be read.
pub(crate) fn route(sources: &ConfigSources, query: &Query<'_>) -> Result<Vec<HostConfig>> {
    let target = resolve(sources, query)?;
    if target.proxy_jump.is_empty() {
        if let Some(command) = &target.proxy_command {
            return Err(proxy_command_error(&target.original_host, command));
        }
        return Ok(vec![target]);
    }
    if target.proxy_jump.len() > MAX_JUMPS {
        return Err(Error::new(
            ErrorCode::GitUnsupported,
            format!(
                "ProxyJump chain of `{}` has more than {MAX_JUMPS} hosts",
                target.original_host
            ),
        ));
    }
    let mut hops = Vec::with_capacity(target.proxy_jump.len() + 1);
    for jump in &target.proxy_jump {
        let hop = resolve(
            sources,
            &Query {
                host: &jump.host,
                user: jump.user.as_deref(),
                port: jump.port,
            },
        )?;
        hops.push(hop);
    }
    hops.push(target);
    Ok(hops)
}

/// Returns the effective configuration of one host. Fails with `E_IO` when
/// a configuration file exists but cannot be read and with `E_ARG` for a
/// malformed `ProxyJump`.
pub(crate) fn resolve(sources: &ConfigSources, query: &Query<'_>) -> Result<HostConfig> {
    let mut walker = Walker {
        sources,
        query,
        collected: Collected::default(),
    };
    if let Some(file) = &sources.user_file {
        walker.file(file, &sources.user_dir(), 0)?;
    }
    if let Some(dir) = &sources.system_dir {
        walker.file(&dir.join("ssh_config"), dir, 0)?;
    }
    build(sources, query, walker.collected)
}

fn proxy_command_error(host: &str, command: &str) -> Error {
    Error::new(
        ErrorCode::GitUnsupported,
        format!("ProxyCommand of host `{host}` runs a program; use ProxyJump instead"),
    )
    .with_problem("proxy_command")
    .with_found(command.to_owned())
    .with_help("git.ssh")
}

#[derive(Debug, Default)]
struct Collected {
    values: Vec<(String, Vec<String>)>,
    identity_files: Vec<String>,
    match_exec: Vec<Location>,
}

impl Collected {
    fn set(&mut self, keyword: String, args: Vec<String>) {
        if keyword == "identityfile" {
            self.identity_files.extend(args.into_iter().take(1));
        } else if self.get(&keyword).is_none() {
            self.values.push((keyword, args));
        }
    }

    fn get(&self, keyword: &str) -> Option<&[String]> {
        self.values
            .iter()
            .find(|(name, _)| name == keyword)
            .map(|(_, args)| args.as_slice())
    }

    fn first(&self, keyword: &str) -> Option<&str> {
        self.get(keyword)
            .and_then(|args| args.first())
            .map(String::as_str)
    }
}

struct Walker<'a> {
    sources: &'a ConfigSources,
    query: &'a Query<'a>,
    collected: Collected,
}

impl Walker<'_> {
    fn file(&mut self, path: &Path, base: &Path, depth: usize) -> Result<()> {
        let text = match std::fs::read(path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(Error::new(
                    ErrorCode::Io,
                    format!("cannot read ssh configuration `{}`: {err}", path.display()),
                ));
            }
        };
        let mut active = true;
        for (index, line) in text.lines().enumerate() {
            let Some((keyword, args)) = tokenize(line) else {
                continue;
            };
            let location = Location {
                file: path.to_path_buf(),
                line: index + 1,
            };
            match keyword.as_str() {
                "host" => active = match_list(self.query.host, args.iter().map(String::as_str)),
                "match" => active = self.matches(&args, location),
                "include" if active && depth < MAX_INCLUDE_DEPTH => {
                    for pattern in &args {
                        for included in self.include_files(pattern, base) {
                            self.file(&included, base, depth + 1)?;
                        }
                    }
                }
                _ if active => self.collected.set(keyword, args),
                _ => {}
            }
        }
        Ok(())
    }

    fn matches(&mut self, args: &[String], location: Location) -> bool {
        let mut result = true;
        let mut index = 0;
        while index < args.len() {
            let (negated, criterion) = match args[index].strip_prefix('!') {
                Some(rest) => (true, rest.to_ascii_lowercase()),
                None => (false, args[index].to_ascii_lowercase()),
            };
            let value = args.get(index + 1).map(String::as_str).unwrap_or_default();
            let (matched, consumed) = match criterion.as_str() {
                "all" | "canonical" | "final" => (true, 1),
                "exec" => {
                    self.collected.match_exec.push(location.clone());
                    (false, 2)
                }
                _ => (self.criterion(&criterion, value), 2),
            };
            result &= matched != negated;
            index += consumed;
        }
        result
    }

    fn criterion(&self, criterion: &str, value: &str) -> bool {
        let patterns = || value.split(',');
        match criterion {
            "host" => {
                let host = self.collected.first("hostname").map_or_else(
                    || self.query.host.to_owned(),
                    |name| name.replace("%h", self.query.host),
                );
                match_list(&host, patterns())
            }
            "originalhost" => match_list(self.query.host, patterns()),
            "user" => {
                let user = self
                    .query
                    .user
                    .or_else(|| self.collected.first("user"))
                    .unwrap_or(&self.sources.local_user);
                match_list(user, patterns())
            }
            "localuser" => match_list(&self.sources.local_user, patterns()),
            _ => false,
        }
    }

    fn include_files(&self, pattern: &str, base: &Path) -> Vec<PathBuf> {
        let path = expand_tilde(pattern, &self.sources.home);
        let path = if path.is_absolute() {
            path
        } else {
            base.join(path)
        };
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Vec::new();
        };
        if !name.contains(['*', '?']) {
            return vec![path];
        }
        let Some(dir) = path.parent() else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|file| wildcard_match(name, file))
            })
            .map(|entry| entry.path())
            .collect();
        files.sort();
        files
    }
}

/// Splits a configuration line into a lowercase keyword and its arguments.
fn tokenize(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let keyword = line[..end].to_ascii_lowercase();
    let rest = line[end..].trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest);
    Some((keyword, split_args(rest)))
}

fn split_args(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut pending = false;
    for c in text.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                pending = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if pending {
                    args.push(std::mem::take(&mut current));
                    pending = false;
                }
            }
            c => {
                current.push(c);
                pending = true;
            }
        }
    }
    if pending {
        args.push(current);
    }
    args
}

fn build(sources: &ConfigSources, query: &Query<'_>, collected: Collected) -> Result<HostConfig> {
    let original_host = query.host.to_owned();
    let host_name = collected.first("hostname").map_or_else(
        || original_host.clone(),
        |name| name.replace("%h", &original_host),
    );
    let user = query
        .user
        .map(str::to_owned)
        .or_else(|| collected.first("user").map(str::to_owned))
        .unwrap_or_else(|| sources.local_user.clone());
    let port = query
        .port
        .or_else(|| collected.first("port").and_then(|port| port.parse().ok()))
        .unwrap_or(DEFAULT_PORT);
    let host_key_alias = collected.first("hostkeyalias").map(str::to_owned);
    let tokens = Tokens {
        sources,
        original_host: &original_host,
        host_name: &host_name,
        user: &user,
        port,
        alias: host_key_alias.as_deref(),
    };
    Ok(HostConfig {
        identity_files: identity_files(sources, &collected, &tokens),
        identities_only: flag(collected.first("identitiesonly")),
        agent: agent_setting(collected.first("identityagent"), &tokens),
        user_known_hosts: known_hosts_files(collected.get("userknownhostsfile"), &tokens, || {
            let dir = sources.user_dir();
            vec![dir.join("known_hosts"), dir.join("known_hosts2")]
        }),
        global_known_hosts: known_hosts_files(
            collected.get("globalknownhostsfile"),
            &tokens,
            || {
                sources
                    .system_dir
                    .iter()
                    .flat_map(|dir| [dir.join("ssh_known_hosts"), dir.join("ssh_known_hosts2")])
                    .collect()
            },
        ),
        strict: collected
            .first("stricthostkeychecking")
            .and_then(strict_mode),
        connect_timeout: seconds(collected.first("connecttimeout")),
        server_alive_interval: seconds(collected.first("serveraliveinterval")),
        server_alive_count_max: collected
            .first("serveralivecountmax")
            .and_then(|n| n.parse().ok()),
        host_key_algorithms: collected.first("hostkeyalgorithms").map(str::to_owned),
        pubkey_algorithms: collected
            .first("pubkeyacceptedalgorithms")
            .or_else(|| collected.first("pubkeyacceptedkeytypes"))
            .map(str::to_owned),
        proxy_jump: proxy_jump(collected.first("proxyjump"))?,
        proxy_command: collected
            .get("proxycommand")
            .map(|args| args.join(" "))
            .filter(|command| !command.eq_ignore_ascii_case("none")),
        match_exec: collected.match_exec,
        original_host,
        host_name,
        user,
        port,
        host_key_alias,
    })
}

struct Tokens<'a> {
    sources: &'a ConfigSources,
    original_host: &'a str,
    host_name: &'a str,
    user: &'a str,
    port: u16,
    alias: Option<&'a str>,
}

impl Tokens<'_> {
    /// Expands `~` and the `%` tokens of `ssh_config(5)`; unknown tokens are
    /// kept unchanged.
    fn expand(&self, value: &str) -> PathBuf {
        let mut out = String::with_capacity(value.len());
        let mut chars = value.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some(token) => self.token(token, &mut out),
                None => out.push('%'),
            }
        }
        expand_tilde(&out, &self.sources.home)
    }

    fn token(&self, token: char, out: &mut String) {
        let local_host = self.sources.local_host.as_str();
        match token {
            '%' => out.push('%'),
            'd' => out.push_str(&self.sources.home.to_string_lossy()),
            'h' => out.push_str(self.host_name),
            'n' => out.push_str(self.original_host),
            'p' => out.push_str(&self.port.to_string()),
            'r' => out.push_str(self.user),
            'u' => out.push_str(&self.sources.local_user),
            'k' => out.push_str(self.alias.unwrap_or(self.original_host)),
            'l' => out.push_str(local_host),
            'L' => out.push_str(local_host.split('.').next().unwrap_or(local_host)),
            other => {
                out.push('%');
                out.push(other);
            }
        }
    }
}

fn expand_tilde(value: &str, home: &Path) -> PathBuf {
    match value.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with(['/', '\\']) => home.join(&rest[1..]),
        _ => PathBuf::from(value),
    }
}

fn identity_files(
    sources: &ConfigSources,
    collected: &Collected,
    tokens: &Tokens<'_>,
) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = collected
        .identity_files
        .iter()
        .filter(|file| !file.eq_ignore_ascii_case("none"))
        .map(|file| tokens.expand(file))
        .collect();
    files.extend(sources.extra_identities.iter().cloned());
    if files.is_empty() && collected.identity_files.is_empty() {
        let dir = sources.user_dir();
        files.extend(DEFAULT_IDENTITIES.iter().map(|name| dir.join(name)));
    }
    files.dedup();
    files
}

fn agent_setting(value: Option<&str>, tokens: &Tokens<'_>) -> AgentSetting {
    match value {
        None => AgentSetting::Default,
        Some(value) if value.eq_ignore_ascii_case("none") => AgentSetting::Disabled,
        Some("SSH_AUTH_SOCK" | "$SSH_AUTH_SOCK" | "${SSH_AUTH_SOCK}") => AgentSetting::Default,
        Some(path) => AgentSetting::Path(tokens.expand(path)),
    }
}

fn known_hosts_files(
    value: Option<&[String]>,
    tokens: &Tokens<'_>,
    defaults: impl FnOnce() -> Vec<PathBuf>,
) -> Vec<PathBuf> {
    match value {
        None => defaults(),
        Some([single]) if single.eq_ignore_ascii_case("none") => Vec::new(),
        Some(files) => files.iter().map(|file| tokens.expand(file)).collect(),
    }
}

fn flag(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        value.eq_ignore_ascii_case("yes") || value.eq_ignore_ascii_case("true")
    })
}

fn strict_mode(value: &str) -> Option<StrictMode> {
    match value.to_ascii_lowercase().as_str() {
        "yes" | "ask" | "true" => Some(StrictMode::Yes),
        "accept-new" => Some(StrictMode::AcceptNew),
        "no" | "off" | "false" => Some(StrictMode::No),
        _ => None,
    }
}

fn seconds(value: Option<&str>) -> Option<Duration> {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|&secs| secs > 0)
        .map(Duration::from_secs)
}

fn proxy_jump(value: Option<&str>) -> Result<Vec<Jump>> {
    match value {
        None => Ok(Vec::new()),
        Some(value) if value.eq_ignore_ascii_case("none") => Ok(Vec::new()),
        Some(value) => value.split(',').map(parse_jump).collect(),
    }
}

fn parse_jump(spec: &str) -> Result<Jump> {
    let invalid = || {
        Error::new(
            ErrorCode::Arg,
            format!("ProxyJump host `{spec}` is malformed; expected `[user@]host[:port]`"),
        )
        .with_problem("invalid_proxy_jump")
    };
    let rest = spec.trim();
    let rest = rest.strip_prefix("ssh://").unwrap_or(rest);
    let (user, address) = match rest.rsplit_once('@') {
        Some((user, address)) => (Some(user.to_owned()), address),
        None => (None, rest),
    };
    let (host, port) = if let Some(bracketed) = address.strip_prefix('[') {
        let (host, tail) = bracketed.split_once(']').ok_or_else(invalid)?;
        (host, tail.strip_prefix(':'))
    } else {
        match address.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (address, None),
        }
    };
    let port = port
        .map(|port| {
            port.trim_end_matches('/')
                .parse::<u16>()
                .map_err(|_| invalid())
        })
        .transpose()?;
    if host.is_empty() || host.starts_with('-') {
        return Err(invalid());
    }
    Ok(Jump {
        user,
        host: host.to_owned(),
        port,
    })
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// Configuration of `host` as produced by an empty configuration.
    pub(crate) fn host_config(host: &str) -> HostConfig {
        HostConfig {
            original_host: host.to_owned(),
            host_name: host.to_owned(),
            user: "user".to_owned(),
            port: DEFAULT_PORT,
            identity_files: Vec::new(),
            identities_only: false,
            agent: AgentSetting::Default,
            user_known_hosts: Vec::new(),
            global_known_hosts: Vec::new(),
            strict: None,
            host_key_alias: None,
            connect_timeout: None,
            server_alive_interval: None,
            server_alive_count_max: None,
            host_key_algorithms: None,
            pubkey_algorithms: None,
            proxy_jump: Vec::new(),
            proxy_command: None,
            match_exec: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        sources: ConfigSources,
    }

    fn fixture(user_config: &str, system_config: Option<&str>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/config"), user_config).unwrap();
        let system_dir = dir.path().join("etc-ssh");
        std::fs::create_dir_all(&system_dir).unwrap();
        if let Some(text) = system_config {
            std::fs::write(system_dir.join("ssh_config"), text).unwrap();
        }
        let sources = ConfigSources {
            user_file: Some(home.join(".ssh/config")),
            home,
            local_user: "user".into(),
            local_host: "laptop.example.net".into(),
            system_dir: Some(system_dir),
            extra_identities: Vec::new(),
        };
        Fixture { _dir: dir, sources }
    }

    fn query(host: &str) -> Query<'_> {
        Query {
            host,
            user: None,
            port: None,
        }
    }

    #[test]
    fn applies_host_block_with_alias() {
        let f = fixture(
            "Host gh\n  HostName github.com\n  User git\n  Port 2222\n",
            None,
        );
        let config = resolve(&f.sources, &query("gh")).unwrap();
        assert_eq!(config.host_name, "github.com");
        assert_eq!(config.user, "git");
        assert_eq!(config.port, 2222);
        assert_eq!(config.known_hosts_name(), "[github.com]:2222");
    }

    #[test]
    fn first_value_wins_and_url_overrides_config() {
        let f = fixture(
            "Host *\n  User first\n  Port 2200\nHost example.com\n  User second\n",
            None,
        );
        let config = resolve(&f.sources, &query("example.com")).unwrap();
        assert_eq!(config.user, "first");
        let config = resolve(
            &f.sources,
            &Query {
                host: "example.com",
                user: Some("git"),
                port: Some(22),
            },
        )
        .unwrap();
        assert_eq!(config.user, "git");
        assert_eq!(config.port, 22);
        assert_eq!(config.known_hosts_name(), "example.com");
    }

    #[test]
    fn defaults_use_local_account_and_standard_files() {
        let f = fixture("", None);
        let config = resolve(&f.sources, &query("example.com")).unwrap();
        assert_eq!(config.user, "user");
        assert_eq!(config.port, DEFAULT_PORT);
        let names: Vec<_> = config
            .identity_files
            .iter()
            .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["id_ed25519", "id_ecdsa", "id_rsa"]);
        assert_eq!(config.agent, AgentSetting::Default);
        assert_eq!(config.user_known_hosts.len(), 2);
        assert_eq!(config.global_known_hosts.len(), 2);
    }

    #[test]
    fn identity_files_accumulate_and_expand_tokens() {
        let f = fixture(
            "Host example.com\n  IdentityFile ~/.ssh/%r@%h\n  IdentityFile \"%d/keys/%u key\"\nHost *\n  IdentityFile ~/.ssh/fallback\n  User git\n",
            None,
        );
        let config = resolve(&f.sources, &query("example.com")).unwrap();
        let home = &f.sources.home;
        assert_eq!(
            config.identity_files,
            [
                home.join(".ssh/git@example.com"),
                PathBuf::from(format!("{}/keys/user key", home.display())),
                home.join(".ssh/fallback"),
            ]
        );
    }

    #[test]
    fn negated_host_pattern_skips_block() {
        let f = fixture("Host *.example.com !build.example.com\n  User team\n", None);
        assert_eq!(
            resolve(&f.sources, &query("git.example.com")).unwrap().user,
            "team"
        );
        assert_eq!(
            resolve(&f.sources, &query("build.example.com"))
                .unwrap()
                .user,
            "user"
        );
    }

    #[test]
    fn match_criteria_combine_with_and() {
        let f = fixture(
            "Match host github.com user git\n  Port 443\nMatch originalhost gh !localuser bob\n  IdentitiesOnly yes\n",
            None,
        );
        let config = resolve(
            &f.sources,
            &Query {
                host: "github.com",
                user: Some("git"),
                port: None,
            },
        )
        .unwrap();
        assert_eq!(config.port, 443);
        assert!(resolve(&f.sources, &query("gh")).unwrap().identities_only);
    }

    #[test]
    fn match_host_uses_host_name_set_earlier() {
        let f = fixture(
            "Host gh\n  HostName github.com\nMatch host github.com\n  User git\n",
            None,
        );
        assert_eq!(resolve(&f.sources, &query("gh")).unwrap().user, "git");
    }

    #[test]
    fn match_exec_is_not_run_and_is_reported() {
        let f = fixture(
            "Match exec \"test -f /tmp/x\"\n  User exec\nHost *\n  User plain\n",
            None,
        );
        let config = resolve(&f.sources, &query("example.com")).unwrap();
        assert_eq!(config.user, "plain");
        assert_eq!(config.match_exec.len(), 1);
        assert_eq!(config.match_exec[0].line, 1);
    }

    #[test]
    fn include_reads_relative_glob_in_order() {
        let f = fixture("Include conf.d/*.conf\n", None);
        let dir = f.sources.home.join(".ssh/conf.d");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.conf"), "Host example.com\n  User second\n").unwrap();
        std::fs::write(dir.join("a.conf"), "Host example.com\n  User first\n").unwrap();
        std::fs::write(dir.join("c.txt"), "Host *\n  User ignored\n").unwrap();
        assert_eq!(
            resolve(&f.sources, &query("example.com")).unwrap().user,
            "first"
        );
    }

    #[test]
    fn include_inside_inactive_block_is_skipped() {
        let f = fixture("Host other\n  Include extra\n", None);
        std::fs::write(f.sources.home.join(".ssh/extra"), "User included\n").unwrap();
        assert_eq!(
            resolve(&f.sources, &query("example.com")).unwrap().user,
            "user"
        );
        assert_eq!(
            resolve(&f.sources, &query("other")).unwrap().user,
            "included"
        );
    }

    #[test]
    fn system_file_is_read_after_user_file() {
        let f = fixture(
            "Host example.com\n  User mine\n",
            Some("Host *\n  User system\n  Port 2022\n"),
        );
        let config = resolve(&f.sources, &query("example.com")).unwrap();
        assert_eq!(config.user, "mine");
        assert_eq!(config.port, 2022);
    }

    #[test]
    fn reads_equals_sign_and_case_insensitive_keywords() {
        let f = fixture(
            "HOST=example.com\nhostname=192.0.2.5\nSTRICTHOSTKEYCHECKING accept-new\n",
            None,
        );
        let config = resolve(&f.sources, &query("example.com")).unwrap();
        assert_eq!(config.host_name, "192.0.2.5");
        assert_eq!(config.strict, Some(StrictMode::AcceptNew));
    }

    #[test]
    fn known_hosts_alias_and_files() {
        let f = fixture(
            "Host example.com\n  HostKeyAlias build-box\n  UserKnownHostsFile ~/.ssh/kh1 ~/.ssh/kh2\n  GlobalKnownHostsFile none\n  IdentityAgent none\n",
            None,
        );
        let config = resolve(&f.sources, &query("example.com")).unwrap();
        assert_eq!(config.known_hosts_name(), "build-box");
        assert_eq!(config.user_known_hosts.len(), 2);
        assert!(config.global_known_hosts.is_empty());
        assert_eq!(config.agent, AgentSetting::Disabled);
    }

    #[test]
    fn route_lists_jump_hosts_before_target() {
        let f = fixture(
            "Host target\n  ProxyJump admin@bastion:2200,inner\nHost inner\n  HostName 198.51.100.1\n  ProxyJump ignored\n",
            None,
        );
        let hops = route(&f.sources, &query("target")).unwrap();
        assert_eq!(hops.len(), 3);
        assert_eq!(hops[0].host_name, "bastion");
        assert_eq!(hops[0].user, "admin");
        assert_eq!(hops[0].port, 2200);
        assert_eq!(hops[1].host_name, "198.51.100.1");
        assert_eq!(hops[2].original_host, "target");
    }

    #[test]
    fn proxy_command_is_refused() {
        let f = fixture("Host target\n  ProxyCommand nc %h %p\n", None);
        let error = route(&f.sources, &query("target")).unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitUnsupported);
        assert!(error.to_string().contains("ProxyJump"));
    }

    #[test]
    fn malformed_proxy_jump_is_an_argument_error() {
        let f = fixture("Host target\n  ProxyJump host:notaport\n", None);
        assert_eq!(
            route(&f.sources, &query("target")).unwrap_err().code(),
            ErrorCode::Arg
        );
    }

    #[test]
    fn parses_jump_forms() {
        assert_eq!(
            parse_jump("ssh://git@[::1]:2222").unwrap(),
            Jump {
                user: Some("git".into()),
                host: "::1".into(),
                port: Some(2222)
            }
        );
        assert_eq!(parse_jump("bastion").unwrap().port, None);
    }

    #[test]
    fn reads_timeouts_and_algorithms() {
        let f = fixture(
            "ConnectTimeout 7\nServerAliveInterval 15\nServerAliveCountMax 2\nHostKeyAlgorithms -ssh-rsa\nPubkeyAcceptedKeyTypes +ssh-rsa\n",
            None,
        );
        let config = resolve(&f.sources, &query("example.com")).unwrap();
        assert_eq!(config.connect_timeout, Some(Duration::from_secs(7)));
        assert_eq!(config.server_alive_interval, Some(Duration::from_secs(15)));
        assert_eq!(config.server_alive_count_max, Some(2));
        assert_eq!(config.host_key_algorithms.as_deref(), Some("-ssh-rsa"));
        assert_eq!(config.pubkey_algorithms.as_deref(), Some("+ssh-rsa"));
    }

    #[test]
    fn missing_files_are_not_errors() {
        let mut f = fixture("", None);
        f.sources.user_file = Some(f.sources.home.join("absent"));
        f.sources.system_dir = None;
        assert_eq!(resolve(&f.sources, &query("h")).unwrap().host_name, "h");
    }
}
