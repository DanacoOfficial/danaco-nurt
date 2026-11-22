// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The examples of the `archive.create` page of the command documentation
//! and the transaction checks of the command, run on the virtual file
//! system.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

mod archive_support;

use archive_support::{
    abs, applied_ok, args, declared, entry, indexes, list_archive, path, run_effect, stage,
};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::journal::{JournalStore, TargetPath};
use nurt_contracts::record::{ChangeAction, EntryType};
use nurt_contracts::stage::{BoundArgs, RecordStream, StageInput};
use nurt_contracts::value::{Record, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;

/// The application of the examples: a build output and its sources.
fn application() -> TestContext {
    let test = TestContext::new();
    for (file, content) in [
        ("/srv/app/README.md", &b"# app\n"[..]),
        ("/srv/app/dist/index.html", &b"<html></html>"[..]),
        ("/srv/app/dist/js/app.js", &b"run();\n"[..]),
        ("/srv/app/src/main.go", &b"package main\n"[..]),
    ] {
        test.fs()
            .write_file(&abs(file), content)
            .expect("the file is stored");
    }
    test.fs()
        .create_dir_all(&abs("/srv/out"))
        .expect("the directory exists");
    test
}

fn records(records: Vec<Record>) -> StageInput<'static> {
    StageInput::records(RecordStream::from_records(records))
}

fn member_paths(test: &TestContext, archive: &str) -> Vec<String> {
    list_archive(test, &args("archive.list", &[("path", path(archive))]))
        .expect("the archive is listed")
        .into_iter()
        .map(|entry| entry.path.unwrap_or_default())
        .collect()
}

#[test]
fn example_archives_entries_relative_to_a_base_directory() {
    let test = application();
    let found = vec![
        entry("/srv/app/dist/index.html", Some("index.html")),
        entry("/srv/app/dist/js", Some("js")),
        entry("/srv/app/dist/js/app.js", Some("js/app.js")),
    ];
    let outcome = run_effect(
        &test,
        "archive.create",
        &args(
            "archive.create",
            &[
                ("to", path("/srv/out/dist.zip")),
                ("base", path("/srv/app")),
            ],
        ),
        records(found),
    )
    .expect("the example runs");

    assert_eq!(
        member_paths(&test, "/srv/out/dist.zip"),
        ["dist/index.html", "dist/js", "dist/js/app.js"]
    );
    assert_eq!(outcome.changes.len(), 1);
    assert_eq!(outcome.changes[0].action, ChangeAction::Create);
    assert_eq!(outcome.counter("files"), 2);
    assert_eq!(outcome.counter("directories"), 1);
}

#[test]
fn example_archives_a_listed_directory() {
    let test = application();
    let listed = vec![
        entry("/srv/app/README.md", Some("README.md")),
        entry("/srv/app/src", Some("src")),
    ];
    run_effect(
        &test,
        "archive.create",
        &args("archive.create", &[("to", path("/srv/out/app.tar.gz"))]),
        records(listed),
    )
    .expect("the example runs");

    let entries = list_archive(
        &test,
        &args("archive.list", &[("path", path("/srv/out/app.tar.gz"))]),
    )
    .expect("the archive is listed");
    let names: Vec<&str> = entries
        .iter()
        .map(|entry| entry.path.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(names, ["README.md", "src", "src/main.go"]);
    assert_eq!(entries[1].kind, EntryType::Directory);
    assert_eq!(entries[0].format.as_deref(), Some("tar_gz"));
}

#[test]
fn example_sets_the_format_and_level() {
    let test = application();
    run_effect(
        &test,
        "archive.create",
        &args(
            "archive.create",
            &[
                ("to", path("/srv/out/app.bin")),
                ("format", Value::Enum("tar_zst".to_owned())),
                ("level", Value::Int(19)),
            ],
        ),
        records(vec![entry("/srv/app/src", None)]),
    )
    .expect("the example runs");

    let entries = list_archive(
        &test,
        &args("archive.list", &[("path", path("/srv/out/app.bin"))]),
    )
    .expect("the archive is listed");
    assert_eq!(entries[0].format.as_deref(), Some("tar_zst"));
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.path.as_deref().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["src", "src/main.go"]
    );
}

#[test]
fn archives_are_reproducible() {
    let first = application();
    let second = application();
    for test in [&first, &second] {
        run_effect(
            test,
            "archive.create",
            &args("archive.create", &[("to", path("/srv/out/app.zip"))]),
            records(vec![entry("/srv/app", None)]),
        )
        .expect("the archive is created");
    }
    let bytes = |test: &TestContext| {
        test.fs()
            .read_file(&abs("/srv/out/app.zip"))
            .expect("the archive exists")
    };
    assert_eq!(bytes(&first), bytes(&second));
}

#[test]
fn every_change_goes_through_the_transaction() {
    let test = application();
    run_effect(
        &test,
        "archive.create",
        &args("archive.create", &[("to", path("/srv/out/app.zip"))]),
        records(vec![entry("/srv/app", None)]),
    )
    .expect("the archive is created");
    assert_eq!(declared(&test), [TargetPath::Fs(abs("/srv/out/app.zip"))]);
    assert_eq!(applied_ok(&test), indexes(1));
}

fn create_args() -> BoundArgs {
    args("archive.create", &[("to", path("/srv/out/app.zip"))])
}

#[test]
fn a_source_changed_after_the_plan_fails_with_stale() {
    let test = application();
    let ctx = test.ctx();
    let stage = stage("archive.create");
    let effect = stage.as_effect().expect("the command changes files");
    let args = create_args();
    let plan = effect
        .plan(&args, records(vec![entry("/srv/app/src", None)]), &ctx)
        .expect("the plan is made");
    test.fs()
        .write_file(&abs("/srv/app/src/main.go"), b"package main // changed\n")
        .expect("another program writes the file");

    let mut tx = test.journal().begin(begin_request("archive.create"));
    let error = effect
        .apply(&args, plan, tx.as_mut(), &ctx)
        .expect_err("the source changed");
    assert_eq!(error.code(), ErrorCode::Stale);
    assert!(applied_ok(&test).is_empty());
}

#[test]
fn an_archive_created_meanwhile_is_not_replaced() {
    let test = application();
    let ctx = test.ctx();
    let stage = stage("archive.create");
    let effect = stage.as_effect().expect("the command changes files");
    let args = create_args();
    let plan = effect
        .plan(&args, records(vec![entry("/srv/app/src", None)]), &ctx)
        .expect("the plan is made");
    test.fs()
        .write_file(&abs("/srv/out/app.zip"), b"other")
        .expect("another program writes the archive");

    let mut tx = test.journal().begin(begin_request("archive.create"));
    let error = effect
        .apply(&args, plan, tx.as_mut(), &ctx)
        .expect_err("the archive exists");
    assert!(
        matches!(error.code(), ErrorCode::Exists | ErrorCode::Stale),
        "{error:?}"
    );
    assert_eq!(
        test.fs()
            .read_file(&abs("/srv/out/app.zip"))
            .expect("the file is kept"),
        b"other"
    );
}

#[test]
fn an_existing_archive_fails_in_the_plan() {
    let test = application();
    test.fs()
        .write_file(&abs("/srv/out/app.zip"), b"old")
        .expect("the archive exists");
    let error = run_effect(
        &test,
        "archive.create",
        &create_args(),
        records(vec![entry("/srv/app", None)]),
    )
    .expect_err("the archive exists");
    assert_eq!(error.code(), ErrorCode::Exists);
}
