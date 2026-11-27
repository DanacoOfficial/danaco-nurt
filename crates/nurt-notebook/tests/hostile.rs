// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-notebook/tests/hostile.rs
// Responsibility:   Notebooks from untrusted sources: malformed and truncated text, wrong types, unknown members that must survive an edit, and addressing at the boundaries.
// Dependencies:     nurt_contracts, support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Notebooks from untrusted sources: malformed and truncated text, wrong
//! types, unknown members that must survive an edit, and addressing at the
//! boundaries.

// Fixtures are built with `unwrap`; invalid fixture data fails the test.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "fixtures in tests fail the test on invalid data"
)]

mod support;

use std::collections::HashSet;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::CellType;
use nurt_contracts::value::Range;
use nurt_notebook::{CellRef, IdGenerator, Notebook, NotebookError, Position, Replace, Selection};
use support::{render, sample};

const WITH_UNKNOWN_MEMBERS: &str = "{\n \"cells\": [\n  {\n   \"cell_type\": \"markdown\",\n   \"extra\":   [1,  2.50],\n   \"id\": \"a1\",\n   \"metadata\": {},\n   \"source\": [\"x\"]\n  }\n ],\n \"metadata\": {},\n \"nbformat\": 4,\n \"nbformat_minor\": 5,\n \"zz_future\":   {\"k\":1.50}\n}\n";

#[test]
fn unknown_members_keep_their_text_when_a_cell_changes() {
    let mut notebook = Notebook::parse(WITH_UNKNOWN_MEMBERS.as_bytes()).unwrap();
    notebook
        .replace(&Replace {
            target: Some(CellRef::Id("a1".into())),
            source: Some("y".into()),
            ..Replace::default()
        })
        .unwrap();
    assert_eq!(
        notebook.render(),
        WITH_UNKNOWN_MEMBERS.replace("\"source\": [\"x\"]", "\"source\": [\n    \"y\"\n   ]")
    );
}

#[test]
fn unknown_members_keep_their_text_when_cells_are_added_and_removed() {
    let mut notebook = Notebook::parse(WITH_UNKNOWN_MEMBERS.as_bytes()).unwrap();
    notebook.set_id_generator(IdGenerator::sequential());
    notebook.insert(&Position::End, CellType::Raw, "r").unwrap();
    notebook
        .remove(&Selection::Ids(vec!["00000001".into()]))
        .unwrap();
    assert_eq!(notebook.render(), WITH_UNKNOWN_MEMBERS);
}

#[test]
fn repeated_root_keys_render_verbatim_and_the_last_one_counts() {
    let text = "{\"cells\": [], \"nbformat\": 3, \"nbformat\": 4}";
    let notebook = Notebook::parse(text.as_bytes()).unwrap();
    assert_eq!(notebook.nbformat(), (4, 0));
    assert_eq!(notebook.render(), text);
}

#[test]
fn every_truncation_of_a_notebook_is_a_format_error() {
    let text = render(&sample(), " ");
    let bytes = text.trim_end().as_bytes();
    for keep in 0..bytes.len() {
        let error = Notebook::parse(&bytes[..keep]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format, "kept {keep} bytes");
    }
    assert!(Notebook::parse(bytes).is_ok());
}

#[test]
fn deeply_nested_members_do_not_panic() {
    let depth = 5_000;
    let text = format!(
        "{{\"cells\": [], \"nbformat\": 4, \"x\": {}{}}}",
        "[".repeat(depth),
        "]".repeat(depth)
    );
    if let Ok(notebook) = Notebook::parse(text.as_bytes()) {
        assert_eq!(notebook.render(), text);
    }
}

#[test]
fn wrong_types_and_versions_are_format_errors() {
    let cases: [(&str, &str); 14] = [
        ("", "not valid JSON"),
        ("   \n", "not valid JSON"),
        ("[]", "not an nbformat 4 notebook"),
        ("{}", "missing `nbformat`"),
        (
            "{\"cells\": [], \"nbformat\": \"4\"}",
            "not a non-negative integer",
        ),
        (
            "{\"cells\": [], \"nbformat\": 4.0}",
            "not a non-negative integer",
        ),
        (
            "{\"cells\": [], \"nbformat\": 5}",
            "format 5 is not supported",
        ),
        (
            "{\"cells\": [], \"nbformat\": 4, \"nbformat_minor\": \"5\"}",
            "`nbformat_minor` is not a non-negative integer",
        ),
        (
            "{\"cells\": [], \"nbformat\": 4, \"nbformat_minor\": -1}",
            "`nbformat_minor` is not a non-negative integer",
        ),
        ("{\"nbformat\": 4}", "missing `cells`"),
        (
            "{\"cells\": [{\"source\": \"x\"}], \"nbformat\": 4}",
            "cell 0 has no cell_type",
        ),
        (
            "{\"cells\": [{\"cell_type\": 5}], \"nbformat\": 4}",
            "cell 0 has a cell_type that is not text",
        ),
        (
            "{\"cells\": [{\"cell_type\": \"code\", \"outputs\": {}}], \"nbformat\": 4}",
            "cell 0 has outputs that are not a list",
        ),
        (
            "{\"cells\": [{\"cell_type\": \"code\", \"source\": [1]}], \"nbformat\": 4}",
            "cell 0 has a source that is neither text nor a list of text",
        ),
    ];
    for (text, message) in cases {
        let error = Notebook::parse(text.as_bytes()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format, "{text:?}: {error}");
        assert!(error.to_string().contains(message), "{text:?}: {error}");
    }
}

#[test]
fn invalid_utf8_reports_the_offset_of_the_first_bad_byte() {
    let mut bytes = b"{\"cells\": [], \"nbformat\": 4, \"k\": \"caf".to_vec();
    let offset = bytes.len();
    bytes.extend_from_slice(&[0xc3, 0x28]);
    bytes.extend_from_slice(b"\"}");
    let error = Notebook::parse(&bytes).unwrap_err();
    assert_eq!(error, NotebookError::NotUtf8 { offset });
}

#[test]
fn syntax_errors_report_the_line_and_column() {
    let error = Notebook::parse(b"{\n \"cells\": [],\n \"nbformat\": 4,\n}").unwrap_err();
    match error {
        NotebookError::InvalidJson { line, column, .. } => {
            assert_eq!(line, 4);
            assert!(column >= 1);
        }
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn a_cell_id_that_is_not_text_is_replaced_by_normalize() {
    let text = "{\n \"cells\": [\n  {\n   \"cell_type\": \"code\",\n   \"execution_count\": null,\n   \"id\": 5,\n   \"metadata\": {},\n   \"outputs\": [],\n   \"source\": []\n  }\n ],\n \"metadata\": {},\n \"nbformat\": 4,\n \"nbformat_minor\": 5\n}\n";
    let mut notebook = Notebook::parse(text.as_bytes()).unwrap();
    notebook.set_id_generator(IdGenerator::sequential());
    assert_eq!(notebook.cell(0).unwrap().id, None);
    let outcome = notebook.normalize();
    assert_eq!((outcome.cells, outcome.notebook), (vec![0], false));
    assert_eq!(
        notebook.render(),
        text.replace("\"id\": 5", "\"id\": \"00000001\"")
    );
}

#[test]
fn random_identifiers_of_new_cells_are_unique_and_valid() {
    let mut notebook = Notebook::parse(render(&sample(), " ").as_bytes()).unwrap();
    let mut seen: HashSet<String> = (0..notebook.len())
        .filter_map(|index| notebook.cell(index).unwrap().id)
        .collect();
    for _ in 0..200 {
        let inserted = notebook
            .insert(&Position::End, CellType::Code, "x")
            .unwrap();
        let id = inserted.id.unwrap();
        assert_eq!(id.len(), 8, "{id}");
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()), "{id}");
        assert!(seen.insert(id), "an identifier was used twice");
    }
}

#[test]
fn positions_and_ranges_outside_the_notebook_change_nothing() {
    let mut notebook = Notebook::parse(render(&sample(), " ").as_bytes()).unwrap();
    for position in [
        Position::Index(-1),
        Position::Index(5),
        Position::Index(i64::MIN),
    ] {
        let error = notebook.insert(&position, CellType::Code, "x").unwrap_err();
        assert!(
            matches!(error, NotebookError::IndexOutOfRange { .. }),
            "{position:?}"
        );
    }
    assert!(
        notebook
            .insert(&Position::Index(4), CellType::Code, "x")
            .is_ok()
    );
    let mut notebook = Notebook::parse(render(&sample(), " ").as_bytes()).unwrap();
    for to in [-1, 4, i64::MAX] {
        assert!(notebook.move_cell(&CellRef::Index(0), to).is_err(), "{to}");
    }
    for to in [-1, 5, i64::MAX] {
        assert!(
            notebook.copy_cell(&CellRef::Index(0), Some(to)).is_err(),
            "{to}"
        );
    }
    for index in [-1, 4, i64::MIN, i64::MAX] {
        let selection = Selection::Range(Range::single(index));
        assert!(notebook.clear_outputs(&selection).is_err(), "{index}");
        let change = Replace {
            target: Some(CellRef::Index(index)),
            source: Some("x".into()),
            ..Replace::default()
        };
        assert!(notebook.replace(&change).is_err(), "{index}");
    }
    let error = notebook
        .remove(&Selection::Range(Range::from_start(-1)))
        .unwrap_err();
    assert_eq!(error, NotebookError::IndexOutOfRange { index: -1, len: 4 });
    assert!(!notebook.is_modified());
    assert_eq!(notebook.render(), render(&sample(), " "));
}

#[test]
fn a_repeated_cells_member_is_a_format_error() {
    let text = "{\"cells\": [], \"cells\": [], \"nbformat\": 4, \"nbformat_minor\": 5}";
    let error = Notebook::parse(text.as_bytes()).unwrap_err();
    assert!(matches!(error, NotebookError::Structure { .. }), "{error}");
    assert_eq!(error.code(), ErrorCode::Format);
}

#[test]
fn json_deeper_than_the_reader_limit_is_invalid_json() {
    let nested = |depth: usize| {
        format!(
            "{{\"cells\": [], \"nbformat\": 4, \"x\": {}{}}}",
            "[".repeat(depth),
            "]".repeat(depth)
        )
    };
    assert!(Notebook::parse(nested(100).as_bytes()).is_ok());
    let error = Notebook::parse(nested(200).as_bytes()).unwrap_err();
    assert!(
        matches!(error, NotebookError::InvalidJson { .. }),
        "{error}"
    );
    assert_eq!(error.code(), ErrorCode::Format);
}

#[test]
fn duplicate_ids_do_not_fail_parsing_so_that_normalize_can_repair_them() {
    let text = "{\"cells\": [\
        {\"cell_type\": \"markdown\", \"id\": \"a\", \"metadata\": {}, \"source\": \"x\"},\
        {\"cell_type\": \"markdown\", \"id\": \"a\", \"metadata\": {}, \"source\": \"y\"}\
        ], \"metadata\": {}, \"nbformat\": 4, \"nbformat_minor\": 5}";
    let mut notebook = Notebook::parse(text.as_bytes()).unwrap();
    let repaired = notebook.normalize();
    assert_eq!(repaired.cells, [1]);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
