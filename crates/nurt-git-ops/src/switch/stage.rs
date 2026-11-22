// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage implementation of `git.switch`, and the parts of the effect report
//! shared with `git.restore`: the checkpoint taken before the change, the
//! identity written to reference logs and the conversion of a plan to the
//! changes and notable effects of the report.

use std::path::{Path, PathBuf};

use bstr::BString;
use gix::ObjectId;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{ExemptWrite, MutationTx};
use nurt_contracts::record::{GitEffect, GitEffectState};
use nurt_contracts::report::{Notable as NotableCode, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::Path as RecordPath;
use nurt_git::apply::{ApplyContext, ApplyPlan, EntryAction, Notable, RefPlan, RefTarget};
use nurt_git::checkpoint::{CreateRequest, plan_create};
use nurt_git::repo::GitRepo;

use super::{SwitchOutcome, SwitchPlan, SwitchRequest, apply_switch, plan_switch};
use crate::reset::args;
use crate::reset::ident::Identity;
use crate::reset::{effect_record, file_changes};

/// Name of the command in the catalog.
const COMMAND: &str = "git.switch";

/// Stage implementation of `git.switch`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SwitchCommand;

struct Planned {
    root: PathBuf,
    common_dir: PathBuf,
    plan: SwitchPlan,
    branch_before: Option<String>,
    checkpoint: Option<String>,
}

impl StageImpl for SwitchCommand {
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

impl EffectStage for SwitchCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let label = args::required_text(args, "repo")?;
        let git = GitRepo::discover(Path::new(&label), ctx)?;
        let request = request_of(args, git.root(), ctx)?;
        let apply_ctx = ApplyContext::open(git.gix())?;
        let mut plan = plan_switch(&apply_ctx, &request)?;
        let checkpoint = if args::flag(args, "checkpoint")? {
            let target = request.branch.as_deref().unwrap_or("HEAD");
            Some(add_checkpoint(
                &git,
                &mut plan.apply,
                format!("before git.switch to {target}"),
                ctx,
            )?)
        } else {
            None
        };
        let root = git.root().to_path_buf();
        let (written, removed) = planned_paths(&plan.apply);
        let mut effect = EffectPlan::new();
        effect.changes = file_changes(&root, &written, &removed);
        effect.notable = notable_entries(&plan.apply.notable);
        effect.bulk = effect.changes.len() > 1;
        Ok(effect.with_state(Planned {
            root,
            common_dir: git.common_dir().to_path_buf(),
            branch_before: current_branch(git.gix()),
            plan,
            checkpoint,
        }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let planned = plan.take_state::<Planned>().ok_or_else(|| {
            Error::new(ErrorCode::StageFailed, "the plan of git.switch is missing")
        })?;
        let git = GitRepo::open(&planned.root, ctx)?;
        let apply_ctx = ApplyContext::open(git.gix())?;
        if planned.checkpoint.is_some() {
            record_objects(tx, &planned.common_dir)?;
        }
        let outcome = apply_switch(tx, &apply_ctx, planned.plan)?;
        let written: Vec<String> = outcome.report.written.iter().map(text).collect();
        let removed: Vec<String> = outcome.report.removed.iter().map(text).collect();
        let changes = file_changes(&planned.root, &written, &removed);
        let notable = notable_entries(&outcome.report.notable);
        let git_effect = switch_effect(
            &planned.root,
            &outcome,
            planned.branch_before,
            planned.checkpoint,
        );
        let record = effect_record(COMMAND, git_effect, &changes, &notable);
        let mut result = EffectOutcome::new(StageOutput::records(RecordStream::once(record)));
        result.changes = changes;
        result.notable = notable;
        Ok(result)
    }
}

fn switch_effect(
    root: &Path,
    outcome: &SwitchOutcome,
    branch_before: Option<String>,
    checkpoint: Option<String>,
) -> GitEffect {
    GitEffect {
        repo: RecordPath::new(root.display().to_string()),
        state: GitEffectState::Completed,
        head_before: outcome.head_before.map(|id| id.to_string()),
        head_after: Some(outcome.head_after.to_string()),
        branch: outcome
            .branch
            .as_deref()
            .map(|name| super::short_branch(name).to_owned()),
        branch_before,
        detached: Some(outcome.branch.is_none()),
        checkpoint_before: checkpoint,
        ..GitEffect::default()
    }
}

fn request_of(args: &BoundArgs, root: &Path, ctx: &ExecCtx) -> Result<SwitchRequest> {
    let create = args::flag(args, "create")?;
    let start_point = args::text(args, "start_point")?;
    if start_point.is_some() && !create {
        return Err(Error::new(
            ErrorCode::Arg,
            "start_point applies to a created branch; add create: true, or give the revision as branch with detach: true",
        )
        .with_problem("start_point_without_create"));
    }
    Ok(SwitchRequest {
        branch: args::text(args, "branch")?,
        create,
        start_point,
        detach: args::flag(args, "detach")?,
        discard_changes: args::flag(args, "discard_changes")?,
        guess: args::flag(args, "guess")?,
        committer: committer(root, ctx)?,
    })
}

/// Committer of reference log lines: `GIT_COMMITTER_*` of the environment of
/// the context, then `user.name` and `user.email`, then the account and host
/// names, at the time of the clock of the context.
pub(crate) fn committer(root: &Path, ctx: &ExecCtx) -> Result<gix::actor::Signature> {
    let repo = GitRepo::open(root, ctx)?;
    Ok(Identity::resolve(&repo).committer)
}

/// Short name of the branch `HEAD` names; `None` when detached or unborn
/// without a name.
pub(crate) fn current_branch(repo: &gix::Repository) -> Option<String> {
    repo.head_name()
        .ok()
        .flatten()
        .map(|name| name.shorten().to_string())
}

/// Records a checkpoint of the working tree and the index in new objects and
/// adds its reference to `plan`; returns the identifier of the checkpoint.
///
/// The checkpoint reference is written by the same transaction as the rest
/// of the plan, so `undo` removes it together with the other changes, while
/// the objects stay in the object database.
pub(crate) fn add_checkpoint(
    git: &GitRepo,
    plan: &mut ApplyPlan,
    label: String,
    ctx: &ExecCtx,
) -> Result<String> {
    let created = plan_create(
        git.root(),
        &CreateRequest {
            label: Some(label),
            index: true,
            exclude: Vec::new(),
        },
        ctx,
    )?;
    let record = created.checkpoint();
    let commit = ObjectId::from_hex(record.commit.as_bytes()).map_err(|err| {
        Error::new(
            ErrorCode::Internal,
            format!(
                "the checkpoint commit {} is not an object id: {err}",
                record.commit
            ),
        )
    })?;
    plan.refs.push(RefPlan::new(
        git.gix(),
        &record.reference,
        RefTarget::Object(commit),
        None,
    )?);
    Ok(record.id.clone())
}

/// Marks the objects written while planning as exempt writes of the object
/// database of `common_dir`.
pub(crate) fn record_objects(tx: &mut dyn MutationTx, common_dir: &Path) -> Result<()> {
    let repo = AbsPath::new(common_dir).map_err(|_| {
        Error::new(
            ErrorCode::Io,
            format!(
                "the repository directory {} is not an absolute path; open the repository by its absolute path",
                common_dir.display()
            ),
        )
    })?;
    tx.record_exempt(ExemptWrite::ObjectStore { repo });
    Ok(())
}

/// Paths the plan writes and removes, relative to the working tree root.
pub(crate) fn planned_paths(plan: &ApplyPlan) -> (Vec<String>, Vec<String>) {
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

/// Notable effects of a plan or a report in the form of the report; kinds
/// without a code in the report are left out.
pub(crate) fn notable_entries(notable: &[Notable]) -> Vec<NotableEntry> {
    notable
        .iter()
        .filter_map(|item| {
            let code = item.kind.parse::<NotableCode>().ok()?;
            let mut entry = NotableEntry::new(code);
            entry.path = item.path.as_ref().map(text);
            Some(entry)
        })
        .collect()
}

pub(crate) fn text(path: &BString) -> String {
    String::from_utf8_lossy(path).into_owned()
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::{ArgValue, StageData};
    use nurt_contracts::value::{Record, Value};
    use nurt_git::apply::testjournal::{RecordingJournal, tree_state};
    use nurt_git::testutil::{Fixture, RepoBuilder, test_ctx};

    use super::*;

    struct Run {
        records: Vec<Record>,
        changes: usize,
        notable: Vec<NotableEntry>,
    }

    fn fixture() -> Fixture {
        RepoBuilder::new()
            .file("a.txt", "one\n")
            .commit("first")
            .reference("refs/heads/side", "HEAD")
            .file("a.txt", "two\n")
            .file("b.txt", "b\n")
            .commit("second")
            .build()
    }

    fn args(root: &Path, pairs: &[(&str, Value)]) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND).with(
            "repo",
            ArgValue::Value(Value::Path(root.display().to_string())),
            false,
        );
        for (name, value) in pairs {
            args = args.with(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    fn run(args: &BoundArgs, journal: &RecordingJournal) -> Result<Run> {
        let ctx = test_ctx();
        let stage = SwitchCommand;
        let effect = stage.as_effect().expect("effect stage");
        let plan = effect.plan(args, StageInput::none(), &ctx)?;
        let mut tx = journal.begin();
        let outcome = effect.apply(args, plan, &mut tx, &ctx)?;
        let changes = outcome.changes.len();
        let notable = outcome.notable.clone();
        let records = match outcome.output.data {
            StageData::Records(stream) => stream.collect::<Result<Vec<_>>>()?,
            _ => Vec::new(),
        };
        Box::new(tx).commit()?;
        Ok(Run {
            records,
            changes,
            notable,
        })
    }

    fn read(root: &Path, path: &str) -> Option<String> {
        std::fs::read_to_string(root.join(path)).ok()
    }

    #[test]
    fn stage_switches_branch_and_undo_restores_the_state() {
        let fixture = fixture();
        let root = fixture.path();
        let before = tree_state(root);
        let journal = RecordingJournal::new();
        let done = run(
            &args(root, &[("branch", Value::Text("side".into()))]),
            &journal,
        )
        .expect("switch");
        assert_eq!(done.records.len(), 1);
        assert_eq!(done.changes, 2);
        assert_eq!(
            read(root, ".git/HEAD").as_deref(),
            Some("ref: refs/heads/side\n")
        );
        assert_eq!(read(root, "a.txt").as_deref(), Some("one\n"));
        assert_eq!(read(root, "b.txt"), None);
        journal.undo_last();
        assert_eq!(tree_state(root), before);
    }

    #[test]
    fn stage_checkpoint_keeps_untracked_file_in_a_reference_undone_with_the_switch() {
        let fixture = fixture();
        let root = fixture.path();
        std::fs::write(root.join("notes.txt"), "draft\n").expect("write");
        let before = tree_state(root);
        let journal = RecordingJournal::new();
        run(
            &args(
                root,
                &[
                    ("branch", Value::Text("side".into())),
                    ("checkpoint", Value::Bool(true)),
                ],
            ),
            &journal,
        )
        .expect("switch");
        let reference = read(root, ".git/refs/nurt/checkpoints/cp-1").expect("checkpoint");
        let repo = fixture.open();
        let commit = ObjectId::from_hex(reference.trim().as_bytes()).expect("object id");
        let tree = repo
            .find_commit(commit)
            .expect("commit")
            .tree()
            .expect("tree");
        assert!(
            tree.lookup_entry_by_path("notes.txt")
                .expect("lookup")
                .is_some()
        );
        assert_eq!(read(root, "notes.txt").as_deref(), Some("draft\n"));
        journal.undo_last();
        assert_eq!(tree_state(root), before);
    }

    #[test]
    fn stage_creates_branch_at_start_point_and_reports_detached_head() {
        let fixture = fixture();
        let root = fixture.path();
        let journal = RecordingJournal::new();
        run(
            &args(
                root,
                &[
                    ("branch", Value::Text("topic".into())),
                    ("create", Value::Bool(true)),
                    ("start_point", Value::Text("side".into())),
                ],
            ),
            &journal,
        )
        .expect("create");
        assert_eq!(
            read(root, ".git/refs/heads/topic"),
            read(root, ".git/refs/heads/side")
        );
        let detached = run(
            &args(
                root,
                &[
                    ("branch", Value::Text("main".into())),
                    ("detach", Value::Bool(true)),
                ],
            ),
            &journal,
        )
        .expect("detach");
        assert!(
            detached
                .notable
                .iter()
                .any(|entry| entry.code == NotableCode::DetachedHead)
        );
        assert_eq!(read(root, ".git/HEAD"), read(root, ".git/refs/heads/main"));
    }

    #[test]
    fn stage_rejects_start_point_without_create() {
        let fixture = fixture();
        let root = fixture.path();
        let ctx = test_ctx();
        let bound = args(
            root,
            &[
                ("branch", Value::Text("side".into())),
                ("start_point", Value::Text("main".into())),
            ],
        );
        let error = SwitchCommand
            .plan(&bound, StageInput::none(), &ctx)
            .expect_err("start point without create");
        assert_eq!(error.code(), ErrorCode::Arg);
        assert_eq!(
            error.data().problem.as_deref(),
            Some("start_point_without_create")
        );
    }

    #[test]
    fn stage_refuses_local_change_before_anything_is_written() {
        let fixture = fixture();
        let root = fixture.path();
        std::fs::write(root.join("a.txt"), "local\n").expect("write");
        let before = tree_state(root);
        let journal = RecordingJournal::new();
        let error = run(
            &args(root, &[("branch", Value::Text("side".into()))]),
            &journal,
        )
        .err()
        .expect("dirty");
        assert_eq!(error.code(), ErrorCode::GitDirty);
        assert_eq!(tree_state(root), before);
    }

    #[test]
    fn notable_kinds_become_report_entries() {
        let entries = notable_entries(&[
            Notable::at("filter_not_applied", "a.txt"),
            Notable {
                kind: "no_such_code".to_owned(),
                path: None,
            },
        ]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].code, NotableCode::FilterNotApplied);
        assert_eq!(entries[0].path.as_deref(), Some("a.txt"));
    }

    #[test]
    fn descriptor_names_the_command() {
        let stage = SwitchCommand;
        assert_eq!(stage.descriptor().command, COMMAND);
        assert!(stage.as_effect().is_some());
        assert!(stage.as_read().is_none());
    }
}
