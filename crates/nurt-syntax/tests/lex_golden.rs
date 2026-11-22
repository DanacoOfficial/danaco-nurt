// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Golden programs: every program lexes without error, literal spans match
//! the expected trees, and three programs have their full token stream
//! checked.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::ast::{Expr, Span};
use nurt_syntax::lex::{
    LexOptions, Token, TokenKind, decode_raw, decode_regex, decode_text, parse_duration, parse_int,
    parse_number, parse_size, tokenize,
};
use serde_json::Value;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Program files under `directory`, recursively, in sorted order.
fn programs(directory: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(directory) else {
        return found;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(programs(&path));
        } else if path
            .extension()
            .is_some_and(|extension| extension == "nurt")
        {
            found.push(path);
        }
    }
    found.sort();
    found
}

/// Options of the variant of a golden file: files of variant B lie in a
/// directory named `b`.
fn options_for(path: &Path) -> LexOptions {
    let variant_b = path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "b");
    LexOptions {
        arguments_by_whitespace: !variant_b,
        ..LexOptions::default()
    }
}

fn lex_file(path: &Path) -> (String, Vec<Token>) {
    let text = fs::read_to_string(path).unwrap();
    let tokens = tokenize(&text, options_for(path))
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    (text, tokens)
}

fn span_of(value: &Value) -> Option<Span> {
    serde_json::from_value(value.get("span")?.clone()).ok()
}

/// Literal and identifier nodes of a tree with their spans.
fn literal_nodes<'a>(value: &'a Value, found: &mut Vec<&'a Value>) {
    match value {
        Value::Object(members) => {
            let literal = members.get("t").and_then(Value::as_str).is_some_and(|tag| {
                matches!(
                    tag,
                    "text" | "regex" | "int" | "num" | "size" | "dur" | "bool" | "null" | "ident"
                )
            });
            if literal && members.contains_key("span") {
                found.push(value);
            }
            members
                .values()
                .for_each(|member| literal_nodes(member, found));
        }
        Value::Array(items) => items.iter().for_each(|item| literal_nodes(item, found)),
        _ => {}
    }
}

/// Decodes a literal token into the JSON form of its tree node.
fn decoded(token: &Token, source: &str) -> Value {
    let expr = match token.kind {
        TokenKind::Text => decode_text(source),
        TokenKind::RawText => decode_raw(source),
        TokenKind::Regex => decode_regex(source),
        TokenKind::Int => parse_int(source),
        TokenKind::Number => parse_number(source),
        TokenKind::Size => parse_size(source),
        TokenKind::Duration => parse_duration(source),
        TokenKind::True | TokenKind::False => Ok(Expr::Bool {
            v: token.kind == TokenKind::True,
            span: None,
        }),
        TokenKind::Null => Ok(Expr::Null { span: None }),
        TokenKind::Ident => Ok(Expr::Ident {
            v: source.to_owned(),
            span: None,
        }),
        ref other => panic!("not a literal token: {other:?}"),
    };
    serde_json::to_value(expr.unwrap()).unwrap()
}

fn check_literal_spans(path: &Path) -> usize {
    let tree_path = path.with_extension("json");
    let Ok(tree) = fs::read_to_string(&tree_path) else {
        return 0;
    };
    let tree: Value = serde_json::from_str(&tree).unwrap();
    let (text, tokens) = lex_file(path);
    let mut nodes = Vec::new();
    literal_nodes(&tree, &mut nodes);
    for node in &nodes {
        let span = span_of(node).unwrap();
        let token = tokens
            .iter()
            .find(|token| token.span == span)
            .unwrap_or_else(|| panic!("{}: no token at {span:?}", path.display()));
        let mut expected = (*node).clone();
        if let Some(members) = expected.as_object_mut() {
            members.remove("span");
        }
        assert_eq!(
            decoded(token, &text[token.range.clone()]),
            expected,
            "{}: {span:?}",
            path.display()
        );
    }
    nodes.len()
}

#[test]
fn lexes_every_golden_program_strictly() {
    let root = workspace();
    let mut paths = programs(&root.join("spec/golden/ast"));
    assert!(
        paths.len() >= 62,
        "missing golden programs: {}",
        paths.len()
    );
    paths.extend(programs(&root.join("tests/d1/golden")));
    for path in &paths {
        let (_, tokens) = lex_file(path);
        assert_eq!(
            tokens.last().map(|token| &token.kind),
            Some(&TokenKind::Eof)
        );
    }
}

#[test]
fn matches_literal_spans_and_values_of_the_golden_trees() {
    let root = workspace();
    let mut paths = programs(&root.join("spec/golden/ast"));
    paths.extend(programs(&root.join("tests/d1/golden")));
    let checked: usize = paths.iter().map(|path| check_literal_spans(path)).sum();
    assert!(checked > 250, "too few literal nodes checked: {checked}");
}

fn stream(name: &str) -> Vec<TokenKind> {
    let path = workspace().join("spec/golden/ast").join(name);
    lex_file(&path)
        .1
        .into_iter()
        .map(|token| token.kind)
        .collect()
}

#[test]
fn reads_the_token_stream_of_task_1() {
    use TokenKind::*;
    assert_eq!(
        stream("task-01-largest-go-files.nurt"),
        [
            Ident, Text, Ident, Colon, Text, PipeGt, Ident, Ident, Gt, Size, And, Ident, Gt, Ident,
            LParen, Duration, RParen, PipeGt, Ident, Ident, Ident, Colon, Ident, PipeGt, Ident,
            Int, Nl, Eof
        ]
    );
}

#[test]
fn reads_the_token_stream_of_task_9() {
    use TokenKind::*;
    assert_eq!(
        stream("task-09-insert-line-after-line.nurt"),
        [
            Ident, Text, PipeGt, Ident, Ident, EqEq, Text, PipeGt, Ident, RawText, Ident, Colon,
            Ident, Nl, Eof
        ]
    );
}

#[test]
fn reads_the_token_stream_of_task_13() {
    use TokenKind::*;
    assert_eq!(
        stream("task-13-transactional-edits.nurt"),
        [
            Ident, Text, Ident, Colon, LBracket, LBrace, Ident, Colon, Text, Comma, Ident, Colon,
            Text, Comma, Ident, Colon, Int, RBrace, Comma, LBrace, Ident, Colon, Text, Comma,
            Ident, Colon, Int, Comma, Ident, Colon, Ident, RBrace, Comma, LBrace, Ident, Colon,
            Int, DotDot, Int, RBrace, RBracket, Nl, Eof
        ]
    );
}
