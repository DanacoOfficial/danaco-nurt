// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Text forms of sizes, durations, instants and ranges.

// Assertions unwrap results whose failure is the failure of the test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use jiff::tz::{self, TimeZone};
use nurt_contracts::value::{Duration, Instant, Range, Size, Value};
use nurt_types::{
    display_duration, display_instant, display_short, display_size, format_duration, format_size,
    parse_duration, parse_instant, parse_range, parse_size,
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

#[test]
fn parses_sizes_with_decimal_and_binary_units() {
    let cases: [(&str, u64); 13] = [
        ("0b", 0),
        ("512b", 512),
        ("1kb", 1_000),
        ("1kib", 1_024),
        ("10mb", 10_000_000),
        ("1.5gib", 1_610_612_736),
        ("2TB", 2_000_000_000_000),
        ("3Tib", 3 * 1_099_511_627_776),
        ("1_000b", 1_000),
        ("0.5kb", 500),
        ("1.0005kb", 1_001),
        ("0.1kib", 102),
        ("18446744073709551615b", u64::MAX),
    ];
    for (text, bytes) in cases {
        assert_eq!(parse_size(text).unwrap().bytes(), bytes, "{text}");
    }
}

#[test]
fn rejects_malformed_and_oversized_sizes() {
    let cases = [
        "",
        "10",
        "kb",
        "1.kb",
        ".5kb",
        "1x",
        "1kbx",
        "1 kb",
        "-1kb",
        "_1kb",
        "1.1234567890123456789kb",
        "18446744073709551616b",
        "16777216tib",
    ];
    for text in cases {
        let error = parse_size(text).unwrap_err();
        assert_eq!(error.type_name, "size", "{text}");
        assert_eq!(error.input, text);
    }
}

#[test]
fn formats_sizes_with_the_largest_exact_unit() {
    let cases: [(u64, &str); 9] = [
        (0, "0b"),
        (1, "1b"),
        (1_000, "1kb"),
        (1_024, "1kib"),
        (1_200, "1200b"),
        (2_048_000, "2000kib"),
        (1_048_576, "1mib"),
        (5_000_000_000_000, "5tb"),
        (u64::MAX, "18446744073709551615b"),
    ];
    for (bytes, text) in cases {
        assert_eq!(format_size(Size::from_bytes(bytes)), text);
    }
}

#[test]
fn displays_sizes_in_the_short_report_form() {
    let cases: [(u64, &str); 10] = [
        (0, "0b"),
        (12, "12b"),
        (999, "999b"),
        (1_000, "1.0kb"),
        (8_000, "8.0kb"),
        (41_230, "41.2kb"),
        (999_949, "999.9kb"),
        (999_950, "1.0mb"),
        (3_400_000_000, "3.4gb"),
        (u64::MAX, "18446744.1tb"),
    ];
    for (bytes, text) in cases {
        assert_eq!(display_size(Size::from_bytes(bytes)), text, "{bytes}");
    }
}

#[test]
fn parses_durations_with_combined_units() {
    let cases: [(&str, i64); 12] = [
        ("250ms", 250),
        ("30s", 30_000),
        ("5m", 300_000),
        ("1h30m", 5_400_000),
        ("30m1h", 5_400_000),
        ("7d", 604_800_000),
        ("1w", 604_800_000),
        ("1d2h3m4s5ms", 93_784_005),
        ("-1h", -3_600_000),
        ("1_000ms", 1_000),
        ("0s", 0),
        ("-9223372036854775808ms", i64::MIN),
    ];
    for (text, millis) in cases {
        assert_eq!(parse_duration(text).unwrap().as_millis(), millis, "{text}");
    }
}

#[test]
fn rejects_malformed_and_oversized_durations() {
    let cases = [
        "",
        "-",
        "1",
        "h",
        "1h1h",
        "1.5h",
        "1hour",
        "1H",
        "1h 30m",
        "9223372036854775808ms",
        "-9223372036854775809ms",
        "99999999999999999999999999999999999999999w",
    ];
    for text in cases {
        assert_eq!(
            parse_duration(text).unwrap_err().type_name,
            "duration",
            "{text}"
        );
    }
}

#[test]
fn formats_durations_canonically() {
    let cases: [(i64, &str); 6] = [
        (0, "0s"),
        (250, "250ms"),
        (1_000, "1s"),
        (5_400_000, "1h30m"),
        (93_784_005, "1d2h3m4s5ms"),
        (-1_250, "-1s250ms"),
    ];
    for (millis, text) in cases {
        assert_eq!(format_duration(Duration::from_millis(millis)), text);
    }
}

#[test]
fn displays_durations_in_the_short_report_form() {
    let cases: [(i64, &str); 11] = [
        (0, "0ms"),
        (999, "999ms"),
        (1_000, "1s"),
        (1_240, "1.2s"),
        (59_949, "59.9s"),
        (59_950, "1m"),
        (90_000, "1m30s"),
        (3_600_000, "1h"),
        (5_412_000, "1h30m"),
        (93_784_005, "1d2h"),
        (-1_240, "-1.2s"),
    ];
    for (millis, text) in cases {
        assert_eq!(
            display_duration(Duration::from_millis(millis)),
            text,
            "{millis}"
        );
    }
}

#[test]
fn parses_instants_with_and_without_offsets() {
    let utc = TimeZone::UTC;
    let plus_one = TimeZone::fixed(tz::offset(1));
    let cases = [
        ("2026-11-21T10:00:00Z", &utc, "2026-11-21T10:00:00Z"),
        ("2026-11-21T11:00:00+01:00", &utc, "2026-11-21T10:00:00Z"),
        ("2026-11-21", &utc, "2026-11-21T00:00:00Z"),
        ("2026-11-21", &plus_one, "2026-11-20T23:00:00Z"),
        ("2026-11-21T10:00:00", &plus_one, "2026-11-21T09:00:00Z"),
        ("2026-11-21T10:00:00.5", &utc, "2026-11-21T10:00:00.500Z"),
    ];
    for (text, rules, expected) in cases {
        assert_eq!(
            parse_instant(text, rules).unwrap().to_string(),
            expected,
            "{text}"
        );
    }
}

#[test]
fn rejects_texts_that_are_not_instants() {
    let cases = [
        "",
        "yesterday",
        "2026-13-01",
        "2026-02-30",
        "2026-11-21T25:00:00",
        "2026-11-21T10:00:00+0100",
        "21.11.2026",
    ];
    for text in cases {
        assert_eq!(
            parse_instant(text, &TimeZone::UTC).unwrap_err().type_name,
            "instant",
            "{text}"
        );
    }
}

#[test]
fn displays_instants_to_the_second() {
    let instant: Instant = "2026-11-21T10:00:00.750Z".parse().unwrap();
    assert_eq!(display_instant(instant), "2026-11-21T10:00:00Z");
    assert_eq!(
        display_instant(Instant::from_unix_nanos(-1)),
        "1969-12-31T23:59:59Z"
    );
}

#[test]
fn parses_closed_open_and_single_ranges() {
    let cases: [(&str, Option<i64>, Option<i64>); 7] = [
        ("5", Some(5), Some(5)),
        ("1..5", Some(1), Some(5)),
        ("40..", Some(40), None),
        ("..9", None, Some(9)),
        ("..", None, None),
        ("-3..-1", Some(-3), Some(-1)),
        ("1_000..2_000", Some(1_000), Some(2_000)),
    ];
    for (text, start, end) in cases {
        let range = parse_range(text).unwrap();
        assert_eq!((range.start(), range.end()), (start, end), "{text}");
    }
}

#[test]
fn rejects_malformed_and_reversed_ranges() {
    for text in [
        "",
        "5..1",
        "a..b",
        "1...5",
        "1..2..3",
        "1.5",
        "99999999999999999999",
    ] {
        assert!(parse_range(text).is_err(), "{text}");
    }
}

#[test]
fn short_form_covers_every_kind_of_value() {
    let instant: Instant = "2026-11-21T10:00:00.250Z".parse().unwrap();
    let cases = [
        (Value::Null, "null"),
        (Value::Bool(true), "true"),
        (Value::Int(-3), "-3"),
        (Value::Number(0.5), "0.5"),
        (
            Value::BigInt("9223372036854775808".to_owned()),
            "9223372036854775808",
        ),
        (Value::Size(Size::from_bytes(41_230)), "41.2kb"),
        (Value::Duration(Duration::from_millis(1_240)), "1.2s"),
        (Value::Instant(instant), "2026-11-21T10:00:00Z"),
        (Value::Path("/srv/app".to_owned()), "/srv/app"),
        (Value::Enum("file".to_owned()), "file"),
        (Value::Range(Range::from_start(40)), "40.."),
        (Value::List(vec![Value::Int(1), Value::from("a")]), "[1, a]"),
    ];
    for (value, text) in cases {
        assert_eq!(display_short(&value), text);
    }
    let map = Value::Map([("a".to_owned(), Value::Int(1))].into_iter().collect());
    assert_eq!(display_short(&map), "{a: 1}");
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn size_round_trips_through_the_canonical_form(bytes in any::<u64>()) {
        let size = Size::from_bytes(bytes);
        prop_assert_eq!(parse_size(&format_size(size)).unwrap(), size);
    }

    #[test]
    fn duration_round_trips_through_the_canonical_form(millis in any::<i64>()) {
        let duration = Duration::from_millis(millis);
        prop_assert_eq!(parse_duration(&format_duration(duration)).unwrap(), duration);
    }

    #[test]
    fn instant_round_trips_through_the_canonical_form(nanos in any::<i64>()) {
        let instant = Instant::from_unix_nanos(nanos);
        prop_assert_eq!(parse_instant(&instant.to_string(), &TimeZone::UTC).unwrap(), instant);
    }

    #[test]
    fn range_round_trips_through_the_canonical_form(
        first in proptest::option::of(any::<i64>()),
        second in proptest::option::of(any::<i64>()),
    ) {
        let (start, end) = match (first, second) {
            (Some(a), Some(b)) => (Some(a.min(b)), Some(a.max(b))),
            other => other,
        };
        let range = Range::new(start, end).unwrap();
        prop_assert_eq!(parse_range(&range.to_string()).unwrap(), range);
    }

    #[test]
    fn short_size_has_no_more_than_three_integer_digits_below_a_petabyte(bytes in 0_u64..1_000_000_000_000_000) {
        let text = display_size(Size::from_bytes(bytes));
        let integer_digits = text.chars().take_while(char::is_ascii_digit).count();
        prop_assert!(integer_digits <= 3, "{}", text);
    }
}
