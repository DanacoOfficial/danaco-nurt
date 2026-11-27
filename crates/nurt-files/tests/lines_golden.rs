// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/lines_golden.rs
// Responsibility:   Examples of `docs/commands/lines.md`, run on the virtual file system.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Examples of `docs/commands/lines.md`, run on the virtual file system.

// Test helpers unwrap results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::Result;
use nurt_contracts::record::{Content, Line, LineEnding};
use nurt_contracts::stage::{ArgValue, BoundArgs, ExecCtx, RecordStream, StageData, StageInput};
use nurt_contracts::value::{self, Range, Record, RecordKind, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::test_path;

/// Files used by the examples.
const FILES: [(&str, &[u8]); 3] = [
    ("/docs/notes.txt", b"alpha\nbeta\r\ngamma\ndelta"),
    ("/docs/a.txt", b"first of a\nsecond of a\n"),
    ("/docs/b.txt", b"first of b\n"),
];

fn project() -> (TestContext, ExecCtx) {
    let test = TestContext::new();
    test.fs()
        .create_dir_all(&test_path("/docs").unwrap())
        .unwrap();
    for (path, content) in FILES {
        test.fs()
            .write_file(&test_path(path).unwrap(), content)
            .unwrap();
    }
    let options = RequestOptions {
        base: Some(test_path("/docs").unwrap().to_language_string()),
        ..RequestOptions::default()
    };
    let ctx = test.builder().options(Arc::new(options)).build();
    (test, ctx)
}

fn lines(ctx: &ExecCtx, values: Vec<(&str, Value)>, input: StageInput<'_>) -> Result<Vec<Line>> {
    let mut args = BoundArgs::new("lines");
    for (name, value) in values {
        args.insert(name, ArgValue::Value(value), false);
    }
    let stage = nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == "lines")
        .unwrap();
    let output = stage.as_read().unwrap().execute(&args, input, ctx)?;
    let StageData::Records(stream) = output.data else {
        panic!("lines produces records");
    };
    stream
        .map(|item| item.map(|record| Line::try_from(record).unwrap()))
        .collect()
}

/// Number, text, line ending and byte offsets of a line.
type Row<'a> = (u64, &'a str, LineEnding, Option<u64>, Option<u64>);

fn range(start: i64, end: i64) -> Value {
    Value::Range(Range::new(Some(start), Some(end)).unwrap())
}

fn path(name: &str) -> String {
    test_path(&format!("/docs/{name}"))
        .unwrap()
        .to_language_string()
}

#[test]
fn range_of_lines_of_a_file() {
    let (_test, ctx) = project();
    let found = lines(
        &ctx,
        vec![
            ("path", Value::Text("notes.txt".to_owned())),
            ("range", range(2, 3)),
        ],
        StageInput::none(),
    )
    .unwrap();
    let summary: Vec<Row<'_>> = found
        .iter()
        .map(|line| {
            (
                line.no,
                line.text.as_str(),
                line.line_ending,
                line.start,
                line.end,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (2, "beta", LineEnding::Crlf, Some(6), Some(10)),
            (3, "gamma", LineEnding::Lf, Some(12), Some(17)),
        ]
    );
    assert!(
        found
            .iter()
            .all(|line| line.path.as_str() == path("notes.txt"))
    );
    assert!(!found[0].etag.is_empty());
}

#[test]
fn lines_of_read_content_keep_the_file_numbering() {
    let (_test, ctx) = project();
    let content = Content {
        path: value::Path::new(path("notes.txt")),
        text: "gamma\ndelta".to_owned(),
        lines: 2,
        etag: "etag-of-notes".to_owned(),
        encoding: "utf-8".to_owned(),
        range: Some(Range::new(Some(3), Some(4)).unwrap()),
        ..Content::default()
    };
    let input = StageInput::records(RecordStream::from_records(vec![Record::from(content)]));
    let found = lines(&ctx, vec![], input).unwrap();
    let summary: Vec<(u64, &str, LineEnding)> = found
        .iter()
        .map(|line| (line.no, line.text.as_str(), line.line_ending))
        .collect();
    assert_eq!(
        summary,
        [(3, "gamma", LineEnding::Lf), (4, "delta", LineEnding::None)]
    );
    assert!(found.iter().all(|line| line.etag == "etag-of-notes"));
}

#[test]
fn first_line_of_every_found_file() {
    let (_test, ctx) = project();
    let entries = ["a.txt", "b.txt"]
        .map(|name| Record::new(RecordKind::Entry).with("path", Value::Path(path(name))))
        .to_vec();
    let input = StageInput::records(RecordStream::from_records(entries));
    let found = lines(&ctx, vec![("range", Value::Int(1))], input).unwrap();
    let summary: Vec<(String, u64, &str)> = found
        .iter()
        .map(|line| (line.path.as_str().to_owned(), line.no, line.text.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            (path("a.txt"), 1, "first of a"),
            (path("b.txt"), 1, "first of b"),
        ]
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
