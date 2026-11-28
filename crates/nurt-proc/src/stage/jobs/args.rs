// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::stage::jobs::args
// Responsibility:   Reading the arguments and the input of the job commands.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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
                format!(
                    "`{}` needs a positive duration for `{name}`",
                    args.command()
                ),
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

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::{ArgValue, RecordStream};
    use nurt_contracts::value::{self, Record, RecordKind};

    use super::*;

    fn bound(items: Vec<(&str, Value)>) -> BoundArgs {
        let mut args = BoundArgs::new("jobs");
        for (name, value) in items {
            args.insert(name, ArgValue::Value(value), false);
        }
        args
    }

    fn millis(value: i64) -> Value {
        Value::Duration(value::Duration::from_millis(value))
    }

    #[test]
    fn a_duration_is_read_and_a_negative_or_foreign_value_is_refused() {
        assert_eq!(duration(&bound(Vec::new()), "wait").unwrap(), None);
        assert_eq!(
            duration(&bound(vec![("wait", Value::Null)]), "wait").unwrap(),
            None
        );
        assert_eq!(
            duration(&bound(vec![("wait", millis(1_500))]), "wait").unwrap(),
            Some(Duration::from_millis(1_500))
        );
        assert_eq!(
            duration(&bound(vec![("wait", millis(0))]), "wait").unwrap(),
            Some(Duration::ZERO)
        );
        let negative = duration(&bound(vec![("wait", millis(-1))]), "wait").unwrap_err();
        assert_eq!(negative.code(), ErrorCode::InvalidValue);
        let text = duration(&bound(vec![("wait", Value::Int(3))]), "wait").unwrap_err();
        assert_eq!(text.code(), ErrorCode::Type);
    }

    #[test]
    fn the_waiting_time_is_default_a_limit_or_forever() {
        assert_eq!(wait(&bound(Vec::new())).unwrap(), Wait::Default);
        assert_eq!(
            wait(&bound(vec![("wait", millis(250))])).unwrap(),
            Wait::For(Duration::from_millis(250))
        );
        for forever in [
            Value::Enum("forever".to_owned()),
            Value::Text("forever".to_owned()),
        ] {
            assert_eq!(
                wait(&bound(vec![("wait", forever)])).unwrap(),
                Wait::Forever
            );
        }
        let other = wait(&bound(vec![("wait", Value::Enum("never".to_owned()))])).unwrap_err();
        assert_eq!(other.code(), ErrorCode::Type);
    }

    #[test]
    fn a_word_is_an_enumeration_or_a_text() {
        let args = bound(vec![
            ("view", Value::Enum("output".to_owned())),
            ("id", Value::Text("j7".to_owned())),
            ("state", Value::Null),
            ("since", Value::Int(1)),
        ]);
        assert_eq!(word(&args, "view").unwrap(), Some("output"));
        assert_eq!(word(&args, "id").unwrap(), Some("j7"));
        assert_eq!(word(&args, "state").unwrap(), None);
        assert_eq!(word(&args, "missing").unwrap(), None);
        assert_eq!(word(&args, "since").unwrap_err().code(), ErrorCode::Type);
    }

    #[test]
    fn a_count_is_a_number_from_zero_that_fits_32_bits() {
        let count_of = |value: Value| count(&bound(vec![("head", value)]), "head");
        assert_eq!(count(&bound(Vec::new()), "head").unwrap(), None);
        assert_eq!(count_of(Value::Null).unwrap(), None);
        assert_eq!(count_of(Value::Int(0)).unwrap(), Some(0));
        assert_eq!(
            count_of(Value::Int(i64::from(u32::MAX))).unwrap(),
            Some(u32::MAX)
        );
        for number in [-1, i64::from(u32::MAX) + 1] {
            let error = count_of(Value::Int(number)).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidValue, "{number}");
        }
        assert_eq!(
            count_of(Value::Text("5".to_owned())).unwrap_err().code(),
            ErrorCode::Type
        );
    }

    #[test]
    fn a_pattern_keeps_its_flags_as_an_inline_group() {
        let pattern_of = |value: Value| pattern(&bound(vec![("filter", value)]), "filter");
        assert_eq!(pattern(&bound(Vec::new()), "filter").unwrap(), None);
        assert_eq!(pattern_of(Value::Null).unwrap(), None);
        assert_eq!(
            pattern_of(Value::Text("err".to_owned())).unwrap(),
            Some("err".to_owned())
        );
        let plain = Value::Regex(value::Regex::new("a+", "").unwrap());
        assert_eq!(pattern_of(plain).unwrap(), Some("a+".to_owned()));
        let flagged = Value::Regex(value::Regex::new("a+", "i").unwrap());
        assert_eq!(pattern_of(flagged).unwrap(), Some("(?i)a+".to_owned()));
        assert_eq!(
            pattern_of(Value::Int(1)).unwrap_err().code(),
            ErrorCode::Type
        );
    }

    #[test]
    fn a_flag_is_false_when_absent() {
        assert!(!flag(&bound(Vec::new()), "dedupe").unwrap());
        assert!(flag(&bound(vec![("dedupe", Value::Bool(true))]), "dedupe").unwrap());
        assert!(!flag(&bound(vec![("dedupe", Value::Bool(false))]), "dedupe").unwrap());
        let error = flag(
            &bound(vec![("dedupe", Value::Text("yes".to_owned()))]),
            "dedupe",
        );
        assert_eq!(error.unwrap_err().code(), ErrorCode::Type);
    }

    #[test]
    fn job_records_are_read_from_the_input_and_other_records_are_refused() {
        let job = |id: &str| {
            Record::from(Job {
                id: id.to_owned(),
                ..Job::default()
            })
        };
        let input = StageInput::records(RecordStream::from_records(vec![job("j1"), job("j2")]));
        let ids: Vec<String> = input_jobs(input)
            .unwrap()
            .into_iter()
            .map(|job| job.id)
            .collect();
        assert_eq!(ids, ["j1", "j2"]);
        let empty = StageInput::records(RecordStream::from_records(Vec::new()));
        assert!(input_jobs(empty).unwrap().is_empty());
        let entry = StageInput::records(RecordStream::once(Record::new(RecordKind::Entry)));
        assert!(input_jobs(entry).is_err());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
