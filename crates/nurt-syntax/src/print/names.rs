// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Naming of positional arguments, the prerequisite of printing in variant C.

use nurt_contracts::ast::{Arg, AstDocument, Expr, Head, KeySeg, Pipeline, Span, Stage, Statement};

use super::{MAX_NESTING_DEPTH, PrintError};

/// Positional parameters of one form of a command, as declared in the
/// command catalog.
///
/// A command whose parameters depend on its input declares several forms; a
/// command without forms has one form made of its parameters with a
/// position. `head` and `follower` translate the input kinds of the form: a
/// form with the input `none` applies to the first stage of a pipeline, a
/// form with any other input kind to a stage after `|>`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PositionalForm {
    /// Names of the positional parameters, in position order.
    pub parameters: Vec<String>,
    /// Index in `parameters` of the repeated parameter, which collects the
    /// surplus positional arguments into one list.
    pub repeated: Option<usize>,
    /// Whether the form applies to the first stage of a pipeline.
    pub head: bool,
    /// Whether the form applies to a stage after `|>`.
    pub follower: bool,
}

/// Source of parameter names for positional arguments.
///
/// The command catalog implements this trait. A command for which
/// [`forms`](Self::forms) returns forms is named by [`name_positional`]
/// according to its forms; any other command is named by
/// [`name`](Self::name).
///
/// # Examples
///
/// ```
/// use nurt_syntax::print::{PositionalForm, PositionalNames};
///
/// struct Catalog;
///
/// impl PositionalNames for Catalog {
///     fn name(&self, command: &str, index: usize) -> Option<String> {
///         match (command, index) {
///             ("find", 0) => Some("path".to_owned()),
///             ("first", 0) => Some("count".to_owned()),
///             _ => None,
///         }
///     }
///
///     fn forms(&self, command: &str) -> Vec<PositionalForm> {
///         let form = |parameters: &[&str], head, follower| PositionalForm {
///             parameters: parameters.iter().map(|&name| name.to_owned()).collect(),
///             repeated: None,
///             head,
///             follower,
///         };
///         match command {
///             "replace" => vec![
///                 form(&["pattern", "with"], true, false),
///                 form(&["with"], false, true),
///             ],
///             _ => Vec::new(),
///         }
///     }
/// }
///
/// assert_eq!(Catalog.name("find", 0).as_deref(), Some("path"));
/// assert_eq!(Catalog.name("find", 1), None);
/// assert_eq!(Catalog.forms("replace").len(), 2);
/// ```
pub trait PositionalNames {
    /// Returns the name of the positional parameter `index` (zero-based, in
    /// the order of the positional parameters) of `command`, or `None` when
    /// the command has no such parameter.
    fn name(&self, command: &str, index: usize) -> Option<String>;

    /// Returns the forms of `command` in catalog order, or an empty list when
    /// [`name`](Self::name) names the arguments of the command. The default
    /// returns an empty list for every command.
    fn forms(&self, _command: &str) -> Vec<PositionalForm> {
        Vec::new()
    }
}

/// Returns a copy of `document` in which every positional argument carries
/// the name of its parameter.
///
/// Positional arguments of a stage are counted from zero in source order,
/// skipping named arguments. When `names.forms` returns no form for the
/// command, the `k`-th one receives `names.name(command, k)`.
///
/// Otherwise one form is selected among those that apply at the place of the
/// stage, or among all forms when none applies: the first form with as many
/// parameters as the stage has positional arguments, else the first form
/// with a repeated parameter and fewer parameters, else the first form with
/// more parameters, else the first form. The arguments take the parameters
/// of the form in order. When there are more arguments than parameters, the
/// repeated parameter takes the surplus: the parameters after it take the
/// last arguments, and the arguments from its position up to them become one
/// list argument, placed where the first of them stands and spanning them.
///
/// Stages inside sub-pipelines are named as well. Spans and metadata are
/// kept. The result prints in variant C.
///
/// # Errors
///
/// Returns [`PrintError::UnnamedArgument`] for the first positional argument
/// without a parameter name and [`PrintError::TooDeep`] for a tree nested
/// deeper than [`MAX_NESTING_DEPTH`].
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::{Arg, AstDocument, Expr, Head, Pipeline, Program, Stage, Statement};
/// use nurt_syntax::parse::Variant;
/// use nurt_syntax::print::{self, PositionalNames};
///
/// struct Names;
///
/// impl PositionalNames for Names {
///     fn name(&self, command: &str, index: usize) -> Option<String> {
///         (command == "first" && index == 0).then(|| "count".to_owned())
///     }
/// }
///
/// let stage = Stage {
///     name: "first".to_owned(),
///     args: vec![Arg { name: None, value: Expr::Int { v: 20, span: None }, span: None }],
///     span: None,
/// };
/// let document = AstDocument::new(Program {
///     statements: vec![Statement::Pipeline(Pipeline {
///         head: Head::Stage(stage),
///         stages: Vec::new(),
///         span: None,
///     })],
/// });
/// let named = print::name_positional(&document, &Names)?;
/// assert_eq!(print::try_print(&named, Variant::C)?, "first count=20\n");
/// # Ok::<(), print::PrintError>(())
/// ```
pub fn name_positional(
    document: &AstDocument,
    names: &dyn PositionalNames,
) -> Result<AstDocument, PrintError> {
    let mut named = document.clone();
    let mut namer = Namer { names };
    name_statements(&mut namer, &mut named.program.statements)?;
    Ok(named)
}

fn name_statements(namer: &mut Namer<'_>, statements: &mut [Statement]) -> Result<(), PrintError> {
    for statement in statements {
        match statement {
            Statement::Let(binding) => namer.pipeline(&mut binding.value, 1)?,
            Statement::Pipeline(pipeline) => namer.pipeline(pipeline, 1)?,
            Statement::When(construct) => {
                for branch in &mut construct.branches {
                    name_statements(namer, &mut branch.body)?;
                }
                if let Some(body) = &mut construct.otherwise {
                    name_statements(namer, body)?;
                }
            }
            Statement::Each(construct) => name_statements(namer, &mut construct.body)?,
            Statement::Repeat(construct) => name_statements(namer, &mut construct.body)?,
            Statement::While(construct) => name_statements(namer, &mut construct.body)?,
            Statement::Steps(construct) => name_statements(namer, &mut construct.body)?,
            Statement::Define(construct) => name_statements(namer, &mut construct.body)?,
            _ => {}
        }
    }
    Ok(())
}

/// Selects the form for a stage with `count` positional arguments; `None`
/// when the command has no forms.
fn select_form(forms: &[PositionalForm], count: usize, head: bool) -> Option<&PositionalForm> {
    let applying: Vec<&PositionalForm> = forms
        .iter()
        .filter(|form| if head { form.head } else { form.follower })
        .collect();
    let candidates = if applying.is_empty() {
        forms.iter().collect()
    } else {
        applying
    };
    let width = |form: &&&PositionalForm| form.parameters.len();
    candidates
        .iter()
        .find(|form| width(form) == count)
        .or_else(|| {
            candidates
                .iter()
                .find(|form| form.repeated.is_some() && width(form) < count)
        })
        .or_else(|| candidates.iter().find(|form| width(form) > count))
        .or_else(|| candidates.first())
        .copied()
}

/// Range from the start of `first` to the end of `last`, when both are known.
fn covering(first: Option<Span>, last: Option<Span>) -> Option<Span> {
    let (first, last) = (first?, last?);
    Some(Span {
        line: first.line,
        column: first.column,
        end_line: last.end_line,
        end_column: last.end_column,
    })
}

/// Replaces the arguments at the indices `grouped` (ascending) by one list
/// argument named `name`, at the index of the first of them.
fn group_arguments(stage: &mut Stage, grouped: &[usize], name: String) {
    let Some(&first) = grouped.first() else {
        return;
    };
    let mut args = Vec::with_capacity(stage.args.len());
    let mut taken = Vec::with_capacity(grouped.len());
    let mut next = 0;
    for (index, arg) in std::mem::take(&mut stage.args).into_iter().enumerate() {
        if grouped.get(next) == Some(&index) {
            next += 1;
            taken.push(arg);
        } else {
            args.push(arg);
        }
    }
    let span = covering(
        taken.first().and_then(|arg| arg.span),
        taken.last().and_then(|arg| arg.span),
    );
    let items = taken.into_iter().map(|arg| arg.value).collect();
    args.insert(
        first,
        Arg {
            name: Some(name),
            value: Expr::List { items, span },
            span,
        },
    );
    stage.args = args;
}

struct Namer<'a> {
    names: &'a dyn PositionalNames,
}

impl Namer<'_> {
    fn pipeline(&mut self, pipeline: &mut Pipeline, depth: usize) -> Result<(), PrintError> {
        if let Head::Stage(stage) = &mut pipeline.head {
            self.stage(stage, true, depth)?;
        }
        for stage in &mut pipeline.stages {
            self.stage(stage, false, depth)?;
        }
        Ok(())
    }

    fn stage(&mut self, stage: &mut Stage, head: bool, depth: usize) -> Result<(), PrintError> {
        let count = stage.args.iter().filter(|arg| arg.name.is_none()).count();
        if count > 0 {
            let forms = self.names.forms(&stage.name);
            match select_form(&forms, count, head) {
                Some(form) => Self::name_by_form(stage, form, count)?,
                None => self.name_by_index(stage)?,
            }
        }
        stage
            .args
            .iter_mut()
            .try_for_each(|arg| self.expr(&mut arg.value, depth + 1))
    }

    fn name_by_index(&self, stage: &mut Stage) -> Result<(), PrintError> {
        for (position, arg) in stage
            .args
            .iter_mut()
            .filter(|arg| arg.name.is_none())
            .enumerate()
        {
            let name = self.names.name(&stage.name, position).ok_or_else(|| {
                PrintError::UnnamedArgument {
                    command: stage.name.clone(),
                    index: position,
                    span: arg.span,
                }
            })?;
            arg.name = Some(name);
        }
        Ok(())
    }

    fn name_by_form(
        stage: &mut Stage,
        form: &PositionalForm,
        count: usize,
    ) -> Result<(), PrintError> {
        let surplus = count.saturating_sub(form.parameters.len());
        let repeated = form.repeated.filter(|_| surplus > 0);
        let mut grouped = Vec::new();
        let mut group_name = None;
        let mut position = 0;
        for (index, arg) in stage.args.iter_mut().enumerate() {
            if arg.name.is_some() {
                continue;
            }
            let slot = match repeated {
                Some(start) if position > start + surplus => position - surplus,
                Some(start) if position >= start => start,
                _ => position,
            };
            let name = form
                .parameters
                .get(slot)
                .ok_or_else(|| PrintError::UnnamedArgument {
                    command: stage.name.clone(),
                    index: position,
                    span: arg.span,
                })?;
            if repeated == Some(slot) {
                grouped.push(index);
                group_name.get_or_insert_with(|| name.clone());
            } else {
                arg.name = Some(name.clone());
            }
            position += 1;
        }
        if let Some(name) = group_name {
            group_arguments(stage, &grouped, name);
        }
        Ok(())
    }

    fn expr(&mut self, expr: &mut Expr, depth: usize) -> Result<(), PrintError> {
        if depth > MAX_NESTING_DEPTH {
            return Err(PrintError::TooDeep {
                limit: MAX_NESTING_DEPTH,
            });
        }
        let next = depth + 1;
        match expr {
            Expr::Sub { p, .. } => self.pipeline(p, next),
            Expr::List { items, .. } | Expr::Call { args: items, .. } => {
                items.iter_mut().try_for_each(|item| self.expr(item, next))
            }
            Expr::Map { entries, .. } => entries
                .iter_mut()
                .try_for_each(|entry| self.expr(&mut entry.v, next)),
            Expr::Spread { x, .. } | Expr::Un { x, .. } => self.expr(x, next),
            Expr::Field { of: Some(of), .. } => self.expr(of, next),
            Expr::Bin { l, r, .. } => {
                self.expr(l, next)?;
                self.expr(r, next)
            }
            Expr::Key { segs, .. } => segs.iter_mut().try_for_each(|seg| match seg {
                KeySeg::Filter { filter } => self.expr(filter, next),
                _ => Ok(()),
            }),
            _ => Ok(()),
        }
    }
}
