// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Formatting of program text with comments and conversion between
//! variants.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::error::ErrorCode;
use nurt_syntax::fmt;
use nurt_syntax::parse::{self, ParseOptions, Variant};
use nurt_syntax::print::{self, PositionalNames};

fn format_a(text: &str) -> String {
    let formatted = fmt::format(text, Variant::A).unwrap_or_else(|error| panic!("{error}\n{text}"));
    let again =
        fmt::format(&formatted, Variant::A).unwrap_or_else(|error| panic!("{error}\n{formatted}"));
    assert_eq!(again, formatted, "formatting is not idempotent");
    formatted
}

#[test]
fn without_comments_formatting_equals_printing() {
    let text = "find   \"/srv/app\" glob: \"**/*.go\" |> filter size > 1000kb and modified > ago(24h)\n\n\n|> first 0x14\n";
    let document = parse::parse(text, Variant::A, &ParseOptions::default()).expect("valid program");
    let formatted = format_a(text);
    assert_eq!(formatted, print::print(&document, Variant::A));
    assert_eq!(
        formatted,
        "find \"/srv/app\" glob: \"**/*.go\"\n|> filter size > 1mb and modified > ago(1d)\n|> first 20\n"
    );
}

#[test]
fn comments_stay_before_and_after_their_elements() {
    let text = "# head\nfind \"/srv\"   glob: \"*.go\" # find\n# before first\n|> first 20 # top\n\n# trailing\n";
    assert_eq!(
        format_a(text),
        "# head\nfind \"/srv\" glob: \"*.go\" # find\n# before first\n|> first 20 # top\n# trailing\n"
    );
}

#[test]
fn interpreter_line_stays_first() {
    let text = "#!/usr/bin/env nurt\n\njobs |> count\n";
    assert_eq!(format_a(text), "#!/usr/bin/env nurt\njobs |> count\n");
}

#[test]
fn end_of_line_comments_of_joined_lines_move_before_the_statement() {
    let text = "jobs # all jobs\n|> count # how many\n";
    assert_eq!(format_a(text), "# all jobs\njobs |> count # how many\n");
}

#[test]
fn comments_inside_a_joined_list_move_before_the_statement() {
    let text = "processes\n|> filter cpu in [\n  1, # one\n  # two\n  2\n]\n";
    assert_eq!(
        format_a(text),
        "# one\n# two\nprocesses |> filter cpu in [1, 2]\n"
    );
}

#[test]
fn comments_keep_their_text() {
    let text = "count #  spaced  text\n";
    assert_eq!(format_a(text), "count #  spaced  text\n");
}

#[test]
fn invalid_text_gives_the_parser_error() {
    let error = fmt::format("filter a < b < c\n", Variant::A).expect_err("invalid program");
    assert_eq!(error.code, ErrorCode::Syntax);
}

struct Names;

impl PositionalNames for Names {
    fn name(&self, command: &str, index: usize) -> Option<String> {
        let name = match (command, index) {
            ("find", 0) => "path",
            ("first", 0) => "count",
            _ => return None,
        };
        Some(name.to_owned())
    }
}

#[test]
fn converts_variant_a_to_variants_b_and_c_with_comments() {
    let text = "# largest\nfind \"/srv\" glob: \"*.go\"\n|> first 20 # top\n";
    assert_eq!(
        fmt::convert(text, Variant::A, Variant::B, None).expect("converts"),
        "# largest\nfind(\"/srv\", glob: \"*.go\")\n|> first(20) # top\n"
    );
    assert_eq!(
        fmt::convert(text, Variant::A, Variant::C, Some(&Names)).expect("converts"),
        "# largest\nfind path=\"/srv\" glob=\"*.go\"\n|> first count=20 # top\n"
    );
}

#[test]
fn conversion_to_variant_c_requires_parameter_names() {
    let text = "find \"/srv\" |> first 20\n";
    let error = fmt::convert(text, Variant::A, Variant::C, None).expect_err("positional arguments");
    assert_eq!(error.code, ErrorCode::Syntax);
    assert_eq!(error.problem.as_deref(), Some("positional_argument"));
    assert_eq!(
        error.at.map(|at| (at.line, at.column)),
        Some((Some(1), Some(6)))
    );
    let error = fmt::convert("sort size\n", Variant::A, Variant::C, Some(&Names))
        .expect_err("unknown name");
    assert_eq!(error.problem.as_deref(), Some("unnamed_argument"));
}
