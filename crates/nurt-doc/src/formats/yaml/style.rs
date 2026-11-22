//! Layout of new block content: indentation step, placement of lists under
//! keys, line break, and the columns where new content of a node starts.
//!
//! The indentation step is the smallest indentation of a block mapping
//! relative to the block mapping that holds it, or else the indentation unit
//! of the text, or two spaces. A list under a key is written at the column
//! of the key when the first block sequence under a key of a block mapping
//! is written that way (`key:\n- a`), and one step deeper otherwise.

use std::ops::Range;

use super::model::{CollectionStyle, NodeId, NodeKind, YamlModel};
use crate::style::{Indent, StyleProbe};

/// Indentation step of a document without nested block mappings or
/// indented lines.
pub(crate) const DEFAULT_INDENT: usize = 2;

/// Largest indentation step taken from the indentation unit of the text.
const MAX_INDENT: usize = 8;

/// Columns between a `-` indicator and the content of its item.
pub(crate) const ITEM_OFFSET: usize = 2;

/// Layout rules for new block content of one document text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Layout {
    /// Indentation step in spaces.
    pub(crate) unit: usize,
    /// Lists under a key start at the column of the key.
    pub(crate) flush_lists: bool,
    /// Line break of new lines.
    pub(crate) eol: &'static str,
}

impl Layout {
    /// Layout of the text of `model`.
    pub(crate) fn of(model: &YamlModel) -> Self {
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
            unit: unit.unwrap_or_else(|| text_unit(model)),
            flush_lists: flush_lists.unwrap_or(false),
            eol: model.line_ending().as_str(),
        }
    }

    /// Column of the content nested one level under a key at `column`.
    pub(crate) const fn nested(&self, column: usize) -> usize {
        column + self.unit
    }

    /// Column of the `-` indicators of a list under a key at `column`.
    pub(crate) const fn list_column(&self, column: usize) -> usize {
        if self.flush_lists {
            column
        } else {
            column + self.unit
        }
    }
}

fn text_unit(model: &YamlModel) -> usize {
    match StyleProbe::new(model.source().as_bytes()).indent_unit() {
        Some(Indent::Spaces(width)) if (1..=MAX_INDENT).contains(&width) => width,
        _ => DEFAULT_INDENT,
    }
}

/// Spaces of an indentation of `count` columns.
pub(crate) fn spaces(count: usize) -> String {
    " ".repeat(count)
}

/// Column of the start of a mapping entry: its `?` indicator or its key.
pub(crate) fn key_column(model: &YamlModel, map: NodeId, index: usize) -> usize {
    model
        .node(map)
        .as_mapping()
        .and_then(|mapping| mapping.entries.get(index))
        .map_or(0, |entry| model.column(entry.span.start))
}

/// Column where block content held by `node` starts: one step under the key
/// of a mapping entry, after `- ` of a sequence item, one step from the left
/// margin for a document root.
pub(crate) fn content_column(model: &YamlModel, node: NodeId, layout: &Layout) -> usize {
    if let Some((index, _)) = model.entry_position(node)
        && let Some(map) = model.parent(node)
    {
        return layout.nested(key_column(model, map, index));
    }
    if let Some((_, item)) = model.item_of(node)
        && let Some(dash) = &item.dash
    {
        return model.column(dash.start) + ITEM_OFFSET;
    }
    let span = &model.node(node).span;
    if model.parent(node).is_some() && !span.is_empty() {
        return layout.nested(model.line_indent(span.start));
    }
    layout.unit
}

/// End of the anchor and tag of `node`, when it has either.
pub(crate) fn properties_end(model: &YamlModel, node: NodeId) -> Option<usize> {
    let yaml = model.node(node);
    let anchor = yaml.anchor.as_ref().map(|anchor| anchor.span.end);
    let tag = yaml.tag.as_ref().map(|tag| tag.span.end);
    anchor.max(tag)
}

/// Range of a comment that follows `offset` on the same line after blanks.
pub(crate) fn inline_comment(model: &YamlModel, offset: usize) -> Option<Range<usize>> {
    let line_end = model.line_end(offset);
    let rest = model.source().get(offset..line_end)?;
    let blanks = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    let start = offset + blanks;
    (blanks > 0 && rest[blanks..].starts_with('#')).then_some(start..line_end)
}

/// `text` with a separating space when it is written right after an
/// indicator (`key:`, `-`, `?`) or a property.
pub(crate) fn separated(model: &YamlModel, offset: usize, text: String) -> String {
    let before = offset
        .checked_sub(1)
        .and_then(|previous| model.source().as_bytes().get(previous));
    let needs_space = !text.is_empty()
        && before.is_some_and(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'[' | b'{'));
    if needs_space {
        format!(" {text}")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(model: &YamlModel) -> NodeId {
        model.documents()[0].root
    }

    #[test]
    fn detects_the_indentation_step_and_list_placement() {
        let model = YamlModel::parse("a:\n    b: 1\nl:\n- x\n").unwrap();
        let layout = Layout::of(&model);
        assert_eq!(layout.unit, 4);
        assert!(layout.flush_lists);
        assert_eq!(layout.list_column(2), 2);
        let model = YamlModel::parse("l:\n  - x\nk: v\r\n").unwrap();
        let layout = Layout::of(&model);
        assert_eq!(layout.unit, 2);
        assert!(!layout.flush_lists);
        assert_eq!(layout.list_column(0), 2);
    }

    #[test]
    fn uses_the_line_break_of_the_text() {
        let model = YamlModel::parse("a: 1\r\nb: 2\r\n").unwrap();
        assert_eq!(Layout::of(&model).eol, "\r\n");
        assert_eq!(Layout::of(&model).unit, DEFAULT_INDENT);
    }

    #[test]
    fn finds_content_columns_of_values_and_items() {
        let model = YamlModel::parse("a:\n  b: x\nl:\n  - y\n").unwrap();
        let layout = Layout::of(&model);
        let a = model.map_get(root(&model), "a").unwrap();
        let b = model.map_get(a, "b").unwrap();
        assert_eq!(content_column(&model, b, &layout), 4);
        let l = model.map_get(root(&model), "l").unwrap();
        let item = model.node(l).as_sequence().unwrap().items[0].node;
        assert_eq!(content_column(&model, item, &layout), 4);
        assert_eq!(content_column(&model, root(&model), &layout), 2);
    }

    #[test]
    fn finds_inline_comments_and_properties() {
        let model = YamlModel::parse("k: &a !t v # note\n").unwrap();
        let value = model.map_get(root(&model), "k").unwrap();
        let end = model.node(value).span.end;
        assert_eq!(model.text(&inline_comment(&model, end).unwrap()), "# note");
        assert_eq!(properties_end(&model, value), Some(8));
        assert!(inline_comment(&model, 1).is_none());
    }

    #[test]
    fn separates_text_written_after_an_indicator() {
        let model = YamlModel::parse("k:\n").unwrap();
        assert_eq!(separated(&model, 2, "v".to_owned()), " v");
        assert_eq!(separated(&model, 0, "v".to_owned()), "v");
    }
}
