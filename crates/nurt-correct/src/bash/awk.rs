// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interpreter of the `awk` subset that has a translation: an optional
//! pattern (`/re/` or `$N op value`) and the action `{print $N, …}`, with
//! the field separator of `-F`. Fields are split by `split` and named `f1`,
//! `f2`, …; other programs are reported as not translatable.

use nurt_contracts::ast::{BinOp, Expr, Span, Stage};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::CorrectionClass;

use super::engine::{self, value_template};
use super::lex::Word;
use super::session::{Piece, Session, Variants, piece_text};
use super::syntax::{Located, Simple};
use super::table::CommandEntry;
use super::template::{Bindings, Bound};

/// Comparison of a field with a constant.
#[derive(Debug, Clone, PartialEq)]
struct Condition {
    field: u32,
    op: BinOp,
    value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
struct Program {
    regex: Option<String>,
    condition: Option<Condition>,
    fields: Vec<u32>,
}

pub(crate) fn translate(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    words: &[Located<Word>],
    input: Option<&str>,
) -> Result<Variants, Error> {
    let parsed = engine::parse_options(session, entry, words)?;
    let Some((program_word, files)) = parsed.operands.split_first() else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "missing_operand",
            simple.span,
            &[("slot", "program"), ("command", "awk")],
        ));
    };
    let text = program_word.value.literal().unwrap_or_default();
    let unsupported = || {
        session
            .error(
                ErrorCode::Uncorrectable,
                "unsupported_awk_program",
                program_word.span,
                &[("program", &text)],
            )
            .with_help(format!("help {}", entry.help))
    };
    if let Some(line) = line_program(&text) {
        return numbered_line(session, simple, input, files, line);
    }
    let program = parse_program(&text).ok_or_else(unsupported)?;
    if let Some(regex) = &program.regex
        && regex::Regex::new(regex).is_err()
    {
        return Err(unsupported());
    }
    let mut bindings = engine::bind_operands(session, files, false)?;
    let mut stages = match (input, files) {
        (None, [_]) => value_template(session, "awk", "source", &bindings, simple.span)?,
        (Some("line"), []) => Vec::new(),
        (None, []) => {
            return Err(session.error(
                ErrorCode::Uncorrectable,
                "standard_input",
                simple.span,
                &[("command", "awk")],
            ));
        }
        _ => {
            return Err(session.error(
                ErrorCode::Uncorrectable,
                "unsupported_stage",
                simple.span,
                &[("command", "awk")],
            ));
        }
    };
    let separator = parsed
        .value_of(entry, "F")
        .and_then(|word| word.value.literal());
    stages.extend(body(
        session,
        simple,
        &program,
        separator.as_deref(),
        &mut bindings,
    )?);
    let piece = Piece {
        stages,
        kind: "line".to_owned(),
        why: Some("awk_fields".to_owned()),
    };
    session.assume(&entry.rule, "awk_fields");
    session.record(
        &entry.rule,
        CorrectionClass::Probable,
        simple.span,
        piece_text(&piece),
        "habit",
    );
    Ok(Variants(vec![piece]))
}

/// `awk 'NR==N' FILE`: the line `N` of the file.
fn numbered_line(
    session: &Session<'_>,
    simple: &Simple,
    input: Option<&str>,
    files: &[Located<Word>],
    line: i64,
) -> Result<Variants, Error> {
    let [_] = files else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_stage",
            simple.span,
            &[("command", "awk")],
        ));
    };
    if input.is_some() {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_stage",
            simple.span,
            &[("command", "awk")],
        ));
    }
    let mut bindings = engine::bind_operands(session, files, false)?;
    bindings.set(
        "range",
        vec![Bound::Node(Expr::Range {
            lo: Some(line),
            hi: Some(line),
            span: None,
        })],
    );
    let stages = value_template(session, "awk", "line", &bindings, simple.span)?;
    let variants = Variants::one(stages, "line");
    session.record(
        "B18",
        CorrectionClass::Certain,
        simple.span,
        piece_text(variants.primary()),
        "habit",
    );
    Ok(variants)
}

/// The number of a program that selects one line, `NR==N`.
fn line_program(text: &str) -> Option<i64> {
    let digits = text
        .trim()
        .strip_prefix("NR")?
        .trim_start()
        .strip_prefix("==")?;
    let digits = digits.trim();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|line| *line > 0)
}

fn body(
    session: &Session<'_>,
    simple: &Simple,
    program: &Program,
    separator: Option<&str>,
    bindings: &mut Bindings,
) -> Result<Vec<Stage>, Error> {
    let span: Span = simple.span;
    let mut stages = Vec::new();
    if let Some(regex) = &program.regex {
        bindings.set(
            "regex",
            vec![Bound::Node(Expr::Regex {
                v: regex.clone(),
                flags: String::new(),
                span: None,
            })],
        );
        stages.extend(value_template(session, "awk", "match", bindings, span)?);
    }
    let widest = program
        .fields
        .iter()
        .chain(program.condition.as_ref().map(|c| &c.field))
        .copied()
        .max()
        .unwrap_or(0);
    if widest == 0 {
        return Ok(stages);
    }
    let names: Vec<Expr> = (1..=widest)
        .map(|n| Expr::Text {
            v: field_name(n),
            span: None,
        })
        .collect();
    bindings.set("delimiter", vec![Bound::Node(delimiter(separator))]);
    bindings.set(
        "names",
        vec![Bound::Node(Expr::List {
            items: names,
            span: None,
        })],
    );
    stages.extend(value_template(session, "awk", "split", bindings, span)?);
    if let Some(condition) = &program.condition {
        let test = Expr::Bin {
            op: condition.op,
            l: Box::new(cell(condition.field)),
            r: Box::new(condition.value.clone()),
            span: None,
        };
        bindings.set("condition", vec![Bound::Node(test)]);
        stages.extend(value_template(session, "awk", "where", bindings, span)?);
    }
    if program.fields == [0] {
        return Ok(stages);
    }
    if program.fields.contains(&0) {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_awk_program",
            span,
            &[("program", "$0")],
        ));
    }
    let fields = program
        .fields
        .iter()
        .map(|n| Bound::Node(cell(*n)))
        .collect();
    bindings.set("fields", fields);
    stages.extend(value_template(session, "awk", "select", bindings, span)?);
    Ok(stages)
}

fn field_name(number: u32) -> String {
    format!("f{number}")
}

/// Field of the `row` records of `split` that holds the cells.
const CELLS: &str = "cells";

/// The cell named for the awk field `$number`: `cells.f<number>`. `split`
/// gives `row` records, whose parts are inside `cells`, not fields of the
/// record.
fn cell(number: u32) -> Expr {
    Expr::Field {
        name: field_name(number),
        of: Some(Box::new(Expr::Ident {
            v: CELLS.to_owned(),
            span: None,
        })),
        span: None,
    }
}

/// Field separator: whitespace runs by default, a single character as
/// text, longer separators as regular expressions.
fn delimiter(separator: Option<&str>) -> Expr {
    match separator {
        None | Some(" ") => Expr::Regex {
            v: r"\s+".to_owned(),
            flags: String::new(),
            span: None,
        },
        Some("\\t") => Expr::Text {
            v: "\t".to_owned(),
            span: None,
        },
        Some(text) if text.chars().count() == 1 => Expr::Text {
            v: text.to_owned(),
            span: None,
        },
        Some(text) => Expr::Regex {
            v: text.to_owned(),
            flags: String::new(),
            span: None,
        },
    }
}

fn parse_program(text: &str) -> Option<Program> {
    let mut rest = text.trim();
    let mut regex = None;
    let mut condition = None;
    if let Some(tail) = rest.strip_prefix('/') {
        let end = tail.find('/')?;
        regex = Some(tail[..end].to_owned());
        rest = tail[end + 1..].trim();
    } else if rest.starts_with('$') {
        let end = rest.find('{').unwrap_or(rest.len());
        condition = Some(parse_condition(rest[..end].trim())?);
        rest = rest[end..].trim();
    }
    let fields = if rest.is_empty() {
        vec![0]
    } else {
        let inner = rest.strip_prefix('{')?.strip_suffix('}')?.trim();
        let list = inner.strip_prefix("print")?;
        if list.is_empty() {
            vec![0]
        } else if list.starts_with([' ', '\t']) {
            list.split(',')
                .map(|item| field(item.trim()))
                .collect::<Option<Vec<_>>>()?
        } else {
            return None;
        }
    };
    (regex.is_some() || condition.is_some() || !rest.is_empty()).then_some(Program {
        regex,
        condition,
        fields,
    })
}

fn field(text: &str) -> Option<u32> {
    let digits = text.strip_prefix('$')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn parse_condition(text: &str) -> Option<Condition> {
    const OPERATORS: &[(&str, BinOp)] = &[
        ("==", BinOp::Eq),
        ("!=", BinOp::Ne),
        (">=", BinOp::Ge),
        ("<=", BinOp::Le),
        (">", BinOp::Gt),
        ("<", BinOp::Lt),
    ];
    let (position, symbol, op) = OPERATORS
        .iter()
        .filter_map(|(symbol, op)| text.find(symbol).map(|position| (position, *symbol, *op)))
        .min_by_key(|(position, symbol, _)| (*position, usize::MAX - symbol.len()))?;
    let field_number = field(text[..position].trim())?;
    if field_number == 0 {
        return None;
    }
    let value_text = text[position + symbol.len()..].trim();
    let value = if let Some(quoted) = value_text
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
    {
        Expr::Text {
            v: quoted.to_owned(),
            span: None,
        }
    } else {
        Expr::Int {
            v: value_text.parse().ok().filter(|v: &i64| *v >= 0)?,
            span: None,
        }
    };
    Some(Condition {
        field: field_number,
        op,
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_lists() {
        assert_eq!(
            parse_program("{print $2}"),
            Some(Program {
                regex: None,
                condition: None,
                fields: vec![2]
            })
        );
        assert_eq!(
            parse_program("{ print $1, $3 }").map(|p| p.fields),
            Some(vec![1, 3])
        );
        assert_eq!(parse_program("{print}").map(|p| p.fields), Some(vec![0]));
        assert_eq!(parse_program("{print $NF}"), None);
        assert_eq!(parse_program("{printf $1}"), None);
        assert_eq!(parse_program("{sum += $1}"), None);
        assert_eq!(parse_program(""), None);
    }

    #[test]
    fn patterns_and_conditions() {
        let program = parse_program("/error/ {print $1}").unwrap();
        assert_eq!(program.regex.as_deref(), Some("error"));
        let filtered = parse_program("$3 >= 100 {print $1}").unwrap();
        assert_eq!(
            filtered.condition,
            Some(Condition {
                field: 3,
                op: BinOp::Ge,
                value: Expr::Int { v: 100, span: None }
            })
        );
        let text = parse_program("$1 == \"GET\"").unwrap();
        assert_eq!(text.fields, vec![0]);
        assert_eq!(
            text.condition.unwrap().value,
            Expr::Text {
                v: "GET".to_owned(),
                span: None
            }
        );
        assert_eq!(parse_program("$0 > 1"), None);
        assert_eq!(parse_program("$1 > x"), None);
        assert_eq!(parse_program("/x"), None);
    }

    #[test]
    fn awk_fields_are_cells_of_the_row_record() {
        assert_eq!(
            cell(3),
            Expr::Field {
                name: "f3".to_owned(),
                of: Some(Box::new(Expr::Ident {
                    v: "cells".to_owned(),
                    span: None
                })),
                span: None
            }
        );
    }

    #[test]
    fn line_programs_select_one_line() {
        assert_eq!(line_program("NR==5"), Some(5));
        assert_eq!(line_program(" NR == 12 "), Some(12));
        assert_eq!(line_program("NR==0"), None);
        assert_eq!(line_program("NR==x"), None);
        assert_eq!(line_program("NR=="), None);
        assert_eq!(line_program("NR==99999999999999999999"), None);
        assert_eq!(line_program("{print}"), None);
    }

    #[test]
    fn programs_with_multibyte_text_do_not_panic() {
        assert_eq!(parse_program("/żółć/ {print $1}").unwrap().regex.as_deref(), Some("żółć"));
        assert_eq!(parse_program("$1 == \"ą\"").unwrap().fields, vec![0]);
        assert_eq!(parse_program("ż"), None);
        assert_eq!(parse_program("$1 > ł"), None);
        assert_eq!(parse_program("{print $1,}"), None);
    }

    #[test]
    fn separators() {
        assert_eq!(
            delimiter(None),
            Expr::Regex {
                v: r"\s+".to_owned(),
                flags: String::new(),
                span: None
            }
        );
        assert_eq!(
            delimiter(Some(",")),
            Expr::Text {
                v: ",".to_owned(),
                span: None
            }
        );
        assert_eq!(
            delimiter(Some("\\t")),
            Expr::Text {
                v: "\t".to_owned(),
                span: None
            }
        );
        assert_eq!(
            delimiter(Some("[,;]")),
            Expr::Regex {
                v: "[,;]".to_owned(),
                flags: String::new(),
                span: None
            }
        );
    }
}
