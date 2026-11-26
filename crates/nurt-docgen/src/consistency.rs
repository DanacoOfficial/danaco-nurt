// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Consistency checks over the Markdown documentation tree.
//!
//! The checks verify that the numbers stated in the documentation agree with
//! the catalog and the token measurement, that no sentence narrows the scope
//! of the product, and that every example passes the program checker.

use std::path::{Path, PathBuf};

use crate::catalog::{CORE_GROUP, Catalog};
use crate::check::ProgramChecker;
use crate::error::{DocgenError, Result, read_text};
use crate::examples::{check_snapshot, extract};
use crate::markdown::parse;

/// Phrases that narrow the scope of the product; none may occur in the documentation.
pub const NARROWING_PHRASES: &[&str] = &[
    "1.0 with",
    "1.0 only",
    "only in 1.0",
    "from 1.0 on",
    "depends on the maturity",
    "depends on maturity",
    "limited scope",
    "reduced scope",
    "narrowed scope",
    "scope is limited",
    "scope is narrowed",
    "subset of the specification",
];

/// Numbers that the documentation may state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// Number of core commands.
    pub core_commands: usize,
    /// Number of all commands.
    pub total_commands: usize,
    /// Number of optional groups.
    pub optional_groups: usize,
    /// Sizes of the groups, for statements about one group.
    pub group_sizes: Vec<usize>,
    /// Measured token count of the core description, when measured.
    pub core_tokens: Option<u64>,
    /// Measured token count of the whole description, when measured.
    pub total_tokens: Option<u64>,
}

impl Facts {
    /// Derives the counts from the catalog; token counts are given separately.
    #[must_use]
    pub fn from_catalog(
        catalog: &Catalog,
        core_tokens: Option<u64>,
        total_tokens: Option<u64>,
    ) -> Self {
        Self {
            core_commands: catalog.group_size(CORE_GROUP),
            total_commands: catalog.total(),
            optional_groups: catalog.optional_groups(),
            group_sizes: catalog.groups.iter().map(|g| g.commands.len()).collect(),
            core_tokens,
            total_tokens,
        }
    }
}

fn parse_number(word: &str) -> Option<u64> {
    let digits: String = word
        .trim_matches(|c: char| !c.is_ascii_digit())
        .chars()
        .filter(|c| *c != ',')
        .collect();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn clean(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

fn prose_lines(text: &str) -> Vec<(usize, String)> {
    let mut lines = Vec::new();
    let mut fenced = false;
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            let without_code: String = line.split('`').step_by(2).collect::<Vec<_>>().join(" ");
            lines.push((index + 1, without_code));
        }
    }
    lines
}

/// Checks the numbers and the narrowing phrases of one document.
///
/// `name` identifies the document in the messages.
#[must_use]
pub fn check_text(name: &str, text: &str, facts: &Facts) -> Vec<String> {
    let mut problems = Vec::new();
    for (line_number, line) in prose_lines(text) {
        let lower = line.to_lowercase();
        for phrase in NARROWING_PHRASES {
            if lower.contains(phrase) {
                problems.push(format!(
                    "{name}:{line_number}: narrows the scope: \"{phrase}\""
                ));
            }
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            let Some(number) = parse_number(word) else {
                continue;
            };
            let next = words.get(i + 1).map(|w| clean(w)).unwrap_or_default();
            let after = words.get(i + 2).map(|w| clean(w)).unwrap_or_default();
            if let Some(problem) = check_claim(number, &next, &after, facts) {
                problems.push(format!("{name}:{line_number}: {problem}"));
            }
        }
    }
    problems
}

fn check_claim(number: u64, next: &str, after: &str, facts: &Facts) -> Option<String> {
    let is = |value: usize| u64::try_from(value).is_ok_and(|v| v == number);
    match (next, after) {
        ("core", "commands") if !is(facts.core_commands) => Some(format!(
            "{number} core commands, the catalog has {}",
            facts.core_commands
        )),
        ("commands", _)
            if !is(facts.total_commands)
                && !is(facts.core_commands)
                && !facts.group_sizes.iter().any(|s| is(*s)) =>
        {
            Some(format!("{number} commands matches no count of the catalog"))
        }
        ("optional" | "command", "groups") | ("groups", _) if !is(facts.optional_groups) => Some(
            format!("{number} groups, the catalog has {}", facts.optional_groups),
        ),
        ("tokens", _) => check_tokens(number, facts),
        _ => None,
    }
}

fn check_tokens(number: u64, facts: &Facts) -> Option<String> {
    let measured: Vec<u64> = facts
        .core_tokens
        .iter()
        .chain(facts.total_tokens.iter())
        .copied()
        .collect();
    if measured.is_empty() {
        return Some(format!(
            "{number} tokens stated, no measurement is available"
        ));
    }
    if measured.contains(&number) {
        None
    } else {
        Some(format!("{number} tokens differ from the measured counts"))
    }
}

/// Lists the Markdown files under a directory recursively, sorted.
///
/// # Errors
///
/// Returns an error when a directory cannot be read.
pub fn markdown_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| DocgenError::io(&dir, e))?;
        for entry in entries {
            let path = entry.map_err(|e| DocgenError::io(&dir, e))?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "md") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Checks the numbers and narrowing phrases in all documents under a directory.
///
/// # Errors
///
/// Returns an error when a file cannot be read.
pub fn check_tree(root: &Path, facts: &Facts) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    for path in markdown_files(root)? {
        let name = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        problems.extend(check_text(&name, &read_text(&path)?, facts));
    }
    Ok(problems)
}

/// Checks every example in all documents under a directory.
///
/// Files whose name starts with an underscore (templates) are skipped.
///
/// # Errors
///
/// Returns an error when a file cannot be read.
pub fn check_examples_in_tree(root: &Path, checker: &dyn ProgramChecker) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    for path in markdown_files(root)? {
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if file_name.starts_with('_') {
            continue;
        }
        let name = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let source = read_text(&path)?;
        let (_, body) = crate::markdown::split_front_matter(&source);
        for (index, example) in extract(&parse(body)).iter().enumerate() {
            for issue in checker.check(&example.program) {
                problems.push(format!("{name}: example {}: {issue}", index + 1));
            }
            if let Some(snapshot) = &example.fs_before {
                for problem in check_snapshot(snapshot) {
                    problems.push(format!("{name}: example {}: {problem}", index + 1));
                }
            }
        }
    }
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::fixtures::catalog;
    use crate::check::CatalogChecker;

    fn facts() -> Facts {
        Facts::from_catalog(&catalog(), Some(2230), Some(4900))
    }

    #[test]
    fn accepts_counts_that_match_the_catalog() {
        let facts = facts();
        assert!(
            check_text(
                "a.md",
                "The 4 core commands and 5 commands in total.",
                &facts
            )
            .is_empty()
        );
        assert!(check_text("a.md", "There is 1 optional groups entry.", &facts).is_empty());
    }

    #[test]
    fn reports_wrong_core_and_total_counts() {
        let facts = facts();
        let problems = check_text("a.md", "The 33 core commands.\nAll 148 commands.", &facts);
        assert_eq!(problems.len(), 2);
        assert!(problems[0].starts_with("a.md:1:"));
        assert!(problems[1].contains("148 commands"));
    }

    #[test]
    fn reports_wrong_group_count() {
        assert_eq!(check_text("a.md", "14 optional groups", &facts()).len(), 1);
    }

    #[test]
    fn checks_token_counts_against_measurement() {
        let facts = facts();
        assert!(check_text("a.md", "About 2,230 tokens.", &facts).is_empty());
        assert_eq!(check_text("a.md", "About 2,000 tokens.", &facts).len(), 1);
        let unmeasured = Facts {
            core_tokens: None,
            total_tokens: None,
            ..facts
        };
        assert_eq!(
            check_text("a.md", "About 2,230 tokens.", &unmeasured).len(),
            1
        );
    }

    #[test]
    fn ignores_code_blocks_and_code_spans() {
        let text = "```text\n999 commands\n```\nUse `148 commands` here.\n";
        assert!(check_text("a.md", text, &facts()).is_empty());
    }

    #[test]
    fn reports_narrowing_phrases() {
        let problems = check_text(
            "a.md",
            "The scope depends on the maturity of the parts.",
            &facts(),
        );
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("narrows the scope"));
    }

    #[test]
    fn checks_examples_of_a_tree() {
        let dir = tempfile::tempdir().expect("temporary directory");
        std::fs::write(
            dir.path().join("a.md"),
            "```nurt example\nfnid \"/a\"\n```\n",
        )
        .expect("write");
        std::fs::write(
            dir.path().join("_t.md"),
            "```nurt example\n<program>\n```\n",
        )
        .expect("write");
        let catalog = catalog();
        let problems = check_examples_in_tree(dir.path(), &CatalogChecker::new(&catalog))
            .expect("tree readable");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].starts_with("a.md: example 1: E_UNKNOWN_COMMAND"));
    }

    #[test]
    fn numbers_with_separators_and_punctuation_are_parsed() {
        assert_eq!(parse_number("2,230"), Some(2230));
        assert_eq!(parse_number("(148)"), Some(148));
        assert_eq!(parse_number("1.0"), None);
        assert_eq!(parse_number("word"), None);
        assert_eq!(parse_number(""), None);
    }

    #[test]
    fn prose_lines_skip_fenced_blocks_and_code_spans_and_keep_line_numbers() {
        let lines = prose_lines("a `x` b\n```\nhidden\n```\nlast\n");
        assert_eq!(
            lines,
            vec![(1, "a   b".to_owned()), (5, "last".to_owned())]
        );
    }

    #[test]
    fn group_counts_are_checked_in_both_phrasings() {
        let facts = facts();
        assert!(check_text("a.md", "There are 1 optional groups.", &facts).is_empty());
        assert_eq!(
            check_text("a.md", "There are 3 command groups.", &facts).len(),
            1
        );
    }

    #[test]
    fn markdown_files_are_listed_in_sorted_order() {
        let dir = tempfile::tempdir().expect("temporary directory");
        std::fs::create_dir(dir.path().join("sub")).expect("create");
        for name in ["b.md", "a.md", "sub/c.md"] {
            std::fs::write(dir.path().join(name), "x").expect("write");
        }
        let files = markdown_files(dir.path()).expect("tree readable");
        let names: Vec<String> = files
            .iter()
            .map(|path| {
                path.strip_prefix(dir.path())
                    .expect("below the root")
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect();
        assert_eq!(names, ["a.md", "b.md", "sub/c.md"]);
    }

    #[test]
    fn a_missing_directory_is_an_error() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert!(markdown_files(&dir.path().join("absent")).is_err());
    }

    #[test]
    #[test]
    fn lists_markdown_files_recursively() {
        let dir = tempfile::tempdir().expect("temporary directory");
        std::fs::create_dir(dir.path().join("sub")).expect("create");
        std::fs::write(dir.path().join("sub").join("b.md"), "x").expect("write");
        std::fs::write(dir.path().join("c.txt"), "x").expect("write");
        let files = markdown_files(dir.path()).expect("tree readable");
        assert_eq!(files.len(), 1);
    }
}
