// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The executor stage of `git.clone`.

use std::path::PathBuf;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{
    ChangeAction, Effect, EffectStatus, GitEffect, GitEffectState, GitRefUpdate,
};
use nurt_contracts::report::{Change, Irreversible, IrreversibleEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Path as RecordPath, Record, Size};

use super::{COMMAND, CloneRequest, check_destination, clone, layout};
use crate::checkpoint::args;

/// Implementation of `git.clone` for the executor.
///
/// The plan checks the address and the destination without contacting the
/// remote. The application creates the directory as one journal target,
/// transfers the repository into it and checks out the branch; a failure
/// rolls the transaction back and removes the directory. The output is one
/// `effect` record followed by one `git_ref_update` record per reference
/// created.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitCloneCommand;

/// State carried from the plan to the application.
struct Planned {
    request: CloneRequest,
}

impl StageImpl for GitCloneCommand {
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

impl EffectStage for GitCloneCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let request = request(args)?;
        check_destination(&request)?;
        layout::stored_url(&request.url)?;
        let mut plan = EffectPlan::new();
        plan.irreversible = vec![IrreversibleEntry::new(Irreversible::RemoteRefUpdated)];
        Ok(plan.with_state(Planned { request }))
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
        let outcome = clone(&planned.request, tx, ctx)?;
        let mut marker = IrreversibleEntry::new(Irreversible::RemoteRefUpdated);
        marker.detail = Some(outcome.url.clone());
        let change = Change {
            path: RecordPath::new(outcome.root.display().to_string()),
            action: ChangeAction::Create,
            ..Change::default()
        };
        let head = outcome.head.map(|id| id.to_string());
        let git = GitEffect {
            repo: RecordPath::new(outcome.root.display().to_string()),
            state: GitEffectState::Completed,
            head_before: None,
            head_after: head,
            branch: outcome.branch.clone(),
            branch_before: None,
            detached: Some(outcome.branch.is_none()),
            ref_updates: Some(outcome.ref_updates.clone()),
            objects_received: Some(outcome.objects_received),
            bytes_received: Some(Size::from_bytes(outcome.bytes_received)),
            ..GitEffect::default()
        };
        let effect = Effect {
            status: EffectStatus::Ok,
            kind: COMMAND.to_owned(),
            targets: Some(1),
            changes: Some(vec![change.clone()]),
            irreversible: Some(vec![marker.clone()]),
            git: Some(git),
            ..Effect::default()
        };
        let mut records = vec![Record::from(effect)];
        records.extend(
            outcome
                .ref_updates
                .into_iter()
                .map(|update: GitRefUpdate| Record::from(update)),
        );
        let mut result =
            EffectOutcome::new(StageOutput::records(RecordStream::from_records(records)));
        result.changes = vec![change];
        result.notable = outcome.notable;
        result.irreversible = vec![marker];
        Ok(result)
    }
}

/// Request of the arguments of a call.
fn request(args: &BoundArgs) -> Result<CloneRequest> {
    let missing = |name: &str| {
        Error::new(
            ErrorCode::MissingParam,
            format!("`{COMMAND}` needs a value for `{name}`"),
        )
    };
    let url = args::text(args, "url")?.ok_or_else(|| missing("url"))?;
    let to = args::text(args, "to")?.ok_or_else(|| missing("to"))?;
    let mut request = CloneRequest::new(url, PathBuf::from(to));
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
    request.branch = args::text(args, "branch")?;
    request.bare = args.bool("bare")?.unwrap_or(false);
    request.accept_host_key = args::text(args, "accept_host_key")?;
    Ok(request)
}
