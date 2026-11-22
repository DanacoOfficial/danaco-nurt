// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Enumeration of the objects a push must send.
//!
//! The set is the closure of the new tips minus everything reachable from
//! the commits the remote is known to have. Trees of the boundary commits
//! are marked as present, so unchanged subtrees and blobs are not sent; this
//! matches the edge handling of `git pack-objects --revs`. Gitlinks are not
//! followed. The same walk serves the connectivity check of the receiving
//! side, where a missing object means an incomplete pack.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use gix::ObjectId;
use gix::objs::{Kind, TreeRefIter};
use nurt_contracts::error::{Error, ErrorCode, Result};

/// Problem name of the error returned for an object that is not in the
/// repository.
pub const MISSING_OBJECT: &str = "missing_object";

/// Lists the objects reachable from `tips` and not reachable from `known`.
///
/// Identifiers in `known` that are absent from the repository are ignored.
/// The result starts with tag objects, then each new commit followed by its
/// new trees and blobs.
///
/// # Errors
///
/// Fails with `E_IO` and problem `missing_object` when an object reachable
/// from `tips` is absent, and with `E_CANCELLED` when `interrupt` is set.
pub fn objects_to_send(
    repo: &gix::Repository,
    tips: &[ObjectId],
    known: &[ObjectId],
    interrupt: &AtomicBool,
) -> Result<Vec<ObjectId>> {
    let mut walk = Walk {
        repo,
        interrupt,
        seen: HashSet::new(),
        out: Vec::new(),
    };
    let mut commit_tips = Vec::new();
    let mut tree_tips = Vec::new();
    for tip in tips {
        walk.classify_tip(*tip, &mut commit_tips, &mut tree_tips)?;
    }
    if !commit_tips.is_empty() {
        let known_commits = walk.known_commits(known);
        walk.add_commits(&commit_tips, &known_commits)?;
    }
    for tree in tree_tips {
        walk.add_tree(tree)?;
    }
    Ok(walk.out)
}

struct Walk<'a> {
    repo: &'a gix::Repository,
    interrupt: &'a AtomicBool,
    seen: HashSet<ObjectId>,
    out: Vec<ObjectId>,
}

impl<'a> Walk<'a> {
    fn classify_tip(
        &mut self,
        tip: ObjectId,
        commits: &mut Vec<ObjectId>,
        trees: &mut Vec<ObjectId>,
    ) -> Result<()> {
        let mut id = tip;
        loop {
            let object = self.find(id)?;
            match object.kind {
                Kind::Tag => {
                    self.emit(id);
                    id = object.try_to_tag_ref().map_err(|_| missing(id))?.target();
                }
                Kind::Commit => {
                    commits.push(id);
                    return Ok(());
                }
                Kind::Tree => {
                    trees.push(id);
                    return Ok(());
                }
                Kind::Blob => {
                    self.emit(id);
                    return Ok(());
                }
            }
        }
    }

    fn known_commits(&self, known: &[ObjectId]) -> Vec<ObjectId> {
        known
            .iter()
            .filter_map(|id| {
                let object = self.repo.find_object(*id).ok()?;
                let peeled = object.peel_tags_to_end().ok()?;
                (peeled.kind == Kind::Commit).then_some(peeled.id)
            })
            .collect()
    }

    fn add_commits(&mut self, tips: &[ObjectId], known: &[ObjectId]) -> Result<()> {
        let walk = self
            .repo
            .rev_walk(tips.iter().copied())
            .with_hidden(known.iter().copied())
            .all()
            .map_err(|error| walk_error(&error))?;
        let mut commits = Vec::new();
        for info in walk {
            self.check_interrupt()?;
            let info = info.map_err(|error| walk_error(&error))?;
            commits.push((info.id, info.parent_ids.to_vec()));
        }
        let new: HashSet<ObjectId> = commits.iter().map(|(id, _)| *id).collect();
        let edges: HashSet<ObjectId> = commits
            .iter()
            .flat_map(|(_, parents)| parents.iter().copied())
            .filter(|parent| !new.contains(parent))
            .collect();
        for edge in edges {
            if let Ok(tree) = self.commit_tree(edge) {
                self.mark_present(tree)?;
            }
        }
        for (id, _) in commits {
            self.emit(id);
            let tree = self.commit_tree(id)?;
            self.add_tree(tree)?;
        }
        Ok(())
    }

    fn commit_tree(&self, id: ObjectId) -> Result<ObjectId> {
        let object = self.find(id)?;
        let commit = object.try_to_commit_ref().map_err(|_| missing(id))?;
        Ok(commit.tree())
    }

    fn add_tree(&mut self, root: ObjectId) -> Result<()> {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !self.seen.insert(id) {
                continue;
            }
            self.check_interrupt()?;
            self.out.push(id);
            let object = self.find(id)?;
            for entry in TreeRefIter::from_bytes(&object.data, self.repo.object_hash()) {
                let entry = entry.map_err(|_| missing(id))?;
                let child = entry.oid.to_owned();
                if entry.mode.is_tree() {
                    stack.push(child);
                } else if !entry.mode.is_commit() && self.seen.insert(child) {
                    if !self.repo.has_object(child) {
                        return Err(missing(child));
                    }
                    self.out.push(child);
                }
            }
        }
        Ok(())
    }

    fn mark_present(&mut self, root: ObjectId) -> Result<()> {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !self.seen.insert(id) {
                continue;
            }
            self.check_interrupt()?;
            let Ok(object) = self.repo.find_object(id) else {
                continue;
            };
            for entry in TreeRefIter::from_bytes(&object.data, self.repo.object_hash()).flatten() {
                let child = entry.oid.to_owned();
                if entry.mode.is_tree() {
                    stack.push(child);
                } else if !entry.mode.is_commit() {
                    self.seen.insert(child);
                }
            }
        }
        Ok(())
    }

    fn emit(&mut self, id: ObjectId) {
        if self.seen.insert(id) {
            self.out.push(id);
        }
    }

    fn find(&self, id: ObjectId) -> Result<gix::Object<'a>> {
        self.repo.find_object(id).map_err(|_| missing(id))
    }

    fn check_interrupt(&self) -> Result<()> {
        if self.interrupt.load(Ordering::Relaxed) {
            return Err(Error::new(ErrorCode::Cancelled, "the push was cancelled"));
        }
        Ok(())
    }
}

fn missing(id: ObjectId) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("object {id} is missing from the repository; run `git fsck` to find the damage"),
    )
    .with_problem(MISSING_OBJECT)
    .with_detail("object", serde_json::Value::String(id.to_string()))
}

fn walk_error(error: &dyn std::fmt::Display) -> Error {
    let text = error.to_string();
    Error::new(
        ErrorCode::Io,
        format!("walking the commit history failed: {text}"),
    )
    .with_problem(MISSING_OBJECT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send_pack::testing::Fixture;

    #[test]
    fn sends_only_objects_unknown_to_the_remote() {
        let fixture = Fixture::new();
        let base = fixture.commit("main", &[("a.txt", "one\n"), ("dir/b.txt", "two\n")], &[]);
        let next = fixture.commit(
            "main",
            &[("a.txt", "one\n"), ("dir/b.txt", "three\n")],
            &[base],
        );
        let repo = fixture.repo();
        let all = objects_to_send(&repo, &[next], &[], &AtomicBool::new(false)).unwrap();
        let incremental =
            objects_to_send(&repo, &[next], &[base], &AtomicBool::new(false)).unwrap();
        assert_eq!(
            all.len(),
            9,
            "two commits, two root trees, two dir trees, three blobs"
        );
        assert_eq!(
            incremental.len(),
            4,
            "commit, root tree, dir tree, new blob"
        );
        assert_eq!(incremental[0], next);
        let none = objects_to_send(&repo, &[next], &[next], &AtomicBool::new(false)).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn follows_annotated_tags_and_ignores_unknown_known_ids() {
        let fixture = Fixture::new();
        let base = fixture.commit("main", &[("a.txt", "one\n")], &[]);
        let tag = fixture.tag("v1", base);
        let repo = fixture.repo();
        let unknown = ObjectId::from_bytes_or_panic(&[9; 20]);
        let objects =
            objects_to_send(&repo, &[tag], &[unknown, tag], &AtomicBool::new(false)).unwrap();
        assert_eq!(objects[0], tag);
        let objects = objects_to_send(&repo, &[tag], &[unknown], &AtomicBool::new(false)).unwrap();
        assert_eq!(objects.len(), 4);
    }

    #[test]
    fn reports_missing_objects_and_cancellation() {
        let fixture = Fixture::new();
        let base = fixture.commit("main", &[("a.txt", "one\n")], &[]);
        let repo = fixture.repo();
        let unknown = ObjectId::from_bytes_or_panic(&[9; 20]);
        let error = objects_to_send(&repo, &[unknown], &[], &AtomicBool::new(false)).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some(MISSING_OBJECT));
        let error = objects_to_send(&repo, &[base], &[], &AtomicBool::new(true)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Cancelled);
        fixture.remove_object(&fixture.blob_id("one\n"));
        let error = objects_to_send(&repo, &[base], &[], &AtomicBool::new(false)).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some(MISSING_OBJECT));
    }

    #[test]
    fn sends_blob_and_tree_tips() {
        let fixture = Fixture::new();
        let blob = fixture.blob_id("loose\n");
        fixture.write_blob("loose\n");
        let base = fixture.commit("main", &[("a.txt", "one\n")], &[]);
        let repo = fixture.repo();
        let tree = repo
            .find_object(base)
            .unwrap()
            .try_to_commit_ref()
            .unwrap()
            .tree();
        let objects = objects_to_send(&repo, &[blob, tree], &[], &AtomicBool::new(false)).unwrap();
        assert_eq!(objects.len(), 3);
    }
}
