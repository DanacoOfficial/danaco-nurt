// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-text::search::reader
// Responsibility:   Search of a reader line by line with a bounded buffer.
// Dependencies:     grep_regex, grep_searcher
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Search of a reader line by line with a bounded buffer.
//!
//! `grep-searcher` selects the matched lines and their context with the
//! joint line filter of all patterns; the matches within a selected line
//! are then found by the pattern set, as in the search of a slice.

use std::io;
use std::mem;
use std::ops::ControlFlow;
use std::sync::{Mutex, PoisonError};

use grep_regex::RegexMatcher;
use grep_searcher::{
    BinaryDetection, Searcher as LineSearcher, SearcherBuilder, Sink, SinkContext, SinkContextKind,
    SinkMatch,
};

use super::emit::Emitter;
use super::hit::{ContextLine, Hit, Match, Summary};
use super::scan::{Locator, Scanner};
use super::{SearchOptions, Searcher};

/// Line copied out of the buffer of the line searcher.
#[derive(Debug)]
struct OwnedLine {
    line: u64,
    start: u64,
    /// Content followed by `\n` whether or not the input ends the line, so
    /// that the scanner treats an empty line as a line.
    bytes: Vec<u8>,
}

impl OwnedLine {
    fn new(line: Option<u64>, start: u64, bytes: &[u8]) -> Self {
        // A `\r` belongs to the line ending only before a `\n`; the search of
        // a slice reads the last line of a text without a line ending the
        // same way, so that both give the same hits.
        let bytes = match bytes.strip_suffix(b"\n") {
            Some(ended) => ended.strip_suffix(b"\r").unwrap_or(ended),
            None => bytes,
        };
        let mut owned = Vec::with_capacity(bytes.len() + 1);
        owned.extend_from_slice(bytes);
        owned.push(b'\n');
        // Line numbers are enabled on the line searcher, so every line
        // carries one.
        Self {
            line: line.unwrap_or_default(),
            start,
            bytes: owned,
        }
    }

    /// Content without the line ending.
    fn content(&self) -> &[u8] {
        self.bytes.strip_suffix(b"\n").unwrap_or(&self.bytes)
    }

    fn context(&self, max_line: Option<usize>) -> ContextLine<'_> {
        ContextLine {
            line: self.line,
            start: self.start,
            bytes: self.content(),
            max_line,
        }
    }
}

/// Hit waiting for its after context.
#[derive(Debug)]
struct PendingHit {
    line: OwnedLine,
    matches: Vec<Match>,
    before: Vec<OwnedLine>,
    after: Vec<OwnedLine>,
}

/// Receiver of the lines selected by the line searcher.
struct LineSink<'s, F> {
    searcher: &'s Searcher,
    emitter: Emitter<F>,
    before: Vec<OwnedLine>,
    pending: Option<PendingHit>,
    /// Whether `max_per_file` is used up and only the after context of the
    /// last hit is collected.
    exhausted: bool,
    stopped: bool,
    backtrack_limit_hits: u64,
}

impl<F> LineSink<'_, F>
where
    F: FnMut(&Hit<'_>) -> ControlFlow<()>,
{
    /// Handles a selected line; returns whether the search continues.
    fn on_selected(&mut self, line: OwnedLine) -> bool {
        if self.stopped {
            return false;
        }
        if self.exhausted {
            self.emitter.mark_limit_reached();
            return self.on_after(line);
        }
        self.flush();
        if self.stopped {
            return false;
        }
        let mut matches = self.locate(&line);
        if !self.searcher.options.invert && matches.is_empty() {
            self.before.clear();
            return true;
        }
        let Some(admission) = self.emitter.admit(matches.len()) else {
            self.emitter.mark_limit_reached();
            return false;
        };
        if admission.keep < matches.len() {
            self.emitter.mark_limit_reached();
            matches.truncate(admission.keep);
        }
        let before = mem::take(&mut self.before);
        self.pending = Some(PendingHit {
            line,
            matches,
            before,
            after: Vec::new(),
        });
        if admission.last {
            self.exhausted = true;
            return self.is_running();
        }
        true
    }

    fn on_after(&mut self, line: OwnedLine) -> bool {
        let wanted = self.searcher.options.after;
        if let Some(pending) = self.pending.as_mut()
            && pending.after.len() < wanted
        {
            pending.after.push(line);
        }
        self.is_running()
    }

    /// Whether the search continues after `max_per_file` is used up: until
    /// the after context is complete and another hit shows that the limit
    /// left something out.
    fn is_running(&mut self) -> bool {
        if !self.exhausted {
            return !self.stopped;
        }
        let wanted = self.searcher.options.after;
        let complete = self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.after.len() >= wanted);
        if complete && self.emitter.limit_reached() {
            self.flush();
            return false;
        }
        !self.stopped
    }

    fn locate(&mut self, line: &OwnedLine) -> Vec<Match> {
        let mut located = Vec::new();
        if self.searcher.options.invert {
            return located;
        }
        let mut scanner = Scanner::new(&self.searcher.set, &line.bytes, false);
        let raw: Vec<_> = std::iter::from_fn(|| scanner.next_match()).collect();
        self.backtrack_limit_hits += scanner.backtrack_limit_hits();
        let locator = Locator {
            set: &self.searcher.set,
            haystack: &line.bytes,
            base: line.start,
            hit_start: 0,
            hit_line: line.line,
            multiline: false,
        };
        locator.locate(&raw, &mut located);
        located
    }

    fn flush(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if self.stopped {
            return;
        }
        let max_line = self.searcher.options.max_line;
        let before: Vec<_> = pending
            .before
            .iter()
            .map(|line| line.context(max_line))
            .collect();
        let after: Vec<_> = pending
            .after
            .iter()
            .map(|line| line.context(max_line))
            .collect();
        let hit = Hit {
            line: pending.line.line,
            end_line: pending.line.line,
            start: pending.line.start,
            bytes: pending.line.content(),
            matches: &pending.matches,
            before: &before,
            after: &after,
            max_line,
            haystack: &pending.line.bytes,
            haystack_offset: pending.line.start,
        };
        if self.emitter.emit(&hit).is_break() {
            self.stopped = true;
        }
    }
}

impl<F> Sink for LineSink<'_, F>
where
    F: FnMut(&Hit<'_>) -> ControlFlow<()>,
{
    type Error = io::Error;

    fn matched(
        &mut self,
        _searcher: &LineSearcher,
        found: &SinkMatch<'_>,
    ) -> Result<bool, io::Error> {
        let mut start = found.absolute_byte_offset();
        let first = found.line_number();
        for (index, bytes) in found.lines().enumerate() {
            let line = OwnedLine::new(first.map(|line| line + index as u64), start, bytes);
            start += bytes.len() as u64;
            if !self.on_selected(line) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn context(
        &mut self,
        _searcher: &LineSearcher,
        context: &SinkContext<'_>,
    ) -> Result<bool, io::Error> {
        let line = OwnedLine::new(
            context.line_number(),
            context.absolute_byte_offset(),
            context.bytes(),
        );
        Ok(match context.kind() {
            SinkContextKind::Before if !self.exhausted => {
                self.before.push(line);
                true
            }
            SinkContextKind::After => self.on_after(line),
            SinkContextKind::Before | SinkContextKind::Other => true,
        })
    }
}

/// Idle line searchers of a [`Searcher`].
///
/// A line searcher owns the buffers it reads a file into, and its
/// configuration depends on the options of the search only. Building one
/// for every file of a large tree allocates and configures it again and
/// again, so a search takes an idle one and gives it back when the file is
/// done; searches on several threads take one each.
#[derive(Debug, Default)]
pub(super) struct LinePool(Mutex<Vec<LineSearcher>>);

impl LinePool {
    fn take(&self, options: &SearchOptions) -> LineSearcher {
        let idle = self.0.lock().unwrap_or_else(PoisonError::into_inner).pop();
        idle.unwrap_or_else(|| {
            SearcherBuilder::new()
                .line_number(true)
                .invert_match(options.invert)
                .before_context(options.before)
                .after_context(options.after)
                .bom_sniffing(false)
                .binary_detection(BinaryDetection::none())
                .build()
        })
    }

    fn give_back(&self, line_searcher: LineSearcher) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line_searcher);
    }

    /// Number of idle line searchers.
    #[cfg(test)]
    pub(super) fn idle(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

/// Searches `reader` with the line filter `matcher` and passes each hit to
/// `sink`.
pub(super) fn search<R, F>(
    searcher: &Searcher,
    matcher: &RegexMatcher,
    reader: R,
    sink: F,
) -> io::Result<Summary>
where
    R: io::Read,
    F: FnMut(&Hit<'_>) -> ControlFlow<()>,
{
    let options = &searcher.options;
    let mut line_searcher = searcher.line_pool.take(options);
    let mut line_sink = LineSink {
        searcher,
        emitter: Emitter::new(options, searcher.set.len(), sink),
        before: Vec::new(),
        pending: None,
        exhausted: false,
        stopped: false,
        backtrack_limit_hits: 0,
    };
    let searched = line_searcher.search_reader(matcher, reader, &mut line_sink);
    searcher.line_pool.give_back(line_searcher);
    searched?;
    line_sink.flush();
    Ok(line_sink.emitter.finish(line_sink.backtrack_limit_hits))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
