// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Timing of the lexer; run with `--release -- --ignored perf_lex`.

// A failed unwrap, expectation or panic is how a test reports a failure;
// the timing test prints its measurement.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]

use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

use nurt_syntax::lex::{LexContext, LexOptions, Lexer, TokenKind};

/// Size of a typical program.
const TYPICAL_BYTES: usize = 2_048;
/// Size of a large input.
const LARGE_BYTES: usize = 1 << 20;
const TYPICAL_LIMIT: Duration = Duration::from_micros(20);
const LARGE_LIMIT: Duration = Duration::from_millis(50);
const TYPICAL_RUNS: u32 = 20_000;
const LARGE_RUNS: u32 = 5;

/// Golden task programs joined until the text reaches `bytes`.
fn corpus(bytes: usize) -> String {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/golden/ast");
    let mut paths: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "nurt")
        })
        .collect();
    paths.sort();
    let programs: Vec<String> = paths
        .iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    let mut text = String::new();
    for program in programs.iter().cycle() {
        if text.len() + program.len() > bytes {
            break;
        }
        text.push_str(program);
    }
    text
}

/// Lexes `text` to the end, choosing contexts like a parser, and returns the
/// number of tokens.
fn lex(text: &str) -> usize {
    let mut lexer = Lexer::new(text, LexOptions::default());
    let mut context = LexContext::Value;
    let mut count = 0;
    loop {
        let token = lexer.next(context).unwrap();
        count += 1;
        context = match token.kind {
            TokenKind::Eof => return count,
            TokenKind::Ident
            | TokenKind::Text
            | TokenKind::RawText
            | TokenKind::Regex
            | TokenKind::Int
            | TokenKind::Number
            | TokenKind::Size
            | TokenKind::Duration
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Null
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::RBrace => LexContext::Operator,
            _ => LexContext::Value,
        };
    }
}

#[test]
#[ignore = "timing; run in release mode"]
fn perf_lex_meets_time_limits() {
    let typical = corpus(TYPICAL_BYTES);
    let started = Instant::now();
    for _ in 0..TYPICAL_RUNS {
        black_box(lex(black_box(&typical)));
    }
    let per_typical = started.elapsed() / TYPICAL_RUNS;

    let large = corpus(LARGE_BYTES);
    let per_large = (0..LARGE_RUNS)
        .map(|_| {
            let started = Instant::now();
            black_box(lex(black_box(&large)));
            started.elapsed()
        })
        .min()
        .unwrap();

    println!(
        "typical program: {} bytes, {:?}; large input: {} bytes, {:?}",
        typical.len(),
        per_typical,
        large.len(),
        per_large
    );
    assert!(
        per_typical < TYPICAL_LIMIT,
        "typical program took {per_typical:?}"
    );
    assert!(per_large < LARGE_LIMIT, "large input took {per_large:?}");
}
