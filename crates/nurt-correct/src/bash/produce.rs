// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Commands that produce or consume text and records around the table
//! entries: redirections into files, `echo`, `printf`, here-documents,
//! `test`, command substitution, `xargs`, `tee` and `at`.

use nurt_contracts::ast::{Expr, Ref, Span, Stage};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::CorrectionClass;

use super::lex::{HereDoc, RedirectOp, Word, lex};
use super::session::{
    Piece, RunOptions, Session, Variants, named, piece_text, pipeline, positional,
};
use super::syntax::{self, Located, Redirection, Simple};
use super::table::CommandEntry;
use super::template::{Bindings, Bound, Conversion, Convert, fill_pipeline, parse_pipeline};
use super::tree::print_stage;

/// Target of an output redirection.
pub(crate) struct Output {
    pub(crate) mode: &'static str,
    pub(crate) target: Located<Word>,
    pub(crate) span: Span,
}

/// How the entries of the previous command reach a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// `xargs` without `-0`: the input is split at whitespace.
    SplitWords,
    /// `find -exec … ;`: one run for every entry.
    OnePerRun,
    /// `xargs -0` and `find -exec … +`: every entry in one run.
    AllAtOnce,
}

/// Translates a simple command with its redirections.
pub(crate) fn redirected(
    session: &Session<'_>,
    simple: &Simple,
    input: Option<&str>,
    options: &mut RunOptions,
) -> Result<Variants, Error> {
    let mut stdin = None;
    let mut here_doc = None;
    for redirection in &simple.redirects {
        match classify(redirection) {
            Redirect::Quiet => options.quiet = true,
            Redirect::Merge => options.merge_stderr = true,
            Redirect::Ignore | Redirect::Output => {}
            Redirect::Input(target) => stdin = Some(target),
            Redirect::HereDoc(doc) => here_doc = Some(doc),
            Redirect::Unsupported => {
                let op = redirection.redirect.op.as_str();
                return Err(session.error(
                    ErrorCode::Uncorrectable,
                    "unsupported_redirection",
                    redirection.span,
                    &[("operator", op)],
                ));
            }
        }
    }
    if let Some(doc) = here_doc {
        return here_document(session, simple, doc);
    }
    if is_content(simple) {
        return content(session, simple);
    }
    match stdin {
        Some(_) if input.is_some() => Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_redirection",
            simple.span,
            &[("operator", "<")],
        )),
        Some(target) => {
            let path = session.convert(&session.operand(&target)?, Conversion::Path)?;
            let head = Variants::one(
                vec![Stage {
                    name: "lines".to_owned(),
                    args: vec![positional(path)],
                    span: Some(target.span),
                }],
                "line",
            );
            let stage = session.words(simple, &simple.words, Some("line"), options)?;
            Ok(head.then(stage))
        }
        None => session.words(simple, &simple.words, input, options),
    }
}

enum Redirect {
    Quiet,
    Merge,
    Ignore,
    Output,
    Input(Located<Word>),
    HereDoc(HereDoc),
    Unsupported,
}

fn classify(redirection: &Redirection) -> Redirect {
    let redirect = &redirection.redirect;
    let target = redirection.target.as_ref().and_then(|t| t.value.literal());
    let null = target.as_deref() == Some("/dev/null");
    match (redirect.fd, redirect.op) {
        (None | Some(1), RedirectOp::Out | RedirectOp::Append | RedirectOp::Clobber) if null => {
            Redirect::Quiet
        }
        (None | Some(1), RedirectOp::Out | RedirectOp::Append | RedirectOp::Clobber) => {
            Redirect::Output
        }
        (None, RedirectOp::OutErr | RedirectOp::AppendOutErr) if null => Redirect::Quiet,
        (None, RedirectOp::OutErr | RedirectOp::AppendOutErr) => Redirect::Output,
        (Some(2), RedirectOp::Out | RedirectOp::Append) if null => Redirect::Ignore,
        (Some(2), RedirectOp::DupOut) if target.as_deref() == Some("1") => Redirect::Merge,
        (None | Some(0), RedirectOp::In) => redirection
            .target
            .clone()
            .map_or(Redirect::Unsupported, Redirect::Input),
        (None | Some(0), RedirectOp::HereDoc | RedirectOp::HereDocStrip) => redirect
            .here_doc
            .clone()
            .map_or(Redirect::Unsupported, Redirect::HereDoc),
        _ => Redirect::Unsupported,
    }
}

/// The file the standard output of a command is written to, unless the
/// command produces its text itself.
pub(crate) fn output_target(simple: &Simple) -> Option<Output> {
    if is_content(simple)
        || simple
            .redirects
            .iter()
            .any(|r| r.redirect.here_doc.is_some())
    {
        return None;
    }
    simple.redirects.iter().rev().find_map(|redirection| {
        if !matches!(classify(redirection), Redirect::Output) {
            return None;
        }
        let mode = Session::redirect_kind(redirection.redirect.op)?;
        Some(Output {
            mode,
            target: redirection.target.clone()?,
            span: redirection.span,
        })
    })
}

fn is_content(simple: &Simple) -> bool {
    simple
        .words
        .first()
        .and_then(|word| word.value.literal())
        .is_some_and(|name| name == "echo" || name == "printf")
}

/// `A > F`: the result of a reading program is written to the file.
pub(crate) fn write_from(
    session: &Session<'_>,
    output: &Output,
    previous: Variants,
) -> Result<Variants, Error> {
    if session.has_effect(previous.primary()) {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "output_redirection",
            output.span,
            &[],
        ));
    }
    let path = session.convert(&session.operand(&output.target)?, Conversion::Path)?;
    let mode = output.mode;
    let variants = previous.map(|piece| {
        let source = Expr::Sub {
            p: pipeline(&piece.stages),
            span: None,
        };
        let stage = Stage {
            name: "write".to_owned(),
            args: vec![
                positional(path.clone()),
                named("from", source),
                named(
                    "mode",
                    Expr::Ident {
                        v: mode.to_owned(),
                        span: None,
                    },
                ),
            ],
            span: Some(output.span),
        };
        Piece {
            stages: vec![stage],
            kind: "effect".to_owned(),
            why: piece.why,
        }
    });
    session.record(
        "B19",
        CorrectionClass::Certain,
        output.span,
        piece_text(variants.primary()),
        "habit",
    );
    Ok(variants)
}

fn write_text(
    session: &Session<'_>,
    rule: &str,
    simple: &Simple,
    text: String,
) -> Result<Variants, Error> {
    let Some(output) = simple.redirects.iter().rev().find_map(|redirection| {
        let mode = Session::redirect_kind(redirection.redirect.op)?;
        Some((mode, redirection.target.clone()?))
    }) else {
        return Err(no_file(session, simple, &text));
    };
    let (mode, target) = output;
    let path = session.convert(&session.operand(&target)?, Conversion::Path)?;
    let stage = Stage {
        name: "write".to_owned(),
        args: vec![
            positional(path),
            positional(Expr::Text {
                v: text,
                span: None,
            }),
            named(
                "mode",
                Expr::Ident {
                    v: mode.to_owned(),
                    span: None,
                },
            ),
        ],
        span: Some(simple.span),
    };
    session.record(
        rule,
        CorrectionClass::Certain,
        simple.span,
        print_stage(&stage),
        "habit",
    );
    Ok(Variants::one(vec![stage], "effect"))
}

fn no_file(session: &Session<'_>, simple: &Simple, text: &str) -> Error {
    let stage = Stage {
        name: "evaluate".to_owned(),
        args: vec![positional(Expr::Text {
            v: text.trim_end_matches('\n').to_owned(),
            span: None,
        })],
        span: None,
    };
    session
        .error(
            ErrorCode::Uncorrectable,
            "output_without_file",
            simple.span,
            &[],
        )
        .with_fix(print_stage(&stage))
}

/// `echo` or `printf` without an output file.
pub(crate) fn content_without_file(
    session: &Session<'_>,
    _entry: &CommandEntry,
    simple: &Simple,
    _words: &[Located<Word>],
) -> Error {
    match text_of(session, simple) {
        Ok(text) => no_file(session, simple, &text),
        Err(error) => error,
    }
}

fn content(session: &Session<'_>, simple: &Simple) -> Result<Variants, Error> {
    let text = text_of(session, simple)?;
    write_text(session, "B19", simple, text)
}

fn text_of(session: &Session<'_>, simple: &Simple) -> Result<String, Error> {
    let name = simple
        .words
        .first()
        .and_then(|word| word.value.literal())
        .unwrap_or_default();
    let arguments = simple.words.get(1..).unwrap_or_default();
    session.reject_glob(arguments)?;
    let mut texts = Vec::new();
    for word in arguments {
        match session.operand(word)? {
            Bound::Word { text, .. } => texts.push((text, word.span)),
            Bound::Node(_) => {
                return Err(session.error(
                    ErrorCode::Uncorrectable,
                    "variable_in_text",
                    word.span,
                    &[("name", &word.value.raw), ("lower", "x")],
                ));
            }
        }
    }
    if name == "printf" {
        printf(session, simple, &texts)
    } else {
        Ok(echo(&texts))
    }
}

fn echo(words: &[(String, Span)]) -> String {
    let mut newline = true;
    let mut escapes = false;
    let mut start = 0;
    for (word, _) in words {
        let is_flag = word.len() > 1
            && word.starts_with('-')
            && word[1..].chars().all(|c| matches!(c, 'n' | 'e' | 'E'));
        if !is_flag {
            break;
        }
        for c in word[1..].chars() {
            match c {
                'n' => newline = false,
                'e' => escapes = true,
                _ => escapes = false,
            }
        }
        start += 1;
    }
    let joined = words[start..]
        .iter()
        .map(|(word, _)| word.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let mut text = if escapes { unescape(&joined) } else { joined };
    if newline {
        text.push('\n');
    }
    text
}

/// Backslash escapes of `echo -e` and `printf`.
fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn printf(
    session: &Session<'_>,
    simple: &Simple,
    words: &[(String, Span)],
) -> Result<String, Error> {
    let Some(((format, span), args)) = words.split_first() else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "missing_operand",
            simple.span,
            &[("slot", "format"), ("command", "printf")],
        ));
    };
    let unsupported = || {
        session.error(
            ErrorCode::Uncorrectable,
            "unsupported_format",
            *span,
            &[("format", format)],
        )
    };
    let mut out = String::new();
    let mut args = args.iter();
    let mut chars = unescape(format).chars().collect::<Vec<_>>().into_iter();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('s') => out.push_str(args.next().map_or("", |(text, _)| text.as_str())),
            _ => return Err(unsupported()),
        }
    }
    if args.next().is_some() {
        return Err(unsupported());
    }
    Ok(out)
}

fn here_document(session: &Session<'_>, simple: &Simple, doc: HereDoc) -> Result<Variants, Error> {
    let is_cat = simple.words.len() == 1 && simple.words[0].value.raw == "cat";
    if !is_cat {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "loop",
            simple.span,
            &[("keyword", "<<")],
        ));
    }
    if !doc.quoted && doc.body.contains(['$', '`']) {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "here_document_expansion",
            doc.span,
            &[],
        ));
    }
    let body = if doc.quoted {
        doc.body
    } else {
        doc.body.replace("\\\\", "\\")
    };
    write_text(session, "B39", simple, body)
}

/// `test EXPR` and `[ EXPR ]`: an assertion on the entry.
pub(crate) fn test(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    words: &[Located<Word>],
) -> Result<Variants, Error> {
    let piece = assertion(session, entry, simple, words)?;
    session.record(
        &entry.rule,
        entry.class,
        simple.span,
        piece_text(&piece),
        "habit",
    );
    Ok(Variants(vec![piece]))
}

/// The stages `info PATH |> assert all: CONDITION` of a test of one path.
fn assertion(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    words: &[Located<Word>],
) -> Result<Piece, Error> {
    let mut words: Vec<&Located<Word>> = words.iter().collect();
    if entry.words[0] == "[" {
        if words.last().is_none_or(|word| word.value.raw != "]") {
            return Err(session.error(
                ErrorCode::Uncorrectable,
                "unsupported_test",
                simple.span,
                &[],
            ));
        }
        words.pop();
    }
    let negated = words.first().is_some_and(|word| word.value.raw == "!");
    if negated {
        words.remove(0);
    }
    let [operator, path] = words.as_slice() else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_test",
            simple.span,
            &[],
        ));
    };
    let Some(condition) = session.table.value_str("test", &operator.value.raw) else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_test",
            operator.span,
            &[],
        ));
    };
    let condition = if negated {
        format!("not {condition}")
    } else {
        condition.to_owned()
    };
    let template =
        parse_pipeline(&format!("info $1 |> assert all: {condition}")).map_err(|error| {
            session.error(
                ErrorCode::InternalCorrection,
                "invalid_table",
                simple.span,
                &[("reason", &error.0)],
            )
        })?;
    let mut bindings = Bindings::default();
    bindings.set("1", vec![session.operand(path)?]);
    let stages = fill_pipeline(&template, &bindings, session, Some(simple.span))?;
    Ok(Piece {
        stages,
        kind: "entry".to_owned(),
        why: None,
    })
}

/// `test … && CMD`: the assertion guards the command, which receives the
/// tested entry as input.
pub(crate) fn guard(
    session: &Session<'_>,
    test_command: &Simple,
    next: &syntax::Pipe,
) -> Result<Variants, Error> {
    let name = test_command.words[0].value.raw.clone();
    let Some(entry) = session.table.command(&[name]) else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_test",
            test_command.span,
            &[],
        ));
    };
    let checked = assertion(session, entry, test_command, &test_command.words[1..])?;
    let command = session.pipe(next, &RunOptions::default())?;
    let tested = checked
        .stages
        .first()
        .and_then(|stage| stage.args.first())
        .map(|arg| arg.value.clone());
    let merged = command
        .primary()
        .stages
        .split_first()
        .and_then(|(stage, rest)| {
            if !rest.is_empty() || session.table.value("stage_input", &stage.name).is_none() {
                return None;
            }
            let positional: Vec<usize> = stage
                .args
                .iter()
                .enumerate()
                .filter(|(_, arg)| arg.name.is_none())
                .map(|(i, _)| i)
                .collect();
            let [only] = positional.as_slice() else {
                return None;
            };
            let same = matches!(
                (&stage.args[*only].value, &tested),
                (Expr::Text { v: a, .. }, Some(Expr::Text { v: b, .. })) if a == b
            );
            same.then(|| {
                let mut stage = stage.clone();
                stage.args.remove(*only);
                stage
            })
        });
    let span = syntax::join(test_command.span, next.span);
    match merged {
        Some(stage) => {
            let mut stages = checked.stages;
            stages.push(stage);
            let piece = Piece {
                stages,
                kind: command.primary().kind.clone(),
                why: None,
            };
            session.record(&entry.rule, entry.class, span, piece_text(&piece), "habit");
            Ok(Variants(vec![piece]))
        }
        None => Err(session
            .error(ErrorCode::Uncorrectable, "guard_target", span, &[])
            .with_next_step(session.program_text(&Variants(vec![checked])))
            .with_next_step(session.program_text(&command))),
    }
}

/// `$(CMD)` as a whole operand: `let x = CMD'` and a reference to the field
/// that bash would split into words.
pub(crate) fn substitution(
    session: &Session<'_>,
    body: &str,
    word: &Located<Word>,
) -> Result<Bound, Error> {
    let failure = || {
        session.error(
            ErrorCode::Uncorrectable,
            "command_substitution",
            word.span,
            &[],
        )
    };
    let tokens = lex(body).map_err(|_| failure())?;
    let script = syntax::parse(&tokens).map_err(|_| failure())?;
    let [item] = script.items.as_slice() else {
        return Err(failure());
    };
    if !item.rest.is_empty() {
        return Err(failure());
    }
    let inner = Session::new(session.table, session.context, body);
    let variants = inner.with_directory(session.directory(), || {
        inner.pipe(&item.first, &RunOptions::default())
    })?;
    let piece = variants.primary();
    if session.has_effect(piece) {
        return Err(failure());
    }
    let field = session
        .table
        .value_str("substitution_field", &piece.kind)
        .ok_or_else(failure)?
        .to_owned();
    let name = session.bind_let(pipeline(&piece.stages));
    let reference = Ref {
        name: name.clone(),
        fields: vec![field.clone()],
        span: Some(word.span),
    };
    session.record(
        "B26",
        CorrectionClass::Probable,
        word.span,
        format!("@{name}.{field}"),
        "ref",
    );
    session.assume("B26", "word_splitting");
    Ok(Bound::Node(Expr::Ref(reference)))
}

/// `A | xargs CMD`: the entries of `A` become the input of `CMD`.
pub(crate) fn xargs(
    session: &Session<'_>,
    simple: &Simple,
    previous: Variants,
) -> Result<Variants, Error> {
    let mut words = &simple.words[1..];
    let mut null = false;
    while let Some(first) = words.first() {
        match first.value.raw.as_str() {
            "-0" | "--null" => null = true,
            "-r" | "--no-run-if-empty" => {}
            flag if flag.starts_with('-') => {
                return Err(session.error(
                    ErrorCode::Uncorrectable,
                    "flag_without_equivalent",
                    first.span,
                    &[
                        ("flag", flag),
                        ("command", "xargs"),
                        ("target", "run"),
                        ("params", ""),
                    ],
                ));
            }
            _ => break,
        }
        words = &words[1..];
    }
    let appends_to_target = matches!(
        words.first().map(|word| word.value.raw.as_str()),
        Some("mv" | "cp")
    );
    let has_target = words
        .iter()
        .any(|word| word.value.raw == "-t" || word.value.raw.starts_with("--target-directory"));
    if appends_to_target && !has_target {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_stage",
            simple.span,
            &[("command", "xargs")],
        ));
    }
    let delivery = if null {
        Delivery::AllAtOnce
    } else {
        Delivery::SplitWords
    };
    apply_to_entries(session, simple, words, previous, delivery, simple.span)
}

/// Applies a command to the entries produced by `previous`: a table entry
/// with a stage form becomes that stage, any other program is run with the
/// paths of the entries.
pub(crate) fn apply_to_entries(
    session: &Session<'_>,
    simple: &Simple,
    words: &[Located<Word>],
    previous: Variants,
    delivery: Delivery,
    span: Span,
) -> Result<Variants, Error> {
    let kind = previous.primary().kind.clone();
    if kind != "path_entry" {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_stage",
            span,
            &[("command", "xargs")],
        ));
    }
    if words.is_empty() {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "missing_operand",
            span,
            &[("slot", "command"), ("command", "xargs")],
        ));
    }
    let names: Vec<String> = words
        .iter()
        .map(|word| word.value.literal().unwrap_or_default())
        .collect();
    // Only a stage that takes the entries as its targets receives them as
    // operands; `wc -l` or `head` as a stage would read the entry list itself.
    let staged = session.table.command(&names).filter(|entry| {
        entry.stages.iter().any(|stage| {
            stage.input.contains(&kind)
                && stage
                    .template
                    .first()
                    .is_some_and(|first| session.table.value("stage_input", &first.name).is_some())
        })
    });
    if staged.is_some() {
        let stage = session.words(simple, words, Some(&kind), &RunOptions::default())?;
        let result = previous.then(stage);
        let split = delivery == Delivery::SplitWords;
        if split {
            session.assume("B33", "file_names_without_spaces");
        }
        let class = if split {
            CorrectionClass::Probable
        } else {
            CorrectionClass::Certain
        };
        session.record("B33", class, span, piece_text(result.primary()), "habit");
        return Ok(result);
    }
    let name = session.bind_let(pipeline(&previous.primary().stages));
    let mut run = session.run_program(simple, words, None)?;
    let reference = Expr::Spread {
        x: Box::new(Expr::Ref(Ref {
            name,
            fields: vec!["path".to_owned()],
            span: None,
        })),
        span: None,
    };
    if let Some(Expr::List { items, .. }) = run
        .primary_mut()
        .stages
        .first_mut()
        .and_then(|stage| stage.args.first_mut())
        .map(|arg| &mut arg.value)
    {
        items.push(reference);
    }
    let assumption = match delivery {
        Delivery::SplitWords => Some("file_names_without_spaces"),
        Delivery::OnePerRun => Some("one_run"),
        Delivery::AllAtOnce => None,
    };
    let class = match assumption {
        Some(key) => {
            session.assume("B33", key);
            CorrectionClass::Probable
        }
        None => CorrectionClass::Certain,
    };
    session.record("B33", class, span, piece_text(run.primary()), "habit");
    Ok(run)
}

/// `A | tee [-a] F`: the result of `A` is written to the file.
pub(crate) fn tee(
    session: &Session<'_>,
    simple: &Simple,
    previous: Variants,
) -> Result<Variants, Error> {
    let mut mode = "overwrite";
    let mut files = Vec::new();
    for word in &simple.words[1..] {
        match word.value.raw.as_str() {
            "-a" | "--append" => mode = "append",
            flag if flag.starts_with('-') => {
                return Err(session.error(
                    ErrorCode::Uncorrectable,
                    "flag_without_equivalent",
                    word.span,
                    &[
                        ("flag", flag),
                        ("command", "tee"),
                        ("target", "write"),
                        ("params", "mode"),
                    ],
                ));
            }
            _ => files.push(word.clone()),
        }
    }
    let [target] = files.as_slice() else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "missing_operand",
            simple.span,
            &[("slot", "file"), ("command", "tee")],
        ));
    };
    let output = Output {
        mode,
        target: target.clone(),
        span: simple.span,
    };
    write_from(session, &output, previous)
}

/// `at TIME CMD`: a scheduled program at an absolute time.
pub(crate) fn at(
    session: &Session<'_>,
    entry: &CommandEntry,
    simple: &Simple,
    words: &[Located<Word>],
) -> Result<Variants, Error> {
    let Some((time, command)) = words.split_first().filter(|(_, rest)| !rest.is_empty()) else {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "missing_operand",
            simple.span,
            &[("slot", "command"), ("command", "at")],
        ));
    };
    let time_text = time.value.literal().unwrap_or_default();
    if !is_absolute_time(&time_text) {
        return Err(session.error(
            ErrorCode::Uncorrectable,
            "time_not_absolute",
            time.span,
            &[("time", &time_text)],
        ));
    }
    let action = session.words(simple, command, None, &RunOptions::default())?;
    let label = command[0].value.literal().unwrap_or_default();
    let stage = Stage {
        name: "schedule.create".to_owned(),
        args: vec![
            positional(Expr::Text {
                v: label,
                span: Some(command[0].span),
            }),
            named(
                "at",
                Expr::Text {
                    v: time_text,
                    span: Some(time.span),
                },
            ),
            named(
                "action",
                Expr::Sub {
                    p: pipeline(&action.primary().stages),
                    span: None,
                },
            ),
        ],
        span: Some(simple.span),
    };
    session.record(
        &entry.rule,
        entry.class,
        simple.span,
        print_stage(&stage),
        "habit",
    );
    Ok(Variants::one(vec![stage], "effect"))
}

/// `YYYY-MM-DDTHH:MM` with optional seconds.
fn is_absolute_time(text: &str) -> bool {
    let shape: String = text
        .chars()
        .map(|c| if c.is_ascii_digit() { 'd' } else { c })
        .collect();
    shape == "dddd-dd-ddTdd:dd" || shape == "dddd-dd-ddTdd:dd:dd"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span {
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 2,
        }
    }

    fn words(list: &[&str]) -> Vec<(String, Span)> {
        list.iter().map(|w| ((*w).to_owned(), span())).collect()
    }

    #[test]
    fn echo_flags_and_escapes() {
        assert_eq!(echo(&words(&["a", "b"])), "a b\n");
        assert_eq!(echo(&words(&["-n", "a"])), "a");
        assert_eq!(echo(&words(&["-e", "a\\tb"])), "a\tb\n");
        assert_eq!(echo(&words(&["-ne", "x\\n"])), "x\n");
        assert_eq!(echo(&words(&["a\\n"])), "a\\n\n");
        assert_eq!(echo(&words(&["-x"])), "-x\n");
    }

    #[test]
    fn unescape_keeps_unknown_escapes() {
        assert_eq!(unescape("a\\qb\\"), "a\\qb\\");
        assert_eq!(unescape("\\r\\\\\\\""), "\r\\\"");
    }

    #[test]
    fn absolute_time_shape() {
        assert!(is_absolute_time("2026-11-22T10:00"));
        assert!(is_absolute_time("2026-11-22T10:00:30"));
        assert!(!is_absolute_time("10:00"));
        assert!(!is_absolute_time("now"));
    }
}
