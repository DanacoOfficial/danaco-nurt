// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Allowed signers and revoked keys of SSH signature verification.
//!
//! An allowed signers line has the form `principals [options] key`. The
//! principals are a comma-separated list, optionally in double quotes. The
//! options are a comma-separated list of `cert-authority`,
//! `namespaces="<patterns>"`, `valid-after="<time>"` and
//! `valid-before="<time>"`; a time is `YYYYMMDD[HHMM[SS]]` in local time, or
//! in UTC when followed by `Z`. A line with an unknown or malformed option
//! is not used, and `cert-authority` lines name certificate authorities,
//! which plain signature keys never match. Empty lines and lines starting
//! with `#` are comments.
//!
//! A revoked keys file lists one public key per line; a binary key
//! revocation list (KRL) is not evaluated. A configured file that does not
//! exist accepts no signer and revokes no key.

use std::path::Path;

use nurt_contracts::error::Result;
use ssh_key::PublicKey;

use super::NAMESPACE;
use crate::sign::error::io_error;

/// Start of the magic number of a binary key revocation list (KRL).
const KRL_MAGIC: &[u8] = b"SSHKRL\n";
const DATE_DIGITS: usize = 8;
const MINUTE_DIGITS: usize = 12;
const SECOND_DIGITS: usize = 14;

/// Principals of the first allowed signers entry in `path` that accepts
/// `public` for the namespace `git` at `signed_at` (seconds since the Unix
/// epoch; `None` skips the validity period). Local times in the file are
/// `utc_offset` seconds east of UTC.
pub(super) fn find_principal(
    path: &Path,
    public: &PublicKey,
    signed_at: Option<i64>,
    utc_offset: i32,
) -> Result<Option<String>> {
    let Some(text) = read(path)? else {
        return Ok(None);
    };
    Ok(text
        .lines()
        .filter_map(|line| parse_line(line, utc_offset))
        .find(|entry| entry.accepts(public, signed_at))
        .map(|entry| entry.principals))
}

/// Whether the revoked keys file `path` is a binary key revocation list,
/// which is not evaluated: its keys would be taken for valid.
pub(super) fn is_krl(path: &Path) -> Result<bool> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(bytes.starts_with(KRL_MAGIC)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io_error(path, &source)),
    }
}

/// Whether the revoked keys file `path` lists `public`.
pub(super) fn is_revoked(path: &Path, public: &PublicKey) -> Result<bool> {
    let Some(text) = read(path)? else {
        return Ok(false);
    };
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| PublicKey::from_openssh(line).ok())
        .any(|key| key.key_data() == public.key_data()))
}

fn read(path: &Path) -> Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error(path, &source)),
    }
}

/// One line of an allowed signers file.
#[derive(Debug)]
struct Entry {
    principals: String,
    options: Options,
    key: PublicKey,
}

#[derive(Debug, Default)]
struct Options {
    cert_authority: bool,
    namespaces: Option<String>,
    valid_after: Option<i64>,
    valid_before: Option<i64>,
}

impl Options {
    fn parse(text: &str, utc_offset: i32) -> Option<Self> {
        let mut options = Self::default();
        for option in split_unquoted(text, ',') {
            let (name, value) = option
                .split_once('=')
                .map_or((option, None), |(name, value)| (name, Some(unquote(value))));
            match (name.to_ascii_lowercase().as_str(), value) {
                ("cert-authority", None) => options.cert_authority = true,
                ("namespaces", Some(value)) => options.namespaces = Some(value.to_owned()),
                ("valid-after", Some(value)) => {
                    options.valid_after = Some(parse_time(value, utc_offset)?)
                }
                ("valid-before", Some(value)) => {
                    options.valid_before = Some(parse_time(value, utc_offset)?)
                }
                _ => return None,
            }
        }
        Some(options)
    }
}

impl Entry {
    fn accepts(&self, public: &PublicKey, signed_at: Option<i64>) -> bool {
        if self.options.cert_authority || self.key.key_data() != public.key_data() {
            return false;
        }
        if let Some(namespaces) = &self.options.namespaces
            && !matches_pattern_list(NAMESPACE, namespaces)
        {
            return false;
        }
        signed_at.is_none_or(|at| {
            self.options.valid_after.is_none_or(|after| at >= after)
                && self.options.valid_before.is_none_or(|before| at <= before)
        })
    }
}

fn parse_line(line: &str, utc_offset: i32) -> Option<Entry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (principals, rest) = next_token(line)?;
    let principals = unquote(principals).to_owned();
    let rest = rest.trim_start();
    if let Ok(key) = PublicKey::from_openssh(rest) {
        return Some(Entry {
            principals,
            options: Options::default(),
            key,
        });
    }
    let (options, key) = next_token(rest)?;
    Some(Entry {
        principals,
        options: Options::parse(options, utc_offset)?,
        key: PublicKey::from_openssh(key.trim_start()).ok()?,
    })
}

/// Splits the first token, which ends at white space outside double
/// quotes, off `text`.
fn next_token(text: &str) -> Option<(&str, &str)> {
    if text.is_empty() {
        return None;
    }
    let mut quoted = false;
    for (index, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => return Some((&text[..index], &text[index..])),
            _ => {}
        }
    }
    Some((text, ""))
}

/// Splits `text` at `separator` outside double quotes.
fn split_unquoted(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (index, c) in text.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == separator && !quoted {
            parts.push(&text[start..index]);
            start = index + c.len_utf8();
        }
    }
    parts.push(&text[start..]);
    parts
}

fn unquote(text: &str) -> &str {
    text.strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(text)
}

/// Whether `name` matches the comma-separated `patterns` with the wildcards
/// `*` and `?`; a pattern starting with `!` that matches rejects the name.
fn matches_pattern_list(name: &str, patterns: &str) -> bool {
    let mut matched = false;
    for pattern in patterns.split(',').map(str::trim) {
        if let Some(negated) = pattern.strip_prefix('!') {
            if glob(name.as_bytes(), negated.as_bytes()) {
                return false;
            }
        } else if glob(name.as_bytes(), pattern.as_bytes()) {
            matched = true;
        }
    }
    matched
}

fn glob(text: &[u8], pattern: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => (0..=text.len()).any(|skip| glob(&text[skip..], rest)),
        Some((b'?', rest)) => !text.is_empty() && glob(&text[1..], rest),
        Some((byte, rest)) => text.first() == Some(byte) && glob(&text[1..], rest),
    }
}

/// Seconds since the Unix epoch of a `YYYYMMDD[HHMM[SS]][Z]` time.
fn parse_time(text: &str, utc_offset: i32) -> Option<i64> {
    let (digits, utc) = text
        .strip_suffix(['Z', 'z'])
        .map_or((text, false), |digits| (digits, true));
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let field = |start: usize, end: usize| digits.get(start..end)?.parse::<i16>().ok();
    let small =
        |start: usize, end: usize| field(start, end).and_then(|value| i8::try_from(value).ok());
    let (hour, minute, second) = match digits.len() {
        DATE_DIGITS => (0, 0, 0),
        MINUTE_DIGITS => (small(8, 10)?, small(10, 12)?, 0),
        SECOND_DIGITS => (small(8, 10)?, small(10, 12)?, small(12, 14)?),
        _ => return None,
    };
    let datetime = jiff::civil::DateTime::new(
        field(0, 4)?,
        small(4, 6)?,
        small(6, 8)?,
        hour,
        minute,
        second,
        0,
    )
    .ok()?;
    let seconds = jiff::tz::TimeZone::UTC
        .to_timestamp(datetime)
        .ok()?
        .as_second();
    Some(if utc {
        seconds
    } else {
        seconds - i64::from(utc_offset)
    })
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{PUBLIC, private_key};
    use super::*;
    use ssh_key::PrivateKey;
    use ssh_key::private::{Ed25519Keypair, KeypairData};

    /// 2026-01-01T00:00:00Z.
    const AT: i64 = 1_767_225_600;
    const HOUR: i32 = 3600;

    fn other_public() -> String {
        PrivateKey::new(
            KeypairData::from(Ed25519Keypair::from_seed(&[8; 32])),
            "other@example.test",
        )
        .expect("key")
        .public_key()
        .to_openssh()
        .expect("encode")
    }

    fn find(content: &str, at: Option<i64>, offset: i32) -> Option<String> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowed_signers");
        std::fs::write(&path, content).expect("write");
        find_principal(&path, private_key().public_key(), at, offset).expect("read")
    }

    #[test]
    fn finds_principals_of_first_matching_key() {
        let content = format!(
            "# team keys\n\nother@example.test {}\n\"a@example.test,b@example.test\" {PUBLIC}\nlate@example.test {PUBLIC}\n",
            other_public()
        );
        assert_eq!(
            find(&content, Some(AT), 0).as_deref(),
            Some("a@example.test,b@example.test")
        );
        assert_eq!(
            find(
                &format!("other@example.test {}\n", other_public()),
                Some(AT),
                0
            ),
            None
        );
    }

    #[test]
    fn applies_namespace_patterns() {
        for (namespaces, accepted) in [
            ("git", true),
            ("file,git", true),
            ("g?t", true),
            ("*", true),
            ("!git,*", false),
            ("file", false),
        ] {
            let content = format!("tester@example.test namespaces=\"{namespaces}\" {PUBLIC}\n");
            assert_eq!(
                find(&content, Some(AT), 0).is_some(),
                accepted,
                "{namespaces}"
            );
        }
    }

    #[test]
    fn applies_validity_period_in_utc_and_local_time() {
        let line = |options: &str| format!("tester@example.test {options} {PUBLIC}\n");
        assert!(find(&line("valid-after=\"20260101\""), Some(AT), HOUR).is_some());
        assert!(find(&line("valid-after=\"20260101010001Z\""), Some(AT), 0).is_none());
        assert!(find(&line("valid-before=\"20251231Z\""), Some(AT), 0).is_none());
        assert!(find(&line("valid-before=\"20251231Z\""), None, 0).is_some());
        assert!(
            find(
                &line("namespaces=\"git\",valid-before=\"202601010000Z\""),
                Some(AT),
                0
            )
            .is_some()
        );
    }

    #[test]
    fn skips_certificate_authorities_and_malformed_lines() {
        let content = format!(
            "ca@example.test cert-authority {PUBLIC}\nbad@example.test unknown-option {PUBLIC}\ndate@example.test valid-after=\"2026\" {PUBLIC}\nlonely\n"
        );
        assert_eq!(find(&content, Some(AT), 0), None);
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn treats_missing_files_as_empty_and_unreadable_ones_as_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let public = private_key().public_key().clone();
        let absent = dir.path().join("absent");
        assert_eq!(
            find_principal(&absent, &public, None, 0).expect("missing"),
            None
        );
        assert!(!is_revoked(&absent, &public).expect("missing"));
        let error = find_principal(dir.path(), &public, None, 0).expect_err("directory");
        assert_eq!(error.code().as_str(), "E_IO");
    }

    #[test]
    fn finds_revoked_keys() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("revoked");
        let public = private_key().public_key().clone();
        std::fs::write(&path, format!("# revoked\n{}\n", other_public())).expect("write");
        assert!(!is_revoked(&path, &public).expect("read"));
        std::fs::write(&path, format!("not a key\n{PUBLIC}\n")).expect("write");
        assert!(is_revoked(&path, &public).expect("read"));
    }

    #[test]
    fn parses_allowed_signer_times() {
        assert_eq!(parse_time("20260101", 0), Some(AT));
        assert_eq!(parse_time("20260101Z", HOUR), Some(AT));
        assert_eq!(parse_time("20260101", HOUR), Some(AT - i64::from(HOUR)));
        assert_eq!(parse_time("202601010130Z", 0), Some(AT + 5400));
        assert_eq!(parse_time("20260101000059z", 0), Some(AT + 59));
        assert_eq!(parse_time("20261301", 0), None);
        assert_eq!(parse_time("2026010", 0), None);
        assert_eq!(parse_time("2026-01-01", 0), None);
    }

    #[test]
    fn splits_tokens_outside_quotes() {
        assert_eq!(next_token("\"a b\" rest"), Some(("\"a b\"", " rest")));
        assert_eq!(next_token("single"), Some(("single", "")));
        assert_eq!(next_token(""), None);
        assert_eq!(split_unquoted("a,\"b,c\",d", ','), ["a", "\"b,c\"", "d"]);
        assert_eq!(unquote("\"x\""), "x");
        assert_eq!(unquote("\"x"), "\"x");
    }
}
