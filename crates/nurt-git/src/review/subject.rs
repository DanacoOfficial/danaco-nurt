// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::review::subject
// Responsibility:   Rules of the subject line of a commit message.
// Dependencies:     nurt_contracts, regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Rules of the subject line of a commit message.

use std::sync::LazyLock;

use nurt_contracts::record::{Finding, FindingKind, FindingSeverity, FindingTarget};
use nurt_contracts::value::Path as RecordPath;
use regex::Regex;

/// Default longest subject line, in characters.
pub const SUBJECT_MAX: usize = 72;

/// Smallest accepted value of `subject_max`.
pub const SUBJECT_MAX_MIN: usize = 20;

/// Conventional Commits header: type, optional scope, optional `!`, colon,
/// space and description.
const HEADER_PATTERN: &str = r"^(?P<type>[A-Za-z]+)(?:\((?P<scope>[^()\s]+)\))?!?: (?P<desc>\S.*)$";

/// First words of a description that are not in the imperative mood.
const NON_IMPERATIVE: [&str; 36] = [
    "added",
    "adds",
    "adding",
    "fixed",
    "fixes",
    "fixing",
    "updated",
    "updates",
    "updating",
    "removed",
    "removes",
    "removing",
    "changed",
    "changes",
    "changing",
    "implemented",
    "implements",
    "implementing",
    "refactored",
    "refactors",
    "refactoring",
    "created",
    "creates",
    "creating",
    "deleted",
    "deletes",
    "deleting",
    "moved",
    "moves",
    "moving",
    "renamed",
    "renames",
    "renaming",
    "improved",
    "improves",
    "improving",
];

#[expect(
    clippy::expect_used,
    reason = "the pattern is a constant checked by the tests of this module"
)]
static HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(HEADER_PATTERN).expect("valid header pattern"));

/// A finding of the commit message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageFinding {
    /// Rule, for example `subject_length`.
    pub rule: &'static str,
    /// Severity.
    pub severity: FindingSeverity,
    /// Line of the message, from 1.
    pub line: u64,
    /// Description with the correction.
    pub message: String,
}

impl MessageFinding {
    fn warning(rule: &'static str, line: u64, message: String) -> Self {
        Self {
            rule,
            severity: FindingSeverity::Warning,
            line,
            message,
        }
    }

    fn error(rule: &'static str, line: u64, message: String) -> Self {
        Self {
            rule,
            severity: FindingSeverity::Error,
            line,
            message,
        }
    }

    /// The `finding` record of the message of the repository at `root`.
    #[must_use]
    pub fn into_finding(self, root: &str) -> Finding {
        Finding {
            path: RecordPath::new(root),
            kind: FindingKind::Subject,
            severity: self.severity,
            message: self.message,
            line: Some(self.line),
            rule: Some(self.rule.to_owned()),
            target: Some(FindingTarget::Message),
            ..Finding::default()
        }
    }
}

/// Checks the subject line of `message`.
///
/// Leading blank lines are skipped, as the cleanup of `git commit` does. The
/// rules are `subject_empty` (error), `subject_length` (more than
/// `subject_max` characters), `subject_period` (a final full stop),
/// `subject_mood` (a first word such as `added` or `fixes`) and `blank_line`
/// (the second line is not blank). With `conventional`, the subject must be a
/// Conventional Commits header (`conventional_header`, error); a header
/// without scope (`missing_scope`) and a description starting with an upper
/// case letter (`subject_case`) are warnings.
///
/// # Examples
///
/// ```
/// use nurt_git::review::check_message;
///
/// let findings = check_message("Fixed the parser.", 72, true);
/// let rules: Vec<&str> = findings.iter().map(|finding| finding.rule).collect();
/// assert_eq!(rules, ["conventional_header", "subject_period", "subject_mood"]);
/// assert!(check_message("fix(parser): accept empty input", 72, true).is_empty());
/// ```
#[must_use]
pub fn check_message(message: &str, subject_max: usize, conventional: bool) -> Vec<MessageFinding> {
    let lines: Vec<&str> = message
        .lines()
        .skip_while(|line| line.trim().is_empty())
        .collect();
    let Some(subject) = lines.first().map(|line| line.trim_end()) else {
        return vec![MessageFinding::error(
            "subject_empty",
            1,
            "the commit message is empty; write a subject line".to_owned(),
        )];
    };
    let mut findings = Vec::new();
    let description = if conventional {
        conventional_rules(subject, &mut findings)
    } else {
        Some(subject)
    };
    let length = subject.chars().count();
    if length > subject_max {
        findings.push(MessageFinding::warning(
            "subject_length",
            1,
            format!(
                "the subject line has {length} characters, more than {subject_max}; shorten it"
            ),
        ));
    }
    if subject.ends_with('.') {
        findings.push(MessageFinding::warning(
            "subject_period",
            1,
            "the subject line ends with a full stop; remove it".to_owned(),
        ));
    }
    let first_word = description
        .unwrap_or(subject)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    if NON_IMPERATIVE.contains(&first_word.as_str()) {
        findings.push(MessageFinding::warning(
            "subject_mood",
            1,
            format!("the subject starts with `{first_word}`; write it in the imperative mood"),
        ));
    }
    if lines.get(1).is_some_and(|line| !line.trim().is_empty()) {
        findings.push(MessageFinding::warning(
            "blank_line",
            2,
            "the second line of the message is not blank; separate the subject from the body with a blank line"
                .to_owned(),
        ));
    }
    findings
}

/// Applies the header rules and returns the description of a valid header.
fn conventional_rules<'a>(subject: &'a str, findings: &mut Vec<MessageFinding>) -> Option<&'a str> {
    let Some(captures) = HEADER.captures(subject) else {
        findings.push(MessageFinding::error(
            "conventional_header",
            1,
            "the subject line is not a Conventional Commits header; write it as `type(scope): description`"
                .to_owned(),
        ));
        return None;
    };
    if captures.name("scope").is_none() {
        findings.push(MessageFinding::warning(
            "missing_scope",
            1,
            "the header has no scope; name the changed area as `type(scope): description`"
                .to_owned(),
        ));
    }
    let description = captures.name("desc").map(|found| found.as_str())?;
    if description.chars().next().is_some_and(char::is_uppercase) {
        findings.push(MessageFinding::warning(
            "subject_case",
            1,
            "the description starts with an upper case letter; start it in lower case".to_owned(),
        ));
    }
    Some(description)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(message: &str, conventional: bool) -> Vec<&'static str> {
        check_message(message, SUBJECT_MAX, conventional)
            .into_iter()
            .map(|finding| finding.rule)
            .collect()
    }

    #[test]
    fn accepts_a_conventional_header_with_scope_and_body() {
        assert!(rules("feat(parser): add numbers\n\nBody text.\n", true).is_empty());
        assert!(rules("fix(io)!: reject empty paths", true).is_empty());
    }

    #[test]
    fn rejects_a_subject_that_is_not_a_conventional_header() {
        let findings = check_message("add numbers", SUBJECT_MAX, true);
        assert_eq!(findings[0].rule, "conventional_header");
        assert_eq!(findings[0].severity, FindingSeverity::Error);
    }

    #[test]
    fn warns_about_a_header_without_scope_and_an_upper_case_description() {
        assert_eq!(
            rules("feat: Add numbers", true),
            ["missing_scope", "subject_case"]
        );
    }

    #[test]
    fn plain_subjects_may_start_with_an_upper_case_letter() {
        assert!(rules("Add numbers to the parser", false).is_empty());
    }

    #[test]
    fn measures_the_subject_in_characters_against_the_limit() {
        let subject = format!("fix(core): {}", "\u{17c}".repeat(30));
        assert!(check_message(&subject, 41, true).is_empty());
        let findings = check_message(&subject, 40, true);
        assert_eq!(findings[0].rule, "subject_length");
        assert!(findings[0].message.contains("41 characters"));
    }

    #[test]
    fn reports_a_final_full_stop_and_a_missing_blank_line() {
        let findings = check_message("fix(core): handle input.\nmore text", SUBJECT_MAX, true);
        let found: Vec<(&str, u64)> = findings
            .iter()
            .map(|finding| (finding.rule, finding.line))
            .collect();
        assert_eq!(found, [("subject_period", 1), ("blank_line", 2)]);
    }

    #[test]
    fn reports_a_first_word_in_the_past_tense() {
        assert_eq!(rules("fix(core): fixed the parser", true), ["subject_mood"]);
        assert_eq!(rules("Updates the docs", false), ["subject_mood"]);
    }

    #[test]
    fn skips_leading_blank_lines_and_reports_an_empty_message() {
        assert!(rules("\n\nfix(core): handle input", true).is_empty());
        let findings = check_message(" \n\n", SUBJECT_MAX, true);
        assert_eq!(findings[0].rule, "subject_empty");
        assert_eq!(findings[0].severity, FindingSeverity::Error);
    }

    #[test]
    fn message_findings_point_at_the_repository_root() {
        let finding = check_message("Added.", SUBJECT_MAX, false)
            .remove(0)
            .into_finding("/work/repo");
        assert_eq!(finding.path, RecordPath::new("/work/repo"));
        assert_eq!(finding.target, Some(FindingTarget::Message));
        assert_eq!(finding.kind, FindingKind::Subject);
        assert_eq!(finding.line, Some(1));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
