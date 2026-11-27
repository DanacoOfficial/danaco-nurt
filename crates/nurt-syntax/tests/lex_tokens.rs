// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/lex_tokens.rs
// Responsibility:   Tokens of the lexical table, line breaks, comments, positions and.
// Dependencies:     TokenKind, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tokens of the lexical table, line breaks, comments, positions and
//! checkpoints.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::ast::Span;
use nurt_contracts::error::ErrorCode;
use nurt_syntax::lex::{
    Glued, LexContext, LexError, LexOptions, Lexer, MAX_IDENTIFIER_LEN, Token, TokenKind, tokenize,
};

fn tokens(text: &str) -> Vec<Token> {
    tokenize(text, LexOptions::default()).unwrap()
}

fn kinds(text: &str) -> Vec<TokenKind> {
    tokens(text).into_iter().map(|token| token.kind).collect()
}

fn error(text: &str) -> LexError {
    tokenize(text, LexOptions::default()).unwrap_err()
}

fn span(line: u32, column: u32, end_line: u32, end_column: u32) -> Span {
    Span {
        line,
        column,
        end_line,
        end_column,
    }
}

#[test]
fn reads_every_operator_of_the_table_as_one_token() {
    let table = [
        ("|>", TokenKind::PipeGt),
        ("==", TokenKind::EqEq),
        ("!=", TokenKind::NotEq),
        ("<", TokenKind::Lt),
        ("<=", TokenKind::Le),
        (">", TokenKind::Gt),
        (">=", TokenKind::Ge),
        ("~", TokenKind::Tilde),
        ("!~", TokenKind::NotTilde),
        ("+", TokenKind::Plus),
        ("-", TokenKind::Minus),
        ("*", TokenKind::Star),
        ("=", TokenKind::Eq),
        (":", TokenKind::Colon),
        (",", TokenKind::Comma),
        (".", TokenKind::Dot),
        ("..", TokenKind::DotDot),
        ("...", TokenKind::Ellipsis),
        ("@", TokenKind::At),
        ("(", TokenKind::LParen),
        (")", TokenKind::RParen),
        ("[", TokenKind::LBracket),
        ("]", TokenKind::RBracket),
        ("{", TokenKind::LBrace),
        ("}", TokenKind::RBrace),
    ];
    for (text, kind) in table {
        let read = tokens(text);
        assert_eq!(read.len(), 2, "{text}");
        assert_eq!(read[0].kind, kind, "{text}");
        assert_eq!(read[0].range, 0..text.len(), "{text}");
        assert_eq!(read[1].kind, TokenKind::Eof);
    }
}

#[test]
fn reads_slash_as_division_in_operator_context() {
    let mut lexer = Lexer::new("a/b", LexOptions::default());
    assert_eq!(
        lexer.next(LexContext::Value).unwrap().kind,
        TokenKind::Ident
    );
    let slash = lexer.next(LexContext::Operator).unwrap();
    assert_eq!((slash.kind, slash.range), (TokenKind::Slash, 1..2));
}

#[test]
fn prefers_the_longest_dot_operator() {
    assert_eq!(
        kinds("....."),
        [TokenKind::Ellipsis, TokenKind::DotDot, TokenKind::Eof]
    );
    assert_eq!(
        kinds("...@x.path"),
        [
            TokenKind::Ellipsis,
            TokenKind::At,
            TokenKind::Ident,
            TokenKind::Dot,
            TokenKind::Ident,
            TokenKind::Eof
        ]
    );
}

#[test]
fn gives_keywords_their_own_kinds() {
    assert_eq!(
        kinds("let true false null and or not in"),
        [
            TokenKind::Let,
            TokenKind::True,
            TokenKind::False,
            TokenKind::Null,
            TokenKind::And,
            TokenKind::Or,
            TokenKind::Not,
            TokenKind::In,
            TokenKind::Eof
        ]
    );
    assert_eq!(
        kinds("letter inside")[..2],
        [TokenKind::Ident, TokenKind::Ident]
    );
}

#[test]
fn splits_command_names_at_dots() {
    assert_eq!(
        kinds("git.status"),
        [
            TokenKind::Ident,
            TokenKind::Dot,
            TokenKind::Ident,
            TokenKind::Eof
        ]
    );
}

#[test]
fn accepts_identifiers_up_to_the_length_limit() {
    let longest = "a".repeat(MAX_IDENTIFIER_LEN);
    assert_eq!(kinds(&longest), [TokenKind::Ident, TokenKind::Eof]);
    let too_long = "a".repeat(MAX_IDENTIFIER_LEN + 1);
    let error = error(&too_long);
    assert_eq!(error.code, ErrorCode::Lexical);
    assert_eq!(error.problem, "name_too_long");
    assert_eq!(error.at.length, Some(65));
}

#[test]
fn reads_a_hyphenated_word_as_subtraction() {
    assert_eq!(
        kinds("case-sensitive"),
        [
            TokenKind::Ident,
            TokenKind::Minus,
            TokenKind::Ident,
            TokenKind::Eof
        ]
    );
}

#[test]
fn rejects_upper_case_names_with_a_lower_case_fix() {
    let error = error("Find \"/a\"");
    assert_eq!(error.code, ErrorCode::Lexical);
    assert_eq!(error.problem, "name_case");
    assert_eq!(error.fix, ["find"]);
    assert_eq!(self::error("caseSensitive: true").fix, ["case_sensitive"]);
}

#[test]
fn rejects_names_outside_ascii() {
    let error = error("zażółć");
    assert_eq!(error.problem, "non_ascii_name");
    assert_eq!(error.found, "zażółć");
    assert_eq!(error.at.length, Some(6));
}

#[test]
fn marks_glued_colon_paren_and_equals() {
    let read = tokens("in: ago(1h) name=1 a==b {in: 1}");
    assert_eq!(
        (read[0].kind.clone(), read[0].glued),
        (TokenKind::In, Glued::Colon)
    );
    assert_eq!(read[2].glued, Glued::LParen);
    assert_eq!(read[6].glued, Glued::Equals);
    assert_eq!(read[9].glued, Glued::None);
    assert_eq!(
        (read[13].kind.clone(), read[13].glued),
        (TokenKind::In, Glued::Colon)
    );
}

#[test]
fn reports_space_before_and_after() {
    let read = tokens("size - 100 size-100");
    assert!(read[1].space_before && read[1].space_after);
    assert!(!read[4].space_before && !read[4].space_after);
    assert!(
        read[0].space_before,
        "start of the input counts as a line start"
    );
}

#[test]
fn ends_a_statement_with_one_line_break_token() {
    let read = tokens("a\n\n\nb");
    let kinds: Vec<_> = read.iter().map(|token| token.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            TokenKind::Ident,
            TokenKind::Nl,
            TokenKind::Ident,
            TokenKind::Eof
        ]
    );
    assert_eq!(read[1].range, 1..2);
    assert_eq!(read[1].span, span(1, 2, 1, 3));
    assert_eq!(read[2].span, span(4, 1, 4, 2));
}

#[test]
fn skips_line_breaks_inside_brackets() {
    for text in ["[1,\n2]", "(a\nb)", "{a: 1\n}", "[\n(\n{\n}\n)\n]"] {
        assert!(!kinds(text).contains(&TokenKind::Nl), "{text:?}");
    }
}

#[test]
fn skips_line_breaks_after_pipe_equals_and_comma() {
    for text in ["a |>\nb", "let x =\nfind", "f(a,\nb)", "edit path=\n\"x\""] {
        assert!(!kinds(text).contains(&TokenKind::Nl), "{text:?}");
    }
}

#[test]
fn skips_a_line_break_before_a_continuing_pipe() {
    assert!(!kinds("a\n|> b").contains(&TokenKind::Nl));
    assert!(!kinds("a\n\n  # note\n\n  |> b").contains(&TokenKind::Nl));
    assert!(!kinds("a # first\n# second\r\n|> b").contains(&TokenKind::Nl));
}

#[test]
fn keeps_a_line_break_after_colon_and_logical_operators() {
    assert_eq!(
        kinds("glob:\n\"x\""),
        [
            TokenKind::Ident,
            TokenKind::Colon,
            TokenKind::Nl,
            TokenKind::Text,
            TokenKind::Eof
        ]
    );
    assert!(kinds("a and\nb").contains(&TokenKind::Nl));
    assert!(kinds("a or\nb").contains(&TokenKind::Nl));
}

#[test]
fn produces_no_line_break_at_the_start_and_one_at_the_end() {
    assert_eq!(kinds("\n\n# c\na"), [TokenKind::Ident, TokenKind::Eof]);
    assert_eq!(
        kinds("a\n"),
        [TokenKind::Ident, TokenKind::Nl, TokenKind::Eof]
    );
    assert_eq!(kinds(""), [TokenKind::Eof]);
}

#[test]
fn treats_crlf_as_one_line_break() {
    let read = tokens("a\r\nb");
    assert_eq!(read[1].kind, TokenKind::Nl);
    assert_eq!(read[1].range, 1..3);
    assert_eq!(read[1].span, span(1, 2, 1, 4));
    assert_eq!(read[2].span, span(2, 1, 2, 2));
}

#[test]
fn rejects_a_lone_carriage_return() {
    let error = error("a\rb");
    assert_eq!(error.problem, "carriage_return");
    assert_eq!((error.at.line, error.at.column), (Some(1), Some(2)));
}

#[test]
fn rejects_unicode_whitespace_outside_literals() {
    for (text, column) in [("a\u{A0}b", 2), ("a\u{200B}b", 2), ("a \u{FEFF}b", 3)] {
        let error = error(text);
        assert_eq!(error.problem, "unicode_whitespace", "{text:?}");
        assert_eq!(error.at.column, Some(column), "{text:?}");
    }
    assert_eq!(kinds("\"a\u{A0}b\""), [TokenKind::Text, TokenKind::Eof]);
}

#[test]
fn skips_a_byte_order_mark_without_counting_it() {
    let read = tokens("\u{FEFF}find");
    assert_eq!(read[0].range, 3..7);
    assert_eq!(read[0].span, span(1, 1, 1, 5));
}

#[test]
fn ignores_a_shebang_only_on_the_first_line() {
    let options = LexOptions {
        keep_comments: true,
        ..LexOptions::default()
    };
    let mut lexer = Lexer::new("\u{FEFF}#!nurt run\nfind\n#!x\n", options);
    let first = lexer.next(LexContext::Value).unwrap();
    assert_eq!(
        (first.kind, first.span),
        (TokenKind::Ident, span(2, 1, 2, 5))
    );
    assert_eq!(
        lexer.next(LexContext::Operator).unwrap().kind,
        TokenKind::Nl
    );
    assert_eq!(lexer.next(LexContext::Value).unwrap().kind, TokenKind::Eof);
    assert_eq!(lexer.comments().len(), 1);
    assert_eq!(lexer.comments()[0].text, "!x");
}

#[test]
fn records_comments_in_source_order() {
    let options = LexOptions {
        keep_comments: true,
        ..LexOptions::default()
    };
    let mut lexer = Lexer::new("# head\r\nfind # tail\n", options);
    while lexer.next(LexContext::Value).unwrap().kind != TokenKind::Eof {}
    let comments = lexer.comments();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0].text, " head");
    assert_eq!(comments[0].span, span(1, 1, 1, 7));
    assert_eq!(comments[1].text, " tail");
    assert_eq!(comments[1].span, span(2, 6, 2, 12));
    let serialized = serde_json::to_value(&comments[1]).unwrap();
    assert_eq!(
        serialized,
        serde_json::json!({"span": [2, 6, 2, 12], "text": " tail"})
    );
}

#[test]
fn keeps_no_comments_by_default() {
    let mut lexer = Lexer::new("a # c", LexOptions::default());
    while lexer.next(LexContext::Value).unwrap().kind != TokenKind::Eof {}
    assert!(lexer.comments().is_empty());
}

#[test]
fn keeps_a_hash_inside_literals() {
    assert_eq!(kinds("\"# x\""), [TokenKind::Text, TokenKind::Eof]);
    assert_eq!(kinds("r#\"#\"#"), [TokenKind::RawText, TokenKind::Eof]);
    assert_eq!(kinds("/#/"), [TokenKind::Regex, TokenKind::Eof]);
}

#[test]
fn reads_raw_text_after_brackets_and_separators_in_every_variant() {
    use TokenKind::{Colon, Comma, Eof, Ident, LParen, Nl, PipeGt, RParen, RawText, Text};
    let cases: [(&str, &[TokenKind]); 5] = [
        (
            "|> insert(r#\"import \"fmt\"\"#, position: after)\n",
            &[
                PipeGt, Ident, LParen, RawText, Comma, Ident, Colon, Ident, RParen, Nl, Eof,
            ],
        ),
        (
            "write(\"/a\", r#\"x\ny\n\"#, mode: create)",
            &[
                Ident, LParen, Text, Comma, RawText, Comma, Ident, Colon, Ident, RParen, Eof,
            ],
        ),
        (
            "f(r\"\\d+\",r\"x\")",
            &[Ident, LParen, RawText, Comma, RawText, RParen, Eof],
        ),
        (
            "f(pattern:r##\"a\"#b\"##)",
            &[Ident, LParen, Ident, Colon, RawText, RParen, Eof],
        ),
        ("insert r#\"x\"# r\"y\"", &[Ident, RawText, RawText, Eof]),
    ];
    for (text, expected) in cases {
        for arguments_by_whitespace in [true, false] {
            for tolerant in [false, true] {
                let options = LexOptions {
                    arguments_by_whitespace,
                    tolerant,
                    ..LexOptions::default()
                };
                let read: Vec<_> = tokenize(text, options)
                    .unwrap()
                    .into_iter()
                    .map(|token| token.kind)
                    .collect();
                assert_eq!(read, expected, "{text:?} {options:?}");
            }
        }
    }
}

#[test]
fn reads_raw_text_in_either_context_after_an_opening_bracket() {
    for arguments_by_whitespace in [true, false] {
        for context in [LexContext::Value, LexContext::Operator] {
            let options = LexOptions {
                arguments_by_whitespace,
                ..LexOptions::default()
            };
            let mut lexer = Lexer::new("f(r#\"a\"#)", options);
            lexer.next(LexContext::Value).unwrap();
            lexer.next(LexContext::Operator).unwrap();
            let raw = lexer.next(context).unwrap();
            assert_eq!((raw.kind, raw.range), (TokenKind::RawText, 2..8));
        }
    }
}

#[test]
fn reads_r_hash_without_quote_as_identifier_and_comment() {
    let options = LexOptions {
        keep_comments: true,
        ..LexOptions::default()
    };
    let mut lexer = Lexer::new("r#x", options);
    let word = lexer.next(LexContext::Value).unwrap();
    assert_eq!(
        (word.kind, word.range, word.space_after),
        (TokenKind::Ident, 0..1, true)
    );
    assert_eq!(
        lexer.next(LexContext::Operator).unwrap().kind,
        TokenKind::Eof
    );
    assert_eq!(lexer.comments()[0].text, "x");
}

#[test]
fn counts_columns_in_unicode_scalar_values() {
    let read = tokens("\"\u{1F600}\" x");
    assert_eq!(read[0].span, span(1, 1, 1, 4));
    assert_eq!(read[1].span, span(1, 5, 1, 6));
    let read = tokens("\"e\u{301}\" x");
    assert_eq!(read[0].span, span(1, 1, 1, 5));
    assert_eq!(read[1].span, span(1, 6, 1, 7));
    let read = tokens("\ty");
    assert_eq!(read[0].span, span(1, 2, 1, 3));
}

#[test]
fn spans_multiline_texts() {
    let read = tokens("\"a\nbc\" x");
    assert_eq!(read[0].span, span(1, 1, 2, 4));
    assert_eq!(read[1].span, span(2, 5, 2, 6));
}

#[test]
fn returns_end_of_input_repeatedly() {
    let mut lexer = Lexer::new("a ", LexOptions::default());
    lexer.next(LexContext::Value).unwrap();
    for _ in 0..3 {
        let end = lexer.next(LexContext::Value).unwrap();
        assert_eq!(
            (end.kind, end.range, end.span),
            (TokenKind::Eof, 2..2, span(1, 3, 1, 3))
        );
    }
}

#[test]
fn rewinds_to_a_checkpoint() {
    let options = LexOptions {
        keep_comments: true,
        ..LexOptions::default()
    };
    let mut lexer = Lexer::new("(a # one\nb)\nc", options);
    assert_eq!(
        lexer.next(LexContext::Value).unwrap().kind,
        TokenKind::LParen
    );
    let checkpoint = lexer.checkpoint();
    let first: Vec<_> = (0..5)
        .map(|_| lexer.next(LexContext::Value).unwrap())
        .collect();
    assert_eq!(lexer.comments().len(), 1);
    lexer.rewind(checkpoint);
    assert!(lexer.comments().is_empty());
    let second: Vec<_> = (0..5)
        .map(|_| lexer.next(LexContext::Value).unwrap())
        .collect();
    assert_eq!(first, second);
    let kinds: Vec<_> = second.into_iter().map(|token| token.kind).collect();
    assert_eq!(
        kinds,
        [
            TokenKind::Ident,
            TokenKind::Ident,
            TokenKind::RParen,
            TokenKind::Nl,
            TokenKind::Ident
        ]
    );
    assert_eq!(lexer.comments().len(), 1);
}

#[test]
fn continues_after_an_error() {
    let mut lexer = Lexer::new("a $ b", LexOptions::default());
    assert_eq!(
        lexer.next(LexContext::Value).unwrap().kind,
        TokenKind::Ident
    );
    let error = lexer.next(LexContext::Operator).unwrap_err();
    assert_eq!(
        (error.problem.as_str(), error.at.column),
        ("variable", Some(3))
    );
    let next = lexer.next(LexContext::Value).unwrap();
    assert_eq!((next.kind, next.range), (TokenKind::Ident, 4..5));
}

#[test]
fn rejects_shell_operators_with_fixes() {
    let cases = [
        ("a | b", "pipe", vec!["|>"]),
        ("a | > b", "pipe", vec!["|>"]),
        ("a |-> b", "pipe", vec!["|>"]),
        ("a -> b", "pipe", vec!["|>"]),
        ("$HOME", "variable", vec![]),
        ("a; b", "separator", vec![]),
        ("a && b", "shell_operator", vec![]),
        ("a || b", "shell_operator", vec![]),
        ("a & b", "shell_operator", vec![]),
        ("`a`", "shell_operator", vec![]),
        ("a >> b", "redirect", vec![]),
        ("a << b", "redirect", vec![]),
        ("!a", "bang", vec![]),
        ("rm --force", "flag", vec![]),
        ("'abc'", "single_quote", vec!["\"abc\""]),
        ("\u{201E}abc\u{201D}", "typographic_quote", vec!["\"abc\""]),
        ("a ? b", "unexpected_character", vec![]),
        ("a \\ b", "unexpected_character", vec![]),
    ];
    for (text, problem, fix) in cases {
        let error = error(text);
        assert_eq!(error.code, ErrorCode::Lexical, "{text}");
        assert_eq!(error.problem, problem, "{text}");
        assert_eq!(error.fix, fix, "{text}");
        assert!(!error.message.ends_with('.'), "{text}");
    }
}

#[test]
fn reads_a_single_minus_before_a_word_as_unary_minus() {
    assert_eq!(
        kinds("rm -rf"),
        [
            TokenKind::Ident,
            TokenKind::Minus,
            TokenKind::Ident,
            TokenKind::Eof
        ]
    );
    assert_eq!(error("Remove-Item -Force").problem, "name_case");
    assert_eq!(error("x -Force").problem, "flag");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
