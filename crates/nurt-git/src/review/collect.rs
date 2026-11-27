// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::review::collect
// Responsibility:   Changed files of a review: the two sides of the scope or the range, the untracked files, and the canonical hunks of each text file.
// Dependencies:     bstr, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Changed files of a review: the two sides of the scope or the range, the
//! untracked files, and the canonical hunks of each text file.

use std::collections::{BTreeMap, BTreeSet};

use bstr::{BString, ByteSlice};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::report::{Notable, NotableEntry};

use super::ReviewRequest;
use super::rules::LARGE_FILE_BYTES;
use crate::commit::access::Worktree;
use crate::diff::command::{WorktreeState, worktree_changes};
use crate::diff::source::{index_files, resolve_tree, tree_files};
use crate::diff::{
    Driver, Entry, FileOptions, FilePair, Hunk, Layout, Reader, configured_lines, diff_contents,
    looks_binary,
};
use crate::repo::GitRepo;
use crate::repo::error::quote_path;
use crate::repo::pathspec::PathFilter;

/// Most untracked files a review reads; the others are counted in a
/// `review_skipped` note.
pub const UNTRACKED_MAX: usize = 300;

/// Changes a review looks at when no range is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The working tree and the untracked files against `HEAD`.
    All,
    /// The index against `HEAD`.
    Staged,
    /// The working tree and the untracked files against the index.
    Working,
}

impl Scope {
    /// The scope with the catalog name `name`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git::review::Scope;
    ///
    /// assert_eq!(Scope::from_name("staged"), Some(Scope::Staged));
    /// assert_eq!(Scope::from_name("cached"), None);
    /// ```
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "all" => Some(Self::All),
            "staged" => Some(Self::Staged),
            "working" => Some(Self::Working),
            _ => None,
        }
    }
}

/// A changed file with the content of both sides.
#[derive(Debug, Clone)]
pub(crate) struct Changed {
    /// Path relative to the repository root.
    pub(crate) path: BString,
    /// Old content; empty for a new file.
    pub(crate) old: Vec<u8>,
    /// New content; empty when a large untracked file is not read.
    pub(crate) new: Vec<u8>,
    /// Size of the new content in bytes.
    pub(crate) size: u64,
    /// The content is binary.
    pub(crate) binary: bool,
    /// Hunks of the difference; empty for binary content.
    pub(crate) hunks: Vec<Hunk>,
    /// The hunks are those of the canonical difference, so their
    /// identifiers select them in `git.stage` and `git.restore`.
    pub(crate) canonical: bool,
}

/// Changed files and the files tracked on the new side.
#[derive(Debug, Clone, Default)]
pub(crate) struct Collected {
    /// Changed files in path order; deleted files are left out.
    pub(crate) changes: Vec<Changed>,
    /// Tracked files of the new side, for the `tests` check.
    pub(crate) tracked: Vec<BString>,
}

/// New side of a changed path.
#[derive(Debug, Clone, Copy)]
enum New {
    /// A stored object.
    Object(Entry),
    /// The working tree file of a tracked path, with the mode of the index.
    Worktree(Option<u32>),
    /// An untracked file with its size; a link has no content to review.
    Untracked { size: u64, link: bool },
}

/// A changed path before its content is read.
#[derive(Debug, Clone, Copy)]
struct Pair {
    old: Option<Entry>,
    new: New,
    canonical: bool,
}

type Pairs = BTreeMap<BString, Pair>;

/// Collects the changed files of `request`.
pub(crate) fn collect(
    repo: &GitRepo,
    request: &ReviewRequest,
    notes: &mut Vec<NotableEntry>,
) -> Result<Collected> {
    let gix = repo.gix();
    let mut reader = Reader::new(gix)?;
    let select = PathFilter::new(repo.root(), &request.paths, &[])?;
    let exclude = PathFilter::new(repo.root(), &[], &request.exclude)?;
    let (mut pairs, tracked) = match &request.base {
        Some(base) => range_pairs(gix, base, request.target.as_deref())?,
        None => {
            if request.scope != Scope::Staged {
                require_workdir(repo)?;
            }
            let pairs = match request.scope {
                Scope::Staged => staged_pairs(gix, reader.index())?,
                Scope::Working => working_pairs(gix, reader.index())?,
                Scope::All => all_pairs(gix, reader.index())?,
            };
            (pairs, index_files(reader.index()).0.into_keys().collect())
        }
    };
    pairs.retain(|path, _| select.matches(path.as_bstr()));
    if request.base.is_none() && request.scope != Scope::Staged {
        add_untracked(repo, reader.index(), &select, &mut pairs, notes)?;
    }
    let before = pairs.len();
    pairs.retain(|path, _| !exclude.excluded(path.as_bstr()));
    let excluded = before - pairs.len();
    if excluded > 0 {
        let mut note = NotableEntry::new(Notable::Excluded);
        note.count = u64::try_from(excluded).ok();
        note.detail = Some("files left out of the review by `exclude`".to_owned());
        notes.push(note);
    }
    let options = FileOptions {
        lines: configured_lines(gix).0,
        layout: Layout::default(),
    };
    let mut changes = Vec::new();
    for (path, pair) in pairs {
        repo.ctx().checkpoint()?;
        if let Some(changed) = load(&mut reader, path, pair, &options)? {
            changes.push(changed);
        }
    }
    Ok(Collected { changes, tracked })
}

fn require_workdir(repo: &GitRepo) -> Result<()> {
    if repo.workdir().is_some() {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::GitUnsupported,
        "the repository has no working tree; review the index with `scope: staged` or a range with `base`",
    )
    .with_problem("bare_repository")
    .with_fix(format!(
        "{} {} scope: staged",
        super::COMMAND,
        quote_path(repo.root())
    )))
}

/// Files of `target` (or `HEAD`) that differ from `base`.
fn range_pairs(
    repo: &gix::Repository,
    base: &str,
    target: Option<&str>,
) -> Result<(Pairs, Vec<BString>)> {
    let old = tree_files(repo, resolve_tree(repo, Some(base))?)?;
    let new = tree_files(repo, resolve_tree(repo, Some(target.unwrap_or("HEAD")))?)?;
    let pairs = new
        .iter()
        .filter(|(path, entry)| old.get(*path) != Some(*entry))
        .map(|(path, entry)| {
            (
                path.clone(),
                Pair {
                    old: old.get(path).copied(),
                    new: New::Object(*entry),
                    canonical: false,
                },
            )
        })
        .collect();
    Ok((pairs, new.into_keys().collect()))
}

/// Index entries that differ from `HEAD`.
fn staged_pairs(repo: &gix::Repository, index: &gix::index::State) -> Result<Pairs> {
    let head = tree_files(repo, resolve_tree(repo, None)?)?;
    let (staged, _) = index_files(index);
    Ok(staged
        .into_iter()
        .filter(|(path, entry)| head.get(path) != Some(entry))
        .map(|(path, entry)| {
            let pair = Pair {
                old: head.get(&path).copied(),
                new: New::Object(entry),
                canonical: true,
            };
            (path, pair)
        })
        .collect())
}

/// Working tree files that differ from the index, and the files with
/// conflicts against the `ours` stage.
fn working_pairs(repo: &gix::Repository, index: &gix::index::State) -> Result<Pairs> {
    let (staged, conflicts) = index_files(index);
    let mut pairs = Pairs::new();
    for (path, state) in worktree_changes(repo)? {
        if state == WorktreeState::Removed {
            continue;
        }
        let old = staged.get(&path).copied();
        pairs.insert(
            path,
            Pair {
                old,
                new: New::Worktree(old.map(|entry| entry.mode)),
                canonical: old.is_some(),
            },
        );
    }
    for (path, stages) in conflicts {
        let ours = stages[1];
        pairs.insert(
            path,
            Pair {
                old: ours,
                new: New::Worktree(ours.map(|entry| entry.mode)),
                canonical: false,
            },
        );
    }
    Ok(pairs)
}

/// Working tree content of every path whose index entry or working tree file
/// differs from `HEAD`.
///
/// The difference against `HEAD` is the canonical one when either the index
/// equals `HEAD` (the working tree side) or the working tree equals the index
/// (the staged side) for the path.
fn all_pairs(repo: &gix::Repository, index: &gix::index::State) -> Result<Pairs> {
    let head = tree_files(repo, resolve_tree(repo, None)?)?;
    let (staged, conflicts) = index_files(index);
    let worktree = worktree_changes(repo)?;
    let paths: BTreeSet<&BString> = staged
        .iter()
        .filter(|(path, entry)| head.get(*path) != Some(*entry))
        .map(|(path, _)| path)
        .chain(worktree.keys())
        .chain(conflicts.keys())
        .collect();
    let mut pairs = Pairs::new();
    for path in paths {
        let old = head.get(path).copied();
        let index_entry = staged.get(path).copied();
        let new = if conflicts.contains_key(path) {
            Some((New::Worktree(None), false))
        } else {
            match (worktree.get(path), index_entry) {
                (Some(WorktreeState::Removed), _) | (None, None) => None,
                (Some(WorktreeState::Changed), entry) => Some((
                    New::Worktree(entry.map(|entry| entry.mode)),
                    entry.is_some() && entry == old,
                )),
                (None, Some(entry)) => Some((New::Object(entry), true)),
            }
        };
        if let Some((new, canonical)) = new {
            pairs.insert(
                path.clone(),
                Pair {
                    old,
                    new,
                    canonical,
                },
            );
        }
    }
    Ok(pairs)
}

/// Adds the untracked files selected by `select`, at most
/// [`UNTRACKED_MAX`].
fn add_untracked(
    repo: &GitRepo,
    index: &gix::index::State,
    select: &PathFilter,
    pairs: &mut Pairs,
    notes: &mut Vec<NotableEntry>,
) -> Result<()> {
    let worktree = Worktree::new(repo)?;
    let untracked = worktree.untracked(index, select)?;
    if untracked.len() > UNTRACKED_MAX {
        let mut note = NotableEntry::new(Notable::ReviewSkipped);
        note.count = u64::try_from(untracked.len() - UNTRACKED_MAX).ok();
        note.detail = Some(format!(
            "only the first {UNTRACKED_MAX} untracked files were reviewed; stage files or review fewer paths"
        ));
        notes.push(note);
    }
    for path in untracked.into_iter().take(UNTRACKED_MAX) {
        let absolute = worktree.absolute(path.as_bstr());
        let Ok(metadata) = std::fs::symlink_metadata(&absolute) else {
            continue;
        };
        let kind = metadata.file_type();
        if !kind.is_file() && !kind.is_symlink() {
            continue;
        }
        pairs.insert(
            path,
            Pair {
                old: None,
                new: New::Untracked {
                    size: metadata.len(),
                    link: metadata.file_type().is_symlink(),
                },
                canonical: false,
            },
        );
    }
    Ok(())
}

/// Reads both sides of a pair; `None` when there is nothing to review: the
/// new side is missing, a link or a submodule, or equal to the old side.
fn load(
    reader: &mut Reader<'_>,
    path: BString,
    pair: Pair,
    options: &FileOptions,
) -> Result<Option<Changed>> {
    let old_entry = pair.old.filter(Entry::is_file);
    let (new, new_entry, size) = match pair.new {
        New::Object(entry) => {
            if !entry.is_file() {
                return Ok(None);
            }
            let data = reader.content(path.as_bstr(), &entry)?;
            let size = data.len() as u64;
            (data, entry, size)
        }
        New::Worktree(index_mode) => match reader.worktree(path.as_bstr(), index_mode)? {
            Some((data, mode)) => {
                let entry = Entry { mode, id: None };
                if !entry.is_file() {
                    return Ok(None);
                }
                let size = data.len() as u64;
                (data, entry, size)
            }
            None => return Ok(None),
        },
        New::Untracked { link: true, .. } => return Ok(None),
        New::Untracked { size, .. } if size > LARGE_FILE_BYTES => {
            return Ok(Some(Changed {
                path,
                old: Vec::new(),
                new: Vec::new(),
                size,
                binary: true,
                hunks: Vec::new(),
                canonical: false,
            }));
        }
        New::Untracked { .. } => match reader.worktree(path.as_bstr(), None)? {
            Some((data, mode)) => {
                let size = data.len() as u64;
                (data, Entry { mode, id: None }, size)
            }
            None => return Ok(None),
        },
    };
    let old = match &old_entry {
        Some(entry) => reader.content(path.as_bstr(), entry)?,
        None => Vec::new(),
    };
    if old_entry.is_some() && old == new {
        return Ok(None);
    }
    let driver = Driver::of(reader, path.as_bstr());
    let binary = driver
        .binary
        .unwrap_or_else(|| looks_binary(&old) || looks_binary(&new));
    let hunks = if binary {
        Vec::new()
    } else {
        let file = FilePair {
            path: path.clone(),
            old_path: None,
            old: old_entry,
            new: Some(new_entry),
            similarity: None,
            copied: false,
        };
        diff_contents(file, &old, &new, options, &driver).hunks
    };
    if !binary && hunks.is_empty() {
        return Ok(None);
    }
    Ok(Some(Changed {
        path,
        old,
        new,
        size,
        binary,
        hunks,
        canonical: pair.canonical,
    }))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
