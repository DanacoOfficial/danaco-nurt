// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Files changed between two trees, with detection of renamed and copied
//! files.

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use gix::diff::tree_with_rewrites::Change;
use gix::objs::tree::{EntryKind, EntryMode};
use nurt_contracts::error::Error;
use nurt_contracts::record::GitFileState;

use super::lines;
use super::paths::{PathFilter, entry_at};
use super::repo::OpenRepo;

/// Similarity, as a fraction of the content, above which two files are a
/// rename or a copy; the default of `git diff`.
const SIMILARITY: f32 = 0.5;

/// Largest number of candidate files compared for renames; the default of
/// `diff.renameLimit`.
const RENAME_LIMIT: usize = 1000;

/// One side of a changed file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Side {
    /// Blob or submodule commit.
    pub(crate) id: ObjectId,
    /// Mode of the entry.
    pub(crate) mode: EntryMode,
}

/// A file changed between two trees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileChange {
    /// Kind of the change.
    pub(crate) status: GitFileState,
    /// Path after the change.
    pub(crate) path: BString,
    /// Path before a rename or a copy.
    pub(crate) old_path: Option<BString>,
    /// Entry before the change; absent for an added file.
    pub(crate) old: Option<Side>,
    /// Entry after the change; absent for a deleted file.
    pub(crate) new: Option<Side>,
}

impl FileChange {
    /// Path before the change.
    pub(crate) fn source_path(&self) -> &BStr {
        self.old_path.as_ref().unwrap_or(&self.path).as_ref()
    }
}

/// Detection of files that moved between the trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Detect {
    /// Every file is compared by path only.
    Nothing,
    /// Deleted and added files are paired into renames.
    Renames,
    /// As [`Detect::Renames`], and an added file may also be a copy of a file
    /// that the same change modified (`-C`).
    Copies,
    /// As [`Detect::Renames`], and an added file may also be a copy of any
    /// file of the old tree, changed or not (`-C -C`).
    RenamesAndCopies,
}

/// Files changed from `old` to `new`, ordered by path; a missing old tree is
/// the empty tree.
///
/// With `limit`, files outside it are left out before moved files are
/// detected, so a file moved into the limit from outside is an addition.
pub(crate) fn tree_changes(
    repo: &OpenRepo,
    old: Option<ObjectId>,
    new: ObjectId,
    detect: Detect,
    limit: Option<&PathFilter>,
) -> Result<Vec<FileChange>, Error> {
    let detect = if limit.is_some() {
        Detect::Nothing
    } else {
        detect
    };
    let read = |what: &str, err: &dyn std::fmt::Display| {
        repo.read_error(format_args!("the {what} tree"), err)
    };
    let old_tree = old
        .map(|id| repo.repo.find_tree(id))
        .transpose()
        .map_err(|err| read("old", &err))?;
    let new_tree = repo.repo.find_tree(new).map_err(|err| read("new", &err))?;
    let options = gix::diff::Options::default().with_rewrites(rewrites(detect));
    let raw = repo
        .repo
        .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), options)
        .map_err(|err| repo.read_error("the difference of two trees", err))?;
    let mut changes: Vec<FileChange> = raw
        .into_iter()
        .filter_map(convert)
        .filter(|change| limit.is_none_or(|filter| filter.matches(change.path.as_ref())))
        .collect();
    if detect == Detect::Copies {
        pair_copies(repo, &mut changes)?;
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

/// Turns each added file that is similar enough to the previous content of a
/// modified file into a copy of it.
///
/// `gix` pairs copies with the new content of modified files, so the pairing
/// is done here against the content before the change, as `git diff -C`
/// does.
fn pair_copies(repo: &OpenRepo, changes: &mut [FileChange]) -> Result<(), Error> {
    let sources: Vec<(BString, Side)> = changes
        .iter()
        .filter(|change| change.status == GitFileState::Modified)
        .filter_map(|change| {
            change
                .old
                .filter(|side| side.mode.kind() != EntryKind::Commit)
                .map(|side| (change.path.clone(), side))
        })
        .collect();
    if sources.is_empty() {
        return Ok(());
    }
    for change in changes
        .iter_mut()
        .filter(|change| change.status == GitFileState::Added)
    {
        let Some(new) = change
            .new
            .filter(|side| side.mode.kind() != EntryKind::Commit)
        else {
            continue;
        };
        let target = repo.blob(new.id)?;
        let mut best: Option<(f32, &(BString, Side))> = None;
        for source in &sources {
            let score = similarity(&repo.blob(source.1.id)?, &target);
            if score >= SIMILARITY && best.is_none_or(|(top, _)| score > top) {
                best = Some((score, source));
            }
        }
        if let Some((_, (path, side))) = best {
            change.status = GitFileState::Copied;
            change.old_path = Some(path.clone());
            change.old = Some(*side);
        }
    }
    Ok(())
}

/// Fraction of the larger content that the two contents share as lines.
fn similarity(old: &[u8], new: &[u8]) -> f32 {
    let largest = old.len().max(new.len());
    if largest == 0 || lines::is_binary(old) || lines::is_binary(new) {
        return 0.0;
    }
    let new_lines = lines::split_lines(new);
    let changed: usize = lines::edits(old, new)
        .iter()
        .map(|edit| {
            new_lines
                .get(edit.after.start as usize..edit.after.end as usize)
                .map_or(0, |added| added.iter().map(|line| line.len()).sum())
        })
        .sum();
    (new.len() - changed.min(new.len())) as f32 / largest as f32
}

fn rewrites(detect: Detect) -> Option<gix::diff::Rewrites> {
    let copies = match detect {
        Detect::Nothing => return None,
        Detect::Renames | Detect::Copies => None,
        Detect::RenamesAndCopies => Some(gix::diff::rewrites::Copies {
            source: gix::diff::rewrites::CopySource::FromSetOfModifiedFilesAndAllSources,
            percentage: Some(SIMILARITY),
        }),
    };
    Some(gix::diff::Rewrites {
        copies,
        percentage: Some(SIMILARITY),
        limit: RENAME_LIMIT,
        track_empty: false,
    })
}

fn convert(change: Change) -> Option<FileChange> {
    match change {
        Change::Addition {
            location,
            entry_mode,
            id,
            ..
        } => (!entry_mode.is_tree()).then_some(FileChange {
            status: GitFileState::Added,
            path: location,
            old_path: None,
            old: None,
            new: Some(Side {
                id,
                mode: entry_mode,
            }),
        }),
        Change::Deletion {
            location,
            entry_mode,
            id,
            ..
        } => (!entry_mode.is_tree()).then_some(FileChange {
            status: GitFileState::Deleted,
            path: location,
            old_path: None,
            old: Some(Side {
                id,
                mode: entry_mode,
            }),
            new: None,
        }),
        Change::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
        } => modification(location, previous_entry_mode, previous_id, entry_mode, id),
        Change::Rewrite {
            source_location,
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            location,
            copy,
            ..
        } => (!entry_mode.is_tree()).then_some(FileChange {
            status: if copy {
                GitFileState::Copied
            } else {
                GitFileState::Renamed
            },
            path: location,
            old_path: Some(source_location),
            old: Some(Side {
                id: source_id,
                mode: source_entry_mode,
            }),
            new: Some(Side {
                id,
                mode: entry_mode,
            }),
        }),
    }
}

fn modification(
    location: BString,
    previous_mode: EntryMode,
    previous_id: ObjectId,
    mode: EntryMode,
    id: ObjectId,
) -> Option<FileChange> {
    if previous_mode.is_tree() && mode.is_tree() {
        return None;
    }
    let status = if previous_mode.kind() == mode.kind() {
        GitFileState::Modified
    } else {
        GitFileState::TypeChanged
    };
    Some(FileChange {
        status,
        path: location,
        old_path: None,
        old: Some(Side {
            id: previous_id,
            mode: previous_mode,
        }),
        new: Some(Side { id, mode }),
    })
}

/// Path of the file that `path` was renamed or copied from in a commit whose
/// tree is `new`, when `path` is new in it.
pub(crate) fn origin_of(
    repo: &OpenRepo,
    old: Option<ObjectId>,
    new: ObjectId,
    path: &BStr,
) -> Result<Option<FileChange>, Error> {
    let Some(old) = old else {
        return Ok(None);
    };
    let changes = tree_changes(repo, Some(old), new, Detect::RenamesAndCopies, None)?;
    if let Some(found) = changes
        .into_iter()
        .find(|change| change.path == path && change.old_path.is_some())
    {
        return Ok(Some(found));
    }
    exact_copy(repo, old, new, path)
}

/// A file of `old` whose content equals the file at `path` in `new`: an
/// exact copy of a file that the commit left unchanged.
fn exact_copy(
    repo: &OpenRepo,
    old: ObjectId,
    new: ObjectId,
    path: &BStr,
) -> Result<Option<FileChange>, Error> {
    let target = entry_at(repo, new, path).map_err(|err| repo.read_error("the new tree", err))?;
    let Some((mode, id)) = target.filter(|(mode, _)| !mode.is_tree()) else {
        return Ok(None);
    };
    let tree = repo
        .repo
        .find_tree(old)
        .map_err(|err| repo.read_error("the old tree", err))?;
    let files = tree
        .traverse()
        .breadthfirst
        .files()
        .map_err(|err| repo.read_error("the old tree", err))?;
    Ok(files
        .into_iter()
        .find(|entry| entry.oid == id && !entry.mode.is_tree() && entry.filepath.as_bstr() != path)
        .map(|entry| FileChange {
            status: GitFileState::Copied,
            path: path.to_owned(),
            old_path: Some(entry.filepath),
            old: Some(Side {
                id: entry.oid,
                mode: entry.mode,
            }),
            new: Some(Side { id, mode }),
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revwalk::fixture::Fixture;

    const TEXT: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";

    #[test]
    fn lists_added_modified_deleted_and_renamed_files() {
        let mut fixture = Fixture::new();
        let first = fixture.commit(
            "first",
            &[
                ("a.txt", Some(TEXT)),
                ("b.txt", Some("b\n")),
                ("d/x", Some("x\n")),
            ],
        );
        let second = fixture.commit(
            "second",
            &[
                ("a.txt", None),
                ("z.txt", Some(TEXT)),
                ("b.txt", Some("b2\n")),
                ("d/x", None),
                ("n.txt", Some("n\n")),
            ],
        );
        let repo = fixture.open();
        let old = repo.tree_of(first).unwrap();
        let new = repo.tree_of(second).unwrap();
        let changes = tree_changes(&repo, Some(old), new, Detect::Renames, None).unwrap();
        let summary: Vec<(GitFileState, String, Option<String>)> = changes
            .iter()
            .map(|c| {
                (
                    c.status,
                    c.path.to_string(),
                    c.old_path.as_ref().map(ToString::to_string),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (GitFileState::Modified, "b.txt".to_owned(), None),
                (GitFileState::Deleted, "d/x".to_owned(), None),
                (GitFileState::Added, "n.txt".to_owned(), None),
                (
                    GitFileState::Renamed,
                    "z.txt".to_owned(),
                    Some("a.txt".to_owned())
                ),
            ]
        );
        assert_eq!(changes[3].source_path(), "a.txt");
        let plain = tree_changes(&repo, Some(old), new, Detect::Nothing, None).unwrap();
        assert_eq!(plain.len(), 5);
        let limited = tree_changes(
            &repo,
            Some(old),
            new,
            Detect::Renames,
            Some(&PathFilter::new(vec!["z.txt".into()])),
        )
        .unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].status, GitFileState::Added);
    }

    #[test]
    fn finds_the_origin_of_copied_files() {
        let mut fixture = Fixture::new();
        let first = fixture.commit("first", &[("a.txt", Some(TEXT))]);
        let second = fixture.commit("copy", &[("b.txt", Some(TEXT))]);
        let repo = fixture.open();
        let old = repo.tree_of(first).unwrap();
        let new = repo.tree_of(second).unwrap();
        let origin = origin_of(&repo, Some(old), new, "b.txt".into())
            .unwrap()
            .unwrap();
        assert_eq!(origin.status, GitFileState::Copied);
        assert_eq!(origin.source_path(), "a.txt");
        assert!(
            origin_of(&repo, None, new, "b.txt".into())
                .unwrap()
                .is_none()
        );
        assert!(
            origin_of(&repo, Some(old), new, "a.txt".into())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reports_type_changes() {
        let mut fixture = Fixture::new();
        let first = fixture.commit("first", &[("t", Some("target\n"))]);
        let second = fixture.commit_link("link", "t", "elsewhere");
        let repo = fixture.open();
        let changes = tree_changes(
            &repo,
            Some(repo.tree_of(first).unwrap()),
            repo.tree_of(second).unwrap(),
            Detect::Nothing,
            None,
        )
        .unwrap();
        assert_eq!(changes[0].status, GitFileState::TypeChanged);
    }
}
