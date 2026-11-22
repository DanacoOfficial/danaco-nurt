// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The file format of the `store` credential helper.
//!
//! Each line is an address `<protocol>://<user>:<password>@<host>[/<path>]`
//! with user name, password and path percent-encoded. A lookup returns the
//! first line whose fields equal every field of the request; fields absent
//! from the request match any value. Storing writes the new line first and
//! keeps the other lines except those for the same protocol, host, path and
//! user; erasing removes the matching lines. Lines that are not such
//! addresses are kept unchanged.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use super::Query;

/// Characters left as they are in user names and passwords: the unreserved
/// characters of RFC 3986.
const USERINFO: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Characters left as they are in paths: the reserved and unreserved
/// characters of RFC 3986.
const PATH: &AsciiSet = &USERINFO
    .remove(b':')
    .remove(b'/')
    .remove(b'?')
    .remove(b'#')
    .remove(b'[')
    .remove(b']')
    .remove(b'@')
    .remove(b'!')
    .remove(b'$')
    .remove(b'&')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b';')
    .remove(b'=');

/// One credential line.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) protocol: String,
    pub(crate) host: String,
    pub(crate) path: Option<String>,
    pub(crate) username: String,
    pub(crate) password: String,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("protocol", &self.protocol)
            .field("host", &self.host)
            .field("path", &self.path)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

fn decode(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

/// Parses a line; `None` for lines without user name and password.
pub(crate) fn parse_line(line: &str) -> Option<Entry> {
    let (protocol, rest) = line.split_once("://")?;
    let (authority, path) = match rest.find('/') {
        Some(slash) => (&rest[..slash], Some(&rest[slash + 1..])),
        None => (rest, None),
    };
    let (userinfo, host) = authority.rsplit_once('@')?;
    let (username, password) = userinfo.split_once(':')?;
    if protocol.is_empty() || host.is_empty() {
        return None;
    }
    Some(Entry {
        protocol: protocol.to_owned(),
        host: decode(host),
        path: path.filter(|p| !p.is_empty()).map(decode),
        username: decode(username),
        password: decode(password),
    })
}

/// Formats `entry` as a line without the line feed.
pub(crate) fn render(entry: &Entry) -> String {
    let mut line = format!(
        "{}://{}:{}@{}",
        entry.protocol,
        utf8_percent_encode(&entry.username, USERINFO),
        utf8_percent_encode(&entry.password, USERINFO),
        entry.host
    );
    if let Some(path) = &entry.path {
        line.push('/');
        line.extend(utf8_percent_encode(path, PATH));
    }
    line
}

/// Whether `entry` satisfies every field present in `query`, and the
/// password when `password` is given.
pub(crate) fn matches(query: &Query, entry: &Entry, password: Option<&str>) -> bool {
    query.protocol == entry.protocol
        && query.host == entry.host
        && query
            .path
            .as_ref()
            .is_none_or(|path| entry.path.as_ref() == Some(path))
        && query
            .username
            .as_ref()
            .is_none_or(|user| *user == entry.username)
        && password.is_none_or(|password| password == entry.password)
}

/// First entry of `content` matching `query`.
pub(crate) fn find(content: &str, query: &Query) -> Option<Entry> {
    content
        .lines()
        .filter_map(parse_line)
        .find(|entry| matches(query, entry, None))
}

fn rewrite(content: &str, first: Option<String>, drop: impl Fn(&Entry) -> bool) -> String {
    let mut out = String::with_capacity(content.len() + first.as_ref().map_or(0, String::len) + 1);
    if let Some(line) = first {
        out.push_str(&line);
        out.push('\n');
    }
    for line in content.lines() {
        if parse_line(line).is_some_and(|entry| drop(&entry)) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Content after storing `entry` into `content`.
pub(crate) fn store(content: &str, entry: &Entry) -> String {
    let query = Query {
        protocol: entry.protocol.clone(),
        host: entry.host.clone(),
        path: entry.path.clone(),
        username: Some(entry.username.clone()),
    };
    rewrite(content, Some(render(entry)), |existing| {
        matches(&query, existing, None)
    })
}

/// Content after erasing the entries of `content` matching `query` and, when
/// given, `password`.
pub(crate) fn erase(content: &str, query: &Query, password: Option<&str>) -> String {
    rewrite(content, None, |existing| matches(query, existing, password))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(host: &str, path: Option<&str>, user: Option<&str>) -> Query {
        Query {
            protocol: "https".to_owned(),
            host: host.to_owned(),
            path: path.map(str::to_owned),
            username: user.map(str::to_owned),
        }
    }

    fn entry(host: &str, user: &str, password: &str) -> Entry {
        Entry {
            protocol: "https".to_owned(),
            host: host.to_owned(),
            path: None,
            username: user.to_owned(),
            password: password.to_owned(),
        }
    }

    #[test]
    fn parses_and_renders_percent_encoded_lines() {
        let line = "https://a%40b:p%3Aw%2F@example.com:8443/org/my%20repo.git";
        let parsed = parse_line(line).expect("credential line");
        assert_eq!(parsed.username, "a@b");
        assert_eq!(parsed.password, "p:w/");
        assert_eq!(parsed.host, "example.com:8443");
        assert_eq!(parsed.path.as_deref(), Some("org/my repo.git"));
        assert_eq!(render(&parsed), line);
    }

    #[test]
    fn rejects_lines_without_user_and_password() {
        assert!(parse_line("https://example.com").is_none());
        assert!(parse_line("https://bob@example.com").is_none());
        assert!(parse_line("# comment").is_none());
    }

    #[test]
    fn absent_request_fields_match_any_value() {
        let content = "https://bob:1@example.com/org/r\nhttps://alice:2@example.com\n";
        assert_eq!(
            find(content, &query("example.com", None, None)).map(|e| e.password),
            Some("1".to_owned())
        );
        assert_eq!(
            find(content, &query("example.com", None, Some("alice"))).map(|e| e.password),
            Some("2".to_owned())
        );
        assert_eq!(
            find(content, &query("example.com", Some("org/x"), None)),
            None
        );
        assert_eq!(find(content, &query("other.com", None, None)), None);
    }

    #[test]
    fn store_puts_the_entry_first_and_replaces_the_same_user() {
        let content = "garbage line\nhttps://bob:old@example.com\nhttps://alice:a@example.com\n";
        let updated = store(content, &entry("example.com", "bob", "new"));
        assert_eq!(
            updated,
            "https://bob:new@example.com\ngarbage line\nhttps://alice:a@example.com\n"
        );
        assert_eq!(
            store(&updated, &entry("example.com", "bob", "new")),
            updated
        );
    }

    #[test]
    fn erase_respects_the_password_when_given() {
        let content = "https://bob:1@example.com\nhttps://bob:2@other.com\n";
        let q = query("example.com", None, Some("bob"));
        assert_eq!(erase(content, &q, Some("9")), content);
        assert_eq!(erase(content, &q, Some("1")), "https://bob:2@other.com\n");
        assert_eq!(
            erase(content, &query("other.com", None, None), None),
            "https://bob:1@example.com\n"
        );
    }

    #[test]
    fn debug_output_hides_the_password() {
        assert!(!format!("{:?}", entry("h", "u", "hidden-value")).contains("hidden-value"));
    }
}
