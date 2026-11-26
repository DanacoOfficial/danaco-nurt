// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Request addresses: validation and normalization.

use std::net::IpAddr;

use nurt_contracts::error::{Error, ErrorCode, Result};
use url::{Host, Url};

/// Longest accepted address, in characters.
pub const MAX_URL_CHARS: usize = 8192;

/// Scheme added to an address written without one.
const ASSUMED_SCHEME: &str = "https://";

/// Kind of the host of an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostKind {
    /// A domain name.
    Name,
    /// An IPv4 address.
    Ipv4,
    /// An IPv6 address.
    Ipv6,
}

impl HostKind {
    /// Stable name: `name`, `ipv4` or `ipv6`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
        }
    }
}

/// An HTTP or HTTPS address after normalization.
///
/// Normalization follows the WHATWG URL standard: the scheme and host are
/// lower case, international domain names are converted to punycode, IPv4
/// addresses written in decimal, octal or hexadecimal notation become dotted
/// addresses, and the default port is removed from the text form. An address
/// without a scheme gets `https://`.
///
/// ```
/// use nurt_net::client::{HostKind, Target};
///
/// let target = Target::parse("http://0x7f.1/a").unwrap();
/// assert_eq!(target.host(), "127.0.0.1");
/// assert_eq!(target.host_kind(), HostKind::Ipv4);
/// assert_eq!(target.port(), 80);
///
/// let assumed = Target::parse("example.test/docs").unwrap();
/// assert!(assumed.scheme_assumed());
/// assert_eq!(assumed.as_str(), "https://example.test/docs");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    input: String,
    url: Url,
    scheme_assumed: bool,
}

impl Target {
    /// Validates and normalizes an address.
    ///
    /// # Errors
    ///
    /// `E_ARG` when the address is longer than [`MAX_URL_CHARS`], contains
    /// control characters, does not parse, has no host or uses a scheme other
    /// than `http` and `https`.
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed.chars().count() > MAX_URL_CHARS {
            return Err(arg_error(format!(
                "the address is longer than {MAX_URL_CHARS} characters"
            )));
        }
        if trimmed.chars().any(char::is_control) {
            return Err(arg_error(
                "the address contains control characters; remove them or percent-encode them",
            ));
        }
        let scheme_assumed = !has_scheme(trimmed);
        let text = if scheme_assumed {
            format!("{ASSUMED_SCHEME}{trimmed}")
        } else {
            trimmed.to_owned()
        };
        let url = Url::parse(&text)
            .map_err(|error| arg_error(format!("the address `{trimmed}` is not valid: {error}")))?;
        let target = Self {
            input: input.to_owned(),
            url,
            scheme_assumed,
        };
        target.validate()?;
        Ok(target)
    }

    /// Resolves a `Location` header against this address.
    ///
    /// # Errors
    ///
    /// `E_NETWORK` with problem `bad_location` when the location does not
    /// form a valid HTTP or HTTPS address.
    pub fn join(&self, location: &str) -> Result<Self> {
        let bad = |reason: String| {
            Error::new(
                ErrorCode::Network,
                format!("the redirect location `{location}` is not valid: {reason}"),
            )
            .with_problem("bad_location")
        };
        if location.chars().any(char::is_control) {
            return Err(bad("it contains control characters".to_owned()));
        }
        let url = self
            .url
            .join(location)
            .map_err(|error| bad(error.to_string()))?;
        let target = Self {
            input: url.as_str().to_owned(),
            url,
            scheme_assumed: false,
        };
        target
            .validate()
            .map_err(|error| bad(error.data().message.clone()))?;
        Ok(target)
    }

    fn validate(&self) -> Result<()> {
        if !matches!(self.url.scheme(), "http" | "https") {
            return Err(arg_error(format!(
                "the scheme `{}` is not supported; use http or https",
                self.url.scheme()
            )));
        }
        if self.url.host().is_none() {
            return Err(arg_error("the address has no host"));
        }
        Ok(())
    }

    /// The address as written by the caller.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// The normalized address.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The normalized address as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    /// Whether `https://` was added to an address without a scheme.
    #[must_use]
    pub fn scheme_assumed(&self) -> bool {
        self.scheme_assumed
    }

    /// Scheme, `http` or `https`.
    #[must_use]
    pub fn scheme(&self) -> &str {
        self.url.scheme()
    }

    /// Host: a punycode domain name, a dotted IPv4 address or an IPv6
    /// address without brackets.
    #[must_use]
    pub fn host(&self) -> String {
        match self.url.host() {
            Some(Host::Ipv6(address)) => address.to_string(),
            Some(Host::Ipv4(address)) => address.to_string(),
            Some(Host::Domain(name)) => name.to_owned(),
            None => String::new(),
        }
    }

    /// Kind of the host.
    #[must_use]
    pub fn host_kind(&self) -> HostKind {
        match self.url.host() {
            Some(Host::Ipv4(_)) => HostKind::Ipv4,
            Some(Host::Ipv6(_)) => HostKind::Ipv6,
            _ => HostKind::Name,
        }
    }

    /// The host as an IP address, when it is one.
    #[must_use]
    pub fn ip(&self) -> Option<IpAddr> {
        match self.url.host() {
            Some(Host::Ipv4(address)) => Some(IpAddr::V4(address)),
            Some(Host::Ipv6(address)) => Some(IpAddr::V6(address)),
            _ => None,
        }
    }

    /// Port, with the default port of the scheme when none is written.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.url
            .port_or_known_default()
            .unwrap_or(DEFAULT_HTTPS_PORT)
    }

    /// Whether the address carries a user name or a password.
    #[must_use]
    pub fn has_userinfo(&self) -> bool {
        !self.url.username().is_empty() || self.url.password().is_some()
    }

    /// Decoded user name and password, when the address carries them.
    #[must_use]
    pub fn credentials(&self) -> Option<(String, String)> {
        if !self.has_userinfo() {
            return None;
        }
        let decode = |text: &str| {
            percent_encoding::percent_decode_str(text)
                .decode_utf8_lossy()
                .into_owned()
        };
        Some((
            decode(self.url.username()),
            decode(self.url.password().unwrap_or_default()),
        ))
    }

    /// The address without user name and password, as sent on the wire.
    #[must_use]
    pub fn without_credentials(&self) -> Url {
        let mut url = self.url.clone();
        if self.has_userinfo() {
            let _ = url.set_username("");
            let _ = url.set_password(None);
        }
        url
    }

    /// The normalized address without user name and password, as shown in
    /// records, reports, messages and files; credentials are never echoed.
    #[must_use]
    pub fn display_url(&self) -> String {
        self.without_credentials().to_string()
    }

    /// `host:port` with brackets around an IPv6 host, as used for pinned
    /// addresses.
    #[must_use]
    pub fn authority(&self) -> String {
        match self.host_kind() {
            HostKind::Ipv6 => format!("[{}]:{}", self.host(), self.port()),
            _ => format!("{}:{}", self.host(), self.port()),
        }
    }

    /// Whether both addresses have the same scheme, host and port.
    #[must_use]
    pub fn same_origin(&self, other: &Self) -> bool {
        self.scheme() == other.scheme()
            && self.host() == other.host()
            && self.port() == other.port()
    }
}

/// Default port of HTTPS, used when the scheme gives no port.
const DEFAULT_HTTPS_PORT: u16 = 443;

fn has_scheme(text: &str) -> bool {
    let Some((scheme, _)) = text.split_once("://") else {
        return false;
    };
    let mut chars = scheme.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn arg_error(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::Arg, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_numeric_ipv4_notations() {
        for input in ["http://2130706433/", "http://0x7f.1/", "http://0177.0.0.1/"] {
            let target = Target::parse(input).unwrap();
            assert_eq!(target.host(), "127.0.0.1", "{input}");
            assert_eq!(target.host_kind(), HostKind::Ipv4);
        }
    }

    #[test]
    fn converts_international_names_to_punycode() {
        let target = Target::parse("https://b\u{fc}cher.example/a?b=1").unwrap();
        let host = target.host();
        assert!(
            host.starts_with("xn--") && host.ends_with(".example"),
            "{host}"
        );
        assert!(host.is_ascii());
        assert_eq!(target.host_kind(), HostKind::Name);
        assert_eq!(target.port(), 443);
    }

    #[test]
    fn keeps_ipv6_without_brackets() {
        let target = Target::parse("http://[::ffff:127.0.0.1]:8080/").unwrap();
        assert_eq!(target.host_kind(), HostKind::Ipv6);
        assert_eq!(target.host(), "::ffff:127.0.0.1");
        assert_eq!(target.authority(), "[::ffff:127.0.0.1]:8080");
    }

    #[test]
    fn assumes_https_without_scheme() {
        let target = Target::parse("localhost:8080/x").unwrap();
        assert!(target.scheme_assumed());
        assert_eq!(target.as_str(), "https://localhost:8080/x");
        assert_eq!(target.port(), 8080);
    }

    #[test]
    fn rejects_control_characters_and_other_schemes() {
        let control = Target::parse("http://a.test/\r\nx").unwrap_err();
        assert_eq!(control.code(), ErrorCode::Arg);
        let ftp = Target::parse("ftp://a.test/").unwrap_err();
        assert_eq!(ftp.code(), ErrorCode::Arg);
        let long = format!("http://a.test/{}", "x".repeat(MAX_URL_CHARS));
        assert_eq!(Target::parse(&long).unwrap_err().code(), ErrorCode::Arg);
    }

    #[test]
    fn reports_and_strips_credentials() {
        let target = Target::parse("http://user:p%40ss@a.test/").unwrap();
        assert!(target.has_userinfo());
        assert_eq!(
            target.credentials(),
            Some(("user".to_owned(), "p@ss".to_owned()))
        );
        assert_eq!(target.without_credentials().as_str(), "http://a.test/");
    }

    #[test]
    fn displayed_address_never_carries_credentials() {
        let target = Target::parse("http://user:p%40ss@a.test:8080/x?y=1#z").unwrap();
        let shown = target.display_url();
        assert_eq!(shown, "http://a.test:8080/x?y=1#z");
        assert!(!shown.contains("user") && !shown.contains("p%40ss"));
        let plain = Target::parse("https://a.test/x").unwrap();
        assert_eq!(plain.display_url(), plain.as_str());
        let joined = target.join("/other").unwrap();
        assert_eq!(joined.display_url(), "http://a.test:8080/other");
        assert!(joined.has_userinfo());
    }

    #[test]
    fn ipv6_literals_and_international_names_are_normalized_for_matching() {
        let target = Target::parse("http://[2001:DB8::1]:80/").unwrap();
        assert_eq!(target.host(), "2001:db8::1");
        assert_eq!(target.port(), 80);
        assert_eq!(target.as_str(), "http://[2001:db8::1]/");
        let upper = Target::parse("https://EXAMPLE.test/a").unwrap();
        assert_eq!(upper.host(), "example.test");
        assert!(Target::parse("https://example.test/").unwrap().same_origin(&upper));
        let other_scheme = Target::parse("http://example.test/").unwrap();
        let secure = Target::parse("https://example.test/").unwrap();
        assert!(!other_scheme.same_origin(&secure));
    }

    #[test]
    fn joins_relative_and_rejects_bad_locations() {
        let base = Target::parse("http://a.test/dir/page").unwrap();
        let next = base.join("../other").unwrap();
        assert_eq!(next.as_str(), "http://a.test/other");
        assert!(base.same_origin(&next));
        let error = base.join("mailto:x@a.test").unwrap_err();
        assert_eq!(error.code(), ErrorCode::Network);
        assert_eq!(error.data().problem.as_deref(), Some("bad_location"));
    }
}
