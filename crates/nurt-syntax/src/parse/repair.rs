// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repair hooks of the tolerant parser.
//!
//! When parsing fails, the parser describes the failure in a [`Failure`]
//! and asks a [`RepairPolicy`] for [`Repair`]s: edits of the program text at
//! the place of the failure, each tagged with the correction rule that
//! proposes it. The parser applies them in a fixed order and parses the
//! edited text again. The parser itself knows no correction rule; the
//! policies live in the correction crate.

use std::collections::HashMap;
use std::ops::Range;

use nurt_contracts::ast::{AstDocument, strip_for_hash};
use nurt_contracts::error::At;

use super::{ParseOptions, Parser, Variant};
use crate::error::SyntaxError;
use crate::lex::Token;

/// Order in which repairs of the rules are tried; repairs of other rules
/// follow in the order proposed.
pub const REPAIR_ORDER: [&str; 8] = ["L10", "L11", "L12", "L13", "L14", "L15", "L08", "L09"];

/// Number of proposals tried at one failure.
pub const MAX_PROPOSALS: usize = 3;

/// Maximum number of alternative trees in [`Repaired::alternatives`].
pub const MAX_ALTERNATIVES: usize = 3;

/// Maximum number of repairs applied to one program in total, which bounds the
/// depth of the search and the stack that it uses.
pub const MAX_REPAIRS_PER_PROGRAM: usize = 256;

/// Edit of the program text proposed by a correction rule: `range` is
/// replaced by `replacement`. An empty range inserts, an empty replacement
/// deletes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Repair {
    /// Identifier of the correction rule, for example `"L10"`.
    pub rule: &'static str,
    /// Byte range of the edited fragment of the text.
    pub range: Range<usize>,
    /// Text that replaces the fragment.
    pub replacement: String,
}

impl Repair {
    /// Inserts `text` at the byte offset `at`.
    #[must_use]
    pub fn insert(rule: &'static str, at: usize, text: impl Into<String>) -> Self {
        Self {
            rule,
            range: at..at,
            replacement: text.into(),
        }
    }

    /// Deletes the fragment `range`.
    #[must_use]
    pub fn delete(rule: &'static str, range: Range<usize>) -> Self {
        Self {
            rule,
            range,
            replacement: String::new(),
        }
    }

    /// Replaces the fragment `range` with `text`.
    #[must_use]
    pub fn replace(rule: &'static str, range: Range<usize>, text: impl Into<String>) -> Self {
        Self {
            rule,
            range,
            replacement: text.into(),
        }
    }
}

/// Place and kind of a parse failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Token at which parsing stopped; a foreign token of the tolerant
    /// lexer when the text contains one.
    pub token: Token,
    /// Index of [`Failure::token`] in the token list given to
    /// [`RepairPolicy::propose`].
    pub index: usize,
    /// Kinds of tokens or constructs that would be valid at the failure.
    pub expected: Vec<String>,
    /// Opening brackets not yet closed at the failure, outermost first.
    pub brackets: Vec<Token>,
    /// Index of the statement that failed, from 0.
    pub statement: usize,
    /// The error of the failed parse.
    pub error: SyntaxError,
}

/// Source of repairs for the tolerant parser.
///
/// # Examples
///
/// ```
/// use nurt_syntax::lex::{Token, TokenKind};
/// use nurt_syntax::parse::{
///     Failure, ParseOptions, Repair, RepairPolicy, Variant, parse_tolerant,
/// };
///
/// /// Closes brackets left open at the end of the program.
/// struct CloseBrackets;
///
/// impl RepairPolicy for CloseBrackets {
///     fn propose(&self, failure: &Failure, _tokens: &[Token], text: &str) -> Vec<Repair> {
///         if failure.token.kind != TokenKind::Eof || failure.brackets.is_empty() {
///             return Vec::new();
///         }
///         let closing: String = failure
///             .brackets
///             .iter()
///             .rev()
///             .map(|open| match open.kind {
///                 TokenKind::LParen => ')',
///                 TokenKind::LBracket => ']',
///                 _ => '}',
///             })
///             .collect();
///         vec![Repair::insert("L10", text.len(), closing)]
///     }
/// }
///
/// let text = "verify (run [\"go\", \"build\"] cwd: \"/a\"";
/// let repaired = parse_tolerant(text, Variant::A, &ParseOptions::default(), &CloseBrackets)?;
/// assert_eq!(repaired.source, format!("{text})"));
/// assert_eq!(repaired.repairs[0].rule, "L10");
/// # Ok::<(), nurt_syntax::error::SyntaxError>(())
/// ```
pub trait RepairPolicy {
    /// Proposes edits of `text` that may remove the failure; `tokens` is the
    /// token stream of `text` read by the tolerant lexer.
    fn propose(&self, failure: &Failure, tokens: &[Token], text: &str) -> Vec<Repair>;
}

/// Policy that proposes no repair: tolerant parsing then equals strict
/// parsing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NoRepair;

impl RepairPolicy for NoRepair {
    fn propose(&self, _failure: &Failure, _tokens: &[Token], _text: &str) -> Vec<Repair> {
        Vec::new()
    }
}

/// A repair applied to the program text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AppliedRepair {
    /// Identifier of the correction rule.
    pub rule: &'static str,
    /// Position of the edited fragment in the original program text.
    pub at: At,
    /// The fragment before the repair.
    pub from: String,
    /// The fragment after the repair.
    pub to: String,
}

/// Result of [`parse_tolerant`](super::parse_tolerant).
#[derive(Debug, Clone, PartialEq)]
pub struct Repaired {
    /// The tree; its spans refer to [`Repaired::source`].
    pub document: AstDocument,
    /// The program text after the repairs, edited minimally.
    pub source: String,
    /// Repairs in the order applied; empty when the text parses strictly.
    pub repairs: Vec<AppliedRepair>,
    /// Other repaired readings that yield different trees, best first; each
    /// has no alternatives of its own.
    pub alternatives: Vec<Repaired>,
}

pub(crate) fn parse_with_repairs(
    text: &str,
    variant: Variant,
    options: &ParseOptions,
    policy: &dyn RepairPolicy,
    keep_escapes: bool,
) -> Result<Repaired, SyntaxError> {
    let strict = match super::parse_with_escapes(text, variant, options, keep_escapes) {
        Ok(document) => {
            return Ok(Repaired {
                document,
                source: text.to_owned(),
                repairs: Vec::new(),
                alternatives: Vec::new(),
            });
        }
        Err(error) => error,
    };
    if options.max_repairs == 0 {
        return Err(strict);
    }
    let mut search = Search {
        original: text,
        variant,
        options,
        policy,
        keep_escapes,
        attempts: HashMap::new(),
        found: Vec::new(),
        keys: Vec::new(),
    };
    search.explore(text.to_owned(), &Path::default());
    let mut found = search.found.into_iter();
    match found.next() {
        Some(mut best) => {
            best.alternatives = found.collect();
            Ok(best)
        }
        None => Err(strict),
    }
}

/// Repairs applied on the way to a text, with the edits needed to map
/// positions back to the original text.
#[derive(Debug, Clone, Default)]
struct Path {
    repairs: Vec<AppliedRepair>,
    edits: Vec<Edit>,
    statements: Vec<usize>,
}

/// An applied edit: at byte `start`, `removed` bytes became `inserted`
/// bytes.
#[derive(Debug, Clone, Copy)]
struct Edit {
    start: usize,
    removed: usize,
    inserted: usize,
}

struct Search<'t, 'o> {
    original: &'t str,
    variant: Variant,
    options: &'o ParseOptions,
    policy: &'o dyn RepairPolicy,
    keep_escapes: bool,
    attempts: HashMap<usize, usize>,
    found: Vec<Repaired>,
    keys: Vec<serde_json::Value>,
}

impl Search<'_, '_> {
    fn explore(&mut self, text: String, path: &Path) {
        let (failure, tokens) = match self.attempt(&text) {
            Attempt::Parsed(document) => {
                self.record(document, text, path);
                return;
            }
            Attempt::Failed(failure, tokens) => (failure, tokens),
            Attempt::Unrepairable => return,
        };
        let used = path
            .statements
            .iter()
            .filter(|&&statement| statement == failure.statement)
            .count();
        if used >= usize::from(self.options.max_repairs)
            || path.repairs.len() >= MAX_REPAIRS_PER_PROGRAM
        {
            return;
        }
        let mut proposals = self.policy.propose(&failure, &tokens, &text);
        proposals.sort_by_key(|repair| rank(repair.rule));
        proposals.truncate(MAX_PROPOSALS);
        for repair in proposals {
            if self.is_complete() || !self.spend_attempt(failure.statement) {
                return;
            }
            let Some(edited) = apply(&text, &repair) else {
                continue;
            };
            let next = self.extend(path, &text, &repair, failure.statement);
            self.explore(edited, &next);
        }
    }

    /// Parses `text` strictly; on failure parses it again with the tolerant
    /// lexer to describe the failure. A text accepted only by the tolerant
    /// lexer yields no tree and no failure, so every tree returned parses
    /// strictly from its source.
    fn attempt(&self, text: &str) -> Attempt {
        if let Ok(document) =
            super::parse_with_escapes(text, self.variant, self.options, self.keep_escapes)
        {
            return Attempt::Parsed(document);
        }
        let Ok(mut parser) =
            Parser::with_escapes(text, self.variant, self.options, true, self.keep_escapes)
        else {
            return Attempt::Unrepairable;
        };
        match super::parse_variant(&mut parser) {
            Ok(_) => Attempt::Unrepairable,
            Err(error) => {
                let token = parser.current().clone();
                let trace = parser.into_trace();
                let failure = Failure {
                    token,
                    index: trace.index,
                    expected: error.expected.clone(),
                    brackets: trace.brackets,
                    statement: trace.statement,
                    error,
                };
                Attempt::Failed(failure, trace.tokens)
            }
        }
    }

    fn is_complete(&self) -> bool {
        self.found.len() > MAX_ALTERNATIVES
    }

    /// Counts an attempt for the statement; returns `false` when the
    /// statement has used its budget of `MAX_PROPOSALS ^ max_repairs`
    /// attempts.
    fn spend_attempt(&mut self, statement: usize) -> bool {
        let budget = MAX_PROPOSALS.saturating_pow(u32::from(self.options.max_repairs));
        let spent = self.attempts.entry(statement).or_insert(0);
        if *spent >= budget {
            return false;
        }
        *spent += 1;
        true
    }

    fn extend(&self, path: &Path, text: &str, repair: &Repair, statement: usize) -> Path {
        let start = path
            .edits
            .iter()
            .rev()
            .fold(repair.range.start, |offset, edit| {
                original_offset(offset, edit)
            });
        let from = text
            .get(repair.range.clone())
            .unwrap_or_default()
            .to_owned();
        let (line, column) = line_and_column(self.original, start);
        let length = u32::try_from(from.chars().count()).unwrap_or(u32::MAX);
        let mut next = path.clone();
        next.repairs.push(AppliedRepair {
            rule: repair.rule,
            at: At::new(line, column, length),
            from,
            to: repair.replacement.clone(),
        });
        next.edits.push(Edit {
            start: repair.range.start,
            removed: repair.range.len(),
            inserted: repair.replacement.len(),
        });
        next.statements.push(statement);
        next
    }

    fn record(&mut self, document: AstDocument, source: String, path: &Path) {
        let key = serde_json::to_value(&document)
            .map(|value| strip_for_hash(&value))
            .unwrap_or(serde_json::Value::Null);
        if self.keys.contains(&key) {
            return;
        }
        self.keys.push(key);
        self.found.push(Repaired {
            document,
            source,
            repairs: path.repairs.clone(),
            alternatives: Vec::new(),
        });
    }
}

enum Attempt {
    Parsed(AstDocument),
    Failed(Failure, Vec<Token>),
    Unrepairable,
}

fn rank(rule: &str) -> usize {
    REPAIR_ORDER
        .iter()
        .position(|&ordered| ordered == rule)
        .unwrap_or(REPAIR_ORDER.len())
}

/// Applies a repair; `None` when the range is outside the text, splits a
/// character or the repair changes nothing.
fn apply(text: &str, repair: &Repair) -> Option<String> {
    let Range { start, end } = repair.range;
    let valid = start <= end && text.is_char_boundary(start) && text.is_char_boundary(end);
    if !valid || text.get(start..end)? == repair.replacement {
        return None;
    }
    Some(super::error::edit(text, &repair.range, &repair.replacement))
}

/// Maps a byte offset of the text after `edit` to the text before it; an
/// offset inside inserted text maps to the start of the edit.
fn original_offset(offset: usize, edit: &Edit) -> usize {
    if offset >= edit.start + edit.inserted {
        offset - edit.inserted + edit.removed
    } else if offset > edit.start {
        edit.start
    } else {
        offset
    }
}

/// Line and column, both from 1, of a byte offset; columns count Unicode
/// scalar values.
fn line_and_column(text: &str, offset: usize) -> (u32, u32) {
    let before = text.get(..offset).unwrap_or(text);
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let line = before.matches('\n').count() + 1;
    let column = before.get(line_start..).unwrap_or_default().chars().count() + 1;
    (
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(column).unwrap_or(u32::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_known_rules_before_unknown_ones() {
        assert!(rank("L10") < rank("L13"));
        assert!(rank("L15") < rank("L08"));
        assert!(rank("L09") < rank("X01"));
    }

    #[test]
    fn rejects_repairs_outside_the_text_or_inside_a_character() {
        assert_eq!(apply("abc", &Repair::insert("L10", 9, ")")), None);
        assert_eq!(apply("é", &Repair::insert("L10", 1, ")")), None);
        assert_eq!(apply("abc", &Repair::replace("L13", 0..1, "a")), None);
        assert_eq!(
            apply("abc", &Repair::delete("L12", 2..3)),
            Some("ab".to_owned())
        );
    }

    #[test]
    fn maps_offsets_back_through_an_insertion() {
        let edit = Edit {
            start: 2,
            removed: 0,
            inserted: 3,
        };
        assert_eq!(original_offset(1, &edit), 1);
        assert_eq!(original_offset(3, &edit), 2);
        assert_eq!(original_offset(6, &edit), 3);
    }

    #[test]
    fn computes_line_and_column_in_scalar_values() {
        assert_eq!(line_and_column("ab\ncé d", 7), (2, 4));
        assert_eq!(line_and_column("abc", 0), (1, 1));
    }
}
