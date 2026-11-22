// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Sparse checkout: which paths of the index have a working tree file.
//!
//! With `core.sparseCheckout` set, the patterns of `info/sparse-checkout`
//! select the paths that are written to the working tree; other paths get
//! the `skip-worktree` flag in the index. Cone mode
//! (`core.sparseCheckoutCone`) selects whole directories: files at the root,
//! files directly inside the parent directories of a selected directory, and
//! everything below a selected directory. Without cone mode the patterns
//! have the syntax of `.gitignore`, the last matching pattern decides, and a
//! path that no pattern matches inherits the decision of its directory.

use std::collections::BTreeSet;

use bstr::{BStr, BString, ByteSlice};
use gix::glob::pattern::Case;
use gix::glob::wildmatch::Mode as WildMode;
use nurt_contracts::error::Result;

use super::error::io_error;
use super::layout::ancestors;

/// Pattern file below the repository directory.
const PATTERN_FILE: [&str; 2] = ["info", "sparse-checkout"];

/// Patterns written by `git sparse-checkout` for the root directory in cone
/// mode.
const CONE_ROOT: [&str; 2] = ["/*", "!/*/"];

/// The sparse checkout selection of a working tree.
#[derive(Debug, Clone, Default)]
pub(crate) enum Sparse {
    /// Sparse checkout is off: every path has a working tree file.
    #[default]
    Off,
    /// Cone mode with its recursive and parent directories.
    Cone {
        recursive: BTreeSet<BString>,
        parents: BTreeSet<BString>,
    },
    /// Patterns with the syntax of `.gitignore`.
    Patterns {
        patterns: Vec<gix::glob::Pattern>,
        case: Case,
    },
}

impl Sparse {
    /// Reads the selection of the working tree of `repo`.
    pub(crate) fn load(repo: &gix::Repository, ignore_case: bool) -> Result<Self> {
        let config = repo.config_snapshot();
        if !config.boolean("core.sparseCheckout").unwrap_or(false) {
            return Ok(Self::Off);
        }
        let cone = config.boolean("core.sparseCheckoutCone").unwrap_or(false);
        let path = PATTERN_FILE
            .iter()
            .fold(repo.git_dir().to_path_buf(), |path, part| path.join(part));
        let text = match std::fs::read(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::Off),
            Err(err) => return Err(io_error(&err, &path)),
        };
        Ok(Self::from_text(&text, cone, ignore_case))
    }

    /// Selection described by the content of a pattern file.
    pub(crate) fn from_text(text: &[u8], cone: bool, ignore_case: bool) -> Self {
        if cone && let Some(selection) = Self::cone(text) {
            return selection;
        }
        let patterns = text
            .lines()
            .map(|line| line.trim_end())
            .filter(|line| !line.is_empty() && !line.starts_with(b"#"))
            .filter_map(gix::glob::parse)
            .collect();
        Self::Patterns {
            patterns,
            case: if ignore_case {
                Case::Fold
            } else {
                Case::Sensitive
            },
        }
    }

    /// Cone selection, or `None` when a line is not a cone pattern; `git`
    /// then falls back to the full pattern syntax.
    fn cone(text: &[u8]) -> Option<Self> {
        let mut positive = BTreeSet::new();
        let mut parents = BTreeSet::new();
        for line in text.lines().map(|line| line.trim_end()) {
            if line.is_empty()
                || line.starts_with(b"#")
                || CONE_ROOT.iter().any(|root| line == root.as_bytes())
            {
                continue;
            }
            if let Some(dir) = line
                .strip_prefix(b"!/")
                .and_then(|rest| rest.strip_suffix(b"/*/"))
            {
                parents.insert(unescape(dir));
            } else {
                let dir = line
                    .strip_prefix(b"/")
                    .and_then(|rest| rest.strip_suffix(b"/"))?;
                positive.insert(unescape(dir));
            }
        }
        let recursive = positive.difference(&parents).cloned().collect();
        Some(Self::Cone { recursive, parents })
    }

    /// Whether the path `path` of the index has a working tree file.
    pub(crate) fn includes(&self, path: &BStr) -> bool {
        match self {
            Self::Off => true,
            Self::Cone { recursive, parents } => {
                let Some(slash) = path.rfind_byte(b'/') else {
                    return true;
                };
                parents.contains(path[..slash].as_bstr())
                    || ancestors(path).any(|dir| recursive.contains(dir))
            }
            Self::Patterns { patterns, case } => {
                let mut current = path;
                let mut is_dir = false;
                loop {
                    if let Some(included) = last_match(patterns, current, is_dir, *case) {
                        return included;
                    }
                    let Some(slash) = current.rfind_byte(b'/') else {
                        return false;
                    };
                    current = current[..slash].as_bstr();
                    is_dir = true;
                }
            }
        }
    }
}

fn last_match(
    patterns: &[gix::glob::Pattern],
    path: &BStr,
    is_dir: bool,
    case: Case,
) -> Option<bool> {
    let basename = path.rfind_byte(b'/').map(|slash| slash + 1);
    patterns
        .iter()
        .rev()
        .find(|pattern| {
            pattern.matches_repo_relative_path(
                path,
                basename,
                Some(is_dir),
                case,
                WildMode::NO_MATCH_SLASH_LITERAL,
            )
        })
        .map(|pattern| !pattern.is_negative())
}

/// Removes the backslash escapes that `git` writes before special
/// characters of directory names in cone patterns.
fn unescape(escaped: &[u8]) -> BString {
    let mut out = BString::default();
    let mut bytes = escaped.iter();
    while let Some(byte) = bytes.next() {
        match (byte, bytes.as_slice().first()) {
            (b'\\', Some(next)) => {
                out.push(*next);
                bytes.next();
            }
            _ => out.push(*byte),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn includes(selection: &Sparse, path: &str) -> bool {
        selection.includes(path.as_bytes().as_bstr())
    }

    #[test]
    fn cone_selects_root_files_parent_files_and_recursive_directories() {
        let text = b"/*\n!/*/\n/a/\n!/a/*/\n/a/b/\n/c\\*d/\n";
        let selection = Sparse::from_text(text, true, false);
        assert!(includes(&selection, "root.txt"));
        assert!(includes(&selection, "a/file.txt"));
        assert!(!includes(&selection, "a/other/x.txt"));
        assert!(includes(&selection, "a/b/deep/x.txt"));
        assert!(includes(&selection, "c*d/x.txt"));
        assert!(!includes(&selection, "z/x.txt"));
    }

    #[test]
    fn cone_with_foreign_line_falls_back_to_patterns() {
        let selection = Sparse::from_text(b"/*\n*.md\n", true, false);
        assert!(matches!(selection, Sparse::Patterns { .. }));
        assert!(includes(&selection, "docs/readme.md"));
    }

    #[test]
    fn patterns_use_last_match_and_directory_inheritance() {
        let selection = Sparse::from_text(b"# comment\n*.txt\n!secret.txt\nsrc/\n", false, false);
        assert!(includes(&selection, "a.txt"));
        assert!(includes(&selection, "deep/b.txt"));
        assert!(!includes(&selection, "secret.txt"));
        assert!(includes(&selection, "src/main.rs"));
        assert!(!includes(&selection, "docs/x.md"));
    }

    #[test]
    fn patterns_fold_case_when_the_file_system_ignores_it() {
        let selection = Sparse::from_text(b"*.TXT\n", false, true);
        assert!(includes(&selection, "a.txt"));
        let selection = Sparse::from_text(b"*.TXT\n", false, false);
        assert!(!includes(&selection, "a.txt"));
    }

    #[test]
    fn off_includes_everything() {
        assert!(includes(&Sparse::Off, "any/path"));
    }
}
