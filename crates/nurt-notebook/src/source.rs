// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-notebook::source
// Responsibility:   Multiline strings of nbformat: a list of lines or a single string.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Multiline strings of nbformat: a list of lines or a single string.

use serde_json::Value;

/// Stored form of a multiline string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextForm {
    /// A list of lines, each ending with its line break except the last.
    Lines,
    /// A single string.
    Single,
}

impl TextForm {
    /// Form of a stored value; `None` for values that are neither.
    pub(crate) fn of(value: &Value) -> Option<Self> {
        match value {
            Value::Array(_) => Some(Self::Lines),
            Value::String(_) => Some(Self::Single),
            _ => None,
        }
    }

    /// Stored value of `text` in this form.
    pub(crate) fn store(self, text: &str) -> Value {
        match self {
            Self::Single => Value::String(text.to_owned()),
            Self::Lines => Value::Array(
                split_lines(text)
                    .into_iter()
                    .map(|line| Value::String(line.to_owned()))
                    .collect(),
            ),
        }
    }
}

/// Text of a multiline value: the string, or the concatenated list items.
///
/// Returns `None` when the value is neither a string nor a list of strings.
pub(crate) fn join(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => items.iter().map(Value::as_str).collect(),
        _ => None,
    }
}

/// Splits text after each line boundary, keeping the boundaries, as Python
/// `str.splitlines(keepends=True)`, which nbformat uses to store sources.
pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((position, c)) = chars.next() {
        let end = match c {
            '\r' => match chars.peek() {
                Some((_, '\n')) => {
                    chars.next();
                    position + 2
                }
                _ => position + 1,
            },
            '\n' | '\u{0b}' | '\u{0c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}'
            | '\u{2029}' => position + c.len_utf8(),
            _ => continue,
        };
        lines.push(&text[start..end]);
        start = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Whether `c` ends a line in Python `str.splitlines`.
fn is_line_boundary(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r'
            | '\u{0b}'
            | '\u{0c}'
            | '\u{1c}'
            | '\u{1d}'
            | '\u{1e}'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// Lines of `text` without their line boundaries, split at every boundary
/// of Python `str.splitlines`, so that no line holds a character that a
/// reader of the text may take for a line break.
pub(crate) fn lines(text: &str) -> Vec<&str> {
    split_lines(text)
        .into_iter()
        .map(|line| line.trim_end_matches(is_line_boundary))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lines_split_at_every_python_boundary() {
        assert_eq!(
            lines("a\rb\r\nc\u{b}d\u{2028}e\n\nf"),
            ["a", "b", "c", "d", "e", "", "f"]
        );
        assert!(lines("").is_empty());
    }

    #[test]
    fn splits_like_python_splitlines() {
        assert_eq!(split_lines("a\nb\r\nc\rd"), ["a\n", "b\r\n", "c\r", "d"]);
        assert_eq!(split_lines("a\n"), ["a\n"]);
        assert_eq!(split_lines("x\u{2028}y"), ["x\u{2028}", "y"]);
        assert!(split_lines("").is_empty());
    }

    #[test]
    fn stores_text_in_both_forms() {
        assert_eq!(TextForm::Lines.store("a\nb"), json!(["a\n", "b"]));
        assert_eq!(TextForm::Lines.store(""), json!([]));
        assert_eq!(TextForm::Single.store("a\nb"), json!("a\nb"));
    }

    #[test]
    fn joins_lists_and_strings() {
        assert_eq!(join(&json!(["a\n", "b"])).as_deref(), Some("a\nb"));
        assert_eq!(join(&json!("a")).as_deref(), Some("a"));
        assert_eq!(join(&json!([1])), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
