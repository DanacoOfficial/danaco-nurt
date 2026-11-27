// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Layout of statements, pipelines and stages in the three variants.

use nurt_contracts::ast::{Arg, Expr, Head, Param, Pipeline, Program, Stage, Statement};

use super::PrintError;
use super::expr::{Ctx, Lead, Out, PREC_ANY, Printer, ends_open, fits, leading};
use super::literal;
use crate::parse::Variant;

/// Printed lines of one or more consecutive elements of a statement.
///
/// Element 0 is the head of the pipeline (with `let name = ` in front of it)
/// and element `k` is the `k`-th stage after the head. `text` holds no
/// trailing line break and may span several lines when a list or a map is
/// broken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Segment {
    /// First element printed in the segment.
    pub(crate) first: usize,
    /// Last element printed in the segment.
    pub(crate) last: usize,
    /// Printed text.
    pub(crate) text: String,
}

/// Position of a stage, which decides how its first argument is guarded.
/// A line of a construct: it maps to no stage of a pipeline.
fn line(text: String) -> Segment {
    Segment {
        first: 0,
        last: 0,
        text,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// A stage of a statement or one of several stages of a sub-pipeline.
    Free,
    /// The only element of a parenthesized sub-pipeline, where the first
    /// argument must start like an argument for the parentheses to read as
    /// a sub-pipeline.
    LoneInParens,
}

impl Printer {
    /// Lays out every statement of a program.
    pub(super) fn program(&mut self, program: &Program) -> Vec<Vec<Segment>> {
        if program.statements.is_empty() {
            self.unprintable("program", "has no statements; add at least one", None);
        }
        program
            .statements
            .iter()
            .map(|statement| self.statement(statement))
            .collect()
    }

    fn statement(&mut self, statement: &Statement) -> Vec<Segment> {
        match statement {
            Statement::Let(binding) => {
                if !literal::is_identifier(&binding.name) {
                    self.unprintable(
                        "let",
                        "has a name that is not an identifier or is a reserved word; rename it",
                        binding.span,
                    );
                }
                let prefix = format!("let {} = ", binding.name);
                self.pipeline_lines(&prefix, &binding.value)
            }
            Statement::Pipeline(pipeline) => self.pipeline_lines("", pipeline),
            Statement::When(construct) => {
                let mut lines = Vec::new();
                for (index, branch) in construct.branches.iter().enumerate() {
                    let lead = match index {
                        0 => "when ",
                        _ => "} otherwise when ",
                    };
                    let head = format!("{lead}{} {{", self.inline(&branch.cond));
                    lines.push(line(head));
                    lines.extend(self.block(&branch.body));
                }
                if let Some(body) = &construct.otherwise {
                    lines.push(line("} otherwise {".to_owned()));
                    lines.extend(self.block(body));
                }
                lines.push(line("}".to_owned()));
                lines
            }
            Statement::Each(construct) => {
                let head = format!(
                    "each {} as {}{} {{",
                    self.inline(&construct.source),
                    construct.name,
                    self.options(&construct.options)
                );
                self.closed(head, &construct.body)
            }
            Statement::Repeat(construct) => {
                let head = format!("repeat{} {{", self.options(&construct.options));
                self.closed(head, &construct.body)
            }
            Statement::While(construct) => {
                let head = format!(
                    "while {}{} {{",
                    self.inline(&construct.cond),
                    self.options(&construct.options)
                );
                self.closed(head, &construct.body)
            }
            Statement::Break(_) => vec![line("break".to_owned())],
            Statement::Continue(_) => vec![line("continue".to_owned())],
            Statement::Steps(construct) => self.closed("steps {".to_owned(), &construct.body),
            Statement::Define(construct) => {
                if !literal::is_identifier(&construct.name) {
                    self.unprintable(
                        "define",
                        "has a name that is not an identifier or is a reserved word; rename it",
                        construct.span,
                    );
                }
                let params: Vec<String> = construct
                    .params
                    .iter()
                    .map(|param| self.declaration(param))
                    .collect();
                let head = format!("define {}({}) {{", construct.name, params.join(", "));
                self.closed(head, &construct.body)
            }
            Statement::Params(construct) => {
                let mut lines = vec![line("params {".to_owned())];
                for param in &construct.params {
                    lines.push(line(format!("    {}", self.declaration(param))));
                }
                lines.push(line("}".to_owned()));
                lines
            }
            _ => {
                self.unprintable(
                    "statement",
                    "has a kind this printer does not know; upgrade the printer",
                    None,
                );
                Vec::new()
            }
        }
    }

    /// A pipeline whose head is a stage with arguments and which has further
    /// stages is printed one stage per line; any other pipeline is printed
    /// on one line when the line fits, otherwise one stage per line.
    /// Prints an expression on one line.
    fn inline(&mut self, expr: &Expr) -> String {
        let mut out = Out::default();
        self.expr(expr, PREC_ANY, Ctx::TOP, &mut out);
        out.text
    }

    /// Prints the options of a loop: `name: value`, and `name` for `true`.
    fn options(&mut self, options: &[Arg]) -> String {
        let mut text = String::new();
        for option in options {
            text.push(' ');
            let name = option.name.as_deref().unwrap_or_default();
            if matches!(option.value, Expr::Bool { v: true, .. }) {
                text.push_str(name);
            } else {
                text.push_str(name);
                text.push_str(": ");
                text.push_str(&self.inline(&option.value));
            }
        }
        text
    }

    /// Prints `name: type = default`.
    fn declaration(&mut self, param: &Param) -> String {
        let mut text = format!("{}: {}", param.name, param.ty);
        if let Some(default) = &param.default {
            text.push_str(" = ");
            text.push_str(&self.inline(default));
        }
        text
    }

    /// Prints the statements of a block, indented by four spaces.
    fn block(&mut self, body: &[Statement]) -> Vec<Segment> {
        let mut lines = Vec::new();
        for statement in body {
            for mut segment in self.statement(statement) {
                segment.text = format!("    {}", segment.text);
                lines.push(segment);
            }
        }
        lines
    }

    /// Prints a header line, the block and the closing brace.
    fn closed(&mut self, head: String, body: &[Statement]) -> Vec<Segment> {
        let mut lines = vec![line(head)];
        lines.extend(self.block(body));
        lines.push(line("}".to_owned()));
        lines
    }

    fn pipeline_lines(&mut self, prefix: &str, pipeline: &Pipeline) -> Vec<Segment> {
        let last = pipeline.stages.len();
        let stacked =
            last > 0 && matches!(&pipeline.head, Head::Stage(stage) if !stage.args.is_empty());
        if !stacked {
            let mut out = Out::default();
            out.push(prefix);
            self.head(&pipeline.head, Place::Free, Ctx::TOP, &mut out);
            for stage in &pipeline.stages {
                out.push(" |> ");
                self.stage(stage, Place::Free, Ctx::TOP, &mut out);
            }
            if last == 0 || fits(&out.text, 0) {
                return vec![Segment {
                    first: 0,
                    last,
                    text: out.text,
                }];
            }
        }
        let mut segments = Vec::with_capacity(last + 1);
        let mut out = Out::default();
        out.push(prefix);
        self.head(&pipeline.head, Place::Free, Ctx::TOP, &mut out);
        segments.push(Segment {
            first: 0,
            last: 0,
            text: out.text,
        });
        for (index, stage) in pipeline.stages.iter().enumerate() {
            let mut out = Out::default();
            out.push("|> ");
            self.stage(stage, Place::Free, Ctx::TOP, &mut out);
            segments.push(Segment {
                first: index + 1,
                last: index + 1,
                text: out.text,
            });
        }
        segments
    }

    /// Prints a pipeline on one line, as inside parentheses.
    pub(super) fn pipeline_flat(&mut self, pipeline: &Pipeline, ctx: Ctx, out: &mut Out) {
        if !self.enter(ctx) {
            out.push("null");
            return;
        }
        let ctx = ctx.flat();
        let place = if pipeline.stages.is_empty() {
            Place::LoneInParens
        } else {
            Place::Free
        };
        self.head(&pipeline.head, place, ctx, out);
        for stage in &pipeline.stages {
            out.push(" |> ");
            self.stage(stage, Place::Free, ctx, out);
        }
    }

    fn head(&mut self, head: &Head, place: Place, ctx: Ctx, out: &mut Out) {
        match head {
            Head::Stage(stage) => self.stage(stage, place, ctx, out),
            Head::Ref(reference) => self.reference(reference, out),
            _ => {
                self.unprintable(
                    "pipeline",
                    "has a head kind this printer does not know; upgrade the printer",
                    None,
                );
                out.push("null");
            }
        }
    }

    fn stage(&mut self, stage: &Stage, place: Place, ctx: Ctx, out: &mut Out) {
        if !literal::is_command_name(&stage.name) {
            self.unprintable(
                "stage",
                "has a command name that is not identifiers joined by `.`; rename the command",
                stage.span,
            );
        }
        out.push(&stage.name);
        match self.variant {
            Variant::B => self.arguments_b(&stage.args, ctx, out),
            variant => self.arguments_spaced(&stage.args, place, variant == Variant::C, ctx, out),
        }
    }

    /// Variants A and C: arguments separated by one space, `name: value`
    /// in A and `name=value` in C.
    fn arguments_spaced(
        &mut self,
        args: &[Arg],
        place: Place,
        named_only: bool,
        ctx: Ctx,
        out: &mut Out,
    ) {
        let ctx = ctx.deeper();
        for (index, arg) in args.iter().enumerate() {
            out.push(" ");
            let followed = index + 1 < args.len();
            match &arg.name {
                Some(name) if named_only => {
                    self.check_argument_name(name, arg);
                    out.push(name);
                    out.push("=");
                    self.expr(&arg.value, PREC_ANY, ctx, out);
                }
                Some(name) => {
                    self.check_argument_name(name, arg);
                    out.push(name);
                    out.push(": ");
                    self.argument_value(&arg.value, false, followed, ctx, out);
                }
                None => {
                    if named_only {
                        self.report(PrintError::PositionalArgument { span: arg.span });
                    }
                    let after_token = index > 0 || place == Place::LoneInParens;
                    self.argument_value(&arg.value, after_token, followed, ctx, out);
                }
            }
        }
    }

    /// Prints an argument value of variant A, in parentheses when its first
    /// or last token would merge with the neighbouring argument.
    fn argument_value(
        &mut self,
        value: &Expr,
        after_token: bool,
        followed: bool,
        ctx: Ctx,
        out: &mut Out,
    ) {
        let guarded = match leading(value) {
            Lead::Not | Lead::OpenStart => after_token,
            Lead::Other => false,
        } || (followed && ends_open(value));
        if guarded {
            out.push("(");
            self.expr(value, PREC_ANY, ctx, out);
            out.push(")");
        } else {
            self.expr(value, PREC_ANY, ctx, out);
        }
    }

    /// Variant B: `name(arg, name: value)`; an argument that is a
    /// sub-pipeline with several stages is written without parentheses.
    fn arguments_b(&mut self, args: &[Arg], ctx: Ctx, out: &mut Out) {
        let ctx = ctx.deeper();
        out.push("(");
        for (index, arg) in args.iter().enumerate() {
            if index > 0 {
                out.push(", ");
            }
            if let Some(name) = &arg.name {
                self.check_argument_name(name, arg);
                out.push(name);
                out.push(": ");
            }
            match &arg.value {
                Expr::Sub { p, span } if !p.stages.is_empty() && self.enter(ctx) => {
                    self.check_sub(p, *span);
                    self.pipeline_flat(p, ctx.deeper(), out);
                }
                value => self.expr(value, PREC_ANY, ctx, out),
            }
        }
        out.push(")");
    }

    fn check_argument_name(&mut self, name: &str, arg: &Arg) {
        if !literal::is_identifier_shape(name) {
            self.unprintable(
                "argument",
                "has a parameter name that is not an identifier `[a-z][a-z0-9_]*`; rename it",
                arg.span,
            );
        }
    }
}
