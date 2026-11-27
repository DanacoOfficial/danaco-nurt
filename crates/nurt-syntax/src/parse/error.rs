// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::parse::error
// Responsibility:   Grammar errors of the parsers with positions and ready corrections.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Grammar errors of the parsers with positions and ready corrections.
//!
//! A correction in `fix` is the complete program text with the smallest
//! edit that removes the error; it is offered only when it is certain.

use std::ops::Range;

use nurt_contracts::ast::{BinOp, Span};
use nurt_contracts::error::At;

use super::{Node, Parser, Variant};
use crate::error::SyntaxError;
use crate::lex::{ForeignKind, LexError, Token, TokenKind};

/// Returns `text` with `range` replaced by `replacement`.
pub(crate) fn edit(text: &str, range: &Range<usize>, replacement: &str) -> String {
    let start = range.start.min(text.len());
    let end = range.end.clamp(start, text.len());
    let mut edited = String::with_capacity(text.len() + replacement.len());
    edited.push_str(text.get(..start).unwrap_or_default());
    edited.push_str(replacement);
    edited.push_str(text.get(end..).unwrap_or_default());
    edited
}

/// Position of a token; the length counts the characters of the token.
pub(crate) fn at_of(p: &Parser<'_>, token: &Token) -> At {
    At::new(
        token.span.line,
        token.span.column,
        char_count(p.slice(token)),
    )
}

fn at_of_node(p: &Parser<'_>, span: Span, range: &Range<usize>) -> At {
    At::new(
        span.line,
        span.column,
        char_count(p.text().get(range.clone()).unwrap_or_default()),
    )
}

fn char_count(text: &str) -> u32 {
    u32::try_from(text.chars().count()).unwrap_or(u32::MAX)
}

/// The fragment of a token as shown in `found`.
fn found(p: &Parser<'_>, token: &Token) -> String {
    match token.kind {
        TokenKind::Eof => "end of input".to_owned(),
        TokenKind::Nl => "end of line".to_owned(),
        _ => p.slice(token).to_owned(),
    }
}

/// The fragment of a token as quoted in a message.
fn quoted(p: &Parser<'_>, token: &Token) -> String {
    match token.kind {
        TokenKind::Eof | TokenKind::Nl => found(p, token),
        _ => format!("`{}`", p.slice(token)),
    }
}

fn at_token(p: &Parser<'_>, token: &Token, problem: &str, message: String) -> SyntaxError {
    SyntaxError::syntax(message)
        .with_at(at_of(p, token))
        .with_found(found(p, token))
        .with_problem(problem)
}

fn at_node(
    p: &Parser<'_>,
    span: Span,
    range: &Range<usize>,
    problem: &str,
    message: String,
) -> SyntaxError {
    SyntaxError::syntax(message)
        .with_at(at_of_node(p, span, range))
        .with_found(p.text().get(range.clone()).unwrap_or_default())
        .with_problem(problem)
}

/// Error for a token that does not fit the grammar at its position.
pub(crate) fn unexpected(p: &Parser<'_>, token: &Token, expected: &[&str]) -> SyntaxError {
    if let TokenKind::Foreign(ForeignKind::Malformed(lexical)) = &token.kind {
        return SyntaxError::from(lexical.clone());
    }
    let message = match expected {
        [] => format!("unexpected {}", quoted(p, token)),
        [only] => format!("expected {only}, found {}", quoted(p, token)),
        [init @ .., last] => format!(
            "expected {} or {last}, found {}",
            init.join(", "),
            quoted(p, token)
        ),
    };
    at_token(p, token, "unexpected_token", message).with_expected(expected.iter().copied())
}

/// Error of a literal that the decoder rejects; the position of the decoder
/// error is moved from the literal to the program text.
pub(crate) fn literal(p: &Parser<'_>, token: &Token, lexical: LexError) -> SyntaxError {
    let mut error = SyntaxError::from(lexical);
    if let Some(at) = error.at {
        let at_line = at.line.unwrap_or(1);
        let at_column = at.column.unwrap_or(1);
        let line = token.span.line.saturating_add(at_line.saturating_sub(1));
        let column = if at_line <= 1 {
            token
                .span
                .column
                .saturating_add(at_column.saturating_sub(1))
        } else {
            at_column
        };
        error.at = Some(At {
            line: Some(line),
            column: Some(column),
            ..at
        });
    } else {
        error.at = Some(at_of(p, token));
    }
    error
}

/// Error for nesting deeper than the maximum depth.
pub(crate) fn too_deep_at(span: Span) -> SyntaxError {
    SyntaxError::syntax("nesting too deep; split the expression or the program")
        .with_at(At::new(span.line, span.column, 1))
        .with_problem("nesting_too_deep")
}

/// Error for a program without statements.
pub(crate) fn empty_program(p: &Parser<'_>) -> SyntaxError {
    at_token(
        p,
        p.current(),
        "empty_program",
        "a program has at least one statement".to_owned(),
    )
    .with_expected(["command", "`let`", "`@name`"])
}

/// Error for a token where a statement must start.
pub(crate) fn not_a_statement(p: &Parser<'_>) -> SyntaxError {
    let token = p.current();
    at_token(
        p,
        token,
        "expected_statement",
        format!(
            "a statement starts with a command, `let` or a reference, found {}",
            quoted(p, token)
        ),
    )
    .with_expected(["command", "`let`", "`@name`"])
}

/// Error for a token where a command name must start.
pub(crate) fn not_a_command(p: &Parser<'_>) -> SyntaxError {
    let token = p.current();
    let message = if is_keyword(&token.kind) {
        format!(
            "{} is a keyword and cannot be a command name",
            quoted(p, token)
        )
    } else {
        format!("expected a command name, found {}", quoted(p, token))
    };
    at_token(p, token, "expected_command", message).with_expected(["command"])
}

/// Error for a command name longer than the limit.
pub(crate) fn name_too_long(p: &Parser<'_>, first: &Token, last: &Token) -> SyntaxError {
    let span = super::join(first.span, last.span);
    let range = first.range.start..last.range.end;
    at_node(
        p,
        span,
        &range,
        "name_too_long",
        format!(
            "command name is longer than {} characters",
            super::MAX_COMMAND_NAME_LEN
        ),
    )
}

/// Error for a missing or reserved name after `let`.
pub(crate) fn bad_let_name(p: &Parser<'_>) -> SyntaxError {
    let token = p.current();
    let message = if is_keyword(&token.kind) {
        format!(
            "{} is a keyword and cannot name a result; choose another name",
            quoted(p, token)
        )
    } else {
        format!("expected a name after `let`, found {}", quoted(p, token))
    };
    at_token(p, token, "expected_name", message).with_expected(["name"])
}

/// Error for a reference after `|>`.
pub(crate) fn reference_after_pipe(p: &Parser<'_>) -> SyntaxError {
    at_token(
        p,
        p.current(),
        "reference_after_pipe",
        "a reference cannot follow `|>`; start the statement with the reference".to_owned(),
    )
    .with_expected(["command"])
}

/// Error for a call written after a field access or a reference.
pub(crate) fn call_on_field(p: &Parser<'_>, name: &Token) -> SyntaxError {
    at_token(
        p,
        name,
        "call_on_field",
        format!(
            "only a bare identifier can be called; {} is a field",
            quoted(p, name)
        ),
    )
}

/// Error for a token after a complete statement.
pub(crate) fn statement_end(p: &Parser<'_>) -> SyntaxError {
    let token = p.current();
    let text = p.text();
    match token.kind {
        TokenKind::Ident | TokenKind::Let | TokenKind::At => {
            let start = whitespace_start(text, token.range.start);
            at_token(
                p,
                token,
                "statement_on_same_line",
                "two statements on one line; start the second statement on a new line".to_owned(),
            )
            .with_fix(edit(text, &(start..token.range.start), "\n"))
        }
        TokenKind::Comma if p.variant == Variant::B => comma_after_stage(p, token),
        TokenKind::Comma => at_token(
            p,
            token,
            "comma_between_arguments",
            "arguments are separated by spaces; remove the `,`".to_owned(),
        )
        .with_fix(remove_separator(text, &token.range)),
        TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => unmatched_bracket(p, token),
        TokenKind::Eq if p.variant == Variant::A && !token.space_before => at_token(
            p,
            token,
            "equals_in_named_argument",
            "a named argument is written `name: value`; replace `=` with `:`".to_owned(),
        )
        .with_fix(edit(
            text,
            &token.range,
            if token.space_after { ":" } else { ": " },
        )),
        _ => unexpected(p, token, statement_end_expected(p.variant)),
    }
}

/// Tokens that may follow a complete stage: in variant B the arguments are
/// inside the parentheses, in A and C they follow the command.
fn statement_end_expected(variant: Variant) -> &'static [&'static str] {
    match variant {
        Variant::B => &["`|>`", "end of line"],
        Variant::A | Variant::C => &["argument", "`|>`", "end of line"],
    }
}

/// Error for `,` after a complete stage of variant B. Removing the comma is
/// offered only when `|>`, a comment or the end of the line follows it;
/// otherwise the arguments after the comma belong inside the parentheses.
fn comma_after_stage(p: &Parser<'_>, comma: &Token) -> SyntaxError {
    let text = p.text();
    let error = at_token(
        p,
        comma,
        "comma_after_stage",
        "a stage ends at its closing parenthesis; remove the `,` or move the arguments inside the parentheses"
            .to_owned(),
    )
    .with_expected(statement_end_expected(Variant::B).iter().copied());
    let rest = text
        .get(comma.range.end..)
        .unwrap_or_default()
        .trim_start_matches([' ', '\t']);
    let ends_stage =
        rest.is_empty() || rest.starts_with(['\n', '\r', '#']) || rest.starts_with("|>");
    if ends_stage {
        let start = whitespace_start(text, comma.range.start);
        error.with_fix(edit(text, &(start..comma.range.end), ""))
    } else {
        error
    }
}

/// Error for a closing bracket after a complete statement, where no bracket
/// is open; the fix removes it with the spaces before it.
fn unmatched_bracket(p: &Parser<'_>, bracket: &Token) -> SyntaxError {
    let text = p.text();
    let start = whitespace_start(text, bracket.range.start);
    let glued_to_next = text
        .get(bracket.range.end..)
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with([' ', '\t', '\n', '\r']));
    at_token(
        p,
        bracket,
        "unmatched_bracket",
        format!("{} closes no open bracket; remove it", quoted(p, bracket)),
    )
    .with_expected(statement_end_expected(p.variant).iter().copied())
    .with_fix(edit(
        text,
        &(start..bracket.range.end),
        if glued_to_next { " " } else { "" },
    ))
}

/// Error for a token where a value must start. In variant B, a value
/// missing before `|>` directly after the `(` or a `,` of the only open
/// argument list means the list is not closed; the fix closes it before
/// `|>`.
pub(crate) fn missing_operand(p: &Parser<'_>) -> SyntaxError {
    let token = p.current();
    match close_before_pipe(p) {
        Some(fix) => at_token(
            p,
            token,
            "unclosed_arguments",
            "missing `)` before `|>`; close the arguments of the stage".to_owned(),
        )
        .with_expected(["value", "`)`"])
        .with_fix(fix),
        None => unexpected(p, token, &["value"]),
    }
}

/// Program text with the argument list closed before `|>`: an opening `(`
/// directly before is followed by `)`, a `,` directly before becomes `)`.
fn close_before_pipe(p: &Parser<'_>) -> Option<String> {
    let only_parenthesis =
        matches!(p.brackets.as_slice(), [open] if open.kind == TokenKind::LParen);
    if p.variant != Variant::B || p.current().kind != TokenKind::PipeGt || !only_parenthesis {
        return None;
    }
    let previous = p.previous.as_ref()?;
    let text = p.text();
    match previous.kind {
        TokenKind::LParen => Some(edit(text, &(previous.range.end..previous.range.end), ")")),
        TokenKind::Comma => Some(edit(text, &previous.range, ")")),
        _ => None,
    }
}

/// Error for a bare command name before `|>` in parentheses of variant B,
/// where every stage has parentheses; the fix appends `()` to the name.
pub(crate) fn stage_without_parentheses(
    p: &Parser<'_>,
    name: &str,
    span: Span,
    range: &Range<usize>,
) -> SyntaxError {
    at_node(
        p,
        span,
        range,
        "missing_parentheses",
        format!("command `{name}` has no arguments: write `{name}()`"),
    )
    .with_expected(["`(`"])
    .with_fix(edit(p.text(), &(range.end..range.end), "()"))
}

/// Start of the run of spaces and tabs that ends at `end`.
fn whitespace_start(text: &str, end: usize) -> usize {
    let before = text.get(..end).unwrap_or_default();
    end - (before.len() - before.trim_end_matches([' ', '\t']).len())
}

/// Text with a separator removed; a space keeps the neighbours apart.
fn remove_separator(text: &str, range: &Range<usize>) -> String {
    let followed_by_space = text
        .get(range.end..)
        .is_some_and(|rest| rest.starts_with([' ', '\t', '\n', '\r']));
    edit(text, range, if followed_by_space { "" } else { " " })
}

/// Error for `not not x`; `inner` is the second `not` and `operand` the
/// expression that starts with it.
pub(crate) fn double_not(p: &Parser<'_>, inner: &Token, operand: &Node) -> SyntaxError {
    at_token(
        p,
        inner,
        "double_not",
        "`not` cannot directly follow `not`; write `not (not x)`".to_owned(),
    )
    .with_fix(parenthesize(p.text(), &operand.range))
}

/// Error for `a == not b`; `operand` is the `not` expression.
pub(crate) fn not_in_comparison(p: &Parser<'_>, operand: &Node) -> SyntaxError {
    at_node(
        p,
        operand.span,
        &operand.range,
        "not_in_comparison",
        "`not` cannot be an operand of a comparison; put it in parentheses".to_owned(),
    )
    .with_fix(parenthesize(p.text(), &operand.range))
}

/// Error for `a < b < c`; `operator` is the second comparison operator and
/// `middle` the operand between the two operators.
pub(crate) fn chained_comparison(
    p: &Parser<'_>,
    operator: &Token,
    next: BinOp,
    middle: &Node,
) -> SyntaxError {
    let text = p.text();
    let middle_text = text.get(middle.range.clone()).unwrap_or_default();
    let insertion = format!(" and {middle_text}");
    let at = middle.range.end..middle.range.end;
    at_token(
        p,
        operator,
        "chained_comparison",
        format!(
            "comparisons cannot be chained; join them with `and`, as in `a < b and b {next} c`"
        ),
    )
    .with_fix(edit(text, &at, &insertion))
}

/// Error for `- -x` and `--x`; `operand` is the expression that starts with
/// the second `-`.
pub(crate) fn double_minus(
    p: &Parser<'_>,
    outer: &Token,
    inner: &Token,
    operand: &Node,
) -> SyntaxError {
    let text = p.text();
    let range = outer.range.start..operand.range.end;
    let operand_text = text.get(operand.range.clone()).unwrap_or_default();
    at_token(
        p,
        inner,
        "double_minus",
        "`-` cannot directly follow `-`; write `-(-x)`".to_owned(),
    )
    .with_fix(edit(text, &range, &format!("-({operand_text})")))
}

/// Returns `text` with `range` enclosed in parentheses.
fn parenthesize(text: &str, range: &Range<usize>) -> String {
    let inner = text.get(range.clone()).unwrap_or_default();
    edit(text, range, &format!("({inner})"))
}

/// Error for a negated range, `-1..5`.
pub(crate) fn negative_range(p: &Parser<'_>, operator: &Token, operand: &Node) -> SyntaxError {
    let range = operator.range.start..operand.range.end;
    at_node(
        p,
        super::join(operator.span, operand.span),
        &range,
        "range_bound",
        "range bounds are non-negative integers".to_owned(),
    )
}

/// Error for a unary `+`.
pub(crate) fn unary_plus(p: &Parser<'_>) -> SyntaxError {
    let token = p.current();
    at_token(
        p,
        token,
        "unary_plus",
        "there is no unary `+`; remove it".to_owned(),
    )
    .with_fix(edit(p.text(), &token.range, ""))
}

/// Error for `...x` outside a list; `range` covers the spread expression.
pub(crate) fn spread_outside_list(p: &Parser<'_>, span: Span, range: &Range<usize>) -> SyntaxError {
    let text = p.text();
    let spread = text.get(range.clone()).unwrap_or_default();
    at_node(
        p,
        span,
        range,
        "spread_outside_list",
        "`...` is allowed only in a list; enclose the spread in `[` and `]`".to_owned(),
    )
    .with_fix(edit(text, range, &format!("[{spread}]")))
}

/// Error for a keyword used as a value.
pub(crate) fn keyword_as_value(p: &Parser<'_>) -> SyntaxError {
    let token = p.current();
    at_token(
        p,
        token,
        "keyword_as_value",
        format!(
            "{} is a keyword and cannot be used as a value",
            quoted(p, token)
        ),
    )
    .with_expected(["value"])
}

/// Error for a range bound that is not an integer literal.
pub(crate) fn range_bound(p: &Parser<'_>, span: Span, range: &Range<usize>) -> SyntaxError {
    at_node(
        p,
        span,
        range,
        "range_bound",
        "range bounds are integers, written without spaces around `..`".to_owned(),
    )
}

/// Error for a trailing comma in the arguments of a call.
pub(crate) fn trailing_comma_in_call(p: &Parser<'_>, comma: &Token) -> SyntaxError {
    at_token(
        p,
        comma,
        "trailing_comma",
        "a call has no trailing comma; remove the `,`".to_owned(),
    )
    .with_fix(edit(p.text(), &comma.range, ""))
}

/// Error for a missing `,` between the items of a list or a map.
pub(crate) fn missing_separator(p: &Parser<'_>, closing: &str) -> SyntaxError {
    unexpected(p, p.current(), &["`,`", closing])
}

/// Error for the second occurrence of a map key.
pub(crate) fn duplicate_key(p: &Parser<'_>, token: &Token, key: &str) -> SyntaxError {
    at_token(
        p,
        token,
        "duplicate_key",
        format!("duplicate key `{key}` in a map; keep one entry"),
    )
}

/// Error for a pipeline in parentheses whose head is neither a command nor
/// a reference.
pub(crate) fn bad_pipeline_head(p: &Parser<'_>, span: Span, range: &Range<usize>) -> SyntaxError {
    at_node(
        p,
        span,
        range,
        "expected_command",
        "a pipeline in parentheses starts with a command or a reference".to_owned(),
    )
    .with_expected(["command", "`@name`"])
}

/// Error for `name :` with whitespace before the colon; `range` covers the
/// name.
pub(crate) fn space_before_colon(p: &Parser<'_>, range: &Range<usize>) -> SyntaxError {
    let colon = p.current();
    at_token(
        p,
        colon,
        "space_before_colon",
        "remove the space before `:`; a named argument is written `name: value`".to_owned(),
    )
    .with_fix(edit(p.text(), &(range.end..colon.range.start), ""))
}

/// Error for a named argument without a value.
pub(crate) fn missing_value(p: &Parser<'_>, name: &Token, colon: &Token) -> SyntaxError {
    let token = p.current();
    at_token(
        p,
        token,
        "missing_value",
        format!(
            "expected a value after `{}{}`, found {}",
            p.slice(name),
            p.slice(colon),
            quoted(p, token)
        ),
    )
    .with_expected(["value"])
}

fn is_keyword(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Let
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Null
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Not
            | TokenKind::In
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_replace_insert_and_delete() {
        assert_eq!(edit("abc", &(1..2), "XY"), "aXYc");
        assert_eq!(edit("abc", &(3..3), ")"), "abc)");
        assert_eq!(edit("abc", &(0..1), ""), "bc");
        assert_eq!(edit("abc", &(5..9), "!"), "abc!");
    }

    #[test]
    fn finds_the_start_of_spaces_and_tabs() {
        assert_eq!(whitespace_start("ab \t c", 5), 2);
        assert_eq!(whitespace_start("abc", 3), 3);
    }

    #[test]
    fn keeps_neighbours_apart_when_removing_a_separator() {
        assert_eq!(remove_separator("a, b", &(1..2)), "a b");
        assert_eq!(remove_separator("a,b", &(1..2)), "a b");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
