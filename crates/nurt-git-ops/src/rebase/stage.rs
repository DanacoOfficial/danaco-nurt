// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage implementation of `git.rebase`.

use std::path::{Path, PathBuf};

use bstr::BString;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_git::apply::{ApplyContext, ApplyPlan, EntryAction};
use nurt_git::repo::GitRepo;

use super::command::{
    RebaseAction, RebaseOnto, RebasePlan, RebaseRequest, RebaseSummary, apply_rebase, plan_rebase,
};
use crate::reset::args;
use crate::reset::{effect_record, file_changes};

/// Name of the command in the catalog.
const COMMAND: &str = "git.rebase";

/// Nanoseconds per second of the clock of the execution context.
const NANOSECONDS_PER_SECOND: i128 = 1_000_000_000;

/// Stage implementation of `git.rebase`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RebaseCommand;

struct Planned {
    root: PathBuf,
    label: String,
    plan: RebasePlan,
}

impl StageImpl for RebaseCommand {
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

impl EffectStage for RebaseCommand {
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
        let plan = plan_rebase(&apply_ctx, &request)?;
        let root = repo.root().to_path_buf();
        let (written, removed) = planned_paths(&plan.apply);
        let mut effect = EffectPlan::new();
        effect.changes = file_changes(&root, &written, &removed);
        effect.notable.clone_from(&plan.summary.notes);
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
            Error::new(ErrorCode::StageFailed, "the plan of git.rebase is missing")
        })?;
        let repo = GitRepo::open(&planned.root, ctx)?;
        let apply_ctx = ApplyContext::open(repo.gix())?;
        let result = apply_rebase(tx, &apply_ctx, planned.plan)?;
        if let Some(error) = result.summary.stop_error(&planned.label) {
            return Err(error);
        }
        let written: Vec<String> = result.report.written.iter().map(text).collect();
        let removed: Vec<String> = result.report.removed.iter().map(text).collect();
        Ok(outcome(&planned.root, &result.summary, &written, &removed))
    }
}

fn outcome<'a>(
    root: &Path,
    summary: &RebaseSummary,
    written: &[String],
    removed: &[String],
) -> EffectOutcome<'a> {
    let changes = file_changes(root, written, removed);
    let records = vec![effect_record(
        COMMAND,
        summary.effect(root),
        &changes,
        &summary.notes,
    )];
    let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::from_records(records)));
    outcome.changes = changes;
    outcome.notable.clone_from(&summary.notes);
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
) -> Result<RebaseRequest> {
    let action = match args::text(args, "action")? {
        Some(value) => RebaseAction::parse(&value).ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("action has the unknown value {value}; use start, continue, skip or abort"),
            )
            .with_found(value.clone())
        })?,
        None => RebaseAction::Start,
    };
    let mut request = RebaseRequest::new(label, action, committer(repo, ctx));
    if action == RebaseAction::Start
        && let Some(onto) = args::text(args, "onto")?
    {
        request.onto = Some(RebaseOnto::resolve(repo, label, &onto)?);
    }
    Ok(request)
}

/// Committer from `GIT_COMMITTER_*` of the environment of the context, then
/// `user.name` and `user.email`, then the account and host names; the time
/// is the clock of the context with the local offset of the host.
fn committer(repo: &gix::Repository, ctx: &ExecCtx) -> gix::actor::Signature {
    let host = ctx.host();
    let info = host.info();
    let config = repo.config_snapshot();
    let var = |name: &str| -> Option<String> {
        host.env_var(name)
            .and_then(|value| value.into_string().ok())
            .filter(|value| !value.is_empty())
    };
    let name = var("GIT_COMMITTER_NAME")
        .or_else(|| config.string("user.name").map(|value| value.to_string()))
        .unwrap_or_else(|| info.user.clone());
    let email = var("GIT_COMMITTER_EMAIL")
        .or_else(|| config.string("user.email").map(|value| value.to_string()))
        .unwrap_or_else(|| format!("{}@{}", info.user, info.hostname));
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
        .map_or(0, |(zone, instant)| zone.to_offset(instant).seconds());
    let time = var("GIT_COMMITTER_DATE")
        .and_then(|value| gix::date::parse(&value, None).ok())
        .unwrap_or_else(|| gix::date::Time::new(seconds, offset));
    gix::actor::Signature {
        name: name.into(),
        email: email.into(),
        time,
    }
}
