// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parents, committer times and trees of commits, read from the commit-graph
//! file when the repository has one and from the commit objects otherwise.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use gix::ObjectId;
use smallvec::SmallVec;

use crate::fetch::{SHALLOW, update::shallow_commits};

/// What the traversal needs to know about one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitInfo {
    /// Parents in their recorded order.
    pub(crate) parents: SmallVec<[ObjectId; 2]>,
    /// Committer time in seconds since the Unix epoch.
    pub(crate) time: i64,
    /// Root tree.
    pub(crate) tree: ObjectId,
}

/// Cache of [`CommitInfo`] for the commits of one repository.
pub(crate) struct CommitCache {
    graph: Option<gix::commitgraph::Graph>,
    shallow: HashSet<ObjectId>,
    known: RefCell<HashMap<ObjectId, CommitInfo>>,
}

impl CommitCache {
    /// Creates an empty cache that uses the commit-graph file of `repo` when
    /// `core.commitGraph` allows it and the repository is not shallow.
    ///
    /// The commits listed in the `shallow` file have no parents, as in `git`.
    pub(crate) fn new(repo: &gix::Repository) -> Self {
        let shallow = std::fs::read(repo.common_dir().join(SHALLOW))
            .map(|content| shallow_commits(&content))
            .unwrap_or_default();
        let graph = if shallow.is_empty() {
            repo.commit_graph_if_enabled().ok().flatten()
        } else {
            None
        };
        Self {
            graph,
            shallow,
            known: RefCell::new(HashMap::new()),
        }
    }

    /// Facts about the commit `id`.
    pub(crate) fn info(&self, repo: &gix::Repository, id: ObjectId) -> Result<CommitInfo, String> {
        if let Some(info) = self.known.borrow().get(&id) {
            return Ok(info.clone());
        }
        let mut info = match self.read_graph(id) {
            Some(info) => info,
            None => from_object(repo, id)?,
        };
        if self.shallow.contains(&id) {
            info.parents.clear();
        }
        self.known.borrow_mut().insert(id, info.clone());
        Ok(info)
    }

    fn read_graph(&self, id: ObjectId) -> Option<CommitInfo> {
        let graph = self.graph.as_ref()?;
        let commit = graph.commit_by_id(id)?;
        let mut parents = SmallVec::new();
        for position in commit.iter_parents() {
            parents.push(graph.id_at(position.ok()?).to_owned());
        }
        Some(CommitInfo {
            parents,
            time: i64::try_from(commit.committer_timestamp()).ok()?,
            tree: commit.root_tree_id().to_owned(),
        })
    }
}

fn from_object(repo: &gix::Repository, id: ObjectId) -> Result<CommitInfo, String> {
    let object = repo.find_object(id).map_err(|err| err.to_string())?;
    if object.kind != gix::object::Kind::Commit {
        return Err(format!("{id} is a {}, not a commit", object.kind));
    }
    let commit = gix::objs::CommitRef::from_bytes(&object.data, repo.object_hash())
        .map_err(|err| err.to_string())?;
    let time = commit.committer().map_err(|err| err.to_string())?.seconds();
    Ok(CommitInfo {
        parents: commit.parents().collect(),
        time,
        tree: commit.tree(),
    })
}

#[cfg(test)]
mod tests {
    use crate::revwalk::fixture::Fixture;

    #[test]
    fn reads_parents_time_and_tree_of_commits() {
        let mut fixture = Fixture::new();
        let first = fixture.commit("first", &[("a.txt", Some("a\n"))]);
        let second = fixture.commit("second", &[("a.txt", Some("b\n"))]);
        let repo = fixture.open();
        let info = repo.commits.info(&repo.repo, second).unwrap();
        assert_eq!(info.parents.as_slice(), &[first]);
        assert_eq!(info.time, fixture.time_of(second));
        assert_eq!(info.tree, repo.tree_of(second).unwrap());
        let cached = repo.commits.info(&repo.repo, second).unwrap();
        assert_eq!(cached, info);
    }

    #[test]
    fn commits_of_the_shallow_file_have_no_parents() {
        let mut fixture = Fixture::new();
        let first = fixture.commit("first", &[("a.txt", Some("a\n"))]);
        let second = fixture.commit("second", &[("a.txt", Some("b\n"))]);
        let third = fixture.commit("third", &[("a.txt", Some("c\n"))]);
        fixture.write_file(".git/shallow", &format!("{second}\n"));
        let repo = fixture.open();
        assert!(
            repo.commits
                .info(&repo.repo, second)
                .unwrap()
                .parents
                .is_empty()
        );
        let info = repo.commits.info(&repo.repo, third).unwrap();
        assert_eq!(info.parents.as_slice(), &[second]);
        assert_ne!(first, second);
    }

    #[test]
    fn rejects_objects_that_are_not_commits() {
        let mut fixture = Fixture::new();
        let first = fixture.commit("first", &[("a.txt", Some("a\n"))]);
        let repo = fixture.open();
        let tree = repo.tree_of(first).unwrap();
        assert!(repo.commits.info(&repo.repo, tree).is_err());
    }
}
