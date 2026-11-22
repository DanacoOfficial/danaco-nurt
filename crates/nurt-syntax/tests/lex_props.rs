// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Properties of the lexer: lossless coverage, monotonic spans, agreement of
//! the tolerant and the strict mode, inverse of the canonical literal forms
//! and absence of panics.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::ast::Expr;
use nurt_syntax::lex::{
    LexContext, LexOptions, Lexer, MAX_SAFE_INTEGER, Token, TokenKind, decode_raw, decode_text,
    duration_ms, invalid_escapes, size_bytes, tokenize,
};
use proptest::prelude::*;

const MAX_SAFE: u64 = MAX_SAFE_INTEGER.unsigned_abs();

/// Fragments of programs, valid and invalid, joined by the program
/// generator.
const FRAGMENTS: &[&str] = &[
    "find",
    "filter",
    "let",
    "not",
    "and",
    "in",
    "x",
    "@recent",
    ".path",
    "git.status",
    "\"/srv/app\"",
    "\"a\\tb\\u{1F600}\"",
    "\"\\d\"",
    "\"multi\nline\"",
    "r#\"q\"\"#",
    "r\"\\\"",
    "r",
    "1r#\"q\"#",
    "0x1r\"q\"",
    "/old\\(/i",
    "/a\\/b/",
    "//",
    "1_000",
    "0x1F",
    "1.5",
    "10mb",
    "1.5gib",
    "1h30m",
    "250ms",
    "5min",
    "10MB",
    "1e5",
    "|>",
    "==",
    "!=",
    "<=",
    ">",
    "~",
    "!~",
    "+",
    "-",
    "*",
    "/",
    "=",
    ":",
    ",",
    ".",
    "..",
    "...",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    "'q'",
    "$",
    "&&",
    "| >",
    "->",
    "--force",
    "\u{201E}a\u{201D}",
    "\u{A0}",
    "\r",
    "\u{FEFF}",
    "é",
    "Find",
    "C:\\x",
    "~/a",
    "#",
    "#!",
];

const SEPARATORS: &[&str] = &[" ", "", "\n", "\t", "\r\n", " # note\n", "\n\n|> "];

fn program() -> impl Strategy<Value = String> {
    prop::collection::vec(
        (
            prop::sample::select(FRAGMENTS),
            prop::sample::select(SEPARATORS),
        ),
        0..30,
    )
    .prop_map(|parts| {
        parts
            .into_iter()
            .flat_map(|(fragment, separator)| [fragment, separator])
            .collect()
    })
}

fn any_text() -> impl Strategy<Value = String> {
    prop::collection::vec(any::<char>(), 0..60)
        .prop_map(|characters| characters.into_iter().collect())
}

/// Line and column of byte `offset`, skipping a byte order mark at the start.
fn position(text: &str, offset: usize) -> (u32, u32) {
    let start = if text.starts_with('\u{FEFF}') { 3 } else { 0 };
    let before = text.get(start.min(offset)..offset).unwrap();
    let line = 1 + before.matches('\n').count();
    let last_line = before.rsplit('\n').next().unwrap_or_default();
    (
        u32::try_from(line).unwrap(),
        1 + u32::try_from(last_line.chars().count()).unwrap(),
    )
}

/// Whether `gap` consists of whitespace, line breaks and comments only; at
/// the start of the input it may also hold a byte order mark and a `#!`
/// line.
fn is_trivia(gap: &str, at_start: bool) -> bool {
    let mut rest = gap;
    if at_start {
        rest = rest.strip_prefix('\u{FEFF}').unwrap_or(rest);
        if rest.starts_with("#!") {
            rest = rest.get(comment_length(rest)..).unwrap();
        }
    }
    while let Some(next) = rest.chars().next() {
        let length = match next {
            ' ' | '\t' | '\n' => 1,
            '\r' if rest.starts_with("\r\n") => 2,
            '#' => comment_length(rest),
            _ => return false,
        };
        rest = rest.get(length..).unwrap();
    }
    true
}

fn comment_length(rest: &str) -> usize {
    match rest.find('\n') {
        Some(newline) if rest[..newline].ends_with('\r') => newline - 1,
        Some(newline) => newline,
        None => rest.len(),
    }
}

fn check_coverage(text: &str, tokens: &[Token]) -> Result<(), TestCaseError> {
    let mut covered = 0;
    let mut previous_end = (1, 1);
    for token in tokens {
        prop_assert!(token.range.start >= covered, "overlap at {:?}", token.range);
        let gap = &text[covered..token.range.start];
        prop_assert!(
            is_trivia(gap, covered == 0),
            "gap {gap:?} before {:?}",
            token.kind
        );
        let start = (token.span.line, token.span.column);
        let end = (token.span.end_line, token.span.end_column);
        if token.kind != TokenKind::Nl {
            prop_assert_eq!(start, position(text, token.range.start));
            prop_assert_eq!(end, position(text, token.range.end));
        }
        prop_assert!(
            start >= previous_end && end >= start,
            "span {:?}",
            token.span
        );
        previous_end = end;
        covered = token.range.end;
    }
    prop_assert_eq!(
        tokens.last().map(|token| token.range.clone()),
        Some(text.len()..text.len())
    );
    Ok(())
}

fn tolerant() -> LexOptions {
    LexOptions {
        tolerant: true,
        keep_comments: true,
        ..LexOptions::default()
    }
}

/// Whether the tolerant tokens of `text` mark no error: no foreign token and
/// no text with an invalid escape.
fn is_free_of_errors(text: &str, tokens: &[Token]) -> bool {
    tokens.iter().all(|token| match token.kind {
        TokenKind::Foreign(_) => false,
        TokenKind::Text => invalid_escapes(&text[token.range.clone()]).is_empty(),
        _ => true,
    })
}

fn check_strict_accepts(text: &str, variant_b: bool) -> Result<(), TestCaseError> {
    let tolerant_options = LexOptions {
        arguments_by_whitespace: !variant_b,
        ..tolerant()
    };
    let tolerant_tokens = tokenize(text, tolerant_options).unwrap();
    if is_free_of_errors(text, &tolerant_tokens) {
        let strict = tokenize(
            text,
            LexOptions {
                tolerant: false,
                ..tolerant_options
            },
        );
        prop_assert_eq!(strict, Ok(tolerant_tokens), "{:?}", text);
    }
    Ok(())
}

/// Canonical text literal of `value`.
fn print_text(value: &str) -> String {
    let controls = value
        .chars()
        .any(|character| character.is_control() && character != '\t' && character != '\n');
    if value.contains(['"', '\\']) && !controls {
        let mut hashes = String::new();
        while value.contains(&format!("\"{hashes}")) {
            hashes.push('#');
        }
        return format!("r{hashes}\"{value}\"{hashes}");
    }
    let mut printed = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => printed.push_str("\\\\"),
            '"' => printed.push_str("\\\""),
            '\n' => printed.push_str("\\n"),
            '\r' => printed.push_str("\\r"),
            '\t' => printed.push_str("\\t"),
            '\0' => printed.push_str("\\0"),
            other if other.is_control() => {
                printed.push_str(&format!("\\u{{{:x}}}", u32::from(other)))
            }
            other => printed.push(other),
        }
    }
    printed.push('"');
    printed
}

/// Canonical duration of `milliseconds`.
fn print_duration(milliseconds: u64) -> String {
    if milliseconds == 0 {
        return "0s".to_owned();
    }
    let mut rest = milliseconds;
    let mut printed = String::new();
    for (unit, length) in [
        ("d", 86_400_000),
        ("h", 3_600_000),
        ("m", 60_000),
        ("s", 1_000),
        ("ms", 1),
    ] {
        if rest >= length {
            printed.push_str(&format!("{}{unit}", rest / length));
            rest %= length;
        }
    }
    printed
}

/// Every printable size form with at most three fraction digits whose value
/// is exactly `bytes`.
fn size_forms(bytes: u64) -> Vec<String> {
    let units: [(&str, u64); 9] = [
        ("b", 1),
        ("kb", 1_000),
        ("mb", 1_000_000),
        ("gb", 1_000_000_000),
        ("tb", 1_000_000_000_000),
        ("kib", 1 << 10),
        ("mib", 1 << 20),
        ("gib", 1 << 30),
        ("tib", 1 << 40),
    ];
    let mut forms = Vec::new();
    for (unit, multiplier) in units {
        let thousandths = u128::from(bytes) * 1_000;
        if thousandths % u128::from(multiplier) != 0 {
            continue;
        }
        let scaled = thousandths / u128::from(multiplier);
        let (whole, fraction) = (scaled / 1_000, scaled % 1_000);
        if fraction == 0 {
            forms.push(format!("{whole}{unit}"));
        } else {
            let digits = format!("{fraction:03}");
            forms.push(format!("{whole}.{}{unit}", digits.trim_end_matches('0')));
        }
    }
    forms
}

fn byte_counts() -> impl Strategy<Value = u64> {
    prop_oneof![
        0..=MAX_SAFE,
        (0_u64..=8_388_608).prop_map(|count| count << 30),
        (0_u64..=9_007_199).prop_map(|count| count * 1_000_000_000),
        (0_u64..=1_000_000).prop_map(|count| count * 512),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20_000))]

    #[test]
    fn covers_generated_programs_losslessly_and_agrees_with_strict_mode(text in program()) {
        let tolerant_tokens = tokenize(&text, tolerant()).unwrap();
        check_coverage(&text, &tolerant_tokens)?;
        if let Ok(strict) = tokenize(&text, LexOptions { keep_comments: true, ..LexOptions::default() }) {
            prop_assert_eq!(strict, tolerant_tokens);
        }
    }

    #[test]
    fn covers_arbitrary_text_losslessly(text in any_text()) {
        let tokens = tokenize(&text, tolerant()).unwrap();
        check_coverage(&text, &tokens)?;
        if let Ok(strict) = tokenize(&text, LexOptions::default()) {
            prop_assert_eq!(strict, tokens);
        }
    }

    #[test]
    fn strict_mode_accepts_generated_programs_that_tolerant_mode_reads_without_errors(text in program(), variant_b in any::<bool>()) {
        check_strict_accepts(&text, variant_b)?;
    }

    #[test]
    fn strict_mode_accepts_arbitrary_text_that_tolerant_mode_reads_without_errors(text in any_text(), variant_b in any::<bool>()) {
        check_strict_accepts(&text, variant_b)?;
    }

    #[test]
    fn terminates_without_panic_in_both_modes(text in any_text(), tolerant_mode in any::<bool>(), variant_b in any::<bool>()) {
        let options = LexOptions {
            arguments_by_whitespace: !variant_b,
            tolerant: tolerant_mode,
            keep_comments: true,
        };
        let mut lexer = Lexer::new(&text, options);
        let mut calls = 0;
        loop {
            let context = if calls % 2 == 0 { LexContext::Value } else { LexContext::Operator };
            let result = lexer.next(context);
            if matches!(&result, Ok(token) if token.kind == TokenKind::Eof) {
                break;
            }
            calls += 1;
            prop_assert!(calls <= 2 * text.len() + 2, "no progress on {text:?}");
        }
    }

    #[test]
    fn decodes_the_canonical_form_of_any_text(value in any_text()) {
        let printed = print_text(&value);
        let decoded = if printed.starts_with('r') { decode_raw(&printed) } else { decode_text(&printed) };
        prop_assert_eq!(decoded.unwrap(), Expr::Text { v: value, span: None });
        let tokens = tokenize(&printed, LexOptions::default()).unwrap();
        prop_assert_eq!(tokens.len(), 2);
        prop_assert_eq!(tokens[0].range.clone(), 0..printed.len());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn reads_back_every_exact_size_form(bytes in byte_counts()) {
        let forms = size_forms(bytes);
        prop_assert!(!forms.is_empty());
        for form in forms {
            prop_assert_eq!(size_bytes(&form).unwrap(), bytes, "{}", form);
        }
    }

    #[test]
    fn reads_back_the_canonical_duration(milliseconds in prop_oneof![0..=MAX_SAFE, 0_u64..10_000_000]) {
        let printed = print_duration(milliseconds);
        prop_assert_eq!(duration_ms(&printed).unwrap(), milliseconds, "{}", printed);
    }
}
