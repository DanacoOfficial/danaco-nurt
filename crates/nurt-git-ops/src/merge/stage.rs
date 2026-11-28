// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::merge::stage
// Responsibility:   Stage implementation of `git.merge`.
// Dependencies:     bstr, nurt_contracts, nurt_git
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Stage implementation of `git.merge`.

use std::path::{Path, PathBuf};

use bstr::BString;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::Record;
use nurt_git::apply::{ApplyContext, ApplyPlan, EntryAction};
use nurt_git::repo::GitRepo;

use super::command::{
    FastForward, MergeAction, MergePlan, MergeRequest, MergeResult, apply_merge, plan_merge,
};
use super::source::MergeTarget;
use crate::reset::args;
use crate::reset::{effect_record, file_changes};

/// Name of the command in the catalog.
const COMMAND: &str = "git.merge";

/// Nanoseconds per second of the clock of the execution context.
const NANOSECONDS_PER_SECOND: i128 = 1_000_000_000;

/// Stage implementation of `git.merge`.
#[derive(Debug, Clone, Copy, Default)]
pub struct MergeCommand;

struct Planned {
    root: PathBuf,
    label: String,
    plan: MergePlan,
}

impl StageImpl for MergeCommand {
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

impl EffectStage for MergeCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let label = args::required_text(args, "repo")?;
        let repo = GitRepo::discover(Path::new(&label), ctx)?;
        let request = request_of(args, repo.gix(), &label, ctx)?;
        let apply_ctx = ApplyContext::open(repo.gix())?;
        let plan = plan_merge(&apply_ctx, &request)?;
        let root = repo.root().to_path_buf();
        let (written, removed) = planned_paths(&plan.apply);
        let mut effect = EffectPlan::new();
        effect.changes = file_changes(&root, &written, &removed);
        effect.notable.clone_from(&plan.notes);
        effect.bulk = effect.changes.len() > 1;
        Ok(effect.with_state(Planned { root, label, plan }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let planned = plan.take_state::<Planned>().ok_or_else(|| {
            Error::new(ErrorCode::StageFailed, "the plan of git.merge is missing")
        })?;
        let repo = GitRepo::open(&planned.root, ctx)?;
        let apply_ctx = ApplyContext::open(repo.gix())?;
        let result = apply_merge(tx, &apply_ctx, planned.plan)?;
        if let Some(error) = result.conflict_error(&planned.label) {
            return Err(error);
        }
        Ok(outcome(&planned.root, &result))
    }
}

fn outcome<'a>(root: &Path, result: &MergeResult) -> EffectOutcome<'a> {
    let written: Vec<String> = result.report.written.iter().map(text).collect();
    let removed: Vec<String> = result.report.removed.iter().map(text).collect();
    let changes = file_changes(root, &written, &removed);
    let mut records = vec![effect_record(
        COMMAND,
        result.effect(root),
        &changes,
        &result.notes,
    )];
    records.extend(
        result
            .conflicts
            .iter()
            .map(|conflict| Record::from(conflict.to_record())),
    );
    let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::from_records(records)));
    outcome.changes = changes;
    outcome.notable.clone_from(&result.notes);
    outcome
}

fn text(path: &BString) -> String {
    String::from_utf8_lossy(path).into_owned()
}

/// Paths the plan writes and removes, relative to the working tree root.
fn planned_paths(plan: &ApplyPlan) -> (Vec<String>, Vec<String>) {
    let mut written = Vec::new();
    let mut removed = Vec::new();
    for entry in &plan.entries {
        match entry.action {
            EntryAction::Delete | EntryAction::Rmdir => removed.push(text(&entry.path)),
            EntryAction::Mkdir => {}
            _ => written.push(text(&entry.path)),
        }
    }
    (written, removed)
}

fn request_of(
    args: &BoundArgs,
    repo: &gix::Repository,
    label: &str,
    ctx: &ExecCtx,
) -> Result<MergeRequest> {
    let action = match args::text(args, "action")? {
        Some(text) => MergeAction::parse(&text)
            .ok_or_else(|| invalid("action", &text, "start, continue or abort"))?,
        None => MergeAction::Start,
    };
    let (author, committer) = identity(repo, ctx);
    let mut request = MergeRequest::new(label, action, committer);
    request.author = author;
    if action == MergeAction::Start
        && let Some(branch) = args::text(args, "branch")?
    {
        request.target = Some(MergeTarget::resolve(repo, &branch)?);
    }
    if let Some(text) = args::text(args, "fast_forward")? {
        request.fast_forward = FastForward::parse(&text)
            .ok_or_else(|| invalid("fast_forward", &text, "auto, only or never"))?;
    }
    request.message = args::text(args, "message")?;
    request.no_commit = args::flag(args, "no_commit")?;
    Ok(request)
}

fn invalid(name: &str, value: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("{name} has the unknown value {value}; use {expected}"),
    )
    .with_found(value.to_owned())
}

/// Author and committer from `GIT_AUTHOR_*` and `GIT_COMMITTER_*` of the
/// environment of the context, then `user.name` and `user.email`, then the
/// account and host names; the time is the clock of the context with the
/// local offset of the host.
fn identity(
    repo: &gix::Repository,
    ctx: &ExecCtx,
) -> (gix::actor::Signature, gix::actor::Signature) {
    let host = ctx.host();
    let info = host.info();
    let config = repo.config_snapshot();
    let var = |name: &str| -> Option<String> {
        host.env_var(name)
            .and_then(|value| value.into_string().ok())
            .filter(|value| !value.is_empty())
    };
    let default_name = config
        .string("user.name")
        .map_or_else(|| info.user.clone(), |value| value.to_string());
    let default_email = config.string("user.email").map_or_else(
        || format!("{}@{}", info.user, info.hostname),
        |value| value.to_string(),
    );
    let seconds = i64::try_from(
        ctx.clock()
            .now()
            .unix_nanos()
            .div_euclid(NANOSECONDS_PER_SECOND),
    )
    .unwrap_or_default();
    let offset = jiff::tz::TimeZone::get(&info.timezone)
        .ok()
        .zip(jiff::Timestamp::from_second(seconds).ok())
        .map_or(0, |(tz, instant)| tz.to_offset(instant).seconds());
    let now = gix::date::Time::new(seconds, offset);
    let pick = |role: &str| gix::actor::Signature {
        name: var(&format!("GIT_{role}_NAME"))
            .unwrap_or_else(|| default_name.clone())
            .into(),
        email: var(&format!("GIT_{role}_EMAIL"))
            .unwrap_or_else(|| default_email.clone())
            .into(),
        time: var(&format!("GIT_{role}_DATE"))
            .and_then(|text| gix::date::parse(&text, None).ok())
            .unwrap_or(now),
    };
    (pick("AUTHOR"), pick("COMMITTER"))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
