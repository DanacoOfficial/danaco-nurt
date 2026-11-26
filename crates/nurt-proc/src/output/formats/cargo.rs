// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Output of `cargo` and `rustc`: `error[E0308]: message` or
//! `warning: message`, followed by the line `--> file:line:col`.

use std::sync::OnceLock;

use nurt_contracts::record::{DiagnosticSeverity, ExecDiagnostic};
use regex::Regex;

use super::Tally;

struct Patterns {
    head: Regex,
    location: Regex,
    finished: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let compile = |source: &str| Regex::new(source).expect("cargo pattern is valid");
        Patterns {
            head: compile(r"^(error|warning)(?:\[([A-Za-z]\d+)\])?: (.+)$"),
            location: compile(r"^\s*--> (.+?):(\d+):(\d+)\s*$"),
            finished: compile(r"^\s+Finished `?\w+`? "),
        }
    })
}

/// Error or warning waiting for its `-->` line.
#[derive(Debug)]
struct Pending {
    error: bool,
    code: Option<String>,
    message: String,
}

/// What the recogniser remembers between lines.
#[derive(Debug, Default)]
pub(super) struct State {
    pending: Option<Pending>,
}

/// Takes one line; returns whether it belongs to the format.
pub(super) fn observe(state: &mut State, tally: &mut Tally, line: &str) -> bool {
    let patterns = patterns();
    if let Some(found) = patterns.location.captures(line) {
        if let Some(pending) = state.pending.take() {
            settle(
                tally,
                pending,
                Some((
                    found[1].to_owned(),
                    found[2].parse().ok(),
                    found[3].parse().ok(),
                )),
            );
        }
        return true;
    }
    if let Some(found) = patterns.head.captures(line) {
        flush(state, tally);
        let message = found[3].to_owned();
        if !is_summary(&message) {
            state.pending = Some(Pending {
                error: &found[1] == "error",
                code: found.get(2).map(|code| code.as_str().to_owned()),
                message,
            });
        }
        return true;
    }
    if patterns.finished.is_match(line) {
        tally.recognised();
        return true;
    }
    false
}

/// Settles the error or warning that no `-->` line followed.
pub(super) fn finish(state: &mut State, tally: &mut Tally) {
    flush(state, tally);
}

fn flush(state: &mut State, tally: &mut Tally) {
    if let Some(pending) = state.pending.take() {
        settle(tally, pending, None);
    }
}

fn settle(tally: &mut Tally, pending: Pending, place: Option<(String, Option<u64>, Option<u64>)>) {
    if !pending.error {
        tally.warning();
        return;
    }
    let (file, line, col) = match place {
        Some((file, line, col)) => (Some(file), line, col),
        None => (None, None, None),
    };
    tally.error(ExecDiagnostic {
        file,
        line,
        col,
        code: pending.code,
        message: pending.message,
        severity: DiagnosticSeverity::Error,
        count: None,
    });
}

/// Summary lines of `cargo` and `rustc` that are not findings themselves.
fn is_summary(message: &str) -> bool {
    message.starts_with("could not compile")
        || message.starts_with("aborting due to")
        || message.starts_with("test failed")
        || message.starts_with("process didn't exit successfully")
        || message.starts_with("build failed")
        || message.starts_with("some errors have")
        || (message.contains(" generated ") && message.contains("warning"))
        || (message.ends_with(" emitted") && message.contains("warning"))
}

#[cfg(test)]
mod tests {
    use super::super::tests::collect;

    #[test]
    fn reads_errors_with_locations_and_counts_warnings() {
        let found = collect(
            "warning: unused variable: `x`\n  --> src/main.rs:3:9\nerror[E0308]: mismatched types\n  --> src/main.rs:41:21\n   |\nerror[E0425]: cannot find value `m`\n --> src/lib.rs:7:5\nwarning: `fx` (bin \"fx\") generated 1 warning\nerror: could not compile `fx` (bin \"fx\") due to 2 previous errors; 1 warning emitted\n",
        );
        assert_eq!(
            (found.tool.as_str(), found.errors, found.warnings),
            ("cargo", 2, 1)
        );
        let first = &found.first[0];
        assert_eq!(first.file.as_deref(), Some("src/main.rs"));
        assert_eq!((first.line, first.col), (Some(41), Some(21)));
        assert_eq!(first.code.as_deref(), Some("E0308"));
        assert_eq!(found.first[1].file.as_deref(), Some("src/lib.rs"));
    }

    #[test]
    fn an_error_without_a_location_is_counted_and_listed() {
        let found = collect("error: linker `cc` not found\nerror: could not compile `fx`\n");
        assert_eq!((found.errors, found.first.len()), (1, 1));
        assert_eq!(found.first[0].file, None);
    }

    #[test]
    fn a_run_without_findings_is_still_recognised() {
        let found = collect(
            "   Compiling a v0.1.0\n    Finished `dev` profile [unoptimized] target(s) in 1.2s\n",
        );
        assert_eq!(
            (found.tool.as_str(), found.errors, found.warnings),
            ("cargo", 0, 0)
        );
    }
}
