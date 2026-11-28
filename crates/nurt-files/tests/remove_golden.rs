// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/remove_golden.rs
// Responsibility:   Examples of `docs/commands/remove.md`, run on the virtual file system.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Examples of `docs/commands/remove.md`, run on the virtual file system.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
mod support_report;

use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Follow, Fs};
use nurt_contracts::journal::JournalStore;
use nurt_contracts::record::{ChangeAction, Effect};
use nurt_contracts::stage::{ArgValue, BoundArgs, RecordStream, StageData, StageImpl, StageInput};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_files::common::etag::etag_text;
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::test_path;

fn stage(command: &str) -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == command)
        .expect("the command is registered")
}

fn abs(text: &str) -> AbsPath {
    test_path(text).expect("test paths are absolute")
}

fn write(test: &TestContext, path: &str, content: &[u8]) -> AbsPath {
    let path = abs(path);
    test.fs()
        .create_dir_all(&path.parent().expect("files have a parent"))
        .expect("parent created");
    test.fs().write_file(&path, content).expect("file written");
    path
}

fn paths(paths: &[&AbsPath]) -> ArgValue {
    ArgValue::Value(Value::List(
        paths
            .iter()
            .map(|path| Value::Path(path.to_language_string()))
            .collect(),
    ))
}

fn etag(test: &TestContext, path: &AbsPath) -> Value {
    let meta = test
        .fs()
        .metadata(path, Follow::NoFollow)
        .expect("the entry exists");
    Value::Text(etag_text(&meta.etag()))
}

fn entry(test: &TestContext, path: &AbsPath) -> Record {
    Record::new(RecordKind::Entry)
        .with("path", Value::Path(path.to_language_string()))
        .with("etag", etag(test, path))
}

/// `line` record as `lines` produces it.
fn line(test: &TestContext, path: &AbsPath, no: i64, text: &str) -> Record {
    Record::new(RecordKind::Line)
        .with("path", Value::Path(path.to_language_string()))
        .with("no", Value::Int(no))
        .with("text", Value::Text(text.to_owned()))
        .with("etag", etag(test, path))
}

/// Plans, applies and commits the stage; returns the `effect` record.
fn execute(test: &TestContext, args: &BoundArgs, input: Option<Vec<Record>>) -> Result<Effect> {
    let stage = stage(args.command());
    let effect = stage.as_effect().expect("an effect stage");
    let ctx = test.ctx();
    let input = input.map_or_else(StageInput::none, |records| {
        StageInput::records(RecordStream::from_records(records))
    });
    let plan = effect.plan(args, input, &ctx)?;
    support_report::assert_plan_valid(args.command(), &plan.changes);
    let mut tx = test.journal().begin(begin_request(args.command()));
    let outcome = effect.apply(args, plan, tx.as_mut(), &ctx)?;
    support_report::assert_changes_valid(args.command(), &outcome.changes);
    let StageData::Records(stream) = outcome.output.data else {
        panic!("the stage returns the effect record");
    };
    let records = stream.collect::<Result<Vec<_>>>()?;
    tx.commit()?;
    let record = records.into_iter().next().expect("one effect record");
    let effect = Effect::try_from(record)?;
    support_report::assert_effect_valid(&effect);
    Ok(effect)
}

/// Every declared target was changed through the transaction.
fn assert_journaled(test: &TestContext) {
    let calls = test.journal().calls();
    let declared: usize = calls
        .iter()
        .map(|call| match call {
            JournalCall::Preserve { targets, .. } => targets.len(),
            _ => 0,
        })
        .sum();
    let applied: BTreeSet<u32> = calls
        .iter()
        .filter_map(|call| match call {
            JournalCall::Apply {
                index, ok: true, ..
            } => Some(index.0),
            _ => None,
        })
        .collect();
    assert!(declared > 0);
    assert_eq!(applied.len(), declared);
}

/// Nothing was declared or changed.
fn assert_untouched(test: &TestContext) {
    let changed = test.journal().calls().iter().any(|call| {
        matches!(
            call,
            JournalCall::Preserve { .. } | JournalCall::Apply { ok: true, .. }
        )
    });
    assert!(!changed, "the journal recorded a change");
}

fn counter(effect: &Effect, name: &str) -> Option<u64> {
    effect.summary.as_ref()?.get(name).copied()
}

/// `remove "/home/user/build.log"`
#[test]
fn example_1_removes_a_file() {
    let test = TestContext::new();
    let log = write(&test, "/home/user/build.log", b"ok\n");
    let args = BoundArgs::new("remove").with("path", paths(&[&log]), false);
    let effect = execute(&test, &args, None).unwrap();
    assert!(!test.fs().exists(&log));
    assert_eq!(effect.kind, "remove");
    assert_eq!(effect.targets, Some(1));
    assert_eq!(
        effect.changes.clone().unwrap()[0].action,
        ChangeAction::Delete
    );
    assert_eq!(counter(&effect, "files"), Some(1));
    assert_eq!(counter(&effect, "bytes"), Some(3));
    assert_journaled(&test);
}

/// `remove "/home/user/app/cache" "/home/user/app/old.log"`
#[test]
fn example_2_removes_a_tree_as_one_target_and_skips_a_missing_path() {
    let test = TestContext::new();
    write(&test, "/home/user/app/cache/a.bin", b"1234");
    write(&test, "/home/user/app/cache/sub/b.bin", b"56");
    let cache = abs("/home/user/app/cache");
    let missing = abs("/home/user/app/old.log");
    let args = BoundArgs::new("remove").with("path", paths(&[&cache, &missing]), false);
    let effect = execute(&test, &args, None).unwrap();
    assert!(!test.fs().exists(&cache));
    assert_eq!(effect.targets, Some(1));
    let changes = effect.changes.clone().unwrap();
    assert_eq!(
        changes[0].flags,
        Some(
            [
                "recursive",
                "directory",
                "files_inside=2",
                "directories_inside=1"
            ]
            .map(str::to_owned)
            .to_vec()
        )
    );
    assert_eq!(counter(&effect, "directories"), Some(2));
    assert_eq!(counter(&effect, "files"), Some(2));
    assert_eq!(counter(&effect, "bytes"), Some(6));
    assert_eq!(counter(&effect, "missing"), Some(1));
    assert_journaled(&test);
}

/// `lines "/home/user/notes.txt" 2..3 |> remove`
#[test]
fn example_3_removes_lines_from_a_file() {
    let test = TestContext::new();
    let notes = write(
        &test,
        "/home/user/notes.txt",
        b"keep\ndraft 1\ndraft 2\nend\n",
    );
    let records = vec![
        line(&test, &notes, 2, "draft 1"),
        line(&test, &notes, 3, "draft 2"),
    ];
    let effect = execute(&test, &BoundArgs::new("remove"), Some(records)).unwrap();
    assert_eq!(test.fs().read_file(&notes).unwrap(), b"keep\nend\n");
    let changes = effect.changes.clone().unwrap();
    assert_eq!(changes[0].action, ChangeAction::Modify);
    assert_eq!(changes[0].removed, Some(2));
    assert_eq!(counter(&effect, "lines"), Some(2));
    assert_journaled(&test);
}

#[test]
fn a_link_is_removed_and_its_target_stays() {
    let test = TestContext::new();
    let target = write(&test, "/home/user/releases/v2/app", b"bin");
    let link = abs("/home/user/current");
    test.fs().symlink("releases/v2", &link).unwrap();
    let args = BoundArgs::new("remove").with("path", paths(&[&link]), false);
    let effect = execute(&test, &args, None).unwrap();
    assert!(!test.fs().exists(&link));
    assert!(test.fs().exists(&target));
    assert_eq!(counter(&effect, "links"), Some(1));
    assert_journaled(&test);
}

#[test]
fn on_missing_error_fails_with_not_found_and_changes_nothing() {
    let test = TestContext::new();
    let present = write(&test, "/home/user/a.txt", b"a");
    let missing = abs("/home/user/none.txt");
    let args = BoundArgs::new("remove")
        .with("path", paths(&[&present, &missing]), false)
        .with(
            "on_missing",
            ArgValue::Value(Value::Enum("error".to_owned())),
            false,
        );
    let error = execute(&test, &args, None).unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert!(test.fs().exists(&present));
    assert_untouched(&test);
}

#[test]
fn entries_changed_since_they_were_read_are_stale() {
    let test = TestContext::new();
    let path = write(&test, "/home/user/report.txt", b"v1");
    let record = entry(&test, &path);
    test.fs().write_file(&path, b"version 2").unwrap();
    let error = execute(&test, &BoundArgs::new("remove"), Some(vec![record])).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(test.fs().read_file(&path).unwrap(), b"version 2");
    assert_untouched(&test);
}

/// The change of a removed tree names the kind of the target and counts the
/// files and directories inside, in the plan and in the applied change.
#[test]
fn removed_tree_reports_the_entries_inside_in_plan_and_result() {
    let test = TestContext::new();
    write(&test, "/home/user/app/cache/a.bin", b"1234");
    write(&test, "/home/user/app/cache/sub/b.bin", b"56");
    write(&test, "/home/user/app/cache/sub/deep/c.bin", b"7");
    let cache = abs("/home/user/app/cache");
    let args = BoundArgs::new("remove").with("path", paths(&[&cache]), false);
    let stage = stage("remove");
    let effect_stage = stage.as_effect().expect("an effect stage");
    let ctx = test.ctx();
    let plan = effect_stage
        .plan(&args, StageInput::none(), &ctx)
        .expect("planned");
    let planned = plan.changes[0].clone();
    assert_eq!(planned.size_before.map(|size| size.bytes()), Some(7));
    let flags = planned.flags.clone().expect("notes of a directory");
    assert_eq!(nurt_contracts::report::tree_counts(&flags), Some((3, 2)));
    assert!(
        flags
            .iter()
            .any(|flag| flag == nurt_contracts::report::FLAG_DIRECTORY)
    );

    let applied = execute(&test, &args, None).unwrap();
    let change = applied.changes.clone().unwrap().remove(0);
    assert_eq!(change.size_before.map(|size| size.bytes()), Some(7));
    assert_eq!(change.flags, planned.flags);
    assert_eq!(counter(&applied, "files"), Some(3));
    assert_eq!(counter(&applied, "directories"), Some(3));
    assert_eq!(counter(&applied, "bytes"), Some(7));
}

/// An empty directory is a directory in the plan too, and holds nothing.
#[test]
fn removed_empty_directory_is_marked_as_a_directory() {
    let test = TestContext::new();
    let empty = abs("/home/user/app/empty");
    test.fs().create_dir_all(&empty).expect("directory created");
    let args = BoundArgs::new("remove").with("path", paths(&[&empty]), false);
    let stage = stage("remove");
    let ctx = test.ctx();
    let plan = stage
        .as_effect()
        .expect("an effect stage")
        .plan(&args, StageInput::none(), &ctx)
        .expect("planned");
    let flags = plan.changes[0].flags.clone().expect("notes of a directory");
    assert_eq!(flags, nurt_contracts::report::tree_flags(0, 0));
    assert_eq!(
        plan.changes[0].size_before.map(|size| size.bytes()),
        Some(0)
    );
}

/// A file carries no notes of a directory.
#[test]
fn removed_file_has_no_directory_notes() {
    let test = TestContext::new();
    let file = write(&test, "/home/user/app/plain.txt", b"abc");
    let args = BoundArgs::new("remove").with("path", paths(&[&file]), false);
    let stage = stage("remove");
    let ctx = test.ctx();
    let plan = stage
        .as_effect()
        .expect("an effect stage")
        .plan(&args, StageInput::none(), &ctx)
        .expect("planned");
    assert_eq!(plan.changes[0].flags, None);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
