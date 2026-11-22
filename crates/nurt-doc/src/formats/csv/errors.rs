// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Errors of the delimited text formats.

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::KeyPath;

/// Input that is not UTF-8 text.
pub(crate) fn not_utf8(valid_up_to: usize) -> Error {
    Error::new(
        ErrorCode::Encoding,
        format!("delimited file is not valid UTF-8 at byte {valid_up_to}; convert it to UTF-8"),
    )
}

/// Key path that the table does not contain.
pub(crate) fn key_not_found(key: &KeyPath, problem: &str) -> Error {
    Error::new(
        ErrorCode::KeyNotFound,
        format!("key {key} not found: {problem}"),
    )
    .with_found(key.to_string())
}

/// Key path segment that has no meaning in a table.
pub(crate) fn key_shape(key: &KeyPath, problem: &str) -> Error {
    Error::new(
        ErrorCode::KeyConflict,
        format!("key {key} does not address a table cell: {problem}"),
    )
    .with_found(key.to_string())
}

/// Value that cannot be stored at the addressed node.
pub(crate) fn value_type(key: &KeyPath, expected: &str, found: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("value of type {found} cannot be stored at {key}; pass {expected}"),
    )
}
