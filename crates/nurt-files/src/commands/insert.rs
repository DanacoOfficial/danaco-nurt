// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::commands::insert
// Responsibility:   The `insert` command: insertion of lines before or after lines and matches, or at the start or end of files.
// Dependencies:     nurt_contracts, nurt_testkit, nurt_text
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `insert` command: insertion of lines before or after lines and
//! matches, or at the start or end of files.
//!
//! | Form | Input | Place |
//! |---|---|---|
//! | source | none | `line` of the file `in` with `position`; without `line`, the start (`before`, `start`) or the end (`after`, `end`) of the file |
//! | stage | `line`, `match` | the line of every record with `position`; `start` and `end` once per file |
//! | stage | `entry` | the start (`before`, `start`) or the end (`after`, `end`) of every file |
//!
//! All insertions into one file are planned against the file as read, so
//! the line numbers of the input do not move because of earlier insertions.
//! The inserted text takes the line endings of the file. Text inserted at
//! the end of a file without a final line break does not add one unless
//! `ensure_newline` is set. A line outside the file fails with
//! `E_OUT_OF_RANGE` before anything is written.

use std::collections::HashMap;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StaticInfo,
};
use nurt_contracts::value::{RecordKind, Value};
use nurt_text::edit_plan::{self, EditOptions, InsertChange, Position};

use super::edit::{
    FileEdit, LINE, apply_edits, check_record_etag, edit_error, effect_plan, missing_param,
    position_param, read_checked, type_error,
};
use super::validate;
use crate::common::params;
use crate::common::path::{path_arg_for_change, record_path};

const COMMAND: &str = "insert";
const TEXT: &str = "text";
const IN: &str = "in";
const ENSURE_NEWLINE: &str = "ensure_newline";

/// The `insert` command.
pub(crate) struct Insert;

/// Stage implementations of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Insert)]
}

impl StageImpl for Insert {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Line, RecordKind::Match, RecordKind::Entry]),
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        text_param(args)?;
        position_param(args)?;
        let mut info = StaticInfo::default();
        if let Some(path) = args.value(IN).and_then(Value::as_str) {
            info.paths.push(path.to_owned());
        }
        Ok(info)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Insert {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let text = text_param(args)?;
        let position = position_param(args)?;
        let options = EditOptions {
            ensure_newline: args.bool(ENSURE_NEWLINE)?.unwrap_or(false),
            ..EditOptions::default()
        };
        let validate = validate::validate_param(args)?;
        let if_match = params::if_match_param(args, ctx)?;
        let targets = if input.is_none() {
            vec![source_target(args, ctx, position)?]
        } else {
            record_targets(input.into_records()?, ctx, position)?
        };

        let mut edits = Vec::with_capacity(targets.len());
        for target in targets {
            ctx.checkpoint()?;
            let file = read_checked(ctx, &target.path, if_match.as_ref())?;
            if let Some(etag) = &target.etag {
                check_record_etag(&file, etag)?;
            }
            let changes: Vec<edit_plan::Change> = target
                .places
                .iter()
                .map(|(line, position)| {
                    edit_plan::Change::Insert(InsertChange {
                        text: text.to_owned(),
                        line: *line,
                        position: *position,
                    })
                })
                .collect();
            let planned = edit_plan::plan(&file.text, &changes, &options)
                .map_err(|error| edit_error(&error, &target.path))?;
            edits.push(FileEdit {
                file,
                text: planned.text,
                replacements: None,
            });
        }
        let notable = if edits.is_empty() {
            vec![NotableEntry::new(Notable::NothingMatched)]
        } else {
            Vec::new()
        };
        effect_plan(ctx, COMMAND, validate, edits, notable)
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        apply_edits(plan, tx, ctx)
    }
}

fn text_param(args: &BoundArgs) -> Result<&str> {
    match args.require(TEXT)? {
        Value::Text(text) => Ok(text),
        _ => Err(type_error(COMMAND, TEXT, "a text")),
    }
}

/// Insertion into one file: the places, each a line and a position, and the
/// entity tag of the input records.
#[derive(Debug)]
struct Target {
    path: AbsPath,
    etag: Option<String>,
    places: Vec<(Option<i64>, Position)>,
}

impl Target {
    fn add(&mut self, place: (Option<i64>, Position)) {
        if !self.places.contains(&place) {
            self.places.push(place);
        }
    }
}

/// Position at the start or end of a file for a place without a line.
fn file_position(position: Position) -> Position {
    match position {
        Position::Before | Position::Start => Position::Start,
        Position::After | Position::End => Position::End,
    }
}

fn source_target(args: &BoundArgs, ctx: &ExecCtx, position: Position) -> Result<Target> {
    let path = path_arg_for_change(args, IN, ctx)?.ok_or_else(|| missing_param(COMMAND, IN))?;
    let line = args.int(LINE)?;
    let place = match (line, position) {
        (Some(line), Position::Before | Position::After) => (Some(line), position),
        _ => (None, file_position(position)),
    };
    Ok(Target {
        path,
        etag: None,
        places: vec![place],
    })
}

/// Files of the stage form in the order of their first record, each with
/// its places in the order of the records.
fn record_targets(
    stream: RecordStream<'_>,
    ctx: &ExecCtx,
    position: Position,
) -> Result<Vec<Target>> {
    let mut targets: Vec<Target> = Vec::new();
    let mut index: HashMap<AbsPath, usize> = HashMap::new();
    for item in stream {
        let record = item?;
        ctx.checkpoint()?;
        let path = record_path(&record, ctx)?;
        let (line, etag) = match record.kind {
            RecordKind::Line => {
                let line = record::Line::try_from(record)?;
                (Some(line.no), line.etag)
            }
            RecordKind::Match => {
                let found = record::Match::try_from(record)?;
                (Some(found.no), found.etag)
            }
            RecordKind::Entry => (None, String::new()),
            other => {
                return Err(Error::new(
                    ErrorCode::Type,
                    format!(
                        "`{COMMAND}` does not accept `{other}` records: pass `line`, `match` or `entry` records"
                    ),
                ));
            }
        };
        let place = match (line, position) {
            (Some(line), Position::Before | Position::After) => {
                (Some(i64::try_from(line).unwrap_or(i64::MAX)), position)
            }
            _ => (None, file_position(position)),
        };
        let slot = *index.entry(path.clone()).or_insert_with(|| {
            targets.push(Target {
                path,
                etag: None,
                places: Vec::new(),
            });
            targets.len() - 1
        });
        let target = &mut targets[slot];
        if !etag.is_empty() {
            target.etag.get_or_insert(etag);
        }
        target.add(place);
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::journal::JournalStore;
    use nurt_contracts::record::Change;
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::{self, Record};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;

    use super::*;
    use crate::commands::edit::fixture::{abs, with_file};
    use crate::common::etag::etag_text;

    fn args(text: &str, extra: &[(&str, Value)]) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND).with(
            TEXT,
            ArgValue::Value(Value::Text(text.to_owned())),
            false,
        );
        for (name, value) in extra {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    fn file_arg(path: &AbsPath) -> (&'static str, Value) {
        (IN, Value::Path(path.to_language_string()))
    }

    fn position(name: &str) -> (&'static str, Value) {
        (super::super::edit::POSITION, Value::Enum(name.to_owned()))
    }

    fn run(test: &TestContext, args: &BoundArgs, input: StageInput<'_>) -> Result<Vec<Change>> {
        let ctx = test.ctx();
        let plan = Insert.plan(args, input, &ctx)?;
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let outcome = Insert.apply(args, plan, tx.as_mut(), &ctx)?;
        tx.commit()?;
        Ok(outcome.changes)
    }

    #[test]
    fn source_form_inserts_before_and_after_a_line() {
        let (test, path) = with_file("/data/a.txt", b"one\ntwo\n");
        run(
            &test,
            &args(
                "zero",
                &[file_arg(&path), (LINE, Value::Int(1)), position("before")],
            ),
            StageInput::none(),
        )
        .unwrap();
        run(
            &test,
            &args("three", &[file_arg(&path), (LINE, Value::Int(3))]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(
            test.fs().read_file(&path).unwrap(),
            b"zero\none\ntwo\nthree\n"
        );
    }

    #[test]
    fn source_form_without_a_line_uses_the_start_or_end() {
        let (test, path) = with_file("/data/a.txt", b"body");
        run(
            &test,
            &args("head", &[file_arg(&path), position("before")]),
            StageInput::none(),
        )
        .unwrap();
        run(&test, &args("tail", &[file_arg(&path)]), StageInput::none()).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"head\nbody\ntail");
        run(
            &test,
            &args(
                "end",
                &[
                    file_arg(&path),
                    position("end"),
                    (ENSURE_NEWLINE, Value::Bool(true)),
                ],
            ),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(
            test.fs().read_file(&path).unwrap(),
            b"head\nbody\ntail\nend\n"
        );
    }

    #[test]
    fn crlf_files_receive_crlf_lines() {
        let (test, path) = with_file("/data/a.txt", b"a\r\nb\r\n");
        let changes = run(
            &test,
            &args("x\ny", &[file_arg(&path), (LINE, Value::Int(1))]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"a\r\nx\r\ny\r\nb\r\n");
        assert_eq!(changes[0].added, Some(2));
        assert_eq!(changes[0].replacements, None);
    }

    #[test]
    fn a_line_outside_the_file_changes_nothing() {
        let (test, path) = with_file("/data/a.txt", b"one\n");
        let error = run(
            &test,
            &args("x", &[file_arg(&path), (LINE, Value::Int(5))]),
            StageInput::none(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::OutOfRange);
        assert_eq!(error.actual, Some(serde_json::json!(5)));
        assert_eq!(test.fs().read_file(&path).unwrap(), b"one\n");
    }

    fn line(path: &AbsPath, no: u64, etag: &str) -> Record {
        Record::from(record::Line {
            path: value::Path::new(path.to_language_string()),
            no,
            etag: etag.to_owned(),
            ..record::Line::default()
        })
    }

    #[test]
    fn line_records_insert_at_every_line_of_the_text_read() {
        let (test, path) = with_file("/data/a.txt", b"a\nb\nc\n");
        let records = vec![line(&path, 1, ""), line(&path, 3, ""), line(&path, 3, "")];
        run(
            &test,
            &args("--", &[]),
            StageInput::records(RecordStream::from_records(records)),
        )
        .unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"a\n--\nb\nc\n--\n");
    }

    #[test]
    fn match_and_entry_records_are_accepted() {
        let (test, path) = with_file("/data/a.txt", b"x\ny\n");
        let found = Record::from(record::Match {
            path: value::Path::new(path.to_language_string()),
            no: 2,
            column: 1,
            text: "y".to_owned(),
            ..record::Match::default()
        });
        run(
            &test,
            &args("before y", &[position("before")]),
            StageInput::records(RecordStream::once(found)),
        )
        .unwrap();
        let entry =
            Record::new(RecordKind::Entry).with("path", Value::Path(path.to_language_string()));
        run(
            &test,
            &args("first", &[position("before")]),
            StageInput::records(RecordStream::once(entry)),
        )
        .unwrap();
        assert_eq!(
            test.fs().read_file(&path).unwrap(),
            b"first\nx\nbefore y\ny\n"
        );
    }

    #[test]
    fn start_and_end_with_line_records_apply_once_per_file() {
        let (test, path) = with_file("/data/a.txt", b"a\nb\n");
        let records = vec![line(&path, 1, ""), line(&path, 2, "")];
        run(
            &test,
            &args("end", &[position("end")]),
            StageInput::records(RecordStream::from_records(records)),
        )
        .unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"a\nb\nend\n");
    }

    #[test]
    fn stale_records_are_rejected() {
        let (test, path) = with_file("/data/a.txt", b"a\n");
        let ctx = test.ctx();
        let file = read_checked(&ctx, &path, None).unwrap();
        let mut tag = file.etag();
        tag.size += 1;
        let error = Insert
            .plan(
                &args("x", &[]),
                StageInput::records(RecordStream::once(line(&path, 1, &etag_text(&tag)))),
                &ctx,
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
    }

    #[test]
    fn empty_input_and_other_records() {
        let test = TestContext::new();
        let ctx = test.ctx();
        let plan = Insert
            .plan(
                &args("x", &[]),
                StageInput::records(RecordStream::empty()),
                &ctx,
            )
            .unwrap();
        assert!(plan.changes.is_empty());
        assert_eq!(plan.notable[0].code, Notable::NothingMatched);
        let row =
            Record::new(RecordKind::Row).with("path", Value::Path(abs("/a").to_language_string()));
        let error = Insert
            .plan(
                &args("x", &[]),
                StageInput::records(RecordStream::once(row)),
                &ctx,
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Type);
        let missing = Insert
            .plan(&args("x", &[]), StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(missing.code(), ErrorCode::MissingParam);
        let wrong = BoundArgs::new(COMMAND).with(TEXT, ArgValue::Value(Value::Int(1)), false);
        assert_eq!(text_param(&wrong).unwrap_err().code(), ErrorCode::Type);
    }

    #[test]
    fn insertion_keeps_the_byte_order_mark_and_multibyte_text() {
        let (test, path) = with_file(
            "/data/a.txt",
            "\u{feff}za\u{17c}\u{f3}\u{142}\u{107}\r\nx\r\n".as_bytes(),
        );
        run(
            &test,
            &args("gl\u{105}", &[file_arg(&path), (LINE, Value::Int(1))]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(
            test.fs().read_file(&path).unwrap(),
            "\u{feff}za\u{17c}\u{f3}\u{142}\u{107}\r\ngl\u{105}\r\nx\r\n".as_bytes()
        );
    }

    #[test]
    fn a_line_below_the_first_fails_with_out_of_range() {
        let (test, path) = with_file("/data/a.txt", b"one\n");
        let error = run(
            &test,
            &args(
                "x",
                &[file_arg(&path), (LINE, Value::Int(0)), position("before")],
            ),
            StageInput::none(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::OutOfRange);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"one\n");
    }

    #[test]
    fn binary_files_are_not_changed() {
        let (test, path) = with_file("/data/a.bin", b"\x7fELF\x00\x01\x02\x00");
        let error = run(
            &test,
            &args("x", &[file_arg(&path), position("end")]),
            StageInput::none(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Binary);
        assert_eq!(
            test.fs().read_file(&path).unwrap(),
            b"\x7fELF\x00\x01\x02\x00"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
