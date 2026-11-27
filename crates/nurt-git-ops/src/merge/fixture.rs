// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::merge::fixture
// Responsibility:   Repositories and objects for unit tests of the merge module.
// Dependencies:     bstr, gix_hash, gix_object, nurt_git
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Repositories and objects for unit tests of the merge module.

use std::path::Path;

use bstr::ByteSlice;
use gix_hash::ObjectId;
use gix_object::tree::EntryKind;

/// Seconds of the fixed test clock: 2026-01-01T12:00:00Z.
pub(crate) const TEST_TIME: i64 = 1_767_268_800;

pub(crate) struct Fixture {
    pub(crate) dir: tempfile::TempDir,
    pub(crate) repo: gix::Repository,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        let repo = nurt_git::testutil::init_isolated(dir.path()).expect("repository");
        Self { dir, repo }
    }

    pub(crate) fn workdir(&self) -> &Path {
        self.dir.path()
    }

    pub(crate) fn blob(&self, data: &[u8]) -> ObjectId {
        self.repo.write_blob(data).expect("blob").detach()
    }

    /// Tree of text files.
    pub(crate) fn tree(&self, files: &[(&str, &str)]) -> ObjectId {
        let entries: Vec<(&str, EntryKind, &[u8])> = files
            .iter()
            .map(|(path, text)| (*path, EntryKind::Blob, text.as_bytes()))
            .collect();
        self.tree_with(&entries)
    }

    /// Tree of entries of any blob kind.
    pub(crate) fn tree_with(&self, entries: &[(&str, EntryKind, &[u8])]) -> ObjectId {
        let mut editor = gix_object::tree::Editor::new(
            gix_object::Tree::empty(),
            &self.repo.objects,
            gix_hash::Kind::Sha1,
        );
        for (path, kind, data) in entries {
            let id = self.blob(data);
            editor
                .upsert(path.split('/'), *kind, id)
                .expect("tree entry");
        }
        editor
            .write(|tree| self.repo.write_object(tree).map(gix::Id::detach))
            .expect("tree")
    }

    pub(crate) fn commit(&self, tree: ObjectId, parents: &[ObjectId], message: &str) -> ObjectId {
        let signature = gix::actor::Signature {
            name: "A".into(),
            email: "a@x".into(),
            time: gix::date::Time::new(TEST_TIME, 0),
        };
        let commit = gix_object::Commit {
            tree,
            parents: parents.iter().copied().collect(),
            author: signature.clone(),
            committer: signature,
            encoding: None,
            message: format!("{message}\n").into(),
            extra_headers: Vec::new(),
        };
        self.repo.write_object(&commit).expect("commit").detach()
    }

    /// Content of the blob at `path` in `tree`; `None` when absent.
    pub(crate) fn read(&self, tree: ObjectId, path: &str) -> Option<String> {
        let tree = self.repo.find_tree(tree).expect("tree");
        let entry = tree.lookup_entry_by_path(path).expect("lookup")?;
        let blob = self.repo.find_blob(entry.object_id()).expect("blob");
        Some(blob.data.to_str_lossy().into_owned())
    }

    pub(crate) fn mode(&self, tree: ObjectId, path: &str) -> Option<EntryKind> {
        let tree = self.repo.find_tree(tree).expect("tree");
        let entry = tree.lookup_entry_by_path(path).expect("lookup")?;
        Some(entry.mode().kind())
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
