// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repositories for tests, built with `gix` in temporary directories.

use gix::ObjectId;
use gix::refs::transaction::PreviousValue;

/// Fixed time of all test commits, so that object identifiers are stable.
const COMMIT_TIME: i64 = 1_700_000_000;

/// Repository in a temporary directory.
pub(crate) struct TestRepo {
    pub(crate) dir: tempfile::TempDir,
    pub(crate) repo: gix::Repository,
}

impl TestRepo {
    pub(crate) fn init() -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        let repo = gix::init(dir.path()).expect("initialized repository");
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

    /// Points `HEAD` at `refs/heads/<branch>`.
    pub(crate) fn set_head(&self, branch: &str) {
        let target: gix::refs::FullName = format!("refs/heads/{branch}").try_into().expect("valid name");
        self.repo
            .edit_reference(gix::refs::transaction::RefEdit {
                change: gix::refs::transaction::Change::Update {
                    log: gix::refs::transaction::LogChange::default(),
                    expected: PreviousValue::Any,
                    new: gix::refs::Target::Symbolic(target),
                },
                name: "HEAD".try_into().expect("valid name"),
                deref: false,
            })
            .expect("HEAD updated");
    }

    /// Writes a commit with `files` on `branch` and returns its identifier.
    pub(crate) fn commit(&self, branch: &str, files: &[(&str, &str)], parents: &[ObjectId], message: &str) -> ObjectId {
        let mut entries: Vec<gix::objs::tree::Entry> = files
            .iter()
            .map(|(name, content)| gix::objs::tree::Entry {
                mode: gix::objs::tree::EntryKind::Blob.into(),
                filename: (*name).into(),
                oid: self.repo.write_blob(content.as_bytes()).expect("blob written").detach(),
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
        let id = self.repo.write_object(&commit).expect("commit written").detach();
        self.repo
            .reference(format!("refs/heads/{branch}"), id, PreviousValue::Any, "test")
            .expect("branch updated");
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
            pgp_signature: None,
        };
        let id = self.repo.write_object(&tag).expect("tag written").detach();
        self.repo
            .reference(format!("refs/tags/{name}"), id, PreviousValue::Any, "test")
            .expect("tag reference written");
        id
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        self.dir.path()
    }
}
