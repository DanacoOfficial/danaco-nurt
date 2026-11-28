// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::powershell
// Responsibility:   Translation of commands written in PowerShell notation into Nurt programs.
// Dependencies:     lex, nurt_contracts, table
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Translation of commands written in PowerShell notation into Nurt
//! programs.
//!
//! A statement that starts with a cmdlet or alias of the translation table,
//! a name of the form `Verb-Noun`, an assignment, the call operator `&`, a
//! variable or a string is read with the PowerShell lexer and translated by
//! the PowerShell translation table. Aliases shared with bash, cmd or Nurt
//! (`ls`, `rm`, `cat`, `curl`, …) are read as PowerShell only when the
//! statement shows PowerShell notation; [`detect`] describes the rule.
//!
//! Every translated cmdlet is reported as a correction with its rule (`P01`
//! to `P42`) and certainty class, and every path written in Windows or
//! registry notation with the path rule `L19`. A translation either keeps
//! the meaning of the command (class `C`) or names its assumption (class
//! `P`); a probable translation of a changing program is not applied in a
//! live call and yields `E_AMBIGUOUS` with ranked candidates. The current
//! directory and the split of a program's argument text are never guessed.
//!
//! Size multipliers of PowerShell are powers of 1024: `1MB` is 1 048 576
//! bytes and becomes `1mib`, `1KB` becomes `1kib`.
//!
//! # Examples
//!
//! ```
//! use nurt_correct::powershell;
//!
//! let translation = powershell::translate("Remove-Item -Path C:\\build\\out -Recurse").expect("a PowerShell habit");
//! assert_eq!(translation.text(), "remove \"C:/build/out\"");
//! assert_eq!(translation.rules[0].rule, "P08");
//! assert_eq!(translation.rules[1].rule, "L19");
//! assert!(powershell::translate("remove \"C:/build/out\"").is_none());
//! ```

mod convert;
mod detect;
pub mod lex;
mod rules;
mod script_block;
mod syntax;
mod table;

use nurt_contracts::ast::{AstDocument, Head, Program, Stage, Statement};
use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::{At, Candidate, Error, ErrorCode};
use nurt_contracts::report::Correction;

pub use crate::bash::{Assumption, Mode, TranslateContext};
pub use lex::lex;
pub use table::{HABITS_TABLE_NAME, PowerShellTable, TABLE_NAME};

use self::lex::Positions;
use self::rules::{AlternativeStage, Session, TranslatedStatement, program_text};
use crate::policy::Class;
use crate::record::Log;

/// Largest number of candidates and alternatives offered for a probable
/// translation.
const MAX_ALTERNATIVES: usize = 3;

/// Help topic of corrections.
const HELP_CORRECTIONS: &str = "help corrections";

/// A `let` name with the record type of its value.
///
/// The record type tells which PowerShell properties a later statement may
/// read from `$name`; a caller that translates a script statement by
/// statement passes the bindings of earlier statements to
/// [`translate_with_lets`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LetBinding {
    /// Name of the result, without `$` or `@`.
    pub name: String,
    /// Record type of the value (`entry`, `line`, `process`, `service`, …);
    /// `None` when it is not known.
    pub record: Option<String>,
}

/// Result of a translation.
#[derive(Debug, Clone, PartialEq)]
pub struct Translation {
    /// The translated program as a surface tree; spans point into the input.
    pub tree: AstDocument,
    /// Corrections applied, in source order and numbered `C1`, `C2`, ….
    pub rules: Vec<Correction>,
    /// Assumptions of probable translations.
    pub assumptions: Vec<Assumption>,
    /// Other interpretations of a probable translation, most likely first;
    /// at most three.
    pub alternatives: Vec<AstDocument>,
    /// Parameters removed by the translation, with the reason.
    pub notes: Vec<Assumption>,
    /// Call parameters set by the translation, such as `preview` for
    /// `-WhatIf`.
    pub options: RequestOptions,
    /// `let` names bound by the translated text, in source order, with the
    /// record type of their values.
    pub bindings: Vec<LetBinding>,
}

impl Translation {
    /// Returns the canonical text of the translated program.
    ///
    /// # Examples
    ///
    /// ```
    /// let translation = nurt_correct::powershell::translate("Get-Process").expect("a PowerShell habit");
    /// assert_eq!(translation.text(), "processes");
    /// ```
    #[must_use]
    pub fn text(&self) -> String {
        program_text(&self.tree)
    }
}

/// Whether the text is a command written in PowerShell notation rather than
/// a Nurt program or a command of another shell.
///
/// # Examples
///
/// ```
/// use nurt_correct::powershell::detect;
///
/// assert!(detect("Get-ChildItem C:\\proj -Recurse"));
/// assert!(detect("ls -Recurse C:\\proj"));
/// assert!(!detect("ls -la /srv"));
/// assert!(!detect("find \"/srv\" glob: \"*.go\""));
/// ```
#[must_use]
pub fn detect(input: &str) -> bool {
    let Ok(table) = PowerShellTable::embedded() else {
        return false;
    };
    lex(input)
        .ok()
        .and_then(|tokens| syntax::parse(&tokens).ok())
        .is_some_and(|script| detect::is_habit(table, &script))
}

/// Translates a PowerShell command in a live call without `base` and
/// without known `let` names.
///
/// Returns `None` when the text is not a PowerShell habit or when the
/// translation ends with an error; [`translate_with`] returns the error.
///
/// # Examples
///
/// ```
/// let translation = nurt_correct::powershell::translate("Get-Service -Name sshd").expect("a PowerShell habit");
/// assert_eq!(translation.text(), "services name: \"sshd\"");
/// ```
#[must_use]
pub fn translate(input: &str) -> Option<Translation> {
    translate_with(input, &TranslateContext::default()).and_then(Result::ok)
}

/// Translates a PowerShell command with the embedded table.
///
/// Returns `None` when the text is not a PowerShell habit; in script mode
/// every statement is translated.
///
/// # Errors
///
/// The inner result is an [`Error`] with the code `E_UNCORRECTABLE`,
/// `E_ONE_EFFECT`, `E_AMBIGUOUS` or `E_PATH_RELATIVE`, with ready fixes,
/// next steps or candidates where they exist; further errors of other
/// statements are listed in the detail `errors`. Text that cannot be read in
/// script mode yields `E_SYNTAX`; an invalid embedded table yields
/// `E_INTERNAL_CORRECTION`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::error::ErrorCode;
/// use nurt_correct::powershell::{TranslateContext, translate_with};
///
/// let error = translate_with("Set-Location C:\\proj", &TranslateContext::default())
///     .expect("a PowerShell habit")
///     .expect_err("the current directory is not changed");
/// assert_eq!(error.code, ErrorCode::Uncorrectable);
/// assert_eq!(error.problem.as_deref(), Some("no_current_directory"));
/// ```
#[must_use]
pub fn translate_with(
    input: &str,
    context: &TranslateContext<'_>,
) -> Option<Result<Translation, Error>> {
    match PowerShellTable::embedded() {
        Ok(table) => translate_with_table(input, context, table),
        Err(error) => Some(Err(Error::new(
            ErrorCode::InternalCorrection,
            format!("invalid PowerShell translation table: {error}"),
        ))),
    }
}

/// Translates a PowerShell command with the given table.
///
/// Returns `None` when the text is not a PowerShell habit.
///
/// # Errors
///
/// As [`translate_with`].
#[must_use]
pub fn translate_with_table(
    input: &str,
    context: &TranslateContext<'_>,
    table: &PowerShellTable,
) -> Option<Result<Translation, Error>> {
    translate_with_lets(input, context, &[], table)
}

/// Translates a PowerShell command with the given table and with `let`
/// names whose record types are known.
///
/// The names of `known` are added to the names of `context.lets`; a
/// statement such as `$logs | Where-Object { $_.Length -gt 1MB }` reads the
/// properties of the record type bound to `$logs`. Returns `None` when the
/// text is not a PowerShell habit.
///
/// # Errors
///
/// As [`translate_with`].
///
/// # Examples
///
/// ```
/// use nurt_correct::powershell::{
///     LetBinding, Mode, PowerShellTable, TranslateContext, translate_with_lets,
/// };
///
/// let table = PowerShellTable::embedded().expect("the embedded table");
/// let context = TranslateContext { mode: Mode::Script, ..TranslateContext::default() };
/// let first = translate_with_lets("$logs = gci C:\\logs", &context, &[], table)
///     .expect("a PowerShell habit")
///     .expect("a translation");
/// assert_eq!(first.bindings, vec![LetBinding { name: "logs".to_owned(), record: Some("entry".to_owned()) }]);
/// let second = translate_with_lets("$logs | Where-Object { $_.Length -gt 1MB }", &context, &first.bindings, table)
///     .expect("a PowerShell habit")
///     .expect("a translation");
/// assert_eq!(second.text(), "@logs |> filter size > 1mib");
/// ```
#[must_use]
pub fn translate_with_lets(
    input: &str,
    context: &TranslateContext<'_>,
    known: &[LetBinding],
    table: &PowerShellTable,
) -> Option<Result<Translation, Error>> {
    let script_mode = context.mode == Mode::Script;
    let tokens = match lex(input) {
        Ok(tokens) => tokens,
        Err(error) if script_mode => {
            return Some(Err(syntax_error(
                input,
                error.offset,
                error.offset,
                &error.to_string(),
            )));
        }
        Err(_) => return None,
    };
    let script = match syntax::parse(&tokens) {
        Ok(script) => script,
        Err(error) if script_mode => {
            return Some(Err(syntax_error(
                input,
                error.start,
                error.end,
                &error.message,
            )));
        }
        Err(_) => return None,
    };
    if script.statements.is_empty() || (!script_mode && !detect::is_habit(table, &script)) {
        return None;
    }
    Some(translate_script(input, context, table, &script, known))
}

fn syntax_error(input: &str, start: usize, end: usize, message: &str) -> Error {
    let positions = Positions::new(input);
    let (line, column) = positions.at(input, start);
    let found = input.get(start..end).unwrap_or_default();
    let length = u32::try_from(found.chars().count()).unwrap_or(u32::MAX);
    Error::new(
        ErrorCode::Syntax,
        format!("the PowerShell text cannot be read: {message}"),
    )
    .with_at(At::new(line, column, length))
    .with_found(found)
    .with_help(HELP_CORRECTIONS)
}

fn translate_script(
    input: &str,
    context: &TranslateContext<'_>,
    table: &PowerShellTable,
    script: &syntax::Script,
    known: &[LetBinding],
) -> Result<Translation, Error> {
    let live = context.mode != Mode::Script;
    let mut session = Session::new(table, context, input);
    session.know_lets(known);
    let known_count = session.let_count();
    let mut statements = Vec::new();
    let mut errors = Vec::new();
    for (index, statement) in script.statements.iter().enumerate() {
        match session.statement(statement, index) {
            Ok(translated) => statements.push(translated),
            Err(error) if live => errors.push(error),
            Err(error) => return Err(error),
        }
    }
    if !errors.is_empty() {
        return Err(first_error(errors));
    }
    if live {
        check_one_pipeline(input, &statements)?;
    }
    let effect = statements.iter().any(|statement| statement.effect);
    let probable = session
        .corrections
        .iter()
        .any(|correction| correction.class == Class::Probable);
    let tree = AstDocument::new(Program {
        statements: statements
            .into_iter()
            .map(|statement| statement.statement)
            .collect(),
    });
    let alternatives = alternative_documents(&tree, &session.alternatives);
    if live && probable && effect {
        return Err(ambiguous(input, &tree, &alternatives, &session.assumptions));
    }
    let mut corrections = std::mem::take(&mut session.corrections);
    corrections.sort_by_key(|correction| correction.at.map(|at| (at.line, at.column)));
    let mut log = Log::new();
    for correction in corrections {
        log.push(correction);
    }
    Ok(Translation {
        tree,
        rules: log.into_report(),
        assumptions: std::mem::take(&mut session.assumptions),
        alternatives,
        notes: std::mem::take(&mut session.notes),
        options: std::mem::take(&mut session.options),
        bindings: session.lets_since(known_count),
    })
}

/// The first error; the others are listed in its detail `errors`.
fn first_error(errors: Vec<Error>) -> Error {
    let mut errors = errors.into_iter();
    let Some(first) = errors.next() else {
        return Error::new(
            ErrorCode::InternalCorrection,
            "translation failed without an error",
        );
    };
    let rest: Vec<serde_json::Value> = errors
        .filter_map(|error| serde_json::to_value(&error).ok())
        .collect();
    if rest.is_empty() {
        first
    } else {
        first.with_detail("errors", serde_json::Value::Array(rest))
    }
}

/// A live call runs one pipeline: two pipelines yield their separate
/// programs as next steps, each with the `let` statements before it.
fn check_one_pipeline(input: &str, statements: &[TranslatedStatement]) -> Result<(), Error> {
    let pipelines = statements
        .iter()
        .filter(|statement| matches!(statement.statement, Statement::Pipeline(_)))
        .count();
    if pipelines <= 1 {
        return Ok(());
    }
    let mut lets = Vec::new();
    let mut error = Error::new(
        ErrorCode::OneEffect,
        "one call runs one pipeline; run the programs one after another",
    )
    .with_at(whole_input(input))
    .with_help(HELP_CORRECTIONS);
    let whole = AstDocument::new(Program {
        statements: statements
            .iter()
            .map(|statement| statement.statement.clone())
            .collect(),
    });
    let mut steps = String::from("steps {\n");
    for line in program_text(&whole).lines() {
        steps.push_str("    ");
        steps.push_str(line);
        steps.push('\n');
    }
    steps.push('}');
    error = error.with_candidate(Candidate {
        rank: 1,
        reference: None,
        program: steps,
        why: None,
    });
    for statement in statements {
        match &statement.statement {
            Statement::Let(_) => lets.push(statement.statement.clone()),
            Statement::Pipeline(_) => {
                let mut program = lets.clone();
                program.push(statement.statement.clone());
                error = error.with_next_step(program_text(&AstDocument::new(Program {
                    statements: program,
                })));
            }
            _ => {}
        }
    }
    Err(error)
}

fn whole_input(input: &str) -> At {
    At::new(
        1,
        1,
        u32::try_from(input.chars().count()).unwrap_or(u32::MAX),
    )
}

/// `E_AMBIGUOUS` for a probable translation of a changing program: the
/// translation and its alternatives as ranked candidates.
fn ambiguous(
    input: &str,
    tree: &AstDocument,
    alternatives: &[AstDocument],
    assumptions: &[Assumption],
) -> Error {
    let why = assumptions
        .iter()
        .map(|assumption| assumption.text.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    let mut error = Error::new(
        ErrorCode::Ambiguous,
        "the translation of a changing command makes an assumption; choose a candidate",
    )
    .with_at(whole_input(input))
    .with_help(HELP_CORRECTIONS);
    let programs = std::iter::once(tree)
        .chain(alternatives.iter())
        .take(MAX_ALTERNATIVES);
    for (rank, document) in (1_u32..).zip(programs) {
        error = error.with_candidate(Candidate {
            rank,
            reference: None,
            program: program_text(document),
            why: (rank == 1 && !why.is_empty()).then(|| why.clone()),
        });
    }
    error
}

/// Programs in which one stage is replaced by its alternative.
fn alternative_documents(
    tree: &AstDocument,
    alternatives: &[AlternativeStage],
) -> Vec<AstDocument> {
    alternatives
        .iter()
        .filter_map(|alternative| replace_stage(tree, alternative))
        .take(MAX_ALTERNATIVES)
        .collect()
}

fn replace_stage(tree: &AstDocument, alternative: &AlternativeStage) -> Option<AstDocument> {
    let mut document = tree.clone();
    let pipeline = match document.program.statements.get_mut(alternative.statement)? {
        Statement::Pipeline(pipeline) => pipeline,
        Statement::Let(binding) => &mut binding.value,
        _ => return None,
    };
    let slot: &mut Stage = match alternative.position {
        0 => match &mut pipeline.head {
            Head::Stage(stage) => stage,
            _ => return None,
        },
        position => pipeline.stages.get_mut(position - 1)?,
    };
    *slot = alternative.stage.clone();
    Some(document)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
