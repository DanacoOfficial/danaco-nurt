// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of `docs/commands/attributes.md`, run on the virtual file
//! system, and the transaction rules of the command.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_report;

use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileMeta, Follow, Fs};
use nurt_contracts::journal::JournalStore;
use nurt_contracts::record::{Change, ChangeAction, Effect};
use nurt_contracts::stage::{ArgValue, BoundArgs, RecordStream, StageData, StageImpl, StageInput};
use nurt_contracts::value::{Instant, Record, RecordKind, Value};
use nurt_files::common::etag::etag_text;
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::test_path;

fn abs(text: &str) -> AbsPath {
    test_path(text).expect("example paths are absolute")
}

fn stage() -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == "attributes")
        .expect("`attributes` is registered")
}

/// File system of the examples: a script, a text file and a small site.
fn home() -> TestContext {
    // The goldens describe the permission bits of a Unix host, which a test
    // on Windows keeps by naming the host.
    let test = TestContext::with_host(nurt_testkit::host::TestHost::linux());
    for path in [
        "/home/user/run.sh",
        "/home/user/notes.txt",
        "/home/user/site/index.html",
        "/home/user/site/css/main.css",
    ] {
        test.fs().write_file(&abs(path), b"content\n").unwrap();
    }
    test
}

fn call(paths: &[&str], params: &[(&str, Value)]) -> BoundArgs {
    let list = paths
        .iter()
        .map(|path| Value::Path(abs(path).to_language_string()))
        .collect();
    let mut args = BoundArgs::new("attributes")
        .with("path", ArgValue::Value(Value::List(list)), false)
        .with("recursive", ArgValue::Value(Value::Bool(false)), true);
    for (name, value) in params {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// Plans and applies one call in a transaction of the recording journal.
fn run(
    test: &TestContext,
    args: &BoundArgs,
    input: StageInput<'_>,
) -> Result<(Vec<Change>, Effect)> {
    let stage = stage();
    let effect_stage = stage.as_effect().expect("`attributes` changes entries");
    let ctx = test.ctx();
    let plan = effect_stage.plan(args, input, &ctx)?;
    support_report::assert_plan_valid(args.command(), &plan.changes);
    let mut tx = test.journal().begin(begin_request("attributes"));
    let outcome = effect_stage.apply(args, plan, tx.as_mut(), &ctx)?;
    support_report::assert_changes_valid(args.command(), &outcome.changes);
    let StageData::Records(mut records) = outcome.output.data else {
        panic!("`attributes` returns an effect record");
    };
    let effect = Effect::try_from(records.next().unwrap()?).unwrap();
    let changes = outcome.changes;
    tx.commit()?;
    Ok((changes, effect))
}

fn meta(test: &TestContext, path: &str) -> FileMeta {
    test.fs().metadata(&abs(path), Follow::NoFollow).unwrap()
}

#[test]
fn makes_a_script_executable() {
    // The example program is `attributes "/home/user/run.sh" mode: "u+x"`.
    let test = home();
    let (changes, effect) = run(
        &test,
        &call(&["/home/user/run.sh"], &[("mode", text("u+x"))]),
        StageInput::none(),
    )
    .unwrap();
    assert_eq!(meta(&test, "/home/user/run.sh").mode, 0o744);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].action, ChangeAction::Attributes);
    assert_eq!(
        changes[0].path.as_str(),
        abs("/home/user/run.sh").to_language_string()
    );
    assert_eq!(effect.kind, "attributes");
    assert_eq!(effect.targets, Some(1));
}

#[test]
fn sets_permissions_and_the_modification_time() {
    // The example program is `attributes "/home/user/notes.txt" mode: "0600" modified: "2026-01-31T12:00:00Z"`.
    let test = home();
    let args = call(
        &["/home/user/notes.txt"],
        &[
            ("mode", text("0600")),
            ("modified", text("2026-01-31T12:00:00Z")),
        ],
    );
    run(&test, &args, StageInput::none()).unwrap();
    let after = meta(&test, "/home/user/notes.txt");
    let expected: Instant = "2026-01-31T12:00:00Z".parse().unwrap();
    assert_eq!(after.mode, 0o600);
    assert_eq!(after.modified_ns, expected.as_unix_nanos());
}

#[test]
fn removes_write_permission_of_a_tree() {
    // The example program is `attributes "/home/user/site" mode: "go-w" recursive: true`.
    let test = home();
    for path in ["/home/user/site", "/home/user/site/index.html"] {
        test.fs().set_mode(&abs(path), 0o777).unwrap();
    }
    let args = call(
        &["/home/user/site"],
        &[("mode", text("go-w")), ("recursive", Value::Bool(true))],
    );
    let (changes, effect) = run(&test, &args, StageInput::none()).unwrap();
    assert_eq!(meta(&test, "/home/user/site").mode, 0o755);
    assert_eq!(meta(&test, "/home/user/site/index.html").mode, 0o755);
    assert_eq!(meta(&test, "/home/user/site/css/main.css").mode, 0o644);
    assert_eq!(changes.len(), 2);
    assert_eq!(
        effect.notable,
        Some(vec![
            "attributes_changed".to_owned(),
            "unchanged".to_owned()
        ])
    );
}

#[test]
fn changes_input_entries_of_a_pipeline() {
    // The example program is `find "/home/user/site" type: file |> attributes readonly: true`.
    let test = home();
    let records: Vec<Record> = ["/home/user/site/index.html", "/home/user/site/css/main.css"]
        .iter()
        .map(|path| {
            let path = abs(path);
            let tag = etag_text(&test.fs().etag(&path, Follow::NoFollow).unwrap());
            Record::new(RecordKind::Entry)
                .with("path", Value::Path(path.to_language_string()))
                .with("etag", Value::Text(tag))
        })
        .collect();
    let args =
        BoundArgs::new("attributes").with("readonly", ArgValue::Value(Value::Bool(true)), false);
    let (changes, _) = run(
        &test,
        &args,
        StageInput::records(RecordStream::from_records(records)),
    )
    .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(meta(&test, "/home/user/site/index.html").mode, 0o444);
    assert_eq!(meta(&test, "/home/user/site/css/main.css").mode, 0o444);
}

#[test]
fn every_change_is_declared_before_it_is_applied() {
    let test = home();
    let args = call(
        &["/home/user/run.sh", "/home/user/notes.txt"],
        &[("mode", text("0700"))],
    );
    run(&test, &args, StageInput::none()).unwrap();
    let calls = test.journal().calls();
    let declared: Vec<_> = calls
        .iter()
        .filter_map(|call| match call {
            JournalCall::Preserve { targets, .. } => Some(targets.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let applied = calls
        .iter()
        .filter(|call| matches!(call, JournalCall::Apply { ok: true, .. }))
        .count();
    assert_eq!(declared.len(), 2);
    assert_eq!(applied, declared.len());
    let first_apply = calls
        .iter()
        .position(|call| matches!(call, JournalCall::Apply { .. }))
        .unwrap();
    let last_preserve = calls
        .iter()
        .rposition(|call| matches!(call, JournalCall::Preserve { .. }))
        .unwrap();
    assert!(last_preserve < first_apply);
}

#[test]
fn a_changed_input_entry_fails_with_stale_and_changes_nothing() {
    let test = home();
    let path = abs("/home/user/notes.txt");
    let tag = etag_text(&test.fs().etag(&path, Follow::NoFollow).unwrap());
    test.fs().write_file(&path, b"edited\n").unwrap();
    let record = Record::new(RecordKind::Entry)
        .with("path", Value::Path(path.to_language_string()))
        .with("etag", Value::Text(tag));
    let args = BoundArgs::new("attributes").with("mode", ArgValue::Value(text("0600")), false);
    let error = run(
        &test,
        &args,
        StageInput::records(RecordStream::once(record)),
    )
    .err()
    .unwrap();
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(meta(&test, "/home/user/notes.txt").mode, 0o644);
    assert!(
        !test
            .journal()
            .calls()
            .iter()
            .any(|call| matches!(call, JournalCall::Apply { .. }))
    );
}
