// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Creating and checking out a branch with `git.branch create: … switch: true`.

use std::path::Path;

use bstr::ByteSlice;
use gix_hash::ObjectId;
use gix_ref::FullName;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::ChangeAction;
use nurt_contracts::report::Change;

use super::{HEADS, read_error};
use crate::apply::{
    ApplyContext, ApplyPlan, EntryAction, RefPlan, RefTarget, ReflogLine, apply_plan, plan_two_way,
};

/// Changes of the working tree, the index, the new branch and `HEAD` that
/// `git switch -c` makes; local changes that do not conflict are carried
/// over.
///
/// The branch is written by the same plan as `HEAD`, before it, so that the
/// reflog directories both need are declared once.
#[derive(Debug)]
pub(crate) struct SwitchPart {
    plan: ApplyPlan,
    branch: FullName,
}

impl SwitchPart {
    /// Plans the creation of `branch` at `commit`, named `from` in the reflog
    /// entry `branch: Created from <from>`, and its checkout.
    ///
    /// # Errors
    ///
    /// `E_GIT_DIRTY` when local changes would be overwritten, `E_GIT_CONFLICT`
    /// when the index has conflicts, and `E_IO` when the repository cannot be
    /// read.
    pub(crate) fn plan(
        repo: &gix::Repository,
        branch: &FullName,
        commit: ObjectId,
        from: &str,
        committer: &gix_actor::Signature,
    ) -> Result<Self> {
        let ctx = ApplyContext::open(repo)?;
        let head_before = repo.head_id().ok().map(gix::Id::detach);
        let head_tree = head_before.map(|id| tree_of(repo, id)).transpose()?;
        let target_tree = tree_of(repo, commit)?;
        let mut plan = plan_two_way(&ctx, head_tree.as_ref().map(|id| id.as_ref()), &target_tree)?;
        let previous = match repo.head_name().map_err(read_error)? {
            Some(name) => name.shorten().to_str_lossy().into_owned(),
            None => head_before.map_or_else(|| "HEAD".to_owned(), |id| id.to_string()),
        };
        let full = branch.as_bstr().to_str_lossy().into_owned();
        let short = branch
            .as_bstr()
            .strip_prefix(HEADS.as_bytes())
            .unwrap_or(branch.as_bstr())
            .to_str_lossy()
            .into_owned();
        let null = ObjectId::null(repo.object_hash());
        plan.refs.push(RefPlan::new(
            repo,
            &full,
            RefTarget::Object(commit),
            Some(ReflogLine {
                old: null,
                new: commit,
                committer: committer.clone(),
                message: format!("branch: Created from {from}"),
            }),
        )?);
        plan.refs.push(RefPlan::new(
            repo,
            "HEAD",
            RefTarget::Symbolic(full),
            Some(ReflogLine {
                old: head_before.unwrap_or(null),
                new: commit,
                committer: committer.clone(),
                message: format!("checkout: moving from {previous} to {short}"),
            }),
        )?);
        Ok(Self {
            plan,
            branch: branch.clone(),
        })
    }

    /// Whether local changes are carried over to the branch.
    pub(crate) fn carries_changes(&self) -> bool {
        !self.plan.carried.is_empty()
    }

    /// Planned changes of files below the working tree `root`, of the index,
    /// of the new branch and of `HEAD` in the repository directory `git_dir`.
    pub(crate) fn changes(&self, root: &Path, git_dir: &Path) -> Vec<Change> {
        let mut out: Vec<Change> = self
            .plan
            .entries
            .iter()
            .filter_map(|entry| {
                let action = match entry.action {
                    EntryAction::Create | EntryAction::Mkdir | EntryAction::Symlink => {
                        ChangeAction::Create
                    }
                    EntryAction::Modify | EntryAction::Chmod | EntryAction::TypeChange => {
                        ChangeAction::Modify
                    }
                    EntryAction::Delete | EntryAction::Rmdir => ChangeAction::Delete,
                };
                let relative = gix_path::from_bstr(entry.path.as_bstr()).ok()?;
                Some(Change::new(
                    root.join(&*relative).to_string_lossy().into_owned(),
                    action,
                ))
            })
            .collect();
        if self.plan.index.is_some() {
            out.push(Change::new(
                git_dir.join("index").to_string_lossy().into_owned(),
                ChangeAction::Modify,
            ));
        }
        if let Ok(relative) = gix_path::from_bstr(self.branch.as_bstr()) {
            out.push(Change::new(
                git_dir.join(&*relative).to_string_lossy().into_owned(),
                ChangeAction::Create,
            ));
        }
        out.push(Change::new(
            git_dir.join("HEAD").to_string_lossy().into_owned(),
            ChangeAction::Modify,
        ));
        out
    }

    /// Creates the branch and checks it out through `tx`.
    ///
    /// # Errors
    ///
    /// The errors of [`apply_plan`]: `E_GIT_LOCKED`, `E_STALE` and the
    /// errors of the transaction.
    pub(crate) fn apply(self, repo: &gix::Repository, tx: &mut dyn MutationTx) -> Result<()> {
        let ctx = ApplyContext::open(repo)?;
        apply_plan(tx, &ctx, self.plan)?;
        Ok(())
    }
}

fn tree_of(repo: &gix::Repository, commit: ObjectId) -> Result<ObjectId> {
    let commit = repo
        .find_object(commit)
        .map_err(read_error)?
        .try_into_commit()
        .map_err(|error| {
            Error::new(
                ErrorCode::GitRevision,
                format!("{commit} is not a commit: {error}; name a commit to switch to"),
            )
        })?;
    Ok(commit.tree_id().map_err(read_error)?.detach())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::RepoBuilder;

    fn committer() -> gix_actor::Signature {
        gix_actor::Signature {
            name: "Test".into(),
            email: "test@example.com".into(),
            time: gix_date::Time {
                seconds: 1_767_225_600,
                offset: 0,
            },
        }
    }

    #[test]
    fn changes_list_a_file_replaced_by_a_directory_as_modified() {
        let fixture = RepoBuilder::new()
            .file("p", "file\n")
            .commit("one")
            .remove("p")
            .file("p/inner", "inner\n")
            .commit("two")
            .build();
        let repo = fixture.open();
        let first = repo.rev_parse_single("HEAD~1").unwrap().detach();
        let branch = FullName::try_from("refs/heads/topic").unwrap();
        let part = SwitchPart::plan(&repo, &branch, first, "HEAD~1", &committer()).unwrap();
        let changes = part.changes(fixture.path(), &fixture.git_dir());
        let action_of = |suffix: &str| {
            changes
                .iter()
                .find(|change| change.path.as_str().ends_with(suffix))
                .map(|change| change.action)
        };
        assert_eq!(action_of("repo/p"), Some(ChangeAction::Modify));
        assert_eq!(action_of("refs/heads/topic"), Some(ChangeAction::Create));
        assert_eq!(action_of("HEAD"), Some(ChangeAction::Modify));
        assert!(part.plan.entries.iter().any(|entry| {
            entry.path == "p" && entry.action == EntryAction::TypeChange
        }));
    }
}
