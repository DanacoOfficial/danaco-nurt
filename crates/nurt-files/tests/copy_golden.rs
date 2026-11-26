// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of `docs/commands/copy.md`, run on the virtual file system.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_report;

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::JournalStore;
use nurt_contracts::record::{ChangeAction, Effect};
use nurt_contracts::stage::{ArgValue, BoundArgs, RecordStream, StageData, StageImpl, StageInput};
use nurt_contracts::value::{Record, RecordKind, Value};
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

fn path(path: &AbsPath) -> ArgValue {
    ArgValue::Value(Value::Path(path.to_language_string()))
}

fn entry(path: &AbsPath) -> Record {
    Record::new(RecordKind::Entry).with("path", Value::Path(path.to_language_string()))
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
    let applied: std::collections::BTreeSet<u32> = calls
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

fn counter(effect: &Effect, name: &str) -> Option<u64> {
    effect.summary.as_ref()?.get(name).copied()
}

/// `copy "/srv/app/config.toml" "/srv/app/config.toml.bak"`
#[test]
fn example_1_copies_a_file_next_to_itself() {
    let test = TestContext::new();
    let source = write(&test, "/srv/app/config.toml", b"port = 8080\n");
    let backup = abs("/srv/app/config.toml.bak");
    let args = BoundArgs::new("copy")
        .with("from", path(&source), false)
        .with("to", path(&backup), false);
    let effect = execute(&test, &args, None).unwrap();
    assert_eq!(test.fs().read_file(&backup).unwrap(), b"port = 8080\n");
    assert_eq!(effect.kind, "copy");
    assert_eq!(effect.targets, Some(1));
    let changes = effect.changes.clone().unwrap();
    assert_eq!(changes[0].action, ChangeAction::Create);
    assert_eq!(counter(&effect, "files"), Some(1));
    assert_journaled(&test);
}

/// `copy "/srv/app/src" into: "/backup/2026-11-22"`
#[test]
fn example_2_copies_a_tree_into_a_new_directory() {
    let test = TestContext::new();
    write(&test, "/srv/app/src/main.rs", b"fn main() {}\n");
    write(&test, "/srv/app/src/lib/mod.rs", b"pub mod a;\n");
    test.fs().create_dir_all(&abs("/backup")).unwrap();
    let args = BoundArgs::new("copy")
        .with("from", path(&abs("/srv/app/src")), false)
        .with("into", path(&abs("/backup/2026-11-22")), false);
    let effect = execute(&test, &args, None).unwrap();
    assert!(test.fs().exists(&abs("/backup/2026-11-22/src/lib/mod.rs")));
    assert_eq!(counter(&effect, "files"), Some(2));
    assert_eq!(counter(&effect, "directories"), Some(2));
    assert_eq!(effect.targets, Some(5));
    assert_journaled(&test);
}

/// `find "/srv/app/logs" glob: "*.log" |> copy into: "/archive/logs" on_conflict: skip`
#[test]
fn example_3_copies_found_entries_and_keeps_existing_copies() {
    let test = TestContext::new();
    let first = write(&test, "/srv/app/logs/a.log", b"new a");
    let second = write(&test, "/srv/app/logs/b.log", b"new b");
    let kept = write(&test, "/archive/logs/a.log", b"old a");
    let args = BoundArgs::new("copy")
        .with("into", path(&abs("/archive/logs")), false)
        .with(
            "on_conflict",
            ArgValue::Value(Value::Enum("skip".to_owned())),
            false,
        );
    let effect = execute(&test, &args, Some(vec![entry(&first), entry(&second)])).unwrap();
    assert_eq!(test.fs().read_file(&kept).unwrap(), b"old a");
    assert_eq!(
        test.fs().read_file(&abs("/archive/logs/b.log")).unwrap(),
        b"new b"
    );
    assert_eq!(counter(&effect, "skipped"), Some(1));
    assert_eq!(counter(&effect, "files"), Some(1));
    assert_journaled(&test);
}
