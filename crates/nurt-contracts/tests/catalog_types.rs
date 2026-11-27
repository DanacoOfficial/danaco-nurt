// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/catalog_types.rs
// Responsibility:   Tests of the type expression parser, input and output kinds and default.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the type expression parser, input and output kinds and default
//! values of the command catalog.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::spec::{
    DefaultKind, DefaultValue, IoSpec, MAX_TYPE_DEPTH, ParamType, ScalarType, TypeParseErrorKind,
    is_command_name, is_identifier,
};

fn parse(text: &str) -> ParamType {
    text.parse()
        .unwrap_or_else(|error| panic!("{text:?} does not parse: {error}"))
}

fn error_kind(text: &str) -> TypeParseErrorKind {
    match text.parse::<ParamType>() {
        Ok(ty) => panic!("{text:?} parsed as {ty}"),
        Err(error) => error.kind,
    }
}

#[test]
fn every_scalar_keyword_parses() {
    for scalar in ScalarType::ALL {
        assert_eq!(parse(scalar.as_str()), ParamType::Scalar(*scalar));
        assert_eq!(ScalarType::from_keyword(scalar.as_str()), Some(*scalar));
    }
}

#[test]
fn compound_types_parse_into_trees() {
    assert_eq!(
        parse("path | list<path>"),
        ParamType::Union(vec![
            ParamType::Scalar(ScalarType::Path),
            ParamType::List(Box::new(ParamType::Scalar(ScalarType::Path))),
        ])
    );
    assert_eq!(
        parse("map<text>"),
        ParamType::Map(Box::new(ParamType::Scalar(ScalarType::Text)))
    );
    assert_eq!(
        parse("enum(file|directory)"),
        ParamType::Enum(vec!["file".to_owned(), "directory".to_owned()])
    );
    assert_eq!(parse("stream<any>"), ParamType::Stream("any".to_owned()));
    assert_eq!(
        parse("list<field | named_expression>"),
        ParamType::List(Box::new(ParamType::Union(vec![
            ParamType::Scalar(ScalarType::Field),
            ParamType::Scalar(ScalarType::NamedExpression),
        ])))
    );
}

#[test]
fn bare_list_and_map_mean_any_elements() {
    assert_eq!(
        parse("list"),
        ParamType::List(Box::new(ParamType::Scalar(ScalarType::Any)))
    );
    assert_eq!(parse("map").to_string(), "map<any>");
}

#[test]
fn canonical_text_round_trips() {
    let cases = [
        (
            "text|regex|list<text|regex>",
            "text | regex | list<text | regex>",
        ),
        ("  list < enum( vcs | deps ) >  ", "list<enum(vcs|deps)>"),
        ("duration | enum(forever)", "duration | enum(forever)"),
        (
            "pipeline | stream<any> | path",
            "pipeline | stream<any> | path",
        ),
        ("list<map>", "list<map<any>>"),
        ("int\t|\ttext", "int | text"),
    ];
    for (text, canonical) in cases {
        let ty = parse(text);
        assert_eq!(ty.to_string(), canonical);
        assert_eq!(parse(canonical), ty);
    }
}

#[test]
fn queries_report_alternatives_lists_scalars_and_enum_values() {
    let ty = parse("text | regex | list<text | regex>");
    assert_eq!(ty.alternatives().len(), 3);
    assert!(ty.accepts_list());
    assert!(ty.contains_scalar(ScalarType::Regex));
    assert!(!ty.contains_scalar(ScalarType::Path));
    assert!(!parse("path").accepts_list());
    assert_eq!(
        parse("path").alternatives(),
        &[ParamType::Scalar(ScalarType::Path)]
    );
    assert_eq!(
        parse("enum(a|b) | list<enum(b|c)>").enum_values(),
        vec!["a", "b", "c"]
    );
    assert!(parse("text").enum_values().is_empty());
}

#[test]
fn incomplete_types_end_unexpectedly() {
    assert_eq!(error_kind(""), TypeParseErrorKind::Empty);
    assert_eq!(error_kind("   "), TypeParseErrorKind::Empty);
    for text in ["list<", "list<text", "text |", "enum(a", "stream<any"] {
        assert!(
            matches!(error_kind(text), TypeParseErrorKind::UnexpectedEnd { .. }),
            "{text:?}"
        );
    }
}

#[test]
fn unexpected_characters_are_reported() {
    let cases = [
        ("enum()", ')'),
        ("enum(a,b)", ','),
        ("Text", 'T'),
        ("text text", 't'),
        ("list<text>>", '>'),
        ("stream<>", '>'),
        ("\u{e9}t\u{e9}", '\u{e9}'),
    ];
    for (text, expected) in cases {
        match error_kind(text) {
            TypeParseErrorKind::UnexpectedChar { found, .. } => {
                assert_eq!(found, expected, "{text:?}")
            }
            other => panic!("{text:?} gave {other:?}"),
        }
    }
}

#[test]
fn invalid_words_are_named_in_errors() {
    let cases = [
        (
            "string",
            TypeParseErrorKind::UnknownType {
                name: "string".to_owned(),
            },
        ),
        (
            "enum(1a)",
            TypeParseErrorKind::InvalidEnumValue {
                value: "1a".to_owned(),
            },
        ),
        (
            "enum(a|a)",
            TypeParseErrorKind::DuplicateEnumValue {
                value: "a".to_owned(),
            },
        ),
        (
            "text | int | text",
            TypeParseErrorKind::DuplicateAlternative {
                alternative: "text".to_owned(),
            },
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(error_kind(text), expected, "{text:?}");
    }
}

#[test]
fn errors_report_the_offset() {
    let error = "text | strng".parse::<ParamType>().unwrap_err();
    assert_eq!(error.offset, 7);
    let message = error.to_string();
    assert!(message.contains("strng"), "{message}");
    assert!(message.ends_with("at offset 7"), "{message}");
}

#[test]
fn nesting_deeper_than_the_limit_is_rejected() {
    let deep = format!(
        "{}text{}",
        "list<".repeat(MAX_TYPE_DEPTH + 2),
        ">".repeat(MAX_TYPE_DEPTH + 2)
    );
    assert_eq!(error_kind(&deep), TypeParseErrorKind::TooDeep);
    let allowed = format!(
        "{}text{}",
        "list<".repeat(MAX_TYPE_DEPTH),
        ">".repeat(MAX_TYPE_DEPTH)
    );
    assert_eq!(parse(&allowed).to_string(), allowed);
}

#[test]
fn types_serialize_as_canonical_text() {
    let ty = parse("glob|list<glob>");
    let json = serde_json::to_value(&ty).unwrap();
    assert_eq!(json, serde_json::json!("glob | list<glob>"));
    let back: ParamType = serde_json::from_value(json).unwrap();
    assert_eq!(back, ty);
    let invalid = serde_json::from_value::<ParamType>(serde_json::json!("list<"));
    assert!(invalid.is_err());
}

#[test]
fn io_kinds_parse_and_print() {
    let cases = [
        ("none", IoSpec::None),
        ("any", IoSpec::Any),
        ("same", IoSpec::Same),
        ("int", IoSpec::Value(ScalarType::Int)),
        ("text", IoSpec::Value(ScalarType::Text)),
        ("entry", IoSpec::Record("entry".to_owned())),
        ("git_status", IoSpec::Record("git_status".to_owned())),
    ];
    for (text, expected) in cases {
        let parsed: IoSpec = text.parse().unwrap();
        assert_eq!(parsed, expected);
        assert_eq!(parsed.to_string(), text);
        assert_eq!(
            serde_json::to_value(&parsed).unwrap(),
            serde_json::json!(text)
        );
    }
    assert!(IoSpec::None.is_none());
    assert!("".parse::<IoSpec>().is_err());
    assert!("Entry".parse::<IoSpec>().is_err());
    assert!("list<entry>".parse::<IoSpec>().is_err());
}

#[test]
fn default_values_are_classified() {
    let cases = [
        ("true", DefaultKind::Bool),
        ("false", DefaultKind::Bool),
        ("null", DefaultKind::Null),
        ("3", DefaultKind::Int),
        ("-1", DefaultKind::Int),
        ("1_000", DefaultKind::Int),
        ("3.5", DefaultKind::Number),
        ("10mb", DefaultKind::Size),
        ("1.5gib", DefaultKind::Size),
        ("512b", DefaultKind::Size),
        ("120s", DefaultKind::Duration),
        ("0s", DefaultKind::Duration),
        ("1h30m", DefaultKind::Duration),
        ("250ms", DefaultKind::Duration),
        ("1..5", DefaultKind::Range),
        ("40..", DefaultKind::Range),
        ("..9", DefaultKind::Range),
        ("\"**\"", DefaultKind::Text),
        ("r\"a\\b\"", DefaultKind::Text),
        ("r#\"x\"#", DefaultKind::Text),
        ("/v\\d+/", DefaultKind::Regex),
        ("[]", DefaultKind::List),
        ("{}", DefaultKind::Map),
        ("asc", DefaultKind::Word),
        ("all", DefaultKind::Word),
        ("10 mb", DefaultKind::Other),
        ("5x", DefaultKind::Other),
        ("", DefaultKind::Other),
    ];
    for (text, kind) in cases {
        assert_eq!(DefaultValue::new(text).kind(), kind, "{text:?}");
    }
    assert!(DefaultValue::new(" ").is_blank());
    assert_eq!(DefaultValue::new("asc").as_str(), "asc");
    assert_eq!(
        serde_json::to_value(DefaultValue::new("3s")).unwrap(),
        serde_json::json!("3s")
    );
}

#[test]
fn names_follow_the_identifier_rules() {
    assert!(is_identifier("on_conflict"));
    assert!(!is_identifier("on-conflict"));
    assert!(!is_identifier("_hidden"));
    assert!(is_command_name("windows.cim"));
    assert!(!is_command_name("windows."));
    assert!(!is_command_name("Git.status"));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
