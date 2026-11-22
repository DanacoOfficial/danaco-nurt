// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of `docs/commands/rename.md`, run on the virtual file system.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Follow, Fs};
use nurt_contracts::journal::JournalStore;
use nurt_contracts::record::{ChangeAction, Effect};
use nurt_contracts::stage::{ArgValue, BoundArgs, RecordStream, StageData, StageImpl, StageInput};
use nurt_contracts::value::{Record, RecordKind, Regex, Value};
use nurt_files::common::etag::etag_text;
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

fn text(value: &str) -> ArgValue {
    ArgValue::Value(Value::Text(value.to_owned()))
}

fn entry(path: &AbsPath) -> Record {
    Record::new(RecordKind::Entry).with("path", Value::Path(path.to_language_string()))
}

/// Names of the entries of a directory, sorted.
fn names(test: &TestContext, dir: &str) -> Vec<String> {
    let mut names: Vec<String> = test
        .fs()
        .read_dir(&abs(dir))
        .unwrap()
        .map(|item| item.unwrap().name.to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
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
    let mut tx = test.journal().begin(begin_request(args.command()));
    let outcome = effect.apply(args, plan, tx.as_mut(), &ctx)?;
    let StageData::Records(stream) = outcome.output.data else {
        panic!("the stage returns the effect record");
    };
    let records = stream.collect::<Result<Vec<_>>>()?;
    tx.commit()?;
    let record = records.into_iter().next().expect("one effect record");
    Ok(Effect::try_from(record)?)
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

/// `rename path: "/home/user/draft.md" to: "final.md"`
#[test]
fn example_1_renames_an_entry_to_a_new_name() {
    let test = TestContext::new();
    let draft = write(&test, "/home/user/draft.md", b"# Notes\n");
    let args = BoundArgs::new("rename")
        .with(
            "path",
            ArgValue::Value(Value::Path(draft.to_language_string())),
            false,
        )
        .with("to", text("final.md"), false);
    let effect = execute(&test, &args, None).unwrap();
    assert_eq!(names(&test, "/home/user"), ["final.md"]);
    let changes = effect.changes.clone().unwrap();
    assert_eq!(changes[0].action, ChangeAction::Rename);
    assert_eq!(changes[0].path.as_str(), draft.to_language_string());
    assert_eq!(counter(&effect, "renamed"), Some(1));
    assert_journaled(&test);
}

/// `list "/home/user/photos" glob: "*.jpeg" |> rename ".jpeg" ".jpg"`
#[test]
fn example_2_replaces_an_extension_in_every_input_name() {
    let test = TestContext::new();
    let first = write(&test, "/home/user/photos/IMG_001.jpeg", b"1");
    let second = write(&test, "/home/user/photos/IMG_002.jpeg", b"2");
    let args = BoundArgs::new("rename")
        .with("pattern", text(".jpeg"), false)
        .with("with", text(".jpg"), false);
    let effect = execute(&test, &args, Some(vec![entry(&first), entry(&second)])).unwrap();
    assert_eq!(
        names(&test, "/home/user/photos"),
        ["IMG_001.jpg", "IMG_002.jpg"]
    );
    assert_eq!(effect.targets, Some(2));
    assert_eq!(counter(&effect, "renamed"), Some(2));
    assert_journaled(&test);
}

/// `list "/home/user/logs" |> rename /(\d{4})-(\d{2})-(\d{2})/ "$3.$2.$1"`
#[test]
fn example_3_rewrites_dates_with_a_regex_template() {
    let test = TestContext::new();
    let log = write(&test, "/home/user/logs/app-2026-11-22.log", b"l");
    let notes = write(&test, "/home/user/logs/notes.txt", b"n");
    let pattern = Regex::new(r"(\d{4})-(\d{2})-(\d{2})", "").unwrap();
    let args = BoundArgs::new("rename")
        .with("pattern", ArgValue::Value(Value::Regex(pattern)), false)
        .with("with", text("$3.$2.$1"), false);
    let effect = execute(&test, &args, Some(vec![entry(&log), entry(&notes)])).unwrap();
    assert_eq!(
        names(&test, "/home/user/logs"),
        ["app-22.11.2026.log", "notes.txt"]
    );
    assert_eq!(counter(&effect, "renamed"), Some(1));
    assert_eq!(counter(&effect, "unchanged"), Some(1));
    assert_journaled(&test);
}

/// `rename path: "/Volumes/data/readme.md" to: "README.md"`
#[test]
fn example_4_changes_only_the_letter_case_on_a_case_insensitive_volume() {
    let test = TestContext::new();
    let mut config = VolumeConfig::default();
    config.features.case_sensitive = false;
    test.fs().add_volume(&abs("/Volumes/data"), config).unwrap();
    let readme = write(&test, "/Volumes/data/readme.md", b"r");
    let args = BoundArgs::new("rename")
        .with(
            "path",
            ArgValue::Value(Value::Path(readme.to_language_string())),
            false,
        )
        .with("to", text("README.md"), false);
    execute(&test, &args, None).unwrap();
    assert_eq!(names(&test, "/Volumes/data"), ["README.md"]);
    assert_journaled(&test);
}

#[test]
fn an_existing_name_fails_by_default_and_changes_nothing() {
    let test = TestContext::new();
    let draft = write(&test, "/home/user/draft.md", b"new");
    let kept = write(&test, "/home/user/final.md", b"old");
    let args = BoundArgs::new("rename")
        .with(
            "path",
            ArgValue::Value(Value::Path(draft.to_language_string())),
            false,
        )
        .with("to", text("final.md"), false);
    let error = execute(&test, &args, None).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Exists);
    assert_eq!(test.fs().read_file(&kept).unwrap(), b"old");
    assert_untouched(&test);
}

#[test]
fn entries_changed_since_they_were_read_are_stale() {
    let test = TestContext::new();
    let path = write(&test, "/home/user/draft.md", b"v1");
    let meta = test.fs().metadata(&path, Follow::NoFollow).unwrap();
    let record = entry(&path).with("etag", Value::Text(etag_text(&meta.etag())));
    test.fs().write_file(&path, b"version 2").unwrap();
    let args = BoundArgs::new("rename").with("to", text("final.md"), false);
    let error = execute(&test, &args, Some(vec![record])).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Stale);
    assert!(test.fs().exists(&path));
    assert_untouched(&test);
}
