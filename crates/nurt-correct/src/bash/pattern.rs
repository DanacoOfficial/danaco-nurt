// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Search patterns of `grep` and `sed`, and the translation of `grep`.
//!
//! A basic regular expression (the default of `grep` and `sed`) is
//! translated into the extended dialect of the language: `\(`, `\)`, `\{`,
//! `\}`, `\|`, `\+`, `\?` lose the backslash and the characters `(`, `)`,
//! `{`, `}`, `|`, `+`, `?` gain one. A pattern that contains those characters
//! reads differently in the two dialects, so its translation is probable:
//! the reading that matches literal text comes first, the translated regular
//! expression next and the extended reading last when it is valid.

use nurt_contracts::ast::Expr;
use nurt_contracts::error::{Error, ErrorCode};

use super::engine::{self, Parsed};
use super::lex::Word;
use super::session::{Session, Variants, is_glob};
use super::syntax::{Located, Simple};
use super::table::CommandEntry;
use super::template::Bound;

/// Dialect of a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    Basic,
    Extended,
    Fixed,
    Perl,
}

/// One interpretation of a pattern with the key of its explanation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reading {
    pub(crate) expr: Expr,
    pub(crate) why: &'static str,
}

/// Interpretations of a pattern, the likeliest first.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Readings {
    pub(crate) items: Vec<Reading>,
    pub(crate) probable: bool,
}

/// Characters special in an extended expression and literal in a basic one.
const EXTENDED_ONLY: &[char] = &['(', ')', '{', '}', '|', '+', '?'];

/// Characters special in both dialects.
const SHARED_SPECIAL: &[char] = &['.', '[', '*', '^', '$'];

pub(crate) fn readings(pattern: &str, dialect: Dialect) -> Result<Readings, String> {
    let regex = |v: String| Expr::Regex {
        v,
        flags: String::new(),
        span: None,
    };
    match dialect {
        Dialect::Fixed => Ok(certain(
            Expr::Text {
                v: pattern.to_owned(),
                span: None,
            },
            "pattern_literal",
        )),
        Dialect::Perl => Ok(certain(regex(pattern.to_owned()), "pattern_extended")),
        Dialect::Extended => {
            validate(pattern)?;
            Ok(certain(regex(pattern.to_owned()), "pattern_extended"))
        }
        Dialect::Basic => basic(pattern),
    }
}

fn certain(expr: Expr, why: &'static str) -> Readings {
    Readings {
        items: vec![Reading { expr, why }],
        probable: false,
    }
}

fn basic(pattern: &str) -> Result<Readings, String> {
    let (shared, extended_only) = scan(pattern);
    let converted = basic_to_extended(pattern);
    validate(&converted)?;
    let regex = |v: String| Expr::Regex {
        v,
        flags: String::new(),
        span: None,
    };
    if !extended_only {
        return Ok(certain(
            regex(converted),
            if shared {
                "pattern_basic"
            } else {
                "pattern_literal"
            },
        ));
    }
    let mut items = Vec::new();
    if !shared {
        items.push(Reading {
            expr: Expr::Text {
                v: pattern.to_owned(),
                span: None,
            },
            why: "pattern_literal",
        });
    }
    items.push(Reading {
        expr: regex(converted.clone()),
        why: "pattern_basic",
    });
    if pattern != converted && validate(pattern).is_ok() {
        items.push(Reading {
            expr: regex(pattern.to_owned()),
            why: "pattern_extended",
        });
    }
    Ok(Readings {
        items,
        probable: true,
    })
}

/// Whether the pattern uses characters special in both dialects (or an
/// escape), and whether it contains characters special only in the
/// extended dialect.
fn scan(pattern: &str) -> (bool, bool) {
    let mut shared = false;
    let mut extended_only = false;
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            shared = true;
            chars.next();
        } else if SHARED_SPECIAL.contains(&c) {
            shared = true;
        } else if EXTENDED_ONLY.contains(&c) {
            extended_only = true;
        }
    }
    (shared, extended_only)
}

fn validate(pattern: &str) -> Result<(), String> {
    regex::Regex::new(pattern)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Translates a basic regular expression into the extended dialect.
pub(crate) fn basic_to_extended(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    let mut at_start = true;
    while let Some(&c) = chars.get(index) {
        index += 1;
        let start = at_start;
        at_start = false;
        match c {
            '\\' => {
                let Some(&next) = chars.get(index) else {
                    out.push_str("\\\\");
                    continue;
                };
                index += 1;
                match next {
                    '(' | '|' => {
                        out.push(next);
                        at_start = true;
                    }
                    ')' | '{' | '}' | '+' | '?' => out.push(next),
                    '<' | '>' => out.push_str("\\b"),
                    _ => {
                        out.push('\\');
                        out.push(next);
                    }
                }
            }
            '[' => index = bracket(&chars, index, &mut out),
            '*' if start => out.push_str("\\*"),
            '^' if !start => out.push_str("\\^"),
            '$' if !at_end(&chars, index) => out.push_str("\\$"),
            c if EXTENDED_ONLY.contains(&c) => {
                out.push('\\');
                out.push(c);
            }
            '^' => {
                out.push(c);
                at_start = true;
            }
            c => out.push(c),
        }
    }
    out
}

fn at_end(chars: &[char], index: usize) -> bool {
    match chars.get(index) {
        None => true,
        Some('\\') => matches!(chars.get(index + 1), Some(')' | '|')),
        Some(_) => false,
    }
}

/// Copies a bracket expression, which has the same syntax in both dialects.
fn bracket(chars: &[char], mut index: usize, out: &mut String) -> usize {
    out.push('[');
    let mut first = true;
    while let Some(&c) = chars.get(index) {
        index += 1;
        out.push(c);
        if c == '^' && first {
            continue;
        }
        if c == '[' && chars.get(index) == Some(&':') {
            while let Some(&inner) = chars.get(index) {
                index += 1;
                out.push(inner);
                if inner == ']' {
                    break;
                }
            }
        } else if c == ']' && !first {
            break;
        }
        first = false;
    }
    index
}

/// The reading as a regular expression for a `filter` condition; literal
/// text is escaped.
pub(crate) fn as_regex(expr: &Expr, ignore_case: bool) -> Expr {
    let flags = if ignore_case {
        "i".to_owned()
    } else {
        String::new()
    };
    match expr {
        Expr::Text { v, span } => Expr::Regex {
            v: regex::escape(v),
            flags,
            span: *span,
        },
        Expr::Regex { v, span, .. } => Expr::Regex {
            v: v.clone(),
            flags,
            span: *span,
        },
        other => other.clone(),
    }
}

/// Translates `grep`.
pub(crate) fn grep(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    words: &[Located<Word>],
    input: Option<&str>,
) -> Result<Variants, Error> {
    let mut parsed = engine::parse_options(session, entry, words)?;
    let pattern_word = pattern_word(session, entry, simple, &mut parsed)?;
    let Bound::Word { text, span, .. } = session.operand(&pattern_word)? else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "variable_in_text",
            pattern_word.span,
            &[("name", &pattern_word.value.raw), ("lower", "x")],
        ));
    };
    let dialect = dialect(entry, &parsed);
    let found = readings(&text, dialect).map_err(|reason| {
        session.error(
            ErrorCode::Uncorrectable,
            "invalid_regex",
            span,
            &[("pattern", &text), ("reason", &reason)],
        )
    })?;
    match input {
        Some(_) if !parsed.operands.is_empty() => {
            return Err(session.error(
                ErrorCode::Uncorrectable,
                "unsupported_stage",
                simple.span,
                &[("command", "grep")],
            ));
        }
        None if parsed.operands.is_empty() => {
            if !parsed.has(entry, "r") {
                return Err(session.error(
                    ErrorCode::Uncorrectable,
                    "standard_input",
                    simple.span,
                    &[("command", "grep")],
                ));
            }
            parsed.operands.push(engine::located(".", simple.span));
        }
        _ => {}
    }
    if parsed.has(entry, "r")
        && let Some(word) = parsed.operands.iter().find(|word| is_glob(&word.value))
    {
        return Err(session.pattern_error(word));
    }
    let mut bindings = engine::bind_operands(session, &parsed.operands, true)?;
    for include in parsed
        .given
        .iter()
        .filter(|given| entry.options[given.index].id == "include")
    {
        if let Some(value) = &include.value {
            let glob = format!("**/{}", value.value.literal().unwrap_or_default());
            bindings.set(
                "include",
                vec![Bound::Word {
                    text: glob,
                    span: value.span,
                    bare: false,
                }],
            );
        }
    }
    let ignore_case = parsed.has(entry, "i");
    parsed.probable = found.probable;
    let mut pieces = Vec::new();
    for reading in &found.items {
        let mut local = bindings.clone();
        let mut expr = reading.expr.clone();
        set_span(&mut expr, span);
        local.set("pattern", vec![Bound::Node(expr.clone())]);
        local.set("needle", vec![Bound::Node(expr.clone())]);
        local.set("regex", vec![Bound::Node(as_regex(&expr, ignore_case))]);
        let built = engine::build(session, entry, simple, &parsed, &local, input)?;
        pieces.extend(built.0.into_iter().map(|mut piece| {
            piece.why = Some(reading.why.to_owned());
            piece
        }));
    }
    if found.probable
        && let Some(first) = found.items.first()
    {
        session.assume(&entry.rule, first.why);
    }
    Ok(Variants(pieces))
}

fn pattern_word(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    parsed: &mut Parsed,
) -> Result<Located<Word>, Error> {
    let explicit: Vec<Located<Word>> = parsed
        .given
        .iter()
        .filter(|given| entry.options[given.index].id == "e")
        .filter_map(|given| given.value.clone())
        .collect();
    match explicit.as_slice() {
        [one] => Ok(one.clone()),
        [] if !parsed.operands.is_empty() => Ok(parsed.operands.remove(0)),
        [] => Err(session.error(
            ErrorCode::Uncorrectable,
            "missing_operand",
            simple.span,
            &[("slot", "pattern"), ("command", "grep")],
        )),
        _ => Err(session.error(
            ErrorCode::Uncorrectable,
            "several_patterns",
            simple.span,
            &[],
        )),
    }
}

fn dialect(entry: &CommandEntry, parsed: &Parsed) -> Dialect {
    if parsed.has(entry, "F") {
        Dialect::Fixed
    } else if parsed.has(entry, "E") {
        Dialect::Extended
    } else if parsed.has(entry, "P") {
        Dialect::Perl
    } else {
        Dialect::Basic
    }
}

pub(crate) fn set_span(expr: &mut Expr, value: nurt_contracts::ast::Span) {
    if let Expr::Text { span, .. } | Expr::Regex { span, .. } = expr {
        *span = Some(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(readings: &Readings) -> Vec<String> {
        readings
            .items
            .iter()
            .map(|reading| match &reading.expr {
                Expr::Text { v, .. } => format!("text {v}"),
                Expr::Regex { v, .. } => format!("regex {v}"),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn basic_expressions_translate_to_extended() {
        assert_eq!(basic_to_extended(r"\(a\|b\)\{2\}"), "(a|b){2}");
        assert_eq!(basic_to_extended("a(b)|c+?"), r"a\(b\)\|c\+\?");
        assert_eq!(basic_to_extended("*a"), r"\*a");
        assert_eq!(basic_to_extended("a^b$c$"), r"a\^b\$c$");
        assert_eq!(basic_to_extended(r"\<word\>"), r"\bword\b");
        assert_eq!(basic_to_extended("[]a(]x"), "[]a(]x");
        assert_eq!(basic_to_extended("[[:digit:]]+"), r"[[:digit:]]\+");
        assert_eq!(basic_to_extended("[^)]"), "[^)]");
        assert_eq!(basic_to_extended("a\\"), "a\\\\");
        assert_eq!(basic_to_extended(r"^\(x\)*$"), r"^(x)*$");
        assert_eq!(basic_to_extended(r"\(a$\)"), r"(a$)");
        assert_eq!(basic_to_extended(r"a$\.b"), r"a\$\.b");
    }

    #[test]
    fn plain_text_is_certain() {
        let found = readings("oldName", Dialect::Basic).unwrap();
        assert!(!found.probable);
        assert_eq!(texts(&found), vec!["regex oldName"]);
    }

    #[test]
    fn literal_parenthesis_is_probable_with_literal_reading_first() {
        let found = readings("oldName(", Dialect::Basic).unwrap();
        assert!(found.probable);
        assert_eq!(texts(&found), vec!["text oldName(", r"regex oldName\("]);
    }

    #[test]
    fn valid_extended_reading_is_offered() {
        let found = readings("a|b", Dialect::Basic).unwrap();
        assert_eq!(texts(&found), vec!["text a|b", r"regex a\|b", "regex a|b"]);
    }

    #[test]
    fn shared_specials_only_are_certain() {
        let found = readings("^foo.*bar$", Dialect::Basic).unwrap();
        assert!(!found.probable);
        assert_eq!(texts(&found), vec!["regex ^foo.*bar$"]);
    }

    #[test]
    fn mixed_specials_have_no_literal_reading() {
        let found = readings("a.(b", Dialect::Basic).unwrap();
        assert_eq!(texts(&found), vec![r"regex a.\(b"]);
        assert!(found.probable);
    }

    #[test]
    fn other_dialects() {
        assert_eq!(
            texts(&readings("a(", Dialect::Fixed).unwrap()),
            vec!["text a("]
        );
        assert_eq!(
            texts(&readings("a|b", Dialect::Extended).unwrap()),
            vec!["regex a|b"]
        );
        assert!(readings("a(", Dialect::Extended).is_err());
        assert_eq!(
            texts(&readings("(?<=a)b", Dialect::Perl).unwrap()),
            vec!["regex (?<=a)b"]
        );
        assert!(readings(r"\(a\)\1", Dialect::Basic).is_err());
    }

    #[test]
    fn regex_form_escapes_text() {
        let text = Expr::Text {
            v: "a.b".to_owned(),
            span: None,
        };
        assert_eq!(
            as_regex(&text, true),
            Expr::Regex {
                v: r"a\.b".to_owned(),
                flags: "i".to_owned(),
                span: None
            }
        );
        let int = Expr::Int { v: 1, span: None };
        assert_eq!(as_regex(&int, false), int);
    }
}
