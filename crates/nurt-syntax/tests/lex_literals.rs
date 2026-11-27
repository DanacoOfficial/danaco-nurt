// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/lex_literals.rs
// Responsibility:   Literal tokens and their decoders: integers, numbers, sizes, durations.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Literal tokens and their decoders: integers, numbers, sizes, durations,
//! texts, raw texts and regular expressions.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::ast::Expr;
use nurt_contracts::error::{Error, ErrorCode, Stage};
use nurt_syntax::lex::{
    LexError, LexOptions, MAX_SAFE_INTEGER, TokenKind, decode_raw, decode_regex, decode_text,
    duration_ms, parse_duration, parse_int, parse_number, parse_size, size_bytes, tokenize,
};

fn kinds(text: &str) -> Vec<TokenKind> {
    tokenize(text, LexOptions::default())
        .unwrap()
        .into_iter()
        .map(|token| token.kind)
        .collect()
}

fn error(text: &str) -> LexError {
    tokenize(text, LexOptions::default()).unwrap_err()
}

fn text(value: &str) -> Expr {
    Expr::Text {
        v: value.to_owned(),
        span: None,
    }
}

fn int(src: &str) -> i64 {
    match parse_int(src).unwrap() {
        Expr::Int { v, .. } => v,
        other => panic!("{other:?}"),
    }
}

#[test]
fn reads_decimal_and_hexadecimal_integers() {
    assert_eq!(int("0"), 0);
    assert_eq!(int("007"), 7);
    assert_eq!(int("1_000_000"), 1_000_000);
    assert_eq!(int("1_"), 1);
    assert_eq!(int("0x1F"), 31);
    assert_eq!(int("0xff_ff"), 65_535);
    assert_eq!(kinds("1_000"), [TokenKind::Int, TokenKind::Eof]);
}

#[test]
fn limits_integers_to_the_safe_range() {
    assert_eq!(int("9007199254740991"), MAX_SAFE_INTEGER);
    assert_eq!(int("0x1FFFFFFFFFFFFF"), MAX_SAFE_INTEGER);
    for src in [
        "9007199254740992",
        "0x20000000000000",
        "99999999999999999999999",
    ] {
        let error = error(src);
        assert_eq!(
            (error.code, error.problem.as_str()),
            (ErrorCode::Lexical, "number_too_large")
        );
        assert_eq!(error.found, src);
    }
}

#[test]
fn rejects_malformed_hexadecimal_integers() {
    assert_eq!(error("0x").problem, "hex_without_digits");
    assert_eq!(error("0x_1").problem, "hex_without_digits");
    let upper = error("0X1F");
    assert_eq!(
        (upper.problem.as_str(), upper.fix.clone()),
        ("hex_prefix", vec!["0x1F".to_owned()])
    );
    assert_eq!(error("0x1.5").problem, "hex_fraction");
    assert_eq!(error("0x1g").problem, "letters_after_number");
}

#[test]
fn rejects_letters_directly_after_a_number() {
    let error = error("20abc");
    assert_eq!(error.problem, "letters_after_number");
    assert_eq!((error.at.column, error.at.length), (Some(1), Some(5)));
    assert!(error.fix.is_empty());
}

#[test]
fn reads_numbers_with_a_fraction() {
    assert_eq!(
        parse_number("3.5").unwrap(),
        Expr::Num { v: 3.5, span: None }
    );
    assert_eq!(
        parse_number("1_000.25").unwrap(),
        Expr::Num {
            v: 1000.25,
            span: None
        }
    );
    assert!(parse_number("3").is_err());
    assert!(parse_int("3.5").is_err());
}

#[test]
fn rejects_exponents_with_the_full_number_as_fix() {
    for (src, fix) in [
        ("1e5", "100000"),
        ("1.5e3", "1500"),
        ("2E-3", "0.002"),
        ("1e+2", "100"),
    ] {
        let error = error(src);
        assert_eq!(error.problem, "exponent", "{src}");
        assert_eq!(error.fix, [fix], "{src}");
    }
}

#[test]
fn separates_dots_that_do_not_start_a_fraction() {
    assert_eq!(
        kinds("1."),
        [TokenKind::Int, TokenKind::Dot, TokenKind::Eof]
    );
    assert_eq!(
        kinds(".5"),
        [TokenKind::Dot, TokenKind::Int, TokenKind::Eof]
    );
    assert_eq!(
        kinds("10..12"),
        [
            TokenKind::Int,
            TokenKind::DotDot,
            TokenKind::Int,
            TokenKind::Eof
        ]
    );
    assert_eq!(kinds("1.5"), [TokenKind::Number, TokenKind::Eof]);
    assert_eq!(
        kinds("40.."),
        [TokenKind::Int, TokenKind::DotDot, TokenKind::Eof]
    );
}

#[test]
fn computes_sizes_exactly() {
    let cases = [
        ("512b", 512),
        ("10mb", 10_000_000),
        ("1.5kb", 1_500),
        ("1.5gib", 1_610_612_736),
        ("2tib", 2_199_023_255_552),
        ("0.5kib", 512),
        ("1_000kb", 1_000_000),
        ("8191.5tib", 9_006_649_498_927_104),
    ];
    for (src, bytes) in cases {
        assert_eq!(size_bytes(src).unwrap(), bytes, "{src}");
        assert_eq!(
            parse_size(src).unwrap(),
            Expr::Size {
                v: bytes,
                span: None
            }
        );
    }
}

#[test]
fn rejects_sizes_that_are_not_whole_or_too_large() {
    assert_eq!(error("0.5b").problem, "size_not_whole");
    assert_eq!(error("0.0001kb").problem, "size_not_whole");
    assert_eq!(error("8192tib").problem, "number_too_large");
    let error = error("10mb5");
    assert_eq!(error.problem, "size_suffix");
}

#[test]
fn rejects_units_in_upper_case_and_after_hexadecimal() {
    let upper = error("10MB");
    assert_eq!(
        (upper.problem.as_str(), upper.fix.clone()),
        ("uppercase_unit", vec!["10mb".to_owned()])
    );
    assert!(error("10M").fix.is_empty());
    let hex = error("0x10mb");
    assert_eq!(
        (hex.problem.as_str(), hex.fix.clone()),
        ("hex_with_unit", vec!["16mb".to_owned()])
    );
}

#[test]
fn distinguishes_minutes_milliseconds_and_megabytes() {
    assert_eq!(kinds("5m"), [TokenKind::Duration, TokenKind::Eof]);
    assert_eq!(kinds("5ms"), [TokenKind::Duration, TokenKind::Eof]);
    assert_eq!(kinds("5mb"), [TokenKind::Size, TokenKind::Eof]);
    assert_eq!(kinds("5mib"), [TokenKind::Size, TokenKind::Eof]);
    assert_eq!(duration_ms("5m").unwrap(), 300_000);
    assert_eq!(duration_ms("5ms").unwrap(), 5);
    assert_eq!(size_bytes("5mb").unwrap(), 5_000_000);
}

#[test]
fn sums_duration_parts() {
    let cases = [
        ("250ms", 250),
        ("30s", 30_000),
        ("1h30m", 5_400_000),
        ("7d", 604_800_000),
        ("2w", 1_209_600_000),
        ("30m1h", 5_400_000),
        ("1s1s", 2_000),
        ("1_000ms", 1_000),
        ("9007199254740991ms", 9_007_199_254_740_991),
    ];
    for (src, milliseconds) in cases {
        assert_eq!(duration_ms(src).unwrap(), milliseconds, "{src}");
        assert_eq!(
            parse_duration(src).unwrap(),
            Expr::Dur {
                v: milliseconds,
                span: None
            }
        );
    }
}

#[test]
fn rejects_malformed_durations() {
    let cases = [
        ("1m30", "duration_without_unit", vec![]),
        ("1h30mb", "mixed_units", vec![]),
        ("1h30x", "unknown_unit", vec![]),
        ("1.5h", "fractional_duration", vec!["1h30m"]),
        ("0.5s", "fractional_duration", vec!["500ms"]),
        ("5min", "unknown_unit", vec!["5m"]),
        ("5sec", "unknown_unit", vec!["5s"]),
        ("9007199254740992ms", "number_too_large", vec![]),
        ("104249991375d", "number_too_large", vec![]),
    ];
    for (src, problem, fix) in cases {
        let error = error(src);
        assert_eq!(error.problem, problem, "{src}");
        assert_eq!(error.fix, fix, "{src}");
        assert_eq!(error.found, src, "{src}");
    }
}

#[test]
fn decodes_every_text_escape() {
    assert_eq!(
        decode_text(r#""\\ \" \n \r \t \0 \u{41} \u{1F600} \u{10FFFF}""#).unwrap(),
        text("\\ \" \n \r \t \0 A \u{1F600} \u{10FFFF}")
    );
}

#[test]
fn keeps_literal_line_breaks_and_normalizes_crlf() {
    assert_eq!(decode_text("\"a\nb\"").unwrap(), text("a\nb"));
    assert_eq!(decode_text("\"a\r\nb\"").unwrap(), text("a\nb"));
    assert_eq!(decode_text("\"a\rb\"").unwrap(), text("a\rb"));
    assert_eq!(decode_raw("r\"a\r\nb\"").unwrap(), text("a\nb"));
}

#[test]
fn rejects_invalid_escapes_at_their_position() {
    let cases = [
        (r#""ab\x""#, "unknown_escape", 4, 2),
        (r#""\u{}""#, "invalid_unicode_escape", 2, 4),
        (r#""\u{110000}""#, "invalid_unicode_escape", 2, 10),
        (r#""\u{D800}""#, "invalid_unicode_escape", 2, 8),
        (r#""\u{1234567}""#, "invalid_unicode_escape", 2, 11),
        (r#""\u41""#, "invalid_unicode_escape", 2, 2),
    ];
    for (src, problem, column, length) in cases {
        let error = decode_text(src).unwrap_err();
        assert_eq!(error.code, ErrorCode::Escape, "{src}");
        assert_eq!(error.problem, problem, "{src}");
        assert_eq!(
            (error.at.line, error.at.column),
            (Some(1), Some(column)),
            "{src}"
        );
        assert_eq!(error.at.length, Some(length), "{src}");
        let lexed = self::error(src);
        assert_eq!(lexed.at, error.at, "{src}");
    }
}

#[test]
fn offers_raw_text_regex_and_path_fixes_for_escapes() {
    let error = decode_text(r#""\d+""#).unwrap_err();
    assert_eq!(error.fix, [r#"r"\d+""#, r"/\d+/"]);
    let error = decode_text(r#""C:\data\x""#).unwrap_err();
    assert_eq!(error.fix, [r#"r"C:\data\x""#, r#""C:/data/x""#]);
    let error = decode_text(r#""a\"b\d""#).unwrap_err();
    assert_eq!(error.fix, [r#"/a\"b\d/"#]);
}

#[test]
fn rejects_an_unterminated_text_at_its_opening_quote() {
    let error = error("find \"abc\nnext");
    assert_eq!(error.code, ErrorCode::Lexical);
    assert_eq!(error.problem, "unterminated_text");
    assert_eq!(
        (error.at.line, error.at.column, error.at.length),
        (Some(1), Some(6), Some(4))
    );
    assert_eq!(error.found, "\"abc");
}

#[test]
fn reads_raw_texts_with_any_number_of_hashes() {
    assert_eq!(decode_raw(r#"r"a\b""#).unwrap(), text(r"a\b"));
    assert_eq!(decode_raw(r##"r#"a"b"#"##).unwrap(), text(r#"a"b"#));
    assert_eq!(decode_raw(r###"r##"a"#b"##"###).unwrap(), text(r##"a"#b"##));
    assert_eq!(decode_raw("r\"\"").unwrap(), text(""));
    assert_eq!(
        kinds(r###"r##"x"##"###),
        [TokenKind::RawText, TokenKind::Eof]
    );
}

#[test]
fn closes_raw_text_at_the_first_matching_delimiter() {
    let tokens = tokenize(r##"r#"a"# "b""##, LexOptions::default()).unwrap();
    assert_eq!(
        (tokens[0].kind.clone(), tokens[0].range.clone()),
        (TokenKind::RawText, 0..6)
    );
    assert_eq!(tokens[1].kind, TokenKind::Text);
}

#[test]
fn rejects_unterminated_raw_text() {
    let error = error("r#\"abc\"");
    assert_eq!(error.problem, "unterminated_text");
    assert_eq!(error.at.column, Some(1));
}

#[test]
fn reads_regular_expressions_with_flags_in_any_order() {
    let decoded = |src| decode_regex(src).unwrap();
    assert_eq!(
        decoded("/a/is"),
        Expr::Regex {
            v: "a".to_owned(),
            flags: "is".to_owned(),
            span: None
        }
    );
    assert_eq!(decoded("/a/si"), decoded("/a/is"));
    assert_eq!(
        decoded(r"/oldName\(/"),
        Expr::Regex {
            v: r"oldName\(".to_owned(),
            flags: String::new(),
            span: None
        }
    );
    assert_eq!(kinds("/a/xsmi"), [TokenKind::Regex, TokenKind::Eof]);
}

#[test]
fn reads_escaped_slashes_in_pairs() {
    let tokens = tokenize(r"/a\/b/ /c\\/d", LexOptions::default());
    let error = tokens.unwrap_err();
    assert_eq!(error.problem, "unquoted_path");
    let decoded = decode_regex(r"/a\/b/").unwrap();
    assert_eq!(
        decoded,
        Expr::Regex {
            v: "a/b".to_owned(),
            flags: String::new(),
            span: None
        }
    );
    let read = tokenize(r"/c\\/ x", LexOptions::default()).unwrap();
    assert_eq!(read[0].range, 0..5);
}

#[test]
fn rejects_invalid_regular_expressions() {
    let cases = [
        ("/a/ii", "repeated_regex_flag", vec!["/a/i"]),
        ("/a/g", "unknown_regex_flag", vec!["/a/"]),
        ("/a/gi", "unknown_regex_flag", vec!["/a/i"]),
        ("/a/i2", "regex_suffix", vec![]),
        ("//", "empty_regex", vec![]),
        ("/a", "unterminated_regex", vec!["\"/a\""]),
        ("/a b", "unterminated_regex", vec!["\"/a\""]),
        ("/srv/app", "unquoted_path", vec!["\"/srv/app\""]),
        ("/srv/app/src", "unquoted_path", vec!["\"/srv/app/src\""]),
    ];
    for (src, problem, fix) in cases {
        let error = error(src);
        assert_eq!(error.code, ErrorCode::Lexical, "{src}");
        assert_eq!(error.problem, problem, "{src}");
        assert_eq!(error.fix, fix, "{src}");
    }
    assert!(error("/a/g").message.contains("every match"));
}

#[test]
fn keeps_a_regular_expression_on_one_line() {
    let error = error("/a\nb/");
    assert_eq!(error.problem, "unterminated_regex");
}

#[test]
fn rejects_fragments_that_are_not_one_literal() {
    for (result, src) in [
        (decode_text("abc").map(|_| ()), "abc"),
        (decode_text("\"a\" ").map(|_| ()), "\"a\" "),
        (decode_raw("\"a\"").map(|_| ()), "\"a\""),
        (decode_regex("/a/ ").map(|_| ()), "/a/ "),
        (parse_int("12 ").map(|_| ()), "12 "),
        (parse_size("12").map(|_| ()), "12"),
        (parse_duration("12mb").map(|_| ()), "12mb"),
        (parse_int("x").map(|_| ()), "x"),
    ] {
        let error = result.unwrap_err();
        assert_eq!(error.problem, "not_a_literal", "{src}");
    }
}

#[test]
fn converts_into_the_contract_error() {
    let lexical = error("a $");
    let converted = Error::from(lexical.clone());
    assert_eq!(converted.code, ErrorCode::Lexical);
    assert_eq!(converted.stage(), Some(Stage::Parse));
    assert_eq!(converted.at, Some(lexical.at.with_stage(Stage::Parse)));
    assert_eq!(converted.found.as_deref(), Some("$"));
    assert_eq!(converted.problem.as_deref(), Some("variable"));
    assert_eq!(converted.help.as_deref(), Some("syntax"));
    let json = serde_json::to_value(&converted).unwrap();
    assert_eq!(json["at"]["stage"], "0");
    assert_eq!(json["code"], "E_LEXICAL");
    assert!(lexical.to_string().starts_with("E_LEXICAL: "));
    let escape = Error::from(decode_text(r#""\q""#).unwrap_err());
    assert_eq!(escape.fix.len(), 2);
    assert_eq!(escape.data().code, ErrorCode::Escape);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
