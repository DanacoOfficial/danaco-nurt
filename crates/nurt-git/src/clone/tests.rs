// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::clone::tests
// Responsibility:   Repository with three commits on `main`, a branch `topic` one commit.
// Dependencies:     gix, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::path::{Path, PathBuf};

use gix::ObjectId;
use nurt_contracts::error::{Error, ErrorCode};

use super::*;
use crate::testutil::{Fixture, RecordingJournal, RepoBuilder, snapshot, test_ctx};

/// Repository with three commits on `main`, a branch `topic` one commit
/// behind, a branch `gone` two commits behind and an annotated tag `v1` on
/// the first commit.
fn upstream() -> Fixture {
    RepoBuilder::new()
        .file("a.txt", "one\n")
        .commit("first")
        .file("a.txt", "two\n")
        .commit("second")
        .executable("run.sh", "echo\n")
        .file("dir/b.txt", "three\n")
        .commit("third")
        .reference("refs/heads/topic", "HEAD~1")
        .reference("refs/heads/gone", "HEAD~2")
        .annotated_tag("v1", "HEAD~2", "release")
        .build()
}

struct Cloned {
    _dir: tempfile::TempDir,
    root: PathBuf,
    outcome: CloneOutcome,
}

impl Cloned {
    fn git_dir(&self, bare: bool) -> PathBuf {
        if bare {
            self.root.clone()
        } else {
            self.root.join(".git")
        }
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.git_dir(false).join(relative))
            .unwrap_or_else(|failure| panic!("{relative}: {failure}"))
    }

    fn id(&self, name: &str) -> Option<ObjectId> {
        crate::testutil::open_isolated(&self.root)
            .expect("clone opens")
            .try_find_reference(name)
            .expect("readable reference")
            .and_then(|reference| reference.target().try_id().map(ToOwned::to_owned))
    }
}

fn try_clone(
    mut request: CloneRequest,
    dir: &tempfile::TempDir,
) -> (RecordingJournal, Result<CloneOutcome, Error>) {
    request.to = dir.path().join("clone");
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let result = clone(&request, &mut *tx, &test_ctx());
    match &result {
        Ok(_) => {
            tx.commit().expect("transaction commits");
        }
        Err(_) => {
            tx.rollback(nurt_contracts::journal::RollbackReason::TargetFailed)
                .expect("rollback");
        }
    }
    (journal, result)
}

fn run(source: &Path, configure: impl FnOnce(&mut CloneRequest)) -> Cloned {
    let dir = tempfile::tempdir().expect("temporary directory");
    let mut request = CloneRequest::new(source.to_string_lossy(), dir.path().join("clone"));
    configure(&mut request);
    let (_, result) = try_clone(request, &dir);
    let outcome = result.expect("clone succeeds");
    Cloned {
        root: dir.path().join("clone"),
        _dir: dir,
        outcome,
    }
}

fn upstream_id(up: &Fixture, name: &str) -> ObjectId {
    up.open()
        .find_reference(name)
        .expect("reference exists")
        .id()
        .detach()
}

#[test]
fn full_clone_has_the_layout_of_git() {
    let up = upstream();
    let clone = run(up.path(), |_| {});
    // Git stores a local path with `/` separators, on Windows too.
    let url = up.path().to_string_lossy().replace('\\', "/");
    assert_eq!(clone.read("HEAD"), "ref: refs/heads/main\n");
    let config = clone.read("config");
    assert!(
        config.contains(&format!("[remote \"origin\"]\n\turl = {url}\n")),
        "{config}"
    );
    assert!(config.contains("\tfetch = +refs/heads/*:refs/remotes/origin/*\n"));
    assert!(config.contains("[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n"));
    let packed = clone.read("packed-refs");
    assert!(packed.starts_with("# pack-refs with: peeled fully-peeled sorted \n"));
    for branch in ["main", "topic", "gone"] {
        assert_eq!(
            clone.id(&format!("refs/remotes/origin/{branch}")),
            Some(upstream_id(&up, &format!("refs/heads/{branch}"))),
            "{branch}"
        );
    }
    assert_eq!(
        clone.id("refs/heads/main"),
        Some(upstream_id(&up, "refs/heads/main"))
    );
    assert_eq!(
        clone.read("refs/remotes/origin/HEAD"),
        "ref: refs/remotes/origin/main\n"
    );
    assert!(packed.contains(" refs/tags/v1\n^"), "{packed}");
    assert_eq!(clone.outcome.branch.as_deref(), Some("main"));
    assert_eq!(
        clone.outcome.head,
        Some(upstream_id(&up, "refs/heads/main"))
    );
}

#[test]
fn full_clone_checks_out_the_files_and_the_index() {
    let up = upstream();
    let clone = run(up.path(), |_| {});
    assert_eq!(
        std::fs::read_to_string(clone.root.join("a.txt")).unwrap(),
        "two\n"
    );
    assert_eq!(
        std::fs::read_to_string(clone.root.join("dir/b.txt")).unwrap(),
        "three\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(clone.root.join("run.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0);
    }
    let repo = crate::testutil::open_isolated(&clone.root).unwrap();
    let index = repo.index().expect("index is written");
    assert_eq!(index.entries().len(), 3);
    assert_eq!(clone.outcome.files_written, 3);
    assert!(!clone.root.join(".git/index.lock").exists());
}

#[test]
fn reflogs_record_the_clone() {
    let up = upstream();
    let clone = run(up.path(), |_| {});
    // Git stores a local path with `/` separators, on Windows too.
    let url = up.path().to_string_lossy().replace('\\', "/");
    for log in ["HEAD", "refs/heads/main", "refs/remotes/origin/HEAD"] {
        let text = clone.read(&format!("logs/{log}"));
        assert_eq!(text.lines().count(), 1, "{log}");
        assert!(text.starts_with(&"0".repeat(40)), "{log}");
        assert!(
            text.ends_with(&format!("\tclone: from {url}\n")),
            "{log}: {text}"
        );
    }
    assert!(
        !clone
            .git_dir(false)
            .join("logs/refs/remotes/origin/main")
            .exists()
    );
}

#[test]
fn reports_every_reference_created() {
    let up = upstream();
    let clone = run(up.path(), |_| {});
    let names: Vec<&str> = clone
        .outcome
        .ref_updates
        .iter()
        .map(|update| update.reference.as_str())
        .collect();
    for expected in [
        "refs/remotes/origin/main",
        "refs/remotes/origin/topic",
        "refs/remotes/origin/gone",
        "refs/tags/v1",
        "refs/heads/main",
    ] {
        assert!(names.contains(&expected), "{expected} in {names:?}");
    }
    assert!(
        clone
            .outcome
            .ref_updates
            .iter()
            .all(|update| update.status == GitRefStatus::New)
    );
    assert!(clone.outcome.objects_received > 0);
    assert!(clone.outcome.bytes_received > 0);
}

#[test]
fn branch_option_checks_out_that_branch() {
    let up = upstream();
    let clone = run(up.path(), |request| {
        request.branch = Some("topic".to_owned())
    });
    assert_eq!(clone.read("HEAD"), "ref: refs/heads/topic\n");
    assert_eq!(
        clone.id("refs/heads/topic"),
        Some(upstream_id(&up, "refs/heads/topic"))
    );
    assert_eq!(
        std::fs::read_to_string(clone.root.join("a.txt")).unwrap(),
        "two\n"
    );
    assert!(!clone.root.join("dir/b.txt").exists());
    assert!(clone.read("config").contains("[branch \"topic\"]"));
    assert_eq!(
        clone.read("refs/remotes/origin/HEAD"),
        "ref: refs/remotes/origin/main\n"
    );
}

#[test]
fn unknown_branch_lists_the_branches_of_the_remote() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let mut request = CloneRequest::new(up.path().to_string_lossy(), dir.path().join("clone"));
    request.branch = Some("nothing".to_owned());
    let (_, result) = try_clone(request, &dir);
    let error = result.unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitRef);
    assert_eq!(error.problem.as_deref(), Some("remote_branch_not_found"));
    assert!(!dir.path().join("clone").exists());
}

#[test]
fn depth_limits_history_and_the_fetched_branches() {
    let up = upstream();
    let clone = run(up.path(), |request| request.depth = Some(1));
    let shallow = clone.read("shallow");
    assert_eq!(
        shallow.trim(),
        upstream_id(&up, "refs/heads/main").to_string()
    );
    let config = clone.read("config");
    assert!(
        config.contains("\tfetch = +refs/heads/main:refs/remotes/origin/main\n"),
        "{config}"
    );
    assert_eq!(clone.id("refs/remotes/origin/topic"), None);
    assert_eq!(clone.id("refs/tags/v1"), None);
    assert!(
        clone
            .outcome
            .notable
            .iter()
            .any(|entry| entry.code == Notable::ShallowRepository)
    );
}

#[test]
fn depth_zero_is_rejected_before_the_directory_is_created() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let mut request = CloneRequest::new(up.path().to_string_lossy(), dir.path().join("clone"));
    request.depth = Some(0);
    let (_, result) = try_clone(request, &dir);
    let error = result.unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert_eq!(error.problem.as_deref(), Some("invalid_depth"));
    assert!(!dir.path().join("clone").exists());
}

#[test]
fn bare_clone_maps_branches_to_local_branches_without_reflogs() {
    let up = upstream();
    let clone = run(up.path(), |request| request.bare = true);
    let git_dir = &clone.root;
    assert_eq!(
        std::fs::read_to_string(git_dir.join("HEAD")).unwrap(),
        "ref: refs/heads/main\n"
    );
    let config = std::fs::read_to_string(git_dir.join("config")).unwrap();
    assert!(config.contains("bare = true"));
    assert!(config.contains("[remote \"origin\"]\n\turl = "));
    assert!(!config.contains("fetch ="));
    assert!(!config.contains("[branch"));
    assert_eq!(
        clone.id("refs/heads/topic"),
        Some(upstream_id(&up, "refs/heads/topic"))
    );
    assert_eq!(
        clone.id("refs/tags/v1"),
        Some(upstream_id(&up, "refs/tags/v1"))
    );
    assert!(!git_dir.join("logs").exists());
    assert!(!git_dir.join("a.txt").exists());
}

#[test]
fn empty_remote_leaves_an_unborn_branch() {
    let up = RepoBuilder::new().build();
    let clone = run(up.path(), |_| {});
    let head = clone.read("HEAD");
    assert!(head.starts_with("ref: refs/heads/"), "{head}");
    assert!(clone.outcome.head.is_none());
    assert!(clone.outcome.ref_updates.is_empty());
    let branch = clone.outcome.branch.clone().expect("branch name");
    assert!(
        clone
            .read("config")
            .contains(&format!("[branch \"{branch}\"]"))
    );
    assert!(!clone.root.join(".git/packed-refs").exists());
    assert_eq!(clone.outcome.files_written, 0);
}

#[test]
fn detached_remote_head_gives_a_detached_clone() {
    let up = upstream();
    let id = upstream_id(&up, "refs/heads/main");
    let repo = up.open();
    let parent = repo
        .find_commit(id)
        .unwrap()
        .parent_ids()
        .next()
        .unwrap()
        .detach();
    std::fs::write(up.git_dir().join("HEAD"), format!("{parent}\n")).unwrap();
    std::fs::remove_file(up.git_dir().join("refs/heads/topic")).unwrap();
    let clone = run(up.path(), |_| {});
    assert_eq!(clone.read("HEAD"), format!("{parent}\n"));
    assert_eq!(clone.outcome.branch, None);
    assert!(
        !clone
            .git_dir(false)
            .join("refs/remotes/origin/HEAD")
            .exists()
    );
    assert_eq!(
        std::fs::read_to_string(clone.root.join("a.txt")).unwrap(),
        "two\n"
    );
    assert!(!clone.read("config").contains("[branch"));
}

#[test]
fn credentials_in_the_address_are_not_stored() {
    let stored = layout::stored_url("https://user:secret@example.com/app.git").unwrap();
    assert!(!stored.contains("secret"));
    assert!(stored.contains("user@example.com"));
}

#[test]
fn existing_destination_is_rejected_and_left_alone() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("clone");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep.txt"), "kept").unwrap();
    let request = CloneRequest::new(up.path().to_string_lossy(), &target);
    let (_, result) = try_clone(request, &dir);
    let error = result.unwrap_err();
    assert_eq!(error.code(), ErrorCode::Exists);
    assert_eq!(error.problem.as_deref(), Some("destination_exists"));
    assert_eq!(
        std::fs::read_to_string(target.join("keep.txt")).unwrap(),
        "kept"
    );
}

#[test]
fn missing_parent_directory_is_rejected() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let request = CloneRequest::new(up.path().to_string_lossy(), dir.path().join("a/b/clone"));
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let error = clone(&request, &mut *tx, &test_ctx()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert_eq!(journal.committed(), 0);
}

#[test]
fn relative_local_address_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let request = CloneRequest::new("relative/repo", dir.path().join("clone"));
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let error = clone(&request, &mut *tx, &test_ctx()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert!(!dir.path().join("clone").exists());
}

#[test]
fn failed_transfer_rolls_back_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-repository");
    let request = CloneRequest::new(missing.to_string_lossy(), dir.path().join("clone"));
    let (_, result) = try_clone(request, &dir);
    assert!(result.is_err());
    assert!(!dir.path().join("clone").exists());
}

#[test]
fn undo_removes_the_new_tree() {
    let up = upstream();
    let dir = tempfile::tempdir().unwrap();
    let before = snapshot(dir.path());
    let request = CloneRequest::new(up.path().to_string_lossy(), dir.path().join("clone"));
    let (journal, result) = try_clone(request, &dir);
    result.expect("clone succeeds");
    assert_ne!(snapshot(dir.path()), before);
    assert_eq!(journal.last_targets(), vec![dir.path().join("clone")]);
    journal.undo_last();
    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn file_address_clones_like_a_path() {
    let up = upstream();
    let address = format!("file://{}", up.path().display());
    let dir = tempfile::tempdir().unwrap();
    let request = CloneRequest::new(address.clone(), dir.path().join("clone"));
    let (_, result) = try_clone(request, &dir);
    let outcome = result.expect("clone succeeds");
    assert_eq!(outcome.url, address);
    assert_eq!(outcome.head, Some(upstream_id(&up, "refs/heads/main")));
}

#[test]
fn clone_of_a_clone_follows_its_branches() {
    let up = upstream();
    let first = run(up.path(), |_| {});
    let second = run(&first.root, |_| {});
    assert_eq!(second.outcome.head, first.outcome.head);
    assert_eq!(
        second.id("refs/remotes/origin/main"),
        first.id("refs/heads/main")
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
