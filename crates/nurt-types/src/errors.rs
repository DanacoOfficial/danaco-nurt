// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-types::errors
// Responsibility:   Construction of the language errors raised by this crate.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Construction of the language errors raised by this crate.

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::value::{Value, ValueError};
use serde_json::json;

/// Types accepted where text is read: text and the text-like types.
pub(crate) const TEXT_TYPES: &[&str] = &["text", "path", "glob", "key", "enum"];

/// `E_TYPE`: `context` accepts the `expected` types and received `found`.
pub(crate) fn type_error(context: &str, expected: &[&str], found: &Value) -> Error {
    Error::new(
        ErrorCode::Type,
        format!(
            "{context} expects {}, found {}",
            type_list(expected),
            found.type_name()
        ),
    )
    .with_expected(json!(expected))
    .with_actual(json!(found.type_name()))
}

fn type_list(types: &[&str]) -> String {
    match types {
        [] => "a value".to_owned(),
        [only] => (*only).to_owned(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}

/// `E_INVALID_VALUE` with the problem `overflow`: the result of `operation`
/// does not fit the type.
pub(crate) fn overflow(operation: &str, type_name: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("the result of {operation} is outside the range of {type_name}"),
    )
    .with_problem("overflow")
}

/// `E_INVALID_VALUE` with the problem `division_by_zero`.
pub(crate) fn division_by_zero() -> Error {
    Error::new(ErrorCode::InvalidValue, "division by zero").with_problem("division_by_zero")
}

/// `E_INVALID_VALUE` for a text that does not form a value of its type.
pub(crate) fn invalid_value(error: &ValueError) -> Error {
    Error::new(ErrorCode::InvalidValue, error.to_string())
        .with_expected(json!([error.type_name]))
        .with_found(error.input.clone())
}

/// `E_INVALID_VALUE` for an argument outside its admissible values.
pub(crate) fn invalid_argument(function: &str, reason: &str) -> Error {
    Error::new(ErrorCode::InvalidValue, format!("`{function}`: {reason}")).with_help("functions")
}

/// `E_ARITY`: `function` takes `expected` arguments and received `found`.
pub(crate) fn arity(function: &str, expected: &str, found: usize) -> Error {
    Error::new(
        ErrorCode::Arity,
        format!("`{function}` takes {expected}, found {found}"),
    )
    .with_expected(json!(expected))
    .with_actual(json!(found))
    .with_help("functions")
}

/// `E_UNKNOWN_FIELD` with the fields that exist; `record_kind` names the
/// kind of the record when known.
pub(crate) fn unknown_field<'a>(
    name: &str,
    record_kind: Option<&str>,
    available: impl Iterator<Item = &'a str>,
) -> Error {
    let fields: Vec<&str> = available.collect();
    let message = match record_kind {
        Some(kind) => format!("unknown field `{name}` of {kind} records"),
        None => format!("unknown field `{name}`"),
    };
    Error::new(ErrorCode::UnknownField, message)
        .with_found(name)
        .with_expected(json!(fields))
}

/// `E_INTERNAL` for a node or operator this version does not evaluate.
pub(crate) fn unsupported(what: &str) -> Error {
    Error::new(
        ErrorCode::Internal,
        format!("{what} is not supported by this version"),
    )
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
