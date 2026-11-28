// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-notebook/tests/error_report.rs
// Responsibility:   Conversion of notebook errors to language errors, messages of the variants and the addressing types.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Conversion of notebook errors to language errors, messages of the
//! variants and the addressing types.

use nurt_contracts::error::ErrorCode;
use nurt_notebook::{CellRef, NotebookError, Position, Selection};
use serde_json::json;

fn unknown_id(candidates: &[&str], notebook_has_ids: bool) -> NotebookError {
    NotebookError::UnknownId {
        id: "c9".to_owned(),
        candidates: candidates.iter().map(|id| (*id).to_owned()).collect(),
        notebook_has_ids,
    }
}

#[test]
fn reports_json_position_and_format() {
    let error = NotebookError::InvalidJson {
        line: 3,
        column: 7,
        detail: "expected value".to_owned(),
    }
    .to_error();
    assert_eq!(error.data().code, ErrorCode::Format);
    assert_eq!(error.data().details["format"], json!("ipynb"));
    assert_eq!(error.data().details["line"], json!(3));
    assert_eq!(error.data().details["column"], json!(7));
}

#[test]
fn reports_utf8_offset_and_structure_detail() {
    let utf8 = NotebookError::NotUtf8 { offset: 12 }.to_error();
    assert_eq!(utf8.data().details["offset"], json!(12));
    let structure = NotebookError::Structure {
        detail: "missing `cells`".to_owned(),
    }
    .to_error();
    assert_eq!(structure.data().details["detail"], json!("missing `cells`"));
}

#[test]
fn reports_repeated_identifier_as_found() {
    let error = NotebookError::DuplicateId {
        id: "c1".to_owned(),
    }
    .to_error();
    assert_eq!(error.data().found.as_deref(), Some("c1"));
}

#[test]
fn reports_valid_index_range_for_every_length() {
    let expected = |len| {
        NotebookError::IndexOutOfRange { index: 9, len }
            .to_error()
            .data()
            .expected
            .clone()
    };
    assert_eq!(expected(0), Some(json!("no cells")));
    assert_eq!(expected(1), Some(json!("0")));
    assert_eq!(expected(4), Some(json!("0..3")));
    let message = NotebookError::IndexOutOfRange { index: 9, len: 0 }.to_string();
    assert!(message.contains("no cells"), "{message}");
}

#[test]
fn reports_unknown_identifier_with_candidates() {
    let error = unknown_id(&["c1", "c2"], true).to_error();
    assert_eq!(error.data().code, ErrorCode::Arg);
    assert_eq!(error.data().found.as_deref(), Some("c9"));
    assert_eq!(error.data().details["candidates"], json!(["c1", "c2"]));
}

#[test]
fn words_unknown_identifier_by_available_help() {
    let without_ids = unknown_id(&[], false).to_string();
    assert!(without_ids.contains("has no cell ids"), "{without_ids}");
    let without_candidates = unknown_id(&[], true).to_string();
    assert!(
        without_candidates.contains("notebook.cells"),
        "{without_candidates}"
    );
    let with_candidates = unknown_id(&["c1", "c2"], true).to_string();
    assert!(
        with_candidates.contains("closest ids: c1, c2"),
        "{with_candidates}"
    );
}

#[test]
fn reports_address_mismatch_with_both_cells() {
    let error = NotebookError::AddressMismatch {
        index: 1,
        id: "c3".to_owned(),
        id_index: 2,
        index_id: Some("c2".to_owned()),
    }
    .to_error();
    assert_eq!(
        error.data().expected,
        Some(json!({ "index": 2, "id": "c3" }))
    );
    assert_eq!(error.data().actual, Some(json!({ "index": 1, "id": "c2" })));
}

#[test]
fn reports_unmatched_pattern_as_found() {
    let error = NotebookError::NoMatch {
        pattern: "needle".to_owned(),
    }
    .to_error();
    assert_eq!(error.data().code, ErrorCode::NoMatch);
    assert_eq!(error.data().found.as_deref(), Some("needle"));
}

#[test]
fn keeps_detail_text_of_invalid_change() {
    let change = NotebookError::InvalidChange {
        detail: "give a source or a pattern".to_owned(),
    };
    assert_eq!(change.to_string(), "give a source or a pattern");
    assert_eq!(change.to_error().data().code, ErrorCode::Arg);
}

#[test]
fn wraps_failed_change_with_its_position() {
    let wrapped = NotebookError::Change {
        position: 3,
        source: Box::new(NotebookError::NoMatch {
            pattern: "x".to_owned(),
        }),
    };
    assert_eq!(wrapped.code(), ErrorCode::NoMatch);
    assert_eq!(wrapped.to_error().data().details["change"], json!(3));
    assert!(wrapped.to_string().starts_with("change 3 failed"));
    let source = std::error::Error::source(&wrapped).map(ToString::to_string);
    assert!(source.is_some_and(|text| text.contains("occurs in no selected cell")));
    let plain = NotebookError::Version { nbformat: 3 };
    assert!(std::error::Error::source(&plain).is_none());
}

#[test]
fn builds_cell_reference_from_optional_parts() {
    let id = || Some("c1".to_owned());
    assert_eq!(
        CellRef::from_parts(Some(1), id()),
        Some(CellRef::IndexAndId(1, "c1".to_owned()))
    );
    assert_eq!(CellRef::from_parts(Some(1), None), Some(CellRef::Index(1)));
    assert_eq!(
        CellRef::from_parts(None, id()),
        Some(CellRef::Id("c1".to_owned()))
    );
    assert_eq!(CellRef::from_parts(None, None), None);
}

#[test]
fn inserts_at_end_by_default() {
    assert_eq!(Position::default(), Position::End);
    assert_ne!(Position::Index(0), Position::AfterId("c1".to_owned()));
    assert_ne!(Selection::All, Selection::Ids(vec!["c1".to_owned()]));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
