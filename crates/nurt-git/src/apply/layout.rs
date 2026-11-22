// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Translation of path-level decisions into ordered working tree changes.
//!
//! The order follows `git`: removals of files, removals of directories left
//! empty (deepest first), new directories (shallowest first), then files and
//! links in path order. A path that changes its kind is one change, so the
//! journal preserves the previous entry as a whole.

use std::collections::{BTreeMap, BTreeSet};

use bstr::{BStr, BString, ByteSlice};
use nurt_contracts::error::Result;
use nurt_contracts::journal::Expect;

use super::changes::{Changes, Write, build_index};
use super::context::ApplyContext;
use super::plan::{ApplyPlan, Content, EntryAction, EntryKind, IndexPlan, PlanEntry};
use super::worktree::{Node, Observation, Worktree, expect_of};

/// Completes a plan from the decisions of a planner.
pub(crate) fn finish(
    ctx: &ApplyContext<'_>,
    worktree: &mut Worktree<'_>,
    changes: Changes,
    write_index: bool,
) -> Result<ApplyPlan> {
    let mut layout = Layout::default();
    layout.removals(worktree, &changes)?;
    layout.directories(worktree, &changes)?;
    layout.writes(worktree, &changes)?;
    layout.empty_directories(worktree)?;
    let index = if write_index {
        Some(IndexPlan {
            state: build_index(&ctx.index, &changes)?,
            expect: ctx.index_expect.clone(),
        })
    } else {
        None
    };
    Ok(ApplyPlan {
        entries: layout.into_entries(),
        index,
        refs: Vec::new(),
        state_files: Vec::new(),
        carried: changes.carried,
        notable: changes.notable,
    })
}

#[derive(Default)]
struct Layout {
    deletes: BTreeMap<BString, PlanEntry>,
    rmdirs: Vec<PlanEntry>,
    dirs: BTreeMap<BString, PlanEntry>,
    files: BTreeMap<BString, PlanEntry>,
    /// Directories created or replaced by the plan; entries below them do
    /// not exist before the change.
    fresh: BTreeSet<BString>,
    /// Paths that the plan removes, used to find directories left empty.
    gone: BTreeSet<BString>,
}

impl Layout {
    fn observe(&self, worktree: &Worktree<'_>, path: &BStr) -> Result<Observation> {
        if ancestors(path).any(|dir| self.fresh.contains(dir)) {
            return Ok(Observation {
                node: Node::Missing,
                meta: None,
            });
        }
        worktree.observe(path)
    }

    fn removals(&mut self, worktree: &Worktree<'_>, changes: &Changes) -> Result<()> {
        for path in &changes.removals {
            if changes.writes.contains_key(path) {
                continue;
            }
            let observation = worktree.observe(path.as_bstr())?;
            if !observation.node.exists() || observation.node.is_dir() {
                continue;
            }
            let expect = expect_of(&worktree.path(path.as_bstr()), &observation)?;
            self.gone.insert(path.clone());
            self.deletes
                .insert(path.clone(), entry(path, EntryAction::Delete, expect, None));
        }
        Ok(())
    }

    fn directories(&mut self, worktree: &Worktree<'_>, changes: &Changes) -> Result<()> {
        let needed: BTreeSet<&BStr> = changes
            .writes
            .keys()
            .flat_map(|path| ancestors(path.as_bstr()))
            .collect();
        for dir in needed {
            let observation = self.observe(worktree, dir)?;
            let action = match observation.node {
                Node::Dir => continue,
                Node::Missing => EntryAction::Mkdir,
                _ => EntryAction::TypeChange,
            };
            let expect = expect_of(&worktree.path(dir), &observation)?;
            self.deletes.remove(dir);
            self.fresh.insert(dir.to_owned());
            let mut planned = entry(&dir.to_owned(), action, expect, None);
            planned.kind = Some(EntryKind::Directory);
            self.dirs.insert(dir.to_owned(), planned);
        }
        Ok(())
    }

    fn writes(&mut self, worktree: &Worktree<'_>, changes: &Changes) -> Result<()> {
        for (path, write) in &changes.writes {
            let observation = self.observe(worktree, path.as_bstr())?;
            let Some(action) = write_action(worktree, write, observation.node) else {
                continue;
            };
            if observation.node.is_dir() {
                self.fold_directory(path.as_bstr());
            }
            let expect = expect_of(&worktree.path(path.as_bstr()), &observation)?;
            let content = (write.kind != EntryKind::Directory).then(|| write.content.clone());
            let mut planned = entry(path, action, expect, content);
            planned.kind = Some(write.kind);
            if action == EntryAction::Mkdir {
                self.fresh.insert(path.clone());
                self.dirs.insert(path.clone(), planned);
            } else {
                self.files.insert(path.clone(), planned);
            }
        }
        Ok(())
    }

    /// Drops the removals below a directory that a file replaces as a whole.
    fn fold_directory(&mut self, dir: &BStr) {
        let mut prefix = dir.to_owned();
        prefix.push(b'/');
        self.deletes
            .retain(|path, _| !path.starts_with(prefix.as_slice()));
        self.fresh.insert(dir.to_owned());
    }

    fn empty_directories(&mut self, worktree: &Worktree<'_>) -> Result<()> {
        let mut candidates: Vec<BString> = self
            .gone
            .iter()
            .flat_map(|path| ancestors(path.as_bstr()).map(ToOwned::to_owned))
            .filter(|dir| !self.dirs.contains_key(dir) && !self.fresh.contains(dir))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        candidates
            .sort_by_key(|dir| std::cmp::Reverse(dir.iter().filter(|byte| **byte == b'/').count()));
        for dir in candidates {
            if self.is_emptied(worktree, dir.as_bstr())? {
                self.gone.insert(dir.clone());
                self.rmdirs
                    .push(entry(&dir, EntryAction::Rmdir, Expect::Unchecked, None));
            }
        }
        Ok(())
    }

    fn is_emptied(&self, worktree: &Worktree<'_>, dir: &BStr) -> Result<bool> {
        let native = worktree.path(dir);
        let Ok(reader) = std::fs::read_dir(&native) else {
            return Ok(false);
        };
        for item in reader {
            let Ok(item) = item else {
                return Ok(false);
            };
            let Ok(name) = gix_path::os_str_into_bstr(&item.file_name()) else {
                return Ok(false);
            };
            let mut child = dir.to_owned();
            child.push(b'/');
            child.extend_from_slice(name);
            if !self.gone.contains(&child) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn into_entries(self) -> Vec<PlanEntry> {
        let mut entries: Vec<PlanEntry> = self.deletes.into_values().collect();
        entries.extend(self.rmdirs);
        entries.extend(self.dirs.into_values());
        entries.extend(self.files.into_values());
        entries
    }
}

fn write_action(worktree: &Worktree<'_>, write: &Write, node: Node) -> Option<EntryAction> {
    let link = write.kind == EntryKind::Symlink && worktree.caps.symlink;
    Some(match (write.kind, node) {
        (EntryKind::Directory, Node::Dir) => return None,
        (EntryKind::Directory, Node::Missing) => EntryAction::Mkdir,
        (_, Node::Missing) if link => EntryAction::Symlink,
        (_, Node::Missing) => EntryAction::Create,
        (EntryKind::File | EntryKind::Executable, Node::File { .. }) if write.mode_only => {
            EntryAction::Chmod
        }
        (_, Node::File { .. }) if !link && write.kind != EntryKind::Directory => {
            EntryAction::Modify
        }
        _ => EntryAction::TypeChange,
    })
}

fn entry(
    path: &BString,
    action: EntryAction,
    expect: Expect,
    content: Option<Content>,
) -> PlanEntry {
    PlanEntry {
        path: path.clone(),
        action,
        expect,
        content,
        kind: None,
    }
}

/// Leading directories of `path`, shallowest first.
pub(crate) fn ancestors(path: &BStr) -> impl Iterator<Item = &BStr> {
    path.iter()
        .enumerate()
        .filter(|(_, byte)| **byte == b'/')
        .map(move |(position, _)| path[..position].as_bstr())
}
