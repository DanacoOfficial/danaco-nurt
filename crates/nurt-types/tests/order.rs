// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-types/tests/order.rs
// Responsibility:   Order, implicit conversions and arithmetic of values.
// Dependencies:     nurt_contracts, proptest
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Order, implicit conversions and arithmetic of values.

// Assertions unwrap results whose failure is the failure of the test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cmp::Ordering;
use std::collections::BTreeSet;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::value::{Duration, Instant, Range, Record, RecordKind, Size, Value};
use nurt_types::{
    OrderedValue, ValueType, add, coerce, comparable, compare, converts_implicitly, div, equal,
    mul, neg, sub,
};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 512,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(0x6e75_7274),
        ..ProptestConfig::default()
    }
}

fn size(bytes: u64) -> Value {
    Value::Size(Size::from_bytes(bytes))
}

fn duration(millis: i64) -> Value {
    Value::Duration(Duration::from_millis(millis))
}

fn instant(text: &str) -> Value {
    Value::Instant(text.parse::<Instant>().unwrap())
}

fn assert_code(error: &Error, code: ErrorCode) {
    assert_eq!(error.code, code, "{error}");
}

fn assert_problem(error: &Error, problem: &str) {
    assert_eq!(error.code, ErrorCode::InvalidValue, "{error}");
    assert_eq!(error.problem.as_deref(), Some(problem), "{error}");
}

#[test]
fn orders_type_families_in_the_documented_sequence() {
    let ascending = [
        Value::Null,
        Value::Bool(true),
        Value::Int(1_000),
        size(1),
        duration(1),
        instant("2026-11-21T10:00:00Z"),
        Value::from("a"),
        Value::Path("/a".to_owned()),
        Value::Glob("*.rs".to_owned()),
        Value::Key("a.b".to_owned()),
        Value::Bytes(vec![0].into()),
        Value::Range(Range::single(1)),
        Value::List(Vec::new()),
        Value::Map(Default::default()),
        Value::Record(Record::new(RecordKind::Entry)),
    ];
    for pair in ascending.windows(2) {
        assert_eq!(compare(&pair[0], &pair[1]), Ordering::Less, "{pair:?}");
        assert!(!comparable(&pair[0], &pair[1]));
    }
}

#[test]
fn compares_integers_and_floats_exactly() {
    let two_pow_63 = 9_223_372_036_854_775_808.0;
    let cases = [
        (Value::Int(1), Value::Number(1.0), Ordering::Equal),
        (Value::Int(2), Value::Number(2.5), Ordering::Less),
        (Value::Int(-1), Value::Number(-1.5), Ordering::Greater),
        (
            Value::Int(i64::MAX),
            Value::Number(two_pow_63),
            Ordering::Less,
        ),
        (
            Value::Int(i64::MIN),
            Value::Number(-two_pow_63),
            Ordering::Equal,
        ),
        (
            Value::Int(9_007_199_254_740_993),
            Value::Number(9_007_199_254_740_992.0),
            Ordering::Greater,
        ),
        (Value::Number(-0.0), Value::Int(0), Ordering::Equal),
        (
            Value::Number(f64::NAN),
            Value::Number(1e300),
            Ordering::Greater,
        ),
        (
            Value::Number(f64::NAN),
            Value::Number(f64::NAN),
            Ordering::Equal,
        ),
        (
            Value::Int(i64::MAX),
            Value::Number(f64::NAN),
            Ordering::Less,
        ),
    ];
    for (left, right, expected) in cases {
        assert_eq!(compare(&left, &right), expected, "{left:?} {right:?}");
        assert_eq!(compare(&right, &left), expected.reverse());
    }
}

#[test]
fn compares_numbers_kept_as_source_text_by_value() {
    let cases = [
        (
            Value::BigInt("9223372036854775808".to_owned()),
            Value::Int(i64::MAX),
            Ordering::Greater,
        ),
        (
            Value::BigInt("-9223372036854775809".to_owned()),
            Value::Int(i64::MIN),
            Ordering::Less,
        ),
        (
            Value::BigInt("9223372036854775808".to_owned()),
            Value::Number(9_223_372_036_854_775_808.0),
            Ordering::Equal,
        ),
        (
            Value::Decimal("1.10".to_owned()),
            Value::Number(1.1),
            Ordering::Equal,
        ),
        (
            Value::Decimal("1e400".to_owned()),
            Value::Number(1e300),
            Ordering::Greater,
        ),
        (
            Value::Decimal("-0.0".to_owned()),
            Value::Int(0),
            Ordering::Equal,
        ),
        (
            Value::Decimal("0.05".to_owned()),
            Value::Decimal("5e-2".to_owned()),
            Ordering::Equal,
        ),
        (
            Value::Decimal("-2.5".to_owned()),
            Value::Decimal("-2.25".to_owned()),
            Ordering::Less,
        ),
        (
            Value::Decimal("not a number".to_owned()),
            Value::Int(1),
            Ordering::Greater,
        ),
    ];
    for (left, right, expected) in cases {
        assert_eq!(compare(&left, &right), expected, "{left:?} {right:?}");
        assert_eq!(compare(&right, &left), expected.reverse());
    }
}

#[test]
fn compares_text_by_code_points_without_locale() {
    assert_eq!(
        compare(&Value::from("Z"), &Value::from("a")),
        Ordering::Less
    );
    assert_eq!(
        compare(&Value::from("e\u{301}"), &Value::from("f")),
        Ordering::Less
    );
    assert_eq!(
        compare(&Value::from("\u{e9}"), &Value::from("f")),
        Ordering::Greater
    );
    assert!(equal(&Value::from("file"), &Value::Enum("file".to_owned())));
    assert!(!equal(&Value::from("/a"), &Value::Path("/a".to_owned())));
}

#[test]
fn compares_composite_values_element_by_element() {
    let short = Value::List(vec![Value::Int(1)]);
    let long = Value::List(vec![Value::Int(1), Value::Int(0)]);
    assert_eq!(compare(&short, &long), Ordering::Less);
    let open_start = Value::Range(Range::up_to(5));
    let closed = Value::Range(Range::single(0));
    let open_end = Value::Range(Range::from_start(0));
    assert_eq!(compare(&open_start, &closed), Ordering::Less);
    assert_eq!(compare(&closed, &open_end), Ordering::Less);
    let entry = Record::new(RecordKind::Entry).with("a", Value::Int(1));
    let line = Record::new(RecordKind::Line).with("a", Value::Int(0));
    assert_eq!(
        compare(&Value::Record(entry.clone()), &Value::Record(line)),
        Ordering::Less
    );
    let larger = entry.clone().with("a", Value::Int(2));
    assert_eq!(
        compare(&Value::Record(entry), &Value::Record(larger)),
        Ordering::Less
    );
}

#[test]
fn ordered_values_deduplicate_equal_numbers() {
    let set: BTreeSet<OrderedValue> = [Value::Int(1), Value::Number(1.0), Value::Int(0)]
        .into_iter()
        .map(OrderedValue::from)
        .collect();
    assert_eq!(set.len(), 2);
}

#[test]
fn implicit_conversions_are_exactly_the_six_text_pairs() {
    let admitted = [
        ValueType::Path,
        ValueType::Glob,
        ValueType::Instant,
        ValueType::Key,
        ValueType::Size,
        ValueType::Duration,
    ];
    for &from in ValueType::ALL {
        for &to in ValueType::ALL {
            let expected = from == ValueType::Text && admitted.contains(&to);
            assert_eq!(converts_implicitly(from, to), expected, "{from} -> {to}");
        }
    }
}

#[test]
fn coerce_converts_text_only_to_the_admitted_types() {
    for &target in ValueType::ALL {
        let sample = match target {
            ValueType::Size => "1kb",
            ValueType::Duration => "1s",
            ValueType::Instant => "2026-11-21",
            _ => "/srv/app",
        };
        let result = coerce(Value::from(sample), target);
        if converts_implicitly(ValueType::Text, target) || target == ValueType::Text {
            assert_eq!(ValueType::of(&result.unwrap()), Some(target), "{target}");
        } else {
            assert_code(&result.unwrap_err(), ErrorCode::Type);
        }
    }
}

#[test]
fn coerce_reports_malformed_text_and_regex_text() {
    assert_code(
        &coerce(Value::from("10q"), ValueType::Size).unwrap_err(),
        ErrorCode::InvalidValue,
    );
    let error = coerce(Value::from("a+"), ValueType::Regex).unwrap_err();
    assert_code(&error, ErrorCode::Type);
    assert_eq!(error.expected, Some(serde_json::json!(["regex"])));
    assert_code(
        &coerce(Value::Int(1), ValueType::Number).unwrap_err(),
        ErrorCode::Type,
    );
    assert_eq!(coerce(Value::Null, ValueType::Size).unwrap(), Value::Null);
    assert_eq!(coerce(size(5), ValueType::Size).unwrap(), size(5));
}

#[test]
fn value_types_name_every_value() {
    assert_eq!(
        ValueType::of(&Value::BigInt("1".to_owned())),
        Some(ValueType::Int)
    );
    assert_eq!(
        ValueType::of(&Value::Decimal("1.0".to_owned())),
        Some(ValueType::Number)
    );
    for &kind in ValueType::ALL {
        assert_eq!(kind.as_str().parse::<ValueType>().unwrap(), kind);
    }
    assert!("money".parse::<ValueType>().is_err());
}

#[test]
fn adds_values_of_every_admitted_pair() {
    let cases = [
        (Value::Int(2), Value::Int(3), Value::Int(5)),
        (Value::Int(1), Value::Number(0.5), Value::Number(1.5)),
        (Value::from("ab"), Value::from("cd"), Value::from("abcd")),
        (size(1_000), size(24), size(1_024)),
        (duration(1_000), duration(-250), duration(750)),
        (
            instant("2026-11-21T10:00:00Z"),
            duration(3_600_000),
            instant("2026-11-21T11:00:00Z"),
        ),
        (
            duration(-3_600_000),
            instant("2026-11-21T10:00:00Z"),
            instant("2026-11-21T09:00:00Z"),
        ),
        (Value::Null, Value::Int(1), Value::Null),
    ];
    for (left, right, expected) in cases {
        assert_eq!(
            add(&left, &right).unwrap(),
            expected,
            "{left:?} + {right:?}"
        );
    }
}

#[test]
fn subtracts_instants_and_units() {
    assert_eq!(
        sub(
            &instant("2026-11-21T10:00:01.5Z"),
            &instant("2026-11-21T10:00:00Z")
        )
        .unwrap(),
        duration(1_500)
    );
    assert_eq!(
        sub(&instant("2026-11-21T10:00:00Z"), &duration(60_000)).unwrap(),
        instant("2026-11-21T09:59:00Z")
    );
    assert_eq!(sub(&size(10), &size(4)).unwrap(), size(6));
    assert_eq!(
        sub(&Value::Number(1.5), &Value::Int(1)).unwrap(),
        Value::Number(0.5)
    );
    assert_problem(&sub(&size(1), &size(2)).unwrap_err(), "overflow");
    assert_problem(
        &sub(&Value::Int(i64::MIN), &Value::Int(1)).unwrap_err(),
        "overflow",
    );
}

#[test]
fn multiplies_and_divides_units_by_numbers() {
    assert_eq!(mul(&size(1_000), &Value::Int(3)).unwrap(), size(3_000));
    assert_eq!(mul(&Value::Int(3), &size(1_000)).unwrap(), size(3_000));
    assert_eq!(mul(&size(3), &Value::Number(0.5)).unwrap(), size(2));
    assert_eq!(
        mul(&duration(1_000), &Value::Int(-2)).unwrap(),
        duration(-2_000)
    );
    assert_eq!(
        mul(&Value::Number(1.5), &duration(1_000)).unwrap(),
        duration(1_500)
    );
    assert_eq!(div(&size(3), &Value::Int(2)).unwrap(), size(1));
    assert_eq!(div(&size(3), &Value::Number(2.0)).unwrap(), size(2));
    assert_eq!(div(&size(3), &size(2)).unwrap(), Value::Number(1.5));
    assert_eq!(div(&duration(-7), &Value::Int(2)).unwrap(), duration(-3));
    assert_eq!(div(&duration(3), &Value::Number(2.0)).unwrap(), duration(2));
    assert_eq!(div(&duration(3), &duration(2)).unwrap(), Value::Number(1.5));
    assert_eq!(div(&Value::Int(6), &Value::Int(3)).unwrap(), Value::Int(2));
    assert_eq!(
        div(&Value::Int(7), &Value::Int(2)).unwrap(),
        Value::Number(3.5)
    );
    assert_eq!(
        div(&Value::Number(1.0), &Value::Int(4)).unwrap(),
        Value::Number(0.25)
    );
}

#[test]
fn reports_overflow_and_division_by_zero() {
    let overflowing = [
        add(&Value::Int(i64::MAX), &Value::Int(1)),
        add(&size(u64::MAX), &size(1)),
        mul(&size(2), &Value::Int(-1)),
        mul(&size(u64::MAX), &Value::Number(2.0)),
        mul(&duration(i64::MAX), &Value::Int(2)),
        mul(&Value::Number(1e308), &Value::Int(10)),
        div(&Value::Int(i64::MIN), &Value::Int(-1)),
        div(&duration(i64::MIN), &Value::Int(-1)),
        neg(&Value::Int(i64::MIN)),
        neg(&duration(i64::MIN)),
    ];
    for result in overflowing {
        assert_problem(&result.unwrap_err(), "overflow");
    }
    let by_zero = [
        div(&Value::Int(1), &Value::Int(0)),
        div(&Value::Number(1.0), &Value::Number(0.0)),
        div(&size(1), &Value::Int(0)),
        div(&size(1), &Value::Number(-0.0)),
        div(&size(1), &size(0)),
        div(&duration(1), &Value::Int(0)),
        div(&duration(1), &duration(0)),
    ];
    for result in by_zero {
        assert_problem(&result.unwrap_err(), "division_by_zero");
    }
}

#[test]
fn rejects_operands_outside_the_arithmetic_rules() {
    let cases = [
        (
            add(&Value::from("a"), &Value::Int(1)),
            serde_json::json!(["text"]),
        ),
        (
            mul(&size(1), &size(1)),
            serde_json::json!(["int", "number"]),
        ),
        (
            add(
                &instant("2026-11-21T10:00:00Z"),
                &instant("2026-11-21T10:00:00Z"),
            ),
            serde_json::json!(["duration"]),
        ),
        (
            sub(&Value::Bool(true), &Value::Int(1)),
            serde_json::json!(["int", "number", "size", "duration", "instant"]),
        ),
        (
            neg(&Value::from("a")),
            serde_json::json!(["int", "number", "duration"]),
        ),
    ];
    for (result, expected) in cases {
        let error = result.unwrap_err();
        assert_code(&error, ErrorCode::Type);
        assert_eq!(error.expected, Some(expected));
    }
}

#[test]
fn negates_numbers_kept_as_text() {
    assert_eq!(
        neg(&Value::BigInt("9223372036854775808".to_owned())).unwrap(),
        Value::BigInt("-9223372036854775808".to_owned())
    );
    assert_eq!(
        neg(&Value::Decimal("-1.10".to_owned())).unwrap(),
        Value::Decimal("1.10".to_owned())
    );
    assert_eq!(neg(&Value::Number(2.5)).unwrap(), Value::Number(-2.5));
    assert_eq!(neg(&Value::Null).unwrap(), Value::Null);
}

fn number_value() -> impl Strategy<Value = Value> {
    let special = prop::sample::select(vec![
        0.5,
        -0.0,
        0.0,
        9_223_372_036_854_775_808.0,
        -9_223_372_036_854_775_808.0,
        9_223_372_036_854_774_784.0,
        1e300,
        f64::NAN,
    ]);
    prop_oneof![
        any::<i64>().prop_map(Value::Int),
        prop::sample::select(vec![i64::MAX, i64::MIN, 9_223_372_036_854_774_900, 0, -1])
            .prop_map(Value::Int),
        (-1e20_f64..1e20_f64).prop_map(Value::Number),
        special.prop_map(Value::Number),
        "-?[1-9][0-9]{19,24}".prop_map(Value::BigInt),
        "-?[0-9]{1,3}(\\.[0-9]{1,3})?(e-?[0-9]{1,2})?".prop_map(Value::Decimal),
        prop::sample::select(vec!["9223372036854774950", "9223372036854775000"])
            .prop_map(|text| Value::Decimal(text.to_owned())),
    ]
}

fn scalar_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        number_value(),
        "[a-c]{0,3}".prop_map(Value::Text),
        "[a-c]{0,3}".prop_map(Value::Enum),
        "[a-c/]{0,4}".prop_map(Value::Path),
        any::<u64>().prop_map(size),
        any::<i64>().prop_map(duration),
        (
            proptest::option::of(-3_i64..3),
            proptest::option::of(3_i64..6)
        )
            .prop_map(|(start, end)| Value::Range(Range::new(start, end).unwrap())),
    ]
}

fn any_value() -> impl Strategy<Value = Value> {
    scalar_value().prop_recursive(2, 12, 3, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..3).prop_map(Value::List),
            prop::collection::vec(("[ab]", inner), 0..3)
                .prop_map(|entries| Value::Map(entries.into_iter().collect())),
        ]
    })
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn order_is_reflexive_and_antisymmetric(left in any_value(), right in any_value()) {
        prop_assert_eq!(compare(&left, &left), Ordering::Equal);
        prop_assert_eq!(compare(&left, &right), compare(&right, &left).reverse());
    }

    #[test]
    fn order_is_transitive(mut values in prop::collection::vec(any_value(), 3..7)) {
        values.sort_by(compare);
        for (index, earlier) in values.iter().enumerate() {
            for later in values.iter().skip(index + 1) {
                prop_assert_ne!(compare(earlier, later), Ordering::Greater);
            }
        }
    }

    #[test]
    fn numbers_are_ordered_transitively(mut values in prop::collection::vec(number_value(), 3..8)) {
        values.sort_by(compare);
        for (index, earlier) in values.iter().enumerate() {
            for later in values.iter().skip(index + 1) {
                prop_assert_ne!(compare(earlier, later), Ordering::Greater);
            }
        }
    }

    #[test]
    fn text_concatenation_is_associative(a in ".{0,4}", b in ".{0,4}", c in ".{0,4}") {
        let (a, b, c) = (Value::Text(a), Value::Text(b), Value::Text(c));
        let left = add(&add(&a, &b).unwrap(), &c).unwrap();
        let right = add(&a, &add(&b, &c).unwrap()).unwrap();
        prop_assert_eq!(left, right);
    }

    #[test]
    fn size_addition_is_associative(a in 0_u64..u64::MAX / 3, b in 0_u64..u64::MAX / 3, c in 0_u64..u64::MAX / 3) {
        let (a, b, c) = (size(a), size(b), size(c));
        let left = add(&add(&a, &b).unwrap(), &c).unwrap();
        let right = add(&a, &add(&b, &c).unwrap()).unwrap();
        prop_assert_eq!(left, right);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
