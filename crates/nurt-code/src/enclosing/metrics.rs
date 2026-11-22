// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Size and complexity of functions and methods.
//!
//! Cyclomatic complexity is 1 plus the number of decision points in the
//! body: conditionals, loops, branches of `switch` and `match`, exception
//! handlers, conditional expressions and the operators `&&`, `||`, `and`
//! and `or`. Nesting counts conditionals, loops, `switch`, `match`, `try` and
//! `with` blocks inside one another. Functions declared inside a function are
//! measured on their own and do not count towards the outer function.

use nurt_contracts::services::{FunctionMetrics, SymbolKind};
use tree_sitter::{Node, Tree};

use super::{declaration_kind, declarations};

/// Node kinds that add a decision point.
const DECISION_KINDS: [&str; 33] = [
    "if_statement",
    "if_expression",
    "if_let_expression",
    "elif_clause",
    "else_if_clause",
    "for_statement",
    "for_in_statement",
    "for_expression",
    "foreach_statement",
    "enhanced_for_statement",
    "for_range_loop",
    "while_statement",
    "while_expression",
    "do_statement",
    "case_clause",
    "switch_case",
    "case_statement",
    "switch_section",
    "expression_case",
    "type_case",
    "communication_case",
    "match_arm",
    "when_entry",
    "catch_clause",
    "except_clause",
    "rescue",
    "conditional_expression",
    "ternary_expression",
    "guard_statement",
    "if",
    "unless",
    "while",
    "until",
];

/// Node kinds that open a nested block.
const NESTING_KINDS: [&str; 22] = [
    "if_statement",
    "if_expression",
    "if_let_expression",
    "for_statement",
    "for_in_statement",
    "for_expression",
    "foreach_statement",
    "enhanced_for_statement",
    "for_range_loop",
    "while_statement",
    "while_expression",
    "loop_expression",
    "do_statement",
    "switch_statement",
    "switch_expression",
    "match_expression",
    "try_statement",
    "try_expression",
    "with_statement",
    "if",
    "while",
    "case",
];

/// Operators that add a decision point.
const BOOLEAN_OPERATORS: [&str; 4] = ["&&", "||", "and", "or"];

/// Node kinds of parameter lists.
const PARAMETER_LIST_KINDS: [&str; 8] = [
    "parameters",
    "formal_parameters",
    "parameter_list",
    "function_value_parameters",
    "method_parameters",
    "lambda_parameters",
    "parameter_clause",
    "function_parameters",
];

/// Size and complexity of every function and method of a tree, in source
/// order.
#[must_use]
pub fn function_metrics(tree: &Tree, source: &[u8]) -> Vec<FunctionMetrics> {
    declarations(tree, source)
        .into_iter()
        .filter(|(info, _)| matches!(info.kind, SymbolKind::Function | SymbolKind::Method))
        .map(|(info, node)| {
            let body = node.child_by_field_name("body").unwrap_or(node);
            let mut counts = Counts::default();
            measure(body, 0, &mut counts);
            FunctionMetrics {
                lines: info.span.end.line.saturating_sub(info.span.start.line) + 1,
                parameters: parameter_count(node),
                cyclomatic: 1 + counts.decisions,
                max_nesting: counts.max_nesting,
                name: info.name,
                container: info.container,
                span: info.span,
            }
        })
        .collect()
}

#[derive(Debug, Default)]
struct Counts {
    decisions: u32,
    max_nesting: u32,
}

fn measure(node: Node<'_>, depth: u32, counts: &mut Counts) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        let kind = child.kind();
        if is_function(kind) {
            continue;
        }
        if DECISION_KINDS.contains(&kind) || is_boolean_operation(child) {
            counts.decisions += 1;
        }
        let nested = if NESTING_KINDS.contains(&kind) {
            let nested = depth + 1;
            counts.max_nesting = counts.max_nesting.max(nested);
            nested
        } else {
            depth
        };
        measure(child, nested, counts);
    }
}

fn is_function(kind: &str) -> bool {
    matches!(
        declaration_kind(kind),
        Some(SymbolKind::Function | SymbolKind::Method)
    )
}

fn is_boolean_operation(node: Node<'_>) -> bool {
    let kind = node.kind();
    if !(kind.contains("binary") || kind.contains("boolean") || kind.contains("logical")) {
        return false;
    }
    if let Some(operator) = node.child_by_field_name("operator") {
        return BOOLEAN_OPERATORS.contains(&operator.kind());
    }
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|child| !child.is_named() && BOOLEAN_OPERATORS.contains(&child.kind()));
    found
}

/// Number of parameters of a declaration: named nodes of its parameter
/// list other than comments.
fn parameter_count(node: Node<'_>) -> u32 {
    let Some(list) = parameter_list(node) else {
        return 0;
    };
    let mut cursor = list.walk();
    let count = list
        .named_children(&mut cursor)
        .filter(|child| !child.kind().contains("comment"))
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Parameter list of a declaration, outside its body.
fn parameter_list(node: Node<'_>) -> Option<Node<'_>> {
    if let Some(list) = node.child_by_field_name("parameters") {
        return Some(list);
    }
    let body = node.child_by_field_name("body").map(|body| body.id());
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    children
        .into_iter()
        .filter(|child| Some(child.id()) != body)
        .find_map(|child| {
            if PARAMETER_LIST_KINDS.contains(&child.kind()) {
                Some(child)
            } else if child.kind().contains("declarator") {
                parameter_list(child)
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_and_nesting_kinds_do_not_contain_declarations() {
        for kind in DECISION_KINDS.iter().chain(NESTING_KINDS.iter()) {
            assert!(declaration_kind(kind).is_none(), "{kind}");
        }
    }
}
