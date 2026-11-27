// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::branch::command
// Responsibility:   Executor implementation of `git.branch`, and the parts shared with the.
// Dependencies:     bstr, gix_hash, gix_ref, nurt_contracts, sha2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Executor implementation of `git.branch`, and the parts shared with the
//! implementations of `git.tag` and `git.stage`.

use bstr::ByteSlice;
use gix_hash::ObjectId;
use gix_ref::FullName;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{Expect, MutationTx, TargetAction, TargetPath, TargetSpec};
use nurt_contracts::record::{ChangeAction, GitEffect, GitEffectState};
use nurt_contracts::report::{Change, Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Path as RecordPath, Record};
use sha2::Digest;

use super::{
    BranchPlan, CreateOptions, HEADS, branch_name, list, list_trash, plan_create_with, plan_delete,
    plan_purge, plan_rename, quoted_repo, read_error,
};
use crate::checkpoint::args;
use crate::checkpoint::support::committer;
use crate::checkpoint::{CreateRequest, plan_create as plan_checkpoint};
use crate::refs_tx::RefChangeKind;
use crate::repo::GitRepo;

/// Name of the command in the catalog.
const COMMAND: &str = "git.branch";

/// Parameters that select a changing action.
const ACTIONS: [&str; 4] = ["create", "rename", "delete", "purge"];

/// Start point of a new branch when `from` is not given.
const DEFAULT_START: &str = "HEAD";

/// Value of `purge` that selects every preserved copy.
const PURGE_ALL: &str = "all";

/// Implementation of `git.branch` for the executor.
///
/// Without `create`, `rename`, `delete` or `purge` the command runs as a
/// read stage and returns `branch` records; otherwise it runs as an effect
/// stage and returns one `effect` record.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitBranchCommand;

/// Changing request of a `git.branch` call.
enum Request {
    Create {
        name: String,
        from: Option<String>,
        switch: bool,
        checkpoint: bool,
    },
    Rename {
        from: String,
        to: String,
    },
    Delete {
        names: Vec<String>,
        force: bool,
    },
    Purge(Option<Vec<String>>),
}

/// Plan carried from [`EffectStage::plan`] to [`EffectStage::apply`].
struct Planned {
    plan: BranchPlan,
    head_before: Option<ObjectId>,
    checkpoint: Option<String>,
}

impl StageImpl for GitBranchCommand {
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

impl ReadStage for GitBranchCommand {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        if let Some(action) = ACTIONS.iter().find(|name| args.is_given(name)) {
            return Err(args::not_a_read(action));
        }
        let repo = open_repo(args, ctx)?;
        let records: Vec<Record> = if args.bool("trash")?.unwrap_or(false) {
            list_trash(repo.gix())?
                .into_iter()
                .map(Record::from)
                .collect()
        } else {
            list(repo.gix(), args.bool("remote")?.unwrap_or(false))?
                .into_iter()
                .map(Record::from)
                .collect()
        };
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

impl EffectStage for GitBranchCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let request = request(args)?;
        let repo = open_repo(args, ctx)?;
        let gix = repo.gix();
        if let Some(expected) = args::text(args, "expect_state")? {
            check_state(gix, &expected)?;
        }
        let check_references = args.bool("check_references")?.unwrap_or(true);
        let who = committer(gix, ctx);
        let head_before = repo.head_id()?;
        let mut notable = Vec::new();
        let mut checkpoint = None;
        let plan = match &request {
            Request::Create {
                name,
                from,
                switch,
                checkpoint: wanted,
            } => {
                let saved = if *wanted {
                    let request = CreateRequest {
                        label: Some(format!("before switching to {name}")),
                        index: true,
                        ..CreateRequest::default()
                    };
                    Some(plan_checkpoint(repo.root(), &request, ctx)?)
                } else {
                    None
                };
                checkpoint = saved.as_ref().map(|plan| plan.checkpoint().id.clone());
                let options = CreateOptions {
                    switch: *switch,
                    checkpoint: saved.as_ref().map(|plan| plan.checkpoint()),
                };
                let from = match from {
                    Some(from) => from.clone(),
                    None if *switch => DEFAULT_START.to_owned(),
                    None => current_branch(gix)?,
                };
                plan_create_with(gix, name, &from, who, &options)?
            }
            Request::Rename { from, to } => {
                if check_references {
                    notable.extend(references_left(gix, &branch_name(gix, from)?)?);
                }
                plan_rename(gix, from, to, who)?
            }
            Request::Delete { names, force } => {
                if check_references {
                    for name in names {
                        notable.extend(references_left(gix, &branch_name(gix, name)?)?);
                    }
                }
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                plan_delete(gix, &names, *force, ctx.config().git.trash_refs, who)?
            }
            Request::Purge(names) => {
                let names: Option<Vec<&str>> = names
                    .as_ref()
                    .map(|names| names.iter().map(String::as_str).collect());
                plan_purge(gix, names.as_deref(), who)?
            }
        };
        for name in plan.unmerged() {
            let mut note = NotableEntry::new(Notable::BranchDeletedUnmerged);
            note.detail = Some(name.clone());
            notable.push(note);
        }
        if plan.carries_changes() {
            notable.push(NotableEntry::new(Notable::LocalChangesCarried));
        }
        let mut effect = EffectPlan::new();
        effect.changes = plan.changes(repo.root(), repo.git_dir());
        effect.notable = notable;
        effect.bulk = effect.changes.len() > 1;
        Ok(effect.with_state(Planned {
            plan,
            head_before,
            checkpoint,
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
        let notable = std::mem::take(&mut plan.notable);
        let planned = plan
            .take_state::<Planned>()
            .ok_or_else(|| wrong_plan(COMMAND))?;
        let repo = open_repo(args, ctx)?;
        let switched = planned.plan.switches();
        let outcomes = planned.plan.apply(repo.gix(), tx)?;
        let changed = switched
            || outcomes
                .iter()
                .any(|outcome| outcome.kind != RefChangeKind::Unchanged);
        let state = if changed {
            GitEffectState::Completed
        } else {
            GitEffectState::NothingToDo
        };
        let mut record = effect_record(&repo, state, planned.head_before)?;
        record.checkpoint_before = planned.checkpoint;
        record.state_token = Some(state_token(repo.gix())?);
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(
            Record::from(record),
        )));
        outcome.changes = changes;
        outcome.notable = notable;
        Ok(outcome)
    }
}

/// Reads the changing request of the arguments.
fn request(args: &BoundArgs) -> Result<Request> {
    let given: Vec<&str> = ACTIONS
        .iter()
        .copied()
        .filter(|name| args.is_given(name))
        .collect();
    if given.len() > 1 {
        return Err(Error::new(
            ErrorCode::Arg,
            format!(
                "{} exclude each other; pass one of them per call",
                given.join(" and ")
            ),
        )
        .with_problem("param_conflict"));
    }
    let switch = args.bool("switch")?.unwrap_or(false);
    let checkpoint = args.bool("checkpoint")?.unwrap_or(false);
    if !given.contains(&"create") && (switch || checkpoint) {
        return Err(Error::new(
            ErrorCode::Arg,
            "switch and checkpoint apply to a branch created by the same call; pass create",
        )
        .with_problem("param_conflict"));
    }
    if checkpoint && !switch {
        return Err(Error::new(
            ErrorCode::Arg,
            "checkpoint is taken before switching; pass switch: true with it",
        )
        .with_problem("param_conflict"));
    }
    match given.first().copied() {
        Some("create") => Ok(Request::Create {
            name: args::text(args, "create")?.unwrap_or_default(),
            from: if args.is_given("from") {
                args::text(args, "from")?
            } else {
                None
            },
            switch,
            checkpoint,
        }),
        Some("rename") => {
            let from = args::text(args, "rename")?.unwrap_or_default();
            let to = args::text(args, "new_name")?.ok_or_else(|| {
                Error::new(
                    ErrorCode::MissingParam,
                    "rename needs the new name of the branch; pass new_name",
                )
                .with_problem("missing_new_name")
            })?;
            Ok(Request::Rename { from, to })
        }
        Some("delete") => Ok(Request::Delete {
            names: args::texts(args, "delete")?,
            force: args.bool("force")?.unwrap_or(false),
        }),
        Some(_) => {
            let names = args::texts(args, "purge")?;
            let all = names.iter().any(|name| name == PURGE_ALL);
            Ok(Request::Purge((!all).then_some(names)))
        }
        None => Err(args::not_an_effect("list")),
    }
}

/// Name of the start point written to the reflog of a branch created
/// without `from`: the current branch, as `git branch <name>` writes it, or
/// `HEAD` when it is detached.
fn current_branch(repo: &gix::Repository) -> Result<String> {
    Ok(repo.head_name().map_err(read_error)?.map_or_else(
        || DEFAULT_START.to_owned(),
        |name| name.shorten().to_str_lossy().into_owned(),
    ))
}

/// Opens the repository named by the argument `repo`.
///
/// # Errors
///
/// The errors of [`GitRepo::discover`].
pub(crate) fn open_repo(args: &BoundArgs, ctx: &ExecCtx) -> Result<GitRepo> {
    GitRepo::discover(&args::repo(args)?, ctx)
}

/// Error for an effect plan produced by another stage.
pub(crate) fn wrong_plan(command: &str) -> Error {
    Error::new(
        ErrorCode::StageFailed,
        format!("`{command}` received a plan of another stage"),
    )
}

/// `effect` record of a completed operation: `HEAD` and the current branch
/// after it, and `head_before`.
///
/// # Errors
///
/// `E_IO` when `HEAD` cannot be read.
pub(crate) fn effect_record(
    repo: &GitRepo,
    state: GitEffectState,
    head_before: Option<ObjectId>,
) -> Result<GitEffect> {
    let head = repo.gix().head().map_err(read_error)?;
    let branch = head
        .referent_name()
        .map(|name| name.shorten().to_str_lossy().into_owned());
    let detached = head.is_detached();
    let head_after = repo.head_id()?;
    let root = AbsPath::new(repo.root()).map_or_else(
        |_| repo.root().to_string_lossy().into_owned(),
        |path| path.to_language_string(),
    );
    Ok(GitEffect {
        repo: RecordPath::new(root),
        state,
        head_before: head_before.map(|id| id.to_string()),
        head_after: head_after.map(|id| id.to_string()),
        branch,
        detached: Some(detached),
        ..GitEffect::default()
    })
}

/// Planned changes of reference files among journal targets: written
/// references and `packed-refs`, and deleted references. Reflogs and the
/// directories that hold them are not listed.
pub(crate) fn target_changes(targets: &[TargetSpec]) -> Vec<Change> {
    targets
        .iter()
        .filter_map(|target| {
            let TargetPath::Fs(path) = &target.path else {
                return None;
            };
            let action = match (&target.action, &target.expect) {
                (TargetAction::Modify, Expect::Absent) => ChangeAction::Create,
                (TargetAction::Modify, _) => ChangeAction::Modify,
                (TargetAction::Delete, Expect::Hash(_)) => ChangeAction::Delete,
                _ => return None,
            };
            Some(Change::new(path.to_language_string(), action))
        })
        .collect()
}

/// Token of the state `git.branch` depends on: `HEAD`, `packed-refs` and
/// the local branches with their commits.
///
/// # Errors
///
/// `E_IO` when the references cannot be read.
pub(crate) fn state_token(repo: &gix::Repository) -> Result<String> {
    let mut hasher = sha2::Sha256::new();
    for file in [
        repo.git_dir().join("HEAD"),
        repo.common_dir().join("packed-refs"),
    ] {
        hasher.update(std::fs::read(&file).unwrap_or_default());
        hasher.update([0]);
    }
    let platform = repo.references().map_err(read_error)?;
    let mut branches = Vec::new();
    for reference in platform.local_branches().map_err(read_error)? {
        let reference = reference.map_err(read_error)?;
        let target = match reference.target().try_id() {
            Some(id) => id.to_string(),
            None => String::new(),
        };
        branches.push((reference.name().as_bstr().to_owned(), target));
    }
    branches.sort();
    for (name, target) in branches {
        hasher.update(&name);
        hasher.update([0]);
        hasher.update(target.as_bytes());
        hasher.update([0]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn check_state(repo: &gix::Repository, expected: &str) -> Result<()> {
    if state_token(repo)? == expected {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::Stale,
        "the branches changed since the state token was read; read the branches again",
    )
    .with_problem("state_changed")
    .with_next_step(format!("git.branch {}", quoted_repo(repo))))
}

/// Other references that name `old`: symbolic references to it and
/// `branch.<name>.merge` entries of branches that track it locally.
fn references_left(repo: &gix::Repository, old: &FullName) -> Result<Vec<NotableEntry>> {
    let mut names = Vec::new();
    let platform = repo.references().map_err(read_error)?;
    for reference in platform.all().map_err(read_error)? {
        let reference = reference.map_err(read_error)?;
        if let gix_ref::TargetRef::Symbolic(target) = reference.target()
            && target == old.as_ref()
        {
            names.push(reference.name().as_bstr().to_str_lossy().into_owned());
        }
    }
    let own = old
        .as_bstr()
        .strip_prefix(HEADS.as_bytes())
        .unwrap_or(old.as_bstr());
    let config = repo.config_snapshot();
    if let Some(sections) = config.plumbing().sections_by_name("branch") {
        for section in sections {
            let Some(branch) = section.header().subsection_name() else {
                continue;
            };
            let local = section
                .value("remote")
                .is_some_and(|remote| remote.as_bytes() == b".");
            let merges = section
                .value("merge")
                .is_some_and(|merge| merge.as_bytes() == old.as_bstr().as_bytes());
            if local && merges && branch != own {
                names.push(format!("branch.{branch}.merge"));
            }
        }
    }
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let mut note = NotableEntry::new(Notable::ReferencesLeft);
    note.path = Some(old.as_bstr().to_str_lossy().into_owned());
    note.count = Some(names.len() as u64);
    note.detail = Some(names.join(", "));
    Ok(vec![note])
}

#[cfg(test)]
mod tests {
    use nurt_contracts::record::{Branch, GitEffect, GitTrashEntry};
    use nurt_contracts::stage::{ArgValue, StageData};
    use nurt_contracts::value::Value;

    use super::super::test_repo::TestRepo;
    use super::*;
    use crate::apply::testjournal::RecordingJournal;
    use crate::testutil::test_ctx;

    fn args(repo: &TestRepo, pairs: &[(&str, Value)]) -> BoundArgs {
        let root = repo
            .git_dir()
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut args =
            BoundArgs::new(COMMAND).with("repo", ArgValue::Value(Value::Path(root)), false);
        for (name, value) in pairs {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    fn records(output: StageOutput<'_>) -> Vec<Record> {
        match output.data {
            StageData::Records(stream) => stream.map(|record| record.unwrap()).collect(),
            other => panic!("expected records, got {other:?}"),
        }
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    #[test]
    fn list_is_a_read_and_changes_are_effects() {
        let repo = TestRepo::new();
        repo.commit("first");
        let ctx = test_ctx();
        let plain = args(&repo, &[]);
        let listed = GitBranchCommand
            .execute(&plain, StageInput::none(), &ctx)
            .unwrap();
        let mut listed = records(listed);
        assert_eq!(listed.len(), 1);
        let branch = Branch::try_from(listed.remove(0)).unwrap();
        assert!(branch.current);
        let create = args(&repo, &[("create", text("topic"))]);
        let error = GitBranchCommand
            .execute(&create, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::StageFailed);
        let effect = GitBranchCommand
            .plan(&create, StageInput::none(), &ctx)
            .unwrap();
        assert_eq!(effect.changes.len(), 1);
        assert!(
            effect.changes[0]
                .path
                .as_str()
                .ends_with("refs/heads/topic")
        );
    }

    #[test]
    fn trash_listing_returns_trash_entries() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        repo.write("refs/nurt/trash/old-100", &format!("{head}\n"));
        let ctx = test_ctx();
        let bound = args(&repo, &[("trash", Value::Bool(true))]);
        let mut found = records(
            GitBranchCommand
                .execute(&bound, StageInput::none(), &ctx)
                .unwrap(),
        );
        assert_eq!(found.len(), 1);
        let entry = GitTrashEntry::try_from(found.remove(0)).unwrap();
        assert_eq!(entry.name, "old");
        assert_eq!(entry.reference, "refs/nurt/trash/old-100");
    }

    #[test]
    fn create_with_switch_moves_head_and_reports_the_branch() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        let ctx = test_ctx();
        let bound = args(
            &repo,
            &[("create", text("topic")), ("switch", Value::Bool(true))],
        );
        let head_file = repo.read("HEAD");
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        let effect = GitBranchCommand
            .plan(&bound, StageInput::none(), &ctx)
            .unwrap();
        assert!(
            effect
                .changes
                .iter()
                .any(|change| change.path.as_str().ends_with("HEAD"))
        );
        let outcome = GitBranchCommand
            .apply(&bound, effect, &mut tx, &ctx)
            .unwrap();
        let record = GitEffect::try_from(records(outcome.output).remove(0)).unwrap();
        Box::new(tx).commit().unwrap();
        assert_eq!(record.branch.as_deref(), Some("topic"));
        assert_eq!(record.head_after, Some(head.to_string()));
        assert_eq!(repo.read("HEAD"), "ref: refs/heads/topic\n");
        let from = head_file
            .trim()
            .trim_start_matches("ref: refs/heads/")
            .to_owned();
        assert!(
            repo.read("logs/HEAD")
                .ends_with(&format!("\tcheckout: moving from {from} to topic\n"))
        );
        assert!(
            repo.read("logs/refs/heads/topic")
                .ends_with("\tbranch: Created from HEAD\n")
        );
        journal.undo_last();
        assert_eq!(repo.read("HEAD"), head_file);
        assert!(!repo.path("refs/heads/topic").exists());
        let existing = args(
            &repo,
            &[("create", text(&from)), ("switch", Value::Bool(true))],
        );
        let error = GitBranchCommand
            .plan(&existing, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("exists"));
        assert_eq!(error.fix.len(), 2);
    }

    #[test]
    fn default_start_point_names_the_current_branch_in_the_reflog() {
        let repo = TestRepo::new();
        repo.commit("first");
        let current = repo.read("HEAD");
        let current = current.trim().trim_start_matches("ref: refs/heads/");
        let ctx = test_ctx();
        let mut bound = args(&repo, &[("create", text("topic"))]);
        bound.insert("from", ArgValue::Value(text("HEAD")), true);
        repo.apply(|tx| {
            let effect = GitBranchCommand.plan(&bound, StageInput::none(), &ctx)?;
            GitBranchCommand.apply(&bound, effect, tx, &ctx).map(|_| ())
        })
        .unwrap();
        assert!(
            repo.read("logs/refs/heads/topic")
                .ends_with(&format!("\tbranch: Created from {current}\n"))
        );
        let explicit = args(&repo, &[("create", text("other")), ("from", text("HEAD"))]);
        repo.apply(|tx| {
            let effect = GitBranchCommand.plan(&explicit, StageInput::none(), &ctx)?;
            GitBranchCommand
                .apply(&explicit, effect, tx, &ctx)
                .map(|_| ())
        })
        .unwrap();
        assert!(
            repo.read("logs/refs/heads/other")
                .ends_with("\tbranch: Created from HEAD\n")
        );
    }

    #[test]
    fn conflicting_parameters_are_rejected() {
        let repo = TestRepo::new();
        repo.commit("first");
        let ctx = test_ctx();
        for pairs in [
            vec![("create", text("a")), ("delete", text("b"))],
            vec![("switch", Value::Bool(true))],
            vec![("create", text("a")), ("checkpoint", Value::Bool(true))],
        ] {
            let error = GitBranchCommand
                .plan(&args(&repo, &pairs), StageInput::none(), &ctx)
                .unwrap_err();
            assert_eq!(error.problem.as_deref(), Some("param_conflict"));
        }
        let error = GitBranchCommand
            .plan(
                &args(&repo, &[("rename", text("main"))]),
                StageInput::none(),
                &ctx,
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::MissingParam);
    }

    #[test]
    fn stale_state_token_stops_the_change() {
        let repo = TestRepo::new();
        repo.commit("first");
        let ctx = test_ctx();
        let token = state_token(&repo.open()).unwrap();
        let fresh = args(
            &repo,
            &[("create", text("a")), ("expect_state", text(&token))],
        );
        assert!(
            GitBranchCommand
                .plan(&fresh, StageInput::none(), &ctx)
                .is_ok()
        );
        repo.commit("second");
        let error = GitBranchCommand
            .plan(&fresh, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Stale);
    }

    #[test]
    fn references_naming_a_deleted_branch_are_reported() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        repo.write("refs/heads/old", &format!("{head}\n"));
        repo.write("refs/heads/alias", "ref: refs/heads/old\n");
        repo.append_config("[branch \"other\"]\n\tremote = .\n\tmerge = refs/heads/old\n");
        let full = FullName::try_from("refs/heads/old").unwrap();
        let notes = references_left(&repo.open(), &full).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].count, Some(2));
        assert_eq!(
            notes[0].detail.as_deref(),
            Some("refs/heads/alias, branch.other.merge")
        );
    }

    #[test]
    fn changing_requests_are_read_from_the_arguments() {
        let repo = TestRepo::new();
        let delete = args(
            &repo,
            &[
                ("delete", Value::List(vec![text("a"), text("b")])),
                ("force", Value::Bool(true)),
            ],
        );
        match request(&delete).unwrap() {
            Request::Delete { names, force } => {
                assert_eq!(names, ["a", "b"]);
                assert!(force);
            }
            _ => panic!("expected a delete request"),
        }
        let everything = args(&repo, &[("purge", text("all"))]);
        assert!(matches!(
            request(&everything).unwrap(),
            Request::Purge(None)
        ));
        let named = args(&repo, &[("purge", text("old-1"))]);
        match request(&named).unwrap() {
            Request::Purge(Some(names)) => assert_eq!(names, ["old-1"]),
            _ => panic!("expected a purge of named copies"),
        }
        let rename = args(&repo, &[("rename", text("a")), ("new_name", text("b"))]);
        match request(&rename).unwrap() {
            Request::Rename { from, to } => assert_eq!((from.as_str(), to.as_str()), ("a", "b")),
            _ => panic!("expected a rename request"),
        }
        let listing = args(&repo, &[]);
        assert!(request(&listing).is_err());
    }

    #[test]
    fn target_changes_map_journal_actions() {
        let path = AbsPath::new(std::env::temp_dir().join("x")).unwrap();
        let spec = |action, expect| TargetSpec {
            path: TargetPath::Fs(path.clone()),
            action,
            expect,
            size_hint: None,
        };
        let hash = Expect::Hash(nurt_contracts::fs::ContentHash([0; 32]));
        let changes = target_changes(&[
            spec(TargetAction::Modify, Expect::Absent),
            spec(TargetAction::Modify, Expect::Unchecked),
            spec(TargetAction::Delete, hash),
            spec(TargetAction::Delete, Expect::Unchecked),
            spec(TargetAction::Create, Expect::Absent),
            spec(TargetAction::Append, Expect::Unchecked),
        ]);
        let actions: Vec<ChangeAction> = changes.iter().map(|change| change.action).collect();
        assert_eq!(
            actions,
            [
                ChangeAction::Create,
                ChangeAction::Modify,
                ChangeAction::Delete
            ]
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
