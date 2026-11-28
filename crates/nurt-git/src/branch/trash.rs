// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::branch::trash
// Responsibility:   Preserved copies of deleted branches under `refs/nurt/trash/`.
// Dependencies:     bstr, gix_hash, gix_ref, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Preserved copies of deleted branches under `refs/nurt/trash/`.

use bstr::ByteSlice;
use gix_hash::ObjectId;
use gix_ref::{FullName, Target};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{GitTrashEntry, GitTrashKind};
use nurt_contracts::value::Instant;

use super::{BranchPlan, TRASH, commit_record, quoted_repo, read_error, short_name};
use crate::refs_tx::{PreviousValue, RefEdit, RefLayout, RefTx};

/// Preserved copies of deleted branches as `git_trash_entry` records, in the
/// order of their references `refs/nurt/trash/<name>-<seconds>`.
///
/// # Errors
///
/// `E_IO` when the references or objects cannot be read.
pub fn list_trash(repo: &gix::Repository) -> Result<Vec<GitTrashEntry>> {
    trash_refs(repo)?
        .into_iter()
        .map(|(full, id)| entry(repo, &full, id))
        .collect()
}

/// Plans the removal of preserved copies: those named by `names`, or all of
/// them when `names` is `None`.
///
/// A name selects a copy by its reference (`refs/nurt/trash/<name>-<seconds>`),
/// by the last component of the reference (`<name>-<seconds>`), or every copy
/// of a deleted branch by the branch name.
///
/// # Errors
///
/// `E_GIT_REF` with problem `missing` for a name without a copy, and the
/// errors of [`RefTx::prepare`].
pub fn plan_purge(
    repo: &gix::Repository,
    names: Option<&[&str]>,
    committer: gix_actor::Signature,
) -> Result<BranchPlan> {
    let kept = trash_refs(repo)?;
    let selected: Vec<(FullName, ObjectId)> = match names {
        None => kept,
        Some(names) => {
            for name in names {
                let exact = kept.iter().any(|(full, _)| is_copy(full, name));
                let of_branch = kept
                    .iter()
                    .any(|(full, _)| !is_copy(full, name) && is_branch_copy(full, name));
                if !exact && !of_branch {
                    return Err(missing(repo, name));
                }
                if exact && of_branch {
                    return Err(ambiguous(repo, name, &kept));
                }
            }
            kept.into_iter()
                .filter(|(full, _)| names.iter().any(|name| names_copy(full, name)))
                .collect()
        }
    };
    let mut refs = RefTx::new(RefLayout::from_repo(repo)?, committer);
    for (full, id) in selected {
        refs.push(RefEdit::delete(
            full,
            PreviousValue::MustExistAndMatch(Target::Object(id)),
            "",
        ));
    }
    Ok(BranchPlan::from_refs(refs.prepare()?))
}

/// Whether `name` selects the copy kept as `full`.
fn names_copy(full: &FullName, name: &str) -> bool {
    is_copy(full, name) || is_branch_copy(full, name)
}

/// Whether `name` is the reference or the last component of the reference of
/// the copy kept as `full`.
fn is_copy(full: &FullName, name: &str) -> bool {
    full.as_bstr() == name.as_bytes() || short_name(full.as_bstr(), TRASH) == name
}

/// Whether the copy kept as `full` is a copy of the branch `name`.
fn is_branch_copy(full: &FullName, name: &str) -> bool {
    split_copy_name(&short_name(full.as_bstr(), TRASH)).0 == name
}

/// Record of the copy kept as `full`.
fn entry(repo: &gix::Repository, full: &FullName, id: ObjectId) -> Result<GitTrashEntry> {
    let copy = short_name(full.as_bstr(), TRASH);
    let (name, seconds) = split_copy_name(&copy);
    let commit = commit_record(repo, &copy, id, false)?;
    let time = seconds
        .and_then(Instant::from_unix_seconds)
        .or(commit.time)
        .unwrap_or_default();
    Ok(GitTrashEntry {
        name: name.to_owned(),
        kind: GitTrashKind::Branch,
        reference: full.as_bstr().to_str_lossy().into_owned(),
        commit: commit.commit,
        time,
        subject: commit.subject,
    })
}

/// Branch name and deletion time of a copy named `<name>-<seconds>`; a name
/// without a numeric suffix is returned whole.
fn split_copy_name(copy: &str) -> (&str, Option<i64>) {
    match copy.rsplit_once('-') {
        Some((name, seconds)) if !name.is_empty() => match seconds.parse::<i64>() {
            Ok(seconds) => (name, Some(seconds)),
            Err(_) => (copy, None),
        },
        _ => (copy, None),
    }
}

/// References below [`TRASH`] with their values, in name order.
fn trash_refs(repo: &gix::Repository) -> Result<Vec<(FullName, ObjectId)>> {
    let platform = repo.references().map_err(read_error)?;
    let mut out = Vec::new();
    for reference in platform.all().map_err(read_error)? {
        let reference = reference.map_err(read_error)?;
        if !reference.name().as_bstr().starts_with(TRASH.as_bytes()) {
            continue;
        }
        if let Some(id) = reference.target().try_id() {
            out.push((reference.name().to_owned(), id.to_owned()));
        }
    }
    out.sort_by(|a, b| a.0.as_bstr().cmp(b.0.as_bstr()));
    Ok(out)
}

/// A name that is the copy of one branch and the branch name of other
/// copies, such as `a-1`: the copy `a-1` and the copies `a-1-<seconds>`.
fn ambiguous(repo: &gix::Repository, name: &str, kept: &[(FullName, ObjectId)]) -> Error {
    let references: Vec<String> = kept
        .iter()
        .filter(|(full, _)| names_copy(full, name))
        .map(|(full, _)| full.as_bstr().to_str_lossy().into_owned())
        .collect();
    Error::new(
        ErrorCode::GitRef,
        format!(
            "{name} names a preserved copy and the copies of a branch; purge the references {}",
            references.join(", ")
        ),
    )
    .with_problem("ambiguous")
    .with_found(name.to_owned())
    .with_next_step(format!("git.branch {} trash: true", quoted_repo(repo)))
}

fn missing(repo: &gix::Repository, name: &str) -> Error {
    Error::new(
        ErrorCode::GitRef,
        format!("{name} is not a preserved branch; list them with trash: true"),
    )
    .with_problem("missing")
    .with_found(name.to_owned())
    .with_next_step(format!("git.branch {} trash: true", quoted_repo(repo)))
}

#[cfg(test)]
mod tests {
    use super::super::test_repo::TestRepo;
    use super::*;

    #[test]
    fn copies_are_listed_as_trash_entries() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        repo.write("refs/nurt/trash/old-100", &format!("{head}\n"));
        repo.write("refs/nurt/trash/fix-1-50", &format!("{head}\n"));
        let kept = list_trash(&repo.open()).unwrap();
        let names: Vec<&str> = kept.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["fix-1", "old"]);
        assert_eq!(kept[1].reference, "refs/nurt/trash/old-100");
        assert_eq!(kept[1].kind, GitTrashKind::Branch);
        assert_eq!(kept[1].commit, head.to_string());
        assert_eq!(kept[1].time, Instant::from_unix_seconds(100).unwrap());
        assert_eq!(kept[1].subject.as_deref(), Some("first"));
    }

    #[test]
    fn copies_are_purged_by_reference_copy_name_branch_name_or_all() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        for copy in ["a-1", "a-2", "b-3", "c-4"] {
            repo.write(&format!("refs/nurt/trash/{copy}"), &format!("{head}\n"));
        }
        let error = plan_purge(&repo.open(), Some(&["none-1"]), repo.signature()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("missing"));
        let plan = plan_purge(&repo.open(), Some(&["a", "a-1"]), repo.signature()).unwrap();
        assert_eq!(plan.outcomes().len(), 2);
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        assert!(!repo.path("refs/nurt/trash/a-1").exists());
        assert!(!repo.path("refs/nurt/trash/a-2").exists());
        let plan = plan_purge(
            &repo.open(),
            Some(&["refs/nurt/trash/b-3"]),
            repo.signature(),
        )
        .unwrap();
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        let plan = plan_purge(&repo.open(), None, repo.signature()).unwrap();
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        assert!(list_trash(&repo.open()).unwrap().is_empty());
    }

    #[test]
    fn a_name_of_a_copy_and_of_a_branch_is_ambiguous() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        for copy in ["a-1", "a-1-5"] {
            repo.write(&format!("refs/nurt/trash/{copy}"), &format!("{head}\n"));
        }
        let error = plan_purge(&repo.open(), Some(&["a-1"]), repo.signature()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("ambiguous"));
        assert!(
            error.message.contains("refs/nurt/trash/a-1-5"),
            "{}",
            error.message
        );
        let plan = plan_purge(
            &repo.open(),
            Some(&["refs/nurt/trash/a-1"]),
            repo.signature(),
        )
        .unwrap();
        assert_eq!(plan.outcomes().len(), 1);
    }

    #[test]
    fn copy_names_split_at_the_last_numeric_suffix() {
        assert_eq!(split_copy_name("topic-17"), ("topic", Some(17)));
        assert_eq!(split_copy_name("fix-a-17"), ("fix-a", Some(17)));
        assert_eq!(split_copy_name("plain"), ("plain", None));
        assert_eq!(split_copy_name("x-y"), ("x-y", None));
        assert_eq!(split_copy_name("-5"), ("-5", None));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
