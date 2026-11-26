// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Errors and warnings of known tools found in program output.
//!
//! The lines of every stream pass through one [`Collector`], which offers
//! each line to the recognisers of the formats in turn. A recogniser is the
//! function `observe` of the module of its format: it takes the line and the
//! [`Tally`] of its tool and returns whether the line belonged to the format.
//! The result has the numbers of errors and warnings and the first distinct
//! errors of the tool with the most findings.
//!
//! | Module | Tool | Line |
//! |---|---|---|
//! | `cargo` | `cargo`, `rustc` | `error[E0308]: message` then `--> file:line:col` |
//! | `pytest` | `pytest` | `FAILED file::test - message` and the traceback |
//! | `tsc` | `tsc` | `file(line,col): error TS2322: message` |
//! | `gcc` | `gcc`, `clang` | `file:line:col: error: message` |
//! | `go` | `go` | `file.go:line:col: message` |

use std::collections::HashMap;

use nurt_contracts::record::{ExecDiagnostic, ExecDiagnostics};

mod cargo;
mod gcc;
mod go;
mod pytest;
mod tsc;

/// Distinct errors kept in the result.
pub(super) const MAX_ERRORS_LISTED: usize = 10;

/// Characters kept of the message and of the file of an error.
const MAX_FIELD_CHARS: usize = 200;

/// Bytes of the response one listed error takes besides its fields.
const RECORD_OVERHEAD_BYTES: usize = 24;

/// Tools whose output format is recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Cargo,
    Pytest,
    Tsc,
    Gcc,
    Go,
}

impl Tool {
    const ALL: [Self; 5] = [Self::Cargo, Self::Pytest, Self::Tsc, Self::Gcc, Self::Go];

    fn name(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Pytest => "pytest",
            Self::Tsc => "tsc",
            Self::Gcc => "gcc",
            Self::Go => "go",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|tool| *tool == self).unwrap_or(0)
    }
}

/// Counts and the first errors of one tool.
#[derive(Debug, Default)]
pub(super) struct Tally {
    seen: bool,
    errors: u64,
    warnings: u64,
    listed: Vec<ExecDiagnostic>,
    index: HashMap<(Option<String>, String), usize>,
}

impl Tally {
    /// Notes that the output has the format of the tool, even when it has no
    /// error or warning.
    pub(super) fn recognised(&mut self) {
        self.seen = true;
    }

    /// Counts a warning.
    pub(super) fn warning(&mut self) {
        self.seen = true;
        self.warnings += 1;
    }

    /// Counts an error. The first [`MAX_ERRORS_LISTED`] distinct errors, by
    /// code and message, are kept; the same error reported again only raises
    /// the count of the listed one.
    pub(super) fn error(&mut self, mut found: ExecDiagnostic) {
        self.seen = true;
        self.errors += 1;
        found.message = one_line(&found.message);
        found.file = found.file.map(|file| one_line(&file));
        let key = (found.code.clone(), found.message.clone());
        if let Some(&at) = self.index.get(&key) {
            let entry = &mut self.listed[at];
            entry.count = Some(entry.count.unwrap_or(1) + 1);
        } else if self.listed.len() < MAX_ERRORS_LISTED {
            self.index.insert(key, self.listed.len());
            self.listed.push(found);
        }
    }

    /// Replaces the counts the tool reports itself in a summary line.
    pub(super) fn totals(&mut self, errors: u64, warnings: Option<u64>) {
        self.errors = self.errors.max(errors);
        if let Some(warnings) = warnings {
            self.warnings = warnings;
        }
    }
}

/// Collects errors and warnings from the lines of the output.
#[derive(Debug, Default)]
pub(super) struct Collector {
    tallies: [Tally; 5],
    cargo: cargo::State,
    pytest: pytest::State,
}

impl Collector {
    /// Looks at one cleaned line.
    pub(super) fn observe(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        let _ = cargo::observe(
            &mut self.cargo,
            &mut self.tallies[Tool::Cargo.index()],
            line,
        ) || pytest::observe(
            &mut self.pytest,
            &mut self.tallies[Tool::Pytest.index()],
            line,
        ) || tsc::observe(&mut self.tallies[Tool::Tsc.index()], line)
            || gcc::observe(&mut self.tallies[Tool::Gcc.index()], line)
            || go::observe(&mut self.tallies[Tool::Go.index()], line);
    }

    /// The numbers and the first errors of the tool with the most findings,
    /// or `None` when no known format was seen.
    pub(super) fn finish(mut self) -> Option<ExecDiagnostics> {
        cargo::finish(&mut self.cargo, &mut self.tallies[Tool::Cargo.index()]);
        pytest::finish(&mut self.pytest, &mut self.tallies[Tool::Pytest.index()]);
        let best = Tool::ALL
            .into_iter()
            .filter(|tool| self.tallies[tool.index()].seen)
            .max_by_key(|tool| {
                let tally = &self.tallies[tool.index()];
                (
                    tally.errors + tally.warnings,
                    std::cmp::Reverse(tool.index()),
                )
            })?;
        let tally = std::mem::take(&mut self.tallies[best.index()]);
        Some(ExecDiagnostics {
            tool: best.name().to_owned(),
            errors: tally.errors,
            warnings: tally.warnings,
            first: tally.listed,
        })
    }
}

/// Text of one line of at most [`MAX_FIELD_CHARS`] characters.
fn one_line(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = joined.chars().count();
    if length <= MAX_FIELD_CHARS {
        return joined;
    }
    let kept: String = joined.chars().take(MAX_FIELD_CHARS).collect();
    format!("{kept}\u{2026} [+{} characters]", length - MAX_FIELD_CHARS)
}

/// Bytes of the response one listed error takes.
pub(super) fn entry_bytes(entry: &ExecDiagnostic) -> usize {
    RECORD_OVERHEAD_BYTES
        + entry.message.len()
        + entry.file.as_ref().map_or(0, String::len)
        + entry.code.as_ref().map_or(0, String::len)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The result of the lines of `text`, which must be a recognised format.
    pub(super) fn collect(text: &str) -> ExecDiagnostics {
        let mut collector = Collector::default();
        for line in text.lines() {
            collector.observe(line);
        }
        collector.finish().expect("a format is recognised")
    }

    #[test]
    fn lists_at_most_ten_distinct_errors() {
        let mut tally = Tally::default();
        for number in 0..30 {
            tally.error(ExecDiagnostic {
                message: format!("problem {number}"),
                ..ExecDiagnostic::default()
            });
        }
        assert_eq!((tally.errors, tally.listed.len()), (30, 10));
    }

    #[test]
    fn collapses_the_same_error_reported_in_many_places() {
        let mut tally = Tally::default();
        for line in 1..=5 {
            tally.error(ExecDiagnostic {
                file: Some("a.rs".to_owned()),
                line: Some(line),
                code: Some("E0599".to_owned()),
                message: "no method named `go`".to_owned(),
                ..ExecDiagnostic::default()
            });
        }
        assert_eq!((tally.errors, tally.listed.len()), (5, 1));
        assert_eq!(tally.listed[0].count, Some(5));
    }

    #[test]
    fn long_messages_stay_on_one_line_and_are_marked() {
        let mut tally = Tally::default();
        tally.error(ExecDiagnostic {
            message: "word\n".repeat(100),
            ..ExecDiagnostic::default()
        });
        let text = &tally.listed[0].message;
        assert!(!text.contains('\n') && text.contains("characters]"));
        assert!(text.chars().count() < 230);
    }

    #[test]
    fn plain_text_is_not_recognised() {
        let mut collector = Collector::default();
        collector.observe("hello: world 1:2");
        collector.observe("note: nothing to see");
        assert!(collector.finish().is_none());
    }
}
