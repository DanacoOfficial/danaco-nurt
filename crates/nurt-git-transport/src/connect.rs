// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Selection of the transport for a repository URL.
//!
//! [`connect`] rewrites the address with the `url.<base>.insteadOf` and
//! `url.<base>.pushInsteadOf` rules of the configuration and opens the
//! transport of its scheme: the in-process local transport for `file://`
//! addresses and local paths, smart HTTP for `http://` and `https://`, and
//! SSH for `ssh://` and the scp-like form `user@host:path`. No transport
//! starts a program, so `git://` and `ext::` addresses are rejected.

use gix_transport::Protocol;
use gix_transport::client::blocking_io::Transport;
use nurt_contracts::error::{Error, ErrorCode};

use crate::http::{self, HttpSettings, redact_text, redact_url};
use crate::local::{self, LocalError};
#[cfg(feature = "git-ssh")]
use crate::ssh::{self, SshOptions};

/// Direction of a transfer, which selects the rewrite rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Fetch or clone: `insteadOf` rules.
    Fetch,
    /// Push: `pushInsteadOf` rules, then `insteadOf` rules.
    Push,
}

/// One rewrite rule: addresses starting with `prefix` start with `base`
/// instead.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    prefix: String,
    base: String,
}

/// Address rewrite rules of the `url.<base>` configuration sections.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UrlRewrites {
    fetch: Vec<Rule>,
    push: Vec<Rule>,
}

impl UrlRewrites {
    /// Rules of every `url.<base>.insteadOf` and `url.<base>.pushInsteadOf`
    /// value of `config`.
    #[must_use]
    pub fn from_config(config: &gix::config::File) -> Self {
        let mut rewrites = Self::default();
        let Some(sections) = config.sections_by_name("url") else {
            return rewrites;
        };
        for section in sections {
            let Some(base) = section.header().subsection_name() else {
                continue;
            };
            let base = base.to_string();
            for prefix in section.body().values("insteadOf") {
                rewrites.add(&base, &prefix.to_string());
            }
            for prefix in section.body().values("pushInsteadOf") {
                rewrites.add_push(&base, &prefix.to_string());
            }
        }
        rewrites
    }

    /// Adds the rule `url.<base>.insteadOf = <prefix>`.
    pub fn add(&mut self, base: &str, prefix: &str) {
        self.fetch.push(Rule {
            prefix: prefix.to_owned(),
            base: base.to_owned(),
        });
    }

    /// Adds the rule `url.<base>.pushInsteadOf = <prefix>`.
    pub fn add_push(&mut self, base: &str, prefix: &str) {
        self.push.push(Rule {
            prefix: prefix.to_owned(),
            base: base.to_owned(),
        });
    }

    /// Whether no rule is configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fetch.is_empty() && self.push.is_empty()
    }

    /// Rewrites `url` as `git` does: the rule with the longest matching
    /// prefix wins; for a push a matching `pushInsteadOf` rule takes
    /// precedence over every `insteadOf` rule.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_transport::connect::{Direction, UrlRewrites};
    ///
    /// let mut rewrites = UrlRewrites::default();
    /// rewrites.add("https://github.com/", "gh:");
    /// rewrites.add_push("ssh://git@github.com/", "https://github.com/");
    /// assert_eq!(rewrites.rewrite("gh:org/repo", Direction::Fetch), "https://github.com/org/repo");
    /// assert_eq!(
    ///     rewrites.rewrite("https://github.com/org/repo", Direction::Push),
    ///     "ssh://git@github.com/org/repo"
    /// );
    /// ```
    #[must_use]
    pub fn rewrite(&self, url: &str, direction: Direction) -> String {
        if direction == Direction::Push
            && let Some(rewritten) = apply_longest(&self.push, url)
        {
            return rewritten;
        }
        apply_longest(&self.fetch, url).unwrap_or_else(|| url.to_owned())
    }
}

fn apply_longest(rules: &[Rule], url: &str) -> Option<String> {
    rules
        .iter()
        .filter(|rule| url.starts_with(rule.prefix.as_str()))
        .max_by_key(|rule| rule.prefix.len())
        .map(|rule| format!("{}{}", rule.base, &url[rule.prefix.len()..]))
}

/// Settings of [`connect`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TransportOptions {
    /// Protocol version requested from the server.
    pub protocol: Protocol,
    /// Address rewrite rules.
    pub rewrites: UrlRewrites,
    /// HTTP settings resolved for the address.
    pub http: HttpSettings,
    /// SSH settings; required for SSH addresses.
    #[cfg(feature = "git-ssh")]
    pub ssh: Option<SshOptions>,
}

impl TransportOptions {
    /// Options with protocol version 2, no rewrite rules and the given HTTP
    /// settings.
    #[must_use]
    pub fn new(http: HttpSettings) -> Self {
        Self {
            protocol: Protocol::V2,
            rewrites: UrlRewrites::default(),
            http,
            #[cfg(feature = "git-ssh")]
            ssh: None,
        }
    }

    /// The same options with the given rewrite rules.
    #[must_use]
    pub fn with_rewrites(mut self, rewrites: UrlRewrites) -> Self {
        self.rewrites = rewrites;
        self
    }

    /// The same options with the given SSH settings.
    #[cfg(feature = "git-ssh")]
    #[must_use]
    pub fn with_ssh(mut self, ssh: SshOptions) -> Self {
        self.ssh = Some(ssh);
        self
    }
}

/// Name of the remote helper in an address of the form `<name>::<address>`,
/// which `git` runs as the program `git-remote-<name>`.
fn remote_helper(address: &str) -> Option<&str> {
    let end = address
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
        .unwrap_or(address.len());
    let name = &address[..end];
    (name.starts_with(|c: char| c.is_ascii_alphanumeric()) && address[end..].starts_with("::"))
        .then_some(name)
}

/// Address after the rewrite rules of `options`, parsed.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` (problem `invalid_url`) when the address
/// cannot be parsed and with `E_GIT_UNSUPPORTED` (problem
/// `unsupported_transport`) for an address of the form `<name>::<address>`.
pub fn resolve_url(
    url: &str,
    direction: Direction,
    options: &TransportOptions,
) -> Result<gix_url::Url, Error> {
    let rewritten = options.rewrites.rewrite(url, direction);
    if let Some(helper) = remote_helper(&rewritten) {
        return Err(unsupported(
            &format!("{helper}::"),
            "the transport starts a program; use https://, ssh:// or a local path",
        ));
    }
    gix_url::parse(rewritten.as_str()).map_err(|failure| {
        Error::new(
            ErrorCode::InvalidValue,
            format!(
                "invalid repository address {}: {}; use https://, ssh://, user@host:path or a local path",
                redact_url(&rewritten),
                redact_text(&failure.to_string()),
            ),
        )
        .with_problem("invalid_url")
        .with_found(redact_url(&rewritten))
    })
}

/// Opens the transport for `url` after the rewrite rules of `options`.
///
/// Opening does not contact the server: the connection is made by the
/// handshake of the transfer.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` for an address that cannot be parsed,
/// `E_GIT_NOT_REPOSITORY` for a local path that is not a repository, and
/// `E_GIT_UNSUPPORTED` for `git://` and `ext::` addresses and, in builds
/// without SSH, for SSH addresses; SSH failures keep the errors of the SSH
/// transport.
pub fn connect(
    url: &str,
    direction: Direction,
    options: &TransportOptions,
) -> Result<Box<dyn Transport>, Error> {
    let parsed = resolve_url(url, direction, options)?;
    match parsed.scheme {
        gix_url::Scheme::File => local::connect(parsed, options.protocol)
            .map(|transport| Box::new(transport) as Box<dyn Transport>)
            .map_err(|failure| local_error(&failure)),
        gix_url::Scheme::Http | gix_url::Scheme::Https => Ok(Box::new(http::connect(
            parsed,
            options.http.clone(),
            options.protocol,
        ))),
        gix_url::Scheme::Ssh => connect_ssh(&parsed, options),
        gix_url::Scheme::Git => Err(unsupported(
            "git://",
            "the unauthenticated git protocol is not supported; use https:// or ssh://",
        )),
        _ => Err(unsupported(
            &parsed.scheme.to_string(),
            "the transport starts a program; use https://, ssh:// or a local path",
        )),
    }
}

#[cfg(feature = "git-ssh")]
fn connect_ssh(
    url: &gix_url::Url,
    options: &TransportOptions,
) -> Result<Box<dyn Transport>, Error> {
    let Some(ssh_options) = &options.ssh else {
        return Err(Error::new(
            ErrorCode::Internal,
            "the SSH settings are not configured for the transfer",
        ));
    };
    ssh::connect(url, options.protocol, ssh_options)
        .map(|transport| Box::new(transport) as Box<dyn Transport>)
}

#[cfg(not(feature = "git-ssh"))]
fn connect_ssh(
    _url: &gix_url::Url,
    _options: &TransportOptions,
) -> Result<Box<dyn Transport>, Error> {
    Err(unsupported(
        "ssh://",
        "this build has no SSH support; use https:// or a local path",
    ))
}

fn unsupported(scheme: &str, reason: &str) -> Error {
    Error::new(
        ErrorCode::GitUnsupported,
        format!("unsupported repository address scheme `{scheme}`: {reason}"),
    )
    .with_problem("unsupported_transport")
    .with_found(scheme)
}

fn local_error(failure: &LocalError) -> Error {
    match failure {
        LocalError::NotRepository { path, .. } => Error::new(
            ErrorCode::GitNotRepository,
            format!("{failure}; pass the path of a git repository"),
        )
        .with_problem("not_repository")
        .with_found(path.to_string_lossy()),
    }
}

#[cfg(test)]
mod tests {
    use bstr::ByteSlice;

    use super::*;

    fn options() -> TransportOptions {
        TransportOptions::new(HttpSettings::default())
    }

    fn parse_config(text: &str) -> gix::config::File {
        gix::config::File::from_bytes_owned(
            &mut text.as_bytes().to_vec(),
            gix::config::file::Metadata::from(gix::config::Source::Local),
            gix::config::file::init::Options::default(),
        )
        .expect("valid configuration")
    }

    #[test]
    fn longest_prefix_wins() {
        let mut rewrites = UrlRewrites::default();
        rewrites.add("https://a.example/", "work:");
        rewrites.add("https://b.example/team/", "work:team/");
        assert_eq!(
            rewrites.rewrite("work:team/x", Direction::Fetch),
            "https://b.example/team/x"
        );
        assert_eq!(
            rewrites.rewrite("work:y", Direction::Fetch),
            "https://a.example/y"
        );
        assert_eq!(rewrites.rewrite("other:z", Direction::Fetch), "other:z");
    }

    #[test]
    fn push_rules_apply_only_to_pushes_and_take_precedence() {
        let mut rewrites = UrlRewrites::default();
        rewrites.add("https://mirror.example/", "https://origin.example/");
        rewrites.add_push("ssh://git@origin.example/", "https://origin.example/");
        let url = "https://origin.example/r.git";
        assert_eq!(
            rewrites.rewrite(url, Direction::Fetch),
            "https://mirror.example/r.git"
        );
        assert_eq!(
            rewrites.rewrite(url, Direction::Push),
            "ssh://git@origin.example/r.git"
        );
    }

    #[test]
    fn push_falls_back_to_instead_of_rules() {
        let mut rewrites = UrlRewrites::default();
        rewrites.add("https://example.com/", "ex:");
        assert_eq!(
            rewrites.rewrite("ex:r", Direction::Push),
            "https://example.com/r"
        );
    }

    #[test]
    fn rules_are_read_from_url_sections() {
        let config = parse_config(
            "[url \"https://github.com/\"]\n\tinsteadOf = gh:\n\tinsteadOf = github:\n[url \"ssh://git@github.com/\"]\n\tpushInsteadOf = https://github.com/\n",
        );
        let rewrites = UrlRewrites::from_config(&config);
        assert_eq!(
            rewrites.rewrite("github:o/r", Direction::Fetch),
            "https://github.com/o/r"
        );
        assert_eq!(
            rewrites.rewrite("https://github.com/o/r", Direction::Push),
            "ssh://git@github.com/o/r"
        );
        assert_eq!(
            rewrites.rewrite("gh:o/r", Direction::Push),
            "https://github.com/o/r"
        );
        assert!(UrlRewrites::from_config(&parse_config("")).is_empty());
    }

    #[test]
    fn git_and_ext_addresses_are_unsupported() {
        let error = connect("git://example.com/r.git", Direction::Fetch, &options())
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::GitUnsupported);
        assert_eq!(error.problem.as_deref(), Some("unsupported_transport"));
    }

    #[test]
    fn remote_helper_addresses_are_unsupported() {
        for address in [
            "ext::sh -c 'echo x' %S",
            "fd::17",
            "ext::ssh -o ProxyCommand=x host",
        ] {
            let error = connect(address, Direction::Fetch, &options())
                .err()
                .unwrap();
            assert_eq!(error.code, ErrorCode::GitUnsupported, "{address}");
            assert_eq!(error.problem.as_deref(), Some("unsupported_transport"));
        }
        let mut rewrites = UrlRewrites::default();
        rewrites.add("ext::sh -c x ", "safe:");
        let error = resolve_url(
            "safe:repo",
            Direction::Fetch,
            &options().with_rewrites(rewrites),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::GitUnsupported);
    }

    #[test]
    fn finds_the_name_of_a_remote_helper_only_before_double_colon() {
        assert_eq!(remote_helper("ext::cmd"), Some("ext"));
        assert_eq!(remote_helper("git+x.y-z::addr"), Some("git+x.y-z"));
        assert_eq!(remote_helper("host:path"), None);
        assert_eq!(remote_helper("user@host::path"), None);
        assert_eq!(remote_helper("::x"), None);
        assert_eq!(remote_helper("https://example.com/r.git"), None);
        assert_eq!(remote_helper("/srv/repo"), None);
        assert_eq!(remote_helper(""), None);
    }

    #[test]
    fn http_addresses_open_without_contacting_the_server() {
        let transport = connect(
            "https://example.invalid/r.git",
            Direction::Fetch,
            &options(),
        )
        .unwrap();
        assert!(transport.to_url().contains_str("example.invalid"));
    }

    #[test]
    fn local_path_that_is_not_a_repository_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let error = connect(&dir.path().to_string_lossy(), Direction::Fetch, &options())
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::GitNotRepository);
    }

    #[test]
    fn local_repository_opens_the_in_process_transport() {
        let dir = tempfile::tempdir().unwrap();
        gix::init_bare(dir.path()).unwrap();
        let url = format!("file://{}", dir.path().display());
        assert!(connect(&url, Direction::Fetch, &options()).is_ok());
    }

    #[test]
    fn rewritten_address_is_parsed() {
        let mut rewrites = UrlRewrites::default();
        rewrites.add("https://example.com/", "ex:");
        let resolved = resolve_url(
            "ex:a/b.git",
            Direction::Fetch,
            &options().with_rewrites(rewrites),
        )
        .unwrap();
        assert_eq!(resolved.scheme, gix_url::Scheme::Https);
        assert_eq!(resolved.host(), Some("example.com"));
    }
}
