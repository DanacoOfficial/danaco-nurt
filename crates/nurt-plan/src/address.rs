// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Network addresses written as literals: URLs and the `user@host:path`
//! form of SSH remotes.

use nurt_contracts::stage::NetworkTarget;

const SSH_SCHEME: &str = "ssh";
const HTTP_PORT: u16 = 80;
const HTTPS_PORT: u16 = 443;
const SSH_PORT: u16 = 22;
const GIT_PORT: u16 = 9418;
const FTP_PORT: u16 = 21;

fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" | "ws" => Some(HTTP_PORT),
        "https" | "wss" => Some(HTTPS_PORT),
        "ssh" | "sftp" | "git+ssh" => Some(SSH_PORT),
        "git" => Some(GIT_PORT),
        "ftp" => Some(FTP_PORT),
        _ => None,
    }
}

fn is_scheme(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Splits `host[:port]` or `[address][:port]`; `None` when the port is not a
/// number.
fn split_host_port(text: &str) -> Option<(String, Option<u16>)> {
    if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port.parse().ok()?),
            None if after.is_empty() => None,
            None => return None,
        };
        return Some((host.to_owned(), port));
    }
    match text.rsplit_once(':') {
        Some((host, port)) => Some((host.to_owned(), Some(port.parse().ok()?))),
        None => Some((text.to_owned(), None)),
    }
}

/// Removes the user information (`user:password@`) from an authority.
fn without_user(authority: &str) -> &str {
    authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
}

fn parse_url(scheme: &str, rest: &str) -> Option<NetworkTarget> {
    if !is_scheme(scheme) {
        return None;
    }
    let scheme = scheme.to_ascii_lowercase();
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let authority = without_user(authority);
    let (host, port) = split_host_port(authority)?;
    if host.is_empty() {
        return None;
    }
    let port = port.or_else(|| default_port(&scheme));
    Some(NetworkTarget {
        url: Some(format!("{scheme}://{authority}{tail}")),
        scheme,
        host: host.to_ascii_lowercase(),
        port,
    })
}

/// Parses the SSH form `[user@]host:path`; a single letter before `:` is a
/// Windows drive, not a host.
fn parse_scp(text: &str) -> Option<NetworkTarget> {
    let (authority, path) = text.split_once(':')?;
    if path.is_empty() || authority.contains(['/', '\\']) {
        return None;
    }
    let host = without_user(authority);
    if host.chars().count() < 2 || host.contains(char::is_whitespace) {
        return None;
    }
    Some(NetworkTarget {
        scheme: SSH_SCHEME.to_owned(),
        host: host.to_ascii_lowercase(),
        port: Some(SSH_PORT),
        url: None,
    })
}

/// Network target of an address literal; `None` when the text is not an
/// address, for example the name of a git remote.
///
/// User information is never kept: the host is reported without it and the
/// URL is rebuilt without it.
pub(crate) fn parse_address(text: &str) -> Option<NetworkTarget> {
    let text = text.trim();
    match text.split_once("://") {
        Some((scheme, rest)) => parse_url(scheme, rest),
        None => parse_scp(text),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests fail by panicking"
    )]

    use super::*;

    #[test]
    fn parses_url_with_port_and_drops_credentials() {
        let target = parse_address("https://user:secret@Example.com:8443/api?q=1").unwrap();
        assert_eq!(target.scheme, "https");
        assert_eq!(target.host, "example.com");
        assert_eq!(target.port, Some(8443));
        assert_eq!(
            target.url.as_deref(),
            Some("https://Example.com:8443/api?q=1")
        );
    }

    #[test]
    fn fills_default_port_of_known_schemes() {
        let cases = [
            ("http://example.org", Some(80)),
            ("https://example.org/a", Some(443)),
            ("ssh://git@example.org/r.git", Some(22)),
            ("git://example.org/r.git", Some(9418)),
            ("ftp://example.org/f", Some(21)),
            ("gopher://example.org", None),
        ];
        for (text, port) in cases {
            assert_eq!(parse_address(text).unwrap().port, port, "{text}");
        }
    }

    #[test]
    fn parses_bracketed_addresses() {
        let target = parse_address("http://[::1]:8080/health").unwrap();
        assert_eq!(target.host, "::1");
        assert_eq!(target.port, Some(8080));
        let bare = parse_address("https://[2001:db8::1]/").unwrap();
        assert_eq!(bare.host, "2001:db8::1");
        assert_eq!(bare.port, Some(443));
        assert!(parse_address("http://[::1]x/").is_none());
    }

    #[test]
    fn parses_ssh_remote_form() {
        let target = parse_address("git@github.com:org/repo.git").unwrap();
        assert_eq!(target.scheme, "ssh");
        assert_eq!(target.host, "github.com");
        assert_eq!(target.port, Some(22));
        assert!(target.url.is_none());
    }

    #[test]
    fn trims_the_text_and_lowercases_scheme_and_host() {
        let target = parse_address("  HTTP://Example.COM/a#frag  ").unwrap();
        assert_eq!(target.scheme, "http");
        assert_eq!(target.host, "example.com");
        assert_eq!(target.port, Some(80));
        assert_eq!(target.url.as_deref(), Some("http://Example.COM/a#frag"));
    }

    #[test]
    fn last_at_sign_of_the_authority_ends_the_user_information() {
        let target = parse_address("https://a@b:c@example.org/x@y").unwrap();
        assert_eq!(target.host, "example.org");
        assert_eq!(target.url.as_deref(), Some("https://example.org/x@y"));
    }

    #[test]
    fn query_without_a_path_ends_the_authority() {
        let target = parse_address("https://example.org?q=1").unwrap();
        assert_eq!(target.host, "example.org");
        assert_eq!(target.url.as_deref(), Some("https://example.org?q=1"));
    }

    #[test]
    fn multibyte_characters_do_not_break_the_split() {
        let target = parse_address("https://ŻÓŁW.example/ą").unwrap();
        assert_eq!(target.url.as_deref(), Some("https://ŻÓŁW.example/ą"));
        let remote = parse_address("żółw@host.example:ścieżka/ą.git").unwrap();
        assert_eq!(remote.host, "host.example");
        assert_eq!(remote.scheme, "ssh");
    }

    #[test]
    fn rejects_names_paths_and_malformed_urls() {
        for text in [
            "origin",
            "C:/work/repo",
            "/srv/repo",
            "./repo:x",
            "http://",
            "http://host:port/",
            "1http://host/",
            "host:",
            "my host:path",
        ] {
            assert!(parse_address(text).is_none(), "{text}");
        }
    }
}
