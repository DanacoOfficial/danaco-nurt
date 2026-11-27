// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::local::rules
// Responsibility:   Grammar rules of local analysis shared by the built-in languages.
// Dependencies:     nurt_contracts, tree_sitter
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Grammar rules of local analysis shared by the built-in languages.
//!
//! The rules rely on the naming conventions of tree-sitter grammars:
//! identifier leaves have a kind ending in `identifier` (PHP uses `name`,
//! Ruby `constant` for capitalized names), and a name after a member
//! operator is the field `field`, `attribute`, `property` or `suffix` of its
//! parent, or the field `name` or `method` of a parent that also has a
//! qualifier (`object`, `receiver`, `scope`, `path`).

use tree_sitter::Node;

/// Identifier kinds that are always the name of a member.
const MEMBER_KINDS: &[&str] = &[
    "field_identifier",
    "property_identifier",
    "private_property_identifier",
];

/// Fields that hold the member name of a member access.
const MEMBER_FIELDS: &[&str] = &["field", "attribute", "property", "suffix"];

/// Fields that hold a name qualified by another field of the same parent.
const QUALIFIED_FIELDS: &[&str] = &["name", "method"];

/// Fields that qualify a name.
const QUALIFIER_FIELDS: &[&str] = &[
    "object",
    "receiver",
    "scope",
    "path",
    "expression",
    "namespace",
    "value",
    "operand",
];

/// Parents whose name child labels an argument and refers to nothing.
const LABEL_PARENTS: &[&str] = &[
    "keyword_argument",
    "named_argument",
    "value_argument_label",
    "navigation_suffix",
];

/// Identifier leaves other than kinds ending in `identifier`.
const OTHER_IDENTIFIERS: &[&str] = &["name", "constant"];

/// Call node kinds of the built-in grammars.
const CALL_KINDS: &[&str] = &[
    "call_expression",
    "call",
    "method_invocation",
    "invocation_expression",
    "function_call_expression",
    "member_call_expression",
    "nullsafe_member_call_expression",
    "scoped_call_expression",
    "macro_invocation",
];

/// Fields that hold the callee of a call node, in order of preference.
const CALLEE_FIELDS: &[&str] = &["function", "name", "method", "macro"];

/// Returns `true` for an identifier leaf.
pub(crate) fn is_identifier(node: Node<'_>) -> bool {
    if node.child_count() > 0 || !node.is_named() {
        return false;
    }
    let kind = node.kind();
    kind.ends_with("identifier") || OTHER_IDENTIFIERS.contains(&kind)
}

/// Returns `true` when an identifier names a member of a value or a
/// qualified path rather than a variable in scope; `field` is the field of
/// the identifier in its `parent`. The parent is passed in because
/// `Node::parent` searches from the root on every call.
pub(crate) fn is_member(node: Node<'_>, field: Option<&str>, parent: Option<Node<'_>>) -> bool {
    if MEMBER_KINDS.contains(&node.kind()) {
        return true;
    }
    let Some(parent) = parent else {
        return false;
    };
    if LABEL_PARENTS.contains(&parent.kind()) {
        return true;
    }
    match field {
        Some(field) if MEMBER_FIELDS.contains(&field) => true,
        Some(field) if QUALIFIED_FIELDS.contains(&field) => has_qualifier(parent, node),
        _ => false,
    }
}

fn has_qualifier(parent: Node<'_>, node: Node<'_>) -> bool {
    QUALIFIER_FIELDS.iter().any(|field| {
        parent
            .child_by_field_name(field)
            .is_some_and(|qualifier| qualifier.id() != node.id())
    })
}

/// Returns `true` for a call node.
pub(crate) fn is_call(node: Node<'_>) -> bool {
    CALL_KINDS.contains(&node.kind())
}

/// The identifier naming the function called by a call node: the last
/// identifier of the callee expression (`run` in `self.worker.run()`).
pub(crate) fn callee_name(node: Node<'_>) -> Option<Node<'_>> {
    let callee = CALLEE_FIELDS
        .iter()
        .find_map(|field| node.child_by_field_name(field))
        .or_else(|| node.named_child(0))?;
    last_identifier(callee)
}

/// The identifier that ends the subtree of `node`, found depth first from the
/// last child with an explicit stack, so that the depth of the tree is not
/// limited by the stack of the thread. Argument lists are not searched.
fn last_identifier(node: Node<'_>) -> Option<Node<'_>> {
    let mut pending = vec![node];
    while let Some(current) = pending.pop() {
        if is_identifier(current) {
            return Some(current);
        }
        let mut cursor = current.walk();
        let children: Vec<Node<'_>> = current
            .named_children(&mut cursor)
            .filter(|child| !child.kind().contains("argument"))
            .collect();
        pending.extend(children);
    }
    None
}

/// Declaration statements whose names become visible after the statement,
/// so that the initializer of `let x = x + 1` refers to an earlier `x`.
const SEQUENTIAL_DECLARATIONS: &[&str] = &[
    "let_declaration",
    "assignment",
    "augmented_assignment",
    "lexical_declaration",
    "variable_declaration",
    "short_var_declaration",
    "var_declaration",
    "local_variable_declaration",
    "local_declaration_statement",
    "property_declaration",
    "assignment_expression",
];

/// Kind fragments of the nodes that end the search for a sequential
/// declaration: a name defined inside them is visible within them.
const DECLARATION_BOUNDARIES: &[&str] = &[
    "parameter",
    "function",
    "lambda",
    "closure",
    "arrow",
    "block",
    "body",
    "for_",
    "catch",
];

/// The end byte of the declaration statement around a local definition,
/// or `None` when the name is visible from its own position (parameters,
/// loop variables, names of other declarations).
///
/// `ancestors` are the nodes from the root to the parent of `node`.
pub(crate) fn sequential_declaration_end(ancestors: &[Node<'_>]) -> Option<usize> {
    for parent in ancestors.iter().rev() {
        let kind = parent.kind();
        if SEQUENTIAL_DECLARATIONS.contains(&kind) {
            return Some(parent.end_byte());
        }
        if DECLARATION_BOUNDARIES
            .iter()
            .any(|fragment| kind.contains(fragment))
        {
            return None;
        }
    }
    None
}

/// Returns `true` when `character` may appear in an identifier.
pub(crate) fn is_identifier_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

#[cfg(test)]
mod tests {
    use nurt_contracts::services::LanguageId;

    use super::*;
    use crate::registry::Registry;

    fn first_call_callee(language: &str, source: &str) -> Option<String> {
        let parsed = Registry::builtin()
            .parse(&LanguageId::new(language).unwrap(), source.as_bytes())
            .unwrap();
        let mut stack = vec![parsed.tree.root_node()];
        while let Some(node) = stack.pop() {
            if is_call(node) {
                return callee_name(node).map(|name| parsed.text(name).into_owned());
            }
            let mut cursor = node.walk();
            let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
            stack.extend(children.into_iter().rev());
        }
        None
    }

    #[test]
    fn callee_is_the_last_identifier_of_the_function_expression() {
        assert_eq!(
            first_call_callee("python", "self.worker.run(1)\n").as_deref(),
            Some("run")
        );
        assert_eq!(
            first_call_callee("go", "package p\nfunc f() { fmt.Println(x) }\n").as_deref(),
            Some("Println")
        );
        assert_eq!(
            first_call_callee("java", "class A { void f() { list.add(item); } }\n").as_deref(),
            Some("add")
        );
        assert_eq!(
            first_call_callee("rust", "fn f() { println!(\"x\"); }\n").as_deref(),
            Some("println")
        );
    }

    #[test]
    fn callee_of_a_deeply_chained_expression_is_found_without_recursion() {
        let source = format!("a{}();\n", "[0]".repeat(100_000));
        assert_eq!(
            first_call_callee("javascript", &source).as_deref(),
            Some("a")
        );
    }

    #[test]
    fn arguments_are_not_searched_for_the_callee() {
        assert_eq!(
            first_call_callee("javascript", "f(g)(h);\n").as_deref(),
            Some("f")
        );
        assert_eq!(
            first_call_callee("python", "obj.method(arg)\n").as_deref(),
            Some("method")
        );
    }

    fn declaration_end_of(language: &str, source: &str, name: &str) -> Option<usize> {
        let parsed = Registry::builtin()
            .parse(&LanguageId::new(language).unwrap(), source.as_bytes())
            .unwrap();
        let mut stack = vec![parsed.tree.root_node()];
        while let Some(node) = stack.pop() {
            if is_identifier(node) && parsed.text(node) == name {
                let mut chain = Vec::new();
                let mut up = node.parent();
                while let Some(parent) = up {
                    chain.push(parent);
                    up = parent.parent();
                }
                chain.reverse();
                return sequential_declaration_end(&chain);
            }
            let mut cursor = node.walk();
            let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
            stack.extend(children.into_iter().rev());
        }
        None
    }

    #[test]
    fn declared_names_are_visible_after_their_statement() {
        let rust = "fn f() { let total = 1; }\n";
        assert_eq!(
            declaration_end_of("rust", rust, "total"),
            Some(rust.find(';').unwrap() + 1)
        );
        let python = "count = 1\n";
        assert_eq!(declaration_end_of("python", python, "count"), Some(9));
    }

    #[test]
    fn parameters_and_loop_variables_are_visible_from_their_position() {
        assert_eq!(
            declaration_end_of("rust", "fn f() { let g = |item| item; }\n", "item"),
            None
        );
        assert_eq!(
            declaration_end_of("python", "for entry in rows:\n    pass\n", "entry"),
            None
        );
    }

    #[test]
    fn identifier_characters_include_dollar_and_underscore() {
        assert!(is_identifier_char('_'));
        assert!(is_identifier_char('$'));
        assert!(is_identifier_char('é'));
        assert!(!is_identifier_char('.'));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
