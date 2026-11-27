// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::args
// Responsibility:   Reading of the arguments of the administration commands, the checks of.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reading of the arguments of the administration commands, the checks of
//! names that reach the command line of a program, and the text form of the
//! programs that undo a change.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::BoundArgs;
use nurt_contracts::value::Value;

/// The text of a parameter that holds text, a path, a key or a word.
pub(crate) fn text(args: &BoundArgs, name: &str) -> Result<Option<String>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Text(text) | Value::Path(text) | Value::Key(text) | Value::Enum(text)) => {
            Ok(Some(text.clone()))
        }
        Some(Value::Int(number)) => Ok(Some(number.to_string())),
        Some(other) => Err(type_error(name, "text", other)),
    }
}

/// The text of a required parameter.
pub(crate) fn required(args: &BoundArgs, name: &str) -> Result<String> {
    text(args, name)?.ok_or_else(|| {
        Error::new(
            ErrorCode::MissingParam,
            format!("`{}` needs `{name}`", args.command()),
        )
    })
}

/// A list of texts; one text is a list of one element.
pub(crate) fn list(args: &BoundArgs, name: &str) -> Result<Option<Vec<String>>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| match item {
                Value::Text(text) | Value::Path(text) | Value::Enum(text) => Ok(text.clone()),
                other => Err(type_error(name, "list<text>", other)),
            })
            .collect::<Result<Vec<_>>>()
            .map(Some),
        Some(Value::Text(text) | Value::Enum(text)) => Ok(Some(vec![text.clone()])),
        Some(other) => Err(type_error(name, "list<text>", other)),
    }
}

/// A boolean parameter; absent is `default`.
pub(crate) fn flag(args: &BoundArgs, name: &str, default: bool) -> Result<bool> {
    Ok(args.bool(name)?.unwrap_or(default))
}

/// An integer parameter within `0..=max`.
pub(crate) fn number(args: &BoundArgs, name: &str, max: i64) -> Result<Option<i64>> {
    match args.int(name)? {
        Some(value) if !(0..=max).contains(&value) => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{name}` must be between 0 and {max}"),
        )
        .with_found(value.to_string())),
        other => Ok(other),
    }
}

fn type_error(name: &str, expected: &str, found: &Value) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{name}` expects {expected}, found {}", found.type_name()),
    )
}

/// Checks a name of an account, a group, a module or a key: not empty, no
/// leading `-` that a program would read as an option, no control character
/// and none of the characters that separate fields of the system files.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE`.
pub(crate) fn name(what: &str, value: &str, allow_space: bool) -> Result<()> {
    let bad = value.is_empty()
        || value.starts_with('-')
        || value.chars().any(|c| {
            c.is_control()
                || matches!(c, ':' | ',' | '/' | '\\' | '"' | '\'' | ';' | '=')
                || (c == ' ' && !allow_space)
        });
    if bad {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{value}` is not a valid {what}"),
        )
        .with_found(value.to_owned()));
    }
    Ok(())
}

/// Checks a text that becomes one argument of a program: not empty, no
/// leading `-` and no control character.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE`.
pub(crate) fn operand(what: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{value}` is not a valid {what}"),
        )
        .with_found(value.to_owned()));
    }
    Ok(())
}

/// A text as a literal of the language, `None` when it has a control
/// character that no literal can hold.
pub(crate) fn literal(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    Some(out)
}

/// A list of texts as a literal of the language.
pub(crate) fn literal_list(values: &[String]) -> Option<String> {
    let items: Option<Vec<String>> = values.iter().map(|value| literal(value)).collect();
    items.map(|items| format!("[{}]", items.join(", ")))
}

/// Builder of the text of one program: the command and its named arguments.
#[derive(Debug, Clone)]
pub(crate) struct Program {
    text: String,
    ok: bool,
}

impl Program {
    /// A program that starts with `command`.
    pub(crate) fn new(command: &str) -> Self {
        Self {
            text: command.to_owned(),
            ok: true,
        }
    }

    /// Adds `name: "value"`.
    #[must_use]
    pub(crate) fn text(mut self, name: &str, value: &str) -> Self {
        match literal(value) {
            Some(text) => self.text.push_str(&format!(" {name}: {text}")),
            None => self.ok = false,
        }
        self
    }

    /// Adds `name: "value"` when there is a value.
    #[must_use]
    pub(crate) fn text_opt(self, name: &str, value: Option<&str>) -> Self {
        match value {
            Some(value) => self.text(name, value),
            None => self,
        }
    }

    /// Adds `name: ["a", "b"]`.
    #[must_use]
    pub(crate) fn list(mut self, name: &str, values: &[String]) -> Self {
        match literal_list(values) {
            Some(text) => self.text.push_str(&format!(" {name}: {text}")),
            None => self.ok = false,
        }
        self
    }

    /// Adds `name: true` or `name: false`.
    #[must_use]
    pub(crate) fn flag(mut self, name: &str, value: bool) -> Self {
        self.text.push_str(&format!(" {name}: {value}"));
        self
    }

    /// Adds `name: 12`.
    #[must_use]
    pub(crate) fn int(mut self, name: &str, value: i64) -> Self {
        self.text.push_str(&format!(" {name}: {value}"));
        self
    }

    /// The text, `None` when an argument could not be written.
    pub(crate) fn finish(self) -> Option<String> {
        self.ok.then_some(self.text)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
