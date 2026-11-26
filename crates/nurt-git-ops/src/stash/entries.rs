// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The list of stash entries: `refs/stash` and its reflog.

use gix::bstr::{BString, ByteSlice};
use gix::hash::ObjectId;
use gix::index::State;
use gix::refs::Target;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_git::apply::{log_path, ref_path};
use nurt_git::refs_tx::{PreviousValue, RefEdit, RefTx};
use nurt_git::repo::GitRepo;
use nurt_git::repo::lock::LockedWrite;

use crate::reset::index;
use crate::reset::repo::{commit_parents, commit_tree, error, full_name, io_error, root_literal};

/// Name of the stash reference.
pub(crate) const STASH_REF: &str = "refs/stash";

/// One entry of a reflog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReflogEntry {
    /// Value before the update.
    pub(crate) old: ObjectId,
    /// Value after the update.
    pub(crate) new: ObjectId,
    /// Everything after the two object names, without the line end:
    /// identity, time, tab and message, kept byte for byte.
    pub(crate) rest: BString,
}

impl ReflogEntry {
    /// Message of the entry.
    pub(crate) fn message(&self) -> String {
        self.rest
            .find_byte(b'\t')
            .map(|tab| self.rest[tab + 1..].to_str_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Time of the entry in seconds since the Unix epoch.
    pub(crate) fn seconds(&self) -> Option<i64> {
        let head = self.rest.split_str(b"\t").next()?;
        let head = head.to_str().ok()?;
        let after_mail = head.rsplit_once("> ")?.1;
        after_mail.split(' ').next()?.parse().ok()
    }

    fn to_line(&self) -> Vec<u8> {
        let mut line = Vec::with_capacity(self.rest.len() + 2 * 41 + 1);
        line.extend_from_slice(self.old.to_hex().to_string().as_bytes());
        line.push(b' ');
        line.extend_from_slice(self.new.to_hex().to_string().as_bytes());
        line.push(b' ');
        line.extend_from_slice(&self.rest);
        line.push(b'\n');
        line
    }
}

/// Parses a reflog file; lines that do not parse are skipped like git does.
fn parse_reflog(data: &[u8]) -> Vec<ReflogEntry> {
    data.lines()
        .filter_map(|line| {
            let mut parts = line.splitn_str(3, b" ");
            let old = ObjectId::from_hex(parts.next()?).ok()?;
            let new = ObjectId::from_hex(parts.next()?).ok()?;
            let rest = BString::from(parts.next()?);
            Some(ReflogEntry { old, new, rest })
        })
        .collect()
}

/// Entries of the reflog of the stash, oldest first.
fn read_reflog(repo: &GitRepo) -> Result<Vec<ReflogEntry>> {
    let path = log_path(repo.gix(), STASH_REF)?;
    match std::fs::read(&path) {
        Ok(data) => Ok(parse_reflog(&data)),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(cause) => Err(io_error("cannot read reflog", cause)),
    }
}

/// Current value of a reference, `None` when it does not exist.
pub(crate) fn read_ref(repo: &gix::Repository, name: &str) -> Result<Option<ObjectId>> {
    let reference = repo
        .try_find_reference(name)
        .map_err(|cause| io_error("cannot read reference", cause))?;
    Ok(reference.and_then(|reference| reference.target().try_id().map(ToOwned::to_owned)))
}

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
pub(crate) fn reflog(repo: &GitRepo) -> Result<Vec<ReflogEntry>> {
    let entries = read_reflog(repo)?;
    if !entries.is_empty() {
        return Ok(entries);
    }
    Ok(read_ref(repo.gix(), STASH_REF)?
        .map(|id| ReflogEntry {
            old: ObjectId::null(repo.gix().object_hash()),
            new: id,
            rest: Default::default(),
        })
        .into_iter()
        .collect())
}

/// Stash entries, newest first.
pub(crate) fn list(repo: &GitRepo) -> Result<Vec<StashEntry>> {
    let entries = reflog(repo)?;
    entries
        .iter()
        .rev()
        .enumerate()
        .map(|(position, entry)| {
            let message = entry.message();
            let parents = commit_parents(repo, entry.new)?;
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
pub(crate) fn resolve(repo: &GitRepo, spec: Option<&str>) -> Result<(Option<usize>, ObjectId)> {
    let root = root_literal(repo);
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
    let Some(entry) = position
        .checked_add(1)
        .and_then(|count| entries.len().checked_sub(count))
        .and_then(|at| entries.get(at))
    else {
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

/// Parents of a stash commit: base and untracked commits.
pub(crate) struct StashParts {
    /// Commit the entry was made on.
    pub(crate) base: ObjectId,
    /// Commit of the untracked files.
    pub(crate) untracked: Option<ObjectId>,
}

/// Parents of `id`, which must be a stash commit.
pub(crate) fn parts(repo: &GitRepo, id: ObjectId) -> Result<StashParts> {
    let parents = commit_parents(repo, id)?;
    match parents.as_slice() {
        [base, _index] => Ok(StashParts {
            base: *base,
            untracked: None,
        }),
        [base, _index, untracked] => Ok(StashParts {
            base: *base,
            untracked: Some(*untracked),
        }),
        _ => Err(error(
            ErrorCode::GitRevision,
            format!("{} is not a stash commit", id.to_hex()),
        )
        .with_problem("not_a_stash")),
    }
}

/// Deletes `refs/stash` with its reflog.
fn delete_stash(refs: &mut RefTx) -> Result<()> {
    refs.push(RefEdit::delete(
        full_name(STASH_REF)?,
        PreviousValue::Any,
        "",
    ));
    Ok(())
}

/// Plans removing the entry at `position`, rewriting the reflog like
/// `git reflog delete --rewrite --updateref`.
pub(crate) fn plan_drop(
    repo: &GitRepo,
    refs: &mut RefTx,
    writes: &mut Vec<LockedWrite>,
    position: usize,
) -> Result<ObjectId> {
    let mut entries = reflog(repo)?;
    let at = position
        .checked_add(1)
        .and_then(|count| entries.len().checked_sub(count))
        .ok_or_else(|| {
            error(
                ErrorCode::GitRevision,
                format!("stash@{{{position}}} does not exist"),
            )
        })?;
    let removed = entries.remove(at);
    if entries.is_empty() {
        delete_stash(refs)?;
        return Ok(removed.new);
    }
    if let Some(next) = entries.get_mut(at) {
        next.old = removed.old;
    }
    let log: Vec<u8> = entries.iter().flat_map(ReflogEntry::to_line).collect();
    writes.push(LockedWrite::new(log_path(repo.gix(), STASH_REF)?, log));
    if position == 0
        && let Some(newest) = entries.last()
    {
        writes.push(LockedWrite::new(
            ref_path(repo.gix(), STASH_REF)?,
            format!("{}\n", newest.new.to_hex()).into_bytes(),
        ));
    }
    Ok(removed.new)
}

/// Plans removing all entries.
pub(crate) fn plan_clear(repo: &GitRepo, refs: &mut RefTx) -> Result<usize> {
    let count = reflog(repo)?.len();
    if count > 0 {
        delete_stash(refs)?;
    }
    Ok(count)
}

/// Plans creating the entry `commit` on top of the stash.
pub(crate) fn plan_create(refs: &mut RefTx, commit: ObjectId, message: &str) -> Result<()> {
    refs.push(
        RefEdit::set(
            full_name(STASH_REF)?,
            Target::Object(commit),
            PreviousValue::Any,
            message,
        )
        .create_reflog(),
    );
    Ok(())
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
pub(crate) fn files(repo: &GitRepo, id: ObjectId) -> Result<Vec<StashFile>> {
    let parts = parts(repo, id)?;
    let base = index::from_tree(repo, commit_tree(repo, parts.base)?)?;
    let work = index::from_tree(repo, commit_tree(repo, id)?)?;
    let mut out = Vec::new();
    differences(&base, &work, &mut out);
    if let Some(untracked) = parts.untracked {
        let state = index::from_tree(repo, commit_tree(repo, untracked)?)?;
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
        assert_eq!(
            branch_of("WIP on main: 1234567 subject").as_deref(),
            Some("main")
        );
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
