// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Commands that change the cells of a notebook: `notebook.insert`,
//! `notebook.replace`, `notebook.move`, `notebook.copy`, `notebook.remove`,
//! `notebook.clear_outputs`, `notebook.normalize` and `notebook.ops`.
//!
//! The plan reads the notebook, applies the change to the parsed document
//! and keeps the new content with its difference. The file is the only
//! target: the apply step replaces its content in one write, and the journal
//! checks that the file still has the entity tag read in the plan. A change
//! that leaves the notebook as it was has no target and writes nothing.
//!
//! The content is kept in the plan because new cell identifiers are random:
//! the preview and the write show the same identifiers.

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{Expect, FileOp, MutationTx};
use nurt_contracts::record::{CellType, Change, Hunk};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds,
    StageDescriptor, StageImpl, StageInput, StaticInfo,
};
use nurt_contracts::value::{Size, Value};
use nurt_notebook::{
    CellRef, Change as NotebookChange, ChangeOutcome, Notebook, NotebookError, Position, Replace,
    Selection, literal_pattern,
};
use nurt_text::diff::unified_diff;
use serde_json::json;

use super::{
    CELL_TYPE, FIND, FindPattern, ID, INDEX, LoadedNotebook, PATH, cell_type_param, conflict,
    count, effect_outcome, find_param, ids_param, load, missing_param, missing_state,
    new_cell_value, notebook_error, notebook_path, range_param, static_info, type_error,
};
use crate::common::etag::{Observed, check_if_match};
use crate::common::mutation::{apply_changes, planned_change};
use crate::common::params::if_match_param;
use crate::common::path::path_field;

const AFTER_ID: &str = "after_id";
const SOURCE: &str = "source";
const WITH: &str = "with";
const WHOLE_WORD: &str = "whole_word";
const RESET_OUTPUTS: &str = "reset_outputs";
const FROM: &str = "from";
const TO: &str = "to";
const CHANGES: &str = "changes";

const CELLS_INSERTED: &str = "cells_inserted";
const CELLS_CHANGED: &str = "cells_changed";
const CELLS_MOVED: &str = "cells_moved";
const CELLS_COPIED: &str = "cells_copied";
const CELLS_REMOVED: &str = "cells_removed";
const CELLS_CLEARED: &str = "cells_cleared";
const REPLACEMENTS: &str = "replacements";
const OPERATIONS: &str = "operations";

/// Lines of context around each hunk of the difference of a change.
const DIFF_CONTEXT: usize = 3;

/// Member of a map of `notebook.ops` that holds the position of the change.
const CHANGE_DETAIL: &str = "change";

/// Command of an editing stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Insert,
    Replace,
    Move,
    Copy,
    Remove,
    ClearOutputs,
    Normalize,
    Ops,
}

impl EditKind {
    const ALL: [Self; 8] = [
        Self::Insert,
        Self::Replace,
        Self::Move,
        Self::Copy,
        Self::Remove,
        Self::ClearOutputs,
        Self::Normalize,
        Self::Ops,
    ];

    fn command(self) -> &'static str {
        match self {
            Self::Insert => "notebook.insert",
            Self::Replace => "notebook.replace",
            Self::Move => "notebook.move",
            Self::Copy => "notebook.copy",
            Self::Remove => "notebook.remove",
            Self::ClearOutputs => "notebook.clear_outputs",
            Self::Normalize => "notebook.normalize",
            Self::Ops => "notebook.ops",
        }
    }
}

/// Implementation of one editing command.
pub(crate) struct NotebookEdit {
    kind: EditKind,
}

/// Stages of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    EditKind::ALL
        .into_iter()
        .map(|kind| Arc::new(NotebookEdit { kind }) as Arc<dyn StageImpl>)
        .collect()
}

impl StageImpl for NotebookEdit {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.kind.command().to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        EditParams::read(self.kind, args)?;
        Ok(static_info(args, &[PATH]))
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for NotebookEdit {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let params = EditParams::read(self.kind, args)?;
        let path = notebook_path(args, ctx)?;
        let mut loaded = load(ctx, &path)?;
        let expected = if_match_param(args, ctx)?;
        check_if_match(
            &path,
            &Observed::Present(loaded.meta.clone(), Some(loaded.hash())),
            expected.as_ref().and_then(|tags| tags.expected(&path)),
        )?;
        let report = params.apply(&mut loaded.notebook, &path)?;
        let write = loaded
            .notebook
            .is_modified()
            .then(|| PlannedWrite::new(&loaded, report.replacements));
        let mut plan = EffectPlan::new();
        if let Some(write) = &write {
            plan.changes
                .push(write.decorate(planned_change(&write.op.target)));
        }
        Ok(plan.with_state(Planned { write, report }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let command = self.kind.command();
        let planned: Planned = plan.take_state().ok_or_else(|| missing_state(command))?;
        let changes = match planned.write {
            Some(write) => write.apply(tx)?,
            None => Vec::new(),
        };
        Ok(effect_outcome(
            command,
            &*tx,
            changes,
            &planned.report.counters,
            planned.report.after,
        ))
    }
}

/// State of a plan: the write, when the notebook changed, and what the
/// edit did.
struct Planned {
    write: Option<PlannedWrite>,
    report: EditReport,
}

/// New content of the notebook with its difference.
struct PlannedWrite {
    op: FileOp,
    size_after: u64,
    diff: Vec<Hunk>,
    added: u64,
    removed: u64,
    replacements: Option<u64>,
}

impl PlannedWrite {
    fn new(loaded: &LoadedNotebook, replacements: Option<u64>) -> Self {
        let new_text = loaded.notebook.render();
        let old_text = String::from_utf8_lossy(&loaded.bytes);
        let hunks = unified_diff(&old_text, &new_text, DIFF_CONTEXT, false);
        let mut added = 0;
        let mut removed = 0;
        let diff = hunks
            .iter()
            .map(|hunk| {
                let lines = hunk.prefixed_lines();
                added += count(lines.iter().filter(|line| line.starts_with('+')).count());
                removed += count(lines.iter().filter(|line| line.starts_with('-')).count());
                Hunk {
                    path: path_field(&loaded.path),
                    old_start: count(hunk.old_start),
                    old_len: count(hunk.old_len),
                    new_start: count(hunk.new_start),
                    new_len: count(hunk.new_len),
                    lines,
                    header: hunk.header.clone(),
                    ..Hunk::default()
                }
            })
            .collect();
        let bytes = new_text.into_bytes();
        let size_after = count(bytes.len());
        let op = FileOp::write(loaded.path.clone(), bytes)
            .expect(Expect::Etag(loaded.etag()))
            .size_hint(loaded.size());
        Self {
            op,
            size_after,
            diff,
            added,
            removed,
            replacements,
        }
    }

    /// The change with the facts known from the plan: the size after, the
    /// difference and the number of replacements.
    fn decorate(&self, mut change: Change) -> Change {
        change
            .size_after
            .get_or_insert(Size::from_bytes(self.size_after));
        change.added = Some(self.added);
        change.removed = Some(self.removed);
        change.replacements = self.replacements;
        change.diff = Some(self.diff.clone());
        change
    }

    fn apply(self, tx: &mut dyn MutationTx) -> Result<Vec<Change>> {
        let changes = apply_changes(tx, vec![self.op.clone()])?;
        Ok(changes
            .into_iter()
            .map(|change| self.decorate(change))
            .collect())
    }
}

/// What an edit did: counters of the `effect` record, the number of
/// replacements and the new cells.
#[derive(Debug, Default)]
struct EditReport {
    counters: Vec<(&'static str, u64)>,
    replacements: Option<u64>,
    after: Option<Value>,
}

impl EditReport {
    fn counted(name: &'static str, value: usize) -> Self {
        Self {
            counters: vec![(name, count(value))],
            ..Self::default()
        }
    }
}

/// Parameters of one edit, read before the notebook.
enum EditParams {
    Insert {
        position: Position,
        cell_type: CellType,
        source: String,
    },
    Replace {
        target: Option<CellRef>,
        source: Option<String>,
        cell_type: Option<CellType>,
        find: Option<FindPattern>,
        with: Option<String>,
        reset_outputs: bool,
    },
    Move {
        from: i64,
        to: i64,
    },
    Copy {
        from: i64,
        to: i64,
    },
    Remove(Selection),
    ClearOutputs(Selection),
    Normalize,
    Ops {
        changes: Vec<NotebookChange>,
        finds: Vec<(usize, FindPattern)>,
    },
}

impl EditParams {
    fn read(kind: EditKind, args: &BoundArgs) -> Result<Self> {
        Ok(match kind {
            EditKind::Insert => {
                let position = match (args.int(INDEX)?, args.text(AFTER_ID)?) {
                    (Some(_), Some(_)) => return Err(conflict(args, INDEX, AFTER_ID)),
                    (Some(index), None) => Position::Index(index),
                    (None, Some(id)) => Position::AfterId(id.to_owned()),
                    (None, None) => Position::End,
                };
                Self::Insert {
                    position,
                    cell_type: cell_type_param(args, CELL_TYPE)?
                        .ok_or_else(|| missing_param(args, CELL_TYPE))?,
                    source: args
                        .text(SOURCE)?
                        .ok_or_else(|| missing_param(args, SOURCE))?
                        .to_owned(),
                }
            }
            EditKind::Replace => {
                let whole_word = args.bool(WHOLE_WORD)?.unwrap_or(false);
                Self::Replace {
                    target: CellRef::from_parts(
                        args.int(INDEX)?,
                        args.text(ID)?.map(str::to_owned),
                    ),
                    source: args.text(SOURCE)?.map(str::to_owned),
                    cell_type: cell_type_param(args, CELL_TYPE)?,
                    find: find_param(args, FIND, whole_word)?,
                    with: args.text(WITH)?.map(str::to_owned),
                    reset_outputs: args.bool(RESET_OUTPUTS)?.unwrap_or(false),
                }
            }
            EditKind::Move => Self::Move {
                from: required_int(args, FROM)?,
                to: required_int(args, TO)?,
            },
            EditKind::Copy => Self::Copy {
                from: required_int(args, FROM)?,
                to: required_int(args, TO)?,
            },
            EditKind::Remove => Self::Remove(selection(args)?.ok_or_else(|| {
                Error::new(
                    ErrorCode::MissingParam,
                    "`notebook.remove` needs `index` or `id` of the cells to remove",
                )
            })?),
            EditKind::ClearOutputs => {
                Self::ClearOutputs(selection(args)?.unwrap_or(Selection::All))
            }
            EditKind::Normalize => Self::Normalize,
            EditKind::Ops => ops_params(args)?,
        })
    }

    fn apply(&self, notebook: &mut Notebook, path: &AbsPath) -> Result<EditReport> {
        let failed = |error: NotebookError| notebook_error(&error, path);
        match self {
            Self::Insert {
                position,
                cell_type,
                source,
            } => {
                let inserted = notebook
                    .insert(position, *cell_type, source)
                    .map_err(failed)?;
                Ok(EditReport {
                    after: Some(new_cell_value(inserted.index, inserted.id.as_deref())),
                    ..EditReport::counted(CELLS_INSERTED, 1)
                })
            }
            Self::Replace {
                target,
                source,
                cell_type,
                find,
                with,
                reset_outputs,
            } => {
                let replaced = notebook
                    .replace(&Replace {
                        target: target.clone(),
                        source: source.clone(),
                        cell_type: *cell_type,
                        find: find.as_ref().map(FindPattern::as_pattern),
                        with: with.clone(),
                        reset_outputs: *reset_outputs,
                    })
                    .map_err(failed)?;
                if let Some(error) = find.as_ref().and_then(FindPattern::template_error) {
                    return Err(error);
                }
                let mut report = EditReport::counted(CELLS_CHANGED, replaced.cells.len());
                if find.is_some() {
                    let replacements = count(replaced.replacements);
                    report.counters.push((REPLACEMENTS, replacements));
                    report.replacements = Some(replacements);
                }
                Ok(report)
            }
            Self::Move { from, to } => {
                notebook
                    .move_cell(&CellRef::Index(*from), *to)
                    .map_err(failed)?;
                Ok(EditReport::counted(CELLS_MOVED, 1))
            }
            Self::Copy { from, to } => {
                let inserted = notebook
                    .copy_cell(&CellRef::Index(*from), Some(*to))
                    .map_err(failed)?;
                Ok(EditReport {
                    after: Some(new_cell_value(inserted.index, inserted.id.as_deref())),
                    ..EditReport::counted(CELLS_COPIED, 1)
                })
            }
            Self::Remove(selection) => {
                let removed = notebook.remove(selection).map_err(failed)?;
                Ok(EditReport::counted(CELLS_REMOVED, removed.len()))
            }
            Self::ClearOutputs(selection) => {
                let cleared = notebook.clear_outputs(selection).map_err(failed)?;
                Ok(EditReport::counted(CELLS_CLEARED, cleared.len()))
            }
            Self::Normalize => {
                let normalized = notebook.normalize();
                Ok(EditReport::counted(CELLS_CHANGED, normalized.cells.len()))
            }
            Self::Ops { changes, finds } => {
                let outcomes = notebook.apply(changes).map_err(failed)?;
                for (position, find) in finds {
                    if let Some(error) = find.template_error() {
                        return Err(error.with_detail(CHANGE_DETAIL, json!(position)));
                    }
                }
                Ok(ops_report(&outcomes))
            }
        }
    }
}

fn required_int(args: &BoundArgs, name: &str) -> Result<i64> {
    args.int(name)?.ok_or_else(|| missing_param(args, name))
}

/// Cells named by `index` (a range) or `id` (one or more identifiers); an
/// `index` inserted as the default yields to `id`.
fn selection(args: &BoundArgs) -> Result<Option<Selection>> {
    let ids = ids_param(args, ID)?;
    let range = if args.is_defaulted(INDEX) && !ids.is_empty() {
        None
    } else {
        range_param(args, INDEX)?
    };
    match (range, ids.is_empty()) {
        (Some(_), false) => Err(conflict(args, INDEX, ID)),
        (Some(range), true) => Ok(Some(Selection::Range(range))),
        (None, false) => Ok(Some(Selection::Ids(ids))),
        (None, true) => Ok(None),
    }
}

/// Changes of `notebook.ops`. A `find` given as a regular expression is
/// compiled here, so that its errors keep the code `E_REGEX`.
fn ops_params(args: &BoundArgs) -> Result<EditParams> {
    let items = match args.value(CHANGES) {
        None | Some(Value::Null) => return Err(missing_param(args, CHANGES)),
        Some(Value::List(items)) => items,
        Some(_) => return Err(type_error(args, CHANGES, "a list of maps")),
    };
    let mut changes = Vec::with_capacity(items.len());
    let mut finds = Vec::new();
    for (position, item) in items.iter().enumerate() {
        let serde_json::Value::Object(map) = item.to_plain_json() else {
            return Err(change_error(
                position,
                NotebookError::InvalidChange {
                    detail: "each change must be a map, for example {action: \"remove\", index: 2}"
                        .to_owned(),
                },
            ));
        };
        let whole_word = matches!(item.get(WHOLE_WORD), Some(Value::Bool(true)));
        let find = match item.get(FIND) {
            Some(value @ Value::Regex(_)) => Some(
                FindPattern::from_value(args, FIND, value, whole_word)
                    .map_err(|error| error.with_detail(CHANGE_DETAIL, json!(position)))?,
            ),
            _ => None,
        };
        let mut compiled = find.as_ref().map(FindPattern::boxed);
        let change =
            NotebookChange::from_json(&map, &mut |value: &serde_json::Value, whole_word: bool| {
                compiled
                    .take()
                    .map_or_else(|| literal_pattern(value, whole_word), Ok)
            })
            .map_err(|error| change_error(position, error))?;
        changes.push(change);
        if let Some(find) = find {
            finds.push((position, find));
        }
    }
    Ok(EditParams::Ops { changes, finds })
}

/// Error of a change of `notebook.ops` read before the notebook.
fn change_error(position: usize, error: NotebookError) -> Error {
    NotebookError::Change {
        position,
        source: Box::new(error),
    }
    .to_error()
}

fn ops_report(outcomes: &[ChangeOutcome]) -> EditReport {
    let mut inserted = 0;
    let mut changed = 0;
    let mut moved = 0;
    let mut copied = 0;
    let mut removed = 0;
    let mut cleared = 0;
    let mut replacements = 0;
    let mut new_cells = Vec::new();
    for (position, outcome) in outcomes.iter().enumerate() {
        match outcome {
            ChangeOutcome::Inserted(cell) => {
                inserted += 1;
                new_cells.push(new_cell_at(position, cell.index, cell.id.as_deref()));
            }
            ChangeOutcome::Copied(cell) => {
                copied += 1;
                new_cells.push(new_cell_at(position, cell.index, cell.id.as_deref()));
            }
            ChangeOutcome::Replaced(replaced) => {
                changed += replaced.cells.len();
                replacements += replaced.replacements;
            }
            ChangeOutcome::Moved(_) => moved += 1,
            ChangeOutcome::Removed(cells) => removed += cells.len(),
            ChangeOutcome::Cleared(cells) => cleared += cells.len(),
            _ => {}
        }
    }
    let counters = [
        (OPERATIONS, outcomes.len()),
        (CELLS_INSERTED, inserted),
        (CELLS_CHANGED, changed),
        (CELLS_MOVED, moved),
        (CELLS_COPIED, copied),
        (CELLS_REMOVED, removed),
        (CELLS_CLEARED, cleared),
        (REPLACEMENTS, replacements),
    ]
    .into_iter()
    .filter(|(name, value)| *value > 0 || *name == OPERATIONS)
    .map(|(name, value)| (name, count(value)))
    .collect();
    EditReport {
        counters,
        replacements: (replacements > 0).then(|| count(replacements)),
        after: (!new_cells.is_empty()).then(|| Value::List(new_cells)),
    }
}

/// Map value of a cell created by the change at `position` of `notebook.ops`.
fn new_cell_at(position: usize, index: usize, id: Option<&str>) -> Value {
    let mut value = new_cell_value(index, id);
    if let Value::Map(fields) = &mut value {
        fields.shift_insert(
            0,
            CHANGE_DETAIL.to_owned(),
            Value::Int(i64::try_from(position).unwrap_or(i64::MAX)),
        );
    }
    value
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::{Follow, Fs};
    use nurt_contracts::journal::JournalStore;
    use nurt_contracts::record::Effect;
    use nurt_contracts::stage::{ArgValue, StageInput};
    use nurt_contracts::value::{Fields, Regex};
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::journal::JournalCall;

    use super::*;
    use crate::commands::notebook::fixture::with_file;
    use crate::common::etag::etag_text;

    const NOTEBOOK: &str = "{\n \"cells\": [\n  {\n   \"cell_type\": \"code\",\n   \"execution_count\": 3,\n   \"metadata\": {},\n   \"outputs\": [],\n   \"source\": [\n    \"x = 1\"\n   ]\n  },\n  {\n   \"cell_type\": \"markdown\",\n   \"metadata\": {},\n   \"source\": [\n    \"# Notes\"\n   ]\n  }\n ],\n \"metadata\": {},\n \"nbformat\": 4,\n \"nbformat_minor\": 4\n}\n";

    fn edit(command: &str) -> NotebookEdit {
        let kind = EditKind::ALL
            .into_iter()
            .find(|kind| kind.command() == command)
            .unwrap();
        NotebookEdit { kind }
    }

    fn run(command: &str, given: &[(&str, Value)]) -> (Result<Effect>, Vec<u8>, Vec<JournalCall>) {
        let (test, path) = with_file("/data/a.ipynb", NOTEBOOK.as_bytes());
        let args = given.iter().fold(
            BoundArgs::new(command).with(
                PATH,
                ArgValue::Value(Value::Path(path.to_language_string())),
                false,
            ),
            |args, (name, value)| args.with(*name, ArgValue::Value(value.clone()), false),
        );
        let stage = edit(command);
        let ctx = test.ctx();
        let result = stage
            .plan(&args, StageInput::none(), &ctx)
            .and_then(|plan| {
                let mut tx = test.journal().begin(begin_request(command));
                let outcome = stage.apply(&args, plan, tx.as_mut(), &ctx)?;
                tx.commit()?;
                let record = StageInput::from(outcome.output)
                    .into_records()?
                    .next()
                    .unwrap()?;
                Ok(Effect::try_from(record).unwrap())
            });
        (
            result,
            test.fs().read_file(&path).unwrap(),
            test.journal().calls(),
        )
    }

    #[test]
    fn moving_a_cell_swaps_the_cell_texts() {
        let (effect, bytes, _) = run(
            "notebook.move",
            &[("from", Value::Int(1)), ("to", Value::Int(0))],
        );
        let effect = effect.unwrap();
        assert_eq!(effect.kind, "notebook.move");
        assert_eq!(effect.targets, Some(1));
        let text = String::from_utf8(bytes).unwrap();
        let markdown = text.find("\"# Notes\"").unwrap();
        let code = text.find("\"x = 1\"").unwrap();
        assert!(markdown < code);
        let change = &effect.changes.unwrap()[0];
        assert!(change.added.unwrap() > 0);
        assert!(change.diff.as_ref().is_some_and(|diff| !diff.is_empty()));
    }

    #[test]
    fn an_edit_without_effect_writes_nothing() {
        let (effect, bytes, calls) = run("notebook.clear_outputs", &[("index", Value::Int(1))]);
        let effect = effect.unwrap();
        assert_eq!(effect.targets, Some(0));
        assert_eq!(bytes, NOTEBOOK.as_bytes());
        assert!(
            !calls
                .iter()
                .any(|call| matches!(call, JournalCall::Apply { .. }))
        );
    }

    #[test]
    fn insert_rejects_index_with_after_id() {
        let (effect, _, _) = run(
            "notebook.insert",
            &[
                ("index", Value::Int(0)),
                ("after_id", Value::Text("a".to_owned())),
                ("cell_type", Value::Enum("code".to_owned())),
                ("source", Value::Text("y".to_owned())),
            ],
        );
        assert_eq!(effect.unwrap_err().code(), ErrorCode::ParamConflict);
    }

    #[test]
    fn remove_needs_a_selection() {
        let (effect, _, _) = run("notebook.remove", &[]);
        assert_eq!(effect.unwrap_err().code(), ErrorCode::MissingParam);
    }

    #[test]
    fn ops_changes_must_be_maps() {
        let (effect, bytes, _) = run(
            "notebook.ops",
            &[("changes", Value::List(vec![Value::Int(1)]))],
        );
        let error = effect.unwrap_err();
        assert_eq!(error.code(), ErrorCode::Arg);
        assert_eq!(error.details.get("change"), Some(&json!(0)));
        assert_eq!(bytes, NOTEBOOK.as_bytes());
    }

    fn written(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn insert_places_the_cell_at_the_index_and_reports_it() {
        let (effect, bytes, _) = run(
            "notebook.insert",
            &[
                ("index", Value::Int(1)),
                ("cell_type", Value::Enum("code".to_owned())),
                ("source", Value::Text("print(2)".to_owned())),
            ],
        );
        let effect = effect.unwrap();
        assert_eq!(effect.kind, "notebook.insert");
        assert_eq!(effect.targets, Some(1));
        assert!(effect.after.is_some());
        let text = written(bytes);
        let first = text.find("\"x = 1\"").unwrap();
        let inserted = text.find("print(2)").unwrap();
        let last = text.find("\"# Notes\"").unwrap();
        assert!(first < inserted && inserted < last);
    }

    #[test]
    fn replace_changes_the_text_of_the_selected_cell() {
        let (effect, bytes, _) = run(
            "notebook.replace",
            &[
                ("index", Value::Int(0)),
                ("find", Value::Text("x = 1".to_owned())),
                ("with", Value::Text("x = 2".to_owned())),
            ],
        );
        assert_eq!(effect.unwrap().targets, Some(1));
        let text = written(bytes);
        assert!(text.contains("\"x = 2\""));
        assert!(!text.contains("\"x = 1\""));
    }

    #[test]
    fn replace_expands_the_groups_of_a_regular_expression() {
        let (effect, bytes, _) = run(
            "notebook.replace",
            &[
                ("find", Value::Regex(Regex::new(r"x = (\d)", "").unwrap())),
                ("with", Value::Text("y = $1$1".to_owned())),
            ],
        );
        assert!(effect.is_ok());
        assert!(written(bytes).contains("\"y = 11\""));
    }

    #[test]
    fn replace_rejects_a_template_with_a_missing_group_and_writes_nothing() {
        let (effect, bytes, _) = run(
            "notebook.replace",
            &[
                ("find", Value::Regex(Regex::new(r"x = (\d)", "").unwrap())),
                ("with", Value::Text("y = $2".to_owned())),
            ],
        );
        assert_eq!(effect.unwrap_err().code(), ErrorCode::Regex);
        assert_eq!(bytes, NOTEBOOK.as_bytes());
    }

    #[test]
    fn copy_repeats_the_cell_at_the_target_index() {
        let (effect, bytes, _) = run(
            "notebook.copy",
            &[("from", Value::Int(0)), ("to", Value::Int(2))],
        );
        assert_eq!(effect.unwrap().targets, Some(1));
        assert_eq!(written(bytes).matches("\"x = 1\"").count(), 2);
    }

    #[test]
    fn ops_apply_every_change_in_one_write() {
        let mut remove = Fields::new();
        remove.insert("action".to_owned(), Value::Text("remove".to_owned()));
        remove.insert("index".to_owned(), Value::Int(1));
        let (effect, bytes, _) = run(
            "notebook.ops",
            &[("changes", Value::List(vec![Value::Map(remove)]))],
        );
        assert_eq!(effect.unwrap().targets, Some(1));
        let text = written(bytes);
        assert!(text.contains("\"x = 1\""));
        assert!(!text.contains("# Notes"));
    }

    #[test]
    fn a_stale_tag_stops_the_edit_in_the_plan() {
        let (test, path) = with_file("/data/a.ipynb", NOTEBOOK.as_bytes());
        let before = test.fs().metadata(&path, Follow::NoFollow).unwrap().etag();
        let changed = NOTEBOOK.replace("x = 1", "x = 100");
        test.fs().write_file(&path, changed.as_bytes()).unwrap();
        let args = BoundArgs::new("notebook.remove")
            .with(
                PATH,
                ArgValue::Value(Value::Path(path.to_language_string())),
                false,
            )
            .with("index", ArgValue::Value(Value::Int(0)), false)
            .with(
                "if_match",
                ArgValue::Value(Value::Text(etag_text(&before))),
                false,
            );
        let error = edit("notebook.remove")
            .plan(&args, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(test.fs().read_file(&path).unwrap(), changed.as_bytes());
    }

    #[test]
    fn ops_report_counts_each_kind_of_change() {
        let report = ops_report(&[ChangeOutcome::Removed(vec![0, 1]), ChangeOutcome::Moved(2)]);
        assert_eq!(
            report.counters,
            [(OPERATIONS, 2), (CELLS_MOVED, 1), (CELLS_REMOVED, 2)]
        );
        assert_eq!(report.after, None);
        assert_eq!(report.replacements, None);
    }
}
