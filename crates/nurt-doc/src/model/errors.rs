// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use nurt_contracts::error::{Candidate, Error, ErrorCode};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::DocFormatId;

use super::LineIndex;

/// Most candidates attached to a key error.
pub(crate) const MAX_CANDIDATES: usize = 8;

/// `E_DOC_PARSE`: `text` is not a valid document of `format`.
///
/// `offset` is the byte where the problem was detected; its line and column
/// are added to the message and to the details `line`, `column` and `byte`.
///
/// ```
/// use nurt_contracts::error::ErrorCode;
/// use nurt_contracts::services::DocFormatId;
/// use nurt_doc::model::doc_parse;
///
/// let error = doc_parse(&DocFormatId::JSON, b"{\n  1}", Some(4), "expected a member name");
/// assert_eq!(error.code, ErrorCode::DocParse);
/// assert_eq!(error.message, "invalid json document at line 2, column 3: expected a member name");
/// ```
#[must_use]
pub fn doc_parse(
    format: &DocFormatId,
    text: &[u8],
    offset: Option<usize>,
    message: impl AsRef<str>,
) -> Error {
    let message = message.as_ref();
    let Some(offset) = offset else {
        return Error::new(
            ErrorCode::DocParse,
            format!("invalid {format} document: {message}"),
        )
        .with_detail("format", format.as_str().into());
    };
    let position = LineIndex::new(text).position(text, offset);
    Error::new(
        ErrorCode::DocParse,
        format!(
            "invalid {format} document at line {}, column {}: {message}",
            position.line, position.column
        ),
    )
    .with_detail("format", format.as_str().into())
    .with_detail("line", position.line.into())
    .with_detail("column", position.column.into())
    .with_detail("byte", offset.into())
}

/// `E_KEY_CONFLICT`: `key` passes through a node of kind `found` where the
/// path needs another kind; `message` says what was expected.
#[must_use]
pub fn key_conflict(key: &KeyPath, found: &str, message: impl AsRef<str>) -> Error {
    Error::new(
        ErrorCode::KeyConflict,
        format!("key `{key}` {}", message.as_ref()),
    )
    .with_found(found)
    .with_detail("key", key.to_string().into())
}

/// `E_KEY_CONFLICT`: `key` matches several members and needs an index or a
/// wildcard; `candidates` are the concrete paths of the members.
#[must_use]
pub fn ambiguous_key(key: &KeyPath, candidates: &[KeyPath]) -> Error {
    let mut error = Error::new(
        ErrorCode::KeyConflict,
        format!(
            "key `{key}` matches {} members; select one by index or all with `[*]`",
            candidates.len()
        ),
    )
    .with_found("repeated")
    .with_detail("key", key.to_string().into());
    for (rank, candidate) in candidates.iter().take(MAX_CANDIDATES).enumerate() {
        error = error.with_candidate(candidate_of(rank, candidate));
    }
    error
}

/// `E_KEY_NOT_FOUND`: `key` selects nothing; `candidates` are existing keys
/// close to it, best first.
#[must_use]
pub fn key_not_found(key: &KeyPath, candidates: &[KeyPath]) -> Error {
    let mut error = Error::new(
        ErrorCode::KeyNotFound,
        format!("key `{key}` does not exist; check the key or set `create: true`"),
    )
    .with_detail("key", key.to_string().into());
    for (rank, candidate) in candidates.iter().take(MAX_CANDIDATES).enumerate() {
        error = error.with_candidate(candidate_of(rank, candidate));
    }
    error
}

fn candidate_of(rank: usize, key: &KeyPath) -> Candidate {
    Candidate {
        rank: u32::try_from(rank + 1).unwrap_or(u32::MAX),
        reference: None,
        program: key.to_string(),
        why: None,
    }
}
