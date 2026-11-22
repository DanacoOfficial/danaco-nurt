// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Statements of a bash script read with `tree-sitter-bash`.

use tree_sitter::Node;

use super::{
    Analysis, Builder, Join, Part, Statement, contains_kind, find_construct, line_comment,
};
use crate::report::Reason;

/// Commands that change the current directory.
const DIRECTORY_COMMANDS: &[&str] = &["cd", "pushd"];

/// Commands that evaluate text as code.
const EVAL_COMMANDS: &[&str] = &["eval"];

/// Commands that run another script in the current shell.
const SOURCE_COMMANDS: &[&str] = &["source", "."];

pub(super) fn segment(builder: &mut Builder<'_>, root: Node<'_>) {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "comment" => {
                let lines = line_comment(super::node_text(builder.source(), child));
                builder.comment(child, lines);
            }
            ";" => builder.separator(),
            "&" => builder.background(child),
            _ if child.is_named() => {
                let analysis = analyze(builder, child);
                builder.statement(child, analysis);
            }
            _ => {}
        }
    }
}

fn analyze(builder: &Builder<'_>, node: Node<'_>) -> Analysis {
    if node.is_error() {
        return builder.invalid(node);
    }
    let source = builder.source();
    if let Some(reason) = find_construct(node, &|node| construct(source, node), &descend) {
        return Analysis::Construct(reason);
    }
    if let Some(invalid) = builder.syntax_error(node, &|_| false) {
        return invalid;
    }
    let mut statement = Statement {
        parts: Vec::new(),
        joins: Vec::new(),
    };
    flatten(builder, node, &mut statement);
    Analysis::Statement(statement)
}

fn construct(source: &str, node: Node<'_>) -> Option<Reason> {
    match node.kind() {
        "for_statement" | "c_style_for_statement" | "while_statement" | "select_statement" => {
            Some(Reason::Loop)
        }
        "if_statement" | "case_statement" => Some(Reason::Condition),
        "function_definition" => Some(Reason::Function),
        "subshell" | "compound_statement" => Some(Reason::Block),
        "command" => {
            let name = command_name(source, node)?;
            if EVAL_COMMANDS.contains(&name) {
                Some(Reason::Eval)
            } else if SOURCE_COMMANDS.contains(&name) {
                Some(Reason::Source)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Text of a here-document is data, not code.
fn descend(node: Node<'_>) -> bool {
    node.kind() != "heredoc_body"
}

fn flatten(builder: &Builder<'_>, node: Node<'_>, statement: &mut Statement) {
    // The grammar attaches a redirection written after a list to the whole
    // list; the shell applies it to the last command of the list.
    if node.kind() == "redirected_statement"
        && let Some(body) = node.child_by_field_name("body")
        && body.kind() == "list"
    {
        flatten(builder, body, statement);
        if let Some(last) = statement.parts.last_mut() {
            last.range.end = node.end_byte();
            last.here_document |= contains_kind(node, "heredoc_redirect");
        }
        return;
    }
    if node.kind() != "list" {
        statement.parts.push(part(builder, node));
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "&&" => statement.joins.push(Join::And),
            "||" => statement.joins.push(Join::Or),
            "comment" => {}
            _ if child.is_named() => flatten(builder, child, statement),
            _ => {}
        }
    }
}

fn part(builder: &Builder<'_>, node: Node<'_>) -> Part {
    let source = builder.source();
    let mut part = builder.part(node);
    part.here_document = contains_kind(node, "heredoc_redirect");
    match node.kind() {
        "variable_assignment" => part.variable = assigned_variable(source, node),
        "command" => part.directory = directory(source, node),
        _ => {}
    }
    part
}

fn assigned_variable(source: &str, node: Node<'_>) -> Option<String> {
    let mut cursor = node.walk();
    let name = node
        .children(&mut cursor)
        .find(|child| child.kind() == "variable_name")?;
    Some(super::node_text(source, name).to_owned())
}

fn command_name<'s>(source: &'s str, node: Node<'_>) -> Option<&'s str> {
    let name = node.child_by_field_name("name")?;
    Some(super::node_text(source, name))
}

/// Literal target of `cd D` or `pushd D`; `None` for other commands and for
/// targets built from expansions.
fn directory(source: &str, node: Node<'_>) -> Option<String> {
    let name = command_name(source, node)?;
    if !DIRECTORY_COMMANDS.contains(&name) {
        return None;
    }
    let mut cursor = node.walk();
    let mut targets = node
        .children_by_field_name("argument", &mut cursor)
        .filter(|argument| !super::node_text(source, *argument).starts_with('-'));
    let target = targets.next()?;
    if targets.next().is_some() {
        return None;
    }
    literal(source, target)
}

fn literal(source: &str, node: Node<'_>) -> Option<String> {
    let text = super::node_text(source, node);
    match node.kind() {
        "word" if !text.contains(['$', '*', '?', '[', '\\', '~']) => Some(text.to_owned()),
        "raw_string" => text
            .strip_prefix('\'')
            .and_then(|inner| inner.strip_suffix('\''))
            .map(str::to_owned),
        "string" if node.named_child_count() <= 1 && !text.contains(['$', '`', '\\']) => text
            .strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .map(str::to_owned),
        _ => None,
    }
}
