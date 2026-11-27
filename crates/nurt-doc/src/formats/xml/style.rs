// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::xml::style
// Responsibility:   Layout conventions of a document, used for new content: line ending, indentation unit, attribute quote and line-oriented layout.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Layout conventions of a document, used for new content: line ending,
//! indentation unit, attribute quote and line-oriented layout.

use std::collections::HashMap;

use super::tree::{Node, NodeId, XmlTree};

/// Indentation unit used when the document shows none.
const DEFAULT_INDENT: &str = "  ";

/// Layout conventions detected in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlStyle {
    /// Dominant line ending, `"\n"` or `"\r\n"`.
    pub eol: &'static str,
    /// Indentation added per nesting level.
    pub indent_unit: String,
    /// Dominant attribute quote.
    pub quote: char,
    /// `true` when the document spans several lines; new elements of a
    /// single-line document are written without line breaks.
    pub multiline: bool,
}

impl Default for XmlStyle {
    fn default() -> Self {
        Self {
            eol: "\n",
            indent_unit: DEFAULT_INDENT.to_owned(),
            quote: '"',
            multiline: false,
        }
    }
}

/// Detects the layout conventions of a parsed tree.
pub(crate) fn detect(tree: &XmlTree) -> XmlStyle {
    let text = tree.text();
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    XmlStyle {
        eol: if crlf > lf { "\r\n" } else { "\n" },
        indent_unit: indent_unit(tree),
        quote: dominant_quote(tree),
        multiline: crlf + lf > 0,
    }
}

/// Most frequent attribute quote; `"` when there is no attribute.
fn dominant_quote(tree: &XmlTree) -> char {
    let (mut double, mut single) = (0usize, 0usize);
    for id in tree.node_ids() {
        if let Some(element) = tree.element(id) {
            for attribute in &element.attributes {
                if attribute.quote == '\'' {
                    single += 1;
                } else {
                    double += 1;
                }
            }
        }
    }
    if single > double { '\'' } else { '"' }
}

/// Most frequent difference between the indentation of an element and the
/// indentation of its parent.
fn indent_unit(tree: &XmlTree) -> String {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut uses_tabs = false;
    for id in tree.node_ids() {
        let (Some(parent), Some(child_indent)) = (tree.parent(id), tree.own_line_indent(id)) else {
            continue;
        };
        uses_tabs |= child_indent.contains('\t');
        let parent_indent = tree.line_indentation(tree.node(parent).span().start);
        if let Some(delta) = child_indent.strip_prefix(parent_indent)
            && !delta.is_empty()
        {
            *counts.entry(delta).or_default() += 1;
        }
    }
    let best = counts
        .into_iter()
        .max_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| b.0.len().cmp(&a.0.len()))
                .then_with(|| b.0.cmp(a.0))
        })
        .map(|(delta, _)| delta.to_owned());
    best.unwrap_or_else(|| {
        if uses_tabs {
            "\t".to_owned()
        } else {
            DEFAULT_INDENT.to_owned()
        }
    })
}

impl XmlTree {
    /// Start of the line that contains `offset`.
    pub(crate) fn line_start(&self, offset: usize) -> usize {
        self.text()[..offset]
            .rfind('\n')
            .map_or(0, |newline| newline + 1)
    }

    /// End of the line that contains `offset`, before its line ending.
    pub(crate) fn line_end(&self, offset: usize) -> usize {
        let end = self.text()[offset..]
            .find('\n')
            .map_or(self.text().len(), |newline| offset + newline);
        if end > offset && self.text()[..end].ends_with('\r') {
            end - 1
        } else {
            end
        }
    }

    /// Leading whitespace of the line that contains `offset`.
    pub(crate) fn line_indentation(&self, offset: usize) -> &str {
        let start = self.line_start(offset);
        let line = &self.text()[start..offset];
        let length = line.len() - line.trim_start_matches([' ', '\t']).len();
        &line[..length]
    }

    /// Indentation of a node that starts its own line; `None` when other
    /// content precedes it on the line.
    pub(crate) fn own_line_indent(&self, node: NodeId) -> Option<&str> {
        let start = self.node(node).span().start;
        let line_start = self.line_start(start);
        let before = &self.text()[line_start..start];
        before
            .chars()
            .all(|c| c == ' ' || c == '\t')
            .then_some(before)
    }

    /// Indentation for a new child of `parent`: the indentation of an
    /// existing child on its own line, otherwise the parent's indentation
    /// and one unit.
    pub(crate) fn child_indent(&self, parent: NodeId) -> String {
        let existing = self.children(parent).iter().find_map(|child| {
            matches!(
                self.node(*child),
                Node::Element(_) | Node::Comment(_) | Node::ProcessingInstruction(_)
            )
            .then(|| self.own_line_indent(*child))
            .flatten()
        });
        existing.map_or_else(
            || {
                format!(
                    "{}{}",
                    self.line_indentation(self.node(parent).span().start),
                    self.style().indent_unit
                )
            },
            str::to_owned,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equally_frequent_indentation_units_resolve_deterministically() {
        let tree = XmlTree::parse("<a>\n  <b/>\n\t\t<c/>\n</a>").unwrap();
        assert_eq!(tree.style().indent_unit, "\t\t");
    }

    #[test]
    fn dominant_indentation_unit_wins() {
        let tree = XmlTree::parse("<a>\n    <b/>\n    <c/>\n\t<d/>\n</a>").unwrap();
        assert_eq!(tree.style().indent_unit, "    ");
    }

    #[test]
    fn crlf_document_selects_crlf_line_ending() {
        let tree = XmlTree::parse("<a>\r\n  <b/>\r\n</a>").unwrap();
        assert_eq!(tree.style().eol, "\r\n");
        assert!(tree.style().multiline);
    }

    #[test]
    fn single_quoted_attributes_select_single_quote() {
        let tree = XmlTree::parse("<a x='1' y='2' z=\"3\"/>").unwrap();
        assert_eq!(tree.style().quote, '\'');
        assert!(!tree.style().multiline);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
