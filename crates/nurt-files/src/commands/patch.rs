// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `patch`: application of a unified or git diff to the files under a root
//! directory, in one transaction.
//!
//! The plan step parses the diff, reads every file it names below `root`,
//! applies all file sections in memory and turns the result into file
//! operations: a file whose hunks do not match fails the whole patch with
//! `E_PATCH_REJECTED` before anything changes. Later sections see the result
//! of earlier ones; each file is written at most once with its final
//! content. The apply step runs the operations through the transaction,
//! which checks atomically that every file still has the entity tag seen by
//! the plan.
//!
//! Changed files keep their encoding, byte order mark and line endings: a
//! diff with `lf` line endings applies to a file with `crlf` line endings,
//! and added lines take the line ending of the file. New files are written
//! in UTF-8 with the line endings of the diff, and missing directories are
//! created. A rename without a change of content is a rename of the file. As
//! with GNU `patch`, a file that a diff without git headers empties is
//! deleted when the time stamp of its new side is the Unix epoch, which GNU
//! `diff -N` writes for a missing file.
//!
//! A git mode change is applied to a file whose content does not change;
//! for a file whose content changes, or a new executable file, the mode is
//! not applied and the change carries the flag `mode_not_applied`.
//!
//! `validate` checks the new text of every created and changed file once
//! the operations are applied (see [`crate::commands::validate`]). A file
//! created by a rename is compared with the text of the file it is renamed
//! from. With `reject`, a syntax error added to any file fails the stage
//! with `E_ROLLED_BACK`, and the executor rolls the whole patch back.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Location, Result};
use nurt_contracts::fs::{AbsPath, FileKind, Follow, Fs, MetaChange};
use nurt_contracts::journal::{Expect, FileOp, MutationTx};
use nurt_contracts::record::{Change, Effect, EffectStatus, HunkStatus};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, Size};
use nurt_text::codec::encode;
use nurt_text::diff::{Hunk, unified_diff};
use nurt_text::patch::{
    ApplyOptions, FileChange, HunkOutcome, Patch, PatchError, PlannedChange, plan as plan_patch,
};
use serde_json::json;

use super::diff::{hunk_record, join_relative};
use crate::commands::validate::{Validate, check_change, validate_param};
use crate::common::etag::{Observed, check_if_match, observe};
use crate::common::mutation::{apply_changes, planned_change};
use crate::common::notable::{at_path, preserved_text};
use crate::common::params::{EncodingParam, IfMatch, if_match_param};
use crate::common::path::path_arg;
use crate::common::text::{TextFile, read_text, text_change};

/// Name of the command in the catalog.
const COMMAND: &str = "patch";

/// Name of the `diff` parameter.
const DIFF: &str = "diff";

/// Name of the `from` parameter.
const FROM: &str = "from";

/// Name of the `root` parameter.
const ROOT: &str = "root";

/// Name of the `reverse` parameter.
const REVERSE: &str = "reverse";

/// Name of the `fuzz` parameter.
const FUZZ: &str = "fuzz";

/// Name of the `strip` parameter.
const STRIP: &str = "strip";

/// Permission bits of the directories created for new files.
const NEW_DIR_MODE: u32 = 0o755;

/// Permission bits of a git file mode such as `100755`.
const MODE_BITS: u32 = 0o7777;

/// Execute bits of a file mode.
const EXECUTE_BITS: u32 = 0o111;

/// Context lines of the differences reported in planned changes.
const DIFF_CONTEXT: usize = 3;

/// Flag of a change with a hunk placed at another line than its header
/// states.
const FLAG_OFFSET: &str = "hunk_offset";

/// Flag of a change with a hunk placed with ignored context lines.
const FLAG_FUZZ: &str = "hunk_fuzz";

/// Flag of a change whose mode from the diff was not applied.
const FLAG_MODE_NOT_APPLIED: &str = "mode_not_applied";

/// Date of the Unix epoch in time stamps of `---` and `+++` lines.
const EPOCH_DATE: &str = "1970-01-01";

/// Local date of the Unix epoch in time zones west of UTC.
const DAY_BEFORE_EPOCH: &str = "1969-12-31";

const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_HOUR: i64 = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY: i64 = 24 * SECONDS_PER_HOUR;

/// Digits of a numeric time zone such as `+0100`.
const ZONE_DIGITS: usize = 4;

/// Stage of `patch`: a source that applies a diff.
pub(crate) struct ApplyPatch;

impl StageImpl for ApplyPatch {
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

impl EffectStage for ApplyPatch {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        if !input.is_none() {
            return Err(Error::new(
                ErrorCode::Type,
                format!("`{COMMAND}` takes no input records: start the pipeline with `{COMMAND}`"),
            ));
        }
        let settings = Settings::from_args(args, ctx)?;
        let text = patch_text(args, ctx)?;
        let patch = Patch::parse(&text).map_err(|error| patch_error(&error, &settings.root))?;
        let fs = ctx.require_fs()?;
        let originals = read_originals(ctx, fs, &patch, &settings)?;
        let planned = plan_patch(&patch, &settings.options, |path| {
            originals
                .get(path)
                .and_then(|original| original.file.as_ref())
                .map(|file| file.text.clone())
        })
        .map_err(|error| patch_error(&error, &settings.root))?;
        let mut outcomes = Outcomes::collect(planned);
        outcomes.remove_emptied(&epoch_paths(&patch, &text, &settings.options), &originals);
        let built = build(fs, &settings, &originals, &outcomes)?;
        let mut plan = EffectPlan::new();
        plan.notable = built.notable;
        plan.changes = built.steps.iter().map(|step| step.change.clone()).collect();
        plan.bulk = built.steps.len() > 1;
        Ok(plan.with_state(PatchState {
            steps: built.steps,
            checks: built.checks,
            validate: settings.validate,
        }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let state = plan.take_state::<PatchState>().ok_or_else(|| {
            Error::new(
                ErrorCode::Internal,
                format!("the plan of `{COMMAND}` has no state: plan the patch again"),
            )
        })?;
        let (ops, planned): (Vec<FileOp>, Vec<Change>) = state
            .steps
            .into_iter()
            .map(|step| (step.op, step.change))
            .unzip();
        let changes: Vec<Change> = if ops.is_empty() {
            Vec::new()
        } else {
            apply_changes(tx, ops)?
                .into_iter()
                .zip(&planned)
                .map(|(applied, planned)| merged_change(applied, planned))
                .collect()
        };
        for check in &state.checks {
            check_change(
                ctx,
                COMMAND,
                &check.path,
                check.before.as_bytes(),
                check.after.as_bytes(),
                state.validate,
            )?;
        }
        let effect = Effect {
            status: EffectStatus::Ok,
            kind: COMMAND.to_owned(),
            op: tx.op().map(|op| op.0),
            targets: Some(u64::try_from(changes.len()).unwrap_or(u64::MAX)),
            changes: Some(changes.clone()),
            notable: (!plan.notable.is_empty()).then(|| {
                plan.notable
                    .iter()
                    .map(|entry| entry.code.as_str().to_owned())
                    .collect()
            }),
            ..Effect::default()
        };
        let output = StageOutput::records(RecordStream::once(Record::from(effect)));
        Ok(EffectOutcome {
            changes,
            ..EffectOutcome::new(output)
        })
    }
}

/// Stages of this module, for the registry.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(ApplyPatch)]
}

/// `root`, `strip`, `fuzz`, `reverse`, `validate` and `if_match`.
struct Settings {
    root: AbsPath,
    options: ApplyOptions,
    validate: Validate,
    if_match: Option<IfMatch>,
}

impl Settings {
    fn from_args(args: &BoundArgs, ctx: &ExecCtx) -> Result<Self> {
        let root = path_arg(args, ROOT, ctx)?.ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                format!(
                    "`{COMMAND}` needs `{ROOT}`: give the directory the paths in the diff are relative to"
                ),
            )
        })?;
        let kind = ctx.require_fs()?.metadata(&root, Follow::Follow)?.kind;
        if kind != FileKind::Directory {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`{ROOT}` must be a directory, and `{root}` is not one"),
            )
            .with_problem("not_a_directory")
            .with_found(root.to_language_string()));
        }
        Ok(Self {
            options: ApplyOptions {
                strip: count_param(args, STRIP)?,
                fuzz: count_param(args, FUZZ)?,
                reverse: args.bool(REVERSE)?.unwrap_or(false),
            },
            validate: validate_param(args)?,
            if_match: if_match_param(args, ctx)?,
            root,
        })
    }

    /// Fails with `E_STALE` when `if_match` expects another tag of an
    /// existing file the patch changes.
    fn check(&self, original: &Original) -> Result<()> {
        let expected = self
            .if_match
            .as_ref()
            .and_then(|tags| tags.expected(&original.path));
        check_if_match(&original.path, &original.observed, expected)
    }
}

/// Non-negative integer parameter; zero without a value.
fn count_param(args: &BoundArgs, name: &str) -> Result<usize> {
    match args.int(name)? {
        None => Ok(0),
        Some(count) => usize::try_from(count).map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("`{name}` must be zero or more, not {count}"),
            )
            .with_found(count.to_string())
        }),
    }
}

/// Text of the diff from `diff` or from the file `from`.
fn patch_text(args: &BoundArgs, ctx: &ExecCtx) -> Result<String> {
    let inline = args.text(DIFF)?;
    let file = path_arg(args, FROM, ctx)?;
    match (inline, file) {
        (Some(_), Some(_)) => Err(Error::new(
            ErrorCode::ParamConflict,
            format!("give the diff either as `{DIFF}` or with `{FROM}`, not both"),
        )),
        (Some(text), None) => Ok(text.to_owned()),
        (None, Some(path)) => Ok(read_text(ctx, &path, EncodingParam::Auto)?.text),
        (None, None) => Err(Error::new(
            ErrorCode::MissingParam,
            format!(
                "`{COMMAND}` needs the diff: give its text as `{DIFF}` or its file as `{FROM}`"
            ),
        )),
    }
}

/// `E_PATCH_REJECTED`, `E_NOT_FOUND`, `E_EXISTS` or `E_FORMAT` for an error of
/// the patch, with the absolute path of the file it concerns.
fn patch_error(error: &PatchError, root: &AbsPath) -> Error {
    let full = |path: &str| {
        join_relative(root, path).map_or_else(|_| path.to_owned(), |file| file.to_language_string())
    };
    let base = Error::new(error.code(), error.to_string());
    match error {
        PatchError::Rejected { path, rejection } => {
            let file = full(path);
            base.with_problem("hunk_rejected")
                .with_location(Location {
                    path: file.clone(),
                    line: Some(u64::try_from(rejection.line).unwrap_or(u64::MAX)),
                    column: None,
                    end_line: None,
                    end_column: None,
                    byte: None,
                })
                .with_expected(json!(rejection.expected))
                .with_actual(json!(rejection.actual))
                .with_detail("hunk", json!(rejection.hunk))
                .with_detail("path", json!(file))
        }
        PatchError::Leftover { path } => base
            .with_problem("delete_mismatch")
            .with_detail("path", json!(full(path))),
        PatchError::NotFound { path } | PatchError::Exists { path } => {
            base.with_detail("path", json!(full(path)))
        }
        PatchError::Malformed { line, .. } => base
            .with_problem("malformed_patch")
            .with_detail("line", json!(line)),
        PatchError::MissingPath { line } => base
            .with_problem("missing_path")
            .with_detail("line", json!(line)),
        PatchError::Strip { path, strip } => base
            .with_problem("strip_too_large")
            .with_found(path.clone())
            .with_detail("strip", json!(strip)),
        PatchError::UnsafePath { path } => {
            base.with_problem("unsafe_path").with_found(path.clone())
        }
        PatchError::Binary { path } => base.with_problem("binary_patch").with_found(path.clone()),
        PatchError::NoChanges => base.with_problem("no_changes"),
    }
}

/// A file named by the patch, as found before the change.
struct Original {
    path: AbsPath,
    observed: Observed,
    /// Content of an existing regular file.
    file: Option<TextFile>,
}

/// Reads every file the patch names, by stripped relative path.
fn read_originals(
    ctx: &ExecCtx,
    fs: &dyn Fs,
    patch: &Patch,
    settings: &Settings,
) -> Result<BTreeMap<String, Original>> {
    let relative_paths = patch
        .paths(settings.options.strip)
        .map_err(|error| patch_error(&error, &settings.root))?;
    let mut originals = BTreeMap::new();
    for relative in relative_paths {
        ctx.checkpoint()?;
        let path = join_relative(&settings.root, &relative)?;
        let observed = observe(fs, &path)?;
        let file = match observed.meta().map(|meta| meta.kind) {
            None => None,
            Some(FileKind::File) => Some(read_text(ctx, &path, EncodingParam::Auto)?),
            Some(kind) => return Err(not_a_file_error(&path, kind)),
        };
        originals.insert(
            relative,
            Original {
                path,
                observed,
                file,
            },
        );
    }
    Ok(originals)
}

fn not_a_file_error(path: &AbsPath, kind: FileKind) -> Error {
    let message = match kind {
        FileKind::Directory => {
            format!("`{path}` is a directory, and the diff changes it as a file")
        }
        FileKind::Symlink | FileKind::Junction => {
            format!("`{path}` is a link: patch the file it points to")
        }
        _ => format!("`{path}` is not a regular file: patch text files only"),
    };
    Error::new(ErrorCode::InvalidValue, message)
        .with_problem("not_a_file")
        .with_found(path.to_language_string())
        .with_detail("path", serde_json::Value::from(path.to_language_string()))
}

/// Final state of one file after all sections of the patch.
#[derive(Debug, Default)]
struct Outcome {
    /// Content; `None` when the file is deleted or renamed away.
    text: Option<String>,
    /// Mode stated by the patch.
    mode: Option<u32>,
    /// Flags of the placement of the hunks.
    flags: BTreeSet<&'static str>,
}

/// Final states of the files and the renames of the patch.
struct Outcomes {
    files: BTreeMap<String, Outcome>,
    renames: Vec<(String, String)>,
}

impl Outcomes {
    fn collect(changes: Vec<PlannedChange>) -> Self {
        let mut files: BTreeMap<String, Outcome> = BTreeMap::new();
        let mut renames = Vec::new();
        for planned in changes {
            let flags = hunk_flags(&planned.hunks);
            let (path, text, mode) = match planned.change {
                FileChange::Create {
                    path,
                    content,
                    mode,
                }
                | FileChange::Modify {
                    path,
                    content,
                    mode,
                } => (path, Some(content), mode),
                FileChange::Delete { path } => (path, None, None),
                FileChange::Rename {
                    from,
                    to,
                    content,
                    mode,
                } => {
                    let source = files.entry(from.clone()).or_default();
                    source.text = None;
                    source.mode = None;
                    renames.push((from, to.clone()));
                    (to, Some(content), mode)
                }
            };
            let outcome = files.entry(path).or_default();
            if text.is_none() {
                outcome.mode = None;
            } else if mode.is_some() {
                outcome.mode = mode;
            }
            outcome.text = text;
            outcome.flags.extend(flags);
        }
        Self { files, renames }
    }

    /// Deletes the existing files of `epoch` that the patch empties.
    fn remove_emptied(&mut self, epoch: &BTreeSet<String>, originals: &BTreeMap<String, Original>) {
        for (path, outcome) in &mut self.files {
            let existed = originals
                .get(path)
                .is_some_and(|original| original.file.is_some());
            if existed && epoch.contains(path) && outcome.text.as_deref() == Some("") {
                outcome.text = None;
            }
        }
    }
}

fn hunk_flags(hunks: &[HunkOutcome]) -> BTreeSet<&'static str> {
    let mut flags = BTreeSet::new();
    if hunks.iter().any(|hunk| hunk.offset != 0) {
        flags.insert(FLAG_OFFSET);
    }
    if hunks.iter().any(|hunk| hunk.fuzz > 0) {
        flags.insert(FLAG_FUZZ);
    }
    flags
}

/// Stripped paths of the sections without git headers whose resulting side
/// (the new side, or the old side in reverse) has the Unix epoch as its time
/// stamp.
fn epoch_paths(patch: &Patch, text: &str, options: &ApplyOptions) -> BTreeSet<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut paths = BTreeSet::new();
    for file in patch.files.iter().filter(|file| !file.git) {
        // `file.line` is the 1-based line of `---`; `+++` follows it.
        let index = if options.reverse {
            file.line.checked_sub(1)
        } else {
            Some(file.line)
        };
        let Some(label) = index.and_then(|index| lines.get(index)) else {
            continue;
        };
        if !epoch_label(label) {
            continue;
        }
        let single = Patch {
            files: vec![file.clone()],
        };
        if let Ok(found) = single.paths(options.strip) {
            paths.extend(found);
        }
    }
    paths
}

/// Returns `true` when the time stamp of a `---` or `+++` line, written as
/// `YYYY-MM-DD HH:MM:SS[.fraction] [+-]HHMM` after a tab, is the Unix epoch.
fn epoch_label(label: &str) -> bool {
    let label = label.strip_suffix('\r').unwrap_or(label);
    let Some((_, stamp)) = label.split_once('\t') else {
        return false;
    };
    let mut parts = stamp.split_whitespace();
    let (Some(date), Some(time)) = (parts.next(), parts.next()) else {
        return false;
    };
    let day = match date {
        EPOCH_DATE => 0,
        DAY_BEFORE_EPOCH => -1,
        _ => return false,
    };
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    if !fraction.bytes().all(|digit| digit == b'0') {
        return false;
    }
    let mut fields = clock.split(':').map(|field| field.parse::<i64>().ok());
    let (Some(Some(hours)), Some(Some(minutes)), Some(Some(seconds)), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return false;
    };
    let offset = match parts.next().map(zone_offset) {
        None => 0,
        Some(Some(offset)) => offset,
        Some(None) => return false,
    };
    day * SECONDS_PER_DAY + hours * SECONDS_PER_HOUR + minutes * SECONDS_PER_MINUTE + seconds
        - offset
        == 0
}

/// Offset of a numeric time zone such as `+0100`, in seconds.
fn zone_offset(zone: &str) -> Option<i64> {
    let sign = match zone.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits = zone.get(1..)?;
    if digits.len() != ZONE_DIGITS || !digits.bytes().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    let (hours, minutes) = digits.split_at(ZONE_DIGITS / 2);
    let hours: i64 = hours.parse().ok()?;
    let minutes: i64 = minutes.parse().ok()?;
    Some(sign * (hours * SECONDS_PER_HOUR + minutes * SECONDS_PER_MINUTE))
}

/// One file operation with its planned `change` record.
struct Step {
    op: FileOp,
    change: Change,
}

/// Text of a created or changed file before and after the patch, checked
/// as `validate` requires once the operations are applied.
#[derive(Debug)]
struct TextCheck {
    path: AbsPath,
    before: String,
    after: String,
}

/// Private state of the plan.
struct PatchState {
    steps: Vec<Step>,
    checks: Vec<TextCheck>,
    validate: Validate,
}

/// Operations, text checks and notable entries of a plan.
struct Built {
    steps: Vec<Step>,
    checks: Vec<TextCheck>,
    notable: Vec<NotableEntry>,
}

/// Turns the final states into file operations: created directories first,
/// then renames, then the other files in path order.
fn build(
    fs: &dyn Fs,
    settings: &Settings,
    originals: &BTreeMap<String, Original>,
    outcomes: &Outcomes,
) -> Result<Built> {
    let mut dirs = BTreeSet::new();
    let mut steps = Vec::new();
    let mut notable = Vec::new();
    let mut checks = Vec::new();
    let mut check = |path: &AbsPath, before: &str, after: &str| {
        if settings.validate != Validate::Off {
            checks.push(TextCheck {
                path: path.clone(),
                before: before.to_owned(),
                after: after.to_owned(),
            });
        }
    };
    let mut handled = BTreeSet::new();
    for (from, to) in &outcomes.renames {
        if let Some(step) = pure_rename(fs, settings, originals, outcomes, from, to, &mut dirs)? {
            handled.insert(from.as_str());
            handled.insert(to.as_str());
            steps.push(step);
        }
    }
    for (relative, outcome) in &outcomes.files {
        if handled.contains(relative.as_str()) {
            continue;
        }
        let original = originals.get(relative);
        match (
            original.and_then(|original| original.file.as_ref()),
            &outcome.text,
        ) {
            (None, None) => {}
            (None, Some(text)) => {
                let path = match original {
                    Some(original) => original.path.clone(),
                    None => join_relative(&settings.root, relative)?,
                };
                missing_parents(fs, &settings.root, &path, &mut dirs)?;
                check(
                    &path,
                    rename_source(originals, outcomes, relative).unwrap_or_default(),
                    text,
                );
                steps.push(create_step(&path, text, outcome));
            }
            (Some(file), None) => {
                check_original(settings, original)?;
                steps.push(delete_step(file, outcome));
            }
            (Some(file), Some(text)) if *text == file.text => {
                if let Some(mode) = changed_mode(file, outcome.mode) {
                    check_original(settings, original)?;
                    steps.push(mode_step(file, mode, outcome));
                } else {
                    notable.push(at_path(Notable::Unchanged, &file.path));
                }
            }
            (Some(file), Some(text)) => {
                check_original(settings, original)?;
                if text.is_empty() {
                    notable.push(at_path(Notable::FileEmptied, &file.path));
                }
                notable.extend(preserved_text(file));
                check(&file.path, &file.text, text);
                steps.push(modify_step(file, text, outcome)?);
            }
        }
    }
    let mut all: Vec<Step> = dirs
        .into_iter()
        .map(|dir| {
            let op = FileOp::create_dir(dir, NEW_DIR_MODE);
            Step {
                change: planned_change(&op.target),
                op,
            }
        })
        .collect();
    all.extend(steps);
    Ok(Built {
        steps: all,
        checks,
        notable,
    })
}

/// Text of the existing file that the patch renames to `relative`.
fn rename_source<'o>(
    originals: &'o BTreeMap<String, Original>,
    outcomes: &Outcomes,
    relative: &str,
) -> Option<&'o str> {
    outcomes
        .renames
        .iter()
        .rev()
        .find(|(_, to)| to == relative)
        .and_then(|(from, _)| originals.get(from))
        .and_then(|original| original.file.as_ref())
        .map(|file| file.text.as_str())
}

fn check_original(settings: &Settings, original: Option<&Original>) -> Result<()> {
    original.map_or(Ok(()), |original| settings.check(original))
}

/// A rename of a file whose content and mode do not change.
fn pure_rename(
    fs: &dyn Fs,
    settings: &Settings,
    originals: &BTreeMap<String, Original>,
    outcomes: &Outcomes,
    from: &str,
    to: &str,
    dirs: &mut BTreeSet<AbsPath>,
) -> Result<Option<Step>> {
    let (Some(source), Some(target)) = (originals.get(from), originals.get(to)) else {
        return Ok(None);
    };
    let Some(file) = &source.file else {
        return Ok(None);
    };
    let moved_away = outcomes
        .files
        .get(from)
        .is_some_and(|outcome| outcome.text.is_none());
    let Some(arrived) = outcomes.files.get(to) else {
        return Ok(None);
    };
    let unchanged = arrived.text.as_deref() == Some(file.text.as_str())
        && changed_mode(file, arrived.mode).is_none();
    if !moved_away || !unchanged || target.observed.exists() {
        return Ok(None);
    }
    settings.check(source)?;
    missing_parents(fs, &settings.root, &target.path, dirs)?;
    let op = FileOp::rename(file.path.clone(), target.path.clone(), false)
        .expect(Expect::Etag(file.etag()))
        .size_hint(file.size);
    let change = Change {
        flags: flag_list(&arrived.flags),
        ..planned_change(&op.target)
    };
    Ok(Some(Step { op, change }))
}

/// Directories between `root` and `path` that do not exist yet.
fn missing_parents(
    fs: &dyn Fs,
    root: &AbsPath,
    path: &AbsPath,
    dirs: &mut BTreeSet<AbsPath>,
) -> Result<()> {
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir == *root || !dir.starts_with(root) || dirs.contains(&dir) {
            break;
        }
        if observe(fs, &dir)?.exists() {
            break;
        }
        current = dir.parent();
        dirs.insert(dir);
    }
    Ok(())
}

/// Mode bits the patch sets, when they differ from those of the file.
fn changed_mode(file: &TextFile, mode: Option<u32>) -> Option<u32> {
    mode.map(|mode| mode & MODE_BITS)
        .filter(|mode| *mode != file.meta.mode & MODE_BITS)
}

fn create_step(path: &AbsPath, text: &str, outcome: &Outcome) -> Step {
    let op = FileOp::create(path.clone(), text.as_bytes().to_vec());
    let mut flags = outcome.flags.clone();
    if outcome.mode.is_some_and(|mode| mode & EXECUTE_BITS != 0) {
        flags.insert(FLAG_MODE_NOT_APPLIED);
    }
    let hunks = unified_diff("", text, DIFF_CONTEXT, false);
    let change = Change {
        size_after: Some(Size::from_bytes(byte_count(text.len()))),
        ..with_diff(
            planned_change(&op.target),
            path,
            &hunks,
            HunkStatus::Added,
            &flags,
        )
    };
    Step { op, change }
}

fn delete_step(file: &TextFile, outcome: &Outcome) -> Step {
    let op = FileOp::remove(file.path.clone())
        .expect(Expect::Etag(file.etag()))
        .size_hint(file.size);
    let hunks = unified_diff(&file.text, "", DIFF_CONTEXT, false);
    let change = with_diff(
        planned_change(&op.target),
        &file.path,
        &hunks,
        HunkStatus::Deleted,
        &outcome.flags,
    );
    Step { op, change }
}

fn mode_step(file: &TextFile, mode: u32, outcome: &Outcome) -> Step {
    let change = MetaChange {
        mode: Some(mode),
        ..MetaChange::default()
    };
    let op = FileOp::set_meta(file.path.clone(), change).expect(Expect::Etag(file.etag()));
    let change = Change {
        flags: flag_list(&outcome.flags),
        ..planned_change(&op.target)
    };
    Step { op, change }
}

/// Write of the new text of an existing file in its encoding and with its
/// byte order mark; the line endings are those produced by the hunks.
fn modify_step(file: &TextFile, text: &str, outcome: &Outcome) -> Result<Step> {
    let bytes = encode(text, &file.encoding)
        .map_err(|error| error.into_error(Some(&file.path.to_language_string())))?;
    let mut flags = outcome.flags.clone();
    if changed_mode(file, outcome.mode).is_some() {
        flags.insert(FLAG_MODE_NOT_APPLIED);
    }
    let hunks = unified_diff(&file.text, text, DIFF_CONTEXT, false);
    let change = with_diff(
        text_change(file, &bytes),
        &file.path,
        &hunks,
        HunkStatus::Modified,
        &flags,
    );
    let op = FileOp::write(file.path.clone(), bytes)
        .expect(Expect::Etag(file.etag()))
        .size_hint(file.size);
    Ok(Step { op, change })
}

/// `change` with the counts of added and removed lines, the difference and
/// the flags.
fn with_diff(
    change: Change,
    path: &AbsPath,
    hunks: &[Hunk],
    status: HunkStatus,
    flags: &BTreeSet<&str>,
) -> Change {
    let (added, removed) = hunks.iter().fold((0, 0), |(added, removed), hunk| {
        let (more, fewer) = hunk.counts();
        (added + more, removed + fewer)
    });
    Change {
        added: Some(byte_count(added)),
        removed: Some(byte_count(removed)),
        diff: Some(
            hunks
                .iter()
                .enumerate()
                .map(|(index, hunk)| hunk_record(hunk, index + 1, path, None, status, false))
                .collect(),
        ),
        flags: flag_list(flags),
        ..change
    }
}

fn flag_list(flags: &BTreeSet<&str>) -> Option<Vec<String>> {
    (!flags.is_empty()).then(|| flags.iter().map(|flag| (*flag).to_owned()).collect())
}

fn byte_count(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// Applied `change` record with the details of the plan.
fn merged_change(applied: Change, planned: &Change) -> Change {
    Change {
        size_before: applied.size_before.or(planned.size_before),
        size_after: applied.size_after,
        hash_before: applied.hash_before,
        hash_after: applied.hash_after,
        store: applied.store,
        irreversible: applied.irreversible,
        ..planned.clone()
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::journal::{JournalStore, TargetIndex, TargetPath};
    use nurt_contracts::record::ChangeAction;
    use nurt_contracts::stage::{ArgValue, StageData};
    use nurt_contracts::value::{RecordKind, Value};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::journal::JournalCall;
    use nurt_testkit::vfs::test_path;

    use super::*;
    use crate::common::etag::etag_text;

    /// Windows-1250 text with Polish letters and CRLF line endings.
    const CP1250_CRLF: &[u8] = b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\nkoniec\r\n";

    fn abs(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    fn with_files(files: &[(&str, &[u8])]) -> TestContext {
        let test = TestContext::new();
        test.fs().create_dir_all(&abs("/work")).unwrap();
        for (path, content) in files {
            test.fs().write_file(&abs(path), content).unwrap();
        }
        test
    }

    /// Lines of a diff, each followed by a line feed.
    fn diff_text(lines: &[&str]) -> String {
        lines.iter().map(|line| format!("{line}\n")).collect()
    }

    fn args(diff: &str, params: &[(&str, Value)]) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND);
        args.insert(DIFF, ArgValue::Value(Value::Text(diff.to_owned())), false);
        args.insert(
            ROOT,
            ArgValue::Value(Value::Path(abs("/work").to_language_string())),
            false,
        );
        for (name, value) in params {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    fn strip_one() -> (&'static str, Value) {
        (STRIP, Value::Int(1))
    }

    #[derive(Debug)]
    struct Run {
        planned: Vec<Change>,
        notable: Vec<NotableEntry>,
        changes: Vec<Change>,
        effect: Effect,
    }

    fn run(test: &TestContext, args: &BoundArgs) -> Result<Run> {
        let ctx = test.ctx();
        let plan = ApplyPatch.plan(args, StageInput::none(), &ctx)?;
        let planned = plan.changes.clone();
        let notable = plan.notable.clone();
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let outcome = ApplyPatch.apply(args, plan, tx.as_mut(), &ctx)?;
        let StageData::Records(stream) = outcome.output.data else {
            panic!("expected an effect record");
        };
        let records: Vec<Record> = stream.collect::<Result<_>>()?;
        tx.commit()?;
        Ok(Run {
            planned,
            notable,
            changes: outcome.changes,
            effect: Effect::try_from(records[0].clone()).unwrap(),
        })
    }

    fn read(test: &TestContext, path: &str) -> Vec<u8> {
        test.fs().read_file(&abs(path)).unwrap()
    }

    fn declared(test: &TestContext) -> Vec<TargetPath> {
        test.journal()
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                JournalCall::Preserve { targets, .. } => Some(targets),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn applied_ok(test: &TestContext) -> Vec<TargetIndex> {
        test.journal()
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                JournalCall::Apply {
                    index, ok: true, ..
                } => Some(index),
                _ => None,
            })
            .collect()
    }

    fn codes(entries: &[NotableEntry]) -> Vec<Notable> {
        entries.iter().map(|entry| entry.code).collect()
    }

    fn actions(changes: &[Change]) -> Vec<ChangeAction> {
        changes.iter().map(|change| change.action).collect()
    }

    fn modify_b() -> String {
        diff_text(&[
            "--- a/notes.txt",
            "+++ b/notes.txt",
            "@@ -1,3 +1,3 @@",
            " a",
            "-b",
            "+B",
            " c",
        ])
    }

    #[test]
    fn descriptor_registers_a_source_effect_stage() {
        let descriptor = ApplyPatch.descriptor();
        assert_eq!(descriptor.command, "patch");
        assert!(descriptor.source);
        assert!(!descriptor.input.accepts(RecordKind::Entry));
        assert!(ApplyPatch.as_effect().is_some());
        assert!(ApplyPatch.as_read().is_none());
        assert_eq!(stages().len(), 1);
    }

    #[test]
    fn unified_diff_changes_a_file_under_the_root() {
        let test = with_files(&[("/work/notes.txt", b"a\nb\nc\n")]);
        let run = run(&test, &args(&modify_b(), &[strip_one()])).unwrap();
        assert_eq!(read(&test, "/work/notes.txt"), b"a\nB\nc\n");
        assert_eq!(actions(&run.changes), [ChangeAction::Modify]);
        let change = &run.changes[0];
        assert_eq!((change.added, change.removed), (Some(1), Some(1)));
        let diff = change.diff.as_ref().unwrap();
        assert_eq!(diff[0].lines, [" a", "-b", "+B", " c"]);
        assert_eq!(change.size_before, Some(Size::from_bytes(6)));
        assert_eq!(change.store.as_deref(), Some("none"));
        assert_eq!(run.planned[0].diff, change.diff);
        assert_eq!(run.effect.kind, "patch");
        assert_eq!(run.effect.targets, Some(1));
    }

    #[test]
    fn strip_zero_keeps_the_prefixes() {
        let test = with_files(&[("/work/notes.txt", b"a\nb\nc\n")]);
        let error = run(&test, &args(&modify_b(), &[])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        let created = test.fs().exists(&abs("/work/a/notes.txt"));
        assert!(!created);
    }

    #[test]
    fn lf_diff_keeps_crlf_encoding_and_byte_order_mark() {
        let test = with_files(&[
            ("/work/notes.txt", b"\xef\xbb\xbfa\r\nb\r\nc\r\n"),
            ("/work/legacy.txt", CP1250_CRLF),
        ]);
        let legacy = diff_text(&[
            "--- a/legacy.txt",
            "+++ b/legacy.txt",
            "@@ -1,2 +1,2 @@",
            " Zielona \u{142}\u{105}ka i \u{17c}\u{f3}\u{142}ty \u{15b}wierszcz",
            "-koniec",
            "+\u{17c}\u{f3}\u{142}w",
        ]);
        let diff = format!("{}{legacy}", modify_b());
        let run = run(&test, &args(&diff, &[strip_one()])).unwrap();
        assert_eq!(
            read(&test, "/work/notes.txt"),
            b"\xef\xbb\xbfa\r\nB\r\nc\r\n"
        );
        assert_eq!(
            read(&test, "/work/legacy.txt"),
            b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\n\xbf\xf3\xb3w\r\n"
        );
        assert!(codes(&run.notable).contains(&Notable::EncodingPreserved));
        assert!(codes(&run.notable).contains(&Notable::LineEndingPreserved));
        assert_eq!(
            run.changes[0].encoding_after.as_deref(),
            Some("windows-1250")
        );
    }

    #[test]
    fn git_diff_creates_deletes_and_renames_in_one_transaction() {
        let test = with_files(&[
            ("/work/gone.txt", b"one\ntwo\n"),
            ("/work/old.txt", b"moved\n"),
            ("/work/edit.txt", b"keep\nold\n"),
        ]);
        let diff = diff_text(&[
            "diff --git a/src/new/main.rs b/src/new/main.rs",
            "new file mode 100644",
            "index 0000000..1111111",
            "--- /dev/null",
            "+++ b/src/new/main.rs",
            "@@ -0,0 +1,2 @@",
            "+fn main() {}",
            "+",
            "diff --git a/gone.txt b/gone.txt",
            "deleted file mode 100644",
            "--- a/gone.txt",
            "+++ /dev/null",
            "@@ -1,2 +0,0 @@",
            "-one",
            "-two",
            "diff --git a/old.txt b/docs/new.txt",
            "similarity index 100%",
            "rename from old.txt",
            "rename to docs/new.txt",
            "diff --git a/edit.txt b/edited.txt",
            "similarity index 60%",
            "rename from edit.txt",
            "rename to edited.txt",
            "--- a/edit.txt",
            "+++ b/edited.txt",
            "@@ -1,2 +1,2 @@",
            " keep",
            "-old",
            "+new",
        ]);
        let run = run(&test, &args(&diff, &[strip_one()])).unwrap();
        assert_eq!(read(&test, "/work/src/new/main.rs"), b"fn main() {}\n\n");
        assert!(!test.fs().exists(&abs("/work/gone.txt")));
        assert!(!test.fs().exists(&abs("/work/old.txt")));
        assert_eq!(read(&test, "/work/docs/new.txt"), b"moved\n");
        assert!(!test.fs().exists(&abs("/work/edit.txt")));
        assert_eq!(read(&test, "/work/edited.txt"), b"keep\nnew\n");
        assert_eq!(
            actions(&run.changes),
            [
                ChangeAction::Create,
                ChangeAction::Create,
                ChangeAction::Create,
                ChangeAction::Rename,
                ChangeAction::Delete,
                ChangeAction::Create,
                ChangeAction::Delete,
                ChangeAction::Create,
            ]
        );
        assert_eq!(
            run.changes[3].to.as_ref().map(|to| to.as_str().to_owned()),
            Some(abs("/work/docs/new.txt").to_language_string())
        );
        let targets = declared(&test);
        assert_eq!(targets.len(), run.changes.len());
        assert_eq!(applied_ok(&test).len(), targets.len());
        assert!(run.planned.len() == targets.len());
    }

    #[test]
    fn every_mutation_goes_through_the_transaction() {
        let test = with_files(&[("/work/a.txt", b"a\n"), ("/work/b.txt", b"b\n")]);
        let diff = diff_text(&[
            "--- a.txt",
            "+++ a.txt",
            "@@ -1 +1 @@",
            "-a",
            "+A",
            "--- b.txt",
            "+++ b.txt",
            "@@ -1 +1 @@",
            "-b",
            "+B",
        ]);
        let ctx = test.ctx();
        let given = args(&diff, &[]);
        let plan = ApplyPatch.plan(&given, StageInput::none(), &ctx).unwrap();
        assert!(plan.bulk);
        assert!(test.journal().calls().is_empty());
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let outcome = ApplyPatch.apply(&given, plan, tx.as_mut(), &ctx).unwrap();
        tx.commit().unwrap();
        let targets = declared(&test);
        assert_eq!(
            targets,
            [
                TargetPath::Fs(abs("/work/a.txt")),
                TargetPath::Fs(abs("/work/b.txt"))
            ]
        );
        assert_eq!(applied_ok(&test), [TargetIndex(0), TargetIndex(1)]);
        assert_eq!(outcome.changes.len(), targets.len());
    }

    #[test]
    fn mode_change_without_content_change_sets_the_mode() {
        let test = with_files(&[("/work/run.sh", b"echo ok\n")]);
        test.fs().set_mode(&abs("/work/run.sh"), 0o644).unwrap();
        let diff = diff_text(&[
            "diff --git a/run.sh b/run.sh",
            "old mode 100644",
            "new mode 100755",
        ]);
        let run = run(&test, &args(&diff, &[strip_one()])).unwrap();
        assert_eq!(actions(&run.changes), [ChangeAction::Attributes]);
        let meta = test
            .fs()
            .metadata(&abs("/work/run.sh"), Follow::NoFollow)
            .unwrap();
        assert_eq!(meta.mode & MODE_BITS, 0o755);
        assert_eq!(read(&test, "/work/run.sh"), b"echo ok\n");
    }

    #[test]
    fn new_executable_files_report_the_mode_not_applied() {
        let test = with_files(&[]);
        let diff = diff_text(&[
            "diff --git a/run.sh b/run.sh",
            "new file mode 100755",
            "--- /dev/null",
            "+++ b/run.sh",
            "@@ -0,0 +1 @@",
            "+echo ok",
        ]);
        let run = run(&test, &args(&diff, &[strip_one()])).unwrap();
        assert_eq!(
            run.changes[0].flags,
            Some(vec![FLAG_MODE_NOT_APPLIED.to_owned()])
        );
        assert_eq!(read(&test, "/work/run.sh"), b"echo ok\n");
    }

    #[test]
    fn reverse_undoes_the_diff() {
        let test = with_files(&[("/work/notes.txt", b"a\nB\nc\n")]);
        run(
            &test,
            &args(&modify_b(), &[strip_one(), (REVERSE, Value::Bool(true))]),
        )
        .unwrap();
        assert_eq!(read(&test, "/work/notes.txt"), b"a\nb\nc\n");
    }

    #[test]
    fn fuzz_and_offsets_place_hunks_and_are_flagged() {
        let test = with_files(&[
            ("/work/f.txt", b"a\nb\nc\nd\ne\n"),
            ("/work/g.txt", b"0\na\nb\nc\n"),
        ]);
        let fuzzy = diff_text(&[
            "--- f.txt",
            "+++ f.txt",
            "@@ -1,5 +1,5 @@",
            " X",
            " b",
            "-c",
            "+C",
            " d",
            " e",
        ]);
        let error = run(&test, &args(&fuzzy, &[])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::PatchRejected);
        let run_fuzzy = run(&test, &args(&fuzzy, &[(FUZZ, Value::Int(1))])).unwrap();
        assert_eq!(read(&test, "/work/f.txt"), b"a\nb\nC\nd\ne\n");
        assert_eq!(run_fuzzy.changes[0].flags, Some(vec![FLAG_FUZZ.to_owned()]));
        let shifted = diff_text(&[
            "--- g.txt",
            "+++ g.txt",
            "@@ -1,3 +1,3 @@",
            " a",
            "-b",
            "+B",
            " c",
        ]);
        let run_shifted = run(&test, &args(&shifted, &[])).unwrap();
        assert_eq!(read(&test, "/work/g.txt"), b"0\na\nB\nc\n");
        assert_eq!(
            run_shifted.changes[0].flags,
            Some(vec![FLAG_OFFSET.to_owned()])
        );
    }

    #[test]
    fn rejected_hunk_changes_nothing() {
        let test = with_files(&[("/work/ok.txt", b"1\n"), ("/work/notes.txt", b"p\nq\nr\n")]);
        let diff = format!(
            "{}{}",
            diff_text(&["--- a/ok.txt", "+++ b/ok.txt", "@@ -1 +1 @@", "-1", "+one"]),
            modify_b()
        );
        let error = run(&test, &args(&diff, &[strip_one()])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::PatchRejected);
        assert_eq!(error.problem.as_deref(), Some("hunk_rejected"));
        let location = error.location.clone().unwrap();
        assert_eq!(location.path, abs("/work/notes.txt").to_language_string());
        assert_eq!(location.line, Some(1));
        assert_eq!(error.expected, Some(json!(["a", "b", "c"])));
        assert_eq!(error.actual, Some(json!(["p", "q", "r"])));
        assert!(test.journal().calls().is_empty());
        assert_eq!(read(&test, "/work/ok.txt"), b"1\n");
    }

    #[test]
    fn emptied_file_with_an_epoch_time_stamp_is_deleted() {
        let test = with_files(&[
            ("/work/gone.txt", b"one\ntwo\n"),
            ("/work/kept.txt", b"x\n"),
        ]);
        let diff = diff_text(&[
            "--- a/gone.txt\t2026-11-20 10:00:00.000000000 +0100",
            "+++ b/gone.txt\t1970-01-01 01:00:00.000000000 +0100",
            "@@ -1,2 +0,0 @@",
            "-one",
            "-two",
            "--- a/kept.txt\t2026-11-20 10:00:00.000000000 +0100",
            "+++ b/kept.txt\t2026-11-21 10:00:00.000000000 +0100",
            "@@ -1 +0,0 @@",
            "-x",
        ]);
        let run = run(&test, &args(&diff, &[strip_one()])).unwrap();
        assert!(!test.fs().exists(&abs("/work/gone.txt")));
        assert_eq!(read(&test, "/work/kept.txt"), b"");
        assert_eq!(
            actions(&run.changes),
            [ChangeAction::Delete, ChangeAction::Modify]
        );
        assert_eq!(codes(&run.notable), [Notable::FileEmptied]);
    }

    #[test]
    fn epoch_time_stamps_are_recognised_in_any_zone() {
        assert!(epoch_label("+++ b/a\t1970-01-01 00:00:00.000000000 +0000"));
        assert!(epoch_label("+++ b/a\t1970-01-01 01:00:00 +0100"));
        assert!(epoch_label(
            "+++ b/a\t1969-12-31 19:00:00.000000000 -0500\r"
        ));
        assert!(epoch_label("--- a\t1970-01-01 00:00:00"));
        assert!(!epoch_label("+++ b/a\t1970-01-01 00:00:01 +0000"));
        assert!(!epoch_label("+++ b/a\t1970-01-01 00:00:00.5 +0000"));
        assert!(!epoch_label("+++ b/a\t2026-11-22 00:00:00 +0000"));
        assert!(!epoch_label("+++ b/a\t1970-01-01 00:00:00 UTC"));
        assert!(!epoch_label("+++ b/a"));
        assert_eq!(zone_offset("-0130"), Some(-5400));
        assert_eq!(zone_offset("+01"), None);
    }

    #[test]
    fn if_match_guards_the_patched_file() {
        let test = with_files(&[
            ("/work/notes.txt", b"a\nb\nc\n"),
            ("/work/other.txt", b"other\n"),
        ]);
        let other = observe(test.fs().as_ref(), &abs("/work/other.txt"))
            .unwrap()
            .etag()
            .unwrap();
        let wrong = (IF_MATCH_NAME, Value::Text(etag_text(&other)));
        let error = run(&test, &args(&modify_b(), &[strip_one(), wrong])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert!(test.journal().calls().is_empty());
        assert_eq!(read(&test, "/work/notes.txt"), b"a\nb\nc\n");

        let current = observe(test.fs().as_ref(), &abs("/work/notes.txt"))
            .unwrap()
            .etag()
            .unwrap();
        let right = (IF_MATCH_NAME, Value::Text(etag_text(&current)));
        run(&test, &args(&modify_b(), &[strip_one(), right])).unwrap();
        assert_eq!(read(&test, "/work/notes.txt"), b"a\nB\nc\n");
    }

    const IF_MATCH_NAME: &str = crate::common::params::IF_MATCH;

    #[test]
    fn a_file_changed_after_the_plan_fails_with_stale() {
        let test = with_files(&[("/work/notes.txt", b"a\nb\nc\n")]);
        let given = args(&modify_b(), &[strip_one()]);
        let ctx = test.ctx();
        let plan = ApplyPatch.plan(&given, StageInput::none(), &ctx).unwrap();
        test.fs()
            .write_file(&abs("/work/notes.txt"), b"changed\n")
            .unwrap();
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let error = ApplyPatch
            .apply(&given, plan, tx.as_mut(), &ctx)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(read(&test, "/work/notes.txt"), b"changed\n");
        assert!(applied_ok(&test).is_empty());
    }

    #[test]
    fn validate_is_read_with_the_shared_parameter() {
        let test = with_files(&[("/work/notes.txt", b"a\nb\nc\n")]);
        let unknown = args(
            &modify_b(),
            &[strip_one(), (VALIDATE, Value::Enum("strict".to_owned()))],
        );
        let error = ApplyPatch
            .plan(&unknown, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnknownValue);
        let wrong = args(&modify_b(), &[strip_one(), (VALIDATE, Value::Bool(true))]);
        let error = ApplyPatch
            .plan(&wrong, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Type);
        let warn = args(
            &modify_b(),
            &[strip_one(), (VALIDATE, Value::Enum("warn".to_owned()))],
        );
        run(&test, &warn).unwrap();
        assert_eq!(read(&test, "/work/notes.txt"), b"a\nB\nc\n");
    }

    #[test]
    fn created_and_changed_text_is_kept_for_validation() {
        let test = with_files(&[
            ("/work/notes.txt", b"a\nb\nc\n"),
            ("/work/edit.txt", b"keep\nold\n"),
            ("/work/gone.txt", b"x\n"),
        ]);
        let diff = format!(
            "{}{}",
            modify_b(),
            diff_text(&[
                "diff --git a/edit.txt b/edited.txt",
                "similarity index 60%",
                "rename from edit.txt",
                "rename to edited.txt",
                "--- a/edit.txt",
                "+++ b/edited.txt",
                "@@ -1,2 +1,2 @@",
                " keep",
                "-old",
                "+new",
                "diff --git a/new.txt b/new.txt",
                "new file mode 100644",
                "--- /dev/null",
                "+++ b/new.txt",
                "@@ -0,0 +1 @@",
                "+fresh",
                "diff --git a/gone.txt b/gone.txt",
                "deleted file mode 100644",
                "--- a/gone.txt",
                "+++ /dev/null",
                "@@ -1 +0,0 @@",
                "-x",
            ])
        );
        let ctx = test.ctx();
        let mut plan = ApplyPatch
            .plan(&args(&diff, &[strip_one()]), StageInput::none(), &ctx)
            .unwrap();
        let state = plan.take_state::<PatchState>().unwrap();
        assert_eq!(state.validate, Validate::Reject);
        let checks: Vec<(String, &str, &str)> = state
            .checks
            .iter()
            .map(|check| {
                (
                    check.path.to_language_string(),
                    check.before.as_str(),
                    check.after.as_str(),
                )
            })
            .collect();
        assert_eq!(
            checks,
            [
                (
                    abs("/work/edited.txt").to_language_string(),
                    "keep\nold\n",
                    "keep\nnew\n"
                ),
                (abs("/work/new.txt").to_language_string(), "", "fresh\n"),
                (
                    abs("/work/notes.txt").to_language_string(),
                    "a\nb\nc\n",
                    "a\nB\nc\n"
                ),
            ]
        );

        let off = args(
            &diff,
            &[strip_one(), (VALIDATE, Value::Enum("off".to_owned()))],
        );
        let mut plan = ApplyPatch.plan(&off, StageInput::none(), &ctx).unwrap();
        let state = plan.take_state::<PatchState>().unwrap();
        assert_eq!(state.validate, Validate::Off);
        assert!(state.checks.is_empty());
    }

    const VALIDATE: &str = crate::commands::validate::VALIDATE;

    #[test]
    fn diff_is_read_from_a_file() {
        let test = with_files(&[
            ("/work/notes.txt", b"a\nb\nc\n"),
            ("/patches/fix.diff", modify_b().as_bytes()),
        ]);
        let mut given = BoundArgs::new(COMMAND);
        given.insert(
            FROM,
            ArgValue::Value(Value::Path(abs("/patches/fix.diff").to_language_string())),
            false,
        );
        given.insert(
            ROOT,
            ArgValue::Value(Value::Path(abs("/work").to_language_string())),
            false,
        );
        given.insert(STRIP, ArgValue::Value(Value::Int(1)), false);
        run(&test, &given).unwrap();
        assert_eq!(read(&test, "/work/notes.txt"), b"a\nB\nc\n");

        given.insert(DIFF, ArgValue::Value(Value::Text(modify_b())), false);
        let error = ApplyPatch
            .plan(&given, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ParamConflict);
    }

    #[test]
    fn invalid_parameters_are_rejected() {
        let test = with_files(&[("/work/notes.txt", b"a\nb\nc\n")]);
        let ctx = test.ctx();
        let mut no_diff = BoundArgs::new(COMMAND);
        no_diff.insert(
            ROOT,
            ArgValue::Value(Value::Path(abs("/work").to_language_string())),
            false,
        );
        let error = ApplyPatch
            .plan(&no_diff, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);

        let negative = args(&modify_b(), &[(STRIP, Value::Int(-1))]);
        let error = ApplyPatch
            .plan(&negative, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);

        let file_root = args(
            &modify_b(),
            &[(
                ROOT,
                Value::Path(abs("/work/notes.txt").to_language_string()),
            )],
        );
        let error = ApplyPatch
            .plan(&file_root, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.problem.as_deref(), Some("not_a_directory"));

        let malformed = args("--- a\n+++ b\n@@ -1 +1 @@\n-a\n", &[]);
        let error = ApplyPatch
            .plan(&malformed, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format);
    }

    #[test]
    fn unsafe_paths_and_existing_files_are_rejected() {
        let test = with_files(&[("/work/new.txt", b"here\n")]);
        let escaping = diff_text(&[
            "--- a/../outside.txt",
            "+++ b/../outside.txt",
            "@@ -0,0 +1 @@",
            "+x",
        ]);
        let error = run(&test, &args(&escaping, &[strip_one()])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format);
        assert_eq!(error.problem.as_deref(), Some("unsafe_path"));
        let existing = diff_text(&[
            "diff --git a/new.txt b/new.txt",
            "new file mode 100644",
            "--- /dev/null",
            "+++ b/new.txt",
            "@@ -0,0 +1 @@",
            "+x",
        ]);
        let error = run(&test, &args(&existing, &[strip_one()])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert_eq!(read(&test, "/work/new.txt"), b"here\n");
    }

    #[test]
    fn mode_equal_to_the_file_is_unchanged() {
        let test = with_files(&[("/work/run.sh", b"echo ok\n")]);
        test.fs().set_mode(&abs("/work/run.sh"), 0o755).unwrap();
        let diff = diff_text(&[
            "diff --git a/run.sh b/run.sh",
            "old mode 100644",
            "new mode 100755",
        ]);
        let run = run(&test, &args(&diff, &[strip_one()])).unwrap();
        assert!(run.changes.is_empty());
        assert_eq!(codes(&run.notable), [Notable::Unchanged]);
        assert!(declared(&test).is_empty());
    }

    #[test]
    fn strip_larger_than_the_path_fails_with_format_before_any_change() {
        let test = with_files(&[("/work/notes.txt", b"a\nb\nc\n")]);
        let error = run(&test, &args(&modify_b(), &[(STRIP, Value::Int(5))])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format);
        assert_eq!(error.problem.as_deref(), Some("strip_too_large"));
        assert_eq!(read(&test, "/work/notes.txt"), b"a\nb\nc\n");
        assert!(test.journal().calls().is_empty());
    }

    #[test]
    fn apply_without_the_state_of_the_plan_fails() {
        let test = with_files(&[]);
        let given = args(&modify_b(), &[]);
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let error = ApplyPatch
            .apply(&given, EffectPlan::new(), tx.as_mut(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Internal);
    }
}
