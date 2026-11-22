// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.restore` command.
//!
//! Restoration copies the selected paths from a source into the index, the
//! working tree or both, in the no-overlay mode of `git restore`: selected
//! paths that the source lacks are removed. Without `source` the working
//! tree is restored from the index and the index from `HEAD`. With `hunks`
//! only the named hunks of the selected files are taken back.

mod hunks;
mod stage;

use bstr::{BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_git::apply::{ApplyContext, ApplyPlan, PathSource, PathsRequest, plan_paths};

pub use stage::RestoreCommand;

/// Parameters of a restoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreRequest {
    /// Pathspecs relative to the working tree root; empty selects every
    /// path.
    pub paths: Vec<BString>,
    /// Revision whose tree is the source; `None` uses the index for the
    /// working tree and `HEAD` for the index.
    pub source: Option<String>,
    /// Restore the index.
    pub staged: bool,
    /// Restore the working tree.
    pub worktree: bool,
    /// Hunk identifiers as in `git.stage` (`path#number[:fingerprint][@rows]`);
    /// when present, only these hunks are taken back: those between the
    /// index and the working tree with `worktree`, those between `HEAD` and
    /// the index with `staged`. Each hunk must lie in a file of `paths`.
    pub hunks: Vec<String>,
}

/// Plans a restoration; the plan is executed with
/// [`apply_plan`](nurt_git::apply::apply_plan).
///
/// # Errors
///
/// Fails with `E_ARG` when neither `staged` nor `worktree` is set,
/// `E_GIT_REVISION` for an unknown source, `E_NO_MATCH` when a pathspec
/// matches nothing, `E_GIT_DIRTY` with the problem `unmerged` for paths with
/// conflict stages restored from the index, and `E_IO` when the repository
/// cannot be read. With `hunks`: `E_GIT_UNSUPPORTED` with the problem
/// `hunks_source` together with `source` and `hunks_both` when both sides are
/// restored, `E_ARG` with `hunk_outside_paths` for a hunk outside `paths` or
/// a malformed identifier, `E_NOT_FOUND` with `not_found` for a hunk or a
/// change that does not exist and `E_STALE` with `changed` for a fingerprint
/// that no longer matches.
pub fn plan_restore(ctx: &ApplyContext<'_>, request: &RestoreRequest) -> Result<ApplyPlan> {
    if !request.staged && !request.worktree {
        return Err(Error::new(
            ErrorCode::Arg,
            "nothing to restore; set staged: true, worktree: true or both",
        )
        .with_problem("no_target"));
    }
    if !request.hunks.is_empty() {
        return hunks::plan_hunks(ctx, request);
    }
    let source = match (&request.source, request.staged) {
        (Some(spec), _) => PathSource::Tree(tree_of(ctx.repo(), spec)?),
        (None, true) => PathSource::Tree(head_tree(ctx.repo())?),
        (None, false) => PathSource::Index,
    };
    plan_paths(
        ctx,
        &PathsRequest {
            source,
            pathspec: request.paths.clone(),
            to_index: request.staged,
            to_worktree: request.worktree,
        },
    )
}

fn head_tree(repo: &gix::Repository) -> Result<ObjectId> {
    match repo.head_id() {
        Ok(_) => tree_of(repo, "HEAD"),
        Err(_) => Ok(ObjectId::empty_tree(repo.object_hash())),
    }
}

fn tree_of(repo: &gix::Repository, spec: &str) -> Result<ObjectId> {
    let unknown = || {
        Error::new(
            ErrorCode::GitRevision,
            format!("could not resolve '{spec}' to a tree; give a commit, tag or tree that exists"),
        )
        .with_problem("unknown")
        .with_found(spec.to_owned())
    };
    let id = repo
        .rev_parse_single(spec.as_bytes().as_bstr())
        .map_err(|_| unknown())?;
    let object = id.object().map_err(|_| unknown())?;
    object
        .peel_to_tree()
        .map(|tree| tree.id)
        .map_err(|_| unknown())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix::object::tree::EntryKind;
    use nurt_contracts::journal::MutationTx;
    use nurt_git::apply::apply_plan;
    use nurt_git::apply::testjournal::{RecordingJournal, tree_state};

    struct Fixture {
        _dir: tempfile::TempDir,
        repo: gix::Repository,
        journal: RecordingJournal,
    }

    fn signature() -> gix::actor::Signature {
        gix::actor::Signature {
            name: "Test".into(),
            email: "test@example.com".into(),
            time: gix::date::Time::new(1_700_000_000, 0),
        }
    }

    fn commit(repo: &gix::Repository, files: &[(&str, &str)], parents: &[ObjectId]) -> ObjectId {
        let mut editor = repo.edit_tree(repo.empty_tree().id).expect("editor");
        for (path, content) in files {
            let blob = repo.write_blob(content.as_bytes()).expect("blob").detach();
            editor.upsert(*path, EntryKind::Blob, blob).expect("upsert");
        }
        let tree = editor.write().expect("tree").detach();
        let mut time = Default::default();
        let mut time2 = Default::default();
        repo.new_commit_as(
            signature().to_ref(&mut time),
            signature().to_ref(&mut time2),
            "c",
            tree,
            parents.iter().copied(),
        )
        .expect("commit")
        .id
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("temporary directory");
        let repo = gix::ThreadSafeRepository::init_opts(
            dir.path(),
            gix::create::Kind::WithWorktree,
            gix::create::Options::default(),
            gix::open::Options::isolated(),
        )
        .expect("init")
        .to_thread_local();
        let first = commit(&repo, &[("a.txt", "old\n"), ("c.txt", "c\n")], &[]);
        let second = commit(&repo, &[("a.txt", "new\n"), ("b.txt", "b\n")], &[first]);
        std::fs::write(
            repo.git_dir().join("refs/heads/main"),
            format!("{second}\n"),
        )
        .expect("ref");
        std::fs::write(repo.git_dir().join("HEAD"), "ref: refs/heads/main\n").expect("head");
        let journal = RecordingJournal::new();
        let ctx = ApplyContext::open(&repo).expect("context");
        let tree = repo
            .find_commit(second)
            .expect("commit")
            .tree_id()
            .expect("tree")
            .detach();
        let plan = nurt_git::apply::plan_new_tree(&ctx, &tree).expect("plan");
        let mut tx = journal.begin();
        apply_plan(&mut tx, &ctx, plan).expect("apply");
        Box::new(tx).commit().expect("commit");
        Fixture {
            _dir: dir,
            repo,
            journal,
        }
    }

    fn restore(fixture: &Fixture, request: &RestoreRequest) -> Result<()> {
        let ctx = ApplyContext::open(&fixture.repo)?;
        let plan = plan_restore(&ctx, request)?;
        let mut tx = fixture.journal.begin();
        apply_plan(&mut tx, &ctx, plan)?;
        Box::new(tx).commit()?;
        Ok(())
    }

    fn request(paths: &[&str]) -> RestoreRequest {
        RestoreRequest {
            paths: paths.iter().map(|path| BString::from(*path)).collect(),
            source: None,
            staged: false,
            worktree: true,
            hunks: Vec::new(),
        }
    }

    fn read(fixture: &Fixture, path: &str) -> Option<String> {
        std::fs::read_to_string(fixture.repo.workdir().expect("workdir").join(path)).ok()
    }

    #[test]
    fn restore_worktree_from_index_by_default() {
        let fixture = fixture();
        let root = fixture.repo.workdir().expect("workdir").to_path_buf();
        std::fs::write(root.join("a.txt"), "local\n").expect("write");
        restore(&fixture, &request(&["a.txt"])).expect("restore");
        assert_eq!(read(&fixture, "a.txt").as_deref(), Some("new\n"));
    }

    #[test]
    fn restore_from_older_revision_removes_paths_absent_there() {
        let fixture = fixture();
        let mut from_first = request(&[]);
        from_first.source = Some("HEAD~1".to_owned());
        restore(&fixture, &from_first).expect("restore");
        assert_eq!(read(&fixture, "a.txt").as_deref(), Some("old\n"));
        assert_eq!(read(&fixture, "b.txt"), None);
        assert_eq!(read(&fixture, "c.txt").as_deref(), Some("c\n"));
    }

    #[test]
    fn restore_staged_takes_index_from_head_and_keeps_worktree() {
        let fixture = fixture();
        let mut staged = request(&["a.txt"]);
        staged.source = Some("HEAD~1".to_owned());
        staged.staged = true;
        staged.worktree = false;
        restore(&fixture, &staged).expect("stage old content");
        staged.source = None;
        restore(&fixture, &staged).expect("unstage");
        let index = fixture.repo.open_index().expect("index");
        let entry = index
            .entry_by_path("a.txt".as_bytes().as_bstr())
            .expect("entry");
        assert_eq!(
            entry.id,
            fixture.repo.write_blob(b"new\n").expect("blob").detach()
        );
        assert_eq!(read(&fixture, "a.txt").as_deref(), Some("new\n"));
    }

    #[test]
    fn restore_rejects_empty_target_and_unknown_source() {
        let fixture = fixture();
        let ctx = ApplyContext::open(&fixture.repo).expect("context");
        let mut nothing = request(&["a.txt"]);
        nothing.worktree = false;
        assert_eq!(
            plan_restore(&ctx, &nothing).expect_err("no target").code(),
            ErrorCode::Arg
        );
        let mut unknown = request(&["a.txt"]);
        unknown.source = Some("missing-branch".to_owned());
        assert_eq!(
            plan_restore(&ctx, &unknown).expect_err("unknown").code(),
            ErrorCode::GitRevision
        );
        let unmatched = request(&["nothing-here"]);
        assert_eq!(
            plan_restore(&ctx, &unmatched).expect_err("no match").code(),
            ErrorCode::NoMatch
        );
    }

    #[test]
    fn undo_of_restore_restores_local_changes() {
        let fixture = fixture();
        let root = fixture.repo.workdir().expect("workdir").to_path_buf();
        std::fs::write(root.join("a.txt"), "precious\n").expect("write");
        let before = tree_state(&root);
        restore(&fixture, &request(&["a.txt"])).expect("restore");
        fixture.journal.undo_last();
        assert_eq!(tree_state(&root), before);
    }
}
