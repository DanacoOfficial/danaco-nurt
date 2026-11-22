// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repositories for tests, built with `gix` in temporary directories.
//!
//! References are written as loose reference files, so that no reflog and
//! therefore no committer identity is needed.

use gix::ObjectId;

/// Fixed time of all test commits, so that object identifiers are stable.
const COMMIT_TIME: i64 = 1_700_000_000;

/// Identity for the reflog entries written by fetches into test repositories.
const IDENTITY: &str = "[user]\n\tname = Test\n\temail = test@example.com\n";

/// Repository in a temporary directory.
pub(crate) struct TestRepo {
    pub(crate) dir: tempfile::TempDir,
    pub(crate) repo: gix::Repository,
}

impl TestRepo {
    pub(crate) fn init() -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        let initialized = gix::init(dir.path()).expect("initialized repository");
        let config = initialized.git_dir().join("config");
        let mut text = std::fs::read_to_string(&config).expect("configuration read");
        text.push_str(IDENTITY);
        std::fs::write(&config, text).expect("configuration written");
        let repo = gix::open(dir.path()).expect("opened repository");
        let this = Self { dir, repo };
        this.set_head("main");
        this
    }

    fn signature() -> gix::actor::Signature {
        gix::actor::Signature {
            name: "Test".into(),
            email: "test@example.com".into(),
            time: gix::date::Time::new(COMMIT_TIME, 0),
        }
    }

    /// Path of the working tree.
    pub(crate) fn path(&self) -> &std::path::Path {
        self.dir.path()
    }

    /// Points `HEAD` at `refs/heads/<branch>`.
    pub(crate) fn set_head(&self, branch: &str) {
        std::fs::write(
            self.repo.git_dir().join("HEAD"),
            format!("ref: refs/heads/{branch}\n"),
        )
        .expect("HEAD written");
    }

    /// Writes the loose reference `name` with `id`.
    pub(crate) fn write_ref(&self, name: &str, id: ObjectId) {
        let path = self.repo.git_dir().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("reference directory");
        }
        std::fs::write(path, format!("{id}\n")).expect("reference written");
    }

    /// Writes a commit with `files` on `branch` and returns its identifier.
    pub(crate) fn commit(
        &self,
        branch: &str,
        files: &[(&str, &str)],
        parents: &[ObjectId],
        message: &str,
    ) -> ObjectId {
        let mut entries: Vec<gix::objs::tree::Entry> = files
            .iter()
            .map(|(name, content)| gix::objs::tree::Entry {
                mode: gix::objs::tree::EntryKind::Blob.into(),
                filename: (*name).into(),
                oid: self
                    .repo
                    .write_blob(content.as_bytes())
                    .expect("blob written")
                    .detach(),
            })
            .collect();
        entries.sort_by(|a, b| a.filename.cmp(&b.filename));
        let tree = self
            .repo
            .write_object(&gix::objs::Tree { entries })
            .expect("tree written")
            .detach();
        let commit = gix::objs::Commit {
            tree,
            parents: parents.iter().copied().collect(),
            author: Self::signature(),
            committer: Self::signature(),
            encoding: None,
            message: format!("{message}\n").into(),
            extra_headers: Vec::new(),
        };
        let id = self
            .repo
            .write_object(&commit)
            .expect("commit written")
            .detach();
        self.write_ref(&format!("refs/heads/{branch}"), id);
        id
    }

    /// Writes an annotated tag `name` on `target` and returns the tag object.
    pub(crate) fn tag(&self, name: &str, target: ObjectId) -> ObjectId {
        let tag = gix::objs::Tag {
            target,
            target_kind: gix::objs::Kind::Commit,
            name: name.into(),
            tagger: Some(Self::signature()),
            message: "release\n".into(),
            signature: None,
        };
        let id = self.repo.write_object(&tag).expect("tag written").detach();
        self.write_ref(&format!("refs/tags/{name}"), id);
        id
    }
}
