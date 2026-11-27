// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-text/tests/edit_plan_changes.rs
// Responsibility:   Replacement, insertion and line deletion through `edit_plan::plan`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Replacement, insertion and line deletion through `edit_plan::plan`.

// Fixture builders outside `#[test]` functions unwrap constant patterns and
// ranges; a failure there is a defect of the test itself.
#![allow(clippy::unwrap_used)]

use std::num::NonZeroUsize;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::Range;
use nurt_text::edit_plan::{
    Change, EditError, EditOptions, InsertChange, Pattern, Position, ReplaceChange, plan,
};
use nurt_text::fuzzy_match::{MatchError, OnMissing, Tolerance, Whitespace};

fn literal(pattern: &str, with: &str) -> ReplaceChange {
    ReplaceChange::new(Pattern::Literal(pattern.to_owned()), with)
}

fn regex(pattern: &str, with: &str) -> ReplaceChange {
    ReplaceChange::new(Pattern::Regex(regex::Regex::new(pattern).unwrap()), with)
}

fn backtracking(pattern: &str, with: &str) -> ReplaceChange {
    ReplaceChange::new(
        Pattern::Backtracking(fancy_regex::Regex::new(pattern).unwrap()),
        with,
    )
}

fn insert(text: &str, line: Option<i64>, position: Position) -> Change {
    Change::Insert(InsertChange {
        text: text.to_owned(),
        line,
        position,
    })
}

fn lines(start: Option<i64>, end: Option<i64>) -> Change {
    Change::DeleteLines(Range::new(start, end).unwrap())
}

fn run(text: &str, changes: Vec<Change>) -> Result<String, EditError> {
    plan(text, &changes, &EditOptions::default()).map(|edit| edit.text)
}

fn replace(text: &str, change: ReplaceChange) -> Result<String, EditError> {
    run(text, vec![Change::Replace(change)])
}

fn error_of(result: Result<String, EditError>) -> EditError {
    result.unwrap_err()
}

#[test]
fn replaces_literal_fragment() {
    assert_eq!(
        replace("let a = 1;\n", literal("a = 1", "a = 2")).unwrap(),
        "let a = 2;\n"
    );
}

#[test]
fn empty_replacement_deletes_fragment() {
    assert_eq!(
        replace("keep drop keep", literal(" drop", "")).unwrap(),
        "keep keep"
    );
}

#[test]
fn replaces_all_matches_by_default() {
    assert_eq!(replace("a-a-a", literal("a", "b")).unwrap(), "b-b-b");
}

#[test]
fn reports_every_replaced_match() {
    let changes = [Change::Replace(literal("x", "y"))];
    let edit = plan("x\n x\n", &changes, &EditOptions::default()).unwrap();
    let positions: Vec<(usize, usize)> = edit.outcomes[0]
        .matches
        .iter()
        .map(|found| (found.line, found.column))
        .collect();
    assert_eq!(edit.outcomes[0].count, 2);
    assert_eq!(positions, [(1, 1), (2, 2)]);
}

#[test]
fn replaces_selected_occurrence_only() {
    let change = ReplaceChange {
        occurrence: NonZeroUsize::new(2),
        ..literal("a", "b")
    };
    assert_eq!(replace("a-a-a", change).unwrap(), "a-b-a");
}

#[test]
fn ambiguous_pattern_with_expect_one_lists_candidates() {
    let change = ReplaceChange {
        expect: Some(Range::single(1)),
        ..literal("value", "result")
    };
    let error = error_of(replace("value\n  value\n", change));
    assert_eq!(error.code(), ErrorCode::Assert);
    let EditError::Match {
        change: 0,
        source:
            MatchError::CountMismatch {
                candidates,
                found: 2,
                ..
            },
    } = &error
    else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(candidates.len(), 2);
    assert_eq!((candidates[1].line, candidates[1].column), (2, 3));
    assert!(
        error
            .to_string()
            .starts_with("change 1: the pattern occurs 2 times, expected 1")
    );
}

#[test]
fn expected_count_accepts_matching_number() {
    let change = ReplaceChange {
        expect: Some(Range::single(3)),
        ..literal("a", "b")
    };
    assert_eq!(replace("a a a", change).unwrap(), "b b b");
}

#[test]
fn expected_count_mismatch_changes_nothing() {
    let change = ReplaceChange {
        expect: Some(Range::new(Some(4), None).unwrap()),
        ..literal("a", "b")
    };
    assert_eq!(error_of(replace("a a a", change)).code(), ErrorCode::Assert);
}

#[test]
fn missing_pattern_is_skipped_by_default() {
    let changes = [Change::Replace(literal("absent", "x"))];
    let edit = plan("text\n", &changes, &EditOptions::default()).unwrap();
    assert_eq!(edit.text, "text\n");
    assert_eq!(edit.outcomes[0].count, 0);
}

#[test]
fn missing_pattern_with_on_missing_error_lists_nearest_fragments() {
    let change = ReplaceChange {
        on_missing: Some(OnMissing::Error),
        ..literal("total = price * amount", "x")
    };
    let error = error_of(replace("let total = price * count;\nother();\n", change));
    assert_eq!(error.code(), ErrorCode::NoMatch);
    let EditError::Match {
        source: MatchError::NoMatch { nearest },
        ..
    } = &error
    else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(nearest[0].line, 1);
    assert_eq!(nearest[0].text, "let total = price * count;");
}

#[test]
fn missing_occurrence_is_out_of_range() {
    let change = ReplaceChange {
        occurrence: NonZeroUsize::new(3),
        on_missing: Some(OnMissing::Error),
        ..literal("a", "b")
    };
    let error = error_of(replace("a a", change));
    assert_eq!(error.code(), ErrorCode::OutOfRange);
}

#[test]
fn on_missing_of_the_edit_applies_to_changes_without_own_value() {
    let options = EditOptions {
        on_missing: OnMissing::Error,
        ..EditOptions::default()
    };
    let result = plan("abc", &[Change::Replace(literal("x", "y"))], &options);
    assert_eq!(result.unwrap_err().code(), ErrorCode::NoMatch);
}

#[test]
fn own_on_missing_overrides_the_edit() {
    let options = EditOptions {
        on_missing: OnMissing::Error,
        ..EditOptions::default()
    };
    let change = ReplaceChange {
        on_missing: Some(OnMissing::Skip),
        ..literal("x", "y")
    };
    assert_eq!(
        plan("abc", &[Change::Replace(change)], &options)
            .unwrap()
            .text,
        "abc"
    );
}

#[test]
fn normalize_tolerates_indentation_with_tabs_against_spaces() {
    let text = "fn f() {\n\tif ready {\n\t\tstart();\n\t}\n}\n";
    let change = literal(
        "    if ready {\n        start();\n    }",
        "    if ready {\n        prepare();\n        start();\n    }",
    );
    assert_eq!(
        replace(text, change).unwrap(),
        "fn f() {\n\tif ready {\n\t\tprepare();\n\t\tstart();\n\t}\n}\n"
    );
}

#[test]
fn normalize_tolerates_trailing_white_space() {
    let text = "first   \nsecond\t\nthird\n";
    assert_eq!(
        replace(text, literal("first\nsecond", "one\ntwo")).unwrap(),
        "one\ntwo\nthird\n"
    );
}

#[test]
fn normalize_tolerates_crlf_and_keeps_it() {
    let text = "a\r\nb\r\nc\r\n";
    assert_eq!(
        replace(text, literal("a\nb\n", "x\ny\nz\n")).unwrap(),
        "x\r\ny\r\nz\r\nc\r\n"
    );
}

#[test]
fn normalize_matches_mixed_indentation_and_writes_the_unit_of_the_text() {
    let text = "class A:\n\tdef f(self):\n\t    return 1\n";
    let change = literal("def f(self):\n    return 1", "def g(self):\n    return 2");
    let changes = [Change::Replace(change)];
    let edit = plan(text, &changes, &EditOptions::default()).unwrap();
    assert_eq!(edit.outcomes[0].matches[0].tolerance, Tolerance::Indent);
    assert_eq!(edit.text, "class A:\n\tdef g(self):\n\t\treturn 2\n");
}

#[test]
fn normalize_tolerates_inner_white_space_runs() {
    assert_eq!(
        replace("call(a,   b);\n", literal("call(a, b)", "call(b, a)")).unwrap(),
        "call(b, a);\n"
    );
}

#[test]
fn exact_whitespace_requires_identical_text() {
    let change = ReplaceChange {
        whitespace: Some(Whitespace::Exact),
        ..literal("a  b", "x")
    };
    assert_eq!(replace("a b", change).unwrap(), "a b");
}

#[test]
fn trailing_whitespace_does_not_tolerate_indentation() {
    let change = ReplaceChange {
        whitespace: Some(Whitespace::Trailing),
        on_missing: Some(OnMissing::Error),
        ..literal("x\ny", "z")
    };
    assert_eq!(
        error_of(replace("  x\n    y\n", change)).code(),
        ErrorCode::NoMatch
    );
}

#[test]
fn indent_whitespace_tolerates_indentation_only() {
    let change = ReplaceChange {
        whitespace: Some(Whitespace::Indent),
        ..literal("x\n  y", "z\n  w")
    };
    assert_eq!(replace("\tx\n\t\ty\n", change).unwrap(), "\tz\n\t\tw\n");
}

#[test]
fn whitespace_of_the_edit_applies_to_changes_without_own_value() {
    let options = EditOptions {
        whitespace: Whitespace::Exact,
        ..EditOptions::default()
    };
    let edit = plan("a  b", &[Change::Replace(literal("a b", "x"))], &options).unwrap();
    assert_eq!(edit.text, "a  b");
}

#[test]
fn hanging_indentation_continues_inserted_lines() {
    let text = "fn f() {\n    old();\n}\n";
    assert_eq!(
        replace(text, literal("old();", "first();\nsecond();")).unwrap(),
        "fn f() {\n    first();\n    second();\n}\n"
    );
}

#[test]
fn case_insensitive_literal_match() {
    let change = ReplaceChange {
        case_sensitive: false,
        ..literal("hello", "bye")
    };
    assert_eq!(replace("Hello HELLO", change).unwrap(), "bye bye");
}

#[test]
fn whole_word_skips_parts_of_identifiers() {
    let change = ReplaceChange {
        whole_word: true,
        ..literal("count", "total")
    };
    assert_eq!(
        replace("count counter recount count_1 count", change).unwrap(),
        "total counter recount count_1 total"
    );
}

#[test]
fn whole_word_applies_to_regular_expressions() {
    let change = ReplaceChange {
        whole_word: true,
        ..regex("id", "key")
    };
    assert_eq!(replace("id idx (id)", change).unwrap(), "key idx (key)");
}

#[test]
fn regex_replacement_expands_groups() {
    assert_eq!(
        replace("a=1, b=2", regex(r"(\w)=(\d)", "$2=$1")).unwrap(),
        "1=a, 2=b"
    );
}

#[test]
fn regex_replacement_expands_named_groups_and_dollar() {
    assert_eq!(
        replace("cost 5", regex(r"(?<n>\d+)", "$$${n}.00")).unwrap(),
        "cost $5.00"
    );
}

#[test]
fn regex_replacement_without_template_is_literal() {
    let change = ReplaceChange {
        template: false,
        ..regex(r"\d", "$1")
    };
    assert_eq!(replace("a1", change).unwrap(), "a$1");
}

#[test]
fn literal_pattern_inserts_dollar_signs_verbatim() {
    assert_eq!(
        replace("price", literal("price", "$1 $$")).unwrap(),
        "$1 $$"
    );
}

#[test]
fn regex_template_errors_carry_position() {
    let error = error_of(replace("abc", regex(r"(b)", "x$2")));
    assert_eq!(error.code(), ErrorCode::Regex);
    let EditError::Template { change: 0, source } = &error else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(source.column(), 2);
    assert_eq!(
        error.to_string(),
        "change 1: group 2 at column 2 does not exist; the pattern has 1 group"
    );
}

#[test]
fn template_error_column_ignores_line_endings_of_the_text() {
    let error = error_of(replace("a\r\nb\r\n", regex("(a)", "x\ny$2")));
    let EditError::Template { source, .. } = &error else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(source.column(), 4);
}

#[test]
fn template_is_checked_before_matching() {
    let change = regex(r"(a)", "${missing}");
    assert!(change.template().is_err());
    assert!(literal("a", "${missing}").template().is_ok());
}

#[test]
fn regex_matches_exactly_regardless_of_whitespace() {
    let change = ReplaceChange {
        whitespace: Some(Whitespace::Normalize),
        ..regex("a b", "x")
    };
    assert_eq!(replace("a  b", change).unwrap(), "a  b");
}

#[test]
fn regex_replacement_follows_crlf_of_the_text() {
    assert_eq!(
        replace("x\r\ny\r\n", regex("x", "1\n2")).unwrap(),
        "1\r\n2\r\ny\r\n"
    );
}

#[test]
fn regex_empty_matches_insert_at_line_starts() {
    assert_eq!(
        replace("a\nb\n", regex("(?m)^", "# ")).unwrap(),
        "# a\n# b\n# "
    );
}

#[test]
fn regex_occurrence_and_expect_apply() {
    let change = ReplaceChange {
        occurrence: NonZeroUsize::new(2),
        expect: Some(Range::single(3)),
        ..regex(r"\d", "#")
    };
    assert_eq!(replace("1 2 3", change).unwrap(), "1 # 3");
}

#[test]
fn backtracking_engine_supports_back_references() {
    assert_eq!(
        replace("aa bc dd", backtracking(r"(\w)\1", "<$1>")).unwrap(),
        "<a> bc <d>"
    );
}

#[test]
fn backtracking_engine_supports_look_ahead_without_groups() {
    assert_eq!(
        replace("foo1 foo bar", backtracking(r"foo(?=\d)", "x")).unwrap(),
        "x1 foo bar"
    );
}

#[test]
fn backtracking_limit_is_reported_as_limit() {
    let regex = fancy_regex::RegexBuilder::new(r"(a+)+\1b")
        .backtrack_limit(100)
        .build()
        .unwrap();
    let change = ReplaceChange::new(Pattern::Backtracking(regex), "x");
    let error = error_of(replace(&"a".repeat(40), change));
    assert_eq!(error, EditError::BacktrackLimit { change: 0 });
    assert_eq!(error.code(), ErrorCode::Limit);
    assert!(error.to_string().contains("backtracking limit"));
}

#[test]
fn inserts_after_line() {
    assert_eq!(
        run("a\nb\n", vec![insert("x", Some(1), Position::After)]).unwrap(),
        "a\nx\nb\n"
    );
}

#[test]
fn inserts_before_line() {
    assert_eq!(
        run("a\nb\n", vec![insert("x\ny", Some(2), Position::Before)]).unwrap(),
        "a\nx\ny\nb\n"
    );
}

#[test]
fn inserts_at_start_and_end_of_text() {
    let changes = vec![
        insert("first", None, Position::Start),
        insert("last", None, Position::End),
    ];
    assert_eq!(run("a\n", changes).unwrap(), "first\na\nlast\n");
}

#[test]
fn inserts_before_line_after_the_last_line() {
    assert_eq!(
        run("a\n", vec![insert("b", Some(2), Position::Before)]).unwrap(),
        "a\nb\n"
    );
}

#[test]
fn inserts_into_empty_text() {
    assert_eq!(
        run("", vec![insert("a", None, Position::End)]).unwrap(),
        "a\n"
    );
}

#[test]
fn insertion_at_end_keeps_missing_final_line_break() {
    assert_eq!(
        run("a\nb", vec![insert("c", Some(2), Position::After)]).unwrap(),
        "a\nb\nc"
    );
    assert_eq!(
        run("a\nb", vec![insert("c", None, Position::End)]).unwrap(),
        "a\nb\nc"
    );
}

#[test]
fn ensure_newline_terminates_the_result() {
    let options = EditOptions {
        ensure_newline: true,
        ..EditOptions::default()
    };
    let changes = [insert("c", None, Position::End)];
    assert_eq!(
        plan("a\r\nb", &changes, &options).unwrap().text,
        "a\r\nb\r\nc\r\n"
    );
    assert_eq!(plan("", &[], &options).unwrap().text, "");
}

#[test]
fn inserted_lines_follow_crlf_of_the_text() {
    assert_eq!(
        run(
            "a\r\nb\r\n",
            vec![insert("x\ny\n", Some(1), Position::After)]
        )
        .unwrap(),
        "a\r\nx\r\ny\r\nb\r\n"
    );
}

#[test]
fn empty_insertion_changes_nothing() {
    let changes = [insert("", Some(1), Position::After)];
    let edit = plan("a\n", &changes, &EditOptions::default()).unwrap();
    assert_eq!(edit.text, "a\n");
    assert_eq!(edit.outcomes[0].count, 0);
}

#[test]
fn insertion_before_or_after_requires_line() {
    let error = error_of(run("a\n", vec![insert("x", None, Position::Before)]));
    assert_eq!(error.code(), ErrorCode::MissingParam);
    assert_eq!(
        error.to_string(),
        "change 1: position `before` requires `line`; give the line or use position `start` or `end`"
    );
}

#[test]
fn insertion_outside_the_text_is_rejected() {
    for (line, position) in [
        (0, Position::After),
        (3, Position::After),
        (4, Position::Before),
    ] {
        let error = error_of(run("a\nb\n", vec![insert("x", Some(line), position)]));
        assert_eq!(
            error,
            EditError::LineOutOfRange {
                change: 0,
                line,
                lines: 2
            }
        );
        assert_eq!(error.code(), ErrorCode::OutOfRange);
    }
}

#[test]
fn deletes_line_range() {
    let changes = [lines(Some(2), Some(3))];
    let edit = plan("a\nb\nc\nd\n", &changes, &EditOptions::default()).unwrap();
    assert_eq!(edit.text, "a\nd\n");
    assert_eq!(edit.outcomes[0].count, 2);
}

#[test]
fn deletes_with_open_bounds() {
    assert_eq!(run("a\nb\nc\n", vec![lines(None, Some(2))]).unwrap(), "c\n");
    assert_eq!(run("a\nb\nc\n", vec![lines(Some(2), None)]).unwrap(), "a\n");
    assert_eq!(run("a\nb\nc\n", vec![lines(None, None)]).unwrap(), "");
}

#[test]
fn deleting_last_line_keeps_missing_final_line_break() {
    assert_eq!(
        run("a\nb\nc", vec![lines(Some(3), Some(3))]).unwrap(),
        "a\nb"
    );
    assert_eq!(run("a\r\nb", vec![lines(Some(2), None)]).unwrap(), "a");
}

#[test]
fn deleting_outside_the_text_is_rejected() {
    let error = error_of(run("a\nb\n", vec![lines(Some(2), Some(5))]));
    assert_eq!(
        error.to_string(),
        "change 1: line 5 is outside the text of 2 lines; give a line from 1 to 2"
    );
    assert_eq!(
        error_of(run("", vec![lines(None, None)])),
        EditError::LineOutOfRange {
            change: 0,
            line: 1,
            lines: 0
        }
    );
}

#[test]
fn replaces_line_range_with_deletion_and_insertion() {
    let changes = vec![
        lines(Some(2), Some(3)),
        insert("X\nY\nZ", Some(2), Position::Before),
    ];
    assert_eq!(run("a\nb\nc\nd\n", changes).unwrap(), "a\nX\nY\nZ\nd\n");
}

#[test]
fn replaces_last_lines_of_unterminated_text() {
    let changes = vec![
        lines(Some(2), Some(2)),
        insert("B", Some(2), Position::After),
    ];
    assert_eq!(run("a\nb", changes).unwrap(), "a\nB");
}

#[test]
fn line_numbers_refer_to_the_text_before_the_edit() {
    let changes = vec![
        insert("new", Some(1), Position::Before),
        lines(Some(2), Some(2)),
        insert("tail", Some(3), Position::After),
    ];
    assert_eq!(run("1\n2\n3\n", changes).unwrap(), "new\n1\n3\ntail\n");
}

#[test]
fn patterns_match_the_text_before_the_edit() {
    let changes = vec![
        Change::Replace(literal("a", "b")),
        Change::Replace(literal("b", "c")),
    ];
    assert_eq!(run("a b", changes).unwrap(), "b c");
}

#[test]
fn insertions_at_one_position_keep_list_order() {
    let changes = vec![
        insert("x", Some(1), Position::After),
        insert("y", Some(2), Position::Before),
    ];
    assert_eq!(run("a\nb\n", changes).unwrap(), "a\nx\ny\nb\n");
}

#[test]
fn overlapping_deletions_conflict() {
    let error = error_of(run(
        "a\nb\nc\nd\n",
        vec![lines(Some(1), Some(2)), lines(Some(2), Some(3))],
    ));
    assert_eq!(
        error,
        EditError::Conflict {
            first: 0,
            second: 1
        }
    );
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert_eq!(
        error.to_string(),
        "changes 1 and 2 overlap; merge them into one change or adjust their ranges"
    );
}

#[test]
fn replacement_inside_deleted_lines_conflicts() {
    let error = error_of(run(
        "a\nb\nc\n",
        vec![Change::Replace(literal("b", "x")), lines(Some(1), Some(2))],
    ));
    assert_eq!(
        error,
        EditError::Conflict {
            first: 0,
            second: 1
        }
    );
}

#[test]
fn insertion_inside_replaced_fragment_conflicts() {
    let error = error_of(run(
        "a\nb\nc\n",
        vec![
            insert("x", Some(1), Position::After),
            Change::Replace(literal("a\nb", "z")),
        ],
    ));
    assert_eq!(
        error,
        EditError::Conflict {
            first: 0,
            second: 1
        }
    );
}

#[test]
fn adjacent_changes_do_not_conflict() {
    let changes = vec![
        lines(Some(1), Some(1)),
        lines(Some(2), Some(2)),
        Change::Replace(literal("c", "C")),
    ];
    assert_eq!(run("a\nb\nc\n", changes).unwrap(), "C\n");
}

#[test]
fn failing_change_leaves_no_partial_result() {
    let changes = [
        Change::Replace(literal("a", "b")),
        Change::Replace(ReplaceChange {
            on_missing: Some(OnMissing::Error),
            ..literal("zzz", "y")
        }),
    ];
    let error = plan("a", &changes, &EditOptions::default()).unwrap_err();
    assert!(matches!(error, EditError::Match { change: 1, .. }));
    assert!(error.to_string().starts_with("change 2: "));
}

#[test]
fn total_expect_counts_replacements_of_all_changes() {
    let changes = [
        Change::Replace(literal("a", "x")),
        Change::Replace(literal("b", "y")),
        insert("z", None, Position::End),
    ];
    let exact = EditOptions {
        expect: Some(Range::single(3)),
        ..EditOptions::default()
    };
    assert_eq!(
        plan("a a b\n", &changes, &exact).unwrap().text,
        "x x y\nz\n"
    );
    let wrong = EditOptions {
        expect: Some(Range::single(2)),
        ..EditOptions::default()
    };
    let error = plan("a a b\n", &changes, &wrong).unwrap_err();
    assert_eq!(
        error,
        EditError::TotalMismatch {
            expected: Range::single(2),
            found: 3
        }
    );
    assert_eq!(error.code(), ErrorCode::Assert);
    assert_eq!(
        error.to_string(),
        "the changes replace 3 matches, expected 2; adjust the patterns or `expect`"
    );
}

#[test]
fn empty_literal_pattern_is_rejected() {
    let error = error_of(replace("abc", literal("", "x")));
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn position_names_round_trip() {
    for name in ["before", "after", "start", "end"] {
        assert_eq!(Position::from_name(name).unwrap().as_str(), name);
    }
    assert_eq!(Position::from_name("middle"), None);
    assert_eq!(Position::default(), Position::After);
}

#[test]
fn errors_without_source_report_none() {
    let error = EditError::Conflict {
        first: 0,
        second: 1,
    };
    assert!(std::error::Error::source(&error).is_none());
    let template = error_of(replace("a", regex("(a)", "$9")));
    assert!(std::error::Error::source(&template).is_some());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
