// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of `docs/commands/move.md`, run on the virtual file system.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_report;

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::JournalStore;
use nurt_contracts::record::{ChangeAction, Effect};
use nurt_contracts::stage::{ArgValue, BoundArgs, StageData, StageImpl, StageInput};
use nurt_contracts::value::Value;
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::{VolumeConfig, test_path};

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

fn path(path: &AbsPath) -> ArgValue {
    ArgValue::Value(Value::Path(path.to_language_string()))
}

/// Plans, applies and commits the source form; returns the `effect` record.
fn execute(test: &TestContext, args: &BoundArgs) -> Result<Effect> {
    let stage = stage(args.command());
    let effect = stage.as_effect().expect("an effect stage");
    let ctx = test.ctx();
    let plan = effect.plan(args, StageInput::none(), &ctx)?;
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
    let applied = calls
        .iter()
        .filter(|call| matches!(call, JournalCall::Apply { ok: true, .. }))
        .count();
    assert!(declared > 0);
    assert_eq!(applied, declared);
}

fn counter(effect: &Effect, name: &str) -> Option<u64> {
    effect.summary.as_ref()?.get(name).copied()
}

/// `move "/home/user/report.txt" "/home/user/done"`
#[test]
fn example_1_moves_a_file_into_an_existing_directory() {
    let test = TestContext::new();
    let source = write(&test, "/home/user/report.txt", b"q3");
    test.fs().create_dir_all(&abs("/home/user/done")).unwrap();
    let args = BoundArgs::new("move")
        .with("from", paths(&[&source]), false)
        .with("to", path(&abs("/home/user/done")), false);
    let effect = execute(&test, &args).unwrap();
    assert!(test.fs().exists(&abs("/home/user/done/report.txt")));
    assert!(!test.fs().exists(&source));
    assert_eq!(effect.notable, Some(vec!["to_is_dir".to_owned()]));
    assert_eq!(
        effect.changes.clone().unwrap()[0].action,
        ChangeAction::Rename
    );
    assert_journaled(&test);
}

/// `move "/srv/a.log" "/srv/b.log" into: "/srv/old"`
#[test]
fn example_2_moves_several_files_into_a_created_directory() {
    let test = TestContext::new();
    let first = write(&test, "/srv/a.log", b"a");
    let second = write(&test, "/srv/b.log", b"b");
    let args = BoundArgs::new("move")
        .with("from", paths(&[&first, &second]), false)
        .with("into", path(&abs("/srv/old")), false);
    let effect = execute(&test, &args).unwrap();
    assert!(test.fs().exists(&abs("/srv/old/a.log")));
    assert!(test.fs().exists(&abs("/srv/old/b.log")));
    assert_eq!(counter(&effect, "moved"), Some(2));
    assert_eq!(effect.targets, Some(3));
    assert_journaled(&test);
}

/// `move "/data/export.csv" "/mnt/usb/export.csv"`
#[test]
fn example_3_moves_a_file_to_another_volume() {
    let test = TestContext::new();
    test.fs()
        .add_volume(&abs("/mnt/usb"), VolumeConfig::default())
        .unwrap();
    let source = write(&test, "/data/export.csv", b"id,name\n");
    let target = abs("/mnt/usb/export.csv");
    let args = BoundArgs::new("move")
        .with("from", paths(&[&source]), false)
        .with("to", path(&target), false);
    let effect = execute(&test, &args).unwrap();
    assert_eq!(test.fs().read_file(&target).unwrap(), b"id,name\n");
    assert!(!test.fs().exists(&source));
    assert_eq!(effect.notable, Some(vec!["cross_device".to_owned()]));
    assert_eq!(counter(&effect, "cross_device"), Some(1));
    assert_journaled(&test);
}
