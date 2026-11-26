// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Executor implementation of `git.tag`.

use std::sync::Arc;

use bstr::ByteSlice;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::GitEffectState;
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::Record;
use nurt_git_transport::sign::{ProgramSigner, Signer, SigningConfig};

use super::{NewTag, TagPlan, TagSigning, list, plan_create, plan_delete};
use crate::branch::command::{effect_record, open_repo, wrong_plan};
use crate::checkpoint::args;
use crate::checkpoint::support::committer;

/// Name of the command in the catalog.
const COMMAND: &str = "git.tag";

/// Default revision of a new tag.
const DEFAULT_REVISION: &str = "HEAD";

/// Implementation of `git.tag` for the executor.
///
/// Without `name` and `delete` the command runs as a read stage and returns
/// `git_tag` records; with `name` it creates a tag and returns its
/// `git_tag` record, with `delete` it returns one `effect` record.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitTagCommand;

/// Plan carried from [`EffectStage::plan`] to [`EffectStage::apply`].
struct Planned {
    plan: TagPlan,
    head: Option<gix_hash::ObjectId>,
}

impl StageImpl for GitTagCommand {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl ReadStage for GitTagCommand {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        if let Some(action) = ["name", "delete"].iter().find(|name| args.is_given(name)) {
            return Err(args::not_a_read(action));
        }
        let repo = open_repo(args, ctx)?;
        let records = list(repo.gix())?.into_iter().map(Record::from).collect();
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

impl EffectStage for GitTagCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let name = args::text(args, "name")?;
        let delete = args::text(args, "delete")?;
        let repo = open_repo(args, ctx)?;
        let gix = repo.gix();
        let tagger = committer(gix, ctx);
        let plan = match (name, delete) {
            (Some(_), Some(_)) => {
                return Err(Error::new(
                    ErrorCode::Arg,
                    "name and delete exclude each other; pass one of them per call",
                )
                .with_problem("param_conflict"));
            }
            (None, None) => return Err(args::not_an_effect("list")),
            (None, Some(delete)) => plan_delete(gix, &delete, tagger)?,
            (Some(name), None) => {
                let revision =
                    args::text(args, "revision")?.unwrap_or_else(|| DEFAULT_REVISION.to_owned());
                let message = args::text(args, "message")?;
                let identity = format!(
                    "{} <{}>",
                    tagger.name.to_str_lossy(),
                    tagger.email.to_str_lossy()
                );
                let mut new = NewTag {
                    name: &name,
                    revision: &revision,
                    message: message.as_deref(),
                    tagger,
                    sign: args.bool("sign")?,
                    signing: None,
                };
                let config = if new.signs(gix) {
                    let mut config = SigningConfig::resolve(
                        gix.config_snapshot().plumbing(),
                        ctx,
                        Some(&identity),
                    )?;
                    if let Some(workdir) = gix.workdir() {
                        config = config.in_work_dir(workdir);
                    }
                    Some(config)
                } else {
                    None
                };
                let signer: Arc<dyn Signer> = ctx
                    .extensions()
                    .get::<dyn Signer>()
                    .unwrap_or_else(|| Arc::new(ProgramSigner::from_ctx(ctx)));
                new.signing = config.as_ref().map(|config| TagSigning {
                    signer: signer.as_ref(),
                    config,
                });
                if let Some(note) = config
                    .as_ref()
                    .and_then(SigningConfig::ignored_programs_note)
                {
                    ctx.note(note);
                }
                plan_create(gix, new)?
            }
        };
        let mut effect = EffectPlan::new();
        effect.changes = plan.changes();
        Ok(effect.with_state(Planned {
            plan,
            head: repo.head_id()?,
        }))
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let changes = std::mem::take(&mut plan.changes);
        let Planned {
            plan: planned,
            head,
        } = plan
            .take_state::<Planned>()
            .ok_or_else(|| wrong_plan(COMMAND))?;
        let repo = open_repo(args, ctx)?;
        let created = planned.record().cloned();
        planned.apply(repo.gix(), tx)?;
        let record = match created {
            Some(tag) => Record::from(tag),
            None => Record::from(effect_record(&repo, GitEffectState::Completed, head)?),
        };
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(record)));
        outcome.changes = changes;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::record::{GitEffect, GitTag, GitTagKind};
    use nurt_contracts::stage::{ArgValue, StageData};
    use nurt_contracts::value::Value;

    use super::*;
    use crate::branch::test_repo::TestRepo;
    use crate::testutil::test_ctx;

    fn args(repo: &TestRepo, pairs: &[(&str, &str)]) -> BoundArgs {
        let root = repo
            .git_dir()
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut args =
            BoundArgs::new(COMMAND).with("repo", ArgValue::Value(Value::Path(root)), false);
        for (name, value) in pairs {
            args.insert(
                *name,
                ArgValue::Value(Value::Text((*value).to_owned())),
                false,
            );
        }
        args
    }

    fn records(output: StageOutput<'_>) -> Vec<Record> {
        match output.data {
            StageData::Records(stream) => stream.map(|record| record.unwrap()).collect(),
            other => panic!("expected records, got {other:?}"),
        }
    }

    fn run(repo: &TestRepo, bound: &BoundArgs) -> Record {
        let ctx = test_ctx();
        repo.apply(|tx| {
            let effect = GitTagCommand.plan(bound, StageInput::none(), &ctx)?;
            Ok(records(GitTagCommand.apply(bound, effect, tx, &ctx)?.output).remove(0))
        })
        .unwrap()
    }

    #[test]
    fn create_list_and_delete_through_the_stage() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        let created = run(&repo, &args(&repo, &[("name", "v1"), ("message", "one")]));
        let created = GitTag::try_from(created).unwrap();
        assert_eq!(created.kind, GitTagKind::Annotated);
        let ctx = test_ctx();
        let plain = args(&repo, &[]);
        let listed = GitTagCommand
            .execute(&plain, StageInput::none(), &ctx)
            .unwrap();
        let mut listed = records(listed);
        assert_eq!(listed.len(), 1);
        let listed = GitTag::try_from(listed.remove(0)).unwrap();
        assert_eq!(listed.target, head.to_string());
        let deleted = run(&repo, &args(&repo, &[("delete", "v1")]));
        let deleted = GitEffect::try_from(deleted).unwrap();
        assert_eq!(deleted.state, GitEffectState::Completed);
        assert!(!repo.path("refs/tags/v1").exists());
    }

    #[test]
    fn name_and_delete_are_exclusive_and_reads_reject_changes() {
        let repo = TestRepo::new();
        repo.commit("first");
        let ctx = test_ctx();
        let both = args(&repo, &[("name", "a"), ("delete", "b")]);
        let error = GitTagCommand
            .plan(&both, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("param_conflict"));
        let error = GitTagCommand
            .execute(&both, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::StageFailed);
        let error = GitTagCommand
            .plan(&args(&repo, &[]), StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::StageFailed);
    }

    #[test]
    fn sign_without_a_message_is_rejected() {
        let repo = TestRepo::new();
        repo.commit("first");
        let ctx = test_ctx();
        let mut bound = args(&repo, &[("name", "s1")]);
        bound.insert("sign", ArgValue::Value(Value::Bool(true)), false);
        let error = GitTagCommand
            .plan(&bound, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("missing_message"));
    }
}
