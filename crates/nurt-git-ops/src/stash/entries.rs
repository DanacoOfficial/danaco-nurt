// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The list of stash entries: `refs/stash` and its reflog.

use gix::bstr::ByteSlice;
use gix::hash::ObjectId;
use gix::index::State;
use nurt_contracts::error::{ErrorCode, Result};

use crate::reset::index;
use crate::reset::refs::{
    ReflogEntry, plan_ref_delete, plan_ref_write, plan_reflog_rewrite, read_ref, read_reflog,
};
use crate::reset::repo::{Repo, error, quote};
use crate::reset::txplan::TxPlan;

/// Name of the stash reference.
pub(crate) const STASH_REF: &str = "refs/stash";

/// One stash entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    /// Position, 0 for the newest.
    pub index: usize,
    /// Commit of the entry.
    pub id: ObjectId,
    /// Message, as in the reflog.
    pub message: String,
    /// Time of the entry in seconds since the Unix epoch.
    pub time: i64,
    /// The entry includes untracked files.
    pub includes_untracked: bool,
    /// Branch the entry was made on; `None` for a detached `HEAD`.
    pub branch: Option<String>,
}

/// Branch named in a stash message (`WIP on <b>: …` or `On <b>: …`).
pub(crate) fn branch_of(message: &str) -> Option<String> {
    let rest = message
        .strip_prefix("WIP on ")
        .or_else(|| message.strip_prefix("On "))?;
    let (branch, _) = rest.split_once(": ")?;
    (branch != "(no branch)").then(|| branch.to_owned())
}

/// Reflog entries of the stash, oldest first; a stash reference without a
/// reflog counts as one entry.
pub(crate) fn reflog(repo: &Repo) -> Result<Vec<ReflogEntry>> {
    let entries = read_reflog(repo, STASH_REF)?;
    if !entries.is_empty() {
        return Ok(entries);
    }
    Ok(read_ref(repo, STASH_REF)?
        .map(|id| ReflogEntry {
            old: ObjectId::null(repo.repo.object_hash()),
            new: id,
            rest: Default::default(),
        })
        .into_iter()
        .collect())
}

/// Stash entries, newest first.
pub(crate) fn list(repo: &Repo) -> Result<Vec<StashEntry>> {
    let entries = reflog(repo)?;
    entries
        .iter()
        .rev()
        .enumerate()
        .map(|(position, entry)| {
            let message = entry.message();
            let parents = repo.commit_parents(entry.new)?;
            Ok(StashEntry {
                index: position,
                id: entry.new,
                time: entry.seconds().unwrap_or_default(),
                includes_untracked: parents.len() > 2,
                branch: branch_of(&message),
                message,
            })
        })
        .collect()
}

/// Position named by `spec`: `stash@{N}`, `N` or nothing for the newest.
fn position_of(spec: Option<&str>) -> Option<usize> {
    let spec = spec.unwrap_or("stash@{0}");
    let digits = spec
        .strip_prefix("stash@{")
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap_or(spec);
    digits.parse().ok()
}

/// Entry named by `spec` with its position in the list; the position is
/// `None` for a commit that is not in the list.
pub(crate) fn resolve(repo: &Repo, spec: Option<&str>) -> Result<(Option<usize>, ObjectId)> {
    let root = quote(&repo.workdir.to_string_lossy());
    let Some(position) = position_of(spec) else {
        let text = spec.unwrap_or_default();
        return Ok((None, repo.resolve_commit(text)?));
    };
    let entries = reflog(repo)?;
    if entries.is_empty() {
        return Err(error(ErrorCode::GitRef, "no stash entries found")
            .with_problem("no_stash")
            .with_fix(format!("git.stash {root} action: push")));
    }
    let Some(entry) = entries.len().checked_sub(position + 1).and_then(|at| entries.get(at)) else {
        return Err(error(
            ErrorCode::GitRevision,
            format!(
                "stash@{{{position}}} does not exist; the stash has {} entries",
                entries.len()
            ),
        )
        .with_problem("out_of_range")
        .with_fix(format!("git.stash {root} action: list")));
    };
    Ok((Some(position), entry.new))
}

/// Parents of a stash commit: base, index and untracked commits.
pub(crate) struct StashParts {
    /// Commit the entry was made on.
    pub(crate) base: ObjectId,
    /// Commit of the index.
    pub(crate) index: ObjectId,
    /// Commit of the untracked files.
    pub(crate) untracked: Option<ObjectId>,
}

/// Parents of `id`, which must be a stash commit.
pub(crate) fn parts(repo: &Repo, id: ObjectId) -> Result<StashParts> {
    let parents = repo.commit_parents(id)?;
    match parents.as_slice() {
        [base, index] => Ok(StashParts {
            base: *base,
            index: *index,
            untracked: None,
        }),
        [base, index, untracked] => Ok(StashParts {
            base: *base,
            index: *index,
            untracked: Some(*untracked),
        }),
        _ => Err(error(
            ErrorCode::GitRevision,
            format!("{} is not a stash commit", id.to_hex()),
        )
        .with_problem("not_a_stash")),
    }
}

/// Plans removing the entry at `position`, rewriting the reflog like
/// `git reflog delete --rewrite --updateref`.
pub(crate) fn plan_drop(tx: &mut TxPlan, repo: &Repo, position: usize) -> Result<ObjectId> {
    let mut entries = reflog(repo)?;
    let at = entries
        .len()
        .checked_sub(position + 1)
        .ok_or_else(|| error(ErrorCode::GitRevision, format!("stash@{{{position}}} does not exist")))?;
    let removed = entries.remove(at);
    if entries.is_empty() {
        plan_ref_delete(tx, repo, STASH_REF)?;
        return Ok(removed.new);
    }
    if let Some(next) = entries.get_mut(at) {
        next.old = removed.old;
    }
    plan_reflog_rewrite(tx, repo, STASH_REF, &entries)?;
    if position == 0
        && let Some(newest) = entries.last()
    {
        plan_ref_write(tx, repo, STASH_REF, newest.new)?;
    }
    Ok(removed.new)
}

/// Plans removing all entries.
pub(crate) fn plan_clear(tx: &mut TxPlan, repo: &Repo) -> Result<usize> {
    let count = reflog(repo)?.len();
    plan_ref_delete(tx, repo, STASH_REF)?;
    Ok(count)
}

/// Change of one path recorded in a stash entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashFile {
    /// Path relative to the root of the working tree.
    pub path: String,
    /// What the entry does to the path.
    pub change: StashFileChange,
}

/// Kind of change of a [`StashFile`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StashFileChange {
    /// The path is added.
    Added,
    /// The content or mode changes.
    Modified,
    /// The path is removed.
    Deleted,
    /// An untracked file is restored.
    Untracked,
}

fn differences(old: &State, new: &State, out: &mut Vec<StashFile>) {
    for entry in new.entries() {
        let path = entry.path(new);
        let change = match index::merged_entry(old, path) {
            None => Some(StashFileChange::Added),
            Some(previous) if previous.id != entry.id || previous.mode != entry.mode => {
                Some(StashFileChange::Modified)
            }
            Some(_) => None,
        };
        if let Some(change) = change {
            out.push(StashFile {
                path: path.to_str_lossy().into_owned(),
                change,
            });
        }
    }
    for entry in old.entries() {
        let path = entry.path(old);
        if index::merged_entry(new, path).is_none() {
            out.push(StashFile {
                path: path.to_str_lossy().into_owned(),
                change: StashFileChange::Deleted,
            });
        }
    }
}

/// Changes recorded in the entry `id`, sorted by path.
pub(crate) fn files(repo: &Repo, id: ObjectId) -> Result<Vec<StashFile>> {
    let parts = parts(repo, id)?;
    let base = index::from_tree(repo, repo.commit_tree(parts.base)?)?;
    let work = index::from_tree(repo, repo.commit_tree(id)?)?;
    let mut out = Vec::new();
    differences(&base, &work, &mut out);
    if let Some(untracked) = parts.untracked {
        let state = index::from_tree(repo, repo.commit_tree(untracked)?)?;
        out.extend(state.entries().iter().map(|entry| StashFile {
            path: entry.path(&state).to_str_lossy().into_owned(),
            change: StashFileChange::Untracked,
        }));
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_is_read_from_both_message_forms() {
        assert_eq!(branch_of("WIP on main: 1234567 subject").as_deref(), Some("main"));
        assert_eq!(branch_of("On topic/x: message").as_deref(), Some("topic/x"));
        assert_eq!(branch_of("WIP on (no branch): 1234567 s"), None);
        assert_eq!(branch_of("something else"), None);
    }

    #[test]
    fn positions_accept_reflog_and_number_forms() {
        assert_eq!(position_of(None), Some(0));
        assert_eq!(position_of(Some("stash@{2}")), Some(2));
        assert_eq!(position_of(Some("3")), Some(3));
        assert_eq!(position_of(Some("main~1")), None);
    }
}
