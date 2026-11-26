// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::io;
use std::ops::ControlFlow;
use std::time::Instant;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::RegexEngine;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

use super::{Match, Pattern, SearchError, SearchOptions, Searcher, Summary};

/// Hit copied out of the sink for comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    line: u64,
    end_line: u64,
    start: u64,
    text: String,
    matches: Vec<Match>,
    before: Vec<(u64, String)>,
    after: Vec<(u64, String)>,
}

fn searcher(patterns: &[Pattern], options: SearchOptions) -> Searcher {
    Searcher::new(patterns, options).unwrap()
}

fn search_slice(searcher: &Searcher, haystack: &[u8]) -> (Vec<Seen>, Summary) {
    let mut seen = Vec::new();
    let summary = searcher.search_slice(haystack, |hit| {
        seen.push(Seen {
            line: hit.line(),
            end_line: hit.end_line(),
            start: hit.start(),
            text: hit.text().into_owned(),
            matches: hit.matches().to_vec(),
            before: hit
                .before()
                .iter()
                .map(|l| (l.line(), l.text().into_owned()))
                .collect(),
            after: hit
                .after()
                .iter()
                .map(|l| (l.line(), l.text().into_owned()))
                .collect(),
        });
        ControlFlow::Continue(())
    });
    (seen, summary)
}

fn search_reader(searcher: &Searcher, haystack: &[u8]) -> (Vec<Seen>, Summary) {
    let mut seen = Vec::new();
    let summary = searcher
        .search_reader(haystack, |hit| {
            seen.push(Seen {
                line: hit.line(),
                end_line: hit.end_line(),
                start: hit.start(),
                text: hit.text().into_owned(),
                matches: hit.matches().to_vec(),
                before: hit
                    .before()
                    .iter()
                    .map(|l| (l.line(), l.text().into_owned()))
                    .collect(),
                after: hit
                    .after()
                    .iter()
                    .map(|l| (l.line(), l.text().into_owned()))
                    .collect(),
            });
            ControlFlow::Continue(())
        })
        .unwrap();
    (seen, summary)
}

/// Searches with both entry points and checks that they agree.
fn search(searcher: &Searcher, haystack: &[u8]) -> (Vec<Seen>, Summary) {
    let from_slice = search_slice(searcher, haystack);
    let from_reader = search_reader(searcher, haystack);
    assert_eq!(
        from_slice, from_reader,
        "slice and reader differ for {haystack:?}"
    );
    from_slice
}

fn matched_texts(searcher: &Searcher, haystack: &[u8]) -> Vec<String> {
    let mut texts = Vec::new();
    searcher.search_slice(haystack, |hit| {
        for found in hit.matches() {
            texts.push(String::from_utf8_lossy(hit.matched(found).unwrap()).into_owned());
        }
        ControlFlow::Continue(())
    });
    texts
}

fn lines_of(seen: &[Seen]) -> Vec<u64> {
    seen.iter().map(|hit| hit.line).collect()
}

fn pattern_error(pattern: Pattern) -> SearchError {
    Searcher::new(&[pattern], SearchOptions::new()).unwrap_err()
}

#[test]
fn literal_pattern_ignores_regex_metacharacters() {
    let s = searcher(&[Pattern::literal("a.b")], SearchOptions::new());
    let (seen, _) = search(&s, b"axb\na.b\n");
    assert_eq!(lines_of(&seen), [2]);
}

#[test]
fn regex_supports_classes_and_counted_repetition() {
    let s = searcher(&[Pattern::regex(r"[a-z]+-\d{2}\b")], SearchOptions::new());
    assert_eq!(matched_texts(&s, b"id abc-12 x-123 Q-45"), ["abc-12"]);
}

#[test]
fn unicode_property_matches_letters_of_any_script() {
    let s = searcher(&[Pattern::regex(r"\p{Greek}+")], SearchOptions::new());
    let (seen, _) = search(&s, "alpha αβγ".as_bytes());
    let found = seen[0].matches[0];
    assert_eq!(
        (found.column, found.end_column, found.start, found.end),
        (7, 10, 6, 12)
    );
    assert_eq!(found.engine, RegexEngine::Linear);
}

#[test]
fn named_groups_accept_both_syntaxes() {
    let s = searcher(
        &[Pattern::regex(
            r"(?<year>\d{4})-(?P<month>\d{2})(-(\d{2}))?",
        )],
        SearchOptions::new(),
    );
    let mut groups = Vec::new();
    s.search_slice(b"on 2026-11 and", |hit| {
        let captures = s.captures(hit, &hit.matches()[0]).unwrap();
        groups.push((
            captures.name("year").map(|g| g.text().into_owned()),
            captures.name("month").map(|g| (g.start, g.end)),
            captures.len(),
            captures.get(3).is_none(),
            captures
                .named()
                .map(|(name, group)| (name.to_owned(), group.is_some()))
                .collect::<Vec<_>>(),
        ));
        ControlFlow::Continue(())
    });
    assert_eq!(
        groups,
        [(
            Some("2026".to_owned()),
            Some((8, 10)),
            5,
            true,
            vec![("year".to_owned(), true), ("month".to_owned(), true)]
        )]
    );
}

#[test]
fn look_ahead_uses_backtracking_engine() {
    let s = searcher(&[Pattern::regex("foo(?=bar)")], SearchOptions::new());
    assert_eq!(s.engine(0), Some(RegexEngine::Backtracking));
    let (seen, _) = search(&s, b"foobaz\nfoobar\n");
    assert_eq!(lines_of(&seen), [2]);
    assert_eq!(seen[0].matches[0].engine, RegexEngine::Backtracking);
    assert_eq!((seen[0].matches[0].start, seen[0].matches[0].end), (7, 10));
}

#[test]
fn look_behind_uses_backtracking_engine() {
    let s = searcher(&[Pattern::regex(r"(?<=\$)\d+")], SearchOptions::new());
    assert_eq!(matched_texts(&s, b"12 $34 5$6"), ["34", "6"]);
}

#[test]
fn back_reference_uses_backtracking_engine_with_captures() {
    let s = searcher(&[Pattern::regex(r"(?<c>\w)\k<c>")], SearchOptions::new());
    assert_eq!(matched_texts(&s, b"hello\nbook"), ["ll", "oo"]);
    let mut letters = Vec::new();
    s.search_slice(b"x\nhello", |hit| {
        let captures = s.captures(hit, &hit.matches()[0]).unwrap();
        letters.push(captures.name("c").map(|g| (g.text().into_owned(), g.start)));
        ControlFlow::Continue(())
    });
    assert_eq!(letters, [Some(("l".to_owned(), 4))]);
}

#[test]
fn look_around_does_not_see_neighbouring_lines() {
    let s = searcher(&[Pattern::regex(r"a(?=\s*b)")], SearchOptions::new());
    let (seen, _) = search(&s, b"a\nb\na b\n");
    assert_eq!(lines_of(&seen), [3]);
}

#[test]
fn backtracking_step_limit_treats_text_as_not_matching() {
    // The nested repetition before the back-reference makes the number of
    // steps grow exponentially with the run of `a`.
    let options = SearchOptions::new().backtrack_limit(1_000);
    let s = searcher(&[Pattern::regex(r"(a*)*\1b")], options);
    let (seen, summary) = search(&s, b"aaaaaaaaaaaaaaaaaaaaaaaaaaaac\nab\n");
    assert_eq!(lines_of(&seen), [2]);
    assert!(summary.backtrack_limit_hits >= 1);
}

#[test]
fn invalid_pattern_reports_code_and_position() {
    let error = pattern_error(Pattern::regex("ab(c"));
    assert_eq!(error.code(), ErrorCode::Regex);
    assert!(
        matches!(
            error,
            SearchError::Pattern {
                pattern: 0,
                position: Some(3),
                ..
            }
        ),
        "{error:?}"
    );
    assert!(
        error
            .to_string()
            .starts_with("invalid regular expression in pattern 0 at character 3: ")
    );

    let error = pattern_error(Pattern::regex("\u{17c}\u{f3}\u{142}w[z-a]"));
    assert!(
        matches!(
            error,
            SearchError::Pattern {
                position: Some(6),
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn invalid_backtracking_pattern_reports_error_of_that_engine() {
    let error = pattern_error(Pattern::regex(r"(?=a)\2"));
    assert_eq!(error.code(), ErrorCode::Regex);
    let SearchError::Pattern { message, .. } = &error else {
        panic!("{error:?}")
    };
    assert!(message.starts_with("invalid back reference"), "{message}");
}

#[test]
fn error_names_the_index_of_the_failing_pattern() {
    let error = Searcher::new(
        &[Pattern::literal("ok"), Pattern::regex("(")],
        SearchOptions::new(),
    )
    .unwrap_err();
    assert!(
        matches!(error, SearchError::Pattern { pattern: 1, .. }),
        "{error:?}"
    );
}

#[test]
fn oversized_pattern_maps_to_limit_code() {
    let error = pattern_error(Pattern::regex(r"(?:\w{1000}){1000}"));
    assert_eq!(error.code(), ErrorCode::Limit);
    assert!(
        matches!(error, SearchError::TooLarge { pattern: 0, .. }),
        "{error:?}"
    );
}

#[test]
fn regex_flags_set_case_dot_and_extended_mode() {
    let flags = "isx".parse().unwrap();
    let s = searcher(
        &[Pattern::regex_with_flags(
            "a . b  # letters around any character",
            flags,
        )],
        SearchOptions::new().multiline(true),
    );
    let (seen, _) = search(&s, b"xA\nBy\n");
    assert_eq!((seen[0].line, seen[0].end_line), (1, 2));
}

#[test]
fn case_insensitive_search_folds_unicode_letters() {
    let options = SearchOptions::new().case_sensitive(false);
    let s = searcher(
        &[
            Pattern::literal("\u{142}\u{f3}d\u{17a}"),
            Pattern::regex("\u{3a3}\u{39f}\u{3a6}\u{399}\u{391}"),
        ],
        options,
    );
    assert_eq!(
        matched_texts(
            &s,
            "\u{141}\u{d3}D\u{179} i \u{141}\u{f3}d\u{17a}\n\u{3c3}\u{3bf}\u{3c6}\u{3b9}\u{3b1}"
                .as_bytes()
        ),
        [
            "\u{141}\u{d3}D\u{179}",
            "\u{141}\u{f3}d\u{17a}",
            "\u{3c3}\u{3bf}\u{3c6}\u{3b9}\u{3b1}"
        ]
    );
}

#[test]
fn whole_word_uses_unicode_word_characters() {
    let s = searcher(
        &[Pattern::literal("kot")],
        SearchOptions::new().whole_word(true),
    );
    let (seen, _) = search(&s, "kot, kotek, \u{17c}kot, kot_ (kot)".as_bytes());
    let columns: Vec<_> = seen[0].matches.iter().map(|m| m.column).collect();
    assert_eq!(columns, [1, 25]);
}

#[test]
fn whole_word_applies_to_regex_and_to_patterns_starting_with_punctuation() {
    let options = SearchOptions::new().whole_word(true);
    let s = searcher(&[Pattern::regex(r"\d+"), Pattern::literal("-x")], options);
    assert_eq!(matched_texts(&s, b"12 a34 56 a-x b -x"), ["12", "56", "-x"]);
    let s = searcher(
        &[Pattern::regex(r"(\w)\1")],
        SearchOptions::new().whole_word(true),
    );
    assert_eq!(matched_texts(&s, b"ll hello"), ["ll"]);
}

#[test]
fn positions_count_bytes_from_start_and_characters_in_line() {
    let s = searcher(&[Pattern::literal("foo")], SearchOptions::new());
    let (seen, _) = search(&s, "ab\nza\u{17c}\u{f3}\u{142}\u{107} foo".as_bytes());
    let found = seen[0].matches[0];
    assert_eq!(
        (found.line, found.column, found.start, found.end),
        (2, 8, 14, 17)
    );
    assert_eq!((found.end_line, found.end_column), (2, 11));
    assert_eq!(seen[0].start, 3);
}

#[test]
fn invalid_utf8_sequence_counts_as_one_column() {
    let s = searcher(&[Pattern::literal("foo")], SearchOptions::new());
    let (seen, _) = search(&s, b"\xff\xfe foo");
    assert_eq!(
        (seen[0].matches[0].column, seen[0].matches[0].start),
        (4, 3)
    );
}

#[test]
fn crlf_line_endings_are_not_part_of_the_line() {
    let s = searcher(&[Pattern::regex("o$")], SearchOptions::new());
    let (seen, _) = search(&s, b"no\r\nfoo\r\n");
    assert_eq!(
        seen.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(),
        ["no", "foo"]
    );
    assert_eq!(seen[1].matches[0].start, 6);
}

#[test]
fn match_never_crosses_a_line_without_multiline() {
    let s = searcher(
        &[Pattern::regex(r"b\s+c"), Pattern::literal("d\ne")],
        SearchOptions::new(),
    );
    let (seen, summary) = search(&s, b"b\nc d\ne b  c\n");
    assert_eq!(lines_of(&seen), [3]);
    assert_eq!(summary.matches, 1);
}

#[test]
fn multiline_match_reports_last_line_and_column() {
    let s = searcher(
        &[Pattern::regex(r"b\nc")],
        SearchOptions::new().multiline(true),
    );
    let (seen, _) = search(&s, b"a\nb\nc\nd");
    assert_eq!(
        (seen[0].line, seen[0].end_line, seen[0].text.as_str()),
        (2, 3, "b\nc")
    );
    let found = seen[0].matches[0];
    assert_eq!(
        (found.line, found.column, found.end_line, found.end_column),
        (2, 1, 3, 2)
    );
}

#[test]
fn multiline_matches_on_shared_lines_form_one_hit() {
    let s = searcher(
        &[Pattern::regex(r"(?s)<.*?>")],
        SearchOptions::new().multiline(true),
    );
    let (seen, summary) = search(&s, b"x <a\nb> <c\nd>\ny\n<e>\n");
    assert_eq!(
        seen.iter()
            .map(|h| (h.line, h.end_line))
            .collect::<Vec<_>>(),
        [(1, 3), (5, 5)]
    );
    assert_eq!((summary.hits, summary.matches), (2, 3));
    let second = seen[0].matches[1];
    assert_eq!(
        (
            second.line,
            second.column,
            second.end_line,
            second.end_column
        ),
        (2, 4, 3, 3)
    );
}

#[test]
fn multiline_invert_skips_every_line_of_a_match() {
    let options = SearchOptions::new().multiline(true).invert(true);
    let s = searcher(&[Pattern::regex(r"b\nc")], options);
    let (seen, _) = search(&s, b"a\nb\nc\nd\n");
    assert_eq!(lines_of(&seen), [1, 4]);
}

#[test]
fn context_stops_at_start_and_end_of_input() {
    let s = searcher(&[Pattern::regex("^[13]$")], SearchOptions::new().context(2));
    let (seen, _) = search(&s, b"1\n2\n3");
    assert_eq!(seen[0].before, []);
    assert_eq!(seen[0].after, [(2, "2".to_owned())]);
    assert_eq!(seen[1].before, []);
    assert_eq!(seen[1].after, []);
}

#[test]
fn context_lines_are_shown_once_between_close_hits() {
    let s = searcher(
        &[Pattern::literal("X")],
        SearchOptions::new().before(2).after(1),
    );
    let (seen, _) = search(&s, b"a\nb\nX\nc\nd\nX\ne\n");
    assert_eq!(seen[0].before, [(1, "a".to_owned()), (2, "b".to_owned())]);
    assert_eq!(seen[0].after, [(4, "c".to_owned())]);
    assert_eq!(seen[1].before, [(5, "d".to_owned())]);
    assert_eq!(seen[1].after, [(7, "e".to_owned())]);
}

#[test]
fn context_never_contains_another_hit() {
    let s = searcher(&[Pattern::literal("X")], SearchOptions::new().context(3));
    let (seen, _) = search(&s, b"X\nX\na\n");
    assert_eq!(seen[0].after, []);
    assert_eq!(seen[1].before, []);
    assert_eq!(seen[1].after, [(3, "a".to_owned())]);
}

#[test]
fn invert_reports_lines_without_a_match() {
    let s = searcher(
        &[Pattern::literal("foo")],
        SearchOptions::new().invert(true).context(1),
    );
    let (seen, summary) = search(&s, b"a\nfoo\n\nb\n");
    assert_eq!(lines_of(&seen), [1, 3, 4]);
    assert!(seen.iter().all(|hit| hit.matches.is_empty()));
    assert_eq!(seen[0].after, [(2, "foo".to_owned())]);
    assert_eq!((summary.hits, summary.matches), (3, 0));
}

#[test]
fn max_per_file_limits_hits_and_reports_the_cut() {
    let s = searcher(
        &[Pattern::literal("x")],
        SearchOptions::new().max_per_file(Some(2)).after(1),
    );
    let (seen, summary) = search(&s, b"x\nx\na\nx\n");
    assert_eq!(lines_of(&seen), [1, 2]);
    assert_eq!(seen[1].after, [(3, "a".to_owned())]);
    assert!(summary.limit_reached);

    let s = searcher(
        &[Pattern::literal("x")],
        SearchOptions::new().max_per_file(Some(3)),
    );
    let (_, summary) = search(&s, b"x\nx\na\nx\n");
    assert!(!summary.limit_reached);
}

#[test]
fn max_per_file_shows_following_hits_as_context_of_the_last_hit() {
    let s = searcher(
        &[Pattern::literal("x")],
        SearchOptions::new().max_per_file(Some(1)).after(2),
    );
    let (seen, summary) = search(&s, b"x\nx\na\nb\n");
    assert_eq!(seen[0].after, [(2, "x".to_owned()), (3, "a".to_owned())]);
    assert!(summary.limit_reached);
}

#[test]
fn only_matching_limit_counts_matches() {
    let options = SearchOptions::new()
        .only_matching(true)
        .max_per_file(Some(3));
    let s = searcher(&[Pattern::literal("ab")], options);
    let (seen, summary) = search(&s, b"ab ab\nab ab\nab\n");
    assert_eq!(
        seen.iter().map(|h| h.matches.len()).collect::<Vec<_>>(),
        [2, 1]
    );
    assert_eq!(
        (summary.hits, summary.matches, summary.limit_reached),
        (2, 3, true)
    );
}

#[test]
fn zero_max_per_file_means_no_limit() {
    let s = searcher(
        &[Pattern::literal("a")],
        SearchOptions::new().max_per_file(Some(0)),
    );
    let (seen, summary) = search(&s, b"a\na\na\n");
    assert_eq!(seen.len(), 3);
    assert!(!summary.limit_reached);
}

#[test]
fn long_line_is_shortened_around_the_first_match() {
    let line = format!("{}needle{}", "a".repeat(1000), "b".repeat(1000));
    let s = searcher(
        &[Pattern::literal("needle")],
        SearchOptions::new().max_line(40).before(1),
    );
    let haystack = format!("{}\n{line}\n", "c".repeat(50));
    let (seen, summary) = search(&s, haystack.as_bytes());
    let expected = format!(
        "[990 characters omitted] {}needle{} [976 characters omitted]",
        "a".repeat(10),
        "b".repeat(24)
    );
    assert_eq!(seen[0].text, expected);
    assert_eq!(
        seen[0].before[0].1,
        format!("{} [10 characters omitted]", "c".repeat(40))
    );
    assert_eq!(summary.truncated_lines, 2);
    assert_eq!(seen[0].matches[0].column, 1001);
}

#[test]
fn zero_max_line_keeps_lines_whole() {
    let line = "x".repeat(DEFAULT_LONG);
    let s = searcher(
        &[Pattern::literal("x")],
        SearchOptions::new().max_line(0).max_per_file(Some(1)),
    );
    let (seen, summary) = search(&s, line.as_bytes());
    assert_eq!(seen[0].text.len(), DEFAULT_LONG);
    assert_eq!(summary.truncated_lines, 0);
}

/// Length above the default `max_line`.
const DEFAULT_LONG: usize = 600;

#[test]
fn several_literals_report_pattern_index_and_counts() {
    let s = searcher(
        &[Pattern::literal("foo"), Pattern::literal("bar")],
        SearchOptions::new(),
    );
    let (seen, summary) = search(&s, b"bar foo bar\nnone\nfoo\n");
    let patterns: Vec<_> = seen
        .iter()
        .flat_map(|h| h.matches.iter().map(|m| m.pattern))
        .collect();
    assert_eq!(patterns, [1, 0, 1, 0]);
    assert_eq!(summary.per_pattern, [2, 2]);
}

#[test]
fn lower_pattern_index_wins_at_the_same_start() {
    let s = searcher(
        &[
            Pattern::literal("ab"),
            Pattern::regex("abc"),
            Pattern::literal("b"),
        ],
        SearchOptions::new(),
    );
    let (seen, summary) = search(&s, b"abc");
    let found: Vec<_> = seen[0]
        .matches
        .iter()
        .map(|m| (m.pattern, m.start, m.end))
        .collect();
    assert_eq!(found, [(0, 0, 2)]);
    assert_eq!(summary.per_pattern, [1, 0, 0]);
}

#[test]
fn mixed_engines_merge_matches_by_position() {
    let s = searcher(
        &[Pattern::regex(r"\d+(?=px)"), Pattern::literal("em")],
        SearchOptions::new(),
    );
    let found: Vec<_> = matched_texts(&s, b"1em 20px 3em");
    assert_eq!(found, ["em", "20", "em"]);
}

#[test]
fn empty_matches_advance_by_one_character() {
    let s = searcher(&[Pattern::regex("x*")], SearchOptions::new());
    let (seen, _) = search(&s, "a\u{17c}\n".as_bytes());
    let columns: Vec<_> = seen[0].matches.iter().map(|m| m.column).collect();
    assert_eq!(columns, [1, 2, 3]);
}

#[test]
fn start_anchor_matches_every_line_including_empty_ones() {
    let s = searcher(&[Pattern::regex("^")], SearchOptions::new());
    let (seen, _) = search(&s, b"a\n\nb\n");
    assert_eq!(lines_of(&seen), [1, 2, 3]);
}

#[test]
fn text_anchors_apply_to_each_line() {
    let s = searcher(&[Pattern::regex(r"\Ab")], SearchOptions::new());
    let (seen, _) = search(&s, b"a\nb\n");
    assert_eq!(lines_of(&seen), [2]);
}

#[test]
fn empty_input_has_no_hits() {
    let s = searcher(&[Pattern::regex("^")], SearchOptions::new().invert(true));
    let (seen, summary) = search(&s, b"");
    assert!(seen.is_empty());
    assert_eq!(summary, Summary::new(1));
}

#[test]
fn empty_pattern_list_matches_nothing() {
    let s = searcher(&[], SearchOptions::new());
    let (seen, _) = search(&s, b"a\n");
    assert!(seen.is_empty());
}

#[test]
fn sink_can_stop_the_search() {
    let s = searcher(&[Pattern::literal("a")], SearchOptions::new());
    for entry in [true, false] {
        let mut lines = Vec::new();
        let mut sink = |hit: &super::Hit<'_>| {
            lines.push(hit.line());
            ControlFlow::Break(())
        };
        let summary = if entry {
            s.search_slice(b"a\na\n", &mut sink)
        } else {
            s.search_reader(&b"a\na\n"[..], &mut sink).unwrap()
        };
        assert_eq!(lines, [1]);
        assert_eq!(summary.hits, 1);
    }
}

#[test]
fn read_failure_maps_to_io_code() {
    struct Failing;
    impl io::Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("device error"))
        }
    }
    let s = searcher(&[Pattern::literal("a")], SearchOptions::new());
    let error = s
        .search_reader(Failing, |_| ControlFlow::Continue(()))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Io);
    assert_eq!(
        error.to_string(),
        "cannot read the searched input: device error"
    );
}

#[test]
fn captures_of_a_literal_are_the_whole_match() {
    let s = searcher(&[Pattern::literal("b")], SearchOptions::new());
    let mut seen = Vec::new();
    s.search_slice(b"ab", |hit| {
        let captures = s.captures(hit, &hit.matches()[0]).unwrap();
        seen.push((
            captures.len(),
            captures.get(0).map(|g| (g.start, g.end)),
            captures.named().count(),
        ));
        ControlFlow::Continue(())
    });
    assert_eq!(seen, [(1, Some((1, 2)), 0)]);
}

/// Random inputs over a small alphabet, searched through a slice and a
/// reader with the same options, give the same hits and counters.
#[test]
fn slice_and_reader_agree_on_random_input() {
    let config = Config {
        cases: 512,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::deterministic_rng(RngAlgorithm::ChaCha));
    let pattern_sets = [
        vec![Pattern::literal("ab")],
        vec![Pattern::literal("a"), Pattern::literal("ba")],
        vec![Pattern::regex("a+b?"), Pattern::literal("b")],
        vec![Pattern::regex("^b|a$")],
        vec![Pattern::regex(r"\bab\b")],
    ];
    let strategy = (
        proptest::collection::vec(prop::sample::select(vec![b'a', b'b', b' ', b'\n']), 0..40),
        0..pattern_sets.len(),
        0usize..3,
        0usize..3,
        any::<bool>(),
        any::<bool>(),
        proptest::option::of(0u64..4),
    );
    runner
        .run(
            &strategy,
            |(haystack, set, before, after, invert, only_matching, max)| {
                let options = SearchOptions::new()
                    .before(before)
                    .after(after)
                    .invert(invert)
                    .only_matching(only_matching)
                    .max_per_file(max);
                let s = searcher(&pattern_sets[set], options);
                prop_assert_eq!(search_slice(&s, &haystack), search_reader(&s, &haystack));
                Ok(())
            },
        )
        .unwrap();
}

/// Size of the generated input of the throughput measurement.
const THROUGHPUT_BYTES: usize = 64 << 20;

/// Lowest accepted throughput of a release build, in mebibytes per second.
const MIN_THROUGHPUT_MIB_PER_SEC: f64 = 100.0;

#[test]
#[ignore = "measurement; run with --ignored in a release build"]
fn search_throughput_reaches_the_minimum() {
    let line = b"the quick brown fox jumps over the lazy dog 0123456789\n";
    let haystack: Vec<u8> = line
        .iter()
        .copied()
        .cycle()
        .take(THROUGHPUT_BYTES)
        .collect();
    let cases = [
        ("literal", Pattern::literal("needle")),
        ("regex", Pattern::regex(r"ne+dle\d")),
        ("backtracking", Pattern::regex(r"needle(?=x)")),
    ];
    for (name, pattern) in cases {
        let s = searcher(&[pattern], SearchOptions::new());
        let started = Instant::now();
        let summary = s.search_slice(&haystack, |_| ControlFlow::Continue(()));
        let seconds = started.elapsed().as_secs_f64();
        let throughput = THROUGHPUT_BYTES as f64 / f64::from(1 << 20) / seconds;
        assert!(
            throughput >= MIN_THROUGHPUT_MIB_PER_SEC,
            "{name}: {throughput:.0} MiB/s"
        );
        assert_eq!(summary.hits, 0);
    }
}
