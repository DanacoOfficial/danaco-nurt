// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/search_golden.rs
// Responsibility:   Examples of `docs/commands/search.md`, run on the virtual file system.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Examples of `docs/commands/search.md`, run on the virtual file system.

// Test helpers unwrap results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::Result;
use nurt_contracts::record::{Entry, Match};
use nurt_contracts::stage::{ArgValue, BoundArgs, ExecCtx, StageData, StageInput};
use nurt_contracts::value::{Record, RecordKind, Regex, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::test_path;

/// Project used by every example: two Rust sources and a log file.
const FILES: [(&str, &str); 4] = [
    ("/proj/src/main.rs", "fn main() {\n    run();\n}\n"),
    ("/proj/src/lib.rs", "pub fn run() {}\n"),
    (
        "/proj/logs/app.log",
        "start\nWARNING: disk almost full\nend\n",
    ),
    ("/proj/README.md", "Call `run` from `fn main`.\n"),
];

fn project() -> (TestContext, ExecCtx) {
    let test = TestContext::new();
    for (path, content) in FILES {
        let path = test_path(path).unwrap();
        test.fs().create_dir_all(&path.parent().unwrap()).unwrap();
        test.fs().write_file(&path, content.as_bytes()).unwrap();
    }
    let options = RequestOptions {
        base: Some(test_path("/proj").unwrap().to_language_string()),
        ..RequestOptions::default()
    };
    let ctx = test.builder().options(Arc::new(options)).build();
    (test, ctx)
}

fn search(ctx: &ExecCtx, values: Vec<(&str, Value)>) -> Result<Vec<Record>> {
    let mut args = BoundArgs::new("search");
    for (name, value) in values {
        args.insert(name, ArgValue::Value(value), false);
    }
    let stage = nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == "search")
        .unwrap();
    let output = stage
        .as_read()
        .unwrap()
        .execute(&args, StageInput::none(), ctx)?;
    let StageData::Records(stream) = output.data else {
        panic!("search produces records");
    };
    stream.collect()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn regex(pattern: &str) -> Value {
    Value::Regex(Regex {
        pattern: pattern.to_owned(),
        flags: Default::default(),
    })
}

fn path(relative: &str) -> String {
    test_path(&format!("/proj/{relative}"))
        .unwrap()
        .to_language_string()
}

fn matches(records: Vec<Record>) -> Vec<Match> {
    records
        .into_iter()
        .map(|record| Match::try_from(record).unwrap())
        .collect()
}

#[test]
fn literal_text_in_a_directory() {
    let (_test, ctx) = project();
    let found = matches(
        search(
            &ctx,
            vec![("pattern", text("fn main")), ("path", text("src"))],
        )
        .unwrap(),
    );
    assert_eq!(found.len(), 1);
    let first = &found[0];
    assert_eq!(first.path.as_str(), path("src/main.rs"));
    assert_eq!(
        (first.no, first.column, first.start, first.end),
        (1, 1, 0, 7)
    );
    assert_eq!(first.text, "fn main");
    assert_eq!(first.line, "fn main() {");
    assert!(!first.etag.is_empty());
}

#[test]
fn regular_expression_with_a_named_group() {
    let (_test, ctx) = project();
    let found = matches(
        search(
            &ctx,
            vec![
                ("pattern", regex(r"fn (?<name>\w+)")),
                ("path", text("src")),
                ("only_matching", Value::Bool(true)),
            ],
        )
        .unwrap(),
    );
    let summary: Vec<(String, u64, u64, String)> = found
        .iter()
        .map(|found| {
            let name = found
                .named
                .as_ref()
                .and_then(|named| named.get("name").cloned().flatten());
            (
                found.path.as_str().to_owned(),
                found.no,
                found.column,
                name.unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (path("src/lib.rs"), 1, 5, "run".to_owned()),
            (path("src/main.rs"), 1, 1, "main".to_owned()),
        ]
    );
    assert_eq!(found[0].line, "fn run");
    assert_eq!(
        found[0].regex_engine.map(|engine| engine.as_str()),
        Some("linear")
    );
}

#[test]
fn case_insensitive_search_with_context() {
    let (_test, ctx) = project();
    let found = matches(
        search(
            &ctx,
            vec![
                ("pattern", text("warning")),
                ("path", text("logs")),
                ("case_sensitive", Value::Bool(false)),
                ("context", Value::Int(1)),
            ],
        )
        .unwrap(),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].no, 2);
    assert_eq!(found[0].text, "WARNING");
    assert_eq!(found[0].before.as_deref(), Some(&["start".to_owned()][..]));
    assert_eq!(found[0].after.as_deref(), Some(&["end".to_owned()][..]));
}

#[test]
fn counts_per_pattern_of_a_list() {
    let (_test, ctx) = project();
    let found = matches(
        search(
            &ctx,
            vec![
                ("pattern", Value::List(vec![text("run"), text("main")])),
                ("path", text("src")),
                ("output", Value::Enum("counts".to_owned())),
            ],
        )
        .unwrap(),
    );
    let summary: Vec<(String, Option<u64>, Option<u64>, u64)> = found
        .iter()
        .map(|found| {
            (
                found.path.as_str().to_owned(),
                found.pattern,
                found.count,
                found.no,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (path("src/lib.rs"), Some(0), Some(1), 1),
            (path("src/main.rs"), Some(0), Some(1), 2),
            (path("src/main.rs"), Some(1), Some(1), 1),
        ]
    );
}

#[test]
fn files_with_matches_sorted_by_count() {
    let (_test, ctx) = project();
    let records = search(
        &ctx,
        vec![
            ("pattern", regex(r"\brun\b|\bmain\b")),
            ("path", text(".")),
            ("output", Value::Enum("files".to_owned())),
            ("sort", Value::Enum("count".to_owned())),
        ],
    )
    .unwrap();
    assert!(
        records
            .iter()
            .all(|record| record.kind == RecordKind::Entry)
    );
    let entries: Vec<(String, Option<u64>)> = records
        .into_iter()
        .map(|record| Entry::try_from(record).unwrap())
        .map(|entry| (entry.path.as_str().to_owned(), entry.match_count))
        .collect();
    assert_eq!(
        entries,
        [
            (path("README.md"), Some(2)),
            (path("src/main.rs"), Some(2)),
            (path("src/lib.rs"), Some(1)),
        ]
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
