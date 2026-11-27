// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Translation of shell structure: lists, `&&`, `||`, `;`, pipes, `cd`,
//! assignments, prefixes (`sudo`, `time`, `timeout`, `nohup`, `env`),
//! redirections, parameter expansions and command substitutions.

use std::cell::{Cell, RefCell};

use nurt_contracts::ast::{
    Arg, AstDocument, Expr, Head, Let, MapEntry, Pipeline, Ref, Span, Stage, Statement,
};
use nurt_contracts::error::{At, Candidate, Error, ErrorCode, Stage as ErrorStage};
use nurt_contracts::report::{self, CorrectionClass, TextPosition};

use crate::record::Correction;

use super::lex::{RedirectOp, Word, WordPart, lex};
use super::syntax::{self, Command, Connector, Item, Located, Pipe, Script, Separator, Simple};
use super::table::BashTable;
use super::template::{Bound, Conversion, Convert};
use super::tree::{document, print_document, print_pipeline, print_program};
use super::{Assumption, Mode, TranslateContext, Translation, detect, engine, produce};

/// Largest number of alternatives kept for one translation.
pub(crate) const MAX_ALTERNATIVES: usize = 3;

/// Flags of the programs that elevate (`sudo`, `doas`, `run0`) that change
/// nothing Nurt needs to know: they only keep the program from asking, from
/// keeping the environment of the account or from refreshing a timestamp.
const ELEVATION_PLAIN_FLAGS: &[&str] = &[
    "-n",
    "--non-interactive",
    "-H",
    "--set-home",
    "-E",
    "--preserve-env",
    "-k",
    "-b",
    "--background",
    "--no-ask-password",
];

/// Flags of the programs that elevate that open a shell or an interactive
/// session instead of running one command.
const ELEVATION_SESSION_FLAGS: &[&str] = &["-i", "--login", "-s", "--shell"];

/// Flags that name the account to run as.
const ELEVATION_USER_FLAGS: &[&str] = &["-u", "--user"];

/// Deepest nesting of command prefixes and command substitutions; a deeper
/// one is an error, so that crafted text cannot exhaust the stack.
const MAX_NESTING: usize = 32;

/// A translated pipeline fragment.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Piece {
    pub(crate) stages: Vec<Stage>,
    pub(crate) kind: String,
    pub(crate) why: Option<String>,
}

/// Interpretations of a fragment, the likeliest first.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Variants(pub(crate) Vec<Piece>);

impl Variants {
    pub(crate) fn one(stages: Vec<Stage>, kind: &str) -> Self {
        Self(vec![Piece {
            stages,
            kind: kind.to_owned(),
            why: None,
        }])
    }

    pub(crate) fn primary(&self) -> &Piece {
        &self.0[0]
    }

    pub(crate) fn primary_mut(&mut self) -> &mut Piece {
        &mut self.0[0]
    }

    /// Joins two fragments: each alternative of one side is combined with
    /// the primary interpretation of the other.
    pub(crate) fn then(self, next: Variants) -> Variants {
        let join = |a: &Piece, b: &Piece| Piece {
            stages: a.stages.iter().chain(&b.stages).cloned().collect(),
            kind: b.kind.clone(),
            why: b.why.clone().or_else(|| a.why.clone()),
        };
        let first = self.primary();
        let second = next.primary();
        let mut pieces = vec![join(first, second)];
        pieces.extend(self.0.iter().skip(1).map(|a| join(a, second)));
        pieces.extend(next.0.iter().skip(1).map(|b| join(first, b)));
        pieces.truncate(MAX_ALTERNATIVES + 1);
        Variants(pieces)
    }

    pub(crate) fn map(self, f: impl Fn(Piece) -> Piece) -> Variants {
        Variants(self.0.into_iter().map(f).collect())
    }
}

#[derive(Debug, Clone)]
struct Pending {
    rule: String,
    class: CorrectionClass,
    span: Span,
    to: String,
    kind: &'static str,
}

#[derive(Debug, Default)]
struct Log {
    corrections: Vec<Pending>,
    assumptions: Vec<Assumption>,
    lets: Vec<Let>,
    named: Vec<String>,
}

/// Options of a program run collected from prefixes and redirections.
#[derive(Debug, Clone, Default)]
pub(crate) struct RunOptions {
    pub(crate) env: Vec<(String, Bound)>,
    pub(crate) timeout: Option<Bound>,
    pub(crate) background: bool,
    pub(crate) merge_stderr: bool,
    pub(crate) quiet: bool,
    pub(crate) span: Option<Span>,
}

impl RunOptions {
    fn is_empty(&self) -> bool {
        self.env.is_empty()
            && self.timeout.is_none()
            && !self.background
            && !self.merge_stderr
            && !self.quiet
    }
}

pub(crate) struct Session<'a> {
    pub(crate) table: &'a BashTable,
    pub(crate) context: &'a TranslateContext<'a>,
    lines: Vec<Vec<char>>,
    cwd: RefCell<Option<String>>,
    log: RefCell<Log>,
    followed: Cell<bool>,
    /// A `set` of the strict mode was the whole statement of a script line.
    strict_only: Cell<bool>,
    depth: Cell<usize>,
}

pub(crate) fn translate(
    input: &str,
    context: &TranslateContext<'_>,
    table: &BashTable,
) -> Option<Result<Translation, Error>> {
    let tokens = lex(input).ok()?;
    let has_word = tokens
        .iter()
        .any(|token| matches!(token.kind, super::lex::PosixTokenKind::Word(_)));
    if !has_word {
        return None;
    }
    if context.mode == Mode::Live && !detect::is_habit(table, &tokens) {
        return None;
    }
    Some(translate_tokens(input, &tokens, context, table))
}

/// Translates a statement that starts with a habit word and shows no Nurt
/// notation, whether or not it shows a feature of the shell.
pub(crate) fn translate_by_word(
    input: &str,
    context: &TranslateContext<'_>,
    table: &BashTable,
) -> Option<Result<Translation, Error>> {
    let tokens = lex(input).ok()?;
    detect::is_habit_by_word(table, &tokens)
        .then(|| translate_tokens(input, &tokens, context, table))
}

fn translate_tokens(
    input: &str,
    tokens: &[super::lex::PosixToken],
    context: &TranslateContext<'_>,
    table: &BashTable,
) -> Result<Translation, Error> {
    let session = Session::new(table, context, input);
    match syntax::parse(tokens) {
        Ok(script) => session.script(&script),
        Err(error) => Err(session.error(
            ErrorCode::Uncorrectable,
            "unsupported_syntax",
            error.span,
            &[("reason", error.message)],
        )),
    }
}

/// A step of a list: a pipeline with the connector before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    First,
    And,
    Or,
    Sequence,
}

#[derive(Debug, Clone, Copy)]
struct Step<'s> {
    pipe: &'s Pipe,
    link: Link,
    background: bool,
}

fn steps(script: &Script) -> Vec<Step<'_>> {
    let mut steps = Vec::new();
    for item in &script.items {
        let link = if steps.is_empty() {
            Link::First
        } else {
            Link::Sequence
        };
        push_item(&mut steps, item, link);
    }
    steps
}

fn push_item<'s>(steps: &mut Vec<Step<'s>>, item: &'s Item, link: Link) {
    let background = item.separator == Some(Separator::Background);
    let count = item.rest.len() + 1;
    let pipes =
        std::iter::once((link, &item.first)).chain(item.rest.iter().map(|(connector, pipe)| {
            let link = match connector {
                Connector::And => Link::And,
                Connector::Or => Link::Or,
            };
            (link, pipe)
        }));
    for (index, (link, pipe)) in pipes.enumerate() {
        steps.push(Step {
            pipe,
            link,
            background: background && index + 1 == count,
        });
    }
}

impl<'a> Session<'a> {
    pub(crate) fn new(
        table: &'a BashTable,
        context: &'a TranslateContext<'a>,
        input: &str,
    ) -> Self {
        Self {
            table,
            context,
            lines: input
                .strip_prefix('\u{feff}')
                .unwrap_or(input)
                .split('\n')
                .map(|line| line.chars().collect())
                .collect(),
            cwd: RefCell::new(None),
            log: RefCell::new(Log::default()),
            followed: Cell::new(false),
            strict_only: Cell::new(false),
            depth: Cell::new(0),
        }
    }

    /// Enters one level of nesting: a command prefix or a command
    /// substitution.
    fn enter(&self, span: Span) -> Result<(), Error> {
        let depth = self.depth.get();
        if depth >= MAX_NESTING {
            return Err(self.error(
                ErrorCode::Uncorrectable,
                "unsupported_syntax",
                span,
                &[("reason", "too many nested commands")],
            ));
        }
        self.depth.set(depth + 1);
        Ok(())
    }

    /// Starts the translation of a command substitution of `outer` in this
    /// session, one level deeper.
    pub(crate) fn nested_in(&self, outer: &Session<'_>, span: Span) -> Result<(), Error> {
        self.depth.set(outer.depth.get());
        self.enter(span)
    }

    /// Translates the words that follow a command prefix.
    pub(crate) fn nested_words(
        &self,
        simple: &Simple,
        words: &[Located<Word>],
        input: Option<&str>,
        options: &RunOptions,
    ) -> Result<Variants, Error> {
        self.enter(simple.span)?;
        let result = self.words(simple, words, input, options);
        self.depth.set(self.depth.get().saturating_sub(1));
        result
    }

    fn script(&self, script: &Script) -> Result<Translation, Error> {
        let steps = steps(script);
        if let Some((span, keyword)) = compound(&steps) {
            return Err(self.loop_error(span, keyword));
        }
        if let Some(index) = steps.iter().position(|step| step.link == Link::Or) {
            return Err(self.alternative_branch(&steps, index));
        }
        let statements = self.statements(&steps)?;
        match self.context.mode {
            Mode::Live => self.finish_live(statements),
            Mode::Script => self.finish_script(statements),
        }
    }

    /// Translates every step after `cd`, `export` and `test … &&` prefixes
    /// were applied; each result is one pipeline with its interpretations.
    fn statements(&self, steps: &[Step<'_>]) -> Result<Vec<(Variants, Span)>, Error> {
        let mut env: Vec<(String, Bound)> = Vec::new();
        let mut results = Vec::new();
        let mut index = 0;
        while index < steps.len() {
            let step = steps[index];
            let last = index + 1 == steps.len();
            if let Some(simple) = single_simple(step.pipe) {
                if let Some(found) = produce::assignment_of(simple) {
                    produce::assignment(self, simple, found)?;
                    index += 1;
                    continue;
                }
                match self.word(simple, 0).as_deref() {
                    Some("cd") => {
                        self.change_directory(simple, last)?;
                        index += 1;
                        continue;
                    }
                    Some("export") => {
                        env.extend(self.export(simple, last)?);
                        index += 1;
                        continue;
                    }
                    Some("set")
                        if self.is_strict_mode(simple)
                            && (!last || self.context.mode == Mode::Script) =>
                    {
                        // Nurt stops a program at the first error, which is
                        // what `set -e` and `pipefail` ask a shell for. In a
                        // script the line may hold nothing else.
                        self.assume("B22", "script_stops");
                        self.strict_only.set(last);
                        index += 1;
                        continue;
                    }
                    Some("test" | "[") if !last && steps[index + 1].link == Link::And => {
                        let guarded = produce::guard(self, simple, steps[index + 1].pipe)?;
                        results.push((guarded, join_steps(step, steps[index + 1])));
                        index += 2;
                        continue;
                    }
                    _ => {}
                }
            }
            if step.link == Link::Sequence && self.context.mode == Mode::Script {
                self.assume("B22", "script_stops");
            }
            let options = RunOptions {
                env: env.clone(),
                background: step.background,
                span: Some(step.pipe.span),
                ..RunOptions::default()
            };
            results.push((self.pipe(step.pipe, &options)?, step.pipe.span));
            index += 1;
        }
        Ok(results)
    }

    fn word(&self, simple: &Simple, index: usize) -> Option<String> {
        simple
            .words
            .get(index)
            .and_then(|word| word.value.literal())
    }

    /// Whether the command is `set` with the options `-e`, `-u` and
    /// `-o pipefail` only, written as `-e -u -o pipefail` or joined as in
    /// `-euo pipefail`.
    fn is_strict_mode(&self, simple: &Simple) -> bool {
        let mut seen = false;
        let mut expecting_name = false;
        for index in 1..simple.words.len() {
            let Some(arg) = self.word(simple, index) else {
                return false;
            };
            if expecting_name {
                if arg != "pipefail" {
                    return false;
                }
                expecting_name = false;
                continue;
            }
            let Some(flags) = arg.strip_prefix('-') else {
                return false;
            };
            let valid = !flags.is_empty()
                && flags.chars().all(|flag| matches!(flag, 'e' | 'u' | 'o'))
                && flags.matches('o').count() == usize::from(flags.ends_with('o'));
            if !valid {
                return false;
            }
            seen = true;
            expecting_name = flags.ends_with('o');
        }
        seen && !expecting_name
    }

    fn change_directory(&self, simple: &Simple, last: bool) -> Result<(), Error> {
        let target = simple.words.get(1).and_then(|word| word.value.literal());
        let absolute = target.as_deref().is_some_and(is_absolute) && simple.words.len() == 2;
        if last || !absolute {
            return Err(self.error(
                ErrorCode::Uncorrectable,
                "current_directory",
                simple.span,
                &[],
            ));
        }
        let directory = trim_trailing_slash(target.as_deref().unwrap_or_default());
        *self.cwd.borrow_mut() = Some(directory.clone());
        self.record(
            "B20",
            CorrectionClass::Certain,
            simple.span,
            format!("cwd: {}", super::tree::print_text(&directory)),
            "habit",
        );
        Ok(())
    }

    fn export(&self, simple: &Simple, last: bool) -> Result<Vec<(String, Bound)>, Error> {
        if last {
            return Err(self.error(
                ErrorCode::Uncorrectable,
                "environment_without_program",
                simple.span,
                &[],
            ));
        }
        let mut env = Vec::new();
        for word in &simple.words[1..] {
            let Some((name, value)) = word.value.raw.split_once('=') else {
                return Err(self.error(
                    ErrorCode::Uncorrectable,
                    "environment_without_program",
                    simple.span,
                    &[],
                ));
            };
            let invalid = || {
                self.error(
                    ErrorCode::Uncorrectable,
                    "environment_without_program",
                    simple.span,
                    &[],
                )
            };
            let bound = match lex(value).map_err(|_| invalid())?.into_iter().next() {
                None => self.text_bound(String::new(), word.span),
                Some(super::lex::PosixToken {
                    kind: super::lex::PosixTokenKind::Word(inner),
                    ..
                }) => self.operand(&Located {
                    value: inner,
                    span: word.span,
                })?,
                Some(_) => return Err(invalid()),
            };
            env.push((name.to_owned(), bound));
        }
        Ok(env)
    }

    pub(crate) fn text_bound(&self, text: String, span: Span) -> Bound {
        Bound::Word {
            text,
            span,
            bare: false,
        }
    }

    /// Translates a pipeline `A | B | C` into `A' |> B' |> C'`.
    pub(crate) fn pipe(&self, pipe: &Pipe, options: &RunOptions) -> Result<Variants, Error> {
        if pipe.negated {
            return Err(self.error(
                ErrorCode::Uncorrectable,
                "unsupported_syntax",
                pipe.span,
                &[("reason", "negated pipeline")],
            ));
        }
        if pipe.merges_stderr {
            return Err(self.error(
                ErrorCode::Uncorrectable,
                "unsupported_syntax",
                pipe.span,
                &[("reason", "standard error in a pipe (`|&`)")],
            ));
        }
        if let Some(result) = produce::echo_into_tee(self, pipe) {
            return result;
        }
        let last = pipe.commands.len() - 1;
        let mut result: Option<Variants> = None;
        for (index, command) in pipe.commands.iter().enumerate() {
            let simple = match command {
                Command::Simple(simple) => simple,
                Command::Compound { keyword, span } => return Err(self.loop_error(*span, keyword)),
            };
            let own = if index == last {
                options.clone()
            } else {
                RunOptions::default()
            };
            let name = simple.words.first().and_then(|word| word.value.literal());
            result = Some(match (result, name.as_deref()) {
                (None, _) => {
                    self.followed.set(pipe.commands.len() > 1);
                    let head = self.simple(simple, None, &own);
                    self.followed.set(false);
                    head?
                }
                (Some(previous), Some("xargs")) => produce::xargs(self, simple, previous)?,
                (Some(previous), Some("tee")) if index == last => {
                    produce::tee(self, simple, previous)?
                }
                (Some(previous), _) => {
                    let kind = previous.primary().kind.clone();
                    let stage = self.simple(simple, Some(&kind), &own)?;
                    self.record(
                        "B24",
                        CorrectionClass::Certain,
                        simple.span,
                        piece_text(stage.primary()),
                        "pipe",
                    );
                    previous.then(stage)
                }
            });
        }
        let result = result.ok_or_else(|| {
            self.error(
                ErrorCode::Uncorrectable,
                "unsupported_syntax",
                pipe.span,
                &[("reason", "empty pipeline")],
            )
        })?;
        match pipe.commands.last() {
            Some(Command::Simple(simple)) => match produce::output_target(simple) {
                Some(output) => produce::write_from(self, &output, result),
                None => Ok(result),
            },
            _ => Ok(result),
        }
    }

    /// Translates a simple command at the head of a pipeline (`input` is
    /// `None`) or as a stage receiving records of the given kind.
    pub(crate) fn simple(
        &self,
        simple: &Simple,
        input: Option<&str>,
        options: &RunOptions,
    ) -> Result<Variants, Error> {
        let mut options = options.clone();
        for assignment in &simple.assignments {
            let word = Located {
                value: assignment.value.clone(),
                span: assignment.span,
            };
            options
                .env
                .push((assignment.name.clone(), self.operand(&word)?));
        }
        produce::redirected(self, simple, input, &mut options)
    }

    /// Translates the words of a simple command after redirections were
    /// handled.
    pub(crate) fn words(
        &self,
        simple: &Simple,
        words: &[Located<Word>],
        input: Option<&str>,
        options: &RunOptions,
    ) -> Result<Variants, Error> {
        let Some(first) = words.first() else {
            return Err(self.error(
                ErrorCode::Uncorrectable,
                "unsupported_syntax",
                simple.span,
                &[("reason", "no command")],
            ));
        };
        // The shell expands a brace expansion into several words; read as one
        // word it would name a path that does not exist, and a change would
        // silently select nothing.
        if let Some(word) = words.iter().find(|word| word.value.has_brace_expansion()) {
            return Err(self.error(
                ErrorCode::Uncorrectable,
                "unsupported_syntax",
                word.span,
                &[("reason", "brace expansion")],
            ));
        }
        let name = first
            .value
            .literal()
            .unwrap_or_else(|| first.value.raw.clone());
        match name.as_str() {
            "sudo" | "doas" | "run0" => {
                return self.elevated(&name, first.span, simple, &words[1..], input, options);
            }
            "su" => return Err(self.elevated_session(simple.span, &name)),
            "time" | "nohup" if words.len() > 1 => {
                let inner = self.nested_words(simple, &words[1..], input, options)?;
                if name == "time" {
                    self.record(
                        "B30",
                        CorrectionClass::Certain,
                        first.span,
                        piece_text(inner.primary()),
                        "habit",
                    );
                }
                return Ok(inner);
            }
            "command" if words.len() > 1 && !is_lookup(words) => {
                return self.nested_words(simple, &words[1..], input, options);
            }
            "timeout" if words.len() > 2 => {
                let mut options = options.clone();
                options.timeout = Some(self.operand(&words[1])?);
                let inner = self.nested_words(simple, &words[2..], input, &options)?;
                self.record(
                    "B30",
                    CorrectionClass::Certain,
                    first.span,
                    piece_text(inner.primary()),
                    "habit",
                );
                return Ok(inner);
            }
            "env" if words.len() > 1 => {
                return self.env_prefix(simple, &words[1..], input, options);
            }
            "cd" | "export" => {
                return Err(self.error(
                    ErrorCode::Uncorrectable,
                    "current_directory",
                    simple.span,
                    &[],
                ));
            }
            _ => {}
        }
        let names: Vec<String> = words
            .iter()
            .map(|word| word.value.literal().unwrap_or_default())
            .collect();
        let translated = match self.table.command(&names) {
            _ if name == "git" => engine::git(self, simple, words, input)?,
            Some(entry) => {
                engine::command(self, entry, simple, &words[entry.words.len()..], input)?
            }
            None if self.table.is_habit(&name) => {
                return Err(self.error(
                    ErrorCode::Uncorrectable,
                    "no_translation",
                    simple.span,
                    &[("command", &name)],
                ));
            }
            None if input.is_some() => {
                return Err(self.error(
                    ErrorCode::Uncorrectable,
                    "unsupported_stage",
                    simple.span,
                    &[("command", &name)],
                ));
            }
            None => self.run_program_with(simple, words, None, options)?,
        };
        self.apply_run_options(translated, options, simple.span)
    }

    fn env_prefix(
        &self,
        simple: &Simple,
        words: &[Located<Word>],
        input: Option<&str>,
        options: &RunOptions,
    ) -> Result<Variants, Error> {
        let mut options = options.clone();
        let mut rest = words;
        while let Some((first, tail)) = rest.split_first() {
            let Some(text) = first.value.literal() else {
                break;
            };
            let Some((name, value)) = text.split_once('=') else {
                break;
            };
            options.env.push((
                name.to_owned(),
                self.text_bound(value.to_owned(), first.span),
            ));
            rest = tail;
        }
        if rest.is_empty() {
            return Err(self.error(
                ErrorCode::Uncorrectable,
                "environment_without_program",
                simple.span,
                &[],
            ));
        }
        self.nested_words(simple, rest, input, &options)
    }

    /// `sudo CMD`, `doas CMD`, `run0 CMD`: the translation of `CMD` with the
    /// call parameter `privilege: elevated` written on its last stage, and
    /// `as_user: "NAME"` for `-u NAME`. An elevated session (`sudo -i`, `sudo -s`) is not a command
    /// and ends with `E_AMBIGUOUS`.
    fn elevated(
        &self,
        program: &str,
        span: Span,
        simple: &Simple,
        words: &[Located<Word>],
        input: Option<&str>,
        options: &RunOptions,
    ) -> Result<Variants, Error> {
        let mut inner = words;
        let mut user: Option<String> = None;
        while let Some((first, tail)) = inner.split_first() {
            let flag = first.value.raw.as_str();
            if flag == "--" {
                inner = tail;
                break;
            }
            if !flag.starts_with('-') || flag == "-" {
                break;
            }
            if ELEVATION_SESSION_FLAGS.contains(&flag) {
                return Err(self.elevated_session(simple.span, program));
            }
            if let Some(name) = flag.strip_prefix("--user=") {
                user = Some(name.to_owned());
                inner = tail;
            } else if ELEVATION_USER_FLAGS.contains(&flag) {
                let Some((name, rest)) = tail.split_first() else {
                    return Err(self.elevated_option(first.span, flag));
                };
                user = Some(
                    name.value
                        .literal()
                        .unwrap_or_else(|| name.value.raw.clone()),
                );
                inner = rest;
            } else if ELEVATION_PLAIN_FLAGS.contains(&flag) {
                inner = tail;
            } else {
                return Err(self.elevated_option(first.span, flag));
            }
        }
        if inner.is_empty() {
            return Err(self.elevated_session(simple.span, program));
        }
        let variants = self.nested_words(simple, inner, input, options)?;
        let call = match &user {
            Some(name) => named(
                "as_user",
                Expr::Text {
                    v: name.clone(),
                    span: None,
                },
            ),
            None => named(
                "privilege",
                Expr::Ident {
                    v: "elevated".to_owned(),
                    span: None,
                },
            ),
        };
        let variants = variants.map(|mut piece| {
            if let Some(stage) = piece.stages.last_mut() {
                append_args(stage, vec![call.clone()]);
            }
            piece
        });
        self.record(
            "B28",
            CorrectionClass::Certain,
            span,
            piece_text(variants.primary()),
            "habit",
        );
        Ok(variants)
    }

    /// `E_AMBIGUOUS`: an elevated session is not a command.
    fn elevated_session(&self, span: Span, program: &str) -> Error {
        self.error(
            ErrorCode::Ambiguous,
            "elevated_session",
            span,
            &[("program", program)],
        )
        .with_next_step("give the command itself with the call parameter `privilege: elevated`")
        .with_next_step(
            "give the account with the call parameter `as_user: \"NAME\"` to run as another account",
        )
    }

    /// `E_UNCORRECTABLE`: a flag of an elevating program that Nurt cannot
    /// keep.
    fn elevated_option(&self, span: Span, flag: &str) -> Error {
        self.error(
            ErrorCode::Uncorrectable,
            "elevated_option",
            span,
            &[("flag", flag)],
        )
    }

    /// Adds run options to the translated program; they apply only to
    /// `run`.
    fn apply_run_options(
        &self,
        variants: Variants,
        options: &RunOptions,
        span: Span,
    ) -> Result<Variants, Error> {
        if options.is_empty() {
            return Ok(variants);
        }
        let last_stage = variants.primary().stages.last();
        let is_run = last_stage.is_some_and(|stage| stage.name == "run");
        let only_timeout = options.timeout.is_some()
            && options.env.is_empty()
            && !options.background
            && !options.merge_stderr
            && !options.quiet;
        let accepts_timeout = last_stage.is_some_and(|stage| {
            self.table
                .params
                .get(&stage.name)
                .is_some_and(|params| params.iter().any(|param| param == "timeout"))
        });
        if !is_run && !(only_timeout && accepts_timeout) {
            let problem = if options.background {
                "background_change"
            } else {
                "options_without_program"
            };
            return Err(self.error(
                ErrorCode::Uncorrectable,
                problem,
                options.span.unwrap_or(span),
                &[],
            ));
        }
        let args = self.run_args(options);
        let variants = variants.map(|mut piece| {
            if let Some(stage) = piece.stages.last_mut() {
                append_args(stage, args.clone());
            }
            piece
        });
        let text = piece_text(variants.primary());
        if !options.env.is_empty() {
            self.record(
                "B31",
                CorrectionClass::Certain,
                options.span.unwrap_or(span),
                text.clone(),
                "habit",
            );
        }
        if options.background {
            self.record(
                "B29",
                CorrectionClass::Certain,
                options.span.unwrap_or(span),
                text,
                "habit",
            );
        }
        Ok(variants)
    }

    fn run_args(&self, options: &RunOptions) -> Vec<Arg> {
        let mut args = Vec::new();
        if !options.env.is_empty() {
            let entries = options
                .env
                .iter()
                .filter_map(|(name, value)| {
                    self.convert(value, Conversion::Text)
                        .ok()
                        .map(|v| MapEntry {
                            k: name.clone(),
                            v,
                            span: None,
                        })
                })
                .collect();
            args.push(named(
                "env",
                Expr::Map {
                    entries,
                    span: None,
                },
            ));
        }
        if let Some(timeout) = &options.timeout
            && let Ok(value) = self.convert(timeout, Conversion::Duration)
        {
            args.push(named("timeout", value));
        }
        if options.background {
            args.push(named(
                "background",
                Expr::Bool {
                    v: true,
                    span: None,
                },
            ));
        }
        if options.merge_stderr {
            args.push(named(
                "merge_stderr",
                Expr::Bool {
                    v: true,
                    span: None,
                },
            ));
        }
        if options.quiet {
            args.push(named(
                "show",
                Expr::Ident {
                    v: "none".to_owned(),
                    span: None,
                },
            ));
        }
        args
    }

    /// Translates a program that is not in the table into `run`; the
    /// working directory comes only from `cd` in the same line.
    pub(crate) fn run_program(
        &self,
        simple: &Simple,
        words: &[Located<Word>],
        cwd: Option<String>,
    ) -> Result<Variants, Error> {
        self.run_program_with(simple, words, cwd, &RunOptions::default())
    }

    /// Like [`Session::run_program`]; the options of the program (`env`,
    /// `timeout`, …) are part of the fixes of the missing directory, as they
    /// are part of the translation that has one.
    fn run_program_with(
        &self,
        simple: &Simple,
        words: &[Located<Word>],
        cwd: Option<String>,
        options: &RunOptions,
    ) -> Result<Variants, Error> {
        self.reject_glob(words)?;
        let mut items = Vec::new();
        for word in words {
            items.push(match self.operand(word)? {
                Bound::Node(Expr::Ref(reference)) => Expr::Spread {
                    x: Box::new(Expr::Ref(reference)),
                    span: Some(word.span),
                },
                bound => self.convert(&bound, Conversion::Text)?,
            });
        }
        let command = Expr::List { items, span: None };
        let cwd = cwd.or_else(|| self.cwd.borrow().clone());
        let Some(directory) = cwd else {
            return Err(self.missing_directory(simple, command, words, options));
        };
        let stage = Stage {
            name: "run".to_owned(),
            args: vec![
                positional(command),
                named(
                    "cwd",
                    Expr::Text {
                        v: directory,
                        span: None,
                    },
                ),
            ],
            span: Some(simple.span),
        };
        self.record(
            "B38",
            CorrectionClass::Certain,
            simple.span,
            super::tree::print_stage(&stage),
            "habit",
        );
        Ok(Variants::one(vec![stage], "exec"))
    }

    fn missing_directory(
        &self,
        simple: &Simple,
        command: Expr,
        words: &[Located<Word>],
        options: &RunOptions,
    ) -> Error {
        let mut error = self.error(
            ErrorCode::Uncorrectable,
            "working_directory",
            simple.span,
            &[],
        );
        for candidate in self.directory_candidates(words) {
            let mut args = vec![
                positional(command.clone()),
                named(
                    "cwd",
                    Expr::Text {
                        v: candidate,
                        span: None,
                    },
                ),
            ];
            args.extend(self.run_args(options));
            let stage = Stage {
                name: "run".to_owned(),
                args,
                span: None,
            };
            error = error.with_fix(super::tree::print_stage(&stage));
        }
        error
    }

    /// Candidates of a working directory taken only from explicit data:
    /// the call parameter `base` and the directories of absolute paths in
    /// the command.
    pub(crate) fn directory_candidates(&self, words: &[Located<Word>]) -> Vec<String> {
        let mut candidates: Vec<String> = self
            .context
            .base
            .map(trim_trailing_slash)
            .into_iter()
            .collect();
        for word in words {
            let Some(text) = word.value.literal() else {
                continue;
            };
            if !is_absolute(&text) {
                continue;
            }
            let trimmed = trim_trailing_slash(&text);
            let parent = match trimmed.rfind('/') {
                Some(0) => "/".to_owned(),
                Some(index) => trimmed[..index].to_owned(),
                None => continue,
            };
            if !candidates.contains(&parent) {
                candidates.push(parent);
            }
        }
        candidates.truncate(MAX_ALTERNATIVES);
        candidates
    }

    /// Rejects unquoted shell patterns, which the shell expands before the
    /// command runs.
    pub(crate) fn reject_glob(&self, words: &[Located<Word>]) -> Result<(), Error> {
        match words.iter().find(|word| is_glob(&word.value)) {
            Some(word) => Err(self.pattern_error(word)),
            None => Ok(()),
        }
    }

    pub(crate) fn pattern_error(&self, word: &Located<Word>) -> Error {
        self.error(
            ErrorCode::Uncorrectable,
            "unsupported_pattern",
            word.span,
            &[("pattern", &word.value.raw)],
        )
    }

    /// Binds an operand word: literal text, a `let` reference for `$NAME`,
    /// or a named result for `$(…)`. Empty quotes around a single expansion
    /// (`"$NAME"`) do not change it.
    pub(crate) fn operand(&self, word: &Located<Word>) -> Result<Bound, Error> {
        if let Some(text) = word.value.literal() {
            return Ok(Bound::Word {
                text,
                span: word.span,
                bare: word.value.is_bare(),
            });
        }
        let parts: Vec<&WordPart> = word
            .value
            .parts
            .iter()
            .filter(|part| !matches!(part, WordPart::Literal { text, .. } if text.is_empty()))
            .collect();
        match parts.as_slice() {
            [WordPart::Variable { name, .. }] => self.variable(name, word),
            [
                WordPart::Substitution {
                    body,
                    arithmetic: false,
                    ..
                },
            ] => produce::substitution(self, body, word),
            _ => Err(self.expansion_error(word)),
        }
    }

    fn variable(&self, name: &str, word: &Located<Word>) -> Result<Bound, Error> {
        let lower = name.to_ascii_lowercase();
        if self.is_named(&lower) {
            let reference = Ref {
                name: lower.clone(),
                fields: Vec::new(),
                span: Some(word.span),
            };
            self.record(
                "B25",
                CorrectionClass::Certain,
                word.span,
                format!("@{lower}"),
                "ref",
            );
            return Ok(Bound::Node(Expr::Ref(reference)));
        }
        Err(self.unknown_variable(name, word.span, "unknown_variable"))
    }

    /// Error for a variable that is not substituted; a variable of the host
    /// (`$HOME`, `$PWD`) points to `host` or `base`.
    fn unknown_variable(&self, name: &str, span: Span, problem: &str) -> Error {
        match self.table.value_str("host_variables", name) {
            Some(hint) => self.error(
                ErrorCode::Uncorrectable,
                "host_variable",
                span,
                &[("name", name), ("hint", hint)],
            ),
            None => {
                let lower = name.to_ascii_lowercase();
                self.error(
                    ErrorCode::Uncorrectable,
                    problem,
                    span,
                    &[("name", name), ("lower", &lower)],
                )
            }
        }
    }

    fn expansion_error(&self, word: &Located<Word>) -> Error {
        let variable = word.value.parts.iter().find_map(|part| match part {
            WordPart::Variable { name, .. } => Some(name.clone()),
            _ => None,
        });
        match variable {
            Some(name) => self.unknown_variable(&name, word.span, "variable_in_text"),
            None => self.error(
                ErrorCode::Uncorrectable,
                "command_substitution",
                word.span,
                &[],
            ),
        }
    }

    /// Resolves a path word: absolute paths are kept, relative paths are
    /// joined to the directory of `cd` in the same line or to the call
    /// parameter `base`; without either they are an error.
    fn path(&self, text: &str, span: Span, bare: bool) -> Result<Expr, Error> {
        if is_absolute(text) {
            let value = trim_trailing_slash(text);
            if bare {
                self.record(
                    "L21",
                    CorrectionClass::Certain,
                    span,
                    super::tree::print_text(&value),
                    "path",
                );
            }
            return Ok(Expr::Text {
                v: value,
                span: Some(span),
            });
        }
        if bare && (text == "~" || text.starts_with("~/")) {
            return Ok(Expr::Text {
                v: trim_trailing_slash(text),
                span: Some(span),
            });
        }
        if let Some(directory) = self.cwd.borrow().as_deref() {
            return Ok(Expr::Text {
                v: join_path(directory, text),
                span: Some(span),
            });
        }
        if let Some(base) = self.context.base {
            let value = join_path(&trim_trailing_slash(base), text);
            self.record(
                "L23",
                CorrectionClass::Certain,
                span,
                super::tree::print_text(&value),
                "path",
            );
            return Ok(Expr::Text {
                v: value,
                span: Some(span),
            });
        }
        Err(self.error(
            ErrorCode::PathRelative,
            "relative_path",
            span,
            &[("path", text)],
        ))
    }

    /// Runs `f` with a working directory set for the resolution of
    /// relative operands, restoring the previous one afterwards.
    pub(crate) fn with_directory<T>(&self, directory: Option<String>, f: impl FnOnce() -> T) -> T {
        let previous = self.cwd.replace(directory);
        let result = f();
        *self.cwd.borrow_mut() = previous;
        result
    }

    /// Whether the command being translated is the head of a pipeline with
    /// further commands.
    pub(crate) fn followed(&self) -> bool {
        self.followed.get()
    }

    pub(crate) fn directory(&self) -> Option<String> {
        self.cwd.borrow().clone()
    }

    /// Records a named result for a command substitution and returns its
    /// name.
    pub(crate) fn bind_let(&self, pipeline: Pipeline) -> String {
        let mut log = self.log.borrow_mut();
        let mut name = "x".to_owned();
        let mut suffix = 1;
        while self.context.lets.contains(&name) || log.lets.iter().any(|known| known.name == name) {
            suffix += 1;
            name = format!("x{suffix}");
        }
        log.lets.push(Let {
            name: name.clone(),
            value: pipeline,
            span: None,
        });
        name
    }

    /// Records the named result of `NAME=$(CMD)` and returns its text.
    pub(crate) fn bind_named_let(&self, name: &str, pipeline: Pipeline) -> String {
        let value = Let {
            name: name.to_owned(),
            value: pipeline,
            span: None,
        };
        let text = print_program(vec![Statement::Let(value.clone())]);
        let mut log = self.log.borrow_mut();
        log.lets.push(value);
        log.named.push(name.to_owned());
        text
    }

    /// Whether `name` is a result of the call or of an assignment before.
    pub(crate) fn is_named(&self, name: &str) -> bool {
        self.context.lets.iter().any(|known| known == name)
            || self.log.borrow().named.iter().any(|known| known == name)
    }

    pub(crate) fn record(
        &self,
        rule: &str,
        class: CorrectionClass,
        span: Span,
        to: String,
        kind: &'static str,
    ) {
        let mut log = self.log.borrow_mut();
        if log
            .corrections
            .iter()
            .any(|known| known.rule == rule && known.span == span)
        {
            return;
        }
        log.corrections.push(Pending {
            rule: rule.to_owned(),
            class,
            span,
            to,
            kind,
        });
    }

    pub(crate) fn assume(&self, rule: &str, key: &str) {
        let Some(text) = self.table.assumption(key) else {
            return;
        };
        let mut log = self.log.borrow_mut();
        if !log
            .assumptions
            .iter()
            .any(|known| known.rule == rule && known.text == text)
        {
            log.assumptions.push(Assumption {
                rule: rule.to_owned(),
                text: text.to_owned(),
            });
        }
    }

    /// Text of the source between the positions of a span.
    pub(crate) fn source(&self, span: Span) -> String {
        let mut text = String::new();
        for line in span.line..=span.end_line {
            let Some(chars) = self.lines.get(line as usize - 1) else {
                break;
            };
            let start = if line == span.line {
                span.column as usize - 1
            } else {
                0
            };
            let end = if line == span.end_line {
                span.end_column as usize - 1
            } else {
                chars.len()
            };
            text.extend(chars.iter().take(end.min(chars.len())).skip(start));
            if line != span.end_line {
                text.push('\n');
            }
        }
        text
    }

    /// Builds an error from the message table.
    pub(crate) fn error(
        &self,
        code: ErrorCode,
        problem: &str,
        span: Span,
        vars: &[(&str, &str)],
    ) -> Error {
        let message = self.table.message(problem);
        let mut text = message.map_or_else(|| problem.replace('_', " "), |m| m.message.clone());
        for (name, value) in vars {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        let length = if span.line == span.end_line {
            span.end_column.saturating_sub(span.column)
        } else {
            0
        };
        let error = Error::new(code, text)
            .with_stage(ErrorStage::Parse)
            .with_at(At::new(span.line, span.column, length))
            .with_found(self.source(span))
            .with_problem(problem);
        match message.and_then(|m| m.help.as_deref()) {
            Some(help) => error.with_help(help),
            None => error,
        }
    }

    fn loop_error(&self, span: Span, keyword: &str) -> Error {
        let mut error = self.error(
            ErrorCode::Uncorrectable,
            "loop",
            span,
            &[("keyword", keyword)],
        );
        let text = self.source(span);
        if let Some(directory) = self.directory() {
            let command = Expr::List {
                items: ["bash", "-c", text.as_str()]
                    .iter()
                    .map(|v| Expr::Text {
                        v: (*v).to_owned(),
                        span: None,
                    })
                    .collect(),
                span: None,
            };
            let stage = Stage {
                name: "run".to_owned(),
                args: vec![
                    positional(command),
                    named(
                        "cwd",
                        Expr::Text {
                            v: directory,
                            span: None,
                        },
                    ),
                ],
                span: None,
            };
            error = error.with_fix(super::tree::print_stage(&stage));
        }
        error
    }

    fn alternative_branch(&self, steps: &[Step<'_>], index: usize) -> Error {
        let span = steps[index].pipe.span;
        let mut error = self.error(ErrorCode::Uncorrectable, "alternative_branch", span, &[]);
        let quiet = Session::new(self.table, self.context, "");
        let left = Session {
            lines: self.lines.clone(),
            ..quiet
        };
        if let Ok(statements) = left.statements(&steps[..index]) {
            for (variants, _) in statements {
                error = error.with_fix(left.program_text(&variants));
            }
        }
        let right = Session {
            lines: self.lines.clone(),
            ..Session::new(self.table, self.context, "")
        };
        let mut tail: Vec<Step<'_>> = steps[index..].to_vec();
        tail[0].link = Link::First;
        if let Ok(statements) = right.statements(&tail) {
            for (variants, _) in statements {
                error = error.with_next_step(right.program_text(&variants));
            }
        }
        error
    }

    /// Program text of the primary interpretation with the named results it
    /// uses.
    pub(crate) fn program_text(&self, variants: &Variants) -> String {
        let mut statements: Vec<Statement> = self
            .log
            .borrow()
            .lets
            .iter()
            .cloned()
            .map(Statement::Let)
            .collect();
        statements.push(Statement::Pipeline(pipeline(&variants.primary().stages)));
        print_program(statements)
    }

    fn finish_live(&self, statements: Vec<(Variants, Span)>) -> Result<Translation, Error> {
        if statements.len() > 1 {
            return Err(self.several_programs(&statements));
        }
        let Some((variants, _)) = statements.into_iter().next() else {
            return self.results_only();
        };
        let effect = self.has_effect(variants.primary());
        let probable = self
            .log
            .borrow()
            .corrections
            .iter()
            .any(|c| c.class == CorrectionClass::Probable);
        if probable && effect {
            return Err(self.ambiguous(&variants));
        }
        Ok(self.translation(vec![variants]))
    }

    fn finish_script(&self, statements: Vec<(Variants, Span)>) -> Result<Translation, Error> {
        if statements.is_empty() {
            return self.results_only();
        }
        Ok(self.translation(
            statements
                .into_iter()
                .map(|(variants, _)| variants)
                .collect(),
        ))
    }

    /// Translation of a line that only names results; without them the line
    /// has no program.
    fn results_only(&self) -> Result<Translation, Error> {
        if self.log.borrow().lets.is_empty() && !self.strict_only.get() {
            return Err(self.error(
                ErrorCode::Uncorrectable,
                "current_directory",
                first_span(&self.lines),
                &[],
            ));
        }
        Ok(self.translation(Vec::new()))
    }

    fn several_programs(&self, statements: &[(Variants, Span)]) -> Error {
        let effects: Vec<String> = statements
            .iter()
            .flat_map(|(variants, _)| variants.primary().stages.iter())
            .filter(|stage| self.table.has_effect(&stage.name))
            .map(|stage| stage.name.clone())
            .collect();
        let span = statements
            .iter()
            .map(|(_, span)| *span)
            .reduce(syntax::join)
            .unwrap_or_else(|| first_span(&self.lines));
        let (code, problem) = if effects.is_empty() {
            (ErrorCode::Uncorrectable, "several_programs")
        } else {
            (ErrorCode::OneEffect, "one_effect")
        };
        let names = effects.join(", ");
        let mut error = self.error(code, problem, span, &[("stages", &names)]);
        for (variants, _) in statements {
            error = error.with_next_step(self.program_text(variants));
        }
        error
    }

    fn ambiguous(&self, variants: &Variants) -> Error {
        let span = self
            .log
            .borrow()
            .corrections
            .iter()
            .find(|c| c.class == CorrectionClass::Probable)
            .map_or_else(|| first_span(&self.lines), |c| c.span);
        let mut error = self.error(ErrorCode::Ambiguous, "probable_change", span, &[]);
        for (rank, piece) in variants.0.iter().take(MAX_ALTERNATIVES).enumerate() {
            let text = self.program_text(&Variants(vec![piece.clone()]));
            error = error.with_candidate(Candidate {
                rank: u32::try_from(rank + 1).unwrap_or(u32::MAX),
                reference: None,
                program: text,
                why: piece
                    .why
                    .as_deref()
                    .and_then(|key| self.table.assumption(key))
                    .map(str::to_owned),
            });
        }
        error
    }

    /// Builds the translation: the primary program, at most three distinct
    /// alternatives and the corrections ordered by their position.
    fn translation(&self, statements: Vec<Variants>) -> Translation {
        let log = self.log.borrow();
        let lets: Vec<Statement> = log.lets.iter().cloned().map(Statement::Let).collect();
        let build = |index: usize| -> AstDocument {
            let mut all = lets.clone();
            for variants in &statements {
                let piece = variants.0.get(index).unwrap_or_else(|| variants.primary());
                all.push(Statement::Pipeline(pipeline(&piece.stages)));
            }
            document(all)
        };
        let tree = build(0);
        let mut texts = vec![print_document(&tree)];
        let mut alternatives = Vec::new();
        for index in 1..=MAX_ALTERNATIVES {
            if statements.iter().all(|variants| variants.0.len() <= index) {
                break;
            }
            let alternative = build(index);
            let text = print_document(&alternative);
            if !texts.contains(&text) {
                texts.push(text);
                alternatives.push(alternative);
            }
        }
        let alternative_texts = texts.split_off(1);
        let mut pending: Vec<&Pending> = log.corrections.iter().collect();
        pending.sort_by_key(|item| (item.span.line, item.span.column));
        let rules = pending
            .into_iter()
            .enumerate()
            .filter_map(|(index, item)| self.correction(index, item, &alternative_texts))
            .collect();
        Translation {
            tree,
            rules,
            assumptions: log.assumptions.clone(),
            alternatives,
        }
    }

    fn correction(
        &self,
        index: usize,
        pending: &Pending,
        alternatives: &[String],
    ) -> Option<Correction> {
        let span = pending.span;
        let length =
            (span.line == span.end_line).then(|| span.end_column.saturating_sub(span.column));
        let written = report::Correction {
            id: format!("C{}", index + 1),
            rule: pending.rule.clone(),
            class: pending.class,
            at: Some(TextPosition {
                line: span.line,
                column: span.column,
                length,
            }),
            from: self.source(span),
            to: pending.to.clone(),
            kind: Some(pending.kind.to_owned()),
            alternatives: if pending.class == CorrectionClass::Probable {
                alternatives.to_vec()
            } else {
                Vec::new()
            },
        };
        Correction::try_from(written).ok()
    }

    pub(crate) fn has_effect(&self, piece: &Piece) -> bool {
        piece
            .stages
            .iter()
            .any(|stage| self.table.has_effect(&stage.name))
    }

    pub(crate) fn redirect_kind(op: RedirectOp) -> Option<&'static str> {
        match op {
            RedirectOp::Out | RedirectOp::Clobber | RedirectOp::OutErr => Some("overwrite"),
            RedirectOp::Append | RedirectOp::AppendOutErr => Some("append"),
            _ => None,
        }
    }
}

impl Convert for Session<'_> {
    type Error = Error;

    fn convert(&self, bound: &Bound, conversion: Conversion) -> Result<Expr, Error> {
        let (text, span, bare) = match bound {
            Bound::Node(expr) => return Ok(expr.clone()),
            Bound::Word { text, span, bare } => (text, *span, *bare),
        };
        match conversion {
            Conversion::Path => self.path(text, span, bare),
            Conversion::Text => Ok(Expr::Text {
                v: text.clone(),
                span: Some(span),
            }),
            // Digits only: `str::parse` also takes a plus sign, and the `+5` of
            // `tail -n +5` means "from line 5", not the last 5 lines.
            Conversion::Int => Some(text)
                .filter(|text| text.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|text| text.parse::<i64>().ok())
                .filter(|v| *v >= 0)
                .map(|v| Expr::Int {
                    v,
                    span: Some(span),
                })
                .ok_or_else(|| {
                    self.error(
                        ErrorCode::Uncorrectable,
                        "invalid_number",
                        span,
                        &[("value", text)],
                    )
                }),
            Conversion::Ident => {
                let lower = text.to_ascii_lowercase();
                if super::tree::is_identifier(&lower) {
                    Ok(Expr::Ident {
                        v: lower,
                        span: Some(span),
                    })
                } else {
                    Err(self.error(
                        ErrorCode::Uncorrectable,
                        "invalid_value",
                        span,
                        &[("value", text)],
                    ))
                }
            }
            Conversion::Duration => parse_duration(text)
                .map(|v| Expr::Dur {
                    v,
                    span: Some(span),
                })
                .ok_or_else(|| {
                    self.error(
                        ErrorCode::Uncorrectable,
                        "invalid_duration",
                        span,
                        &[("value", text)],
                    )
                }),
        }
    }

    fn missing(&self, slot: &str) -> Error {
        self.error(
            ErrorCode::Uncorrectable,
            "missing_operand",
            first_span(&self.lines),
            &[("slot", slot)],
        )
    }
}

fn first_span(lines: &[Vec<char>]) -> Span {
    let width = lines.first().map_or(0, Vec::len);
    Span {
        line: 1,
        column: 1,
        end_line: 1,
        end_column: u32::try_from(width + 1).unwrap_or(u32::MAX),
    }
}

/// Milliseconds of a shell duration: a number of seconds with an optional
/// fraction and an optional suffix `s`, `m`, `h` or `d`.
pub(crate) fn parse_duration(text: &str) -> Option<u64> {
    const UNITS: &[(char, f64)] = &[
        ('s', 1_000.0),
        ('m', 60_000.0),
        ('h', 3_600_000.0),
        ('d', 86_400_000.0),
    ];
    let (number, factor) = match text.chars().last() {
        Some(c) if c.is_ascii_alphabetic() => {
            let factor = UNITS.iter().find(|(unit, _)| *unit == c)?.1;
            (&text[..text.len() - 1], factor)
        }
        _ => (text, 1_000.0),
    };
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let value: f64 = number.parse().ok()?;
    let millis = (value * factor).round();
    if !(millis.is_finite() && (0.0..MAX_DURATION_MILLIS).contains(&millis)) {
        return None;
    }
    format!("{millis:.0}").parse().ok()
}

/// Upper bound of a translated duration, far above any practical value.
const MAX_DURATION_MILLIS: f64 = 1e15;

fn compound<'s>(steps: &[Step<'s>]) -> Option<(Span, &'s str)> {
    steps.iter().find_map(|step| {
        step.pipe.commands.iter().find_map(|command| match command {
            Command::Compound { span, keyword } => Some((*span, keyword.as_str())),
            Command::Simple(_) => None,
        })
    })
}

fn single_simple(pipe: &Pipe) -> Option<&Simple> {
    match pipe.commands.as_slice() {
        [Command::Simple(simple)] if !pipe.negated => Some(simple),
        _ => None,
    }
}

fn is_lookup(words: &[Located<Word>]) -> bool {
    words
        .first()
        .is_some_and(|word| word.value.raw == "command")
        && words.get(1).is_some_and(|word| word.value.raw == "-v")
}

fn join_steps(a: Step<'_>, b: Step<'_>) -> Span {
    syntax::join(a.pipe.span, b.pipe.span)
}

/// Whether the word is an unquoted shell pattern without expansions.
pub(crate) fn is_glob(word: &Word) -> bool {
    word.has_unquoted_glob() && !word.has_expansion()
}

pub(crate) fn is_absolute(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.starts_with('/')
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}

pub(crate) fn trim_trailing_slash(text: &str) -> String {
    let trimmed = text.trim_end_matches('/');
    if trimmed.is_empty() && text.starts_with('/') {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Joins a relative path to an absolute directory without normalising
/// `..`; `./` prefixes are dropped.
pub(crate) fn join_path(directory: &str, relative: &str) -> String {
    let mut rest = relative;
    while let Some(stripped) = rest.strip_prefix("./") {
        rest = stripped;
    }
    let rest = trim_trailing_slash(rest);
    if rest.is_empty() || rest == "." {
        return directory.to_owned();
    }
    if directory.ends_with('/') {
        format!("{directory}{rest}")
    } else {
        format!("{directory}/{rest}")
    }
}

pub(crate) fn pipeline(stages: &[Stage]) -> Pipeline {
    let mut stages = stages.to_vec();
    let head = if stages.is_empty() {
        Stage {
            name: String::new(),
            args: Vec::new(),
            span: None,
        }
    } else {
        stages.remove(0)
    };
    let span = head.span;
    Pipeline {
        head: Head::Stage(head),
        stages,
        span,
    }
}

pub(crate) fn piece_text(piece: &Piece) -> String {
    print_pipeline(&pipeline(&piece.stages))
}

pub(crate) fn named(name: &str, value: Expr) -> Arg {
    Arg {
        name: Some(name.to_owned()),
        value,
        span: None,
    }
}

pub(crate) fn positional(value: Expr) -> Arg {
    let span = super::template::expr_span(&value);
    Arg {
        name: None,
        value,
        span,
    }
}

/// Appends arguments to a stage; a named argument already present is kept.
pub(crate) fn append_args(stage: &mut Stage, args: Vec<Arg>) {
    for arg in args {
        let duplicate =
            arg.name.is_some() && stage.args.iter().any(|existing| existing.name == arg.name);
        if !duplicate {
            stage.args.push(arg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_accept_fractions_and_units() {
        assert_eq!(parse_duration("300"), Some(300_000));
        assert_eq!(parse_duration("1.5m"), Some(90_000));
        assert_eq!(parse_duration("2h"), Some(7_200_000));
        assert_eq!(parse_duration("1d"), Some(86_400_000));
        assert_eq!(parse_duration("5x"), None);
        assert_eq!(parse_duration("s"), None);
        assert_eq!(parse_duration("-1"), None);
    }

    #[test]
    fn durations_reject_malformed_and_huge_values() {
        assert_eq!(parse_duration("0"), Some(0));
        assert_eq!(parse_duration("0.0004"), Some(0));
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("5ą"), None);
        assert_eq!(parse_duration("1e3"), None);
        assert_eq!(parse_duration("1.5.2"), None);
        assert_eq!(parse_duration("99999999999999999999"), None);
    }

    fn text(v: &str) -> Expr {
        Expr::Text {
            v: v.to_owned(),
            span: None,
        }
    }

    #[test]
    fn named_arguments_are_not_duplicated() {
        let mut stage = Stage {
            name: "run".to_owned(),
            args: vec![named("cwd", text("/a"))],
            span: None,
        };
        append_args(
            &mut stage,
            vec![
                named("cwd", text("/b")),
                named("env", text("x")),
                positional(text("y")),
            ],
        );
        let names: Vec<Option<&str>> = stage.args.iter().map(|arg| arg.name.as_deref()).collect();
        assert_eq!(names, [Some("cwd"), Some("env"), None]);
        assert_eq!(stage.args[0].value, text("/a"));
    }

    #[test]
    fn an_empty_pipeline_has_an_unnamed_head() {
        let built = pipeline(&[]);
        assert!(matches!(&built.head, Head::Stage(stage) if stage.name.is_empty()));
        assert!(built.stages.is_empty());
    }

    #[test]
    fn absolute_paths_accept_both_separators() {
        assert!(is_absolute("C:\\x"));
        assert!(!is_absolute("C:"));
        assert!(!is_absolute(""));
        assert_eq!(trim_trailing_slash("//"), "/");
        assert_eq!(join_path("/srv/zażółć", "./ą/"), "/srv/zażółć/ą");
    }

    #[test]
    fn paths_are_joined_without_normalisation() {
        assert_eq!(join_path("/srv/app", "./src/"), "/srv/app/src");
        assert_eq!(join_path("/srv/app", "."), "/srv/app");
        assert_eq!(join_path("/", "etc"), "/etc");
        assert_eq!(join_path("/srv/app", "../x"), "/srv/app/../x");
        assert_eq!(trim_trailing_slash("/"), "/");
        assert_eq!(trim_trailing_slash("/srv//"), "/srv");
        assert!(is_absolute("C:/x"));
        assert!(is_absolute("c:\\x"));
        assert!(!is_absolute("x/y"));
    }

    #[test]
    fn alternatives_join_with_the_primary_of_the_other_side() {
        let piece = |name: &str| Piece {
            stages: vec![Stage {
                name: name.to_owned(),
                args: Vec::new(),
                span: None,
            }],
            kind: "line".to_owned(),
            why: None,
        };
        let left = Variants(vec![piece("a"), piece("b")]);
        let right = Variants(vec![piece("c"), piece("d")]);
        let joined = left.then(right);
        let names: Vec<String> = joined
            .0
            .iter()
            .map(|piece| {
                piece
                    .stages
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        assert_eq!(names, ["a c", "b c", "a d"]);
    }
}
