// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Search of a byte slice held in memory.

use std::mem;
use std::ops::ControlFlow;

use super::Searcher;
use super::emit::Emitter;
use super::hit::{ContextLine, Hit, Match, Summary};
use super::pattern::PatternSet;
use super::scan::{Locator, RawMatch, Scanner, content, count_lines, line_end, line_start};

/// Lines of one hit: `start` is the offset of the first line, `end` the
/// offset of the line break after the last line or the length of the slice.
#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
    line: u64,
    end_line: u64,
}

/// Line numbers of increasing offsets.
#[derive(Debug)]
struct LineCounter {
    pos: usize,
    line: u64,
}

impl LineCounter {
    fn line_at(&mut self, haystack: &[u8], offset: usize) -> u64 {
        if let Some(skipped) = haystack.get(self.pos..offset) {
            self.line += count_lines(skipped);
            self.pos = offset;
        }
        self.line
    }
}

/// Hits of a slice in order, without context.
struct HitSource<'s, 'h> {
    scanner: Scanner<'s, 'h>,
    haystack: &'h [u8],
    multiline: bool,
    invert: bool,
    /// Match found after the end of the previous block.
    pending: Option<RawMatch>,
    /// Start of the next line to consider with `invert`.
    cursor: usize,
    /// Next block of matched lines with `invert`; `Some(None)` after the
    /// last block.
    block_ahead: Option<Option<(usize, usize)>>,
    lines: LineCounter,
    spare: Vec<RawMatch>,
}

impl<'s, 'h> HitSource<'s, 'h> {
    fn new(set: &'s PatternSet, haystack: &'h [u8], multiline: bool, invert: bool) -> Self {
        Self {
            scanner: Scanner::new(set, haystack, multiline),
            haystack,
            multiline,
            invert,
            pending: None,
            cursor: 0,
            block_ahead: None,
            lines: LineCounter { pos: 0, line: 1 },
            spare: Vec::new(),
        }
    }

    fn next_hit(&mut self, matches: &mut Vec<RawMatch>) -> Option<Span> {
        let (start, end) = if self.invert {
            matches.clear();
            self.next_unmatched_line()?
        } else {
            self.next_block(matches, false)?
        };
        let line = self.lines.line_at(self.haystack, start);
        let end_line = line + count_lines(&self.haystack[start..end]);
        Some(Span {
            start,
            end,
            line,
            end_line,
        })
    }

    /// Lines covered by the next match and by every following match that
    /// starts on the last of those lines.
    fn next_block(
        &mut self,
        matches: &mut Vec<RawMatch>,
        first_only: bool,
    ) -> Option<(usize, usize)> {
        matches.clear();
        let first = self.pending.take().or_else(|| self.scanner.next_match())?;
        let start = line_start(self.haystack, first.start);
        let mut end = self.last_line_end(first);
        matches.push(first);
        if first_only {
            self.scanner.skip_to(end + 1);
            return Some((start, end));
        }
        while let Some(found) = self.scanner.next_match() {
            if found.start > end {
                self.pending = Some(found);
                break;
            }
            end = end.max(self.last_line_end(found));
            matches.push(found);
        }
        Some((start, end))
    }

    fn last_line_end(&self, found: RawMatch) -> usize {
        if self.multiline && found.end > found.start {
            line_end(self.haystack, found.end - 1)
        } else {
            line_end(self.haystack, found.start)
        }
    }

    fn next_unmatched_line(&mut self) -> Option<(usize, usize)> {
        loop {
            let block = match self.block_ahead {
                Some(block) => block,
                None => {
                    let mut spare = mem::take(&mut self.spare);
                    let block = self.next_block(&mut spare, !self.multiline);
                    self.spare = spare;
                    self.block_ahead = Some(block);
                    block
                }
            };
            let limit = block.map_or(self.haystack.len(), |(start, _)| start);
            if self.cursor < limit {
                let start = self.cursor;
                let end = line_end(self.haystack, start);
                self.cursor = end + 1;
                return Some((start, end));
            }
            let (_, end) = block?;
            self.cursor = end + 1;
            self.block_ahead = None;
        }
    }
}

/// State of one search between hits.
struct SliceRun<'s, 'h, F> {
    searcher: &'s Searcher,
    haystack: &'h [u8],
    emitter: Emitter<F>,
    /// Start of the first line not yet passed to the sink as a hit or as
    /// context.
    floor: usize,
    located: Vec<Match>,
    before: Vec<ContextLine<'h>>,
    after: Vec<ContextLine<'h>>,
}

impl<F> SliceRun<'_, '_, F>
where
    F: FnMut(&Hit<'_>) -> ControlFlow<()>,
{
    fn emit(&mut self, span: Span, raw: &[RawMatch], bound: usize) -> ControlFlow<()> {
        let options = &self.searcher.options;
        let haystack = self.haystack;
        let locator = Locator {
            set: &self.searcher.set,
            haystack,
            base: 0,
            hit_start: span.start,
            hit_line: span.line,
            multiline: options.multiline,
        };
        locator.locate(raw, &mut self.located);
        self.before.clear();
        let mut next_start = span.start;
        let mut line = span.line;
        while self.before.len() < options.before && next_start > self.floor {
            let end = next_start - 1;
            let start = line_start(haystack, end);
            line -= 1;
            self.before
                .push(context_line(haystack, start, end, line, options.max_line));
            next_start = start;
        }
        self.before.reverse();
        self.after.clear();
        let mut start = span.end + 1;
        let mut line = span.end_line;
        while self.after.len() < options.after && start < bound.min(haystack.len()) {
            let end = line_end(haystack, start);
            line += 1;
            self.after
                .push(context_line(haystack, start, end, line, options.max_line));
            start = end + 1;
        }
        self.floor = start;
        let hit = Hit {
            line: span.line,
            end_line: span.end_line,
            start: span.start as u64,
            bytes: content(haystack, span.start, span.end),
            matches: &self.located,
            before: &self.before,
            after: &self.after,
            max_line: options.max_line,
            haystack,
            haystack_offset: 0,
        };
        self.emitter.emit(&hit)
    }
}

fn context_line(
    haystack: &[u8],
    start: usize,
    end: usize,
    line: u64,
    max_line: Option<usize>,
) -> ContextLine<'_> {
    ContextLine {
        line,
        start: start as u64,
        bytes: content(haystack, start, end),
        max_line,
    }
}

/// Searches `haystack` and passes each hit to `sink`.
pub(super) fn search<F>(searcher: &Searcher, haystack: &[u8], sink: F) -> Summary
where
    F: FnMut(&Hit<'_>) -> ControlFlow<()>,
{
    let options = &searcher.options;
    let mut source = HitSource::new(&searcher.set, haystack, options.multiline, options.invert);
    let mut run = SliceRun {
        searcher,
        haystack,
        emitter: Emitter::new(options, searcher.set.len(), sink),
        floor: 0,
        located: Vec::new(),
        before: Vec::new(),
        after: Vec::new(),
    };
    let mut current_raw = Vec::new();
    let mut next_raw = Vec::new();
    let mut current = source.next_hit(&mut current_raw);
    while let Some(span) = current {
        let Some(admission) = run.emitter.admit(current_raw.len()) else {
            run.emitter.mark_limit_reached();
            break;
        };
        let next = source.next_hit(&mut next_raw);
        if admission.keep < current_raw.len() || (admission.last && next.is_some()) {
            run.emitter.mark_limit_reached();
        }
        let bound = match next {
            Some(next) if !admission.last => next.start,
            _ => haystack.len(),
        };
        if run
            .emit(span, &current_raw[..admission.keep], bound)
            .is_break()
            || admission.last
        {
            break;
        }
        current = next;
        mem::swap(&mut current_raw, &mut next_raw);
    }
    run.emitter.finish(source.scanner.backtrack_limit_hits())
}
