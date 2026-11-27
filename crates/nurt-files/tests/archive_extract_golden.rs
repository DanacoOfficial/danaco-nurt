// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/archive_extract_golden.rs
// Responsibility:   The examples of the `archive.extract` page of the command documentation.
// Dependencies:     archive_support, nurt_archive, nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The examples of the `archive.extract` page of the command documentation
//! and the transaction checks of the command, run on the virtual file
//! system.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

mod archive_support;

use archive_support::{
    abs, applied_ok, args, build, choice, declared, files, indexes, mutated_paths, path,
    run_effect, stage,
};
use nurt_archive::codec::Algorithm;
use nurt_archive::create::Input;
use nurt_archive::list::Format;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::{Follow, Fs};
use nurt_contracts::journal::{JournalStore, TargetPath};
use nurt_contracts::record::ChangeAction;
use nurt_contracts::stage::StageInput;
use nurt_contracts::value::Value;
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;

const PERMISSION_BITS: u32 = 0o7777;

/// The archive of the first example: a data directory with a script.
fn data_archive() -> Vec<u8> {
    build(
        Format::Tar(Some(Algorithm::Gzip)),
        vec![
            Input::directory("csv").with_mode(0o755),
            Input::file("csv/2026.csv", 8).with_mode(0o644),
            Input::file("load.sh", 10).with_mode(0o755),
        ],
        &[("csv/2026.csv", b"id,total"), ("load.sh", b"#!/bin/sh\n")],
    )
}

#[test]
fn example_extracts_a_compressed_tar_archive() {
    let test = TestContext::new();
    test.fs()
        .write_file(&abs("/srv/dl/data.tar.gz"), &data_archive())
        .expect("the archive is stored");
    test.fs()
        .create_dir_all(&abs("/srv"))
        .expect("the directory exists");

    let outcome = run_effect(
        &test,
        "archive.extract",
        &args(
            "archive.extract",
            &[
                ("path", path("/srv/dl/data.tar.gz")),
                ("into", path("/srv/data")),
            ],
        ),
        StageInput::none(),
    )
    .expect("the example runs");

    assert_eq!(
        test.fs()
            .read_file(&abs("/srv/data/csv/2026.csv"))
            .expect("the file is extracted"),
        b"id,total"
    );
    let script = test
        .fs()
        .metadata(&abs("/srv/data/load.sh"), Follow::NoFollow)
        .expect("the script is extracted");
    assert_eq!(script.mode & PERMISSION_BITS, 0o755);
    assert_eq!(outcome.counter("files"), 2);
    assert_eq!(outcome.counter("directories"), 2);
    assert_eq!(outcome.counter("bytes"), 18);
    assert_eq!(outcome.effect.kind, "archive.extract");
    let actions: Vec<ChangeAction> = outcome.changes.iter().map(|change| change.action).collect();
    assert_eq!(actions, [ChangeAction::Create; 4]);
}

#[test]
fn example_strips_the_top_directory_and_selects_members() {
    let test = TestContext::new();
    let archive = files(
        Format::Zip,
        &[
            ("project-1.2/README.md", b"# project\n"),
            ("project-1.2/docs/guide.md", b"# guide\n"),
            ("project-1.2/src/main.go", b"package main\n"),
        ],
    );
    test.fs()
        .write_file(&abs("/home/user/dl/project-1.2.zip"), &archive)
        .expect("the archive is stored");

    let outcome = run_effect(
        &test,
        "archive.extract",
        &args(
            "archive.extract",
            &[
                ("path", path("/home/user/dl/project-1.2.zip")),
                ("into", path("/home/user/docs")),
                ("only", Value::Glob("*.md".to_owned())),
                ("strip_components", Value::Int(1)),
            ],
        ),
        StageInput::none(),
    )
    .expect("the example runs");

    assert!(test.fs().exists(&abs("/home/user/docs/README.md")));
    assert!(test.fs().exists(&abs("/home/user/docs/docs/guide.md")));
    assert!(!test.fs().exists(&abs("/home/user/docs/src")));
    assert_eq!(outcome.counter("files"), 2);
    assert_eq!(outcome.counter("skipped"), 1);
}

#[test]
fn example_keeps_existing_files() {
    let test = TestContext::new();
    let archive = files(
        Format::Tar(None),
        &[
            ("config.toml", b"port = 80\n"),
            ("static/app.js", b"run()\n"),
        ],
    );
    test.fs()
        .write_file(&abs("/srv/dl/release.tar"), &archive)
        .expect("the archive is stored");
    test.fs()
        .write_file(&abs("/srv/app/config.toml"), b"port = 8080\n")
        .expect("the configuration exists");

    let outcome = run_effect(
        &test,
        "archive.extract",
        &args(
            "archive.extract",
            &[
                ("path", path("/srv/dl/release.tar")),
                ("into", path("/srv/app")),
                ("on_conflict", choice("skip")),
            ],
        ),
        StageInput::none(),
    )
    .expect("the example runs");

    assert_eq!(
        test.fs()
            .read_file(&abs("/srv/app/config.toml"))
            .expect("the configuration is kept"),
        b"port = 8080\n"
    );
    assert_eq!(
        test.fs()
            .read_file(&abs("/srv/app/static/app.js"))
            .expect("the new file is extracted"),
        b"run()\n"
    );
    assert_eq!(outcome.counter("skipped"), 1);
    assert_eq!(outcome.counter("files"), 1);
}

#[test]
fn overwritten_files_are_reported() {
    let test = TestContext::new();
    test.fs()
        .write_file(
            &abs("/srv/dl/a.zip"),
            &files(Format::Zip, &[("a.txt", b"new")]),
        )
        .expect("the archive is stored");
    test.fs()
        .write_file(&abs("/srv/out/a.txt"), b"old")
        .expect("the file exists");
    let outcome = run_effect(
        &test,
        "archive.extract",
        &args(
            "archive.extract",
            &[("path", path("/srv/dl/a.zip")), ("into", path("/srv/out"))],
        ),
        StageInput::none(),
    )
    .expect("the extraction runs");
    assert_eq!(outcome.changes[0].action, ChangeAction::Modify);
    assert_eq!(outcome.notable("overwrote_existing").len(), 1);
    assert_eq!(
        test.fs()
            .read_file(&abs("/srv/out/a.txt"))
            .expect("the file exists"),
        b"new"
    );
}

#[test]
fn every_change_goes_through_the_transaction() {
    let test = TestContext::new();
    let archive = build(
        Format::Tar(None),
        vec![
            Input::directory("d"),
            Input::file("d/a.txt", 1),
            Input::file("d/b.txt", 1),
            Input::symlink("d/latest", "b.txt"),
        ],
        &[("d/a.txt", b"a"), ("d/b.txt", b"b")],
    );
    test.fs()
        .write_file(&abs("/in/a.tar"), &archive)
        .expect("the archive is stored");

    run_effect(
        &test,
        "archive.extract",
        &args(
            "archive.extract",
            &[("path", path("/in/a.tar")), ("into", path("/out"))],
        ),
        StageInput::none(),
    )
    .expect("the extraction runs");

    let targets = declared(&test);
    let expected: Vec<TargetPath> = [
        "/out",
        "/out/d",
        "/out/d/a.txt",
        "/out/d/b.txt",
        "/out/d/latest",
    ]
    .iter()
    .map(|target| TargetPath::Fs(abs(target)))
    .collect();
    assert_eq!(targets, expected);
    let mut changed = applied_ok(&test);
    changed.sort();
    assert_eq!(changed, indexes(targets.len()));
    assert!(
        mutated_paths(&test)
            .iter()
            .all(|changed| changed.starts_with(&abs("/out")))
    );
}

fn stale_args() -> nurt_contracts::stage::BoundArgs {
    args(
        "archive.extract",
        &[("path", path("/in/a.zip")), ("into", path("/out"))],
    )
}

#[test]
fn an_archive_changed_after_the_plan_fails_with_stale_before_any_change() {
    let test = TestContext::new();
    test.fs()
        .write_file(
            &abs("/in/a.zip"),
            &files(Format::Zip, &[("a.txt", b"first")]),
        )
        .expect("the archive is stored");
    let ctx = test.ctx();
    let stage = stage("archive.extract");
    let effect = stage.as_effect().expect("the command changes files");
    let args = stale_args();
    let plan = effect
        .plan(&args, StageInput::none(), &ctx)
        .expect("the plan is made");
    test.fs()
        .write_file(
            &abs("/in/a.zip"),
            &files(Format::Zip, &[("a.txt", b"second version")]),
        )
        .expect("the archive is replaced");

    let mut tx = test.journal().begin(begin_request("archive.extract"));
    let error = effect
        .apply(&args, plan, tx.as_mut(), &ctx)
        .expect_err("the archive changed");
    assert_eq!(error.code(), ErrorCode::Stale);
    assert!(declared(&test).is_empty());
    assert!(!test.fs().exists(&abs("/out")));
}

#[test]
fn a_target_changed_after_the_plan_fails_with_stale_and_is_kept() {
    let test = TestContext::new();
    test.fs()
        .write_file(
            &abs("/in/a.zip"),
            &files(Format::Zip, &[("a.txt", b"archived")]),
        )
        .expect("the archive is stored");
    test.fs()
        .write_file(&abs("/out/a.txt"), b"old")
        .expect("the file exists");
    let ctx = test.ctx();
    let stage = stage("archive.extract");
    let effect = stage.as_effect().expect("the command changes files");
    let args = stale_args();
    let plan = effect
        .plan(&args, StageInput::none(), &ctx)
        .expect("the plan is made");
    test.fs()
        .write_file(&abs("/out/a.txt"), b"written meanwhile")
        .expect("another program writes the file");

    let mut tx = test.journal().begin(begin_request("archive.extract"));
    let error = effect
        .apply(&args, plan, tx.as_mut(), &ctx)
        .expect_err("the target appeared");
    assert_eq!(error.code(), ErrorCode::Stale);
    assert!(applied_ok(&test).is_empty());
    assert_eq!(
        test.fs()
            .read_file(&abs("/out/a.txt"))
            .expect("the file is kept"),
        b"written meanwhile"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
