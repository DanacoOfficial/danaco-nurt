// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! YAML tags: explicit typing of scalars and the rule that keeps or removes a
//! tag when the tagged value changes.
//!
//! A tag is reported in `item.tag` as written in the document (`!Ref`,
//! `!!str`, `!e!widget`, `!<tag:yaml.org,2002:str>`). The resolved tag
//! (handle replaced by its `%TAG` prefix) decides whether a scalar is typed
//! explicitly: `!!str 10` is the text `"10"` and `!!binary` is binary data.
//!
//! When a value is replaced, a core tag stays if the new value is of its type
//! and any other tag stays if the new value has the same shape (scalar,
//! mapping, sequence) as the old one. Otherwise the tag is removed together
//! with the old value and the change carries the flag [`TAG_REMOVED_FLAG`].
//!
//! ```
//! use nurt_contracts::value::Value;
//! use nurt_doc::formats::yaml::tag::{self, CoreTag};
//!
//! let core = CoreTag::of("tag:yaml.org,2002:binary");
//! assert_eq!(core, Some(CoreTag::Binary));
//! let value = core.and_then(|core| tag::typed_scalar(core, "aGVs\n bG8="));
//! assert!(matches!(value, Some(Value::Bytes(bytes)) if bytes.as_slice() == b"hello"));
//! ```

use nurt_contracts::value::{Bytes, Value};

use super::model::{NodeId, NodeKind, ScalarStyle, YamlModel};
use crate::edit::ByteEdit;
use crate::model::Span;

/// Prefix of the tags of the YAML core and type repository
/// (`!!str` = `tag:yaml.org,2002:str`).
pub const CORE_PREFIX: &str = "tag:yaml.org,2002:";

/// Flag of a change that removed the tag of the replaced value.
pub const TAG_REMOVED_FLAG: &str = "tag_removed";

/// Number of base64 characters per line of a `!!binary` block scalar when
/// the replaced value gives no line width.
pub const BASE64_LINE_WIDTH: usize = 76;

/// Tags of the YAML core and type repository with a defined meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoreTag {
    /// `!!str`: text.
    Str,
    /// `!!int`: integer.
    Int,
    /// `!!float`: floating point number.
    Float,
    /// `!!bool`: truth value.
    Bool,
    /// `!!null`: null.
    Null,
    /// `!!binary`: base64-encoded binary data.
    Binary,
    /// `!!timestamp`: date or point in time.
    Timestamp,
    /// `!!map`: mapping.
    Map,
    /// `!!seq`: sequence.
    Seq,
    /// `!!set`: mapping whose values are null.
    Set,
    /// `!!omap`: sequence of single-pair mappings with unique keys.
    Omap,
    /// `!!pairs`: sequence of single-pair mappings.
    Pairs,
}

impl CoreTag {
    /// Core tag of a resolved tag; `None` for any other tag.
    pub fn of(resolved: &str) -> Option<Self> {
        let name = resolved.strip_prefix(CORE_PREFIX)?;
        Some(match name {
            "str" => Self::Str,
            "int" => Self::Int,
            "float" => Self::Float,
            "bool" => Self::Bool,
            "null" => Self::Null,
            "binary" => Self::Binary,
            "timestamp" => Self::Timestamp,
            "map" => Self::Map,
            "seq" => Self::Seq,
            "set" => Self::Set,
            "omap" => Self::Omap,
            "pairs" => Self::Pairs,
            _ => return None,
        })
    }

    /// Returns `true` when `value` is a value of this tag, so that the tag
    /// stays valid when the value is written in place of the old one.
    pub fn admits(self, value: &Value) -> bool {
        match self {
            Self::Str => matches!(value, Value::Text(_)),
            Self::Int => matches!(value, Value::Int(_) | Value::BigInt(_)),
            Self::Float => matches!(value, Value::Number(_) | Value::Decimal(_)),
            Self::Bool => matches!(value, Value::Bool(_)),
            Self::Null => matches!(value, Value::Null),
            Self::Binary => matches!(value, Value::Bytes(_)),
            Self::Timestamp => matches!(value, Value::Date(_) | Value::Instant(_)),
            Self::Map | Self::Set => matches!(value, Value::Map(_) | Value::Record(_)),
            Self::Seq | Self::Omap | Self::Pairs => matches!(value, Value::List(_)),
        }
    }
}

/// Shape of a node or of a value written to a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeShape {
    /// Scalar, including the empty node.
    Scalar,
    /// Mapping.
    Mapping,
    /// Sequence.
    Sequence,
}

impl NodeShape {
    /// Shape a value takes when written to a document.
    pub fn of_value(value: &Value) -> Self {
        match value {
            Value::Map(_) | Value::Record(_) => Self::Mapping,
            Value::List(_) => Self::Sequence,
            _ => Self::Scalar,
        }
    }

    /// Shape of a node; an alias has the shape of a scalar because it has
    /// no tag of its own.
    pub fn of_node(model: &YamlModel, node: NodeId) -> Self {
        match model.node(node).kind {
            NodeKind::Mapping(_) => Self::Mapping,
            NodeKind::Sequence(_) => Self::Sequence,
            NodeKind::Scalar(_) | NodeKind::Alias(_) => Self::Scalar,
        }
    }
}

/// Decision about the tag of a node whose value is replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagAction {
    /// The tag stays in front of the new value.
    Keep,
    /// The tag is removed with the old value.
    Remove,
}

/// Decides whether the resolved tag `resolved` of a node of shape `shape`
/// stays when the node receives `value`.
///
/// A core tag stays when the new value is of its type. The non-specific tag
/// `!` marks text and stays only with text. Any other tag stays when the
/// new value has the shape of the node.
///
/// # Examples
///
/// ```
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::yaml::tag::{self, NodeShape, TagAction};
///
/// let text = Value::Text("Other".to_owned());
/// assert_eq!(tag::on_set("!Ref", NodeShape::Scalar, &text), TagAction::Keep);
/// assert_eq!(
///     tag::on_set("tag:yaml.org,2002:int", NodeShape::Scalar, &text),
///     TagAction::Remove
/// );
/// ```
pub fn on_set(resolved: &str, shape: NodeShape, value: &Value) -> TagAction {
    let keeps = match CoreTag::of(resolved) {
        Some(core) => core.admits(value),
        None if resolved == "!" => matches!(value, Value::Text(_)),
        None => NodeShape::of_value(value) == shape,
    };
    if keeps {
        TagAction::Keep
    } else {
        TagAction::Remove
    }
}

/// Tag edit that accompanies the replacement of the value of a node.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TagPlan {
    /// Edits that remove the tag; empty when the tag stays or the node has
    /// none.
    pub edits: Vec<ByteEdit>,
    /// Tag as written, when it is removed.
    pub removed: Option<String>,
}

/// Plans the tag of `node` for the replacement of its value by `value`.
///
/// The removal covers the tag and the spaces that separate it from the next
/// token on the same line; a tag at the end of its line is removed with the
/// spaces in front of it. The edits never touch the content of the node, so
/// they combine with an edit that replaces the content.
pub fn plan_set(model: &YamlModel, node: NodeId, value: &Value) -> TagPlan {
    let Some(tag) = &model.node(node).tag else {
        return TagPlan::default();
    };
    let shape = NodeShape::of_node(model, node);
    if on_set(&tag.resolved, shape, value) == TagAction::Keep {
        return TagPlan::default();
    }
    let bytes = model.source().as_bytes();
    let mut end = tag.span.end;
    while matches!(bytes.get(end), Some(b' ' | b'\t')) {
        end += 1;
    }
    let mut start = tag.span.start;
    if end >= model.line_end(tag.span.end) {
        end = tag.span.end;
        while start > 0 && matches!(bytes.get(start - 1), Some(b' ' | b'\t')) {
            start -= 1;
        }
    }
    TagPlan {
        edits: vec![ByteEdit::delete(Span::new(start, end))],
        removed: Some(tag.text.clone()),
    }
}

/// Value of a scalar with an explicit core tag.
///
/// `text` is the decoded content of the scalar. Returns `None` when the tag
/// does not determine the value (collection tags, `!!timestamp`) or the
/// content is not a valid value of the tag; the scalar is then typed by the
/// ordinary rules and keeps its tag in `item.tag`.
pub fn typed_scalar(core: CoreTag, text: &str) -> Option<Value> {
    match core {
        CoreTag::Str => Some(Value::Text(text.to_owned())),
        CoreTag::Null => Some(Value::Null),
        CoreTag::Bool => typed_bool(text).map(Value::Bool),
        CoreTag::Int => typed_int(text),
        CoreTag::Float => typed_float(text),
        CoreTag::Binary => decode_binary(text).map(Value::Bytes),
        CoreTag::Timestamp
        | CoreTag::Map
        | CoreTag::Seq
        | CoreTag::Set
        | CoreTag::Omap
        | CoreTag::Pairs => None,
    }
}

/// Decodes the base64 content of a `!!binary` scalar; white space and line
/// breaks are ignored.
pub fn decode_binary(text: &str) -> Option<Bytes> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    Bytes::from_base64(&compact).ok()
}

/// Text that replaces the content of `node` with binary data.
///
/// A node tagged `!!binary` keeps its scalar style: a block scalar keeps its
/// header, its content indentation and the width of its first line, any
/// other scalar receives the base64 text on one line. A node without a
/// `!!binary` tag receives `!!binary` and the text on one line.
pub fn render_binary(model: &YamlModel, node: NodeId, data: &Bytes) -> String {
    let yaml = model.node(node);
    let encoded = data.to_base64();
    let tagged = yaml
        .tag
        .as_ref()
        .is_some_and(|tag| CoreTag::of(&tag.resolved) == Some(CoreTag::Binary));
    if !tagged {
        return format!("!!binary {encoded}");
    }
    let Some(scalar) = yaml.as_scalar() else {
        return encoded;
    };
    let Some(header) = scalar.block.as_ref().filter(|_| scalar.style.is_block()) else {
        return if scalar.style == ScalarStyle::Plain {
            encoded
        } else {
            format!("\"{encoded}\"")
        };
    };
    let width = scalar
        .value
        .lines()
        .next()
        .map(str::len)
        .filter(|&width| width > 0)
        .unwrap_or(BASE64_LINE_WIDTH);
    let eol = model.line_ending().as_str();
    let indent = " ".repeat(header.content_indent);
    let mut text = model.text(&header.span).to_owned();
    for line in encoded.as_bytes().chunks(width) {
        text.push_str(eol);
        text.push_str(&indent);
        text.push_str(&String::from_utf8_lossy(line));
    }
    text
}

/// Truth values of the core schema and of the YAML 1.1 boolean type, which
/// an explicit `!!bool` selects in either version.
fn typed_bool(text: &str) -> Option<bool> {
    match text {
        "true" | "True" | "TRUE" | "yes" | "Yes" | "YES" | "on" | "On" | "ON" | "y" | "Y" => {
            Some(true)
        }
        "false" | "False" | "FALSE" | "no" | "No" | "NO" | "off" | "Off" | "OFF" | "n" | "N" => {
            Some(false)
        }
        _ => None,
    }
}

fn typed_int(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let (negative, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let (radix, body) = if let Some(hex) = digits.strip_prefix("0x") {
        (16, hex)
    } else if let Some(octal) = digits.strip_prefix("0o") {
        (8, octal)
    } else {
        (10, digits)
    };
    if body.is_empty() || !body.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let signed = i128::from_str_radix(body, radix)
        .ok()
        .map(|magnitude| if negative { -magnitude } else { magnitude });
    match signed.and_then(|value| i64::try_from(value).ok()) {
        Some(value) => Some(Value::Int(value)),
        None if radix == 10 => {
            let sign = if negative { "-" } else { "" };
            Some(Value::BigInt(format!("{sign}{}", body.trim_start_matches('0'))))
        }
        None => None,
    }
}

fn typed_float(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let number: f64 = trimmed.parse().ok()?;
    if !number.is_finite() {
        return None;
    }
    if number.to_string() == trimmed {
        Some(Value::Number(number))
    } else {
        Some(Value::Decimal(trimmed.to_owned()))
    }
}
