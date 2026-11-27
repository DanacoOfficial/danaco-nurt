// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-text::eol
// Responsibility:   Line terminators: detection, conversion, per-line preservation and.
// Dependencies:     memchr, nurt_contracts, similar
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Line terminators: detection, conversion, per-line preservation and
//! positions.
//!
//! A line terminator is `\r\n`, `\n`, or a `\r` not followed by `\n`. Other
//! Unicode separators (U+0085, U+2028, U+2029) are ordinary characters.
//! Functions that take bytes accept any ASCII-compatible encoding, where the
//! terminators are the single bytes `0x0D` and `0x0A`; UTF-16 content is
//! decoded before it is passed to them.

use std::borrow::Cow;
use std::ops::Range;

use memchr::memchr2;
use nurt_contracts::services::LineEnding;
use similar::{Algorithm, DiffOp, capture_diff_slices};

const CR: u8 = b'\r';
const LF: u8 = b'\n';

/// Line terminator of a single line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Terminator {
    /// `\n`.
    Lf,
    /// `\r\n`.
    CrLf,
    /// `\r`.
    Cr,
}

impl Terminator {
    /// The terminator characters.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::eol::Terminator;
    ///
    /// assert_eq!(Terminator::CrLf.as_str(), "\r\n");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
}

impl From<Terminator> for LineEnding {
    fn from(terminator: Terminator) -> Self {
        match terminator {
            Terminator::Lf => Self::Lf,
            Terminator::CrLf => Self::CrLf,
            Terminator::Cr => Self::Cr,
        }
    }
}

/// Position of a character in a text.
///
/// Lines and columns start at 1; columns count Unicode scalar values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Position {
    /// Line.
    pub line: u64,
    /// Column.
    pub column: u64,
}

impl Position {
    /// Position of the first character of a text.
    pub const START: Self = Self { line: 1, column: 1 };
}

/// One line of a text with its terminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line<'a> {
    /// Content of the line without the terminator.
    pub content: &'a str,
    /// Terminator; `None` for a last line without one.
    pub terminator: Option<Terminator>,
}

/// Iterator over the lines of a text, created by [`lines`].
#[derive(Debug, Clone)]
pub struct Lines<'a> {
    rest: &'a str,
}

impl<'a> Iterator for Lines<'a> {
    type Item = Line<'a>;

    fn next(&mut self) -> Option<Line<'a>> {
        if self.rest.is_empty() {
            return None;
        }
        let bytes = self.rest.as_bytes();
        let Some(at) = memchr2(CR, LF, bytes) else {
            let content = self.rest;
            self.rest = "";
            return Some(Line {
                content,
                terminator: None,
            });
        };
        let terminator = terminator_at(bytes, at);
        let content = &self.rest[..at];
        self.rest = &self.rest[at + terminator.as_str().len()..];
        Some(Line {
            content,
            terminator: Some(terminator),
        })
    }
}

/// Splits a text into lines.
///
/// A final terminator does not start another line: `"a\nb\n"` has two lines
/// and the empty text has none.
///
/// # Examples
///
/// ```
/// use nurt_text::eol::{Terminator, lines};
///
/// let terminators: Vec<_> = lines("a\r\nb\nc").map(|line| line.terminator).collect();
/// assert_eq!(terminators, [Some(Terminator::CrLf), Some(Terminator::Lf), None]);
/// ```
#[must_use]
pub fn lines(text: &str) -> Lines<'_> {
    Lines { rest: text }
}

/// Terminator starting at `at`, where `bytes[at]` is `\r` or `\n`.
fn terminator_at(bytes: &[u8], at: usize) -> Terminator {
    if bytes.get(at) == Some(&LF) {
        Terminator::Lf
    } else if bytes.get(at + 1) == Some(&LF) {
        Terminator::CrLf
    } else {
        Terminator::Cr
    }
}

/// Terminators found in a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scan {
    None,
    Uniform(Terminator),
    Mixed,
}

fn scan(bytes: &[u8]) -> Scan {
    let mut found = None;
    let mut at = 0;
    while let Some(relative) = bytes.get(at..).and_then(|rest| memchr2(CR, LF, rest)) {
        let start = at + relative;
        let terminator = terminator_at(bytes, start);
        match found {
            None => found = Some(terminator),
            Some(previous) if previous != terminator => return Scan::Mixed,
            Some(_) => {}
        }
        at = start + terminator.as_str().len();
    }
    found.map_or(Scan::None, Scan::Uniform)
}

/// Detects the line ending convention of a text.
///
/// The result is [`LineEnding::Mixed`] when lines end with different
/// terminators and [`LineEnding::None`] when the text has no terminator.
///
/// # Examples
///
/// ```
/// use nurt_contracts::services::LineEnding;
/// use nurt_text::eol::detect;
///
/// assert_eq!(detect(b"a\r\nb\r\n"), LineEnding::CrLf);
/// assert_eq!(detect(b"a\r\nb\n"), LineEnding::Mixed);
/// assert_eq!(detect(b"a"), LineEnding::None);
/// ```
#[must_use]
pub fn detect(bytes: &[u8]) -> LineEnding {
    match scan(bytes) {
        Scan::None => LineEnding::None,
        Scan::Uniform(terminator) => terminator.into(),
        Scan::Mixed => LineEnding::Mixed,
    }
}

/// Replaces every line terminator with `target`.
///
/// A missing final terminator stays missing. The text is borrowed when it
/// already uses `target` only.
///
/// # Examples
///
/// ```
/// use nurt_text::eol::{Terminator, convert};
///
/// assert_eq!(convert("a\nb\r\nc", Terminator::CrLf), "a\r\nb\r\nc");
/// ```
#[must_use]
pub fn convert(text: &str, target: Terminator) -> Cow<'_, str> {
    if lines(text).all(|line| line.terminator.is_none_or(|found| found == target)) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + text.len() / 16);
    for line in lines(text) {
        out.push_str(line.content);
        if line.terminator.is_some() {
            out.push_str(target.as_str());
        }
    }
    Cow::Owned(out)
}

/// Restores the line terminators of `original` in `edited`, a changed copy
/// of it.
///
/// Lines of `edited` keep or lack a terminator as they do; only its kind is
/// chosen. When `original` uses one terminator, every line of `edited` gets
/// it. When `original` mixes terminators, lines are matched by content: a
/// line whose content is unchanged gets its original terminator, a replaced
/// line gets the terminator of the line it replaces, an inserted line gets
/// the terminator of the preceding original line, and the prevailing
/// terminator of `original` fills the remaining cases. An empty line that
/// would get `\n` directly after a `\r` gets `\r` instead, because the two
/// would read back as one `\r\n` terminator. When `original` has no
/// terminator, `edited` is returned unchanged.
///
/// # Examples
///
/// ```
/// use nurt_text::eol::preserve;
///
/// let original = "one\r\ntwo\nthree\r\n";
/// let edited = "one\nTWO\nnew\nthree\n";
/// assert_eq!(preserve(original, edited), "one\r\nTWO\nnew\nthree\r\n");
/// ```
#[must_use]
pub fn preserve<'e>(original: &str, edited: &'e str) -> Cow<'e, str> {
    match scan(original.as_bytes()) {
        Scan::None => Cow::Borrowed(edited),
        Scan::Uniform(terminator) => convert(edited, terminator),
        Scan::Mixed => Cow::Owned(preserve_mixed(original, edited)),
    }
}

fn preserve_mixed(original: &str, edited: &str) -> String {
    let old: Vec<Line<'_>> = lines(original).collect();
    let new: Vec<Line<'_>> = lines(edited).collect();
    let old_content: Vec<&str> = old.iter().map(|line| line.content).collect();
    let new_content: Vec<&str> = new.iter().map(|line| line.content).collect();
    let old_terminator = |index: usize| old.get(index).and_then(|line| line.terminator);

    let mut source: Vec<Option<Terminator>> = vec![None; new.len()];
    for op in capture_diff_slices(Algorithm::Myers, &old_content, &new_content) {
        match op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for offset in 0..len {
                    assign(
                        &mut source,
                        new_index + offset,
                        old_terminator(old_index + offset),
                    );
                }
            }
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => {
                let neighbour = old_index
                    .checked_sub(1)
                    .and_then(old_terminator)
                    .or_else(|| old_terminator(old_index));
                for offset in 0..new_len {
                    assign(&mut source, new_index + offset, neighbour);
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                let last = old_len.saturating_sub(1);
                for offset in 0..new_len {
                    let replaced = old_terminator(old_index + offset.min(last));
                    assign(&mut source, new_index + offset, replaced);
                }
            }
            DiffOp::Delete { .. } => {}
        }
    }

    let fallback = prevailing(&old);
    let mut out = String::with_capacity(edited.len() + edited.len() / 16);
    let mut previous = None;
    for (line, chosen) in new.iter().zip(source) {
        out.push_str(line.content);
        if let Some(present) = line.terminator {
            let mut terminator = chosen.or(fallback).unwrap_or(present);
            if previous == Some(Terminator::Cr)
                && line.content.is_empty()
                && terminator == Terminator::Lf
            {
                terminator = Terminator::Cr;
            }
            out.push_str(terminator.as_str());
            previous = Some(terminator);
        }
    }
    out
}

fn assign(slots: &mut [Option<Terminator>], index: usize, value: Option<Terminator>) {
    if let Some(slot) = slots.get_mut(index) {
        *slot = value;
    }
}

/// Terminator of the largest number of lines; a tie goes to the one that
/// occurs first.
fn prevailing(lines: &[Line<'_>]) -> Option<Terminator> {
    let mut counts: Vec<(Terminator, usize)> = Vec::with_capacity(3);
    for terminator in lines.iter().filter_map(|line| line.terminator) {
        match counts.iter_mut().find(|(kind, _)| *kind == terminator) {
            Some((_, count)) => *count += 1,
            None => counts.push((terminator, 1)),
        }
    }
    let mut best: Option<(Terminator, usize)> = None;
    for (kind, count) in counts {
        if best.is_none_or(|(_, top)| count > top) {
            best = Some((kind, count));
        }
    }
    best.map(|(kind, _)| kind)
}

/// Position directly after the last character of `text`.
pub(crate) fn end_position(text: &str) -> Position {
    let mut line = 1_u64;
    let mut last_line_start = 0;
    let mut consumed = 0;
    for item in lines(text) {
        consumed += item.content.len();
        if let Some(terminator) = item.terminator {
            consumed += terminator.as_str().len();
            line += 1;
            last_line_start = consumed;
        }
    }
    let column = text
        .get(last_line_start..)
        .map_or(0, |tail| tail.chars().count());
    Position {
        line,
        column: column as u64 + 1,
    }
}

/// Mapping between byte offsets and line and column positions of a text.
///
/// # Examples
///
/// ```
/// use nurt_text::eol::{LineIndex, Position};
///
/// let index = LineIndex::new("kůň\r\nžluťoučký");
/// let position = Position { line: 2, column: 3 };
/// assert_eq!(index.position(index.offset(position).unwrap()), Some(position));
/// assert_eq!(index.line_count(), 2);
/// ```
#[derive(Debug, Clone)]
pub struct LineIndex<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    /// Builds the index of a text.
    #[must_use]
    pub fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        let mut offset = 0;
        for line in lines(text) {
            offset += line.content.len();
            if let Some(terminator) = line.terminator {
                offset += terminator.as_str().len();
                starts.push(offset);
            }
        }
        Self { text, starts }
    }

    /// Number of lines, counted as by [`lines`].
    #[must_use]
    pub fn line_count(&self) -> usize {
        let trailing_empty = self.starts.last() == Some(&self.text.len());
        self.starts.len() - usize::from(trailing_empty)
    }

    /// Position of the character at byte `offset`.
    ///
    /// `offset` may equal the length of the text. The result is `None` when
    /// `offset` is past the end or not on a character boundary. The `\n` of
    /// a `\r\n` pair belongs to the terminator; its position is the column
    /// after the `\r`, which [`LineIndex::offset`] does not accept.
    #[must_use]
    pub fn position(&self, offset: usize) -> Option<Position> {
        let before = self.text.get(..offset)?;
        let line = self.starts.partition_point(|&start| start <= offset);
        let start = self.starts.get(line.checked_sub(1)?).copied()?;
        let column = before.get(start..)?.chars().count();
        Some(Position {
            line: line as u64,
            column: column as u64 + 1,
        })
    }

    /// Byte offset of a position.
    ///
    /// The column may point directly after the last character of the line.
    /// The result is `None` when the line does not exist or the column is
    /// past that point.
    #[must_use]
    pub fn offset(&self, position: Position) -> Option<usize> {
        let content = self.line_content(usize::try_from(position.line).ok()?)?;
        let skip = usize::try_from(position.column.checked_sub(1)?).ok()?;
        let line_text = self.text.get(content.clone())?;
        let within = line_text
            .char_indices()
            .map(|(at, _)| at)
            .chain(std::iter::once(line_text.len()))
            .nth(skip)?;
        Some(content.start + within)
    }

    /// Byte range of the content of a line, without its terminator.
    fn line_content(&self, line: usize) -> Option<Range<usize>> {
        let start = *self.starts.get(line.checked_sub(1)?)?;
        let rest = self.text.get(start..)?;
        let first = lines(rest).next().map_or(0, |item| item.content.len());
        Some(start..start + first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminators(text: &str) -> Vec<Option<Terminator>> {
        lines(text).map(|line| line.terminator).collect()
    }

    #[test]
    fn splits_lines_with_every_terminator() {
        let parts: Vec<&str> = lines("a\nb\r\nc\rd").map(|line| line.content).collect();
        assert_eq!(parts, ["a", "b", "c", "d"]);
        assert_eq!(
            terminators("a\nb\r\nc\rd"),
            [
                Some(Terminator::Lf),
                Some(Terminator::CrLf),
                Some(Terminator::Cr),
                None
            ]
        );
    }

    #[test]
    fn final_terminator_does_not_start_a_line() {
        assert_eq!(lines("").count(), 0);
        assert_eq!(lines("\n").count(), 1);
        assert_eq!(lines("a\nb\n").count(), 2);
        assert_eq!(lines("a\n\n").count(), 2);
    }

    #[test]
    fn carriage_return_at_end_is_a_terminator() {
        assert_eq!(terminators("a\r"), [Some(Terminator::Cr)]);
        assert_eq!(
            terminators("\r\r\n"),
            [Some(Terminator::Cr), Some(Terminator::CrLf)]
        );
    }

    #[test]
    fn unicode_separators_are_not_terminators() {
        assert_eq!(lines("a\u{2028}b\u{85}c").count(), 1);
        assert_eq!(detect("a\u{2029}b".as_bytes()), LineEnding::None);
    }

    #[test]
    fn detects_each_convention() {
        assert_eq!(detect(b""), LineEnding::None);
        assert_eq!(detect(b"one line"), LineEnding::None);
        assert_eq!(detect(b"a\nb\n"), LineEnding::Lf);
        assert_eq!(detect(b"a\r\nb\r\n"), LineEnding::CrLf);
        assert_eq!(detect(b"a\rb\r"), LineEnding::Cr);
        assert_eq!(detect(b"a\r\nb\n"), LineEnding::Mixed);
        assert_eq!(detect(b"a\rb\n"), LineEnding::Mixed);
        assert_eq!(detect(b"a\r\nb\r"), LineEnding::Mixed);
    }

    #[test]
    fn detects_terminators_in_single_byte_encodings() {
        let windows_1250 = b"Za\xbf\xf3\xb3\xe6\r\ng\xea\x9cl\xb9\r\n";
        assert_eq!(detect(windows_1250), LineEnding::CrLf);
    }

    #[test]
    fn terminator_maps_to_contract_line_ending() {
        assert_eq!(LineEnding::from(Terminator::Lf), LineEnding::Lf);
        assert_eq!(LineEnding::from(Terminator::CrLf), LineEnding::CrLf);
        assert_eq!(LineEnding::from(Terminator::Cr), LineEnding::Cr);
    }

    #[test]
    fn converts_every_terminator() {
        assert_eq!(convert("a\nb\r\nc\rd", Terminator::Lf), "a\nb\nc\nd");
        assert_eq!(
            convert("a\nb\r\nc\rd", Terminator::CrLf),
            "a\r\nb\r\nc\r\nd"
        );
        assert_eq!(convert("a\nb\r\nc\n", Terminator::Cr), "a\rb\rc\r");
    }

    #[test]
    fn conversion_keeps_missing_final_terminator() {
        assert_eq!(convert("a\nb", Terminator::CrLf), "a\r\nb");
        assert_eq!(convert("", Terminator::CrLf), "");
    }

    #[test]
    fn conversion_borrows_text_already_in_target() {
        assert!(matches!(
            convert("a\r\nb", Terminator::CrLf),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            convert("no terminator", Terminator::Lf),
            Cow::Borrowed(_)
        ));
        assert!(matches!(convert("a\nb", Terminator::CrLf), Cow::Owned(_)));
    }

    #[test]
    fn preserve_applies_uniform_terminator_to_inserted_lines() {
        let original = "one\r\ntwo\r\n";
        let edited = "one\r\nnew\ntwo\r\n";
        assert_eq!(preserve(original, edited), "one\r\nnew\r\ntwo\r\n");
    }

    #[test]
    fn preserve_keeps_missing_final_terminator() {
        assert_eq!(preserve("a\r\nb", "a\nb\nc"), "a\r\nb\r\nc");
        assert_eq!(preserve("a\r\nb\r\n", "a\nb"), "a\r\nb");
    }

    #[test]
    fn preserve_returns_edited_when_original_has_no_terminator() {
        assert!(matches!(
            preserve("single", "single\nadded\r\n"),
            Cow::Borrowed("single\nadded\r\n")
        ));
    }

    #[test]
    fn preserve_keeps_terminator_of_each_unchanged_line_in_mixed_text() {
        let original = "a\r\nb\nc\rd\r\n";
        let edited = "a\nb\nc\nd\n";
        assert_eq!(preserve(original, edited), original);
    }

    #[test]
    fn preserve_uses_replaced_line_terminator() {
        let original = "a\r\nb\nc\r\n";
        let edited = "a\nB\nc\n";
        assert_eq!(preserve(original, edited), "a\r\nB\nc\r\n");
    }

    #[test]
    fn preserve_uses_preceding_line_for_inserted_lines() {
        let original = "a\r\nb\nc\r\n";
        let edited = "a\nb\nx\ny\nc\n";
        assert_eq!(preserve(original, edited), "a\r\nb\nx\ny\nc\r\n");
    }

    #[test]
    fn preserve_uses_following_line_for_lines_inserted_at_start() {
        let original = "a\nb\r\nc\r\n";
        let edited = "x\na\nb\nc\n";
        assert_eq!(preserve(original, edited), "x\na\nb\r\nc\r\n");
    }

    #[test]
    fn preserve_falls_back_to_prevailing_terminator() {
        let original = "a\nb\r\nc\r\nd";
        let edited = "a\nb\nc\nd\ne\n";
        assert_eq!(preserve(original, edited), "a\nb\r\nc\r\nd\r\ne\r\n");
    }

    #[test]
    fn preserve_handles_deleted_lines() {
        let original = "a\r\nb\nc\r\n";
        let edited = "a\nc\n";
        assert_eq!(preserve(original, edited), "a\r\nc\r\n");
    }

    #[test]
    fn prevailing_terminator_breaks_ties_by_first_occurrence() {
        let tie: Vec<Line<'_>> = lines("a\nb\r\n").collect();
        assert_eq!(prevailing(&tie), Some(Terminator::Lf));
        let tie: Vec<Line<'_>> = lines("a\r\nb\n").collect();
        assert_eq!(prevailing(&tie), Some(Terminator::CrLf));
        assert_eq!(prevailing(&[]), None);
    }

    #[test]
    fn end_position_counts_lines_and_characters() {
        assert_eq!(end_position(""), Position::START);
        assert_eq!(end_position("kůň"), Position { line: 1, column: 4 });
        assert_eq!(end_position("a\r\nbc"), Position { line: 2, column: 3 });
        assert_eq!(end_position("a\rb\n"), Position { line: 3, column: 1 });
    }

    #[test]
    fn line_index_maps_offsets_to_positions() {
        let text = "ab\r\nžč\rx";
        let index = LineIndex::new(text);
        assert_eq!(index.position(0), Some(Position { line: 1, column: 1 }));
        assert_eq!(index.position(2), Some(Position { line: 1, column: 3 }));
        assert_eq!(index.position(4), Some(Position { line: 2, column: 1 }));
        assert_eq!(index.position(6), Some(Position { line: 2, column: 2 }));
        assert_eq!(index.position(9), Some(Position { line: 3, column: 1 }));
        assert_eq!(
            index.position(text.len()),
            Some(Position { line: 3, column: 2 })
        );
    }

    #[test]
    fn line_index_rejects_invalid_offsets() {
        let index = LineIndex::new("ža");
        assert_eq!(index.position(1), None);
        assert_eq!(index.position(4), None);
    }

    #[test]
    fn line_index_maps_positions_to_offsets() {
        let text = "ab\r\nžč\rx";
        let index = LineIndex::new(text);
        assert_eq!(index.offset(Position { line: 1, column: 1 }), Some(0));
        assert_eq!(index.offset(Position { line: 1, column: 3 }), Some(2));
        assert_eq!(index.offset(Position { line: 2, column: 2 }), Some(6));
        assert_eq!(index.offset(Position { line: 2, column: 3 }), Some(8));
        assert_eq!(index.offset(Position { line: 3, column: 2 }), Some(10));
        assert_eq!(index.offset(Position { line: 1, column: 4 }), None);
        assert_eq!(index.offset(Position { line: 4, column: 1 }), None);
        assert_eq!(index.offset(Position { line: 0, column: 1 }), None);
        assert_eq!(index.offset(Position { line: 1, column: 0 }), None);
    }

    #[test]
    fn line_index_counts_lines_like_lines_iterator() {
        for text in [
            "",
            "a",
            "a\n",
            "\n",
            "a\nb",
            "a\nb\n",
            "a\r\n\r\n",
            "a\rb\r",
        ] {
            assert_eq!(
                LineIndex::new(text).line_count(),
                lines(text).count(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn line_index_positions_after_final_terminator() {
        let index = LineIndex::new("a\n");
        assert_eq!(index.position(2), Some(Position { line: 2, column: 1 }));
        assert_eq!(index.offset(Position { line: 2, column: 1 }), Some(2));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
