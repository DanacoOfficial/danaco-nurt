// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Executor implementation of `git.stage`.

use std::path::Path;

use bstr::BString;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{ChangeAction, GitEffectState, GitIndexSummary};
use nurt_contracts::report::{Change, Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, Value};

use super::{Action, ChangeKind, StagePlan, StageRequest, plan};
use crate::branch::command::{effect_record, open_repo, wrong_plan};
use crate::checkpoint::args;
use crate::diff::{DiffRequest, run_diff};

/// Name of the command in the catalog.
const COMMAND: &str = "git.stage";

/// Implementation of `git.stage` for the executor.
///
/// `action: list` runs as a read stage and returns `hunk` records with
/// identifiers; `add` and `unstage` run as effect stages and return one
/// `effect` record.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitStageCommand;

/// Plan carried from [`EffectStage::plan`] to [`EffectStage::apply`].
struct Planned {
    plan: StagePlan,
}

impl StageImpl for GitStageCommand {
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

impl ReadStage for GitStageCommand {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let action = args::action(args)?;
        if action != "list" {
            return Err(args::not_a_read(&action));
        }
        let mut request = DiffRequest::new(args::repo(args)?);
        request.staged = args.bool("staged")?.unwrap_or(false);
        request.paths = args::texts(args, "paths")?;
        request.exclude = patterns(args, "exclude")?;
        let output = run_diff(&request, ctx)?;
        for note in output.notes {
            ctx.note(note);
        }
        Ok(StageOutput::records(RecordStream::from_records(
            output.records,
        )))
    }
}

impl EffectStage for GitStageCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let action = match args::action(args)?.as_str() {
            "add" => Action::Add,
            "unstage" => Action::Unstage,
            other => return Err(args::not_an_effect(other)),
        };
        let repo = open_repo(args, ctx)?;
        let root = repo.root().to_path_buf();
        let request = StageRequest {
            paths: args::texts(args, "paths")?
                .iter()
                .map(|text| relative_path(&root, text))
                .collect::<Result<_>>()?,
            hunks: args::texts(args, "hunks")?,
            intent_to_add: args.bool("intent_to_add")?.unwrap_or(false),
            exclude: patterns(args, "exclude")?,
        };
        let planned = plan(&repo, action, &request)?;
        let mut effect = EffectPlan::new();
        if !planned.is_empty() {
            let index = repo.index_path();
            let kind = if index.exists() {
                ChangeAction::Modify
            } else {
                ChangeAction::Create
            };
            let mut change = Change::new(index.to_string_lossy().into_owned(), kind);
            change.added = Some(count(&planned, |kind| kind != ChangeKind::Deleted));
            change.removed = Some(count(&planned, |kind| kind == ChangeKind::Deleted));
            effect.changes.push(change);
        }
        if planned.excluded() > 0 {
            let mut note = NotableEntry::new(Notable::Ignored);
            note.count = Some(planned.excluded());
            note.detail = Some("files left out by exclude".to_owned());
            effect.notable.push(note);
        }
        if !planned.ignored().is_empty() {
            let names: Vec<String> = planned.ignored().iter().map(ToString::to_string).collect();
            let mut note = NotableEntry::new(Notable::Ignored);
            note.count = Some(names.len() as u64);
            note.detail = Some(format!(
                "ignored files named explicitly were staged (git add needs -f): {}",
                names.join(", ")
            ));
            effect.notable.push(note);
        }
        Ok(effect.with_state(Planned { plan: planned }))
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let changes = std::mem::take(&mut plan.changes);
        let notable = std::mem::take(&mut plan.notable);
        let Planned { plan: planned } = plan
            .take_state::<Planned>()
            .ok_or_else(|| wrong_plan(COMMAND))?;
        let repo = open_repo(args, ctx)?;
        let head = repo.head_id()?;
        let summary = GitIndexSummary {
            files: planned.entry_count(),
            added: count(&planned, |kind| kind != ChangeKind::Deleted),
            removed: count(&planned, |kind| kind == ChangeKind::Deleted),
        };
        let staged = planned.apply(&repo, tx)?;
        let state = if staged.is_empty() {
            GitEffectState::NothingToDo
        } else {
            GitEffectState::Completed
        };
        let mut record = effect_record(&repo, state, head)?;
        record.index = Some(summary);
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(
            Record::from(record),
        )));
        outcome.changes = changes;
        outcome.notable = notable;
        Ok(outcome)
    }
}

/// Glob patterns given as one pattern or text, or as a list of them.
fn patterns(args: &BoundArgs, name: &str) -> Result<Vec<String>> {
    let pattern = |value: &Value| match value {
        Value::Glob(text) | Value::Text(text) => Ok(text.clone()),
        _ => Err(Error::new(
            ErrorCode::Type,
            format!("`{COMMAND}` expects list<glob> for `{name}`; pass patterns such as \"*.log\""),
        )),
    };
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items.iter().map(pattern).collect(),
        Some(value) => pattern(value).map(|text| vec![text]),
    }
}

fn count(plan: &StagePlan, select: impl Fn(ChangeKind) -> bool) -> u64 {
    plan.changes()
        .iter()
        .filter(|change| select(change.kind))
        .count() as u64
}

/// Path of `text` relative to the working tree `root`, `/`-separated; an
/// absolute path must lie inside the working tree.
fn relative_path(root: &Path, text: &str) -> Result<BString> {
    let path = Path::new(text);
    if !path.is_absolute() {
        return Ok(BString::from(text));
    }
    let canonical_root = std::fs::canonicalize(root).ok();
    let relative = path
        .strip_prefix(root)
        .ok()
        .or_else(|| {
            canonical_root
                .as_deref()
                .and_then(|canonical| path.strip_prefix(canonical).ok())
        })
        .ok_or_else(|| outside(root, text))?;
    if relative.as_os_str().is_empty() {
        return Ok(BString::from("."));
    }
    let bytes = gix_path::into_bstr(relative).map_err(|_| outside(root, text))?;
    Ok(gix_path::to_unix_separators_on_windows(bytes).into_owned())
}

fn outside(root: &Path, text: &str) -> Error {
    Error::new(
        ErrorCode::Arg,
        format!(
            "{text} is outside the working tree {}; pass a path inside it",
            root.display()
        ),
    )
    .with_problem("invalid_path")
    .with_found(text.to_owned())
}

#[cfg(test)]
mod tests {
    use nurt_contracts::record::{GitEffect, Hunk};
    use nurt_contracts::stage::{ArgValue, StageData};
    use nurt_contracts::value::Value;

    use super::*;
    use crate::testutil::{RecordingJournal, RepoBuilder, test_ctx};

    fn args(root: &Path, pairs: &[(&str, Value)]) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND).with(
            "repo",
            ArgValue::Value(Value::Path(root.to_string_lossy().into_owned())),
            false,
        );
        for (name, value) in pairs {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    fn texts(items: &[&str]) -> Value {
        Value::List(
            items
                .iter()
                .map(|item| Value::Text((*item).to_owned()))
                .collect(),
        )
    }

    fn records(output: StageOutput<'_>) -> Vec<Record> {
        match output.data {
            StageData::Records(stream) => stream.map(|record| record.unwrap()).collect(),
            other => panic!("expected records, got {other:?}"),
        }
    }

    #[test]
    fn list_returns_hunks_with_identifiers() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\nb\n")
            .commit("first")
            .worktree_file("a.txt", "a\nB\n")
            .build();
        let ctx = test_ctx();
        let bound = args(fixture.path(), &[("action", Value::Enum("list".into()))]);
        let output = GitStageCommand
            .execute(&bound, StageInput::none(), &ctx)
            .unwrap();
        let mut found = records(output);
        assert_eq!(found.len(), 1);
        let hunk = Hunk::try_from(found.remove(0)).unwrap();
        assert!(hunk.id.is_some_and(|id| id.starts_with("a.txt#1")));
        let error = GitStageCommand
            .plan(&bound, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::StageFailed);
    }

    #[test]
    fn add_plans_the_index_and_reports_an_effect() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "a\n")
            .commit("first")
            .worktree_file("a.txt", "b\n")
            .worktree_file("skip.log", "x\n")
            .build();
        let ctx = test_ctx();
        let absolute = fixture.path().join("a.txt");
        let bound = args(
            fixture.path(),
            &[
                ("action", Value::Enum("add".into())),
                (
                    "paths",
                    texts(&[absolute.to_string_lossy().as_ref(), "skip.log"]),
                ),
                (
                    "exclude",
                    Value::List(vec![Value::Glob("*.log".to_owned())]),
                ),
            ],
        );
        let effect = GitStageCommand
            .plan(&bound, StageInput::none(), &ctx)
            .unwrap();
        assert_eq!(effect.changes.len(), 1);
        assert_eq!(effect.notable[0].count, Some(1));
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        let outcome = GitStageCommand
            .apply(&bound, effect, tx.as_mut(), &ctx)
            .unwrap();
        tx.commit().unwrap();
        let effect = GitEffect::try_from(records(outcome.output).remove(0)).unwrap();
        assert_eq!(effect.state, GitEffectState::Completed);
        assert_eq!(effect.index.map(|index| index.files), Some(1));
        let repo = open_repo(&bound, &ctx).unwrap();
        assert_eq!(repo.read_index().unwrap().state.entries().len(), 1);
        journal.undo_last();
        let restored = repo.read_index().unwrap();
        let entry = &restored.state.entries()[0];
        assert_ne!(entry.id, effect_blob(&repo, b"b\n"));
    }

    fn effect_blob(repo: &crate::repo::GitRepo, data: &[u8]) -> gix_hash::ObjectId {
        gix_object::compute_hash(repo.gix().object_hash(), gix_object::Kind::Blob, data).unwrap()
    }

    #[test]
    fn absolute_paths_outside_the_working_tree_are_rejected() {
        let root = Path::new("/srv/repo");
        assert_eq!(relative_path(root, "/srv/repo/a/b.txt").unwrap(), "a/b.txt");
        assert_eq!(relative_path(root, "/srv/repo").unwrap(), ".");
        assert_eq!(relative_path(root, "dir/x").unwrap(), "dir/x");
        let error = relative_path(root, "/srv/other/x").unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("invalid_path"));
    }
}
