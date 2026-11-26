// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The changes of `move` and `rename` satisfy the rules of the report: the
//! golden tests of the commands validate their plans and outcomes with the
//! shared helper, and this file checks the helper itself and the previous
//! path of every rename on the virtual file system.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_report;

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::JournalStore;
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::report::{Report, ReportStatus, ReportViolation};
use nurt_contracts::stage::{ArgValue, BoundArgs, StageData, StageImpl, StageInput};
use nurt_contracts::value::{Size, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
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

fn path_value(path: &AbsPath) -> ArgValue {
    ArgValue::Value(Value::Path(path.to_language_string()))
}

/// Plans and applies a call; returns the changes of the plan and of the
/// outcome.
fn plan_and_apply(test: &TestContext, args: &BoundArgs) -> Result<(Vec<Change>, Vec<Change>)> {
    let stage = stage(args.command());
    let effect = stage.as_effect().expect("an effect stage");
    let ctx = test.ctx();
    let plan = effect.plan(args, StageInput::none(), &ctx)?;
    let planned = plan.changes.clone();
    let mut tx = test.journal().begin(begin_request(args.command()));
    let outcome = effect.apply(args, plan, tx.as_mut(), &ctx)?;
    let StageData::Records(stream) = outcome.output.data else {
        panic!("the stage returns the effect record");
    };
    stream.collect::<Result<Vec<_>>>()?;
    tx.commit()?;
    Ok((planned, outcome.changes))
}

#[test]
fn a_rename_without_the_previous_path_breaks_the_report_rules() {
    let mut report = Report::new(ReportStatus::Ok, "rename");
    report.op = Some(1);
    report.changes = vec![Change::new("/srv/a.txt", ChangeAction::Rename)];
    assert!(matches!(
        report.validate(),
        Err(ReportViolation::RenameWithoutFrom { .. })
    ));
    report.changes[0].from = Some("/srv/b.txt".into());
    assert_eq!(report.validate(), Ok(()));
}

#[test]
#[should_panic(expected = "breaks a report rule")]
fn the_helper_rejects_a_rename_without_the_previous_path() {
    let changes = [Change::new("/srv/a.txt", ChangeAction::Rename)];
    support_report::assert_changes_valid("rename", &changes);
}

#[test]
#[should_panic(expected = "breaks a report rule")]
fn the_helper_rejects_a_planned_rename_without_the_previous_path() {
    let changes = [Change::new("/srv/a.txt", ChangeAction::Rename)];
    support_report::assert_plan_valid("move", &changes);
}

#[test]
fn the_helper_accepts_changes_without_a_rename() {
    let changes = [
        Change::new("/srv/a.txt", ChangeAction::Modify),
        Change::new("/srv/b.txt", ChangeAction::Create),
    ];
    support_report::assert_plan_valid("write", &changes);
    support_report::assert_changes_valid("write", &changes);
    support_report::assert_changes_valid("write", &[]);
}

#[test]
fn a_rename_of_an_entry_gives_the_previous_path_in_plan_and_outcome() {
    let test = TestContext::new();
    let draft = write(&test, "/home/user/draft.md", b"# Notes\n");
    let args = BoundArgs::new("rename")
        .with("path", path_value(&draft), false)
        .with(
            "to",
            ArgValue::Value(Value::Text("final.md".to_owned())),
            false,
        );
    let (planned, applied) = plan_and_apply(&test, &args).unwrap();
    for changes in [&planned, &applied] {
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].action, ChangeAction::Rename);
        assert!(changes[0].from.is_some(), "a rename gives `from`");
        support_report::assert_changes_valid("rename", changes);
        // Whichever of `path`, `from` and `to` holds the old and the new
        // name, together they name both of them.
        let mentioned: Vec<&str> = [
            Some(changes[0].path.as_str()),
            changes[0].from.as_ref().map(|from| from.as_str()),
            changes[0].to.as_ref().map(|to| to.as_str()),
        ]
        .into_iter()
        .flatten()
        .collect();
        assert!(mentioned.contains(&draft.to_language_string().as_str()));
        assert!(
            mentioned.iter().any(|path| path.ends_with("/final.md")),
            "{mentioned:?}"
        );
    }
}

#[test]
fn every_moved_file_gives_the_previous_path_in_plan_and_outcome() {
    let test = TestContext::new();
    let first = write(&test, "/srv/a.log", b"a");
    let second = write(&test, "/srv/b.log", b"b");
    let args = BoundArgs::new("move")
        .with(
            "from",
            ArgValue::Value(Value::List(vec![
                Value::Path(first.to_language_string()),
                Value::Path(second.to_language_string()),
            ])),
            false,
        )
        .with("into", path_value(&abs("/srv/old")), false);
    let (planned, applied) = plan_and_apply(&test, &args).unwrap();
    for changes in [&planned, &applied] {
        let renames: Vec<&Change> = changes
            .iter()
            .filter(|change| change.action == ChangeAction::Rename)
            .collect();
        assert_eq!(renames.len(), 2);
        assert!(renames.iter().all(|change| change.from.is_some()));
        support_report::assert_changes_valid("move", changes);
    }
}

#[test]
fn a_move_to_another_name_gives_the_previous_path() {
    let test = TestContext::new();
    let source = write(&test, "/srv/old name.txt", b"x");
    let args = BoundArgs::new("move")
        .with(
            "from",
            ArgValue::Value(Value::List(vec![Value::Path(source.to_language_string())])),
            false,
        )
        .with("to", path_value(&abs("/srv/new name.txt")), false);
    let (planned, applied) = plan_and_apply(&test, &args).unwrap();
    support_report::assert_plan_valid("move", &planned);
    support_report::assert_changes_valid("move", &applied);
    assert!(applied.iter().all(|change| change.from.is_some()));
}

fn paths_value(paths: &[&AbsPath]) -> ArgValue {
    ArgValue::Value(Value::List(
        paths
            .iter()
            .map(|path| Value::Path(path.to_language_string()))
            .collect(),
    ))
}

#[test]
fn a_removed_file_gives_its_size_before_in_plan_and_outcome() {
    let test = TestContext::new();
    let log = write(&test, "/home/user/build.log", b"ok\n");
    let args = BoundArgs::new("remove").with("path", paths_value(&[&log]), false);
    let (planned, applied) = plan_and_apply(&test, &args).unwrap();
    for changes in [&planned, &applied] {
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].action, ChangeAction::Delete);
        assert_eq!(
            changes[0].size_before,
            Some(Size::from_bytes(3)),
            "the report builder derives `mass_delete` from the sizes"
        );
        support_report::assert_changes_valid("remove", changes);
    }
}

#[test]
fn a_removed_tree_gives_a_size_before() {
    let test = TestContext::new();
    write(&test, "/home/user/app/cache/a.bin", b"1234");
    write(&test, "/home/user/app/cache/sub/b.bin", b"56");
    let cache = abs("/home/user/app/cache");
    let args = BoundArgs::new("remove").with("path", paths_value(&[&cache]), false);
    let (planned, applied) = plan_and_apply(&test, &args).unwrap();
    for changes in [&planned, &applied] {
        assert_eq!(changes.len(), 1);
        assert!(changes[0].size_before.is_some());
        support_report::assert_changes_valid("remove", changes);
    }
}
