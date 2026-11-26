// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of `docs/commands/create.md`, run on the virtual file system.

// Test code unwraps results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used)]

mod support_report;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::env::Clock;
use nurt_contracts::error::Result;
use nurt_contracts::fs::{AbsPath, FileKind, Follow, Fs};
use nurt_contracts::journal::JournalStore;
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::stage::{ArgValue, BoundArgs, StageImpl, StageInput};
use nurt_contracts::value::Value;
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::test_path;

const COMMAND: &str = "create";

fn abs(path: &str) -> AbsPath {
    test_path(path).unwrap()
}

fn set(args: BoundArgs, name: &str, value: Value) -> BoundArgs {
    args.with(name, ArgValue::Value(value), false)
}

fn create_args(path: &str, kind: &str) -> BoundArgs {
    let args = set(
        BoundArgs::new(COMMAND),
        "path",
        Value::Path(abs(path).to_language_string()),
    );
    set(args, "type", Value::Enum(kind.to_owned()))
}

fn stage() -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == COMMAND)
        .unwrap()
}

fn run(test: &TestContext, args: &BoundArgs) -> Result<Vec<Change>> {
    let stage = stage();
    let effect = stage.as_effect().unwrap();
    let ctx = test.ctx();
    let plan = effect.plan(args, StageInput::none(), &ctx)?;
    support_report::assert_plan_valid(args.command(), &plan.changes);
    let mut tx = test.journal().begin(begin_request(COMMAND));
    let changes = effect.apply(args, plan, tx.as_mut(), &ctx)?.changes;
    support_report::assert_changes_valid(args.command(), &changes);
    tx.commit()?;
    Ok(changes)
}

fn kind_of(test: &TestContext, path: &str) -> FileKind {
    test.fs()
        .metadata(&abs(path), Follow::NoFollow)
        .unwrap()
        .kind
}

fn assert_every_change_declared(test: &TestContext) {
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
    assert_eq!(declared, applied);
}

#[test]
fn creates_a_directory_with_its_parents() {
    let test = TestContext::new();
    test.fs().create_dir_all(&abs("/home/user/app")).unwrap();
    let changes = run(
        &test,
        &create_args("/home/user/app/build/cache", "directory"),
    )
    .unwrap();
    assert_eq!(changes.len(), 2);
    assert!(
        changes
            .iter()
            .all(|change| change.action == ChangeAction::Create)
    );
    assert_eq!(
        kind_of(&test, "/home/user/app/build/cache"),
        FileKind::Directory
    );
    assert_every_change_declared(&test);
}

#[test]
fn touches_an_existing_file() {
    let test = TestContext::new();
    let notes = abs("/home/user/notes.txt");
    test.fs().create_dir_all(&abs("/home/user")).unwrap();
    test.fs().write_file(&notes, b"keep\n").unwrap();
    test.clock().advance(Duration::from_secs(3600));
    let args = set(
        create_args("/home/user/notes.txt", "file"),
        "on_exists",
        Value::Enum("touch".to_owned()),
    );
    let changes = run(&test, &args).unwrap();
    assert_eq!(changes[0].action, ChangeAction::Attributes);
    let meta = test.fs().metadata(&notes, Follow::NoFollow).unwrap();
    assert_eq!(
        i128::from(meta.modified_ns),
        test.clock().now().unix_nanos()
    );
    assert_eq!(test.fs().read_file(&notes).unwrap(), b"keep\n");
    assert_every_change_declared(&test);
}

#[test]
fn creates_a_symbolic_link_to_a_release() {
    let test = TestContext::new();
    test.fs()
        .create_dir_all(&abs("/home/user/app/releases/v2"))
        .unwrap();
    let args = set(
        create_args("/home/user/app/current", "link"),
        "target",
        Value::Text("releases/v2".to_owned()),
    );
    let changes = run(&test, &args).unwrap();
    assert_eq!(changes[0].action, ChangeAction::Link);
    assert_eq!(kind_of(&test, "/home/user/app/current"), FileKind::Symlink);
    assert_eq!(
        test.fs().read_link(&abs("/home/user/app/current")).unwrap(),
        PathBuf::from("releases/v2")
    );
    assert_every_change_declared(&test);
}

#[test]
fn creates_a_temporary_file() {
    let test = TestContext::new();
    let args = set(
        create_args("/home/user/tmp", "file"),
        "temporary",
        Value::Bool(true),
    );
    let changes = run(&test, &args).unwrap();
    let created = changes.last().unwrap();
    let prefix = format!("{}/nurt-", abs("/home/user/tmp").to_language_string());
    assert!(created.path.as_str().starts_with(&prefix));
    let path = AbsPath::new(created.path.as_str()).unwrap();
    assert_eq!(test.fs().read_file(&path).unwrap(), b"");
    assert_every_change_declared(&test);
}
