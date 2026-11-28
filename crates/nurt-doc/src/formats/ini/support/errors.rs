// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::ini::support::errors
// Responsibility:   Errors raised by the document formats.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Errors raised by the document formats.

use nurt_contracts::error::{Error, ErrorCode, Location};

use super::text::{LineIndex, to_u64};

/// Most key candidates listed in an error.
const MAX_CANDIDATES: usize = 8;

/// `E_DOC_PARSE` for a document of `format` that is malformed at `offset`.
pub(crate) fn parse_error(format: &str, message: &str, text: &str, offset: usize) -> Error {
    let position = LineIndex::new(text).position(text, offset);
    Error::new(
        ErrorCode::DocParse,
        format!(
            "{format} document is malformed at line {}: {message}",
            position.line
        ),
    )
    .with_location(Location {
        path: String::new(),
        line: Some(u64::from(position.line)),
        column: Some(u64::from(position.column)),
        end_line: None,
        end_column: None,
        byte: Some(to_u64(offset)),
    })
    .with_detail("format", serde_json::Value::from(format))
}

/// `E_UNSUPPORTED_CONSTRUCT` naming the construct that cannot be changed
/// without losing layout or meaning.
pub(crate) fn unsupported(construct: &str, message: &str) -> Error {
    Error::new(ErrorCode::UnsupportedConstruct, message).with_found(construct)
}

/// `E_KEY_NOT_FOUND` with the closest existing keys.
pub(crate) fn key_not_found(key: &str, candidates: &[String]) -> Error {
    candidates.iter().take(MAX_CANDIDATES).fold(
        Error::new(
            ErrorCode::KeyNotFound,
            format!("key `{key}` does not exist; use an existing key or `create: true`"),
        )
        .with_found(key),
        |error, candidate| error.with_sample(candidate.clone()),
    )
}

/// `E_KEY_CONFLICT` for a key path that does not fit the document, with the
/// kind of node found and the concrete keys to use instead.
pub(crate) fn key_conflict(message: &str, found: &str, candidates: &[String]) -> Error {
    candidates.iter().take(MAX_CANDIDATES).fold(
        Error::new(ErrorCode::KeyConflict, message).with_found(found),
        |error, candidate| error.with_sample(candidate.clone()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_errors_carry_the_position() {
        let error = parse_error("ini", "expected `]`", "a=1\n[b\n", 4);
        assert_eq!(error.code(), ErrorCode::DocParse);
        let location = error.location.clone().unwrap();
        assert_eq!((location.line, location.column), (Some(2), Some(1)));
        assert!(error.message.contains("line 2"));
    }

    #[test]
    fn key_errors_list_at_most_eight_candidates() {
        let candidates: Vec<String> = (0..10).map(|i| format!("k[{i}]")).collect();
        let error = key_conflict("ambiguous key", "repeated key", &candidates);
        assert_eq!(error.sample.len(), MAX_CANDIDATES);
        assert_eq!(key_not_found("a", &candidates).sample.len(), MAX_CANDIDATES);
        assert_eq!(unsupported("ini.x", "m").found.as_deref(), Some("ini.x"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
