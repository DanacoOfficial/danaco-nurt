// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Edits of YAML mappings and of whole nodes.
//!
//! A new key of a block mapping is written after the last entry, at the
//! column of its siblings; a flow mapping receives the key before its
//! closing bracket, and an empty `{}` stays a flow mapping. Removing an entry
//! of a block mapping removes its lines together with the comment above it
//! and the comment on its last line; removing the only entry leaves `{}`, so
//! the key keeps a mapping value. A node whose value changes kind (scalar,
//! mapping, list) is rewritten in block style in block context and in flow
//! style inside flow collections; its anchor and tag stay.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::value::{Fields, Value};

use super::comment::Comments;
use super::merge;
use super::model::{CollectionStyle, MapEntry, MappingNode, NodeId, NodeKind, YamlModel};
use super::scalar::{self, key_text};
use super::style::{self, ITEM_OFFSET, Layout, spaces};
use crate::edit::ByteEdit;
use crate::model::Span;

/// Position of a node in its parent, which decides how new block content
/// is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Value of a block mapping entry; `column` is the column of the key.
    Value { colon_end: usize, column: usize },
    /// Item of a block sequence; `column` is the column of its content.
    Item { dash_end: usize, column: usize },
    /// Root of a document.
    Root,
}

impl Place {
    fn of(model: &YamlModel, node: NodeId, layout: &Layout) -> Option<Self> {
        let parent = model.parent(node);
        if let (Some(map), Some((index, entry))) = (parent, model.entry_position(node)) {
            let colon = entry.colon.as_ref()?;
            return Some(Self::Value {
                colon_end: colon.end,
                column: style::key_column(model, map, index),
            });
        }
        if let Some((_, item)) = model.item_of(node) {
            let dash = item.dash.as_ref()?;
            return Some(Self::Item {
                dash_end: dash.end,
                column: style::content_column(model, node, layout)
                    .max(model.column(dash.start) + ITEM_OFFSET),
            });
        }
        parent.is_none().then_some(Self::Root)
    }
}

/// Edits that add the entry `name: value` at the end of the mapping `map`.
///
/// # Errors
///
/// `E_UNSUPPORTED_CONSTRUCT` `yaml.complex_key` for a key that needs the
/// explicit form, `E_KEY_CONFLICT` for a single-pair mapping inside a flow
/// sequence, and the errors of [`scalar::new_text`].
pub(crate) fn insert_entry(
    model: &YamlModel,
    map: NodeId,
    name: &str,
    value: &Value,
    layout: &Layout,
) -> Result<Vec<ByteEdit>> {
    merge::check_new_key(name)?;
    let Some(mapping) = model.node(map).as_mapping() else {
        return Err(Error::new(
            ErrorCode::KeyConflict,
            "a key can only be added to a map; set the parent to a map first",
        ));
    };
    match mapping.style {
        CollectionStyle::Block => insert_block(mapping, name, value, layout),
        CollectionStyle::Flow => insert_flow(model, mapping, name, value, layout),
        CollectionStyle::FlowPair => Err(Error::new(
            ErrorCode::KeyConflict,
            "a single-pair mapping inside a flow sequence holds one key; set the whole list item instead",
        )),
    }
}

fn insert_block(
    mapping: &MappingNode,
    name: &str,
    value: &Value,
    layout: &Layout,
) -> Result<Vec<ByteEdit>> {
    let offset = mapping.entries.last().map_or(0, |last| last.span.end);
    let text = format!(
        "{}{}{}:{}",
        layout.eol,
        spaces(mapping.indent),
        key_text(name, false),
        block_value(value, mapping.indent, layout)?
    );
    Ok(vec![ByteEdit::insert(offset, text)])
}

fn insert_flow(
    model: &YamlModel,
    mapping: &MappingNode,
    name: &str,
    value: &Value,
    layout: &Layout,
) -> Result<Vec<ByteEdit>> {
    let entry = format!("{}: {}", key_text(name, true), flow_value(value)?);
    let Some(last) = mapping.entries.last() else {
        let open = mapping
            .delimiters
            .as_ref()
            .map_or(0, |delimiters| delimiters.open.end);
        return Ok(vec![ByteEdit::insert(open, entry)]);
    };
    if let Some(comma) = &last.comma {
        return Ok(vec![ByteEdit::insert(comma.end, format!(" {entry},"))]);
    }
    let close_line = mapping
        .delimiters
        .as_ref()
        .map_or(0, |delimiters| model.line_start(delimiters.close.start));
    let own_lines =
        model.starts_line(last.span.start) && model.line_start(last.span.end) < close_line;
    let text = if own_lines {
        format!(
            ",{}{}{entry}",
            layout.eol,
            spaces(model.column(last.span.start))
        )
    } else {
        format!(", {entry}")
    };
    Ok(vec![ByteEdit::insert(last.span.end, text)])
}

/// Edits that remove the entry `index` of the mapping `map`.
pub(crate) fn remove_entry(
    model: &YamlModel,
    map: NodeId,
    index: usize,
    layout: &Layout,
) -> Vec<ByteEdit> {
    let Some(mapping) = model.node(map).as_mapping() else {
        return Vec::new();
    };
    let Some(entry) = mapping.entries.get(index) else {
        return Vec::new();
    };
    if mapping.entries.len() == 1 {
        return clear(model, map, layout);
    }
    match mapping.style {
        CollectionStyle::Block => remove_block_entry(model, mapping, index, entry),
        _ => remove_flow_entry(model, mapping, index, entry),
    }
}

fn remove_block_entry(
    model: &YamlModel,
    mapping: &MappingNode,
    index: usize,
    entry: &MapEntry,
) -> Vec<ByteEdit> {
    let next = mapping.entries.get(index + 1);
    if let Some(next) = next.filter(|_| index == 0 && !model.starts_line(entry.span.start)) {
        return vec![ByteEdit::delete(Span::new(
            entry.span.start,
            next.span.start,
        ))];
    }
    let column = model.column(entry.span.start);
    let removal = Comments::new(model).removal_span(entry.span.clone(), Some(column + 1));
    vec![ByteEdit::delete(Span::new(removal.start, removal.end))]
}

fn remove_flow_entry(
    model: &YamlModel,
    mapping: &MappingNode,
    index: usize,
    entry: &MapEntry,
) -> Vec<ByteEdit> {
    if let Some(comma) = &entry.comma {
        let end = skip_blanks(model, comma.end);
        return vec![ByteEdit::delete(Span::new(entry.span.start, end))];
    }
    let previous_end = index
        .checked_sub(1)
        .and_then(|previous| mapping.entries.get(previous))
        .map_or(entry.span.start, |previous| previous.span.end);
    vec![ByteEdit::delete(Span::new(previous_end, entry.span.end))]
}

fn skip_blanks(model: &YamlModel, offset: usize) -> usize {
    let bytes = model.source().as_bytes();
    let mut end = offset;
    while matches!(bytes.get(end), Some(b' ' | b'\t')) {
        end += 1;
    }
    end
}

/// Edits that remove every entry of a mapping or every item of a sequence,
/// leaving `{}` or `[]` in place of the collection.
pub(crate) fn clear(model: &YamlModel, node: NodeId, layout: &Layout) -> Vec<ByteEdit> {
    let yaml = model.node(node);
    let (empty, spans, flow) = match &yaml.kind {
        NodeKind::Mapping(mapping) => (
            "{}",
            mapping
                .entries
                .iter()
                .map(|entry| entry.span.clone())
                .collect::<Vec<_>>(),
            mapping.delimiters.clone(),
        ),
        NodeKind::Sequence(sequence) => (
            "[]",
            sequence
                .items
                .iter()
                .map(|item| item.span.clone())
                .collect(),
            sequence.delimiters.clone(),
        ),
        _ => return Vec::new(),
    };
    if let Some(delimiters) = flow {
        return vec![ByteEdit::delete(Span::new(
            delimiters.open.end,
            delimiters.close.start,
        ))];
    }
    let (Some(first), Some(last)) = (spans.first(), spans.last()) else {
        return Vec::new();
    };
    match Place::of(model, node, layout) {
        Some(Place::Value { colon_end, .. }) if model.line_start(first.start) > colon_end => {
            let anchor = style::properties_end(model, node).unwrap_or(colon_end);
            let column = model.column(first.start);
            let removal =
                Comments::new(model).removal_span(first.start..last.end, Some(column + 1));
            vec![
                ByteEdit::insert(anchor, format!(" {empty}")),
                ByteEdit::delete(Span::new(removal.start, removal.end)),
            ]
        }
        _ => vec![ByteEdit::replace(
            Span::new(yaml.span.start, yaml.span.end),
            empty,
        )],
    }
}

/// Edits that give `node` the value `value`.
///
/// A scalar that receives a scalar keeps its style where possible
/// ([`scalar::replace`]). A node inside a flow collection, and a flow
/// collection, receives flow text. Any other node is rewritten as block
/// content: a mapping or list under a key starts on the next line, one step
/// deeper (lists at the column of the key when the document writes them so),
/// a mapping or list in a list item starts after `- `.
///
/// # Errors
///
/// `E_UNSUPPORTED_CONSTRUCT` `doc.out_of_scope_change` for a node in a
/// position that cannot hold block content (an explicit key or a value
/// without `:`), and the errors of [`scalar::new_text`].
pub(crate) fn replace(
    model: &YamlModel,
    node: NodeId,
    value: &Value,
    layout: &Layout,
) -> Result<Vec<ByteEdit>> {
    let yaml = model.node(node);
    if !is_block_collection(value) && yaml.as_scalar().is_some() {
        let (range, text) = scalar::replace(model, node, value, layout)?;
        return Ok(vec![ByteEdit::replace(
            Span::new(range.start, range.end),
            text,
        )]);
    }
    let flow_node = matches!(&yaml.kind, NodeKind::Mapping(m) if m.style != CollectionStyle::Block)
        || matches!(&yaml.kind, NodeKind::Sequence(s) if s.style != CollectionStyle::Block);
    if yaml.in_flow || flow_node {
        let text = style::separated(model, yaml.span.start, flow_value(value)?);
        return Ok(vec![ByteEdit::replace(
            Span::new(yaml.span.start, yaml.span.end),
            text,
        )]);
    }
    let Some(place) = Place::of(model, node, layout) else {
        return Err(crate::constructs::unsupported(
            crate::constructs::DOC_OUT_OF_SCOPE_CHANGE,
            "the node is an explicit key or a value without `:` and cannot receive block content",
        ));
    };
    replace_block(model, node, value, place, layout)
}

fn replace_block(
    model: &YamlModel,
    node: NodeId,
    value: &Value,
    place: Place,
    layout: &Layout,
) -> Result<Vec<ByteEdit>> {
    let yaml = model.node(node);
    let properties = style::properties_end(model, node);
    let (start, text) = match place {
        Place::Value { colon_end, column } => (
            properties.unwrap_or(colon_end),
            block_value(value, column, layout)?,
        ),
        Place::Item { dash_end, column } => (
            properties.unwrap_or(dash_end),
            format!(" {}", block_content(value, column, layout)?),
        ),
        Place::Root => match properties {
            Some(end) if is_block_collection(value) => (
                end,
                format!("{}{}", layout.eol, block_content(value, 0, layout)?),
            ),
            Some(end) => (end, format!(" {}", block_content(value, 0, layout)?)),
            None => (yaml.span.start, block_content(value, 0, layout)?),
        },
    };
    let mut end = yaml.span.end.max(start);
    let mut text = text;
    if yaml.as_scalar().is_some()
        && text.starts_with(layout.eol)
        && let Some(comment) = style::inline_comment(model, end)
    {
        text = format!(" {}{text}", model.text(&comment));
        end = comment.end;
    }
    Ok(vec![ByteEdit::replace(Span::new(start, end), text)])
}

fn is_block_collection(value: &Value) -> bool {
    match value {
        Value::Map(fields) => !fields.is_empty(),
        Value::List(items) => !items.is_empty(),
        _ => false,
    }
}

/// Text after `key:` for `value` under a key at `column`: a space and a
/// scalar, or a line break and nested block content.
///
/// # Errors
///
/// As for [`scalar::new_text`].
pub(crate) fn block_value(value: &Value, column: usize, layout: &Layout) -> Result<String> {
    match value {
        Value::Map(fields) if !fields.is_empty() => {
            let nested = layout.nested(column);
            Ok(format!(
                "{}{}{}",
                layout.eol,
                spaces(nested),
                block_entries(fields, nested, layout)?
            ))
        }
        Value::List(items) if !items.is_empty() => {
            let dash = layout.list_column(column);
            Ok(format!(
                "{}{}{}",
                layout.eol,
                spaces(dash),
                block_content(value, dash, layout)?
            ))
        }
        _ => Ok(format!(
            " {}",
            leaf_text(value, layout.nested(column), layout)?
        )),
    }
}

/// Block text of `value` whose first line follows an indicator or starts at
/// `column`, with further lines at `column`.
///
/// # Errors
///
/// As for [`scalar::new_text`].
pub(crate) fn block_content(value: &Value, column: usize, layout: &Layout) -> Result<String> {
    match value {
        Value::Map(fields) if !fields.is_empty() => block_entries(fields, column, layout),
        Value::List(items) if !items.is_empty() => {
            let lines = items
                .iter()
                .map(|item| {
                    Ok(format!(
                        "- {}",
                        block_content(item, column + ITEM_OFFSET, layout)?
                    ))
                })
                .collect::<Result<Vec<String>>>()?;
            Ok(lines.join(&format!("{}{}", layout.eol, spaces(column))))
        }
        _ => leaf_text(value, column.max(layout.unit), layout),
    }
}

fn block_entries(fields: &Fields, column: usize, layout: &Layout) -> Result<String> {
    let lines = fields
        .iter()
        .map(|(name, item)| {
            merge::check_new_key(name)?;
            Ok(format!(
                "{}:{}",
                key_text(name, false),
                block_value(item, column, layout)?
            ))
        })
        .collect::<Result<Vec<String>>>()?;
    Ok(lines.join(&format!("{}{}", layout.eol, spaces(column))))
}

fn leaf_text(value: &Value, indent: usize, layout: &Layout) -> Result<String> {
    match value {
        Value::Map(_) | Value::List(_) => flow_value(value),
        other => scalar::new_text(other, false, indent, layout.eol),
    }
}

/// Flow text of `value`: `{k: v}`, `[a, b]` or a scalar.
///
/// # Errors
///
/// As for [`scalar::new_text`].
pub(crate) fn flow_value(value: &Value) -> Result<String> {
    match value {
        Value::Map(fields) => {
            let parts = fields
                .iter()
                .map(|(name, item)| Ok(format!("{}: {}", key_text(name, true), flow_value(item)?)))
                .collect::<Result<Vec<String>>>()?;
            Ok(format!("{{{}}}", parts.join(", ")))
        }
        Value::List(items) => {
            let parts = items
                .iter()
                .map(flow_value)
                .collect::<Result<Vec<String>>>()?;
            Ok(format!("[{}]", parts.join(", ")))
        }
        other => scalar::new_text(other, true, 0, "\n"),
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::edit::apply;

    fn edited(text: &str, plan: impl Fn(&YamlModel, NodeId, &Layout) -> Vec<ByteEdit>) -> String {
        let model = YamlModel::parse(text).unwrap();
        let layout = Layout::of(&model);
        let edits = plan(&model, model.documents()[0].root, &layout);
        String::from_utf8(apply(text.as_bytes(), &edits).unwrap().bytes).unwrap()
    }

    fn map(pairs: &[(&str, Value)]) -> Value {
        Value::Map(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect::<IndexMap<_, _>>(),
        )
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    #[test]
    fn inserts_keys_after_the_last_entry_at_the_sibling_column() {
        let out = edited(
            "a:\n  b: 1\n  c:\n    d: 2\n# end\n",
            |model, root, layout| {
                let a = model.map_get(root, "a").unwrap();
                insert_entry(model, a, "e", &Value::Int(3), layout).unwrap()
            },
        );
        assert_eq!(out, "a:\n  b: 1\n  c:\n    d: 2\n  e: 3\n# end\n");
    }

    #[test]
    fn inserts_keys_into_flow_mappings() {
        let insert = |source: &str| {
            edited(source, |model, root, layout| {
                let m = model.map_get(root, "m").unwrap();
                insert_entry(model, m, "k", &text("v"), layout).unwrap()
            })
        };
        assert_eq!(insert("m: {}\n"), "m: {k: v}\n");
        assert_eq!(insert("m: {a: 1}\n"), "m: {a: 1, k: v}\n");
        assert_eq!(insert("m: {a: 1,}\n"), "m: {a: 1, k: v,}\n");
        assert_eq!(insert("m: {\n  a: 1\n}\n"), "m: {\n  a: 1,\n  k: v\n}\n");
    }

    #[test]
    fn writes_nested_values_as_block_content() {
        let out = edited("a: 1\n", |model, root, layout| {
            let value = map(&[
                (
                    "x",
                    Value::List(vec![Value::Int(1), map(&[("size", text("z"))])]),
                ),
                ("t", text("l1\nl2\n")),
            ]);
            insert_entry(model, root, "node", &value, layout).unwrap()
        });
        assert_eq!(
            out,
            "a: 1\nnode:\n  x:\n    - 1\n    - size: z\n  t: |\n    l1\n    l2\n"
        );
    }

    #[test]
    fn removes_entries_with_their_comments() {
        let out = edited(
            "a: 1\n# about b\nb: |\n  x\n  y\nc: 3 # c\n",
            |model, root, layout| remove_entry(model, root, 1, layout),
        );
        assert_eq!(out, "a: 1\nc: 3 # c\n");
        let out = edited("a: 1\nc: 3 # c\n", |model, root, layout| {
            remove_entry(model, root, 1, layout)
        });
        assert_eq!(out, "a: 1\n");
    }

    #[test]
    fn removing_the_only_entry_leaves_an_empty_mapping() {
        let out = edited("k:\n  a: 1\nz: 0\n", |model, root, layout| {
            let k = model.map_get(root, "k").unwrap();
            remove_entry(model, k, 0, layout)
        });
        assert_eq!(out, "k: {}\nz: 0\n");
        let out = edited("m: {a: 1}\n", |model, root, layout| {
            let m = model.map_get(root, "m").unwrap();
            remove_entry(model, m, 0, layout)
        });
        assert_eq!(out, "m: {}\n");
    }

    #[test]
    fn removes_flow_entries_with_their_commas() {
        let remove = |index: usize| {
            edited("m: {a: 1, b: 2, c: 3}\n", move |model, root, layout| {
                let m = model.map_get(root, "m").unwrap();
                remove_entry(model, m, index, layout)
            })
        };
        assert_eq!(remove(0), "m: {b: 2, c: 3}\n");
        assert_eq!(remove(2), "m: {a: 1, b: 2}\n");
    }

    #[test]
    fn removes_the_first_entry_of_a_mapping_in_a_list_item() {
        let out = edited("- a: 1\n  b: 2\n", |model, root, layout| {
            let item = model.node(root).as_sequence().unwrap().items[0].node;
            remove_entry(model, item, 0, layout)
        });
        assert_eq!(out, "- b: 2\n");
    }

    #[test]
    fn changes_the_kind_of_a_value() {
        let out = edited("a: 1 # note\nb: x\n", |model, root, layout| {
            let a = model.map_get(root, "a").unwrap();
            replace(model, a, &map(&[("k", Value::Bool(true))]), layout).unwrap()
        });
        assert_eq!(out, "a: # note\n  k: true\nb: x\n");
        let out = edited("a:\n  k: 1\nb: x\n", |model, root, layout| {
            let a = model.map_get(root, "a").unwrap();
            replace(model, a, &text("v"), layout).unwrap()
        });
        assert_eq!(out, "a: v\nb: x\n");
        let out = edited("a: &x\n  - 1\n", |model, root, layout| {
            let a = model.map_get(root, "a").unwrap();
            replace(model, a, &map(&[("k", Value::Int(2))]), layout).unwrap()
        });
        assert_eq!(out, "a: &x\n  k: 2\n");
    }

    #[test]
    fn rewrites_flow_values_in_flow_style() {
        let out = edited("m: {a: 1, b: [x]}\n", |model, root, layout| {
            let m = model.map_get(root, "m").unwrap();
            let b = model.map_get(m, "b").unwrap();
            replace(model, b, &map(&[("k", Value::List(Vec::new()))]), layout).unwrap()
        });
        assert_eq!(out, "m: {a: 1, b: {k: []}}\n");
    }

    #[test]
    fn writes_mappings_into_list_items_after_the_dash() {
        let out = edited("- 1\n- 2\n", |model, root, layout| {
            let item = model.node(root).as_sequence().unwrap().items[1].node;
            replace(
                model,
                item,
                &map(&[("a", Value::Int(1)), ("b", Value::Int(2))]),
                layout,
            )
            .unwrap()
        });
        assert_eq!(out, "- 1\n- a: 1\n  b: 2\n");
    }

    #[test]
    fn clears_block_lists_under_keys() {
        let out = edited("l:\n  - 1\n  - 2\nk: 0\n", |model, root, layout| {
            let l = model.map_get(root, "l").unwrap();
            clear(model, l, layout)
        });
        assert_eq!(out, "l: []\nk: 0\n");
    }

    #[test]
    fn refuses_single_pair_mappings_and_complex_keys() {
        let model = YamlModel::parse("[a: 1]\n").unwrap();
        let layout = Layout::of(&model);
        let item = model
            .node(model.documents()[0].root)
            .as_sequence()
            .unwrap()
            .items[0]
            .node;
        let error = insert_entry(&model, item, "b", &Value::Int(2), &layout).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict);
        let model = YamlModel::parse("a: 1\n").unwrap();
        let root = model.documents()[0].root;
        assert!(insert_entry(&model, root, "x\ny", &Value::Null, &layout).is_err());
    }
}
