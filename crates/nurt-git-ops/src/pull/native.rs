// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::pull::native
// Responsibility:   Transfer and rebase engines of a pull built on the fetch and rebase.
// Dependencies:     bstr, nurt_contracts, nurt_git
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Transfer and rebase engines of a pull built on the fetch and rebase
//! planners of the crate family.

use bstr::{BString, ByteSlice};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::stage::ExecCtx;
use nurt_git::apply::{ApplyContext, ApplyPlan, RefPlan, RefTarget, ReflogLine};
use nurt_git::fetch::{FetchPlan, FetchRequest as GitFetchRequest, plan_fetch};
use nurt_git::repo::GitRepo;

use super::integrate::identity;
use super::settings::RebaseKind;
use super::steps::{
    FetchOutcome, FetchRequest, FetchedHead, LocalOutcome, PullFetch, PullRebase, RebaseError,
    RebaseRequest,
};
use crate::rebase::{RebaseOnto, RebaseRequest as EngineRequest, apply_rebase, plan_rebase};
use crate::reset::file_changes;

/// Reflog message of the creation of `refs/remotes/<remote>/HEAD`.
const REMOTE_HEAD_REFLOG: &str = "fetch";

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
        call.refspecs = request
            .refspecs
            .iter()
            .map(|spec| tracking_refspec(repo, &request.remote, spec))
            .collect();
        call.prune = Some(false);
        call.reflog_action = Some(request.reflog_action.clone());
        call.accept_host_key.clone_from(&request.accept_host_key);
        let plan = plan_fetch(repo, &call, tx)?;
        let (_, committer) = identity(repo, ctx);
        let mut refs = plan.ref_plans(repo.gix(), &committer)?;
        refs.extend(remote_head_plan(repo, &plan, &committer)?);
        Ok(FetchOutcome {
            url: plan.url.clone(),
            refs,
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

/// Plan of the symbolic `refs/remotes/<remote>/HEAD` that the first fetch of
/// a remote creates; `ref_plans` covers object references only.
fn remote_head_plan(
    repo: &GitRepo,
    plan: &FetchPlan,
    committer: &gix::actor::Signature,
) -> Result<Option<RefPlan>> {
    let Some((name, target)) = &plan.remote_head else {
        return Ok(None);
    };
    let Some(new) = plan
        .changes
        .iter()
        .find(|change| &change.name == target)
        .and_then(|change| change.new)
    else {
        return Ok(None);
    };
    let line = ReflogLine {
        old: repo.gix().object_hash().null(),
        new,
        committer: committer.clone(),
        message: REMOTE_HEAD_REFLOG.to_owned(),
    };
    RefPlan::new(
        repo.gix(),
        name,
        RefTarget::Symbolic(target.clone()),
        Some(line),
    )
    .map(Some)
}

/// Adds the remote-tracking destination that the configured refspecs give to
/// the branch or reference `spec`, which `git` updates opportunistically;
/// a `spec` with a destination or without a matching configured refspec is
/// returned unchanged.
fn tracking_refspec(repo: &GitRepo, remote: &str, spec: &str) -> String {
    if spec.contains(':') {
        return spec.to_owned();
    }
    let source = if spec.starts_with("refs/") {
        spec.to_owned()
    } else {
        format!("refs/heads/{spec}")
    };
    let key = format!("remote.{remote}.fetch");
    let configured = repo.gix().config_snapshot();
    let values = configured.plumbing().strings(&key).unwrap_or_default();
    for value in &values {
        let text = value.to_str_lossy();
        let text = text.strip_prefix('+').unwrap_or(&text);
        let Some((from, to)) = text.split_once(':') else {
            continue;
        };
        if let (Some(prefix), Some(target)) = (from.strip_suffix('*'), to.strip_suffix('*'))
            && let Some(rest) = source.strip_prefix(prefix)
        {
            return format!("{source}:{target}{rest}");
        }
        if from == source && !to.is_empty() {
            return format!("{source}:{to}");
        }
    }
    spec.to_owned()
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
        if request.kind == RebaseKind::Merges
            && replays_merge_commits(repo, request).map_err(RebaseError::Refused)?
        {
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
        })
    }
}

/// Whether the local commits to replay include a merge commit; without one
/// `rebase.merges` produces the same history as a plain rebase.
fn replays_merge_commits(repo: &GitRepo, request: &RebaseRequest) -> Result<bool> {
    let gix = repo.gix();
    let walk_error = |error: &dyn std::fmt::Display| {
        Error::new(
            ErrorCode::Io,
            format!("cannot walk the local commits: {error}"),
        )
    };
    let head = gix.head_id().map_err(|error| walk_error(&error))?;
    let walk = gix
        .rev_walk([head.detach()])
        .with_hidden([request.upstream])
        .all()
        .map_err(|error| walk_error(&error))?;
    for info in walk {
        let info = info.map_err(|error| walk_error(&error))?;
        if info.parent_ids.len() > 1 {
            return Ok(true);
        }
    }
    Ok(false)
}

fn merges_unsupported() -> Error {
    Error::new(
        ErrorCode::GitUnsupported,
        "a rebase that recreates merge commits is not available; give `mode: merge`",
    )
    .with_problem("rebase_merges")
}

#[cfg(test)]
mod tests;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
