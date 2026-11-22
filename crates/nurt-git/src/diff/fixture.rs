// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test repositories built in temporary directories without the `git`
//! program.

use std::path::{Path, PathBuf};

use gix::ObjectId;
use gix::object::tree::EntryKind;

/// A repository with a working tree in a temporary directory.
pub(crate) struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    /// An empty repository.
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        gix::init(dir.path()).expect("repository");
        Self { dir }
    }

    /// Root of the working tree.
    pub(crate) fn root(&self) -> &Path {
        self.dir.path()
    }

    /// The repository opened without the configuration of the system and
    /// the account, with a fixed identity for reference logs.
    pub(crate) fn repo(&self) -> gix::Repository {
        let options = gix::open::Options::isolated()
            .config_overrides(["user.name=Tester", "user.email=tester@example.com"]);
        gix::open_opts(self.root(), options).expect("open")
    }

    /// Writes a worktree file, creating its directories.
    pub(crate) fn write(&self, path: &str, content: &str) {
        self.write_bytes(path, content.as_bytes());
    }

    /// Writes a worktree file with raw content.
    pub(crate) fn write_bytes(&self, path: &str, content: &[u8]) {
        let full = self.full(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("directories");
        }
        std::fs::write(full, content).expect("write");
    }

    /// Removes a worktree file.
    pub(crate) fn remove(&self, path: &str) {
        std::fs::remove_file(self.full(path)).expect("remove");
    }

    fn full(&self, path: &str) -> PathBuf {
        self.root().join(path)
    }

    fn head_tree(&self, repo: &gix::Repository) -> ObjectId {
        repo.head_tree_id_or_empty().expect("head tree").detach()
    }

    fn tree_with(
        &self,
        repo: &gix::Repository,
        base: ObjectId,
        files: &[(&str, &str)],
    ) -> ObjectId {
        let mut editor = repo.edit_tree(base).expect("editor");
        for (path, content) in files {
            let blob = repo.write_blob(content.as_bytes()).expect("blob").detach();
            editor.upsert(*path, EntryKind::Blob, blob).expect("upsert");
        }
        editor.write().expect("tree").detach()
    }

    fn write_index(&self, repo: &gix::Repository, tree: ObjectId) {
        let mut index = repo.index_from_tree(&tree).expect("index");
        self.save_index(&mut index);
    }

    /// Writes `index` to its file.
    // Test repositories are prepared outside a journal transaction.
    #[allow(clippy::disallowed_methods)]
    pub(crate) fn save_index(&self, index: &mut gix::index::File) {
        index
            .write(gix::index::write::Options::default())
            .expect("write index");
    }

    /// Commits `files` on top of `HEAD`, writes them to the working tree and
    /// makes the index match the new commit. Returns the commit.
    pub(crate) fn commit(&self, files: &[(&str, &str)]) -> ObjectId {
        let repo = self.repo();
        let tree = self.tree_with(&repo, self.head_tree(&repo), files);
        let parent = repo.head_id().ok().map(gix::Id::detach);
        let signature = gix::actor::Signature {
            name: "Tester".into(),
            email: "tester@example.com".into(),
            time: gix::date::Time::new(1_767_225_600, 0),
        };
        let commit = gix::objs::Commit {
            tree,
            parents: parent.into_iter().collect(),
            author: signature.clone(),
            committer: signature,
            encoding: None,
            message: "change\n".into(),
            extra_headers: Vec::new(),
        };
        let id = repo.write_object(&commit).expect("commit").detach();
        let head = repo.head_name().expect("head").map_or_else(
            || "refs/heads/main".to_owned(),
            |name| name.as_bstr().to_string(),
        );
        repo.reference(
            head.as_str(),
            id,
            gix::refs::transaction::PreviousValue::Any,
            "commit",
        )
        .expect("reference");
        for (path, content) in files {
            self.write(path, content);
        }
        self.write_index(&repo, tree);
        id
    }

    /// Stages `files` on top of `HEAD` without committing and without
    /// touching the working tree.
    pub(crate) fn stage(&self, files: &[(&str, &str)]) {
        let repo = self.repo();
        let tree = self.tree_with(&repo, self.head_tree(&repo), files);
        self.write_index(&repo, tree);
    }
}
