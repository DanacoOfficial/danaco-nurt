// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Cheat sheets "Nurt for bash users" and "Nurt for PowerShell users".
//!
//! A sheet is generated from a correction table: a TOML file in the format
//! `nurt-correction-table/1` that pairs a habit of another shell with the
//! canonical form in Nurt and states whether the correction is applied
//! automatically or the program is rejected with a fix.

use std::path::Path;

use serde::Deserialize;

use crate::check::ProgramChecker;
use crate::error::{DocgenError, Result, read_text};

/// Format identifier of a correction table.
pub const TABLE_FORMAT: &str = "nurt-correction-table/1";

/// Result of the correction of a habit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The program is corrected and runs.
    Corrected,
    /// The program is rejected with an error that carries the fix.
    Error,
    /// The program is rejected with an error that carries no ready program;
    /// the canonical form is what to write instead.
    Rejected,
}

impl Outcome {
    /// Returns the label used in the sheets.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Corrected => "corrected automatically",
            Self::Error => "error with fix",
            Self::Rejected => "error without fix",
        }
    }
}

/// Pair of a habit and its canonical form.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Identifier of the rule in the correction table.
    pub id: String,
    /// Topic the entry is listed under.
    pub topic: String,
    /// Habit as written in the other shell.
    pub habit: String,
    /// Canonical form in Nurt; the fix when the outcome is an error, the
    /// program to write instead when the outcome is a rejection.
    pub canonical: String,
    /// What Nurt does with the habit.
    pub outcome: Outcome,
    /// Code of the error of a rejected habit, such as `E_UNCORRECTABLE`;
    /// empty for the other outcomes.
    #[serde(default)]
    pub code: String,
    /// Remark, empty when there is none.
    #[serde(default)]
    pub note: String,
}

/// Correction table of one shell.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    /// Format identifier, `nurt-correction-table/1`.
    pub format: String,
    /// Name of the shell, such as `bash`.
    pub shell: String,
    /// Title of the sheet.
    pub title: String,
    /// Operating system of the paths of the sheet, `linux` or `windows`;
    /// empty when the sheet states none.
    #[serde(default)]
    pub host: String,
    /// Call parameter `base` of the examples, an absolute directory; empty
    /// when the examples need none.
    #[serde(default)]
    pub base: String,
    /// Entries in the order of the table.
    #[serde(rename = "entry")]
    pub entries: Vec<Entry>,
}

/// Loads a correction table.
///
/// # Errors
///
/// Returns an error when the file cannot be read, is not a table of the
/// supported format, or has duplicate entry identifiers.
pub fn load_table(path: &Path) -> Result<Table> {
    let table: Table = toml::from_str(&read_text(path)?)
        .map_err(|e| DocgenError::invalid(path.display().to_string(), e.to_string()))?;
    if table.format != TABLE_FORMAT {
        return Err(DocgenError::invalid(
            path.display().to_string(),
            format!("format must be {TABLE_FORMAT}"),
        ));
    }
    Ok(table)
}

/// Checks a table: unique identifiers, non-empty fields and canonical forms
/// that pass the checker.
#[must_use]
pub fn check_table(table: &Table, checker: &dyn ProgramChecker) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for entry in &table.entries {
        if seen.contains(&entry.id.as_str()) {
            problems.push(format!("{}: duplicate identifier", entry.id));
        }
        seen.push(&entry.id);
        if entry.habit.trim().is_empty() || entry.canonical.trim().is_empty() {
            problems.push(format!(
                "{}: habit and canonical form are required",
                entry.id
            ));
        }
        if (entry.outcome == Outcome::Rejected) == entry.code.is_empty() {
            problems.push(format!(
                "{}: the error code is required for a rejected habit and for it only",
                entry.id
            ));
        }
        for issue in checker.check(&entry.canonical) {
            problems.push(format!("{}: {issue}", entry.id));
        }
    }
    problems
}

/// Text on one table row with the pipe escaped.
fn flatten(text: &str) -> String {
    text.replace(['\r', '\n'], " ").replace('|', "\\|")
}

/// Code span of a table cell; the fence is longer than every run of
/// backticks in the text.
fn cell(text: &str) -> String {
    let flat = flatten(text);
    let longest = flat.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    if longest == 0 {
        format!("`{flat}`")
    } else {
        let fence = "`".repeat(longest + 1);
        format!("{fence} {flat} {fence}")
    }
}

/// Returns the display name of a shell: `PowerShell` for `powershell`, the
/// capitalized name otherwise.
#[must_use]
pub fn shell_label(shell: &str) -> String {
    if shell == "powershell" {
        return "PowerShell".to_string();
    }
    let mut chars = shell.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Returns the file name of the generated sheet of a table.
#[must_use]
pub fn sheet_file_name(table: &Table) -> String {
    format!("nurt-for-{}-users.md", table.shell)
}

/// Renders a sheet in Markdown: one table per topic, in order of first appearance.
#[must_use]
pub fn render(table: &Table) -> String {
    let shell_title = shell_label(&table.shell);
    let shell_title = shell_title.as_str();
    let mut topics: Vec<&str> = Vec::new();
    for entry in &table.entries {
        if !topics.contains(&entry.topic.as_str()) {
            topics.push(&entry.topic);
        }
    }
    let mut out = format!(
        "---\ntitle: {}\n---\n# {}\n\nEvery row pairs a {shell_title} command with its form in Nurt. The last column states whether Nurt corrects the habit automatically, rejects it with an error that carries the fix, or rejects it with an error that carries no program and asks for the program of the row.\n",
        table.title, table.title
    );
    if !table.host.is_empty() {
        out.push_str(&format!(
            "\nThe paths are those of a {} host{}.\n",
            shell_label(&table.host),
            if table.base.is_empty() {
                String::new()
            } else {
                format!(", and the call parameter `base` is `{}`", table.base)
            }
        ));
    }
    for topic in topics {
        out.push_str(&format!(
            "\n## {topic}\n\n| Rule | {shell_title} | Nurt | Result |\n|---|---|---|---|\n"
        ));
        for entry in table.entries.iter().filter(|e| e.topic == topic) {
            let mut result = entry.outcome.label().to_string();
            if !entry.note.is_empty() {
                result.push_str(&format!(": {}", flatten(&entry.note)));
            }
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                entry.id,
                cell(&entry.habit),
                cell(&entry.canonical),
                result
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::fixtures::catalog;
    use crate::check::CatalogChecker;

    const TABLE: &str = r#"
format = "nurt-correction-table/1"
shell = "bash"
title = "Nurt for bash users"

[[entry]]
id = "B01"
topic = "Files"
habit = "rm -rf /home/user/build"
canonical = 'remove "/home/user/build"'
outcome = "corrected"

[[entry]]
id = "B22"
topic = "Control flow"
habit = "a && b"
canonical = 'count'
outcome = "error"
note = "two effects in one program"
"#;

    fn table() -> Table {
        toml::from_str(TABLE).expect("table fixture parses")
    }

    #[test]
    fn renders_one_table_per_topic_with_result_labels() {
        let text = render(&table());
        assert!(text.contains("## Files\n"));
        assert!(text.contains("## Control flow\n"));
        assert!(text.contains("| B01 | `rm -rf /home/user/build` | `remove \"/home/user/build\"` | corrected automatically |"));
        assert!(text.contains("error with fix: two effects in one program"));
    }

    #[test]
    fn code_span_fence_is_longer_than_any_backtick_run_in_the_text() {
        assert_eq!(cell("a|b"), "`a\\|b`");
        assert_eq!(cell("echo `date`"), "`` echo `date` ``");
        assert_eq!(cell("a ``b`` c"), "``` a ``b`` c ```");
        assert_eq!(cell("l1\r\nl2"), "`l1  l2`");
        assert_eq!(
            cell("za\u{17c}\u{f3}\u{142}\u{107}"),
            "`za\u{17c}\u{f3}\u{142}\u{107}`"
        );
    }

    #[test]
    fn notes_with_pipes_and_line_breaks_stay_in_their_row() {
        let mut table = table();
        table.entries[1].note = "a | b\nc".to_owned();
        let text = render(&table);
        assert!(text.contains("error with fix: a \\| b c |\n"), "{text}");
    }

    #[test]
    fn labels_and_file_names_of_sheets() {
        assert_eq!(shell_label("powershell"), "PowerShell");
        assert_eq!(shell_label("bash"), "Bash");
        assert_eq!(shell_label(""), "");
        assert_eq!(sheet_file_name(&table()), "nurt-for-bash-users.md");
        assert_eq!(Outcome::Corrected.label(), "corrected automatically");
        assert_eq!(Outcome::Rejected.label(), "error without fix");
    }

    #[test]
    fn a_rejected_habit_has_an_error_code_and_no_other_habit_has() {
        let catalog = catalog();
        let mut table = table();
        table.entries[0].outcome = Outcome::Rejected;
        table.entries[1].code = "E_UNCORRECTABLE".to_owned();
        let problems = check_table(&table, &CatalogChecker::new(&catalog));
        let coded: Vec<&String> = problems
            .iter()
            .filter(|problem| problem.contains("error code is required"))
            .collect();
        assert_eq!(coded.len(), 2, "{problems:?}");
        table.entries[0].code = "E_UNCORRECTABLE".to_owned();
        table.entries[1].code.clear();
        let problems = check_table(&table, &CatalogChecker::new(&catalog));
        assert!(
            !problems
                .iter()
                .any(|problem| problem.contains("error code"))
        );
    }

    #[test]
    fn states_the_host_and_the_base_of_the_examples() {
        let mut table = table();
        assert!(!render(&table).contains("host"));
        table.host = "windows".to_owned();
        table.base = "C:/Users/user".to_owned();
        let text = render(&table);
        assert!(
            text.contains("The paths are those of a Windows host, and the call parameter `base` is `C:/Users/user`."),
            "{text}"
        );
        table.base.clear();
        assert!(render(&table).contains("The paths are those of a Windows host."));
    }

    #[test]
    fn check_reports_canonical_forms_that_do_not_bind() {
        let catalog = catalog();
        let problems = check_table(&table(), &CatalogChecker::new(&catalog));
        assert_eq!(problems.len(), 1);
        assert!(problems[0].starts_with("B22: E_STAGE_INPUT"));
    }

    #[test]
    fn check_reports_duplicate_identifiers() {
        let mut table = table();
        table.entries.push(table.entries[0].clone());
        let catalog = catalog();
        let problems = check_table(&table, &CatalogChecker::new(&catalog));
        assert!(problems.iter().any(|p| p.contains("duplicate identifier")));
    }

    #[test]
    fn rejects_unknown_format() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("t.toml");
        std::fs::write(
            &path,
            TABLE.replace("correction-table/1", "correction-table/9"),
        )
        .expect("write");
        assert!(load_table(&path).is_err());
    }
}
