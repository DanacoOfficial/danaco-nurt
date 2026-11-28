// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-bind/tests/program_fields.rs
// Responsibility:   Binding of the fields that the `program` commands add to their records, in `select`, `filter` and `sort`.
// Dependencies:     bind_support, nurt_contracts, nurt_syntax
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Binding of the fields that the `program` commands add to their records:
//! the examples of the command pages bind, and a field that no record has
//! is still refused.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod bind_support;

use bind_support::{bind_with, linux};
use nurt_bind::BindError;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_syntax::parse::{ParseOptions, parse};

/// Reads and binds a program on a Linux host.
fn bind_text(text: &str) -> Result<(), BindError> {
    let document = parse(text, SyntaxVariant::A, &ParseOptions::default())
        .unwrap_or_else(|error| panic!("`{text}` does not parse: {error:?}"));
    bind_with(&document, &linux()).map(drop)
}

fn assert_binds(text: &str) {
    if let Err(error) = bind_text(text) {
        panic!("`{text}` does not bind: {error}");
    }
}

#[test]
fn binds_the_version_source_of_program_find() {
    assert_binds(
        r#"program.find "python3" "node" versions: true |> select name version version_source"#,
    );
    assert_binds(r#"program.find "git" versions: true |> filter version_source == "package""#);
    assert_binds(r#"program.find "vi" |> select name alias_of"#);
}

#[test]
fn binds_the_guide_fields_of_program_describe() {
    assert_binds(
        r#"program.describe "ffmpeg" include_guide: false |> select name guide_sections guide_etag"#,
    );
}

#[test]
fn binds_the_alias_fields_of_program_describe() {
    assert_binds(r#"program.describe "vi" |> select name requested alias_kind path"#);
    assert_binds(r#"program.describe "vi" |> filter alias_kind == "symlink" |> select name"#);
}

#[test]
fn binds_the_query_field_of_program_search() {
    assert_binds(
        r#"program.search ["compress files", "edit images"] limit: 1 |> select query name"#,
    );
    assert_binds(r#"program.search "pdf merge" source: [path, package] |> select name path score"#);
}

#[test]
fn binds_the_added_fields_in_filter_and_sort() {
    assert_binds(r#"program.search "pdf" |> filter has_guide == true |> sort score order: desc"#);
    assert_binds(r#"program.search "pdf" |> filter kind == "guide" |> select name instead"#);
    assert_binds(
        r#"program.describe "ffmpeg" help: true |> select name help help_status executed"#,
    );
    assert_binds(r#"program.catalog |> sort name |> select name level"#);
    assert_binds(r#"program.catalog "video" |> select name level parent summary"#);
    assert_binds(r#"program.describe "tar" |> select when_to_use not_for example pitfalls man"#);
}

#[test]
fn still_refuses_a_field_that_no_program_record_has() {
    for text in [
        r#"program.find "git" |> select name colour"#,
        r#"program.describe "tar" |> filter colour == "red""#,
        r#"program.search "pdf" |> sort colour"#,
        r#"program.catalog |> select name colour"#,
    ] {
        let error = bind_text(text).expect_err(text);
        assert_eq!(error.code(), ErrorCode::UnknownField, "{text}");
    }
}
