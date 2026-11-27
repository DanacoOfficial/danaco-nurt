// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::merge::tests_command
// Responsibility:   Tests of the `git.merge` actions against repositories in temporary.
// Dependencies:     bstr, gix_hash, gix_index, nurt_contracts, nurt_git
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the `git.merge` actions against repositories in temporary
//! directories, with a recording journal that can undo each action.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bstr::ByteSlice;
use gix_hash::ObjectId;
use gix_index::entry::Stage;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{GitEffectState, GitInProgress};
use nurt_contracts::report::Notable;
use nurt_contracts::stage::StageImpl;
use nurt_git::apply::testjournal::{RecordingJournal, Snapshot, tree_state};
use nurt_git::apply::{ApplyContext, apply_plan, plan_new_tree};

use super::fixture::{Fixture, TEST_TIME};
use super::*;

/// Content that marks a file as removed in [`files`].
const GONE: &str = "\0gone";

const BASE: &[(&str, &str)] = &[("a.txt", "a\n"), ("f.txt", "1\n2\n3\n"), ("u.txt", "u\n")];

fn signature() -> gix::actor::Signature {
    gix::actor::Signature {
        name: "Merger".into(),
        email: "merger@example.com".into(),
        time: gix::date::Time::new(TEST_TIME, 0),
    }
}

/// The base files with `changes` applied.
fn files(changes: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut out: BTreeMap<&str, &str> = BASE.iter().copied().collect();
    for (path, content) in changes {
        if *content == GONE {
            out.remove(path);
        } else {
            out.insert(path, content);
        }
    }
    out.into_iter().collect()
}

struct Repo {
    fixture: Fixture,
    journal: RecordingJournal,
    base: ObjectId,
    main: ObjectId,
    topic: ObjectId,
}

impl Repo {
    /// Repository with `main` and `topic` branching from a base commit;
    /// an empty change list keeps the branch at the base. `main` is
    /// checked out.
    fn new(main: &[(&'static str, &'static str)], topic: &[(&'static str, &'static str)]) -> Self {
        let fixture = Fixture::new();
        let base = fixture.commit(fixture.tree(BASE), &[], "base");
        let side = |changes: &[(&'static str, &'static str)], name: &str| {
            if changes.is_empty() {
                base
            } else {
                fixture.commit(fixture.tree(&files(changes)), &[base], name)
            }
        };
        let main = side(main, "main");
        let topic = side(topic, "topic");
        let repo = Self {
            fixture,
            journal: RecordingJournal::new(),
            base,
            main,
            topic,
        };
        repo.write_git("refs/heads/main", &format!("{main}\n"));
        repo.write_git("refs/heads/topic", &format!("{topic}\n"));
        repo.write_git("HEAD", "ref: refs/heads/main\n");
        repo.checkout(main);
        repo
    }

    fn gix(&self) -> &gix::Repository {
        &self.fixture.repo
    }

    fn root(&self) -> &Path {
        self.fixture.workdir()
    }

    fn label(&self) -> String {
        self.root().display().to_string()
    }

    fn git_path(&self, name: &str) -> PathBuf {
        self.gix().git_dir().join(name)
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

    /// Replaces the index with the entries of `tree`.
    fn stage_tree(&self, tree: ObjectId) {
        let state = gix_index::State::from_tree(&tree, &self.gix().objects, Default::default())
            .expect("index from tree");
        let path = self.git_path("index");
        let mut bytes = Vec::new();
        gix_index::File::from_state(state, path.clone())
            .write_to(&mut bytes, gix_index::write::Options::default())
            .expect("serialize index");
        std::fs::write(path, bytes).expect("index");
    }

    fn index(&self) -> gix_index::State {
        ApplyContext::open(self.gix())
            .expect("context")
            .index()
            .clone()
    }

    fn stages(&self, path: &str) -> Vec<Stage> {
        let index = self.index();
        index
            .entries()
            .iter()
            .filter(|entry| entry.path(&index) == path.as_bytes().as_bstr())
            .map(|entry| entry.stage())
            .collect()
    }

    fn branch(&self) -> ObjectId {
        ObjectId::from_hex(self.read_git("refs/heads/main").trim().as_bytes()).expect("branch")
    }

    fn commit_info(&self, id: ObjectId) -> (Vec<ObjectId>, String, ObjectId) {
        let commit = self.gix().find_object(id).expect("commit").into_commit();
        let decoded = commit.decode().expect("decoded commit");
        (
            decoded.parents().collect(),
            decoded.message.to_str_lossy().into_owned(),
            decoded.tree(),
        )
    }

    /// Working tree and Git directory without the object database, whose
    /// additions are exempt from the journal.
    fn state(&self) -> BTreeMap<PathBuf, Snapshot> {
        tree_state(self.root())
            .into_iter()
            .filter(|(path, _)| !path.starts_with(".git/objects"))
            .collect()
    }

    fn start(&self, revision: &str) -> MergeRequest {
        let target = MergeTarget::resolve(self.gix(), revision).expect("target");
        MergeRequest::start(self.label(), target, signature())
    }

    fn action(&self, action: MergeAction) -> MergeRequest {
        MergeRequest::new(self.label(), action, signature())
    }

    fn run(&self, request: &MergeRequest) -> Result<MergeResult> {
        let ctx = ApplyContext::open(self.gix())?;
        let plan = plan_merge(&ctx, request)?;
        let mut tx = self.journal.begin();
        let result = apply_merge(&mut tx, &ctx, plan)?;
        Box::new(tx).commit()?;
        Ok(result)
    }

    fn log_ends_with(&self, name: &str, message: &str) -> bool {
        self.read_git(name).ends_with(&format!("\t{message}\n"))
    }
}

fn hex_line(id: ObjectId) -> String {
    format!("{id}\n")
}

fn conflicting() -> Repo {
    Repo::new(&[("f.txt", "1\nmain\n3\n")], &[("f.txt", "1\ntopic\n3\n")])
}

fn diverged() -> Repo {
    Repo::new(&[("a.txt", "a main\n")], &[("b.txt", "b\n")])
}

#[test]
fn fast_forward_moves_branch_working_tree_and_logs() {
    let repo = Repo::new(&[], &[("b.txt", "b\n")]);
    let result = repo.run(&repo.start("topic")).expect("merge");
    assert_eq!(result.state, GitEffectState::FastForward);
    assert_eq!(result.head_after, Some(repo.topic));
    assert_eq!(repo.branch(), repo.topic);
    assert_eq!(repo.read("b.txt"), "b\n");
    assert_eq!(repo.read_git("ORIG_HEAD"), hex_line(repo.base));
    assert!(repo.log_ends_with("logs/refs/heads/main", "merge topic: Fast-forward"));
    assert!(repo.log_ends_with("logs/HEAD", "merge topic: Fast-forward"));
    assert!(
        result
            .notes
            .iter()
            .any(|note| note.code == Notable::FastForward)
    );
    assert!(!repo.has_git("MERGE_HEAD"));
}

#[test]
fn fast_forward_only_refuses_diverged_history_without_writing() {
    let repo = diverged();
    let before = repo.state();
    let mut request = repo.start("topic");
    request.fast_forward = FastForward::Only;
    let error = repo.run(&request).expect_err("diverged");
    assert_eq!(error.code(), ErrorCode::GitNonFastForward);
    assert_eq!(error.data().problem.as_deref(), Some("not_possible"));
    assert_eq!(repo.state(), before);
}

#[test]
fn merging_an_ancestor_is_up_to_date() {
    let repo = Repo::new(&[("a.txt", "a main\n")], &[]);
    let result = repo.run(&repo.start("topic")).expect("merge");
    assert_eq!(result.state, GitEffectState::UpToDate);
    assert_eq!(repo.branch(), repo.main);
    assert_eq!(repo.read_git("ORIG_HEAD"), hex_line(repo.main));
    assert!(result.commit.is_none());
}

#[test]
fn clean_merge_creates_commit_with_both_parents() {
    let repo = diverged();
    let result = repo.run(&repo.start("topic")).expect("merge");
    assert_eq!(result.state, GitEffectState::Completed);
    let commit = result.commit.expect("merge commit");
    assert_eq!(repo.branch(), commit);
    let (parents, message, tree) = repo.commit_info(commit);
    assert_eq!(parents, vec![repo.main, repo.topic]);
    assert_eq!(message, "Merge branch 'topic'\n");
    assert_eq!(
        repo.fixture.read(tree, "a.txt").as_deref(),
        Some("a main\n")
    );
    assert_eq!(repo.fixture.read(tree, "b.txt").as_deref(), Some("b\n"));
    assert_eq!(repo.read("b.txt"), "b\n");
    let reflog = "merge topic: Merge made by the 'ort' strategy.";
    assert!(repo.log_ends_with("logs/refs/heads/main", reflog));
    assert!(repo.log_ends_with("logs/HEAD", reflog));
    assert!(!repo.has_git("MERGE_HEAD"));
    let effect = result.effect(repo.root());
    assert_eq!(effect.state, GitEffectState::Completed);
    assert_eq!(effect.in_progress, Some(GitInProgress::None));
    assert_eq!(effect.branch.as_deref(), Some("main"));
    assert!(result.conflict_error(&repo.label()).is_none());
}

#[test]
fn never_fast_forward_creates_commit_with_given_message() {
    let repo = Repo::new(&[], &[("b.txt", "b\n")]);
    let mut request = repo.start("topic");
    request.fast_forward = FastForward::Never;
    request.message = Some("Combine topic  \n\n\n".to_owned());
    let result = repo.run(&request).expect("merge");
    let (parents, message, _) = repo.commit_info(result.commit.expect("commit"));
    assert_eq!(parents, vec![repo.base, repo.topic]);
    assert_eq!(message, "Combine topic\n");
}

#[test]
fn detached_head_merge_names_head_as_destination() {
    let repo = diverged();
    repo.write_git("HEAD", &hex_line(repo.main));
    let result = repo.run(&repo.start("topic")).expect("merge");
    let commit = result.commit.expect("commit");
    assert_eq!(repo.read_git("HEAD"), hex_line(commit));
    assert_eq!(repo.branch(), repo.main);
    let (_, message, _) = repo.commit_info(commit);
    assert_eq!(message, "Merge branch 'topic' into HEAD\n");
    assert_eq!(result.effect(repo.root()).detached, Some(true));
}

#[test]
fn conflict_leaves_markers_stages_and_state_files() {
    let repo = conflicting();
    let result = repo.run(&repo.start("topic")).expect("merge");
    assert_eq!(result.state, GitEffectState::Conflicts);
    assert_eq!(
        repo.read("f.txt"),
        "1\n<<<<<<< HEAD\nmain\n=======\ntopic\n>>>>>>> topic\n3\n"
    );
    assert_eq!(
        repo.stages("f.txt"),
        vec![Stage::Base, Stage::Ours, Stage::Theirs]
    );
    assert_eq!(repo.read_git("MERGE_HEAD"), hex_line(repo.topic));
    assert_eq!(
        repo.read_git("MERGE_MSG"),
        "Merge branch 'topic'\n\n# Conflicts:\n#\tf.txt\n"
    );
    assert_eq!(repo.read_git("MERGE_MODE"), "");
    assert!(repo.has_git("AUTO_MERGE"));
    assert_eq!(repo.read_git("ORIG_HEAD"), hex_line(repo.main));
    assert_eq!(repo.branch(), repo.main);
    let error = result
        .conflict_error(&repo.label())
        .expect("conflict error");
    assert_eq!(error.code(), ErrorCode::GitConflict);
    assert_eq!(error.data().sample, vec!["f.txt".to_owned()]);
    let effect = result.effect(repo.root());
    assert_eq!(effect.in_progress, Some(GitInProgress::Merge));
    assert_eq!(effect.merge_head, Some(repo.topic.to_string()));
    assert_eq!(effect.conflicts.map(|list| list.len()), Some(1));
}

#[test]
fn favored_side_resolves_conflicting_hunks() {
    let repo = conflicting();
    let mut request = repo.start("topic");
    request.favor = Some(Favor::Theirs);
    let result = repo.run(&request).expect("merge");
    assert_eq!(result.state, GitEffectState::Completed);
    assert_eq!(repo.read("f.txt"), "1\ntopic\n3\n");
}

#[test]
fn no_commit_stops_clean_merge_before_the_commit() {
    let repo = diverged();
    let mut request = repo.start("topic");
    request.no_commit = true;
    request.fast_forward = FastForward::Never;
    let result = repo.run(&request).expect("merge");
    assert_eq!(result.state, GitEffectState::Stopped);
    assert_eq!(repo.branch(), repo.main);
    assert_eq!(repo.read("b.txt"), "b\n");
    assert_eq!(repo.read_git("MERGE_MSG"), "Merge branch 'topic'\n");
    assert_eq!(repo.read_git("MERGE_MODE"), "no-ff");
    assert!(repo.has_git("AUTO_MERGE"));
}

#[test]
fn continue_concludes_stopped_merge() {
    let repo = diverged();
    let mut request = repo.start("topic");
    request.no_commit = true;
    repo.run(&request).expect("merge");
    let result = repo
        .run(&repo.action(MergeAction::Continue))
        .expect("continue");
    let commit = result.commit.expect("commit");
    assert_eq!(repo.branch(), commit);
    let (parents, message, _) = repo.commit_info(commit);
    assert_eq!(parents, vec![repo.main, repo.topic]);
    assert_eq!(message, "Merge branch 'topic'\n");
    assert!(repo.log_ends_with("logs/HEAD", "commit (merge): Merge branch 'topic'"));
    for name in ["MERGE_HEAD", "MERGE_MSG", "MERGE_MODE", "AUTO_MERGE"] {
        assert!(!repo.has_git(name), "{name} remains");
    }
}

#[test]
fn continue_after_resolution_commits_the_index() {
    let repo = conflicting();
    repo.run(&repo.start("topic")).expect("merge");
    let resolved = repo.fixture.tree(&files(&[("f.txt", "1\nboth\n3\n")]));
    repo.write("f.txt", "1\nboth\n3\n");
    repo.stage_tree(resolved);
    let result = repo
        .run(&repo.action(MergeAction::Continue))
        .expect("continue");
    let (_, message, tree) = repo.commit_info(result.commit.expect("commit"));
    assert_eq!(tree, resolved);
    assert_eq!(message, "Merge branch 'topic'\n");
}

#[test]
fn continue_refuses_unresolved_conflicts() {
    let repo = conflicting();
    repo.run(&repo.start("topic")).expect("merge");
    let error = repo
        .run(&repo.action(MergeAction::Continue))
        .expect_err("unmerged");
    assert_eq!(error.code(), ErrorCode::GitConflict);
    assert_eq!(error.data().problem.as_deref(), Some("unmerged_paths"));
}

#[test]
fn abort_restores_the_state_before_the_merge() {
    let repo = conflicting();
    repo.run(&repo.start("topic")).expect("merge");
    let result = repo.run(&repo.action(MergeAction::Abort)).expect("abort");
    assert_eq!(result.head_after, Some(repo.main));
    assert_eq!(repo.read("f.txt"), "1\nmain\n3\n");
    assert_eq!(repo.stages("f.txt"), vec![Stage::Unconflicted]);
    for name in ["MERGE_HEAD", "MERGE_MSG", "MERGE_MODE", "AUTO_MERGE"] {
        assert!(!repo.has_git(name), "{name} remains");
    }
    assert_eq!(repo.read_git("ORIG_HEAD"), hex_line(repo.main));
    assert!(repo.log_ends_with("logs/HEAD", "reset: moving to HEAD"));
    assert_eq!(repo.branch(), repo.main);
}

#[test]
fn abort_keeps_local_change_of_path_the_merge_did_not_touch() {
    let repo = conflicting();
    repo.write("u.txt", "local\n");
    repo.run(&repo.start("topic")).expect("merge");
    repo.run(&repo.action(MergeAction::Abort)).expect("abort");
    assert_eq!(repo.read("u.txt"), "local\n");
    assert_eq!(repo.read("f.txt"), "1\nmain\n3\n");
}

#[test]
fn abort_refuses_edit_of_path_changed_by_the_merge() {
    let repo = diverged();
    let mut request = repo.start("topic");
    request.no_commit = true;
    repo.run(&request).expect("merge");
    repo.write("b.txt", "edited\n");
    let error = repo
        .run(&repo.action(MergeAction::Abort))
        .expect_err("edited");
    assert_eq!(error.code(), ErrorCode::GitDirty);
    assert_eq!(error.data().problem.as_deref(), Some("local_changes"));
    assert_eq!(repo.read("b.txt"), "edited\n");
}

#[test]
fn start_refuses_while_a_merge_is_in_progress() {
    let repo = conflicting();
    repo.run(&repo.start("topic")).expect("merge");
    let error = repo.run(&repo.start("topic")).expect_err("in progress");
    assert_eq!(error.code(), ErrorCode::GitInProgress);
    assert_eq!(error.data().problem.as_deref(), Some("merge"));
}

#[test]
fn continue_and_abort_require_a_stopped_merge() {
    let repo = diverged();
    for action in [MergeAction::Continue, MergeAction::Abort] {
        let error = repo.run(&repo.action(action)).expect_err("no merge");
        assert_eq!(error.code(), ErrorCode::GitInProgress);
        assert_eq!(
            error.data().problem.as_deref(),
            Some("no_merge_in_progress")
        );
    }
    let error = repo
        .run(&repo.action(MergeAction::Start))
        .expect_err("no branch");
    assert_eq!(error.code(), ErrorCode::MissingParam);
}

#[test]
fn local_change_of_merged_path_blocks_the_merge_before_any_write() {
    let repo = Repo::new(&[("a.txt", "a main\n")], &[("f.txt", "1\ntopic\n3\n")]);
    repo.write("f.txt", "local\n");
    let before = repo.state();
    let error = repo.run(&repo.start("topic")).expect_err("dirty");
    assert_eq!(error.code(), ErrorCode::GitDirty);
    assert!(
        error
            .data()
            .fix
            .iter()
            .any(|fix| fix.starts_with("git.stash"))
    );
    assert_eq!(repo.state(), before);
}

#[test]
fn staged_change_blocks_three_way_merge() {
    let repo = diverged();
    let staged = repo
        .fixture
        .tree(&files(&[("a.txt", "a main\n"), ("s.txt", "staged\n")]));
    repo.stage_tree(staged);
    let error = repo.run(&repo.start("topic")).expect_err("staged");
    assert_eq!(error.code(), ErrorCode::GitDirty);
    assert_eq!(error.data().problem.as_deref(), Some("staged_changes"));
    assert_eq!(error.data().sample, vec!["s.txt".to_owned()]);
}

#[test]
fn unrelated_histories_are_merged_only_when_allowed() {
    let repo = diverged();
    let other = repo
        .fixture
        .commit(repo.fixture.tree(&[("o.txt", "o\n")]), &[], "other");
    repo.write_git("refs/heads/other", &hex_line(other));
    let error = repo.run(&repo.start("other")).expect_err("unrelated");
    assert_eq!(error.data().problem.as_deref(), Some("unrelated_histories"));
    let mut request = repo.start("other");
    request.allow_unrelated = true;
    let result = repo.run(&request).expect("merge");
    assert_eq!(result.state, GitEffectState::Completed);
    assert_eq!(repo.read("o.txt"), "o\n");
}

#[test]
fn undo_restores_every_action_byte_for_byte() {
    let cases: [(fn() -> Repo, bool); 3] = [
        (|| Repo::new(&[], &[("b.txt", "b\n")]), false),
        (diverged, false),
        (conflicting, true),
    ];
    for (setup, stops) in cases {
        let repo = setup();
        let before = repo.state();
        repo.run(&repo.start("topic")).expect("merge");
        let after = repo.state();
        assert_ne!(after, before);
        if stops {
            repo.run(&repo.action(MergeAction::Abort)).expect("abort");
            repo.journal.undo_last();
            assert_eq!(repo.state(), after);
        }
        repo.journal.undo_last();
        assert_eq!(repo.state(), before);
    }
}

#[test]
fn undo_restores_concluded_merge() {
    let repo = diverged();
    let mut request = repo.start("topic");
    request.no_commit = true;
    repo.run(&request).expect("merge");
    let stopped = repo.state();
    repo.run(&repo.action(MergeAction::Continue))
        .expect("continue");
    repo.journal.undo_last();
    assert_eq!(repo.state(), stopped);
}

fn stage_args(repo: &Repo, extra: &[(&str, &str)]) -> nurt_contracts::stage::BoundArgs {
    use nurt_contracts::stage::{ArgValue, BoundArgs};
    use nurt_contracts::value::Value;
    let mut args =
        BoundArgs::new("git.merge").with("repo", ArgValue::Value(Value::Path(repo.label())), false);
    for (name, value) in extra {
        let value = match *value {
            "true" => Value::Bool(true),
            text => Value::Text(text.to_owned()),
        };
        args = args.with(*name, ArgValue::Value(value), false);
    }
    args
}

fn run_stage(repo: &Repo, extra: &[(&str, &str)]) -> Result<Vec<nurt_contracts::value::Record>> {
    use nurt_contracts::stage::{StageData, StageInput};
    let ctx = nurt_git::testutil::test_ctx();
    let args = stage_args(repo, extra);
    let stage = MergeCommand;
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
fn stage_merges_and_reports_the_effect() {
    let repo = diverged();
    assert_eq!(MergeCommand.descriptor().command, "git.merge".to_owned());
    let records = run_stage(&repo, &[("branch", "topic")]).expect("merge");
    assert_eq!(records.len(), 1);
    assert_ne!(repo.branch(), repo.main);
    assert_eq!(repo.read("b.txt"), "b\n");
}

#[test]
fn stage_reports_conflicts_as_applied_error() {
    let repo = conflicting();
    let error = run_stage(&repo, &[("branch", "topic")]).expect_err("conflict");
    assert_eq!(error.code(), ErrorCode::GitConflict);
    assert!(error.data().effect_applied);
    assert!(repo.has_git("MERGE_HEAD"));
    assert!(repo.read("f.txt").contains("<<<<<<< HEAD"));
    let error = run_stage(&repo, &[("action", "abort")]);
    assert!(error.is_ok(), "{error:?}");
    assert!(!repo.has_git("MERGE_HEAD"));
}

#[test]
fn stage_rejects_unknown_parameter_values() {
    let repo = diverged();
    for (name, value) in [("action", "skip"), ("fast_forward", "sometimes")] {
        let error = run_stage(&repo, &[("branch", "topic"), (name, value)]).expect_err(name);
        assert_eq!(error.code(), ErrorCode::InvalidValue, "{name}");
    }
}

#[test]
fn parses_action_and_fast_forward_values() {
    for action in [
        MergeAction::Start,
        MergeAction::Continue,
        MergeAction::Abort,
    ] {
        assert_eq!(MergeAction::parse(action.as_str()), Some(action));
    }
    for policy in [FastForward::Auto, FastForward::Only, FastForward::Never] {
        assert_eq!(FastForward::parse(policy.as_str()), Some(policy));
    }
    assert_eq!(MergeAction::default(), MergeAction::Start);
    assert_eq!(FastForward::default(), FastForward::Auto);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
