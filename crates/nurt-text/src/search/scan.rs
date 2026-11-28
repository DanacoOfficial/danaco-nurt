// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-text::search::scan
// Responsibility:   Leftmost-first scanning of a byte slice with every pattern of a set.
// Dependencies:     memchr
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Leftmost-first scanning of a byte slice with every pattern of a set.
//!
//! Patterns are searched independently and merged by start offset; at the
//! same start the pattern with the lower index wins, which is the meaning of
//! an alternation of the patterns. Without `multiline` a match never
//! contains a line break: a candidate that crosses one is searched again
//! within its line.

use memchr::{memchr, memrchr};

use super::hit::{Match, char_count};
use super::pattern::{MatchSource, PatternSet};

/// Match of one pattern as byte offsets into the scanned slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RawMatch {
    pub(super) pattern: usize,
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Debug, Clone, Copy)]
enum Slot {
    Unknown,
    Found(RawMatch),
    Exhausted,
}

/// Iterator over the matches of a pattern set in a slice.
pub(super) struct Scanner<'s, 'h> {
    set: &'s PatternSet,
    haystack: &'h [u8],
    multiline: bool,
    sources: &'s [MatchSource],
    slots: Vec<Slot>,
    pos: usize,
    last_end: Option<usize>,
    backtrack_limit_hits: u64,
    /// Bounds of the line found last, `start..=end`, where `end` is the offset
    /// of its line break or the length of the slice.
    line: Option<(usize, usize)>,
}

impl<'s, 'h> Scanner<'s, 'h> {
    pub(super) fn new(set: &'s PatternSet, haystack: &'h [u8], multiline: bool) -> Self {
        let sources = set.sources();
        let slots = vec![Slot::Unknown; sources.len()];
        Self {
            set,
            haystack,
            multiline,
            sources,
            slots,
            pos: 0,
            last_end: None,
            backtrack_limit_hits: 0,
            line: None,
        }
    }

    /// Start and end of the line that contains `pos`. The line of the previous
    /// answer is reused while `pos` stays on it, so that many matches on one
    /// long line do not search that line for its breaks once per match.
    pub(super) fn line_bounds(&mut self, pos: usize) -> (usize, usize) {
        if let Some((start, end)) = self.line
            && start <= pos
            && pos <= end
        {
            return (start, end);
        }
        let bounds = (line_start(self.haystack, pos), line_end(self.haystack, pos));
        if pos <= self.haystack.len() {
            self.line = Some(bounds);
        }
        bounds
    }

    pub(super) const fn backtrack_limit_hits(&self) -> u64 {
        self.backtrack_limit_hits
    }

    /// Continues the scan at `pos`, forgetting matches that start earlier.
    pub(super) fn skip_to(&mut self, pos: usize) {
        if pos > self.pos {
            self.pos = pos;
            self.last_end = None;
            self.forget_before_pos();
        }
    }

    /// Next match; an empty match at the end of the previous match is
    /// skipped, as in the iteration of the `regex` crate.
    pub(super) fn next_match(&mut self) -> Option<RawMatch> {
        loop {
            self.fill();
            let best = self.leftmost()?;
            if best.start == best.end && self.last_end == Some(best.start) {
                self.pos = best.start + sequence_length_at(self.haystack, best.start);
                self.forget_before_pos();
                continue;
            }
            if !is_on_line(self.haystack, best.start) {
                return None;
            }
            self.pos = best.end;
            self.last_end = Some(best.end);
            self.forget_before_pos();
            return Some(best);
        }
    }

    fn fill(&mut self) {
        for index in 0..self.slots.len() {
            if matches!(self.slots[index], Slot::Unknown) {
                let source = self.sources[index];
                self.slots[index] = self
                    .search(source, self.pos)
                    .map_or(Slot::Exhausted, Slot::Found);
            }
        }
    }

    fn leftmost(&self) -> Option<RawMatch> {
        self.slots
            .iter()
            .filter_map(|slot| match slot {
                Slot::Found(found) => Some(*found),
                Slot::Unknown | Slot::Exhausted => None,
            })
            .min_by_key(|found| (found.start, found.pattern))
    }

    fn forget_before_pos(&mut self) {
        for slot in &mut self.slots {
            if let Slot::Found(found) = slot
                && found.start < self.pos
            {
                *slot = Slot::Unknown;
            }
        }
    }

    fn search(&mut self, source: MatchSource, from: usize) -> Option<RawMatch> {
        if self.multiline {
            self.find(source, self.haystack, from, self.haystack.len())
        } else if self.set.searches_by_line(source) {
            self.find_line_by_line(source, from)
        } else {
            self.find_within_line(source, from)
        }
    }

    fn find(
        &mut self,
        source: MatchSource,
        haystack: &[u8],
        from: usize,
        bound: usize,
    ) -> Option<RawMatch> {
        match self.set.find(source, haystack, from, bound) {
            Ok(found) => found.map(|(pattern, start, end)| RawMatch {
                pattern,
                start,
                end,
            }),
            Err(_) => {
                self.backtrack_limit_hits += 1;
                None
            }
        }
    }

    /// Searches the whole remaining slice and confines a candidate that
    /// crosses a line break to its line.
    fn find_within_line(&mut self, source: MatchSource, mut from: usize) -> Option<RawMatch> {
        let length = self.haystack.len();
        loop {
            let candidate = self.find(source, self.haystack, from, length)?;
            let (first, line_end) = self.line_bounds(candidate.start);
            if candidate.end <= line_end {
                return Some(candidate);
            }
            let line_start = first.max(from);
            if let Some(found) = self.find(source, self.haystack, line_start, line_end) {
                return Some(found);
            }
            from = line_end + 1;
            if from > length {
                return None;
            }
        }
    }

    /// Searches each line as a separate text: look-around does not see
    /// neighbouring lines, `\A` and `\z` anchor to the line and the step
    /// limit of the backtracking engine applies to one line.
    fn find_line_by_line(&mut self, source: MatchSource, from: usize) -> Option<RawMatch> {
        let haystack = self.haystack;
        let mut start = self.line_bounds(from).0;
        while start < haystack.len() {
            let end = self.line_bounds(start).1;
            let line = &haystack[start..end];
            if let Some(found) = self.find(source, line, from.saturating_sub(start), line.len()) {
                return Some(RawMatch {
                    start: found.start + start,
                    end: found.end + start,
                    ..found
                });
            }
            start = end + 1;
        }
        None
    }
}

/// Offset of the start of the line that contains `pos`.
pub(super) fn line_start(haystack: &[u8], pos: usize) -> usize {
    haystack
        .get(..pos)
        .and_then(|before| memrchr(b'\n', before))
        .map_or(0, |at| at + 1)
}

/// Offset of the line break that ends the line containing `pos`, or the
/// length of the slice for the last line.
pub(super) fn line_end(haystack: &[u8], pos: usize) -> usize {
    haystack
        .get(pos..)
        .and_then(|after| memchr(b'\n', after))
        .map_or(haystack.len(), |at| pos + at)
}

/// Content of the line `start..end` without a `\r` that precedes its line
/// break.
pub(super) fn content(haystack: &[u8], start: usize, end: usize) -> &[u8] {
    let line = &haystack[start..end];
    if end < haystack.len() {
        line.strip_suffix(b"\r").unwrap_or(line)
    } else {
        line
    }
}

/// Number of line breaks in `bytes`.
pub(super) fn count_lines(bytes: &[u8]) -> u64 {
    bytes.iter().filter(|&&byte| byte == b'\n').count() as u64
}

/// Whether `pos` belongs to a line: the position after a final line break
/// and the position in an empty slice do not.
fn is_on_line(haystack: &[u8], pos: usize) -> bool {
    pos < haystack.len()
        || (pos == haystack.len() && haystack.last().is_some_and(|&byte| byte != b'\n'))
}

/// Length of the UTF-8 sequence that starts at `pos`, 1 for an invalid one
/// or for the end of the slice.
fn sequence_length_at(haystack: &[u8], pos: usize) -> usize {
    match haystack.get(pos) {
        Some(&byte) if byte >= 0xF0 => 4,
        Some(&byte) if byte >= 0xE0 => 3,
        Some(&byte) if byte >= 0xC0 => 2,
        _ => 1,
    }
}

/// Converts raw matches of one hit into [`Match`] values.
pub(super) struct Locator<'s, 'h> {
    pub(super) set: &'s PatternSet,
    pub(super) haystack: &'h [u8],
    /// Absolute offset of the first byte of `haystack`.
    pub(super) base: u64,
    /// Offset of the first line of the hit in `haystack`.
    pub(super) hit_start: usize,
    /// Number of the first line of the hit.
    pub(super) hit_line: u64,
    pub(super) multiline: bool,
}

/// Column reached on a line, reused for the next match on the same line.
#[derive(Debug, Clone, Copy)]
struct ColumnCursor {
    line_start: usize,
    offset: usize,
    column: u64,
}

impl Locator<'_, '_> {
    pub(super) fn locate(&self, raw: &[RawMatch], out: &mut Vec<Match>) {
        out.clear();
        let mut cursor = ColumnCursor {
            line_start: self.hit_start,
            offset: self.hit_start,
            column: 1,
        };
        for found in raw {
            out.push(self.locate_one(*found, &mut cursor));
        }
    }

    fn locate_one(&self, found: RawMatch, cursor: &mut ColumnCursor) -> Match {
        let haystack = self.haystack;
        let (line, first) = if self.multiline {
            let start = line_start(haystack, found.start).max(self.hit_start);
            (
                self.hit_line + count_lines(&haystack[self.hit_start..found.start]),
                start,
            )
        } else {
            (self.hit_line, self.hit_start)
        };
        if cursor.line_start != first || cursor.offset > found.start {
            *cursor = ColumnCursor {
                line_start: first,
                offset: first,
                column: 1,
            };
        }
        let column = cursor.column + char_count(&haystack[cursor.offset..found.start]) as u64;
        *cursor = ColumnCursor {
            line_start: first,
            offset: found.start,
            column,
        };
        let (end_line, end_column) = if found.end == found.start {
            (line, column)
        } else if self.multiline {
            let last = found.end - 1;
            let last_start = line_start(haystack, last).max(first);
            let end_line = line + count_lines(&haystack[found.start..last]);
            (
                end_line,
                1 + char_count(&haystack[last_start..found.end]) as u64,
            )
        } else {
            (
                line,
                column + char_count(&haystack[found.start..found.end]) as u64,
            )
        };
        Match {
            pattern: found.pattern,
            start: self.base + found.start as u64,
            end: self.base + found.end as u64,
            line,
            column,
            end_line,
            end_column,
            engine: self
                .set
                .get(found.pattern)
                .map(|p| p.engine())
                .unwrap_or_default(),
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
