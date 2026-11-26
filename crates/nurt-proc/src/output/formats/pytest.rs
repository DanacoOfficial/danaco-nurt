// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Output of `pytest`: the lines `FAILED file::test - message` and
//! `ERROR file::test - message` of the short summary, the traceback line
//! `file.py:line: Exception` and the detail lines `E   text` of each failure,
//! and the closing line `1 failed, 30 passed, 2 warnings in 0.17s`.

use std::collections::HashMap;
use std::sync::OnceLock;

use nurt_contracts::record::{DiagnosticSeverity, ExecDiagnostic};
use regex::Regex;

use super::Tally;

struct Patterns {
    result: Regex,
    header: Regex,
    location: Regex,
    detail: Regex,
    summary: Regex,
    count: Regex,
}

/// The patterns; `None` when one does not compile, which the tests rule out.
fn patterns() -> Option<&'static Patterns> {
    static PATTERNS: OnceLock<Option<Patterns>> = OnceLock::new();
    PATTERNS
        .get_or_init(|| {
            Some(Patterns {
                result: Regex::new(r"^(FAILED|ERROR) (\S+?)(?:::(\S+))?(?: - (.*))?$").ok()?,
                header: Regex::new(r"^_{2,} (?:ERROR collecting )?(.+?) _{2,}$").ok()?,
                location: Regex::new(r"^(\S+\.py):(\d+): (\S.*)$").ok()?,
                detail: Regex::new(r"^E\s+(\S.*)$").ok()?,
                summary: Regex::new(r"^(?:=+ )?((?:\d+ [a-z]+(?:, )?)+) in [\d.]+s").ok()?,
                count: Regex::new(r"(\d+) (failed|errors?|warnings?)").ok()?,
            })
        })
        .as_ref()
}

/// Where and why a test failed, from its traceback.
#[derive(Debug, Default, Clone)]
struct Failure {
    line: Option<u64>,
    code: Option<String>,
    detail: Option<String>,
}

/// What the recogniser remembers between lines.
#[derive(Debug, Default)]
pub(super) struct State {
    name: Option<String>,
    failures: HashMap<String, Failure>,
    summary_errors: u64,
    summary_warnings: Option<u64>,
}

/// Takes one line; returns whether it belongs to the format.
pub(super) fn observe(state: &mut State, tally: &mut Tally, line: &str) -> bool {
    let Some(patterns) = patterns() else {
        return false;
    };
    if let Some(found) = patterns.result.captures(line) {
        let path = found[2].to_owned();
        let name = found.get(3).map_or_else(
            || path.clone(),
            |name| name.as_str().rsplit("::").next().unwrap_or("").to_owned(),
        );
        let failure = state.failures.get(&name).cloned().unwrap_or_default();
        let message = found
            .get(4)
            .map(|text| text.as_str().to_owned())
            .or(failure.detail)
            .unwrap_or_else(|| format!("{} failed", found[1].to_lowercase()));
        tally.error(ExecDiagnostic {
            file: Some(path),
            line: failure.line,
            col: None,
            code: failure.code,
            message,
            severity: DiagnosticSeverity::Error,
            count: None,
        });
        return true;
    }
    if let Some(found) = patterns.header.captures(line) {
        state.name = Some(found[1].to_owned());
        return true;
    }
    if let Some(found) = patterns.location.captures(line) {
        if let Some(name) = state.name.clone()
            && !found[1].contains("site-packages")
        {
            let entry = state.failures.entry(name).or_default();
            entry.line = found[2].parse().ok();
            let word = &found[3];
            entry.code = (word.chars().all(|c| c.is_alphanumeric() || c == '_')
                && word.starts_with(char::is_uppercase))
            .then(|| word.to_owned());
        }
        return true;
    }
    if let Some(found) = patterns.detail.captures(line) {
        if let Some(name) = state.name.clone() {
            let entry = state.failures.entry(name).or_default();
            entry.detail.get_or_insert_with(|| found[1].to_owned());
        }
        return true;
    }
    if let Some(found) = patterns.summary.captures(line) {
        tally.recognised();
        for count in patterns.count.captures_iter(&found[1]) {
            let number: u64 = count[1].parse().unwrap_or(0);
            if count[2].starts_with("warning") {
                state.summary_warnings = Some(number);
            } else {
                state.summary_errors += number;
            }
        }
        return true;
    }
    false
}

/// Applies the counts of the closing line, which also cover the failures
/// the short summary did not list.
pub(super) fn finish(state: &mut State, tally: &mut Tally) {
    tally.totals(state.summary_errors, state.summary_warnings);
}

#[cfg(test)]
mod tests {
    use super::super::tests::collect;

    #[test]
    fn reads_failures_with_the_traceback_line_and_counts_warnings() {
        let found = collect(
            "=== FAILURES ===\n___ test_bad ___\n\n    def test_bad():\n>       assert data['a'] == 2\nE       assert 1 == 2\n\ntest_fx.py:64: AssertionError\n=== short test summary info ===\nFAILED test_fx.py::test_bad - assert 1 == 2\n=== 1 failed, 30 passed, 2 warnings in 0.17s ===\n",
        );
        assert_eq!(
            (found.tool.as_str(), found.errors, found.warnings),
            ("pytest", 1, 2)
        );
        let first = &found.first[0];
        assert_eq!(first.file.as_deref(), Some("test_fx.py"));
        assert_eq!(first.line, Some(64));
        assert_eq!(first.code.as_deref(), Some("AssertionError"));
        assert_eq!(first.message, "assert 1 == 2");
    }

    #[test]
    fn takes_the_message_from_the_detail_line_when_the_summary_has_none() {
        let found = collect(
            "___ test_a ___\nE   ValueError: boom\ntest_a.py:3: ValueError\nFAILED test_a.py::test_a\n1 failed in 0.1s\n",
        );
        assert_eq!(found.first[0].message, "ValueError: boom");
    }

    #[test]
    fn the_closing_line_covers_failures_the_summary_did_not_list() {
        let found = collect("FAILED t.py::a - x\n3 failed, 1 passed in 0.2s\n");
        assert_eq!((found.errors, found.first.len()), (3, 1));
    }

    #[test]
    fn a_passing_run_is_recognised_with_its_warnings() {
        let found = collect("31 passed, 4 warnings in 0.3s\n");
        assert_eq!(
            (found.tool.as_str(), found.errors, found.warnings),
            ("pytest", 0, 4)
        );
    }
}
