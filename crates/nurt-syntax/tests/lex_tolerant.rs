// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Foreign tokens of the tolerant mode.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::error::ErrorCode;
use nurt_syntax::lex::{
    ForeignKind, LexContext, LexOptions, Lexer, Token, TokenKind, decode_text, invalid_escapes,
    tokenize,
};

const TOLERANT: LexOptions = LexOptions {
    arguments_by_whitespace: true,
    tolerant: true,
    keep_comments: false,
};

fn tokens(text: &str) -> Vec<Token> {
    tokenize(text, TOLERANT).unwrap()
}

/// Kind and source fragment of every token except the end of the input.
fn fragments(text: &str) -> Vec<(TokenKind, &str)> {
    tokens(text)
        .into_iter()
        .filter(|token| token.kind != TokenKind::Eof)
        .map(|token| (token.kind, &text[token.range]))
        .collect()
}

fn foreign(kind: ForeignKind) -> TokenKind {
    TokenKind::Foreign(kind)
}

#[test]
fn keeps_quoted_texts_of_other_conventions() {
    assert_eq!(
        fragments("replace 'newName('"),
        [
            (TokenKind::Ident, "replace"),
            (
                foreign(ForeignKind::SingleQuoted { content: 9..17 }),
                "'newName('"
            )
        ]
    );
    let text = "x \u{201E}abc\u{201D}";
    let read = tokens(text);
    assert_eq!(
        read[1].kind,
        foreign(ForeignKind::TypographicQuoted { content: 5..8 })
    );
    assert_eq!(&text[read[1].range.clone()], "\u{201E}abc\u{201D}");
    let read = tokens("\u{201C}a\"");
    assert_eq!(
        read[0].kind,
        foreign(ForeignKind::TypographicQuoted { content: 3..4 })
    );
}

#[test]
fn keeps_pipes_and_arrows_of_other_shells() {
    assert_eq!(
        fragments("a | b | > c |-> d -> e"),
        [
            (TokenKind::Ident, "a"),
            (foreign(ForeignKind::Pipe), "|"),
            (TokenKind::Ident, "b"),
            (foreign(ForeignKind::PipeSpaceGt), "| >"),
            (TokenKind::Ident, "c"),
            (foreign(ForeignKind::PipeArrow), "|->"),
            (TokenKind::Ident, "d"),
            (foreign(ForeignKind::Arrow), "->"),
            (TokenKind::Ident, "e"),
        ]
    );
}

#[test]
fn continues_a_pipeline_on_a_line_that_starts_with_a_foreign_pipe() {
    let kinds: Vec<_> = tokens("a\n| b\n-> c")
        .into_iter()
        .map(|token| token.kind)
        .collect();
    assert!(!kinds.contains(&TokenKind::Nl));
}

#[test]
fn keeps_flags_and_shell_operators() {
    assert_eq!(
        fragments("rm --force $x; a && b || c & `d` >> e << f -Recurse"),
        [
            (TokenKind::Ident, "rm"),
            (foreign(ForeignKind::Flag), "--force"),
            (foreign(ForeignKind::Dollar), "$"),
            (TokenKind::Ident, "x"),
            (foreign(ForeignKind::Semicolon), ";"),
            (TokenKind::Ident, "a"),
            (foreign(ForeignKind::AndAnd), "&&"),
            (TokenKind::Ident, "b"),
            (foreign(ForeignKind::OrOr), "||"),
            (TokenKind::Ident, "c"),
            (foreign(ForeignKind::Amp), "&"),
            (foreign(ForeignKind::Backtick), "`"),
            (TokenKind::Ident, "d"),
            (foreign(ForeignKind::Backtick), "`"),
            (foreign(ForeignKind::Redirect), ">>"),
            (TokenKind::Ident, "e"),
            (foreign(ForeignKind::Redirect), "<<"),
            (TokenKind::Ident, "f"),
            (foreign(ForeignKind::Flag), "-Recurse"),
        ]
    );
}

#[test]
fn keeps_unquoted_paths() {
    let cases = [
        ("find /srv/app glob", "/srv/app"),
        ("find C:\\proj\\src", "C:\\proj\\src"),
        ("find C:/proj/src", "C:/proj/src"),
        ("find c:\\proj", "c:\\proj"),
        ("find //server/share", "//server/share"),
    ];
    for (text, path) in cases {
        let read = fragments(text);
        assert_eq!(read[1], (foreign(ForeignKind::BarePath), path), "{text}");
    }
}

#[test]
fn keeps_relative_paths_in_value_context() {
    for (text, path) in [
        ("~/notes", "~/notes"),
        ("./src", "./src"),
        ("../a/b", "../a/b"),
    ] {
        let mut lexer = Lexer::new(text, TOLERANT);
        let token = lexer.next(LexContext::Value).unwrap();
        assert_eq!(
            (token.kind, &text[token.range]),
            (foreign(ForeignKind::BarePath), path)
        );
    }
    let mut lexer = Lexer::new("~/a/", TOLERANT);
    assert_eq!(
        lexer.next(LexContext::Value).unwrap().kind,
        TokenKind::Tilde
    );
}

#[test]
fn keeps_words_that_are_not_identifiers() {
    assert_eq!(
        fragments("Find caseSensitive _x zażółć"),
        [
            (foreign(ForeignKind::LooseWord), "Find"),
            (foreign(ForeignKind::LooseWord), "caseSensitive"),
            (foreign(ForeignKind::LooseWord), "_x"),
            (foreign(ForeignKind::ForeignWord), "zażółć"),
        ]
    );
}

#[test]
fn splits_a_number_from_an_unknown_unit() {
    let cases = [
        ("10MB", TokenKind::Int, "10", "MB"),
        ("5min", TokenKind::Int, "5", "min"),
        ("20abc", TokenKind::Int, "20", "abc"),
        ("1.5h", TokenKind::Number, "1.5", "h"),
        ("0x1g", TokenKind::Int, "0x1", "g"),
    ];
    for (text, number, digits, word) in cases {
        assert_eq!(
            fragments(text),
            [(number, digits), (foreign(ForeignKind::LooseWord), word)],
            "{text}"
        );
    }
}

#[test]
fn keeps_whitespace_of_other_kinds() {
    assert_eq!(
        fragments("a\u{A0}\u{200B}b"),
        [
            (TokenKind::Ident, "a"),
            (foreign(ForeignKind::Whitespace), "\u{A0}\u{200B}"),
            (TokenKind::Ident, "b"),
        ]
    );
    assert_eq!(
        fragments("a\rb")[1],
        (foreign(ForeignKind::Whitespace), "\r")
    );
    let read = tokens("a\u{A0}\r\nb");
    assert_eq!(read[1].kind, foreign(ForeignKind::Whitespace));
    assert_eq!(read[2].kind, TokenKind::Nl);
}

#[test]
fn keeps_a_text_with_invalid_escapes() {
    let text = r#"search "\d+" "C:\data""#;
    let read = tokens(text);
    assert_eq!(read[1].kind, TokenKind::Text);
    assert_eq!(invalid_escapes(&text[read[1].range.clone()]), vec![1..3]);
    assert_eq!(read[2].kind, TokenKind::Text);
    assert_eq!(invalid_escapes(&text[read[2].range.clone()]), vec![3..5]);
    assert_eq!(
        decode_text(&text[read[1].range.clone()]).unwrap_err().code,
        ErrorCode::Escape
    );
}

#[test]
fn keeps_an_unterminated_text_to_the_end_of_the_input() {
    let text = "replace \"newName(\n|> count";
    let read = tokens(text);
    assert_eq!(
        read[1].kind,
        foreign(ForeignKind::UnterminatedText { first_line_end: 17 })
    );
    assert_eq!(read[1].range, 8..text.len());
    assert_eq!(read[2].kind, TokenKind::Eof);
    let read = tokens("r#\"a\r\nb");
    assert_eq!(
        read[0].kind,
        foreign(ForeignKind::UnterminatedText { first_line_end: 4 })
    );
}

#[test]
fn reports_malformed_literals_with_the_strict_error() {
    for (text, problem) in [
        ("1e5", "exponent"),
        ("/a/ii", "repeated_regex_flag"),
        ("0x", "hex_without_digits"),
        ("9007199254740992", "number_too_large"),
        ("1m30", "duration_without_unit"),
    ] {
        let read = tokens(text);
        let TokenKind::Foreign(ForeignKind::Malformed(error)) = &read[0].kind else {
            panic!("{text}: {:?}", read[0].kind);
        };
        assert_eq!(error.problem, problem, "{text}");
        assert_eq!(read[0].range, 0..text.len(), "{text}");
    }
}

#[test]
fn keeps_other_characters_one_by_one() {
    assert_eq!(
        fragments("a ?! \u{20AC}"),
        [
            (TokenKind::Ident, "a"),
            (foreign(ForeignKind::Other), "?"),
            (foreign(ForeignKind::Other), "!"),
            (foreign(ForeignKind::Other), "\u{20AC}"),
        ]
    );
    assert_eq!(fragments("'abc")[0], (foreign(ForeignKind::Other), "'"));
}

#[test]
fn reads_valid_programs_like_the_strict_lexer() {
    let program = "let recent = find \"/srv/app\" glob: \"**/*.ts\" |> filter modified > ago(1h)\n\
                   @recent |> count\n\
                   edit \"/a\" changes: [{replace: r#\"x\"#, line: 3..5}] size: 1.5mib wait: 1h30m\n\
                   search /oldName\\(/i |> first 20 -1 # done\n";
    let strict = tokenize(program, LexOptions::default()).unwrap();
    assert_eq!(tokenize(program, TOLERANT).unwrap(), strict);
}

#[test]
fn never_returns_an_error() {
    let junk = "\u{0}\u{7F}\\'\"\u{201C}|-| >~/..//x/gg 0x 1e 9e999 \u{FEFF}\r\u{301}";
    let mut lexer = Lexer::new(junk, TOLERANT);
    for _ in 0..junk.len() + 2 {
        let token = lexer.next(LexContext::Value).unwrap();
        if token.kind == TokenKind::Eof {
            return;
        }
    }
    panic!("the lexer did not reach the end of the input");
}
