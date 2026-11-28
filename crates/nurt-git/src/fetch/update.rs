// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::fetch::update
// Responsibility:   Selection of the reference updates of a fetch, as `git fetch` decides them.
// Dependencies:     bstr, gix, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Selection of the reference updates of a fetch, as `git fetch` decides
//! them: which remote references are stored, which tags are followed, how
//! every local reference changes and which ones are pruned.

use std::collections::HashSet;

use bstr::ByteSlice;
use gix::ObjectId;
use gix::protocol::fetch::refmap::{Source, SpecIndex};
use gix::protocol::fetch::{RefMap, Tags};
use gix::protocol::handshake::Ref;
use gix::refspec::RefSpec;
use gix::refspec::instruction::Fetch;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::GitRefStatus;

use super::error::read_error;
use super::head::{self, Entry};
use super::remote::Resolved;
use super::{COMMAND, RefChange};
use crate::repo::GitRepo;
use crate::repo::error::quote_path;

/// Planned updates and the heads of `FETCH_HEAD`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Selection {
    /// Changes of local references: pruned references first, then the
    /// fetched ones in the order of the refspecs.
    pub(crate) changes: Vec<RefChange>,
    /// Lines of `FETCH_HEAD`, with the heads to merge marked.
    pub(crate) heads: Vec<Entry>,
}

/// A remote reference with the identifiers it advertises.
#[derive(Debug, Clone)]
pub(crate) struct RemoteRef {
    /// Full name on the remote.
    pub(crate) name: String,
    /// Object the reference points to: a tag object for annotated tags.
    pub(crate) id: ObjectId,
    /// Object after peeling tags.
    pub(crate) peeled: ObjectId,
}

/// Name and identifiers of the remote side of a mapping; `None` for an
/// unborn branch.
pub(crate) fn remote_ref(source: &Source) -> Option<RemoteRef> {
    match source {
        Source::ObjectId(id) => Some(RemoteRef {
            name: id.to_string(),
            id: *id,
            peeled: *id,
        }),
        Source::Ref(reference) => advertised(reference),
    }
}

/// Identifiers of an advertised reference; `None` for an unborn branch.
pub(crate) fn advertised(reference: &Ref) -> Option<RemoteRef> {
    let (name, id, peeled) = match reference {
        Ref::Direct {
            full_ref_name,
            object,
        } => (full_ref_name, *object, *object),
        Ref::Peeled {
            full_ref_name,
            tag,
            object,
        } => (full_ref_name, *tag, *object),
        Ref::Symbolic {
            full_ref_name,
            tag,
            object,
            ..
        } => (full_ref_name, tag.unwrap_or(*object), *object),
        Ref::Unborn { .. } => return None,
    };
    Some(RemoteRef {
        name: name.to_str_lossy().into_owned(),
        id,
        peeled,
    })
}

/// Refspec that produced a mapping.
fn spec_of(ref_map: &RefMap, index: SpecIndex) -> Option<&RefSpec> {
    match index {
        SpecIndex::ExplicitInRemote(position) => ref_map.refspecs.get(position),
        SpecIndex::Implicit(position) => ref_map.extra_refspecs.get(position),
    }
}

/// Refuses a local reference name that is not a valid reference name, such as
/// one with a `..` component, before it is turned into a file path.
fn checked_local(name: &str) -> Result<()> {
    if gix::refs::FullName::try_from(name).is_ok() {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::GitRef,
        format!("the refspec maps to `{name}`, which is not a valid reference name; fix the destination of the refspec"),
    )
    .with_problem("invalid_name")
    .with_found(name.to_owned()))
}

/// Selects the updates of a fetch whose objects are in the object store.
///
/// # Errors
///
/// `E_GIT_REF` with problem `checked_out` when a refspec would update the
/// branch checked out in the working tree, and `E_IO` when the references
/// cannot be read.
pub(crate) fn select(
    repo: &GitRepo,
    ref_map: &RefMap,
    resolved: &Resolved,
    action: &str,
    shallow: &HashSet<ObjectId>,
) -> Result<Selection> {
    let checked_out = if repo.is_bare() {
        None
    } else {
        repo.head_ref()?.map(|name| name.as_bstr().to_string())
    };
    let mut seen = HashSet::new();
    let mut fetched = Vec::new();
    let mut heads = Vec::new();
    for mapping in &ref_map.mappings {
        let Some(remote) = remote_ref(&mapping.remote) else {
            continue;
        };
        let implicit = matches!(mapping.spec_index, SpecIndex::Implicit(_));
        let local = mapping
            .local
            .as_ref()
            .map(|name| name.to_str_lossy().into_owned());
        if let Some(local) = &local {
            checked_local(local)?;
        }
        if implicit && !follows(repo.gix(), resolved.tags, local.as_deref(), &remote)? {
            continue;
        }
        if let Some(local) = &local
            && !seen.insert(local.clone())
        {
            continue;
        }
        heads.push(Entry {
            id: remote.id,
            remote_name: remote.name.clone(),
            spec: match mapping.spec_index {
                SpecIndex::ExplicitInRemote(position) => Some(position),
                SpecIndex::Implicit(_) => None,
            },
            merge: false,
        });
        if let Some(local) = local {
            let force = spec_of(ref_map, mapping.spec_index)
                .is_some_and(|spec| spec.allow_non_fast_forward());
            let update = Update {
                local: &local,
                remote: &remote,
                force,
                checked_out: checked_out.as_deref(),
                action,
                shallow,
            };
            fetched.push(update.change(repo)?);
        }
    }
    let first_is_exact = ref_map
        .refspecs
        .first()
        .is_some_and(|spec| !spec_is_pattern(spec));
    head::mark_merge(
        &mut heads,
        resolved.explicit,
        &merge_refs(repo, resolved),
        first_is_exact,
    );
    let mut changes = if resolved.prune {
        prune(repo.gix(), ref_map, resolved, &seen, action)?
    } else {
        Vec::new()
    };
    changes.extend(fetched);
    Ok(Selection { changes, heads })
}

/// Whether a tag of the implicit tag refspec is stored: with all tags every
/// tag is, otherwise only a tag that does not exist locally and points to
/// an object that is now in the object store.
fn follows(
    repo: &gix::Repository,
    tags: Tags,
    local: Option<&str>,
    remote: &RemoteRef,
) -> Result<bool> {
    match tags {
        Tags::All => Ok(true),
        Tags::None => Ok(false),
        Tags::Included => {
            let exists = match local {
                Some(name) => current_id(repo, name)?.is_some(),
                None => false,
            };
            Ok(!exists && repo.has_object(remote.peeled) && repo.has_object(remote.id))
        }
    }
}

/// Remote references named by `branch.<current>.merge` when the current
/// branch follows the fetched remote.
fn merge_refs(repo: &GitRepo, resolved: &Resolved) -> Vec<String> {
    let (Some(remote), Ok(Some(head))) = (&resolved.name, repo.head_ref()) else {
        return Vec::new();
    };
    let head = head.as_bstr().to_string();
    let Some(branch) = head.strip_prefix(crate::repo::records::HEADS) else {
        return Vec::new();
    };
    if repo
        .config_string(&format!("branch.{branch}.remote"))
        .as_deref()
        != Some(remote)
    {
        return Vec::new();
    }
    repo.config()
        .strings(&format!("branch.{branch}.merge"))
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.to_str_lossy().into_owned())
        .collect()
}

/// Whether a refspec has a pattern.
fn spec_is_pattern(spec: &RefSpec) -> bool {
    match spec.to_ref().instruction() {
        gix::refspec::Instruction::Fetch(
            Fetch::Only { src } | Fetch::AndUpdate { src, .. } | Fetch::Exclude { src },
        ) => src.contains(&b'*'),
        gix::refspec::Instruction::Push(_) => false,
    }
}

/// Value of a local reference that points directly at an object.
pub(crate) fn current_id(repo: &gix::Repository, name: &str) -> Result<Option<ObjectId>> {
    let reference = repo
        .try_find_reference(name)
        .map_err(|failure| read_error("read a reference", &failure))?;
    Ok(reference.and_then(|reference| reference.target().try_id().map(ToOwned::to_owned)))
}

/// Update of one local reference from one remote reference.
struct Update<'a> {
    local: &'a str,
    remote: &'a RemoteRef,
    force: bool,
    checked_out: Option<&'a str>,
    shallow: &'a HashSet<ObjectId>,
    action: &'a str,
}

impl Update<'_> {
    /// The change `git fetch` makes.
    fn change(&self, repo: &GitRepo) -> Result<RefChange> {
        let gix = repo.gix();
        let old = current_id(gix, self.local)?;
        let new = self.remote.id;
        if old == Some(new) {
            return Ok(self.result(old, GitRefStatus::UpToDate, None, ""));
        }
        if self.checked_out == Some(self.local) {
            return Err(checked_out(repo, self.local));
        }
        let Some(previous) = old else {
            return Ok(self.result(old, GitRefStatus::New, None, self.storing()));
        };
        if self.local.starts_with(crate::repo::records::TAGS) {
            return Ok(if self.force {
                self.result(old, GitRefStatus::Forced, None, "updating tag")
            } else {
                self.result(
                    old,
                    GitRefStatus::Rejected,
                    Some("would_clobber_existing_tag"),
                    "",
                )
            });
        }
        if !(is_commit(gix, previous) && is_commit(gix, new)) {
            return Ok(self.result(old, GitRefStatus::Updated, None, self.storing()));
        }
        if is_ancestor(gix, previous, new, self.shallow) {
            Ok(self.result(old, GitRefStatus::Updated, None, "fast-forward"))
        } else if self.force {
            Ok(self.result(old, GitRefStatus::Forced, None, "forced-update"))
        } else {
            Ok(self.result(old, GitRefStatus::Rejected, Some("non_fast_forward"), ""))
        }
    }

    /// Reflog suffix of a new reference, named after the remote reference.
    fn storing(&self) -> &'static str {
        if self.remote.name.starts_with(crate::repo::records::TAGS) {
            "storing tag"
        } else if self.remote.name.starts_with(crate::repo::records::HEADS) {
            "storing head"
        } else {
            "storing ref"
        }
    }

    fn result(
        &self,
        old: Option<ObjectId>,
        status: GitRefStatus,
        reason: Option<&str>,
        what: &str,
    ) -> RefChange {
        RefChange {
            name: self.local.to_owned(),
            remote_ref: Some(self.remote.name.clone()),
            old,
            new: Some(self.remote.id),
            status,
            reason: reason.map(str::to_owned),
            message: if what.is_empty() {
                String::new()
            } else {
                format!("{}: {what}", self.action)
            },
        }
    }
}

/// Whether `id` names a commit.
fn is_commit(repo: &gix::Repository, id: ObjectId) -> bool {
    repo.find_object(id)
        .is_ok_and(|object| object.kind == gix::object::Kind::Commit)
}

/// Whether `ancestor` is reachable from `descendant`; false when the
/// history needed to decide is missing, as in a shallow repository.
/// `shallow` holds the commits that become the boundary of the history with
/// this fetch: their parents are not followed, as `git` does once it has
/// updated the `shallow` file.
fn is_ancestor(
    repo: &gix::Repository,
    ancestor: ObjectId,
    descendant: ObjectId,
    shallow: &HashSet<ObjectId>,
) -> bool {
    if !shallow.is_empty() {
        return reaches(repo, ancestor, descendant, shallow);
    }
    repo.merge_base(ancestor, descendant)
        .is_ok_and(|base| base.is_some_and(|base| base.detach() == ancestor))
}

/// Error for an update of the branch checked out in the working tree.
fn checked_out(repo: &GitRepo, name: &str) -> Error {
    Error::new(
        ErrorCode::GitRef,
        format!("refusing to fetch into the branch `{name}` checked out in {}; fetch into a remote-tracking reference or switch to another branch", quote_path(repo.root())),
    )
    .with_problem("checked_out")
    .with_found(name.to_owned())
    .with_fix(format!("{COMMAND} {}", quote_path(repo.root())))
}

/// Stale references: local references a refspec maps to whose remote
/// reference no longer exists.
fn prune(
    repo: &gix::Repository,
    ref_map: &RefMap,
    resolved: &Resolved,
    fetched: &HashSet<String>,
    action: &str,
) -> Result<Vec<RefChange>> {
    let remote_names: HashSet<String> = ref_map
        .remote_refs
        .iter()
        .map(|reference| {
            let (name, _, _) = reference.unpack();
            name.to_str_lossy().into_owned()
        })
        .collect();
    let mut specs: Vec<&RefSpec> = ref_map.refspecs.iter().collect();
    if resolved.tags == Tags::All {
        specs.extend(ref_map.extra_refspecs.iter());
    }
    let locals = local_references(repo)?;
    let mut stale = Vec::new();
    let mut listed = HashSet::new();
    for spec in specs {
        let gix::refspec::Instruction::Fetch(Fetch::AndUpdate { src, dst, .. }) =
            spec.to_ref().instruction()
        else {
            continue;
        };
        let (src, dst) = (src.to_str_lossy(), dst.to_str_lossy());
        for (name, id) in &locals {
            let Some(source) = reverse_map(&src, &dst, name) else {
                continue;
            };
            if remote_names.contains(&source) || fetched.contains(name) || !listed.insert(name) {
                continue;
            }
            stale.push(RefChange {
                name: name.clone(),
                remote_ref: None,
                old: Some(*id),
                new: None,
                status: GitRefStatus::Deleted,
                reason: None,
                message: format!("{action}: deleted"),
            });
        }
    }
    Ok(stale)
}

/// Every local reference that points directly at an object.
fn local_references(repo: &gix::Repository) -> Result<Vec<(String, ObjectId)>> {
    let platform = repo
        .references()
        .map_err(|failure| read_error("list references", &failure))?;
    let all = platform
        .all()
        .map_err(|failure| read_error("list references", &failure))?;
    Ok(all
        .filter_map(std::result::Result::ok)
        .filter_map(|reference| {
            let id = reference.target().try_id()?.to_owned();
            Some((reference.name().as_bstr().to_string(), id))
        })
        .collect())
}

/// Remote name that `dst` maps to `local` from, for a refspec `src:dst`
/// with at most one `*` on each side.
fn reverse_map(src: &str, dst: &str, local: &str) -> Option<String> {
    match (src.split_once('*'), dst.split_once('*')) {
        (Some((src_prefix, src_suffix)), Some((dst_prefix, dst_suffix))) => {
            let middle = local.strip_prefix(dst_prefix)?.strip_suffix(dst_suffix)?;
            Some(format!("{src_prefix}{middle}{src_suffix}"))
        }
        (None, None) => (local == dst).then(|| src.to_owned()),
        _ => None,
    }
}

/// Whether `ancestor` is found walking the parents from `start`, without
/// leaving the commits of the shallow boundary.
fn reaches(
    repo: &gix::Repository,
    ancestor: ObjectId,
    start: ObjectId,
    shallow: &HashSet<ObjectId>,
) -> bool {
    let mut seen = HashSet::new();
    let mut pending = vec![start];
    while let Some(id) = pending.pop() {
        if id == ancestor {
            return true;
        }
        if shallow.contains(&id) || !seen.insert(id) {
            continue;
        }
        if let Ok(commit) = repo.find_commit(id) {
            pending.extend(commit.parent_ids().map(gix::Id::detach));
        }
    }
    false
}

/// Commits listed in the content of a `shallow` file.
pub(crate) fn shallow_commits(content: &[u8]) -> HashSet<ObjectId> {
    content
        .lines()
        .filter_map(|line| ObjectId::from_hex(line.trim()).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_names_that_are_not_reference_names_are_refused() {
        checked_local("refs/remotes/origin/main").unwrap();
        for name in ["refs/heads/../../x", "refs/heads/a..b", "refs/heads//x"] {
            let error = checked_local(name).unwrap_err();
            assert_eq!(error.problem.as_deref(), Some("invalid_name"), "{name}");
        }
    }
    #[test]
    fn reverse_map_inverts_glob_refspecs() {
        assert_eq!(
            reverse_map(
                "refs/heads/*",
                "refs/remotes/origin/*",
                "refs/remotes/origin/a/b"
            )
            .as_deref(),
            Some("refs/heads/a/b")
        );
        assert_eq!(
            reverse_map(
                "refs/heads/*",
                "refs/remotes/origin/*",
                "refs/remotes/other/a"
            ),
            None
        );
        assert_eq!(
            reverse_map(
                "refs/heads/main",
                "refs/remotes/o/main",
                "refs/remotes/o/main"
            )
            .as_deref(),
            Some("refs/heads/main")
        );
        assert_eq!(reverse_map("refs/heads/*", "refs/x", "refs/x"), None);
    }

    #[test]
    fn unborn_remote_branches_have_no_identifiers() {
        let unborn = Ref::Unborn {
            full_ref_name: "HEAD".into(),
            target: "refs/heads/main".into(),
        };
        assert!(advertised(&unborn).is_none());
        let id = ObjectId::from_bytes_or_panic(&[7; 20]);
        let tag = ObjectId::from_bytes_or_panic(&[8; 20]);
        let peeled = advertised(&Ref::Peeled {
            full_ref_name: "refs/tags/v1".into(),
            tag,
            object: id,
        })
        .unwrap();
        assert_eq!((peeled.id, peeled.peeled), (tag, id));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
