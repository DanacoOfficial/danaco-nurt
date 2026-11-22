// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Language errors of `sql`.

use std::io;

use nurt_contracts::error::{Error, ErrorCode, Location};
use nurt_contracts::value::Value;
use rusqlite::ErrorCode as SqliteCode;
use serde_json::json;

use super::SqlRequest;
use super::guard::Denial;

/// Part of the call a failure belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Site {
    /// Opening and configuring the connection.
    Open,
    /// One statement, numbered from 1.
    Statement(usize),
    /// Opening, committing or checkpointing the transaction of a change.
    Transaction,
}

impl Site {
    fn describe(self) -> String {
        match self {
            Self::Open => "opening the database".to_owned(),
            Self::Statement(number) => format!("statement {number}"),
            Self::Transaction => "the transaction".to_owned(),
        }
    }
}

/// Converts a failure reported by SQLite.
pub(super) fn from_sqlite(error: &rusqlite::Error, request: &SqlRequest, site: Site) -> Error {
    match error {
        rusqlite::Error::SqliteFailure(failure, message) => {
            let text = message.clone().unwrap_or_else(|| failure.to_string());
            from_failure(failure, &text, request, site)
        }
        rusqlite::Error::SqlInputError {
            error: failure,
            msg,
            ..
        } => from_failure(failure, msg, request, site),
        other => Error::new(
            ErrorCode::Sql,
            format!(
                "{} failed: {}",
                site.describe(),
                lower_first(&other.to_string())
            ),
        ),
    }
}

fn from_failure(
    failure: &rusqlite::ffi::Error,
    text: &str,
    request: &SqlRequest,
    site: Site,
) -> Error {
    let display = &request.display;
    let error = match failure.code {
        SqliteCode::DatabaseBusy | SqliteCode::DatabaseLocked => Error::new(
            ErrorCode::InUse,
            format!(
                "{display} is locked by another connection; retry later or raise `sql.busy_timeout`"
            ),
        ),
        SqliteCode::NotADatabase | SqliteCode::DatabaseCorrupt => doc_parse(request, text),
        SqliteCode::ReadOnly if !request.write => read_only(request, site),
        SqliteCode::ReadOnly | SqliteCode::PermissionDenied => Error::new(
            ErrorCode::Permission,
            format!(
                "{display} cannot be written: {text}; check the permissions of the file and its directory"
            ),
        ),
        SqliteCode::CannotOpen => Error::new(
            ErrorCode::Io,
            format!("{display} cannot be opened: {text}; check the path and its permissions"),
        ),
        SqliteCode::DiskFull => Error::new(
            ErrorCode::NoSpace,
            format!("the volume of {display} is full; free space and run the statements again"),
        ),
        SqliteCode::SystemIoFailure => Error::new(
            ErrorCode::Io,
            format!("input or output error on {display}: {text}"),
        ),
        SqliteCode::OperationInterrupted => Error::new(
            ErrorCode::Cancelled,
            format!("{} was interrupted", site.describe()),
        ),
        _ => Error::new(
            ErrorCode::Sql,
            format!("{} failed: {text}", site.describe()),
        ),
    };
    let error = error
        .with_detail("path", json!(display))
        .with_detail("sqlite_code", json!(failure.extended_code));
    match site {
        Site::Statement(number) => error.with_detail("statement", json!(number)),
        Site::Open | Site::Transaction => error,
    }
}

/// The file is not an SQLite database, or its content is damaged.
pub(super) fn doc_parse(request: &SqlRequest, reason: &str) -> Error {
    Error::new(
        ErrorCode::DocParse,
        format!(
            "{} is not a valid SQLite database: {reason}; pass a database file that is not encrypted or damaged",
            request.display
        ),
    )
    .with_location(Location {
        path: request.display.clone(),
        line: None,
        column: None,
        end_line: None,
        end_column: None,
        byte: None,
    })
    .with_detail("format", json!("sqlite"))
}

/// Failure to access the database file before SQLite opens it.
pub(super) fn file_error(error: &io::Error, request: &SqlRequest) -> Error {
    let display = &request.display;
    let (code, message) = match error.kind() {
        io::ErrorKind::NotFound => (
            ErrorCode::NotFound,
            format!("database file {display} does not exist; check the path"),
        ),
        io::ErrorKind::PermissionDenied => (
            ErrorCode::Permission,
            format!("database file {display} cannot be read; check its permissions"),
        ),
        _ => (
            ErrorCode::Io,
            format!(
                "database file {display} cannot be read: {}",
                lower_first(&error.to_string())
            ),
        ),
    };
    Error::new(code, message).with_detail("path", json!(display))
}

/// A statement changes the database while `write` is false.
pub(super) fn read_only(request: &SqlRequest, site: Site) -> Error {
    let error = Error::new(
        ErrorCode::Sql,
        format!(
            "{} changes the database; add `write: true` to allow changes",
            site.describe()
        ),
    )
    .with_problem("write_required");
    match write_fix(request) {
        Some(fix) => error.with_fix(fix),
        None => error,
    }
}

/// A statement refused by the lexical checks.
pub(super) fn denied(denial: &Denial, number: usize) -> Error {
    let (problem, message) = match denial {
        Denial::Attach(keyword) => (
            "attach",
            format!(
                "statement {number} uses {keyword}; `sql` works on one database file, so run the statements on each file separately"
            ),
        ),
        Denial::VacuumInto => (
            "vacuum_into",
            format!(
                "statement {number} uses VACUUM INTO, which writes another file; copy the database file instead"
            ),
        ),
        Denial::Pragma(name) => (
            "pragma",
            format!(
                "statement {number} sets `{name}`, which points SQLite to files outside the database; remove the pragma"
            ),
        ),
        Denial::Transaction(keyword) => (
            "transaction",
            format!(
                "statement {number} uses {keyword}; with `write: true` all statements run in one transaction, so use SAVEPOINT for nested transactions"
            ),
        ),
    };
    Error::new(ErrorCode::Sql, message)
        .with_problem(problem)
        .with_detail("statement", json!(number))
}

/// The text of a prepared statement cannot be read back for the checks.
pub(super) fn unreadable(number: usize) -> Error {
    Error::new(
        ErrorCode::Sql,
        format!("statement {number} is too long to be checked; split it into shorter statements"),
    )
    .with_problem("statement_length")
    .with_detail("statement", json!(number))
}

/// `params` holds fewer values than the parameters of the statements.
pub(super) fn missing_params(number: usize, bound: usize) -> Error {
    Error::new(
        ErrorCode::Sql,
        format!(
            "statement {number} has more parameters than `params` provides after {bound} values; pass one value per parameter in statement order"
        ),
    )
    .with_problem("parameter_count")
    .with_detail("statement", json!(number))
}

/// `params` holds more values than the parameters of the statements.
pub(super) fn extra_params(used: usize, given: usize) -> Error {
    Error::new(
        ErrorCode::Sql,
        format!(
            "the statements have {used} parameters but `params` holds {given} values; remove the extra values"
        ),
    )
    .with_problem("parameter_count")
    .with_expected(json!(used))
    .with_actual(json!(given))
}

/// The program that repeats the call with `write: true`, when every
/// parameter value has a literal form.
fn write_fix(request: &SqlRequest) -> Option<String> {
    let mut program = format!("sql {} {}", quote(&request.display), quote(&request.query));
    if !request.params.is_empty() {
        let values = request
            .params
            .iter()
            .map(literal)
            .collect::<Option<Vec<_>>>()?;
        program.push_str(&format!(" params: [{}]", values.join(", ")));
    }
    program.push_str(" write: true");
    Some(program)
}

fn literal(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("null".to_owned()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Int(integer) => Some(integer.to_string()),
        Value::BigInt(digits) | Value::Decimal(digits) => Some(digits.clone()),
        Value::Number(number) if number.is_finite() => Some(format!("{number:?}")),
        Value::Text(text) => Some(quote(text)),
        _ => None,
    }
}

/// Text literal of the language: `"…"` with the escapes `\\`, `\"`, `\n`,
/// `\r`, `\t`, `\0` and `\u{HEX}` for other control characters.
fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\0' => quoted.push_str("\\0"),
            control if control.is_control() => {
                quoted.push_str(&format!("\\u{{{:X}}}", u32::from(control)));
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn request(params: Vec<Value>) -> SqlRequest {
        SqlRequest {
            path: "/home/user/app.db".into(),
            display: "/home/user/app.db".to_owned(),
            query: "insert into t values (?)\n".to_owned(),
            params,
            write: false,
            busy_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn fix_repeats_the_call_with_write() {
        let fix = write_fix(&request(vec![
            Value::Int(1),
            Value::Text("a\"b\u{1}".to_owned()),
            Value::Null,
            Value::Bool(true),
            Value::Number(1.5),
        ]));
        assert_eq!(
            fix.as_deref(),
            Some(
                "sql \"/home/user/app.db\" \"insert into t values (?)\\n\" \
                 params: [1, \"a\\\"b\\u{1}\", null, true, 1.5] write: true"
            )
        );
    }

    #[test]
    fn fix_is_absent_for_values_without_literal() {
        let error = read_only(&request(vec![Value::List(Vec::new())]), Site::Statement(1));
        assert!(error.fix.is_empty());
        assert_eq!(error.problem.as_deref(), Some("write_required"));
    }

    #[test]
    fn maps_io_errors_by_kind() {
        let request = request(Vec::new());
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(file_error(&denied, &request).code(), ErrorCode::Permission);
        let other = io::Error::other("Broken");
        let error = file_error(&other, &request);
        assert_eq!(error.code(), ErrorCode::Io);
        assert!(error.message.ends_with("broken"));
    }

    #[test]
    fn maps_sqlite_codes() {
        let mut writing = request(Vec::new());
        writing.write = true;
        let cases = [
            (SqliteCode::DiskFull, ErrorCode::NoSpace),
            (SqliteCode::SystemIoFailure, ErrorCode::Io),
            (SqliteCode::CannotOpen, ErrorCode::Io),
            (SqliteCode::ReadOnly, ErrorCode::Permission),
            (SqliteCode::OperationInterrupted, ErrorCode::Cancelled),
            (SqliteCode::ConstraintViolation, ErrorCode::Sql),
        ];
        for (code, expected) in cases {
            let failure = rusqlite::ffi::Error {
                code,
                extended_code: 0,
            };
            let error = rusqlite::Error::SqliteFailure(failure, None);
            assert_eq!(
                from_sqlite(&error, &writing, Site::Transaction).code(),
                expected
            );
        }
        let other = from_sqlite(&rusqlite::Error::InvalidQuery, &writing, Site::Open);
        assert_eq!(other.code(), ErrorCode::Sql);
    }
}
