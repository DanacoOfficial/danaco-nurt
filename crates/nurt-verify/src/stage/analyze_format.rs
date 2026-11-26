// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `analyze.format` command.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::Finding;
use nurt_contracts::report::{Change, ChangeAction, Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::Record;

use crate::analyze::execute::run_captured;
use crate::analyze::format::{SAMPLE_LINES_MAX, changed_lines};
use crate::analyze::scope::{Scope, resolve};
use crate::analyze::tools::{Tool, candidates, resolve_program};
use crate::stage::analyze::{base_dir, bool_arg, count_arg, list_arg};

/// Default number of changed lines shown per file.
const SAMPLE_LINES_DEFAULT: usize = 12;

/// Stage of `analyze.format`.
pub(crate) struct Format;

/// Files one formatter changes.
struct Batch {
    tool: Tool,
    program: PathBuf,
    config: Option<String>,
    files: Vec<String>,
}

/// State carried from the plan to the application.
struct Planned {
    dir: PathBuf,
    batches: Vec<Batch>,
}

impl StageImpl for Format {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "analyze.format".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

/// Files the formatter listed that are in scope. A formatter that checks the
/// whole project lists files outside the requested paths and outside the
/// project too; none of them is rewritten.
fn in_scope(found: Vec<String>, own: &[String]) -> Vec<String> {
    found
        .into_iter()
        .filter(|file| own.contains(file))
        .collect()
}

/// Files each available formatter would change, found with its check mode.
fn plan_batches(ctx: &ExecCtx, dir: &Path, scope: &Scope) -> Result<Vec<Batch>> {
    let search = ctx.host().env_var("PATH");
    let mut batches = Vec::new();
    for candidate in candidates(dir, &scope.extensions()) {
        let tool = candidate.tool;
        if !tool.formats() {
            continue;
        }
        let Some(program) = resolve_program(tool, dir, search.as_deref()) else {
            continue;
        };
        let own = scope.with_extensions(tool.extensions());
        if own.is_empty() {
            continue;
        }
        let mut command = vec![program.to_string_lossy().into_owned()];
        command.extend(tool.check_args(&own));
        let captured = run_captured(ctx, dir, command, None)?;
        let found = tool
            .parse(&captured.output, dir)
            .into_iter()
            .map(|finding: Finding| finding.path.as_str().to_owned())
            .collect();
        let files = in_scope(found, &own);
        if !files.is_empty() {
            batches.push(Batch {
                tool,
                program,
                config: candidate.config.clone(),
                files,
            });
        }
    }
    Ok(batches)
}

/// Directory for one sample under `base`, created new: a name that another
/// account planted in the shared temporary directory is refused instead of
/// followed.
fn fresh_scratch(base: &Path, serial: usize) -> Option<PathBuf> {
    let scratch = base.join(format!("nurt-format-{}-{serial}", std::process::id()));
    std::fs::create_dir(&scratch).ok().map(|()| scratch)
}

fn sample_of(ctx: &ExecCtx, dir: &Path, batch: &Batch, file: &str, limit: usize) -> Vec<String> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let Ok(before) = std::fs::read_to_string(dir.join(file)) else {
        return Vec::new();
    };
    let Some(arguments) = batch.tool.write_args(&[file.to_owned()]) else {
        return Vec::new();
    };
    let Some(scratch) = fresh_scratch(
        &std::env::temp_dir(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ) else {
        return Vec::new();
    };
    let target = scratch.join(file);
    let prepared = target
        .parent()
        .is_some_and(|parent| std::fs::create_dir_all(parent).is_ok())
        && std::fs::write(&target, &before).is_ok();
    if let Some(config) = &batch.config {
        let _ = std::fs::copy(dir.join(config), scratch.join(config));
    }
    let mut command = vec![batch.program.to_string_lossy().into_owned()];
    command.extend(arguments);
    let after = if prepared && run_captured(ctx, &scratch, command, None).is_ok() {
        std::fs::read_to_string(&target).ok()
    } else {
        None
    };
    let _ = std::fs::remove_dir_all(&scratch);
    after.map_or_else(Vec::new, |after| changed_lines(&before, &after, limit))
}

impl EffectStage for Format {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let dir = base_dir(ctx)?;
        let scope = resolve(
            ctx,
            &dir,
            &list_arg(args, "path"),
            bool_arg(args, "changed_only", false),
        )?;
        let batches = plan_batches(ctx, &dir, &scope)?;
        let limit =
            count_arg(args, "sample_lines", SAMPLE_LINES_DEFAULT).clamp(1, SAMPLE_LINES_MAX);
        let mut plan = EffectPlan::new();
        for batch in &batches {
            for file in &batch.files {
                let mut change = Change::new(file.clone(), ChangeAction::Modify);
                let sample = sample_of(ctx, &dir, batch, file, limit);
                change.added = Some(sample.iter().filter(|l| l.starts_with('+')).count() as u64);
                change.removed = Some(sample.iter().filter(|l| l.starts_with('-')).count() as u64);
                change.after = (!sample.is_empty()).then(|| sample.join("\n"));
                plan.changes.push(change);
            }
        }
        if batches.is_empty() {
            plan.notable
                .push(NotableEntry::new(Notable::NothingMatched));
        }
        Ok(plan.with_state(Planned { dir, batches }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let Some(planned) = plan.take_state::<Planned>() else {
            return Err(Error::new(
                ErrorCode::Internal,
                "the formatting plan is missing",
            ));
        };
        for batch in &planned.batches {
            let Some(arguments) = batch.tool.write_args(&batch.files) else {
                continue;
            };
            let mut command = vec![batch.program.to_string_lossy().into_owned()];
            command.extend(arguments);
            let exact = batch
                .files
                .iter()
                .map(|file| planned.dir.join(file))
                .collect();
            let captured = run_captured(ctx, &planned.dir, command, Some(exact))?;
            if captured.output.exit_code != Some(0) {
                return Err(Error::new(
                    ErrorCode::Spawn,
                    format!("{} failed to format the files", batch.tool.name()),
                )
                .with_fix("analyze.check checks: [format]"));
            }
        }
        let records: Vec<Record> = Vec::new();
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::new(
            records.into_iter().map(Ok),
        )));
        outcome.changes = plan.changes;
        outcome.notable = plan.notable;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::{fresh_scratch, in_scope};

    #[cfg(unix)]
    #[test]
    fn a_planted_scratch_name_is_refused_and_a_free_one_is_created() {
        let base = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let planted = format!("nurt-format-{}-7", std::process::id());
        std::os::unix::fs::symlink(elsewhere.path(), base.path().join(planted)).unwrap();
        assert_eq!(fresh_scratch(base.path(), 7), None);
        assert!(elsewhere.path().read_dir().unwrap().next().is_none());
        let fresh = fresh_scratch(base.path(), 8).unwrap();
        assert!(fresh.is_dir());
        assert_eq!(fresh_scratch(base.path(), 8), None);
    }

    #[test]
    fn files_outside_the_scope_are_not_rewritten() {
        let own = vec!["src/a.rs".to_owned(), "src/b.rs".to_owned()];
        let found = vec![
            "src/b.rs".to_owned(),
            "/work/other/lib.rs".to_owned(),
            "../x.rs".to_owned(),
            "src/c.rs".to_owned(),
        ];
        assert_eq!(in_scope(found, &own), ["src/b.rs"]);
        assert!(in_scope(Vec::new(), &own).is_empty());
        assert!(in_scope(vec!["src/a.rs".to_owned()], &[]).is_empty());
    }
}
