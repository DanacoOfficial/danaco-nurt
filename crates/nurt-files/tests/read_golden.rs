// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/read_golden.rs
// Responsibility:   Examples of the reference page of `read`, run on a virtual file system.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Examples of the reference page of `read`, run on a virtual file system.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::record::Content;
use nurt_contracts::stage::{ArgValue, BoundArgs, StageData, StageImpl, StageInput};
use nurt_contracts::value::{Range, Regex, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::test_path;

fn read_stage() -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == "read")
        .expect("read is registered")
}

fn file(test: &TestContext, path: &str, content: &str) -> AbsPath {
    let path = test_path(path).expect("test paths are absolute");
    test.fs()
        .write_file(&path, content.as_bytes())
        .expect("the virtual file system accepts the file");
    path
}

fn read(test: &TestContext, path: &AbsPath, params: Vec<(&str, Value)>) -> Vec<Content> {
    let args = params.into_iter().fold(
        BoundArgs::new("read").with(
            "path",
            ArgValue::Value(Value::Path(path.to_language_string())),
            false,
        ),
        |args, (name, value)| args.with(name, ArgValue::Value(value), false),
    );
    let stage = read_stage();
    let ctx = test.ctx();
    let output = stage
        .as_read()
        .expect("read is a read stage")
        .execute(&args, StageInput::none(), &ctx)
        .expect("the arguments are valid");
    match output.data {
        StageData::Records(stream) => stream
            .map(|record| {
                Content::try_from(record.expect("the file is readable")).expect("content")
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn range(start: i64, end: i64) -> Range {
    Range::new(Some(start), Some(end)).expect("start is not after end")
}

/// A Go source file of 212 lines whose `main` function starts on line 44.
fn main_go() -> String {
    (1..=212)
        .map(|line| match line {
            41 => "func helper() {}\n".to_owned(),
            42 | 47 => "\n".to_owned(),
            43 => "// main starts the server.\n".to_owned(),
            44 => "func main() {\n".to_owned(),
            45 => "\thelper()\n".to_owned(),
            46 => "}\n".to_owned(),
            other => format!("// line {other}\n"),
        })
        .collect()
}

#[test]
fn lines_around_a_pattern_with_numbers() {
    let test = TestContext::new();
    let path = file(&test, "/srv/app/main.go", &main_go());
    let read = read(
        &test,
        &path,
        vec![
            (
                "around",
                Value::Regex(Regex::new("func main", "").expect("flags")),
            ),
            ("context", Value::Int(3)),
            ("line_numbers", Value::Bool(true)),
        ],
    );
    assert_eq!(read.len(), 1);
    let content = &read[0];
    assert_eq!(content.range, Some(range(41, 47)));
    assert_eq!(content.total_lines, Some(212));
    assert_eq!(content.lines, 7);
    assert_eq!(
        content.text,
        "41  func helper() {}\n42  \n43  // main starts the server.\n44  func main() {\n45  \thelper()\n46  }\n47  \n"
    );
}

#[test]
fn unchanged_file_is_returned_without_text() {
    let test = TestContext::new();
    let path = file(&test, "/srv/app/main.go", &main_go());
    let first = read(&test, &path, Vec::new());
    let hash = first[0]
        .etag
        .split("sha256:")
        .nth(1)
        .expect("the etag carries the content hash");
    let again = read(
        &test,
        &path,
        vec![("unchanged_since", Value::Text(hash[..8].to_owned()))],
    );
    assert_eq!(again[0].unchanged, Some(true));
    assert_eq!(again[0].text, "");
    assert_eq!(again[0].etag, first[0].etag);
}

#[test]
fn overlapping_ranges_are_merged_in_order() {
    let test = TestContext::new();
    let text: String = (1..=12).map(|line| format!("entry {line}\n")).collect();
    let path = file(&test, "/srv/app/CHANGELOG.md", &text);
    let read = read(
        &test,
        &path,
        vec![(
            "lines",
            Value::List(vec![
                Value::Range(range(5, 8)),
                Value::Range(range(1, 2)),
                Value::Range(range(7, 10)),
            ]),
        )],
    );
    assert_eq!(read[0].ranges, Some(vec![range(5, 10), range(1, 2)]));
    assert_eq!(read[0].lines, 8);
    assert!(read[0].text.starts_with("entry 5\n"));
    assert!(read[0].text.ends_with("entry 10\nentry 1\nentry 2\n"));
}

#[test]
fn nested_markdown_section() {
    let test = TestContext::new();
    let readme = "# App\n\n## Install\n\n### Linux\n\napt install app\n\n### Windows\n\nwinget install app\n\n## Usage\n\napp run\n";
    let path = file(&test, "/srv/app/README.md", readme);
    let read = read(
        &test,
        &path,
        vec![("section", Value::Text("Install > Linux".to_owned()))],
    );
    assert_eq!(read[0].text, "### Linux\n\napt install app\n\n");
    assert_eq!(read[0].range, Some(range(5, 8)));
}

#[test]
fn last_lines_of_a_log() {
    let test = TestContext::new();
    let path = file(
        &test,
        "/var/log/app.log",
        "start\nrequest 1\nrequest 2\nstop",
    );
    let read = read(&test, &path, vec![("last", Value::Int(2))]);
    assert_eq!(read[0].text, "request 2\nstop");
    assert_eq!(read[0].range, Some(range(3, 4)));
    assert_eq!(read[0].total_lines, Some(4));
}

fn read_code(test: &TestContext, path: &AbsPath, params: Vec<(&str, Value)>) -> ErrorCode {
    let args = params.into_iter().fold(
        BoundArgs::new("read").with(
            "path",
            ArgValue::Value(Value::Path(path.to_language_string())),
            false,
        ),
        |args, (name, value)| args.with(name, ArgValue::Value(value), false),
    );
    let stage = read_stage();
    let ctx = test.ctx();
    let output =
        stage
            .as_read()
            .expect("read is a read stage")
            .execute(&args, StageInput::none(), &ctx);
    let error = match output {
        Err(error) => error,
        Ok(output) => match output.data {
            StageData::Records(stream) => stream.filter_map(Result::err).next(),
            _ => None,
        }
        .expect("the read fails"),
    };
    error.code()
}

#[test]
fn lines_beyond_the_end_of_the_file_are_out_of_range() {
    let test = TestContext::new();
    let path = file(&test, "/srv/app/notes.txt", "one\ntwo\nthree\n");
    for lines in [
        Value::Int(4),
        Value::Range(range(4, 9)),
        Value::List(vec![Value::Range(range(1, 2)), Value::Int(10)]),
    ] {
        assert_eq!(
            read_code(&test, &path, vec![("lines", lines)]),
            ErrorCode::OutOfRange
        );
    }
    let clipped = read(&test, &path, vec![("lines", Value::Range(range(3, 9)))]);
    assert_eq!(clipped[0].text, "three\n");
}

#[test]
fn cell_index_beyond_the_notebook_is_out_of_range() {
    let test = TestContext::new();
    let notebook = r##"{"cells": [{"cell_type": "markdown", "id": "intro", "metadata": {}, "source": "# Title"}], "metadata": {}, "nbformat": 4, "nbformat_minor": 5}"##;
    let path = file(&test, "/srv/app/analysis.ipynb", notebook);
    assert_eq!(
        read_code(&test, &path, vec![("cell", Value::Int(3))]),
        ErrorCode::OutOfRange
    );
    assert_eq!(
        read_code(
            &test,
            &path,
            vec![("cell", Value::Text("absent".to_owned()))]
        ),
        ErrorCode::KeyNotFound
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
