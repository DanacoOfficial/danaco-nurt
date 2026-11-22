// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::spec::CatalogIssue;
use serde_json::json;

const ISSUES_DETAIL: &str = "issues";

/// Error for a catalog that failed loading or a registration that failed
/// validation; every problem is listed in the `issues` detail.
pub(crate) fn invalid(subject: &str, issues: &[CatalogIssue]) -> Error {
    let first = issues
        .first()
        .map_or_else(|| "no details".to_owned(), ToString::to_string);
    let message = if issues.len() > 1 {
        format!(
            "{subject} is invalid: {first} (and {} more)",
            issues.len() - 1
        )
    } else {
        format!("{subject} is invalid: {first}")
    };
    let listed = serde_json::to_value(issues).unwrap_or_default();
    Error::new(ErrorCode::Catalog, message).with_detail(ISSUES_DETAIL, listed)
}

/// Error for a catalog source that cannot be read or parsed.
pub(crate) fn catalog_message(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::Catalog, message)
}

/// Error for a command name that is not in the catalog.
pub(crate) fn unknown_command(name: &str) -> Error {
    Error::new(
        ErrorCode::UnknownCommand,
        format!("unknown command `{name}`"),
    )
    .with_found(name)
}

/// Error for a group name that is not in the catalog.
pub(crate) fn unknown_group(name: &str) -> Error {
    Error::new(ErrorCode::UnknownName, format!("unknown group `{name}`")).with_found(name)
}

/// Error for a command that has no implementation for the requested use.
pub(crate) fn unsupported_input(command: &str, found: Option<&str>, accepted: &[String]) -> Error {
    let list = if accepted.is_empty() {
        "none".to_owned()
    } else {
        accepted.join(", ")
    };
    let (message, found) = match found {
        Some(kind) => (
            format!("`{command}` does not accept `{kind}` records, accepted kinds: {list}"),
            kind,
        ),
        None => (
            format!("`{command}` cannot start a pipeline, accepted input kinds: {list}"),
            "none",
        ),
    };
    Error::new(ErrorCode::Type, message)
        .with_found(found)
        .with_expected(json!(accepted))
}

/// Error for a command that exists in the catalog but has no implementation.
pub(crate) fn not_implemented(command: &str) -> Error {
    Error::new(
        ErrorCode::Internal,
        format!("command `{command}` has no registered implementation"),
    )
}
