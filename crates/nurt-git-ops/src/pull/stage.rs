// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.pull` stage: checks in the plan step, transfer and integration
//! in one transaction in the apply step.

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
use super::integrate::{self, Failure, MergeInput};
use super::message::merge_message;
use super::params::{PullMode, PullParams};
use super::probe::{self, HeadState, Repo};
use super::settings::{
    ConfigSource, FfPolicy, RebaseKind, Strategy, Target, full_source_ref, reflog_action,
    resolve_strategy, resolve_target, tracking_ref,
};
use super::steps::{
    FetchOutcome, FetchRequest, LocalOutcome, PullFetch, PullRebase, RebaseError, RebaseRequest,
};

const COMMAND: &str = "git.pull";

/// Implementation of `git.pull`.
///
/// The stage composes the transfer engine and, for pulls that rebase, the
/// rebase engine; merges and fast-forwards are planned with
/// [`plan_merge`](crate::merge::plan_merge). Every local write of the
/// transfer and of the integration goes through one transaction, whose
/// targets are preserved together before the first write.
pub struct GitPull {
    fetch: Arc<dyn PullFetch>,
    rebase: Option<Arc<dyn PullRebase>>,
}

impl GitPull {
    /// Creates the stage over the given transfer engine; a pull that has to
    /// rebase fails with `E_GIT_UNSUPPORTED` until a rebase engine is added
    /// with [`GitPull::with_rebase`].
    #[must_use]
    pub fn new(fetch: Arc<dyn PullFetch>) -> Self {
        Self {
            fetch,
            rebase: None,
        }
    }

    /// Adds the engine of pulls that rebase.
    #[must_use]
    pub fn with_rebase(mut self, rebase: Arc<dyn PullRebase>) -> Self {
        self.rebase = Some(rebase);
        self
    }
}

impl std::fmt::Debug for GitPull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitPull")
            .field("rebase", &self.rebase.is_some())
            .finish_non_exhaustive()
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
    accept_host_key: Option<String>,
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
    fn read(params: &PullParams, ctx: &ExecCtx) -> Result<Self> {
        let repo = probe::open(&params.repo, ctx)?;
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
            accept_host_key: params.accept_host_key.clone(),
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

    fn transfer_marker(&self, count: Option<u64>) -> IrreversibleEntry {
        IrreversibleEntry {
            reason: Irreversible::RemoteRefUpdated,
            path: None,
            raw_path: None,
            count,
            detail: Some(self.target.remote.clone()),
        }
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

impl GitPull {
    fn integrate(
        &self,
        plan: &PullPlan,
        repo: &Repo,
        fetched: &FetchOutcome,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> std::result::Result<LocalOutcome, Failure> {
        if fetched.for_merge.is_empty() {
            return Err(Failure::Refused(plan.no_candidates()));
        }
        let theirs: Vec<ObjectId> = fetched.for_merge.iter().map(|head| head.id).collect();
        let relation = relation(repo, plan.head.id(), &theirs).map_err(Failure::Refused)?;
        let action = decide(&plan.strategy, &relation)
            .map_err(|refusal| Failure::Refused(plan.refusal(refusal)))?;
        ctx.checkpoint().map_err(Failure::Failed)?;
        match action {
            Action::InitialCheckout => self.merge(plan, repo, fetched, FfPolicy::Allow, tx, ctx),
            Action::Merge(_) if theirs.len() > 1 => {
                Err(Failure::Refused(errors::octopus_merge(&plan.call)))
            }
            Action::Merge(policy) => self.merge(plan, repo, fetched, policy, tx, ctx),
            Action::Rebase(kind) => self.rebase(plan, repo, fetched, kind, tx, ctx),
        }
    }

    fn merge(
        &self,
        plan: &PullPlan,
        repo: &Repo,
        fetched: &FetchOutcome,
        fast_forward: FfPolicy,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> std::result::Result<LocalOutcome, Failure> {
        let descriptions: Vec<String> = fetched
            .for_merge
            .iter()
            .map(|head| head.description.clone())
            .collect();
        let input = MergeInput {
            label: plan.call.repo(),
            fetched,
            message: merge_message(
                &descriptions,
                plan.head.branch(),
                plan.suppress_dest.as_deref(),
            ),
            fast_forward,
            reflog_action: &plan.reflog_action,
            initial: plan.head.id().is_none(),
        };
        integrate::merge(repo.git(), &input, tx, ctx)
    }

    fn rebase(
        &self,
        plan: &PullPlan,
        repo: &Repo,
        fetched: &FetchOutcome,
        kind: RebaseKind,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> std::result::Result<LocalOutcome, Failure> {
        let Some(engine) = &self.rebase else {
            return Err(Failure::Refused(errors::rebase_unavailable(&plan.call)));
        };
        let onto = fetched.for_merge[0].id;
        let request = RebaseRequest {
            repo: plan.call.repo().to_owned(),
            upstream: rebase_upstream(repo, plan, onto).map_err(Failure::Refused)?,
            onto,
            kind,
            autostash: plan.strategy.autostash,
            reflog_action: plan.reflog_action.clone(),
        };
        engine
            .rebase(repo.git(), &request, fetched.local_plan(), tx, ctx)
            .map_err(|error| match error {
                RebaseError::Refused(error) => Failure::Refused(error),
                RebaseError::Failed(error) => Failure::Failed(error),
            })
    }
}

fn outcome<'a>(plan: &PullPlan, fetched: FetchOutcome, local: LocalOutcome) -> EffectOutcome<'a> {
    let irreversible = vec![plan.transfer_marker(Some(fetched.ref_updates.len() as u64))];
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
        changes: (!local.changes.is_empty()).then(|| local.changes.clone()),
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
        let plan = PullPlan::read(&params, ctx)?;
        let mut effect = EffectPlan::new();
        effect.irreversible.push(plan.transfer_marker(None));
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
            accept_host_key: plan.accept_host_key.clone(),
        };
        let before = probe::open(&plan.root, ctx)?;
        let fetched = self.fetch.fetch(before.git(), &request, tx, ctx)?;
        tx.mark_irreversible(plan.transfer_marker(Some(fetched.ref_updates.len() as u64)));
        let repo = probe::open(&plan.root, ctx)?;
        match self.integrate(&plan, &repo, &fetched, tx, ctx) {
            Ok(local) => Ok(outcome(&plan, fetched, local)),
            Err(Failure::Refused(error)) => {
                integrate::write_transfer(repo.git(), &fetched, &error, tx)?;
                Err(errors::after_transfer(error, &fetched.ref_updates))
            }
            Err(Failure::Failed(error)) if error.effect_applied => {
                Err(errors::after_transfer(error, &fetched.ref_updates))
            }
            Err(Failure::Failed(error)) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests;
