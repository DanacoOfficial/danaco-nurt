// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::enclosing
// Responsibility:   Enclosing declarations of source positions.
// Dependencies:     metrics, nurt_contracts, tree_sitter
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Enclosing declarations of source positions.
//!
//! A declaration is a syntax node that defines a named code element: a
//! function, method, class, structure, interface, enumeration, module,
//! type or macro. Declarations are recognized by the node kinds of the
//! grammars, so the same rules serve every language of the registry. The
//! enclosing declaration of a position is the innermost declaration whose
//! node contains it; its container is the chain of the outer declarations.
//!
//! ```
//! use nurt_code::enclosing::enclosing;
//! use nurt_contracts::services::SymbolKind;
//!
//! let source = b"struct S;\nimpl S {\n    fn run(&self) {\n        go();\n    }\n}\n";
//! let mut parser = tree_sitter::Parser::new();
//! parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
//! let tree = parser.parse(source, None).ok_or("no tree")?;
//! let offset = source.windows(4).position(|w| w == b"go()").ok_or("missing")?;
//! let symbol = enclosing(&tree, source, offset).ok_or("no symbol")?;
//! assert_eq!(symbol.name, "run");
//! assert_eq!(symbol.kind, SymbolKind::Method);
//! assert_eq!(symbol.container.as_deref(), Some("S"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod metrics;

pub use metrics::function_metrics;

use nurt_contracts::services::{SymbolInfo, SymbolKind};
use tree_sitter::{Node, Tree};

use crate::spans::LineIndex;

/// Kind of declaration of a node kind; `None` for other nodes.
fn declaration_kind(kind: &str) -> Option<SymbolKind> {
    let symbol = match kind {
        "function_definition"
        | "function_declaration"
        | "function_item"
        | "generator_function_declaration"
        | "function_signature_item"
        | "local_function_statement"
        | "preproc_function_def" => SymbolKind::Function,
        "method_definition"
        | "method_declaration"
        | "method"
        | "singleton_method"
        | "constructor_declaration"
        | "secondary_constructor"
        | "init_declaration" => SymbolKind::Method,
        "class_definition" | "class_declaration" | "class" | "class_specifier"
        | "object_declaration" | "record_declaration" | "singleton_class" => SymbolKind::Class,
        "struct_item" | "struct_specifier" | "struct_declaration" | "union_item"
        | "union_specifier" => SymbolKind::Struct,
        "interface_declaration" | "trait_item" | "trait_declaration" | "protocol_declaration" => {
            SymbolKind::Interface
        }
        "enum_item" | "enum_declaration" | "enum_specifier" => SymbolKind::Enum,
        "mod_item"
        | "module"
        | "namespace_definition"
        | "namespace_declaration"
        | "file_scoped_namespace_declaration" => SymbolKind::Module,
        "type_item"
        | "type_alias_declaration"
        | "type_spec"
        | "type_alias"
        | "impl_item"
        | "typealias_declaration" => SymbolKind::Type,
        "macro_definition" => SymbolKind::Macro,
        _ => return None,
    };
    Some(symbol)
}

/// Fields that hold the name of a declaration, in order of preference. The
/// `declarator` precedes the `type`, which holds the return type of a C or
/// C++ function and the implemented type of a Rust `impl` block.
const NAME_FIELDS: [&str; 3] = ["name", "declarator", "type"];

/// Name of a declaration node and the node holding it.
fn declaration_name<'t>(node: Node<'t>, source: &[u8]) -> Option<(String, Node<'t>)> {
    let named = NAME_FIELDS
        .iter()
        .find_map(|field| node.child_by_field_name(field))
        .or_else(|| {
            let mut cursor = node.walk();

            node.named_children(&mut cursor)
                .find(|child| is_name_kind(child.kind()))
        })?;
    let name = innermost_name(named);
    let text = name.utf8_text(source).ok()?.trim().to_owned();
    (!text.is_empty()).then_some((text, name))
}

fn is_name_kind(kind: &str) -> bool {
    kind.ends_with("identifier") || kind == "constant" || kind == "name" || kind == "word"
}

/// Identifier inside a declarator chain (`*f(int)` in C, `S<T>` in Rust).
fn innermost_name(node: Node<'_>) -> Node<'_> {
    let mut current = node;
    loop {
        if is_name_kind(current.kind()) {
            return current;
        }
        let inner = current
            .child_by_field_name("declarator")
            .or_else(|| current.child_by_field_name("name"))
            .or_else(|| current.child_by_field_name("type"))
            .or_else(|| {
                let mut cursor = current.walk();

                current
                    .named_children(&mut cursor)
                    .find(|child| is_name_kind(child.kind()))
            });
        match inner {
            Some(next) => current = next,
            None => return current,
        }
    }
}

/// Declaration found in a tree.
struct Declaration<'t> {
    node: Node<'t>,
    kind: SymbolKind,
    name: String,
    name_node: Node<'t>,
}

fn declaration<'t>(node: Node<'t>, source: &[u8]) -> Option<Declaration<'t>> {
    let kind = declaration_kind(node.kind())?;
    let (name, name_node) = declaration_name(node, source)?;
    Some(Declaration {
        node,
        kind,
        name,
        name_node,
    })
}

/// Declarations containing `node`, innermost first, including `node`.
///
/// The nodes between the root and `node` are found by descending from the
/// root: `Node::parent` searches from the root on every call, so walking up
/// with it takes time quadratic in the depth of the tree.
fn ancestors<'t>(root: Node<'t>, node: Node<'t>, source: &[u8]) -> Vec<Declaration<'t>> {
    let mut path = vec![root];
    let mut current = root;
    while current.id() != node.id() {
        match current.child_with_descendant(node) {
            Some(next) if next.id() != current.id() => {
                path.push(next);
                current = next;
            }
            _ => break,
        }
    }
    path.into_iter()
        .rev()
        .filter_map(|node| declaration(node, source))
        .collect()
}

/// Innermost declaration whose node contains the byte `offset`.
///
/// A function declared inside a class, structure, interface or
/// implementation is reported as a method.
#[must_use]
pub fn enclosing(tree: &Tree, source: &[u8], offset: usize) -> Option<SymbolInfo> {
    let node = tree.root_node().descendant_for_byte_range(offset, offset)?;
    let chain = ancestors(tree.root_node(), node, source);
    let (inner, outer) = chain.split_first()?;
    let outer: Vec<&Declaration<'_>> = outer.iter().rev().collect();
    Some(symbol_info(inner, &outer, source, &LineIndex::new(source)))
}

/// Symbol of a declaration with its outer declarations, outermost first.
fn symbol_info(
    inner: &Declaration<'_>,
    outer: &[&Declaration<'_>],
    source: &[u8],
    lines: &LineIndex<'_>,
) -> SymbolInfo {
    let container = (!outer.is_empty()).then(|| {
        outer
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>()
            .join(".")
    });
    let in_type = outer.last().is_some_and(|parent| {
        matches!(
            parent.kind,
            SymbolKind::Class | SymbolKind::Struct | SymbolKind::Interface | SymbolKind::Type
        )
    });
    let kind = if inner.kind == SymbolKind::Function && in_type {
        SymbolKind::Method
    } else {
        inner.kind
    };
    SymbolInfo {
        name: inner.name.clone(),
        kind,
        container,
        depth: u32::try_from(outer.len()).unwrap_or(u32::MAX),
        span: lines.span(inner.node.start_byte(), inner.node.end_byte()),
        name_span: lines.span(inner.name_node.start_byte(), inner.name_node.end_byte()),
        signature: signature(inner.node, source),
        exported: exported(inner, source),
    }
}

/// Text of a declaration before its body, on one line.
fn signature(node: Node<'_>, source: &[u8]) -> Option<String> {
    let end = node
        .child_by_field_name("body")
        .map_or(node.end_byte(), |body| body.start_byte());
    let text = std::str::from_utf8(source.get(node.start_byte()..end)?).ok()?;
    let first_line = if node.child_by_field_name("body").is_some() {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        text.lines().next().unwrap_or_default().trim().to_owned()
    };
    let signature = first_line.trim_end_matches(['{', ':', ' ']).to_owned();
    (!signature.is_empty()).then_some(signature)
}

/// Visibility of a declaration outside its module, for languages whose
/// syntax shows it: Rust (`pub`), Go (initial capital letter), JavaScript
/// and TypeScript (`export`) and Python (no leading `_`).
fn exported(declaration: &Declaration<'_>, source: &[u8]) -> Option<bool> {
    let node = declaration.node;
    let kind = node.kind();
    if kind.ends_with("_item") {
        let mut cursor = node.walk();
        let public = node
            .children(&mut cursor)
            .any(|child| child.kind() == "visibility_modifier");
        return Some(public);
    }
    let parent_kind = node.parent().map(|parent| parent.kind());
    if parent_kind == Some("export_statement") {
        return Some(true);
    }
    let text = node.utf8_text(source).ok()?;
    if text.starts_with("func ") || kind == "type_spec" {
        return Some(declaration.name.starts_with(|c: char| c.is_uppercase()));
    }
    if kind == "function_definition" && text.starts_with("def ")
        || kind == "class_definition" && text.starts_with("class ")
    {
        return Some(!declaration.name.starts_with('_'));
    }
    None
}

/// Every declaration of a tree in source order, with the node of the
/// declaration.
pub(crate) fn declarations<'t>(tree: &'t Tree, source: &[u8]) -> Vec<(SymbolInfo, Node<'t>)> {
    let lines = LineIndex::new(source);
    let mut walker = Walker {
        source,
        lines: &lines,
        stack: Vec::new(),
        out: Vec::new(),
    };
    walker.walk(tree.root_node());
    walker.out
}

/// Work of the walk: a node to enter, or the end of the declaration entered
/// last.
enum Step<'t> {
    Enter(Node<'t>),
    Leave,
}

struct Walker<'s, 't> {
    source: &'s [u8],
    lines: &'s LineIndex<'s>,
    stack: Vec<Declaration<'t>>,
    out: Vec<(SymbolInfo, Node<'t>)>,
}

impl<'t> Walker<'_, 't> {
    /// Visits the subtree of `root` in source order with an explicit stack,
    /// so that the depth of the tree is not limited by the stack of the
    /// thread.
    fn walk(&mut self, root: Node<'t>) {
        let mut pending = vec![Step::Enter(root)];
        while let Some(step) = pending.pop() {
            let node = match step {
                Step::Leave => {
                    self.stack.pop();
                    continue;
                }
                Step::Enter(node) => node,
            };
            if let Some(found) = declaration(node, self.source) {
                let outer: Vec<&Declaration<'t>> = self.stack.iter().collect();
                let info = symbol_info(&found, &outer, self.source, self.lines);
                self.out.push((info, node));
                self.stack.push(found);
                pending.push(Step::Leave);
            }
            let mut cursor = node.walk();
            let children: Vec<Node<'t>> = node.children(&mut cursor).collect();
            pending.extend(children.into_iter().rev().map(Step::Enter));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaration_kinds_cover_common_grammars() {
        assert_eq!(
            declaration_kind("function_item"),
            Some(SymbolKind::Function)
        );
        assert_eq!(
            declaration_kind("method_declaration"),
            Some(SymbolKind::Method)
        );
        assert_eq!(declaration_kind("class_specifier"), Some(SymbolKind::Class));
        assert_eq!(declaration_kind("trait_item"), Some(SymbolKind::Interface));
        assert_eq!(declaration_kind("call_expression"), None);
    }

    /// Depth of the nesting that would overflow the stack of a test thread
    /// in a walk by recursion.
    const DEEP: usize = 300_000;

    fn parse(language: tree_sitter::Language, source: &[u8]) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn c_function_is_named_by_its_declarator_not_by_its_return_type() {
        let source = b"static int *find(int n) {\n    return 0;\n}\n";
        let tree = parse(tree_sitter_c::LANGUAGE.into(), source);
        let offset = source.windows(6).position(|w| w == b"return").unwrap();
        let symbol = enclosing(&tree, source, offset).unwrap();
        assert_eq!(symbol.name, "find");
        assert_eq!(symbol.kind, SymbolKind::Function);
        assert_eq!(symbol.name_span.start.line, 1);
        assert_eq!(symbol.name_span.start.column, 13);
    }

    #[test]
    fn declarations_are_listed_in_source_order_with_their_containers() {
        let source = b"mod a {\n    fn f() {\n        fn g() {}\n    }\n    fn h() {}\n}\n";
        let tree = parse(tree_sitter_rust::LANGUAGE.into(), source);
        let found: Vec<(String, u32, Option<String>)> = declarations(&tree, source)
            .into_iter()
            .map(|(info, _)| (info.name, info.depth, info.container))
            .collect();
        let expected = [
            ("a", 0, None),
            ("f", 1, Some("a")),
            ("g", 2, Some("a.f")),
            ("h", 1, Some("a")),
        ];
        assert_eq!(found.len(), expected.len());
        for (found, expected) in found.iter().zip(expected) {
            assert_eq!(found.0, expected.0);
            assert_eq!(found.1, expected.1);
            assert_eq!(found.2.as_deref(), expected.2);
        }
    }

    #[test]
    fn deeply_nested_source_is_walked_without_recursion() {
        let source = format!("{}1{}", "[".repeat(DEEP), "]".repeat(DEEP));
        let tree = parse(tree_sitter_json::LANGUAGE.into(), source.as_bytes());
        assert!(declarations(&tree, source.as_bytes()).is_empty());
        assert!(enclosing(&tree, source.as_bytes(), DEEP).is_none());
    }

    #[test]
    fn name_kinds_include_identifiers_and_constants() {
        assert!(is_name_kind("type_identifier"));
        assert!(is_name_kind("constant"));
        assert!(!is_name_kind("block"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
