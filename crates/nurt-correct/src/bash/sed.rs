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
        return Err(session.error(ErrorCode::Uncorrectable, "unsupported_stage", simple.span, &[("command", "sed")]));
    }
    let mut parsed = engine::parse_options(session, entry, words)?;
    let script_word = match parsed.value_of(entry, "e") {
        Some(word) => word.clone(),
        None if !parsed.operands.is_empty() => parsed.operands.remove(0),
        None => return Err(session.error(ErrorCode::Uncorrectable, "missing_operand", simple.span, &[("slot", "script"), ("command", "sed")])),
    };
    let Some(text) = script_word.value.literal() else {
        return Err(session.error(ErrorCode::Uncorrectable, "variable_in_text", script_word.span, &[("name", &script_word.value.raw), ("lower", "x")]));
    };
    if parsed.operands.is_empty() {
        return Err(session.error(ErrorCode::Uncorrectable, "standard_input", simple.span, &[("command", "sed")]));
    }
    let unsupported = || {
        session
            .error(ErrorCode::Uncorrectable, "unsupported_sed_script", script_word.span, &[("script", &text)])
            .with_help(format!("help {}", entry.help))
    };
    let script = parse_script(&text).ok_or_else(unsupported)?;
    let in_place = parsed.has(entry, "i");
    let quiet = parsed.has(entry, "n");
    let dialect = if parsed.has(entry, "E") { Dialect::Extended } else { Dialect::Basic };
    let readings = |pattern: &str| {
        pattern::readings(pattern, dialect).map_err(|reason| {
            session.error(ErrorCode::Uncorrectable, "invalid_regex", script_word.span, &[("pattern", pattern), ("reason", &reason)])
        })
    };
    let mut bindings = engine::bind_operands(session, &parsed.operands)?;
    match script {
        Script::Substitute { .. } if !in_place || quiet => Err(session.error(ErrorCode::Uncorrectable, "sed_without_in_place", simple.span, &[])),
        Script::Substitute { pattern, replacement, global, occurrence, ignore_case } => {
            let found = readings(&pattern)?;
            bindings.set("with", vec![Bound::Node(Expr::Text { v: template_text(&replacement), span: Some(script_word.span) })]);
            let mut extra = Vec::new();
            if !global {
                bindings.set("occurrence", vec![Bound::Node(Expr::Int { v: occurrence.unwrap_or(1), span: None })]);
                extra.push("occurrence");
            }
            if ignore_case {
                extra.push("ignore_case");
            }
            substitute(session, simple, &bindings, &found, &extra, script_word.span)
        }
        Script::Print(_) | Script::Delete(_) if parsed.operands.len() > 1 => {
            Err(session.error(ErrorCode::Uncorrectable, "too_many_operands", simple.span, &[("command", "sed")]))
        }
        Script::Print(address) if quiet && !in_place => lines(session, simple, &mut bindings, &address, "print", &readings),
        Script::Delete(address) if in_place && !quiet => lines(session, simple, &mut bindings, &address, "delete", &readings),
        _ => Err(unsupported()),
    }
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
        pieces.push(Piece { stages, kind: "effect".to_owned(), why: Some(reading.why.to_owned()) });
    }
    let class = if found.probable { CorrectionClass::Probable } else { CorrectionClass::Certain };
    if found.probable && let Some(first) = found.items.first() {
        session.assume("B16", first.why);
    }
    let variants = Variants(pieces);
    session.record("B16", class, simple.span, piece_text(variants.primary()), "habit");
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
            bindings.set("range", vec![Bound::Node(Expr::Int { v: *line, span: None })]);
            ("range", None)
        }
        Address::Range(lo, hi) => {
            bindings.set("range", vec![Bound::Node(Expr::Range { lo: Some(*lo), hi: *hi, span: None })]);
            ("range", None)
        }
        Address::Last => ("last", None),
        Address::Match(pattern) => ("match", Some(readings(pattern)?)),
    };
    let template = format!("{action}_{key}");
    let kind = if action == "print" { "line" } else { "effect" };
    let mut pieces = Vec::new();
    let items = found.as_ref().map_or_else(|| vec![None], |found| found.items.iter().map(Some).collect());
    for reading in items {
        let mut local = bindings.clone();
        if let Some(reading) = reading {
            local.set("regex", vec![Bound::Node(pattern::as_regex(&reading.expr, false))]);
        }
        let stages = value_template(session, "sed", &template, &local, simple.span)?;
        pieces.push(Piece { stages, kind: kind.to_owned(), why: reading.map(|r| r.why.to_owned()) });
    }
    let probable = found.as_ref().is_some_and(|found| found.probable);
    if probable && let Some(first) = found.as_ref().and_then(|found| found.items.first()) {
        session.assume("B17", first.why);
    }
    let class = if probable { CorrectionClass::Probable } else { CorrectionClass::Certain };
    let variants = Variants(pieces);
    session.record("B17", class, simple.span, piece_text(variants.primary()), "habit");
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
        let Some(current) = parts.last_mut() else { return None };
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
    let [pattern, replacement, flags] = parts.as_slice() else { return None };
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
    let occurrence = if digits.is_empty() { None } else { Some(digits.parse().ok()?) };
    if pattern.is_empty() || (global && occurrence.is_some()) {
        return None;
    }
    Some(Script::Substitute { pattern: pattern.clone(), replacement: replacement.clone(), global, occurrence, ignore_case })
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
        Some(tail) if tail.starts_with('$') => Some((Address::Range(first, None), tail[1..].trim())),
        Some(tail) => {
            let (last, rest) = number(tail)?;
            (last >= first).then_some((Address::Range(first, Some(last)), rest.trim()))
        }
        None => Some((Address::Line(first), rest.trim())),
    }
}

fn number(text: &str) -> Option<(i64, &str)> {
    let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let value: i64 = text[..end].parse().ok()?;
    (value >= 1).then_some((value, &text[end..]))
}

/// Translates a `sed` replacement into a replacement template: `&` is the
/// whole match `$0`, `\N` a group `$N`, and a literal `$` is written `$$`.
fn template_text(replacement: &str) -> String {
    let chars: Vec<char> = replacement.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    let group = |out: &mut String, number: char, next: Option<&char>| {
        let attached = next.is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_');
        if attached {
            out.push_str(&format!("${{{number}}}"));
        } else {
            out.push('$');
            out.push(number);
        }
    };
    while let Some(&c) = chars.get(index) {
        index += 1;
        match c {
            '&' => group(&mut out, '0', chars.get(index)),
            '$' => out.push_str("$$"),
            '\\' => {
                let Some(&next) = chars.get(index) else {
                    out.push('\\');
                    continue;
                };
                index += 1;
                match next {
                    d if d.is_ascii_digit() => group(&mut out, d, chars.get(index)),
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
            Some(Script::Substitute { pattern: "a/b".into(), replacement: "c".into(), global: true, occurrence: None, ignore_case: false })
        );
        assert_eq!(
            parse_script(r"s/a\/b/x/2I"),
            Some(Script::Substitute { pattern: "a/b".into(), replacement: "x".into(), global: false, occurrence: Some(2), ignore_case: true })
        );
        assert_eq!(parse_script("s/a/b/w f"), None);
        assert_eq!(parse_script("s/a/b"), None);
        assert_eq!(parse_script("s//b/"), None);
        assert_eq!(parse_script("s/a/b/2g"), None);
    }

    #[test]
    fn addresses() {
        assert_eq!(parse_script("5p"), Some(Script::Print(Address::Line(5))));
        assert_eq!(parse_script("10,20p"), Some(Script::Print(Address::Range(10, Some(20)))));
        assert_eq!(parse_script("3,$d"), Some(Script::Delete(Address::Range(3, None))));
        assert_eq!(parse_script("$d"), Some(Script::Delete(Address::Last)));
        assert_eq!(parse_script(r"/^\s*debug\s*=/d"), Some(Script::Delete(Address::Match(r"^\s*debug\s*=".into()))));
        assert_eq!(parse_script("/a/a text"), None);
        assert_eq!(parse_script("5,2p"), None);
        assert_eq!(parse_script("0d"), None);
        assert_eq!(parse_script("/a"), None);
    }

    #[test]
    fn replacement_templates() {
        assert_eq!(template_text("[&]"), "[$0]");
        assert_eq!(template_text(r"\1x"), "${1}x");
        assert_eq!(template_text(r"\2-"), "$2-");
        assert_eq!(template_text("$5"), "$$5");
        assert_eq!(template_text(r"a\&b\n"), "a&b\n");
        assert_eq!(template_text("&x"), "${0}x");
        assert_eq!(template_text("x\\"), "x\\");
    }
}
