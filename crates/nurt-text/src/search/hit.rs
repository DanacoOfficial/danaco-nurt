// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Results of a search: hits, matches, context lines and counters.

use std::borrow::Cow;

use bstr::ByteSlice;
use nurt_contracts::record::RegexEngine;

/// Share of the `max_line` window placed before the first match when a long
/// line is shortened: a quarter of the window shows what precedes the match.
const FOCUS_LEAD_DIVISOR: usize = 4;

/// Match of one pattern.
///
/// Offsets count bytes from the start of the searched input; lines count
/// from 1; columns count Unicode scalar values from 1, an invalid UTF-8
/// sequence counting as one character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Match {
    /// Index of the pattern in the list given to the searcher, from 0.
    pub pattern: usize,
    /// Byte offset of the first byte of the match.
    pub start: u64,
    /// Byte offset after the last byte of the match.
    pub end: u64,
    /// Line of the first character of the match.
    pub line: u64,
    /// Column of the first character of the match.
    pub column: u64,
    /// Line of the last character of the match; `line` for an empty match.
    pub end_line: u64,
    /// Column after the last character of the match on `end_line`; a line
    /// break inside the match counts as one character.
    pub end_column: u64,
    /// Engine that evaluated the pattern.
    pub engine: RegexEngine,
}

/// Line shown around a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextLine<'a> {
    pub(super) line: u64,
    pub(super) start: u64,
    pub(super) bytes: &'a [u8],
    pub(super) max_line: Option<usize>,
}

impl<'a> ContextLine<'a> {
    /// Line number, from 1.
    #[must_use]
    pub const fn line(&self) -> u64 {
        self.line
    }

    /// Byte offset of the start of the line.
    #[must_use]
    pub const fn start(&self) -> u64 {
        self.start
    }

    /// Content of the line without its line ending.
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Text of the line, shortened to `max_line` characters with a marker
    /// of the omitted characters.
    #[must_use]
    pub fn text(&self) -> Cow<'a, str> {
        render_line(self.bytes, self.max_line, 0)
    }

    /// Whether [`ContextLine::text`] is shortened.
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        is_long(self.bytes, self.max_line)
    }
}

/// Line or block of lines reported by a search.
///
/// Without `multiline` a hit is one line; with `multiline` it is the block
/// of lines covered by matches that touch each other. With `invert` a hit is
/// one line without a match and [`Hit::matches`] is empty.
#[derive(Debug, Clone, Copy)]
pub struct Hit<'a> {
    pub(super) line: u64,
    pub(super) end_line: u64,
    pub(super) start: u64,
    pub(super) bytes: &'a [u8],
    pub(super) matches: &'a [Match],
    pub(super) before: &'a [ContextLine<'a>],
    pub(super) after: &'a [ContextLine<'a>],
    pub(super) max_line: Option<usize>,
    /// Text the match offsets refer to, starting at `haystack_offset`.
    pub(super) haystack: &'a [u8],
    pub(super) haystack_offset: u64,
}

impl<'a> Hit<'a> {
    /// First line, from 1.
    #[must_use]
    pub const fn line(&self) -> u64 {
        self.line
    }

    /// Last line, from 1.
    #[must_use]
    pub const fn end_line(&self) -> u64 {
        self.end_line
    }

    /// Byte offset of the start of the first line.
    #[must_use]
    pub const fn start(&self) -> u64 {
        self.start
    }

    /// Byte offset after the content of the last line, before its line
    /// ending.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.start + self.bytes.len() as u64
    }

    /// Content of the lines without the line ending of the last line.
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Matches in the order of their start; with `only_matching` the total
    /// over all hits does not exceed `max_per_file`.
    #[must_use]
    pub const fn matches(&self) -> &'a [Match] {
        self.matches
    }

    /// Lines before the hit, nearest last.
    #[must_use]
    pub const fn before(&self) -> &'a [ContextLine<'a>] {
        self.before
    }

    /// Lines after the hit, nearest first.
    #[must_use]
    pub const fn after(&self) -> &'a [ContextLine<'a>] {
        self.after
    }

    /// Text of the lines joined by `\n`, each shortened to `max_line`
    /// characters with a marker of the omitted characters; the first line
    /// keeps its first match in view.
    #[must_use]
    pub fn text(&self) -> Cow<'a, str> {
        let focus = self.matches.first().map_or(0, |m| {
            usize::try_from(m.column.saturating_sub(1)).unwrap_or(usize::MAX)
        });
        if !self.bytes.contains(&b'\n') {
            return render_line(self.bytes, self.max_line, focus);
        }
        let lines = self
            .bytes
            .split(|&byte| byte == b'\n')
            .enumerate()
            .map(|(index, line)| {
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                render_line(line, self.max_line, if index == 0 { focus } else { 0 })
            });
        Cow::Owned(lines.collect::<Vec<_>>().join("\n"))
    }

    /// Text of the lines joined by `\n`, none of them shortened.
    #[must_use]
    pub fn full_text(&self) -> String {
        self.lines()
            .map(|line| String::from_utf8_lossy(line).into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whether [`Hit::text`] shortens a line.
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.lines().any(|line| is_long(line, self.max_line))
    }

    /// Matched bytes of `found`, one of [`Hit::matches`]; `None` for a match
    /// of another hit.
    #[must_use]
    pub fn matched(&self, found: &Match) -> Option<&'a [u8]> {
        let from = usize::try_from(found.start.checked_sub(self.haystack_offset)?).ok()?;
        let to = usize::try_from(found.end.checked_sub(self.haystack_offset)?).ok()?;
        self.haystack.get(from..to)
    }

    /// Text the offsets of the matches refer to and the absolute offset of
    /// its first byte.
    pub(super) const fn haystack(&self) -> (&'a [u8], u64) {
        (self.haystack, self.haystack_offset)
    }

    pub(super) fn lines(&self) -> impl Iterator<Item = &'a [u8]> {
        self.bytes
            .split(|&byte| byte == b'\n')
            .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
    }
}

/// Capture group of a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group<'a> {
    /// Byte offset of the first byte of the group.
    pub start: u64,
    /// Byte offset after the last byte of the group.
    pub end: u64,
    /// Captured bytes.
    pub bytes: &'a [u8],
}

impl<'a> Group<'a> {
    /// Captured text; invalid UTF-8 becomes U+FFFD.
    #[must_use]
    pub fn text(&self) -> Cow<'a, str> {
        String::from_utf8_lossy(self.bytes)
    }
}

/// Capture groups of a match; group 0 is the whole match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captures<'a> {
    pub(super) groups: Vec<Option<Group<'a>>>,
    pub(super) names: &'a [Option<String>],
}

impl<'a> Captures<'a> {
    /// Number of groups, the whole match included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Whether there is no group; never true for captures of a match.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Group by index; `None` for a group that did not take part in the
    /// match or does not exist.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Group<'a>> {
        self.groups.get(index)?.as_ref()
    }

    /// Group by name.
    #[must_use]
    pub fn name(&self, name: &str) -> Option<&Group<'a>> {
        let index = self
            .names
            .iter()
            .position(|candidate| candidate.as_deref() == Some(name))?;
        self.get(index)
    }

    /// Named groups in the order of the pattern, with `None` for a group
    /// that did not take part in the match.
    pub fn named(&self) -> impl Iterator<Item = (&str, Option<&Group<'a>>)> {
        self.names
            .iter()
            .enumerate()
            .filter_map(|(index, name)| Some((name.as_deref()?, self.get(index))))
    }
}

/// Counters of one search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Summary {
    /// Hits passed to the sink.
    pub hits: u64,
    /// Matches passed to the sink.
    pub matches: u64,
    /// Matches passed to the sink by pattern index.
    pub per_pattern: Vec<u64>,
    /// Whether `max_per_file` left out a hit or a match.
    pub limit_reached: bool,
    /// Lines passed to the sink, hits and context, longer than `max_line`.
    pub truncated_lines: u64,
    /// Searches the backtracking engine abandoned at its step limit; the
    /// text being searched was treated as not matching.
    pub backtrack_limit_hits: u64,
}

impl Summary {
    pub(super) fn new(patterns: usize) -> Self {
        Self {
            per_pattern: vec![0; patterns],
            ..Self::default()
        }
    }
}

/// Number of characters as counted by [`Match::column`].
pub(super) fn char_count(bytes: &[u8]) -> usize {
    if bytes.is_ascii() {
        bytes.len()
    } else {
        bytes.chars().count()
    }
}

/// Whether a line is longer than `max_line` characters; a limit of 0 means
/// no limit.
pub(super) fn is_long(bytes: &[u8], max_line: Option<usize>) -> bool {
    max_line.is_some_and(|max| max > 0 && bytes.len() > max && char_count(bytes) > max)
}

/// Text of one line shortened to `max_line` characters around the
/// character at index `focus`.
fn render_line(bytes: &[u8], max_line: Option<usize>, focus: usize) -> Cow<'_, str> {
    let text = String::from_utf8_lossy(bytes);
    let Some(max) = max_line.filter(|&max| max > 0) else {
        return text;
    };
    let total = text.chars().count();
    if total <= max {
        return text;
    }
    let first = focus
        .saturating_sub(max / FOCUS_LEAD_DIVISOR)
        .min(total - max);
    let last = first + max;
    let mut shown = String::with_capacity(max + 2 * omission(total).len());
    if first > 0 {
        shown.push_str(&omission(first));
        shown.push(' ');
    }
    shown.extend(text.chars().skip(first).take(max));
    if last < total {
        shown.push(' ');
        shown.push_str(&omission(total - last));
    }
    Cow::Owned(shown)
}

/// Marker of characters left out of a shortened line.
fn omission(count: usize) -> String {
    format!("[{count} characters omitted]")
}
