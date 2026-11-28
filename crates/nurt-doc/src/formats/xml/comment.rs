// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::xml::comment
// Responsibility:   Comments bound to elements.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Comments bound to elements.
//!
//! A comment is bound to the element that follows it when only whitespace
//! without a blank line separates them; consecutive bound comments form
//! one block. Comments, processing instructions and the document type
//! declaration are never rewritten by edits of other nodes.

use super::cdata::trim_xml;
use super::tree::{Node, NodeId, XmlTree};

impl XmlTree {
    /// Nodes on the same level as `node`, including `node`.
    pub(crate) fn siblings(&self, node: NodeId) -> &[NodeId] {
        match self.parent(node) {
            Some(parent) => self.children(parent),
            None => self.document_nodes(),
        }
    }

    /// Comments bound to `node`, in document order.
    pub(crate) fn bound_comments(&self, node: NodeId) -> Vec<NodeId> {
        let siblings = self.siblings(node);
        let Some(position) = siblings.iter().position(|sibling| *sibling == node) else {
            return Vec::new();
        };
        let mut bound = Vec::new();
        for sibling in siblings[..position].iter().rev() {
            match self.node(*sibling) {
                Node::Comment(_) => bound.push(*sibling),
                Node::Text(text) => {
                    let raw = &self.text()[text.span.clone()];
                    if !trim_xml(raw).is_empty() || raw.matches('\n').count() > 1 {
                        break;
                    }
                }
                _ => break,
            }
        }
        bound.reverse();
        bound
    }

    /// Text of the comments bound to `node`, without delimiters and
    /// surrounding whitespace, one comment per line.
    ///
    /// This is the `comment` field of an item read from the document.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse("<a>\n  <!-- pinned -->\n  <b/>\n\n  <!-- x -->\n\n  <c/>\n</a>")?;
    /// let mut elements = tree.child_elements(tree.root());
    /// let (b, c) = (elements.next().unwrap(), elements.next().unwrap());
    /// assert_eq!(tree.comment_before(b).as_deref(), Some("pinned"));
    /// assert_eq!(tree.comment_before(c), None);
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn comment_before(&self, node: NodeId) -> Option<String> {
        let texts: Vec<&str> = self
            .bound_comments(node)
            .into_iter()
            .filter_map(|id| self.comment_text(id))
            .collect();
        (!texts.is_empty()).then(|| texts.join("\n"))
    }

    /// Content of a comment without delimiters and surrounding whitespace;
    /// `None` for other nodes.
    pub fn comment_text(&self, node: NodeId) -> Option<&str> {
        match self.node(node) {
            Node::Comment(comment) => Some(trim_xml(&self.text()[comment.content.clone()])),
            _ => None,
        }
    }

    /// End of the last comment that follows `node` on the same line, or
    /// the end of `node`.
    pub(crate) fn end_with_line_comments(&self, node: NodeId) -> usize {
        let mut end = self.node(node).span().end;
        let siblings = self.siblings(node);
        let Some(position) = siblings.iter().position(|sibling| *sibling == node) else {
            return end;
        };
        for sibling in &siblings[position + 1..] {
            match self.node(*sibling) {
                Node::Comment(comment) => end = comment.span.end,
                Node::Text(text) => {
                    let raw = &self.text()[text.span.clone()];
                    if raw.contains('\n') || !trim_xml(raw).is_empty() {
                        break;
                    }
                }
                _ => break,
            }
        }
        end
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
