// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Transfer and rebase engines of a pull built on the fetch and rebase
//! planners of the crate family.

use bstr::BString;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::stage::ExecCtx;
use nurt_git::apply::{ApplyContext, ApplyPlan};
use nurt_git::fetch::{FetchRequest as GitFetchRequest, plan_fetch};
use nurt_git::repo::GitRepo;

use super::integrate::identity;
use super::settings::RebaseKind;
use super::steps::{
    FetchOutcome, FetchRequest, FetchedHead, LocalOutcome, PullFetch, PullRebase, RebaseError,
    RebaseRequest,
};
use crate::rebase::{RebaseOnto, RebaseRequest as EngineRequest, apply_rebase, plan_rebase};
use crate::reset::file_changes;

/// Transfer engine that plans the fetch with [`plan_fetch`].
///
/// Remote-tracking references are never pruned: a pull does not delete
/// references, and the plan of the integration cannot express deletions.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlanFetch;

impl PullFetch for PlanFetch {
    fn fetch(
        &self,
        repo: &GitRepo,
        request: &FetchRequest,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<FetchOutcome> {
        let mut call = GitFetchRequest::new(request.remote.clone());
        call.refspecs.clone_from(&request.refspecs);
        call.prune = Some(false);
        call.reflog_action = Some(request.reflog_action.clone());
        call.accept_host_key.clone_from(&request.accept_host_key);
        let plan = plan_fetch(repo, &call, tx)?;
        let (_, committer) = identity(repo, ctx);
        Ok(FetchOutcome {
            url: plan.url.clone(),
            refs: plan.ref_plans(repo.gix(), &committer)?,
            state_files: plan.state_files(),
            ref_updates: plan.ref_updates(),
            for_merge: plan
                .for_merge
                .iter()
                .map(|head| FetchedHead {
                    id: head.id,
                    description: head.description.clone(),
                    remote_ref: head.remote_ref.clone(),
                })
                .collect(),
            objects_received: plan.objects_received,
            bytes_received: plan.bytes_received,
            notable: plan.notable,
        })
    }
}

/// Rebase engine that plans the rebase with [`plan_rebase`] and writes the
/// transfer in the same transaction.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlanRebase;

impl PullRebase for PlanRebase {
    fn rebase(
        &self,
        repo: &GitRepo,
        request: &RebaseRequest,
        transfer: ApplyPlan,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> std::result::Result<LocalOutcome, RebaseError> {
        if request.kind == RebaseKind::Merges {
            return Err(RebaseError::Refused(merges_unsupported()));
        }
        let ApplyPlan {
            refs, state_files, ..
        } = transfer;
        let apply_ctx = ApplyContext::open(repo.gix()).map_err(RebaseError::Refused)?;
        let (_, committer) = identity(repo, ctx);
        let mut engine = EngineRequest::start(
            request.repo.clone(),
            RebaseOnto {
                commit: request.onto,
                label: request.onto.to_string(),
            },
            committer,
        );
        engine.upstream = Some(request.upstream);
        engine.reflog_action = Some(request.reflog_action.clone());
        let mut plan = plan_rebase(&apply_ctx, &engine).map_err(RebaseError::Refused)?;
        plan.apply.refs.extend(refs);
        plan.apply.state_files.extend(state_files);
        let result = apply_rebase(tx, &apply_ctx, plan).map_err(RebaseError::Failed)?;
        if let Some(error) = result.summary.stop_error(&request.repo) {
            return Err(RebaseError::Failed(error));
        }
        let text = |path: &BString| String::from_utf8_lossy(path).into_owned();
        let written: Vec<String> = result.report.written.iter().map(text).collect();
        let removed: Vec<String> = result.report.removed.iter().map(text).collect();
        Ok(LocalOutcome {
            effect: result.summary.effect(repo.root()),
            changes: file_changes(repo.root(), &written, &removed),
            notable: result.summary.notes,
            ..LocalOutcome::default()
        })
    }
}

fn merges_unsupported() -> Error {
    Error::new(
        ErrorCode::GitUnsupported,
        "a rebase that recreates merge commits is not available; give `mode: merge`",
    )
    .with_problem("rebase_merges")
}
