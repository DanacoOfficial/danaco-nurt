// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Binding of YAML comments to nodes.
//!
//! A comment belongs to the node it describes:
//!
//! - a block of comment lines directly above the first line of an entry or
//!   sequence item, with no blank line in between and indented no deeper
//!   than the entry, is the *leading* comment of that entry;
//! - a comment after the content on the line of an entry is its *inline*
//!   comment;
//! - comment lines after the last item of a block collection, indented at
//!   least as deep as the items, are the *trailing* comment of the
//!   collection;
//! - comment lines before the first node of a document that are not the
//!   leading comment of that node are the comment of the document.
//!
//! The comment of a node reported in `item.comment` is its leading comment
//! followed by its inline comment. Removing an entry removes its lines, the
//! leading comment and, for a collection, the trailing comment; a comment
//! block separated from the entry by a blank line stays.
//!
//! Only spans reported by the parser are comments: a line of a block scalar
//! that starts with `#` is content.

use std::ops::Range;

use super::model::YamlModel;

/// Comments of one YAML text with the queries that bind them to nodes.
#[derive(Debug, Clone, Copy)]
pub struct Comments<'a> {
    model: &'a YamlModel,
    source: &'a str,
    spans: &'a [Range<usize>],
}

impl<'a> Comments<'a> {
    /// Comments of the stream described by `model`.
    pub fn new(model: &'a YamlModel) -> Self {
        Self {
            model,
            source: model.source(),
            spans: model.comments(),
        }
    }

    /// Text of a comment without `#`, one following space and trailing
    /// white space.
    pub fn text(&self, span: &Range<usize>) -> String {
        let raw = self.source.get(span.clone()).unwrap_or_default();
        let body = raw.strip_prefix('#').unwrap_or(raw);
        let body = body.strip_prefix(' ').unwrap_or(body);
        body.trim_end().to_owned()
    }

    /// Comment on the line of `offset` that starts at or after `offset`.
    pub fn inline_after(&self, offset: usize) -> Option<Range<usize>> {
        let end = self.model.line_end(offset);
        let first = self.spans.partition_point(|span| span.start < offset);
        self.spans
            .get(first)
            .filter(|span| span.start < end)
            .cloned()
    }

    /// Leading comment of the node whose first token starts at `start`: the
    /// comment-only lines directly above its line, top to bottom.
    ///
    /// Empty when the node is not the first token of its line (a key after
    /// `- ` belongs to the sequence item, which takes the comment). A comment
    /// line indented deeper than the node ends the block: it is the trailing
    /// comment of the collection above.
    pub fn leading(&self, start: usize) -> Vec<Range<usize>> {
        let line = self.model.line_start(start);
        if !self.source[line..start].trim().is_empty() {
            return Vec::new();
        }
        let column = start - line;
        let mut block = Vec::new();
        let mut cursor = line;
        while cursor > 0 {
            let previous = self.model.line_start(cursor - 1);
            match self.comment_line(previous) {
                Some(span) if span.start - previous <= column => {
                    block.push(span);
                    cursor = previous;
                }
                _ => break,
            }
        }
        block.reverse();
        block
    }

    /// Trailing comment of a block collection whose last item ends at
    /// `end`: comment-only lines after that item, without blank lines in
    /// between, indented by at least `column` bytes.
    pub fn trailing(&self, end: usize, column: usize) -> Vec<Range<usize>> {
        let mut block = Vec::new();
        let mut cursor = self.model.next_line_start(end);
        while cursor < self.source.len() {
            match self.comment_line(cursor) {
                Some(span) if span.start - cursor >= column => {
                    block.push(span);
                    cursor = self.model.next_line_start(cursor);
                }
                _ => break,
            }
        }
        block
    }

    /// Comment of a document: comment lines between `start` (start of the
    /// document text) and `first_node` (first token of its root node) that
    /// are not the leading comment of the root node.
    pub fn document(&self, start: usize, first_node: usize) -> Vec<Range<usize>> {
        let leading = self.leading(first_node);
        let bound = leading.first().map_or(first_node, |span| span.start);
        self.spans
            .iter()
            .filter(|span| span.start >= start && span.end <= bound)
            .filter(|span| {
                self.comment_line(self.model.line_start(span.start))
                    .is_some()
            })
            .cloned()
            .collect()
    }

    /// `item.comment` of a node whose first token starts at `start` and
    /// whose first line holds content up to `content_end`: the leading
    /// comment and the inline comment, one line each.
    pub fn describe(&self, start: usize, content_end: usize) -> Option<String> {
        let mut spans = self.leading(start);
        spans.extend(self.inline_after(content_end));
        self.join(&spans)
    }

    /// Texts of `spans` joined with line breaks; `None` for no spans.
    pub fn join(&self, spans: &[Range<usize>]) -> Option<String> {
        if spans.is_empty() {
            return None;
        }
        let lines: Vec<String> = spans.iter().map(|span| self.text(span)).collect();
        Some(lines.join("\n"))
    }

    /// Bytes removed with a block entry or item whose span is `entry`.
    ///
    /// The span grows to whole lines, including the line break after the
    /// last line, and to the leading comment. With `trailing_column`, the
    /// trailing comment of the collection held by the entry, indented by at
    /// least that many bytes, is removed as well.
    pub fn removal_span(
        &self,
        entry: Range<usize>,
        trailing_column: Option<usize>,
    ) -> Range<usize> {
        let leading = self.leading(entry.start);
        let start = leading.first().map_or_else(
            || self.model.line_start(entry.start),
            |span| self.model.line_start(span.start),
        );
        let last = entry.end.saturating_sub(1).max(entry.start);
        let mut end = self.model.next_line_start(last);
        if let Some(column) = trailing_column
            && let Some(span) = self.trailing(last, column).last()
        {
            end = self.model.next_line_start(span.start);
        }
        start..end
    }

    /// Span of the comment when the line starting at `line` holds only a
    /// comment.
    fn comment_line(&self, line: usize) -> Option<Range<usize>> {
        let end = self.model.line_end(line);
        let text = self.source.get(line..end)?;
        let indent = text.len() - text.trim_start().len();
        let start = line + indent;
        let index = self
            .spans
            .binary_search_by_key(&start, |span| span.start)
            .ok()?;
        Some(self.spans[index].clone())
    }
}
