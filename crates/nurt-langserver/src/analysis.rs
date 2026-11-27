// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-langserver::analysis
// Responsibility:   Diagnostics, quick fixes and formatting of a document.
// Dependencies:     ScalarType, lsp_types, nurt_bind, nurt_contracts, nurt_correct, nurt_syntax, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Diagnostics, quick fixes and formatting of a document.
//!
//! A document is read with the correction of notation and, when the session
//! has a binding catalog, bound with the correction of names and values.
//! Each correction becomes a diagnostic with its rule as the code; the
//! error that stops reading or binding becomes a diagnostic with its error
//! code, ready fixes and candidates. Quick fixes replace the corrected text,
//! write the corrected program or apply a fix or a candidate.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::ops::Range as Bytes;

use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Diagnostic, DiagnosticSeverity,
    NumberOrString, Range, TextEdit, Uri, WorkspaceEdit,
};
use nurt_bind::{BindContext, Catalog, bind_with_correction};
use nurt_contracts::config::Config;
use nurt_contracts::error::{At, Error, ErrorCode};
use nurt_contracts::spec::{ParamType, ScalarType};
use nurt_correct::lexical::{ArgumentShape, CatalogView, front};
use nurt_correct::policy::{Class, is_effecting};
use nurt_correct::record::Correction;
use nurt_correct::tables::Tables;
use nurt_syntax::error::SyntaxError;
use nurt_syntax::fmt;
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use nurt_syntax::print::try_print;
use serde_json::{Value, json};

use crate::catalog::{Commands, tag};
use crate::position::{Encoding, Lines};

/// Source name of every diagnostic.
pub(crate) const SOURCE: &str = "nurt";

/// Problem of a text without statements; an empty document is not an error
/// while it is being written.
const EMPTY_PROGRAM: &str = "empty_program";

/// Title of the quick fix that writes the program with every correction.
const CORRECTED_PROGRAM: &str = "Write the corrected program";

/// What a document is read and bound with.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Engine<'a> {
    /// Syntax variant of the document.
    pub(crate) variant: Variant,
    /// Position encoding of the session.
    pub(crate) encoding: Encoding,
    /// Settings of correction and of the tolerant syntax.
    pub(crate) config: &'a Config,
    /// Correction tables; without them only the parser reports errors.
    pub(crate) tables: Option<&'a Tables>,
    /// Catalog that the program is bound to.
    pub(crate) catalog: Option<&'a Catalog>,
    /// Command signatures, for the correction of notation without a
    /// binding catalog.
    pub(crate) commands: &'a Commands,
    /// Context of binding.
    pub(crate) context: &'a BindContext,
}

/// Diagnostics of a document and their quick fixes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Analysis {
    /// Diagnostics in the order of the corrections, or the one error.
    pub(crate) diagnostics: Vec<Diagnostic>,
    fixes: Vec<Fix>,
}

/// Quick fix of a diagnostic.
#[derive(Debug, Clone)]
struct Fix {
    title: String,
    diagnostic: Diagnostic,
    edit: TextEdit,
    preferred: bool,
}

/// Corrections of a program that was read and the corrected program.
#[derive(Debug, Default)]
struct Reading {
    corrections: Vec<Correction>,
    canonical: Option<String>,
}

impl Engine<'_> {
    /// Reads and binds a document and returns its diagnostics.
    pub(crate) fn analyze(&self, text: &str) -> Analysis {
        let lines = Lines::new(text);
        let mut analysis = Analysis::default();
        match self.read(text) {
            Ok(reading) => analysis.add_corrections(text, &lines, reading, self.encoding),
            Err(error) if is_empty_program(error.problem.as_deref()) => {}
            Err(error) => analysis.add_error(text, &lines, &error, self.variant, self.encoding),
        }
        analysis
    }

    fn read(&self, text: &str) -> Result<Reading, Error> {
        let Some(tables) = self.tables else {
            return parse(text, self.variant, &ParseOptions::default())
                .map(|_| Reading::default())
                .map_err(Error::from);
        };
        if let Some(catalog) = self.catalog {
            let outcome = bind_with_correction(
                text,
                self.variant,
                catalog,
                tables,
                self.config,
                self.context,
            )?;
            return Ok(Reading {
                corrections: outcome.corrections,
                canonical: Some(outcome.canonical),
            });
        }
        let mut syntax = self.config.syntax.clone();
        syntax.variant = self.variant;
        let view = View::new(self.commands);
        let read = front(text, &syntax, &self.config.correction, tables, &view)?;
        let canonical = try_print(&read.document, self.variant)
            .ok()
            .map(|printed| printed.trim_end().to_owned());
        Ok(Reading {
            corrections: read.corrections,
            canonical,
        })
    }
}

impl Analysis {
    fn add_corrections(
        &mut self,
        text: &str,
        lines: &Lines<'_>,
        reading: Reading,
        encoding: Encoding,
    ) {
        if reading.corrections.is_empty() {
            return;
        }
        let whole = reading.canonical.map(|canonical| {
            TextEdit::new(
                lines.range(0..text.len(), encoding),
                program_text(text, &canonical),
            )
        });
        for correction in &reading.corrections {
            let bytes = correction
                .at
                .map(|at| span(lines, at.line, at.column, at.length));
            let range = bytes
                .clone()
                .map_or_else(Range::default, |bytes| lines.range(bytes, encoding));
            let diagnostic = correction_diagnostic(correction, range);
            if let Some(bytes) = bytes
                && text.get(bytes.clone()) == Some(correction.from.as_str())
            {
                self.fixes.push(Fix {
                    title: format!("Replace `{}` with `{}`", correction.from, correction.to),
                    diagnostic: diagnostic.clone(),
                    edit: TextEdit::new(lines.range(bytes, encoding), correction.to.clone()),
                    preferred: false,
                });
            }
            if let Some(edit) = &whole {
                self.fixes.push(Fix {
                    title: CORRECTED_PROGRAM.to_owned(),
                    diagnostic: diagnostic.clone(),
                    edit: edit.clone(),
                    preferred: true,
                });
            }
            self.diagnostics.push(diagnostic);
        }
    }

    fn add_error(
        &mut self,
        text: &str,
        lines: &Lines<'_>,
        error: &Error,
        variant: Variant,
        encoding: Encoding,
    ) {
        let bytes = error_bytes(lines, error.at);
        let range = bytes
            .clone()
            .map_or_else(Range::default, |bytes| lines.range(bytes, encoding));
        let severity = if error.code() == ErrorCode::Ambiguous {
            DiagnosticSeverity::WARNING
        } else {
            DiagnosticSeverity::ERROR
        };
        let diagnostic = Diagnostic {
            range,
            severity: Some(severity),
            code: Some(NumberOrString::String(error.code().as_str().to_owned())),
            source: Some(SOURCE.to_owned()),
            message: error_message(error),
            data: error.help.as_ref().map(|topic| json!({ "help": topic })),
            ..Diagnostic::default()
        };
        let lexical = matches!(error.code(), ErrorCode::Lexical | ErrorCode::Escape);
        for (index, program) in error.fix.iter().enumerate() {
            let (edit, local) = fix_edit(
                text,
                lines,
                bytes.clone(),
                program,
                lexical,
                variant,
                encoding,
            );
            let title = if local {
                format!("Replace with `{program}`")
            } else {
                format!("Apply fix: {}", first_line(program))
            };
            self.fixes.push(Fix {
                title,
                diagnostic: diagnostic.clone(),
                edit,
                preferred: index == 0,
            });
        }
        let whole = lines.range(0..text.len(), encoding);
        for candidate in &error.candidates {
            self.fixes.push(Fix {
                title: format!(
                    "Use candidate {}: {}",
                    candidate.rank,
                    first_line(&candidate.program)
                ),
                diagnostic: diagnostic.clone(),
                edit: TextEdit::new(whole, program_text(text, &candidate.program)),
                preferred: false,
            });
        }
        self.diagnostics.push(diagnostic);
    }

    /// Returns the quick fixes of the diagnostics that overlap `range`,
    /// each title once.
    pub(crate) fn actions(&self, uri: &Uri, range: Range) -> Vec<CodeActionOrCommand> {
        let mut titles: Vec<&str> = Vec::new();
        let mut actions = Vec::new();
        for fix in &self.fixes {
            let found = fix.diagnostic.range;
            let overlaps = found.start <= range.end && range.start <= found.end;
            if !overlaps || titles.contains(&fix.title.as_str()) {
                continue;
            }
            titles.push(&fix.title);
            // The protocol type `WorkspaceEdit::changes` is keyed by `Uri`, which the lint flags.
            #[allow(clippy::mutable_key_type)]
            let changes = HashMap::from([(uri.clone(), vec![fix.edit.clone()])]);
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: fix.title.clone(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![fix.diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..WorkspaceEdit::default()
                }),
                is_preferred: Some(fix.preferred),
                ..CodeAction::default()
            }));
        }
        actions
    }
}

/// Diagnostic of a correction: the rule is the code, and the message is
/// the line of the correction report, `C1 L03 C: Find → find (command)`.
fn correction_diagnostic(correction: &Correction, range: Range) -> Diagnostic {
    let class = tag(serde_json::to_value(correction.class));
    let mut message = String::new();
    if !correction.id.is_empty() {
        let _ = write!(message, "{} ", correction.id);
    }
    let _ = write!(
        message,
        "{} {class}: {} → {} ({})",
        correction.rule, correction.from, correction.to, correction.kind
    );
    if !correction.alternatives.is_empty() {
        let _ = write!(message, "; also: {}", correction.alternatives.join(", "));
    }
    let severity = if correction.class == Class::Certain {
        DiagnosticSeverity::HINT
    } else {
        DiagnosticSeverity::INFORMATION
    };
    Diagnostic {
        range,
        severity: Some(severity),
        code: Some(NumberOrString::String(correction.rule.to_string())),
        source: Some(SOURCE.to_owned()),
        message,
        data: Some(json!({
            "id": correction.id,
            "class": class,
            "kind": correction.kind.as_str(),
            "from": correction.from,
            "to": correction.to,
        })),
        ..Diagnostic::default()
    }
}

/// Message of an error with what was expected, the fixes, the candidates
/// and the next steps, one per line.
fn error_message(error: &Error) -> String {
    let mut message = error.message.clone();
    match &error.expected {
        Some(Value::Array(items)) => {
            let shown: Vec<String> = items.iter().map(shown).collect();
            let _ = write!(message, "\nexpected: {}", shown.join(", "));
        }
        Some(other) => {
            let _ = write!(message, "\nexpected: {}", shown(other));
        }
        None => {}
    }
    for fix in &error.fix {
        let _ = write!(message, "\nfix: {fix}");
    }
    for candidate in &error.candidates {
        let _ = write!(
            message,
            "\ncandidate {}: {}",
            candidate.rank, candidate.program
        );
    }
    for step in &error.next_steps {
        let _ = write!(message, "\nnext step: {step}");
    }
    message
}

fn shown(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Edit of a fix and whether it replaces only the text the error points at.
///
/// A fix of the lexer replaces the fragment at the error and a fix of the
/// parser is the whole program; the reading that parses is taken, and the
/// one of the error code when both or neither do.
fn fix_edit(
    text: &str,
    lines: &Lines<'_>,
    bytes: Option<Bytes<usize>>,
    program: &str,
    lexical: bool,
    variant: Variant,
    encoding: Encoding,
) -> (TextEdit, bool) {
    let whole = TextEdit::new(
        lines.range(0..text.len(), encoding),
        program_text(text, program),
    );
    let Some(bytes) = bytes else {
        return (whole, false);
    };
    let mut replaced = text.to_owned();
    replaced.replace_range(bytes.clone(), program);
    let local = match (parses(&replaced, variant), parses(&whole.new_text, variant)) {
        (true, false) => true,
        (false, true) => false,
        _ => lexical,
    };
    if local {
        (
            TextEdit::new(lines.range(bytes, encoding), program.to_owned()),
            true,
        )
    } else {
        (whole, false)
    }
}

fn parses(text: &str, variant: Variant) -> bool {
    parse(text, variant, &ParseOptions::default()).is_ok()
}

/// A program that replaces `text`, ending with a line break when `text`
/// does.
fn program_text(text: &str, program: &str) -> String {
    if text.ends_with('\n') && !program.ends_with('\n') {
        format!("{program}\n")
    } else {
        program.to_owned()
    }
}

/// The first line of a program, with `…` when more lines follow.
fn first_line(program: &str) -> String {
    let mut lines = program.lines();
    let first = lines.next().unwrap_or_default();
    if lines.next().is_some() {
        format!("{first} …")
    } else {
        first.to_owned()
    }
}

/// Byte range of a position of a correction report.
fn span(lines: &Lines<'_>, line: u32, column: u32, length: Option<u32>) -> Bytes<usize> {
    let start = lines.column_offset(line, column);
    start..lines.advance(start, length.unwrap_or(0))
}

/// Byte range that an error points at, if it has a position.
fn error_bytes(lines: &Lines<'_>, at: Option<At>) -> Option<Bytes<usize>> {
    let at = at?;
    let (line, column) = at.line.zip(at.column)?;
    Some(span(lines, line, column, at.length))
}

fn is_empty_program(problem: Option<&str>) -> bool {
    problem == Some(EMPTY_PROGRAM)
}

/// Formats a document into its canonical text.
///
/// Returns no edit when the text is canonical or empty, and one edit of the
/// whole document otherwise.
///
/// # Errors
///
/// Returns a message with the position of the syntax error when the text is
/// not a valid program.
pub(crate) fn formatting(
    text: &str,
    variant: Variant,
    encoding: Encoding,
) -> Result<Vec<TextEdit>, String> {
    let formatted = match fmt::format(text, variant) {
        Ok(formatted) => formatted,
        Err(error) if is_empty_program(error.problem.as_deref()) => return Ok(Vec::new()),
        Err(error) => return Err(format_failure(&error)),
    };
    if formatted == text {
        return Ok(Vec::new());
    }
    let range = Lines::new(text).range(0..text.len(), encoding);
    Ok(vec![TextEdit::new(range, formatted)])
}

fn format_failure(error: &SyntaxError) -> String {
    match error.at.and_then(|at| at.line.zip(at.column)) {
        Some((line, column)) => {
            format!(
                "cannot format the document: {} at line {line}, column {column}",
                error.message
            )
        }
        None => format!("cannot format the document: {}", error.message),
    }
}

/// The command signatures as seen by the correction of notation.
struct View<'a> {
    commands: &'a Commands,
    names: Vec<&'a str>,
}

impl<'a> View<'a> {
    fn new(commands: &'a Commands) -> Self {
        Self {
            commands,
            names: commands.iter().map(|spec| spec.name.as_str()).collect(),
        }
    }
}

impl CatalogView for View<'_> {
    fn is_command(&self, name: &str) -> bool {
        self.commands.get(name).is_some()
    }

    fn command_names(&self) -> &[&str] {
        &self.names
    }

    fn is_mutating(&self, name: &str) -> bool {
        self.commands.get(name).is_some_and(is_effecting)
    }

    fn can_take_argument(&self, command: &str, position: usize, shape: ArgumentShape) -> bool {
        self.commands.get(command).is_some_and(|spec| {
            spec.positional()
                .get(position)
                .is_some_and(|param| accepts(&param.ty, shape))
        })
    }
}

/// Whether a value of the shape fits a place of the type.
fn accepts(ty: &ParamType, shape: ArgumentShape) -> bool {
    match ty {
        ParamType::Scalar(scalar) => scalar_accepts(*scalar, shape),
        ParamType::Enum(_) => shape == ArgumentShape::Word,
        ParamType::List(inner) => accepts(inner, shape),
        ParamType::Union(alternatives) => alternatives
            .iter()
            .any(|alternative| accepts(alternative, shape)),
        _ => false,
    }
}

fn scalar_accepts(scalar: ScalarType, shape: ArgumentShape) -> bool {
    use ScalarType as S;
    if matches!(scalar, S::Any | S::Expression) {
        return true;
    }
    match shape {
        ArgumentShape::Word => matches!(
            scalar,
            S::Field | S::Pipeline | S::NamedExpression | S::Bool
        ),
        ArgumentShape::Text => matches!(
            scalar,
            S::Text
                | S::Bytes
                | S::Path
                | S::Glob
                | S::Key
                | S::Instant
                | S::Percent
                | S::Size
                | S::Duration
        ),
        ArgumentShape::Regex => scalar == S::Regex,
        ArgumentShape::Number => matches!(
            scalar,
            S::Int | S::Number | S::Size | S::Duration | S::Range | S::Percent
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use lsp_types::Position;
    use nurt_bind::BindContext;
    use nurt_contracts::error::{Candidate, Stage};
    use nurt_correct::tables::load_embedded;

    use super::*;
    use crate::catalog::tests::sample;

    struct Setup {
        config: Config,
        tables: Tables,
        commands: Commands,
        context: BindContext,
    }

    impl Setup {
        fn new() -> Self {
            Self {
                config: Config::default(),
                tables: load_embedded().unwrap(),
                commands: sample(),
                context: BindContext::for_host(),
            }
        }

        fn engine(&self, encoding: Encoding) -> Engine<'_> {
            Engine {
                variant: Variant::A,
                encoding,
                config: &self.config,
                tables: Some(&self.tables),
                catalog: None,
                commands: &self.commands,
                context: &self.context,
            }
        }
    }

    fn uri() -> Uri {
        "file:///work/a.nurt".parse().unwrap()
    }

    fn whole() -> Range {
        Range::new(Position::new(0, 0), Position::new(99, 0))
    }

    /// Titles and new texts of the quick fixes in `range`.
    fn edits(analysis: &Analysis, range: Range) -> Vec<(String, String)> {
        analysis
            .actions(&uri(), range)
            .into_iter()
            .map(|action| {
                let CodeActionOrCommand::CodeAction(action) = action else {
                    panic!("expected a code action")
                };
                #[allow(
                    clippy::mutable_key_type,
                    reason = "the map type is fixed by the protocol crate"
                )]
                let changes = action.edit.unwrap().changes.unwrap();
                (action.title, changes[&uri()][0].new_text.clone())
            })
            .collect()
    }

    #[test]
    fn reports_no_diagnostics_for_valid_and_empty_programs() {
        let setup = Setup::new();
        let engine = setup.engine(Encoding::Utf16);
        for text in ["find \"/srv\" |> count\n", "", "# only a comment\n"] {
            assert!(engine.analyze(text).diagnostics.is_empty(), "{text:?}");
        }
    }

    #[test]
    fn reports_corrections_with_rule_class_and_replaced_text() {
        let setup = Setup::new();
        let analysis = setup
            .engine(Encoding::Utf16)
            .analyze("Count |> sort size\n");
        let [diagnostic] = analysis.diagnostics.as_slice() else {
            panic!("expected one diagnostic: {:?}", analysis.diagnostics)
        };
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("L03".to_owned()))
        );
        assert!(
            diagnostic.message.starts_with("C1 L03 "),
            "{}",
            diagnostic.message
        );
        assert!(diagnostic.message.contains("Count → count"));
        assert_eq!(
            diagnostic.range,
            Range::new(Position::new(0, 0), Position::new(0, 5))
        );
        assert_eq!(diagnostic.data.as_ref().unwrap()["to"], "count");
        let fixes = edits(&analysis, diagnostic.range);
        assert_eq!(
            fixes[0],
            (
                "Replace `Count` with `count`".to_owned(),
                "count".to_owned()
            )
        );
        assert_eq!(fixes[1].0, CORRECTED_PROGRAM);
        assert_eq!(fixes[1].1, "count |> sort size\n");
        let elsewhere = Range::new(Position::new(3, 0), Position::new(3, 1));
        assert!(analysis.actions(&uri(), elsewhere).is_empty());
    }

    #[test]
    fn reports_errors_of_reading_with_their_fix() {
        let setup = Setup::new();
        let analysis = setup
            .engine(Encoding::Utf16)
            .analyze("count\n|> filter a < b < c\n");
        let [diagnostic] = analysis.diagnostics.as_slice() else {
            panic!("expected one diagnostic: {:?}", analysis.diagnostics)
        };
        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
        assert!(
            diagnostic.message.contains("\nfix: "),
            "{}",
            diagnostic.message
        );
        let fixes = edits(&analysis, whole());
        let [(title, program)] = fixes.as_slice() else {
            panic!("expected one fix: {fixes:?}")
        };
        assert!(title.starts_with("Apply fix: count"), "{title}");
        assert!(program.ends_with("a < b and b < c\n"), "{program}");
    }

    #[test]
    fn reads_strictly_when_correction_is_disabled() {
        let mut setup = Setup::new();
        setup.config.correction.enabled = false;
        let analysis = setup.engine(Encoding::Utf16).analyze("Count\n");
        let [diagnostic] = analysis.diagnostics.as_slice() else {
            panic!("expected one diagnostic: {:?}", analysis.diagnostics)
        };
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("E_LEXICAL".to_owned()))
        );
    }

    #[test]
    fn reports_only_errors_of_the_parser_without_tables() {
        let setup = Setup::new();
        let engine = Engine {
            tables: None,
            ..setup.engine(Encoding::Utf32)
        };
        assert!(engine.analyze("Count |> sort size").diagnostics.len() == 1);
        assert!(engine.analyze("count |> sort size").diagnostics.is_empty());
        let text = "count \"\u{1f600}\" )";
        let utf16 = Engine {
            encoding: Encoding::Utf16,
            ..engine
        };
        assert_eq!(
            engine.analyze(text).diagnostics[0].range.start,
            Position::new(0, 10)
        );
        assert_eq!(
            utf16.analyze(text).diagnostics[0].range.start,
            Position::new(0, 11)
        );
    }

    #[test]
    fn reports_ambiguity_as_a_warning_with_candidates() {
        let text = "remove \"/srv/a\"\n";
        let lines = Lines::new(text);
        let mut error = Error::new(ErrorCode::Ambiguous, "two readings")
            .with_stage(Stage::Bind)
            .with_at(At {
                line: Some(1),
                column: Some(8),
                length: Some(8),
                stage: Some(Stage::Bind),
            })
            .with_candidate(Candidate {
                rank: 1,
                reference: None,
                program: "remove \"/srv/a\" recursive: true".to_owned(),
                why: None,
            });
        error.expected = Some(json!(["file", 2]));
        error.next_steps.push("list \"/srv/a\"".to_owned());
        let mut analysis = Analysis::default();
        analysis.add_error(text, &lines, &error, Variant::A, Encoding::Utf16);
        let diagnostic = &analysis.diagnostics[0];
        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(
            diagnostic.message,
            "two readings\nexpected: file, 2\ncandidate 1: remove \"/srv/a\" recursive: true\nnext step: list \"/srv/a\""
        );
        assert_eq!(
            diagnostic.range,
            Range::new(Position::new(0, 7), Position::new(0, 15))
        );
        assert_eq!(
            edits(&analysis, whole()),
            [(
                "Use candidate 1: remove \"/srv/a\" recursive: true".to_owned(),
                "remove \"/srv/a\" recursive: true\n".to_owned()
            )]
        );
    }

    #[test]
    fn applies_lexical_fixes_to_the_fragment_at_the_error() {
        let text = "find '/srv'\n";
        let lines = Lines::new(text);
        let mut error = Error::new(ErrorCode::Lexical, "texts are written in double quotes")
            .with_at(At {
                line: Some(1),
                column: Some(6),
                length: Some(6),
                stage: None,
            })
            .with_fix("\"/srv\"")
            .with_help("syntax");
        error.expected = Some(json!("text"));
        let mut analysis = Analysis::default();
        analysis.add_error(text, &lines, &error, Variant::A, Encoding::Utf16);
        assert_eq!(
            analysis.diagnostics[0].data,
            Some(json!({"help": "syntax"}))
        );
        assert!(
            analysis.diagnostics[0]
                .message
                .contains("\nexpected: text\n")
        );
        assert_eq!(
            edits(&analysis, whole()),
            [("Replace with `\"/srv\"`".to_owned(), "\"/srv\"".to_owned())]
        );
        let unplaced = Error::new(ErrorCode::Syntax, "broken").with_fix("count\ncount");
        let mut analysis = Analysis::default();
        analysis.add_error(text, &lines, &unplaced, Variant::A, Encoding::Utf16);
        assert_eq!(analysis.diagnostics[0].range, Range::default());
        assert_eq!(
            edits(&analysis, whole()),
            [("Apply fix: count …".to_owned(), "count\ncount\n".to_owned())]
        );
    }

    #[test]
    fn corrections_after_wide_characters_are_placed_in_the_units_of_the_encoding() {
        let setup = Setup::new();
        let text = "find \"\u{1f600}\" |> Count\n";
        let utf16 = setup.engine(Encoding::Utf16).analyze(text);
        assert_eq!(
            utf16.diagnostics[0].range,
            Range::new(Position::new(0, 13), Position::new(0, 18))
        );
        let utf32 = setup.engine(Encoding::Utf32).analyze(text);
        assert_eq!(
            utf32.diagnostics[0].range,
            Range::new(Position::new(0, 12), Position::new(0, 17))
        );
        let fixes = edits(&utf16, whole());
        assert_eq!(fixes[0].1, "count");
    }

    #[test]
    fn a_program_replaces_the_text_with_the_line_ending_of_the_text() {
        assert_eq!(program_text("count\n", "count"), "count\n");
        assert_eq!(program_text("count\n", "count\n"), "count\n");
        assert_eq!(program_text("count", "count"), "count");
        assert_eq!(program_text("", "count"), "count");
        assert_eq!(program_text("\n", ""), "\n");
    }

    #[test]
    fn the_first_line_of_a_program_is_shown_with_a_mark_for_more() {
        assert_eq!(first_line("count"), "count");
        assert_eq!(first_line("count\nsort size"), "count \u{2026}");
        assert_eq!(
            first_line("\u{142}\u{f3}d\u{17a}\r\nx"),
            "\u{142}\u{f3}d\u{17a} \u{2026}"
        );
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn positions_of_an_error_need_a_line_and_a_column() {
        let lines = Lines::new("find \"\u{142}\" |> x\n");
        let at = |line, column| At {
            line,
            column,
            length: Some(1),
            stage: None,
        };
        assert_eq!(error_bytes(&lines, None), None);
        assert_eq!(error_bytes(&lines, Some(at(Some(1), None))), None);
        assert_eq!(error_bytes(&lines, Some(at(None, Some(1)))), None);
        let found = error_bytes(&lines, Some(at(Some(1), Some(13)))).unwrap();
        assert_eq!(&"find \"\u{142}\" |> x\n"[found], "x");
    }

    #[test]
    fn a_span_without_a_length_is_empty_and_a_long_one_ends_with_the_text() {
        let lines = Lines::new("ab\ncd");
        assert_eq!(span(&lines, 1, 2, None), 1..1);
        assert_eq!(span(&lines, 2, 1, Some(99)), 3..5);
        assert_eq!(span(&lines, 9, 9, Some(1)), 5..5);
    }

    #[test]
    fn a_fix_without_a_position_replaces_the_whole_text() {
        let text = "count )\n";
        let lines = Lines::new(text);
        let (edit, local) = fix_edit(
            text,
            &lines,
            None,
            "count",
            true,
            Variant::A,
            Encoding::Utf16,
        );
        assert!(!local);
        assert_eq!(edit.new_text, "count\n");
        assert_eq!(
            edit.range,
            Range::new(Position::new(0, 0), Position::new(1, 0))
        );
    }

    #[test]
    fn a_lexical_fix_replaces_only_the_fragment_at_the_error() {
        let text = "find '/srv'\n";
        let lines = Lines::new(text);
        let (edit, local) = fix_edit(
            text,
            &lines,
            Some(5..11),
            "\"/srv\"",
            true,
            Variant::A,
            Encoding::Utf16,
        );
        assert!(local);
        assert_eq!(edit.new_text, "\"/srv\"");
        assert_eq!(
            edit.range,
            Range::new(Position::new(0, 5), Position::new(0, 11))
        );
    }

    #[test]
    fn only_the_empty_program_problem_is_not_an_error() {
        assert!(is_empty_program(Some("empty_program")));
        assert!(!is_empty_program(Some("trailing_input")));
        assert!(!is_empty_program(None));
    }

    #[test]
    fn matches_argument_shapes_to_types() {
        let path: ParamType = "path".parse().unwrap();
        let pattern: ParamType = "text | regex".parse().unwrap();
        let order: ParamType = "enum(asc|desc)".parse().unwrap();
        let fields: ParamType = "list<field>".parse().unwrap();
        let count: ParamType = "int".parse().unwrap();
        assert!(accepts(&path, ArgumentShape::Text));
        assert!(!accepts(&path, ArgumentShape::Word));
        assert!(accepts(&pattern, ArgumentShape::Regex));
        assert!(accepts(&order, ArgumentShape::Word));
        assert!(!accepts(&order, ArgumentShape::Number));
        assert!(accepts(&fields, ArgumentShape::Word));
        assert!(accepts(&count, ArgumentShape::Number));
        let setup = Setup::new();
        let view = View::new(&setup.commands);
        assert!(view.is_command("sort") && !view.is_command("remove"));
        assert_eq!(view.command_names(), ["count", "find", "sort"]);
        assert!(!view.is_mutating("sort"));
        assert!(view.can_take_argument("sort", 0, ArgumentShape::Word));
        assert!(!view.can_take_argument("sort", 1, ArgumentShape::Word));
        assert!(!view.can_take_argument("remove", 0, ArgumentShape::Word));
    }

    #[test]
    fn formats_into_one_edit_of_the_whole_document() {
        let text = "find \"/srv/app\"   glob: \"*.go\" |> first 0x14\n";
        let edits = formatting(text, Variant::A, Encoding::Utf16).unwrap();
        let [edit] = edits.as_slice() else {
            panic!("expected one edit: {edits:?}")
        };
        assert_eq!(
            edit.range,
            Range::new(Position::new(0, 0), Position::new(1, 0))
        );
        assert_eq!(
            edit.new_text,
            "find \"/srv/app\" glob: \"*.go\"\n|> first 20\n"
        );
        let canonical = edit.new_text.clone();
        assert!(
            formatting(&canonical, Variant::A, Encoding::Utf16)
                .unwrap()
                .is_empty()
        );
        assert!(
            formatting("", Variant::A, Encoding::Utf16)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn refuses_to_format_invalid_programs() {
        let failure = formatting("count )", Variant::A, Encoding::Utf16).unwrap_err();
        assert!(
            failure.starts_with("cannot format the document: "),
            "{failure}"
        );
        assert!(failure.ends_with("at line 1, column 7"), "{failure}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
