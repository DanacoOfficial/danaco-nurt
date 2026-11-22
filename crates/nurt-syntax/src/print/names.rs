// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Naming of positional arguments, the prerequisite of printing in variant C.

use nurt_contracts::ast::{AstDocument, Expr, Head, KeySeg, Pipeline, Stage, Statement};

use super::{MAX_NESTING_DEPTH, PrintError};

/// Source of parameter names for positional arguments.
///
/// The command catalog implements this trait: the `index`-th positional
/// parameter of a command has the name returned for it.
///
/// # Examples
///
/// ```
/// use nurt_syntax::print::PositionalNames;
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
/// }
///
/// assert_eq!(Catalog.name("find", 0).as_deref(), Some("path"));
/// assert_eq!(Catalog.name("find", 1), None);
/// ```
pub trait PositionalNames {
    /// Returns the name of the positional parameter `index` (zero-based, in
    /// the order of the positional parameters) of `command`, or `None` when
    /// the command has no such parameter.
    fn name(&self, command: &str, index: usize) -> Option<String>;
}

/// Returns a copy of `document` in which every positional argument carries
/// the name of its parameter.
///
/// Positional arguments of a stage are counted from zero in source order,
/// skipping named arguments; the `k`-th one receives
/// `names.name(stage.name, k)`. Stages inside sub-pipelines are named as
/// well. Spans and metadata are kept. The result prints in variant C.
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
    for statement in &mut named.program.statements {
        match statement {
            Statement::Let(binding) => namer.pipeline(&mut binding.value, 1)?,
            Statement::Pipeline(pipeline) => namer.pipeline(pipeline, 1)?,
            _ => {}
        }
    }
    Ok(named)
}

struct Namer<'a> {
    names: &'a dyn PositionalNames,
}

impl Namer<'_> {
    fn pipeline(&mut self, pipeline: &mut Pipeline, depth: usize) -> Result<(), PrintError> {
        if let Head::Stage(stage) = &mut pipeline.head {
            self.stage(stage, depth)?;
        }
        for stage in &mut pipeline.stages {
            self.stage(stage, depth)?;
        }
        Ok(())
    }

    fn stage(&mut self, stage: &mut Stage, depth: usize) -> Result<(), PrintError> {
        let mut position = 0;
        for arg in &mut stage.args {
            if arg.name.is_none() {
                let name = self.names.name(&stage.name, position).ok_or_else(|| {
                    PrintError::UnnamedArgument {
                        command: stage.name.clone(),
                        index: position,
                        span: arg.span,
                    }
                })?;
                arg.name = Some(name);
                position += 1;
            }
            self.expr(&mut arg.value, depth + 1)?;
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
