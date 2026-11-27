// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-text/tests/search_positions.rs
// Responsibility:   Positions, line endings, context and limits of the search of a slice and.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Positions, line endings, context and limits of the search of a slice and
//! of a reader.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

use std::ops::ControlFlow;

use nurt_contracts::record::RegexEngine;
use nurt_text::search::{Pattern, SearchOptions, Searcher};

/// Match found in a test: line, column, byte offset and pattern index.
type Found = (u64, u64, u64, usize);

fn searcher(patterns: &[Pattern], options: SearchOptions) -> Searcher {
    Searcher::new(patterns, options).unwrap()
}

fn in_slice(searcher: &Searcher, haystack: &[u8]) -> Vec<Found> {
    let mut found = Vec::new();
    searcher.search_slice(haystack, |hit| {
        for item in hit.matches() {
            found.push((item.line, item.column, item.start, item.pattern));
        }
        ControlFlow::Continue(())
    });
    found
}

fn in_reader(searcher: &Searcher, haystack: &[u8]) -> Vec<Found> {
    let mut found = Vec::new();
    searcher
        .search_reader(haystack, |hit| {
            for item in hit.matches() {
                found.push((item.line, item.column, item.start, item.pattern));
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    found
}

#[test]
fn dollar_matches_before_a_crlf_line_ending() {
    let searcher = searcher(&[Pattern::regex("a$")], SearchOptions::new());
    let haystack = b"alpha\r\nbeta\r\n";
    let expected = vec![(1, 5, 4, 0), (2, 4, 10, 0)];
    assert_eq!(in_slice(&searcher, haystack), expected);
    assert_eq!(in_reader(&searcher, haystack), expected);
}

#[test]
fn columns_count_characters_and_offsets_count_bytes() {
    let searcher = searcher(&[Pattern::literal("x")], SearchOptions::new());
    let haystack = "\u{17c}\u{f3}\u{142}\u{107} x\n".as_bytes();
    assert_eq!(in_slice(&searcher, haystack), vec![(1, 6, 9, 0)]);
}

#[test]
fn several_patterns_give_the_same_matches_in_a_slice_and_in_a_reader() {
    let searcher = searcher(
        &[Pattern::literal("foo"), Pattern::regex("ba+r")],
        SearchOptions::new(),
    );
    let haystack = b"foo\nxx\nbaaar foo\nzzz\n";
    let expected = vec![(1, 1, 0, 0), (3, 1, 7, 1), (3, 7, 13, 0)];
    assert_eq!(in_slice(&searcher, haystack), expected);
    assert_eq!(in_reader(&searcher, haystack), expected);
}

#[test]
fn case_folding_follows_unicode() {
    let options = SearchOptions::new().case_sensitive(false);
    let searcher = searcher(&[Pattern::literal("\u{17c}\u{f3}\u{142}w")], options);
    let haystack = "\u{17b}\u{d3}\u{141}W\n".as_bytes();
    assert_eq!(in_slice(&searcher, haystack), vec![(1, 1, 0, 0)]);
}

#[test]
fn whole_word_rejects_matches_inside_words() {
    let options = SearchOptions::new().whole_word(true);
    let searcher = searcher(&[Pattern::literal("cat")], options);
    let found = in_slice(&searcher, b"concat cat cats\n");
    assert_eq!(found, vec![(1, 8, 7, 0)]);
}

#[test]
fn engines_are_reported_per_pattern() {
    let searcher = searcher(
        &[Pattern::literal("a.b"), Pattern::regex(r"(\w)\1")],
        SearchOptions::new(),
    );
    assert_eq!(searcher.engine(0), Some(RegexEngine::Linear));
    assert_eq!(searcher.engine(1), Some(RegexEngine::Backtracking));
    assert_eq!(searcher.engine(2), None);
    assert_eq!(searcher.pattern_count(), 2);
}

#[test]
fn max_per_file_limits_the_hits_and_reports_it() {
    let options = SearchOptions::new().max_per_file(Some(1));
    let searcher = searcher(&[Pattern::literal("a")], options);
    let mut lines = Vec::new();
    let summary = searcher.search_slice(b"a\na\na\n", |hit| {
        lines.push(hit.line());
        ControlFlow::Continue(())
    });
    assert_eq!(lines, [1]);
    assert_eq!(summary.hits, 1);
    assert!(summary.limit_reached);
}

#[test]
fn invert_reports_the_lines_without_a_match() {
    let searcher = searcher(&[Pattern::literal("b")], SearchOptions::new().invert(true));
    let mut lines = Vec::new();
    searcher.search_slice(b"a\nb\nc\n", |hit| {
        assert!(hit.matches().is_empty());
        lines.push(hit.line());
        ControlFlow::Continue(())
    });
    assert_eq!(lines, [1, 3]);
}

#[test]
fn context_lines_surround_a_hit() {
    let searcher = searcher(&[Pattern::literal("c")], SearchOptions::new().context(1));
    let mut seen = Vec::new();
    searcher.search_slice(b"a\nb\nc\nd\ne\n", |hit| {
        seen.push((
            hit.before()[0].text().into_owned(),
            hit.text().into_owned(),
            hit.after()[0].text().into_owned(),
        ));
        ControlFlow::Continue(())
    });
    assert_eq!(seen, [("b".to_owned(), "c".to_owned(), "d".to_owned())]);
}

#[test]
fn long_lines_are_shortened_around_the_match() {
    let searcher = searcher(
        &[Pattern::literal("needle")],
        SearchOptions::new().max_line(20),
    );
    let line = format!("{}needle{}\n", "x".repeat(60), "y".repeat(60));
    let mut shortened = Vec::new();
    let summary = searcher.search_slice(line.as_bytes(), |hit| {
        assert!(hit.is_truncated());
        shortened.push(hit.text().into_owned());
        ControlFlow::Continue(())
    });
    assert_eq!(summary.truncated_lines, 1);
    assert!(shortened[0].contains("needle"), "{}", shortened[0]);
    assert!(
        shortened[0].contains("characters omitted"),
        "{}",
        shortened[0]
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
