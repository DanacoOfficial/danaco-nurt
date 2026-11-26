// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Translation of PowerShell statements into Nurt statements by the
//! translation table.
//!
//! A command is bound against the parameters of its table entry, a variant
//! of the entry is chosen by the bound parameters and the place of the
//! command in the pipeline, and the variant gives the Nurt stages. Every
//! translated command is reported as one correction with the rule and class
//! of the variant; every path written in Windows or registry notation is
//! reported with the path rule.

use nurt_contracts::ast::{
    Arg, AstDocument, BinOp, Expr, Head, Let, MapEntry, Pipeline as NurtPipeline, Program, Ref,
    Span, Stage as NurtStage, Statement as NurtStatement,
};
use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::{At, Error, ErrorCode, Stage as ErrorStage};
use nurt_contracts::report::TextPosition;
use nurt_syntax::parse::Variant as SyntaxVariant;
use nurt_syntax::print::{print, print_expr};

use super::LetBinding;
use super::convert::{self, PathProblem};
use super::lex::{Positions, StrPart};
use super::script_block;
use super::syntax::{
    Command, Element, Pipeline, RawArg, Redirection, Statement, StatementKind, Value, ValueKind,
};
use super::table::{
    Absent, Cmdlet, Handler, Param, PowerShellTable, Record, Role, SetValue, ValueType, Variant,
};
use super::{Assumption, Mode, TranslateContext};
use crate::policy::Class;
use crate::record::{Correction, CorrectionKind, RuleId};

/// Help topic of corrections.
pub(crate) const HELP_CORRECTIONS: &str = "help corrections";

/// Prefix written before a glob of a recursive search.
const RECURSIVE_GLOB: &str = "**/";

/// Nurt stage that runs a program.
const RUN_COMMAND: &str = "run";

/// Parameter of `run` that names the working directory.
const CWD_PARAMETER: &str = "cwd";

/// Nurt stage of a web request, a read or a change by its method.
const HTTP_REQUEST: &str = "http.request";

/// Methods of a request that do not change the server, in lower case.
const READ_ONLY_METHODS: [&str; 3] = ["get", "head", "options"];

/// Nurt stage that reads a file as a whole.
const READ_COMMAND: &str = "read";

/// Nurt stage that reads a file as lines.
const LINES_COMMAND: &str = "lines";

/// Nurt stage that filters records, for `-Include` and `-Exclude`.
const FILTER_COMMAND: &str = "filter";

/// Nurt stage that keeps fields, for `ForEach-Object { $_.Name }`.
const SELECT_COMMAND: &str = "select";

/// Nurt stage that reads an entry, for `icacls P` and `attrib F`.
const INFO_COMMAND: &str = "info";

/// Nurt stage that changes attributes, for `attrib +r F`.
const ATTRIBUTES_COMMAND: &str = "attributes";

/// Field of an entry compared by `-Include` and `-Exclude`.
const NAME_FIELD: &str = "name";

/// Field of an entry compared by `Test-Path -PathType`.
const TYPE_FIELD: &str = "type";

/// Parameters of `search` for `-Context`.
const CONTEXT_ARG: &str = "context";
const BEFORE_ARG: &str = "before";
const AFTER_ARG: &str = "after";

/// Unit of `-Milliseconds`.
const MILLISECOND_UNIT: &str = "ms";

/// Unit of `-Seconds`.
const SECOND_UNIT: &str = "s";

/// Regex flag that ignores case.
const IGNORE_CASE_FLAG: &str = "i";

/// Entry of the table used for redirections, `> F` and `>> F`.
const REDIRECTION_CMDLET: &str = "Out-File";

/// Parameter of `Out-File` for `>>`.
const APPEND_PARAM: &str = "Append";

/// Parameter that suppresses the line break of written values.
const NO_NEWLINE_PARAM: &str = "NoNewline";

/// Entry of the table read by `Compare-Object` arguments.
const CONTENT_CMDLET: &str = "Get-Content";

/// Preference variables of PowerShell.
const PREFERENCE_VARIABLES: [&str; 8] = [
    "ErrorActionPreference",
    "WarningPreference",
    "VerbosePreference",
    "DebugPreference",
    "InformationPreference",
    "ProgressPreference",
    "ConfirmPreference",
    "WhatIfPreference",
];

/// Words of the language that cannot name a `let` result.
const RESERVED_NAMES: [&str; 8] = ["let", "true", "false", "null", "and", "or", "not", "in"];

/// Attribute flags of `attrib`.
const ATTRIB_FLAGS: [(&str, &str, bool); 4] = [
    ("+r", "readonly", true),
    ("-r", "readonly", false),
    ("+h", "hidden", true),
    ("-h", "hidden", false),
];

/// Result of a translated statement.
pub(crate) struct TranslatedStatement {
    pub(crate) statement: NurtStatement,
    pub(crate) effect: bool,
}

/// An alternative stage of a probable translation.
pub(crate) struct AlternativeStage {
    pub(crate) statement: usize,
    pub(crate) position: usize,
    pub(crate) stage: NurtStage,
}

/// State of one translation.
pub(crate) struct Session<'a> {
    pub(crate) table: &'a PowerShellTable,
    pub(crate) context: &'a TranslateContext<'a>,
    pub(crate) input: &'a str,
    positions: Positions,
    pub(crate) corrections: Vec<Correction>,
    pub(crate) assumptions: Vec<Assumption>,
    pub(crate) notes: Vec<Assumption>,
    pub(crate) options: RequestOptions,
    pub(crate) alternatives: Vec<AlternativeStage>,
    lets: Vec<(String, Option<String>)>,
    location: Option<String>,
    statement: usize,
}

/// Stages of a pipeline under construction.
#[derive(Default)]
struct Chain {
    head: Option<Head>,
    stages: Vec<NurtStage>,
    record: Option<String>,
    merge_next: Option<Vec<Arg>>,
    literal: Option<Value>,
    hypothetical: bool,
}

impl Chain {
    fn push(&mut self, stage: NurtStage) {
        if self.head.is_none() {
            self.head = Some(Head::Stage(stage));
        } else {
            self.stages.push(stage);
        }
    }

    fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    fn len(&self) -> usize {
        usize::from(self.head.is_some()) + self.stages.len()
    }

    fn last_mut(&mut self) -> Option<&mut NurtStage> {
        match self.stages.last_mut() {
            Some(stage) => Some(stage),
            None => match &mut self.head {
                Some(Head::Stage(stage)) => Some(stage),
                _ => None,
            },
        }
    }

    /// Every stage, head first.
    fn all_stages(&self) -> Vec<NurtStage> {
        let head = match &self.head {
            Some(Head::Stage(stage)) => Some(stage.clone()),
            _ => None,
        };
        head.into_iter()
            .chain(self.stages.iter().cloned())
            .collect()
    }

    fn pipeline(self, span: Span) -> Option<NurtPipeline> {
        Some(NurtPipeline {
            head: self.head?,
            stages: self.stages,
            span: Some(span),
        })
    }
}

/// A parameter bound to its argument.
#[derive(Clone)]
struct BoundParam<'t> {
    param: &'t Param,
    value: Option<Value>,
    start: usize,
    end: usize,
}

/// Arguments of a command bound to the parameters of its entry.
struct Bound<'t> {
    items: Vec<BoundParam<'t>>,
}

impl<'t> Bound<'t> {
    fn has(&self, name: &str) -> bool {
        self.items
            .iter()
            .any(|item| item.param.name.eq_ignore_ascii_case(name))
    }

    fn get(&self, name: &str) -> Option<&BoundParam<'t>> {
        self.items
            .iter()
            .find(|item| item.param.name.eq_ignore_ascii_case(name))
    }

    fn of(&self, param: &Param) -> Option<&BoundParam<'t>> {
        self.items
            .iter()
            .find(|item| std::ptr::eq(item.param, param))
    }

    fn text_of(&self, name: &str) -> Option<String> {
        self.get(name)?.value.as_ref()?.plain_text()
    }

    fn with_role(&self, role: Role) -> Option<&BoundParam<'t>> {
        self.items.iter().find(|item| item.param.role == Some(role))
    }
}

/// Conditions of variant selection that do not come from parameters.
#[derive(Clone, Copy)]
struct Place {
    piped: bool,
    continued: bool,
    wildcard: bool,
    registry: bool,
}

/// Arguments collected for one Nurt stage.
struct StageArgs {
    name: String,
    positional: Vec<Arg>,
    named: Vec<Arg>,
}

impl StageArgs {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            positional: Vec::new(),
            named: Vec::new(),
        }
    }

    fn has_named(&self, name: &str) -> bool {
        self.named
            .iter()
            .any(|arg| arg.name.as_deref() == Some(name))
    }

    fn place(&mut self, arg: Option<&str>, values: Vec<Expr>) {
        match arg {
            Some(name) => self
                .named
                .extend(values.into_iter().map(|value| named(name, value))),
            None => self.positional.extend(values.into_iter().map(positional)),
        }
    }

    fn into_stage(self, span: Span) -> NurtStage {
        let mut args = self.positional;
        args.extend(self.named);
        NurtStage {
            name: self.name,
            args,
            span: Some(span),
        }
    }
}

/// Stages a variant builds, before they are pushed to the chain.
struct Built {
    stages: Vec<StageArgs>,
    emitted: Vec<(usize, StageArgs)>,
    extra: Vec<NurtStage>,
    run_list: Vec<Expr>,
    alternative: Option<Vec<Expr>>,
    wildcard_head: Option<NurtStage>,
    class: Class,
}

pub(crate) fn text(v: impl Into<String>, span: Span) -> Expr {
    Expr::Text {
        v: v.into(),
        span: Some(span),
    }
}

pub(crate) fn ident(v: impl Into<String>, span: Span) -> Expr {
    Expr::Ident {
        v: v.into(),
        span: Some(span),
    }
}

fn positional(value: Expr) -> Arg {
    Arg {
        name: None,
        value,
        span: None,
    }
}

fn named(name: &str, value: Expr) -> Arg {
    Arg {
        name: Some(name.to_owned()),
        value,
        span: None,
    }
}

/// Canonical text of a program without the final line break.
pub(crate) fn program_text(document: &AstDocument) -> String {
    print(document, SyntaxVariant::A).trim_end().to_owned()
}

/// Canonical text of one statement.
pub(crate) fn statement_text(statement: &NurtStatement) -> String {
    program_text(&AstDocument::new(Program {
        statements: vec![statement.clone()],
    }))
}

/// Text of translated stages on one line, for the `to` member of a
/// correction.
fn fragment_text(stages: &[NurtStage]) -> String {
    let Some((first, rest)) = stages.split_first() else {
        return String::new();
    };
    let pipeline = NurtPipeline {
        head: Head::Stage(first.clone()),
        stages: rest.to_vec(),
        span: None,
    };
    statement_text(&NurtStatement::Pipeline(pipeline)).replace("\n|> ", " |> ")
}

fn expr_text(expr: &Expr) -> String {
    print_expr(expr, SyntaxVariant::A)
}

impl<'a> Session<'a> {
    pub(crate) fn new(
        table: &'a PowerShellTable,
        context: &'a TranslateContext<'a>,
        input: &'a str,
    ) -> Self {
        Self {
            table,
            context,
            input,
            positions: Positions::new(input),
            corrections: Vec::new(),
            assumptions: Vec::new(),
            notes: Vec::new(),
            options: RequestOptions::default(),
            alternatives: Vec::new(),
            lets: context
                .lets
                .iter()
                .map(|name| (name.clone(), None))
                .collect(),
            location: None,
            statement: 0,
        }
    }

    pub(crate) fn span(&self, start: usize, end: usize) -> Span {
        self.positions.span(self.input, start, end)
    }

    pub(crate) fn source(&self, start: usize, end: usize) -> &'a str {
        self.input.get(start..end).unwrap_or_default()
    }

    fn at(&self, start: usize, end: usize) -> At {
        let (line, column) = self.positions.at(self.input, start);
        let length = self.source(start, end).chars().count();
        At::new(line, column, u32::try_from(length).unwrap_or(u32::MAX))
    }

    fn position(&self, start: usize, end: usize) -> TextPosition {
        let (line, column) = self.positions.at(self.input, start);
        let length = self.source(start, end).chars().count();
        TextPosition {
            line,
            column,
            length: Some(u32::try_from(length).unwrap_or(u32::MAX)),
        }
    }

    /// An error of the parse stage at a range of the input.
    pub(crate) fn error(
        &self,
        code: ErrorCode,
        message: impl Into<String>,
        start: usize,
        end: usize,
    ) -> Error {
        let stage = code.stages().first().copied().unwrap_or(ErrorStage::Parse);
        Error::new(code, message)
            .with_stage(stage)
            .with_at(self.at(start, end))
            .with_found(self.source(start, end))
    }

    /// An `E_UNCORRECTABLE` error with its problem and help topic.
    pub(crate) fn uncorrectable(
        &self,
        problem: &str,
        message: impl Into<String>,
        start: usize,
        end: usize,
        help: &str,
    ) -> Error {
        self.error(ErrorCode::Uncorrectable, message, start, end)
            .with_problem(problem)
            .with_help(help)
    }

    fn record_correction(
        &mut self,
        rule: RuleId,
        range: (usize, usize),
        to: String,
        kind: CorrectionKind,
    ) {
        let from = self.source(range.0, range.1).to_owned();
        let correction = Correction::new(rule, Class::Certain, from, to, kind)
            .with_at(self.position(range.0, range.1));
        self.corrections.push(correction);
    }

    /// Inserts the correction of a command before the path corrections
    /// recorded while its arguments were converted.
    fn command_correction(
        &mut self,
        rule: RuleId,
        class: Class,
        range: (usize, usize),
        to: String,
    ) {
        let index = self
            .corrections
            .iter()
            .rposition(|correction| {
                correction
                    .at
                    .is_some_and(|at| self.byte_before(at, range.0))
            })
            .map_or(0, |index| index + 1);
        let from = self.source(range.0, range.1).to_owned();
        let correction = Correction::new(rule, class, from, to, CorrectionKind::Habit)
            .with_at(self.position(range.0, range.1));
        self.corrections.insert(index, correction);
    }

    fn byte_before(&self, at: TextPosition, offset: usize) -> bool {
        let (line, column) = self.positions.at(self.input, offset);
        (at.line, at.column) < (line, column)
    }

    pub(crate) fn assume(&mut self, rule: RuleId, text: &str) {
        let assumption = Assumption {
            rule: rule.to_string(),
            text: text.to_owned(),
        };
        if !self.assumptions.contains(&assumption) {
            self.assumptions.push(assumption);
        }
    }

    fn note(&mut self, rule: RuleId, text: &str) {
        if !text.is_empty() {
            self.notes.push(Assumption {
                rule: rule.to_string(),
                text: text.to_owned(),
            });
        }
    }

    fn let_record(&self, name: &str) -> Option<Option<String>> {
        self.lets
            .iter()
            .rev()
            .find(|(known, _)| known == name)
            .map(|(_, record)| record.clone())
    }

    /// Adds `let` names whose record type is known from earlier calls; a
    /// later binding of a name replaces an earlier one.
    pub(crate) fn know_lets(&mut self, known: &[LetBinding]) {
        self.lets.extend(
            known
                .iter()
                .map(|binding| (binding.name.clone(), binding.record.clone())),
        );
    }

    /// Number of `let` names known so far.
    pub(crate) fn let_count(&self) -> usize {
        self.lets.len()
    }

    /// The `let` names bound after the first `start` known names.
    pub(crate) fn lets_since(&self, start: usize) -> Vec<LetBinding> {
        self.lets
            .get(start..)
            .unwrap_or_default()
            .iter()
            .map(|(name, record)| LetBinding {
                name: name.clone(),
                record: record.clone(),
            })
            .collect()
    }

    fn known_let(&self, written: &str) -> Option<String> {
        let_name(written).filter(|name| self.let_record(name).is_some())
    }

    /// Translates one statement of the script.
    pub(crate) fn statement(
        &mut self,
        statement: &Statement,
        index: usize,
    ) -> Result<TranslatedStatement, Error> {
        self.statement = index;
        match &statement.kind {
            StatementKind::Pipeline(pipeline) => {
                let (pipeline, _) = self.pipeline(pipeline)?;
                self.require_working_directory(&pipeline, statement.start, statement.end)?;
                let effect = self.pipeline_effect(&pipeline);
                Ok(TranslatedStatement {
                    statement: NurtStatement::Pipeline(pipeline),
                    effect,
                })
            }
            StatementKind::Assign {
                var,
                var_start,
                var_end,
                value,
            } => {
                if var.scope.is_some() {
                    return Err(self.uncorrectable(
                        "variable_scope",
                        "scoped and environment variables are not set; give environment variables to a program with run env:",
                        statement.start,
                        statement.end,
                        "help run",
                    ));
                }
                if is_preference_variable(&var.name) {
                    return Err(self.uncorrectable(
                        "preference_variable",
                        format!(
                            "`${}` has no Nurt form: a program stops at its first error and shows every result; remove the assignment, and use `on_missing: skip` of the command instead of -ErrorAction SilentlyContinue",
                            var.name
                        ),
                        statement.start,
                        statement.end,
                        HELP_CORRECTIONS,
                    ));
                }
                let name = let_name(&var.name).ok_or_else(|| {
                    self.uncorrectable(
                        "variable_name",
                        format!(
                            "`${}` is not a valid let name; use lower-case letters, digits and `_`, and not a word of the language",
                            var.name
                        ),
                        *var_start,
                        *var_end,
                        HELP_CORRECTIONS,
                    )
                })?;
                let (pipeline, record) = self.pipeline(value)?;
                if self.pipeline_effect(&pipeline) {
                    return Err(self.uncorrectable(
                        "effect_in_let",
                        "let names the result of reading; run the changing command as its own program",
                        statement.start,
                        statement.end,
                        "help let",
                    ));
                }
                let rule = self.table.syntax.assignment;
                self.record_correction(
                    rule,
                    (*var_start, value.start),
                    format!("let {name} ="),
                    CorrectionKind::Habit,
                );
                self.lets.push((name.clone(), record));
                let span = self.span(statement.start, statement.end);
                Ok(TranslatedStatement {
                    statement: NurtStatement::Let(Let {
                        name,
                        value: pipeline,
                        span: Some(span),
                    }),
                    effect: false,
                })
            }
        }
    }

    pub(crate) fn pipeline_effect(&self, pipeline: &NurtPipeline) -> bool {
        let head = match &pipeline.head {
            Head::Stage(stage) => Some(stage),
            _ => None,
        };
        head.into_iter()
            .chain(pipeline.stages.iter())
            .any(|stage| self.stage_effect(stage))
    }

    /// Whether a stage changes the world or runs a program. A request is a
    /// change unless its method is a literal `get`, `head` or `options`; a
    /// method that is not a literal, such as a variable, is not known to read.
    fn stage_effect(&self, stage: &NurtStage) -> bool {
        if stage.name == HTTP_REQUEST {
            let reads = stage
                .args
                .iter()
                .find(|arg| arg.name.as_deref() == Some("method"))
                .is_none_or(|arg| match &arg.value {
                    Expr::Ident { v, .. } | Expr::Text { v, .. } => {
                        READ_ONLY_METHODS.contains(&v.to_ascii_lowercase().as_str())
                    }
                    _ => false,
                });
            return !reads;
        }
        self.table.is_effect(&stage.name)
    }

    /// Translates a pipeline; returns it with the record kind of its output.
    fn pipeline(&mut self, pipeline: &Pipeline) -> Result<(NurtPipeline, Option<String>), Error> {
        let mut chain = Chain::default();
        let count = pipeline.elements.len();
        for (index, element) in pipeline.elements.iter().enumerate() {
            let continued = index + 1 < count;
            match element {
                Element::Expression(value) if index == 0 => {
                    self.expression_head(value, continued, &mut chain)?;
                }
                Element::Expression(value) => {
                    return Err(self.uncorrectable(
                        "expression_in_pipeline",
                        "a value inside a pipeline has no Nurt form; write the stage that produces it",
                        value.start,
                        value.end,
                        HELP_CORRECTIONS,
                    ));
                }
                Element::Command(command) => {
                    self.command(command, continued, &mut chain)?;
                    for redirection in &command.redirections {
                        self.redirection(redirection, &mut chain)?;
                    }
                }
            }
        }
        let (start, end) = (pipeline.start, pipeline.end);
        if chain.merge_next.is_some() {
            return Err(self.uncorrectable(
                "conversion_without_write",
                "ConvertTo-Json only shows text unless a file is written; write the records with write format: json",
                start,
                end,
                "help write",
            ));
        }
        if chain.literal.is_some() {
            return Err(self.uncorrectable(
                "value_without_target",
                "the value is not written anywhere; show it with evaluate or store it with write",
                start,
                end,
                "help write",
            ));
        }
        let hypothetical = chain.hypothetical;
        let record = chain.record.clone();
        let span = self.span(start, end);
        let pipeline = chain.pipeline(span).ok_or_else(|| {
            self.uncorrectable(
                "empty_pipeline",
                "the pipeline produces nothing",
                start,
                end,
                HELP_CORRECTIONS,
            )
        })?;
        if hypothetical {
            let fix = statement_text(&NurtStatement::Pipeline(pipeline));
            return Err(self.no_current_directory(start, end).with_fix(fix));
        }
        Ok((pipeline, record))
    }

    /// A script has no current directory, so a program of a script needs its
    /// working directory: `run` takes `cwd` (or the call parameter `base`).
    fn require_working_directory(
        &self,
        pipeline: &NurtPipeline,
        start: usize,
        end: usize,
    ) -> Result<(), Error> {
        if self.context.mode != Mode::Script || self.context.base.is_some() {
            return Ok(());
        }
        let head = match &pipeline.head {
            Head::Stage(stage) => Some(stage),
            _ => None,
        };
        let missing = head.into_iter().chain(&pipeline.stages).any(|stage| {
            stage.name == RUN_COMMAND
                && !stage
                    .args
                    .iter()
                    .any(|arg| arg.name.as_deref() == Some(CWD_PARAMETER))
        });
        if missing {
            return Err(self.uncorrectable(
                "working_directory",
                "the working directory of the program is not given; write `Start-Process -WorkingDirectory <absolute path>` or give the call parameter base",
                start,
                end,
                "help run",
            ));
        }
        Ok(())
    }

    fn no_current_directory(&self, start: usize, end: usize) -> Error {
        let mut error = self.uncorrectable(
            "no_current_directory",
            "no current directory; paths are absolute or relative to the call parameter base",
            start,
            end,
            HELP_CORRECTIONS,
        );
        if let Some(location) = &self.location {
            error = error.with_detail("base", serde_json::Value::from(location.clone()));
        }
        error
    }

    fn expression_head(
        &mut self,
        value: &Value,
        continued: bool,
        chain: &mut Chain,
    ) -> Result<(), Error> {
        let span = self.span(value.start, value.end);
        let range = (value.start, value.end);
        match &value.kind {
            ValueKind::Variable(var) if var.is_environment() => {
                let syntax = &self.table.syntax;
                let stage = NurtStage {
                    name: syntax.environment_command.clone(),
                    args: vec![named(
                        &syntax.environment_param,
                        text(var.name.clone(), span),
                    )],
                    span: Some(span),
                };
                let rule = syntax.environment;
                self.command_correction(
                    rule,
                    Class::Certain,
                    range,
                    fragment_text(std::slice::from_ref(&stage)),
                );
                chain.push(stage);
                Ok(())
            }
            ValueKind::Variable(var) => {
                let name = self
                    .known_let(&var.name)
                    .ok_or_else(|| self.unknown_variable(&var.name, value.start, value.end))?;
                chain.record = self.let_record(&name).flatten();
                chain.head = Some(Head::Ref(Ref {
                    name: name.clone(),
                    fields: Vec::new(),
                    span: Some(span),
                }));
                let rule = self.table.syntax.assignment;
                self.record_correction(rule, range, format!("@{name}"), CorrectionKind::Ref);
                Ok(())
            }
            ValueKind::Paren(script) => match script.statements.as_slice() {
                [
                    Statement {
                        kind: StatementKind::Pipeline(inner),
                        ..
                    },
                ] => {
                    let (pipeline, record) = self.pipeline(inner)?;
                    chain.head = Some(pipeline.head);
                    chain.stages = pipeline.stages;
                    chain.record = record;
                    Ok(())
                }
                _ => Err(self.expression_error(value)),
            },
            ValueKind::Literal(_)
            | ValueKind::Expandable(_)
            | ValueKind::Bare(_)
            | ValueKind::Number(_) => {
                if continued {
                    chain.literal = Some(value.clone());
                    return Ok(());
                }
                let expr = self.output_value(value)?;
                let syntax = &self.table.syntax;
                let stage = NurtStage {
                    name: syntax.output_command.clone(),
                    args: vec![positional(expr)],
                    span: Some(span),
                };
                let rule = syntax.output;
                self.command_correction(
                    rule,
                    Class::Certain,
                    range,
                    fragment_text(std::slice::from_ref(&stage)),
                );
                chain.push(stage);
                Ok(())
            }
            _ => Err(self.expression_error(value)),
        }
    }

    fn expression_error(&self, value: &Value) -> Error {
        self.uncorrectable(
            "expression",
            "a PowerShell expression has no Nurt form; write the commands that compute the value",
            value.start,
            value.end,
            HELP_CORRECTIONS,
        )
    }

    fn unknown_variable(&self, name: &str, start: usize, end: usize) -> Error {
        self.uncorrectable(
            "unknown_variable",
            format!("`${name}` is not a result named with let in this program"),
            start,
            end,
            "help let",
        )
    }

    /// Value of `Write-Output` or of a string statement.
    fn output_value(&self, value: &Value) -> Result<Expr, Error> {
        let span = self.span(value.start, value.end);
        match &value.kind {
            ValueKind::Number(number) => Ok(match number.int {
                Some(int) if number.exponent == 0 && int >= 0 => Expr::Int {
                    v: int,
                    span: Some(span),
                },
                _ => text(self.source(value.start, value.end), span),
            }),
            ValueKind::Variable(var) if !var.is_environment() => match self.known_let(&var.name) {
                Some(name) => Ok(Expr::Ref(Ref {
                    name,
                    fields: Vec::new(),
                    span: Some(span),
                })),
                None => Err(self.unknown_variable(&var.name, value.start, value.end)),
            },
            ValueKind::List(items) => Ok(Expr::List {
                items: items
                    .iter()
                    .map(|item| self.output_value(item))
                    .collect::<Result<_, _>>()?,
                span: Some(span),
            }),
            _ => Ok(text(self.plain(value)?, span)),
        }
    }

    fn redirection(&mut self, redirection: &Redirection, chain: &mut Chain) -> Result<(), Error> {
        let redirect = redirection.redirect;
        let target = redirection
            .target
            .clone()
            .filter(|_| redirect.stream.is_none() && !redirect.merge)
            .ok_or_else(|| {
                self.uncorrectable(
                    "stream_redirection",
                    "only the output is redirected; errors are part of the result",
                    redirection.start,
                    redirection.end,
                    HELP_CORRECTIONS,
                )
            })?;
        let table = self.table;
        let cmdlet = table.cmdlet(REDIRECTION_CMDLET).ok_or_else(|| {
            self.error(
                ErrorCode::InternalCorrection,
                "the translation table has no Out-File entry",
                redirection.start,
                redirection.end,
            )
        })?;
        let mut args = vec![RawArg::Value(target)];
        if redirect.append {
            args.push(RawArg::Param {
                name: APPEND_PARAM.to_owned(),
                value: None,
                start: redirection.start,
                end: redirection.start,
            });
        }
        let command = Command {
            name: REDIRECTION_CMDLET.to_owned(),
            name_start: redirection.start,
            name_end: redirection.end,
            call: None,
            args,
            redirections: Vec::new(),
            start: redirection.start,
            end: redirection.end,
        };
        self.cmdlet_command(&command, cmdlet, false, chain)
    }

    fn command(
        &mut self,
        command: &Command,
        continued: bool,
        chain: &mut Chain,
    ) -> Result<(), Error> {
        if command.call.is_some() {
            return self.call(command, chain);
        }
        let table = self.table;
        let Some(cmdlet) = table.cmdlet(&command.name) else {
            let message = if table.is_cmdlet_form(&command.name) {
                format!("{} has no Nurt translation", command.name)
            } else {
                format!("`{}` is not a cmdlet; run a program with run", command.name)
            };
            return Err(self.uncorrectable(
                "unknown_cmdlet",
                message,
                command.name_start,
                command.name_end,
                HELP_CORRECTIONS,
            ));
        };
        match cmdlet.handler {
            Some(Handler::Where) => self.where_object(command, cmdlet, chain),
            Some(Handler::ForEach) => self.foreach_object(command, cmdlet, chain),
            Some(Handler::Attrib) => self.attrib(command, cmdlet, chain),
            Some(Handler::Icacls) => self.icacls(command, cmdlet, chain),
            None => self.cmdlet_command(command, cmdlet, continued, chain),
        }
    }

    /// `& program args` becomes `run ["program", "args"…]`.
    fn call(&mut self, command: &Command, chain: &mut Chain) -> Result<(), Error> {
        let span = self.span(command.start, command.end);
        if !chain.is_empty() {
            return Err(self.uncorrectable(
                "program_in_pipeline",
                "a program does not read records from a pipeline; run it as its own program",
                command.start,
                command.end,
                "help run",
            ));
        }
        let program = command.call.as_deref().ok_or_else(|| {
            self.uncorrectable(
                "program_value",
                "the program is missing",
                command.start,
                command.end,
                "help run",
            )
        })?;
        let mut items = vec![self.program_expr(program)?];
        for arg in &command.args {
            match arg {
                RawArg::Param { start, end, .. } => {
                    items.push(text(self.source(*start, *end), self.span(*start, *end)));
                }
                RawArg::Value(value) => items.push(self.argument_text(value)?),
            }
        }
        let stage = NurtStage {
            name: RUN_COMMAND.to_owned(),
            args: vec![positional(Expr::List {
                items,
                span: Some(span),
            })],
            span: Some(span),
        };
        let rule = self.table.syntax.call;
        self.command_correction(
            rule,
            Class::Certain,
            (command.start, command.end),
            fragment_text(std::slice::from_ref(&stage)),
        );
        chain.push(stage);
        Ok(())
    }

    /// Text of one argument passed to a program.
    fn argument_text(&self, value: &Value) -> Result<Expr, Error> {
        let span = self.span(value.start, value.end);
        match &value.kind {
            ValueKind::Number(_) => Ok(text(self.source(value.start, value.end), span)),
            _ => value
                .plain_text()
                .map(|plain| text(plain, span))
                .ok_or_else(|| {
                    self.uncorrectable(
                        "argument_value",
                        "only literal arguments are passed to a program; write the value itself",
                        value.start,
                        value.end,
                        "help run",
                    )
                }),
        }
    }

    /// The program of `& program` or `Start-Process`: a Windows path is
    /// written in Nurt notation, a name as written.
    fn program_expr(&mut self, value: &Value) -> Result<Expr, Error> {
        let span = self.span(value.start, value.end);
        let plain = value.plain_text().ok_or_else(|| {
            self.uncorrectable(
                "program_value",
                "the program must be a literal name or path",
                value.start,
                value.end,
                "help run",
            )
        })?;
        if plain.contains(['\\', '/'])
            && !plain.starts_with('.')
            && let Ok(path) = convert::path(&plain, &self.table.registry, self.context.base)
        {
            if path.changed {
                let rule = self.table.path_rule;
                let to = expr_text(&text(path.text.clone(), span));
                self.record_correction(rule, (value.start, value.end), to, CorrectionKind::Path);
            }
            return Ok(text(path.text, span));
        }
        Ok(text(plain, span))
    }

    /// Binds the arguments of a command to the parameters of its entry.
    fn bind(
        &self,
        command: &Command,
        cmdlet: &'a Cmdlet,
        chain: &mut Chain,
    ) -> Result<Bound<'a>, Error> {
        let mut items: Vec<BoundParam<'a>> = Vec::new();
        for (name, value) in cmdlet.preset(&command.name) {
            if let Some(param) = cmdlet.params.iter().find(|param| param.is_named(name)) {
                items.push(BoundParam {
                    param,
                    value: Some(Value {
                        kind: ValueKind::Bare(value.clone()),
                        start: command.name_start,
                        end: command.name_end,
                    }),
                    start: command.name_start,
                    end: command.name_end,
                });
            }
        }
        let mut args = command.args.iter().peekable();
        while let Some(arg) = args.next() {
            match arg {
                RawArg::Param {
                    name,
                    value,
                    start,
                    end,
                } => {
                    let param = self.resolve_param(cmdlet, name, *start, *end)?;
                    if items.iter().any(|item| std::ptr::eq(item.param, param)) {
                        return Err(self.uncorrectable(
                            "repeated_parameter",
                            format!("-{} is given twice", param.name),
                            *start,
                            *end,
                            HELP_CORRECTIONS,
                        ));
                    }
                    if param.switch {
                        let off = value.as_ref().is_some_and(is_false);
                        if !off {
                            items.push(BoundParam {
                                param,
                                value: None,
                                start: *start,
                                end: *end,
                            });
                        } else if self.is_common(param) {
                            items.push(BoundParam {
                                param,
                                value: value.clone(),
                                start: *start,
                                end: *end,
                            });
                        }
                        continue;
                    }
                    let value = match value {
                        Some(value) => value.clone(),
                        None => match args.next_if(|next| matches!(next, RawArg::Value(_))) {
                            Some(RawArg::Value(value)) => value.clone(),
                            _ => {
                                return Err(self.uncorrectable(
                                    "missing_value",
                                    format!("-{} needs a value", param.name),
                                    *start,
                                    *end,
                                    HELP_CORRECTIONS,
                                ));
                            }
                        },
                    };
                    let end = value.end;
                    items.push(BoundParam {
                        param,
                        value: Some(value),
                        start: *start,
                        end,
                    });
                }
                RawArg::Value(value) => {
                    let mut candidates: Vec<&'a Param> = cmdlet
                        .params
                        .iter()
                        .filter(|param| param.position.is_some())
                        .collect();
                    candidates.sort_by_key(|param| param.position);
                    let next = candidates
                        .into_iter()
                        .find(|param| !items.iter().any(|item| std::ptr::eq(item.param, *param)));
                    let param = next.ok_or_else(|| {
                        self.uncorrectable(
                            "extra_argument",
                            format!(
                                "{} takes no further argument here; name the parameter",
                                cmdlet.name()
                            ),
                            value.start,
                            value.end,
                            HELP_CORRECTIONS,
                        )
                    })?;
                    items.push(BoundParam {
                        param,
                        value: Some(value.clone()),
                        start: value.start,
                        end: value.end,
                    });
                }
            }
        }
        if let Some(literal) = chain.literal.take() {
            let param = cmdlet
                .params
                .iter()
                .find(|param| param.from_pipeline)
                .filter(|param| !items.iter().any(|item| std::ptr::eq(item.param, *param)))
                .ok_or_else(|| {
                    self.uncorrectable(
                        "value_input",
                        format!("{} does not take a value from the pipeline", cmdlet.name()),
                        literal.start,
                        literal.end,
                        HELP_CORRECTIONS,
                    )
                })?;
            items.push(BoundParam {
                param,
                start: literal.start,
                end: literal.end,
                value: Some(literal),
            });
        }
        Ok(Bound { items })
    }

    fn resolve_param(
        &self,
        cmdlet: &'a Cmdlet,
        written: &str,
        start: usize,
        end: usize,
    ) -> Result<&'a Param, Error> {
        let all: Vec<&'a Param> = cmdlet
            .params
            .iter()
            .chain(self.table.common.iter())
            .collect();
        if let Some(param) = all.iter().copied().find(|param| param.is_named(written)) {
            return Ok(param);
        }
        let matches: Vec<&'a Param> = all
            .into_iter()
            .filter(|param| param.has_prefix(written))
            .collect();
        match matches.as_slice() {
            [param] => Ok(*param),
            [] => {
                let known: Vec<String> = cmdlet
                    .params
                    .iter()
                    .map(|param| format!("-{}", param.name))
                    .collect();
                let list = if known.is_empty() {
                    "none".to_owned()
                } else {
                    known.join(", ")
                };
                Err(self.uncorrectable(
                    "unknown_parameter",
                    format!(
                        "-{written} of {} has no Nurt translation; translated parameters: {list}",
                        cmdlet.name()
                    ),
                    start,
                    end,
                    HELP_CORRECTIONS,
                ))
            }
            several => {
                let names: Vec<String> = several
                    .iter()
                    .map(|param| format!("-{}", param.name))
                    .collect();
                Err(self.uncorrectable(
                    "ambiguous_parameter",
                    format!(
                        "-{written} matches {}; write the full name",
                        names.join(", ")
                    ),
                    start,
                    end,
                    HELP_CORRECTIONS,
                ))
            }
        }
    }

    /// Translates a cmdlet by its parameters and variants.
    fn cmdlet_command(
        &mut self,
        command: &Command,
        cmdlet: &'a Cmdlet,
        continued: bool,
        chain: &mut Chain,
    ) -> Result<(), Error> {
        let range = (command.start, command.end);
        if cmdlet.raw_args {
            let variant = cmdlet
                .variants
                .first()
                .ok_or_else(|| self.internal(range))?;
            return Err(self.variant_error(variant, &Bound { items: Vec::new() }, range));
        }
        let bound = self.bind(command, cmdlet, chain)?;
        let piped_input = self.path_input(&bound, chain)?;
        let main_path = bound
            .with_role(Role::Path)
            .filter(|_| !piped_input)
            .cloned();
        let main_text = main_path
            .as_ref()
            .and_then(|item| item.value.as_ref()?.plain_text());
        let wildcard = main_path.as_ref().is_some_and(|item| item.param.wildcards)
            && main_text.as_deref().is_some_and(convert::has_wildcard);
        let registry = main_text
            .as_deref()
            .and_then(|text| convert::path(text, &self.table.registry, self.context.base).ok())
            .is_some_and(|path| path.registry);
        let selecting_head = wildcard
            && cmdlet
                .variants
                .iter()
                .all(|variant| variant.wildcard_arg.is_none());
        let piped = !chain.is_empty() || piped_input || selecting_head;
        if let Some(location) = bound.with_role(Role::Location) {
            self.remember_location(location);
        }
        let place = Place {
            piped,
            continued,
            wildcard,
            registry,
        };
        let variant = select_variant(cmdlet, &bound, place).ok_or_else(|| {
            self.uncorrectable(
                "no_translation",
                format!(
                    "{} has no Nurt translation with these parameters",
                    cmdlet.name()
                ),
                command.start,
                command.end,
                HELP_CORRECTIONS,
            )
        })?;
        if variant.error.is_some() {
            return Err(self.variant_error(variant, &bound, range));
        }
        let missing_path = cmdlet
            .params
            .iter()
            .any(|param| param.role == Some(Role::Path))
            && main_path.is_none()
            && !piped
            && variant.merge.is_none();
        let start_len = chain.len();
        let class = self.build(command, cmdlet, variant, &bound, place, missing_path, chain)?;
        let produced: Vec<NurtStage> = chain.all_stages().into_iter().skip(start_len).collect();
        if let Some(output) = variant.output.as_ref().or(cmdlet.output.as_ref()) {
            chain.record = Some(output.clone());
        }
        let to = if produced.is_empty() {
            chain
                .last_mut()
                .map(|stage| fragment_text(std::slice::from_ref(stage)))
                .unwrap_or_default()
        } else {
            fragment_text(&produced)
        };
        self.command_correction(variant.rule, class, range, to);
        Ok(())
    }

    fn internal(&self, range: (usize, usize)) -> Error {
        self.error(
            ErrorCode::InternalCorrection,
            "the translation table entry has no variant",
            range.0,
            range.1,
        )
    }

    /// Whether the path argument is the pipeline input: `$_` or a name bound
    /// with let. A let name becomes the head of the pipeline.
    fn path_input(&mut self, bound: &Bound<'_>, chain: &mut Chain) -> Result<bool, Error> {
        let Some(item) = bound.with_role(Role::Path) else {
            return Ok(false);
        };
        let Some(Value {
            kind: ValueKind::Variable(var),
            start,
            end,
        }) = &item.value
        else {
            return Ok(false);
        };
        if var.is_pipeline_item() {
            return Ok(true);
        }
        let name = self
            .known_let(&var.name)
            .ok_or_else(|| self.unknown_variable(&var.name, *start, *end))?;
        if !chain.is_empty() {
            return Err(self.uncorrectable(
                "two_inputs",
                "the command has a pipeline input and a let input; use one",
                *start,
                *end,
                HELP_CORRECTIONS,
            ));
        }
        chain.record = self.let_record(&name).flatten();
        chain.head = Some(Head::Ref(Ref {
            name: name.clone(),
            fields: Vec::new(),
            span: Some(self.span(*start, *end)),
        }));
        let rule = self.table.syntax.assignment;
        self.record_correction(
            rule,
            (*start, *end),
            format!("@{name}"),
            CorrectionKind::Ref,
        );
        Ok(true)
    }

    fn remember_location(&mut self, item: &BoundParam<'_>) {
        let path = item
            .value
            .as_ref()
            .and_then(Value::plain_text)
            .and_then(|text| convert::path(&text, &self.table.registry, None).ok());
        if let Some(path) = path {
            self.location = Some(path.text);
        }
    }

    fn variant_error(&self, variant: &Variant, bound: &Bound<'_>, range: (usize, usize)) -> Error {
        let code = match variant.error.as_deref() {
            Some("E_ONE_EFFECT") => ErrorCode::OneEffect,
            Some("E_AMBIGUOUS") => ErrorCode::Ambiguous,
            _ => ErrorCode::Uncorrectable,
        };
        let mut error = self.error(
            code,
            variant.message.clone().unwrap_or_default(),
            range.0,
            range.1,
        );
        if let Some(problem) = &variant.problem {
            error = error.with_problem(problem);
        }
        error = error.with_help(variant.help.as_deref().unwrap_or(HELP_CORRECTIONS));
        for fix in variant
            .fix
            .iter()
            .filter_map(|template| self.fill_template(template, bound))
        {
            error = error.with_fix(fix);
        }
        if let (Some("no_current_directory"), Some(location)) =
            (variant.problem.as_deref(), &self.location)
        {
            error = error.with_detail("base", serde_json::Value::from(location.clone()));
        }
        error
    }

    /// Fills `{Param}` placeholders with the Nurt path of bound values and
    /// `{Param:text}` placeholders with the value as written; `None` when a
    /// placeholder has no usable value.
    fn fill_template(&self, template: &str, bound: &Bound<'_>) -> Option<String> {
        let mut out = String::new();
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(rest.get(..open)?);
            let after = rest.get(open + 1..)?;
            let close = after.find('}')?;
            let (name, verbatim) = match after.get(..close)?.strip_suffix(":text") {
                Some(name) => (name, true),
                None => (after.get(..close)?, false),
            };
            let value = bound.text_of(name)?;
            let text = if verbatim {
                value
            } else {
                convert::path(&value, &self.table.registry, self.context.base)
                    .ok()?
                    .text
            };
            out.push_str(&expr_text(&Expr::Text {
                v: text,
                span: None,
            }));
            rest = after.get(close + 1..)?;
        }
        out.push_str(rest);
        Some(out)
    }

    /// Builds the stages of a variant and pushes them to the chain; returns
    /// the class of the translation.
    #[expect(
        clippy::too_many_arguments,
        reason = "a variant is built from the command, its entry, the bound arguments and the place in the pipeline"
    )]
    fn build(
        &mut self,
        command: &Command,
        cmdlet: &'a Cmdlet,
        variant: &'a Variant,
        bound: &Bound<'a>,
        place: Place,
        missing_path: bool,
        chain: &mut Chain,
    ) -> Result<Class, Error> {
        let span = self.span(command.start, command.end);
        let range = (command.start, command.end);
        let mut built = Built {
            stages: variant
                .stages
                .iter()
                .map(|name| StageArgs::new(name))
                .collect(),
            emitted: Vec::new(),
            extra: Vec::new(),
            run_list: Vec::new(),
            alternative: None,
            wildcard_head: None,
            class: variant.class.unwrap_or(Class::Certain),
        };
        if missing_path {
            let location = self
                .location
                .clone()
                .ok_or_else(|| self.no_current_directory(command.start, command.end))?;
            chain.hypothetical = true;
            if let Some(stage) = built.stages.first_mut() {
                stage.positional.push(positional(text(location, span)));
            }
        }
        if let Some(param) = cmdlet
            .params
            .iter()
            .find(|param| param.required.is_some() && !bound.has(&param.name))
        {
            return Err(self.uncorrectable(
                "missing_parameter",
                param.required.clone().unwrap_or_default(),
                command.start,
                command.end,
                HELP_CORRECTIONS,
            ));
        }
        let table = self.table;
        for param in cmdlet.params.iter().chain(table.common.iter()) {
            if let Some(item) = bound.of(param) {
                self.bound_param(item, cmdlet, variant, bound, place, chain, &mut built)?;
            }
        }
        self.registry_name(bound, &mut built.stages)?;
        self.child_name(bound, &mut built.stages)?;
        self.finish_run(&mut built, chain.len(), span);
        for set in &variant.set {
            let value = set_value(&set.value, span);
            let target = match (&set.stage, &variant.merge) {
                (Some(name), _) => built.stages.iter_mut().find(|stage| &stage.name == name),
                (None, Some(_)) => continue,
                (None, None) => built.stages.first_mut(),
            };
            target
                .ok_or_else(|| self.internal(range))?
                .place(set.arg.as_deref(), vec![value]);
        }
        for absent in variant
            .absent
            .iter()
            .filter(|absent| !absent.params.iter().any(|name| bound.has(name)))
        {
            self.absent(absent, chain, &mut built.stages, span, range)?;
        }
        if let (Class::Probable, Some(assumption)) =
            (variant.class.unwrap_or(Class::Certain), &variant.assumption)
        {
            self.assume(variant.rule, assumption);
        }
        if let Some(merge) = &variant.merge {
            self.merge(variant, merge, span, chain, range)?;
            return Ok(built.class);
        }
        if let Some(head) = built.wildcard_head.take() {
            if !chain.is_empty() {
                return Err(self.uncorrectable(
                    "wildcard_in_pipeline",
                    "a path with a wildcard selects its own input; start a new program with it",
                    command.start,
                    command.end,
                    HELP_CORRECTIONS,
                ));
            }
            chain.push(head);
        }
        for stage in built.stages {
            push_stage(chain, stage.into_stage(span));
        }
        for stage in built.extra {
            chain.push(stage);
        }
        built.emitted.sort_by_key(|(order, _)| *order);
        for (_, stage) in built.emitted {
            chain.push(stage.into_stage(span));
        }
        Ok(built.class)
    }

    /// Converts one bound parameter and places its values.
    #[expect(
        clippy::too_many_arguments,
        reason = "a parameter is placed by its entry, the variant, the other arguments and the place in the pipeline"
    )]
    fn bound_param(
        &mut self,
        item: &BoundParam<'a>,
        cmdlet: &'a Cmdlet,
        variant: &'a Variant,
        bound: &Bound<'a>,
        place: Place,
        chain: &Chain,
        built: &mut Built,
    ) -> Result<(), Error> {
        let param = item.param;
        let rule = variant.rule;
        if let Some(message) = &param.error {
            return Err(self.uncorrectable(
                "unsupported_parameter",
                message.clone(),
                item.start,
                item.end,
                HELP_CORRECTIONS,
            ));
        }
        if let Some((_, note)) = variant
            .drop
            .iter()
            .find(|(name, _)| param.name.eq_ignore_ascii_case(name))
        {
            self.note(rule, note);
            return Ok(());
        }
        if self.common_param(item, &mut built.stages, rule)? {
            return Ok(());
        }
        if let Some(note) = &param.drop {
            self.note(rule, note);
            return Ok(());
        }
        match param.role {
            Some(Role::Path) if item.value.as_ref().is_some_and(is_variable) => return Ok(()),
            Some(Role::Path) if place.wildcard => {
                built.wildcard_head =
                    self.wildcard_path(item, variant, bound, &mut built.stages)?;
                return Ok(());
            }
            Some(Role::Include | Role::Exclude) => {
                built
                    .extra
                    .push(self.name_filter(item, param.role == Some(Role::Exclude))?);
                return Ok(());
            }
            Some(
                Role::NoNewline
                | Role::Selector
                | Role::Location
                | Role::ChildName
                | Role::RegistryName,
            ) => {
                return Ok(());
            }
            Some(Role::Program) => {
                if let Some(value) = &item.value {
                    let program = self.program_expr(value)?;
                    built.run_list.insert(0, program);
                }
                return Ok(());
            }
            Some(Role::Arguments) => {
                let (items, alternative) = self.arguments(item)?;
                if alternative.is_some() {
                    built.class = Class::Probable;
                    if let Some(assumption) = &param.assumption {
                        self.assume(rule, assumption);
                    }
                    built.alternative = alternative;
                }
                built.run_list.extend(items);
                return Ok(());
            }
            _ => {}
        }
        if param.ty == Some(ValueType::Context) {
            let args = self.context_args(item)?;
            let stage = built
                .stages
                .first_mut()
                .ok_or_else(|| self.internal((item.start, item.end)))?;
            stage.named.extend(args);
            return Ok(());
        }
        if param.ty.is_none() && param.set.is_none() && param.emit.is_none() {
            return Ok(());
        }
        let values = match &param.emit_with {
            Some(name) => match bound.get(name) {
                Some(source) => {
                    self.param_values(source, bound, variant, chain, &mut built.class)?
                }
                None => Vec::new(),
            },
            None => self.param_values(item, bound, variant, chain, &mut built.class)?,
        };
        let arg_name = variant
            .args
            .iter()
            .find(|(name, _)| param.name.eq_ignore_ascii_case(name))
            .map(|(_, arg)| arg.as_str())
            .or(param.arg.as_deref());
        if let Some(emit) = &param.emit {
            let mut stage = StageArgs::new(emit);
            stage.place(
                arg_name.filter(|_| param.ty != Some(ValueType::IntEquals)),
                values,
            );
            let order = cmdlet
                .params
                .iter()
                .position(|candidate| std::ptr::eq(candidate, param))
                .unwrap_or(usize::MAX);
            built.emitted.push((order, stage));
            return Ok(());
        }
        if !param.positional && arg_name.is_none() {
            return Ok(());
        }
        let target = match (&param.stage, &variant.merge) {
            (Some(name), _) => built.stages.iter_mut().find(|stage| &stage.name == name),
            (None, Some(_)) => None,
            (None, None) => built.stages.first_mut(),
        };
        let target = target.ok_or_else(|| {
            self.uncorrectable(
                "parameter_not_applicable",
                format!(
                    "-{} has no Nurt form in this use of {}",
                    param.name,
                    cmdlet.name()
                ),
                item.start,
                item.end,
                HELP_CORRECTIONS,
            )
        })?;
        target.place(
            if param.positional && !renamed_by(variant, param) {
                None
            } else {
                arg_name
            },
            values,
        );
        Ok(())
    }

    /// Puts the program and its arguments into the list of `run`; a
    /// probable split of the arguments keeps the unsplit text as an
    /// alternative.
    fn finish_run(&mut self, built: &mut Built, position: usize, span: Span) {
        if built.run_list.is_empty() {
            return;
        }
        let Some(stage) = built
            .stages
            .iter_mut()
            .find(|stage| stage.name == RUN_COMMAND)
        else {
            return;
        };
        if let Some(alternative) = built.alternative.take() {
            let mut items: Vec<Expr> = built.run_list.first().cloned().into_iter().collect();
            items.extend(alternative);
            let mut other = StageArgs::new(RUN_COMMAND);
            other.positional.push(positional(Expr::List {
                items,
                span: Some(span),
            }));
            other.named = stage.named.clone();
            self.alternatives.push(AlternativeStage {
                statement: self.statement,
                position,
                stage: other.into_stage(span),
            });
        }
        let items = std::mem::take(&mut built.run_list);
        stage.positional.insert(
            0,
            positional(Expr::List {
                items,
                span: Some(span),
            }),
        );
    }

    fn merge(
        &mut self,
        variant: &Variant,
        merge: &str,
        span: Span,
        chain: &mut Chain,
        range: (usize, usize),
    ) -> Result<(), Error> {
        let args: Vec<Arg> = variant
            .set
            .iter()
            .map(|set| {
                let value = set_value(&set.value, span);
                match &set.arg {
                    Some(name) => named(name, value),
                    None => positional(value),
                }
            })
            .collect();
        if variant.merge_next {
            chain.merge_next = Some(args);
            return Ok(());
        }
        if chain.stages.is_empty()
            && let Some(Head::Stage(head)) = &mut chain.head
            && head.name == LINES_COMMAND
            && merge == READ_COMMAND
        {
            READ_COMMAND.clone_into(&mut head.name);
        }
        match chain.last_mut() {
            Some(stage) if stage.name == merge => {
                stage.args.extend(args);
                Ok(())
            }
            _ => Err(self.uncorrectable(
                "conversion_input",
                format!("the conversion applies to the output of {merge}; read the file first"),
                range.0,
                range.1,
                HELP_CORRECTIONS,
            )),
        }
    }

    fn absent(
        &self,
        absent: &Absent,
        chain: &Chain,
        stages: &mut [StageArgs],
        span: Span,
        range: (usize, usize),
    ) -> Result<(), Error> {
        let Some(stage) = stages.first_mut() else {
            return Ok(());
        };
        if absent.self_field {
            let field = chain
                .record
                .as_deref()
                .and_then(|name| self.table.record(name))
                .and_then(|record| record.self_field.clone())
                .ok_or_else(|| {
                    self.uncorrectable(
                        "missing_property",
                        "name the property; PowerShell uses the default properties of the object",
                        range.0,
                        range.1,
                        HELP_CORRECTIONS,
                    )
                })?;
            stage.positional.push(positional(ident(field, span)));
            return Ok(());
        }
        if let (Some(arg), Some(value)) = (&absent.arg, &absent.value)
            && !stage.has_named(arg)
        {
            stage.named.push(named(arg, set_value(value, span)));
        }
        Ok(())
    }

    /// Common parameters `-WhatIf`, `-Confirm`, `-ErrorAction`, `-Verbose`
    /// and `-Debug`; returns whether the item was one.
    fn common_param(
        &mut self,
        item: &BoundParam<'_>,
        stages: &mut [StageArgs],
        variant_rule: RuleId,
    ) -> Result<bool, Error> {
        let param = item.param;
        if !self.is_common(param) {
            return Ok(false);
        }
        let rule = param.rule.unwrap_or(variant_rule);
        let range = (item.start, item.end);
        match param.role {
            Some(Role::Preview) if item.value.is_none() => {
                self.options.preview = Some(true);
                self.record_correction(
                    rule,
                    range,
                    "preview: true".to_owned(),
                    CorrectionKind::Habit,
                );
            }
            Some(Role::ErrorAction) => {
                let written = item
                    .value
                    .as_ref()
                    .and_then(Value::plain_text)
                    .unwrap_or_default();
                let mapped = param
                    .mapped_value(&written)
                    .ok_or_else(|| self.unknown_value(item, &written))?;
                let target = param
                    .stage
                    .as_ref()
                    .and_then(|name| stages.iter_mut().find(|stage| &stage.name == name));
                match (target, param.arg.as_ref()) {
                    (Some(stage), Some(arg)) if !mapped.is_empty() => {
                        stage
                            .named
                            .push(named(arg, ident(mapped, self.span(item.start, item.end))));
                        self.record_correction(
                            rule,
                            range,
                            format!("{arg}: {mapped}"),
                            CorrectionKind::Habit,
                        );
                    }
                    _ => {
                        self.record_correction(rule, range, String::new(), CorrectionKind::Habit);
                        let note = param.drop.clone().unwrap_or_default();
                        self.note(rule, &note);
                    }
                }
            }
            _ => {
                self.record_correction(rule, range, String::new(), CorrectionKind::Habit);
                let note = param.drop.clone().unwrap_or_default();
                self.note(rule, &note);
            }
        }
        Ok(true)
    }

    /// Whether the parameter is a common parameter of every cmdlet.
    fn is_common(&self, param: &Param) -> bool {
        self.table
            .common
            .iter()
            .any(|common| std::ptr::eq(common, param))
    }

    fn unknown_value(&self, item: &BoundParam<'_>, written: &str) -> Error {
        let param = item.param;
        let values: Vec<&str> = param.values.iter().map(|(name, _)| name.as_str()).collect();
        self.uncorrectable(
            "unknown_value",
            format!(
                "-{} {written} has no Nurt form; values: {}",
                param.name,
                values.join(", ")
            ),
            item.start,
            item.end,
            HELP_CORRECTIONS,
        )
    }

    /// Converts the value of a bound parameter into Nurt expressions.
    fn param_values(
        &mut self,
        item: &BoundParam<'_>,
        bound: &Bound<'_>,
        variant: &Variant,
        chain: &Chain,
        class: &mut Class,
    ) -> Result<Vec<Expr>, Error> {
        let param = item.param;
        let Some(value) = &item.value else {
            let span = self.span(item.start, item.end);
            return Ok(param.set.iter().map(|set| set_value(set, span)).collect());
        };
        let span = self.span(value.start, value.end);
        let Some(ty) = param.ty else {
            return Ok(Vec::new());
        };
        let expr = match ty {
            ValueType::Path => self.path_expr(value)?,
            ValueType::Text | ValueType::Output => self.output_value(value)?,
            ValueType::Texts => Expr::List {
                items: list_items(value)
                    .into_iter()
                    .map(|item| self.plain(item).map(|plain| text(plain, span)))
                    .collect::<Result<_, _>>()?,
                span: Some(span),
            },
            ValueType::CimEquals => {
                let written = self.plain(value)?;
                if convert::has_wildcard(&written) || written.contains(['\'', '\\']) {
                    return Err(self.value_error(
                        item,
                        "a name with wildcards, quotation marks or backslashes has no filter form; write the filter with windows.cim where:",
                    ));
                }
                text(format!("{} = '{written}'", param.name), span)
            }
            ValueType::IntEquals => Expr::Bin {
                op: BinOp::Eq,
                l: Box::new(ident(param.arg.as_deref().unwrap_or_default(), span)),
                r: Box::new(Expr::Int {
                    v: self.int(value)?,
                    span: Some(span),
                }),
                span: Some(span),
            },
            ValueType::LineText => {
                let lines: Vec<String> = list_items(value)
                    .into_iter()
                    .map(|item| self.plain(item))
                    .collect::<Result<_, _>>()?;
                let mut joined = lines.join("\n");
                if !bound.has(NO_NEWLINE_PARAM) {
                    joined.push('\n');
                }
                text(joined, span)
            }
            ValueType::Int => Expr::Int {
                v: self.int(value)?,
                span: Some(span),
            },
            ValueType::Depth => Expr::Int {
                v: self.int(value)?.saturating_add(1),
                span: Some(span),
            },
            ValueType::Upto => Expr::Range {
                lo: None,
                hi: Some(self.int(value)?),
                span: Some(span),
            },
            ValueType::Seconds | ValueType::Milliseconds => {
                let unit = if ty == ValueType::Seconds {
                    SECOND_UNIT
                } else {
                    MILLISECOND_UNIT
                };
                let ms = convert::duration_ms(self.number(value)?, unit).ok_or_else(|| {
                    self.value_error(item, "the duration must be a whole number of milliseconds")
                })?;
                Expr::Dur {
                    v: ms,
                    span: Some(span),
                }
            }
            ValueType::Fields | ValueType::Field => {
                let fields =
                    self.fields(value, chain, param.order_assumption, variant.rule, class)?;
                if ty == ValueType::Field && fields.len() != 1 {
                    return Err(self.value_error(item, "one property is grouped at a time"));
                }
                return Ok(fields);
            }
            ValueType::Enum | ValueType::EnumText | ValueType::TypeCheck => {
                let written = self.plain(value)?;
                let mapped = param
                    .mapped_value(&written)
                    .ok_or_else(|| self.unknown_value(item, &written))?;
                match ty {
                    ValueType::EnumText => text(mapped, span),
                    ValueType::TypeCheck => Expr::Bin {
                        op: BinOp::Eq,
                        l: Box::new(ident(TYPE_FIELD, span)),
                        r: Box::new(text(mapped, span)),
                        span: Some(span),
                    },
                    _ => ident(mapped, span),
                }
            }
            ValueType::Name => {
                let written = self.plain(value)?;
                if convert::has_wildcard(&written) {
                    Expr::Regex {
                        v: convert::wildcard_regex(&written),
                        flags: IGNORE_CASE_FLAG.to_owned(),
                        span: Some(span),
                    }
                } else {
                    text(written, span)
                }
            }
            ValueType::NameGlob => {
                let written = self.plain(value)?;
                if variant.recursive && !written.contains(['/', '\\']) {
                    text(format!("{RECURSIVE_GLOB}{written}"), span)
                } else {
                    text(written, span)
                }
            }
            ValueType::Regex => {
                let literal = param
                    .literal_when
                    .as_deref()
                    .is_some_and(|name| bound.has(name));
                let mut patterns = list_items(value)
                    .into_iter()
                    .map(|item| self.pattern(item, literal))
                    .collect::<Result<Vec<_>, Error>>()?;
                if patterns.len() == 1 {
                    patterns.remove(0)
                } else {
                    Expr::List {
                        items: patterns,
                        span: Some(span),
                    }
                }
            }
            ValueType::Map => self.map(value)?,
            ValueType::Value => match &value.kind {
                ValueKind::Variable(var)
                    if var.scope.is_none()
                        && (var.name.eq_ignore_ascii_case("true") || is_false(value))
                        && renamed_by(variant, param) =>
                {
                    Expr::Bool {
                        v: !is_false(value),
                        span: Some(span),
                    }
                }
                _ if renamed_by(variant, param) => {
                    return Err(self.value_error(item, "write $true or $false"));
                }
                // `write` takes the value of a registry entry as text; the
                // argument `value_type` gives its type.
                ValueKind::Number(number)
                    if number.exponent == 0
                        && number.int.is_some_and(|int| int >= 0)
                        && variant.registry != Some(true) =>
                {
                    Expr::Int {
                        v: number.int.unwrap_or_default(),
                        span: Some(span),
                    }
                }
                ValueKind::Number(_) => text(self.source(value.start, value.end), span),
                _ => text(self.plain(value)?, span),
            },
            ValueType::NewName => return self.new_name(item),
            ValueType::ContentPath => self.content_path(value)?,
            ValueType::Context | ValueType::Program | ValueType::Arguments => return Ok(Vec::new()),
        };
        Ok(vec![expr])
    }

    fn pattern(&self, value: &Value, literal: bool) -> Result<Expr, Error> {
        let span = self.span(value.start, value.end);
        let pattern = self.plain(value)?;
        if literal {
            return Ok(text(pattern, span));
        }
        regex::Regex::new(&pattern).map_err(|_| {
            self.uncorrectable(
                "regex",
                "the regular expression uses a construct Nurt does not support; use -SimpleMatch for literal text",
                value.start,
                value.end,
                "help search",
            )
        })?;
        Ok(Expr::Regex {
            v: pattern,
            flags: String::new(),
            span: Some(span),
        })
    }

    fn value_error(&self, item: &BoundParam<'_>, message: &str) -> Error {
        self.uncorrectable(
            "invalid_value",
            message.to_owned(),
            item.start,
            item.end,
            HELP_CORRECTIONS,
        )
    }

    fn plain(&self, value: &Value) -> Result<String, Error> {
        match &value.kind {
            ValueKind::Number(_) => Ok(self.source(value.start, value.end).to_owned()),
            ValueKind::Variable(var) if var.is_environment() => {
                let syntax = &self.table.syntax;
                Err(self
                    .uncorrectable(
                        "environment_variable",
                        format!(
                            "environment variables are not expanded; read the value with {} {}: \"{}\"",
                            syntax.environment_command, syntax.environment_param, var.name
                        ),
                        value.start,
                        value.end,
                        "help environment",
                    )
                    .with_fix(format!(
                        "{} {}: \"{}\"",
                        syntax.environment_command, syntax.environment_param, var.name
                    )))
            }
            ValueKind::Variable(var) => Err(self.unknown_variable(&var.name, value.start, value.end)),
            _ => value.plain_text().ok_or_else(|| {
                let environment = matches!(&value.kind, ValueKind::Expandable(parts)
                    if parts.iter().any(|part| matches!(part, StrPart::Variable(var) if var.is_environment())));
                let message = if environment {
                    "environment variables are not expanded in text; write the value itself"
                } else {
                    "text with variables or subexpressions is not expanded; write the text itself"
                };
                self.uncorrectable("expanded_text", message, value.start, value.end, HELP_CORRECTIONS)
            }),
        }
    }

    fn number(&self, value: &Value) -> Result<f64, Error> {
        match &value.kind {
            ValueKind::Number(number) if number.exponent == 0 => Ok(number.value),
            _ => self.plain(value)?.parse().map_err(|_| {
                self.uncorrectable(
                    "invalid_value",
                    "a number is required",
                    value.start,
                    value.end,
                    HELP_CORRECTIONS,
                )
            }),
        }
    }

    fn int(&self, value: &Value) -> Result<i64, Error> {
        let invalid = || {
            self.uncorrectable(
                "invalid_value",
                "a non-negative whole number is required",
                value.start,
                value.end,
                HELP_CORRECTIONS,
            )
        };
        match &value.kind {
            ValueKind::Number(number) => number
                .bytes()
                .and_then(|bytes| i64::try_from(bytes).ok())
                .ok_or_else(invalid),
            _ => self
                .plain(value)?
                .parse::<i64>()
                .ok()
                .filter(|int| *int >= 0)
                .ok_or_else(invalid),
        }
    }

    /// A path argument in Nurt notation; records the path correction.
    fn path_expr(&mut self, value: &Value) -> Result<Expr, Error> {
        let span = self.span(value.start, value.end);
        let written = self.plain(value)?;
        let path = convert::path(&written, &self.table.registry, self.context.base)
            .map_err(|problem| self.path_error(&problem, value))?;
        if path.changed {
            let rule = self.table.path_rule;
            let to = expr_text(&text(path.text.clone(), span));
            self.record_correction(rule, (value.start, value.end), to, CorrectionKind::Path);
        }
        Ok(text(path.text, span))
    }

    fn path_error(&self, problem: &PathProblem, value: &Value) -> Error {
        let (start, end) = (value.start, value.end);
        match problem {
            PathProblem::Relative => self
                .error(
                    ErrorCode::PathRelative,
                    "relative path without the call parameter base; write an absolute path or set base",
                    start,
                    end,
                )
                .with_help(HELP_CORRECTIONS),
            PathProblem::Home => self.uncorrectable(
                "home_directory",
                "~ is not expanded; write the absolute path of the home directory",
                start,
                end,
                "help host",
            ),
            PathProblem::Variable => self.uncorrectable(
                "variable_in_path",
                "variables are not expanded in paths; write the path itself",
                start,
                end,
                HELP_CORRECTIONS,
            ),
            PathProblem::Environment(name) => {
                let syntax = &self.table.syntax;
                let fix = match name {
                    Some(name) => format!("{} {}: \"{name}\"", syntax.environment_command, syntax.environment_param),
                    None => syntax.environment_command.clone(),
                };
                self.uncorrectable(
                    "environment_drive",
                    "environment variables are read with environment",
                    start,
                    end,
                    "help environment",
                )
                .with_fix(fix)
            }
        }
    }

    /// A wildcard path: the pattern becomes a glob argument of the command
    /// or a selecting stage before it.
    fn wildcard_path(
        &mut self,
        item: &BoundParam<'_>,
        variant: &Variant,
        bound: &Bound<'_>,
        stages: &mut [StageArgs],
    ) -> Result<Option<NurtStage>, Error> {
        let Some(value) = &item.value else {
            return Ok(None);
        };
        let written = self.plain(value)?;
        let (directory, pattern) = convert::split_wildcard(&written).ok_or_else(|| {
            self.uncorrectable(
                "wildcard_directory",
                "a wildcard is allowed in the last part of the path only",
                value.start,
                value.end,
                HELP_CORRECTIONS,
            )
        })?;
        // The directory starts after the opening quote of a quoted path and
        // ends before the closing one.
        let quote = usize::from(matches!(
            value.kind,
            ValueKind::Literal(_) | ValueKind::Expandable(_)
        ));
        let directory_start = value.start + quote;
        let directory_value = Value {
            kind: ValueKind::Bare(directory.to_owned()),
            start: directory_start,
            end: (directory_start + directory.len()).min(value.end.saturating_sub(quote)),
        };
        let directory_expr = self.path_expr(&directory_value)?;
        let span = self.span(value.start, value.end);
        if let Some(glob_arg) = &variant.wildcard_arg {
            let glob = if variant.recursive {
                format!("{RECURSIVE_GLOB}{pattern}")
            } else {
                pattern.to_owned()
            };
            if let Some(stage) = stages.first_mut() {
                if stage.has_named(glob_arg) {
                    return Err(self.uncorrectable(
                        "two_patterns",
                        "give the pattern in the path or in -Filter, not both",
                        item.start,
                        item.end,
                        HELP_CORRECTIONS,
                    ));
                }
                stage.positional.push(positional(directory_expr));
                stage.named.push(named(glob_arg, text(glob, span)));
            }
            return Ok(None);
        }
        let wildcard = &self.table.wildcard;
        let mut head = StageArgs::new(&wildcard.command);
        head.positional.push(positional(directory_expr));
        head.named
            .push(named(&wildcard.glob_arg, text(pattern, span)));
        for set in &wildcard.set {
            head.place(set.arg.as_deref(), vec![set_value(&set.value, span)]);
        }
        for absent in wildcard
            .absent
            .iter()
            .filter(|absent| !absent.params.iter().any(|name| bound.has(name)))
        {
            if let (Some(arg), Some(value)) = (&absent.arg, &absent.value) {
                head.named.push(named(arg, set_value(value, span)));
            }
        }
        Ok(Some(head.into_stage(span)))
    }

    /// `-Include` and `-Exclude` become `filter name ~ /…/i`.
    fn name_filter(&self, item: &BoundParam<'_>, exclude: bool) -> Result<NurtStage, Error> {
        let span = self.span(item.start, item.end);
        let value = item
            .value
            .as_ref()
            .ok_or_else(|| self.value_error(item, "a pattern is required"))?;
        let patterns: Vec<String> = list_items(value)
            .into_iter()
            .map(|item| {
                self.plain(item)
                    .map(|pattern| convert::wildcard_regex(&pattern))
            })
            .collect::<Result<_, _>>()?;
        let regex = if patterns.len() == 1 {
            patterns.concat()
        } else {
            format!("(?:{})", patterns.join("|"))
        };
        let op = if exclude {
            BinOp::NotMatch
        } else {
            BinOp::Match
        };
        Ok(NurtStage {
            name: FILTER_COMMAND.to_owned(),
            args: vec![positional(Expr::Bin {
                op,
                l: Box::new(ident(NAME_FIELD, span)),
                r: Box::new(Expr::Regex {
                    v: regex,
                    flags: IGNORE_CASE_FLAG.to_owned(),
                    span: Some(span),
                }),
                span: Some(span),
            })],
            span: Some(span),
        })
    }

    /// `-Name` of the registry: the value name is the last part of the path.
    fn registry_name(&self, bound: &Bound<'_>, stages: &mut [StageArgs]) -> Result<(), Error> {
        let Some(value) = bound
            .with_role(Role::RegistryName)
            .and_then(|item| item.value.as_ref())
        else {
            return Ok(());
        };
        let name = self.plain(value)?;
        let scheme = &self.table.registry.scheme;
        let path_arg = stages
            .iter_mut()
            .flat_map(|stage| stage.positional.iter_mut())
            .find(
                |arg| matches!(&arg.value, Expr::Text { v, .. } if v.starts_with(scheme.as_str())),
            );
        if let Some(Arg {
            value: Expr::Text { v, .. },
            ..
        }) = path_arg
        {
            v.push('/');
            v.push_str(&name);
        }
        Ok(())
    }

    /// `New-Item -Path D -Name F` names the entry `D/F`.
    fn child_name(&self, bound: &Bound<'_>, stages: &mut [StageArgs]) -> Result<(), Error> {
        let Some(item) = bound.with_role(Role::ChildName) else {
            return Ok(());
        };
        let Some(value) = &item.value else {
            return Ok(());
        };
        let name = self.plain(value)?;
        match stages
            .first_mut()
            .and_then(|stage| stage.positional.first_mut())
        {
            Some(Arg {
                value: Expr::Text { v, .. },
                ..
            }) => {
                if !v.ends_with('/') {
                    v.push('/');
                }
                v.push_str(&name.replace('\\', "/"));
                Ok(())
            }
            _ => Err(self.no_current_directory(item.start, item.end)),
        }
    }

    /// `-ArgumentList`: a list is kept; one text with white space is split,
    /// which is probable, and the unsplit text is the alternative.
    fn arguments(&self, item: &BoundParam<'_>) -> Result<(Vec<Expr>, Option<Vec<Expr>>), Error> {
        let Some(value) = &item.value else {
            return Ok((Vec::new(), None));
        };
        let span = self.span(value.start, value.end);
        if let ValueKind::List(items) = &value.kind {
            let exprs = items
                .iter()
                .map(|item| self.argument_text(item))
                .collect::<Result<_, _>>()?;
            return Ok((exprs, None));
        }
        let whole = self.plain(value)?;
        let parts: Vec<&str> = whole.split_whitespace().collect();
        if parts.len() <= 1 {
            return Ok((vec![text(whole, span)], None));
        }
        let split = parts.iter().map(|part| text(*part, span)).collect();
        Ok((split, Some(vec![text(whole, span)])))
    }

    /// `-Context 2` becomes `context: 2`; `-Context 1,3` becomes
    /// `before: 1 after: 3`.
    fn context_args(&self, item: &BoundParam<'_>) -> Result<Vec<Arg>, Error> {
        let invalid = || {
            self.value_error(
                item,
                "-Context takes one number or two numbers separated by a comma",
            )
        };
        let value = item.value.as_ref().ok_or_else(invalid)?;
        let span = self.span(value.start, value.end);
        let numbers = list_items(value)
            .into_iter()
            .map(|item| self.int(item))
            .collect::<Result<Vec<_>, _>>()?;
        let int = |v: i64| Expr::Int {
            v,
            span: Some(span),
        };
        match numbers.as_slice() {
            [both] => Ok(vec![named(CONTEXT_ARG, int(*both))]),
            [before, after] => Ok(vec![
                named(BEFORE_ARG, int(*before)),
                named(AFTER_ARG, int(*after)),
            ]),
            _ => Err(invalid()),
        }
    }

    /// `-NewName N` becomes `to: "N"`; `($_.Name + "S")` becomes `/$/ "S"`.
    fn new_name(&self, item: &BoundParam<'_>) -> Result<Vec<Expr>, Error> {
        let Some(value) = &item.value else {
            return Ok(Vec::new());
        };
        let span = self.span(value.start, value.end);
        if let Some(plain) = value.plain_text() {
            return Ok(vec![text(plain, span)]);
        }
        let inner = match &value.kind {
            ValueKind::Paren(script) => single_expression(script),
            ValueKind::Binary { .. } => Some(value),
            _ => None,
        };
        let affix = inner.and_then(|inner| match &inner.kind {
            ValueKind::Binary { op, left, right } if op == "+" => {
                if is_item_name(left) {
                    right
                        .plain_text()
                        .map(|suffix| ("$", suffix.replace('$', "$$")))
                } else if is_item_name(right) {
                    left.plain_text()
                        .map(|prefix| ("^", prefix.replace('$', "$$")))
                } else {
                    None
                }
            }
            _ => None,
        });
        let (anchor, added) = affix.ok_or_else(|| {
            self.uncorrectable(
                "computed_name",
                "a computed name has no Nurt form; rename with a pattern: rename /old/ \"new\"",
                value.start,
                value.end,
                "help rename",
            )
        })?;
        Ok(vec![
            Expr::Regex {
                v: anchor.to_owned(),
                flags: String::new(),
                span: Some(span),
            },
            text(added, span),
        ])
    }

    /// `(Get-Content F)` as an argument of `Compare-Object` is the file `F`.
    fn content_path(&mut self, value: &Value) -> Result<Expr, Error> {
        let table = self.table;
        let path = match &value.kind {
            ValueKind::Paren(script) => match single_command_of(script) {
                Some(command)
                    if table.cmdlet(&command.name).is_some_and(|cmdlet| {
                        cmdlet.name().eq_ignore_ascii_case(CONTENT_CMDLET)
                    }) =>
                {
                    command.args.iter().find_map(|arg| match arg {
                        RawArg::Value(value) => Some(value.clone()),
                        RawArg::Param { .. } => None,
                    })
                }
                _ => None,
            },
            _ => None,
        };
        match path {
            Some(path) => self.path_expr(&path),
            None => Err(self.uncorrectable(
                "compare_objects",
                "only files are compared; compare them with diff \"A\" \"B\"",
                value.start,
                value.end,
                "help diff",
            )),
        }
    }

    fn map(&self, value: &Value) -> Result<Expr, Error> {
        let span = self.span(value.start, value.end);
        let ValueKind::Hash(entries) = &value.kind else {
            return Err(self.uncorrectable(
                "invalid_value",
                "a hash table @{ name = value } is required",
                value.start,
                value.end,
                HELP_CORRECTIONS,
            ));
        };
        let entries = entries
            .iter()
            .map(|(key, value)| {
                Ok(MapEntry {
                    k: key.clone(),
                    v: text(self.plain(value)?, span),
                    span: Some(span),
                })
            })
            .collect::<Result<_, Error>>()?;
        Ok(Expr::Map {
            entries,
            span: Some(span),
        })
    }

    /// Property names mapped to the fields of the input record.
    fn fields(
        &mut self,
        value: &Value,
        chain: &Chain,
        order: bool,
        rule: RuleId,
        class: &mut Class,
    ) -> Result<Vec<Expr>, Error> {
        let record = self.input_record(chain.record.as_deref(), value.start, value.end)?;
        let mut exprs = Vec::new();
        for item in list_items(value) {
            let span = self.span(item.start, item.end);
            if matches!(item.kind, ValueKind::Hash(_)) {
                return Err(self.uncorrectable(
                    "calculated_property",
                    "calculated properties have no Nurt form; select fields and compute with evaluate",
                    item.start,
                    item.end,
                    "help select",
                ));
            }
            let property = self.plain(item)?;
            let field = record
                .field(&property)
                .ok_or_else(|| self.unknown_property(&property, record, item.start, item.end))?;
            if order && field.is_text() {
                *class = Class::Probable;
                let assumption = self.table.comparison.order_assumption.clone();
                self.assume(rule, &assumption);
            }
            exprs.push(ident(field.field.clone(), span));
        }
        Ok(exprs)
    }

    pub(crate) fn input_record(
        &self,
        record: Option<&str>,
        start: usize,
        end: usize,
    ) -> Result<&'a Record, Error> {
        let table = self.table;
        record.and_then(|name| table.record(name)).ok_or_else(|| {
            self.uncorrectable(
                "unknown_record",
                "the properties of the input objects are not known; write the Nurt fields",
                start,
                end,
                "help describe",
            )
        })
    }

    pub(crate) fn unknown_property(
        &self,
        property: &str,
        record: &Record,
        start: usize,
        end: usize,
    ) -> Error {
        self.uncorrectable(
            "unknown_property",
            format!(
                "property {property} has no field in {} records; properties: {}",
                record.name,
                record.property_names().join(", ")
            ),
            start,
            end,
            "help describe",
        )
    }

    fn where_object(
        &mut self,
        command: &Command,
        cmdlet: &Cmdlet,
        chain: &mut Chain,
    ) -> Result<(), Error> {
        if chain.is_empty() {
            return Err(self.uncorrectable(
                "filter_without_input",
                "Where-Object filters the output of a command; write the command before it",
                command.start,
                command.end,
                "help filter",
            ));
        }
        let record = self.input_record(chain.record.as_deref(), command.start, command.end)?;
        let condition = script_block::where_condition(self, command, record)?;
        let span = self.span(command.start, command.end);
        let stage = NurtStage {
            name: FILTER_COMMAND.to_owned(),
            args: vec![positional(condition.expr)],
            span: Some(span),
        };
        let class = if condition.assumptions.is_empty() {
            cmdlet.class.unwrap_or(Class::Certain)
        } else {
            Class::Probable
        };
        for assumption in &condition.assumptions {
            self.assume(cmdlet.rule, assumption);
        }
        let to = fragment_text(std::slice::from_ref(&stage));
        chain.push(stage);
        self.command_correction(cmdlet.rule, class, (command.start, command.end), to);
        Ok(())
    }

    fn foreach_object(
        &mut self,
        command: &Command,
        cmdlet: &Cmdlet,
        chain: &mut Chain,
    ) -> Result<(), Error> {
        let range = (command.start, command.end);
        if let Some(properties) = script_block::selected_properties(command) {
            let record = self.input_record(chain.record.as_deref(), command.start, command.end)?;
            let span = self.span(command.start, command.end);
            let args = properties
                .iter()
                .map(|property| {
                    record
                        .field(property)
                        .map(|field| positional(ident(field.field.clone(), span)))
                        .ok_or_else(|| {
                            self.unknown_property(property, record, command.start, command.end)
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let stage = NurtStage {
                name: SELECT_COMMAND.to_owned(),
                args,
                span: Some(span),
            };
            let to = fragment_text(std::slice::from_ref(&stage));
            chain.push(stage);
            self.command_correction(cmdlet.rule, Class::Certain, range, to);
            return Ok(());
        }
        let mut error = self.uncorrectable(
            "no_loops",
            "no loops; one pattern per step",
            range.0,
            range.1,
            HELP_CORRECTIONS,
        );
        if let Some(inner) = script_block::single_command(command) {
            let saved = (
                self.corrections.len(),
                self.assumptions.len(),
                self.notes.len(),
            );
            let mut proposal = Chain {
                head: chain.head.clone(),
                stages: chain.stages.clone(),
                record: chain.record.clone(),
                ..Chain::default()
            };
            if self.command(&inner, false, &mut proposal).is_ok() {
                if let Some(stage) = proposal.all_stages().last() {
                    error = error.with_help(format!("help {}", stage.name));
                }
                if let Some(pipeline) = proposal.pipeline(self.span(range.0, range.1)) {
                    error = error.with_fix(statement_text(&NurtStatement::Pipeline(pipeline)));
                }
            }
            self.corrections.truncate(saved.0);
            self.assumptions.truncate(saved.1);
            self.notes.truncate(saved.2);
        }
        Err(error)
    }

    /// `attrib [+r|-r|+h|-h] F` becomes `attributes "F" readonly: true`;
    /// `attrib F` becomes `info "F"`.
    fn attrib(
        &mut self,
        command: &Command,
        cmdlet: &Cmdlet,
        chain: &mut Chain,
    ) -> Result<(), Error> {
        let span = self.span(command.start, command.end);
        let mut path = None;
        let mut flags = Vec::new();
        for arg in &command.args {
            let (written, start, end, value) = match arg {
                RawArg::Param {
                    name, start, end, ..
                } => (format!("-{name}"), *start, *end, None),
                RawArg::Value(value) => (
                    value.plain_text().unwrap_or_default(),
                    value.start,
                    value.end,
                    Some(value),
                ),
            };
            let flag = ATTRIB_FLAGS
                .iter()
                .find(|(name, _, _)| name.eq_ignore_ascii_case(&written));
            match (flag, value) {
                (Some((_, name, set)), _) => flags.push(named(
                    name,
                    Expr::Bool {
                        v: *set,
                        span: Some(span),
                    },
                )),
                (None, Some(value)) if path.is_none() && !is_cmd_switch(&written) => {
                    path = Some(self.path_expr(value)?)
                }
                _ => {
                    return Err(self.uncorrectable(
                        "attrib_flag",
                        "only +r, -r, +h and -h have a Nurt form; set other attributes with attributes",
                        start,
                        end,
                        "help attributes",
                    ));
                }
            }
        }
        let path = path.ok_or_else(|| self.no_current_directory(command.start, command.end))?;
        let name = if flags.is_empty() {
            INFO_COMMAND
        } else {
            ATTRIBUTES_COMMAND
        };
        let mut args = vec![positional(path)];
        args.extend(flags);
        let stage = NurtStage {
            name: name.to_owned(),
            args,
            span: Some(span),
        };
        let to = fragment_text(std::slice::from_ref(&stage));
        chain.push(stage);
        self.command_correction(
            cmdlet.rule,
            Class::Certain,
            (command.start, command.end),
            to,
        );
        Ok(())
    }

    /// `icacls P` becomes `info "P"`; changes of permissions have no
    /// translation.
    fn icacls(
        &mut self,
        command: &Command,
        cmdlet: &Cmdlet,
        chain: &mut Chain,
    ) -> Result<(), Error> {
        let span = self.span(command.start, command.end);
        let [RawArg::Value(value)] = command.args.as_slice() else {
            return Err(self.uncorrectable(
                "permission_change",
                "permissions are changed with attributes acl:",
                command.start,
                command.end,
                "help attributes",
            ));
        };
        let path = self.path_expr(value)?;
        let stage = NurtStage {
            name: INFO_COMMAND.to_owned(),
            args: vec![positional(path)],
            span: Some(span),
        };
        let to = fragment_text(std::slice::from_ref(&stage));
        chain.push(stage);
        self.command_correction(
            cmdlet.rule,
            Class::Certain,
            (command.start, command.end),
            to,
        );
        Ok(())
    }
}

fn set_value(value: &SetValue, span: Span) -> Expr {
    match value {
        SetValue::Bool(flag) => Expr::Bool {
            v: *flag,
            span: Some(span),
        },
        SetValue::Int(int) => Expr::Int {
            v: *int,
            span: Some(span),
        },
        SetValue::Ident(name) => ident(name.clone(), span),
        SetValue::Text(value) => text(value.clone(), span),
        SetValue::Call(function) => Expr::Call {
            function: function.clone(),
            args: Vec::new(),
            span: Some(span),
        },
    }
}

/// Pushes a stage; the arguments of a pending `ConvertTo-Json` join it.
fn push_stage(chain: &mut Chain, mut stage: NurtStage) {
    if let Some(args) = chain.merge_next.take() {
        for arg in args {
            let present =
                arg.name.is_some() && stage.args.iter().any(|known| known.name == arg.name);
            if !present {
                stage.args.push(arg);
            }
        }
    }
    chain.push(stage);
}

/// Elements of a comma list, or the value itself.
pub(crate) fn list_items(value: &Value) -> Vec<&Value> {
    match &value.kind {
        ValueKind::List(items) => items.iter().collect(),
        _ => vec![value],
    }
}

fn single_expression(script: &super::syntax::Script) -> Option<&Value> {
    match script.statements.as_slice() {
        [
            Statement {
                kind: StatementKind::Pipeline(Pipeline { elements, .. }),
                ..
            },
        ] => match elements.as_slice() {
            [Element::Expression(value)] => Some(value),
            _ => None,
        },
        _ => None,
    }
}

fn single_command_of(script: &super::syntax::Script) -> Option<&Command> {
    match script.statements.as_slice() {
        [
            Statement {
                kind: StatementKind::Pipeline(Pipeline { elements, .. }),
                ..
            },
        ] => match elements.as_slice() {
            [Element::Command(command)] => Some(command),
            _ => None,
        },
        _ => None,
    }
}

/// Whether the variant gives its own Nurt argument to the parameter.
fn renamed_by(variant: &Variant, param: &Param) -> bool {
    variant
        .args
        .iter()
        .any(|(name, _)| param.name.eq_ignore_ascii_case(name))
}

fn is_false(value: &Value) -> bool {
    matches!(&value.kind, ValueKind::Variable(var) if var.scope.is_none() && var.name.eq_ignore_ascii_case("false"))
}

fn is_variable(value: &Value) -> bool {
    matches!(value.kind, ValueKind::Variable(_))
}

/// A switch of a Windows program, `/s`.
fn is_cmd_switch(written: &str) -> bool {
    written.len() == 2 && written.starts_with('/')
}

/// `$_.Name`.
fn is_item_name(value: &Value) -> bool {
    matches!(
        &value.kind,
        ValueKind::Member { base, name, args: None }
            if matches!(&base.kind, ValueKind::Variable(var) if var.is_pipeline_item()) && name.eq_ignore_ascii_case("Name")
    )
}

/// Whether the variable is a preference variable of PowerShell, which changes
/// the behaviour of every later command and has no counterpart in Nurt.
fn is_preference_variable(name: &str) -> bool {
    PREFERENCE_VARIABLES
        .iter()
        .any(|known| known.eq_ignore_ascii_case(name))
}

/// The let name of a PowerShell variable: lower case, `[a-z][a-z0-9_]*`, and
/// not a word of the language.
pub(crate) fn let_name(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let mut chars = lower.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    (valid && !RESERVED_NAMES.contains(&lower.as_str())).then_some(lower)
}

/// The first variant whose conditions hold.
fn select_variant<'t>(cmdlet: &'t Cmdlet, bound: &Bound<'_>, place: Place) -> Option<&'t Variant> {
    cmdlet.variants.iter().find(|variant| {
        variant.when.iter().all(|name| bound.has(name))
            && (variant.when_any.is_empty() || variant.when_any.iter().any(|name| bound.has(name)))
            && !variant.unless.iter().any(|name| bound.has(name))
            && variant.when_value.iter().all(|(name, value)| {
                bound
                    .text_of(name)
                    .is_some_and(|written| written.eq_ignore_ascii_case(value))
            })
            && variant.piped.is_none_or(|piped| piped == place.piped)
            && variant
                .continued
                .is_none_or(|continued| continued == place.continued)
            && variant
                .wildcard
                .is_none_or(|wildcard| wildcard == place.wildcard)
            && variant
                .registry
                .is_none_or(|registry| registry == place.registry)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn let_names_are_lower_case_identifiers() {
        assert_eq!(let_name("Files"), Some("files".to_owned()));
        assert_eq!(let_name("my_list2"), Some("my_list2".to_owned()));
        assert_eq!(let_name("_x"), None);
        assert_eq!(let_name("a-b"), None);
    }

    #[test]
    fn words_of_the_language_are_not_let_names() {
        for word in ["null", "true", "False", "in", "and", "or", "not", "let"] {
            assert_eq!(let_name(word), None, "{word}");
        }
        assert_eq!(let_name("nullable"), Some("nullable".to_owned()));
        assert_eq!(let_name("input"), Some("input".to_owned()));
    }

    #[test]
    fn preference_variables_are_recognized_in_any_letter_case() {
        assert!(is_preference_variable("ErrorActionPreference"));
        assert!(is_preference_variable("erroractionpreference"));
        assert!(is_preference_variable("ProgressPreference"));
        assert!(!is_preference_variable("colorPreference"));
        assert!(!is_preference_variable("Preference"));
    }

    #[test]
    fn windows_program_switches_are_recognized() {
        assert!(is_cmd_switch("/s"));
        assert!(!is_cmd_switch("/srv/a"));
    }
}
