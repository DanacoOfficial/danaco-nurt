// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::sql::names
// Responsibility:   Database names of `ATTACH` and `VACUUM INTO`: plain paths and `file:` URIs, and their resolution like every path argument.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Database names of `ATTACH` and `VACUUM INTO`: plain paths and `file:`
//! URIs, and their resolution like every path argument.
//!
//! The bundled SQLite is built with URI names enabled, so `file:/srv/x.db`
//! names the file `/srv/x.db`. A relative path is resolved against the
//! `base` of the call, and without a base it fails with `E_PATH_RELATIVE`,
//! as every path argument does.

use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;

/// What a database name refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DatabaseName {
    /// A temporary or in-memory database: no file.
    Memory,
    /// A file, as a path or as a `file:` URI.
    File {
        /// Path as written, decoded.
        path: String,
        /// `true` for a `file:` URI.
        uri: bool,
        /// Query of the URI without the question mark.
        query: Option<String>,
    },
    /// A URI whose file cannot be told.
    Unknown,
}

/// Classifies the name that `ATTACH` or `VACUUM INTO` receives.
pub(super) fn database_name(name: &str) -> DatabaseName {
    database_name_on(name, cfg!(windows))
}

/// [`database_name`] for a system with or without drive letters.
fn database_name_on(name: &str, windows: bool) -> DatabaseName {
    if name.is_empty() || name == ":memory:" {
        return DatabaseName::Memory;
    }
    let Some(rest) = name.strip_prefix("file:") else {
        return DatabaseName::File {
            path: name.to_owned(),
            uri: false,
            query: None,
        };
    };
    let rest = rest.split('#').next().unwrap_or_default();
    let (location, query) = match rest.split_once('?') {
        Some((location, query)) => (location, Some(query.to_owned())),
        None => (rest, None),
    };
    let path = match location.strip_prefix("//") {
        Some(authority_and_path) => match authority_and_path.find('/') {
            Some(index) => {
                let authority = &authority_and_path[..index];
                if !authority.is_empty() && authority != "localhost" {
                    return DatabaseName::Unknown;
                }
                &authority_and_path[index..]
            }
            None => return DatabaseName::Unknown,
        },
        None => location,
    };
    let Some(path) = percent_decode(path) else {
        return DatabaseName::Unknown;
    };
    let path = if windows {
        without_slash_before_drive(&path).to_owned()
    } else {
        path
    };
    let in_memory = query
        .as_deref()
        .is_some_and(|query| query.split('&').any(|pair| pair == "mode=memory"));
    if path.is_empty() || path == ":memory:" || in_memory {
        return DatabaseName::Memory;
    }
    DatabaseName::File {
        path,
        uri: true,
        query,
    }
}

/// `/C:/db/a.db` of a URI is the path `C:/db/a.db` on Windows, as SQLite
/// reads it.
fn without_slash_before_drive(path: &str) -> &str {
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && bytes.get(3).is_none_or(|next| matches!(next, b'/' | b'\\'))
    {
        &path[1..]
    } else {
        path
    }
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let digits = text.get(index + 1..index + 3)?;
            decoded.push(u8::from_str_radix(digits, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '%' => encoded.push_str("%25"),
            '?' => encoded.push_str("%3f"),
            '#' => encoded.push_str("%23"),
            other => encoded.push(other),
        }
    }
    encoded
}

/// Absolute form of `path`: a relative path is joined to `base`.
///
/// # Errors
///
/// Fails with `E_PATH_RELATIVE` for a relative path without a base, and with
/// `E_INVALID_VALUE` for an empty path or a path above the root.
pub(super) fn absolute(base: Option<&Path>, path: &str) -> Result<PathBuf> {
    if path.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "a path cannot be empty: give a file",
        ));
    }
    let written = PathBuf::from(path);
    let joined = if written.is_absolute() {
        written
    } else {
        let Some(base) = base else {
            return Err(Error::new(
                ErrorCode::PathRelative,
                format!(
                    "relative path `{path}` needs a base directory: give an absolute path or set the `base` option"
                ),
            )
            .with_found(path));
        };
        base.join(written)
    };
    AbsPath::new(joined)
        .map(|absolute| absolute.as_path().to_path_buf())
        .map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("path `{path}` leads above the root directory"),
            )
            .with_found(path)
        })
}

/// The name that SQLite receives for the resolved file: the plain path, or
/// the URI with the same query.
pub(super) fn written(file: &Path, uri: bool, query: Option<&str>) -> String {
    written_on(file, uri, query, cfg!(windows))
}

/// [`written`] for a system with or without drive letters.
fn written_on(file: &Path, uri: bool, query: Option<&str>, windows: bool) -> String {
    let text = file.to_string_lossy();
    if !uri {
        return text.into_owned();
    }
    let text = if windows {
        let slashes = text.replace('\\', "/");
        if slashes.as_bytes().get(1) == Some(&b':') {
            format!("/{slashes}")
        } else if slashes.starts_with("//") {
            // A share: `file:////host/share/a.db`, because the authority of a
            // URI must stay empty.
            format!("//{slashes}")
        } else {
            slashes
        }
    } else {
        text.into_owned()
    };
    let mut name = format!("file:{}", percent_encode(&text));
    if let Some(query) = query {
        name.push('?');
        name.push_str(query);
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_names() {
        assert_eq!(database_name(""), DatabaseName::Memory);
        assert_eq!(database_name(":memory:"), DatabaseName::Memory);
        assert_eq!(
            database_name("file::memory:?cache=shared"),
            DatabaseName::Memory
        );
        assert_eq!(database_name("file:x.db?mode=memory"), DatabaseName::Memory);
        assert_eq!(
            database_name("/srv/a.db"),
            DatabaseName::File {
                path: "/srv/a.db".to_owned(),
                uri: false,
                query: None
            }
        );
        assert_eq!(
            database_name("file:///srv/a%20b.db?mode=ro#frag"),
            DatabaseName::File {
                path: "/srv/a b.db".to_owned(),
                uri: true,
                query: Some("mode=ro".to_owned())
            }
        );
        assert_eq!(
            database_name("file:rel.db"),
            DatabaseName::File {
                path: "rel.db".to_owned(),
                uri: true,
                query: None
            }
        );
        assert_eq!(database_name("file://host/x.db"), DatabaseName::Unknown);
        assert_eq!(database_name("file:/x%zz.db"), DatabaseName::Unknown);
    }

    #[test]
    fn resolves_like_a_path_argument() {
        let base = std::env::temp_dir();
        assert_eq!(
            absolute(Some(&base), "a/../b.db").unwrap(),
            base.join("b.db")
        );
        let error = absolute(None, "b.db").unwrap_err();
        assert_eq!(error.code(), ErrorCode::PathRelative);
        assert_eq!(
            absolute(None, "").unwrap_err().code(),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn reads_the_drive_letter_of_a_windows_uri() {
        let on = |name| database_name_on(name, true);
        for (name, expected) in [
            ("file:///C:/db/a.db", "C:/db/a.db"),
            ("file:/c:/db/a.db", "c:/db/a.db"),
            ("file:///C:", "C:"),
            ("file:///C:%5Cdb%5Ca.db", "C:\\db\\a.db"),
            ("file:///srv/a.db", "/srv/a.db"),
            ("file:///Cx/a.db", "/Cx/a.db"),
        ] {
            assert_eq!(
                on(name),
                DatabaseName::File {
                    path: expected.to_owned(),
                    uri: true,
                    query: None
                },
                "{name}"
            );
        }
        assert_eq!(
            database_name_on("file:///C:/db/a.db", false),
            DatabaseName::File {
                path: "/C:/db/a.db".to_owned(),
                uri: true,
                query: None
            }
        );
    }

    #[test]
    fn writes_a_windows_uri_with_forward_slashes() {
        assert_eq!(
            written_on(Path::new("C:\\db\\a b.db"), true, Some("mode=ro"), true),
            "file:/C:/db/a b.db?mode=ro"
        );
        assert_eq!(
            written_on(Path::new("C:\\db\\a.db"), false, None, true),
            "C:\\db\\a.db"
        );
    }

    #[test]
    fn writes_a_share_with_an_empty_authority() {
        assert_eq!(
            written_on(Path::new("\\\\srv\\share\\a.db"), true, None, true),
            "file:////srv/share/a.db"
        );
    }

    #[test]
    fn writes_the_resolved_name() {
        let file = Path::new("/srv/a?b.db");
        assert_eq!(written(file, false, None), "/srv/a?b.db");
        assert_eq!(
            written(file, true, Some("mode=ro")),
            "file:/srv/a%3fb.db?mode=ro"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
