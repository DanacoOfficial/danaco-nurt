// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage implementation of `git.restore`.

use std::path::{Component, Path, PathBuf};

use bstr::BString;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{GitEffect, GitEffectState};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::Path as RecordPath;
use nurt_git::apply::{ApplyContext, ApplyPlan, apply_plan};
use nurt_git::repo::GitRepo;

use super::{RestoreRequest, plan_restore};
use crate::reset::args;
use crate::reset::{effect_record, file_changes};
use crate::stash::state_token;
use crate::switch::stage::{
    add_checkpoint, current_branch, notable_entries, planned_paths, record_objects, text,
};

/// Name of the command in the catalog.
const COMMAND: &str = "git.restore";

/// Stage implementation of `git.restore`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RestoreCommand;

struct Planned {
    root: PathBuf,
    common_dir: PathBuf,
    plan: ApplyPlan,
    objects_written: bool,
    staged: bool,
    checkpoint: Option<String>,
}

impl StageImpl for RestoreCommand {
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

impl EffectStage for RestoreCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let label = args::required_text(args, "repo")?;
        let git = GitRepo::discover(Path::new(&label), ctx)?;
        let root = git.root().to_path_buf();
        if let Some(expected) = args::text(args, "expect_state")? {
            check_state(&root, &expected)?;
        }
        let request = request_of(args, &root)?;
        let apply_ctx = ApplyContext::open(git.gix())?;
        let mut plan = plan_restore(&apply_ctx, &request)?;
        let checkpoint = if args::flag(args, "checkpoint")? {
            Some(add_checkpoint(
                &git,
                &mut plan,
                "before git.restore".to_owned(),
                ctx,
            )?)
        } else {
            None
        };
        let (written, removed) = planned_paths(&plan);
        let mut effect = EffectPlan::new();
        effect.changes = file_changes(&root, &written, &removed);
        effect.notable = notable_entries(&plan.notable);
        effect.bulk = effect.changes.len() > 1;
        Ok(effect.with_state(Planned {
            root,
            common_dir: git.common_dir().to_path_buf(),
            plan,
            objects_written: checkpoint.is_some() || !request.hunks.is_empty(),
            staged: request.staged,
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
            Error::new(ErrorCode::StageFailed, "the plan of git.restore is missing")
        })?;
        let git = GitRepo::open(&planned.root, ctx)?;
        let apply_ctx = ApplyContext::open(git.gix())?;
        if planned.objects_written {
            record_objects(tx, &planned.common_dir)?;
        }
        let report = apply_plan(tx, &apply_ctx, planned.plan)?;
        let written: Vec<String> = report.written.iter().map(text).collect();
        let removed: Vec<String> = report.removed.iter().map(text).collect();
        let state = if written.is_empty() && removed.is_empty() && !planned.staged {
            GitEffectState::NothingToDo
        } else {
            GitEffectState::Completed
        };
        let changes = file_changes(&planned.root, &written, &removed);
        let notable = notable_entries(&report.notable);
        let head = git.gix().head_id().ok().map(|id| id.to_string());
        let git_effect = GitEffect {
            repo: RecordPath::new(planned.root.display().to_string()),
            state,
            head_before: head.clone(),
            head_after: head,
            branch: current_branch(git.gix()),
            checkpoint_before: planned.checkpoint,
            state_token: Some(state_token(&planned.root)?),
            ..GitEffect::default()
        };
        let record = effect_record(COMMAND, git_effect, &changes, &notable);
        let mut result = EffectOutcome::new(StageOutput::records(RecordStream::once(record)));
        result.changes = changes;
        result.notable = notable;
        Ok(result)
    }
}

fn request_of(args: &BoundArgs, root: &Path) -> Result<RestoreRequest> {
    let paths = args::list(args, "paths")?
        .iter()
        .map(|path| relative_path(root, path))
        .collect::<Result<Vec<_>>>()?;
    let staged = args::flag(args, "staged")?;
    Ok(RestoreRequest {
        paths,
        source: args::text(args, "source")?,
        staged,
        // As in Git, `staged` alone leaves the working tree untouched.
        worktree: args.bool("worktree")?.unwrap_or(!staged),
        hunks: args::list(args, "hunks")?,
    })
}

/// Pathspec of `path` relative to the working tree root: absolute paths
/// below the root lose the root prefix, other pathspecs are kept as given.
fn relative_path(root: &Path, path: &str) -> Result<BString> {
    let given = Path::new(path);
    if !given.is_absolute() {
        return Ok(BString::from(path));
    }
    let outside = || {
        Error::new(
            ErrorCode::InvalidValue,
            format!(
                "{path} is outside the repository at {}; give paths inside it",
                root.display()
            ),
        )
        .with_problem("outside_repository")
        .with_found(path.to_owned())
    };
    let relative = given.strip_prefix(root).map_err(|_| outside())?;
    let mut parts: Vec<String> = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::ParentDir if parts.pop().is_none() => return Err(outside()),
            _ => {}
        }
    }
    Ok(if parts.is_empty() {
        BString::from(".")
    } else {
        BString::from(parts.join("/"))
    })
}

fn check_state(root: &Path, expected: &str) -> Result<()> {
    if state_token(root)? == expected {
        return Ok(());
    }
    let quoted = serde_json::Value::String(root.display().to_string());
    Err(Error::new(
        ErrorCode::Stale,
        "the repository changed since the state token was read; read the state again",
    )
    .with_problem("state_changed")
    .with_fix(format!("git.status {quoted}")))
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::{ArgValue, StageData};
    use nurt_contracts::value::{Record, Value};
    use nurt_git::apply::testjournal::{RecordingJournal, tree_state};
    use nurt_git::testutil::{Fixture, RepoBuilder, test_ctx};

    use super::*;

    fn fixture() -> Fixture {
        RepoBuilder::new()
            .file("a.txt", "one\n")
            .file("b.txt", "b\n")
            .commit("first")
            .file("a.txt", "two\n")
            .commit("second")
            .worktree_file("a.txt", "local\n")
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

    fn paths(items: &[&str]) -> Value {
        Value::List(
            items
                .iter()
                .map(|item| Value::Path((*item).to_owned()))
                .collect(),
        )
    }

    fn run(args: &BoundArgs, journal: &RecordingJournal) -> Result<(Vec<Record>, usize)> {
        let ctx = test_ctx();
        let stage = RestoreCommand;
        let effect = stage.as_effect().expect("effect stage");
        let plan = effect.plan(args, StageInput::none(), &ctx)?;
        let mut tx = journal.begin();
        let outcome = effect.apply(args, plan, &mut tx, &ctx)?;
        let changes = outcome.changes.len();
        let records = match outcome.output.data {
            StageData::Records(stream) => stream.collect::<Result<Vec<_>>>()?,
            _ => Vec::new(),
        };
        Box::new(tx).commit()?;
        Ok((records, changes))
    }

    fn read(root: &Path, path: &str) -> Option<String> {
        std::fs::read_to_string(root.join(path)).ok()
    }

    #[test]
    fn stage_restores_worktree_file_and_undo_brings_back_the_local_change() {
        let fixture = fixture();
        let root = fixture.path();
        let before = tree_state(root);
        let journal = RecordingJournal::new();
        let (records, changes) =
            run(&args(root, &[("paths", paths(&["a.txt"]))]), &journal).expect("restore");
        assert_eq!(records.len(), 1);
        assert_eq!(changes, 1);
        assert_eq!(read(root, "a.txt").as_deref(), Some("two\n"));
        journal.undo_last();
        assert_eq!(tree_state(root), before);
    }

    #[test]
    fn stage_accepts_absolute_paths_and_a_source_revision() {
        let fixture = fixture();
        let root = fixture.path();
        let absolute = root.join("a.txt").display().to_string();
        let journal = RecordingJournal::new();
        run(
            &args(
                root,
                &[
                    ("paths", paths(&[absolute.as_str()])),
                    ("source", Value::Text("HEAD~1".into())),
                ],
            ),
            &journal,
        )
        .expect("restore");
        assert_eq!(read(root, "a.txt").as_deref(), Some("one\n"));
    }

    #[test]
    fn stage_refuses_a_stale_state_token_and_accepts_the_current_one() {
        let fixture = fixture();
        let root = fixture.path();
        let journal = RecordingJournal::new();
        let stale = args(
            root,
            &[
                ("paths", paths(&["a.txt"])),
                ("expect_state", Value::Text("0".repeat(64))),
            ],
        );
        let error = run(&stale, &journal).expect_err("stale");
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(read(root, "a.txt").as_deref(), Some("local\n"));
        let token = state_token(root).expect("token");
        let current = args(
            root,
            &[
                ("paths", paths(&["a.txt"])),
                ("expect_state", Value::Text(token)),
            ],
        );
        run(&current, &journal).expect("restore");
        assert_eq!(read(root, "a.txt").as_deref(), Some("two\n"));
    }

    #[test]
    fn stage_checkpoint_keeps_the_discarded_change() {
        let fixture = fixture();
        let root = fixture.path();
        let before = tree_state(root);
        let journal = RecordingJournal::new();
        run(
            &args(
                root,
                &[
                    ("paths", paths(&["a.txt"])),
                    ("checkpoint", Value::Bool(true)),
                ],
            ),
            &journal,
        )
        .expect("restore");
        let reference = read(root, ".git/refs/nurt/checkpoints/cp-1").expect("checkpoint");
        let repo = fixture.open();
        let commit = gix::ObjectId::from_hex(reference.trim().as_bytes()).expect("object id");
        let tree = repo
            .find_commit(commit)
            .expect("commit")
            .tree()
            .expect("tree");
        let entry = tree
            .lookup_entry_by_path("a.txt")
            .expect("lookup")
            .expect("entry");
        let blob = repo.find_blob(entry.object_id()).expect("blob");
        assert_eq!(blob.data.as_slice(), b"local\n");
        journal.undo_last();
        assert_eq!(tree_state(root), before);
    }

    #[test]
    fn stage_restores_selected_hunk() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n")
            .commit("first")
            .worktree_file("a.txt", "one\n2\n3\n4\n5\n6\n7\n8\n9\nten\n")
            .build();
        let root = fixture.path();
        let journal = RecordingJournal::new();
        run(
            &args(
                root,
                &[
                    ("paths", paths(&["a.txt"])),
                    ("hunks", Value::List(vec![Value::Text("a.txt#1".into())])),
                ],
            ),
            &journal,
        )
        .expect("restore");
        assert_eq!(
            read(root, "a.txt").as_deref(),
            Some("1\n2\n3\n4\n5\n6\n7\n8\n9\nten\n")
        );
    }

    #[test]
    fn worktree_is_restored_unless_turned_off() {
        let root = Path::new("/r");
        let defaults = request_of(&args(root, &[("paths", paths(&["a"]))]), root).expect("request");
        assert!(defaults.worktree);
        assert!(!defaults.staged);
        let staged = request_of(
            &args(
                root,
                &[
                    ("paths", paths(&["a"])),
                    ("staged", Value::Bool(true)),
                    ("worktree", Value::Bool(false)),
                ],
            ),
            root,
        )
        .expect("request");
        assert!(staged.staged);
        assert!(!staged.worktree);
    }

    #[test]
    fn staged_alone_leaves_the_working_tree_alone() {
        let root = Path::new("/r");
        let only_index = request_of(
            &args(
                root,
                &[("paths", paths(&["a"])), ("staged", Value::Bool(true))],
            ),
            root,
        )
        .expect("request");
        assert!(only_index.staged);
        assert!(!only_index.worktree);
        let both = request_of(
            &args(
                root,
                &[
                    ("paths", paths(&["a"])),
                    ("staged", Value::Bool(true)),
                    ("worktree", Value::Bool(true)),
                ],
            ),
            root,
        )
        .expect("request");
        assert!(both.staged && both.worktree);
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn absolute_paths_outside_the_root_are_refused() {
        let root = Path::new("/r/repo");
        assert_eq!(
            relative_path(root, "/r/repo/src/a.txt").expect("inside"),
            "src/a.txt"
        );
        assert_eq!(relative_path(root, "/r/repo").expect("root"), ".");
        assert_eq!(relative_path(root, "*.txt").expect("pattern"), "*.txt");
        let error = relative_path(root, "/elsewhere/a.txt").expect_err("outside");
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.data().problem.as_deref(), Some("outside_repository"));
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn parent_components_of_absolute_paths_are_resolved_not_dropped() {
        let root = Path::new("/r/repo");
        assert_eq!(
            relative_path(root, "/r/repo/sub/../a.txt").expect("inside"),
            "a.txt"
        );
        let error = relative_path(root, "/r/repo/../other/a.txt").expect_err("outside");
        assert_eq!(error.data().problem.as_deref(), Some("outside_repository"));
    }

    #[test]
    fn descriptor_names_the_command() {
        let stage = RestoreCommand;
        assert_eq!(stage.descriptor().command, COMMAND);
        assert!(stage.as_effect().is_some());
    }
}
