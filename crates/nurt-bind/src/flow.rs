// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Binding of the control constructs: conditions, loops, `steps`, functions
//! and script parameters.
//!
//! Everything is checked before any effect: conditions are `bool`, loop
//! options have their names and types, names refer to parameters, `let`
//! results or loop variables, functions are called with the declared
//! arguments, and `break` and `continue` stay inside a loop.

use nurt_contracts::ast::{
    Arg, BinOp, Expr, MapEntry, Pipeline, Ref, Span, Stage, Statement, UnOp,
};
use nurt_contracts::bound::{
    BoundArg, BoundBranch, BoundCall, BoundDefine, BoundEach, BoundExpr, BoundJump, BoundLimit,
    BoundParam, BoundParams, BoundRepeat, BoundStatement, BoundSteps, BoundWhen, BoundWhile, Phase,
};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::spec::{ParamType, ScalarType};
use nurt_correct::distance::{EXPECTED_LIMIT, NameEntry, NameIndex, NameKind, expected};

use crate::binder::{Binder, Flow, unsupported};
use crate::cast::Place;
use crate::error::BindError;

/// A function defined earlier in the program.
#[derive(Debug, Clone)]
pub(crate) struct Function {
    pub(crate) name: String,
    pub(crate) params: Vec<BoundParam>,
}

/// Names a construct reads without `@`: the loop variables in scope.
const EXIT_CODE: &str = "exit_code";

/// What is known about the type of a bound condition.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Bool,
    Other(&'static str),
    Unknown,
}

/// Options of a loop after their check.
#[derive(Debug, Default)]
struct LoopOptions {
    parallel: Option<u32>,
    atomic: Option<bool>,
    every_ms: Option<u64>,
    limit: Option<BoundLimit>,
    until: Option<BoundExpr>,
}

impl Binder<'_> {
    /// Binds a control construct at the top of a program or in a block.
    pub(crate) fn construct(
        &mut self,
        index: usize,
        statement: &Statement,
    ) -> Result<BoundStatement, BindError> {
        match statement {
            Statement::When(construct) => {
                let mut branches = Vec::with_capacity(construct.branches.len());
                for branch in &construct.branches {
                    let cond = self.condition(&branch.cond)?;
                    let body = self.block(index, &branch.body, &[], false)?;
                    branches.push(BoundBranch {
                        cond,
                        body,
                        span: branch.span,
                    });
                }
                let otherwise = match &construct.otherwise {
                    Some(body) => Some(self.block(index, body, &[], false)?),
                    None => None,
                };
                Ok(BoundStatement::When(BoundWhen {
                    branches,
                    otherwise,
                    span: construct.span,
                }))
            }
            Statement::Each(construct) => {
                let options =
                    self.loop_options("each", &construct.options, &["parallel", "atomic"])?;
                let source = self.sequence(&construct.source)?;
                let body = self.block(index, &construct.body, &[&construct.name], true)?;
                Ok(BoundStatement::Each(BoundEach {
                    source,
                    name: construct.name.clone(),
                    parallel: options.parallel,
                    atomic: options.atomic.unwrap_or(true),
                    body,
                    span: construct.span,
                }))
            }
            Statement::Repeat(construct) => {
                let options = self.loop_options(
                    "repeat",
                    &construct.options,
                    &["until", "every", "limit", "atomic"],
                )?;
                let Some(until) = options.until else {
                    return Err(BindError::new(
                        ErrorCode::MissingParam,
                        "`repeat` needs `until:` with a condition",
                    )
                    .at(construct.span)
                    .expected(&["until: <bool>".to_owned()]));
                };
                let body = self.block(index, &construct.body, &[], true)?;
                Ok(BoundStatement::Repeat(BoundRepeat {
                    until,
                    every_ms: options.every_ms,
                    limit: options.limit,
                    atomic: options.atomic.unwrap_or(true),
                    body,
                    span: construct.span,
                }))
            }
            Statement::While(construct) => {
                let options =
                    self.loop_options("while", &construct.options, &["limit", "atomic"])?;
                let cond = self.condition(&construct.cond)?;
                let body = self.block(index, &construct.body, &[], true)?;
                Ok(BoundStatement::While(BoundWhile {
                    cond,
                    limit: options.limit,
                    atomic: options.atomic.unwrap_or(true),
                    body,
                    span: construct.span,
                }))
            }
            Statement::Break(jump) | Statement::Continue(jump) => {
                let word = if matches!(statement, Statement::Break(_)) {
                    "break"
                } else {
                    "continue"
                };
                if self.loops == 0 {
                    return Err(BindError::new(
                        ErrorCode::Arg,
                        format!("`{word}` is allowed only inside a loop"),
                    )
                    .at(jump.span));
                }
                let bound = BoundJump { span: jump.span };
                Ok(if word == "break" {
                    BoundStatement::Break(bound)
                } else {
                    BoundStatement::Continue(bound)
                })
            }
            Statement::Steps(construct) => {
                let body = self.block(index, &construct.body, &[], false)?;
                Ok(BoundStatement::Steps(BoundSteps {
                    body,
                    span: construct.span,
                }))
            }
            Statement::Define(construct) => {
                let params = self.declarations(&construct.params)?;
                if self.catalog.command(&construct.name).is_some() {
                    return Err(BindError::new(
                        ErrorCode::Arg,
                        format!(
                            "`{}` is a command; choose another name for the function",
                            construct.name
                        ),
                    )
                    .at(construct.span));
                }
                if self
                    .functions
                    .iter()
                    .any(|known| known.name == construct.name)
                {
                    return Err(BindError::new(
                        ErrorCode::Arg,
                        format!("the function `{}` is already defined", construct.name),
                    )
                    .at(construct.span));
                }
                self.functions.push(Function {
                    name: construct.name.clone(),
                    params: params.clone(),
                });
                let body = self.function_body(index, &construct.body, &params)?;
                Ok(BoundStatement::Define(BoundDefine {
                    name: construct.name.clone(),
                    params,
                    body,
                    span: construct.span,
                }))
            }
            Statement::Params(construct) => {
                if index != 0 || self.nesting != 0 {
                    return Err(BindError::new(
                        ErrorCode::Arg,
                        "`params` is the first statement of a program",
                    )
                    .at(construct.span));
                }
                let params = self.declarations(&construct.params)?;
                for param in &params {
                    self.lets.push((param.name.clone(), Flow::any()));
                }
                Ok(BoundStatement::Params(BoundParams {
                    params,
                    span: construct.span,
                }))
            }
            _ => Err(unsupported("statement")),
        }
    }

    /// Binds the statements of a block in a scope of their own: names
    /// defined inside are not visible after the block, and corrections are
    /// not offered inside.
    fn block(
        &mut self,
        index: usize,
        body: &[Statement],
        variables: &[&str],
        in_loop: bool,
    ) -> Result<Vec<BoundStatement>, BindError> {
        let lets = self.lets.len();
        let vars = self.vars.len();
        let corrector = self.corrector.take();
        for name in variables {
            self.vars.push((*name).to_owned());
            self.lets.push(((*name).to_owned(), Flow::any()));
        }
        self.nesting += 1;
        self.loops += usize::from(in_loop);
        let mut bound = Vec::with_capacity(body.len());
        let mut failure = None;
        for statement in body {
            match self.statement(index, statement) {
                Ok(statement) => bound.push(statement),
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        self.loops -= usize::from(in_loop);
        self.nesting -= 1;
        self.corrector = corrector;
        self.lets.truncate(lets);
        self.vars.truncate(vars);
        match failure {
            Some(error) => Err(error),
            None => Ok(bound),
        }
    }

    /// Binds the body of a function: its parameters are its only names.
    fn function_body(
        &mut self,
        index: usize,
        body: &[Statement],
        params: &[BoundParam],
    ) -> Result<Vec<BoundStatement>, BindError> {
        let lets = std::mem::take(&mut self.lets);
        let vars = std::mem::take(&mut self.vars);
        let loops = std::mem::take(&mut self.loops);
        let isolated = std::mem::replace(&mut self.isolated, true);
        for param in params {
            self.lets.push((param.name.clone(), Flow::any()));
        }
        let bound = self.block(index, body, &[], false);
        self.lets = lets;
        self.vars = vars;
        self.loops = loops;
        self.isolated = isolated;
        bound
    }

    /// Binds the declared parameters of a function or of a script.
    fn declarations(
        &mut self,
        params: &[nurt_contracts::ast::Param],
    ) -> Result<Vec<BoundParam>, BindError> {
        let mut bound: Vec<BoundParam> = Vec::with_capacity(params.len());
        for param in params {
            if bound.iter().any(|known| known.name == param.name) {
                return Err(BindError::new(
                    ErrorCode::Arg,
                    format!("the parameter `{}` is declared twice", param.name),
                )
                .at(param.span));
            }
            let ty = ParamType::parse(&param.ty).map_err(|error| {
                BindError::new(
                    ErrorCode::Type,
                    format!("`{}` is not a type: {error}", param.ty),
                )
                .at(param.span)
                .expected(&type_names())
            })?;
            let default = match &param.default {
                Some(default) => Some(self.value(
                    default,
                    &ty,
                    Place {
                        command: "params",
                        param: &param.name,
                        per_record: false,
                    },
                )?),
                None => None,
            };
            bound.push(BoundParam {
                name: param.name.clone(),
                ty: param.ty.clone(),
                default,
            });
        }
        Ok(bound)
    }

    /// Binds the condition of a construct and checks that it is `bool`.
    fn condition(&mut self, expr: &Expr) -> Result<BoundExpr, BindError> {
        let bound = self.construct_expression(expr)?;
        match kind_of(&bound) {
            Kind::Other(found) => Err(BindError::new(
                ErrorCode::Type,
                format!("a condition is `bool`, found {found}; compare the value with `==`, `<` or `exists`"),
            )
            .at(crate::cast::expr_span(expr))
            .expected(&["bool".to_owned()])),
            Kind::Bool | Kind::Unknown => Ok(bound),
        }
    }

    /// Binds the records of an `each`: a list, a reference or a reading
    /// pipeline in parentheses.
    fn sequence(&mut self, expr: &Expr) -> Result<BoundExpr, BindError> {
        let bound = self.construct_expression(expr)?;
        match kind_of_sequence(&bound) {
            Some(found) => Err(BindError::new(
                ErrorCode::Type,
                format!("`each` visits a list of records, found {found}"),
            )
            .at(crate::cast::expr_span(expr))
            .expected(&["list".to_owned()])),
            None => Ok(bound),
        }
    }

    /// Binds an expression of a construct: loop variables are read without
    /// `@`, a reading pipeline in parentheses is a value and `exit_code` is
    /// the exit code of the last program run.
    fn construct_expression(&mut self, expr: &Expr) -> Result<BoundExpr, BindError> {
        let mut rewritten = expr.clone();
        let lets = self.lets.len();
        let vars = self.vars.len();
        self.vars.push(EXIT_CODE.to_owned());
        if self.let_flow(EXIT_CODE).is_none() {
            self.lets.push((EXIT_CODE.to_owned(), Flow::any()));
        }
        rewrite_expr(&mut rewritten, &self.vars);
        let sub = std::mem::replace(&mut self.allow_sub, true);
        let bound = self.expression(&rewritten);
        self.allow_sub = sub;
        self.lets.truncate(lets);
        self.vars.truncate(vars);
        bound
    }

    /// Checks the options of a loop against the names `allowed`.
    fn loop_options(
        &mut self,
        construct: &str,
        options: &[Arg],
        allowed: &[&str],
    ) -> Result<LoopOptions, BindError> {
        let mut seen: Vec<&str> = Vec::new();
        let mut checked = LoopOptions::default();
        for option in options {
            let Some(name) = option.name.as_deref() else {
                return Err(BindError::new(
                    ErrorCode::Arg,
                    format!("an option of `{construct}` is written as `name: value`"),
                )
                .at(option.span));
            };
            if !allowed.contains(&name) {
                let index = NameIndex::new(
                    allowed
                        .iter()
                        .map(|known| NameEntry::name((*known).to_owned(), NameKind::Param)),
                );
                return Err(BindError::new(
                    ErrorCode::UnknownParam,
                    format!("`{construct}` has no option `{name}`"),
                )
                .at(option.span)
                .found(name)
                .expected(&expected(name, &index, NameKind::Param, EXPECTED_LIMIT)));
            }
            if seen.contains(&name) {
                return Err(BindError::new(
                    ErrorCode::ParamConflict,
                    format!("the option `{name}` of `{construct}` is written twice"),
                )
                .at(option.span));
            }
            seen.push(name);
            match name {
                "parallel" => {
                    let value = integer_option(&option.value, "parallel")?;
                    checked.parallel = Some(u32::try_from(value).map_err(|_| {
                        option_error("parallel", "an integer from 1 to 4294967295", option)
                    })?);
                    if value < 1 {
                        return Err(option_error("parallel", "an integer of at least 1", option));
                    }
                }
                "atomic" => match &option.value {
                    Expr::Bool { v, .. } => checked.atomic = Some(*v),
                    _ => return Err(option_error("atomic", "`true` or `false`", option)),
                },
                "every" => match &option.value {
                    Expr::Dur { v, .. } => checked.every_ms = Some(*v),
                    _ => return Err(option_error("every", "a duration such as `5s`", option)),
                },
                "limit" => {
                    checked.limit = Some(match &option.value {
                        Expr::Dur { v, .. } => BoundLimit::Time { ms: *v },
                        Expr::Int { v, .. } if *v >= 0 => BoundLimit::Iterations {
                            n: u64::try_from(*v).unwrap_or(0),
                        },
                        _ => {
                            return Err(option_error(
                                "limit",
                                "a duration such as `2m` or a number of iterations",
                                option,
                            ));
                        }
                    });
                }
                _ => checked.until = Some(self.condition(&option.value)?),
            }
        }
        Ok(checked)
    }

    /// Binds a call of a defined function, `None` when the stage names no
    /// function.
    pub(crate) fn call(&mut self, stage: &Stage) -> Option<Result<BoundCall, BindError>> {
        let function = self
            .functions
            .iter()
            .find(|known| known.name == stage.name)?
            .clone();
        Some(self.bind_call(&function, stage))
    }

    fn bind_call(&mut self, function: &Function, stage: &Stage) -> Result<BoundCall, BindError> {
        let params = &function.params;
        let mut slots: Vec<Option<BoundExpr>> = vec![None; params.len()];
        let mut next = 0;
        for arg in &stage.args {
            let slot = match &arg.name {
                Some(name) => params
                    .iter()
                    .position(|param| &param.name == name)
                    .ok_or_else(|| unknown_parameter(function, name, arg.span))?,
                None => {
                    if let Expr::Ident { v, .. } = &arg.value
                        && let Some(found) = params
                            .iter()
                            .position(|param| &param.name == v && param.ty == "bool")
                        && slots[found].is_none()
                    {
                        slots[found] = Some(BoundExpr::Bool { v: true });
                        continue;
                    }
                    if let Expr::Ident { v, .. } = &arg.value
                        && let Some(found) = params
                            .iter()
                            .position(|param| &param.name == v && param.ty != "bool")
                        && slots[found].is_none()
                        && !params.iter().any(|param| {
                            param.ty.starts_with("enum") && param.ty.contains(v.as_str())
                        })
                    {
                        return Err(missing_value(&params[found], arg.span));
                    }
                    while next < params.len() && slots[next].is_some() {
                        next += 1;
                    }
                    if next >= params.len() {
                        return Err(BindError::new(
                            ErrorCode::Arity,
                            format!(
                                "`{}` takes {} argument(s), more were given",
                                function.name,
                                params.len()
                            ),
                        )
                        .at(arg.span));
                    }
                    next
                }
            };
            if slots[slot].is_some() {
                return Err(BindError::new(
                    ErrorCode::ParamConflict,
                    format!(
                        "the parameter `{}` of `{}` is given twice",
                        params[slot].name, function.name
                    ),
                )
                .at(arg.span));
            }
            let ty = ParamType::parse(&params[slot].ty).map_err(|error| {
                BindError::new(
                    ErrorCode::Type,
                    format!("`{}` is not a type: {error}", params[slot].ty),
                )
            })?;
            let mut value = arg.value.clone();
            rewrite_expr(&mut value, &self.vars);
            slots[slot] = Some(self.value(
                &value,
                &ty,
                Place {
                    command: &function.name,
                    param: &params[slot].name,
                    per_record: false,
                },
            )?);
        }
        let mut args = Vec::with_capacity(params.len());
        for (param, slot) in params.iter().zip(slots) {
            match (slot, &param.default) {
                (Some(value), _) => args.push(BoundArg::given(param.name.clone(), value)),
                (None, Some(default)) => {
                    args.push(BoundArg::from_default(param.name.clone(), default.clone()));
                }
                (None, None) => {
                    return Err(BindError::new(
                        ErrorCode::MissingParam,
                        format!("`{}` needs `{}: {}`", function.name, param.name, param.ty),
                    )
                    .at(stage.span)
                    .expected(std::slice::from_ref(&param.ty)));
                }
            }
        }
        Ok(BoundCall {
            name: function.name.clone(),
            args,
            span: stage.span,
        })
    }

    /// Binds a reading pipeline written in parentheses inside an
    /// expression of a construct.
    pub(crate) fn value_pipeline(
        &mut self,
        pipeline: &Pipeline,
        span: Option<Span>,
    ) -> Result<BoundExpr, BindError> {
        let bound = self.sub_pipeline(pipeline)?;
        let changing = bound
            .head_phase()
            .into_iter()
            .chain(bound.stages.iter().map(|stage| stage.phase))
            .any(Phase::is_effect);
        if changing {
            return Err(BindError::new(
                ErrorCode::Type,
                "a pipeline in an expression only reads; move the change to a statement",
            )
            .at(span));
        }
        Ok(BoundExpr::Sub { p: bound })
    }
}

trait HeadPhase {
    fn head_phase(&self) -> Option<Phase>;
}

impl HeadPhase for nurt_contracts::bound::BoundPipeline {
    fn head_phase(&self) -> Option<Phase> {
        match &self.head {
            nurt_contracts::bound::BoundHead::Stage(stage) => Some(stage.phase),
            _ => None,
        }
    }
}

fn type_names() -> Vec<String> {
    ScalarType::ALL
        .iter()
        .map(|scalar| format!("{scalar:?}").to_lowercase())
        .collect()
}

/// A parameter named without a value that is not `bool`.
fn missing_value(param: &BoundParam, span: Option<Span>) -> BindError {
    BindError::new(
        ErrorCode::MissingParam,
        format!(
            "`{}` needs a value: write `{}: <{}>`; only a parameter of type bool is given without one",
            param.name, param.name, param.ty
        ),
    )
    .at(span)
    .found(param.name.clone())
    .expected(std::slice::from_ref(&param.ty))
    .help("syntax")
}

fn unknown_parameter(function: &Function, name: &str, span: Option<Span>) -> BindError {
    let index = NameIndex::new(
        function
            .params
            .iter()
            .map(|param| NameEntry::name(param.name.clone(), NameKind::Param)),
    );
    BindError::new(
        ErrorCode::UnknownParam,
        format!("`{}` has no parameter `{name}`", function.name),
    )
    .at(span)
    .found(name)
    .expected(&expected(name, &index, NameKind::Param, EXPECTED_LIMIT))
}

fn option_error(name: &str, wanted: &str, option: &Arg) -> BindError {
    BindError::new(ErrorCode::Type, format!("the option `{name}` is {wanted}"))
        .at(option.span)
        .expected(&[wanted.to_owned()])
}

fn integer_option(expr: &Expr, name: &str) -> Result<i64, BindError> {
    match expr {
        Expr::Int { v, .. } => Ok(*v),
        other => Err(BindError::new(
            ErrorCode::Type,
            format!("the option `{name}` is an integer"),
        )
        .at(crate::cast::expr_span(other))),
    }
}

/// What the bound condition is, as far as it is known before the run.
fn kind_of(expr: &BoundExpr) -> Kind {
    match expr {
        BoundExpr::Bool { .. } => Kind::Bool,
        BoundExpr::Un { op: UnOp::Not, .. } => Kind::Bool,
        BoundExpr::Bin { op, .. } => match op {
            BinOp::Eq
            | BinOp::Ne
            | BinOp::Lt
            | BinOp::Le
            | BinOp::Gt
            | BinOp::Ge
            | BinOp::Match
            | BinOp::NotMatch
            | BinOp::In
            | BinOp::And
            | BinOp::Or => Kind::Bool,
            _ => Kind::Other("a value"),
        },
        BoundExpr::Call { function, .. } if function == "exists" => Kind::Bool,
        BoundExpr::Null => Kind::Other("null"),
        BoundExpr::Int { .. } | BoundExpr::Num { .. } => Kind::Other("a number"),
        BoundExpr::Text { .. } | BoundExpr::Path { .. } => Kind::Other("text"),
        BoundExpr::List { .. } => Kind::Other("a list"),
        BoundExpr::Map { .. } => Kind::Other("a map"),
        _ => Kind::Unknown,
    }
}

/// Names what a bound expression is when it cannot be a list of records.
fn kind_of_sequence(expr: &BoundExpr) -> Option<&'static str> {
    match expr {
        BoundExpr::Null => Some("null"),
        BoundExpr::Bool { .. } => Some("a bool"),
        BoundExpr::Int { .. } | BoundExpr::Num { .. } => Some("a number"),
        BoundExpr::Text { .. } | BoundExpr::Path { .. } => Some("text"),
        BoundExpr::Map { .. } => Some("a map"),
        _ => None,
    }
}

/// Replaces the names of `vars` by references: `f` becomes `@f` and `f.path`
/// becomes `@f.path`.
pub(crate) fn rewrite_expr(expr: &mut Expr, vars: &[String]) {
    if vars.is_empty() {
        return;
    }
    if let Some(reference) = variable(expr, vars) {
        *expr = Expr::Ref(reference);
        return;
    }
    match expr {
        Expr::Field { of: Some(of), .. } => rewrite_expr(of, vars),
        Expr::Call { args, .. } => args.iter_mut().for_each(|arg| rewrite_expr(arg, vars)),
        Expr::Un { x, .. } | Expr::Spread { x, .. } => rewrite_expr(x, vars),
        Expr::Bin { l, r, .. } => {
            rewrite_expr(l, vars);
            rewrite_expr(r, vars);
        }
        Expr::List { items, .. } => items.iter_mut().for_each(|item| rewrite_expr(item, vars)),
        Expr::Map { entries, .. } => entries
            .iter_mut()
            .for_each(|MapEntry { v, .. }| rewrite_expr(v, vars)),
        Expr::Sub { p, .. } => rewrite_pipeline(p, vars),
        _ => {}
    }
}

/// Rewrites the arguments of every stage of a pipeline.
pub(crate) fn rewrite_pipeline(pipeline: &mut Pipeline, vars: &[String]) {
    if let nurt_contracts::ast::Head::Stage(stage) = &mut pipeline.head {
        rewrite_stage(stage, vars);
    }
    for stage in &mut pipeline.stages {
        rewrite_stage(stage, vars);
    }
}

fn rewrite_stage(stage: &mut Stage, vars: &[String]) {
    for arg in &mut stage.args {
        rewrite_expr(&mut arg.value, vars);
    }
}

/// The reference a variable written without `@` stands for.
fn variable(expr: &Expr, vars: &[String]) -> Option<Ref> {
    match expr {
        Expr::Ident { v, span } if vars.contains(v) => Some(Ref {
            name: v.clone(),
            fields: Vec::new(),
            span: *span,
        }),
        Expr::Field {
            name,
            of: Some(of),
            span,
        } => {
            let mut reference = variable(of, vars)?;
            reference.fields.push(name.clone());
            reference.span = *span;
            Some(reference)
        }
        _ => None,
    }
}
