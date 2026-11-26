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

use super::comment::Comments;
use super::model::{CollectionStyle, NodeId, NodeKind, SequenceNode, YamlModel};
use super::multidoc;
use super::scalar::{
    date_text, format_float, is_plain_safe, key_text, literal_block, quote_double,
};

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
pub fn index_target(
    model: &YamlModel,
    seq: NodeId,
    index: i64,
    key: &KeyPath,
) -> Result<IndexTarget> {
    let seq = model.anchors().resolve(seq).unwrap_or(seq);
    let Some(sequence) = model.node(seq).as_sequence() else {
        return Err(key_conflict(
            key,
            kind_name(model, seq),
            "is not a list; an index selects an item of a list",
        ));
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
            format!(
                "is past the end of a list of {length} items; use index {length} or `[+]` to append"
            ),
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
pub fn plan_append(
    model: &YamlModel,
    node: NodeId,
    value: &Value,
    key: &KeyPath,
) -> Result<SeqPlan> {
    let target = model.anchors().resolve(node).unwrap_or(node);
    let layout = Layout::of(model);
    let yaml = model.node(target);
    let edits = match &yaml.kind {
        NodeKind::Sequence(sequence) if sequence.style == CollectionStyle::Block => {
            append_block(model, sequence, value, &layout)
        }
        NodeKind::Sequence(sequence) => append_flow(model, sequence, value),
        NodeKind::Scalar(_) if yaml.is_empty_node() => {
            append_to_empty(model, target, value, &layout)
        }
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
    let target = model.anchors().resolve(node).unwrap_or(node);
    let Some(sequence) = model.node(target).as_sequence() else {
        return Err(key_conflict(
            key,
            kind_name(model, target),
            "is not a list; an index selects an item of a list",
        ));
    };
    let length = sequence.items.len();
    let position = if index < 0 {
        usize::try_from(index.unsigned_abs())
            .ok()
            .and_then(|back| length.checked_sub(back))
    } else {
        usize::try_from(index)
            .ok()
            .filter(|&position| position < length)
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

fn append_block(
    model: &YamlModel,
    sequence: &SequenceNode,
    value: &Value,
    layout: &Layout,
) -> Vec<ByteEdit> {
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
            (same_line && !empty && content > dash.start).then(|| {
                model
                    .column(content)
                    .saturating_sub(model.column(dash.start))
            })
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

fn append_to_empty(
    model: &YamlModel,
    node: NodeId,
    value: &Value,
    layout: &Layout,
) -> Vec<ByteEdit> {
    let yaml = model.node(node);
    if yaml.parent.is_none() {
        let (offset, newline) = multidoc::empty_root_insertion(model, yaml.document)
            .unwrap_or((yaml.span.start, false));
        let lead = if newline { layout.eol } else { "" };
        let text = format!(
            "{lead}- {}",
            block_content(value, DEFAULT_ITEM_OFFSET, layout)
        );
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
        let anchor = entry
            .colon
            .as_ref()
            .map_or(yaml.span.start, |colon| colon.end);
        let offset = model.line_end(anchor.max(yaml.full_span().end));
        let content = block_content(value, dash_column + DEFAULT_ITEM_OFFSET, layout);
        let text = format!("{}{}- {content}", layout.eol, spaces(dash_column));
        return vec![ByteEdit::insert(offset, text)];
    }
    let preceding = yaml
        .full_span()
        .start
        .checked_sub(1)
        .and_then(|index| model.source().as_bytes().get(index));
    let lead =
        if matches!(preceding, Some(b' ' | b'\t' | b'\n' | b'[' | b'{')) || preceding.is_none() {
            ""
        } else {
            " "
        };
    vec![ByteEdit::insert(
        yaml.full_span().end,
        format!("{lead}[{}]", flow_text(value)),
    )]
}

fn remove_block(
    model: &YamlModel,
    seq: NodeId,
    sequence: &SequenceNode,
    position: usize,
) -> Vec<ByteEdit> {
    let item = &sequence.items[position];
    let start = item
        .dash
        .as_ref()
        .map_or(item.span.start, |dash| dash.start);
    let inline = !model
        .text(&(model.line_start(start)..start))
        .trim()
        .is_empty();
    if sequence.items.len() == 1 {
        return remove_only_block_item(model, seq, start, item.span.end, inline);
    }
    if inline {
        let next = sequence.items.get(position + 1).map(|next| {
            next.dash
                .as_ref()
                .map_or(next.span.start, |dash| dash.start)
        });
        return match next {
            Some(next) => vec![ByteEdit::delete(Span::new(start, next))],
            None => Vec::new(),
        };
    }
    let comments = Comments::new(model);
    let span = comments.removal_span(start..item.span.end, Some(sequence.indent + 1));
    vec![ByteEdit::delete(Span::new(span.start, span.end))]
}

fn remove_only_block_item(
    model: &YamlModel,
    seq: NodeId,
    start: usize,
    end: usize,
    inline: bool,
) -> Vec<ByteEdit> {
    let content_end = model
        .node(seq)
        .as_sequence()
        .and_then(|sequence| sequence.items.first())
        .map_or(end, |item| model.node(item.node).full_span().end);
    let colon = model
        .entry_of(seq)
        .and_then(|entry| entry.colon.clone())
        .filter(|colon| model.line_start(colon.start) < model.line_start(start));
    let yaml = model.node(seq);
    let properties_end = [
        yaml.anchor.as_ref().map(|anchor| anchor.span.end),
        yaml.tag.as_ref().map(|tag| tag.span.end),
    ]
    .into_iter()
    .flatten()
    .filter(|&property_end| property_end <= start)
    .max();
    match colon {
        Some(colon) if !inline => {
            let comments = Comments::new(model);
            let span = comments.removal_span(start..end, Some(model.column(start) + 1));
            let empty_at =
                properties_end.map_or(colon.end, |property_end| property_end.max(colon.end));
            vec![
                ByteEdit::insert(empty_at, " []"),
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
    match position
        .checked_sub(1)
        .and_then(|previous| items.get(previous))
    {
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
            let Some(parent) = model
                .parent(id)
                .and_then(|parent| model.node(parent).as_mapping())
            else {
                continue;
            };
            if parent.style != CollectionStyle::Block {
                continue;
            }
            match &model.node(id).kind {
                NodeKind::Mapping(child)
                    if child.style == CollectionStyle::Block && child.indent > parent.indent =>
                {
                    let step = child.indent - parent.indent;
                    unit = Some(unit.map_or(step, |unit| unit.min(step)));
                }
                NodeKind::Sequence(child)
                    if child.style == CollectionStyle::Block && flush_lists.is_none() =>
                {
                    flush_lists = Some(child.indent == parent.indent);
                }
                _ => {}
            }
        }
        Self {
            unit: unit.unwrap_or_else(|| super::style::text_unit(model)),
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
        Value::Map(fields) if !fields.is_empty() => block_entries(fields.iter(), column, layout),
        Value::Record(record) if !record.fields.is_empty() => {
            block_entries(record.fields.iter(), column, layout)
        }
        Value::List(items) if !items.is_empty() => {
            let lines: Vec<String> = items
                .iter()
                .map(|item| {
                    format!(
                        "- {}",
                        block_content(item, column + DEFAULT_ITEM_OFFSET, layout)
                    )
                })
                .collect();
            lines.join(&format!("{}{}", layout.eol, spaces(column)))
        }
        Value::Text(text) if text.contains('\n') => {
            literal_block(text, column, layout.eol).unwrap_or_else(|| quote_double(text))
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
                key_text(key, false),
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
        Value::Text(text) if text.contains('\n') => {
            let text =
                literal_block(text, nested, layout.eol).unwrap_or_else(|| quote_double(text));
            format!(" {text}")
        }
        _ => format!(" {}", scalar_text(value, false)),
    }
}

/// Text of `value` in flow style: `[a, b]`, `{k: v}` or a scalar.
fn flow_text(value: &Value) -> String {
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
        .map(|(key, value)| format!("{}: {}", key_text(key, true), flow_text(value)))
        .collect();
    format!("{{{}}}", parts.join(", "))
}

/// Text of a scalar value on one line. Text that a YAML 1.1 or 1.2 reader
/// would type as anything but a string is double-quoted, by the rules of
/// [`is_plain_safe`].
fn scalar_text(value: &Value, flow: bool) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(flag) => flag.to_string(),
        Value::Int(number) => number.to_string(),
        Value::BigInt(text) | Value::Decimal(text) => text.clone(),
        Value::Number(number) => format_float(*number),
        Value::Text(text) => plain_or_quoted(text, flow),
        Value::Date(date) => date_text(date),
        Value::Bytes(bytes) => format!("!!binary {}", bytes.to_base64()),
        Value::List(_) | Value::Map(_) | Value::Record(_) => flow_text(value),
        other => plain_or_quoted(&serialized_text(other), flow),
    }
}

fn plain_or_quoted(text: &str, flow: bool) -> String {
    if is_plain_safe(text, flow) {
        text.to_owned()
    } else {
        quote_double(text)
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

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::value::{Bytes, Date, DateKind, Record, RecordKind, Value};

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
    fn writes_scalars_of_every_type_on_one_line() {
        assert_eq!(scalar_text(&Value::Null, false), "null");
        assert_eq!(scalar_text(&Value::Bool(false), false), "false");
        assert_eq!(scalar_text(&Value::Int(-4), false), "-4");
        assert_eq!(
            scalar_text(&Value::Decimal("1.10".to_owned()), false),
            "1.10"
        );
        assert_eq!(scalar_text(&Value::Number(2.0), false), "2.0");
        assert_eq!(scalar_text(&text("web"), false), "web");
        assert_eq!(scalar_text(&text("yes"), false), "\"yes\"");
        assert_eq!(scalar_text(&text("a, b"), true), "\"a, b\"");
        assert_eq!(
            scalar_text(&Value::Bytes(Bytes::new(b"hello".to_vec())), false),
            "!!binary aGVsbG8="
        );
    }

    #[test]
    fn aligns_keys_of_a_mapping_item() {
        let mut fields = IndexMap::new();
        fields.insert("name".to_owned(), text("x"));
        fields.insert("size".to_owned(), Value::Int(3));
        fields.insert("tags".to_owned(), Value::List(vec![text("a"), text("b")]));
        let content = block_content(&Value::Map(fields), 4, &layout());
        assert_eq!(
            content,
            "name: x\n    size: 3\n    tags:\n      - a\n      - b"
        );
    }

    #[test]
    fn writes_lists_at_the_key_column_in_flush_documents() {
        let flush = Layout {
            flush_lists: true,
            ..layout()
        };
        let mut fields = IndexMap::new();
        fields.insert("tags".to_owned(), Value::List(vec![text("a")]));
        assert_eq!(
            block_content(&Value::Map(fields), 2, &flush),
            "tags:\n  - a"
        );
    }

    #[test]
    fn writes_nested_lists_and_empty_collections() {
        let nested = Value::List(vec![
            Value::List(vec![text("a"), text("b")]),
            Value::List(Vec::new()),
        ]);
        assert_eq!(
            block_content(&nested, 2, &layout()),
            "- - a\n    - b\n  - []"
        );
        assert_eq!(
            block_content(&Value::Map(IndexMap::new()), 2, &layout()),
            "{}"
        );
    }

    #[test]
    fn writes_multi_line_text_as_a_literal_block() {
        assert_eq!(
            block_content(&text("a\nb\n"), 4, &layout()),
            "|\n    a\n    b"
        );
        assert_eq!(
            block_content(&text("a\n\nb"), 2, &layout()),
            "|-\n  a\n\n  b"
        );
        assert_eq!(block_content(&text("a\n\n"), 2, &layout()), "|+\n  a\n");
        assert_eq!(block_content(&text(" a\nb"), 2, &layout()), "\" a\\nb\"");
        let mut fields = IndexMap::new();
        fields.insert("run".to_owned(), text("make\ntest\n"));
        assert_eq!(
            block_content(&Value::Map(fields), 2, &layout()),
            "run: |\n    make\n    test"
        );
    }

    #[test]
    fn writes_flow_collections() {
        let mut fields = IndexMap::new();
        fields.insert(
            "k".to_owned(),
            Value::List(vec![Value::Int(1), text("a,b")]),
        );
        assert_eq!(flow_text(&Value::Map(fields)), "{k: [1, \"a,b\"]}");
        assert_eq!(flow_text(&text("multi\nline")), "\"multi\\nline\"");
    }

    fn parse(source: &str) -> YamlModel {
        YamlModel::parse(source).expect("valid YAML")
    }

    fn key(path: &str) -> KeyPath {
        KeyPath::parse(path).expect("key path")
    }

    fn value_node(model: &YamlModel, name: &str) -> NodeId {
        let root = model.documents()[0].root;
        model.map_get(root, name).expect("key exists")
    }

    /// Source after appending `value` to `node` of the model of `source`.
    fn append_to(source: &str, node: impl Fn(&YamlModel) -> NodeId, value: &Value) -> String {
        let model = parse(source);
        let plan = plan_append(&model, node(&model), value, &key("list[+]")).expect("append plan");
        let applied = crate::edit::apply(source.as_bytes(), &plan.edits).expect("edits apply");
        String::from_utf8(applied.bytes).expect("edits keep UTF-8")
    }

    #[test]
    fn creates_a_list_at_the_key_column_in_flush_documents() {
        let appended = append_to("a:\n- 1\nb:\n", |model| value_node(model, "b"), &text("x"));
        assert_eq!(appended, "a:\n- 1\nb:\n- x\n");
    }

    #[test]
    fn appends_inside_a_flow_list_that_holds_only_a_comment() {
        let appended = append_to(
            "a: [ # none\n  ]\n",
            |model| value_node(model, "a"),
            &text("x"),
        );
        assert_eq!(appended, "a: [x # none\n  ]\n");
    }

    #[test]
    fn creates_lists_in_empty_roots_flow_values_and_items() {
        let root = append_to("---\n", |model| model.documents()[0].root, &text("x"));
        assert_eq!(
            parse(&root).document_value(0).expect("value"),
            Value::List(vec![text("x")])
        );
        let flow = append_to(
            "a: {b: }\n",
            |model| {
                model
                    .map_get(value_node(model, "a"), "b")
                    .expect("key exists")
            },
            &text("x"),
        );
        let model = parse(&flow);
        let b = model
            .map_get(value_node(&model, "a"), "b")
            .expect("key exists");
        assert_eq!(model.value(b).expect("value"), Value::List(vec![text("x")]));
        let item = append_to(
            "-\n- a\n",
            |model| {
                let root = model.documents()[0].root;
                model.node(root).as_sequence().expect("list").items[0].node
            },
            &text("x"),
        );
        assert_eq!(
            parse(&item).document_value(0).expect("value"),
            Value::List(vec![Value::List(vec![text("x")]), text("a")])
        );
    }

    #[test]
    fn rejects_indices_and_removals_outside_lists() {
        let model = parse("m: {a: 1}\nn: 1\nl: [a, b]\ne:\n");
        let map = value_node(&model, "m");
        let error = index_target(&model, map, 0, &key("m[0]")).expect_err("not a list");
        assert_eq!(error.code, ErrorCode::KeyConflict);
        let scalar = value_node(&model, "n");
        let error = plan_remove(&model, scalar, 0, &key("n[0]")).expect_err("not a list");
        assert_eq!(error.code, ErrorCode::KeyConflict);
        let list = value_node(&model, "l");
        for index in [2, -3] {
            let error = plan_remove(&model, list, index, &key("l[0]")).expect_err("no item");
            assert_eq!(error.code, ErrorCode::KeyNotFound);
        }
        assert_eq!(kind_name(&model, map), "map");
        assert_eq!(kind_name(&model, list), "list");
        assert_eq!(kind_name(&model, scalar), "scalar");
        assert_eq!(kind_name(&model, value_node(&model, "e")), "null");
    }

    #[test]
    fn writes_records_nested_mappings_dates_and_paths() {
        let mut record = Record::new(RecordKind::Item);
        record.fields.insert("name".to_owned(), text("x"));
        let mut inner = IndexMap::new();
        inner.insert("k".to_owned(), Value::Int(1));
        let mut fields = IndexMap::new();
        fields.insert("record".to_owned(), Value::Record(record.clone()));
        fields.insert("map".to_owned(), Value::Map(inner));
        assert_eq!(
            block_content(&Value::Map(fields), 2, &layout()),
            "record:\n    name: x\n  map:\n    k: 1"
        );
        assert_eq!(
            block_content(&Value::Record(record.clone()), 2, &layout()),
            "name: x"
        );
        assert_eq!(flow_text(&Value::Record(record)), "{name: x}");
        let date = Value::Date(Date {
            kind: DateKind::LocalDate,
            text: "2026-11-21".to_owned(),
        });
        assert_eq!(scalar_text(&date, false), "2026-11-21");
        let path = Value::Path("conf/app.yaml".to_owned());
        assert_eq!(scalar_text(&path, false), "conf/app.yaml");
    }

    #[test]
    fn layout_takes_the_indentation_unit_from_the_text_without_nested_mappings() {
        let model = parse("l:\n    - x\n");
        assert_eq!(Layout::of(&model).unit, 4);
        let model = parse("l:\n  - x\n");
        assert_eq!(Layout::of(&model).unit, DEFAULT_INDENT_UNIT);
        let model = parse("- x\n");
        assert_eq!(Layout::of(&model).unit, DEFAULT_INDENT_UNIT);
    }

    #[test]
    fn appended_nested_mapping_uses_the_indentation_of_the_text() {
        let mut inner = IndexMap::new();
        inner.insert("n".to_owned(), Value::Int(1));
        let mut item = IndexMap::new();
        item.insert("k".to_owned(), Value::Map(inner));
        let appended = append_to(
            "l:\n    - x\n",
            |model| value_node(model, "l"),
            &Value::Map(item),
        );
        assert_eq!(appended, "l:\n    - x\n    - k:\n          n: 1\n");
    }

    #[test]
    fn quotes_a_date_that_a_yaml_reader_would_type_as_a_number() {
        let clock = Value::Date(Date {
            kind: DateKind::LocalDate,
            text: "07:32:00".to_owned(),
        });
        assert_eq!(scalar_text(&clock, false), "\"07:32:00\"");
        assert_eq!(flow_text(&clock), "\"07:32:00\"");
    }

    #[test]
    fn appends_to_block_lists_with_the_dash_column_of_the_siblings() {
        let appended = append_to(
            "list:\n    - a\n    - b # last\nz: 1\n",
            |model| value_node(model, "list"),
            &text("c"),
        );
        assert_eq!(appended, "list:\n    - a\n    - b # last\n    - c\nz: 1\n");
    }

    #[test]
    fn removes_items_of_flow_and_block_lists() {
        let source = "l: [a, b, c]\nm:\n  - x\n  - y\n";
        let model = parse(source);
        let flow = plan_remove(&model, value_node(&model, "l"), -1, &key("l[-1]")).unwrap();
        let out = crate::edit::apply(source.as_bytes(), &flow.edits).unwrap();
        assert_eq!(
            String::from_utf8(out.bytes).unwrap(),
            "l: [a, b]\nm:\n  - x\n  - y\n"
        );
        let block = plan_remove(&model, value_node(&model, "m"), 0, &key("m[0]")).unwrap();
        let out = crate::edit::apply(source.as_bytes(), &block.edits).unwrap();
        assert_eq!(
            String::from_utf8(out.bytes).unwrap(),
            "l: [a, b, c]\nm:\n  - y\n"
        );
    }

    #[test]
    fn index_equal_to_the_length_appends_and_larger_index_conflicts() {
        let model = parse("l: [a, b]\n");
        let list = value_node(&model, "l");
        assert_eq!(
            index_target(&model, list, 2, &key("l[2]")).unwrap(),
            IndexTarget::Append
        );
        assert!(matches!(
            index_target(&model, list, -2, &key("l[-2]")).unwrap(),
            IndexTarget::Item(_)
        ));
        assert_eq!(
            index_target(&model, list, 3, &key("l[3]"))
                .unwrap_err()
                .code,
            ErrorCode::KeyConflict
        );
        assert_eq!(
            index_target(&model, list, i64::MIN, &key("l[0]"))
                .unwrap_err()
                .code,
            ErrorCode::KeyNotFound
        );
    }
}
