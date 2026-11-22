// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! YAML sequences: indices, appending, removal of items and the text of new
//! values written into sequences.
//!
//! A block sequence under a key may be indented (`  - a`) or written at the
//! column of the key (`- a`); new items take the column of the existing
//! dashes and the distance between dash and content of the first item, so
//! that the keys of a new mapping item line up with the keys of its
//! siblings. A flow sequence (`[a, b]`) stays a flow sequence, on one line
//! or with one item per line as written; an empty `[]` receives its first
//! item inside the brackets.
//!
//! A new item is placed after the last item and before the comment lines
//! that close the sequence. Removing an item removes its lines together with
//! the comment above it and the comment on its line; removing the only item
//! of a block sequence leaves an empty flow sequence `[]`, so the key keeps
//! a list value.

use nurt_contracts::error::Result;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::Value;

use super::anchor::Anchors;
use super::comment::Comments;
use super::model::{CollectionStyle, NodeId, NodeKind, SequenceNode, YamlModel};
use super::multidoc;

use crate::edit::ByteEdit;
use crate::model::{Span, key_conflict, key_not_found};

/// Indentation step used when the document has no nested block mapping.
pub const DEFAULT_INDENT_UNIT: usize = 2;

/// Distance from a dash to the content of its item when the sequence gives
/// none: `- ` is two columns.
pub const DEFAULT_ITEM_OFFSET: usize = 2;

/// Edits of a change of a sequence and the sequence they change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqPlan {
    /// Sequence (or empty node) that receives the change, after aliases
    /// have been followed; a change of an aliased node is shared.
    pub target: NodeId,
    /// Edits of the source text.
    pub edits: Vec<ByteEdit>,
}

/// Element selected by an index of a key path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexTarget {
    /// Existing item.
    Item(NodeId),
    /// Position after the last item: the index equals the length.
    Append,
}

/// Item of `seq` selected by `index`; a negative index counts from the end.
///
/// The sequence is reached through aliases.
///
/// # Errors
///
/// Returns `E_KEY_NOT_FOUND` for a negative index before the first item and
/// `E_KEY_CONFLICT` for an index past the end or a node that is not a
/// sequence.
pub fn index_target(model: &YamlModel, seq: NodeId, index: i64, key: &KeyPath) -> Result<IndexTarget> {
    let anchors = Anchors::new(model);
    let seq = anchors.resolve(seq).unwrap_or(seq);
    let Some(sequence) = model.node(seq).as_sequence() else {
        return Err(key_conflict(key, kind_name(model, seq), "is not a list; an index selects an item of a list"));
    };
    let length = i64::try_from(sequence.items.len()).unwrap_or(i64::MAX);
    let position = if index < 0 { length + index } else { index };
    if position < 0 {
        return Err(key_not_found(key, &[]));
    }
    if position == length {
        return Ok(IndexTarget::Append);
    }
    if position > length {
        return Err(key_conflict(
            key,
            "list",
            format!("is past the end of a list of {length} items; use index {length} or `[+]` to append"),
        ));
    }
    usize::try_from(position)
        .ok()
        .and_then(|position| sequence.items.get(position))
        .map(|item| IndexTarget::Item(item.node))
        .ok_or_else(|| key_not_found(key, &[]))
}

/// Plans appending `value` to the sequence `node`.
///
/// `node` may be an alias of a sequence, a sequence, or the empty node,
/// which becomes a sequence of one item: a block sequence under a key of a
/// block mapping and at the root of a document, a flow sequence elsewhere.
///
/// # Errors
///
/// Returns `E_KEY_CONFLICT` when `node` is a mapping or a scalar other than
/// the empty node.
pub fn plan_append(model: &YamlModel, node: NodeId, value: &Value, key: &KeyPath) -> Result<SeqPlan> {
    let anchors = Anchors::new(model);
    let target = anchors.resolve(node).unwrap_or(node);
    let layout = Layout::of(model);
    let yaml = model.node(target);
    let edits = match &yaml.kind {
        NodeKind::Sequence(sequence) if sequence.style == CollectionStyle::Block => {
            append_block(model, sequence, value, &layout)
        }
        NodeKind::Sequence(sequence) => append_flow(model, sequence, value),
        NodeKind::Scalar(_) if yaml.is_empty_node() => append_to_empty(model, target, value, &layout),
        _ => {
            return Err(key_conflict(
                key,
                kind_name(model, target),
                "is not a list; `[+]` appends to a list",
            ));
        }
    };
    Ok(SeqPlan { target, edits })
}

/// Plans removing the item `index` (negative from the end) of the sequence
/// `node`, which may be an alias of a sequence.
///
/// # Errors
///
/// Returns `E_KEY_CONFLICT` when `node` is not a sequence and
/// `E_KEY_NOT_FOUND` when the index selects no item.
pub fn plan_remove(model: &YamlModel, node: NodeId, index: i64, key: &KeyPath) -> Result<SeqPlan> {
    let anchors = Anchors::new(model);
    let target = anchors.resolve(node).unwrap_or(node);
    let Some(sequence) = model.node(target).as_sequence() else {
        return Err(key_conflict(key, kind_name(model, target), "is not a list; an index selects an item of a list"));
    };
    let length = sequence.items.len();
    let position = if index < 0 {
        usize::try_from(index.unsigned_abs()).ok().and_then(|back| length.checked_sub(back))
    } else {
        usize::try_from(index).ok().filter(|&position| position < length)
    };
    let Some(position) = position else {
        return Err(key_not_found(key, &[]));
    };
    let edits = if sequence.style == CollectionStyle::Block {
        remove_block(model, target, sequence, position)
    } else {
        remove_flow(model, sequence, position)
    };
    Ok(SeqPlan { target, edits })
}

fn kind_name(model: &YamlModel, node: NodeId) -> &'static str {
    let yaml = model.node(node);
    match &yaml.kind {
        NodeKind::Mapping(_) => "map",
        NodeKind::Sequence(_) => "list",
        NodeKind::Alias(_) => "alias",
        NodeKind::Scalar(_) if yaml.is_empty_node() => "null",
        NodeKind::Scalar(_) => "scalar",
    }
}

fn append_block(model: &YamlModel, sequence: &SequenceNode, value: &Value, layout: &Layout) -> Vec<ByteEdit> {
    let Some(last) = sequence.items.last() else {
        return Vec::new();
    };
    let dash_column = sequence.indent;
    let offset = item_offset(model, sequence);
    let content = block_content(value, dash_column + offset, layout);
    let text = format!(
        "{}{}-{}{content}",
        layout.eol,
        spaces(dash_column),
        spaces(offset.saturating_sub(1)),
    );
    vec![ByteEdit::insert(last.span.end, text)]
}

/// Distance between the dash and the content of the first item that has
/// content on the line of its dash.
fn item_offset(model: &YamlModel, sequence: &SequenceNode) -> usize {
    sequence
        .items
        .iter()
        .find_map(|item| {
            let dash = item.dash.as_ref()?;
            let content = model.node(item.node).full_span().start;
            let same_line = model.line_start(content) == model.line_start(dash.start);
            let empty = model.node(item.node).is_empty_node();
            (same_line && !empty && content > dash.start)
                .then(|| model.column(content).saturating_sub(model.column(dash.start)))
        })
        .filter(|&offset| offset > 1)
        .unwrap_or(DEFAULT_ITEM_OFFSET)
}

fn append_flow(model: &YamlModel, sequence: &SequenceNode, value: &Value) -> Vec<ByteEdit> {
    let text = flow_text(value);
    let Some(delimiters) = &sequence.delimiters else {
        return Vec::new();
    };
    let Some(last) = sequence.items.last() else {
        let inside = Span::new(delimiters.open.end, delimiters.close.start);
        let interior = model.text(&(inside.start..inside.end));
        let replacement = if interior.is_empty() {
            text
        } else if interior.bytes().all(|byte| byte == b' ') {
            format!(" {text} ")
        } else {
            return vec![ByteEdit::insert(inside.start, text)];
        };
        return vec![ByteEdit::replace(inside, replacement)];
    };
    let node_end = model.node(last.node).full_span().end;
    let open_line = model.line_start(delimiters.open.start);
    let multiline = model.line_start(model.node(last.node).full_span().start) != open_line;
    let eol = model.line_ending().as_str();
    let separator = if multiline {
        let column = model.column(model.node(last.node).full_span().start);
        format!("{eol}{}", spaces(column))
    } else {
        " ".to_owned()
    };
    match &last.comma {
        Some(comma) => vec![ByteEdit::insert(comma.end, format!("{separator}{text},"))],
        None => vec![ByteEdit::insert(node_end, format!(",{separator}{text}"))],
    }
}

fn append_to_empty(model: &YamlModel, node: NodeId, value: &Value, layout: &Layout) -> Vec<ByteEdit> {
    let yaml = model.node(node);
    if yaml.parent.is_none() {
        let (offset, newline) =
            multidoc::empty_root_insertion(model, yaml.document).unwrap_or((yaml.span.start, false));
        let lead = if newline { layout.eol } else { "" };
        let text = format!("{lead}- {}", block_content(value, DEFAULT_ITEM_OFFSET, layout));
        return vec![ByteEdit::insert(offset, text)];
    }
    let block_parent = yaml
        .parent
        .and_then(|parent| model.node(parent).as_mapping())
        .filter(|mapping| mapping.style == CollectionStyle::Block && !yaml.in_flow);
    let entry = model.entry_of(node);
    if let (Some(mapping), Some(entry)) = (block_parent, entry) {
        let dash_column = if layout.flush_lists {
            mapping.indent
        } else {
            mapping.indent + layout.unit
        };
        let anchor = entry.colon.as_ref().map_or(yaml.span.start, |colon| colon.end);
        let offset = model.line_end(anchor.max(yaml.full_span().end));
        let content = block_content(value, dash_column + DEFAULT_ITEM_OFFSET, layout);
        let text = format!("{}{}- {content}", layout.eol, spaces(dash_column));
        return vec![ByteEdit::insert(offset, text)];
    }
    let preceding = yaml.full_span().start.checked_sub(1).and_then(|index| model.source().as_bytes().get(index));
    let lead = if matches!(preceding, Some(b' ' | b'\t' | b'\n' | b'[' | b'{')) || preceding.is_none() {
        ""
    } else {
        " "
    };
    vec![ByteEdit::insert(yaml.full_span().end, format!("{lead}[{}]", flow_text(value)))]
}

fn remove_block(model: &YamlModel, seq: NodeId, sequence: &SequenceNode, position: usize) -> Vec<ByteEdit> {
    let item = &sequence.items[position];
    let start = item.dash.as_ref().map_or(item.span.start, |dash| dash.start);
    let inline = !model.text(&(model.line_start(start)..start)).trim().is_empty();
    if sequence.items.len() == 1 {
        return remove_only_block_item(model, seq, start, item.span.end, inline);
    }
    if inline {
        let next = sequence.items.get(position + 1).map(|next| next.dash.as_ref().map_or(next.span.start, |dash| dash.start));
        return match next {
            Some(next) => vec![ByteEdit::delete(Span::new(start, next))],
            None => Vec::new(),
        };
    }
    let comments = Comments::new(model);
    let span = comments.removal_span(start..item.span.end, Some(sequence.indent + 1));
    vec![ByteEdit::delete(Span::new(span.start, span.end))]
}

fn remove_only_block_item(model: &YamlModel, seq: NodeId, start: usize, end: usize, inline: bool) -> Vec<ByteEdit> {
    let content_end = model
        .node(seq)
        .as_sequence()
        .and_then(|sequence| sequence.items.first())
        .map_or(end, |item| model.node(item.node).full_span().end);
    let colon = model
        .entry_of(seq)
        .and_then(|entry| entry.colon.clone())
        .filter(|colon| model.line_start(colon.start) < model.line_start(start));
    match colon {
        Some(colon) if !inline => {
            let comments = Comments::new(model);
            let span = comments.removal_span(start..end, Some(model.column(start) + 1));
            vec![
                ByteEdit::insert(colon.end, " []"),
                ByteEdit::delete(Span::new(span.start, span.end)),
            ]
        }
        _ => vec![ByteEdit::replace(Span::new(start, content_end), "[]")],
    }
}

fn remove_flow(model: &YamlModel, sequence: &SequenceNode, position: usize) -> Vec<ByteEdit> {
    let items = &sequence.items;
    let item = &items[position];
    let node = model.node(item.node).full_span();
    let item_end = item.comma.as_ref().map_or(node.end, |comma| comma.end);
    if let Some(next) = items.get(position + 1) {
        let next_start = model.node(next.node).full_span().start;
        return vec![ByteEdit::delete(Span::new(node.start, next_start))];
    }
    match position.checked_sub(1).and_then(|previous| items.get(previous)) {
        Some(previous) => {
            let previous_end = model.node(previous.node).full_span().end;
            let start = match (&item.comma, &previous.comma) {
                (Some(_), Some(comma)) => comma.end,
                _ => previous_end,
            };
            vec![ByteEdit::delete(Span::new(start, item_end))]
        }
        None => vec![ByteEdit::delete(Span::new(node.start, item_end))],
    }
}

/// Layout rules for new block content, taken from the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    unit: usize,
    flush_lists: bool,
    eol: &'static str,
}

impl Layout {
    /// Indentation step, list placement under keys and line break of
    /// `model`.
    ///
    /// The step is the smallest indentation of a block mapping relative to
    /// the block mapping that holds it. Lists are written at the column of
    /// their key when the first block sequence under a key of a block
    /// mapping is written that way.
    pub fn of(model: &YamlModel) -> Self {
        let mut unit: Option<usize> = None;
        let mut flush_lists: Option<bool> = None;
        for id in model.node_ids() {
            let Some(parent) = model.parent(id).and_then(|parent| model.node(parent).as_mapping()) else {
                continue;
            };
            if parent.style != CollectionStyle::Block {
                continue;
            }
            match &model.node(id).kind {
                NodeKind::Mapping(child) if child.style == CollectionStyle::Block && child.indent > parent.indent => {
                    let step = child.indent - parent.indent;
                    unit = Some(unit.map_or(step, |unit| unit.min(step)));
                }
                NodeKind::Sequence(child) if child.style == CollectionStyle::Block && flush_lists.is_none() => {
                    flush_lists = Some(child.indent == parent.indent);
                }
                _ => {}
            }
        }
        Self {
            unit: unit.unwrap_or(DEFAULT_INDENT_UNIT),
            flush_lists: flush_lists.unwrap_or(false),
            eol: model.line_ending().as_str(),
        }
    }
}

fn spaces(count: usize) -> String {
    " ".repeat(count)
}

/// Text of `value` as block content whose first line follows an indicator
/// (`- ` or `key: `) and whose further lines start at `column`.
fn block_content(value: &Value, column: usize, layout: &Layout) -> String {
    match value {
        Value::Map(fields) if !fields.is_empty() => {
            block_entries(fields.iter(), column, layout)
        }
        Value::Record(record) if !record.fields.is_empty() => {
            block_entries(record.fields.iter(), column, layout)
        }
        Value::List(items) if !items.is_empty() => {
            let lines: Vec<String> = items
                .iter()
                .map(|item| format!("- {}", block_content(item, column + DEFAULT_ITEM_OFFSET, layout)))
                .collect();
            lines.join(&format!("{}{}", layout.eol, spaces(column)))
        }
        Value::Text(text) if text.contains('\n') => {
            literal_block(text, column, layout).unwrap_or_else(|| double_quoted(text))
        }
        _ => scalar_text(value, false),
    }
}

fn block_entries<'a>(
    fields: impl Iterator<Item = (&'a String, &'a Value)>,
    column: usize,
    layout: &Layout,
) -> String {
    let lines: Vec<String> = fields
        .map(|(key, value)| {
            format!(
                "{}:{}",
                text_scalar(key, false),
                value_after_key(value, column, layout)
            )
        })
        .collect();
    lines.join(&format!("{}{}", layout.eol, spaces(column)))
}

/// Text after `key:` for a value whose key starts at `column`.
fn value_after_key(value: &Value, column: usize, layout: &Layout) -> String {
    let nested = column + layout.unit;
    match value {
        Value::Map(fields) if !fields.is_empty() => format!(
            "{}{}{}",
            layout.eol,
            spaces(nested),
            block_entries(fields.iter(), nested, layout)
        ),
        Value::Record(record) if !record.fields.is_empty() => format!(
            "{}{}{}",
            layout.eol,
            spaces(nested),
            block_entries(record.fields.iter(), nested, layout)
        ),
        Value::List(items) if !items.is_empty() => {
            let dash = if layout.flush_lists { column } else { nested };
            format!(
                "{}{}{}",
                layout.eol,
                spaces(dash),
                block_content(value, dash, layout)
            )
        }
        Value::Text(text) if text.contains('\n') => match literal_block(text, nested, layout) {
            Some(block) => format!(" {block}"),
            None => format!(" {}", double_quoted(text)),
        },
        _ => format!(" {}", scalar_text(value, false)),
    }
}

/// Literal block scalar `|` for multi-line text with lines at `column`;
/// `None` when the text cannot be written as a literal block (leading space
/// on the first line, carriage returns, control characters).
fn literal_block(text: &str, column: usize, layout: &Layout) -> Option<String> {
    let first = text.lines().next().unwrap_or_default();
    if first.starts_with(' ') || text.contains('\r') || text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return None;
    }
    let body = text.trim_end_matches('\n');
    let trailing = text.len() - body.len();
    let chomping = match trailing {
        0 => "-",
        1 => "",
        _ => "+",
    };
    let indent = spaces(column.max(1));
    let mut out = format!("|{chomping}");
    for line in body.split('\n') {
        out.push_str(layout.eol);
        if !line.is_empty() {
            out.push_str(&indent);
            out.push_str(line);
        }
    }
    for _ in 1..trailing {
        out.push_str(layout.eol);
    }
    Some(out)
}

/// Text of `value` in flow style: `[a, b]`, `{k: v}` or a scalar.
pub fn flow_text(value: &Value) -> String {
    match value {
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(flow_text).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Map(fields) => flow_map(fields.iter()),
        Value::Record(record) => flow_map(record.fields.iter()),
        _ => scalar_text(value, true),
    }
}

fn flow_map<'a>(fields: impl Iterator<Item = (&'a String, &'a Value)>) -> String {
    let parts: Vec<String> = fields
        .map(|(key, value)| format!("{}: {}", text_scalar(key, true), flow_text(value)))
        .collect();
    format!("{{{}}}", parts.join(", "))
}

/// Text of a scalar value; text that a YAML 1.1 or 1.2 reader would type as
/// anything but a string is quoted.
///
/// # Examples
///
/// ```
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::yaml::seq::scalar_text;
///
/// assert_eq!(scalar_text(&Value::Text("web".to_owned()), false), "web");
/// assert_eq!(scalar_text(&Value::Text("yes".to_owned()), false), "\"yes\"");
/// assert_eq!(scalar_text(&Value::Number(2.0), false), "2.0");
/// ```
pub fn scalar_text(value: &Value, flow: bool) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(flag) => flag.to_string(),
        Value::Int(number) => number.to_string(),
        Value::BigInt(text) | Value::Decimal(text) => text.clone(),
        Value::Number(number) => float_text(*number),
        Value::Text(text) => text_scalar(text, flow),
        Value::Date(date) => date.text.clone(),
        Value::Bytes(bytes) => format!("!!binary {}", bytes.to_base64()),
        Value::List(_) | Value::Map(_) | Value::Record(_) => flow_text(value),
        other => text_scalar(&serialized_text(other), flow),
    }
}

/// Text form of values without a YAML counterpart (sizes, durations,
/// paths): the payload of their serialized form.
fn serialized_text(value: &Value) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|json| json.get("v").cloned())
        .map(|payload| match payload {
            serde_json::Value::String(text) => text,
            other => other.to_string(),
        })
        .unwrap_or_default()
}

fn float_text(number: f64) -> String {
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
    let exponent = if exponent.starts_with('-') {
        exponent.to_owned()
    } else {
        format!("+{exponent}")
    };
    format!("{mantissa}e{exponent}")
}

/// Plain text when every YAML reader reads it back as the same string,
/// double-quoted text otherwise.
pub fn text_scalar(text: &str, flow: bool) -> String {
    if plain_safe(text, flow) {
        text.to_owned()
    } else {
        double_quoted(text)
    }
}

const INDICATORS: &[char] = &[
    '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`',
];

const FLOW_INDICATORS: &[char] = &[',', '[', ']', '{', '}'];

const RESERVED_WORDS: &[&str] = &[
    "null", "~", "true", "false", "yes", "no", "on", "off", "y", "n", "<<", "=",
];

fn plain_safe(text: &str, flow: bool) -> bool {
    let Some(first) = text.chars().next() else {
        return false;
    };
    if INDICATORS.contains(&first)
        || text.starts_with(char::is_whitespace)
        || text.ends_with(char::is_whitespace)
        || text.ends_with(':')
        || text.contains(": ")
        || text.contains(" #")
        || text.chars().any(|c| c.is_control())
        || (flow && text.contains(FLOW_INDICATORS))
    {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    !RESERVED_WORDS.contains(&lower.as_str()) && !looks_numeric(&lower)
}

/// Returns `true` for text that a YAML 1.1 or 1.2 reader may type as a
/// number or a timestamp: digits with signs, points, exponents, radix
/// prefixes, underscores, sexagesimal colons and date separators, and the
/// special floats.
fn looks_numeric(lower: &str) -> bool {
    if matches!(lower.trim_start_matches(['+', '-']), ".inf" | ".nan") {
        return true;
    }
    let body = lower.trim_start_matches(['+', '-']);
    let starts_numeric = body.starts_with(|c: char| c.is_ascii_digit())
        || (body.starts_with('.') && body[1..].starts_with(|c: char| c.is_ascii_digit()));
    starts_numeric
        && body
            .chars()
            .all(|c| c.is_ascii_hexdigit() || matches!(c, 'x' | 'o' | '_' | '.' | ':' | '+' | '-' | 't' | 'z' | ' '))
}

/// Double-quoted scalar with the escapes of YAML 1.2, section 5.7.
pub fn double_quoted(text: &str) -> String {
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
            c if c.is_control() => {
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

/// Text of binary data for an item or value position; see
/// [`super::tag::render_binary`] for the replacement of an existing node.
pub fn binary_text(data: &nurt_contracts::value::Bytes) -> String {
    format!("!!binary {}", data.to_base64())
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;
    use nurt_contracts::value::{Bytes, Value};

    use super::*;

    fn layout() -> Layout {
        Layout {
            unit: 2,
            flush_lists: false,
            eol: "\n",
        }
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    #[test]
    fn quotes_text_typed_by_yaml_1_1_or_1_2() {
        for typed in ["yes", "No", "on", "OFF", "y", "~", "null", "true", "12", "0x1F", "0o17", "1_000", "1:20", ".5", "-.inf", ".NaN", "2026-11-22", "1e3", "<<", "="] {
            assert!(scalar_text(&text(typed), false).starts_with('"'), "{typed}");
        }
        for plain in ["web", "eu-west-1", "a b", "v1.2", "x86_64", "nginx:1.25", "true story", "0.0.0.0/0x"] {
            assert_eq!(scalar_text(&text(plain), false), plain);
        }
    }

    #[test]
    fn quotes_indicators_and_flow_characters() {
        assert_eq!(scalar_text(&text("a, b"), false), "a, b");
        assert_eq!(scalar_text(&text("a, b"), true), "\"a, b\"");
        assert_eq!(scalar_text(&text("- x"), false), "\"- x\"");
        assert_eq!(scalar_text(&text("key: value"), false), "\"key: value\"");
        assert_eq!(scalar_text(&text("a #b"), false), "\"a #b\"");
        assert_eq!(scalar_text(&text(""), false), "\"\"");
        assert_eq!(scalar_text(&text(" lead"), false), "\" lead\"");
    }

    #[test]
    fn escapes_double_quoted_text() {
        assert_eq!(double_quoted("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\x01\"");
    }

    #[test]
    fn writes_numbers_that_read_back_as_floats() {
        assert_eq!(scalar_text(&Value::Number(1.5), false), "1.5");
        assert_eq!(scalar_text(&Value::Number(1e300), false), "1.0e+300");
        assert_eq!(scalar_text(&Value::Number(2.5e-8), false), "2.5e-8");
        assert_eq!(scalar_text(&Value::Int(-4), false), "-4");
        assert_eq!(scalar_text(&Value::Decimal("1.10".to_owned()), false), "1.10");
        assert_eq!(scalar_text(&Value::Null, false), "null");
        assert_eq!(scalar_text(&Value::Bool(false), false), "false");
    }

    #[test]
    fn writes_binary_data_with_its_tag() {
        let data = Bytes::new(b"hello".to_vec());
        assert_eq!(scalar_text(&Value::Bytes(data.clone()), false), "!!binary aGVsbG8=");
        assert_eq!(binary_text(&data), "!!binary aGVsbG8=");
    }

    #[test]
    fn aligns_keys_of_a_mapping_item() {
        let mut fields = IndexMap::new();
        fields.insert("name".to_owned(), text("x"));
        fields.insert("size".to_owned(), Value::Int(3));
        fields.insert("tags".to_owned(), Value::List(vec![text("a"), text("b")]));
        let content = block_content(&Value::Map(fields), 4, &layout());
        assert_eq!(content, "name: x\n    size: 3\n    tags:\n      - a\n      - b");
    }

    #[test]
    fn writes_lists_at_the_key_column_in_flush_documents() {
        let flush = Layout {
            flush_lists: true,
            ..layout()
        };
        let mut fields = IndexMap::new();
        fields.insert("tags".to_owned(), Value::List(vec![text("a")]));
        assert_eq!(block_content(&Value::Map(fields), 2, &flush), "tags:\n  - a");
    }

    #[test]
    fn writes_nested_lists_and_empty_collections() {
        let nested = Value::List(vec![Value::List(vec![text("a"), text("b")]), Value::List(Vec::new())]);
        assert_eq!(block_content(&nested, 2, &layout()), "- - a\n    - b\n  - []");
        assert_eq!(block_content(&Value::Map(IndexMap::new()), 2, &layout()), "{}");
    }

    #[test]
    fn writes_multi_line_text_as_a_literal_block() {
        assert_eq!(block_content(&text("a\nb\n"), 4, &layout()), "|\n    a\n    b");
        assert_eq!(block_content(&text("a\n\nb"), 2, &layout()), "|-\n  a\n\n  b");
        assert_eq!(block_content(&text("a\n\n"), 2, &layout()), "|+\n  a\n");
        assert_eq!(block_content(&text(" a\nb"), 2, &layout()), "\" a\\nb\"");
    }

    #[test]
    fn writes_flow_collections() {
        let mut fields = IndexMap::new();
        fields.insert("k".to_owned(), Value::List(vec![Value::Int(1), text("a,b")]));
        assert_eq!(flow_text(&Value::Map(fields)), "{k: [1, \"a,b\"]}");
        assert_eq!(flow_text(&text("multi\nline")), "\"multi\\nline\"");
    }
}
