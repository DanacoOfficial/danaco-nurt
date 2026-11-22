// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Difference of one file and the canonical hunks that identifiers number.
//!
//! The canonical difference compares the working tree with the index, or
//! the index with `HEAD`, with three context lines and without rename
//! detection, using the configured algorithm (`diff.algorithm`) and
//! indentation heuristic (`diff.indentHeuristic`).

use std::ops::Range;

use bstr::{BStr, BString, ByteSlice};
use nurt_contracts::error::Error;
use nurt_contracts::record::HunkStatus;

use super::funcname::{FuncMatcher, builtin_patterns};
use super::hunk::{Hunk, Layout, TextDiff};
use super::lines::{Algorithm, LineOptions};
use super::source::{DiffAttr, Entry, Reader, looks_binary, mode_text, read_error};

/// Side of the canonical difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// The working tree compared with the index (changes not staged).
    Worktree,
    /// The index compared with `HEAD` (staged changes).
    Staged,
}

/// A file compared between two sides; a missing side means the file was
/// added or deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePair {
    /// File after the change, or the deleted file.
    pub path: BString,
    /// File before a rename or copy.
    pub old_path: Option<BString>,
    /// Old side.
    pub old: Option<Entry>,
    /// New side.
    pub new: Option<Entry>,
    /// Similarity of a rename or copy in percent.
    pub similarity: Option<u32>,
    /// The pair is a copy, not a rename.
    pub copied: bool,
}

impl FilePair {
    /// Status of the file in a hunk record.
    #[must_use]
    pub fn status(&self) -> HunkStatus {
        match (&self.old, &self.new) {
            (None, _) => HunkStatus::Added,
            (_, None) => HunkStatus::Deleted,
            _ if self.old_path.is_some() && !self.copied => HunkStatus::Renamed,
            _ if self.old_path.is_some() => HunkStatus::Added,
            _ => HunkStatus::Modified,
        }
    }
}

/// How the difference of a file is computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileOptions {
    /// Line comparison.
    pub lines: LineOptions,
    /// Shape of the hunks.
    pub layout: Layout,
}

/// Settings of the `diff` driver of a path.
#[derive(Debug, Clone, Default)]
pub struct Driver {
    /// Function line recognizer.
    pub funcs: FuncMatcher,
    /// Binary decision: `Some` when set by attribute or configuration.
    pub binary: Option<bool>,
    /// Word pattern (`diff.<name>.wordRegex` or `diff.wordRegex`).
    pub word_regex: Option<String>,
    /// A text conversion program is configured and not run.
    pub textconv_skipped: bool,
    /// A configured function pattern failed to compile.
    pub invalid_pattern: Option<String>,
}

impl Driver {
    /// The driver of `path`.
    pub fn of(reader: &mut Reader<'_>, path: &BStr) -> Self {
        let attr = reader.diff_attr(path);
        let config = reader.repo().config_snapshot();
        let global_words = config
            .string("diff.wordRegex")
            .map(|value| value.to_str_lossy().into_owned());
        let mut driver = Self {
            word_regex: global_words,
            ..Self::default()
        };
        match attr {
            DiffAttr::Auto => {}
            DiffAttr::Binary => driver.binary = Some(true),
            DiffAttr::Text => driver.binary = Some(false),
            DiffAttr::Driver(name) => {
                let key = |field: &str| format!("diff.{name}.{field}");
                driver.binary = config.boolean(key("binary").as_str());
                if let Some(words) = config.string(key("wordRegex").as_str()) {
                    driver.word_regex = Some(words.to_str_lossy().into_owned());
                }
                driver.textconv_skipped = config.string(key("textconv").as_str()).is_some();
                let configured = config
                    .string(key("xfuncname").as_str())
                    .or_else(|| config.string(key("funcname").as_str()))
                    .map(|value| value.to_str_lossy().into_owned());
                let patterns = configured.as_deref().or_else(|| builtin_patterns(&name));
                if let Some(patterns) = patterns {
                    match FuncMatcher::from_patterns(patterns, false) {
                        Ok(funcs) => driver.funcs = funcs,
                        Err(error) => driver.invalid_pattern = Some(error.pattern),
                    }
                }
            }
        }
        driver
    }
}

/// Difference of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// The compared pair.
    pub pair: FilePair,
    /// Hunks; empty for binary files and for changes of the mode only.
    pub hunks: Vec<Hunk>,
    /// Added lines; `None` for binary files.
    pub added: Option<u64>,
    /// Removed lines; `None` for binary files.
    pub removed: Option<u64>,
    /// The content is binary.
    pub binary: bool,
}

impl FileDiff {
    /// The file has no change under the comparison options.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
            && !self.binary
            && self.pair.old_path.is_none()
            && self.pair.old.map(|entry| entry.mode) == self.pair.new.map(|entry| entry.mode)
            && self.pair.old.is_some()
            && self.pair.new.is_some()
    }

    /// Mode before, in octal.
    #[must_use]
    pub fn old_mode(&self) -> Option<String> {
        self.pair.old.map(|entry| mode_text(entry.mode))
    }

    /// Mode after, in octal.
    #[must_use]
    pub fn new_mode(&self) -> Option<String> {
        self.pair.new.map(|entry| mode_text(entry.mode))
    }
}

/// Computes the difference of a pair from given contents.
#[must_use]
pub fn diff_contents(
    pair: FilePair,
    old: &[u8],
    new: &[u8],
    options: &FileOptions,
    driver: &Driver,
) -> FileDiff {
    diff_texts(pair, old, new, options, driver, None)
}

/// Computes the difference of a pair from given contents; with `keep`, only
/// the changes that touch these lines of the new text (from 0, end
/// exclusive) are kept and counted.
#[must_use]
pub fn diff_texts(
    pair: FilePair,
    old: &[u8],
    new: &[u8],
    options: &FileOptions,
    driver: &Driver,
    keep: Option<&Range<usize>>,
) -> FileDiff {
    let binary = driver
        .binary
        .unwrap_or_else(|| looks_binary(old) || looks_binary(new));
    let status = pair.status();
    if binary {
        let changed = old != new || pair.old.is_none() || pair.new.is_none();
        let mut hunks = Vec::new();
        if changed {
            let mut hunk = Hunk::binary_placeholder(pair.path.clone());
            hunk.status = if status == HunkStatus::Modified {
                HunkStatus::Binary
            } else {
                status
            };
            decorate(&mut hunk, &pair);
            hunks.push(hunk);
        }
        return FileDiff {
            pair,
            hunks,
            added: None,
            removed: None,
            binary: true,
        };
    }
    let mut text = TextDiff::new(old, new, &options.lines);
    if let Some(lines) = keep {
        text.retain_new_lines(lines);
    }
    let (added, removed) = text.counts();
    let mut hunks = text.hunks(&pair.path, &options.layout, &driver.funcs);
    for hunk in &mut hunks {
        hunk.status = status;
        decorate(hunk, &pair);
    }
    FileDiff {
        pair,
        hunks,
        added: Some(added),
        removed: Some(removed),
        binary: false,
    }
}

fn decorate(hunk: &mut Hunk, pair: &FilePair) {
    hunk.old_path = pair.old_path.clone();
    hunk.old_mode = pair.old.map(|entry| mode_text(entry.mode));
    hunk.new_mode = pair.new.map(|entry| mode_text(entry.mode));
}

/// Reads both sides of a pair and computes its difference.
///
/// # Errors
///
/// Fails when an object or a worktree file cannot be read.
pub fn diff_pair(
    reader: &mut Reader<'_>,
    pair: FilePair,
    options: &FileOptions,
    driver: &Driver,
) -> Result<FileDiff, Error> {
    let old_path = pair.old_path.clone().unwrap_or_else(|| pair.path.clone());
    let old = match &pair.old {
        Some(entry) => reader.content(old_path.as_bstr(), entry)?,
        None => Vec::new(),
    };
    let new = match &pair.new {
        Some(entry) => reader.content(pair.path.as_bstr(), entry)?,
        None => Vec::new(),
    };
    Ok(diff_contents(pair, &old, &new, options, driver))
}

/// Line comparison options from the repository configuration
/// (`diff.algorithm`, `diff.indentHeuristic`) and whether the configured
/// algorithm was substituted.
#[must_use]
pub fn configured_lines(repo: &gix::Repository) -> (LineOptions, bool) {
    let config = repo.config_snapshot();
    let (algorithm, substituted) = config
        .string("diff.algorithm")
        .and_then(|value| Algorithm::from_config(&value.to_str_lossy()))
        .unwrap_or((Algorithm::Myers, false));
    let indent_heuristic = config.boolean("diff.indentHeuristic").unwrap_or(true);
    let options = LineOptions {
        algorithm,
        ignore_whitespace: false,
        indent_heuristic,
    };
    (options, substituted)
}

/// Canonical hunks of `path` on `side`, numbered from 1.
///
/// The path is relative to the repository root with `/` separators. A path
/// without a difference, an untracked path and a path with conflict stages
/// have no hunks. A binary file has one hunk without lines. The hunks carry
/// the file status and modes.
///
/// # Errors
///
/// Fails when the repository has no working tree for [`Side::Worktree`] or
/// when an object, the index or a worktree file cannot be read.
///
/// # Examples
///
/// ```no_run
/// use nurt_git::diff::{hunks_for, Side};
///
/// let repo = gix::open("/home/user/project")?;
/// for hunk in hunks_for(&repo, Side::Worktree, "src/main.rs".into())? {
///     println!("{} {}", hunk.id(), hunk.header_line());
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn hunks_for(repo: &gix::Repository, side: Side, path: &BStr) -> Result<Vec<Hunk>, Error> {
    let mut reader = Reader::new(repo)?;
    let index_entry = reader
        .index()
        .entry_by_path_and_stage(path, gix::index::entry::Stage::Unconflicted)
        .filter(|entry| {
            !entry
                .flags
                .contains(gix::index::entry::Flags::INTENT_TO_ADD)
        })
        .map(|entry| Entry {
            mode: entry.mode.bits(),
            id: Some(entry.id),
        });
    let pair = match side {
        Side::Worktree => {
            let Some(old) = index_entry else {
                return Ok(Vec::new());
            };
            let new = reader
                .worktree(path, Some(old.mode))?
                .map(|(data, mode)| (data, Entry { mode, id: None }));
            let old_data = reader.content(path, &old)?;
            let (new_data, new_entry) = match new {
                Some((data, entry)) => (data, Some(entry)),
                None => (Vec::new(), None),
            };
            let pair = canonical_pair(path, Some(old), new_entry);
            let driver = Driver::of(&mut reader, path);
            return Ok(diff_contents(
                pair,
                &old_data,
                &new_data,
                &canonical_options(repo),
                &driver,
            )
            .hunks);
        }
        Side::Staged => {
            let head = super::source::resolve_tree(repo, None)?;
            let tree = repo
                .find_tree(head)
                .map_err(|error| read_error("tree", &error))?;
            let relative =
                gix::path::from_bstr(path).map_err(|error| read_error("path", &error))?;
            let old = tree
                .lookup_entry_by_path(relative)
                .map_err(|error| read_error("tree", &error))?
                .filter(|entry| !entry.mode().is_tree())
                .map(|entry| Entry {
                    mode: super::source::tree_mode(entry.mode()),
                    id: Some(entry.object_id()),
                });
            canonical_pair(path, old, index_entry)
        }
    };
    if pair.old.is_none() && pair.new.is_none() {
        return Ok(Vec::new());
    }
    let driver = Driver::of(&mut reader, path);
    Ok(diff_pair(&mut reader, pair, &canonical_options(repo), &driver)?.hunks)
}

fn canonical_pair(path: &BStr, old: Option<Entry>, new: Option<Entry>) -> FilePair {
    FilePair {
        path: path.to_owned(),
        old_path: None,
        old,
        new,
        similarity: None,
        copied: false,
    }
}

fn canonical_options(repo: &gix::Repository) -> FileOptions {
    FileOptions {
        lines: configured_lines(repo).0,
        layout: Layout::default(),
    }
}

/// Canonical hunks of two texts of a modified file, with the default
/// options and the default function line rule.
///
/// # Examples
///
/// ```
/// use nurt_git::diff::text_hunks;
///
/// let hunks = text_hunks("a.txt", b"one\ntwo\n", b"one\n2\n");
/// assert_eq!(hunks.len(), 1);
/// assert_eq!(hunks[0].header_line(), "@@ -1,2 +1,2 @@");
/// ```
#[must_use]
pub fn text_hunks(path: &str, old: &[u8], new: &[u8]) -> Vec<Hunk> {
    let text = TextDiff::new(old, new, &LineOptions::default());
    text.hunks(
        &BString::from(path),
        &Layout::default(),
        &FuncMatcher::default_rule(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::fixture::Fixture;
    use crate::diff::hunk::LineKind;

    fn entry(mode: u32) -> Option<Entry> {
        Some(Entry { mode, id: None })
    }

    fn pair(old: Option<Entry>, new: Option<Entry>) -> FilePair {
        canonical_pair("f.txt".into(), old, new)
    }

    #[test]
    fn statuses_follow_sides() {
        assert_eq!(pair(None, entry(0o100_644)).status(), HunkStatus::Added);
        assert_eq!(pair(entry(0o100_644), None).status(), HunkStatus::Deleted);
        assert_eq!(
            pair(entry(0o100_644), entry(0o100_644)).status(),
            HunkStatus::Modified
        );
        let mut renamed = pair(entry(0o100_644), entry(0o100_644));
        renamed.old_path = Some("g.txt".into());
        assert_eq!(renamed.status(), HunkStatus::Renamed);
        renamed.copied = true;
        assert_eq!(renamed.status(), HunkStatus::Added);
    }

    #[test]
    fn binary_content_yields_one_hunk_without_lines() {
        let diff = diff_contents(
            pair(entry(0o100_644), entry(0o100_644)),
            b"a\0b",
            b"a\0c",
            &FileOptions::default(),
            &Driver::default(),
        );
        assert!(diff.binary);
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].status, HunkStatus::Binary);
        assert_eq!(diff.added, None);
        let same = diff_contents(
            pair(entry(0o100_644), entry(0o100_644)),
            b"\0",
            b"\0",
            &FileOptions::default(),
            &Driver::default(),
        );
        assert!(same.hunks.is_empty());
    }

    #[test]
    fn driver_can_force_text_or_binary() {
        let forced_text = Driver {
            binary: Some(false),
            ..Driver::default()
        };
        let diff = diff_contents(
            pair(entry(0o100_644), entry(0o100_644)),
            b"\0a\n",
            b"\0b\n",
            &FileOptions::default(),
            &forced_text,
        );
        assert!(!diff.binary);
        assert_eq!((diff.added, diff.removed), (Some(1), Some(1)));
        let forced_binary = Driver {
            binary: Some(true),
            ..Driver::default()
        };
        let diff = diff_contents(
            pair(entry(0o100_644), entry(0o100_644)),
            b"a\n",
            b"b\n",
            &FileOptions::default(),
            &forced_binary,
        );
        assert!(diff.binary);
    }

    #[test]
    fn mode_change_alone_has_no_hunks_but_is_not_empty() {
        let diff = diff_contents(
            pair(entry(0o100_644), entry(0o100_755)),
            b"a\n",
            b"a\n",
            &FileOptions::default(),
            &Driver::default(),
        );
        assert!(diff.hunks.is_empty());
        assert!(!diff.is_empty());
        assert_eq!(diff.old_mode().as_deref(), Some("100644"));
        assert_eq!(diff.new_mode().as_deref(), Some("100755"));
    }

    #[test]
    fn hunks_for_reads_worktree_and_staged_sides() {
        let fixture = Fixture::new();
        fixture.commit(&[("a.txt", "1\n2\n3\n"), ("b.txt", "x\n")]);
        fixture.stage(&[("a.txt", "1\nTWO\n3\n")]);
        fixture.write("a.txt", "1\nTWO\n3\nfour\n");
        let repo = fixture.repo();
        let staged = hunks_for(&repo, Side::Staged, "a.txt".into()).unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].header_line(), "@@ -1,3 +1,3 @@");
        let worktree = hunks_for(&repo, Side::Worktree, "a.txt".into()).unwrap();
        assert_eq!(worktree.len(), 1);
        let added: Vec<_> = worktree[0]
            .lines
            .iter()
            .filter(|line| line.kind == LineKind::Added)
            .map(|line| line.text.to_string())
            .collect();
        assert_eq!(added, ["four"]);
        assert!(
            hunks_for(&repo, Side::Worktree, "b.txt".into())
                .unwrap()
                .is_empty()
        );
        assert!(
            hunks_for(&repo, Side::Worktree, "missing.txt".into())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn hunks_for_reports_deleted_worktree_file() {
        let fixture = Fixture::new();
        fixture.commit(&[("a.txt", "1\n2\n")]);
        fixture.remove("a.txt");
        let repo = fixture.repo();
        let hunks = hunks_for(&repo, Side::Worktree, "a.txt".into()).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].status, HunkStatus::Deleted);
        assert_eq!(hunks[0].header_line(), "@@ -1,2 +0,0 @@");
    }

    #[test]
    fn text_hunks_number_hunks_of_a_file() {
        let old = "a\n".repeat(20);
        let mut new_lines: Vec<String> = vec!["a".to_owned(); 20];
        new_lines[1] = "b".to_owned();
        new_lines[18] = "c".to_owned();
        let new = new_lines.join("\n") + "\n";
        let hunks = text_hunks("f", old.as_bytes(), new.as_bytes());
        assert_eq!(
            hunks.iter().map(|hunk| hunk.number).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(hunks[1].id().to_string(), "f#2");
    }
}
