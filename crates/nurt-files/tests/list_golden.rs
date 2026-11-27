// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/list_golden.rs
// Responsibility:   The examples of the `list` page of the command documentation, run on the virtual file system.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The examples of the `list` page of the command documentation, run on the
//! virtual file system.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::record::{Entry, EntryType};
use nurt_contracts::stage::{ArgValue, BoundArgs, RenderHint, StageImpl, StageInput};
use nurt_contracts::value::{Size, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::test_path;

fn stage(command: &str) -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == command)
        .expect("the command is registered")
}

fn language(path: &str) -> String {
    test_path(path)
        .expect("test paths are absolute")
        .to_language_string()
}

fn list(path: &str, given: &[(&str, Value)]) -> BoundArgs {
    given.iter().fold(
        BoundArgs::new("list").with("path", ArgValue::Value(Value::Path(language(path))), false),
        |args, (name, value)| args.with(*name, ArgValue::Value(value.clone()), false),
    )
}

fn choice(text: &str) -> Value {
    Value::Enum(text.to_owned())
}

/// The file system of the examples: an application directory with sources,
/// documentation and a read-me file.
fn application() -> TestContext {
    let test = TestContext::new();
    for (path, content) in [
        ("/home/user/app/README.md", "# app\n".as_bytes()),
        ("/home/user/app/docs/guide.md", &[b'g'; 2048]),
        ("/home/user/app/src/main.go", "package main\n".as_bytes()),
        (
            "/home/user/app/src/util/util.go",
            "package util\n".as_bytes(),
        ),
    ] {
        test.fs()
            .write_file(&test_path(path).expect("test paths are absolute"), content)
            .expect("the virtual file system accepts the file");
    }
    test
}

fn run(test: &TestContext, args: &BoundArgs) -> (Vec<Entry>, Option<RenderHint>) {
    let stage = stage("list");
    let ctx = test.ctx();
    let output = stage
        .as_read()
        .expect("list reads")
        .execute(args, StageInput::none(), &ctx)
        .expect("the example runs");
    let render = output.render;
    let entries = StageInput::from(output)
        .into_records()
        .expect("list produces records")
        .map(|item| {
            Entry::try_from(item.expect("every record is produced"))
                .expect("every record is an entry")
        })
        .collect();
    (entries, render)
}

fn relatives(entries: &[Entry]) -> Vec<&str> {
    entries
        .iter()
        .map(|entry| entry.relative.as_deref().unwrap_or_default())
        .collect()
}

#[test]
fn example_lists_the_entries_of_a_directory() {
    let test = application();
    let (entries, render) = run(&test, &list("/home/user/app", &[]));
    assert_eq!(relatives(&entries), ["README.md", "docs", "src"]);
    assert_eq!(render, None);
    assert_eq!(entries[0].kind, EntryType::File);
    assert_eq!(
        entries[0].path.as_str(),
        language("/home/user/app/README.md")
    );
    assert_eq!(entries[1].kind, EntryType::Directory);
}

#[test]
fn example_shows_two_levels_as_a_tree() {
    let test = application();
    let (entries, render) = run(
        &test,
        &list(
            "/home/user/app",
            &[("depth", Value::Int(2)), ("view", choice("tree"))],
        ),
    );
    assert_eq!(render, Some(RenderHint::Tree));
    assert_eq!(
        relatives(&entries),
        [
            "README.md",
            "docs",
            "docs/guide.md",
            "src",
            "src/main.go",
            "src/util"
        ]
    );
    let depths: Vec<Option<u64>> = entries.iter().map(|entry| entry.depth).collect();
    assert_eq!(
        depths,
        [Some(1), Some(1), Some(2), Some(1), Some(2), Some(2)]
    );
}

#[test]
fn example_shows_directory_totals_largest_first() {
    let test = application();
    let (entries, render) = run(
        &test,
        &list(
            "/home/user/app",
            &[
                ("view", choice("sizes")),
                ("sort", choice("size")),
                ("order", choice("desc")),
            ],
        ),
    );
    assert_eq!(render, Some(RenderHint::Sizes));
    assert_eq!(relatives(&entries), ["docs", "src", "README.md"]);
    assert_eq!(entries[0].total_size, Some(Size::from_bytes(2048)));
    assert_eq!(entries[0].file_count, Some(1));
    assert_eq!(entries[1].total_size, Some(Size::from_bytes(13 + 13)));
    assert_eq!(entries[1].file_count, Some(2));
    assert_eq!(entries[2].total_size, None);
}

#[test]
fn example_lists_files_most_recently_modified_first() {
    let test = application();
    test.clock().advance(Duration::from_secs(5));
    test.fs()
        .write_file(
            &test_path("/home/user/app/src/version.go").expect("test paths are absolute"),
            b"package main\n",
        )
        .expect("the virtual file system accepts the file");
    let (entries, render) = run(
        &test,
        &list(
            "/home/user/app/src",
            &[
                ("type", choice("file")),
                ("sort", choice("modified")),
                ("order", choice("desc")),
            ],
        ),
    );
    assert_eq!(render, None);
    assert_eq!(relatives(&entries), ["version.go", "main.go"]);
    assert!(entries.iter().all(|entry| entry.kind == EntryType::File));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
