// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading the arguments and the input of the job commands.

use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::Job;
use nurt_contracts::stage::{BoundArgs, StageInput};
use nurt_contracts::value::Value;

/// Waiting time of a run: the default of the command, a duration, or until
/// the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wait {
    /// Not given.
    Default,
    /// Wait at most this long.
    For(Duration),
    /// `forever`: wait until the program ends.
    Forever,
}

/// Duration argument `name`.
///
/// # Errors
///
/// Fails with `E_TYPE` for another type and `E_INVALID_VALUE` for a
/// negative duration.
pub(crate) fn duration(args: &BoundArgs, name: &str) -> Result<Option<Duration>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Duration(value)) => value.to_std().map(Some).ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("`{}` needs a positive duration for `{name}`", args.command()),
            )
        }),
        Some(other) => Err(type_error(args, name, "duration", other)),
    }
}

/// Argument `wait` of type `duration | enum(forever)`.
///
/// # Errors
///
/// Fails with `E_TYPE` for another type.
pub(crate) fn wait(args: &BoundArgs) -> Result<Wait> {
    match args.value("wait") {
        Some(Value::Enum(text) | Value::Text(text)) if text == "forever" => Ok(Wait::Forever),
        _ => Ok(duration(args, "wait")?.map_or(Wait::Default, Wait::For)),
    }
}

/// Enumeration or text argument `name`.
///
/// # Errors
///
/// Fails with `E_TYPE` for another type.
pub(crate) fn word<'a>(args: &'a BoundArgs, name: &str) -> Result<Option<&'a str>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Enum(text) | Value::Text(text)) => Ok(Some(text.as_str())),
        Some(other) => Err(type_error(args, name, "text", other)),
    }
}

/// Non-negative integer argument `name`.
///
/// # Errors
///
/// Fails with `E_TYPE` for another type and `E_INVALID_VALUE` for a
/// negative number.
pub(crate) fn count(args: &BoundArgs, name: &str) -> Result<Option<u32>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Int(number)) => u32::try_from(*number).map(Some).map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("`{}` needs a number from 0 for `{name}`", args.command()),
            )
            .with_found(number.to_string())
        }),
        Some(other) => Err(type_error(args, name, "int", other)),
    }
}

/// Regular expression argument `name` as a pattern with inline flags.
///
/// # Errors
///
/// Fails with `E_TYPE` for another type.
pub(crate) fn pattern(args: &BoundArgs, name: &str) -> Result<Option<String>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Regex(regex)) if regex.flags.is_empty() => Ok(Some(regex.pattern.clone())),
        Some(Value::Regex(regex)) => Ok(Some(format!("(?{}){}", regex.flags, regex.pattern))),
        Some(Value::Text(text)) => Ok(Some(text.clone())),
        Some(other) => Err(type_error(args, name, "regex", other)),
    }
}

/// Boolean argument `name`, `false` when absent.
///
/// # Errors
///
/// Fails with `E_TYPE` for another type.
pub(crate) fn flag(args: &BoundArgs, name: &str) -> Result<bool> {
    Ok(args.bool(name)?.unwrap_or(false))
}

/// Job records of the input.
///
/// # Errors
///
/// Fails with `E_TYPE` when the input is not a stream of job records and
/// with the error of the stream.
pub(crate) fn input_jobs(input: StageInput<'_>) -> Result<Vec<Job>> {
    input
        .into_records()?
        .map(|record| Job::try_from(record?).map_err(Error::from))
        .collect()
}

fn type_error(args: &BoundArgs, name: &str, expected: &str, found: &Value) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{}` expects {expected} for `{name}`", args.command()),
    )
    .with_found(found.type_name())
}
