// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repair policy of the tolerant parser for the rules `L08` to `L15`.
//!
//! [`LexicalRepairs`] answers a parse failure with at most one repair per
//! rule. The parser tries them in the order `L10`, `L11`, `L12`, `L13`,
//! `L14`, `L15`, `L08`, `L09`, applies at most three repairs per statement
//! and returns readings with different trees as alternatives:
//!
//! * `L10` (class S): brackets still open at the end of the program are
//!   closed in reverse order after the last token.
//! * `L11` (class S): in call notation, a parenthesis still open before `|>`
//!   is closed before it.
//! * `L12` (class S): a closing bracket without an opening one, followed only
//!   by closing brackets up to the end of the program, is removed.
//! * `L13` (class C): `|`, `| >`, `|->` and `->` between a complete operand
//!   and a command name become `|>`.
//! * `L14` (class S): `|>` is inserted before a command name that the command
//!   of the stage cannot take as an argument.
//! * `L15` (class S): a comma between the arguments of a stage is removed in
//!   word and named notation and inserted between call arguments.
//! * `L08` (class P): a text not closed on its line is closed at the end of
//!   the line, unless `|>` follows on the line.
//! * `L09` (class P): a quote directly after a closed text is removed.
//!
//! Repairs never change the characters of a literal: they insert or remove
//! delimiters, brackets, pipes and commas only.

use nurt_syntax::lex::{ForeignKind, Token, TokenKind};
use nurt_syntax::parse::{Failure, Repair, RepairPolicy, Variant};

use super::rules::Rule;
use super::{ArgumentShape, CatalogView};

/// The canonical pipe operator.
const PIPE: &str = "|>";

/// Quote of a text literal.
const QUOTE: char = '"';

/// Policy of the tolerant parser that proposes the repairs of the lexical
/// rules `L08` to `L15`.
///
/// # Examples
///
/// ```
/// use nurt_correct::lexical::repairs::LexicalRepairs;
/// use nurt_correct::lexical::{ArgumentShape, CatalogView};
/// use nurt_syntax::parse::{ParseOptions, Variant, parse_tolerant};
///
/// struct Commands;
///
/// impl CatalogView for Commands {
///     fn is_command(&self, name: &str) -> bool {
///         ["verify", "run", "count"].contains(&name)
///     }
///     fn command_names(&self) -> &[&str] {
///         &["verify", "run", "count"]
///     }
///     fn is_mutating(&self, name: &str) -> bool {
///         name == "run"
///     }
///     fn can_take_argument(&self, _: &str, _: usize, _: ArgumentShape) -> bool {
///         false
///     }
/// }
///
/// let policy = LexicalRepairs::new(Variant::A, &Commands, &|_| false);
/// let text = "verify (run [\"go\", \"build\"] cwd: \"/a\"";
/// let repaired = parse_tolerant(text, Variant::A, &ParseOptions::default(), &policy)?;
/// assert_eq!(repaired.source, format!("{text})"));
/// assert_eq!(repaired.repairs[0].rule, "L10");
/// # Ok::<(), nurt_syntax::error::SyntaxError>(())
/// ```
pub struct LexicalRepairs<'a> {
    variant: Variant,
    catalog: &'a dyn CatalogView,
    is_habit: &'a dyn Fn(&str) -> bool,
}

impl<'a> LexicalRepairs<'a> {
    /// Policy for programs of `variant`; `is_habit` tells whether a word is a
    /// habit of another shell, such as `grep`.
    #[must_use]
    pub fn new(
        variant: Variant,
        catalog: &'a dyn CatalogView,
        is_habit: &'a dyn Fn(&str) -> bool,
    ) -> Self {
        Self {
            variant,
            catalog,
            is_habit,
        }
    }
}

impl RepairPolicy for LexicalRepairs<'_> {
    fn propose(&self, failure: &Failure, tokens: &[Token], text: &str) -> Vec<Repair> {
        let site = Site {
            failure,
            tokens,
            text,
        };
        [
            close_brackets(&site),
            self.close_call(&site),
            drop_closing(&site),
            self.pipe_notation(&site),
            self.missing_pipe(&site),
            self.commas(&site),
            close_text(&site),
            drop_quote(&site),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

impl LexicalRepairs<'_> {
    /// Rule `L11`.
    fn close_call(&self, site: &Site<'_>) -> Option<Repair> {
        let innermost = site.failure.brackets.last()?;
        let applies = self.variant == Variant::B
            && site.failure.token.kind == TokenKind::PipeGt
            && innermost.kind == TokenKind::LParen;
        applies.then(|| Repair::insert(Rule::L11.id(), site.previous_end(), ")"))
    }

    /// Rule `L13`.
    fn pipe_notation(&self, site: &Site<'_>) -> Option<Repair> {
        let foreign_pipe = matches!(
            site.failure.token.kind,
            TokenKind::Foreign(
                ForeignKind::Pipe
                    | ForeignKind::PipeSpaceGt
                    | ForeignKind::PipeArrow
                    | ForeignKind::Arrow
            )
        );
        let between_stages = foreign_pipe
            && site.at_stage_level()
            && site
                .previous()
                .is_some_and(|token| ends_operand(&token.kind))
            && self.command_at(site, site.failure.index + 1);
        between_stages
            .then(|| Repair::replace(Rule::L13.id(), site.failure.token.range.clone(), PIPE))
    }

    /// Rule `L14`.
    fn missing_pipe(&self, site: &Site<'_>) -> Option<Repair> {
        let index = site.failure.index;
        let applies = site.failure.token.kind == TokenKind::Ident
            && site.at_stage_level()
            && site
                .previous()
                .is_some_and(|token| ends_operand(&token.kind) && !is_argument_name(token))
            && self.command_at(site, index);
        if !applies {
            return None;
        }
        let (command, position) = site.stage_before(index)?;
        if self
            .catalog
            .can_take_argument(&command, position, ArgumentShape::Word)
        {
            return None;
        }
        let at = site.failure.token.range.start;
        Some(Repair::insert(Rule::L14.id(), at, format!("{PIPE} ")))
    }

    /// Rule `L15`.
    fn commas(&self, site: &Site<'_>) -> Option<Repair> {
        let token = &site.failure.token;
        if self.variant == Variant::B {
            let in_call = site
                .failure
                .brackets
                .last()
                .is_some_and(|open| open.kind == TokenKind::LParen);
            let missing = in_call
                && starts_operand(&token.kind)
                && site.previous().is_some_and(|previous| {
                    ends_operand(&previous.kind) && previous.range.end < token.range.start
                });
            return missing.then(|| Repair::insert(Rule::L15.id(), site.previous_end(), ","));
        }
        let extra = token.kind == TokenKind::Comma
            && token.space_after
            && site.failure.brackets.is_empty()
            && site
                .previous()
                .is_some_and(|token| ends_operand(&token.kind));
        extra.then(|| Repair::delete(Rule::L15.id(), token.range.clone()))
    }

    /// Whether a command name or a habit word starts at token `index`.
    fn command_at(&self, site: &Site<'_>, index: usize) -> bool {
        let Some(first) = site.tokens.get(index) else {
            return false;
        };
        if first.kind != TokenKind::Ident {
            return false;
        }
        let name = site.dotted_name(index);
        self.catalog.is_command(&name) || (self.is_habit)(site.slice(first))
    }
}

/// A parse failure with the tokens and the text it refers to.
struct Site<'a> {
    failure: &'a Failure,
    tokens: &'a [Token],
    text: &'a str,
}

impl Site<'_> {
    fn slice(&self, token: &Token) -> &str {
        self.text.get(token.range.clone()).unwrap_or_default()
    }

    fn previous(&self) -> Option<&Token> {
        self.failure
            .index
            .checked_sub(1)
            .and_then(|index| self.tokens.get(index))
    }

    /// End of the last token before the failure that is not a line break,
    /// or the start of the failing token.
    fn previous_end(&self) -> usize {
        self.tokens
            .get(..self.failure.index)
            .unwrap_or_default()
            .iter()
            .rev()
            .find(|token| !matches!(token.kind, TokenKind::Nl | TokenKind::Eof))
            .map_or(self.failure.token.range.start, |token| token.range.end)
    }

    /// Whether only parentheses are open at the failure, so that it is at the
    /// level of the stages of a pipeline or a sub-pipeline.
    fn at_stage_level(&self) -> bool {
        self.failure
            .brackets
            .iter()
            .all(|open| open.kind == TokenKind::LParen)
    }

    /// Identifiers from token `index` joined by `.` without spaces.
    fn dotted_name(&self, index: usize) -> String {
        let mut name = String::new();
        let mut cursor = index;
        while let Some(token) = self.tokens.get(cursor) {
            if token.kind != TokenKind::Ident {
                break;
            }
            name.push_str(self.slice(token));
            let joined = self.tokens.get(cursor + 1).is_some_and(|dot| {
                dot.kind == TokenKind::Dot && !dot.space_before && !dot.space_after
            });
            if !joined {
                break;
            }
            name.push('.');
            cursor += 2;
        }
        name
    }

    /// Command of the stage that contains token `index` and the number of
    /// its positional arguments before that token.
    fn stage_before(&self, index: usize) -> Option<(String, usize)> {
        let before = self.tokens.get(..index)?;
        let mut depth = 0usize;
        let mut start = 0;
        for (position, token) in before.iter().enumerate().rev() {
            match token.kind {
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => depth += 1,
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                    if depth == 0 {
                        start = position + 1;
                        break;
                    }
                    depth -= 1;
                }
                TokenKind::PipeGt | TokenKind::Nl if depth == 0 => {
                    start = position + 1;
                    break;
                }
                _ => {}
            }
        }
        let is_let = before
            .get(start)
            .is_some_and(|token| token.kind == TokenKind::Let);
        if is_let {
            start += LET_HEAD_TOKENS;
        }
        let command = self.dotted_name(start);
        if command.is_empty() {
            return None;
        }
        let name_tokens = command.matches('.').count() * 2 + 1;
        let positional = self.positional_count(start + name_tokens, index);
        Some((command, positional))
    }

    /// Number of positional arguments that start between tokens `from` and
    /// `to` outside brackets.
    fn positional_count(&self, from: usize, to: usize) -> usize {
        let mut depth = 0usize;
        let mut count = 0;
        for position in from..to {
            let Some(token) = self.tokens.get(position) else {
                break;
            };
            let after_name = position
                .checked_sub(1)
                .and_then(|previous| self.tokens.get(previous))
                .is_some_and(|previous| matches!(previous.kind, TokenKind::Colon | TokenKind::Eq));
            if depth == 0 && starts_operand(&token.kind) && !after_name && !is_argument_name(token)
            {
                count += 1;
            }
            match token.kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        }
        count
    }
}

/// Tokens of `let name =` before the command of the pipeline.
const LET_HEAD_TOKENS: usize = 3;

/// Rule `L10`.
fn close_brackets(site: &Site<'_>) -> Option<Repair> {
    let failure = site.failure;
    if failure.token.kind != TokenKind::Eof || failure.brackets.is_empty() {
        return None;
    }
    let closing: String = failure
        .brackets
        .iter()
        .rev()
        .filter_map(|open| closer(&open.kind))
        .collect();
    Some(Repair::insert(Rule::L10.id(), site.previous_end(), closing))
}

/// Rule `L12`.
fn drop_closing(site: &Site<'_>) -> Option<Repair> {
    let failure = site.failure;
    let token = &failure.token;
    let wanted = closer_kind(&token.kind)?;
    let matched = failure
        .brackets
        .last()
        .is_some_and(|open| open.kind == wanted);
    let at_end = site
        .tokens
        .get(failure.index + 1..)
        .unwrap_or_default()
        .iter()
        .all(|after| {
            closer_kind(&after.kind).is_some()
                || matches!(after.kind, TokenKind::Nl | TokenKind::Eof)
        });
    (!matched && at_end).then(|| Repair::delete(Rule::L12.id(), token.range.clone()))
}

/// Rule `L08`.
fn close_text(site: &Site<'_>) -> Option<Repair> {
    let token = &site.failure.token;
    let TokenKind::Foreign(ForeignKind::UnterminatedText { first_line_end }) = token.kind else {
        return None;
    };
    if follows_text(site) {
        return None;
    }
    let start = token.range.start;
    let line = site.text.get(start..first_line_end)?;
    let (opening, closing) = delimiters(line);
    let content = line.get(opening..).unwrap_or_default();
    if content.contains(PIPE) {
        return None;
    }
    let kept = line.trim_end_matches([' ', '\t', '\r']);
    if kept.len() <= opening {
        return None;
    }
    Some(Repair::insert(Rule::L08.id(), start + kept.len(), closing))
}

/// Rule `L09`.
fn drop_quote(site: &Site<'_>) -> Option<Repair> {
    let token = &site.failure.token;
    let is_unterminated = matches!(
        token.kind,
        TokenKind::Foreign(ForeignKind::UnterminatedText { .. })
    );
    let starts_with_quote = site
        .text
        .get(token.range.clone())
        .is_some_and(|written| written.starts_with(QUOTE));
    (is_unterminated && starts_with_quote && follows_text(site)).then(|| {
        let start = token.range.start;
        Repair::delete(Rule::L09.id(), start..start + QUOTE.len_utf8())
    })
}

/// Whether the failing token starts directly after a closed text.
fn follows_text(site: &Site<'_>) -> bool {
    site.previous().is_some_and(|previous| {
        matches!(previous.kind, TokenKind::Text | TokenKind::RawText)
            && previous.range.end == site.failure.token.range.start
    })
}

/// Length of the opening delimiter of an unterminated text and the closing
/// delimiter that ends it: `"` for text, `"#…` for raw text.
fn delimiters(line: &str) -> (usize, String) {
    let Some(after_r) = line.strip_prefix('r') else {
        return (QUOTE.len_utf8(), QUOTE.to_string());
    };
    let hashes = after_r.len() - after_r.trim_start_matches('#').len();
    let closing = format!("{QUOTE}{}", "#".repeat(hashes));
    (1 + hashes + QUOTE.len_utf8(), closing)
}

fn closer(open: &TokenKind) -> Option<char> {
    match open {
        TokenKind::LParen => Some(')'),
        TokenKind::LBracket => Some(']'),
        TokenKind::LBrace => Some('}'),
        _ => None,
    }
}

/// Opening bracket matched by a closing bracket.
fn closer_kind(close: &TokenKind) -> Option<TokenKind> {
    match close {
        TokenKind::RParen => Some(TokenKind::LParen),
        TokenKind::RBracket => Some(TokenKind::LBracket),
        TokenKind::RBrace => Some(TokenKind::LBrace),
        _ => None,
    }
}

/// Whether the token is the name of a named argument, `glob:` or `glob=`.
fn is_argument_name(token: &Token) -> bool {
    token.kind == TokenKind::Ident
        && matches!(
            token.glued,
            nurt_syntax::lex::Glued::Colon | nurt_syntax::lex::Glued::Equals
        )
}

/// Whether a token of the kind completes an operand.
fn ends_operand(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Ident
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Null
            | TokenKind::Text
            | TokenKind::RawText
            | TokenKind::Regex
            | TokenKind::Int
            | TokenKind::Number
            | TokenKind::Size
            | TokenKind::Duration
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::RBrace
    )
}

/// Whether a token of the kind starts an operand.
fn starts_operand(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Ident
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Null
            | TokenKind::Text
            | TokenKind::RawText
            | TokenKind::Regex
            | TokenKind::Int
            | TokenKind::Number
            | TokenKind::Size
            | TokenKind::Duration
            | TokenKind::LBracket
            | TokenKind::LBrace
            | TokenKind::At
    )
}

#[cfg(test)]
mod tests {
    use nurt_syntax::parse::{ParseOptions, Repaired, parse_tolerant};

    use super::*;

    const COMMANDS: [&str; 8] = [
        "find", "search", "count", "verify", "run", "replace", "remove", "list",
    ];

    struct Commands;

    impl CatalogView for Commands {
        fn is_command(&self, name: &str) -> bool {
            COMMANDS.contains(&name)
        }
        fn command_names(&self) -> &[&str] {
            &COMMANDS
        }
        fn is_mutating(&self, name: &str) -> bool {
            matches!(name, "run" | "replace" | "remove")
        }
        fn can_take_argument(&self, _: &str, _: usize, _: ArgumentShape) -> bool {
            false
        }
    }

    fn repair(variant: Variant, text: &str) -> Option<Repaired> {
        let policy = LexicalRepairs::new(variant, &Commands, &|word| word == "grep");
        parse_tolerant(text, variant, &ParseOptions::default(), &policy).ok()
    }

    fn rules(repaired: &Repaired) -> Vec<&'static str> {
        repaired
            .repairs
            .iter()
            .map(|applied| applied.rule)
            .collect()
    }

    #[test]
    fn closes_brackets_open_at_the_end_of_the_program() {
        let repaired = repair(Variant::A, "verify (run [\"go\", \"build\"] cwd: \"/a\"");
        let repaired = repaired.expect("repaired");
        assert_eq!(
            repaired.source,
            "verify (run [\"go\", \"build\"] cwd: \"/a\")"
        );
        assert_eq!(rules(&repaired), ["L10"]);
        let nested = repair(Variant::A, "verify (run [\"go\", \"build\"").expect("repaired");
        assert_eq!(nested.source, "verify (run [\"go\", \"build\"])");
    }

    #[test]
    fn closes_a_call_before_a_pipe_in_call_notation() {
        let repaired = repair(Variant::B, "find(\"/a\" |> count()").expect("repaired");
        assert_eq!(repaired.source, "find(\"/a\") |> count()");
        assert_eq!(rules(&repaired), ["L11"]);
    }

    #[test]
    fn removes_an_extra_closing_bracket_at_the_end() {
        let repaired = repair(Variant::B, "find(\"/a\") |> count())").expect("repaired");
        assert_eq!(repaired.source, "find(\"/a\") |> count()");
        assert_eq!(rules(&repaired), ["L12"]);
    }

    #[test]
    fn removes_commas_between_arguments_in_word_notation() {
        let repaired = repair(Variant::A, "find \"/a\", glob: \"*.go\"").expect("repaired");
        assert_eq!(repaired.source, "find \"/a\" glob: \"*.go\"");
        assert_eq!(rules(&repaired), ["L15"]);
    }

    #[test]
    fn inserts_a_comma_between_call_arguments() {
        let repaired = repair(Variant::B, "find(\"/a\" glob: \"*.go\")").expect("repaired");
        assert_eq!(repaired.source, "find(\"/a\", glob: \"*.go\")");
        assert_eq!(rules(&repaired), ["L15"]);
    }

    #[test]
    fn closes_a_text_at_the_end_of_its_line() {
        let repaired = repair(Variant::A, "replace \"x\" \"newName(").expect("repaired");
        assert_eq!(repaired.source, "replace \"x\" \"newName(\"");
        assert_eq!(rules(&repaired), ["L08"]);
        let raw = repair(Variant::A, "replace \"x\" r#\"a\"b").expect("repaired");
        assert_eq!(raw.source, "replace \"x\" r#\"a\"b\"#");
    }

    #[test]
    fn keeps_a_text_open_when_a_pipe_follows_on_its_line() {
        assert!(repair(Variant::A, "find \"/a |> count").is_none());
    }

    #[test]
    fn removes_an_extra_quote_after_a_text() {
        let repaired = repair(Variant::A, "find \"/a\" glob: \"*.go\"\"").expect("repaired");
        assert_eq!(repaired.source, "find \"/a\" glob: \"*.go\"");
        assert_eq!(rules(&repaired), ["L09"]);
    }

    #[test]
    fn inserts_a_pipe_before_a_command_in_call_notation() {
        let repaired = repair(Variant::B, "find(\"/a\") search(/x/)").expect("repaired");
        assert_eq!(repaired.source, "find(\"/a\") |> search(/x/)");
        assert_eq!(rules(&repaired), ["L14"]);
    }

    #[test]
    fn proposes_nothing_for_a_valid_program_or_foreign_words() {
        let repaired = repair(Variant::A, "find \"/a\" |> count").expect("parsed");
        assert!(repaired.repairs.is_empty());
        assert!(repair(Variant::A, "find \"/a\" ; count").is_none());
    }

    #[test]
    fn computes_delimiters_of_unterminated_texts() {
        assert_eq!(delimiters("\"abc"), (1, "\"".to_owned()));
        assert_eq!(delimiters("r##\"abc"), (4, "\"##".to_owned()));
        assert_eq!(delimiters("r\"abc"), (2, "\"".to_owned()));
    }
}
