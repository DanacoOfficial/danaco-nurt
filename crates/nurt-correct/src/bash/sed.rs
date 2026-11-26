// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interpreter of `sed` scripts: one substitution `s/A/B/flags` in place,
//! and line addresses with `p` (with `-n`) or `d` (in place). Other scripts
//! are reported as not translatable.

use nurt_contracts::ast::Expr;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::CorrectionClass;

use super::engine::{self, value_template};
use super::lex::Word;
use super::pattern::{self, Dialect, Readings};
use super::session::{Piece, Session, Variants, piece_text};
use super::syntax::{Located, Simple};
use super::table::CommandEntry;
use super::template::{Bindings, Bound};

/// Capture groups that the anchoring of a pattern to the first match of a
/// line puts before the groups of the pattern: the text before the match and
/// the match.
const ANCHOR_GROUPS: u32 = 2;

/// Line address of a `p` or `d` command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Address {
    Line(i64),
    Range(i64, Option<i64>),
    Last,
    Match(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Script {
    Substitute {
        pattern: String,
        replacement: String,
        global: bool,
        occurrence: Option<i64>,
        ignore_case: bool,
    },
    Print(Address),
    Delete(Address),
}

pub(crate) fn translate(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    words: &[Located<Word>],
    input: Option<&str>,
) -> Result<Variants, Error> {
    if input.is_some() {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_stage",
            simple.span,
            &[("command", "sed")],
        ));
    }
    let mut parsed = engine::parse_options(session, entry, words)?;
    let script_word = match parsed.value_of(entry, "e") {
        Some(word) => word.clone(),
        None if !parsed.operands.is_empty() => parsed.operands.remove(0),
        None => {
            return Err(session.error(
                ErrorCode::Uncorrectable,
                "missing_operand",
                simple.span,
                &[("slot", "script"), ("command", "sed")],
            ));
        }
    };
    let Some(text) = script_word.value.literal() else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "variable_in_text",
            script_word.span,
            &[("name", &script_word.value.raw), ("lower", "x")],
        ));
    };
    if parsed.operands.is_empty() {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "standard_input",
            simple.span,
            &[("command", "sed")],
        ));
    }
    let unsupported = || {
        session
            .error(
                ErrorCode::Uncorrectable,
                "unsupported_sed_script",
                script_word.span,
                &[("script", &text)],
            )
            .with_help(format!("help {}", entry.help))
    };
    let script = parse_script(&text).ok_or_else(unsupported)?;
    let expressions = parsed
        .given
        .iter()
        .filter(|given| entry.options[given.index].id == "e")
        .count();
    if expressions > 1 {
        return Err(unsupported());
    }
    let in_place = parsed.has(entry, "i");
    let quiet = parsed.has(entry, "n");
    let dialect = if parsed.has(entry, "E") {
        Dialect::Extended
    } else {
        Dialect::Basic
    };
    let readings = |pattern: &str| {
        pattern::readings(pattern, dialect).map_err(|reason| {
            session.error(
                ErrorCode::Uncorrectable,
                "invalid_regex",
                script_word.span,
                &[("pattern", pattern), ("reason", &reason)],
            )
        })
    };
    let mut bindings = engine::bind_operands(session, &parsed.operands, false)?;
    match script {
        Script::Substitute { .. } if !in_place || quiet => Err(session.error(
            ErrorCode::Uncorrectable,
            "sed_without_in_place",
            simple.span,
            &[],
        )),
        Script::Substitute {
            pattern,
            replacement,
            global,
            occurrence,
            ignore_case,
        } => {
            // Without `g` sed replaces the first match of every line, which
            // `replace` has no parameter for: `occurrence` counts in the whole
            // file. The pattern is anchored to the first match of a line
            // instead, and the template restores the text before the match.
            // A later occurrence has no such form.
            let anchored = !global;
            if anchored && occurrence.is_some_and(|number| number != 1) {
                return Err(unsupported());
            }
            let found = readings(&pattern)?;
            let found = if anchored {
                anchor_readings(found).ok_or_else(unsupported)?
            } else {
                found
            };
            let template = if anchored {
                format!("${{1}}{}", template_text(&replacement, ANCHOR_GROUPS))
            } else {
                template_text(&replacement, 0)
            };
            bindings.set(
                "with",
                vec![Bound::Node(Expr::Text {
                    v: template,
                    span: Some(script_word.span),
                })],
            );
            let mut extra = Vec::new();
            if ignore_case {
                extra.push("ignore_case");
            }
            substitute(session, simple, &bindings, &found, &extra, script_word.span)
        }
        Script::Print(_) | Script::Delete(_) if parsed.operands.len() > 1 => Err(session.error(
            ErrorCode::Uncorrectable,
            "too_many_operands",
            simple.span,
            &[("command", "sed")],
        )),
        Script::Print(address) if quiet && !in_place => {
            lines(session, simple, &mut bindings, &address, "print", &readings)
        }
        Script::Delete(address) if in_place && !quiet => lines(
            session,
            simple,
            &mut bindings,
            &address,
            "delete",
            &readings,
        ),
        Script::Delete(address @ (Address::Line(_) | Address::Range(..)))
            if !in_place && !quiet =>
        {
            without_lines(session, simple, &mut bindings, &address)
        }
        _ => Err(unsupported()),
    }
}

/// `sed 'Nd' FILE` without `-i`: the lines that remain are read.
fn without_lines(
    session: &Session<'_>,
    simple: &Simple,
    bindings: &mut Bindings,
    address: &Address,
) -> Result<Variants, Error> {
    let number = |v: i64| Bound::Node(Expr::Int { v, span: None });
    let key = match address {
        Address::Line(line) => {
            bindings.set("lo", vec![number(*line)]);
            "view_line"
        }
        Address::Range(lo, Some(hi)) => {
            bindings.set("lo", vec![number(*lo)]);
            bindings.set("hi", vec![number(*hi)]);
            "view_range"
        }
        Address::Range(lo, None) => {
            bindings.set("lo", vec![number(*lo)]);
            "view_from"
        }
        Address::Last | Address::Match(_) => {
            return Err(session.error(
                ErrorCode::Uncorrectable,
                "unsupported_sed_script",
                simple.span,
                &[("script", "d")],
            ));
        }
    };
    let stages = value_template(session, "sed", key, bindings, simple.span)?;
    let variants = Variants::one(stages, "line");
    session.record(
        "B17",
        CorrectionClass::Certain,
        simple.span,
        piece_text(variants.primary()),
        "habit",
    );
    Ok(variants)
}

fn substitute(
    session: &Session<'_>,
    simple: &Simple,
    bindings: &Bindings,
    found: &Readings,
    extra: &[&str],
    pattern_span: nurt_contracts::ast::Span,
) -> Result<Variants, Error> {
    let mut pieces = Vec::new();
    for reading in &found.items {
        let mut local = bindings.clone();
        let mut expr = reading.expr.clone();
        pattern::set_span(&mut expr, pattern_span);
        local.set("pattern", vec![Bound::Node(expr)]);
        let mut stages = value_template(session, "sed", "substitute", &local, simple.span)?;
        for key in extra {
            let more = value_template(session, "sed", key, &local, simple.span)?;
            if let (Some(stage), Some(added)) = (stages.last_mut(), more.into_iter().next()) {
                super::session::append_args(stage, added.args);
            }
        }
        pieces.push(Piece {
            stages,
            kind: "effect".to_owned(),
            why: Some(reading.why.to_owned()),
        });
    }
    let class = if found.probable {
        CorrectionClass::Probable
    } else {
        CorrectionClass::Certain
    };
    if found.probable
        && let Some(first) = found.items.first()
    {
        session.assume("B16", first.why);
    }
    let variants = Variants(pieces);
    session.record(
        "B16",
        class,
        simple.span,
        piece_text(variants.primary()),
        "habit",
    );
    Ok(variants)
}

fn lines(
    session: &Session<'_>,
    simple: &Simple,
    bindings: &mut Bindings,
    address: &Address,
    action: &str,
    readings: &dyn Fn(&str) -> Result<Readings, Error>,
) -> Result<Variants, Error> {
    let (key, found) = match address {
        Address::Line(line) => {
            bindings.set(
                "range",
                vec![Bound::Node(Expr::Int {
                    v: *line,
                    span: None,
                })],
            );
            ("range", None)
        }
        Address::Range(lo, hi) => {
            bindings.set(
                "range",
                vec![Bound::Node(Expr::Range {
                    lo: Some(*lo),
                    hi: *hi,
                    span: None,
                })],
            );
            ("range", None)
        }
        Address::Last => ("last", None),
        Address::Match(pattern) => ("match", Some(readings(pattern)?)),
    };
    let template = format!("{action}_{key}");
    let kind = if action == "print" { "line" } else { "effect" };
    let mut pieces = Vec::new();
    let items = found.as_ref().map_or_else(
        || vec![None],
        |found| found.items.iter().map(Some).collect(),
    );
    for reading in items {
        let mut local = bindings.clone();
        if let Some(reading) = reading {
            local.set(
                "regex",
                vec![Bound::Node(pattern::as_regex(&reading.expr, false))],
            );
        }
        let stages = value_template(session, "sed", &template, &local, simple.span)?;
        pieces.push(Piece {
            stages,
            kind: kind.to_owned(),
            why: reading.map(|r| r.why.to_owned()),
        });
    }
    let probable = found.as_ref().is_some_and(|found| found.probable);
    if probable && let Some(first) = found.as_ref().and_then(|found| found.items.first()) {
        session.assume("B17", first.why);
    }
    let class = if probable {
        CorrectionClass::Probable
    } else {
        CorrectionClass::Certain
    };
    let variants = Variants(pieces);
    session.record(
        "B17",
        class,
        simple.span,
        piece_text(variants.primary()),
        "habit",
    );
    Ok(variants)
}

fn parse_script(text: &str) -> Option<Script> {
    let text = text.trim().trim_end_matches(';').trim_end();
    if let Some(rest) = text.strip_prefix('s')
        && let Some(delimiter) = rest.chars().next()
        && delimiter != '\\'
        && delimiter != '\n'
        && !delimiter.is_alphanumeric()
    {
        return substitution(&rest[delimiter.len_utf8()..], delimiter);
    }
    let (address, command) = address(text)?;
    match command {
        "p" => Some(Script::Print(address)),
        "d" => Some(Script::Delete(address)),
        _ => None,
    }
}

fn substitution(text: &str, delimiter: char) -> Option<Script> {
    let mut parts = vec![String::new()];
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        let current = parts.last_mut()?;
        match c {
            '\\' => match chars.next() {
                Some(next) if next == delimiter => current.push(next),
                Some(next) => {
                    current.push('\\');
                    current.push(next);
                }
                None => return None,
            },
            c if c == delimiter => parts.push(String::new()),
            c => current.push(c),
        }
    }
    let [pattern, replacement, flags] = parts.as_slice() else {
        return None;
    };
    let mut global = false;
    let mut ignore_case = false;
    let mut digits = String::new();
    for c in flags.chars() {
        match c {
            'g' => global = true,
            'i' | 'I' => ignore_case = true,
            c if c.is_ascii_digit() => digits.push(c),
            _ => return None,
        }
    }
    let occurrence = if digits.is_empty() {
        None
    } else {
        Some(digits.parse().ok().filter(|number: &i64| *number > 0)?)
    };
    if pattern.is_empty() || (global && occurrence.is_some()) || !supported_replacement(replacement)
    {
        return None;
    }
    Some(Script::Substitute {
        pattern: pattern.clone(),
        replacement: replacement.clone(),
        global,
        occurrence,
        ignore_case,
    })
}

fn address(text: &str) -> Option<(Address, &str)> {
    if let Some(rest) = text.strip_prefix('/') {
        let mut pattern = String::new();
        let mut chars = rest.char_indices();
        while let Some((index, c)) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some((_, '/')) => pattern.push('/'),
                    Some((_, next)) => {
                        pattern.push('\\');
                        pattern.push(next);
                    }
                    None => return None,
                },
                '/' => return Some((Address::Match(pattern), rest[index + 1..].trim())),
                c => pattern.push(c),
            }
        }
        return None;
    }
    if let Some(rest) = text.strip_prefix('$') {
        return Some((Address::Last, rest.trim()));
    }
    let (first, rest) = number(text)?;
    match rest.strip_prefix(',') {
        Some(tail) if tail.starts_with('$') => {
            Some((Address::Range(first, None), tail[1..].trim()))
        }
        Some(tail) => {
            let (last, rest) = number(tail)?;
            (last >= first).then_some((Address::Range(first, Some(last)), rest.trim()))
        }
        None => Some((Address::Line(first), rest.trim())),
    }
}

fn number(text: &str) -> Option<(i64, &str)> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let value: i64 = text[..end].parse().ok()?;
    (value >= 1).then_some((value, &text[end..]))
}

/// Whether every escape of a replacement has a translation: a backslash
/// before a letter other than `n` and `t` (`\U`, `\L`, `\x41`) is not
/// translated.
fn supported_replacement(replacement: &str) -> bool {
    let mut chars = replacement.chars();
    while let Some(c) = chars.next() {
        if c == '\\'
            && chars
                .next()
                .is_some_and(|next| next.is_ascii_alphabetic() && !matches!(next, 'n' | 't'))
        {
            return false;
        }
    }
    true
}

/// Pattern anchored to the first match of a line, `^(.*?)(PATTERN)`, in
/// every reading; `None` when a reading has a back-reference, which the two
/// added groups would renumber. Readings that became equal, such as a
/// literal and its escaped regular expression, are kept once.
fn anchor_readings(mut found: Readings) -> Option<Readings> {
    for reading in &mut found.items {
        let (pattern, flags, span) = match &reading.expr {
            Expr::Text { v, span } => (regex::escape(v), String::new(), *span),
            Expr::Regex { v, flags, span } if !has_back_reference(v) => {
                (v.clone(), flags.clone(), *span)
            }
            _ => return None,
        };
        reading.expr = Expr::Regex {
            v: format!("^(.*?)({pattern})"),
            flags,
            span,
        };
    }
    let mut unique: Vec<pattern::Reading> = Vec::new();
    for reading in found.items {
        if !unique.iter().any(|known| known.expr == reading.expr) {
            unique.push(reading);
        }
    }
    found.probable = found.probable && unique.len() > 1;
    found.items = unique;
    Some(found)
}

/// Whether a regular expression has a back-reference such as `\1`.
fn has_back_reference(pattern: &str) -> bool {
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.next().is_some_and(|next| matches!(next, '1'..='9')) {
            return true;
        }
    }
    false
}

/// Translates a `sed` replacement into a replacement template: `&` is the
/// whole match, `\N` a group, and a literal `$` is written `$$`. The numbers
/// of the groups are `shift` higher than in the script.
fn template_text(replacement: &str, shift: u32) -> String {
    let chars: Vec<char> = replacement.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    let group = |out: &mut String, number: u32, next: Option<&char>| {
        let attached = next.is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_');
        if attached || number > 9 {
            out.push_str(&format!("${{{number}}}"));
        } else {
            out.push_str(&format!("${number}"));
        }
    };
    while let Some(&c) = chars.get(index) {
        index += 1;
        match c {
            '&' => group(&mut out, shift, chars.get(index)),
            '$' => out.push_str("$$"),
            '\\' => {
                let Some(&next) = chars.get(index) else {
                    out.push('\\');
                    continue;
                };
                index += 1;
                match next {
                    d if d.is_ascii_digit() => {
                        let number = d.to_digit(10).map_or(shift, |digit| digit + shift);
                        group(&mut out, number, chars.get(index));
                    }
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    other => out.push(other),
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutions_with_other_delimiters_and_flags() {
        assert_eq!(
            parse_script("s|a/b|c|g"),
            Some(Script::Substitute {
                pattern: "a/b".into(),
                replacement: "c".into(),
                global: true,
                occurrence: None,
                ignore_case: false
            })
        );
        assert_eq!(
            parse_script(r"s/a\/b/x/2I"),
            Some(Script::Substitute {
                pattern: "a/b".into(),
                replacement: "x".into(),
                global: false,
                occurrence: Some(2),
                ignore_case: true
            })
        );
        assert_eq!(parse_script("s/a/b/w f"), None);
        assert_eq!(parse_script("s/a/b"), None);
        assert_eq!(parse_script("s//b/"), None);
        assert_eq!(parse_script("s/a/b/2g"), None);
    }

    #[test]
    fn addresses() {
        assert_eq!(parse_script("5p"), Some(Script::Print(Address::Line(5))));
        assert_eq!(
            parse_script("10,20p"),
            Some(Script::Print(Address::Range(10, Some(20))))
        );
        assert_eq!(
            parse_script("3,$d"),
            Some(Script::Delete(Address::Range(3, None)))
        );
        assert_eq!(parse_script("$d"), Some(Script::Delete(Address::Last)));
        assert_eq!(
            parse_script(r"/^\s*debug\s*=/d"),
            Some(Script::Delete(Address::Match(r"^\s*debug\s*=".into())))
        );
        assert_eq!(parse_script("/a/a text"), None);
        assert_eq!(parse_script("5,2p"), None);
        assert_eq!(parse_script("0d"), None);
        assert_eq!(parse_script("/a"), None);
    }

    #[test]
    fn replacement_templates() {
        assert_eq!(template_text("[&]", 0), "[$0]");
        assert_eq!(template_text(r"\1x", 0), "${1}x");
        assert_eq!(template_text(r"\2-", 0), "$2-");
        assert_eq!(template_text("$5", 0), "$$5");
        assert_eq!(template_text(r"a\&b\n", 0), "a&b\n");
        assert_eq!(template_text("&x", 0), "${0}x");
        assert_eq!(template_text("x\\", 0), "x\\");
        assert_eq!(template_text(r"a\tb", 0), "a\tb");
    }

    #[test]
    fn templates_of_an_anchored_pattern_count_the_added_groups() {
        assert_eq!(template_text("[&]", ANCHOR_GROUPS), "[$2]");
        assert_eq!(template_text(r"\1x", ANCHOR_GROUPS), "${3}x");
        assert_eq!(template_text(r"\8-", ANCHOR_GROUPS), "${10}-");
    }

    #[test]
    fn anchoring_escapes_a_literal_and_refuses_back_references() {
        let literal = Readings {
            items: vec![pattern::Reading {
                expr: Expr::Text {
                    v: "a.b".to_owned(),
                    span: None,
                },
                why: "pattern_literal",
            }],
            probable: false,
        };
        let anchored = anchor_readings(literal).expect("a literal can be anchored");
        assert_eq!(
            anchored.items[0].expr,
            Expr::Regex {
                v: r"^(.*?)(a\.b)".to_owned(),
                flags: String::new(),
                span: None
            }
        );
        let reference = Readings {
            items: vec![pattern::Reading {
                expr: Expr::Regex {
                    v: r"(a)\1".to_owned(),
                    flags: String::new(),
                    span: None,
                },
                why: "pattern_extended",
            }],
            probable: false,
        };
        assert!(anchor_readings(reference).is_none());
        assert!(has_back_reference(r"(a)\1"));
        assert!(!has_back_reference(r"a\\1"));
    }

    #[test]
    fn rejects_replacement_escapes_without_a_translation() {
        assert_eq!(parse_script(r"s/a/\U&/"), None);
        assert_eq!(parse_script(r"s/a/\x41/"), None);
        assert_eq!(parse_script(r"s/a/\l&/g"), None);
        assert!(parse_script(r"s/a/b\n\t\&\\\1/").is_some());
        assert!(supported_replacement(""));
        assert!(!supported_replacement("a\\E"));
    }

    #[test]
    fn rejects_an_occurrence_of_zero() {
        assert_eq!(parse_script("s/a/b/0"), None);
        assert!(parse_script("s/a/b/1").is_some());
    }

    #[test]
    fn handles_multibyte_delimiters_and_text() {
        assert_eq!(
            parse_script("s§ą§b§g"),
            Some(Script::Substitute {
                pattern: "ą".into(),
                replacement: "b".into(),
                global: true,
                occurrence: None,
                ignore_case: false
            })
        );
        assert_eq!(parse_script("s"), None);
        assert_eq!(parse_script(""), None);
        assert_eq!(template_text("zażółć&", 0), "zażółć$0");
    }

    #[test]
    fn unterminated_scripts_are_rejected() {
        assert_eq!(parse_script("s/a/b\\"), None);
        assert_eq!(parse_script("/a\\"), None);
        assert_eq!(
            parse_script(r"/a\/b\.c/p"),
            Some(Script::Print(Address::Match(r"a/b\.c".into())))
        );
        assert_eq!(parse_script("5x"), None);
    }
}
