// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Byte edits, offset maps, line diffs and the scope, meaning and identity
//! guards.
// Helpers outside `#[test]` functions fail the test on invalid fixtures.
#![allow(clippy::expect_used)]

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::report::Notable;
use nurt_contracts::value::Value;
use nurt_doc::constructs::{DOC_OUT_OF_SCOPE_CHANGE, DOC_ROUND_TRIP_LOSS, DOC_SEMANTIC_MISMATCH};
use nurt_doc::edit::guard::{
    EditPlan, Expectation, Operation, ReferenceParser, Surroundings, check_meaning,
    check_round_trip, check_scope, commit,
};
use nurt_doc::edit::{ByteEdit, EditReport, apply, check_overlaps, diff};
use nurt_doc::model::{FormatSemantics, MapSemantics, Repeated, Span};
use serde_json::json;

/// Semantics of a format whose text has no types: every scalar compares as
/// its text.
struct Untyped;

impl FormatSemantics for Untyped {
    fn repeated(&self) -> Repeated {
        Repeated::LastWins
    }

    fn canonical(&self, value: Value) -> Value {
        match value {
            Value::Map(fields) => Value::Map(
                fields
                    .into_iter()
                    .map(|(name, value)| (name, self.canonical(value)))
                    .collect(),
            ),
            Value::List(items) => {
                Value::List(items.into_iter().map(|item| self.canonical(item)).collect())
            }
            Value::Int(number) => Value::Text(number.to_string()),
            other => other,
        }
    }
}

/// Splitting step of the deterministic diff cases.
const LCG_MULTIPLIER: u64 = 6_364_136_223_846_793_005;
/// Increment of the deterministic diff cases.
const LCG_INCREMENT: u64 = 1_442_695_040_888_963_407;
/// Number of deterministic diff cases.
const DIFF_CASES: usize = 300;

/// Reference reader of `name=value` lines; values are text.
struct Lines;

impl ReferenceParser for Lines {
    fn parse(&self, text: &[u8]) -> Result<Vec<Value>> {
        let text =
            std::str::from_utf8(text).map_err(|_| Error::new(ErrorCode::DocParse, "not UTF-8"))?;
        let mut fields = nurt_contracts::value::Fields::new();
        for line in text.lines().filter(|line| !line.is_empty()) {
            let (name, value) = line.split_once('=').ok_or_else(|| {
                Error::new(ErrorCode::DocParse, format!("line `{line}` has no `=`"))
            })?;
            fields.insert(name.to_owned(), Value::Text(value.to_owned()));
        }
        Ok(vec![Value::Map(fields)])
    }
}

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).expect("valid key path")
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn before(original: &[u8]) -> Vec<Value> {
    Lines.parse(original).expect("original parses")
}

#[test]
fn apply_replaces_inserts_and_deletes_in_one_pass() {
    let original = b"a = 1\nb = 2\n";
    let edits = [
        ByteEdit::insert(original.len(), "c = 3\n"),
        ByteEdit::replace(Span::new(4, 5), "10"),
        ByteEdit::delete(Span::new(6, 12)),
    ];
    let applied = apply(original, &edits).expect("edits apply");
    assert_eq!(applied.bytes, b"a = 10\nc = 3\n");
    assert_eq!(
        applied.offsets.replaced(),
        &[Span::new(4, 5), Span::new(6, 12), Span::at(12)]
    );
    assert_eq!(
        applied.offsets.changed(),
        &[Span::new(4, 6), Span::at(7), Span::new(7, 13)]
    );
}

#[test]
fn offset_map_moves_offsets_after_edits() {
    let applied = apply(b"abcdef", &[ByteEdit::delete(Span::new(1, 3))]).expect("edit applies");
    let map = &applied.offsets;
    assert_eq!(map.map(0), 0);
    assert_eq!(map.map(1), 1);
    assert_eq!(map.map(2), 1);
    assert_eq!(map.map(3), 1);
    assert_eq!(map.map(6), 4);
    let grown = apply(b"ab", &[ByteEdit::replace(Span::new(0, 1), "xyz")]).expect("edit applies");
    assert_eq!(grown.offsets.map(1), 3);
    assert_eq!(grown.offsets.map(2), 4);
}

#[test]
fn insertion_at_boundary_of_replacement_is_allowed() {
    let edits = [
        ByteEdit::insert(4, "Y"),
        ByteEdit::replace(Span::new(2, 4), "X"),
    ];
    let applied = apply(b"abcdef", &edits).expect("edits touch without overlap");
    assert_eq!(applied.bytes, b"abXYef");
}

#[test]
fn insertions_at_the_same_offset_keep_their_order() {
    let edits = [ByteEdit::insert(1, "1"), ByteEdit::insert(1, "2")];
    assert_eq!(apply(b"ab", &edits).expect("edits apply").bytes, b"a12b");
}

#[test]
fn overlapping_edits_are_key_conflict() {
    let edits = [
        ByteEdit::replace(Span::new(2, 6), "x"),
        ByteEdit::insert(4, "y"),
    ];
    let error = apply(b"abcdefgh", &edits).expect_err("insertion inside replacement");
    assert_eq!(error.code, ErrorCode::KeyConflict);
    assert_eq!(error.found.as_deref(), Some("overlap"));
    let shared = [
        ByteEdit::delete(Span::new(0, 3)),
        ByteEdit::delete(Span::new(2, 4)),
    ];
    assert_eq!(
        check_overlaps(&shared, 8).expect_err("shared byte").code,
        ErrorCode::KeyConflict
    );
}

#[test]
fn edit_past_the_end_is_invalid_value() {
    let error = apply(b"ab", &[ByteEdit::delete(Span::new(1, 3))]).expect_err("past the end");
    assert_eq!(error.code, ErrorCode::InvalidValue);
}

#[test]
fn noop_edit_is_detected() {
    assert!(ByteEdit::replace(Span::new(1, 2), "b").is_noop(b"abc"));
    assert!(!ByteEdit::replace(Span::new(1, 2), "x").is_noop(b"abc"));
    assert!(!ByteEdit::replace(Span::new(2, 9), "c").is_noop(b"abc"));
}

#[test]
fn diff_of_equal_texts_is_empty() {
    assert!(diff(b"a\nb\n", b"a\nb\n").is_empty());
    assert!(diff(b"", b"").is_empty());
}

#[test]
fn diff_narrows_changes_to_differing_bytes() {
    let old = b"a = 1\nb = 2\nc = 3\n";
    let new = b"a = 10\nb = 2\nc = 30\n";
    let edits = diff(old, new);
    assert_eq!(edits.len(), 2);
    assert!(edits.iter().all(|edit| edit.span.len() <= 1), "{edits:?}");
    assert_eq!(apply(old, &edits).expect("diff applies").bytes, new);
}

#[test]
fn diff_handles_inserted_removed_and_crlf_lines() {
    let pairs: [(&[u8], &[u8]); 6] = [
        (b"a\nb\nc\n", b"a\nx\nb\nc\n"),
        (b"a\nb\nc\n", b"a\nc\n"),
        (b"a\r\nb\r\n", b"a\r\nB\r\nc\r\n"),
        (b"", b"new\n"),
        (b"old\n", b""),
        (b"no final newline", b"no final newline\n"),
    ];
    for (old, new) in pairs {
        let edits = diff(old, new);
        assert_eq!(
            apply(old, &edits).expect("diff applies").bytes,
            new,
            "{old:?} -> {new:?}"
        );
    }
}

#[test]
fn diff_keeps_separate_changes_apart_when_the_common_end_splits_a_line() {
    let old = b"# a\nA=1\nB=2\nA=3\n";
    let new = b"B=2\n";
    let edits = diff(old, new);
    assert_eq!(
        edits,
        [
            ByteEdit::delete(Span::new(0, 8)),
            ByteEdit::delete(Span::new(12, 16))
        ]
    );
    assert_eq!(apply(old, &edits).expect("diff applies").bytes, new);
    let tail = diff(b"x = 1\ny = 2", b"x = 1\ny = 2\nz = 3");
    assert_eq!(tail, [ByteEdit::insert(11, "\nz = 3")]);
}

#[test]
fn diff_of_large_rewrite_is_one_edit() {
    let old: Vec<u8> = (0..3000)
        .flat_map(|line| format!("a{line}\n").into_bytes())
        .collect();
    let new: Vec<u8> = (0..3000)
        .flat_map(|line| format!("b{line}\n").into_bytes())
        .collect();
    let edits = diff(&old, &new);
    assert_eq!(edits.len(), 1);
    assert_eq!(apply(&old, &edits).expect("diff applies").bytes, new);
}

fn next(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(LCG_MULTIPLIER)
        .wrapping_add(LCG_INCREMENT);
    *state >> 33
}

#[test]
fn diff_reproduces_target_for_generated_line_edits() {
    let mut state = 7;
    for case in 0..DIFF_CASES {
        let lines: Vec<String> = (0..next(&mut state) % 12)
            .map(|line| format!("k{line} = {}\n", next(&mut state) % 5))
            .collect();
        let mut changed = lines.clone();
        for _ in 0..=next(&mut state) % 4 {
            let at = usize::try_from(next(&mut state)).unwrap_or(0) % (changed.len() + 1);
            match next(&mut state) % 3 {
                0 => changed.insert(at, format!("new{case} = 1\n")),
                1 if at < changed.len() => {
                    changed.remove(at);
                }
                _ if at < changed.len() => changed[at] = format!("k{at} = changed\n"),
                _ => {}
            }
        }
        let old = lines.concat().into_bytes();
        let new = changed.concat().into_bytes();
        let edits = diff(&old, &new);
        assert_eq!(
            apply(&old, &edits).expect("diff applies").bytes,
            new,
            "case {case}"
        );
    }
}

#[test]
fn commit_applies_change_within_target() {
    let original = b"a=1\nb=2\n";
    let mut plan = EditPlan::new();
    plan.target(Span::new(2, 3))
        .edit(ByteEdit::replace(Span::new(2, 3), "9"));
    plan.report.replacements = 1;
    let expectation =
        Expectation::new(before(original), &MapSemantics::LAST_WINS).set(key("a"), text("9"));
    let committed = commit(original, plan, &expectation, &Lines).expect("change is accepted");
    assert_eq!(committed.bytes, b"a=9\nb=2\n");
    assert_eq!(committed.report.changed, [Span::new(2, 3)]);
    assert_eq!(committed.report.replacements, 1);
}

#[test]
fn setting_current_value_changes_no_byte() {
    let original = b"a=1\nb=2\n";
    let mut plan = EditPlan::new();
    plan.target(Span::new(2, 3))
        .edit(ByteEdit::replace(Span::new(2, 3), "1"));
    plan.report.replacements = 1;
    let expectation =
        Expectation::new(before(original), &MapSemantics::LAST_WINS).set(key("a"), text("1"));
    let committed = commit(original, plan, &expectation, &Lines).expect("change is accepted");
    assert_eq!(committed.bytes, original);
    assert!(committed.report.is_unchanged());
    assert_eq!(committed.report.replacements, 0);
}

#[test]
fn scope_guard_rejects_edit_outside_targets() {
    let original = b"a=1\nb=2\n";
    let mut plan = EditPlan::new();
    plan.target(Span::new(2, 3))
        .edit(ByteEdit::replace(Span::new(2, 3), "9"))
        .edit(ByteEdit::replace(Span::new(6, 7), "9"));
    let expectation = Expectation::new(before(original), &MapSemantics::LAST_WINS)
        .set(key("a"), text("9"))
        .set(key("b"), text("9"));
    let error = commit(original, plan, &expectation, &Lines).expect_err("b is not a target");
    assert_eq!(error.code, ErrorCode::UnsupportedConstruct);
    assert_eq!(error.found.as_deref(), Some(DOC_OUT_OF_SCOPE_CHANGE));
    assert!(
        error.message.contains("line 2, column 3"),
        "{}",
        error.message
    );
    assert!(
        error
            .message
            .ends_with("edit the text with `replace` or `edit` instead")
    );
    assert_eq!(error.details.get("byte"), Some(&json!(6)));
}

#[test]
fn scope_guard_allows_separators_and_line_breaks_next_to_target() {
    let original = b"a=1\nb=2\nc=3\n";
    let line = Span::new(4, 7);
    let removal = [ByteEdit::delete(Span::new(4, 8))];
    assert!(check_scope(original, &[line], &removal, Surroundings::DEFAULT).is_ok());
    let list = b"[1, 2, 3]";
    let element = [ByteEdit::delete(Span::new(3, 6))];
    assert!(check_scope(list, &[Span::new(4, 5)], &element, Surroundings::DEFAULT).is_ok());
    let wider = [ByteEdit::delete(Span::new(1, 6))];
    assert!(check_scope(list, &[Span::new(4, 5)], &wider, Surroundings::DEFAULT).is_err());
    let statements = b"a;b";
    let semicolon = [ByteEdit::delete(Span::new(1, 3))];
    assert!(
        check_scope(
            statements,
            &[Span::new(2, 3)],
            &semicolon,
            Surroundings::DEFAULT
        )
        .is_err()
    );
    assert!(
        check_scope(
            statements,
            &[Span::new(2, 3)],
            &semicolon,
            Surroundings::DEFAULT.with(b";")
        )
        .is_ok()
    );
}

#[test]
fn insertion_point_target_accepts_insertion_only_there() {
    let original = b"a=1\n";
    let at_end = [ByteEdit::insert(4, "b=2\n")];
    assert!(check_scope(original, &[Span::at(4)], &at_end, Surroundings::DEFAULT).is_ok());
    let at_start = [ByteEdit::insert(0, "b=2\n")];
    assert!(check_scope(original, &[Span::at(4)], &at_start, Surroundings::DEFAULT).is_err());
}

#[test]
fn meaning_guard_rejects_unexpected_value() {
    let original = b"a=1\nb=2\n";
    let mut plan = EditPlan::new();
    plan.target(Span::new(2, 3))
        .edit(ByteEdit::replace(Span::new(2, 3), "9"));
    let expectation =
        Expectation::new(before(original), &MapSemantics::LAST_WINS).set(key("a"), text("8"));
    let error = commit(original, plan, &expectation, &Lines).expect_err("9 is not 8");
    assert_eq!(error.code, ErrorCode::UnsupportedConstruct);
    assert_eq!(error.found.as_deref(), Some(DOC_SEMANTIC_MISMATCH));
    assert_eq!(error.details.get("difference"), Some(&json!("[0].a")));
    assert!(
        error.message.starts_with("the change of `a`"),
        "{}",
        error.message
    );
}

#[test]
fn meaning_guard_rejects_unparsable_result() {
    let original = b"a=1\n";
    let mut plan = EditPlan::new();
    plan.target(Span::new(0, 3))
        .edit(ByteEdit::replace(Span::new(1, 2), ":"));
    let expectation = Expectation::new(before(original), &MapSemantics::LAST_WINS);
    let error = commit(original, plan, &expectation, &Lines).expect_err("line without `=`");
    assert_eq!(error.found.as_deref(), Some(DOC_SEMANTIC_MISMATCH));
    assert!(
        error.message.contains("would make the document invalid"),
        "{}",
        error.message
    );
}

#[test]
fn meaning_guard_accepts_removal() {
    let original = b"a=1\nb=2\n";
    let mut plan = EditPlan::new();
    plan.target(Span::new(4, 7))
        .edit(ByteEdit::delete(Span::new(4, 8)));
    let expectation = Expectation::new(before(original), &MapSemantics::LAST_WINS).remove(key("b"));
    assert_eq!(
        expectation.operations(),
        &[Operation::Remove { key: key("b") }]
    );
    let committed = commit(original, plan, &expectation, &Lines).expect("removal is accepted");
    assert_eq!(committed.bytes, b"a=1\n");
}

#[test]
fn expectation_selects_one_document() {
    let documents = vec![
        Value::from_plain_json(json!({"a": 1})),
        Value::from_plain_json(json!({"a": 2})),
    ];
    let second = Expectation::new(documents.clone(), &MapSemantics::LAST_WINS)
        .set(key("#1:a"), Value::Int(3));
    let expected = second.expected().expect("second document exists");
    assert_eq!(expected[1], Value::from_plain_json(json!({"a": 3})));
    assert_eq!(expected[0], documents[0]);
    let every = Expectation::new(documents.clone(), &MapSemantics::LAST_WINS)
        .set(key("#*:a"), Value::Int(3));
    assert_eq!(
        every.expected().expect_err("every document").code,
        ErrorCode::KeyConflict
    );
    let missing = Expectation::new(documents, &MapSemantics::LAST_WINS).remove(key("#7:a"));
    assert_eq!(
        missing.expected().expect_err("no such document").code,
        ErrorCode::KeyNotFound
    );
}

#[test]
fn check_meaning_compares_all_documents() {
    let expectation = Expectation::new(before(b"a=1\n"), &MapSemantics::LAST_WINS);
    assert!(check_meaning(b"a=1\n", &expectation, &Lines).is_ok());
    let error = check_meaning(b"a=1\nb=2\n", &expectation, &Lines).expect_err("b was added");
    assert_eq!(error.details.get("difference"), Some(&json!("[0].b")));
}

#[test]
fn meaning_guard_compares_canonical_forms_of_the_format() {
    let original = b"a=1\n";
    let plan = || {
        let mut plan = EditPlan::new();
        plan.target(Span::new(2, 3))
            .edit(ByteEdit::replace(Span::new(2, 3), "2"));
        plan
    };
    let typed =
        Expectation::new(before(original), &MapSemantics::LAST_WINS).set(key("a"), Value::Int(2));
    let error = commit(original, plan(), &typed, &Lines).expect_err("text is not an integer");
    assert_eq!(error.found.as_deref(), Some(DOC_SEMANTIC_MISMATCH));
    let untyped = Expectation::new(before(original), &Untyped).set(key("a"), Value::Int(2));
    let committed = commit(original, plan(), &untyped, &Lines).expect("integer reads as text");
    assert_eq!(committed.bytes, b"a=2\n");
    let other = Expectation::new(before(original), &Untyped).set(key("a"), Value::Int(3));
    assert!(commit(original, plan(), &other, &Lines).is_err());
    assert_eq!(
        MapSemantics::CONFLICT.canonical(Value::Int(2)),
        Value::Int(2)
    );
}

#[test]
fn plan_from_rewrite_uses_differences() {
    let original = b"a=1\nb=2\n";
    let plan = EditPlan::from_rewrite(original, b"a=1\nb=3\n", vec![Span::new(6, 7)]);
    assert_eq!(plan.edits, [ByteEdit::replace(Span::new(6, 7), "3")]);
    assert_eq!(plan.surroundings, Surroundings::default());
    let expectation =
        Expectation::new(before(original), &MapSemantics::LAST_WINS).set(key("b"), text("3"));
    assert!(commit(original, plan, &expectation, &Lines).is_ok());
}

#[test]
fn identity_guard_reports_first_differing_position() {
    assert!(check_round_trip(b"a: 1\n", b"a: 1\n").is_ok());
    let error = check_round_trip(b"a: 1\nb: 2\n", b"a: 1\nb:  2\n").expect_err("bytes differ");
    assert_eq!(error.code, ErrorCode::UnsupportedConstruct);
    assert_eq!(error.found.as_deref(), Some(DOC_ROUND_TRIP_LOSS));
    assert!(
        error.message.contains("line 2, column 4"),
        "{}",
        error.message
    );
    assert_eq!(error.details.get("byte"), Some(&json!(8)));
}

#[test]
fn surroundings_list_permitted_bytes() {
    for byte in *b" \t\r\n," {
        assert!(Surroundings::DEFAULT.allows(byte));
    }
    assert!(!Surroundings::DEFAULT.allows(b';'));
    assert!(Surroundings::DEFAULT.with(b";:").allows(b':'));
}

#[test]
fn report_merges_flags_and_replacements() {
    let mut report = EditReport::default();
    report.flag("keys_created");
    report.flag("keys_created");
    let mut other = EditReport {
        replacements: 2,
        ..EditReport::default()
    };
    other.flag("duplicate_key");
    other.flag("keys_created");
    report.absorb(other);
    assert_eq!(report.flags, ["keys_created", "duplicate_key"]);
    assert_eq!(report.replacements, 2);
    assert!(report.is_unchanged());
}

#[test]
fn report_keeps_one_notable_effect_per_code() {
    let mut report = EditReport::default();
    report.note(Notable::KeysCreated, Some("a.b".to_owned()));
    report.note(Notable::KeysCreated, None);
    let mut other = EditReport::default();
    other.note(Notable::DuplicateKey, None);
    other.note(Notable::KeysCreated, Some("c".to_owned()));
    report.absorb(other);
    let codes: Vec<Notable> = report.notable.iter().map(|entry| entry.code).collect();
    assert_eq!(codes, [Notable::KeysCreated, Notable::DuplicateKey]);
    assert_eq!(report.notable[0].detail.as_deref(), Some("a.b"));
    assert_eq!(report.notable[1].detail, None);
}
