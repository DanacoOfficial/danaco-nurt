// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::revwalk::args
// Responsibility:   Arguments of the history commands read from bound arguments.
// Dependencies:     bstr, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Arguments of the history commands read from bound arguments.

use bstr::BString;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::stage::BoundArgs;
use nurt_contracts::value::{Range, Regex, RegexFlags, Value};

/// Error for an argument whose value cannot be used.
pub(crate) fn invalid(args: &BoundArgs, name: &str, message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidValue, message).with_found(format!("{} {name}:", args.command()))
}

fn type_error(args: &BoundArgs, name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{}` expects {expected} for `{name}`", args.command()),
    )
}

fn path_text(value: &Value) -> Option<&str> {
    match value {
        Value::Path(path) | Value::Text(path) => Some(path),
        _ => None,
    }
}

/// A path argument.
pub(crate) fn path(args: &BoundArgs, name: &str) -> Result<Option<String>, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => path_text(value)
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| type_error(args, name, "a path")),
    }
}

/// A path argument that must be given.
pub(crate) fn required_path(args: &BoundArgs, name: &str) -> Result<String, Error> {
    path(args, name)?.ok_or_else(|| {
        Error::new(
            ErrorCode::MissingParam,
            format!("`{}` needs a value for `{name}`", args.command()),
        )
    })
}

/// A list of paths; a single path is a list of one.
pub(crate) fn paths(args: &BoundArgs, name: &str) -> Result<Vec<String>, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| {
                path_text(item)
                    .map(str::to_owned)
                    .ok_or_else(|| type_error(args, name, "a list of paths"))
            })
            .collect(),
        Some(value) => path_text(value)
            .map(|text| vec![text.to_owned()])
            .ok_or_else(|| type_error(args, name, "a list of paths")),
    }
}

/// A list of texts; a single text is a list of one.
pub(crate) fn texts(args: &BoundArgs, name: &str) -> Result<Vec<String>, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| match item {
                Value::Text(text) => Ok(text.clone()),
                _ => Err(type_error(args, name, "a list of texts")),
            })
            .collect(),
        Some(Value::Text(text)) => Ok(vec![text.clone()]),
        Some(_) => Err(type_error(args, name, "a list of texts")),
    }
}

/// A list of glob patterns; a single pattern is a list of one.
pub(crate) fn globs(args: &BoundArgs, name: &str) -> Result<Vec<BString>, Error> {
    let pattern = |value: &Value| match value {
        Value::Glob(text) | Value::Text(text) => Ok(BString::from(text.as_str())),
        _ => Err(type_error(args, name, "a list of glob patterns")),
    };
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items.iter().map(pattern).collect(),
        Some(value) => Ok(vec![pattern(value)?]),
    }
}

/// A text argument.
pub(crate) fn text(args: &BoundArgs, name: &str) -> Result<Option<String>, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Text(text) | Value::Enum(text)) => Ok(Some(text.clone())),
        Some(_) => Err(type_error(args, name, "a text")),
    }
}

/// A boolean argument, `false` when absent.
pub(crate) fn flag(args: &BoundArgs, name: &str) -> Result<bool, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(_) => Err(type_error(args, name, "a boolean")),
    }
}

/// A boolean argument, `default` when absent.
pub(crate) fn flag_or(args: &BoundArgs, name: &str, default: bool) -> Result<bool, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(_) => Err(type_error(args, name, "a boolean")),
    }
}

/// A count: an integer that is 0 or more.
pub(crate) fn count(args: &BoundArgs, name: &str) -> Result<Option<u64>, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Int(number)) => u64::try_from(*number).map(Some).map_err(|_| {
            invalid(
                args,
                name,
                format!("`{name}` must be 0 or more, not {number}"),
            )
        }),
        Some(_) => Err(type_error(args, name, "an integer")),
    }
}

/// An instant as whole seconds since the Unix epoch, rounded down.
pub(crate) fn seconds(args: &BoundArgs, name: &str) -> Result<Option<i64>, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Instant(instant)) => {
            Ok(Some(instant.as_unix_nanos().div_euclid(1_000_000_000)))
        }
        Some(_) => Err(type_error(args, name, "an instant")),
    }
}

/// An inclusive line range.
pub(crate) fn range(args: &BoundArgs, name: &str) -> Result<Option<Range>, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Range(range)) => Ok(Some(*range)),
        Some(Value::Int(line)) => Ok(Some(Range::single(*line))),
        Some(_) => Err(type_error(args, name, "a range")),
    }
}

/// Pattern of a regular expression argument; a text is a pattern without
/// flags.
pub(crate) fn pattern(args: &BoundArgs, name: &str) -> Result<Option<Regex>, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Regex(regex)) => Ok(Some(regex.clone())),
        Some(Value::Text(text)) => Ok(Some(Regex {
            pattern: text.clone(),
            flags: RegexFlags::default(),
        })),
        Some(_) => Err(type_error(args, name, "a regular expression")),
    }
}

/// A regular expression for text.
pub(crate) fn regex(args: &BoundArgs, name: &str) -> Result<Option<regex::Regex>, Error> {
    let Some(source) = pattern(args, name)? else {
        return Ok(None);
    };
    regex::RegexBuilder::new(&source.pattern)
        .case_insensitive(source.flags.case_insensitive)
        .multi_line(source.flags.multi_line)
        .dot_matches_new_line(source.flags.dot_matches_new_line)
        .ignore_whitespace(source.flags.extended)
        .build()
        .map(Some)
        .map_err(|err| regex_error(args, name, &err))
}

/// A regular expression for bytes.
pub(crate) fn bytes_regex(
    args: &BoundArgs,
    name: &str,
    source: &Regex,
) -> Result<regex::bytes::Regex, Error> {
    regex::bytes::RegexBuilder::new(&source.pattern)
        .case_insensitive(source.flags.case_insensitive)
        .multi_line(source.flags.multi_line)
        .dot_matches_new_line(source.flags.dot_matches_new_line)
        .ignore_whitespace(source.flags.extended)
        .build()
        .map_err(|err| regex_error(args, name, &err))
}

fn regex_error(args: &BoundArgs, name: &str, err: &regex::Error) -> Error {
    let detail = err.to_string();
    let last = detail.lines().last().unwrap_or_default().trim().to_owned();
    Error::new(
        ErrorCode::Regex,
        format!("`{name}` is not a valid regular expression: {last}"),
    )
    .with_found(format!("{} {name}:", args.command()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revwalk::fixture::args;
    use nurt_contracts::value::Instant;

    #[test]
    fn reads_paths_texts_and_flags() {
        let bound = args(
            "git.log",
            &[
                (
                    "path",
                    Value::List(vec![Value::Path("/r/a".into()), Value::Text("/r/b".into())]),
                ),
                ("one", Value::Path("/r/c".into())),
                ("revs", Value::List(vec![Value::Text("a".into())])),
                ("revision", Value::Text("HEAD".into())),
                ("group_by", Value::Enum("type".into())),
                ("stat", Value::Bool(true)),
            ],
        );
        assert_eq!(paths(&bound, "path").unwrap(), ["/r/a", "/r/b"]);
        assert_eq!(paths(&bound, "one").unwrap(), ["/r/c"]);
        assert!(paths(&bound, "absent").unwrap().is_empty());
        assert_eq!(texts(&bound, "revs").unwrap(), ["a"]);
        assert_eq!(texts(&bound, "revision").unwrap(), ["HEAD"]);
        assert_eq!(text(&bound, "group_by").unwrap().as_deref(), Some("type"));
        assert!(flag(&bound, "stat").unwrap());
        assert!(!flag(&bound, "absent").unwrap());
        assert_eq!(required_path(&bound, "one").unwrap(), "/r/c");
        assert_eq!(
            required_path(&bound, "absent").unwrap_err().code(),
            ErrorCode::MissingParam
        );
        assert_eq!(
            flag(&bound, "revision").unwrap_err().code(),
            ErrorCode::Type
        );
        assert_eq!(path(&bound, "stat").unwrap_err().code(), ErrorCode::Type);
    }

    #[test]
    fn reads_globs_defaults_and_rejects_values_of_other_types() {
        let bound = args(
            "git.log",
            &[
                (
                    "exclude",
                    Value::List(vec![
                        Value::Glob("*.lock".into()),
                        Value::Text("dist/**".into()),
                    ]),
                ),
                ("single", Value::Glob("*.md".into())),
                ("flag", Value::Bool(true)),
                ("number", Value::Int(7)),
            ],
        );
        assert_eq!(
            globs(&bound, "exclude").unwrap(),
            [BString::from("*.lock"), BString::from("dist/**")]
        );
        assert_eq!(globs(&bound, "single").unwrap(), [BString::from("*.md")]);
        assert!(globs(&bound, "absent").unwrap().is_empty());
        assert_eq!(globs(&bound, "number").unwrap_err().code(), ErrorCode::Type);
        assert!(flag_or(&bound, "flag", false).unwrap());
        assert!(flag_or(&bound, "absent", true).unwrap());
        assert_eq!(
            flag_or(&bound, "number", true).unwrap_err().code(),
            ErrorCode::Type
        );
        assert_eq!(range(&bound, "flag").unwrap_err().code(), ErrorCode::Type);
        assert_eq!(texts(&bound, "number").unwrap_err().code(), ErrorCode::Type);
        assert_eq!(paths(&bound, "number").unwrap_err().code(), ErrorCode::Type);
    }

    #[test]
    fn rejects_negative_counts_and_reads_instants() {
        let bound = args(
            "git.log",
            &[
                ("count", Value::Int(-1)),
                ("skip", Value::Int(3)),
                (
                    "since",
                    Value::Instant(Instant::from_unix_nanos(1_500_000_000)),
                ),
                ("lines", Value::Int(4)),
            ],
        );
        assert_eq!(
            count(&bound, "count").unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        assert_eq!(count(&bound, "skip").unwrap(), Some(3));
        assert_eq!(seconds(&bound, "since").unwrap(), Some(1));
        assert_eq!(range(&bound, "lines").unwrap(), Some(Range::single(4)));
    }

    #[test]
    fn compiles_regular_expressions_with_flags() {
        let bound = args(
            "git.log",
            &[
                ("message", Value::Regex(Regex::new("^fix", "i").unwrap())),
                ("author", Value::Text("ann".into())),
                ("bad", Value::Text("(".into())),
            ],
        );
        assert!(
            regex(&bound, "message")
                .unwrap()
                .unwrap()
                .is_match("FIX bug")
        );
        assert!(regex(&bound, "author").unwrap().unwrap().is_match("Joanna"));
        assert_eq!(regex(&bound, "bad").unwrap_err().code(), ErrorCode::Regex);
        let source = pattern(&bound, "message").unwrap().unwrap();
        assert!(
            bytes_regex(&bound, "message", &source)
                .unwrap()
                .is_match(b"Fixed")
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
