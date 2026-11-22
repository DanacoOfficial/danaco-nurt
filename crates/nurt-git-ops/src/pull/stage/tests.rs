// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the `git.pull` stage against repositories in temporary
//! directories: a transfer engine that plans reference updates of objects
//! already in the repository, the merge engine of the crate, and a
//! recording journal that undoes each pull byte for byte.

use std::collections::BTreeMap;
use std::fs;
use std::sync::Mutex;

use bstr::ByteSlice;
use nurt_contracts::record::{GitEffectState, GitRefStatus, GitRefUpdate};
use nurt_contracts::stage::ArgValue;
use nurt_contracts::value::{RecordKind, Value};
use nurt_git::apply::testjournal::{RecordingJournal, Snapshot, assert_declared, tree_state};
use nurt_git::apply::{ApplyContext, RefPlan, RefTarget, ReflogLine, apply_plan};
use nurt_git::repo::GitRepo;
use nurt_git::testutil::test_ctx;

use super::super::probe::tests::Fixture;
use super::super::steps::{FetchOutcome, FetchedHead};
use super::*;

const BASE: &[(&str, &str)] = &[("a.txt", "a\n")];
const LOCAL: &[(&str, &str)] = &[("a.txt", "a\n"), ("l.txt", "l\n")];
const UPSTREAM: &[(&str, &str)] = &[("a.txt", "a\nb\n")];

/// `origin` is the upstream remote of `main`; the identity makes merge
/// commits independent of the host.
const REMOTE_CONFIG: &str = "[remote \"origin\"]\n\turl = ../upstream\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n[user]\n\tname = Puller\n\temail = puller@example.com\n";

const URL: &str = "../upstream";

fn signature() -> gix::actor::Signature {
    gix::actor::Signature {
        name: "Fetcher".into(),
        email: "fetcher@example.com".into(),
        time: gix::date::Time::new(1_767_268_800, 0),
    }
}

/// Transfer engine whose heads are already in the object store; it plans
/// the remote-tracking references and `FETCH_HEAD` and records each
/// request.
struct FakeFetch {
    heads: Vec<(ObjectId, &'static str)>,
    error: Option<Error>,
    calls: Mutex<Vec<FetchRequest>>,
}

impl FakeFetch {
    fn new(heads: Vec<(ObjectId, &'static str)>) -> Arc<Self> {
        Arc::new(Self {
            heads,
            error: None,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn failing(error: Error) -> Arc<Self> {
        Arc::new(Self {
            heads: Vec::new(),
            error: Some(error),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<FetchRequest> {
        self.calls.lock().unwrap().clone()
    }
}

fn current(repo: &GitRepo, name: &str) -> ObjectId {
    repo.gix()
        .try_find_reference(name)
        .unwrap()
        .map_or(ObjectId::null(gix::hash::Kind::Sha1), |mut found| {
            found.peel_to_id().unwrap().detach()
        })
}

impl PullFetch for FakeFetch {
    fn fetch(
        &self,
        repo: &GitRepo,
        request: &FetchRequest,
        _tx: &mut dyn MutationTx,
        _ctx: &ExecCtx,
    ) -> Result<FetchOutcome> {
        self.calls.lock().unwrap().push(request.clone());
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let mut outcome = FetchOutcome {
            url: URL.to_owned(),
            objects_received: 3,
            bytes_received: 300,
            ..FetchOutcome::default()
        };
        let mut text = String::new();
        for (id, branch) in &self.heads {
            let name = format!("refs/remotes/origin/{branch}");
            let old = current(repo, &name);
            let line = ReflogLine {
                old,
                new: *id,
                committer: signature(),
                message: format!("{}: fast-forward", request.reflog_action),
            };
            outcome.refs.push(RefPlan::new(
                repo.gix(),
                &name,
                RefTarget::Object(*id),
                Some(line),
            )?);
            outcome.ref_updates.push(GitRefUpdate {
                reference: name,
                status: GitRefStatus::Updated,
                remote: request.remote.clone(),
                old: Some(old.to_string()),
                new: Some(id.to_string()),
                ..GitRefUpdate::default()
            });
            let description = format!("branch '{branch}' of {URL}");
            text.push_str(&format!("{id}\t\t{description}\n"));
            outcome.for_merge.push(FetchedHead {
                id: *id,
                description,
                remote_ref: format!("refs/heads/{branch}"),
            });
        }
        outcome
            .state_files
            .push(FetchOutcome::fetch_head_file(text.into_bytes()));
        Ok(outcome)
    }
}

/// Rebase engine that writes the transfer alone and records each request.
struct FakeRebase {
    refuse: bool,
    calls: Mutex<Vec<RebaseRequest>>,
}

impl FakeRebase {
    fn new(refuse: bool) -> Arc<Self> {
        Arc::new(Self {
            refuse,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<RebaseRequest> {
        self.calls.lock().unwrap().clone()
    }
}

impl PullRebase for FakeRebase {
    fn rebase(
        &self,
        repo: &GitRepo,
        request: &RebaseRequest,
        transfer: nurt_git::apply::ApplyPlan,
        tx: &mut dyn MutationTx,
        _ctx: &ExecCtx,
    ) -> std::result::Result<LocalOutcome, RebaseError> {
        self.calls.lock().unwrap().push(request.clone());
        if self.refuse {
            return Err(RebaseError::Refused(Error::new(
                ErrorCode::GitDirty,
                "local changes would be overwritten by the rebase",
            )));
        }
        let ctx = ApplyContext::open(repo.gix()).map_err(RebaseError::Failed)?;
        apply_plan(tx, &ctx, transfer).map_err(RebaseError::Failed)?;
        Ok(LocalOutcome {
            effect: GitEffect {
                state: GitEffectState::Completed,
                ..GitEffect::default()
            },
            ..LocalOutcome::default()
        })
    }
}

/// History `base` - `local` on `main` and `base` - `upstream` on the
/// remote; `refs/remotes/origin/main` is at `tracking`.
struct Scenario {
    fixture: Fixture,
    journal: RecordingJournal,
    base: ObjectId,
    local: ObjectId,
    upstream: ObjectId,
}

impl Scenario {
    fn new(config: &str) -> Self {
        Self::build(config, Some(LOCAL), UPSTREAM)
    }

    /// `main` is at `base`, behind the remote.
    fn behind(config: &str) -> Self {
        Self::build(config, None, UPSTREAM)
    }

    fn build(config: &str, local: Option<&[(&str, &str)]>, upstream: &[(&str, &str)]) -> Self {
        let mut fixture = Fixture::new();
        let base = fixture.commit(BASE, &[], "base");
        let local = match local {
            Some(files) => fixture.commit(files, &[base], "local"),
            None => base,
        };
        let upstream = fixture.commit(upstream, &[base], "upstream");
        Self::assemble(fixture, [base, local, base, upstream], config)
    }

    /// Sets `main` and the remote-tracking reference, writes the
    /// configuration and checks out `main`.
    fn assemble(
        fixture: Fixture,
        [base, local, tracking, upstream]: [ObjectId; 4],
        config: &str,
    ) -> Self {
        fixture.set_ref("refs/heads/main", local);
        fixture.set_ref("refs/remotes/origin/main", tracking);
        fixture.config(REMOTE_CONFIG);
        fixture.config(config);
        fixture.checkout(local);
        Self {
            fixture,
            journal: RecordingJournal::new(),
            base,
            local,
            upstream,
        }
    }

    fn label(&self) -> String {
        display_path(self.fixture.root())
    }

    fn args(&self, extra: &[(&str, Value)]) -> BoundArgs {
        let mut args =
            BoundArgs::new(COMMAND).with("repo", ArgValue::Value(Value::Path(self.label())), false);
        for (name, value) in extra {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    /// Plans and applies a pull; the transaction is committed when the pull
    /// succeeded or reports `effect_applied`, as the executor does.
    fn run(&self, stage: &GitPull, extra: &[(&str, Value)]) -> Result<Vec<Record>> {
        let args = self.args(extra);
        let ctx = test_ctx();
        let plan = stage.plan(&args, StageInput::none(), &ctx)?;
        let mut tx = self.journal.begin();
        let result = stage
            .apply(&args, plan, &mut tx, &ctx)
            .and_then(|outcome| StageInput::from(outcome.output).into_records())
            .and_then(|records| records.collect_bounded(100));
        let applied = match &result {
            Ok(_) => true,
            Err(error) => error.effect_applied,
        };
        if applied {
            Box::new(tx).commit().unwrap();
        }
        result
    }

    fn plan_error(&self, extra: &[(&str, Value)]) -> Error {
        GitPull::new(FakeFetch::new(Vec::new()))
            .plan(&self.args(extra), StageInput::none(), &test_ctx())
            .unwrap_err()
    }

    /// Working tree and repository directory without the object store,
    /// whose additions are exempt from the journal.
    fn state(&self) -> BTreeMap<PathBuf, Snapshot> {
        tree_state(self.fixture.root())
            .into_iter()
            .filter(|(path, _)| !path.starts_with(".git/objects"))
            .collect()
    }

    fn git_path(&self, name: &str) -> PathBuf {
        self.fixture.git_dir().join(name)
    }

    fn read_git(&self, name: &str) -> String {
        fs::read_to_string(self.git_path(name)).unwrap()
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.fixture.root().join(path)).unwrap()
    }

    fn write(&self, path: &str, content: &str) {
        fs::write(self.fixture.root().join(path), content).unwrap();
    }

    fn reference(&self, name: &str) -> ObjectId {
        ObjectId::from_hex(self.read_git(name).trim().as_bytes()).unwrap()
    }

    fn branch(&self) -> ObjectId {
        self.reference("refs/heads/main")
    }

    fn commit_info(&self, id: ObjectId) -> (Vec<ObjectId>, String) {
        let commit = self.fixture.repo.find_object(id).unwrap().into_commit();
        let decoded = commit.decode().unwrap();
        (
            decoded.parents().collect(),
            decoded.message.to_str_lossy().into_owned(),
        )
    }

    fn fix(&self, mode: &str) -> String {
        format!("git.pull \"{}\" mode: {mode}", self.label())
    }
}

fn mode(mode: &str) -> (&'static str, Value) {
    ("mode", Value::Enum(mode.into()))
}

fn text(name: &'static str, value: &str) -> (&'static str, Value) {
    (name, Value::Text(value.into()))
}

fn effect(records: &[Record]) -> Effect {
    Effect::try_from(records[0].clone()).unwrap()
}

#[test]
fn describes_itself_as_a_source_effect_stage() {
    let stage = GitPull::new(FakeFetch::new(Vec::new()));
    let descriptor = stage.descriptor();
    assert_eq!(descriptor.command, COMMAND);
    assert!(descriptor.source);
    assert!(stage.as_effect().is_some());
    assert!(stage.as_read().is_none());
    assert!(format!("{stage:?}").contains("rebase: false"));
    let with_rebase = stage.with_rebase(FakeRebase::new(false));
    assert!(format!("{with_rebase:?}").contains("rebase: true"));
}

fn assert_fast_forward_files(scenario: &Scenario) {
    assert!(
        scenario
            .read_git("FETCH_HEAD")
            .starts_with(&format!("{}\t\tbranch 'main' of {URL}", scenario.upstream))
    );
    assert!(
        scenario
            .read_git("logs/refs/heads/main")
            .ends_with("\tpull: Fast-forward\n")
    );
}

#[test]
fn fast_forwards_in_one_transaction_that_undo_restores() {
    let scenario = Scenario::behind("");
    let before = scenario.state();
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main")]);
    let records = scenario.run(&GitPull::new(fetch.clone()), &[]).unwrap();
    assert_eq!(
        fetch.calls(),
        [FetchRequest {
            remote: "origin".into(),
            refspecs: Vec::new(),
            reflog_action: "pull".into(),
            accept_host_key: None,
        }]
    );
    assert_eq!(scenario.branch(), scenario.upstream);
    assert_eq!(
        scenario.reference("refs/remotes/origin/main"),
        scenario.upstream
    );
    assert_eq!(scenario.read("a.txt"), "a\nb\n");
    assert_fast_forward_files(&scenario);
    assert_eq!(scenario.journal.len(), 1);
    let effect = effect(&records);
    assert_eq!(
        effect.irreversible.unwrap()[0].reason,
        Irreversible::RemoteRefUpdated
    );
    assert_eq!(effect.target.as_deref(), Some("remote:origin"));
    let git = effect.git.unwrap();
    assert_eq!(git.state, GitEffectState::FastForward);
    assert_eq!(git.head_before, Some(scenario.base.to_string()));
    assert_eq!(git.head_after, Some(scenario.upstream.to_string()));
    assert_eq!(git.objects_received, Some(3));
    assert_eq!(git.ref_updates.unwrap().len(), 1);
    assert_eq!(records[1].kind, RecordKind::GitRefUpdate);
    scenario.journal.undo_last();
    assert_eq!(scenario.state(), before);
}

#[test]
fn creates_a_merge_commit_in_merge_mode() {
    let scenario = Scenario::new("");
    let before = scenario.state();
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main")]);
    let records = scenario
        .run(&GitPull::new(fetch), &[mode("merge")])
        .unwrap();
    let merge = scenario.branch();
    let (parents, message) = scenario.commit_info(merge);
    assert_eq!(parents, [scenario.local, scenario.upstream]);
    assert_eq!(message, format!("Merge branch 'main' of {URL}\n"));
    assert_eq!(scenario.read("a.txt"), "a\nb\n");
    assert_eq!(scenario.read("l.txt"), "l\n");
    assert!(
        scenario
            .read_git("logs/refs/heads/main")
            .ends_with("\tpull --no-rebase: Merge made by the 'ort' strategy.\n")
    );
    let git = effect(&records).git.unwrap();
    assert_eq!(git.state, GitEffectState::Completed);
    assert_eq!(git.head_after, Some(merge.to_string()));
    scenario.journal.undo_last();
    assert_eq!(scenario.state(), before);
}

#[test]
fn creates_a_merge_commit_when_fast_forward_is_disabled() {
    let scenario = Scenario::behind("[pull]\n\tff = false\n");
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main")]);
    scenario.run(&GitPull::new(fetch), &[]).unwrap();
    let (parents, _) = scenario.commit_info(scenario.branch());
    assert_eq!(parents, [scenario.base, scenario.upstream]);
}

#[test]
fn merges_with_an_explicit_mode_despite_fast_forward_only_configuration() {
    let scenario = Scenario::new("[pull]\n\tff = only\n");
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main")]);
    scenario
        .run(&GitPull::new(fetch), &[mode("merge")])
        .unwrap();
    let (parents, _) = scenario.commit_info(scenario.branch());
    assert_eq!(parents.len(), 2);
}

#[test]
fn declares_every_changed_file_of_a_merge_and_undo_restores_them() {
    let scenario = Scenario::new("[pull]\n\trebase = false\n");
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main")]);
    let root = scenario.fixture.root();
    let before = tree_state(root);
    scenario.run(&GitPull::new(fetch), &[]).unwrap();
    let after = tree_state(root);
    assert_ne!(after, before);
    assert_declared(root, &before, &after, &scenario.journal);
    scenario.journal.undo_last();
    assert_eq!(tree_state(root), before);
}

#[test]
fn reports_an_up_to_date_branch() {
    let scenario = Scenario::new("");
    let fetch = FakeFetch::new(vec![(scenario.base, "main")]);
    let records = scenario.run(&GitPull::new(fetch), &[]).unwrap();
    assert_eq!(scenario.branch(), scenario.local);
    assert_eq!(
        effect(&records).git.unwrap().state,
        GitEffectState::UpToDate
    );
}

#[test]
fn refuses_diverged_histories_and_keeps_the_transfer() {
    let scenario = Scenario::new("");
    let before = scenario.state();
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main")]);
    let error = scenario.run(&GitPull::new(fetch), &[]).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitNonFastForward);
    assert_eq!(error.problem.as_deref(), Some("diverged"));
    assert!(error.effect_applied);
    assert_eq!(error.fix, [scenario.fix("merge"), scenario.fix("rebase")]);
    assert_eq!(
        error.details["ref_updates"][0]["ref"],
        serde_json::json!("refs/remotes/origin/main")
    );
    assert_eq!(scenario.branch(), scenario.local);
    assert_eq!(
        scenario.reference("refs/remotes/origin/main"),
        scenario.upstream
    );
    assert!(scenario.git_path("FETCH_HEAD").exists());
    assert_eq!(scenario.journal.len(), 1);
    scenario.journal.undo_last();
    assert_eq!(scenario.state(), before);
}

#[test]
fn stops_with_conflicts_after_writing_the_transfer() {
    let scenario = Scenario::build(
        "[pull]\n\trebase = false\n",
        Some(&[("a.txt", "a\nlocal\n")]),
        UPSTREAM,
    );
    let before = scenario.state();
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main")]);
    let error = scenario.run(&GitPull::new(fetch), &[]).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitConflict);
    assert!(error.effect_applied);
    assert!(error.details.contains_key("ref_updates"));
    assert_eq!(
        scenario.read_git("MERGE_HEAD"),
        format!("{}\n", scenario.upstream)
    );
    assert!(
        scenario
            .read_git("MERGE_MSG")
            .starts_with(&format!("Merge branch 'main' of {URL}\n"))
    );
    assert!(
        scenario
            .read("a.txt")
            .contains(&format!(">>>>>>> {}\n", scenario.upstream))
    );
    assert_eq!(
        scenario.reference("refs/remotes/origin/main"),
        scenario.upstream
    );
    assert_eq!(scenario.journal.len(), 1);
    scenario.journal.undo_last();
    assert_eq!(scenario.state(), before);
}

#[test]
fn refuses_local_changes_in_the_way_of_a_fast_forward() {
    let scenario = Scenario::behind("");
    scenario.write("a.txt", "mine\n");
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main")]);
    let error = scenario.run(&GitPull::new(fetch), &[]).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitDirty);
    assert!(error.effect_applied);
    assert_eq!(scenario.read("a.txt"), "mine\n");
    assert_eq!(scenario.branch(), scenario.base);
    assert!(scenario.git_path("FETCH_HEAD").exists());
}

#[test]
fn refuses_a_non_fast_forward_in_fast_forward_only_mode() {
    let scenario = Scenario::new("");
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main")]);
    let error = scenario
        .run(&GitPull::new(fetch), &[mode("fast_forward_only")])
        .unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("not_fast_forward"));
    assert_eq!(error.fix[0], scenario.fix("merge"));
    assert_eq!(scenario.branch(), scenario.local);
}

#[test]
fn refuses_several_heads_in_a_merge() {
    let scenario = Scenario::new("");
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main"), (scenario.base, "dev")]);
    let error = scenario
        .run(&GitPull::new(fetch), &[mode("merge")])
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::GitUnsupported);
    assert_eq!(error.problem.as_deref(), Some("octopus_merge"));
    assert!(error.effect_applied);
}

#[test]
fn rebases_through_the_rebase_engine_in_the_same_transaction() {
    let scenario = Scenario::new("[pull]\n\trebase = true\n");
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main")]);
    let rebase = FakeRebase::new(false);
    let stage = GitPull::new(fetch).with_rebase(rebase.clone());
    let records = scenario.run(&stage, &[]).unwrap();
    assert_eq!(
        rebase.calls(),
        [RebaseRequest {
            repo: scenario.label(),
            onto: scenario.upstream,
            upstream: scenario.upstream,
            kind: RebaseKind::Plain,
            autostash: false,
            reflog_action: "pull".into(),
        }]
    );
    assert!(scenario.git_path("FETCH_HEAD").exists());
    assert_eq!(scenario.journal.len(), 1);
    let git = effect(&records).git.unwrap();
    assert_eq!(git.state, GitEffectState::Completed);
    assert_eq!(git.branch.as_deref(), Some("main"));
    assert_eq!(git.head_before, Some(scenario.local.to_string()));
}

#[test]
fn limits_the_rebase_to_commits_after_the_fork_point() {
    let mut fixture = Fixture::new();
    let base = fixture.commit(BASE, &[], "base");
    let old = fixture.commit(&[("a.txt", "old\n")], &[base], "old upstream");
    let local = fixture.commit(&[("a.txt", "old\n"), ("l.txt", "l\n")], &[old], "local");
    let upstream = fixture.commit(UPSTREAM, &[base], "upstream");
    let scenario = Scenario::assemble(
        fixture,
        [base, local, old, upstream],
        "[branch \"main\"]\n\trebase = merges\n",
    );
    let rebase = FakeRebase::new(false);
    let stage = GitPull::new(FakeFetch::new(vec![(upstream, "main")])).with_rebase(rebase.clone());
    scenario.run(&stage, &[mode("rebase")]).unwrap();
    let request = &rebase.calls()[0];
    assert_eq!(request.upstream, old);
    assert_eq!(request.kind, RebaseKind::Plain);
    assert_eq!(request.reflog_action, "pull --rebase");
}

#[test]
fn keeps_the_transfer_when_the_rebase_engine_refuses() {
    let scenario = Scenario::new("[pull]\n\trebase = true\n");
    let stage = GitPull::new(FakeFetch::new(vec![(scenario.upstream, "main")]))
        .with_rebase(FakeRebase::new(true));
    let error = scenario.run(&stage, &[]).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitDirty);
    assert!(error.effect_applied);
    assert!(scenario.git_path("FETCH_HEAD").exists());
    assert_eq!(scenario.branch(), scenario.local);
}

#[test]
fn refuses_a_rebase_without_an_engine_after_the_transfer() {
    let scenario = Scenario::new("[pull]\n\trebase = true\n");
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main")]);
    let error = scenario.run(&GitPull::new(fetch), &[]).unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("rebase_unavailable"));
    assert!(error.effect_applied);
    assert_eq!(error.fix, [scenario.fix("merge")]);
    assert!(scenario.git_path("FETCH_HEAD").exists());
}

#[test]
fn fast_forwards_a_rebase_without_an_engine() {
    let scenario = Scenario::behind("[pull]\n\trebase = true\n");
    let fetch = FakeFetch::new(vec![(scenario.upstream, "main")]);
    scenario.run(&GitPull::new(fetch), &[]).unwrap();
    assert_eq!(scenario.branch(), scenario.upstream);
}

#[test]
fn refuses_a_rebase_with_uncommitted_changes_before_the_transfer() {
    let scenario = Scenario::new("[pull]\n\trebase = true\n");
    scenario.write("l.txt", "changed\n");
    let error = scenario.plan_error(&[]);
    assert_eq!(error.code, ErrorCode::GitDirty);
    assert_eq!(error.problem.as_deref(), Some("unstaged_changes"));
    assert_eq!(error.sample, ["l.txt"]);
    assert!(!error.effect_applied);
}

#[allow(clippy::disallowed_methods)] // fixture setup writes the index outside the journal
fn stage_tree(scenario: &Scenario, tree: gix::Id<'_>) {
    let mut index = scenario.fixture.repo.index_from_tree(&tree).unwrap();
    index.write(Default::default()).unwrap();
}

#[test]
fn refuses_a_rebase_with_staged_changes() {
    let scenario = Scenario::new("");
    let tree = scenario
        .fixture
        .repo
        .find_commit(scenario.upstream)
        .unwrap()
        .tree_id()
        .unwrap();
    stage_tree(&scenario, tree);
    scenario.write("a.txt", "a\nb\n");
    fs::remove_file(scenario.fixture.root().join("l.txt")).unwrap();
    let error = scenario.plan_error(&[mode("rebase")]);
    assert_eq!(error.problem.as_deref(), Some("staged_changes"));
}

#[test]
fn autostash_lets_a_rebase_start_with_changes() {
    let scenario = Scenario::new("[rebase]\n\tautoStash = true\n");
    scenario.write("l.txt", "changed\n");
    let rebase = FakeRebase::new(false);
    let stage =
        GitPull::new(FakeFetch::new(vec![(scenario.upstream, "main")])).with_rebase(rebase.clone());
    scenario.run(&stage, &[mode("rebase")]).unwrap();
    assert!(rebase.calls()[0].autostash);
}

#[test]
fn refuses_unfinished_operations_before_the_transfer() {
    let scenario = Scenario::new("");
    fs::create_dir(scenario.git_path("rebase-merge")).unwrap();
    assert_eq!(
        scenario.plan_error(&[]).problem.as_deref(),
        Some("rebase_in_progress")
    );
    fs::write(
        scenario.git_path("MERGE_HEAD"),
        format!("{}\n", scenario.upstream),
    )
    .unwrap();
    assert_eq!(
        scenario.plan_error(&[]).problem.as_deref(),
        Some("merge_in_progress")
    );
}

#[test]
fn refuses_an_invalid_configuration_value() {
    let error = Scenario::new("[pull]\n\trebase = preserve\n").plan_error(&[]);
    assert_eq!(error.code, ErrorCode::GitUnsupported);
    assert_eq!(error.fix.len(), 2);
    let ff = Scenario::new("[pull]\n\tff = maybe\n").plan_error(&[]);
    assert_eq!(ff.fix.len(), 1);
}

#[test]
fn explains_why_nothing_can_be_merged() {
    let scenario = Scenario::new("");
    let stage = GitPull::new(FakeFetch::new(Vec::new()));
    let problem = |scenario: &Scenario, extra: &[(&str, Value)]| {
        let error = scenario.run(&stage, extra).unwrap_err();
        assert!(error.effect_applied);
        error.problem.clone()
    };
    assert_eq!(
        problem(&scenario, &[text("branch", "gone")]).as_deref(),
        Some("no_merge_candidates")
    );
    assert_eq!(
        problem(&scenario, &[text("remote", "other")]).as_deref(),
        Some("branch_required")
    );
    assert_eq!(problem(&scenario, &[]).as_deref(), Some("upstream_gone"));
    assert_eq!(scenario.read_git("FETCH_HEAD"), "");

    let without_upstream = Scenario::new("");
    let config = without_upstream.git_path("config");
    let text_before = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        text_before.replace("\tmerge = refs/heads/main\n", ""),
    )
    .unwrap();
    assert_eq!(
        problem(&without_upstream, &[]).as_deref(),
        Some("no_upstream")
    );

    let detached = Scenario::new("");
    fs::write(detached.git_path("HEAD"), format!("{}\n", detached.local)).unwrap();
    assert_eq!(problem(&detached, &[]).as_deref(), Some("detached_head"));
}

#[test]
fn passes_the_explicit_branch_and_host_key_to_the_transfer() {
    let scenario = Scenario::new("[pull]\n\trebase = false\n");
    let fetch = FakeFetch::new(vec![(scenario.upstream, "dev")]);
    scenario
        .run(
            &GitPull::new(fetch.clone()),
            &[
                text("remote", "origin"),
                text("branch", "dev"),
                text("accept_host_key", "SHA256:AAAA"),
            ],
        )
        .unwrap();
    let request = &fetch.calls()[0];
    assert_eq!(request.refspecs, ["dev"]);
    assert_eq!(request.reflog_action, "pull origin dev");
    assert_eq!(request.accept_host_key.as_deref(), Some("SHA256:AAAA"));
    let (_, message) = scenario.commit_info(scenario.branch());
    assert_eq!(message, format!("Merge branch 'dev' of {URL}\n"));
    assert_eq!(
        scenario.reference("refs/remotes/origin/dev"),
        scenario.upstream
    );
}

#[test]
fn checks_out_the_fetched_head_on_an_unborn_branch() {
    let mut fixture = Fixture::new();
    let upstream = fixture.commit(UPSTREAM, &[], "init");
    fixture.config(REMOTE_CONFIG);
    let scenario = Scenario {
        fixture,
        journal: RecordingJournal::new(),
        base: upstream,
        local: upstream,
        upstream,
    };
    let before = scenario.state();
    let fetch = FakeFetch::new(vec![(upstream, "main")]);
    let records = scenario
        .run(
            &GitPull::new(fetch),
            &[text("remote", "origin"), text("branch", "main")],
        )
        .unwrap();
    assert_eq!(scenario.branch(), upstream);
    assert_eq!(scenario.read("a.txt"), "a\nb\n");
    assert!(
        scenario
            .read_git("logs/refs/heads/main")
            .ends_with("\tinitial pull\n")
    );
    assert!(!scenario.git_path("ORIG_HEAD").exists());
    assert_eq!(effect(&records).git.unwrap().head_before, None);
    scenario.journal.undo_last();
    assert_eq!(scenario.state(), before);
}

#[test]
fn reports_a_failed_transfer_without_writing() {
    let scenario = Scenario::behind("");
    let before = scenario.state();
    let fetch = FakeFetch::failing(Error::new(ErrorCode::Io, "connection reset"));
    let error = scenario.run(&GitPull::new(fetch), &[]).unwrap_err();
    assert_eq!(error.code, ErrorCode::Io);
    assert!(!error.effect_applied);
    assert!(scenario.journal.is_empty());
    assert_eq!(scenario.state(), before);
}

#[test]
fn previews_the_remote_and_the_mode() {
    let scenario = Scenario::new("[pull]\n\trebase = true\n");
    let stage = GitPull::new(FakeFetch::new(Vec::new()));
    let plan = stage
        .plan(&scenario.args(&[]), StageInput::none(), &test_ctx())
        .unwrap();
    assert_eq!(plan.irreversible[0].reason, Irreversible::RemoteRefUpdated);
    let preview = Effect::try_from(plan.preview[0].clone()).unwrap();
    assert_eq!(preview.status, EffectStatus::Planned);
    assert_eq!(preview.action.as_deref(), Some("rebase"));
    assert_eq!(preview.target.as_deref(), Some("remote:origin"));
    let only = Strategy {
        rebase: RebaseKind::No,
        ff: Some(FfPolicy::Only),
        rebase_unspecified: false,
        autostash: false,
    };
    let merge = Strategy {
        ff: None,
        rebase_unspecified: true,
        ..only
    };
    assert_eq!(strategy_label(&only), "fast_forward_only");
    assert_eq!(strategy_label(&merge), "merge");
}

#[test]
fn rejects_an_apply_without_a_plan() {
    let stage = GitPull::new(FakeFetch::new(Vec::new()));
    let args = BoundArgs::new(COMMAND);
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let error = stage
        .apply(&args, EffectPlan::new(), &mut tx, &test_ctx())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Internal);
}
