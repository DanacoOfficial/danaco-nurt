// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Selection of the proxy server of a request.

use nurt_contracts::env::Host;
use nurt_contracts::error::{Error, ErrorCode, Result};
use url::Url;

use super::address::Target;

/// Proxy server requested for a request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ProxyChoice {
    /// The `http.proxy` setting, else the variables `HTTPS_PROXY`,
    /// `HTTP_PROXY` and `ALL_PROXY` of the engine process, filtered by
    /// `NO_PROXY`.
    #[default]
    System,
    /// A direct connection.
    None,
    /// The given proxy server.
    Url(String),
}

/// Proxy server used for `target`, or `None` for a direct connection.
///
/// Variables are read through `host`, in upper case first and then in lower
/// case. `NO_PROXY` holds comma-separated host names, domain suffixes
/// (`.example.test` or `example.test`, both matching subdomains), IP
/// addresses, optional `:port` suffixes and `*` for every host.
///
/// # Errors
///
/// `E_ARG` when the proxy address does not parse or its scheme is not `http`
/// or `https`.
///
/// ```
/// # use std::ffi::OsString;
/// # use std::path::PathBuf;
/// # use nurt_contracts::env::{Host, HostInfo, Os, PathStyle};
/// use nurt_net::client::{effective_proxy, ProxyChoice, Target};
/// # struct Env(HostInfo);
/// # impl Host for Env {
/// #     fn info(&self) -> &HostInfo { &self.0 }
/// #     fn env_var(&self, name: &str) -> Option<OsString> {
/// #         (name == "HTTPS_PROXY").then(|| OsString::from("proxy.test:3128"))
/// #     }
/// #     fn env_vars(&self) -> Vec<(OsString, OsString)> { Vec::new() }
/// # }
/// # let host = Env(HostInfo { os: Os::Linux, os_version: String::new(),
/// #     architecture: String::new(), hostname: String::new(), user: String::new(),
/// #     home: PathBuf::from("/"), temp: PathBuf::from("/tmp"), path_style: PathStyle::Posix,
/// #     timezone: "UTC".to_owned(), locale: None, cpus: 1 });
///
/// let target = Target::parse("https://example.test/").unwrap();
/// let proxy = effective_proxy(&ProxyChoice::System, None, &host, &target).unwrap();
/// assert_eq!(proxy.unwrap().as_str(), "http://proxy.test:3128/");
/// let direct = effective_proxy(&ProxyChoice::None, None, &host, &target).unwrap();
/// assert!(direct.is_none());
/// ```
pub fn effective_proxy(
    choice: &ProxyChoice,
    configured: Option<&str>,
    host: &dyn Host,
    target: &Target,
) -> Result<Option<Url>> {
    let text = match choice {
        ProxyChoice::None => return Ok(None),
        ProxyChoice::Url(text) => Some(text.clone()),
        ProxyChoice::System => system_proxy(configured, host, target),
    };
    text.filter(|text| !text.trim().is_empty())
        .map(|text| parse_proxy(&text))
        .transpose()
}

fn system_proxy(configured: Option<&str>, host: &dyn Host, target: &Target) -> Option<String> {
    let no_proxy = variable(host, "NO_PROXY").unwrap_or_default();
    if bypasses(&no_proxy, target) {
        return None;
    }
    if let Some(configured) = configured {
        return Some(configured.to_owned());
    }
    let scheme_variable = if target.scheme() == "https" {
        "HTTPS_PROXY"
    } else {
        "HTTP_PROXY"
    };
    variable(host, scheme_variable).or_else(|| variable(host, "ALL_PROXY"))
}

fn variable(host: &dyn Host, name: &str) -> Option<String> {
    host.env_var(name)
        .or_else(|| host.env_var(&name.to_ascii_lowercase()))
        .map(|value| value.to_string_lossy().trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn bypasses(no_proxy: &str, target: &Target) -> bool {
    let host = target.host();
    let port = target.port();
    no_proxy
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| entry_matches(entry, &host, port))
}

fn entry_matches(entry: &str, host: &str, port: u16) -> bool {
    if entry == "*" {
        return true;
    }
    let (name, entry_port) = split_port(entry);
    if entry_port.is_some_and(|entry_port| entry_port != port) {
        return false;
    }
    let name = name
        .trim_start_matches("*.")
        .trim_start_matches('.')
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    host == name || host.ends_with(&format!(".{name}"))
}

fn split_port(entry: &str) -> (&str, Option<u16>) {
    if entry.matches(':').count() > 1 && !entry.starts_with('[') {
        return (entry, None);
    }
    match entry.rsplit_once(':') {
        Some((name, port)) => match port.parse() {
            Ok(port) => (name, Some(port)),
            Err(_) => (entry, None),
        },
        None => (entry, None),
    }
}

fn parse_proxy(text: &str) -> Result<Url> {
    let text = text.trim();
    let with_scheme = if text.contains("://") {
        text.to_owned()
    } else {
        format!("http://{text}")
    };
    let url = Url::parse(&with_scheme).map_err(|error| {
        Error::new(
            ErrorCode::Arg,
            format!("the proxy address `{text}` is not valid: {error}"),
        )
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(Error::new(
            ErrorCode::Arg,
            format!("the proxy `{text}` is not supported; give an http:// or https:// proxy"),
        ));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(text: &str) -> Target {
        Target::parse(text).unwrap()
    }

    #[test]
    fn no_proxy_matches_suffixes_ports_and_wildcard() {
        let api = target("http://api.example.test:8080/");
        assert!(bypasses("example.test", &api));
        assert!(bypasses(".example.test", &api));
        assert!(bypasses("api.example.test:8080", &api));
        assert!(!bypasses("api.example.test:9090", &api));
        assert!(!bypasses("other.test", &api));
        assert!(bypasses("other.test, *", &api));
        assert!(bypasses("[::1]", &target("http://[::1]/")));
    }

    #[test]
    fn rejects_unsupported_proxy_schemes() {
        assert_eq!(
            parse_proxy("socks5://p.test:1080").unwrap_err().code(),
            ErrorCode::Arg
        );
        assert_eq!(parse_proxy("http://").unwrap_err().code(), ErrorCode::Arg);
        assert_eq!(
            parse_proxy("p.test:3128").unwrap().as_str(),
            "http://p.test:3128/"
        );
    }
}
