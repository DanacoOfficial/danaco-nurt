// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Layout of a parsed document: line structure, indentation, separators
//! and comment placement, used to give new content the style of its
//! neighbours.

use super::syntax::{Entry, NodeId, NodeKind, Span, Tree};

/// Indentation unit used when the document has no indented entry.
const DEFAULT_INDENT_UNIT: &str = "  ";

/// Separator between a member name and its value when the document has no
/// member to copy it from.
const DEFAULT_COLON: &str = ": ";

/// Style of new content, derived from the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Style {
    /// One level of indentation.
    pub(crate) indent_unit: String,
    /// Line ending: `\n` or `\r\n`, whichever dominates.
    pub(crate) eol: &'static str,
    /// Text between a member name and its value, for example `": "`.
    pub(crate) colon: String,
    /// Space after a comma between entries on one line.
    pub(crate) comma_space: String,
    /// Space inside the brackets of a single-line container.
    pub(crate) bracket_space: String,
    /// Multi-line containers end with a trailing comma (JSONC).
    pub(crate) trailing_comma: bool,
}

impl Style {
    /// Style for a scalar value, which has no entries to lay out; it needs
    /// no scan of the document.
    pub(crate) const fn scalar() -> Self {
        Self {
            indent_unit: String::new(),
            eol: "\n",
            colon: String::new(),
            comma_space: String::new(),
            bracket_space: String::new(),
            trailing_comma: false,
        }
    }
}

/// Read access to the layout of one document.
pub(crate) struct Layout<'d> {
    pub(crate) text: &'d str,
    pub(crate) tree: &'d Tree,
}

impl<'d> Layout<'d> {
    pub(crate) const fn new(text: &'d str, tree: &'d Tree) -> Self {
        Self { text, tree }
    }

    /// Offset of the start of the line containing `offset`.
    pub(crate) fn line_start(&self, offset: usize) -> usize {
        let newlines = &self.tree.newlines;
        let before = newlines.partition_point(|at| *at < offset);
        before.checked_sub(1).map_or(0, |index| newlines[index] + 1)
    }

    /// Offset of the first line feed at or after `offset`.
    fn next_newline(&self, offset: usize) -> Option<usize> {
        let newlines = &self.tree.newlines;
        newlines
            .get(newlines.partition_point(|at| *at < offset))
            .copied()
    }

    /// Offset of the line break ending the line containing `offset`, or the
    /// end of the text; a `\r` before the `\n` belongs to the line break.
    pub(crate) fn line_end(&self, offset: usize) -> usize {
        let end = self.next_newline(offset).unwrap_or(self.text.len());
        if end > offset && self.text.as_bytes()[end - 1] == b'\r' && end < self.text.len() {
            end - 1
        } else {
            end
        }
    }

    /// Offset after the line break ending the line containing `offset`.
    pub(crate) fn next_line_start(&self, offset: usize) -> usize {
        self.next_newline(offset)
            .map_or(self.text.len(), |pos| pos + 1)
    }

    /// Leading whitespace of the line containing `offset`.
    pub(crate) fn indent_of(&self, offset: usize) -> &'d str {
        let start = self.line_start(offset);
        let line = &self.text[start..];
        let width = line.len() - line.trim_start_matches([' ', '\t']).len();
        &line[..width]
    }

    /// The text before `offset` on its line is whitespace only.
    pub(crate) fn starts_line(&self, offset: usize) -> bool {
        self.text[self.line_start(offset)..offset]
            .bytes()
            .all(|b| b == b' ' || b == b'\t')
    }

    /// The span contains a line break.
    pub(crate) fn is_multiline(&self, span: Span) -> bool {
        let newlines = &self.tree.newlines;
        let first = newlines.partition_point(|offset| *offset < span.start);
        newlines.get(first).is_some_and(|offset| *offset < span.end)
    }

    /// Comments that start inside the span.
    pub(crate) fn comments_in(&self, span: Span) -> impl Iterator<Item = Span> + '_ {
        let first = self.tree.comments.partition_point(|c| c.start < span.start);
        self.tree.comments[first..]
            .iter()
            .copied()
            .take_while(move |c| c.start < span.end)
    }

    /// Skips spaces, tabs and comments that start on the current line and
    /// returns the offset of the first other character.
    pub(crate) fn skip_same_line_trivia(&self, mut pos: usize) -> usize {
        let bytes = self.text.as_bytes();
        loop {
            match bytes.get(pos) {
                Some(b' ' | b'\t') => pos += 1,
                Some(b'/') => match self.comment_at(pos) {
                    Some(comment) => pos = comment.end,
                    None => return pos,
                },
                _ => return pos,
            }
        }
    }

    /// Comment starting exactly at `pos`.
    pub(crate) fn comment_at(&self, pos: usize) -> Option<Span> {
        let index = self
            .tree
            .comments
            .binary_search_by_key(&pos, |c| c.start)
            .ok()?;
        Some(self.tree.comments[index])
    }

    /// Comments directly above the entry starting at `start`: each one is
    /// alone on its lines and ends on the line before the next one or the
    /// entry. A blank line ends the block.
    pub(crate) fn leading_comments(&self, start: usize) -> Option<Span> {
        let first = self.tree.comments.partition_point(|c| c.start < start);
        let mut block_start = self.line_start(start);
        let mut found = None;
        for comment in self.tree.comments[..first].iter().rev() {
            let attached = self.next_line_start(comment.end.saturating_sub(1)) == block_start
                && self.starts_line(comment.start)
                && self.text[comment.end..self.line_end(comment.end)]
                    .trim()
                    .is_empty();
            if !attached {
                break;
            }
            block_start = self.line_start(comment.start);
            found = Some(Span::new(
                comment.start,
                found.map_or(comment.end, |span: Span| span.end),
            ));
        }
        found
    }

    /// Comment on the same line after the end of an entry, its comma
    /// included.
    pub(crate) fn trailing_comment(&self, entry: &Entry) -> Option<Span> {
        let after = entry
            .comma
            .map_or(self.tree.nodes[entry.value].span.end, |comma| comma + 1);
        let mut pos = after;
        let bytes = self.text.as_bytes();
        while matches!(bytes.get(pos), Some(b' ' | b'\t')) {
            pos += 1;
        }
        self.comment_at(pos)
    }

    /// Style for content added to the container `container`.
    pub(crate) fn style_for(&self, container: NodeId) -> Style {
        self.tree.styles.get_or_compute(container, || Style {
            indent_unit: self.indent_unit(),
            eol: self.eol(),
            colon: self.colon(container),
            comma_space: self.comma_space(container),
            bracket_space: self.bracket_space(),
            trailing_comma: self.trailing_comma(container),
        })
    }

    fn eol(&self) -> &'static str {
        if self.tree.crlf_majority {
            "\r\n"
        } else {
            "\n"
        }
    }

    /// Difference between the indentation of the first entry of a
    /// multi-line container and the indentation of the container line.
    fn indent_unit(&self) -> String {
        for node in &self.tree.nodes {
            let Some(entry) = node.entries().and_then(<[Entry]>::first) else {
                continue;
            };
            let start = entry.start(self.tree);
            if !self.starts_line(start)
                || self.line_start(start) == self.line_start(node.span.start)
            {
                continue;
            }
            let inner = self.indent_of(start);
            if let Some(unit) = inner.strip_prefix(self.indent_of(node.span.start))
                && !unit.is_empty()
            {
                return unit.to_owned();
            }
        }
        DEFAULT_INDENT_UNIT.to_owned()
    }

    /// Separator text of the nearest member: the container first, then the
    /// whole document.
    fn colon(&self, container: NodeId) -> String {
        let own = self.tree.nodes[container].entries().unwrap_or_default();
        let all = self.tree.nodes.iter().filter_map(|node| match &node.kind {
            NodeKind::Object(entries) => Some(entries.as_slice()),
            _ => None,
        });
        std::iter::once(own)
            .chain(all)
            .flat_map(|entries| entries.iter())
            .find_map(|entry| {
                let key = entry.key.as_ref()?;
                let between = &self.text[key.span.end..self.tree.nodes[entry.value].span.start];
                (!between.contains(['\n', '/'])).then(|| between.to_owned())
            })
            .unwrap_or_else(|| DEFAULT_COLON.to_owned())
    }

    /// Whitespace after a comma that is followed by an entry on the same
    /// line.
    fn comma_space(&self, container: NodeId) -> String {
        let own = std::iter::once(container);
        let all = 0..self.tree.nodes.len();
        own.chain(all)
            .find_map(|id| {
                let entries = self.tree.nodes[id].entries()?;
                entries.windows(2).find_map(|pair| {
                    let comma = pair[0].comma?;
                    let between = &self.text[comma + 1..pair[1].start(self.tree)];
                    between
                        .bytes()
                        .all(|b| b == b' ' || b == b'\t')
                        .then(|| between.to_owned())
                })
            })
            .unwrap_or_else(|| {
                if self.colon(container).ends_with(' ') {
                    " ".to_owned()
                } else {
                    String::new()
                }
            })
    }

    /// Space inside the brackets of the first non-empty single-line
    /// container.
    fn bracket_space(&self) -> String {
        self.tree
            .nodes
            .iter()
            .find(|node| {
                node.entries().is_some_and(|e| !e.is_empty()) && !self.is_multiline(node.span)
            })
            .map(|node| {
                let inner = &self.text[node.span.start + 1..];
                let width = inner.len() - inner.trim_start_matches([' ', '\t']).len();
                inner[..width].to_owned()
            })
            .unwrap_or_default()
    }

    /// Trailing comma style: the container itself when it has entries,
    /// otherwise the first non-empty multi-line container of the document.
    fn trailing_comma(&self, container: NodeId) -> bool {
        let has_trailing = |id: NodeId| {
            let node = &self.tree.nodes[id];
            node.entries()
                .and_then(<[Entry]>::last)
                .filter(|_| self.is_multiline(node.span))
                .map(|last| last.comma.is_some())
        };
        has_trailing(container)
            .or_else(|| (0..self.tree.nodes.len()).find_map(has_trailing))
            .unwrap_or(false)
    }
}

/// Text of a comment without its markers, trimmed.
pub(crate) fn comment_text(comment: &str) -> &str {
    comment
        .strip_prefix("//")
        .or_else(|| {
            comment
                .strip_prefix("/*")
                .map(|inner| inner.strip_suffix("*/").unwrap_or(inner))
        })
        .unwrap_or(comment)
        .trim()
}
