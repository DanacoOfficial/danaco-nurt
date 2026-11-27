// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/value_types.rs
// Responsibility:   Units, instants, ranges, regular expressions, bytes, values and the.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// Tests assert through `unwrap`, `expect` and `panic!`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Units, instants, ranges, regular expressions, bytes, values and the
//! conversion between dynamic and typed records.

use std::str::FromStr;

use nurt_contracts::record::{
    Change, ChangeAction, Content, Entry, EntryType, LineEnding, RecordError, TypedRecord,
};
use nurt_contracts::value::{
    Bytes, Duration, Fields, Instant, Path, Range, Record, RecordKind, Regex, RegexFlags, Size,
    Value,
};
use serde_json::json;

#[test]
fn instant_formats_in_utc_with_minimal_fraction() {
    let cases = [
        (0, "1970-01-01T00:00:00Z"),
        (1_000_000, "1970-01-01T00:00:00.001Z"),
        (1_000, "1970-01-01T00:00:00.000001Z"),
        (1, "1970-01-01T00:00:00.000000001Z"),
        (-1_000_000_000, "1969-12-31T23:59:59Z"),
        (-1, "1969-12-31T23:59:59.999999999Z"),
        (1_795_255_200_000_000_000, "2026-11-21T10:00:00Z"),
        (951_782_400_000_000_000, "2000-02-29T00:00:00Z"),
    ];
    for (nanos, text) in cases {
        let instant = Instant::from_unix_nanos(nanos);
        assert_eq!(instant.to_string(), text);
        assert_eq!(Instant::from_str(text).unwrap(), instant, "{text}");
    }
}

#[test]
fn instant_parses_offsets_and_fractions() {
    let utc = Instant::from_str("2026-11-21T10:00:00Z").unwrap();
    assert_eq!(Instant::from_str("2026-11-21T11:00:00+01:00").unwrap(), utc);
    assert_eq!(Instant::from_str("2026-11-21T05:30:00-04:30").unwrap(), utc);
    assert_eq!(Instant::from_str("2026-11-21t10:00:00z").unwrap(), utc);
    assert_eq!(
        Instant::from_str("2026-11-21T10:00:00.5Z")
            .unwrap()
            .to_string(),
        "2026-11-21T10:00:00.500Z"
    );
    assert_eq!(
        Instant::from_str("2026-11-21T10:00:00.1234Z")
            .unwrap()
            .to_string(),
        "2026-11-21T10:00:00.123400Z"
    );
}

#[test]
fn instant_rejects_invalid_text() {
    for text in [
        "",
        "2026-11-21",
        "2026-11-21 10:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-02-29T00:00:00Z",
        "2100-02-29T00:00:00Z",
        "2026-11-31T00:00:00Z",
        "2026-11-21T24:00:00Z",
        "2026-11-21T10:60:00Z",
        "2026-11-21T10:00:60Z",
        "2026-11-21T10:00:00",
        "2026-11-21T10:00:00.Z",
        "2026-11-21T10:00:00.1234567891Z",
        "2026-11-21T10:00:00+0100",
        "2026-11-21T10:00:00Zx",
        "9999-12-31T23:59:59Z",
    ] {
        assert!(Instant::from_str(text).is_err(), "{text} must be rejected");
    }
    assert!(Instant::from_str("2024-02-29T00:00:00Z").is_ok());
}

#[test]
fn instant_arithmetic_and_system_time() {
    let start = Instant::from_str("2026-11-21T10:00:00Z").unwrap();
    let later = start.checked_add(Duration::from_millis(1500)).unwrap();
    assert_eq!(later.to_string(), "2026-11-21T10:00:01.500Z");
    assert_eq!(
        later.duration_since(start),
        Some(Duration::from_millis(1500))
    );
    assert_eq!(later.checked_sub(Duration::from_millis(1500)), Some(start));
    let system = start.to_system_time().unwrap();
    assert_eq!(Instant::from_system_time(system), Some(start));
    let before_epoch = Instant::from_unix_nanos(-5_000_000_000);
    assert_eq!(
        Instant::from_system_time(before_epoch.to_system_time().unwrap()),
        Some(before_epoch)
    );
    assert_eq!(
        Instant::from_unix_seconds(60).unwrap().to_string(),
        "1970-01-01T00:01:00Z"
    );
}

#[test]
fn sizes_and_durations_serialize_as_integers() {
    let size = Size::from_bytes(1024);
    assert_eq!(size.bytes(), 1024);
    assert_eq!(
        size.checked_add(Size::from_bytes(1)),
        Some(Size::from_bytes(1025))
    );
    assert_eq!(Size::ZERO.checked_sub(size), None);
    assert_eq!(serde_json::to_value(size).unwrap(), json!(1024));
    let duration = Duration::from_secs(90).unwrap();
    assert_eq!(duration.as_millis(), 90_000);
    assert_eq!(duration.to_std(), Some(std::time::Duration::from_secs(90)));
    assert_eq!(Duration::from_millis(-1).to_std(), None);
    assert!(Duration::from_millis(-1).is_negative());
    assert_eq!(
        Duration::from_std(std::time::Duration::from_millis(250)),
        Some(Duration::from_millis(250))
    );
    assert_eq!(serde_json::to_value(duration).unwrap(), json!(90_000));
}

#[test]
fn ranges_are_inclusive_and_ordered() {
    assert_eq!(Range::single(5).to_string(), "5");
    assert_eq!(Range::new(Some(1), Some(5)).unwrap().to_string(), "1..5");
    assert_eq!(Range::from_start(40).to_string(), "40..");
    assert_eq!(Range::up_to(9).to_string(), "..9");
    assert_eq!(Range::full().to_string(), "..");
    assert!(Range::new(Some(5), Some(1)).is_err());
    assert!(Range::from_start(40).contains(41));
    assert!(!Range::up_to(9).contains(10));
    assert_eq!(
        serde_json::to_value(Range::from_start(40)).unwrap(),
        json!({"start": 40})
    );
    assert_eq!(serde_json::to_value(Range::full()).unwrap(), json!({}));
    let range: Range = serde_json::from_value(json!({"start": 1, "end": 5})).unwrap();
    assert_eq!(range, Range::new(Some(1), Some(5)).unwrap());
    assert!(serde_json::from_value::<Range>(json!({"start": 5, "end": 1})).is_err());
    assert!(serde_json::from_value::<Range>(json!({"start": 1, "step": 2})).is_err());
}

#[test]
fn regex_flags_are_canonical_and_slashes_escaped() {
    let regex = Regex::new("a/b+", "xi").unwrap();
    assert_eq!(regex.flags.to_string(), "ix");
    assert_eq!(regex.to_string(), "/a\\/b+/ix");
    assert_eq!(Regex::new("a\\/b", "").unwrap().to_string(), "/a\\/b/");
    assert_eq!(Regex::new("\\\\/", "").unwrap().to_string(), "/\\\\\\//");
    assert_eq!(
        serde_json::to_value(&regex).unwrap(),
        json!({"pattern": "a/b+", "flags": "ix"})
    );
    assert_eq!(
        serde_json::to_value(Regex::new("x", "").unwrap()).unwrap(),
        json!({"pattern": "x"})
    );
    assert!(RegexFlags::from_str("ii").is_err());
    assert!(RegexFlags::from_str("g").is_err());
    assert!(RegexFlags::from_str("").unwrap().is_empty());
}

#[test]
fn base64_matches_rfc_4648_vectors() {
    let vectors = [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ];
    for (plain, encoded) in vectors {
        let bytes = Bytes::new(plain.as_bytes().to_vec());
        assert_eq!(bytes.to_base64(), encoded);
        assert_eq!(Bytes::from_base64(encoded).unwrap(), bytes);
    }
    let all: Vec<u8> = (0..=255).collect();
    let bytes = Bytes::new(all.clone());
    assert_eq!(
        Bytes::from_base64(&bytes.to_base64()).unwrap().into_vec(),
        all
    );
    for invalid in ["Zg=", "Z===", "Zg==Zg==", "Zm9v!", "=Zm9"] {
        assert!(Bytes::from_base64(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn tagged_json_form_of_values() {
    let mut map = Fields::new();
    map.insert("b".to_owned(), Value::Int(1));
    map.insert("a".to_owned(), Value::Null);
    let cases = [
        (Value::Null, json!({"t": "null"})),
        (Value::Bool(true), json!({"t": "bool", "v": true})),
        (Value::Int(-3), json!({"t": "int", "v": -3})),
        (
            Value::BigInt("18446744073709551616".to_owned()),
            json!({"t": "big_int", "v": "18446744073709551616"}),
        ),
        (Value::Number(2.5), json!({"t": "number", "v": 2.5})),
        (
            Value::Decimal("1.10".to_owned()),
            json!({"t": "decimal", "v": "1.10"}),
        ),
        (
            Value::Size(Size::from_bytes(10)),
            json!({"t": "size", "v": 10}),
        ),
        (
            Value::Duration(Duration::from_millis(250)),
            json!({"t": "duration", "v": 250}),
        ),
        (
            Value::Instant(Instant::from_unix_nanos(0)),
            json!({"t": "instant", "v": "1970-01-01T00:00:00Z"}),
        ),
        (
            Value::Path("/tmp".to_owned()),
            json!({"t": "path", "v": "/tmp"}),
        ),
        (
            Value::Bytes(Bytes::new(b"foo".to_vec())),
            json!({"t": "bytes", "v": "Zm9v"}),
        ),
        (
            Value::Enum("asc".to_owned()),
            json!({"t": "enum", "v": "asc"}),
        ),
        (
            Value::Map(map),
            json!({"t": "map", "v": {"b": {"t": "int", "v": 1}, "a": {"t": "null"}}}),
        ),
    ];
    for (value, expected) in cases {
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(json, expected);
        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            serde_json::to_string(&expected).unwrap()
        );
        assert_eq!(serde_json::from_value::<Value>(json).unwrap(), value);
    }
    assert!(serde_json::from_value::<Value>(json!({"t": "int", "v": 1, "x": 2})).is_err());
    assert!(serde_json::from_value::<Value>(json!({"t": "size", "v": -1})).is_err());
}

#[test]
fn plain_json_form_of_values() {
    let record = Record::new(RecordKind::Map)
        .with("size", Value::Size(Size::from_bytes(12)))
        .with("when", Value::Instant(Instant::from_unix_nanos(0)))
        .with("lines", Value::Range(Range::new(Some(1), Some(5)).unwrap()))
        .with("pattern", Value::Regex(Regex::new("a+", "i").unwrap()))
        .with("nan", Value::Number(f64::NAN));
    assert_eq!(
        Value::Record(record).to_plain_json(),
        json!({
            "size": 12,
            "when": "1970-01-01T00:00:00Z",
            "lines": "1..5",
            "pattern": "/a+/i",
            "nan": null
        })
    );
    let plain = json!({"n": 1, "big": 18_446_744_073_709_551_615_u64, "x": 0.5, "s": "t", "l": [null, true]});
    let value = Value::from_plain_json(plain.clone());
    assert_eq!(
        value.get("big"),
        Some(&Value::BigInt("18446744073709551615".to_owned()))
    );
    assert_eq!(value.get("x"), Some(&Value::Number(0.5)));
    assert_eq!(value.get("s").and_then(Value::as_str), Some("t"));
    assert_eq!(value.to_plain_json()["l"], json!([null, true]));
}

#[test]
fn value_accessors_return_payloads() {
    assert_eq!(Value::from(3).as_int(), Some(3));
    assert_eq!(Value::from(3).as_number(), Some(3.0));
    assert_eq!(Value::from(u64::MAX).type_name(), "int");
    assert_eq!(Value::from(true).as_bool(), Some(true));
    assert_eq!(Value::from("x").as_str(), Some("x"));
    assert!(Value::default().is_null());
    assert_eq!(
        Value::from(vec![Value::Null]).as_list().map(<[Value]>::len),
        Some(1)
    );
    assert_eq!(Value::from(Path::from("/a")).as_str(), Some("/a"));
    assert_eq!(
        Value::Record(Record::new(RecordKind::Map)).type_name(),
        "record"
    );
}

#[test]
fn record_kinds_round_trip() {
    for &kind in RecordKind::ALL {
        assert_eq!(RecordKind::from_str(kind.as_str()), Ok(kind));
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.as_str()));
    }
    assert!(RecordKind::from_str("unknown_kind").is_err());
    for &kind in TypedRecord::KINDS {
        assert!(RecordKind::ALL.contains(&kind));
    }
    let untyped: Vec<RecordKind> = RecordKind::ALL
        .iter()
        .copied()
        .filter(|kind| !TypedRecord::KINDS.contains(kind))
        .collect();
    assert_eq!(untyped, [RecordKind::Map]);
}

#[test]
fn record_operations_keep_order() {
    let mut record = Record::new(RecordKind::Map)
        .with("b", Value::Int(1))
        .with("a", Value::Int(2));
    assert_eq!(record.set("b", Value::Int(3)), Some(Value::Int(1)));
    assert_eq!(record.field_names().collect::<Vec<_>>(), ["b", "a"]);
    assert_eq!(record.remove("b"), Some(Value::Int(3)));
    assert_eq!(record.len(), 1);
    assert!(!record.is_empty());
    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(
        json,
        json!({"kind": "map", "fields": {"a": {"t": "int", "v": 2}}})
    );
}

fn entry() -> Entry {
    Entry {
        path: Path::from("/home/user/notes.txt"),
        name: "notes.txt".to_owned(),
        kind: EntryType::File,
        size: Size::from_bytes(1200),
        modified: Instant::from_str("2026-11-21T09:12:00Z").unwrap(),
        line_ending: Some(LineEnding::Crlf),
        acl: Some(vec!["user::rw-".to_owned()]),
        ..Entry::default()
    }
}

#[test]
fn typed_records_convert_with_language_types() {
    let record = Record::from(entry());
    assert_eq!(record.kind, RecordKind::Entry);
    assert_eq!(
        record.field_names().collect::<Vec<_>>(),
        [
            "path",
            "name",
            "type",
            "size",
            "modified",
            "acl",
            "line_ending"
        ]
    );
    assert_eq!(
        record.get("path"),
        Some(&Value::Path("/home/user/notes.txt".to_owned()))
    );
    assert_eq!(record.get("type"), Some(&Value::Enum("file".to_owned())));
    assert_eq!(
        record.get("line_ending"),
        Some(&Value::Enum("crlf".to_owned()))
    );
    assert_eq!(Entry::try_from(record).unwrap(), entry());
}

#[test]
fn nested_records_convert() {
    let content = Content {
        path: Path::from("/home/user/notes.txt"),
        text: "a\nb\n".to_owned(),
        lines: 2,
        size: Size::from_bytes(4),
        etag: "sha256:00".to_owned(),
        encoding: "utf-8".to_owned(),
        line_ending: LineEnding::Lf,
        ..Content::default()
    };
    let change = Change {
        path: Path::from("/home/user/notes.txt"),
        action: ChangeAction::Modify,
        ..Change::default()
    };
    let typed = TypedRecord::from(content.clone());
    assert_eq!(typed.kind(), RecordKind::Content);
    let record = Record::from(typed.clone());
    assert_eq!(TypedRecord::try_from(record).unwrap(), typed);
    let record = Record::from(change.clone());
    assert_eq!(Change::try_from(record).unwrap(), change);
}

#[test]
fn conversion_errors_name_the_field() {
    let mut record = Record::from(entry());
    record.set("size", Value::Text("big".to_owned()));
    assert_eq!(
        Entry::try_from(record).unwrap_err(),
        RecordError::FieldType {
            field: "size".to_owned(),
            expected: "size".to_owned(),
            found: "text".to_owned()
        }
    );
    let mut record = Record::from(entry());
    record.remove("name");
    assert_eq!(
        Entry::try_from(record).unwrap_err(),
        RecordError::MissingField {
            field: "name".to_owned()
        }
    );
    let record = Record::from(entry()).with("colour", Value::Null);
    assert_eq!(
        Entry::try_from(record).unwrap_err(),
        RecordError::UnknownField {
            field: "colour".to_owned()
        }
    );
    let mut record = Record::from(entry());
    record.set("acl", Value::List(vec![Value::Int(1)]));
    assert_eq!(
        Entry::try_from(record).unwrap_err(),
        RecordError::FieldType {
            field: "acl.0".to_owned(),
            expected: "text".to_owned(),
            found: "int".to_owned()
        }
    );
    let mut record = Record::from(entry());
    record.set("type", Value::Enum("socket".to_owned()));
    assert!(
        matches!(Entry::try_from(record), Err(RecordError::FieldType { field, .. }) if field == "type")
    );
    assert_eq!(
        TypedRecord::try_from(Record::new(RecordKind::Map)).unwrap_err(),
        RecordError::Untyped {
            kind: RecordKind::Map
        }
    );
}

#[test]
fn plain_json_of_typed_records() {
    let json = serde_json::to_value(entry()).unwrap();
    assert_eq!(
        json,
        json!({
            "path": "/home/user/notes.txt",
            "name": "notes.txt",
            "type": "file",
            "size": 1200,
            "modified": "2026-11-21T09:12:00Z",
            "acl": ["user::rw-"],
            "line_ending": "crlf"
        })
    );
    let typed = serde_json::to_value(TypedRecord::from(entry())).unwrap();
    assert_eq!(typed["kind"], "entry");
    assert_eq!(typed["record"], json);
    assert_eq!(Record::from(entry()).to_plain_json(), json);
}

#[test]
fn enumerations_round_trip() {
    for &value in EntryType::ALL {
        assert_eq!(EntryType::from_str(value.as_str()), Ok(value));
    }
    for &value in LineEnding::ALL {
        assert_eq!(serde_json::to_value(value).unwrap(), json!(value.as_str()));
    }
    assert!(EntryType::from_str("socket").is_err());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
