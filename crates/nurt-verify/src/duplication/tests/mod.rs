// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::duplication::tests
// Responsibility:   Detection of duplicated blocks on tokens produced by a small lexer that.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Detection of duplicated blocks on tokens produced by a small lexer that
//! stands in for the code service.

use nurt_contracts::services::{ByteRange, SpanClass, SyntaxToken};

use super::{DUPLICATION_MIN_LINES, SourceTokens, detect, normalize};

const KEYWORDS: [&str; 6] = ["let", "return", "fn", "import", "use", "if"];

/// Ten lines of code that share no block with the generated functions.
const DISTINCT_CODE: &str = "if a { return 1; }\nlet q = \"s\";\nfn g(b, c) { return b; }\n\
                             let r = h(1)(2);\nif b { if c { return 2; } }\nlet s = t + u + v + w;\n\
                             return x;\nfn k() { }\nlet m = n(o, p, q, r, s);\nif z { return 0; }\n";

fn scan_while(bytes: &[u8], mut index: usize, keep: impl Fn(u8) -> bool) -> usize {
    while index < bytes.len() && keep(bytes[index]) {
        index += 1;
    }
    index
}

fn lex(text: &str) -> Vec<SyntaxToken> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let start = index;
        let (kind, class) = if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        } else if bytes[index..].starts_with(b"//") {
            index = scan_while(bytes, index, |b| b != b'\n');
            ("comment".to_owned(), SpanClass::Comment)
        } else if byte == b'"' {
            index = scan_while(bytes, index + 1, |b| b != b'"') + 1;
            ("string_literal".to_owned(), SpanClass::String)
        } else if byte.is_ascii_digit() {
            index = scan_while(bytes, index, |b| b.is_ascii_digit());
            ("number".to_owned(), SpanClass::Code)
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            index = scan_while(bytes, index, |b| b.is_ascii_alphanumeric() || b == b'_');
            let word = &text[start..index];
            if KEYWORDS.contains(&word) {
                (word.to_owned(), SpanClass::Code)
            } else {
                ("identifier".to_owned(), SpanClass::Code)
            }
        } else {
            index += 1;
            ("punct".to_owned(), SpanClass::Code)
        };
        tokens.push(SyntaxToken {
            bytes: ByteRange::new(start as u64, index as u64),
            kind,
            class,
        });
    }
    tokens
}

fn source(path: &str, text: &str) -> SourceTokens {
    normalize(path, text.as_bytes(), &lex(text))
}

fn body(names: [&str; 3], lines: usize) -> String {
    let mut text = String::new();
    for index in 0..lines {
        text.push_str(&format!(
            "    let {} = {}({}, {}){};\n",
            names[0],
            names[1],
            names[2],
            index,
            " + 1".repeat(index)
        ));
    }
    text
}

fn function(name: &str, names: [&str; 3], lines: usize) -> String {
    format!("fn {name}(a) {{\n{}    return 1;\n}}\n", body(names, lines))
}

#[test]
fn reports_later_copy_with_renamed_identifiers() {
    let first = source("src/a.rs", &function("one", ["x", "f", "y"], 10));
    let second = source("src/b.rs", &function("two", ["total", "g", "item"], 10));
    let report = detect(&[second, first], DUPLICATION_MIN_LINES);
    assert_eq!(report.findings.len(), 1);
    let finding = &report.findings[0];
    assert_eq!(finding.path.as_str(), "src/b.rs");
    assert_eq!(finding.line, Some(1));
    let related = finding.related.as_ref().unwrap();
    assert_eq!(related[0].path.as_str(), "src/a.rs");
    assert!(finding.value.unwrap() >= DUPLICATION_MIN_LINES as f64);
}

#[test]
fn ignores_block_shorter_than_minimum() {
    let first = source("a.rs", &function("one", ["x", "f", "y"], 3));
    let second = source("b.rs", &function("two", ["x", "f", "y"], 3));
    let report = detect(&[first, second], DUPLICATION_MIN_LINES);
    assert!(report.findings.is_empty());
    assert_eq!(report.duplicated_lines, 0);
    assert_eq!(report.percent(), 0.0);
}

#[test]
fn reports_share_of_duplicated_lines() {
    let first = source("a.rs", &function("one", ["x", "f", "y"], 10));
    let second = source("b.rs", &function("two", ["x", "f", "y"], 10));
    let report = detect(&[first, second], DUPLICATION_MIN_LINES);
    assert_eq!(report.code_lines, 26);
    assert_eq!(report.duplicated_lines, 26);
    assert_eq!(report.percent(), 100.0);
}

#[test]
fn share_counts_distinct_code_as_not_duplicated() {
    let first = source("a.rs", &function("one", ["x", "f", "y"], 10));
    let second = source("b.rs", &function("two", ["x", "f", "y"], 10));
    let distinct = source("c.rs", DISTINCT_CODE);
    let report = detect(&[first, second, distinct], DUPLICATION_MIN_LINES);
    assert_eq!(report.code_lines, 36);
    assert_eq!(report.duplicated_lines, 26);
    assert_eq!(report.percent(), 72.2);
}

#[test]
fn ignores_comments_and_white_space() {
    let plain = function("one", ["x", "f", "y"], 10);
    let noisy = plain
        .replace('\n', "  // note\n\n")
        .replace("    let", "\t\tlet");
    let report = detect(
        &[source("a.rs", &plain), source("b.rs", &noisy)],
        DUPLICATION_MIN_LINES,
    );
    assert_eq!(report.findings.len(), 1);
}

#[test]
fn result_does_not_depend_on_input_order() {
    let files = [
        source("c.rs", &function("three", ["p", "h", "q"], 12)),
        source("a.rs", &function("one", ["x", "f", "y"], 12)),
        source("b.rs", &function("two", ["m", "g", "n"], 12)),
    ];
    let forward = detect(&files, DUPLICATION_MIN_LINES);
    let reversed: Vec<SourceTokens> = files.iter().rev().cloned().collect();
    assert_eq!(forward, detect(&reversed, DUPLICATION_MIN_LINES));
    assert_eq!(forward.findings.len(), 2);
    assert_eq!(forward.findings[0].path.as_str(), "b.rs");
    assert_eq!(forward.findings[1].path.as_str(), "c.rs");
}

#[test]
fn skips_dependency_declarations() {
    let imports: String = (0..12)
        .map(|index| format!("import module{index};\n"))
        .collect();
    let report = detect(
        &[source("a.rs", &imports), source("b.rs", &imports)],
        DUPLICATION_MIN_LINES,
    );
    assert!(report.findings.is_empty());
    assert_eq!(report.code_lines, 0);
    assert_eq!(report.percent(), 0.0);
}

#[test]
fn reports_repeated_function_in_one_file() {
    let repeated = format!(
        "{}{}",
        function("one", ["x", "f", "y"], 10),
        function("two", ["x", "f", "y"], 10)
    );
    let report = detect(&[source("a.rs", &repeated)], DUPLICATION_MIN_LINES);
    assert_eq!(report.findings.len(), 1);
    let related = report.findings[0].related.as_ref().unwrap();
    assert_eq!(related[0].path.as_str(), "a.rs");
    assert!(related[0].line < report.findings[0].line.unwrap());
}

#[test]
fn distinct_code_has_no_findings() {
    let first = source("a.rs", &function("one", ["x", "f", "y"], 10));
    let report = detect(
        &[first, source("b.rs", DISTINCT_CODE)],
        DUPLICATION_MIN_LINES,
    );
    assert!(report.findings.is_empty());
    assert_eq!(report.duplicated_lines, 0);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
