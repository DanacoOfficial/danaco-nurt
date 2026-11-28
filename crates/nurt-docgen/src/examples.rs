// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-docgen::examples
// Responsibility:   Executable example blocks of command pages and cookbook recipes.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Executable example blocks of command pages and cookbook recipes.
//!
//! An example is a fenced block with the info string `nurt example`, written
//! in variant A of the syntax. It may be followed by a block `json fs-before`
//! (a snapshot in the format `nurt-fs-snapshot/1`) and a block `text expected`
//! (a fragment that must appear in the text result).

use crate::markdown::{Block, Document};

/// Info string of a program block.
pub const EXAMPLE_INFO: &str = "nurt example";
/// Info string of a file system snapshot block.
pub const FS_BEFORE_INFO: &str = "json fs-before";
/// Info string of an expected result block.
pub const EXPECTED_INFO: &str = "text expected";
/// Format identifier of a file system snapshot.
pub const SNAPSHOT_FORMAT: &str = "nurt-fs-snapshot/1";

/// Example program with its optional snapshot and expected result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    /// Program text in variant A.
    pub program: String,
    /// Snapshot of the virtual file system before the run, as JSON text.
    pub fs_before: Option<String>,
    /// Fragment that must appear in the text result.
    pub expected: Option<String>,
}

/// Extracts the examples of a document in source order.
///
/// A snapshot or expected block belongs to the nearest preceding program
/// block; a heading ends the association.
#[must_use]
pub fn extract(document: &Document) -> Vec<Example> {
    let mut examples: Vec<Example> = Vec::new();
    let mut open = false;
    for block in &document.blocks {
        match block {
            Block::Heading { .. } => open = false,
            Block::Code { info, text } if info == EXAMPLE_INFO => {
                examples.push(Example {
                    program: text.clone(),
                    fs_before: None,
                    expected: None,
                });
                open = true;
            }
            Block::Code { info, text } if open && info == FS_BEFORE_INFO => {
                if let Some(last) = examples.last_mut() {
                    last.fs_before = Some(text.clone());
                }
            }
            Block::Code { info, text } if open && info == EXPECTED_INFO => {
                if let Some(last) = examples.last_mut() {
                    last.expected = Some(text.clone());
                }
            }
            _ => {}
        }
    }
    examples
}

/// Whether the path starts with a drive letter and `:/`.
fn is_drive_path(path: &str) -> bool {
    path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic) && path.get(1..3) == Some(":/")
}

/// Checks that a snapshot is a JSON object of the format `nurt-fs-snapshot/1`
/// with a list of entries that have a path and a type.
///
/// Returns the list of problems; an empty list means the snapshot is valid.
#[must_use]
pub fn check_snapshot(text: &str) -> Vec<String> {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => return vec![format!("snapshot is not valid JSON: {error}")],
    };
    let mut problems = Vec::new();
    if value.get("format").and_then(|f| f.as_str()) != Some(SNAPSHOT_FORMAT) {
        problems.push(format!("snapshot format must be {SNAPSHOT_FORMAT}"));
    }
    let Some(entries) = value.get("entries").and_then(|e| e.as_array()) else {
        problems.push("snapshot has no entries list".to_string());
        return problems;
    };
    for (index, entry) in entries.iter().enumerate() {
        let path = entry.get("path").and_then(|p| p.as_str());
        let kind = entry.get("type").and_then(|t| t.as_str());
        match path {
            Some(path) if path.starts_with('/') || is_drive_path(path) => {}
            _ => problems.push(format!("entry {index} has no absolute path")),
        }
        if !matches!(kind, Some("file" | "dir" | "symlink")) {
            problems.push(format!("entry {index} has an unknown type"));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::parse;

    const PAGE: &str = "## Examples\n\nFirst.\n\n```nurt example\nfind \"/home/user\"\n```\n\n```json fs-before\n{}\n```\n\n```text expected\n/home/user/a\n```\n\nSecond.\n\n```nurt example\ncount\n```\n";

    #[test]
    fn extracts_programs_with_attached_blocks() {
        let examples = extract(&parse(PAGE));
        assert_eq!(examples.len(), 2);
        assert_eq!(examples[0].program, "find \"/home/user\"");
        assert_eq!(examples[0].fs_before.as_deref(), Some("{}"));
        assert_eq!(examples[0].expected.as_deref(), Some("/home/user/a"));
        assert_eq!(examples[1].fs_before, None);
    }

    #[test]
    fn heading_ends_association_of_blocks() {
        let examples = extract(&parse(
            "```nurt example\ncount\n```\n\n## Next\n\n```text expected\nx\n```\n",
        ));
        assert_eq!(examples[0].expected, None);
    }

    #[test]
    fn accepts_valid_snapshot() {
        let text = r#"{"format":"nurt-fs-snapshot/1","entries":[{"path":"/home/user/a","type":"file","content_utf8":"x"}]}"#;
        assert!(check_snapshot(text).is_empty());
    }

    #[test]
    fn rejects_snapshot_with_wrong_format_and_relative_path() {
        let text = r#"{"format":"x","entries":[{"path":"a","type":"file"}]}"#;
        assert_eq!(check_snapshot(text).len(), 2);
    }

    #[test]
    fn accepts_drive_paths_only_with_a_letter_and_a_slash() {
        assert!(is_drive_path("C:/Users/user"));
        assert!(is_drive_path("d:/x"));
        for path in ["1:/x", "C:x", "C:\\x", ":/x", "\u{17c}:/x", "", "C"] {
            assert!(!is_drive_path(path), "{path:?}");
        }
    }

    #[test]
    fn reports_unknown_entry_types_and_missing_lists() {
        let text = r#"{"format":"nurt-fs-snapshot/1","entries":[{"path":"/a","type":"socket"},{"type":"file"}]}"#;
        let problems = check_snapshot(text);
        assert_eq!(problems.len(), 2);
        assert!(problems[0].contains("unknown type"));
        assert!(problems[1].contains("no absolute path"));
        let none = check_snapshot(r#"{"format":"nurt-fs-snapshot/1"}"#);
        assert_eq!(none, vec!["snapshot has no entries list".to_string()]);
    }

    #[test]
    fn rejects_invalid_json_snapshot() {
        assert_eq!(check_snapshot("{").len(), 1);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
