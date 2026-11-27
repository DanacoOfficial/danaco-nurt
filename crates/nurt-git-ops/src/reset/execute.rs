// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::reset::execute
// Responsibility:   Execution of a planned repository change in one journal transaction.
// Dependencies:     gix, nurt_contracts, nurt_git
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Execution of a planned repository change in one journal transaction.
//!
//! A change has three parts: the working tree, the index and the state
//! files ([`ApplyPlan`]), the references and their reflogs
//! ([`PreparedRefTx`]), and files replaced as a whole, such as a rewritten
//! reflog ([`LockedWrite`]). The reference and file targets are declared
//! first, the working tree plan then declares its own targets and applies
//! them, and the files and references are written last, so that every
//! target is preserved before the first change.

use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::path::Path;

use gix::actor::Signature;
use gix::bstr::{BString, ByteSlice};
use gix::hash::ObjectId;
use gix::refs::Target;
use nurt_contracts::error::Result;
use nurt_contracts::journal::{Expect, MutationTx};
use nurt_git::apply::{
    ApplyContext, ApplyPlan, EntryAction, EntryKind, PlanEntry, StateFilePlan, apply_plan,
};
use nurt_git::refs_tx::{PreparedRefTx, PreviousValue, RefEdit, RefLayout, RefTx};
use nurt_git::repo::GitRepo;
use nurt_git::repo::lock::{LockedWrite, LockedWrites, content_hash, record_object_writes};

use super::repo::{HeadState, full_name, io_error, join_slash};

/// Targets of the index protocol: the lock file and its rename.
const INDEX_TARGETS: usize = 2;

/// A working tree plan that changes nothing.
pub(crate) fn empty_plan() -> ApplyPlan {
    ApplyPlan {
        entries: Vec::new(),
        index: None,
        refs: Vec::new(),
        state_files: Vec::new(),
        carried: Vec::new(),
        notable: Vec::new(),
    }
}

/// Removal of the state files `names` of the repository directory that
/// exist, with the names removed.
pub(crate) fn state_removal(
    repo: &GitRepo,
    names: &[&str],
) -> Result<(Vec<StateFilePlan>, Vec<String>)> {
    let mut plans = Vec::new();
    let mut removed = Vec::new();
    for name in names {
        let path = repo.git_dir().join(name);
        match std::fs::read(&path) {
            Ok(data) => {
                plans.push(StateFilePlan {
                    path: (*name).to_owned(),
                    content: None,
                    expect: Expect::Hash(content_hash(&data)),
                });
                removed.push((*name).to_owned());
            }
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {}
            Err(cause) => return Err(io_error("cannot read repository file", cause)),
        }
    }
    Ok((plans, removed))
}

/// Write of a state file of the repository directory.
pub(crate) fn state_write(repo: &GitRepo, name: &str, content: Vec<u8>) -> Result<StateFilePlan> {
    let path = repo.git_dir().join(name);
    let expect = match std::fs::read(&path) {
        Ok(data) => Expect::Hash(content_hash(&data)),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Expect::Absent,
        Err(cause) => return Err(io_error("cannot read repository file", cause)),
    };
    Ok(StateFilePlan {
        path: name.to_owned(),
        content: Some(content),
        expect,
    })
}

/// Working tree paths a plan writes and removes, relative to the root.
pub(crate) fn touched(plan: &ApplyPlan) -> (Vec<String>, Vec<String>) {
    let mut written = Vec::new();
    let mut removed = Vec::new();
    for entry in &plan.entries {
        let path = entry.path.to_str_lossy().into_owned();
        match entry.action {
            EntryAction::Delete => removed.push(path),
            EntryAction::Create
            | EntryAction::Modify
            | EntryAction::Symlink
            | EntryAction::Chmod => written.push(path),
            EntryAction::TypeChange if entry.kind != Some(EntryKind::Directory) => {
                written.push(path);
            }
            EntryAction::TypeChange | EntryAction::Mkdir | EntryAction::Rmdir => {}
        }
    }
    (written, removed)
}

fn depth(path: &[u8]) -> usize {
    path.iter().filter(|byte| **byte == b'/').count()
}

fn parents(path: &[u8]) -> impl Iterator<Item = BString> + '_ {
    path.iter()
        .enumerate()
        .filter(|(_, byte)| **byte == b'/')
        .map(move |(at, _)| BString::from(&path[..at]))
}

fn is_emptied(root: &Path, dir: &BString, gone: &BTreeSet<BString>) -> bool {
    let Ok(reader) = std::fs::read_dir(join_slash(root, dir)) else {
        return false;
    };
    reader.into_iter().all(|child| {
        child.is_ok_and(|child| {
            gix::path::os_str_into_bstr(&child.file_name()).is_ok_and(|name| {
                let mut path = dir.clone();
                path.push(b'/');
                path.extend_from_slice(name);
                gone.contains(&path)
            })
        })
    })
}

/// Adds the removal of the untracked `files` to `plan`, followed by the
/// removal of the directories that are left empty.
pub(crate) fn remove_files(plan: &mut ApplyPlan, root: &Path, files: &[BString]) -> Result<()> {
    let mut gone: BTreeSet<BString> = plan
        .entries
        .iter()
        .filter(|entry| matches!(entry.action, EntryAction::Delete | EntryAction::Rmdir))
        .map(|entry| entry.path.clone())
        .collect();
    let mut deletes = Vec::new();
    for path in files {
        let native = join_slash(root, path);
        let is_link = std::fs::symlink_metadata(&native)
            .map_err(|cause| io_error("cannot read file", cause))?
            .file_type()
            .is_symlink();
        let expect = if is_link {
            Expect::Unchecked
        } else {
            let data =
                std::fs::read(&native).map_err(|cause| io_error("cannot read file", cause))?;
            Expect::Hash(content_hash(&data))
        };
        deletes.push(PlanEntry {
            path: path.clone(),
            action: EntryAction::Delete,
            expect,
            content: None,
            kind: None,
        });
        gone.insert(path.clone());
    }
    let kept: Vec<BString> = plan
        .entries
        .iter()
        .filter(|entry| !matches!(entry.action, EntryAction::Delete | EntryAction::Rmdir))
        .map(|entry| entry.path.clone())
        .collect();
    let mut dirs: Vec<BString> = files
        .iter()
        .flat_map(|path| parents(path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    dirs.sort_by_key(|dir| Reverse(depth(dir)));
    let mut rmdirs = Vec::new();
    for dir in dirs {
        let mut prefix = dir.clone();
        prefix.push(b'/');
        let in_use = kept.iter().any(|path| path.starts_with(prefix.as_slice()));
        if gone.contains(&dir) || in_use || !is_emptied(root, &dir, &gone) {
            continue;
        }
        gone.insert(dir.clone());
        rmdirs.push(PlanEntry {
            path: dir,
            action: EntryAction::Rmdir,
            expect: Expect::Unchecked,
            content: None,
            kind: None,
        });
    }
    let entries = std::mem::take(&mut plan.entries);
    let (old_deletes, rest): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .partition(|entry| entry.action == EntryAction::Delete);
    let (old_rmdirs, rest): (Vec<_>, Vec<_>) = rest
        .into_iter()
        .partition(|entry| entry.action == EntryAction::Rmdir);
    rmdirs.extend(old_rmdirs);
    rmdirs.sort_by_key(|entry| Reverse(depth(&entry.path)));
    plan.entries = old_deletes
        .into_iter()
        .chain(deletes)
        .chain(rmdirs)
        .chain(rest)
        .collect();
    Ok(())
}

/// Reference transaction of `repo` for entries written as `committer`.
pub(crate) fn ref_tx(repo: &GitRepo, committer: &Signature) -> Result<RefTx> {
    Ok(RefTx::new(
        RefLayout::from_repo(repo.gix())?,
        committer.clone(),
    ))
}

/// Moves the current branch, or a detached `HEAD`, to `target` and records
/// the previous commit in `ORIG_HEAD`.
pub(crate) fn move_head(
    refs: &mut RefTx,
    head: &HeadState,
    target: ObjectId,
    message: &str,
) -> Result<()> {
    refs.push(
        RefEdit::set(
            full_name("HEAD")?,
            Target::Object(target),
            PreviousValue::Any,
            message,
        )
        .deref(),
    );
    if let Some(old) = head.id {
        refs.push(RefEdit::set(
            full_name("ORIG_HEAD")?,
            Target::Object(old),
            PreviousValue::Any,
            message,
        ));
    }
    Ok(())
}

/// A planned change of one repository.
pub(crate) struct Change {
    /// Working tree, index and state files.
    pub(crate) plan: ApplyPlan,
    /// References and reflogs.
    pub(crate) refs: Option<PreparedRefTx>,
    /// Files replaced as a whole.
    pub(crate) writes: Vec<LockedWrite>,
    /// Objects were added to the object store.
    pub(crate) objects: bool,
}

impl Change {
    /// A change that does nothing yet.
    pub(crate) fn new() -> Self {
        Self {
            plan: empty_plan(),
            refs: None,
            writes: Vec::new(),
            objects: false,
        }
    }

    /// Prepares the reference edits of `refs`, reading the current values
    /// and checking the lock files.
    pub(crate) fn set_refs(&mut self, refs: RefTx) -> Result<()> {
        self.refs = if refs.edits().is_empty() {
            None
        } else {
            Some(refs.prepare()?)
        };
        Ok(())
    }

    /// Number of journal targets the change declares, approximately for
    /// directories created while writing.
    pub(crate) fn target_count(&self) -> usize {
        let index = if self.plan.index.is_some() {
            INDEX_TARGETS
        } else {
            0
        };
        self.plan.entries.len()
            + index
            + self.plan.state_files.len()
            + self.refs.as_ref().map_or(0, |refs| refs.targets().len())
            + self.writes.len()
    }

    /// Whether the change has no target.
    pub(crate) fn is_empty(&self) -> bool {
        self.target_count() == 0
    }

    /// Performs the change on `repo` through `tx`.
    ///
    /// Fails with `E_GIT_LOCKED` when a lock file of `git` appeared,
    /// `E_STALE` when a target changed after the plan and with the errors
    /// of the journal.
    pub(crate) fn run(self, repo: &GitRepo, tx: &mut dyn MutationTx) -> Result<()> {
        let Self {
            plan,
            mut refs,
            writes,
            objects,
        } = self;
        if objects {
            record_object_writes(tx, repo);
        }
        if let Some(refs) = refs.as_mut() {
            refs.declare(tx)?;
        }
        let writes = LockedWrites::new(writes);
        let first = if writes.writes().is_empty() {
            None
        } else {
            Some(writes.declare(tx)?)
        };
        if !plan.is_empty() {
            let ctx = ApplyContext::open(repo.gix())?;
            apply_plan(tx, &ctx, plan)?;
        }
        if let Some(first) = first {
            writes.apply(tx, first)?;
        }
        if let Some(refs) = refs {
            refs.apply(tx)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, action: EntryAction, kind: Option<EntryKind>) -> PlanEntry {
        PlanEntry {
            path: BString::from(path),
            action,
            expect: Expect::Unchecked,
            content: None,
            kind,
        }
    }

    #[test]
    fn touched_separates_writes_from_removals() {
        let mut plan = empty_plan();
        plan.entries = vec![
            entry("gone.txt", EntryAction::Delete, None),
            entry("dir", EntryAction::Rmdir, None),
            entry("new", EntryAction::Mkdir, Some(EntryKind::Directory)),
            entry("new/a.txt", EntryAction::Create, Some(EntryKind::File)),
            entry(
                "was-file",
                EntryAction::TypeChange,
                Some(EntryKind::Directory),
            ),
            entry("link", EntryAction::TypeChange, Some(EntryKind::Symlink)),
            entry("run.sh", EntryAction::Chmod, Some(EntryKind::Executable)),
        ];
        let (written, removed) = touched(&plan);
        assert_eq!(written, vec!["new/a.txt", "link", "run.sh"]);
        assert_eq!(removed, vec!["gone.txt"]);
        assert!(!plan.is_empty());
        assert!(empty_plan().is_empty());
    }

    #[test]
    fn removed_files_leave_no_empty_directory_behind() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/b/u.txt"), "u").unwrap();
        std::fs::write(dir.path().join("a/keep.txt"), "k").unwrap();
        let mut plan = empty_plan();
        plan.entries = vec![entry("x.txt", EntryAction::Delete, None)];
        remove_files(&mut plan, dir.path(), &[BString::from("a/b/u.txt")]).unwrap();
        let steps: Vec<(String, EntryAction)> = plan
            .entries
            .iter()
            .map(|entry| (entry.path.to_string(), entry.action))
            .collect();
        assert_eq!(
            steps,
            vec![
                ("x.txt".to_owned(), EntryAction::Delete),
                ("a/b/u.txt".to_owned(), EntryAction::Delete),
                ("a/b".to_owned(), EntryAction::Rmdir),
            ]
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
