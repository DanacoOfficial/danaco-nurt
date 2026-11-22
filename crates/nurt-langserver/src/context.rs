// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Place of the cursor in the tokens of a program text.
//!
//! The analysis works on tokens, not on the syntax tree, because the text
//! being edited is usually not a valid program. A stage starts at the start
//! of the text, after `|>`, after a line break, after `let name =` and inside
//! an opening parenthesis; its first name, with dotted parts, is the command.

use std::ops::Range;

use nurt_syntax::lex::{Glued, LexOptions, Token, TokenKind, tokenize};
use nurt_syntax::parse::Variant;

use crate::position::Lines;

/// What the cursor can complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Place {
    /// The name of a command; `replace` is the name typed so far.
    Command {
        /// Bytes of the name typed so far.
        replace: Range<usize>,
    },
    /// The label of a parameter of `command`.
    Parameter {
        /// Name of the command of the stage.
        command: String,
        /// Labels already written in the stage.
        used: Vec<String>,
        /// Bytes of the label typed so far.
        replace: Range<usize>,
    },
    /// The value of the labelled parameter `param` of `command`.
    Value {
        /// Name of the command of the stage.
        command: String,
        /// Name of the parameter.
        param: String,
        /// Bytes of the value typed so far.
        replace: Range<usize>,
    },
    /// A name after `@`.
    Reference {
        /// Names bound by `let` before the cursor, in order.
        names: Vec<String>,
        /// Bytes of the name typed so far.
        replace: Range<usize>,
    },
    /// Nothing to complete: a literal, a comment or an expression.
    Other,
}

/// Name under the cursor that the catalog describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// The command name of a stage.
    Command {
        /// Name of the command.
        name: String,
        /// Bytes of the name.
        range: Range<usize>,
    },
    /// A parameter label.
    Parameter {
        /// Name of the command of the stage.
        command: String,
        /// Name of the parameter.
        param: String,
        /// Bytes of the label.
        range: Range<usize>,
    },
}

/// Reads the tokens of a text without the end token, stopping before the
/// first lexical error.
pub(crate) fn tokens(text: &str, variant: Variant) -> Vec<Token> {
    let options = LexOptions {
        arguments_by_whitespace: variant != Variant::B,
        tolerant: true,
        keep_comments: false,
    };
    let mut tokens = match tokenize(text, options) {
        Ok(tokens) => tokens,
        Err(error) => {
            let end = error
                .at
                .line
                .zip(error.at.column)
                .map_or(0, |(line, column)| {
                    Lines::new(text).column_offset(line, column)
                });
            text.get(..end)
                .and_then(|prefix| tokenize(prefix, options).ok())
                .unwrap_or_default()
        }
    };
    tokens.retain(|token| token.kind != TokenKind::Eof);
    tokens
}

/// Determines what can be completed at byte `offset`.
pub(crate) fn place(text: &str, tokens: &[Token], offset: usize, variant: Variant) -> Place {
    if in_literal_or_comment(text, tokens, offset) {
        return Place::Other;
    }
    let before = tokens.partition_point(|token| token.range.end <= offset);
    let word = word_start(tokens, before, offset);
    let start = if word < before {
        tokens[word].range.start
    } else {
        offset
    };
    let replace = start..offset;
    if word > 0 && tokens[word - 1].kind == TokenKind::At && tokens[word - 1].range.end == start {
        let names = let_names(text, &tokens[..word]);
        return Place::Reference { names, replace };
    }
    let Some(segment) = segment(text, tokens, word, variant) else {
        return Place::Other;
    };
    let Some(command) = segment.command else {
        return if segment.first == word {
            Place::Command { replace }
        } else {
            Place::Other
        };
    };
    let args_start = if segment.call {
        segment.first
    } else {
        command.next
    };
    let args = tokens.get(args_start..word).unwrap_or_default();
    if let [.., label, separator] = args
        && is_label(label)
        && matches!(separator.kind, TokenKind::Colon | TokenKind::Eq)
    {
        let param = slice(text, &label.range);
        return Place::Value {
            command: command.name,
            param,
            replace,
        };
    }
    let opens_argument = if segment.call {
        args.last()
            .is_none_or(|token| token.kind == TokenKind::Comma)
    } else {
        let spaced = tokens[word - 1].range.end < start;
        spaced && args.last().is_none_or(|token| !is_operator(&token.kind))
    };
    if !opens_argument {
        return Place::Other;
    }
    let used = args
        .iter()
        .filter(|token| is_label(token))
        .map(|token| slice(text, &token.range))
        .collect();
    Place::Parameter {
        command: command.name,
        used,
        replace,
    }
}

/// Finds the command name or parameter label at byte `offset`.
pub(crate) fn target(
    text: &str,
    tokens: &[Token],
    offset: usize,
    variant: Variant,
) -> Option<Target> {
    let index = tokens
        .iter()
        .position(|token| token.range.start <= offset && offset < token.range.end)
        .or_else(|| {
            tokens
                .iter()
                .position(|token| token.range.end == offset && token.kind == TokenKind::Ident)
        })?;
    let token = &tokens[index];
    if token.kind != TokenKind::Ident {
        return None;
    }
    if is_label(token) {
        let command = segment(text, tokens, index, variant)?.command?;
        let param = slice(text, &token.range);
        return Some(Target::Parameter {
            command: command.name,
            param,
            range: token.range.clone(),
        });
    }
    let segment = segment(text, tokens, index + 1, variant)?;
    let command = if segment.call {
        segment.command
    } else {
        chain(text, tokens, segment.first, tokens.len())
    };
    let command = command.filter(|command| {
        command.range.start <= token.range.start && token.range.end <= command.range.end
    })?;
    Some(Target::Command {
        name: command.name,
        range: command.range,
    })
}

/// Command name of a stage: a name and its dotted parts.
#[derive(Debug)]
struct Chain {
    name: String,
    range: Range<usize>,
    /// Index of the token after the name.
    next: usize,
}

/// Stage that contains a token.
#[derive(Debug)]
struct Segment {
    /// Index of the first token of the stage, or of the first argument of a
    /// call.
    first: usize,
    command: Option<Chain>,
    /// Whether the stage is a call `name(…)` of variant B.
    call: bool,
}

/// Finds the stage that contains the tokens before index `end`; returns
/// `None` inside a list or a map.
fn segment(text: &str, tokens: &[Token], end: usize, variant: Variant) -> Option<Segment> {
    let end = end.min(tokens.len());
    let mut depth = 0_usize;
    for index in (0..end).rev() {
        match tokens[index].kind {
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => depth += 1,
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace if depth > 0 => depth -= 1,
            TokenKind::LParen => return Some(opened(text, tokens, index, end, variant)),
            TokenKind::LBracket | TokenKind::LBrace => return None,
            TokenKind::PipeGt | TokenKind::Nl if depth == 0 => {
                return Some(starting(text, tokens, index + 1, end));
            }
            TokenKind::Eq if depth == 0 && binds_name(tokens, index) => {
                return Some(starting(text, tokens, index + 1, end));
            }
            _ => {}
        }
    }
    Some(starting(text, tokens, 0, end))
}

fn starting(text: &str, tokens: &[Token], first: usize, end: usize) -> Segment {
    Segment {
        first,
        command: chain(text, tokens, first, end),
        call: false,
    }
}

/// Stage inside the parenthesis at index `paren`: the arguments of a call in
/// variant B, otherwise a nested pipeline.
fn opened(text: &str, tokens: &[Token], paren: usize, end: usize, variant: Variant) -> Segment {
    let callee = paren.checked_sub(1).map(|index| &tokens[index]);
    let is_call = variant == Variant::B
        && callee
            .is_some_and(|token| token.kind == TokenKind::Ident && token.glued == Glued::LParen);
    if is_call {
        let start = chain_start(tokens, paren - 1);
        if let Some(command) = chain(text, tokens, start, paren) {
            return Segment {
                first: paren + 1,
                command: Some(command),
                call: true,
            };
        }
    }
    starting(text, tokens, paren + 1, end)
}

/// Reads a command name starting at index `first`, before index `end`.
fn chain(text: &str, tokens: &[Token], first: usize, end: usize) -> Option<Chain> {
    let end = end.min(tokens.len());
    if first >= end || tokens[first].kind != TokenKind::Ident {
        return None;
    }
    let mut last = first;
    while last + 2 < end && dotted(tokens, last) {
        last += 2;
    }
    let range = tokens[first].range.start..tokens[last].range.end;
    Some(Chain {
        name: slice(text, &range),
        range,
        next: last + 1,
    })
}

/// Index of the first part of the dotted name that ends at index `last`.
fn chain_start(tokens: &[Token], last: usize) -> usize {
    let mut start = last;
    while start >= 2 && dotted(tokens, start - 2) {
        start -= 2;
    }
    start
}

/// Whether the name at `index` continues with `.name` without spaces.
fn dotted(tokens: &[Token], index: usize) -> bool {
    match tokens.get(index..index + 3) {
        Some([name, dot, part]) => {
            name.kind == TokenKind::Ident
                && dot.kind == TokenKind::Dot
                && part.kind == TokenKind::Ident
                && adjacent(name, dot)
                && adjacent(dot, part)
        }
        _ => false,
    }
}

/// Index of the first token of the word that ends at `offset`; `before` when
/// no word ends there.
fn word_start(tokens: &[Token], before: usize, offset: usize) -> usize {
    let mut start = before;
    while start > 0 {
        let token = &tokens[start - 1];
        let joined = if start == before {
            token.range.end == offset
        } else {
            adjacent(token, &tokens[start])
        };
        if !joined || !(is_word(&token.kind) || token.kind == TokenKind::Dot) {
            break;
        }
        start -= 1;
    }
    while start < before && tokens[start].kind == TokenKind::Dot {
        start += 1;
    }
    start
}

/// Whether `offset` lies inside a literal, a comment or text the lexer
/// rejected.
fn in_literal_or_comment(text: &str, tokens: &[Token], offset: usize) -> bool {
    let inside = tokens.iter().any(|token| {
        token.range.start < offset && offset < token.range.end && !is_word(&token.kind)
    });
    if inside {
        return true;
    }
    let before = tokens.partition_point(|token| token.range.end <= offset);
    let gap_start = before
        .checked_sub(1)
        .map_or(0, |index| tokens[index].range.end);
    text.get(gap_start..offset)
        .is_some_and(|gap| gap.contains(['#', '"', '\'']))
}

/// Whether the `=` at `index` follows `let name`.
fn binds_name(tokens: &[Token], index: usize) -> bool {
    index >= 2
        && tokens[index - 1].kind == TokenKind::Ident
        && tokens[index - 2].kind == TokenKind::Let
}

/// Names bound by `let` in `tokens`, without repetitions.
fn let_names(text: &str, tokens: &[Token]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for pair in tokens.windows(2) {
        if let [keyword, name] = pair
            && keyword.kind == TokenKind::Let
            && name.kind == TokenKind::Ident
        {
            let name = slice(text, &name.range);
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn adjacent(left: &Token, right: &Token) -> bool {
    left.range.end == right.range.start
}

fn is_label(token: &Token) -> bool {
    token.kind == TokenKind::Ident && matches!(token.glued, Glued::Colon | Glued::Equals)
}

fn is_word(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Ident
            | TokenKind::Let
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Null
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Not
            | TokenKind::In
    )
}

/// Whether a token is followed by an operand rather than by a new argument.
fn is_operator(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::EqEq
            | TokenKind::NotEq
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::Tilde
            | TokenKind::NotTilde
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Not
            | TokenKind::In
            | TokenKind::DotDot
            | TokenKind::Ellipsis
            | TokenKind::Dot
            | TokenKind::Comma
    )
}

fn slice(text: &str, range: &Range<usize>) -> String {
    text.get(range.clone()).unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Place at the cursor marker of `marked`, its last `|`.
    fn place_at(marked: &str, variant: Variant) -> Place {
        let offset = marked.rfind('|').unwrap();
        let text = format!("{}{}", &marked[..offset], &marked[offset + 1..]);
        place(&text, &tokens(&text, variant), offset, variant)
    }

    fn target_at(marked: &str, variant: Variant) -> Option<Target> {
        let offset = marked.rfind('|').unwrap();
        let text = format!("{}{}", &marked[..offset], &marked[offset + 1..]);
        target(&text, &tokens(&text, variant), offset, variant)
    }

    fn parameter(command: &str, used: &[&str], replace: Range<usize>) -> Place {
        Place::Parameter {
            command: command.to_owned(),
            used: used.iter().map(|name| (*name).to_owned()).collect(),
            replace,
        }
    }

    #[test]
    fn completes_command_at_start_of_text_and_after_pipe() {
        assert_eq!(place_at("|", Variant::A), Place::Command { replace: 0..0 });
        assert_eq!(
            place_at("fi|", Variant::A),
            Place::Command { replace: 0..2 }
        );
        assert_eq!(
            place_at("find \"/srv\" |> so|", Variant::A),
            Place::Command { replace: 15..17 }
        );
        assert_eq!(
            place_at("count\n|> |", Variant::A),
            Place::Command { replace: 9..9 }
        );
        assert_eq!(
            place_at("let big = |", Variant::A),
            Place::Command { replace: 10..10 }
        );
    }

    #[test]
    fn completes_dotted_command_names_as_one_word() {
        assert_eq!(
            place_at("git.st|", Variant::A),
            Place::Command { replace: 0..6 }
        );
        assert_eq!(
            place_at("git.|", Variant::A),
            Place::Command { replace: 0..4 }
        );
    }

    #[test]
    fn completes_parameters_after_a_space() {
        assert_eq!(
            place_at("find \"/srv\" |", Variant::A),
            parameter("find", &[], 12..12)
        );
        assert_eq!(
            place_at("find \"/srv\" glob: \"*.go\" ty|", Variant::A),
            parameter("find", &["glob"], 25..27)
        );
        assert_eq!(place_at("find \"/srv\"|", Variant::A), Place::Other);
    }

    #[test]
    fn completes_parameters_of_named_and_call_notations() {
        assert_eq!(
            place_at("sort by=size |", Variant::C),
            parameter("sort", &["by"], 13..13)
        );
        assert_eq!(
            place_at("sort(size, |", Variant::B),
            parameter("sort", &[], 11..11)
        );
        assert_eq!(
            place_at("find(\"/srv\", glob: \"*\", |", Variant::B),
            parameter("find", &["glob"], 24..24)
        );
    }

    #[test]
    fn completes_values_after_a_label() {
        let value = |replace| Place::Value {
            command: "sort".to_owned(),
            param: "order".to_owned(),
            replace,
        };
        assert_eq!(place_at("sort size order: |", Variant::A), value(17..17));
        assert_eq!(place_at("sort size order: de|", Variant::A), value(17..19));
        assert_eq!(place_at("sort by=size order=|", Variant::C), value(19..19));
        assert_eq!(place_at("sort(size, order: |", Variant::B), value(18..18));
    }

    #[test]
    fn completes_references_bound_earlier() {
        let text = "let big = find \"/srv\"\nlet small = count\n@|";
        let names = vec!["big".to_owned(), "small".to_owned()];
        assert_eq!(
            place_at(text, Variant::A),
            Place::Reference {
                names,
                replace: 41..41
            }
        );
    }

    #[test]
    fn completes_nothing_in_literals_comments_and_expressions() {
        assert_eq!(place_at("find \"/s|rv\"", Variant::A), Place::Other);
        assert_eq!(place_at("count # a |", Variant::A), Place::Other);
        assert_eq!(place_at("filter size > |", Variant::A), Place::Other);
        assert_eq!(
            place_at("find| \"/srv", Variant::A),
            Place::Command { replace: 0..4 }
        );
        assert_eq!(place_at("find \"/srv |", Variant::A), Place::Other);
        assert_eq!(place_at("set [1, |", Variant::A), Place::Other);
    }

    #[test]
    fn finds_command_and_parameter_under_the_cursor() {
        let command = |name: &str, range| {
            Some(Target::Command {
                name: name.to_owned(),
                range,
            })
        };
        assert_eq!(
            target_at("fi|nd \"/srv\"", Variant::A),
            command("find", 0..4)
        );
        assert_eq!(
            target_at("count |> git.s|tatus", Variant::A),
            command("git.status", 9..19)
        );
        assert_eq!(
            target_at("gi|t.status \"/srv\"", Variant::A),
            command("git.status", 0..10)
        );
        assert_eq!(
            target_at("git.sta|tus(all: true)", Variant::B),
            command("git.status", 0..10)
        );
        assert_eq!(target_at("first(2|0)", Variant::B), None);
        assert_eq!(
            target_at("sort(size, ord|er: desc)", Variant::B),
            Some(Target::Parameter {
                command: "sort".to_owned(),
                param: "order".to_owned(),
                range: 11..16,
            })
        );
        assert_eq!(target_at("sort si|ze", Variant::A), None);
    }

    #[test]
    fn keeps_tokens_before_a_lexical_error() {
        let text = "find \"/srv\" |> count \"abc";
        let kinds: Vec<TokenKind> = tokens(text, Variant::A)
            .into_iter()
            .map(|token| token.kind)
            .collect();
        assert_eq!(
            kinds[..4],
            [
                TokenKind::Ident,
                TokenKind::Text,
                TokenKind::PipeGt,
                TokenKind::Ident
            ]
        );
    }
}
