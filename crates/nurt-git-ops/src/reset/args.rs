// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::reset::args
// Responsibility:   Reading bound arguments of the `git.*` commands.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reading bound arguments of the `git.*` commands.

use std::path::PathBuf;

use nurt_contracts::error::{Error, ErrorCode, Result, Stage};
use nurt_contracts::stage::BoundArgs;
use nurt_contracts::value::Value;

fn type_error(args: &BoundArgs, name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{}` expects {expected} for `{name}`", args.command()),
    )
    .with_stage(Stage::Bind)
}

/// Text of a text, path, enumeration or glob value.
fn as_text(value: &Value) -> Option<&str> {
    match value {
        Value::Text(text) | Value::Path(text) | Value::Enum(text) | Value::Glob(text) => {
            Some(text.as_str())
        }
        _ => None,
    }
}

/// Optional text argument.
pub(crate) fn text(args: &BoundArgs, name: &str) -> Result<Option<String>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => as_text(value)
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| type_error(args, name, "text")),
    }
}

/// Required text argument.
pub(crate) fn required_text(args: &BoundArgs, name: &str) -> Result<String> {
    text(args, name)?.ok_or_else(|| {
        Error::new(
            ErrorCode::MissingParam,
            format!("`{}` needs a value for `{name}`", args.command()),
        )
        .with_stage(Stage::Bind)
    })
}

/// Required path argument.
pub(crate) fn path(args: &BoundArgs, name: &str) -> Result<PathBuf> {
    required_text(args, name).map(PathBuf::from)
}

/// Boolean argument, `false` when absent.
pub(crate) fn flag(args: &BoundArgs, name: &str) -> Result<bool> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(type_error(args, name, "bool")),
    }
}

/// List of texts, empty when absent.
pub(crate) fn list(args: &BoundArgs, name: &str) -> Result<Vec<String>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| {
                as_text(item)
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| type_error(args, name, "list<text>"))
            })
            .collect(),
        Some(value) => as_text(value)
            .map(|text| vec![text.to_owned()])
            .ok_or_else(|| type_error(args, name, "list<text>")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
mod tests {
    use nurt_contracts::stage::ArgValue;

    use super::*;

    fn args() -> BoundArgs {
        BoundArgs::new("git.stash")
            .with("repo", ArgValue::Value(Value::Path("/r".into())), false)
            .with("flag", ArgValue::Value(Value::Bool(true)), false)
            .with(
                "paths",
                ArgValue::Value(Value::List(vec![
                    Value::Path("a".into()),
                    Value::Text("b".into()),
                ])),
                false,
            )
            .with("count", ArgValue::Value(Value::Int(1)), false)
    }

    #[test]
    fn reads_paths_flags_and_lists() {
        let args = args();
        assert_eq!(path(&args, "repo").unwrap(), PathBuf::from("/r"));
        assert!(flag(&args, "flag").unwrap());
        assert!(!flag(&args, "missing").unwrap());
        assert_eq!(list(&args, "paths").unwrap(), vec!["a", "b"]);
        assert!(list(&args, "missing").unwrap().is_empty());
    }

    #[test]
    fn rejects_values_of_another_type() {
        let args = args();
        assert_eq!(text(&args, "count").unwrap_err().code, ErrorCode::Type);
        assert_eq!(flag(&args, "repo").unwrap_err().code, ErrorCode::Type);
        assert_eq!(
            required_text(&args, "missing").unwrap_err().code,
            ErrorCode::MissingParam
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
