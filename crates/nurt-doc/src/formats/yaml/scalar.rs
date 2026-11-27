// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::yaml::scalar
// Responsibility:   Typing of YAML scalars and the text of new scalar values.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Typing of YAML scalars and the text of new scalar values.
//!
//! A plain scalar is typed by the rules of the YAML version of its document:
//! the core schema of YAML 1.2, or the types of YAML 1.1 (`yes`, `0755`,
//! `1:30`, timestamps) when the document declares `%YAML 1.1` or the
//! configuration selects it. Quoted and block scalars are text. A scalar with
//! a core tag (`!!str`, `!!int`) takes the type of its tag.
//!
//! New text is written so that both a YAML 1.1 and a YAML 1.2 reader read it
//! back as text: a text that either version would type as `null`, a boolean,
//! a number or a timestamp is quoted whatever the version of the document.
//! A replaced scalar keeps its style when the style can represent the new
//! value; otherwise it becomes a double-quoted scalar. Multi-line text
//! becomes a literal block scalar `|` with the chomping indicator chosen from
//! its final line breaks.

use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::value::{Date, DateKind, Value};

use super::model::{
    Chomping, NodeId, ScalarNode, ScalarStyle, TagProperty, YamlModel, YamlVersion,
};
use super::style::{self, Layout};
use super::tag::{self, CoreTag};
use crate::value::{PlainKind, QuoteRules, Syntax, needs_quotes, parse_number, yaml_plain_kind};

/// Characters that end a plain scalar inside `[…]` or `{…}`.
const FLOW_INDICATORS: &[char] = &[',', '[', ']', '{', '}'];

/// Byte order mark, which YAML does not allow inside a plain or block
/// scalar.
const BYTE_ORDER_MARK: char = '\u{FEFF}';

/// Length of the date part `YYYY-MM-DD` of a timestamp.
const DATE_LENGTH: usize = 10;

/// Value of a scalar node.
///
/// A core tag decides the type when the content is a valid value of the
/// tag; the non-specific tag `!` makes the scalar text. Otherwise a plain
/// scalar is typed by the rules of `version` and every other style is text.
///
/// # Examples
///
/// ```
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::yaml::{ScalarNode, ScalarStyle, YamlVersion};
/// use nurt_doc::formats::yaml::scalar::scalar_value;
///
/// let plain = ScalarNode { style: ScalarStyle::Plain, value: "yes".to_owned(), block: None };
/// assert_eq!(scalar_value(&plain, None, YamlVersion::V1_1), Value::Bool(true));
/// assert_eq!(scalar_value(&plain, None, YamlVersion::V1_2), Value::Text("yes".to_owned()));
/// ```
#[must_use]
pub fn scalar_value(scalar: &ScalarNode, tag: Option<&TagProperty>, version: YamlVersion) -> Value {
    if let Some(tag) = tag {
        if let Some(value) =
            CoreTag::of(&tag.resolved).and_then(|core| tag::typed_scalar(core, &scalar.value))
        {
            return value;
        }
        if tag.resolved == "!" {
            return Value::Text(scalar.value.clone());
        }
    }
    match scalar.style {
        ScalarStyle::Plain => plain_value(&scalar.value, version),
        _ => Value::Text(scalar.value.clone()),
    }
}

/// Value of the plain scalar `text` under the typing rules of `version`.
///
/// Numbers keep their lexeme where the binary form would change it
/// ([`Value::Decimal`]); YAML 1.1 timestamps become [`Value::Date`].
///
/// # Examples
///
/// ```
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::yaml::YamlVersion;
/// use nurt_doc::formats::yaml::scalar::plain_value;
///
/// assert_eq!(plain_value("0755", YamlVersion::V1_1), Value::Int(493));
/// assert_eq!(plain_value("0755", YamlVersion::V1_2), Value::Int(755));
/// assert_eq!(plain_value("~", YamlVersion::V1_2), Value::Null);
/// ```
#[must_use]
pub fn plain_value(text: &str, version: YamlVersion) -> Value {
    let syntax = syntax_of(version);
    match yaml_plain_kind(text, version) {
        PlainKind::Null => Value::Null,
        PlainKind::Bool => Value::Bool(is_true(text)),
        PlainKind::Int | PlainKind::Float => {
            parse_number(text, syntax).unwrap_or_else(|| Value::Text(text.to_owned()))
        }
        PlainKind::Timestamp => Value::Date(Date {
            kind: date_kind(text),
            text: text.to_owned(),
        }),
        _ => Value::Text(text.to_owned()),
    }
}

/// Number syntax of a YAML version.
#[must_use]
pub const fn syntax_of(version: YamlVersion) -> Syntax {
    match version {
        YamlVersion::V1_1 => Syntax::Yaml11,
        _ => Syntax::Yaml12,
    }
}

fn is_true(text: &str) -> bool {
    matches!(
        text,
        "true" | "True" | "TRUE" | "y" | "Y" | "yes" | "Yes" | "YES" | "on" | "On" | "ON"
    )
}

fn date_kind(text: &str) -> DateKind {
    let time = text.get(DATE_LENGTH..).unwrap_or_default();
    if time.is_empty() {
        DateKind::LocalDate
    } else if time.ends_with(['Z', 'z']) || time.contains(['+', '-']) {
        DateKind::OffsetDateTime
    } else {
        DateKind::LocalDateTime
    }
}

/// Returns `true` when `text` written as a plain scalar is read back as the
/// same text by YAML 1.1 and YAML 1.2 readers; `flow` adds the restrictions
/// of plain scalars inside `[…]` and `{…}`.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::yaml::scalar::is_plain_safe;
///
/// assert!(is_plain_safe("nginx:1.27", false));
/// assert!(!is_plain_safe("on", false));
/// assert!(!is_plain_safe("1.0", false));
/// assert!(!is_plain_safe("a, b", true));
/// ```
#[must_use]
pub fn is_plain_safe(text: &str, flow: bool) -> bool {
    !text.is_empty()
        && !needs_quotes(text, QuoteRules::Yaml)
        && !text.starts_with("...")
        && !text.contains(BYTE_ORDER_MARK)
        && !(flow && text.contains(FLOW_INDICATORS))
}

/// Double-quoted scalar with the escapes of YAML 1.2, section 5.7.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::yaml::scalar::quote_double;
///
/// assert_eq!(quote_double("a \"b\"\n"), r#""a \"b\"\n""#);
/// ```
#[must_use]
pub fn quote_double(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if c.is_control() || c == BYTE_ORDER_MARK || c == '\u{2028}' || c == '\u{2029}' => {
                let code = u32::from(c);
                if code <= 0xFF {
                    out.push_str(&format!("\\x{code:02X}"));
                } else {
                    out.push_str(&format!("\\u{code:04X}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Single-quoted scalar; `None` when the text has a line break or a
/// character that a single-quoted scalar cannot hold.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::yaml::scalar::quote_single;
///
/// assert_eq!(quote_single("it's").as_deref(), Some("'it''s'"));
/// assert_eq!(quote_single("a\nb"), None);
/// ```
#[must_use]
pub fn quote_single(text: &str) -> Option<String> {
    if text
        .chars()
        .any(|c| (c.is_control() && c != '\t') || c == BYTE_ORDER_MARK)
    {
        return None;
    }
    Some(format!("'{}'", text.replace('\'', "''")))
}

/// Literal block scalar `|` holding `text`, with content lines indented by
/// `indent` spaces and separated by `eol`.
///
/// The chomping indicator follows the final line breaks of the text: `|-`
/// for none, `|` for one, `|+` for more. The text after the last content
/// line holds the further line breaks of `|+` except the last one, which
/// the line break that follows the scalar provides. Returns `None` for text
/// that a literal block cannot hold: an empty text, a first line starting
/// with a space, carriage returns and control characters.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::yaml::scalar::literal_block;
///
/// assert_eq!(literal_block("a\nb\n", 2, "\n").as_deref(), Some("|\n  a\n  b"));
/// assert_eq!(literal_block("a\nb", 4, "\n").as_deref(), Some("|-\n    a\n    b"));
/// assert_eq!(literal_block(" a\n", 2, "\n"), None);
/// ```
#[must_use]
pub fn literal_block(text: &str, indent: usize, eol: &str) -> Option<String> {
    block(text, '|', indent, eol)
}

fn block(text: &str, indicator: char, indent: usize, eol: &str) -> Option<String> {
    let body = text.trim_end_matches('\n');
    let unsupported = body.is_empty()
        || body.starts_with([' ', '\t'])
        || text
            .chars()
            .any(|c| (c.is_control() && c != '\n' && c != '\t') || c == BYTE_ORDER_MARK);
    if unsupported {
        return None;
    }
    let trailing = text.len() - body.len();
    let chomping = match trailing {
        0 => "-",
        1 => "",
        _ => "+",
    };
    let margin = " ".repeat(indent.max(1));
    let mut out = format!("{indicator}{chomping}");
    for line in body.split('\n') {
        out.push_str(eol);
        if !line.is_empty() {
            out.push_str(&margin);
            out.push_str(line);
        }
    }
    for _ in 1..trailing {
        out.push_str(eol);
    }
    Some(out)
}

/// Shortest text of a floating point number that YAML 1.1 and YAML 1.2
/// readers both read as the same number: a fraction or an exponent with a
/// signed exponent, `.inf`, `-.inf` or `.nan`.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::yaml::scalar::format_float;
///
/// assert_eq!(format_float(2.0), "2.0");
/// assert_eq!(format_float(1e300), "1.0e+300");
/// assert_eq!(format_float(f64::NEG_INFINITY), "-.inf");
/// ```
#[must_use]
pub fn format_float(number: f64) -> String {
    if number.is_nan() {
        return ".nan".to_owned();
    }
    if number.is_infinite() {
        return if number > 0.0 { ".inf" } else { "-.inf" }.to_owned();
    }
    let text = format!("{number:?}");
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return text;
    };
    let mantissa = if mantissa.contains('.') {
        mantissa.to_owned()
    } else {
        format!("{mantissa}.0")
    };
    let sign = if exponent.starts_with('-') { "" } else { "+" };
    format!("{mantissa}e{sign}{exponent}")
}

/// Text of a key of a new mapping entry: plain when safe, double-quoted
/// otherwise.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::yaml::scalar::key_text;
///
/// assert_eq!(key_text("a.b", false), "a.b");
/// assert_eq!(key_text("no", false), "\"no\"");
/// ```
#[must_use]
pub fn key_text(name: &str, flow: bool) -> String {
    if is_plain_safe(name, flow) {
        name.to_owned()
    } else {
        quote_double(name)
    }
}

/// Text of a new scalar value at a position whose block content would be
/// indented by `indent` spaces; `flow` selects the rules of flow
/// collections, where text is never written as a block scalar.
///
/// # Errors
///
/// `E_INVALID_VALUE` for a number lexeme that is not a YAML number and for
/// values without a scalar form (lists, maps).
pub fn new_text(value: &Value, flow: bool, indent: usize, eol: &str) -> Result<String> {
    match value {
        Value::Null => Ok("null".to_owned()),
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Int(number) => Ok(number.to_string()),
        Value::BigInt(digits) => Ok(digits.clone()),
        Value::Number(number) => Ok(format_float(*number)),
        Value::Decimal(lexeme) => decimal_text(lexeme),
        Value::Date(date) => Ok(date_text(date)),
        Value::Bytes(data) => Ok(format!("!!binary {}", data.to_base64())),
        Value::Text(text) => Ok(text_scalar(text, flow, indent, eol)),
        other => Err(Error::new(
            ErrorCode::InvalidValue,
            format!(
                "a {} value has no YAML scalar form; pass text, a number, a boolean or null",
                crate::value::value_type(other)
            ),
        )),
    }
}

fn decimal_text(lexeme: &str) -> Result<String> {
    let valid = parse_number(lexeme, Syntax::Yaml12).is_some()
        || parse_number(lexeme, Syntax::Yaml11).is_some();
    if valid {
        Ok(lexeme.to_owned())
    } else {
        Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{lexeme}` is not a YAML number; pass a number or text"),
        ))
    }
}

/// Text of a date value: the date as written when a YAML 1.1 reader types it
/// as a timestamp, a double-quoted scalar otherwise.
pub(crate) fn date_text(date: &Date) -> String {
    if yaml_plain_kind(&date.text, YamlVersion::V1_1) == PlainKind::Timestamp {
        date.text.clone()
    } else {
        quote_double(&date.text)
    }
}

/// Text of a new text value: plain when safe, a literal block for multi-line
/// text outside flow collections, double-quoted otherwise.
fn text_scalar(text: &str, flow: bool, indent: usize, eol: &str) -> String {
    if text.contains('\n')
        && !flow
        && let Some(block) = literal_block(text, indent, eol)
    {
        return block;
    }
    if is_plain_safe(text, flow) {
        text.to_owned()
    } else {
        quote_double(text)
    }
}

/// New text of the scalar `node` holding `value` and the bytes it replaces.
///
/// Text keeps the style of the scalar when the style can represent it: a
/// plain scalar stays plain when the text is plain-safe, a single-quoted
/// scalar stays single-quoted without line breaks, a block scalar stays a
/// block scalar (`>` for single-line text, `|` for several lines). Any other
/// combination gives a double-quoted scalar, or a literal block for
/// multi-line text outside flow collections. Values of other types are
/// written plain. A comment on the last line of a scalar that becomes a
/// block scalar moves to the header line.
///
/// # Errors
///
/// As for [`new_text`].
pub(crate) fn replace(
    model: &YamlModel,
    node: NodeId,
    value: &Value,
    layout: &Layout,
) -> Result<(Range<usize>, String)> {
    let yaml = model.node(node);
    let scalar = yaml.as_scalar();
    let style_of = scalar.map_or(ScalarStyle::Plain, |scalar| scalar.style);
    let indent = scalar.and_then(|scalar| scalar.block.as_ref()).map_or_else(
        || style::content_column(model, node, layout),
        |header| header.content_indent,
    );
    let text = match value {
        Value::Text(text) => styled_text(text, style_of, yaml.in_flow, indent, layout.eol),
        other => new_text(other, yaml.in_flow, indent, layout.eol)?,
    };
    let mut range = yaml.span.clone();
    let mut text = style::separated(model, range.start, text);
    if text.starts_with(['|', '>'])
        && text.contains(layout.eol)
        && !style_of.is_block()
        && let Some(comment) = style::inline_comment(model, range.end)
    {
        text = move_comment(model, &text, &comment, layout.eol);
        range.end = comment.end;
    }
    Ok((range, text))
}

/// Block scalar text with the comment `comment` placed after its header.
fn move_comment(model: &YamlModel, text: &str, comment: &Range<usize>, eol: &str) -> String {
    let (header, body) = text.split_once(eol).unwrap_or((text, ""));
    format!("{header} {}{eol}{body}", model.text(comment))
}

fn styled_text(text: &str, style_of: ScalarStyle, flow: bool, indent: usize, eol: &str) -> String {
    let multiline = text.contains('\n');
    let kept = match style_of {
        ScalarStyle::Plain if !multiline && is_plain_safe(text, flow) => Some(text.to_owned()),
        ScalarStyle::SingleQuoted if !multiline => quote_single(text),
        ScalarStyle::Literal => literal_block(text, indent, eol),
        ScalarStyle::Folded if multiline => literal_block(text, indent, eol),
        ScalarStyle::Folded => block(text, '>', indent, eol),
        _ => None,
    };
    kept.unwrap_or_else(|| {
        if multiline && !flow {
            literal_block(text, indent, eol).unwrap_or_else(|| quote_double(text))
        } else {
            quote_double(text)
        }
    })
}

/// Chomping indicator that reads `text` back with its final line breaks.
#[must_use]
pub fn chomping_for(text: &str) -> Chomping {
    match text.len() - text.trim_end_matches('\n').len() {
        0 => Chomping::Strip,
        1 => Chomping::Clip,
        _ => Chomping::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(style: ScalarStyle, value: &str) -> ScalarNode {
        ScalarNode {
            style,
            value: value.to_owned(),
            block: None,
        }
    }

    #[test]
    fn types_plain_scalars_by_version() {
        let cases = [
            ("on", YamlVersion::V1_1, Value::Bool(true)),
            ("off", YamlVersion::V1_1, Value::Bool(false)),
            ("on", YamlVersion::V1_2, Value::Text("on".to_owned())),
            ("0x1F", YamlVersion::V1_2, Value::Int(31)),
            ("0o17", YamlVersion::V1_2, Value::Int(15)),
            ("0b101", YamlVersion::V1_1, Value::Int(5)),
            ("1_000", YamlVersion::V1_1, Value::Int(1000)),
            ("1:30", YamlVersion::V1_1, Value::Int(90)),
            ("1.5", YamlVersion::V1_2, Value::Number(1.5)),
            ("1.10", YamlVersion::V1_2, Value::Decimal("1.10".to_owned())),
            ("", YamlVersion::V1_2, Value::Null),
            ("NULL", YamlVersion::V1_2, Value::Null),
        ];
        for (text, version, expected) in cases {
            assert_eq!(plain_value(text, version), expected, "{text}");
        }
    }

    #[test]
    fn types_yaml_1_1_timestamps_as_dates() {
        let Value::Date(date) = plain_value("2026-11-22", YamlVersion::V1_1) else {
            panic!("expected a date");
        };
        assert_eq!(date.kind, DateKind::LocalDate);
        let Value::Date(date) = plain_value("2026-11-22T10:00:00Z", YamlVersion::V1_1) else {
            panic!("expected a date");
        };
        assert_eq!(date.kind, DateKind::OffsetDateTime);
        assert_eq!(
            plain_value("2026-11-22", YamlVersion::V1_2),
            Value::Text("2026-11-22".to_owned())
        );
    }

    #[test]
    fn quoted_and_block_scalars_are_text() {
        let quoted = scalar(ScalarStyle::DoubleQuoted, "true");
        assert_eq!(
            scalar_value(&quoted, None, YamlVersion::V1_2),
            Value::Text("true".to_owned())
        );
        let block = scalar(ScalarStyle::Literal, "1\n");
        assert_eq!(
            scalar_value(&block, None, YamlVersion::V1_2),
            Value::Text("1\n".to_owned())
        );
    }

    #[test]
    fn core_tags_decide_the_type() {
        let tagged = |resolved: &str| TagProperty {
            text: String::new(),
            resolved: resolved.to_owned(),
            span: 0..0,
        };
        let plain = scalar(ScalarStyle::Plain, "12");
        let str_tag = tagged("tag:yaml.org,2002:str");
        assert_eq!(
            scalar_value(&plain, Some(&str_tag), YamlVersion::V1_2),
            Value::Text("12".to_owned())
        );
        let quoted = scalar(ScalarStyle::SingleQuoted, "12");
        let int_tag = tagged("tag:yaml.org,2002:int");
        assert_eq!(
            scalar_value(&quoted, Some(&int_tag), YamlVersion::V1_2),
            Value::Int(12)
        );
        let local = tagged("!");
        assert_eq!(
            scalar_value(&plain, Some(&local), YamlVersion::V1_2),
            Value::Text("12".to_owned())
        );
    }

    #[test]
    fn quotes_text_that_any_version_would_type() {
        for text in [
            "yes",
            "No",
            "~",
            "null",
            "1.0",
            "0o17",
            "012",
            "1:30",
            "2026-11-22",
            ".inf",
            "<<",
        ] {
            assert!(!is_plain_safe(text, false), "{text}");
        }
        for text in ["web", "nginx:1.27", "a b", "path/to/x", "x-1"] {
            assert!(is_plain_safe(text, false), "{text}");
        }
        assert!(!is_plain_safe("", false));
        assert!(!is_plain_safe("a # b", false));
        assert!(!is_plain_safe("a: b", false));
        assert!(!is_plain_safe("...", false));
        assert!(!is_plain_safe("\u{FEFF}a", false));
    }

    #[test]
    fn escapes_double_quoted_text() {
        assert_eq!(quote_double("\t\u{7}\u{FEFF}\\"), "\"\\t\\x07\\uFEFF\\\\\"");
        assert_eq!(quote_single("a\u{1}"), None);
    }

    #[test]
    fn chooses_chomping_from_final_line_breaks() {
        assert_eq!(
            literal_block("a\n\n", 2, "\n").as_deref(),
            Some("|+\n  a\n")
        );
        assert_eq!(
            literal_block("a\n\nb\n", 2, "\r\n").as_deref(),
            Some("|\r\n  a\r\n\r\n  b")
        );
        assert_eq!(literal_block("\n", 2, "\n"), None);
        assert_eq!(chomping_for("a"), Chomping::Strip);
        assert_eq!(chomping_for("a\n"), Chomping::Clip);
        assert_eq!(chomping_for("a\n\n"), Chomping::Keep);
    }

    #[test]
    fn writes_new_scalars() {
        let text = |value: Value| new_text(&value, false, 2, "\n").unwrap();
        assert_eq!(text(Value::Null), "null");
        assert_eq!(text(Value::Number(1e300)), "1.0e+300");
        assert_eq!(text(Value::Number(f64::NAN)), ".nan");
        assert_eq!(text(Value::Text("yes".to_owned())), "\"yes\"");
        assert_eq!(text(Value::Text("a\nb\n".to_owned())), "|\n  a\n  b");
        assert_eq!(text(Value::Decimal("1.10".to_owned())), "1.10");
        assert_eq!(
            text(Value::Date(Date {
                kind: DateKind::LocalDate,
                text: "2026-11-22".to_owned()
            })),
            "2026-11-22"
        );
        assert_eq!(
            new_text(&Value::Text("a\nb".to_owned()), true, 2, "\n").unwrap(),
            "\"a\\nb\""
        );
        assert!(new_text(&Value::Decimal("x".to_owned()), false, 2, "\n").is_err());
        assert!(new_text(&Value::List(Vec::new()), false, 2, "\n").is_err());
        assert_eq!(key_text("", false), "\"\"");
    }

    #[test]
    fn keeps_the_style_when_it_represents_the_text() {
        let styled = |text: &str, style: ScalarStyle| styled_text(text, style, false, 2, "\n");
        assert_eq!(styled("b", ScalarStyle::Plain), "b");
        assert_eq!(styled("true", ScalarStyle::Plain), "\"true\"");
        assert_eq!(styled("it's", ScalarStyle::SingleQuoted), "'it''s'");
        assert_eq!(styled("x", ScalarStyle::DoubleQuoted), "\"x\"");
        assert_eq!(styled("one", ScalarStyle::Literal), "|-\n  one");
        assert_eq!(styled("one", ScalarStyle::Folded), ">-\n  one");
        assert_eq!(styled("a\nb\n", ScalarStyle::Folded), "|\n  a\n  b");
        assert_eq!(styled("a\nb", ScalarStyle::Plain), "|-\n  a\n  b");
        assert_eq!(styled(" a\nb", ScalarStyle::Plain), "\" a\\nb\"");
        assert_eq!(
            styled_text("a\nb", ScalarStyle::Plain, true, 2, "\n"),
            "\"a\\nb\""
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
