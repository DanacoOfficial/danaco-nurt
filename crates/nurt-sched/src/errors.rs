// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Errors reported by the schedule commands and the schedule module.

use std::path::Path;

use nurt_contracts::error::{Error, ErrorCode, Stage};

/// A value of the right type that is not valid for a schedule.
pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidValue, message).with_stage(Stage::Bind)
}

/// Parameters that exclude each other or are missing from a group.
pub(crate) fn conflict(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::Arg, message).with_stage(Stage::Bind)
}

/// A run of the schedule is in progress.
pub(crate) fn busy(schedule: &str, run: &str) -> Error {
    Error::new(
        ErrorCode::JobState,
        format!("schedule `{schedule}` is running; wait until run `{run}` ends"),
    )
    .with_actual(serde_json::Value::String(run.to_owned()))
}

/// Schedules that do not exist.
pub(crate) fn not_found(ids: &[String]) -> Error {
    let list = ids.join("`, `");
    Error::new(
        ErrorCode::NotFound,
        format!("no schedule `{list}`; list schedules with `schedule.list`"),
    )
    .with_actual(serde_json::Value::from(ids.to_vec()))
}

/// A file of the schedule store cannot be read or written.
pub(crate) fn io(path: &Path, error: &std::io::Error) -> Error {
    Error::new(ErrorCode::Io, format!("{}: {error}", path.display()))
}

/// A file of the schedule store does not have the expected form.
pub(crate) fn format(path: &Path, problem: impl std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::Format,
        format!("{} is not a valid schedule file: {problem}", path.display()),
    )
}

/// An internal step failed.
pub(crate) fn failed(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::StageFailed, message)
}
