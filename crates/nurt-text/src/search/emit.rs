// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Delivery of hits to the sink of the caller, with `max_per_file` and the
//! counters of a search.

use std::ops::ControlFlow;

use super::SearchOptions;
use super::hit::{Hit, Summary, is_long};

/// Room left for one hit under `max_per_file`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Admission {
    /// Number of matches of the hit to report.
    pub(super) keep: usize,
    /// Whether the hit uses up the limit.
    pub(super) last: bool,
}

/// Passes hits to the sink and counts them.
pub(super) struct Emitter<F> {
    sink: F,
    summary: Summary,
    /// Units left under `max_per_file`; `None` without a limit.
    remaining: Option<u64>,
    /// Whether a unit is a match (`only_matching` without `invert`) rather
    /// than a hit.
    per_match: bool,
    max_line: Option<usize>,
}

impl<F> Emitter<F>
where
    F: FnMut(&Hit<'_>) -> ControlFlow<()>,
{
    pub(super) fn new(options: &SearchOptions, patterns: usize, sink: F) -> Self {
        Self {
            sink,
            summary: Summary::new(patterns),
            remaining: options.max_per_file,
            per_match: options.only_matching && !options.invert,
            max_line: options.max_line,
        }
    }

    /// Reserves room for a hit with `matches` matches; `None` when the limit
    /// is used up.
    pub(super) fn admit(&mut self, matches: usize) -> Option<Admission> {
        let Some(remaining) = self.remaining.as_mut() else {
            return Some(Admission {
                keep: matches,
                last: false,
            });
        };
        if *remaining == 0 {
            return None;
        }
        let keep = if self.per_match {
            let keep = usize::try_from(*remaining).map_or(matches, |left| left.min(matches));
            *remaining -= keep as u64;
            keep
        } else {
            *remaining -= 1;
            matches
        };
        Some(Admission {
            keep,
            last: *remaining == 0,
        })
    }

    pub(super) fn mark_limit_reached(&mut self) {
        self.summary.limit_reached = true;
    }

    pub(super) const fn limit_reached(&self) -> bool {
        self.summary.limit_reached
    }

    pub(super) fn emit(&mut self, hit: &Hit<'_>) -> ControlFlow<()> {
        let summary = &mut self.summary;
        summary.hits += 1;
        summary.matches += hit.matches().len() as u64;
        for found in hit.matches() {
            if let Some(count) = summary.per_pattern.get_mut(found.pattern) {
                *count += 1;
            }
        }
        if self.max_line.is_some() {
            let context = hit
                .before()
                .iter()
                .chain(hit.after())
                .map(|line| line.bytes());
            let long = hit
                .lines()
                .chain(context)
                .filter(|line| is_long(line, self.max_line))
                .count();
            summary.truncated_lines += long as u64;
        }
        (self.sink)(hit)
    }

    pub(super) fn finish(mut self, backtrack_limit_hits: u64) -> Summary {
        self.summary.backtrack_limit_hits = backtrack_limit_hits;
        self.summary
    }
}
