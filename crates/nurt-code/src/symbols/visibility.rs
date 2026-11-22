// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Visibility of symbols outside their module.
//!
//! | Language | Exported when |
//! |---|---|
//! | Python | the name does not start with `_` |
//! | Go | the name starts with an upper-case letter |
//! | Rust | the item has a visibility modifier (`pub`, `pub(crate)`); enum variants and trait items follow their container |
//! | JavaScript, TypeScript | top level: inside `export`; members: not `private`, `protected` or `#name` |
//! | C, C++ | not `static` |
//! | Java, C# | `public`; interface members and enum constants follow their container |
//! | Kotlin | not `private`, `protected` or `internal` |
//! | Swift | `public` or `open`; protocol members and enum cases follow their container |
//! | PHP | not `private` or `protected` |
//!
//! Other languages do not define visibility (`None`).

use nurt_contracts::services::SymbolKind;
use tree_sitter::Node;

use super::Parent;
use crate::registry::ParsedSource;

/// Levels of ancestors searched for an `export` statement.
const EXPORT_DEPTH: usize = 4;

pub(super) fn exported(
    parsed: &ParsedSource<'_>,
    node: Node<'_>,
    name: &str,
    parent: Option<Parent>,
) -> Option<bool> {
    let inherited = parent.and_then(|parent| parent.exported);
    let parent_kind = parent.and_then(|parent| parent.kind);
    let in_interface = parent_kind == Some(SymbolKind::Interface);
    let in_enum = parent_kind == Some(SymbolKind::Enum);
    let words = modifier_words(parsed, node);
    let has = |word: &str| words.iter().any(|w| w == word);
    match parsed.language.as_str() {
        "python" => Some(!name.starts_with('_')),
        "go" => Some(name.chars().next().is_some_and(char::is_uppercase)),
        "rust" => {
            if has_child(node, "visibility_modifier") {
                Some(true)
            } else if node.kind() == "enum_variant" || in_interface {
                inherited
            } else {
                Some(false)
            }
        }
        "javascript" | "typescript" | "tsx" => {
            if parent.is_some_and(Parent::is_type) {
                if in_interface {
                    inherited
                } else {
                    Some(!(has("private") || has("protected") || name.starts_with('#')))
                }
            } else {
                Some(inside_export(node))
            }
        }
        "c" | "cpp" => Some(!has_static_storage(parsed, node)),
        "java" | "csharp" => {
            if in_interface || in_enum {
                inherited.or(Some(true))
            } else {
                Some(has("public"))
            }
        }
        "kotlin" => Some(!(has("private") || has("protected") || has("internal"))),
        "swift" => {
            if in_interface || in_enum {
                inherited
            } else {
                Some(has("public") || has("open"))
            }
        }
        "php" => Some(!(has("private") || has("protected"))),
        _ => None,
    }
}

fn has_child(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| child.kind() == kind)
}

/// Words of the modifier children of a definition (`modifiers`,
/// `modifier`, `visibility_modifier`, `accessibility_modifier`).
fn modifier_words(parsed: &ParsedSource<'_>, node: Node<'_>) -> Vec<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.kind().contains("modifier"))
        .flat_map(|child| {
            parsed
                .text(child)
                .split(|c: char| !c.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn inside_export(node: Node<'_>) -> bool {
    let mut current = node.parent();
    for _ in 0..EXPORT_DEPTH {
        match current {
            Some(ancestor) if ancestor.kind() == "export_statement" => return true,
            Some(ancestor) if ancestor.kind() == "program" => return false,
            Some(ancestor) => current = ancestor.parent(),
            None => return false,
        }
    }
    false
}

fn has_static_storage(parsed: &ParsedSource<'_>, node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == "storage_class_specifier" && parsed.text(child) == "static")
}
