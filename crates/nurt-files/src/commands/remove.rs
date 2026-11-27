// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::commands::remove
// Responsibility:   `remove`: removes files, links and directory trees, and lines and.
// Dependencies:     nurt_contracts, nurt_testkit, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `remove`: removes files, links and directory trees, and lines and
//! matches from files.
//!
//! Every removed entry is one target of the transaction: a directory is
//! removed with its content as one target, a link is removed as a link and
//! its target stays. A path inside another removed path is removed once,
//! with the enclosing directory. `line` and `match` records remove their
//! text from the file they come from; the file is written once, in its
//! original encoding and line endings.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Etag, FileKind, FileMeta, Fs};
use nurt_contracts::journal::{Expect, FileOp, Mutation, MutationTx};
use nurt_contracts::report::{Notable, NotableEntry, tree_flags};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, StageDescriptor,
    StageImpl, StageInput,
};
use nurt_contracts::value::{Record, RecordKind, Size, Value};
use serde_json::Value as Json;

use super::conflict::{
    CHECK_REFERENCES, CHECKPOINT_INTERVAL, Source, Step, count, effect_output, flag,
    input_kind_error, missing_state, references_left, run_steps, walk_tree,
};
use crate::common::etag::{Observed, check_if_match, parse_etag, stale_error};
use crate::common::params::{EncodingParam, OnMissing, on_missing_param};
use crate::common::path::{PATH_FIELD, path_list_arg_for_change, record_path};
use crate::common::text::{TextFile, encode_preserving, read_text, text_change};
use crate::registry;

const COMMAND: &str = "remove";
const PATH: &str = "path";
const LINE_FIELD: &str = "no";
const COLUMN_FIELD: &str = "column";
const TEXT_FIELD: &str = "text";
const ETAG_FIELD: &str = "etag";

/// Flag of the change of a directory removed with its content.
const TREE_FLAG: &str = "recursive";

/// Stage of `remove` for entries, lines and matches.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Remove;

/// Stages of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Remove)]
}

impl StageImpl for Remove {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![
                RecordKind::Entry,
                RecordKind::RegistryValue,
                RecordKind::Line,
                RecordKind::Match,
            ]),
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

/// Counters of the `effect` record.
#[derive(Debug, Clone, Copy, Default)]
struct Summary {
    removed: u64,
    files: u64,
    directories: u64,
    links: u64,
    bytes: u64,
    missing: u64,
    lines: u64,
    matches: u64,
}

impl Summary {
    fn pairs(self) -> [(&'static str, u64); 8] {
        [
            ("removed", self.removed),
            ("files", self.files),
            ("directories", self.directories),
            ("links", self.links),
            ("bytes", self.bytes),
            ("missing", self.missing),
            ("lines", self.lines),
            ("matches", self.matches),
        ]
    }
}

/// Private state of the plan: the removals, and the text edits computed
/// again from the files in the apply step.
#[derive(Debug)]
struct RemoveState {
    steps: Vec<Step>,
    tree_bytes: Vec<(usize, u64)>,
    edits: Vec<TextEdit>,
    summary: Summary,
}

impl EffectStage for Remove {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let fs = ctx.require_fs()?;
        let on_missing = on_missing_param(args)?;
        let check_references = flag(args, CHECK_REFERENCES, false)?;
        let (entries, edits, registered) = read_input(args, input, ctx)?;
        let mut planner = Planner {
            fs,
            ctx,
            on_missing,
            steps: Vec::new(),
            tree_bytes: Vec::new(),
            removed: Vec::new(),
            edits: Vec::new(),
            edit_changes: Vec::new(),
            summary: Summary::default(),
        };
        planner.plan_entries(entries)?;
        planner.plan_registry(&registered)?;
        planner.plan_edits(edits)?;
        let mut notable = Vec::new();
        if check_references && let Some(entry) = references_left(ctx, &planner.removed)? {
            notable.push(entry);
        }
        Ok(planner.finish(notable))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let fs = ctx.require_fs()?;
        let state = plan
            .take_state::<RemoveState>()
            .ok_or_else(|| missing_state(COMMAND))?;
        let mut steps = state.steps;
        for edit in &state.edits {
            let file = edit.read_current(ctx)?;
            let (step, _) = edit.step(&file)?;
            steps.push(step);
        }
        let mut changes = run_steps(tx, steps, fs)?;
        for (index, bytes) in &state.tree_bytes {
            if let Some(change) = changes.get_mut(*index) {
                change.size_before = Some(Size::from_bytes(*bytes));
            }
        }
        let output = effect_output(COMMAND, tx, &changes, &plan.notable, &state.summary.pairs());
        let mut outcome = EffectOutcome::new(output);
        outcome.changes = changes;
        Ok(outcome)
    }
}

/// Entries, text edits and registry paths of the input.
type Input = (Vec<Source>, BTreeMap<AbsPath, TextEdit>, Vec<String>);

/// Entries and text edits of the input, in input order for entries and by
/// path for edits, and the registry paths to remove.
fn read_input(args: &BoundArgs, input: StageInput<'_>, ctx: &ExecCtx) -> Result<Input> {
    let mut entries = Vec::new();
    let mut edits = BTreeMap::new();
    let mut registered = Vec::new();
    if input.is_none() {
        args.require(PATH)?;
        if let Some(texts) = registry::path_texts(args, PATH)? {
            return Ok((entries, edits, texts));
        }
        entries = path_list_arg_for_change(args, PATH, ctx)?
            .into_iter()
            .map(Source::from_path)
            .collect();
        return Ok((entries, edits, registered));
    }
    for (index, item) in input.into_records()?.enumerate() {
        if index % CHECKPOINT_INTERVAL == 0 {
            ctx.checkpoint()?;
        }
        let record = item?;
        match record.kind {
            RecordKind::Entry | RecordKind::RegistryValue => match registry::record_text(&record) {
                Some(text) => registered.push(text),
                None if record.kind == RecordKind::Entry => {
                    entries.push(Source::from_record(record, ctx)?);
                }
                None => return Err(input_kind_error(COMMAND, &record)),
            },
            RecordKind::Line | RecordKind::Match => {
                let path = record_path(&record, ctx)?;
                edits
                    .entry(path.clone())
                    .or_insert_with(|| TextEdit::new(path))
                    .add(&record)?;
            }
            _ => return Err(input_kind_error(COMMAND, &record)),
        }
    }
    Ok((entries, edits, registered))
}

struct Planner<'a> {
    fs: &'a dyn Fs,
    ctx: &'a ExecCtx,
    on_missing: OnMissing,
    steps: Vec<Step>,
    tree_bytes: Vec<(usize, u64)>,
    removed: Vec<AbsPath>,
    edits: Vec<TextEdit>,
    edit_changes: Vec<Step>,
    summary: Summary,
}

impl Planner<'_> {
    fn plan_entries(&mut self, entries: Vec<Source>) -> Result<()> {
        let mut present: Vec<(AbsPath, FileMeta)> = Vec::new();
        for (index, source) in entries.into_iter().enumerate() {
            if index % CHECKPOINT_INTERVAL == 0 {
                self.ctx.checkpoint()?;
            }
            match source.observe(self.fs)? {
                Observed::Present(meta, _) => present.push((source.path, meta)),
                Observed::Absent if self.on_missing == OnMissing::Skip => self.summary.missing += 1,
                Observed::Absent => return Err(missing_error(&source.path)),
            }
        }
        present.sort_by(|left, right| left.0.cmp(&right.0));
        let mut kept: Option<AbsPath> = None;
        for (path, meta) in present {
            if kept.as_ref().is_some_and(|outer| path.starts_with(outer)) {
                continue;
            }
            self.remove_entry(&path, &meta)?;
            kept = Some(path);
        }
        Ok(())
    }

    fn remove_entry(&mut self, path: &AbsPath, meta: &FileMeta) -> Result<()> {
        let mut bytes = meta.size;
        let mut recursive = false;
        let mut inside_files = 0_u64;
        let mut inside_directories = 0_u64;
        match meta.kind {
            FileKind::Directory => {
                bytes = 0;
                self.summary.directories += 1;
                for entry in walk_tree(self.fs, self.ctx, path)? {
                    recursive = true;
                    match entry.kind {
                        FileKind::Directory => {
                            self.summary.directories += 1;
                            inside_directories += 1;
                        }
                        FileKind::Symlink | FileKind::Junction => {
                            self.summary.links += 1;
                            inside_files += 1;
                        }
                        _ => {
                            self.summary.files += 1;
                            inside_files += 1;
                            bytes = bytes.saturating_add(entry.meta.map_or(0, |meta| meta.size));
                        }
                    }
                }
            }
            FileKind::Symlink | FileKind::Junction => self.summary.links += 1,
            _ => self.summary.files += 1,
        }
        self.summary.bytes = self.summary.bytes.saturating_add(bytes);
        self.summary.removed += 1;
        let op = FileOp::remove(path.clone())
            .expect(Expect::Etag(meta.etag()))
            .size_hint(bytes);
        let mut step = Step::op(op);
        if meta.kind == FileKind::Directory {
            let mut change = step.change();
            let mut flags = Vec::new();
            if recursive {
                flags.push(TREE_FLAG.to_owned());
            }
            flags.extend(tree_flags(inside_files, inside_directories));
            change.flags = Some(flags);
            step = step.with_detail(change);
        }
        if meta.kind == FileKind::Directory {
            self.tree_bytes.push((self.steps.len(), bytes));
        }
        self.steps.push(step);
        self.removed.push(path.clone());
        Ok(())
    }

    fn plan_registry(&mut self, texts: &[String]) -> Result<()> {
        let removals =
            registry::plan_removals(self.ctx, texts, self.on_missing == OnMissing::Skip)?;
        self.summary.removed += removals.keys + removals.values;
        self.summary.directories += removals.keys;
        self.summary.files += removals.values;
        self.summary.missing += removals.missing;
        self.steps.extend(removals.steps);
        Ok(())
    }

    fn plan_edits(&mut self, edits: BTreeMap<AbsPath, TextEdit>) -> Result<()> {
        for (_, mut edit) in edits {
            self.ctx.checkpoint()?;
            if let Some(removed) = self
                .removed
                .iter()
                .find(|removed| edit.path.starts_with(removed))
            {
                return Err(Error::new(
                    ErrorCode::InvalidValue,
                    format!(
                        "`{}` is removed with `{removed}` and edited in the same call: remove the file or its lines, not both",
                        edit.path
                    ),
                )
                .with_detail(PATH_FIELD, Json::from(edit.path.to_language_string())));
            }
            let file = read_text(self.ctx, &edit.path, EncodingParam::Auto)?;
            check_if_match(
                &edit.path,
                &Observed::Present(file.meta.clone(), Some(file.hash)),
                edit.etag.as_ref(),
            )?;
            edit.etag = Some(file.etag());
            let (step, counts) = edit.step(&file)?;
            self.summary.lines += counts.lines;
            self.summary.matches += counts.matches;
            self.edit_changes.push(step);
            self.edits.push(edit);
        }
        Ok(())
    }

    fn finish(self, mut notable: Vec<NotableEntry>) -> EffectPlan {
        let targets = self.steps.len() + self.edit_changes.len();
        if targets == 0 {
            notable.push(NotableEntry::new(Notable::NothingMatched));
        }
        let changes = self
            .steps
            .iter()
            .chain(&self.edit_changes)
            .map(Step::change)
            .collect();
        let mut plan = EffectPlan::new().with_state(RemoveState {
            steps: self.steps,
            tree_bytes: self.tree_bytes,
            edits: self.edits,
            summary: self.summary,
        });
        plan.changes = changes;
        plan.notable = notable;
        plan.bulk = targets > 1;
        plan
    }
}

fn missing_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::NotFound,
        format!("`{path}` does not exist: pass `on_missing: skip` to ignore missing paths"),
    )
    .with_detail(PATH_FIELD, Json::from(path.to_language_string()))
}

/// Match to remove: its line and column, from 1, and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchSpan {
    line: u64,
    column: u64,
    text: String,
}

/// Lines and matches removed from one file.
#[derive(Debug, Clone)]
struct TextEdit {
    path: AbsPath,
    etag: Option<Etag>,
    lines: BTreeSet<u64>,
    matches: Vec<MatchSpan>,
}

/// Numbers of removed lines and matches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Counts {
    lines: u64,
    matches: u64,
}

impl TextEdit {
    fn new(path: AbsPath) -> Self {
        Self {
            path,
            etag: None,
            lines: BTreeSet::new(),
            matches: Vec::new(),
        }
    }

    fn add(&mut self, record: &Record) -> Result<()> {
        if let Some(Value::Text(text)) = record.get(ETAG_FIELD) {
            let tag = parse_etag(text)?;
            match &self.etag {
                Some(known) if !known.matches(&tag) => {
                    return Err(stale_error(&self.path, known, Some(&tag)));
                }
                Some(_) => {}
                None => self.etag = Some(tag),
            }
        }
        let line = number(record, LINE_FIELD)?;
        if record.kind == RecordKind::Line {
            self.lines.insert(line);
            return Ok(());
        }
        let text = match record.get(TEXT_FIELD) {
            Some(Value::Text(text)) => text.clone(),
            _ => return Err(field_error(record, TEXT_FIELD, "a text")),
        };
        let span = MatchSpan {
            line,
            column: number(record, COLUMN_FIELD)?,
            text,
        };
        if !self.matches.contains(&span) {
            self.matches.push(span);
        }
        Ok(())
    }

    /// Reads the file again for the apply step and checks that it is the
    /// version the plan read.
    fn read_current(&self, ctx: &ExecCtx) -> Result<TextFile> {
        let file = read_text(ctx, &self.path, EncodingParam::Auto)?;
        check_if_match(
            &self.path,
            &Observed::Present(file.meta.clone(), Some(file.hash)),
            self.etag.as_ref(),
        )?;
        Ok(file)
    }

    /// Step that writes the file without the removed text.
    fn step(&self, file: &TextFile) -> Result<(Step, Counts)> {
        let (text, counts) = self.apply(&file.text)?;
        let bytes = encode_preserving(&text, file)?;
        let mut change = text_change(file, &bytes);
        change.removed = (counts.lines > 0).then_some(counts.lines);
        change.replacements = (counts.matches > 0).then_some(counts.matches);
        let step = Step::new(file.target(), vec![Mutation::WriteBytes(bytes)]).with_detail(change);
        Ok((step, counts))
    }

    /// Text without the removed lines and matches. A removed line takes its
    /// line ending with it; when the last line of a text without a final
    /// line ending is removed, the line ending before it goes as well.
    fn apply(&self, text: &str) -> Result<(String, Counts)> {
        let starts = line_starts(text);
        let line_range = |line: u64| -> Result<Range<usize>> {
            let index = usize::try_from(line - 1).unwrap_or(usize::MAX);
            let start = *starts
                .get(index)
                .ok_or_else(|| self.line_error(line, starts.len()))?;
            let end = starts.get(index + 1).copied().unwrap_or(text.len());
            Ok(start..end)
        };
        let mut ranges = Vec::new();
        for line in &self.lines {
            ranges.push(line_range(*line)?);
        }
        for span in &self.matches {
            let line = line_range(span.line)?;
            // The column counts within the line, up to its line ending; the
            // text of a match may run on into the following lines.
            let within = text[line.clone()].trim_end_matches(['\n', '\r']);
            let offset = within
                .char_indices()
                .map(|(offset, _)| offset)
                .chain(std::iter::once(within.len()))
                .nth(usize::try_from(span.column - 1).unwrap_or(usize::MAX))
                .filter(|offset| text[line.start + offset..].starts_with(&span.text))
                .ok_or_else(|| self.moved_match_error(span))?;
            let start = line.start + offset;
            ranges.push(start..start + span.text.len());
        }
        ranges.sort_by_key(|range| (range.start, range.end));
        let mut result = String::with_capacity(text.len());
        let mut position = 0;
        for range in ranges {
            if range.start > position {
                result.push_str(&text[position..range.start]);
            }
            position = position.max(range.end);
        }
        result.push_str(&text[position..]);
        let open_end = !text.ends_with(['\n', '\r']);
        let last_removed = self
            .lines
            .last()
            .is_some_and(|line| usize::try_from(*line).is_ok_and(|line| line == starts.len()));
        if open_end && last_removed {
            let ending = if result.ends_with("\r\n") {
                2
            } else {
                usize::from(result.ends_with(['\n', '\r']))
            };
            result.truncate(result.len() - ending);
        }
        let counts = Counts {
            lines: count(self.lines.len()),
            matches: count(self.matches.len()),
        };
        Ok((result, counts))
    }

    fn line_error(&self, line: u64, lines: usize) -> Error {
        Error::new(
            ErrorCode::Stale,
            format!(
                "line {line} is outside `{}`, which has {lines} lines: read the file again",
                self.path
            ),
        )
        .with_detail(PATH_FIELD, Json::from(self.path.to_language_string()))
    }

    fn moved_match_error(&self, span: &MatchSpan) -> Error {
        Error::new(
            ErrorCode::Stale,
            format!(
                "`{}` no longer has `{}` at line {}, column {}: search the file again",
                self.path, span.text, span.line, span.column
            ),
        )
        .with_detail(PATH_FIELD, Json::from(self.path.to_language_string()))
    }
}

/// Byte offsets where the lines of `text` start; `\n`, `\r\n` and `\r` end
/// a line, and a final line ending starts no further line.
fn line_starts(text: &str) -> Vec<usize> {
    if text.is_empty() {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    let mut starts = vec![0];
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => starts.push(index + 1),
            b'\r' => {
                if bytes.get(index + 1) == Some(&b'\n') {
                    index += 1;
                }
                starts.push(index + 1);
            }
            _ => {}
        }
        index += 1;
    }
    if starts.last() == Some(&text.len()) {
        starts.pop();
    }
    starts
}

/// Positive number in a field of a `line` or `match` record.
fn number(record: &Record, field: &str) -> Result<u64> {
    match record.get(field) {
        Some(Value::Int(value)) if *value >= 1 => {
            u64::try_from(*value).map_err(|_| field_error(record, field, "a number from 1"))
        }
        _ => Err(field_error(record, field, "a number from 1")),
    }
}

fn field_error(record: &Record, field: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!(
            "the `{field}` field of a `{}` record must be {expected}: pass records of `read`, `lines` or `search`",
            record.kind.as_str()
        ),
    )
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::{Follow, Holder};
    use nurt_contracts::record::ChangeAction;
    use nurt_contracts::stage::{ArgValue, StageImpl};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::Fault;

    use super::super::conflict::testing::{
        abs, assert_journaled, assert_untouched, choice_arg, dir, entry, file, paths_arg, run,
        text_arg,
    };
    use super::*;
    use crate::common::etag::etag_text;
    use crate::common::params::ON_MISSING;

    fn args(paths: &[&AbsPath]) -> BoundArgs {
        BoundArgs::new(COMMAND).with(PATH, paths_arg(paths), false)
    }

    fn etag_of(test: &TestContext, path: &AbsPath) -> Value {
        Value::Text(etag_text(
            &test.fs().metadata(path, Follow::NoFollow).unwrap().etag(),
        ))
    }

    fn line(test: &TestContext, path: &AbsPath, no: i64) -> Record {
        Record::new(RecordKind::Line)
            .with("path", Value::Path(path.to_language_string()))
            .with("no", Value::Int(no))
            .with("etag", etag_of(test, path))
    }

    fn found(test: &TestContext, path: &AbsPath, no: i64, column: i64, text: &str) -> Record {
        Record::new(RecordKind::Match)
            .with("path", Value::Path(path.to_language_string()))
            .with("no", Value::Int(no))
            .with("column", Value::Int(column))
            .with("text", Value::Text(text.to_owned()))
            .with("etag", etag_of(test, path))
    }

    #[test]
    fn descriptor_accepts_entries_lines_and_matches() {
        let descriptor = Remove.descriptor();
        assert_eq!(descriptor.command, "remove");
        for kind in [RecordKind::Entry, RecordKind::Line, RecordKind::Match] {
            assert!(descriptor.input.accepts(kind));
        }
        assert!(!descriptor.input.accepts(RecordKind::Item));
    }

    #[test]
    fn removes_files_and_directories_each_as_one_target() {
        let test = TestContext::new();
        let single = file(&test, "/data/a.txt", b"abc");
        file(&test, "/data/tree/x.txt", b"12345");
        file(&test, "/data/tree/sub/y.txt", b"1");
        let tree = abs("/data/tree");
        let run = run(&Remove, &test, &args(&[&single, &tree]), None).unwrap();
        assert!(!test.fs().exists(&single));
        assert!(!test.fs().exists(&tree));
        assert_eq!(run.changes.len(), 2);
        assert!(
            run.changes
                .iter()
                .all(|change| change.action == ChangeAction::Delete)
        );
        let tree_change = run
            .planned
            .iter()
            .find(|change| change.path.as_str() == tree.to_language_string())
            .unwrap();
        assert_eq!(
            tree_change.flags,
            Some(vec![
                TREE_FLAG.to_owned(),
                "directory".to_owned(),
                "files_inside=2".to_owned(),
                "directories_inside=1".to_owned(),
            ])
        );
        assert_eq!(run.summary("files"), Some(3));
        assert_eq!(run.summary("directories"), Some(2));
        assert_eq!(run.summary("bytes"), Some(9));
        assert_journaled(&test);
    }

    #[test]
    fn dl_15_a_path_inside_another_is_removed_once() {
        let test = TestContext::new();
        let inner = file(&test, "/data/tree/x.txt", b"x");
        let tree = abs("/data/tree");
        let run = run(&Remove, &test, &args(&[&inner, &tree, &tree]), None).unwrap();
        assert_eq!(run.changes.len(), 1);
        assert_eq!(run.changes[0].path.as_str(), tree.to_language_string());
    }

    #[test]
    fn dl_16_a_link_is_removed_and_its_target_stays() {
        let test = TestContext::new();
        let target = file(&test, "/data/target.txt", b"t");
        test.fs().symlink("target.txt", &abs("/data/link")).unwrap();
        let run = run(&Remove, &test, &args(&[&abs("/data/link")]), None).unwrap();
        assert!(!test.fs().exists(&abs("/data/link")));
        assert!(test.fs().exists(&target));
        assert_eq!(run.summary("links"), Some(1));
    }

    #[test]
    fn dl_03_and_dl_23_missing_paths_follow_on_missing() {
        let test = TestContext::new();
        let present = file(&test, "/data/a.txt", b"a");
        let missing = abs("/data/none");
        let mixed = run(&Remove, &test, &args(&[&missing, &present]), None).unwrap();
        assert_eq!(mixed.summary("missing"), Some(1));
        assert!(!test.fs().exists(&present));

        let test = TestContext::new();
        let call = args(&[&missing]).with(ON_MISSING, choice_arg("error"), false);
        let error = run(&Remove, &test, &call, None).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        assert!(error.message.contains("on_missing: skip"));

        let skipped = run(&Remove, &test, &args(&[&missing]), None).unwrap();
        assert_eq!(skipped.codes(), [Notable::NothingMatched]);
        let info = entry(&missing).with("exists", Value::Bool(false));
        let from_info = run(&Remove, &test, &BoundArgs::new(COMMAND), Some(vec![info])).unwrap();
        assert_eq!(from_info.summary("missing"), Some(1));
    }

    #[test]
    fn d_4_relative_or_missing_paths_fail_before_any_change() {
        let test = TestContext::new();
        file(&test, "/data/a.txt", b"a");
        let relative = BoundArgs::new(COMMAND).with(PATH, text_arg("data/a.txt"), false);
        let error = run(&Remove, &test, &relative, None).unwrap_err();
        assert_eq!(error.code(), ErrorCode::PathRelative);
        let absent = BoundArgs::new(COMMAND).with(PATH, ArgValue::Absent, false);
        let error = run(&Remove, &test, &absent, None).unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
        assert_untouched(&test);
    }

    #[test]
    fn removes_lines_and_keeps_line_endings() {
        let test = TestContext::new();
        let path = file(&test, "/data/a.txt", b"one\r\ntwo\r\nthree\r\n");
        let records = vec![line(&test, &path, 1), line(&test, &path, 3)];
        let run = run(&Remove, &test, &BoundArgs::new(COMMAND), Some(records)).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"two\r\n");
        assert_eq!(run.changes[0].removed, Some(2));
        assert_eq!(run.changes[0].action, ChangeAction::Modify);
        assert_eq!(run.summary("lines"), Some(2));
        assert_journaled(&test);
    }

    #[test]
    fn removing_the_last_open_line_drops_the_line_ending_before_it() {
        let edit = TextEdit {
            path: abs("/a"),
            etag: None,
            lines: BTreeSet::from([3]),
            matches: Vec::new(),
        };
        assert_eq!(edit.apply("a\nb\nc").unwrap().0, "a\nb");
        assert_eq!(edit.apply("a\nb\nc\n").unwrap().0, "a\nb\n");
        assert_eq!(edit.apply("a\n").unwrap_err().code(), ErrorCode::Stale);
        assert_eq!(line_starts("a\r\nb\rc\n"), [0, 3, 5]);
        assert!(line_starts("").is_empty());
    }

    #[test]
    fn removing_the_last_open_line_keeps_the_blank_lines_before_it() {
        let edit = TextEdit {
            path: abs("/a"),
            etag: None,
            lines: BTreeSet::from([3]),
            matches: Vec::new(),
        };
        assert_eq!(edit.apply("a\n\nc").unwrap().0, "a\n");
        assert_eq!(edit.apply("a\r\n\r\nc").unwrap().0, "a\r\n");
    }

    #[test]
    fn line_numbers_below_one_are_rejected() {
        let test = TestContext::new();
        let path = file(&test, "/data/a.txt", b"one\n");
        for no in [0, -1] {
            let records = vec![line(&test, &path, no)];
            let error = run(&Remove, &test, &BoundArgs::new(COMMAND), Some(records)).unwrap_err();
            assert_eq!(error.code(), ErrorCode::Type);
        }
        assert_untouched(&test);
    }

    #[test]
    fn a_removed_directory_reports_the_bytes_of_its_content_as_planned() {
        let test = TestContext::new();
        file(&test, "/data/tree/x.txt", b"1");
        let tree = abs("/data/tree");
        let run = run(&Remove, &test, &args(&[&tree]), None).unwrap();
        assert_eq!(run.planned[0].size_before, Some(Size::from_bytes(1)));
        assert_eq!(run.changes[0].size_before, run.planned[0].size_before);
    }

    #[test]
    fn an_empty_directory_is_removed_with_zero_bytes() {
        let test = TestContext::new();
        let empty = dir(&test, "/data/empty");
        let run = run(&Remove, &test, &args(&[&empty]), None).unwrap();
        assert!(!test.fs().exists(&empty));
        assert_eq!(run.summary("directories"), Some(1));
        assert_eq!(run.summary("bytes"), Some(0));
        assert_eq!(
            run.planned[0].flags,
            Some(vec![
                "directory".to_owned(),
                "files_inside=0".to_owned(),
                "directories_inside=0".to_owned(),
            ])
        );
        assert_eq!(run.changes[0].size_before, Some(Size::from_bytes(0)));
    }

    #[test]
    fn match_records_need_a_text() {
        let test = TestContext::new();
        let path = file(&test, "/data/a.txt", b"foo\n");
        let record = Record::new(RecordKind::Match)
            .with("path", Value::Path(path.to_language_string()))
            .with("no", Value::Int(1))
            .with("column", Value::Int(1))
            .with("etag", etag_of(&test, &path));
        let error = run(&Remove, &test, &BoundArgs::new(COMMAND), Some(vec![record])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Type);
        assert_untouched(&test);
    }

    #[test]
    fn a_match_that_is_no_longer_at_its_column_is_stale() {
        let test = TestContext::new();
        let path = file(&test, "/data/a.txt", b"foo bar\n");
        let records = vec![found(&test, &path, 1, 2, "foo")];
        let error = run(&Remove, &test, &BoundArgs::new(COMMAND), Some(records)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"foo bar\n");
        assert_untouched(&test);
    }

    #[test]
    fn a_column_past_the_end_of_its_line_does_not_reach_the_next_line() {
        let test = TestContext::new();
        let path = file(&test, "/data/a.txt", b"ab\nfoo\n");
        let records = vec![found(&test, &path, 1, 4, "foo")];
        let error = run(&Remove, &test, &BoundArgs::new(COMMAND), Some(records)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"ab\nfoo\n");
        assert_untouched(&test);
    }

    #[test]
    fn a_match_that_starts_at_a_line_ending_is_removed() {
        let test = TestContext::new();
        let path = file(&test, "/data/a.txt", b"ab\nfoo\n");
        let records = vec![found(&test, &path, 1, 3, "\nfoo")];
        run(&Remove, &test, &BoundArgs::new(COMMAND), Some(records)).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"ab\n");
    }

    #[test]
    fn removes_matches_by_line_and_column() {
        let test = TestContext::new();
        let path = file(&test, "/data/a.txt", "caf\u{e9} foo bar foo\n".as_bytes());
        let records = vec![
            found(&test, &path, 1, 6, "foo"),
            found(&test, &path, 1, 14, "foo"),
        ];
        let run = run(&Remove, &test, &BoundArgs::new(COMMAND), Some(records)).unwrap();
        assert_eq!(
            test.fs().read_file(&path).unwrap(),
            "caf\u{e9}  bar \n".as_bytes()
        );
        assert_eq!(run.changes[0].replacements, Some(2));
        assert_eq!(run.summary("matches"), Some(2));
    }

    #[test]
    fn line_records_of_a_changed_file_are_stale() {
        let test = TestContext::new();
        let path = file(&test, "/data/a.txt", b"one\ntwo\n");
        let record = line(&test, &path, 1);
        test.fs().write_file(&path, b"zero\none\ntwo\n").unwrap();
        let error = run(&Remove, &test, &BoundArgs::new(COMMAND), Some(vec![record])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"zero\none\ntwo\n");
        assert_untouched(&test);
    }

    #[test]
    fn a_file_cannot_be_removed_and_edited_at_once() {
        let test = TestContext::new();
        let path = file(&test, "/data/a.txt", b"one\n");
        let records = vec![entry(&path), line(&test, &path, 1)];
        let error = run(&Remove, &test, &BoundArgs::new(COMMAND), Some(records)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[cfg(unix)]
    #[test]
    fn a_record_of_a_path_that_is_not_valid_unicode_removes_that_entry() {
        let test = TestContext::new();
        let path = AbsPath::from_raw_bytes(b"/data/bad\xffname.txt").unwrap();
        dir(&test, "/data");
        test.fs().write_file(&path, b"abc").unwrap();
        let record = entry(&path).with(
            "raw_path",
            Value::Bytes(path.raw_bytes().expect("the name is not valid Unicode")),
        );
        let run = run(&Remove, &test, &BoundArgs::new(COMMAND), Some(vec![record])).unwrap();
        assert!(!test.fs().exists(&path));
        assert_eq!(run.changes.len(), 1);
        assert_eq!(run.changes[0].path.as_str(), "/data/bad\u{fffd}name.txt");
        assert_eq!(run.changes[0].raw_path, path.raw_bytes());
        assert_journaled(&test);
    }

    #[cfg(unix)]
    #[test]
    fn the_display_form_of_a_path_that_is_not_valid_unicode_names_no_entry() {
        let test = TestContext::new();
        let path = AbsPath::from_raw_bytes(b"/data/bad\xffname.txt").unwrap();
        dir(&test, "/data");
        test.fs().write_file(&path, b"abc").unwrap();
        let record = entry(&path);
        let run = run(&Remove, &test, &BoundArgs::new(COMMAND), Some(vec![record])).unwrap();
        assert!(run.changes.is_empty());
        assert_eq!(run.codes(), [Notable::NothingMatched]);
        assert!(test.fs().exists(&path));
    }

    #[cfg(unix)]
    #[test]
    fn a_path_written_by_a_stage_wins_over_the_raw_path_of_the_record() {
        let test = TestContext::new();
        let bad = AbsPath::from_raw_bytes(b"/data/bad\xffname.txt").unwrap();
        dir(&test, "/data");
        test.fs().write_file(&bad, b"abc").unwrap();
        let other = file(&test, "/data/other.txt", b"o");
        let record = entry(&other).with(
            "raw_path",
            Value::Bytes(bad.raw_bytes().expect("the name is not valid Unicode")),
        );
        run(&Remove, &test, &BoundArgs::new(COMMAND), Some(vec![record])).unwrap();
        assert!(!test.fs().exists(&other));
        assert!(test.fs().exists(&bad));
    }

    #[test]
    fn check_references_reports_names_left_in_other_files() {
        let test = TestContext::new();
        let target = file(&test, "/p/helpers.rs", b"pub fn help() {}\n");
        file(&test, "/p/main.rs", b"mod helpers;\n");
        let call =
            args(&[&target]).with(CHECK_REFERENCES, ArgValue::Value(Value::Bool(true)), false);
        let run = run(&Remove, &test, &call, None).unwrap();
        assert_eq!(run.codes(), [Notable::ReferencesLeft]);
        assert_eq!(run.notable[0].count, Some(1));
    }

    #[test]
    fn a_file_held_open_fails_with_the_holders() {
        let test = TestContext::new();
        let path = file(&test, "/data/a.txt", b"a");
        let holders = vec![Holder {
            pid: 9,
            name: "office".to_owned(),
        }];
        test.fs().set_holders(&path, holders.clone());
        test.fs()
            .inject(Fault::in_use(&path, "remove_file", holders));
        let error = run(&Remove, &test, &args(&[&path]), None).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InUse);
        assert!(error.details.contains_key("holders"));
        assert!(test.fs().exists(&path));
    }

    #[test]
    fn removes_ten_thousand_entries_with_one_target_each() {
        let test = TestContext::new();
        let total = 10_000;
        dir(&test, "/many");
        let mut records = Vec::with_capacity(total);
        for index in 0..total {
            let path = abs(&format!("/many/f{index}"));
            test.fs().write_file(&path, b"x").unwrap();
            records.push(entry(&path));
        }
        let run = run(&Remove, &test, &BoundArgs::new(COMMAND), Some(records)).unwrap();
        assert_eq!(run.changes.len(), total);
        assert_eq!(run.summary("removed"), Some(i64::try_from(total).unwrap()));
        assert!(!test.fs().exists(&abs("/many/f9999")));
        assert_journaled(&test);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
