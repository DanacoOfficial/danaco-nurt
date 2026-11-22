// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Statements of a PowerShell script read with `tree-sitter-powershell`.

use tree_sitter::Node;

use super::{Analysis, Builder, Join, Statement, find_construct, line_comment, node_text};
use crate::report::Reason;

/// Commands and aliases that change the current location, in lower case.
const DIRECTORY_COMMANDS: &[&str] = &[
    "cd",
    "chdir",
    "sl",
    "set-location",
    "pushd",
    "push-location",
];

/// Parameters of the location commands that name the target, in lower case.
const DIRECTORY_PARAMETERS: &[&str] = &["-path", "-literalpath"];

/// Commands and aliases that evaluate text as code, in lower case.
const EVAL_COMMANDS: &[&str] = &["invoke-expression", "iex"];

/// Invocation operator that runs a script in the current scope.
const DOT_SOURCE: &str = ".";

pub(super) fn segment(builder: &mut Builder<'_>, root: Node<'_>) {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "comment" => comment(builder, child),
            "statement_list" => statements(builder, child),
            "param_block" => builder.statement(child, Analysis::Construct(Reason::Parameters)),
            "ERROR" => {
                let analysis = builder.invalid(child);
                builder.statement(child, analysis);
            }
            _ => {}
        }
    }
}

fn statements(builder: &mut Builder<'_>, list: Node<'_>) {
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        match child.kind() {
            "comment" => comment(builder, child),
            "empty_statement" => builder.separator(),
            "background_operator" => builder.background(child),
            _ if child.is_named() => {
                let analysis = analyze(builder, child);
                builder.statement(child, analysis);
            }
            _ => {}
        }
    }
}

fn comment(builder: &mut Builder<'_>, node: Node<'_>) {
    let text = node_text(builder.source(), node);
    let lines = match text
        .strip_prefix("<#")
        .and_then(|inner| inner.strip_suffix("#>"))
    {
        Some(inner) => block_comment(inner),
        None => line_comment(text),
    };
    builder.comment(node, lines);
}

/// Converts the text of a block comment `<# ... #>` to `#` comment lines.
fn block_comment(inner: &str) -> Vec<String> {
    let lines: Vec<&str> = inner.lines().map(str::trim).collect();
    let first = lines.iter().position(|line| !line.is_empty());
    let last = lines.iter().rposition(|line| !line.is_empty());
    let (Some(first), Some(last)) = (first, last) else {
        return vec!["#".to_owned()];
    };
    lines
        .get(first..=last)
        .unwrap_or_default()
        .iter()
        .map(|line| {
            if line.is_empty() {
                "#".to_owned()
            } else {
                format!("# {line}")
            }
        })
        .collect()
}

fn analyze(builder: &Builder<'_>, node: Node<'_>) -> Analysis {
    if node.is_error() {
        return builder.invalid(node);
    }
    let source = builder.source();
    if let Some(reason) = find_construct(node, &|node| construct(source, node), &descend) {
        return Analysis::Construct(reason);
    }
    if let Some(invalid) = builder.syntax_error(node, &|error| multiplier_suffix(source, error)) {
        return invalid;
    }
    if node.kind() != "pipeline" {
        return builder.invalid(node);
    }
    let mut statement = Statement {
        parts: Vec::new(),
        joins: Vec::new(),
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "assignment_expression" => {
                let mut part = builder.part(child);
                part.variable = assigned_variable(source, child);
                statement.parts.push(part);
            }
            "pipeline_chain" => {
                let mut part = builder.part(child);
                part.directory = directory(source, child);
                statement.parts.push(part);
            }
            "pipeline_chain_tail" => statement.joins.push(match node_text(source, child) {
                "||" => Join::Or,
                _ => Join::And,
            }),
            _ => {}
        }
    }
    Analysis::Statement(statement)
}

fn construct(source: &str, node: Node<'_>) -> Option<Reason> {
    match node.kind() {
        "foreach_statement" | "for_statement" | "while_statement" | "do_statement" => {
            Some(Reason::Loop)
        }
        "if_statement" | "switch_statement" => Some(Reason::Condition),
        "function_statement" | "class_statement" | "enum_statement" => Some(Reason::Function),
        "try_statement"
        | "trap_statement"
        | "data_statement"
        | "parallel_statement"
        | "sequence_statement"
        | "inlinescript_statement" => Some(Reason::Block),
        "flow_control_statement" | "label" => Some(Reason::Control),
        "command" => command_construct(source, node),
        _ => None,
    }
}

fn command_construct(source: &str, node: Node<'_>) -> Option<Reason> {
    let mut cursor = node.walk();
    let operator = node
        .children(&mut cursor)
        .find(|child| child.kind() == "command_invokation_operator");
    if operator.is_some_and(|operator| node_text(source, operator) == DOT_SOURCE) {
        return Some(Reason::Source);
    }
    let name = command_name(source, node)?;
    EVAL_COMMANDS
        .contains(&name.as_str())
        .then_some(Reason::Eval)
}

/// Returns `true` for an error node that is the multiplier of a numeric
/// literal (`1MB`, `10kb`), which the grammar does not read; the habit tables
/// read the statement with their own lexicon.
fn multiplier_suffix(source: &str, error: Node<'_>) -> bool {
    let text = node_text(source, error).to_ascii_lowercase();
    let follows_digit = source
        .get(..error.start_byte())
        .and_then(|before| before.chars().next_back())
        .is_some_and(|previous| previous.is_ascii_digit());
    follows_digit && MULTIPLIERS.contains(&text.as_str())
}

/// Multipliers of PowerShell numeric literals, in lower case.
const MULTIPLIERS: &[&str] = &["kb", "mb", "gb", "tb", "pb"];

/// Script blocks passed to cmdlets (`Where-Object { ... }`) are arguments
/// translated by the habit tables, not statements of the script.
fn descend(node: Node<'_>) -> bool {
    node.kind() != "script_block_expression"
}

fn command_name(source: &str, node: Node<'_>) -> Option<String> {
    let name = node.child_by_field_name("command_name")?;
    Some(node_text(source, name).to_ascii_lowercase())
}

fn assigned_variable(source: &str, node: Node<'_>) -> Option<String> {
    let mut cursor = node.walk();
    let left = node
        .children(&mut cursor)
        .find(|child| child.kind() == "left_assignment_expression")?;
    let name = node_text(source, left).strip_prefix('$')?;
    let simple = name.starts_with(|first: char| first.is_ascii_alphabetic() || first == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    simple.then(|| name.to_owned())
}

/// Literal target of a location command (`cd D`, `Set-Location -Path D`).
fn directory(source: &str, chain: Node<'_>) -> Option<String> {
    let mut cursor = chain.walk();
    let mut commands = chain
        .children(&mut cursor)
        .filter(|child| child.kind() == "command");
    let command = commands.next()?;
    if commands.next().is_some() {
        return None;
    }
    let name = command_name(source, command)?;
    if !DIRECTORY_COMMANDS.contains(&name.as_str()) {
        return None;
    }
    let elements = command.child_by_field_name("command_elements")?;
    let mut cursor = elements.walk();
    let mut values = Vec::new();
    for element in elements.named_children(&mut cursor) {
        match element.kind() {
            "command_argument_sep" => {}
            "command_parameter" => {
                let parameter = node_text(source, element).to_ascii_lowercase();
                if !DIRECTORY_PARAMETERS.contains(&parameter.as_str()) {
                    return None;
                }
            }
            _ => values.push(element),
        }
    }
    match values.as_slice() {
        [value] => literal(source, *value),
        _ => None,
    }
}

fn literal(source: &str, node: Node<'_>) -> Option<String> {
    let text = node_text(source, node);
    if text.contains(['$', '`', '*', '?']) {
        return None;
    }
    let unquoted = text
        .strip_prefix('\'')
        .and_then(|inner| inner.strip_suffix('\''))
        .or_else(|| {
            text.strip_prefix('"')
                .and_then(|inner| inner.strip_suffix('"'))
        });
    match (node.kind(), unquoted) {
        ("generic_token", None) => Some(text.to_owned()),
        (_, Some(inner)) => Some(inner.to_owned()),
        _ => None,
    }
}
