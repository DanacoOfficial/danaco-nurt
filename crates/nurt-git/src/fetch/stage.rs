// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The executor stage of `git.fetch`.

use std::path::PathBuf;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{
    ChangeAction, Effect, EffectStatus, GitEffect, GitEffectState, GitRefStatus,
};
use nurt_contracts::report::{Change, Irreversible, IrreversibleEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Path as RecordPath, Record, Size};

use super::{COMMAND, FetchPlan, FetchRequest, TagMode, fetch, ref_file, reflog_action, remote};
use crate::checkpoint::args;
use crate::repo::GitRepo;
use crate::repo::records::short_ref_name;

/// Remote fetched when the call names none.
const DEFAULT_REMOTE: &str = "origin";

/// Implementation of `git.fetch` for the executor.
///
/// The plan checks the repository and the remote without contacting it; the
/// transfer and every write happen when the effect is applied. The output
/// is one `effect` record followed by one `git_ref_update` record per
/// reference considered.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitFetchCommand;

/// State carried from the plan to the application.
struct Planned {
    path: PathBuf,
    request: FetchRequest,
}

impl StageImpl for GitFetchCommand {
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

impl EffectStage for GitFetchCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let path = args::repo(args)?;
        let request = request(args)?;
        let repo = GitRepo::discover(&path, ctx)?;
        remote::resolve(&repo, &request)?;
        let mut plan = EffectPlan::new();
        plan.irreversible = vec![IrreversibleEntry::new(Irreversible::RemoteRefUpdated)];
        Ok(plan.with_state(Planned { path, request }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let planned = plan.take_state::<Planned>().ok_or_else(|| {
            Error::new(
                ErrorCode::Internal,
                format!("the plan of `{COMMAND}` was made by another command"),
            )
        })?;
        let repo = GitRepo::discover(&planned.path, ctx)?;
        let head = repo.head_id()?;
        let fetched = fetch(&repo, &planned.request, tx)?;
        let changes = changes(&repo, &fetched);
        let mut marker = IrreversibleEntry::new(Irreversible::RemoteRefUpdated);
        marker.detail = Some(fetched.url.clone());
        let branch = repo
            .head_ref()?
            .map(|name| short_ref_name(&name.as_bstr().to_string()).to_owned());
        let git = GitEffect {
            repo: RecordPath::new(repo.root().display().to_string()),
            state: if fetched.is_up_to_date() {
                GitEffectState::UpToDate
            } else {
                GitEffectState::Completed
            },
            head_before: head.map(|id| id.to_string()),
            head_after: head.map(|id| id.to_string()),
            branch: branch.clone(),
            branch_before: branch,
            detached: Some(repo.head_ref()?.is_none()),
            ref_updates: Some(fetched.ref_updates()),
            objects_received: Some(fetched.objects_received),
            bytes_received: Some(Size::from_bytes(fetched.bytes_received)),
            ..GitEffect::default()
        };
        let effect = Effect {
            status: EffectStatus::Ok,
            kind: COMMAND.to_owned(),
            targets: u64::try_from(changes.len()).ok(),
            changes: Some(changes.clone()),
            irreversible: Some(vec![marker.clone()]),
            git: Some(git),
            ..Effect::default()
        };
        let mut records = vec![Record::from(effect)];
        records.extend(fetched.ref_updates().into_iter().map(Record::from));
        let mut outcome =
            EffectOutcome::new(StageOutput::records(RecordStream::from_records(records)));
        outcome.changes = changes;
        outcome.notable = fetched.notable;
        outcome.irreversible = vec![marker];
        Ok(outcome)
    }
}

/// Request of the arguments of a call.
fn request(args: &BoundArgs) -> Result<FetchRequest> {
    let given = |name: &str| args.is_given(name) && !args.is_defaulted(name);
    let remote = args::text(args, "remote")?.unwrap_or_else(|| DEFAULT_REMOTE.to_owned());
    let mut request = FetchRequest::new(remote);
    if given("prune") {
        request.prune = args.bool("prune")?;
    }
    if args.bool("tags")? == Some(true) {
        request.tags = Some(TagMode::All);
    }
    request.depth = match args::count(args, "depth")? {
        None => None,
        Some(depth) => Some(u32::try_from(depth).map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("depth {depth} is too large; pass at most {}", u32::MAX),
            )
            .with_problem("invalid_depth")
        })?),
    };
    request.accept_host_key = args::text(args, "accept_host_key")?;
    request.reflog_action = Some(reflog_action(&request, given("remote")));
    Ok(request)
}

/// Changed reference files, for the report.
fn changes(repo: &GitRepo, plan: &FetchPlan) -> Vec<Change> {
    plan.changes
        .iter()
        .filter(|change| change.writes())
        .map(|change| Change {
            path: RecordPath::new(
                ref_file(repo.common_dir(), &change.name)
                    .display()
                    .to_string(),
            ),
            action: match change.status {
                GitRefStatus::New => ChangeAction::Create,
                GitRefStatus::Deleted => ChangeAction::Delete,
                _ => ChangeAction::Modify,
            },
            ..Change::default()
        })
        .collect()
}
