// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::lexical::tree
// Responsibility:   Rules applied to the parsed tree: `L14` and `L18`.
// Dependencies:     nurt_contracts, nurt_syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Rules applied to the parsed tree: `L14` and `L18`.
//!
//! The grammar accepts a positional argument after a named one and a command
//! name as a bare value, so these notations parse. [`front()`](super::front::front)
//! applies the rules to programs that needed other corrections; the binder
//! applies them to any program whose binding fails.

use nurt_contracts::ast::{
    Arg, AstDocument, Expr, Head, Pipeline, Program, Span, Stage, Statement,
};
use nurt_syntax::parse::Variant;
use nurt_syntax::print::try_print;

use super::{ArgumentShape, CatalogView};

/// The canonical pipe operator.
const PIPE: &str = "|>";

/// Kind of a change made to a tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TreeRule {
    /// Rule `L14`: a stage was split before a command name.
    MissingPipe,
    /// Rule `L18`: positional arguments were moved before named ones.
    ArgumentOrder,
}

/// A change made to a tree, with the place of the changed stage or name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeChange {
    /// Rule of the change.
    pub rule: TreeRule,
    /// Span of the changed stage (`L18`) or of the command name (`L14`), in
    /// the text the tree was parsed from.
    pub span: Option<Span>,
    /// Notation before the change.
    pub from: String,
    /// Notation after the change.
    pub to: String,
}

/// Moves positional arguments written after named arguments before them,
/// keeping the relative order of both groups (`L18`, class C).
///
/// The binder matches named arguments by name, so the move keeps the
/// meaning of every stage. Spans of the arguments are kept.
///
/// # Examples
///
/// ```
/// use nurt_correct::lexical::tree::reorder_arguments;
/// use nurt_syntax::parse::{ParseOptions, Variant, parse};
/// use nurt_syntax::print::print;
///
/// let mut document = parse("find glob: \"*.go\" \"/srv/app\"", Variant::A, &ParseOptions::default())?;
/// let changes = reorder_arguments(&mut document, Variant::A);
/// assert_eq!(changes.len(), 1);
/// assert_eq!(print(&document, Variant::A).trim_end(), "find \"/srv/app\" glob: \"*.go\"");
/// # Ok::<(), nurt_syntax::error::SyntaxError>(())
/// ```
pub fn reorder_arguments(document: &mut AstDocument, variant: Variant) -> Vec<TreeChange> {
    let mut changes = Vec::new();
    for_each_stage(&mut document.program, &mut |stage| {
        reorder_stage(stage, variant, &mut changes);
    });
    changes
}

/// Splits a stage before a bare command name that the command of the stage
/// cannot take as a positional argument (`L14`, class S).
///
/// `find "/a" glob: "*.go" search /x/` becomes
/// `find "/a" glob: "*.go" |> search /x/`. Arguments after the name become
/// the arguments of the new stage.
///
/// # Examples
///
/// ```
/// use nurt_correct::lexical::tree::split_missing_pipes;
/// use nurt_correct::lexical::{ArgumentShape, CatalogView};
/// use nurt_syntax::parse::{ParseOptions, Variant, parse};
/// use nurt_syntax::print::print;
///
/// struct Commands;
///
/// impl CatalogView for Commands {
///     fn is_command(&self, name: &str) -> bool {
///         ["find", "search"].contains(&name)
///     }
///     fn command_names(&self) -> &[&str] {
///         &["find", "search"]
///     }
///     fn is_mutating(&self, _: &str) -> bool {
///         false
///     }
///     fn can_take_argument(&self, _: &str, _: usize, _: ArgumentShape) -> bool {
///         false
///     }
/// }
///
/// let text = "find \"/a\" glob: \"*.go\" search /x/";
/// let mut document = parse(text, Variant::A, &ParseOptions::default())?;
/// let changes = split_missing_pipes(&mut document, &Commands);
/// assert_eq!(changes.len(), 1);
/// assert_eq!(
///     print(&document, Variant::A).trim_end(),
///     "find \"/a\" glob: \"*.go\"\n|> search /x/"
/// );
/// # Ok::<(), nurt_syntax::error::SyntaxError>(())
/// ```
pub fn split_missing_pipes(
    document: &mut AstDocument,
    catalog: &dyn CatalogView,
) -> Vec<TreeChange> {
    let mut changes = Vec::new();
    for statement in &mut document.program.statements {
        if let Some(pipeline) = statement_pipeline(statement) {
            split_with_sub_pipelines(pipeline, catalog, &mut changes);
        }
    }
    changes
}

/// Splits the sub-pipelines in the arguments of the stages, then the
/// pipeline itself, so that `L14` applies at every depth as `L18` does.
fn split_with_sub_pipelines(
    pipeline: &mut Pipeline,
    catalog: &dyn CatalogView,
    changes: &mut Vec<TreeChange>,
) {
    let head = match &mut pipeline.head {
        Head::Stage(stage) => Some(stage),
        _ => None,
    };
    for stage in head.into_iter().chain(pipeline.stages.iter_mut()) {
        for arg in &mut stage.args {
            if let Expr::Sub { p, .. } = &mut arg.value {
                split_with_sub_pipelines(p, catalog, changes);
            }
        }
    }
    split_pipeline(pipeline, catalog, changes);
}

fn statement_pipeline(statement: &mut Statement) -> Option<&mut Pipeline> {
    match statement {
        Statement::Let(binding) => Some(&mut binding.value),
        Statement::Pipeline(pipeline) => Some(pipeline),
        _ => None,
    }
}

/// Calls `visit` on every stage of the program, sub-pipelines included.
fn for_each_stage(program: &mut Program, visit: &mut dyn FnMut(&mut Stage)) {
    for statement in &mut program.statements {
        if let Some(pipeline) = statement_pipeline(statement) {
            visit_pipeline(pipeline, visit);
        }
    }
}

fn visit_pipeline(pipeline: &mut Pipeline, visit: &mut dyn FnMut(&mut Stage)) {
    if let Head::Stage(stage) = &mut pipeline.head {
        visit_stage(stage, visit);
    }
    for stage in &mut pipeline.stages {
        visit_stage(stage, visit);
    }
}

fn visit_stage(stage: &mut Stage, visit: &mut dyn FnMut(&mut Stage)) {
    for arg in &mut stage.args {
        if let Expr::Sub { p, .. } = &mut arg.value {
            visit_pipeline(p, visit);
        }
    }
    visit(stage);
}

fn reorder_stage(stage: &mut Stage, variant: Variant, changes: &mut Vec<TreeChange>) {
    let first_named = stage.args.iter().position(|arg| arg.name.is_some());
    let misplaced = first_named.is_some_and(|first| {
        stage
            .args
            .get(first..)
            .unwrap_or_default()
            .iter()
            .any(|arg| arg.name.is_none())
    });
    if !misplaced {
        return;
    }
    let from = stage_text(stage, variant);
    let (positional, named): (Vec<Arg>, Vec<Arg>) =
        stage.args.drain(..).partition(|arg| arg.name.is_none());
    stage.args = positional.into_iter().chain(named).collect();
    changes.push(TreeChange {
        rule: TreeRule::ArgumentOrder,
        span: stage.span,
        from,
        to: stage_text(stage, variant),
    });
}

fn split_pipeline(
    pipeline: &mut Pipeline,
    catalog: &dyn CatalogView,
    changes: &mut Vec<TreeChange>,
) {
    let head_is_stage = matches!(pipeline.head, Head::Stage(_));
    let mut stages: Vec<Stage> = Vec::with_capacity(pipeline.stages.len() + 1);
    if let Head::Stage(stage) = &pipeline.head {
        stages.push(stage.clone());
    }
    stages.append(&mut pipeline.stages);
    let mut index = 0;
    while index < stages.len() {
        if let Some(split) = stages
            .get_mut(index)
            .and_then(|stage| split_stage(stage, catalog))
        {
            changes.push(TreeChange {
                rule: TreeRule::MissingPipe,
                span: split.span,
                from: split.name.clone(),
                to: format!("{PIPE} {}", split.name),
            });
            stages.insert(index + 1, split);
        }
        index += 1;
    }
    if head_is_stage && !stages.is_empty() {
        let head = stages.remove(0);
        pipeline.head = Head::Stage(head);
    }
    pipeline.stages = stages;
}

/// Removes from `stage` the first bare command name that it cannot take
/// and the arguments after it; returns them as a new stage.
fn split_stage(stage: &mut Stage, catalog: &dyn CatalogView) -> Option<Stage> {
    let mut position = 0;
    let mut found = None;
    for (index, arg) in stage.args.iter().enumerate() {
        if arg.name.is_some() {
            continue;
        }
        if let Expr::Ident { v, .. } = &arg.value {
            let splits = index > 0
                && catalog.is_command(v)
                && !catalog.can_take_argument(&stage.name, position, ArgumentShape::Word);
            if splits {
                found = Some(index);
                break;
            }
        }
        position += 1;
    }
    let index = found?;
    let rest = stage.args.split_off(index + 1);
    let name_arg = stage.args.pop()?;
    let Expr::Ident { v, span } = name_arg.value else {
        return None;
    };
    Some(Stage {
        name: v,
        args: rest,
        span,
    })
}

/// Text of one stage in the variant, without the line end.
fn stage_text(stage: &Stage, variant: Variant) -> String {
    let document = AstDocument::new(Program {
        statements: vec![Statement::Pipeline(Pipeline {
            head: Head::Stage(stage.clone()),
            stages: Vec::new(),
            span: None,
        })],
    });
    try_print(&document, variant)
        .map(|text| text.trim_end().to_owned())
        .unwrap_or_else(|_| stage.name.clone())
}

#[cfg(test)]
mod tests {
    use nurt_syntax::parse::{ParseOptions, parse};
    use nurt_syntax::print::print;

    use super::*;

    struct Commands;

    impl CatalogView for Commands {
        fn is_command(&self, name: &str) -> bool {
            self.command_names().contains(&name)
        }
        fn command_names(&self) -> &[&str] {
            &["find", "search", "count", "sort", "verify", "run"]
        }
        fn is_mutating(&self, _: &str) -> bool {
            false
        }
        fn can_take_argument(&self, command: &str, _: usize, _: ArgumentShape) -> bool {
            command == "sort"
        }
    }

    fn parsed(text: &str) -> AstDocument {
        parse(text, Variant::A, &ParseOptions::default()).expect("valid program")
    }

    fn printed(document: &AstDocument) -> String {
        print(document, Variant::A).trim_end().to_owned()
    }

    #[test]
    fn splits_a_stage_inside_a_sub_pipeline() {
        let mut document = parsed("verify (find \"/a\" glob: \"*.go\" search /x/)");
        let changes = split_missing_pipes(&mut document, &Commands);
        assert_eq!(changes.len(), 1, "{}", printed(&document));
        assert_eq!(changes[0].rule, TreeRule::MissingPipe);
        assert!(
            printed(&document).contains("|> search /x/"),
            "{}",
            printed(&document)
        );
    }

    #[test]
    fn moves_positional_arguments_before_named_ones() {
        let mut document = parsed("find glob: \"*.go\" \"/srv/app\" hidden: true");
        let changes = reorder_arguments(&mut document, Variant::A);
        assert_eq!(
            printed(&document),
            "find \"/srv/app\" glob: \"*.go\" hidden: true"
        );
        assert_eq!(changes[0].rule, TreeRule::ArgumentOrder);
        assert_eq!(
            changes[0].from,
            "find glob: \"*.go\" \"/srv/app\" hidden: true"
        );
    }

    #[test]
    fn keeps_arguments_already_in_order() {
        let mut document = parsed("find \"/srv\" glob: \"*.go\" |> first 5");
        assert!(reorder_arguments(&mut document, Variant::A).is_empty());
    }

    #[test]
    fn reorders_arguments_inside_sub_pipelines() {
        let mut document = parsed("verify (run cwd: \"/a\" [\"go\", \"test\"])");
        let changes = reorder_arguments(&mut document, Variant::A);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            printed(&document),
            "verify (run [\"go\", \"test\"] cwd: \"/a\")"
        );
    }

    #[test]
    fn splits_a_stage_before_a_command_name() {
        let mut document = parsed("find \"/a\" search /x/ |> count");
        let changes = split_missing_pipes(&mut document, &Commands);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].to, "|> search");
        assert_eq!(printed(&document), "find \"/a\"\n|> search /x/\n|> count");
    }

    #[test]
    fn splits_a_stage_inside_a_let_statement() {
        let mut document = parsed("let r = find \"/a\" search /x/");
        let changes = split_missing_pipes(&mut document, &Commands);
        assert_eq!(changes.len(), 1);
        let Some(Statement::Let(binding)) = document.program.statements.first() else {
            panic!("a let statement is expected");
        };
        assert_eq!(binding.value.stages.len(), 1);
        assert_eq!(binding.value.stages[0].name, "search");
    }

    #[test]
    fn does_not_split_before_the_first_argument() {
        let mut document = parsed("find search");
        assert!(split_missing_pipes(&mut document, &Commands).is_empty());
    }

    #[test]
    fn keeps_command_names_that_the_stage_takes_as_values() {
        let mut document = parsed("find \"/a\" |> sort count");
        assert!(split_missing_pipes(&mut document, &Commands).is_empty());
        let mut first = parsed("count");
        assert!(split_missing_pipes(&mut first, &Commands).is_empty());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
