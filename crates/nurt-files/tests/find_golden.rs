// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The examples of the `find` page of the command documentation, run on the
//! virtual file system.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::{Entry, EntryType};
use nurt_contracts::stage::{ArgValue, BoundArgs, StageImpl, StageInput};
use nurt_contracts::value::Value;
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

fn find(path: &str, given: &[(&str, Value)]) -> BoundArgs {
    given.iter().fold(
        BoundArgs::new("find").with("path", ArgValue::Value(Value::Path(language(path))), false),
        |args, (name, value)| args.with(*name, ArgValue::Value(value.clone()), false),
    )
}

fn choice(text: &str) -> Value {
    Value::Enum(text.to_owned())
}

/// The file system of the examples: a repository with sources in two
/// languages, a dependency directory and the repository metadata.
fn repository() -> TestContext {
    let test = TestContext::new();
    for path in [
        "/home/user/app/.git/HEAD",
        "/home/user/app/Cargo.toml",
        "/home/user/app/src/main.rs",
        "/home/user/app/src/parser/lexer.rs",
        "/home/user/app/web/fooBar.ts",
        "/home/user/app/web/fabric.ts",
        "/home/user/app/web/node_modules/left-pad/index.ts",
    ] {
        test.fs()
            .write_file(&test_path(path).expect("test paths are absolute"), b"\n")
            .expect("the virtual file system accepts the file");
    }
    test
}

fn run(test: &TestContext, args: &BoundArgs) -> Result<Vec<Entry>, ErrorCode> {
    let stage = stage("find");
    let ctx = test.ctx();
    let output = stage
        .as_read()
        .expect("find reads")
        .execute(args, StageInput::none(), &ctx)
        .map_err(|error| error.code())?;
    Ok(StageInput::from(output)
        .into_records()
        .expect("find produces records")
        .map(|item| {
            Entry::try_from(item.expect("every record is produced"))
                .expect("every record is an entry")
        })
        .collect())
}

fn relatives(entries: &[Entry]) -> Vec<&str> {
    entries
        .iter()
        .map(|entry| entry.relative.as_deref().unwrap_or_default())
        .collect()
}

#[test]
fn example_finds_sources_at_any_depth() {
    let test = repository();
    let entries = run(
        &test,
        &find(
            "/home/user/app",
            &[("glob", Value::Glob("**/*.rs".to_owned()))],
        ),
    )
    .expect("the example runs");
    assert_eq!(relatives(&entries), ["src/main.rs", "src/parser/lexer.rs"]);
    assert!(entries.iter().all(|entry| entry.kind == EntryType::File));
}

#[test]
fn example_finds_typed_files_without_dependencies() {
    let test = repository();
    let entries = run(
        &test,
        &find(
            "/home/user/app",
            &[
                (
                    "exclude_preset",
                    Value::List(vec![choice("vcs"), choice("deps")]),
                ),
                ("file_type", Value::List(vec![Value::Text("ts".to_owned())])),
            ],
        ),
    )
    .expect("the example runs");
    assert_eq!(relatives(&entries), ["web/fabric.ts", "web/fooBar.ts"]);
}

#[test]
fn example_ranks_names_by_similarity() {
    let test = repository();
    let entries = run(
        &test,
        &find(
            "/home/user/app",
            &[
                ("fuzzy", Value::Text("fb".to_owned())),
                ("sort", choice("rank")),
            ],
        ),
    )
    .expect("the example runs");
    assert_eq!(relatives(&entries), ["web/fooBar.ts", "web/fabric.ts"]);
    assert!(entries[0].rank > entries[1].rank);
}

#[test]
fn example_yields_one_entry_for_one_path() {
    let test = repository();
    let entries = run(
        &test,
        &find(
            "/home/user/app/Cargo.toml",
            &[("depth", Value::Int(0)), ("type", choice("any"))],
        ),
    )
    .expect("the example runs");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].path.as_str(),
        language("/home/user/app/Cargo.toml")
    );
    let missing = run(
        &test,
        &find(
            "/home/user/app/Cargo.lock",
            &[("depth", Value::Int(0)), ("type", choice("any"))],
        ),
    );
    assert_eq!(missing.err(), Some(ErrorCode::NotFound));
}
