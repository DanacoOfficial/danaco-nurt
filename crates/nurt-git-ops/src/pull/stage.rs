// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.pull` stage: checks in the plan step, transfer and integration
//! in the apply step.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gix::ObjectId;
use nurt_contracts::error::{Error, ErrorCode, Result, Stage};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{Effect, EffectStatus, GitEffect};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{self, Record, Size};

use super::decide::{Action, Refusal, Relation, decide};
use super::errors::{self, PullCall};
use super::message::merge_message;
use super::params::{PullMode, PullParams};
use super::probe::{self, HeadState, Repo};
use super::settings::{
    ConfigSource, FfPolicy, RebaseKind, Strategy, Target, full_source_ref, reflog_action,
    resolve_strategy, resolve_target, tracking_ref,
};
use super::steps::{
    FetchOutcome, FetchRequest, InitialRequest, LocalOutcome, MergeRequest, PullSteps,
    RebaseRequest,
};

const COMMAND: &str = "git.pull";

/// Reflog message `git` writes when a pull creates the current branch.
const INITIAL_PULL: &str = "initial pull";

/// Implementation of `git.pull`.
///
/// The stage composes the transfer and integration engines given as
/// [`PullSteps`]; it decides which engine runs and with which parameters, and
/// it writes nothing itself.
pub struct GitPull {
    steps: Arc<dyn PullSteps>,
}

impl GitPull {
    /// Creates the stage over the given engines.
    #[must_use]
    pub fn new(steps: Arc<dyn PullSteps>) -> Self {
        Self { steps }
    }
}

impl std::fmt::Debug for GitPull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitPull").finish_non_exhaustive()
    }
}

/// State carried from the plan step to the apply step.
struct PullPlan {
    root: PathBuf,
    head: HeadState,
    strategy: Strategy,
    target: Target,
    reflog_action: String,
    fork_point: Option<ObjectId>,
    suppress_dest: Option<Vec<String>>,
    call: PullCall,
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn internal(message: &str) -> Error {
    Error::new(ErrorCode::Internal, message).with_stage(Stage::Apply)
}

fn check_unfinished(repo: &Repo, call: &PullCall) -> Result<()> {
    let unmerged = repo.unmerged_paths()?;
    if !unmerged.is_empty() {
        return Err(errors::unmerged_files(call, &unmerged));
    }
    if repo.merge_in_progress() {
        return Err(errors::merge_in_progress(call));
    }
    if repo.rebase_in_progress() {
        return Err(errors::rebase_in_progress(call));
    }
    Ok(())
}

/// `git pull --rebase` refuses unstaged changes first, then staged ones.
fn check_clean(repo: &Repo, head: &HeadState, call: &PullCall) -> Result<()> {
    let unstaged = repo.unstaged_paths()?;
    if !unstaged.is_empty() {
        return Err(errors::uncommitted_changes(call, false, &unstaged));
    }
    if let Some(id) = head.id() {
        let staged = repo.staged_paths(Some(id))?;
        if !staged.is_empty() {
            return Err(errors::uncommitted_changes(call, true, &staged));
        }
    }
    Ok(())
}

fn config_fixes(key: &str) -> &'static [PullMode] {
    if key == "pull.ff" {
        &[PullMode::FastForwardOnly]
    } else if key.ends_with(".rebase") {
        &[PullMode::Merge, PullMode::Rebase]
    } else {
        &[]
    }
}

/// Remote-tracking reference of the branch being pulled, whose reflog gives
/// the fork point of a rebase.
fn upstream_tracking(repo: &Repo, target: &Target) -> Option<String> {
    let source = match target.refspecs.first() {
        Some(refspec) => full_source_ref(refspec),
        None if target.upstream_remote => target.merge_refs.first()?.clone(),
        None => return None,
    };
    if target.remote == "." {
        return Some(source);
    }
    let refspecs = repo.get_all(&format!("remote.{}.fetch", target.remote));
    tracking_ref(&refspecs, &source)
}

fn strategy_label(strategy: &Strategy) -> &'static str {
    match (strategy.ff, strategy.rebase) {
        (Some(FfPolicy::Only), _) => PullMode::FastForwardOnly.as_str(),
        (_, RebaseKind::No) => PullMode::Merge.as_str(),
        (_, RebaseKind::Plain | RebaseKind::Merges) => PullMode::Rebase.as_str(),
    }
}

impl PullPlan {
    fn read(params: &PullParams) -> Result<Self> {
        let repo = probe::open(&params.repo)?;
        let head = repo.head()?;
        let call = PullCall::new(
            &display_path(&params.repo),
            params.remote.as_deref(),
            params.branch.as_deref(),
        );
        check_unfinished(&repo, &call)?;
        let branch = head.branch();
        let strategy = resolve_strategy(params.mode, &repo, branch).map_err(|invalid| {
            errors::invalid_config(
                &call,
                &invalid.key,
                &invalid.value,
                config_fixes(&invalid.key),
            )
        })?;
        let target = resolve_target(params, &repo, branch);
        let mut fork_point = None;
        if strategy.rebase != RebaseKind::No {
            if !strategy.autostash {
                check_clean(&repo, &head, &call)?;
            }
            if let (Some(tracking), Some(id)) = (upstream_tracking(&repo, &target), head.id()) {
                fork_point = repo.fork_point(&tracking, id)?;
            }
        }
        let suppress = repo.get_all("merge.suppressDest");
        Ok(Self {
            root: repo.root().to_path_buf(),
            reflog_action: reflog_action(params, &target.remote),
            suppress_dest: (!suppress.is_empty()).then_some(suppress),
            head,
            strategy,
            target,
            fork_point,
            call,
        })
    }

    fn preview(&self) -> Record {
        let git = GitEffect {
            repo: value::Path::new(display_path(&self.root)),
            head_before: self.head.id().map(|id| id.to_string()),
            branch: self.head.branch().map(str::to_owned),
            ..GitEffect::default()
        };
        Record::from(Effect {
            status: EffectStatus::Planned,
            kind: COMMAND.to_owned(),
            target: Some(format!("remote:{}", self.target.remote)),
            action: Some(strategy_label(&self.strategy).to_owned()),
            git: Some(git),
            ..Effect::default()
        })
    }

    fn no_candidates(&self) -> Error {
        if !self.target.refspecs.is_empty() {
            return errors::no_merge_candidates(&self.call, &self.target.refspecs);
        }
        match self.head.branch() {
            None => errors::detached_head(&self.call),
            Some(branch) if self.target.merge_refs.is_empty() => {
                errors::no_upstream(&self.call, branch, &self.target.remote)
            }
            Some(branch) if !self.target.upstream_remote => {
                errors::no_branch_for_remote(&self.call, &self.target.remote, branch)
            }
            Some(_) => {
                let merge_ref = self.target.merge_refs.first().map_or("", String::as_str);
                errors::upstream_gone(&self.call, merge_ref)
            }
        }
    }

    fn refusal(&self, refusal: Refusal) -> Error {
        match refusal {
            Refusal::Diverged => errors::diverged(&self.call),
            Refusal::NotFastForward => errors::not_fast_forward(&self.call),
            Refusal::MultipleIntoUnborn => errors::multiple_heads(&self.call, "check out"),
            Refusal::RebaseMultiple => errors::multiple_heads(&self.call, "rebase onto"),
            Refusal::FastForwardMultiple => errors::multiple_heads(&self.call, "fast-forward to"),
        }
    }
}

fn relation(repo: &Repo, head: Option<ObjectId>, theirs: &[ObjectId]) -> Result<Relation> {
    let Some(head) = head else {
        return Ok(Relation {
            unborn: true,
            can_fast_forward: false,
            up_to_date: false,
            heads: theirs.len(),
        });
    };
    let can_fast_forward = match theirs {
        [one] => repo.is_ancestor(head, *one)?,
        _ => false,
    };
    let mut up_to_date = true;
    for id in theirs {
        if !repo.is_ancestor(*id, head)? {
            up_to_date = false;
            break;
        }
    }
    Ok(Relation {
        unborn: false,
        can_fast_forward,
        up_to_date,
        heads: theirs.len(),
    })
}

impl GitPull {
    fn integrate(
        &self,
        plan: &PullPlan,
        fetched: &FetchOutcome,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<LocalOutcome> {
        if fetched.for_merge.is_empty() {
            return Err(plan.no_candidates());
        }
        let theirs: Vec<ObjectId> = fetched.for_merge.iter().map(|head| head.id).collect();
        let repo = probe::open(&plan.root)?;
        let relation = relation(&repo, plan.head.id(), &theirs)?;
        let action = decide(&plan.strategy, &relation).map_err(|refusal| plan.refusal(refusal))?;
        ctx.checkpoint()?;
        match action {
            Action::InitialCheckout => {
                let request = InitialRequest {
                    theirs: theirs[0],
                    branch: format!("refs/heads/{}", plan.head.branch().unwrap_or("main")),
                    reflog_message: INITIAL_PULL.to_owned(),
                };
                self.steps.checkout_initial(&plan.root, &request, tx, ctx)
            }
            Action::Merge(fast_forward) => {
                let descriptions: Vec<String> = fetched
                    .for_merge
                    .iter()
                    .map(|head| head.description.clone())
                    .collect();
                let request = MergeRequest {
                    message: merge_message(
                        &descriptions,
                        plan.head.branch(),
                        plan.suppress_dest.as_deref(),
                    ),
                    theirs,
                    fast_forward,
                    reflog_action: plan.reflog_action.clone(),
                };
                self.steps.merge(&plan.root, &request, tx, ctx)
            }
            Action::Rebase(kind) => {
                let onto = theirs[0];
                let request = RebaseRequest {
                    upstream: rebase_upstream(&repo, plan, onto)?,
                    onto,
                    kind,
                    autostash: plan.strategy.autostash,
                    reflog_action: plan.reflog_action.clone(),
                };
                self.steps.rebase(&plan.root, &request, tx, ctx)
            }
        }
    }
}

/// The fork point limits the replayed commits only when it differs from the
/// merge base; otherwise `git` passes the fetched head, which also drops
/// commits already applied upstream.
fn rebase_upstream(repo: &Repo, plan: &PullPlan, onto: ObjectId) -> Result<ObjectId> {
    let (Some(fork_point), Some(head)) = (plan.fork_point, plan.head.id()) else {
        return Ok(onto);
    };
    if repo.merge_base(head, onto)? == Some(fork_point) {
        Ok(onto)
    } else {
        Ok(fork_point)
    }
}

fn outcome<'a>(plan: &PullPlan, fetched: FetchOutcome, local: LocalOutcome) -> EffectOutcome<'a> {
    let irreversible = vec![IrreversibleEntry {
        reason: Irreversible::RemoteRefUpdated,
        path: None,
        count: Some(fetched.ref_updates.len() as u64),
        detail: Some(plan.target.remote.clone()),
    }];
    let mut notable = fetched.notable;
    notable.extend(local.notable);
    let mut git = local.effect;
    git.repo = value::Path::new(display_path(&plan.root));
    if git.branch.is_none() {
        git.branch = plan.head.branch().map(str::to_owned);
    }
    if git.head_before.is_none() {
        git.head_before = plan.head.id().map(|id| id.to_string());
    }
    git.ref_updates = Some(fetched.ref_updates.clone());
    git.objects_received = Some(fetched.objects_received);
    git.bytes_received = Some(Size::from_bytes(fetched.bytes_received));
    let conflicts = git.conflicts.clone().unwrap_or_default();
    let effect = Effect {
        status: EffectStatus::Ok,
        kind: COMMAND.to_owned(),
        targets: Some(local.changes.len() as u64),
        notable: (!notable.is_empty()).then(|| {
            notable
                .iter()
                .map(|entry| entry.code.as_str().to_owned())
                .collect()
        }),
        irreversible: Some(irreversible.clone()),
        target: Some(format!("remote:{}", plan.target.remote)),
        git: Some(git),
        ..Effect::default()
    };
    let mut records = vec![Record::from(effect)];
    records.extend(fetched.ref_updates.into_iter().map(Record::from));
    records.extend(conflicts.into_iter().map(Record::from));
    EffectOutcome {
        output: StageOutput::records(RecordStream::from_records(records)),
        changes: local.changes,
        notable,
        irreversible,
        detached: false,
    }
}

impl StageImpl for GitPull {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for GitPull {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        ctx.checkpoint()?;
        let params = PullParams::from_args(args)?;
        let plan = PullPlan::read(&params)?;
        let mut effect = EffectPlan::new();
        effect.irreversible.push(IrreversibleEntry {
            reason: Irreversible::RemoteRefUpdated,
            path: None,
            count: None,
            detail: Some(plan.target.remote.clone()),
        });
        effect.preview.push(plan.preview());
        effect.bulk = true;
        Ok(effect.with_state(plan))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let plan: PullPlan = plan
            .take_state()
            .ok_or_else(|| internal("the plan of git.pull carries no state"))?;
        let request = FetchRequest {
            remote: plan.target.remote.clone(),
            refspecs: plan.target.refspecs.clone(),
            reflog_action: plan.reflog_action.clone(),
        };
        let fetched = self.steps.fetch(&plan.root, &request, tx, ctx)?;
        tx.mark_irreversible(IrreversibleEntry {
            reason: Irreversible::RemoteRefUpdated,
            path: None,
            count: Some(fetched.ref_updates.len() as u64),
            detail: Some(plan.target.remote.clone()),
        });
        let local = self
            .integrate(&plan, &fetched, tx, ctx)
            .map_err(|error| errors::after_transfer(error, &fetched.ref_updates))?;
        Ok(outcome(&plan, fetched, local))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fs;
    use std::sync::Mutex;
    use std::time::Duration;

    use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
    use nurt_contracts::fs::AbsPath;
    use nurt_contracts::journal::{
        AfterState, Applied, BeforeState, ExemptWrite, ExternalChange, Mutation, OpHandle, OpId,
        OpKind, PreserveMode, Preserved, RollbackReason, RollbackReport, TargetAction, TargetIndex,
        TargetPath, TargetSpec,
    };
    use nurt_contracts::record::{GitConflict, GitEffectState, GitRefStatus, GitRefUpdate};
    use nurt_contracts::stage::{ArgValue, CancelReason, CancelToken};
    use nurt_contracts::value::{RecordKind, Value};

    use super::super::probe::tests::Fixture;
    use super::super::steps::FetchedHead;
    use super::*;

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> WallTime {
            WallTime(0)
        }

        fn monotonic(&self) -> Duration {
            Duration::ZERO
        }

        fn stamp(&self) -> ClockStamp {
            ClockStamp {
                wall: WallTime(0),
                boot: BootId("boot".into()),
                since_boot_with_suspend: Duration::ZERO,
                since_boot_active: Duration::ZERO,
            }
        }

        fn sleep(
            &self,
            _duration: Duration,
            _cancel: &CancelToken,
        ) -> std::result::Result<(), CancelReason> {
            Ok(())
        }
    }

    struct EmptyHost(HostInfo);

    impl Host for EmptyHost {
        fn info(&self) -> &HostInfo {
            &self.0
        }

        fn env_var(&self, _name: &str) -> Option<OsString> {
            None
        }

        fn env_vars(&self) -> Vec<(OsString, OsString)> {
            Vec::new()
        }
    }

    fn ctx() -> ExecCtx {
        let info = HostInfo {
            os: Os::Linux,
            os_version: "6.8.0".into(),
            architecture: "x86_64".into(),
            hostname: "host".into(),
            user: "user".into(),
            home: PathBuf::from("/home/user"),
            temp: PathBuf::from("/tmp"),
            path_style: PathStyle::Posix,
            timezone: "UTC".into(),
            locale: None,
            cpus: 1,
        };
        ExecCtx::builder(Arc::new(FixedClock), Arc::new(EmptyHost(info))).build()
    }

    /// Transaction that applies byte writes to disk, keeps the previous
    /// content and restores it on [`RecordingTx::undo`]; any write outside a
    /// declared target panics.
    #[derive(Default)]
    struct RecordingTx {
        targets: Vec<(PathBuf, Option<Vec<u8>>)>,
        irreversible: Vec<IrreversibleEntry>,
        applied: usize,
    }

    impl RecordingTx {
        fn undo(&self) {
            for (path, before) in self.targets.iter().rev() {
                match before {
                    Some(bytes) => fs::write(path, bytes).unwrap(),
                    None => {
                        let _ = fs::remove_file(path);
                    }
                }
            }
        }
    }

    impl MutationTx for RecordingTx {
        fn op(&self) -> Option<OpId> {
            Some(OpId(1))
        }

        fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
            assert_eq!(self.applied, 0, "preserve after the first change");
            let mut preserved = Vec::new();
            for spec in targets {
                let TargetPath::Fs(path) = &spec.path else {
                    panic!("provider target")
                };
                let before = fs::read(path.as_path()).ok();
                let index = TargetIndex(u32::try_from(self.targets.len()).unwrap());
                preserved.push(Preserved {
                    index,
                    volume: None,
                    mode: PreserveMode::Copy,
                    bytes_held: before.as_ref().map_or(0, |bytes| bytes.len() as u64),
                    before: BeforeState {
                        exists: before.is_some(),
                        ..BeforeState::default()
                    },
                    irreversible: None,
                    notable: Vec::new(),
                });
                self.targets.push((path.as_path().to_path_buf(), before));
            }
            Ok(preserved)
        }

        fn stage(
            &mut self,
            _target: TargetIndex,
        ) -> Result<Box<dyn nurt_contracts::fs::WriteHandle>> {
            panic!("staged writes are not used by these tests")
        }

        fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
            let (path, _) = &self.targets[target.0 as usize];
            let Mutation::WriteBytes(bytes) = mutation else {
                panic!("unexpected mutation")
            };
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
            self.applied += 1;
            Ok(Applied {
                index: target,
                after: AfterState {
                    exists: true,
                    ..AfterState::default()
                },
            })
        }

        fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
            panic!("external writes are not used by these tests")
        }

        fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
            self.irreversible.push(marker);
        }

        fn record_exempt(&mut self, _write: ExemptWrite) {}

        fn commit(self: Box<Self>) -> Result<OpHandle> {
            Ok(OpHandle {
                op: Some(OpId(1)),
                kind: OpKind::command(COMMAND),
                bulk: true,
                targets: Vec::new(),
                irreversible: Vec::new(),
                notable: Vec::new(),
                exempt: Vec::new(),
            })
        }

        fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
            self.undo();
            Ok(RollbackReport {
                op: Some(OpId(1)),
                restored: Vec::new(),
                failed: Vec::new(),
            })
        }
    }

    fn write_through(tx: &mut dyn MutationTx, path: PathBuf, bytes: &[u8]) {
        let spec = TargetSpec {
            path: TargetPath::Fs(AbsPath::new(path).unwrap()),
            action: TargetAction::Modify,
            expect: Default::default(),
            size_hint: None,
        };
        let preserved = tx.preserve(&[spec]).unwrap();
        tx.apply(preserved[0].index, Mutation::WriteBytes(bytes.to_vec()))
            .unwrap();
    }

    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        Fetch(FetchRequest),
        Merge(MergeRequest),
        Rebase(RebaseRequest),
        Initial(InitialRequest),
    }

    /// Engines that write `FETCH_HEAD` and the branch through the
    /// transaction and record every request.
    struct FakeSteps {
        heads: Vec<FetchedHead>,
        local: std::result::Result<GitEffectState, Error>,
        calls: Mutex<Vec<Call>>,
    }

    impl FakeSteps {
        fn new(heads: Vec<FetchedHead>) -> Self {
            Self {
                heads,
                local: Ok(GitEffectState::FastForward),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }

        fn local(&self, repo: &Path, tx: &mut dyn MutationTx, call: Call) -> Result<LocalOutcome> {
            self.calls.lock().unwrap().push(call);
            let state = self.local.clone()?;
            let head = self
                .heads
                .first()
                .map(|head| head.id.to_string())
                .unwrap_or_default();
            write_through(
                tx,
                repo.join(".git/refs/heads/main"),
                format!("{head}\n").as_bytes(),
            );
            Ok(LocalOutcome {
                effect: GitEffect {
                    state,
                    head_after: Some(head),
                    conflicts: Some(vec![GitConflict {
                        path: "none.txt".into(),
                        ..GitConflict::default()
                    }]),
                    ..GitEffect::default()
                },
                changes: Vec::new(),
                notable: Vec::new(),
            })
        }
    }

    impl PullSteps for FakeSteps {
        fn fetch(
            &self,
            repo: &Path,
            request: &FetchRequest,
            tx: &mut dyn MutationTx,
            _ctx: &ExecCtx,
        ) -> Result<FetchOutcome> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Fetch(request.clone()));
            let text: String = self
                .heads
                .iter()
                .map(|head| format!("{}\t\t{}\n", head.id, head.description))
                .collect();
            write_through(tx, repo.join(".git/FETCH_HEAD"), text.as_bytes());
            Ok(FetchOutcome {
                ref_updates: vec![GitRefUpdate {
                    reference: "refs/remotes/origin/main".into(),
                    status: GitRefStatus::Updated,
                    remote: request.remote.clone(),
                    ..GitRefUpdate::default()
                }],
                for_merge: self.heads.clone(),
                objects_received: 3,
                bytes_received: 300,
                notable: Vec::new(),
            })
        }

        fn merge(
            &self,
            repo: &Path,
            request: &MergeRequest,
            tx: &mut dyn MutationTx,
            _ctx: &ExecCtx,
        ) -> Result<LocalOutcome> {
            self.local(repo, tx, Call::Merge(request.clone()))
        }

        fn rebase(
            &self,
            repo: &Path,
            request: &RebaseRequest,
            tx: &mut dyn MutationTx,
            _ctx: &ExecCtx,
        ) -> Result<LocalOutcome> {
            self.local(repo, tx, Call::Rebase(request.clone()))
        }

        fn checkout_initial(
            &self,
            repo: &Path,
            request: &InitialRequest,
            tx: &mut dyn MutationTx,
            _ctx: &ExecCtx,
        ) -> Result<LocalOutcome> {
            self.local(repo, tx, Call::Initial(request.clone()))
        }
    }

    fn head(id: ObjectId, branch: &str) -> FetchedHead {
        FetchedHead {
            id,
            description: format!("branch '{branch}' of ../upstream"),
            remote_ref: format!("refs/heads/{branch}"),
        }
    }

    /// History `base` - `local` on `main` and `base` - `upstream` fetched;
    /// `main` tracks `origin/main`.
    struct Scenario {
        fixture: Fixture,
        base: ObjectId,
        local: ObjectId,
        upstream: ObjectId,
    }

    impl Scenario {
        fn new(config: &str) -> Self {
            let mut fixture = Fixture::new();
            let base = fixture.commit(&[("a.txt", "a\n")], &[], "base");
            let local = fixture.commit(&[("a.txt", "a\n"), ("l.txt", "l\n")], &[base], "local");
            let upstream = fixture.commit(&[("a.txt", "a\nb\n")], &[base], "upstream");
            fixture.set_ref("refs/heads/main", local);
            fixture.set_ref("refs/remotes/origin/main", base);
            fixture.checkout(local, &[("a.txt", "a\n"), ("l.txt", "l\n")]);
            let path = fixture.repo.git_dir().join("config");
            let mut text = fs::read_to_string(&path).unwrap();
            text.push_str("[remote \"origin\"]\n\turl = ../upstream\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n");
            text.push_str("[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n");
            text.push_str(config);
            fs::write(path, text).unwrap();
            Self {
                fixture,
                base,
                local,
                upstream,
            }
        }

        fn args(&self, extra: &[(&str, Value)]) -> BoundArgs {
            let mut args = BoundArgs::new(COMMAND).with(
                "repo",
                ArgValue::Value(Value::Path(display_path(self.fixture.root()))),
                false,
            );
            for (name, value) in extra {
                args.insert(*name, ArgValue::Value(value.clone()), false);
            }
            args
        }

        fn run(
            &self,
            steps: &Arc<FakeSteps>,
            extra: &[(&str, Value)],
        ) -> (Result<Vec<Record>>, RecordingTx) {
            let stage = GitPull::new(steps.clone());
            let args = self.args(extra);
            let ctx = ctx();
            let plan = stage.plan(&args, StageInput::none(), &ctx).unwrap();
            let mut tx = RecordingTx::default();
            let result = stage
                .apply(&args, plan, &mut tx, &ctx)
                .and_then(|outcome| StageInput::from(outcome.output).into_records())
                .and_then(|records| records.collect_bounded(100));
            (result, tx)
        }

        fn plan_error(&self, extra: &[(&str, Value)]) -> Error {
            let stage = GitPull::new(Arc::new(FakeSteps::new(Vec::new())));
            stage
                .plan(&self.args(extra), StageInput::none(), &ctx())
                .unwrap_err()
        }
    }

    fn mode(mode: &str) -> (&'static str, Value) {
        ("mode", Value::Enum(mode.into()))
    }

    fn text(name: &'static str, value: &str) -> (&'static str, Value) {
        (name, Value::Text(value.into()))
    }

    #[test]
    fn describes_itself_as_a_source_effect_stage() {
        let stage = GitPull::new(Arc::new(FakeSteps::new(Vec::new())));
        let descriptor = stage.descriptor();
        assert_eq!(descriptor.command, COMMAND);
        assert!(descriptor.source);
        assert!(stage.as_effect().is_some());
        assert!(stage.as_read().is_none());
        assert!(format!("{stage:?}").starts_with("GitPull"));
    }

    #[test]
    fn fast_forwards_when_the_local_branch_is_behind() {
        let scenario = Scenario::new("");
        fs::write(
            scenario.fixture.repo.git_dir().join("refs/heads/main"),
            format!("{}\n", scenario.base),
        )
        .unwrap();
        scenario
            .fixture
            .checkout(scenario.base, &[("a.txt", "a\n")]);
        let steps = Arc::new(FakeSteps::new(vec![head(scenario.upstream, "main")]));
        let (records, tx) = scenario.run(&steps, &[]);
        let records = records.unwrap();
        let calls = steps.calls();
        assert_eq!(
            calls[0],
            Call::Fetch(FetchRequest {
                remote: "origin".into(),
                refspecs: Vec::new(),
                reflog_action: "pull".into(),
            })
        );
        let Call::Merge(merge) = &calls[1] else {
            panic!("expected a merge")
        };
        assert_eq!(merge.fast_forward, FfPolicy::Allow);
        assert_eq!(merge.message, "Merge branch 'main' of ../upstream");
        assert_eq!(merge.theirs, [scenario.upstream]);
        assert_eq!(tx.irreversible[0].reason, Irreversible::RemoteRefUpdated);
        assert_eq!(records[0].kind, RecordKind::Effect);
        let effect = Effect::try_from(records[0].clone()).unwrap();
        let git = effect.git.unwrap();
        assert_eq!(git.ref_updates.unwrap().len(), 1);
        assert_eq!(git.branch.as_deref(), Some("main"));
        assert_eq!(git.head_before, Some(scenario.base.to_string()));
        assert_eq!(git.objects_received, Some(3));
        assert_eq!(records[1].kind, RecordKind::GitRefUpdate);
        assert_eq!(records[2].kind, RecordKind::GitConflict);
    }

    #[test]
    fn undo_restores_fetch_head_and_the_branch() {
        let scenario = Scenario::new("[pull]\n\trebase = false\n");
        let git_dir = scenario.fixture.repo.git_dir().to_path_buf();
        let branch_before = fs::read(git_dir.join("refs/heads/main")).unwrap();
        let steps = Arc::new(FakeSteps::new(vec![head(scenario.upstream, "main")]));
        let (records, tx) = scenario.run(&steps, &[]);
        assert!(records.is_ok());
        assert_ne!(
            fs::read(git_dir.join("refs/heads/main")).unwrap(),
            branch_before
        );
        assert!(git_dir.join("FETCH_HEAD").exists());
        tx.undo();
        assert_eq!(
            fs::read(git_dir.join("refs/heads/main")).unwrap(),
            branch_before
        );
        assert!(!git_dir.join("FETCH_HEAD").exists());
    }

    #[test]
    fn refuses_diverged_histories_after_the_transfer() {
        let scenario = Scenario::new("");
        let steps = Arc::new(FakeSteps::new(vec![head(scenario.upstream, "main")]));
        let (records, tx) = scenario.run(&steps, &[]);
        let error = records.unwrap_err();
        assert_eq!(error.code, ErrorCode::GitNonFastForward);
        assert_eq!(error.problem.as_deref(), Some("diverged"));
        assert!(error.effect_applied);
        assert_eq!(
            error.details["ref_updates"][0]["ref"],
            serde_json::json!("refs/remotes/origin/main")
        );
        assert_eq!(steps.calls().len(), 1);
        assert_eq!(tx.targets.len(), 1, "only FETCH_HEAD was written");
    }

    #[test]
    fn merges_with_an_explicit_mode_and_names_it_in_the_reflog_action() {
        let scenario = Scenario::new("[pull]\n\tff = only\n");
        let steps = Arc::new(FakeSteps::new(vec![head(scenario.upstream, "main")]));
        let (records, _) = scenario.run(&steps, &[mode("merge")]);
        assert!(records.is_ok());
        let Call::Merge(merge) = &steps.calls()[1] else {
            panic!("expected a merge")
        };
        assert_eq!(merge.reflog_action, "pull --no-rebase");
        assert_eq!(merge.fast_forward, FfPolicy::Allow);
    }

    #[test]
    fn refuses_a_non_fast_forward_in_fast_forward_only_mode() {
        let scenario = Scenario::new("");
        let steps = Arc::new(FakeSteps::new(vec![head(scenario.upstream, "main")]));
        let (records, _) = scenario.run(&steps, &[mode("fast_forward_only")]);
        let error = records.unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("not_fast_forward"));
        assert_eq!(
            error.fix[0],
            format!(
                "git.pull \"{}\" mode: merge",
                display_path(scenario.fixture.root())
            )
        );
    }

    #[test]
    fn rebases_onto_the_fetched_head_with_configuration() {
        let scenario = Scenario::new("[pull]\n\trebase = true\n");
        let steps = Arc::new(FakeSteps::new(vec![head(scenario.upstream, "main")]));
        let (records, _) = scenario.run(&steps, &[]);
        assert!(records.is_ok());
        let Call::Rebase(rebase) = &steps.calls()[1] else {
            panic!("expected a rebase")
        };
        assert_eq!(rebase.onto, scenario.upstream);
        assert_eq!(
            rebase.upstream, scenario.upstream,
            "the fork point equals the merge base"
        );
        assert_eq!(rebase.kind, RebaseKind::Plain);
        assert_eq!(rebase.reflog_action, "pull");
    }

    #[test]
    fn limits_the_rebase_to_commits_after_the_fork_point() {
        let mut scenario = Scenario::new("[branch \"main\"]\n\trebase = merges\n");
        let old_upstream =
            scenario
                .fixture
                .commit(&[("a.txt", "old\n")], &[scenario.base], "old upstream");
        let local = scenario.fixture.commit(
            &[("a.txt", "old\n"), ("l.txt", "l\n")],
            &[old_upstream],
            "local on old upstream",
        );
        scenario.fixture.set_ref("refs/heads/main", local);
        scenario
            .fixture
            .set_ref("refs/remotes/origin/main", old_upstream);
        scenario
            .fixture
            .checkout(local, &[("a.txt", "old\n"), ("l.txt", "l\n")]);
        scenario.local = local;
        let steps = Arc::new(FakeSteps::new(vec![head(scenario.upstream, "main")]));
        let (records, _) = scenario.run(&steps, &[mode("rebase")]);
        assert!(records.is_ok());
        let Call::Rebase(rebase) = &steps.calls()[1] else {
            panic!("expected a rebase")
        };
        assert_eq!(rebase.upstream, old_upstream);
        assert_eq!(rebase.kind, RebaseKind::Plain);
        assert_eq!(rebase.reflog_action, "pull --rebase");
    }

    #[test]
    fn refuses_a_rebase_with_uncommitted_changes_before_the_transfer() {
        let scenario = Scenario::new("[pull]\n\trebase = true\n");
        fs::write(scenario.fixture.root().join("l.txt"), "changed\n").unwrap();
        let error = scenario.plan_error(&[]);
        assert_eq!(error.code, ErrorCode::GitDirty);
        assert_eq!(error.problem.as_deref(), Some("unstaged_changes"));
        assert_eq!(error.sample, ["l.txt"]);
        assert!(!error.effect_applied);
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
        let mut index = scenario.fixture.repo.index_from_tree(&tree).unwrap();
        index.write(Default::default()).unwrap();
        fs::write(scenario.fixture.root().join("a.txt"), "a\nb\n").unwrap();
        fs::remove_file(scenario.fixture.root().join("l.txt")).unwrap();
        let error = scenario.plan_error(&[mode("rebase")]);
        assert_eq!(error.problem.as_deref(), Some("staged_changes"));
    }

    #[test]
    fn autostash_lets_a_rebase_start_with_changes() {
        let scenario = Scenario::new("[rebase]\n\tautoStash = true\n");
        fs::write(scenario.fixture.root().join("l.txt"), "changed\n").unwrap();
        let steps = Arc::new(FakeSteps::new(vec![head(scenario.upstream, "main")]));
        let (records, _) = scenario.run(&steps, &[mode("rebase")]);
        assert!(records.is_ok());
        let Call::Rebase(rebase) = &steps.calls()[1] else {
            panic!("expected a rebase")
        };
        assert!(rebase.autostash);
    }

    #[test]
    fn refuses_unfinished_operations_before_the_transfer() {
        let scenario = Scenario::new("");
        let git_dir = scenario.fixture.repo.git_dir().to_path_buf();
        fs::create_dir(git_dir.join("rebase-merge")).unwrap();
        assert_eq!(
            scenario.plan_error(&[]).problem.as_deref(),
            Some("rebase_in_progress")
        );
        fs::write(
            git_dir.join("MERGE_HEAD"),
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
        let scenario = Scenario::new("[pull]\n\trebase = preserve\n");
        let error = scenario.plan_error(&[]);
        assert_eq!(error.code, ErrorCode::GitUnsupported);
        assert_eq!(error.fix.len(), 2);
        let ff = Scenario::new("[pull]\n\tff = maybe\n").plan_error(&[]);
        assert_eq!(ff.fix.len(), 1);
    }

    #[test]
    fn explains_why_nothing_can_be_merged() {
        let scenario = Scenario::new("");
        let steps = Arc::new(FakeSteps::new(Vec::new()));
        let (records, _) = scenario.run(&steps, &[text("branch", "gone")]);
        assert_eq!(
            records.unwrap_err().problem.as_deref(),
            Some("no_merge_candidates")
        );
        let (records, _) = scenario.run(&steps, &[text("remote", "other")]);
        assert_eq!(
            records.unwrap_err().problem.as_deref(),
            Some("branch_required")
        );
        let (records, _) = scenario.run(&steps, &[]);
        assert_eq!(
            records.unwrap_err().problem.as_deref(),
            Some("upstream_gone")
        );

        let without_upstream = Scenario::new("");
        let config = without_upstream.fixture.repo.git_dir().join("config");
        let text_before = fs::read_to_string(&config).unwrap();
        fs::write(
            &config,
            text_before.replace("\tmerge = refs/heads/main\n", ""),
        )
        .unwrap();
        let (records, _) = without_upstream.run(&steps, &[]);
        let error = records.unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("no_upstream"));
        assert!(error.effect_applied);

        let detached = Scenario::new("");
        fs::write(
            detached.fixture.repo.git_dir().join("HEAD"),
            format!("{}\n", detached.local),
        )
        .unwrap();
        let (records, _) = detached.run(&steps, &[]);
        assert_eq!(
            records.unwrap_err().problem.as_deref(),
            Some("detached_head")
        );
    }

    #[test]
    fn passes_the_explicit_branch_to_the_transfer() {
        let scenario = Scenario::new("[pull]\n\trebase = false\n");
        let steps = Arc::new(FakeSteps::new(vec![head(scenario.upstream, "dev")]));
        let (records, _) = scenario.run(&steps, &[text("remote", "origin"), text("branch", "dev")]);
        assert!(records.is_ok());
        let Call::Fetch(fetch) = &steps.calls()[0] else {
            panic!("expected a fetch")
        };
        assert_eq!(fetch.refspecs, ["dev"]);
        assert_eq!(fetch.reflog_action, "pull origin dev");
        let Call::Merge(merge) = &steps.calls()[1] else {
            panic!("expected a merge")
        };
        assert_eq!(merge.message, "Merge branch 'dev' of ../upstream");
    }

    #[test]
    fn checks_out_the_fetched_head_on_an_unborn_branch() {
        let mut fixture = Fixture::new();
        let upstream = fixture.commit(&[("a.txt", "a\n")], &[], "init");
        let scenario = Scenario {
            fixture,
            base: upstream,
            local: upstream,
            upstream,
        };
        let steps = Arc::new(FakeSteps::new(vec![head(upstream, "main")]));
        let (records, _) =
            scenario.run(&steps, &[text("remote", "origin"), text("branch", "main")]);
        assert!(records.is_ok());
        let Call::Initial(initial) = &steps.calls()[1] else {
            panic!("expected an initial checkout")
        };
        assert_eq!(initial.branch, "refs/heads/main");
        assert_eq!(initial.reflog_message, INITIAL_PULL);
    }

    #[test]
    fn keeps_the_transfer_when_the_merge_reports_conflicts() {
        let scenario = Scenario::new("[pull]\n\trebase = false\n");
        let mut steps = FakeSteps::new(vec![head(scenario.upstream, "main")]);
        steps.local =
            Err(Error::new(ErrorCode::GitConflict, "conflicts in a.txt").with_effect_applied(true));
        let (records, _) = scenario.run(&Arc::new(steps), &[]);
        let error = records.unwrap_err();
        assert_eq!(error.code, ErrorCode::GitConflict);
        assert!(error.effect_applied);
        assert!(error.details.contains_key("ref_updates"));
    }

    #[test]
    fn previews_the_remote_and_the_mode() {
        let scenario = Scenario::new("[pull]\n\trebase = true\n");
        let stage = GitPull::new(Arc::new(FakeSteps::new(Vec::new())));
        let args = scenario.args(&[]);
        let plan = stage.plan(&args, StageInput::none(), &ctx()).unwrap();
        assert_eq!(plan.irreversible[0].reason, Irreversible::RemoteRefUpdated);
        let preview = Effect::try_from(plan.preview[0].clone()).unwrap();
        assert_eq!(preview.status, EffectStatus::Planned);
        assert_eq!(preview.action.as_deref(), Some("rebase"));
        assert_eq!(preview.target.as_deref(), Some("remote:origin"));
        let labels: BTreeMap<_, _> = [
            (
                strategy_label(&Strategy {
                    rebase: RebaseKind::No,
                    ff: Some(FfPolicy::Only),
                    rebase_unspecified: false,
                    autostash: false,
                }),
                1,
            ),
            (
                strategy_label(&Strategy {
                    rebase: RebaseKind::No,
                    ff: None,
                    rebase_unspecified: true,
                    autostash: false,
                }),
                2,
            ),
        ]
        .into_iter()
        .collect();
        assert!(labels.contains_key("fast_forward_only") && labels.contains_key("merge"));
    }

    #[test]
    fn rejects_an_apply_without_a_plan() {
        let stage = GitPull::new(Arc::new(FakeSteps::new(Vec::new())));
        let args = BoundArgs::new(COMMAND);
        let ctx = ctx();
        let mut tx = RecordingTx::default();
        let error = stage
            .apply(&args, EffectPlan::new(), &mut tx, &ctx)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Internal);
    }
}
