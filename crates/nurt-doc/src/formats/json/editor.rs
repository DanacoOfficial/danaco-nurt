// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Byte edits that replace, insert and remove values of a parsed document.
//!
//! Every operation produces replacements of byte spans of the original
//! text; nothing outside these spans changes. New content takes the style
//! of its neighbours (see [`Layout::style_for`]).

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::value::Value;

use super::layout::{Layout, Style};
use super::render;
use super::syntax::{Entry, NodeId, Span};
use crate::value::to_document_value;

/// Replacement of one byte span of the original text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) span: Span,
    pub(crate) text: String,
}

impl Change {
    fn insert(at: usize, text: String) -> Self {
        Self {
            span: Span::new(at, at),
            text,
        }
    }

    fn delete(span: Span) -> Self {
        Self {
            span,
            text: String::new(),
        }
    }
}

/// Applies non-overlapping changes to `text`; changes at the same offset
/// keep their order.
///
/// # Errors
///
/// `E_KEY_CONFLICT` when two changes overlap.
pub(crate) fn apply(text: &str, changes: &[Change]) -> Result<String, Error> {
    let mut sorted: Vec<&Change> = changes.iter().collect();
    sorted.sort_by_key(|change| (change.span.start, change.span.end));
    let mut out =
        String::with_capacity(text.len() + changes.iter().map(|c| c.text.len()).sum::<usize>());
    let mut cursor = 0;
    for change in sorted {
        if change.span.start < cursor {
            return Err(Error::new(
                ErrorCode::KeyConflict,
                "two changes overlap in the document; set the keys in separate commands",
            ));
        }
        out.push_str(&text[cursor..change.span.start]);
        out.push_str(&change.text);
        cursor = change.span.end;
    }
    out.push_str(&text[cursor..]);
    Ok(out)
}

/// Single change that turns `old` into `new`: the region between their
/// common prefix and common suffix, on character boundaries.
pub(crate) fn difference(old: &str, new: &str) -> Change {
    let (a, b) = (old.as_bytes(), new.as_bytes());
    let mut prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    while !(old.is_char_boundary(prefix) && new.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    let limit = a.len().min(b.len()) - prefix;
    let mut suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(limit)
        .take_while(|(x, y)| x == y)
        .count();
    while !(old.is_char_boundary(a.len() - suffix) && new.is_char_boundary(b.len() - suffix)) {
        suffix -= 1;
    }
    Change {
        span: Span::new(prefix, a.len() - suffix),
        text: new[prefix..b.len() - suffix].to_owned(),
    }
}

/// Edit operations on one parsed document.
pub(crate) struct Editor<'d> {
    layout: Layout<'d>,
}

impl<'d> Editor<'d> {
    pub(crate) const fn new(layout: Layout<'d>) -> Self {
        Self { layout }
    }

    /// Replaces the value of `node` with `value`.
    ///
    /// # Errors
    ///
    /// `E_INVALID_VALUE` when the value has no JSON form.
    pub(crate) fn replace(&self, node: NodeId, value: &Value) -> Result<Vec<Change>, Error> {
        let tree = self.layout.tree;
        let span = tree.nodes[node].span;
        let style_source = tree.nodes[node].parent.unwrap_or(node);
        let style = if matches!(
            to_document_value(value.clone()),
            Value::Map(_) | Value::List(_)
        ) {
            self.layout.style_for(style_source)
        } else {
            Style::scalar()
        };
        let multiline = self.context_multiline(node);
        let indent = self.layout.indent_of(span.start);
        let text = render::render(value, indent, &style, multiline)?;
        Ok(vec![Change { span, text }])
    }

    /// Adds a member to the end of an object (`key` set) or an element to
    /// the end of an array (`key` unset).
    ///
    /// # Errors
    ///
    /// `E_INVALID_VALUE` when the value has no JSON form.
    pub(crate) fn append(
        &self,
        container: NodeId,
        key: Option<&str>,
        value: &Value,
    ) -> Result<Vec<Change>, Error> {
        let entries = self.layout.tree.nodes[container]
            .entries()
            .unwrap_or_default();
        let style = self.layout.style_for(container);
        match entries.last() {
            None => self.append_to_empty(container, key, value, &style),
            Some(last)
                if self
                    .layout
                    .is_multiline(self.layout.tree.nodes[container].span) =>
            {
                self.append_on_new_line(last, key, value, &style)
            }
            Some(last) => self.append_inline(last, key, value, &style),
        }
    }

    /// Removes entry `index` of an object or array, with its separator and
    /// its comments: comment lines directly above it and a comment after it
    /// on the same line. Comments separated by a blank line stay.
    pub(crate) fn remove(&self, container: NodeId, index: usize) -> Vec<Change> {
        let tree = self.layout.tree;
        let entries = tree.nodes[container].entries().unwrap_or_default();
        let Some(entry) = entries.get(index) else {
            return Vec::new();
        };
        let range = self.whole_lines(container, entries, index);
        if entries.len() == 1 {
            let span = tree.nodes[container].span;
            let interior = Span::new(span.start + 1, span.end - 1);
            let removed =
                range.unwrap_or_else(|| Span::new(entry.start(tree), self.entry_end(entry)));
            let other_comments = self
                .layout
                .comments_in(interior)
                .any(|c| c.start < removed.start || c.end > removed.end);
            if !other_comments {
                return vec![Change::delete(interior)];
            }
            return vec![Change::delete(removed)];
        }
        match range {
            Some(lines) => {
                let mut changes = vec![Change::delete(lines)];
                if index + 1 == entries.len()
                    && entry.comma.is_none()
                    && let Some(comma) = entries[index - 1].comma
                {
                    changes.push(Change::delete(Span::new(comma, comma + 1)));
                }
                changes
            }
            None => vec![Change::delete(self.inline_range(entries, index))],
        }
    }

    /// Removes the entries at the ascending positions `indices` of one
    /// container in one step: each run of adjacent entries goes with its
    /// separator as a unit, and the runs are separated by entries that stay,
    /// so the changes do not overlap and need no repeated reading of the
    /// text. The result is the one of removing the entries one at a time
    /// ([`Editor::remove`]), last first: in a container with comments, the
    /// comments directly above an entry and after it on its line go with it,
    /// and a comment of neither kind between two removed entries ends the
    /// run and stays. Returns `None` for an arrangement that needs the
    /// entries one at a time: removed entries that share their lines with
    /// other content in a container whose entries all go, or at the end of a
    /// container with a comment between them.
    pub(crate) fn remove_runs(&self, container: NodeId, indices: &[usize]) -> Option<Vec<Change>> {
        let tree = self.layout.tree;
        let span = tree.nodes[container].span;
        let interior = Span::new(span.start + 1, span.end - 1);
        let entries = tree.nodes[container].entries().unwrap_or_default();
        let commented = self.layout.comments_in(interior).next().is_some();
        let runs = self.runs(entries, indices, commented);
        if commented {
            return self.commented_runs(container, entries, interior, &runs);
        }
        let mut changes = Vec::new();
        for (first, last) in runs {
            changes.extend(self.run(span, entries, container, first, last)?);
        }
        Some(changes)
    }

    /// Maximal runs `(first, last)` of adjacent positions in `indices`. With
    /// `commented`, a comment between two entries that goes with neither of
    /// them ends the run, because it stays when the entries go.
    fn runs(&self, entries: &[Entry], indices: &[usize], commented: bool) -> Vec<(usize, usize)> {
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for &index in indices {
            match runs.last_mut() {
                Some((_, last))
                    if *last + 1 == index
                        && (!commented || self.comments_go_with_entries(entries, *last)) =>
                {
                    *last = index;
                }
                _ => runs.push((index, index)),
            }
        }
        runs
    }

    /// Every comment between the entry `index` and the next one is a comment
    /// after the entry on its line or one of the lines directly above the
    /// next entry.
    fn comments_go_with_entries(&self, entries: &[Entry], index: usize) -> bool {
        let layout = &self.layout;
        let from = self.entry_end(&entries[index]);
        let next = entries[index + 1].start(layout.tree);
        let leading = layout.leading_comments(next);
        let line = layout.line_start(from);
        layout.comments_in(Span::new(from, next)).all(|comment| {
            leading.is_some_and(|block| comment.start >= block.start)
                || layout.line_start(comment.start) == line
        })
    }

    /// Changes that remove `runs` of entries from a container with comments.
    fn commented_runs(
        &self,
        container: NodeId,
        entries: &[Entry],
        interior: Span,
        runs: &[(usize, usize)],
    ) -> Option<Vec<Change>> {
        let layout = &self.layout;
        let count: usize = runs.iter().map(|(first, last)| last - first + 1).sum();
        if count == entries.len() {
            let mut lines = Vec::with_capacity(runs.len());
            for &(first, last) in runs {
                lines.push(self.whole_run_lines(container, entries, first, last)?);
            }
            let stays = layout.comments_in(interior).any(|comment| {
                let at = lines.partition_point(|range| range.start <= comment.start);
                !at.checked_sub(1)
                    .is_some_and(|found| comment.end <= lines[found].end)
            });
            return Some(if stays {
                lines.into_iter().map(Change::delete).collect()
            } else {
                vec![Change::delete(interior)]
            });
        }
        let mut changes = Vec::new();
        for (position, &(first, last)) in runs.iter().enumerate() {
            let at_end = last + 1 == entries.len();
            if at_end && position > 0 && runs[position - 1].1 + 1 == first {
                return None;
            }
            match self.whole_run_lines(container, entries, first, last) {
                Some(lines) => {
                    changes.push(Change::delete(lines));
                    if at_end
                        && entries[last].comma.is_none()
                        && let Some(comma) =
                            first.checked_sub(1).and_then(|index| entries[index].comma)
                    {
                        changes.push(Change::delete(Span::new(comma, comma + 1)));
                    }
                }
                // A comment inside a run that shares its line with other
                // content stays when the entries go one at a time.
                None if first == last => {
                    changes.push(Change::delete(self.inline_run(entries, first, last)?));
                }
                None => return None,
            }
        }
        Some(changes)
    }

    /// Changes that remove the entries `first..=last` of a container without
    /// comments.
    fn run(
        &self,
        span: Span,
        entries: &[Entry],
        container: NodeId,
        first: usize,
        last: usize,
    ) -> Option<Vec<Change>> {
        if first == 0 && last + 1 == entries.len() {
            return Some(vec![Change::delete(Span::new(
                span.start + 1,
                span.end - 1,
            ))]);
        }
        let previous = first.checked_sub(1).map(|index| &entries[index]);
        let Some(lines) = self.whole_run_lines(container, entries, first, last) else {
            return Some(vec![Change::delete(self.inline_run(entries, first, last)?)]);
        };
        let mut changes = vec![Change::delete(lines)];
        if last + 1 == entries.len()
            && entries[last].comma.is_none()
            && let Some(comma) = previous.and_then(|entry| entry.comma)
        {
            changes.push(Change::delete(Span::new(comma, comma + 1)));
        }
        Some(changes)
    }

    /// Lines occupied only by the entries `first..=last`, when the container
    /// spans several lines.
    fn whole_run_lines(
        &self,
        container: NodeId,
        entries: &[Entry],
        first: usize,
        last: usize,
    ) -> Option<Span> {
        let layout = &self.layout;
        let start = entries[first].start(layout.tree);
        if !layout.is_multiline(layout.tree.nodes[container].span) || !layout.starts_line(start) {
            return None;
        }
        if last + 1 < entries.len() && entries[last].comma.is_none() {
            return None;
        }
        let after = layout.skip_same_line_trivia(self.entry_end(&entries[last]));
        if !matches!(
            layout.text.as_bytes().get(after),
            None | Some(b'\r' | b'\n')
        ) {
            return None;
        }
        let first_line = layout
            .leading_comments(start)
            .map_or(start, |block| block.start);
        Some(Span::new(
            layout.line_start(first_line),
            layout.next_line_start(after),
        ))
    }

    /// Range of the entries `first..=last` that share their lines with other
    /// content: the entries with the separator on one side.
    fn inline_run(&self, entries: &[Entry], first: usize, last: usize) -> Option<Span> {
        let tree = self.layout.tree;
        let previous_comma = first.checked_sub(1).and_then(|index| entries[index].comma);
        let end = &entries[last];
        Some(match (end.comma, previous_comma) {
            (Some(comma), Some(previous)) => Span::new(previous + 1, comma + 1),
            (Some(comma), None) => {
                let after = self.layout.skip_same_line_trivia(comma + 1);
                let stop = match self.layout.text.as_bytes().get(after) {
                    Some(b'\r' | b'\n') => after,
                    _ => entries.get(last + 1).map_or(after, |next| next.start(tree)),
                };
                Span::new(entries[first].start(tree), stop)
            }
            (None, _) => {
                let before = entries.get(first.checked_sub(1)?)?;
                Span::new(
                    tree.nodes[before.value].span.end,
                    tree.nodes[end.value].span.end,
                )
            }
        })
    }

    /// Offset after the entry value and its comma.
    fn entry_end(&self, entry: &Entry) -> usize {
        entry
            .comma
            .map_or(self.layout.tree.nodes[entry.value].span.end, |comma| {
                comma + 1
            })
    }

    /// Lines occupied only by the entry and its comments, when the
    /// container spans several lines.
    fn whole_lines(&self, container: NodeId, entries: &[Entry], index: usize) -> Option<Span> {
        let layout = &self.layout;
        let entry = &entries[index];
        let start = entry.start(layout.tree);
        if !layout.is_multiline(layout.tree.nodes[container].span) || !layout.starts_line(start) {
            return None;
        }
        if index + 1 < entries.len() && entry.comma.is_none() {
            return None;
        }
        let after = layout.skip_same_line_trivia(self.entry_end(entry));
        if !matches!(
            layout.text.as_bytes().get(after),
            None | Some(b'\r' | b'\n')
        ) {
            return None;
        }
        let first_line = layout.leading_comments(start).map_or_else(
            || layout.line_start(start),
            |comments| layout.line_start(comments.start),
        );
        Some(Span::new(first_line, layout.next_line_start(after)))
    }

    /// Range of an entry that shares its line with other content: the
    /// entry with the separator on one side.
    fn inline_range(&self, entries: &[Entry], index: usize) -> Span {
        let tree = self.layout.tree;
        let entry = &entries[index];
        let value_end = tree.nodes[entry.value].span.end;
        let previous_comma = index.checked_sub(1).and_then(|i| entries[i].comma);
        match (entry.comma, previous_comma) {
            (Some(comma), Some(previous)) => Span::new(previous + 1, comma + 1),
            (Some(comma), None) => {
                let after = self.layout.skip_same_line_trivia(comma + 1);
                let end = match self.layout.text.as_bytes().get(after) {
                    Some(b'\r' | b'\n') => after,
                    _ => entries
                        .get(index + 1)
                        .map_or(after, |next| next.start(tree)),
                };
                Span::new(entry.start(tree), end)
            }
            (None, _) => {
                let previous_end = tree.nodes[entries[index - 1].value].span.end;
                Span::new(previous_end, value_end)
            }
        }
    }

    fn append_to_empty(
        &self,
        container: NodeId,
        key: Option<&str>,
        value: &Value,
        style: &Style,
    ) -> Result<Vec<Change>, Error> {
        let layout = &self.layout;
        let span = layout.tree.nodes[container].span;
        let close = span.end - 1;
        let interior = Span::new(span.start + 1, close);
        let has_comments = layout.comments_in(interior).next().is_some();
        if !self.context_multiline(container) {
            let entry = self.entry_text(key, value, "", style, false)?;
            let space = &style.bracket_space;
            return Ok(vec![if has_comments {
                Change::insert(close, format!("{entry}{space}"))
            } else {
                Change {
                    span: interior,
                    text: format!("{space}{entry}{space}"),
                }
            }]);
        }
        let indent = layout.indent_of(span.start);
        let inner = format!("{indent}{}", style.indent_unit);
        let entry = self.entry_text(key, value, &inner, style, true)?;
        let trailing = if style.trailing_comma { "," } else { "" };
        let eol = style.eol;
        Ok(vec![if !has_comments {
            Change {
                span: interior,
                text: format!("{eol}{inner}{entry}{trailing}{eol}{indent}"),
            }
        } else if layout.starts_line(close) {
            Change::insert(
                layout.line_start(close),
                format!("{inner}{entry}{trailing}{eol}"),
            )
        } else {
            Change::insert(close, format!("{eol}{inner}{entry}{trailing}{eol}{indent}"))
        }])
    }

    fn append_on_new_line(
        &self,
        last: &Entry,
        key: Option<&str>,
        value: &Value,
        style: &Style,
    ) -> Result<Vec<Change>, Error> {
        let layout = &self.layout;
        let value_end = layout.tree.nodes[last.value].span.end;
        let indent = layout.indent_of(last.start(layout.tree));
        let entry = self.entry_text(key, value, indent, style, true)?;
        let at = layout.skip_same_line_trivia(self.entry_end(last));
        let eol = style.eol;
        Ok(match last.comma {
            Some(_) => vec![Change::insert(at, format!("{eol}{indent}{entry},"))],
            None if at == value_end => vec![Change::insert(at, format!(",{eol}{indent}{entry}"))],
            None => vec![
                Change::insert(value_end, ",".to_owned()),
                Change::insert(at, format!("{eol}{indent}{entry}")),
            ],
        })
    }

    fn append_inline(
        &self,
        last: &Entry,
        key: Option<&str>,
        value: &Value,
        style: &Style,
    ) -> Result<Vec<Change>, Error> {
        let entry = self.entry_text(key, value, "", style, false)?;
        let space = &style.comma_space;
        Ok(vec![match last.comma {
            Some(comma) => Change::insert(comma + 1, format!("{space}{entry},")),
            None => Change::insert(
                self.layout.tree.nodes[last.value].span.end,
                format!(",{space}{entry}"),
            ),
        }])
    }

    fn entry_text(
        &self,
        key: Option<&str>,
        value: &Value,
        indent: &str,
        style: &Style,
        multiline: bool,
    ) -> Result<String, Error> {
        match key {
            Some(key) => render::render_member(key, value, indent, style, multiline),
            None => render::render(value, indent, style, multiline),
        }
    }

    /// New containers inside `node` go on several lines when the
    /// enclosing container does, or, for the root, when the document does.
    fn context_multiline(&self, node: NodeId) -> bool {
        let layout = &self.layout;
        match layout.tree.nodes[node].parent {
            Some(parent) => layout.is_multiline(layout.tree.nodes[parent].span),
            None => layout.text.trim().contains('\n'),
        }
    }
}
