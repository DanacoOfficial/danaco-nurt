// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the `git.rebase` actions against repositories in temporary
//! directories, with a recording journal that can undo each action.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bstr::ByteSlice;
use gix::ObjectId;
use gix_index::entry::Stage;
use gix_object::tree::EntryKind;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{GitEffectState, GitInProgress};
use nurt_contracts::report::Notable;
use nurt_contracts::stage::{ArgValue, BoundArgs, StageData, StageImpl, StageInput};
use nurt_contracts::value::{Record, Value};
use nurt_git::apply::testjournal::{RecordingJournal, Snapshot, tree_state};
use nurt_git::apply::{ApplyContext, apply_plan, plan_new_tree};

use super::*;

/// Author time of fixture commits: 2026-01-01T12:00:00Z.
const TEST_TIME: i64 = 1_767_268_800;

/// Seconds between the author time and the commit time of replayed commits.
const COMMIT_DELAY: i64 = 100;

/// Content that marks a file as removed in a change list.
const GONE: &str = "\0gone";

const BASE: &[(&str, &str)] = &[("a.txt", "a\n"), ("f.txt", "1\n2\n3\n")];

type Changes<'a> = &'a [(&'a str, &'a str)];

fn committer() -> gix::actor::Signature {
    gix::actor::Signature {
        name: "Committer".into(),
        email: "committer@example.com".into(),
        time: gix::date::Time::new(TEST_TIME + COMMIT_DELAY, 0),
    }
}

struct Repo {
    dir: tempfile::TempDir,
    repo: gix::Repository,
    journal: RecordingJournal,
    files: BTreeMap<String, String>,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        let repo = gix::init(dir.path()).expect("repository");
        Self {
            dir,
            repo,
            journal: RecordingJournal::new(),
            files: BASE
                .iter()
                .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
                .collect(),
        }
    }

    /// Repository whose `main` has one commit `m1` with `main` changes
    /// over `base`, and whose `topic` has one commit per entry of `topic`;
    /// `topic` is checked out.
    fn diverged(main: Changes<'_>, topic: &[(&str, Changes<'_>)]) -> Self {
        let mut repo = Self::new();
        let base = repo.commit(None, &[], "base");
        let main_tip = if main.is_empty() {
            base
        } else {
            repo.commit(Some(base), main, "m1")
        };
        repo.reset_files();
        let mut tip = base;
        for (message, changes) in topic {
            tip = repo.commit(Some(tip), changes, message);
        }
        repo.write_git("refs/heads/main", &format!("{main_tip}\n"));
        repo.write_git("refs/heads/topic", &format!("{tip}\n"));
        repo.write_git("HEAD", "ref: refs/heads/topic\n");
        repo.checkout(tip);
        repo
    }

    fn reset_files(&mut self) {
        self.files = BASE
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect();
    }

    fn gix(&self) -> &gix::Repository {
        &self.repo
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn label(&self) -> String {
        self.root().display().to_string()
    }

    fn tree(&self, files: &BTreeMap<String, String>) -> ObjectId {
        let mut editor = gix_object::tree::Editor::new(
            gix_object::Tree::empty(),
            &self.repo.objects,
            gix_hash::Kind::Sha1,
        );
        for (path, text) in files {
            let id = self
                .repo
                .write_blob(text.as_bytes())
                .expect("blob")
                .detach();
            editor
                .upsert(path.split('/'), EntryKind::Blob, id)
                .expect("tree entry");
        }
        editor
            .write(|tree| self.repo.write_object(tree).map(gix::Id::detach))
            .expect("tree")
    }

    /// Commits the current files with `changes` applied on `parent`.
    fn commit(
        &mut self,
        parent: Option<ObjectId>,
        changes: Changes<'_>,
        message: &str,
    ) -> ObjectId {
        for (path, text) in changes {
            if *text == GONE {
                self.files.remove(*path);
            } else {
                self.files.insert((*path).to_owned(), (*text).to_owned());
            }
        }
        let author = gix::actor::Signature {
            name: "Author".into(),
            email: "author@example.com".into(),
            time: gix::date::Time::new(TEST_TIME, 3600),
        };
        let commit = gix_object::Commit {
            tree: self.tree(&self.files),
            parents: parent.into_iter().collect(),
            author: author.clone(),
            committer: author,
            encoding: None,
            message: format!("{message}\n").into(),
            extra_headers: Vec::new(),
        };
        self.repo.write_object(&commit).expect("commit").detach()
    }

    fn git_path(&self, name: &str) -> PathBuf {
        self.repo.git_dir().join(name)
    }

    fn write_git(&self, name: &str, content: &str) {
        let path = self.git_path(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("directories");
        std::fs::write(path, content).expect("git file");
    }

    fn read_git(&self, name: &str) -> String {
        std::fs::read_to_string(self.git_path(name)).expect("git file")
    }

    fn has_git(&self, name: &str) -> bool {
        self.git_path(name).exists()
    }

    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.root().join(path)).expect("working tree file")
    }

    fn write(&self, path: &str, content: &str) {
        std::fs::write(self.root().join(path), content).expect("working tree file");
    }

    fn checkout(&self, commit: ObjectId) {
        let tree = self.tree_of(commit);
        let ctx = ApplyContext::open(self.gix()).expect("context");
        let plan = plan_new_tree(&ctx, &tree).expect("checkout plan");
        let mut tx = self.journal.begin();
        apply_plan(&mut tx, &ctx, plan).expect("checkout");
        Box::new(tx).commit().expect("commit");
    }

    fn tree_of(&self, commit: ObjectId) -> ObjectId {
        self.gix()
            .find_commit(commit)
            .expect("commit")
            .tree_id()
            .expect("tree")
            .detach()
    }

    /// Replaces the index with the entries of `files`.
    fn stage(&self, files: &[(&str, &str)]) {
        let map = files
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect();
        let tree = self.tree(&map);
        let state = gix_index::State::from_tree(&tree, &self.gix().objects, Default::default())
            .expect("index from tree");
        let path = self.git_path("index");
        let mut bytes = Vec::new();
        gix_index::File::from_state(state, path.clone())
            .write_to(&mut bytes, gix_index::write::Options::default())
            .expect("serialize index");
        std::fs::write(path, bytes).expect("index");
    }

    fn stages(&self, path: &str) -> Vec<Stage> {
        let ctx = ApplyContext::open(self.gix()).expect("context");
        let index = ctx.index();
        index
            .entries()
            .iter()
            .filter(|entry| entry.path(index) == path.as_bytes().as_bstr())
            .map(|entry| entry.stage())
            .collect()
    }

    fn reference(&self, name: &str) -> ObjectId {
        ObjectId::from_hex(self.read_git(name).trim().as_bytes()).expect("reference")
    }

    /// Messages of a reference log, oldest first.
    fn log(&self, name: &str) -> Vec<String> {
        self.read_git(name)
            .lines()
            .filter_map(|line| line.split_once('\t').map(|(_, message)| message.to_owned()))
            .collect()
    }

    /// Subjects and author lines of the first-parent history of `tip`,
    /// newest first.
    fn history(&self, tip: ObjectId) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut next = Some(tip);
        while let Some(id) = next {
            let commit = self.gix().find_commit(id).expect("commit");
            let decoded = commit.decode().expect("decoded");
            out.push((
                decoded.message.to_str_lossy().trim_end().to_owned(),
                decoded
                    .author()
                    .expect("author")
                    .name
                    .to_str_lossy()
                    .into_owned(),
            ));
            next = decoded.parents().next();
        }
        out
    }

    /// Working tree and Git directory without the object database.
    fn state(&self) -> BTreeMap<PathBuf, Snapshot> {
        tree_state(self.root())
            .into_iter()
            .filter(|(path, _)| !path.starts_with(".git/objects"))
            .collect()
    }

    fn request(&self, action: RebaseAction) -> RebaseRequest {
        RebaseRequest::new(self.label(), action, committer())
    }

    fn start(&self, revision: &str) -> RebaseRequest {
        let onto = RebaseOnto::resolve(self.gix(), &self.label(), revision).expect("onto");
        RebaseRequest::start(self.label(), onto, committer())
    }

    fn run(&self, request: &RebaseRequest) -> Result<RebaseResult> {
        let fresh = gix::open(self.root()).expect("reopen");
        let ctx = ApplyContext::open(&fresh)?;
        let plan = plan_rebase(&ctx, request)?;
        let mut tx = self.journal.begin();
        let result = apply_rebase(&mut tx, &ctx, plan)?;
        Box::new(tx).commit()?;
        Ok(result)
    }
}

/// `main` changes `a.txt`; `topic` adds `b.txt`, changes `f.txt` and adds
/// `c.txt`; with `conflict` `main` also changes the line of `f.txt` that
/// `topic` changes.
fn three_commits(conflict: bool) -> Repo {
    let main: Changes<'_> = if conflict {
        &[("a.txt", "a main\n"), ("f.txt", "1\nmain\n3\n")]
    } else {
        &[("a.txt", "a main\n")]
    };
    Repo::diverged(
        main,
        &[
            ("t1", &[("b.txt", "b\n")]),
            ("t2", &[("f.txt", "1\ntopic\n3\n")]),
            ("t3", &[("c.txt", "c\n")]),
        ],
    )
}

fn subjects(repo: &Repo, tip: ObjectId) -> Vec<String> {
    repo.history(tip)
        .into_iter()
        .map(|(subject, _)| subject)
        .collect()
}

#[test]
fn linear_rebase_replays_commits_onto_the_new_base() {
    let repo = three_commits(false);
    let result = repo.run(&repo.start("main")).expect("rebase");
    let summary = &result.summary;
    assert_eq!(summary.state, GitEffectState::Completed);
    assert_eq!(summary.commits.len(), 3);
    let tip = repo.reference("refs/heads/topic");
    assert_eq!(summary.head_after, Some(tip));
    assert_eq!(subjects(&repo, tip), vec!["t3", "t2", "t1", "m1", "base"]);
    assert!(
        repo.history(tip)
            .iter()
            .all(|(_, author)| author == "Author")
    );
    assert_eq!(repo.read("a.txt"), "a main\n");
    assert_eq!(repo.read("f.txt"), "1\ntopic\n3\n");
    assert_eq!(repo.read("c.txt"), "c\n");
    let commit = repo.gix().find_commit(tip).expect("commit");
    let decoded = commit.decode().expect("decoded");
    assert_eq!(decoded.committer().expect("committer").name, "Committer");
    assert_eq!(
        decoded
            .author()
            .expect("author")
            .time()
            .expect("time")
            .offset,
        3600
    );
    assert!(
        summary
            .notes
            .iter()
            .any(|note| note.code == Notable::HistoryRewritten)
    );
}

#[test]
fn linear_rebase_writes_the_logs_of_git_and_undoes_byte_for_byte() {
    let repo = three_commits(false);
    let before = repo.state();
    let orig = repo.reference("refs/heads/topic");
    let main = repo.reference("refs/heads/main");
    repo.run(&repo.start("main")).expect("rebase");
    assert_eq!(repo.read_git("HEAD"), "ref: refs/heads/topic\n");
    assert_eq!(repo.read_git("ORIG_HEAD"), format!("{orig}\n"));
    assert!(!repo.has_git("rebase-merge"));
    assert!(!repo.has_git("REBASE_HEAD"));
    assert_eq!(
        repo.log("logs/HEAD"),
        vec![
            "rebase (start): checkout main".to_owned(),
            "rebase (pick): t1".to_owned(),
            "rebase (pick): t2".to_owned(),
            "rebase (pick): t3".to_owned(),
            "rebase (finish): returning to refs/heads/topic".to_owned(),
        ]
    );
    assert_eq!(
        repo.log("logs/refs/heads/topic"),
        vec![format!("rebase (finish): refs/heads/topic onto {main}")]
    );
    repo.journal.undo_last();
    assert_eq!(repo.state(), before);
}

#[test]
fn rebase_onto_an_ancestor_without_merges_is_up_to_date() {
    let repo = three_commits(false);
    let before = repo.state();
    let base = repo.history(repo.reference("refs/heads/topic")).len();
    assert_eq!(base, 4);
    let result = repo.run(&repo.start("topic~3")).expect("rebase");
    assert_eq!(result.summary.state, GitEffectState::UpToDate);
    assert_eq!(repo.state(), before);
}

#[test]
fn commit_already_in_the_new_base_is_not_replayed() {
    let repo = Repo::diverged(
        &[("b.txt", "b\n")],
        &[("t1", &[("b.txt", "b\n")]), ("t2", &[("c.txt", "c\n")])],
    );
    let result = repo.run(&repo.start("main")).expect("rebase");
    assert_eq!(result.summary.commits.len(), 1);
    assert_eq!(result.summary.dropped.len(), 1);
    let subjects: Vec<String> = repo
        .history(repo.reference("refs/heads/topic"))
        .into_iter()
        .map(|(subject, _)| subject)
        .collect();
    assert_eq!(subjects, vec!["t2", "m1", "base"]);
}

#[test]
fn commit_that_becomes_empty_is_dropped_and_empty_commit_is_kept() {
    let repo = Repo::diverged(
        &[("f.txt", "1\nX\n3\nmore\n")],
        &[
            ("empty", &[]),
            ("becomes empty", &[("f.txt", "1\nX\n3\n")]),
            ("real", &[("t.txt", "t\n")]),
        ],
    );
    let result = repo.run(&repo.start("main")).expect("rebase");
    let subjects: Vec<String> = repo
        .history(repo.reference("refs/heads/topic"))
        .into_iter()
        .map(|(subject, _)| subject)
        .collect();
    assert_eq!(subjects, vec!["real", "empty", "m1", "base"]);
    assert_eq!(result.summary.dropped.len(), 1);
    assert!(
        !repo
            .log("logs/HEAD")
            .iter()
            .any(|line| line.contains("becomes empty"))
    );
}

/// Second commit of `topic` in [`three_commits`].
fn second(repo: &Repo) -> (ObjectId, String) {
    let id = repo.gix().rev_parse_single("topic~1").expect("t2").detach();
    (id, id.to_hex_with_len(7).to_string())
}

#[test]
fn conflict_stops_with_a_detached_head_and_conflict_stages() {
    let repo = three_commits(true);
    let before = repo.state();
    let orig = repo.reference("refs/heads/topic");
    let result = repo.run(&repo.start("main")).expect("rebase");
    let summary = &result.summary;
    assert_eq!(summary.state, GitEffectState::Conflicts);
    assert_eq!(summary.step, Some(RebaseStep { done: 2, total: 3 }));
    let error = summary.stop_error(&repo.label()).expect("conflict error");
    assert_eq!(error.code(), ErrorCode::GitConflict);
    assert!(error.data().effect_applied);
    let effect = summary.effect(repo.root());
    assert_eq!(effect.in_progress, Some(GitInProgress::Rebase));
    assert_eq!(repo.reference("refs/heads/topic"), orig);
    assert_eq!(repo.history(repo.reference("HEAD"))[0].0, "t1");
    let (_, short) = second(&repo);
    assert_eq!(
        repo.read("f.txt"),
        format!("1\n<<<<<<< HEAD\nmain\n=======\ntopic\n>>>>>>> {short} (t2)\n3\n")
    );
    assert_eq!(repo.read("b.txt"), "b\n");
    assert!(!repo.root().join("c.txt").exists());
    assert_eq!(
        repo.stages("f.txt"),
        vec![Stage::Base, Stage::Ours, Stage::Theirs]
    );
    repo.journal.undo_last();
    assert_eq!(repo.state(), before);
}

#[test]
fn conflict_writes_the_state_directory_of_git() {
    let repo = three_commits(true);
    let orig = repo.reference("refs/heads/topic");
    repo.run(&repo.start("main")).expect("rebase");
    let (t2, _) = second(&repo);
    let names: std::collections::BTreeSet<String> =
        std::fs::read_dir(repo.git_path("rebase-merge"))
            .expect("state directory")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
    let expected = [
        "author-script",
        "done",
        "drop_redundant_commits",
        "end",
        "git-rebase-todo",
        "git-rebase-todo.backup",
        "head-name",
        "interactive",
        "message",
        "msgnum",
        "no-reschedule-failed-exec",
        "onto",
        "orig-head",
        "patch",
        "rewritten-list",
        "stopped-sha",
    ];
    assert_eq!(
        names,
        expected.iter().map(|name| (*name).to_owned()).collect()
    );
    let file = |name: &str| repo.read_git(&format!("rebase-merge/{name}"));
    assert_eq!(file("head-name"), "refs/heads/topic\n");
    assert_eq!(file("msgnum"), "2\n");
    assert_eq!(file("end"), "3\n");
    assert_eq!(file("stopped-sha"), format!("{t2}\n"));
    assert_eq!(file("orig-head"), format!("{orig}\n"));
    assert_eq!(
        file("author-script"),
        "GIT_AUTHOR_NAME='Author'\nGIT_AUTHOR_EMAIL='author@example.com'\nGIT_AUTHOR_DATE='@1767268800 +0100'\n"
    );
    let message = "t2\n\n# Conflicts:\n#\tf.txt\n";
    assert_eq!(file("message"), message);
    assert_eq!(repo.read_git("MERGE_MSG"), message);
    assert_eq!(repo.read_git("REBASE_HEAD"), format!("{t2}\n"));
    assert!(file("patch").contains("-2\n+topic\n"));
    assert!(file("git-rebase-todo").ends_with(" # t3\n"));
    assert!(repo.has_git("AUTO_MERGE"));
    assert_eq!(
        repo.log("logs/HEAD"),
        vec![
            "rebase (start): checkout main".to_owned(),
            "rebase (pick): t1".to_owned()
        ]
    );
}

#[test]
fn continue_commits_the_resolution_and_finishes() {
    let repo = three_commits(true);
    repo.run(&repo.start("main")).expect("stop");
    repo.write("f.txt", "1\nresolved\n3\n");
    repo.stage(&[
        ("a.txt", "a main\n"),
        ("b.txt", "b\n"),
        ("f.txt", "1\nresolved\n3\n"),
    ]);
    let before = repo.state();
    let result = repo
        .run(&repo.request(RebaseAction::Continue))
        .expect("continue");
    assert_eq!(result.summary.state, GitEffectState::Completed);
    let tip = repo.reference("refs/heads/topic");
    let subjects: Vec<String> = repo.history(tip).into_iter().map(|(s, _)| s).collect();
    assert_eq!(subjects, vec!["t3", "t2", "t1", "m1", "base"]);
    assert_eq!(repo.read("f.txt"), "1\nresolved\n3\n");
    assert_eq!(repo.read("c.txt"), "c\n");
    assert_eq!(repo.read_git("HEAD"), "ref: refs/heads/topic\n");
    assert!(!repo.has_git("rebase-merge"));
    assert!(!repo.has_git("MERGE_MSG"));
    let log = repo.log("logs/HEAD");
    assert_eq!(
        log[log.len() - 3..].to_vec(),
        vec![
            "rebase (continue): t2".to_owned(),
            "rebase (pick): t3".to_owned(),
            "rebase (finish): returning to refs/heads/topic".to_owned(),
        ]
    );
    repo.journal.undo_last();
    assert_eq!(repo.state(), before);
}

#[test]
fn continue_refuses_unresolved_conflicts_and_unstaged_changes() {
    let repo = three_commits(true);
    repo.run(&repo.start("main")).expect("stop");
    let before = repo.state();
    let error = repo
        .run(&repo.request(RebaseAction::Continue))
        .expect_err("unmerged");
    assert_eq!(error.code(), ErrorCode::GitConflict);
    assert!(!error.data().effect_applied);
    repo.stage(&[
        ("a.txt", "a main\n"),
        ("b.txt", "b\n"),
        ("f.txt", "1\nresolved\n3\n"),
    ]);
    repo.write("f.txt", "1\nedited\n3\n");
    let error = repo
        .run(&repo.request(RebaseAction::Continue))
        .expect_err("unstaged");
    assert_eq!(error.code(), ErrorCode::GitDirty);
    assert_ne!(repo.state(), before);
    assert!(repo.has_git("rebase-merge"));
}

#[test]
fn skip_drops_the_stopped_commit() {
    let repo = three_commits(true);
    repo.run(&repo.start("main")).expect("stop");
    let before = repo.state();
    let result = repo.run(&repo.request(RebaseAction::Skip)).expect("skip");
    assert_eq!(result.summary.state, GitEffectState::Completed);
    let subjects: Vec<String> = repo
        .history(repo.reference("refs/heads/topic"))
        .into_iter()
        .map(|(subject, _)| subject)
        .collect();
    assert_eq!(subjects, vec!["t3", "t1", "m1", "base"]);
    assert_eq!(repo.read("f.txt"), "1\nmain\n3\n");
    assert!(
        repo.stages("f.txt")
            .iter()
            .all(|stage| *stage == Stage::Unconflicted)
    );
    assert!(!repo.has_git("REBASE_HEAD"));
    repo.journal.undo_last();
    assert_eq!(repo.state(), before);
}

#[test]
fn abort_returns_to_the_original_branch() {
    let repo = three_commits(true);
    let orig = repo.reference("refs/heads/topic");
    repo.run(&repo.start("main")).expect("stop");
    let before = repo.state();
    let result = repo.run(&repo.request(RebaseAction::Abort)).expect("abort");
    assert_eq!(result.summary.head_after, Some(orig));
    assert_eq!(repo.read_git("HEAD"), "ref: refs/heads/topic\n");
    assert_eq!(repo.read("f.txt"), "1\ntopic\n3\n");
    assert_eq!(repo.read("a.txt"), "a\n");
    assert!(!repo.has_git("rebase-merge"));
    assert!(!repo.has_git("REBASE_HEAD"));
    assert!(!repo.has_git("AUTO_MERGE"));
    assert_eq!(
        repo.log("logs/HEAD").last().map(String::as_str),
        Some("rebase (abort): returning to refs/heads/topic")
    );
    repo.journal.undo_last();
    assert_eq!(repo.state(), before);
}

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the fixture adds a submodule entry by writing the index directly, outside a transaction"
)]
fn changed_submodule_is_not_an_unstaged_change() {
    let repo = three_commits(false);
    let sub = repo.root().join("sub");
    let inner = gix::init(&sub).expect("nested repository");
    let author = committer();
    let inner_commit = gix_object::Commit {
        tree: inner.empty_tree().id,
        parents: Default::default(),
        author: author.clone(),
        committer: author,
        encoding: None,
        message: "inner\n".into(),
        extra_headers: Vec::new(),
    };
    let inner_head = inner.write_object(&inner_commit).expect("commit").detach();
    std::fs::write(sub.join(".git").join("HEAD"), format!("{inner_head}\n")).expect("HEAD");
    repo.write(
        ".gitmodules",
        "[submodule \"sub\"]\n\tpath = sub\n\turl = https://example.com/sub.git\n",
    );
    let recorded = repo.reference("refs/heads/main");
    let mut index = repo.gix().open_index().expect("index");
    index.dangerously_push_entry(
        Default::default(),
        recorded,
        gix::index::entry::Flags::empty(),
        gix::index::entry::Mode::COMMIT,
        "sub".into(),
    );
    index.sort_entries();
    index.write(Default::default(), Default::default()).expect("index written");
    let fresh = gix::open(repo.root()).expect("reopen");
    let unstaged = super::command::unstaged_paths(&fresh).expect("status");
    assert!(
        !unstaged.iter().any(|path| path == "sub"),
        "a submodule was reported as unstaged: {unstaged:?}"
    );
}

#[test]
fn uncommitted_changes_refuse_the_rebase() {
    let repo = three_commits(false);
    repo.write("a.txt", "local\n");
    let before = repo.state();
    let error = repo.run(&repo.start("main")).expect_err("unstaged");
    assert_eq!(error.code(), ErrorCode::GitDirty);
    assert_eq!(error.data().problem.as_deref(), Some("unstaged_changes"));
    assert_eq!(error.data().sample, vec!["a.txt".to_owned()]);
    assert_eq!(repo.state(), before);
    repo.write("a.txt", "a\n");
    repo.stage(&[
        ("a.txt", "a\n"),
        ("b.txt", "b\n"),
        ("c.txt", "c\n"),
        ("f.txt", "1\ntopic\n3\n"),
        ("n.txt", "n\n"),
    ]);
    repo.write("n.txt", "n\n");
    let error = repo.run(&repo.start("main")).expect_err("staged");
    assert_eq!(error.data().problem.as_deref(), Some("staged_changes"));
}

#[test]
fn untracked_file_in_the_way_of_the_new_base_is_refused() {
    let repo = Repo::diverged(&[("n.txt", "main\n")], &[("t1", &[("b.txt", "b\n")])]);
    repo.write("n.txt", "untracked\n");
    let before = repo.state();
    let error = repo.run(&repo.start("main")).expect_err("untracked");
    assert_eq!(error.code(), ErrorCode::GitDirty);
    assert_eq!(
        error.data().problem.as_deref(),
        Some("untracked_overwritten")
    );
    assert_eq!(repo.state(), before);
}

#[test]
fn untracked_file_stops_a_pick_that_skip_runs_again() {
    let repo = Repo::diverged(
        &[("a.txt", "a main\n")],
        &[("t1", &[("x.txt", "x\n")]), ("t2", &[("x.txt", GONE)])],
    );
    repo.write("x.txt", "untracked\n");
    let result = repo.run(&repo.start("main")).expect("stop");
    assert_eq!(result.summary.state, GitEffectState::Stopped);
    assert_eq!(result.summary.blocked, vec![bstr::BString::from("x.txt")]);
    let error = result
        .summary
        .stop_error(&repo.label())
        .expect("stop error");
    assert_eq!(error.code(), ErrorCode::GitDirty);
    assert!(!repo.has_git("rebase-merge/stopped-sha"));
    assert!(!repo.has_git("rebase-merge/message"));
    assert!(repo.has_git("REBASE_HEAD"));
    let todo = repo.read_git("rebase-merge/git-rebase-todo");
    assert_eq!(todo.lines().count(), 2);
    assert!(todo.starts_with(&format!(
        "pick {} # t1\n",
        repo.read_git("REBASE_HEAD").trim()
    )));
    assert_eq!(repo.read("x.txt"), "untracked\n");
    std::fs::remove_file(repo.root().join("x.txt")).expect("remove");
    let result = repo.run(&repo.request(RebaseAction::Skip)).expect("skip");
    assert_eq!(result.summary.state, GitEffectState::Completed);
    assert_eq!(result.summary.commits.len(), 2);
    assert!(!repo.root().join("x.txt").exists());
}

#[test]
fn actions_need_their_preconditions() {
    let repo = three_commits(false);
    for action in [
        RebaseAction::Continue,
        RebaseAction::Skip,
        RebaseAction::Abort,
    ] {
        let error = repo.run(&repo.request(action)).expect_err("no rebase");
        assert_eq!(error.code(), ErrorCode::GitInProgress);
    }
    let error = repo
        .run(&repo.request(RebaseAction::Start))
        .expect_err("no onto");
    assert_eq!(error.code(), ErrorCode::MissingParam);
    let error = RebaseOnto::resolve(repo.gix(), &repo.label(), "nothing").expect_err("revision");
    assert_eq!(error.code(), ErrorCode::GitRevision);
    let config = repo.read_git("config");
    repo.write_git("config", &format!("{config}[commit]\n\tgpgSign = true\n"));
    let error = repo.run(&repo.start("main")).expect_err("signing");
    assert_eq!(error.code(), ErrorCode::GitSign);
    repo.write_git(
        "config",
        &format!("{config}[rebase]\n\tupdateRefs = true\n"),
    );
    let error = repo.run(&repo.start("main")).expect_err("update refs");
    assert_eq!(error.code(), ErrorCode::GitUnsupported);
}

#[test]
fn rebase_while_stopped_is_refused() {
    let repo = three_commits(true);
    repo.run(&repo.start("main")).expect("stop");
    let error = repo.run(&repo.start("main")).expect_err("in progress");
    assert_eq!(error.code(), ErrorCode::GitInProgress);
}

#[test]
fn detached_head_is_rebased_without_a_branch() {
    let repo = three_commits(false);
    let tip = repo.reference("refs/heads/topic");
    repo.write_git("HEAD", &format!("{tip}\n"));
    let result = repo.run(&repo.start("main")).expect("rebase");
    assert_eq!(result.summary.branch, None);
    let head = repo.reference("HEAD");
    assert_ne!(head, tip);
    assert_eq!(repo.reference("refs/heads/topic"), tip);
    assert!(
        !repo
            .log("logs/HEAD")
            .iter()
            .any(|line| line.contains("finish"))
    );
}

#[test]
fn branch_behind_the_new_base_fast_forwards() {
    let repo = Repo::diverged(&[("a.txt", "a main\n")], &[]);
    let main = repo.reference("refs/heads/main");
    let result = repo.run(&repo.start("main")).expect("rebase");
    assert_eq!(result.summary.state, GitEffectState::FastForward);
    assert_eq!(repo.reference("refs/heads/topic"), main);
    assert_eq!(repo.read("a.txt"), "a main\n");
    assert_eq!(
        repo.log("logs/HEAD"),
        vec![
            "rebase (start): checkout main".to_owned(),
            "rebase (finish): returning to refs/heads/topic".to_owned(),
        ]
    );
}

#[test]
fn reflog_action_of_the_caller_prefixes_the_logs() {
    let repo = three_commits(false);
    let mut request = repo.start("main");
    request.reflog_action = Some("pull --rebase".to_owned());
    repo.run(&request).expect("rebase");
    assert_eq!(
        repo.log("logs/HEAD").first().map(String::as_str),
        Some("pull --rebase (start): checkout main")
    );
}

fn stage_args(repo: &Repo, extra: &[(&str, &str)]) -> BoundArgs {
    let mut args = BoundArgs::new("git.rebase").with(
        "repo",
        ArgValue::Value(Value::Path(repo.label())),
        false,
    );
    for (name, value) in extra {
        args = args.with(
            *name,
            ArgValue::Value(Value::Text((*value).to_owned())),
            false,
        );
    }
    args
}

fn run_stage(repo: &Repo, extra: &[(&str, &str)]) -> Result<Vec<Record>> {
    let ctx = nurt_git::testutil::test_ctx();
    let args = stage_args(repo, extra);
    let stage = RebaseCommand;
    let effect = stage.as_effect().expect("effect stage");
    let plan = effect.plan(&args, StageInput::none(), &ctx)?;
    let mut tx = repo.journal.begin();
    let outcome = match effect.apply(&args, plan, &mut tx, &ctx) {
        Ok(outcome) => outcome,
        Err(error) => {
            if error.data().effect_applied {
                Box::new(tx).commit()?;
            }
            return Err(error);
        }
    };
    Box::new(tx).commit()?;
    match outcome.output.data {
        StageData::Records(stream) => stream.collect(),
        _ => Err(nurt_contracts::error::Error::new(
            ErrorCode::StageFailed,
            "the stage produced no records",
        )),
    }
}

#[test]
fn stage_rebases_and_reports_the_effect() {
    let repo = three_commits(false);
    assert_eq!(RebaseCommand.descriptor().command, "git.rebase".to_owned());
    let records = run_stage(&repo, &[("onto", "main")]).expect("rebase");
    assert_eq!(records.len(), 1);
    assert_eq!(repo.read("a.txt"), "a main\n");
}

#[test]
fn stage_reports_a_stop_as_an_applied_error_and_continues() {
    let repo = three_commits(true);
    let error = run_stage(&repo, &[("onto", "main")]).expect_err("conflict");
    assert_eq!(error.code(), ErrorCode::GitConflict);
    assert!(error.data().effect_applied);
    assert!(repo.has_git("rebase-merge"));
    let error = run_stage(&repo, &[("action", "rewind")]).expect_err("unknown action");
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    run_stage(&repo, &[("action", "abort")]).expect("abort");
    assert!(!repo.has_git("rebase-merge"));
}
