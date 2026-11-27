// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport::http::settings
// Responsibility:   HTTP options of one remote address, resolved from the repository.
// Dependencies:     bstr
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! HTTP options of one remote address, resolved from the repository
//! configuration (`http.*`, `http.<url>.*`, `remote.<name>.proxy`) and the
//! process environment with the precedence of `git`.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use bstr::ByteSlice;

use super::failure::{NetworkFailure, NetworkProblem, redact_url};
use super::urlmatch::{self, UrlParts};

/// Default of `http.postBuffer`: request bodies up to this size are sent with
/// `Content-Length`, larger ones with chunked transfer encoding.
pub const DEFAULT_POST_BUFFER: usize = 1024 * 1024;

/// Time allowed for establishing a connection; the default of curl.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(300);

/// `http.*` keys whose function the backend does not provide; they are
/// reported instead of being silently dropped.
const UNSUPPORTED_KEYS: [&str; 8] = [
    "sslCert",
    "sslKey",
    "sslCertPasswordProtected",
    "sslVersion",
    "sslCipherList",
    "cookieFile",
    "pinnedPubkey",
    "curloptResolve",
];

/// When redirects of a response are followed (`http.followRedirects`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FollowRedirects {
    /// Only for the first request of a connection, the reference discovery.
    #[default]
    Initial,
    /// For every request.
    All,
    /// Never.
    None,
}

/// Bearer token sent as `Authorization: Bearer`; never shown by `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct BearerToken(String);

impl BearerToken {
    /// Wraps a token.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// The token itself, for building the request header.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BearerToken(<redacted>)")
    }
}

/// Unusual condition found while resolving the options, for the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpNote {
    /// `http.sslVerify` is false or `GIT_SSL_NO_VERIFY` is set: server
    /// certificates are not verified.
    TlsVerificationDisabled,
    /// A configuration key without effect in this backend.
    OptionIgnored {
        /// Full key name, for example `http.sslCert`.
        key: String,
    },
}

/// HTTP options for requests to one remote address.
#[derive(Clone, PartialEq, Eq)]
pub struct HttpSettings {
    /// `name: value` lines added to every request (`http.extraHeader`).
    pub extra_headers: Vec<String>,
    /// Redirect policy.
    pub follow_redirects: FollowRedirects,
    /// Minimum transfer speed in bytes per second; 0 disables the check.
    pub low_speed_limit: u32,
    /// Time the speed may stay below the limit.
    pub low_speed_time: Duration,
    /// Proxy address, `None` for a direct connection.
    pub proxy: Option<String>,
    /// `User-Agent` header.
    pub user_agent: String,
    /// Time allowed for establishing a connection.
    pub connect_timeout: Duration,
    /// PEM file with trusted certificates, replacing the system store.
    pub ca_info: Option<PathBuf>,
    /// Directory with PEM files of trusted certificates, replacing the system
    /// store.
    pub ca_path: Option<PathBuf>,
    /// Verify server certificates.
    pub ssl_verify: bool,
    /// Largest request body sent with `Content-Length`.
    pub post_buffer: usize,
    /// Bearer token, used when the transport sends no `Authorization` header.
    pub bearer: Option<BearerToken>,
    /// Conditions to report.
    pub notes: Vec<HttpNote>,
}

/// The extra headers may carry credentials, so only their number is shown, and
/// the user name and password of the proxy address are removed.
impl fmt::Debug for HttpSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpSettings")
            .field("extra_headers", &self.extra_headers.len())
            .field("follow_redirects", &self.follow_redirects)
            .field("low_speed_limit", &self.low_speed_limit)
            .field("low_speed_time", &self.low_speed_time)
            .field("proxy", &self.proxy.as_deref().map(redact_url))
            .field("user_agent", &self.user_agent)
            .field("connect_timeout", &self.connect_timeout)
            .field("ca_info", &self.ca_info)
            .field("ca_path", &self.ca_path)
            .field("ssl_verify", &self.ssl_verify)
            .field("post_buffer", &self.post_buffer)
            .field("bearer", &self.bearer)
            .field("notes", &self.notes)
            .finish()
    }
}

impl Default for HttpSettings {
    fn default() -> Self {
        Self {
            extra_headers: Vec::new(),
            follow_redirects: FollowRedirects::Initial,
            low_speed_limit: 0,
            low_speed_time: Duration::ZERO,
            proxy: None,
            user_agent: default_user_agent(),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            ca_info: None,
            ca_path: None,
            ssl_verify: true,
            post_buffer: DEFAULT_POST_BUFFER,
            bearer: None,
            notes: Vec::new(),
        }
    }
}

/// `User-Agent` used when `http.userAgent` is not set; servers recognise Git
/// clients by the `git/` prefix.
#[must_use]
pub fn default_user_agent() -> String {
    format!("git/nurt-{}", env!("CARGO_PKG_VERSION"))
}

/// Values of the `http` sections that apply to one address.
struct HttpSections<'c> {
    config: &'c gix::config::File,
    url: String,
}

impl HttpSections<'_> {
    /// Value of `http.<key>` from the most specific matching section; later
    /// sections win ties.
    fn value(&self, key: &str) -> Option<String> {
        urlmatch::best_value(self.config, "http", key, &self.url)
    }

    /// All values of `http.<key>` from matching sections in configuration
    /// order; an empty value clears the values before it.
    fn values(&self, key: &str) -> Vec<String> {
        urlmatch::all_values(self.config, "http", key, &self.url)
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn parse_size(value: &str) -> Option<u64> {
    let value = value.trim();
    let (digits, factor) = match value.chars().last().map(|c| c.to_ascii_lowercase()) {
        Some('k') => (&value[..value.len() - 1], 1024),
        Some('m') => (&value[..value.len() - 1], 1024 * 1024),
        Some('g') => (&value[..value.len() - 1], 1024 * 1024 * 1024),
        _ => (value, 1),
    };
    digits.parse::<u64>().ok()?.checked_mul(factor)
}

fn invalid(url: &str, key: &str, value: &str) -> NetworkFailure {
    NetworkFailure::new(
        NetworkProblem::Connect,
        url,
        format!("invalid value '{value}' of {key}; correct it in the git configuration"),
    )
}

fn normalize_proxy(proxy: &str) -> String {
    if proxy.contains("://") {
        proxy.to_owned()
    } else {
        format!("http://{proxy}")
    }
}

/// Whether `host` is excluded from proxying by a `NO_PROXY` list.
fn no_proxy_matches(list: &str, host: &str) -> bool {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    list.split([',', ' '])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            let entry = entry.trim_start_matches('.').to_ascii_lowercase();
            entry == "*" || host == entry || host.ends_with(&format!(".{entry}"))
        })
}

impl HttpSettings {
    /// Resolves the options for requests to `url`.
    ///
    /// `remote` is the name of the configured remote, if any, for
    /// `remote.<name>.proxy`; `env` reads a variable of the process
    /// environment. Environment variables of `git` (`GIT_SSL_NO_VERIFY`,
    /// `GIT_SSL_CAINFO`, `GIT_SSL_CAPATH`, `GIT_HTTP_LOW_SPEED_LIMIT`,
    /// `GIT_HTTP_LOW_SPEED_TIME`, `GIT_HTTP_USER_AGENT`) override the
    /// configuration; the proxy variables (`https_proxy`, `http_proxy`,
    /// `all_proxy`, `no_proxy`) apply when the configuration sets no proxy.
    ///
    /// # Errors
    ///
    /// Fails with [`NetworkProblem::Proxy`] for a proxy authentication method
    /// other than `basic` or `anyauth`, and with [`NetworkProblem::Connect`]
    /// for values that cannot be parsed.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_transport::http::HttpSettings;
    ///
    /// let config: gix::config::File =
    ///     "[http]\n\tpostBuffer = 2m\n[http \"https://example.com\"]\n\tsslVerify = false\n"
    ///         .parse()
    ///         .expect("valid configuration");
    /// let settings = HttpSettings::resolve(&config, "https://example.com/r.git", None, &|_| None)
    ///     .expect("valid options");
    /// assert_eq!(settings.post_buffer, 2 * 1024 * 1024);
    /// assert!(!settings.ssl_verify);
    /// ```
    pub fn resolve(
        config: &gix::config::File,
        url: &str,
        remote: Option<&str>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, NetworkFailure> {
        let sections = HttpSections {
            config,
            url: url.to_owned(),
        };
        let mut settings = Self {
            extra_headers: sections.values("extraHeader"),
            ..Self::default()
        };
        settings.apply_redirects_and_limits(&sections, url, env)?;
        settings.apply_tls(&sections, url, env)?;
        settings.apply_proxy(config, &sections, url, remote, env)?;
        settings.note_unsupported(&sections);
        Ok(settings)
    }

    fn apply_redirects_and_limits(
        &mut self,
        sections: &HttpSections<'_>,
        url: &str,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(), NetworkFailure> {
        if let Some(value) = sections.value("followRedirects") {
            self.follow_redirects = match value.trim().to_ascii_lowercase().as_str() {
                "initial" => FollowRedirects::Initial,
                other => match parse_bool(other) {
                    Some(true) => FollowRedirects::All,
                    Some(false) => FollowRedirects::None,
                    None => return Err(invalid(url, "http.followRedirects", &value)),
                },
            };
        }
        if let Some(value) = sections.value("postBuffer") {
            let size = parse_size(&value).ok_or_else(|| invalid(url, "http.postBuffer", &value))?;
            self.post_buffer = usize::try_from(size).unwrap_or(usize::MAX).max(1);
        }
        let limit = env("GIT_HTTP_LOW_SPEED_LIMIT").or_else(|| sections.value("lowSpeedLimit"));
        if let Some(value) = limit {
            let parsed =
                parse_size(&value).ok_or_else(|| invalid(url, "http.lowSpeedLimit", &value))?;
            self.low_speed_limit = u32::try_from(parsed).unwrap_or(u32::MAX);
        }
        let time = env("GIT_HTTP_LOW_SPEED_TIME").or_else(|| sections.value("lowSpeedTime"));
        if let Some(value) = time {
            let seconds =
                parse_size(&value).ok_or_else(|| invalid(url, "http.lowSpeedTime", &value))?;
            self.low_speed_time = Duration::from_secs(seconds);
        }
        if let Some(agent) = env("GIT_HTTP_USER_AGENT").or_else(|| sections.value("userAgent")) {
            self.user_agent = agent;
        }
        Ok(())
    }

    fn apply_tls(
        &mut self,
        sections: &HttpSections<'_>,
        url: &str,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(), NetworkFailure> {
        if let Some(value) = sections.value("sslVerify") {
            self.ssl_verify =
                parse_bool(&value).ok_or_else(|| invalid(url, "http.sslVerify", &value))?;
        }
        if env("GIT_SSL_NO_VERIFY").is_some() {
            self.ssl_verify = false;
        }
        if !self.ssl_verify {
            self.notes.push(HttpNote::TlsVerificationDisabled);
        }
        self.ca_info = env("GIT_SSL_CAINFO")
            .or_else(|| sections.value("sslCAInfo"))
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        self.ca_path = env("GIT_SSL_CAPATH")
            .or_else(|| sections.value("sslCAPath"))
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        Ok(())
    }

    fn apply_proxy(
        &mut self,
        config: &gix::config::File,
        sections: &HttpSections<'_>,
        url: &str,
        remote: Option<&str>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(), NetworkFailure> {
        let remote_value = |key: &str| {
            remote.and_then(|name| {
                config
                    .string_by("remote", Some(name.into()), key)
                    .map(|v| v.to_str_lossy().into_owned())
            })
        };
        let method = remote_value("proxyAuthMethod").or_else(|| sections.value("proxyAuthMethod"));
        if let Some(method) = method {
            let method = method.trim().to_ascii_lowercase();
            if method != "basic" && method != "anyauth" {
                return Err(NetworkFailure::new(
                    NetworkProblem::Proxy,
                    url,
                    format!(
                        "proxy authentication method '{method}' is not supported; set http.proxyAuthMethod to basic"
                    ),
                ));
            }
        }
        let configured = remote_value("proxy").or_else(|| sections.value("proxy"));
        let https = url.to_ascii_lowercase().starts_with("https://");
        let from_env = || {
            let scheme_vars: &[&str] = if https {
                &["https_proxy", "HTTPS_PROXY"]
            } else {
                &["http_proxy"]
            };
            scheme_vars
                .iter()
                .chain(&["all_proxy", "ALL_PROXY"])
                .find_map(|name| env(name).filter(|value| !value.is_empty()))
        };
        let proxy = match configured {
            Some(value) if value.is_empty() => None,
            Some(value) => Some(value),
            None => from_env(),
        };
        let host = UrlParts::parse(url)
            .map(|_| host_of(url))
            .unwrap_or_default();
        let excluded = env("no_proxy")
            .or_else(|| env("NO_PROXY"))
            .is_some_and(|list| no_proxy_matches(&list, &host));
        self.proxy = proxy.filter(|_| !excluded).map(|p| normalize_proxy(&p));
        Ok(())
    }

    fn note_unsupported(&mut self, sections: &HttpSections<'_>) {
        for key in UNSUPPORTED_KEYS {
            if sections.value(key).is_some() {
                self.notes.push(HttpNote::OptionIgnored {
                    key: format!("http.{key}"),
                });
            }
        }
        if sections
            .value("version")
            .is_some_and(|v| !v.eq_ignore_ascii_case("HTTP/1.1"))
        {
            self.notes.push(HttpNote::OptionIgnored {
                key: "http.version".to_owned(),
            });
        }
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn resolve(
        config: &str,
        url: &str,
        remote: Option<&str>,
        env: &[(&str, &str)],
    ) -> Result<HttpSettings, NetworkFailure> {
        let config: gix::config::File = config.parse().expect("valid configuration");
        let env: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        HttpSettings::resolve(&config, url, remote, &|name| env.get(name).cloned())
    }

    #[test]
    fn uses_defaults_without_configuration() {
        let settings = resolve("", "https://h/r", None, &[]).expect("defaults");
        assert_eq!(settings.post_buffer, DEFAULT_POST_BUFFER);
        assert!(settings.ssl_verify && settings.proxy.is_none() && settings.notes.is_empty());
        assert!(settings.user_agent.starts_with("git/nurt-"));
        assert_eq!(settings.follow_redirects, FollowRedirects::Initial);
    }

    #[test]
    fn most_specific_url_section_wins() {
        let config = "[http]\n\tproxy = global:1\n[http \"https://h\"]\n\tproxy = host:2\n[http \"https://h/org\"]\n\tproxy = org:3\n[http \"https://other\"]\n\tproxy = other:4\n";
        let settings = resolve(config, "https://h/org/r", None, &[]).expect("valid");
        assert_eq!(settings.proxy.as_deref(), Some("http://org:3"));
        let settings = resolve(config, "https://h/x/r", None, &[]).expect("valid");
        assert_eq!(settings.proxy.as_deref(), Some("http://host:2"));
    }

    #[test]
    fn remote_proxy_precedes_http_proxy_and_empty_disables() {
        let config = "[http]\n\tproxy = http://p:1\n[remote \"origin\"]\n\tproxy = http://r:2\n";
        let settings = resolve(config, "https://h/r", Some("origin"), &[]).expect("valid");
        assert_eq!(settings.proxy.as_deref(), Some("http://r:2"));
        let settings = resolve(
            "[http]\n\tproxy =\n",
            "https://h/r",
            None,
            &[("https_proxy", "e:3")],
        )
        .expect("valid");
        assert_eq!(settings.proxy, None);
    }

    #[test]
    fn environment_proxy_respects_scheme_and_no_proxy() {
        let env = [
            ("https_proxy", "s:1"),
            ("http_proxy", "p:2"),
            ("ALL_PROXY", "a:3"),
        ];
        assert_eq!(
            resolve("", "https://h/r", None, &env)
                .expect("valid")
                .proxy
                .as_deref(),
            Some("http://s:1")
        );
        assert_eq!(
            resolve("", "http://h/r", None, &env)
                .expect("valid")
                .proxy
                .as_deref(),
            Some("http://p:2")
        );
        let only_all = [("ALL_PROXY", "a:3"), ("NO_PROXY", "example.com")];
        assert_eq!(
            resolve("", "https://h/r", None, &only_all)
                .expect("valid")
                .proxy
                .as_deref(),
            Some("http://a:3")
        );
        assert_eq!(
            resolve("", "https://git.example.com/r", None, &only_all)
                .expect("valid")
                .proxy,
            None
        );
        assert_eq!(
            resolve(
                "",
                "https://h/r",
                None,
                &[("ALL_PROXY", "a:3"), ("no_proxy", "*")]
            )
            .expect("valid")
            .proxy,
            None
        );
    }

    #[test]
    fn rejects_unsupported_proxy_authentication() {
        let error = resolve(
            "[http]\n\tproxyAuthMethod = ntlm\n",
            "https://h/r",
            None,
            &[],
        )
        .err();
        assert_eq!(error.map(|e| e.problem()), Some(NetworkProblem::Proxy));
    }

    #[test]
    fn collects_extra_headers_and_empty_value_clears() {
        let config = "[http]\n\textraHeader = A: 1\n\textraHeader =\n\textraHeader = B: 2\n[http \"https://h\"]\n\textraHeader = C: 3\n[http \"https://x\"]\n\textraHeader = D: 4\n";
        let settings = resolve(config, "https://h/r", None, &[]).expect("valid");
        assert_eq!(settings.extra_headers, ["B: 2", "C: 3"]);
    }

    #[test]
    fn environment_overrides_tls_and_speed_options() {
        let config = "[http]\n\tsslCAInfo = /c.pem\n\tlowSpeedLimit = 10\n\tlowSpeedTime = 20\n";
        let env = [
            ("GIT_SSL_NO_VERIFY", "1"),
            ("GIT_SSL_CAINFO", "/e.pem"),
            ("GIT_HTTP_LOW_SPEED_TIME", "5"),
        ];
        let settings = resolve(config, "https://h/r", None, &env).expect("valid");
        assert!(!settings.ssl_verify);
        assert_eq!(settings.notes, [HttpNote::TlsVerificationDisabled]);
        assert_eq!(settings.ca_info, Some(PathBuf::from("/e.pem")));
        assert_eq!(
            (settings.low_speed_limit, settings.low_speed_time),
            (10, Duration::from_secs(5))
        );
    }

    #[test]
    fn parses_redirect_policy_and_reports_invalid_values() {
        let all = resolve(
            "[http]\n\tfollowRedirects = true\n",
            "https://h/r",
            None,
            &[],
        )
        .expect("valid");
        assert_eq!(all.follow_redirects, FollowRedirects::All);
        let none = resolve(
            "[http]\n\tfollowRedirects = false\n",
            "https://h/r",
            None,
            &[],
        )
        .expect("valid");
        assert_eq!(none.follow_redirects, FollowRedirects::None);
        assert!(
            resolve(
                "[http]\n\tfollowRedirects = sometimes\n",
                "https://h/r",
                None,
                &[]
            )
            .is_err()
        );
        assert!(resolve("[http]\n\tpostBuffer = lots\n", "https://h/r", None, &[]).is_err());
    }

    #[test]
    fn reports_options_without_effect() {
        let settings = resolve(
            "[http]\n\tsslCert = /c\n\tversion = HTTP/2\n",
            "https://h/r",
            None,
            &[],
        )
        .expect("valid");
        let keys: Vec<_> = settings
            .notes
            .iter()
            .filter_map(|n| match n {
                HttpNote::OptionIgnored { key } => Some(key.as_str()),
                HttpNote::TlsVerificationDisabled => None,
            })
            .collect();
        assert_eq!(keys, ["http.sslCert", "http.version"]);
    }

    #[test]
    fn bearer_token_is_hidden_from_debug() {
        let token = BearerToken::new("secret");
        assert_eq!(format!("{token:?}"), "BearerToken(<redacted>)");
        assert_eq!(token.expose(), "secret");
    }

    #[test]
    fn debug_output_hides_header_values_and_proxy_credentials() {
        let settings = HttpSettings {
            extra_headers: vec!["Authorization: Bearer hidden-value".to_owned()],
            proxy: Some("http://user:hidden-pass@proxy.example:3128".to_owned()),
            ..HttpSettings::default()
        };
        let text = format!("{settings:?}");
        assert!(!text.contains("hidden-value") && !text.contains("hidden-pass"));
        assert!(text.contains("proxy.example:3128"));
    }

    #[test]
    fn matches_hosts_against_no_proxy_lists() {
        assert!(no_proxy_matches(
            "localhost, .example.com",
            "git.example.com"
        ));
        assert!(no_proxy_matches("example.com", "EXAMPLE.com"));
        assert!(no_proxy_matches("::1", "[::1]"));
        assert!(!no_proxy_matches("example.com", "notexample.com"));
        assert!(!no_proxy_matches("", "example.com"));
    }

    #[test]
    fn parses_sizes_with_suffixes() {
        assert_eq!(parse_size("3k"), Some(3072));
        assert_eq!(parse_size("1G"), Some(1 << 30));
        assert_eq!(parse_size("x"), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
