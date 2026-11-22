// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Replay of commits in the object database.
//!
//! Every pick merges the change of a commit (its first parent against the
//! commit) into the current result with the three-way tree merge of
//! `git.merge`, writes the new commit and moves on; the working tree, the
//! index and the references are not touched. The first pick that leaves
//! conflicts, or whose result would overwrite an untracked file, stops the
//! replay, as Git stops the sequencer.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::Result;

use crate::merge::{MergeLabels, MergeOptions, MergeOutcome, merge_trees};

use super::error;
use super::history::TodoItem;
use super::objects::{self, CommitInfo, NewCommit};

/// Value of `AUTO_MERGE` after a replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AutoMerge {
    /// No pick ran; the file stays as it is.
    Keep,
    /// The last merge produced this tree.
    Set(ObjectId),
    /// The last pick removed the file.
    Remove,
}

/// One line of the log of `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeadLine {
    /// Value before.
    pub(super) old: ObjectId,
    /// Value after.
    pub(super) new: ObjectId,
    /// Message.
    pub(super) message: String,
}

/// State of a replay between picks.
#[derive(Debug, Clone)]
pub(super) struct Progress {
    /// Commit the next pick builds on.
    pub(super) head: ObjectId,
    /// Tree of [`Self::head`].
    pub(super) tree: ObjectId,
    /// Commands run, including a stopped one.
    pub(super) done: Vec<TodoItem>,
    /// Lines for the log of `HEAD`, oldest first.
    pub(super) lines: Vec<HeadLine>,
    /// Original and new commit of every replayed commit.
    pub(super) rewritten: Vec<(ObjectId, ObjectId)>,
    /// Commits written.
    pub(super) created: Vec<ObjectId>,
    /// Commits dropped because their change is already present.
    pub(super) dropped: Vec<ObjectId>,
    /// Value of `AUTO_MERGE`.
    pub(super) auto_merge: AutoMerge,
    /// At least one pick ran, which removes `REBASE_HEAD`.
    pub(super) attempted: bool,
}

impl Progress {
    /// Replay that starts on `head` with the tree `tree`.
    pub(super) fn new(head: ObjectId, tree: ObjectId) -> Self {
        Self {
            head,
            tree,
            done: Vec::new(),
            lines: Vec::new(),
            rewritten: Vec::new(),
            created: Vec::new(),
            dropped: Vec::new(),
            auto_merge: AutoMerge::Keep,
            attempted: false,
        }
    }

    /// Moves to the commit `new` with the tree `tree` and logs it.
    pub(super) fn advance(&mut self, new: ObjectId, tree: ObjectId, message: String) {
        self.lines.push(HeadLine {
            old: self.head,
            new,
            message,
        });
        self.head = new;
        self.tree = tree;
    }
}

/// Reason a replay stopped.
#[derive(Debug, Clone)]
pub(super) enum Stop {
    /// The pick left conflicts in the result.
    Conflict {
        /// Stopped command.
        item: TodoItem,
        /// Picked commit.
        commit: CommitInfo,
        /// Tree of the first parent of the picked commit.
        parent_tree: ObjectId,
        /// Merge result with the conflict stages.
        outcome: Box<MergeOutcome>,
    },
    /// The result of the pick would overwrite untracked files; the command
    /// is scheduled again.
    Untracked {
        /// Stopped command.
        item: TodoItem,
        /// Picked commit.
        commit: CommitInfo,
        /// Untracked files in the way.
        paths: Vec<BString>,
    },
}

impl Stop {
    /// Stopped command.
    pub(super) fn item(&self) -> &TodoItem {
        match self {
            Self::Conflict { item, .. } | Self::Untracked { item, .. } => item,
        }
    }

    /// Picked commit.
    pub(super) fn commit(&self) -> &CommitInfo {
        match self {
            Self::Conflict { commit, .. } | Self::Untracked { commit, .. } => commit,
        }
    }
}

/// Result of a replay.
#[derive(Debug, Clone)]
pub(super) struct Replayed {
    /// State after the last pick.
    pub(super) progress: Progress,
    /// Reason of a stop; `None` when every command ran.
    pub(super) stop: Option<Stop>,
    /// Commands left after a stop, starting with the stopped one when it is
    /// scheduled again.
    pub(super) todo: Vec<TodoItem>,
}

/// Untracked files of the working tree that a replay must not overwrite.
pub(super) struct UntrackedGuard<'r> {
    repo: &'r gix::Repository,
    root: PathBuf,
    tracked: HashSet<BString>,
    excludes: gix_worktree::Stack,
}

impl<'r> UntrackedGuard<'r> {
    /// Guard for the working tree at `root` with the index `index`.
    pub(super) fn new(
        repo: &'r gix::Repository,
        root: &Path,
        index: &gix_index::State,
    ) -> Result<Self> {
        let excludes = repo
            .excludes(
                index,
                None,
                gix_worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
            )
            .map_err(|cause| error::io("read the ignore rules", cause))?
            .detach();
        let tracked = index
            .entries()
            .iter()
            .map(|entry| entry.path(index).to_owned())
            .collect();
        Ok(Self {
            repo,
            root: root.to_path_buf(),
            tracked,
            excludes,
        })
    }

    /// Untracked, not ignored files at the paths that the change from the
    /// tree `old` to the tree `new` creates.
    pub(super) fn collisions(&mut self, old: ObjectId, new: ObjectId) -> Result<Vec<BString>> {
        let mut out = Vec::new();
        for change in objects::diff_trees(self.repo, old, new)? {
            if change.old.is_some() || change.new.is_none() || self.tracked.contains(&change.path) {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(native(&self.root, change.path.as_bstr()))
            else {
                continue;
            };
            if meta.is_dir() {
                self.collect_untracked_in(&change.path, &mut out)?;
            } else if !self.is_ignored(change.path.as_bstr(), false)? {
                out.push(change.path);
            }
        }
        Ok(out)
    }

    /// Adds the untracked, not ignored files below the directory `dir`; a
    /// directory holding only tracked files is replaced without loss.
    fn collect_untracked_in(&mut self, dir: &BString, out: &mut Vec<BString>) -> Result<()> {
        let entries = std::fs::read_dir(native(&self.root, dir.as_bstr()))
            .map_err(|cause| error::io("read the working tree", cause))?;
        for entry in entries {
            let entry = entry.map_err(|cause| error::io("read the working tree", cause))?;
            let mut path = dir.clone();
            path.push(b'/');
            path.extend_from_slice(entry.file_name().to_string_lossy().as_bytes());
            let is_dir = entry
                .file_type()
                .map_err(|cause| error::io("read the working tree", cause))?
                .is_dir();
            if is_dir {
                self.collect_untracked_in(&path, out)?;
            } else if !self.tracked.contains(&path) && !self.is_ignored(path.as_bstr(), false)? {
                out.push(path);
            }
        }
        Ok(())
    }

    fn is_ignored(&mut self, path: &BStr, is_dir: bool) -> Result<bool> {
        let mode = is_dir.then_some(gix_index::entry::Mode::DIR);
        let platform = self
            .excludes
            .at_entry(path, mode, &self.repo.objects)
            .map_err(|cause| error::io("read the ignore rules", cause))?;
        Ok(platform.is_excluded())
    }
}

/// Native path of the slash separated `path` below `root`.
fn native(root: &Path, path: &BStr) -> PathBuf {
    let mut out = root.to_path_buf();
    for part in path.split_str("/") {
        out.push(part.to_str_lossy().as_ref());
    }
    out
}

/// Picks commands one after another in the object database.
pub(super) struct Replay<'r> {
    repo: &'r gix::Repository,
    options: MergeOptions,
    committer: gix::actor::Signature,
    action: String,
    guard: UntrackedGuard<'r>,
}

impl<'r> Replay<'r> {
    /// Replay with the merge settings of `repo`; `action` prefixes the
    /// messages of the log of `HEAD`.
    pub(super) fn new(
        repo: &'r gix::Repository,
        committer: gix::actor::Signature,
        action: String,
        guard: UntrackedGuard<'r>,
    ) -> Result<Self> {
        Ok(Self {
            repo,
            options: MergeOptions::from_repo(repo)?,
            committer,
            action,
            guard,
        })
    }

    /// Runs `todo` from `progress` until every command ran or one stops.
    pub(super) fn run(&mut self, mut progress: Progress, todo: Vec<TodoItem>) -> Result<Replayed> {
        let mut rest = todo.into_iter();
        while let Some(item) = rest.next() {
            if let Some(stop) = self.pick(&mut progress, &item)? {
                let mut remaining = Vec::new();
                if matches!(stop, Stop::Untracked { .. }) {
                    remaining.push(item);
                }
                remaining.extend(rest);
                return Ok(Replayed {
                    progress,
                    stop: Some(stop),
                    todo: remaining,
                });
            }
        }
        Ok(Replayed {
            progress,
            stop: None,
            todo: Vec::new(),
        })
    }

    /// Message of the log of `HEAD` for `what` with `subject`.
    pub(super) fn line(&self, what: &str, subject: &BStr) -> String {
        format!("{} ({what}): {}", self.action, subject.to_str_lossy())
    }

    fn pick(&mut self, progress: &mut Progress, item: &TodoItem) -> Result<Option<Stop>> {
        let commit = objects::read_commit(self.repo, item.id)?;
        progress.attempted = true;
        progress.done.push(item.clone());
        progress.auto_merge = AutoMerge::Remove;
        if commit.parents.as_slice() == [progress.head] {
            return self.fast_forward(progress, item, commit);
        }
        let parent_tree = match commit.parents.first() {
            Some(parent) => objects::tree_of(self.repo, *parent)?,
            None => objects::empty_tree(self.repo),
        };
        let outcome = merge_trees(
            self.repo,
            parent_tree.as_ref(),
            progress.tree.as_ref(),
            commit.tree.as_ref(),
            &self.options_for(item.id, &commit),
        )?;
        let blocked = self.guard.collisions(progress.tree, outcome.tree)?;
        if !blocked.is_empty() {
            return Ok(Some(Stop::Untracked {
                item: item.clone(),
                commit,
                paths: blocked,
            }));
        }
        progress.auto_merge = AutoMerge::Set(outcome.tree);
        if !outcome.is_clean() {
            return Ok(Some(Stop::Conflict {
                item: item.clone(),
                commit,
                parent_tree,
                outcome: Box::new(outcome),
            }));
        }
        if outcome.tree == progress.tree && commit.tree != parent_tree {
            progress.auto_merge = AutoMerge::Remove;
            progress.dropped.push(item.id);
            return Ok(None);
        }
        let new = objects::write_commit(
            self.repo,
            &NewCommit {
                tree: outcome.tree,
                parent: progress.head,
                author: commit.author.as_bstr(),
                encoding: commit.encoding.as_ref().map(|encoding| encoding.as_bstr()),
                message: &commit.message,
            },
            &self.committer,
        )?;
        let message = self.line("pick", commit.subject());
        progress.advance(new, outcome.tree, message);
        progress.rewritten.push((item.id, new));
        progress.created.push(new);
        Ok(None)
    }

    /// Reuses a commit whose only parent is the current commit.
    fn fast_forward(
        &mut self,
        progress: &mut Progress,
        item: &TodoItem,
        commit: CommitInfo,
    ) -> Result<Option<Stop>> {
        let blocked = self.guard.collisions(progress.tree, commit.tree)?;
        if !blocked.is_empty() {
            return Ok(Some(Stop::Untracked {
                item: item.clone(),
                commit,
                paths: blocked,
            }));
        }
        let message = self.line("pick", commit.subject());
        progress.advance(item.id, commit.tree, message);
        progress.rewritten.push((item.id, item.id));
        Ok(None)
    }

    /// Merge settings with the conflict marker labels Git uses for a pick:
    /// `HEAD`, `<short> (<subject>)` and `parent of <short> (<subject>)`.
    fn options_for(&self, id: ObjectId, commit: &CommitInfo) -> MergeOptions {
        let theirs = format!(
            "{} ({})",
            objects::short(self.repo, id),
            commit.subject().to_str_lossy()
        );
        let mut options = self.options.clone();
        options.labels = MergeLabels {
            base: Some(format!("parent of {theirs}")),
            ours: "HEAD".to_owned(),
            theirs,
        };
        options
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_paths_join_every_component() {
        let path = native(Path::new("/r"), BStr::new("d/e/f.txt"));
        assert_eq!(path, Path::new("/r").join("d").join("e").join("f.txt"));
    }

    #[test]
    fn advancing_logs_the_previous_and_the_new_commit() {
        let one = ObjectId::from_bytes_or_panic(&[1; 20]);
        let two = ObjectId::from_bytes_or_panic(&[2; 20]);
        let mut progress = Progress::new(one, one);
        progress.advance(two, two, "rebase (pick): x".to_owned());
        assert_eq!(progress.head, two);
        assert_eq!(
            progress.lines,
            vec![HeadLine {
                old: one,
                new: two,
                message: "rebase (pick): x".to_owned()
            }]
        );
        assert_eq!(progress.auto_merge, AutoMerge::Keep);
    }
}
