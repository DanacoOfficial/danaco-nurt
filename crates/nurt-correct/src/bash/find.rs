// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interpreter of `find` predicates.
//!
//! The start directory becomes the source of `find`; `-name`, `-iname`,
//! `-type` and `-maxdepth` become its parameters, `-mtime`, `-mmin` and
//! `-size` become `filter` stages with the exact rounding rules of `find`,
//! and `-delete` and `-exec` become the stage that receives the entries.
//! The flag-to-parameter mapping is read from the table entry; this module
//! computes the values.

use nurt_contracts::ast::{BinOp, Expr, Span};
use nurt_contracts::error::{Error, ErrorCode};

use super::engine::{self, Given, Parsed};
use super::lex::Word;
use super::produce::{self, Delivery};
use super::session::{Session, Variants};
use super::syntax::{Located, Simple};
use super::table::CommandEntry;
use super::template::{Bindings, Bound};

/// Milliseconds of the time units of `-mtime` and `-mmin`.
const DAY_MILLIS: u64 = 86_400_000;
const MINUTE_MILLIS: u64 = 60_000;

/// Comparison written with a sign before a number: `+N`, `-N` or `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sign {
    More,
    Less,
    Exactly,
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
            &[("command", "find")],
        ));
    }
    let start = words
        .iter()
        .position(|word| {
            word.value.raw.starts_with('-') || word.value.raw == "!" || word.value.raw == "("
        })
        .unwrap_or(words.len());
    let (paths, predicates) = words.split_at(start);
    let start_path = match paths {
        [] => engine::located(".", simple.span),
        [path] => path.clone(),
        _ => {
            return Err(session.error(
                ErrorCode::Uncorrectable,
                "too_many_operands",
                simple.span,
                &[("command", "find")],
            ));
        }
    };
    let (parsed, exec) = predicates_of(session, entry, predicates)?;
    let mut parsed = Parsed {
        operands: vec![start_path],
        ..parsed
    };
    let mut bindings = engine::bind_operands(session, &parsed.operands, false)?;
    let readings = bind_values(session, entry, &parsed, &mut bindings)?;
    let mut pieces = Vec::new();
    for (index, size) in readings.iter().enumerate() {
        let mut local = bindings.clone();
        if let Some(condition) = size {
            local.set("size", vec![Bound::Node(condition.clone())]);
        }
        parsed.probable = readings.len() > 1;
        let built = engine::build(session, entry, simple, &parsed, &local, None)?;
        let mut piece = built.0.into_iter().next().ok_or_else(|| {
            session.error(
                ErrorCode::Uncorrectable,
                "no_translation",
                simple.span,
                &[("command", "find")],
            )
        })?;
        piece.why = Some(
            if index == 0 {
                "binary_units"
            } else {
                "decimal_units"
            }
            .to_owned(),
        );
        pieces.push(piece);
    }
    let variants = Variants(pieces);
    match exec {
        Some(exec) => apply_exec(session, simple, &exec, variants),
        None => Ok(variants),
    }
}

/// Command of `-exec … ;` or `-exec … +` with `{}` removed.
struct Exec {
    words: Vec<Located<Word>>,
    delivery: Delivery,
    span: Span,
}

fn predicates_of(
    session: &Session<'_>,
    entry: &CommandEntry,
    words: &[Located<Word>],
) -> Result<(Parsed, Option<Exec>), Error> {
    let mut parsed = Parsed::default();
    let mut exec = None;
    let mut index = 0;
    while let Some(word) = words.get(index) {
        index += 1;
        let flag = word
            .value
            .literal()
            .unwrap_or_else(|| word.value.raw.clone());
        if flag == "-exec" || flag == "-execdir" {
            let (found, next) = exec_words(session, words, index, word.span)?;
            exec = Some(found);
            index = next;
            continue;
        }
        let Some(option) = entry
            .options
            .iter()
            .position(|option| option.flags.contains(&flag))
        else {
            return Err(session
                .error(
                    ErrorCode::Uncorrectable,
                    "unsupported_find_predicate",
                    word.span,
                    &[("predicate", &flag)],
                )
                .with_help(format!("help {}", entry.help)));
        };
        let value = if entry.options[option].value.is_some() {
            let Some(value) = words.get(index) else {
                return Err(session.error(
                    ErrorCode::Uncorrectable,
                    "flag_without_value",
                    word.span,
                    &[("flag", &flag)],
                ));
            };
            index += 1;
            Some(value.clone())
        } else {
            None
        };
        parsed.given.push(Given {
            index: option,
            value,
            span: word.span,
        });
    }
    Ok((parsed, exec))
}

fn exec_words(
    session: &Session<'_>,
    words: &[Located<Word>],
    start: usize,
    span: Span,
) -> Result<(Exec, usize), Error> {
    let end = words[start..]
        .iter()
        .position(|word| matches!(word.value.literal().as_deref(), Some(";" | "+")))
        .map(|offset| start + offset)
        .ok_or_else(|| {
            session.error(
                ErrorCode::Uncorrectable,
                "unsupported_find_predicate",
                span,
                &[("predicate", "-exec")],
            )
        })?;
    let delivery = if words[end].value.literal().as_deref() == Some("+") {
        Delivery::AllAtOnce
    } else {
        Delivery::OnePerRun
    };
    let command: Vec<Located<Word>> = words[start..end].to_vec();
    let placeholders = command.iter().filter(|word| word.value.raw == "{}").count();
    if placeholders != 1 || command.first().is_none_or(|word| word.value.raw == "{}") {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_find_predicate",
            span,
            &[("predicate", "-exec")],
        ));
    }
    let words = command
        .into_iter()
        .filter(|word| word.value.raw != "{}")
        .collect();
    Ok((
        Exec {
            words,
            delivery,
            span,
        },
        end + 1,
    ))
}

fn apply_exec(
    session: &Session<'_>,
    simple: &Simple,
    exec: &Exec,
    variants: Variants,
) -> Result<Variants, Error> {
    produce::apply_to_entries(
        session,
        simple,
        &exec.words,
        variants,
        exec.delivery,
        exec.span,
    )
}

/// Binds the computed values of the given predicates; returns the readings
/// of `-size`: one for exact units, two (binary first) for `k`, `M`, `G`.
fn bind_values(
    session: &Session<'_>,
    entry: &CommandEntry,
    parsed: &Parsed,
    bindings: &mut Bindings,
) -> Result<Vec<Option<Expr>>, Error> {
    let mut readings = vec![None];
    for given in &parsed.given {
        let option = &entry.options[given.index];
        let Some(value) = &given.value else { continue };
        let text = value.value.literal().unwrap_or_default();
        match option.id.as_str() {
            "name" | "iname" => bindings.set("glob", vec![glob(session, value, &text)?]),
            "type" => {
                let kind = session
                    .table
                    .value_str("find_type", &text)
                    .ok_or_else(|| invalid(session, value.span, &text))?;
                bindings.set(
                    "type",
                    vec![Bound::Node(Expr::Ident {
                        v: kind.to_owned(),
                        span: Some(value.span),
                    })],
                );
            }
            "mtime" | "mmin" => {
                let unit = if option.id == "mtime" {
                    DAY_MILLIS
                } else {
                    MINUTE_MILLIS
                };
                let (sign, number) =
                    signed(&text).ok_or_else(|| invalid(session, value.span, &text))?;
                bindings.set(&option.id, vec![Bound::Node(age(sign, number, unit))]);
            }
            "size" => readings = size_readings(session, value.span, &text)?,
            _ => bindings.push(&option.id, session.operand(value)?),
        }
    }
    Ok(readings)
}

fn glob(session: &Session<'_>, value: &Located<Word>, text: &str) -> Result<Bound, Error> {
    if text.contains('/') {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_find_predicate",
            value.span,
            &[("predicate", text)],
        ));
    }
    Ok(Bound::Word {
        text: format!("**/{text}"),
        span: value.span,
        bare: false,
    })
}

fn invalid(session: &Session<'_>, span: Span, text: &str) -> Error {
    session.error(
        ErrorCode::Uncorrectable,
        "invalid_value",
        span,
        &[("value", text)],
    )
}

fn signed(text: &str) -> Option<(Sign, u64)> {
    let (sign, digits) = match text.as_bytes().first()? {
        b'+' => (Sign::More, &text[1..]),
        b'-' => (Sign::Less, &text[1..]),
        _ => (Sign::Exactly, text),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((sign, digits.parse().ok()?))
}

/// Condition of `-mtime` or `-mmin`: the age in whole units is compared
/// with the number, so `+N` means at least `N + 1` units.
fn age(sign: Sign, number: u64, unit: u64) -> Expr {
    let modified = || Expr::Ident {
        v: "modified".to_owned(),
        span: None,
    };
    let ago = |units: u64| Expr::Call {
        function: "ago".to_owned(),
        args: vec![Expr::Dur {
            v: units.saturating_mul(unit),
            span: None,
        }],
        span: None,
    };
    let compare = |op: BinOp, units: u64| binary(op, modified(), ago(units));
    match sign {
        Sign::Less => compare(BinOp::Gt, number),
        Sign::More => compare(BinOp::Le, number.saturating_add(1)),
        Sign::Exactly => binary(
            BinOp::And,
            compare(BinOp::Le, number),
            compare(BinOp::Gt, number.saturating_add(1)),
        ),
    }
}

/// Readings of `-size`: the size in whole units (rounded up) is compared
/// with the number.
fn size_readings(
    session: &Session<'_>,
    span: Span,
    text: &str,
) -> Result<Vec<Option<Expr>>, Error> {
    let unit = text.chars().last().filter(char::is_ascii_alphabetic);
    let number_text = unit.map_or(text, |_| &text[..text.len() - 1]);
    let (sign, number) = signed(number_text).ok_or_else(|| invalid(session, span, text))?;
    let unit_key = unit.map_or_else(|| "b".to_owned(), |c| c.to_string());
    let factor = |map: &str| {
        session
            .table
            .value(map, &unit_key)
            .and_then(serde_json::Value::as_u64)
    };
    let binary_factor = factor("find_size").ok_or_else(|| invalid(session, span, text))?;
    let mut readings = vec![Some(size_condition(sign, number, binary_factor))];
    if let Some(decimal) = factor("find_size_decimal") {
        readings.push(Some(size_condition(sign, number, decimal)));
    }
    Ok(readings)
}

fn size_condition(sign: Sign, number: u64, factor: u64) -> Expr {
    let size = || Expr::Ident {
        v: "size".to_owned(),
        span: None,
    };
    let bytes = |units: u64| Expr::Size {
        v: units.saturating_mul(factor),
        span: None,
    };
    match sign {
        Sign::More => binary(BinOp::Gt, size(), bytes(number)),
        Sign::Less => binary(BinOp::Le, size(), bytes(number.saturating_sub(1))),
        Sign::Exactly => binary(
            BinOp::And,
            binary(BinOp::Gt, size(), bytes(number.saturating_sub(1))),
            binary(BinOp::Le, size(), bytes(number)),
        ),
    }
}

fn binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::Bin {
        op,
        l: Box::new(left),
        r: Box::new(right),
        span: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tree::print_stage;
    use super::*;
    use nurt_contracts::ast::{Arg, Stage};

    fn show(expr: Expr) -> String {
        let stage = Stage {
            name: "filter".to_owned(),
            args: vec![Arg {
                name: None,
                value: expr,
                span: None,
            }],
            span: None,
        };
        print_stage(&stage)
    }

    #[test]
    fn signs_are_parsed() {
        assert_eq!(signed("+7"), Some((Sign::More, 7)));
        assert_eq!(signed("-1"), Some((Sign::Less, 1)));
        assert_eq!(signed("3"), Some((Sign::Exactly, 3)));
        assert_eq!(signed("+"), None);
        assert_eq!(signed("x1"), None);
    }

    #[test]
    fn ages_follow_find_rounding() {
        assert_eq!(
            show(age(Sign::Less, 1, DAY_MILLIS)),
            "filter modified > ago(1d)"
        );
        assert_eq!(
            show(age(Sign::More, 7, DAY_MILLIS)),
            "filter modified <= ago(8d)"
        );
        assert_eq!(
            show(age(Sign::Exactly, 2, MINUTE_MILLIS)),
            "filter modified <= ago(2m) and modified > ago(3m)"
        );
    }

    #[test]
    fn sizes_follow_find_rounding() {
        assert_eq!(
            show(size_condition(Sign::More, 1, 1_048_576)),
            "filter size > 1mib"
        );
        assert_eq!(
            show(size_condition(Sign::Less, 1, 1_048_576)),
            "filter size <= 0b"
        );
        assert_eq!(
            show(size_condition(Sign::Exactly, 2, 1024)),
            "filter size > 1kib and size <= 2kib"
        );
    }
}
