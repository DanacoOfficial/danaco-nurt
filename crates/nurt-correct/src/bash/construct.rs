// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::bash::construct
// Responsibility:   Translation of shell conditions, loops, functions and lists with changes into the constructs of the language.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Translation of shell conditions, loops, functions and lists with
//! changes into the constructs of the language (`when`, `each`, `while`,
//! `repeat`, `steps`, `define`, `params`).
//!
//! The simple commands inside a construct are translated by the table of
//! simple commands in script mode; this module only builds the construct
//! around them and prints it as program text of syntax variant A.

use nurt_contracts::ast::{Head, Span, Statement};
use nurt_contracts::error::{Candidate, Error, ErrorCode};

use crate::record::Correction;

use super::compound::{self, Block, CaseArm, Entry, Node, Simple};
use super::lex::{Word, WordPart, lex};
use super::params::{ParamKind, ScriptParam, rewrite_positionals};
use super::session::{self, Session};
use super::syntax::Connector;
use super::table::BashTable;
use super::tree::{print_program, print_text};
use super::{Assumption, Mode, TranslateContext, Translation};

/// Rule of conditions, loops, functions and lists with changes.
const CONSTRUCT_RULE: &str = "B41";

/// Rule of script and function parameters.
const PARAMS_RULE: &str = "B42";

const INDENT: &str = "    ";

/// The command that waits; the end of the body of an `until` loop.
const WAIT_COMMAND: &str = "sle\u{65}p";

const NO_SPAN: Span = Span {
    line: 1,
    column: 1,
    end_line: 1,
    end_column: 1,
};

/// Translated lines and the number of statements that change something.
#[derive(Debug, Default)]
struct Unit {
    lines: Vec<String>,
    changes: usize,
}

impl Unit {
    fn single(line: String, changes: usize) -> Self {
        Self {
            lines: vec![line],
            changes,
        }
    }

    fn append(&mut self, other: Unit) {
        self.lines.extend(other.lines);
        self.changes += other.changes;
    }
}

/// A condition with the statements that must run before it.
struct Cond {
    prelude: Vec<String>,
    expr: String,
}

/// A function defined earlier in the script.
#[derive(Clone)]
struct Function {
    name: String,
    params: Vec<ScriptParam>,
}

struct Constructs<'t> {
    table: &'t BashTable,
    context: &'t TranslateContext<'t>,
    names: Vec<String>,
    functions: Vec<Function>,
    rules: Vec<Correction>,
    assumptions: Vec<Assumption>,
    params: Vec<ScriptParam>,
    /// Loop variables bound to records of paths.
    records: Vec<String>,
    counter: usize,
    probable: bool,
    effects: bool,
}

/// Translates a script that contains compound commands.
pub(crate) fn translate(
    input: &str,
    context: &TranslateContext<'_>,
    table: &BashTable,
) -> Result<Translation, Error> {
    let rewritten = rewrite_positionals(input);
    let session = Session::new(table, context, &rewritten.text);
    let tokens = lex(&rewritten.text).map_err(|error| {
        session.error(
            ErrorCode::Uncorrectable,
            "unsupported_syntax",
            NO_SPAN,
            &[("reason", &error.to_string())],
        )
    })?;
    let block = compound::parse(&tokens).map_err(|error| {
        session.error(
            ErrorCode::Uncorrectable,
            "unsupported_syntax",
            error.span,
            &[("reason", error.message)],
        )
    })?;
    let mut names: Vec<String> = context.lets.to_vec();
    names.extend(rewritten.params.iter().map(|p| p.name.clone()));
    let mut constructs = Constructs {
        table,
        context,
        names,
        functions: context
            .functions
            .iter()
            .map(|signature| Function {
                name: signature.name.clone(),
                params: vec![ScriptParam::placeholder(); signature.params],
            })
            .collect(),
        rules: Vec::new(),
        assumptions: Vec::new(),
        params: rewritten.params,
        records: Vec::new(),
        counter: 0,
        probable: false,
        effects: false,
    };
    let unit = constructs.block(&session, &block)?;
    let mut lines = Vec::new();
    if !constructs.params.is_empty() {
        lines.push("params {".to_owned());
        lines.extend(
            constructs
                .params
                .iter()
                .map(|param| format!("{INDENT}{}", param.declaration())),
        );
        lines.push("}".to_owned());
        let notes: Vec<String> = constructs
            .params
            .iter()
            .filter_map(|param| param.note.clone())
            .collect();
        for text in notes {
            constructs.assumptions.push(Assumption {
                rule: PARAMS_RULE.to_owned(),
                text,
            });
        }
    }
    lines.extend(unit.lines);
    let text = lines.join("\n");
    if constructs.probable && constructs.effects && context.mode == Mode::Live {
        let error = session.error(ErrorCode::Ambiguous, "probable_change", NO_SPAN, &[]);
        return Err(error.with_candidate(Candidate {
            rank: 1,
            reference: None,
            program: text,
            why: None,
        }));
    }
    session.record(
        CONSTRUCT_RULE,
        nurt_contracts::report::CorrectionClass::Certain,
        first_span(&block),
        text.lines().next().unwrap_or_default().to_owned(),
        "habit",
    );
    session.construct_translation(text, constructs.rules, constructs.assumptions)
}

fn first_span(block: &Block) -> Span {
    block.entries.first().map_or(NO_SPAN, Entry::span)
}

fn indent(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| {
            if line.is_empty() {
                line
            } else {
                format!("{INDENT}{line}")
            }
        })
        .collect()
}

fn word_text(word: &Word) -> Option<String> {
    word.literal()
}

fn first_word(simple: &Simple) -> String {
    simple.words.first().and_then(word_text).unwrap_or_default()
}

impl Constructs<'_> {
    fn child_context<'c>(&'c self, names: &'c [String]) -> TranslateContext<'c> {
        TranslateContext {
            base: self.context.base,
            lets: names,
            functions: &[],
            mode: Mode::Script,
        }
    }

    fn fail(&self, session: &Session<'_>, span: Span, reason: &str) -> Error {
        session.error(
            ErrorCode::Uncorrectable,
            "unsupported_construct",
            span,
            &[("reason", reason)],
        )
    }

    fn fresh(&mut self, stem: &str) -> String {
        self.counter += 1;
        format!("{stem}{}", self.counter)
    }

    // ----- lists ---------------------------------------------------------

    fn block(&mut self, session: &Session<'_>, block: &Block) -> Result<Unit, Error> {
        let mut unit = Unit::default();
        let mut index = 0;
        while index < block.entries.len() {
            let run = self.plain_run(block, index);
            if run > 0 {
                let first = block.entries[index].span();
                let last = block.entries[index + run - 1].span();
                let span = Span {
                    line: first.line,
                    column: first.column,
                    end_line: last.end_line,
                    end_column: last.end_column,
                };
                unit.append(self.simple_text(session, span)?);
                index += run;
                continue;
            }
            unit.append(self.entry(session, &block.entries[index])?);
            index += 1;
        }
        Ok(unit)
    }

    /// Number of entries from `start` that are plain commands translated
    /// together, so that `cd` and `export` reach the commands after them.
    fn plain_run(&self, block: &Block, start: usize) -> usize {
        block.entries[start..]
            .iter()
            .take_while(|entry| self.is_plain(entry))
            .count()
    }

    fn is_plain(&self, entry: &Entry) -> bool {
        !entry.background && entry.rest.is_empty() && self.is_plain_node(&entry.first)
    }

    fn is_plain_node(&self, node: &Node) -> bool {
        let Node::Simple(simple) = node else {
            return false;
        };
        if simple.negated || simple.double_bracket || simple.words.is_empty() {
            return false;
        }
        let first = first_word(simple);
        !(matches!(
            first.as_str(),
            "[" | "test"
                | "break"
                | "continue"
                | "return"
                | "exit"
                | "shift"
                | "true"
                | "false"
                | ":"
        ) || self.functions.iter().any(|f| f.name == first))
    }

    fn entry(&mut self, session: &Session<'_>, entry: &Entry) -> Result<Unit, Error> {
        if entry.background {
            return Err(self.fail(session, entry.span(), "a command in the background"));
        }
        if entry.rest.is_empty() {
            return self.node(session, &entry.first);
        }
        let nodes: Vec<&Node> = std::iter::once(&entry.first)
            .chain(entry.rest.iter().map(|(_, node)| node))
            .collect();
        if entry.rest.iter().all(|(c, _)| *c == Connector::And) {
            return self.and_list(session, &nodes, entry.span());
        }
        if entry.rest.len() == 1 {
            return self.or_pair(session, nodes[0], nodes[1], entry.span());
        }
        Err(self.fail(
            session,
            entry.span(),
            "`&&` and `||` mixed in one list; write the branches with `if`",
        ))
    }

    fn is_test(node: &Node) -> bool {
        match node {
            Node::Simple(simple) => {
                simple.double_bracket
                    || simple.negated
                    || matches!(first_word(simple).as_str(), "[" | "test")
            }
            _ => false,
        }
    }

    /// `A && B && C`: a test first makes a `when`, commands with several
    /// changes make `steps`.
    fn and_list(
        &mut self,
        session: &Session<'_>,
        nodes: &[&Node],
        span: Span,
    ) -> Result<Unit, Error> {
        if nodes.iter().all(|node| self.is_plain_node(node)) {
            let mut unit = self.simple_text(session, span)?;
            if unit.changes >= 2 {
                unit = Self::steps(unit);
            }
            return Ok(unit);
        }
        if Self::is_test(nodes[0]) {
            let cond = self.node_condition(session, nodes[0], false, span)?;
            let mut body = Unit::default();
            for node in &nodes[1..] {
                body.append(self.node(session, node)?);
            }
            return Ok(Self::when(&cond.expr, body));
        }
        let mut body = Unit::default();
        for node in nodes {
            body.append(self.node(session, node)?);
        }
        if body.changes >= 2 {
            body = Self::steps(body);
        }
        Ok(body)
    }

    fn steps(body: Unit) -> Unit {
        let mut lines = vec!["steps {".to_owned()];
        lines.extend(indent(body.lines));
        lines.push("}".to_owned());
        Unit {
            lines,
            changes: body.changes,
        }
    }

    fn when(expr: &str, body: Unit) -> Unit {
        let mut lines = vec![format!("when {expr} {{")];
        lines.extend(indent(body.lines));
        lines.push("}".to_owned());
        Unit {
            lines,
            changes: body.changes,
        }
    }

    /// `A || B`: `B` runs when `A` fails.
    fn or_pair(
        &mut self,
        session: &Session<'_>,
        left: &Node,
        right: &Node,
        span: Span,
    ) -> Result<Unit, Error> {
        if let Node::Simple(simple) = right
            && matches!(first_word(simple).as_str(), "true" | ":")
            && simple.words.len() == 1
        {
            self.probable = true;
            self.assumptions.push(Assumption {
                    rule: CONSTRUCT_RULE.to_owned(),
                    text: "a failure of the command before `|| true` is ignored by the shell; the program stops at it".to_owned(),
                });
            return self.node(session, left);
        }
        let cond = self.node_condition(session, left, true, span)?;
        let negated = negate(&cond.expr);
        let body = self.node(session, right)?;
        let mut unit = Unit {
            lines: cond.prelude,
            changes: 0,
        };
        unit.append(Self::when(&negated, body));
        Ok(unit)
    }

    // ----- nodes ----------------------------------------------------------

    fn node(&mut self, session: &Session<'_>, node: &Node) -> Result<Unit, Error> {
        match node {
            Node::Simple(simple) => self.simple(session, simple),
            Node::If { arms, other, span } => self.if_command(session, arms, other.as_ref(), *span),
            Node::For {
                name,
                items,
                body,
                span,
            } => self.for_command(session, name, items.as_deref(), body, *span),
            Node::While {
                until,
                condition,
                body,
                span,
            } => self.while_command(session, *until, condition, body, *span),
            Node::Case {
                subject,
                arms,
                span,
            } => self.case_command(session, subject, arms, *span),
            Node::Function { name, span, .. } => self.function(session, name, *span),
            Node::Group { body, .. } => self.block(session, body),
            Node::Unsupported { keyword, span } => Err(session.loop_error(*span, keyword)),
        }
    }

    fn simple(&mut self, session: &Session<'_>, simple: &Simple) -> Result<Unit, Error> {
        let first = first_word(simple);
        match first.as_str() {
            "break" | "continue" if simple.words.len() == 1 => {
                return Ok(Unit::single(first, 0));
            }
            "shift" | "true" | ":" => return Ok(Unit::default()),
            "return" | "exit" => {
                return Err(self.fail(
                    session,
                    simple.span,
                    "`return` and `exit` end the shell; a program ends at its last statement",
                ));
            }
            _ => {}
        }
        if Self::is_test(&Node::Simple(simple.clone())) {
            return Err(self.fail(session, simple.span, "a test outside a condition"));
        }
        if let Some(function) = self.functions.iter().find(|f| f.name == first).cloned() {
            return self.call(session, simple, &function);
        }
        self.simple_text(session, simple.span)
    }

    fn call(
        &mut self,
        session: &Session<'_>,
        simple: &Simple,
        function: &Function,
    ) -> Result<Unit, Error> {
        let arguments = &simple.words[1..];
        if arguments.len() > function.params.len() {
            return Err(self.fail(
                session,
                simple.span,
                "more arguments than the function takes",
            ));
        }
        let mut values = Vec::new();
        for word in arguments {
            values.push(self.value(session, word, simple.span, true)?);
        }
        Ok(Unit::single(
            format!("{}({})", function.name, values.join(", ")),
            1,
        ))
    }

    /// Translates the commands of a span with the table of simple commands.
    fn simple_text(&mut self, session: &Session<'_>, span: Span) -> Result<Unit, Error> {
        let text = session.source(span);
        let names = self.names.clone();
        let context = self.child_context(&names);
        let translation = match session::translate_plain(&text, &context, self.table) {
            Some(Ok(translation)) => translation,
            Some(Err(error)) => return Err(error),
            None => return Err(self.fail(session, span, "the command has no translation")),
        };
        let mut unit = Unit::default();
        for statement in &translation.tree.program.statements {
            let printed = self.path_fields(&print_program(vec![statement.clone()]));
            if let Statement::Let(binding) = statement {
                self.names.push(binding.name.clone());
            }
            unit.changes += usize::from(self.statement_changes(statement));
            unit.lines.extend(printed.split('\n').map(str::to_owned));
        }
        let offset = span.line.saturating_sub(1);
        for mut rule in translation.rules {
            if let Some(at) = &mut rule.at {
                at.line = at.line.saturating_add(offset);
            }
            if rule.class == crate::policy::Class::Probable {
                self.probable = true;
            }
            self.rules.push(rule);
        }
        self.assumptions.extend(translation.assumptions);
        self.effects |= unit.changes > 0;
        Ok(unit)
    }

    /// A reference to a record of a loop is the path of the record where a
    /// path is expected.
    fn path_fields(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for name in &self.records {
            let needle = format!("@{name}");
            let mut result = String::new();
            let mut rest = out.as_str();
            while let Some(index) = rest.find(&needle) {
                let after = &rest[index + needle.len()..];
                let continues = after
                    .chars()
                    .next()
                    .is_some_and(|c| c == '.' || c == '_' || c.is_ascii_alphanumeric());
                result.push_str(&rest[..index + needle.len()]);
                if !continues {
                    result.push_str(".path");
                }
                rest = after;
            }
            result.push_str(rest);
            out = result;
        }
        out
    }

    fn statement_changes(&self, statement: &Statement) -> bool {
        let Statement::Pipeline(pipeline) = statement else {
            return false;
        };
        let head = match &pipeline.head {
            Head::Stage(stage) => self.table.has_effect(&stage.name),
            _ => false,
        };
        head || pipeline
            .stages
            .iter()
            .any(|stage| self.table.has_effect(&stage.name))
    }

    // ----- conditions ------------------------------------------------------

    /// Condition of an `if` or `while`: tests, commands that run a program,
    /// and their combinations.
    fn condition(
        &mut self,
        session: &Session<'_>,
        block: &Block,
        allow_prelude: bool,
    ) -> Result<Cond, Error> {
        let [entry] = block.entries.as_slice() else {
            return Err(self.fail(
                session,
                first_span(block),
                "the condition is not one command or test",
            ));
        };
        if entry.background {
            return Err(self.fail(session, entry.span(), "a condition in the background"));
        }
        let single = entry.rest.is_empty();
        let mut cond =
            self.node_condition(session, &entry.first, allow_prelude && single, entry.span())?;
        for (connector, node) in &entry.rest {
            let next = self.node_condition(session, node, false, entry.span())?;
            let word = if *connector == Connector::And {
                "and"
            } else {
                "or"
            };
            cond.expr = format!(
                "{} {word} {}",
                group(&cond.expr, word),
                group(&next.expr, word)
            );
        }
        Ok(cond)
    }

    fn node_condition(
        &mut self,
        session: &Session<'_>,
        node: &Node,
        allow_prelude: bool,
        span: Span,
    ) -> Result<Cond, Error> {
        let Node::Simple(simple) = node else {
            return Err(self.fail(session, node.span(), "a compound command as a condition"));
        };
        let first = first_word(simple);
        let expr = if simple.double_bracket {
            self.test_expression(session, &simple.words, span)?
        } else {
            match first.as_str() {
                "[" | "test" => {
                    let mut words: &[Word] = &simple.words[1..];
                    if first == "[" {
                        match words.last().and_then(word_text).as_deref() {
                            Some("]") => words = &words[..words.len() - 1],
                            _ => return Err(self.fail(session, span, "`[` without `]`")),
                        }
                    }
                    self.test_expression(session, words, span)?
                }
                "true" | ":" => "true".to_owned(),
                "false" => "false".to_owned(),
                _ => return self.command_condition(session, simple, allow_prelude),
            }
        };
        let expr = if simple.negated { negate(&expr) } else { expr };
        Ok(Cond {
            prelude: Vec::new(),
            expr,
        })
    }

    /// A command as a condition: its exit code, from a `run` bound by `let`.
    fn command_condition(
        &mut self,
        session: &Session<'_>,
        simple: &Simple,
        allow_prelude: bool,
    ) -> Result<Cond, Error> {
        if !allow_prelude {
            return Err(self.fail(
                session,
                simple.span,
                "a command can be a condition only alone in `if`, `elif` or `||`",
            ));
        }
        let unit = self.simple_text(session, simple.span)?;
        let [line] = unit.lines.as_slice() else {
            return Err(self.fail(session, simple.span, "the condition is not one program"));
        };
        if !line.starts_with("run [") {
            return Err(self.fail(
                session,
                simple.span,
                "the exit code of this command is not available; compare a reading pipeline instead",
            ));
        }
        let name = self.fresh("r");
        let op = if simple.negated { "!=" } else { "==" };
        Ok(Cond {
            prelude: vec![format!("let {name} = {line}")],
            expr: format!("{name}.exit_code {op} 0"),
        })
    }

    fn test_expression(
        &self,
        session: &Session<'_>,
        words: &[Word],
        span: Span,
    ) -> Result<String, Error> {
        let mut at = 0;
        let expr = self.test_or(session, words, &mut at, span)?;
        if at != words.len() {
            return Err(self.fail(session, span, "unexpected word in the test"));
        }
        Ok(expr)
    }

    fn test_or(
        &self,
        session: &Session<'_>,
        words: &[Word],
        at: &mut usize,
        span: Span,
    ) -> Result<String, Error> {
        let mut expr = self.test_and(session, words, at, span)?;
        while matches!(
            words.get(*at).and_then(word_text).as_deref(),
            Some("-o" | "||")
        ) {
            *at += 1;
            let next = self.test_and(session, words, at, span)?;
            expr = format!("{} or {}", group(&expr, "or"), group(&next, "or"));
        }
        Ok(expr)
    }

    fn test_and(
        &self,
        session: &Session<'_>,
        words: &[Word],
        at: &mut usize,
        span: Span,
    ) -> Result<String, Error> {
        let mut expr = self.test_not(session, words, at, span)?;
        while matches!(
            words.get(*at).and_then(word_text).as_deref(),
            Some("-a" | "&&")
        ) {
            *at += 1;
            let next = self.test_not(session, words, at, span)?;
            expr = format!("{} and {}", group(&expr, "and"), group(&next, "and"));
        }
        Ok(expr)
    }

    fn test_not(
        &self,
        session: &Session<'_>,
        words: &[Word],
        at: &mut usize,
        span: Span,
    ) -> Result<String, Error> {
        if words.get(*at).and_then(word_text).as_deref() == Some("!") {
            *at += 1;
            let inner = self.test_not(session, words, at, span)?;
            return Ok(negate(&inner));
        }
        self.test_primary(session, words, at, span)
    }

    fn test_primary(
        &self,
        session: &Session<'_>,
        words: &[Word],
        at: &mut usize,
        span: Span,
    ) -> Result<String, Error> {
        let Some(word) = words.get(*at) else {
            return Err(self.fail(session, span, "the test ends early"));
        };
        let text = word_text(word).unwrap_or_default();
        if text == "(" || text == "\\(" {
            *at += 1;
            let inner = self.test_or(session, words, at, span)?;
            if !matches!(
                words.get(*at).and_then(word_text).as_deref(),
                Some(")" | "\\)")
            ) {
                return Err(self.fail(session, span, "unbalanced parentheses in the test"));
            }
            *at += 1;
            return Ok(format!("({inner})"));
        }
        let next_text = words.get(*at + 1).and_then(word_text);
        if let Some(operator) = next_text.as_deref().filter(|op| is_binary(op)) {
            let right = words
                .get(*at + 2)
                .ok_or_else(|| self.fail(session, span, "the test ends early"))?;
            let numeric = operator.starts_with('-');
            let left = self.value(session, word, span, numeric)?;
            let result = self.binary(session, operator, &left, right, span, numeric)?;
            *at += 3;
            return Ok(result);
        }
        if is_unary(&text) && words.get(*at + 1).is_some() {
            let value = self.value(session, &words[*at + 1], span, false)?;
            *at += 2;
            return self.unary(session, &text, &value, span);
        }
        let value = self.value(session, word, span, false)?;
        *at += 1;
        Ok(format!("{value} != \"\""))
    }

    fn binary(
        &self,
        session: &Session<'_>,
        operator: &str,
        left: &str,
        right: &Word,
        span: Span,
        numeric: bool,
    ) -> Result<String, Error> {
        let symbol = match operator {
            "=" | "==" | "-eq" => "==",
            "!=" | "-ne" => "!=",
            "-gt" => ">",
            "-ge" => ">=",
            "-lt" => "<",
            "-le" => "<=",
            "=~" => {
                let pattern = word_text(right)
                    .ok_or_else(|| self.fail(session, span, "a pattern with expansions"))?;
                return Ok(format!("{left} ~ {}", regex_literal(&pattern)));
            }
            _ => {
                return Err(self.fail(session, span, "string ordering has no translation"));
            }
        };
        if matches!(operator, "=" | "==" | "!=")
            && let Some(text) = word_text(right).filter(|t| t.contains(['*', '?', '[']))
        {
            let matched = format!("{left} ~ {}", regex_literal(&glob_regex(&text)));
            return Ok(if operator == "!=" {
                negate(&matched)
            } else {
                matched
            });
        }
        let right = self.value(session, right, span, numeric)?;
        Ok(format!("{left} {symbol} {right}"))
    }

    fn unary(
        &self,
        session: &Session<'_>,
        operator: &str,
        value: &str,
        span: Span,
    ) -> Result<String, Error> {
        let typed =
            |kind: &str| format!("exists {value} and (info {value} |> first 1).type == \"{kind}\"");
        Ok(match operator {
            "-e" | "-a" => format!("exists {value}"),
            "-f" => typed("file"),
            "-d" => typed("directory"),
            "-L" | "-h" => typed("link"),
            "-s" => format!("exists {value} and (info {value} |> first 1).size > 0"),
            "-z" => format!("{value} == \"\""),
            "-n" => format!("{value} != \"\""),
            _ => {
                return Err(self.fail(
                    session,
                    span,
                    "this file test has no translation; read the attributes with `info`",
                ));
            }
        })
    }

    /// Value of a word as an expression: text, a number, a variable or a sum
    /// of them.
    fn value(
        &self,
        session: &Session<'_>,
        word: &Word,
        span: Span,
        numeric: bool,
    ) -> Result<String, Error> {
        let mut pieces = Vec::new();
        for part in &word.parts {
            match part {
                WordPart::Literal { text, .. } => {
                    if text.is_empty() {
                        continue;
                    }
                    let whole = word.parts.len() == 1;
                    if numeric && whole && text.chars().all(|c| c.is_ascii_digit()) {
                        pieces.push(text.clone());
                    } else {
                        pieces.push(print_text(text));
                    }
                }
                WordPart::Variable { name, .. } => {
                    let lower = name.to_ascii_lowercase();
                    if !self.names.contains(&lower) {
                        return Err(session.error(
                            ErrorCode::Uncorrectable,
                            "unknown_variable",
                            span,
                            &[("name", name.as_str()), ("lower", &lower)],
                        ));
                    }
                    pieces.push(format!("@{lower}"));
                }
                WordPart::Substitution { .. } => {
                    return Err(self.fail(session, span, "command substitution in a value"));
                }
            }
        }
        if pieces.is_empty() {
            return Ok("\"\"".to_owned());
        }
        Ok(pieces.join(" + "))
    }

    // ----- if, case ----------------------------------------------------------

    fn if_command(
        &mut self,
        session: &Session<'_>,
        arms: &[(Block, Block)],
        other: Option<&Block>,
        _span: Span,
    ) -> Result<Unit, Error> {
        let mut unit = Unit::default();
        let (first_condition, first_body) = &arms[0];
        let cond = self.condition(session, first_condition, true)?;
        unit.lines.extend(cond.prelude);
        let body = self.block(session, first_body)?;
        unit.lines.push(format!("when {} {{", cond.expr));
        unit.changes += body.changes;
        unit.lines.extend(indent(body.lines));
        self.if_tail(session, &mut unit, &arms[1..], other)?;
        Ok(unit)
    }

    /// Closes the block of a `when` and adds the `otherwise` parts.
    fn if_tail(
        &mut self,
        session: &Session<'_>,
        unit: &mut Unit,
        arms: &[(Block, Block)],
        other: Option<&Block>,
    ) -> Result<(), Error> {
        let Some(((condition, body), rest)) = arms.split_first() else {
            if let Some(block) = other {
                let body = self.block(session, block)?;
                unit.lines.push("} otherwise {".to_owned());
                unit.changes += body.changes;
                unit.lines.extend(indent(body.lines));
            }
            unit.lines.push("}".to_owned());
            return Ok(());
        };
        let cond = self.condition(session, condition, true)?;
        if cond.prelude.is_empty() {
            let body = self.block(session, body)?;
            unit.lines
                .push(format!("}} otherwise when {} {{", cond.expr));
            unit.changes += body.changes;
            unit.lines.extend(indent(body.lines));
            return self.if_tail(session, unit, rest, other);
        }
        let mut nested = Unit::default();
        nested.lines.extend(cond.prelude);
        let body = self.block(session, body)?;
        nested.lines.push(format!("when {} {{", cond.expr));
        nested.changes += body.changes;
        nested.lines.extend(indent(body.lines));
        self.if_tail(session, &mut nested, rest, other)?;
        unit.lines.push("} otherwise {".to_owned());
        unit.changes += nested.changes;
        unit.lines.extend(indent(nested.lines));
        unit.lines.push("}".to_owned());
        Ok(())
    }

    fn case_command(
        &mut self,
        session: &Session<'_>,
        subject: &Word,
        arms: &[CaseArm],
        span: Span,
    ) -> Result<Unit, Error> {
        let value = self.value(session, subject, span, false)?;
        let mut unit = Unit::default();
        let mut opened = false;
        for (index, arm) in arms.iter().enumerate() {
            let default = arm
                .patterns
                .iter()
                .any(|pattern| word_text(pattern).as_deref() == Some("*"));
            if default && index + 1 != arms.len() {
                return Err(self.fail(session, span, "`*)` before the last branch of `case`"));
            }
            let body = self.block(session, &arm.body)?;
            unit.changes += body.changes;
            if default {
                if !opened {
                    return Ok(Unit {
                        lines: body.lines,
                        changes: unit.changes,
                    });
                }
                unit.lines.push("} otherwise {".to_owned());
            } else {
                let mut alternatives = Vec::new();
                for pattern in &arm.patterns {
                    alternatives.push(self.pattern(session, &value, pattern, span)?);
                }
                let cond = alternatives.join(" or ");
                if opened {
                    unit.lines.push(format!("}} otherwise when {cond} {{"));
                } else {
                    unit.lines.push(format!("when {cond} {{"));
                    opened = true;
                }
            }
            unit.lines.extend(indent(body.lines));
        }
        if opened {
            unit.lines.push("}".to_owned());
        }
        Ok(unit)
    }

    fn pattern(
        &self,
        session: &Session<'_>,
        value: &str,
        pattern: &Word,
        span: Span,
    ) -> Result<String, Error> {
        let Some(text) = word_text(pattern) else {
            return Err(self.fail(session, span, "a `case` pattern with expansions"));
        };
        if text.contains(['*', '?', '[']) {
            return Ok(format!("{value} ~ {}", regex_literal(&glob_regex(&text))));
        }
        Ok(format!("{value} == {}", print_text(&text)))
    }

    // ----- loops --------------------------------------------------------------

    fn for_command(
        &mut self,
        session: &Session<'_>,
        name: &str,
        items: Option<&[Word]>,
        body: &Block,
        span: Span,
    ) -> Result<Unit, Error> {
        let source = match items {
            None => {
                if !self.params.iter().any(|p| p.name == "args") {
                    self.params.push(ScriptParam {
                        name: "args".to_owned(),
                        kind: ParamKind::List,
                        default: Some("[]".to_owned()),
                        note: None,
                    });
                    self.names.push("args".to_owned());
                }
                "@args".to_owned()
            }
            Some(words) => self.loop_source(session, words, span)?,
        };
        let variable = name.to_ascii_lowercase();
        let record = source.starts_with('(');
        self.names.push(variable.clone());
        if record {
            self.records.push(variable.clone());
        }
        let inner = self.block(session, body);
        self.names.pop();
        if record {
            self.records.pop();
        }
        let inner = inner?;
        let mut lines = vec![format!("each {source} as {variable} {{")];
        lines.extend(indent(inner.lines));
        lines.push("}".to_owned());
        Ok(Unit {
            lines,
            changes: inner.changes,
        })
    }

    fn loop_source(
        &mut self,
        session: &Session<'_>,
        words: &[Word],
        span: Span,
    ) -> Result<String, Error> {
        if let [word] = words {
            if word.has_unquoted_glob() && !word.has_expansion() {
                return self.glob_source(session, word, span);
            }
            if let [
                WordPart::Substitution {
                    body,
                    arithmetic: false,
                    ..
                },
            ] = word.parts.as_slice()
            {
                let names = self.names.clone();
                let context = self.child_context(&names);
                return match session::translate_plain(body, &context, self.table) {
                    Some(Ok(translation)) if translation.tree.program.statements.len() == 1 => {
                        let text = print_program(translation.tree.program.statements);
                        Ok(format!("({})", text.replace("\n|>", " |>")))
                    }
                    Some(Err(error)) => Err(error),
                    _ => Err(self.fail(
                        session,
                        span,
                        "the list of `for` is not one reading command",
                    )),
                };
            }
            let parts: Vec<&WordPart> = word
                .parts
                .iter()
                .filter(|part| !matches!(part, WordPart::Literal { text, .. } if text.is_empty()))
                .collect();
            if let [WordPart::Variable { name, .. }] = parts.as_slice()
                && name == "args"
            {
                return Ok("@args".to_owned());
            }
        }
        let mut values = Vec::new();
        for word in words {
            if word.has_unquoted_glob() && !word.has_expansion() {
                return Err(self.fail(
                    session,
                    span,
                    "a pattern together with other items of `for`",
                ));
            }
            values.push(self.value(session, word, span, false)?);
        }
        Ok(format!("[{}]", values.join(", ")))
    }

    /// The files a pattern names: the reading translation of `ls PATTERN`.
    fn glob_source(&self, session: &Session<'_>, word: &Word, span: Span) -> Result<String, Error> {
        let text = format!("ls {}", word.raw);
        let names = self.names.clone();
        let context = self.child_context(&names);
        let Some(Ok(translation)) = session::translate_plain(&text, &context, self.table) else {
            return Err(self.fail(session, span, "the pattern of `for` has no translation"));
        };
        let printed = print_program(translation.tree.program.statements).replace("\n|>", " |>");
        if printed.starts_with("find ") && !printed.contains(" |> ") {
            Ok(format!("({printed})"))
        } else {
            Err(self.fail(session, span, "the pattern of `for` has no translation"))
        }
    }

    fn while_command(
        &mut self,
        session: &Session<'_>,
        until: bool,
        condition: &Block,
        body: &Block,
        span: Span,
    ) -> Result<Unit, Error> {
        if let Some(unit) = self.getopts(session, condition, body, span)? {
            return Ok(unit);
        }
        let cond = self.condition(session, condition, false)?;
        let mut inner = self.block(session, body)?;
        let head = if until {
            let every = take_wait(body, &mut inner);
            self.assumptions.push(Assumption {
                rule: CONSTRUCT_RULE.to_owned(),
                text: "`until` tests before the first iteration; `repeat` runs the body once before its first test".to_owned(),
            });
            match every {
                Some(every) => format!("repeat until: {} every: {every}", cond.expr),
                None => format!("repeat until: {}", cond.expr),
            }
        } else {
            format!("while {}", cond.expr)
        };
        let mut lines = vec![format!("{head} {{")];
        lines.extend(indent(std::mem::take(&mut inner.lines)));
        lines.push("}".to_owned());
        Ok(Unit {
            lines,
            changes: inner.changes,
        })
    }

    /// `while getopts … do case … esac done` declares script parameters.
    fn getopts(
        &mut self,
        session: &Session<'_>,
        condition: &Block,
        body: &Block,
        span: Span,
    ) -> Result<Option<Unit>, Error> {
        let [entry] = condition.entries.as_slice() else {
            return Ok(None);
        };
        let Node::Simple(simple) = &entry.first else {
            return Ok(None);
        };
        if first_word(simple) != "getopts" {
            return Ok(None);
        }
        let optstring = simple.words.get(1).and_then(word_text).unwrap_or_default();
        let [case_entry] = body.entries.as_slice() else {
            return Err(self.fail(session, span, "the body of `getopts` is not one `case`"));
        };
        let Node::Case { arms, .. } = &case_entry.first else {
            return Err(self.fail(session, span, "the body of `getopts` is not one `case`"));
        };
        let letters: Vec<char> = optstring.trim_start_matches(':').chars().collect();
        for arm in arms {
            for pattern in &arm.patterns {
                let Some(text) = word_text(pattern) else {
                    continue;
                };
                let mut chars = text.chars();
                let (Some(letter), None) = (chars.next(), chars.next()) else {
                    continue;
                };
                let Some(position) = letters.iter().position(|c| *c == letter) else {
                    continue;
                };
                let takes_argument = letters.get(position + 1) == Some(&':');
                let name =
                    assigned_name(arm).unwrap_or_else(|| letter.to_ascii_lowercase().to_string());
                let (kind, default) = if takes_argument {
                    (ParamKind::Text, None)
                } else {
                    (ParamKind::Bool, Some("false".to_owned()))
                };
                self.names.push(name.clone());
                self.params.push(ScriptParam {
                    name,
                    kind,
                    default,
                    note: Some(format!("option -{letter} of getopts is a parameter")),
                });
            }
        }
        Ok(Some(Unit::default()))
    }

    // ----- functions ------------------------------------------------------------

    fn function(&mut self, session: &Session<'_>, name: &str, span: Span) -> Result<Unit, Error> {
        let text = session.source(span);
        let (Some(open), Some(close)) = (text.find('{'), text.rfind('}')) else {
            return Err(self.fail(session, span, "a function without a body"));
        };
        let rewritten = rewrite_positionals(&text[open + 1..close]);
        let inner_session = Session::new(self.table, self.context, &rewritten.text);
        let tokens =
            lex(&rewritten.text).map_err(|error| self.fail(session, span, &error.to_string()))?;
        let block =
            compound::parse(&tokens).map_err(|error| self.fail(session, span, error.message))?;
        let saved_names = self.names.clone();
        let saved_params = std::mem::take(&mut self.params);
        self.names
            .extend(rewritten.params.iter().map(|p| p.name.clone()));
        self.functions.push(Function {
            name: name.to_owned(),
            params: rewritten.params.clone(),
        });
        let body = self.block(&inner_session, &block);
        self.names = saved_names;
        self.params = saved_params;
        let body = body?;
        let declarations: Vec<String> = rewritten
            .params
            .iter()
            .map(ScriptParam::declaration)
            .collect();
        let mut lines = vec![format!("define {name}({}) {{", declarations.join(", "))];
        lines.extend(indent(body.lines));
        lines.push("}".to_owned());
        for param in &rewritten.params {
            if let Some(note) = &param.note {
                self.assumptions.push(Assumption {
                    rule: PARAMS_RULE.to_owned(),
                    text: format!("{name}: {note}"),
                });
            }
        }
        Ok(Unit { lines, changes: 0 })
    }
}

/// Variable assigned in a branch of the `case` of `getopts`.
fn assigned_name(arm: &CaseArm) -> Option<String> {
    arm.body
        .entries
        .iter()
        .find_map(|entry| match &entry.first {
            Node::Simple(simple) => simple.words.first().and_then(|word| {
                word.raw
                    .split_once('=')
                    .map(|(name, _)| name.to_ascii_lowercase())
                    .filter(|name| {
                        !name.is_empty()
                            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    })
            }),
            _ => None,
        })
}

/// Removes the trailing wait of the body of an `until` loop and returns it
/// as the `every` option.
fn take_wait(body: &Block, unit: &mut Unit) -> Option<String> {
    if body.entries.len() < 2 {
        return None;
    }
    let Node::Simple(simple) = &body.entries.last()?.first else {
        return None;
    };
    let [command, amount] = simple.words.as_slice() else {
        return None;
    };
    if word_text(command).as_deref() != Some(WAIT_COMMAND) {
        return None;
    }
    let amount = word_text(amount)?;
    let digits = amount.trim_end_matches(['s', 'm', 'h', 'd']);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !unit.lines.last()?.starts_with(WAIT_COMMAND) {
        return None;
    }
    unit.lines.pop();
    Some(if amount == digits {
        format!("{amount}s")
    } else {
        amount
    })
}

fn negate(expr: &str) -> String {
    if let Some(inner) = expr
        .strip_prefix("not (")
        .and_then(|e| e.strip_suffix(')'))
        .filter(|inner| !inner.contains(')'))
    {
        return inner.to_owned();
    }
    format!("not ({expr})")
}

/// Parenthesizes an operand of `and`/`or` when it holds the other one.
fn group(expr: &str, operator: &str) -> String {
    let other = if operator == "and" { " or " } else { " and " };
    if expr.contains(other) && !expr.starts_with('(') {
        format!("({expr})")
    } else {
        expr.to_owned()
    }
}

fn is_binary(op: &str) -> bool {
    matches!(
        op,
        "=" | "==" | "!=" | "=~" | "<" | ">" | "-eq" | "-ne" | "-gt" | "-ge" | "-lt" | "-le"
    )
}

fn is_unary(op: &str) -> bool {
    matches!(
        op,
        "-e" | "-a" | "-f" | "-d" | "-L" | "-h" | "-s" | "-z" | "-n" | "-r" | "-w" | "-x"
    )
}

/// Regular expression that matches what a shell pattern matches.
fn glob_regex(pattern: &str) -> String {
    let mut out = String::from("^");
    for c in pattern.chars() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '.' | '+' | '(' | ')' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.push('$');
    out
}

/// A regular expression written as a literal of the language.
fn regex_literal(pattern: &str) -> String {
    format!("/{}/", pattern.replace('/', "\\/"))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
