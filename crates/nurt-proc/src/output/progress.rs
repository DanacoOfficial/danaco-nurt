// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::output::progress
// Responsibility:   Summary lines and progress taken from program output.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Summary lines and progress taken from program output.

use nurt_contracts::record::Progress;

use super::filter::Pattern;

/// Patterns of `summary:` and `progress:`, compiled once per response.
#[derive(Debug, Default)]
pub(super) struct Watch {
    summary: Vec<Pattern>,
    progress: Option<Pattern>,
}

impl Watch {
    /// Watches the lines matching `summary` and `progress`.
    pub(super) fn new(summary: Vec<Pattern>, progress: Option<Pattern>) -> Self {
        Self { summary, progress }
    }

    /// Whether any pattern is set.
    pub(super) fn is_active(&self) -> bool {
        !self.summary.is_empty() || self.progress.is_some()
    }

    /// Matches found in one stream.
    pub(super) fn observer(&self) -> Observer<'_> {
        Observer {
            watch: self,
            summary: vec![None; self.summary.len()],
            progress: None,
        }
    }
}

/// Last matches of the watched patterns in one stream.
#[derive(Debug)]
pub(super) struct Observer<'w> {
    watch: &'w Watch,
    summary: Vec<Option<String>>,
    progress: Option<Progress>,
}

impl Observer<'_> {
    /// Checks one cleaned line.
    pub(super) fn observe(&mut self, line: &str) {
        for (pattern, last) in self.watch.summary.iter().zip(&mut self.summary) {
            if pattern.is_match(line) {
                *last = Some(line.to_owned());
            }
        }
        if let Some(percent) = self
            .watch
            .progress
            .as_ref()
            .and_then(|pattern| pattern.number(line))
        {
            self.progress = Some(Progress {
                percent,
                line: line.to_owned(),
            });
        }
    }
}

/// Matches of all streams.
#[derive(Debug, Default)]
pub(super) struct Findings {
    /// Last matching line of each summary pattern that matched, in the order
    /// of the patterns.
    pub(super) summary_lines: Vec<String>,
    /// Last progress line.
    pub(super) progress: Option<Progress>,
}

/// Combines the matches of the streams in order of preference.
///
/// Separate spool files do not record the order of lines across streams, so
/// a match in an earlier stream (standard output) takes precedence over a
/// match in a later one (standard error).
pub(super) fn combine(observers: Vec<Observer<'_>>) -> Findings {
    let mut summary: Vec<Option<String>> = Vec::new();
    let mut progress = None;
    for observer in observers {
        if summary.is_empty() {
            summary = observer.summary;
        } else {
            for (kept, found) in summary.iter_mut().zip(observer.summary) {
                if kept.is_none() {
                    *kept = found;
                }
            }
        }
        progress = progress.or(observer.progress);
    }
    Findings {
        summary_lines: summary.into_iter().flatten().collect(),
        progress,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch() -> Watch {
        Watch::new(
            vec![
                Pattern::compile("summary", r"\d+ passed").unwrap(),
                Pattern::compile("summary", r"^Finished").unwrap(),
            ],
            Some(Pattern::compile("progress", r"(\d+)/\d+ (\d+)%").unwrap()),
        )
    }

    #[test]
    fn keeps_last_match_of_each_pattern() {
        let watch = watch();
        let mut observer = watch.observer();
        for line in ["1 passed", "x 1/4 25%", "3 passed", "y 2/4 50%", "other"] {
            observer.observe(line);
        }
        let found = combine(vec![observer]);
        assert_eq!(found.summary_lines, ["3 passed"]);
        let progress = found.progress.unwrap();
        assert_eq!(progress.percent, Some(2.0));
        assert_eq!(progress.line, "y 2/4 50%");
    }

    #[test]
    fn prefers_standard_output_over_standard_error() {
        let watch = watch();
        let mut stdout = watch.observer();
        stdout.observe("5 passed");
        let mut stderr = watch.observer();
        stderr.observe("9 passed");
        stderr.observe("Finished dev");
        stderr.observe("a 3/3 100%");
        let found = combine(vec![stdout, stderr]);
        assert_eq!(found.summary_lines, ["5 passed", "Finished dev"]);
        assert_eq!(found.progress.unwrap().line, "a 3/3 100%");
        assert!(watch.is_active());
        assert!(!Watch::default().is_active());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
