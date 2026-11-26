// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Implicit conversions of texts to typed values and text functions on
//! multi-byte characters.

// Assertions unwrap results whose failure is the failure of the test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use jiff::tz::{self, TimeZone};
use nurt_contracts::bound::BoundExpr;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::{Duration, Instant, Value};
use nurt_types::{Evaluator, FixedEnv, ValueType, coerce, coerce_in};

fn instant(text: &str) -> Value {
    Value::Instant(text.parse::<Instant>().unwrap())
}

fn text(value: &str) -> BoundExpr {
    BoundExpr::Text {
        v: value.to_owned(),
    }
}

fn call(function: &str, args: Vec<BoundExpr>) -> Value {
    let expr = BoundExpr::Call {
        function: function.to_owned(),
        args,
    };
    let env = FixedEnv::new(Instant::UNIX_EPOCH, 1);
    Evaluator::new().eval(&expr, None, &env).unwrap()
}

#[test]
fn coerce_reads_day_and_week_durations() {
    let cases = [
        ("24h", 86_400_000),
        ("1w", 604_800_000),
        ("1w1d", 691_200_000),
    ];
    for (written, millis) in cases {
        assert_eq!(
            coerce(Value::from(written), ValueType::Duration).unwrap(),
            Value::Duration(Duration::from_millis(millis)),
            "{written}"
        );
    }
}

#[test]
fn coerce_applies_the_local_time_rules_only_to_instants_without_an_offset() {
    let plus_two = TimeZone::fixed(tz::offset(2));
    assert_eq!(
        coerce_in(Value::from("2026-11-21"), ValueType::Instant, &plus_two).unwrap(),
        instant("2026-11-20T22:00:00Z")
    );
    assert_eq!(
        coerce_in(
            Value::from("2026-11-21T10:00:00Z"),
            ValueType::Instant,
            &plus_two
        )
        .unwrap(),
        instant("2026-11-21T10:00:00Z")
    );
}

#[test]
fn coerce_keeps_multi_byte_text_of_paths_globs_and_keys() {
    let written = "/srv/ünï/日本語/*.rs";
    assert_eq!(
        coerce(Value::from(written), ValueType::Path).unwrap(),
        Value::Path(written.to_owned())
    );
    assert_eq!(
        coerce(Value::from(written), ValueType::Glob).unwrap(),
        Value::Glob(written.to_owned())
    );
    assert_eq!(
        coerce(Value::from("ключ.日本"), ValueType::Key).unwrap(),
        Value::Key("ключ.日本".to_owned())
    );
}

#[test]
fn coerce_reports_the_malformed_multi_byte_text_as_found() {
    for (written, target) in [
        ("10мб", ValueType::Size),
        ("1ч", ValueType::Duration),
        ("вчера", ValueType::Instant),
    ] {
        let error = coerce(Value::from(written), target).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidValue, "{written}");
        assert_eq!(error.found.as_deref(), Some(written));
    }
}

#[test]
fn coerce_rejects_values_that_are_not_texts_for_typed_places() {
    for target in [
        ValueType::Size,
        ValueType::Duration,
        ValueType::Instant,
        ValueType::Glob,
    ] {
        let error = coerce(Value::Int(1), target).unwrap_err();
        assert_eq!(error.code, ErrorCode::Type, "{target}");
        assert_eq!(error.expected, Some(serde_json::json!([target.as_str()])));
    }
    let error = coerce(Value::Path("/a".to_owned()), ValueType::Glob).unwrap_err();
    assert_eq!(error.code, ErrorCode::Type);
    assert_eq!(
        coerce(Value::Null, ValueType::Duration).unwrap(),
        Value::Null
    );
}

#[test]
fn text_functions_count_characters_and_keep_multi_byte_text() {
    assert_eq!(call("len", vec![text("日本語")]), Value::Int(3));
    assert_eq!(call("upper", vec![text("straße")]), Value::from("STRASSE"));
    assert_eq!(
        call("replace_text", vec![text("añb"), text("ñ"), text("n")]),
        Value::from("anb")
    );
    assert_eq!(
        call("parent", vec![text("/srv/ü/ß")]),
        Value::Path("/srv/ü".to_owned())
    );
    assert_eq!(
        call(
            "join_path",
            vec![text("/srv/ü"), text("日本"), text("../é")]
        ),
        Value::Path("/srv/ü/é".to_owned())
    );
}

#[test]
fn ago_reads_a_week_written_as_text() {
    assert_eq!(
        call("ago", vec![text("1w")]),
        instant("1969-12-25T00:00:00Z")
    );
}
