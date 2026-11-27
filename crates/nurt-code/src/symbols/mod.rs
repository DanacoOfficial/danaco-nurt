// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::symbols
// Responsibility:   Symbol extraction from syntax trees.
// Dependencies:     nurt_contracts, tree_sitter
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Symbol extraction from syntax trees.
//!
//! [`extract`] runs the symbol query of the grammar (see
//! [`crate::registry`]) and turns its matches into [`SymbolInfo`] values in
//! source order. The kind given by the query is refined by the container:
//! a function inside a type is a method, a method outside any type is a
//! function, a variable inside a type is a field, and a variable or field
//! named in upper case (`MAX_SIZE`) is a constant. Variables declared inside
//! a function body are local and are not symbols.

pub mod matching;
pub mod records;
mod visibility;

use std::cmp::Reverse;
use std::collections::BTreeMap;

use nurt_contracts::error::Result;
use nurt_contracts::services::{LanguageId, SymbolInfo, SymbolKind};
use tree_sitter::{Node, QueryCursor, QueryMatch, StreamingIterator};

use crate::registry::{ParsedSource, Registry, to_u32};

/// Longest signature kept, in bytes; longer signatures end with
/// [`SIGNATURE_ELLIPSIS`].
pub const SIGNATURE_MAX: usize = 160;

/// Marker appended to a shortened signature.
pub const SIGNATURE_ELLIPSIS: &str = "...";

/// Kind named by a capture `@definition.<kind>` or by the `kind` parameter.
///
/// ```
/// use nurt_code::symbols::parse_kind;
/// use nurt_contracts::services::SymbolKind;
///
/// assert_eq!(parse_kind("method"), Some(SymbolKind::Method));
/// assert_eq!(parse_kind("any"), None);
/// ```
#[must_use]
pub fn parse_kind(text: &str) -> Option<SymbolKind> {
    Some(match text {
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "class" => SymbolKind::Class,
        "struct" => SymbolKind::Struct,
        "interface" => SymbolKind::Interface,
        "enum" => SymbolKind::Enum,
        "variable" => SymbolKind::Variable,
        "constant" => SymbolKind::Constant,
        "field" => SymbolKind::Field,
        "module" => SymbolKind::Module,
        "type" => SymbolKind::Type,
        "macro" => SymbolKind::Macro,
        _ => return None,
    })
}

/// Stable name of a kind, as accepted by [`parse_kind`].
#[must_use]
pub fn kind_name(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Interface => "interface",
        SymbolKind::Enum => "enum",
        SymbolKind::Variable => "variable",
        SymbolKind::Constant => "constant",
        SymbolKind::Field => "field",
        SymbolKind::Module => "module",
        SymbolKind::Type => "type",
        SymbolKind::Macro => "macro",
        _ => "other",
    }
}

/// Returns the symbols of source text in source order.
///
/// # Errors
///
/// Fails with `E_UNSUPPORTED_LANGUAGE` when the language has no grammar.
///
/// # Examples
///
/// ```
/// use nurt_code::registry::Registry;
/// use nurt_code::symbols::symbols_of;
/// use nurt_contracts::services::{LanguageId, SymbolKind};
///
/// let source = b"class Stack:\n    def push(self, item):\n        pass\n";
/// let symbols = symbols_of(&Registry::builtin(), source, &LanguageId::new("python")?)?;
/// assert_eq!(symbols[1].name, "push");
/// assert_eq!(symbols[1].kind, SymbolKind::Method);
/// assert_eq!(symbols[1].container.as_deref(), Some("Stack"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn symbols_of(
    registry: &Registry,
    source: &[u8],
    language: &LanguageId,
) -> Result<Vec<SymbolInfo>> {
    let parsed = registry.parse(language, source)?;
    Ok(extract(&parsed))
}

/// Symbols of a parsed source in source order.
#[must_use]
pub fn extract(parsed: &ParsedSource<'_>) -> Vec<SymbolInfo> {
    let Some(query) = parsed.grammar.tags() else {
        return Vec::new();
    };
    let names = query.capture_names();
    let mut found: BTreeMap<(usize, Reverse<usize>, usize), Raw<'_>> = BTreeMap::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, parsed.tree.root_node(), parsed.source);
    while let Some(found_match) = matches.next() {
        let Some(raw) = raw_definition(found_match, names, parsed) else {
            continue;
        };
        let key = (
            raw.node.start_byte(),
            Reverse(raw.node.end_byte()),
            raw.name_node
                .map_or(raw.node.start_byte(), |node| node.start_byte()),
        );
        match found.get(&key) {
            Some(existing) if existing.pattern <= raw.pattern => {}
            _ => {
                found.insert(key, raw);
            }
        }
    }
    Builder::new(parsed).build(found.into_values())
}

struct Raw<'t> {
    node: Node<'t>,
    name_node: Option<Node<'t>>,
    kind: Option<SymbolKind>,
    container: Option<String>,
    pattern: usize,
}

fn raw_definition<'t>(
    found: &QueryMatch<'_, 't>,
    names: &[&str],
    parsed: &ParsedSource<'_>,
) -> Option<Raw<'t>> {
    let mut node = None;
    let mut kind = None;
    let mut name_node = None;
    let mut container = None;
    for capture in found.captures() {
        let capture_name = names.get(usize::try_from(capture.index).ok()?).copied()?;
        match capture_name {
            "name" => name_node = Some(capture.node),
            "container.name" => container = Some(normalize_name(&parsed.text(capture.node))),
            "scope" => node = Some(capture.node),
            other => {
                if let Some(text) = other.strip_prefix("definition.") {
                    kind = Some(parse_kind(text)?);
                    node = Some(capture.node);
                }
            }
        }
    }
    Some(Raw {
        node: node?,
        name_node,
        kind,
        container,
        pattern: found.pattern_index,
    })
}

/// Container of the symbols being visited.
struct Open {
    start: usize,
    end: usize,
    qualified: String,
    kind: Option<SymbolKind>,
    exported: Option<bool>,
}

/// Facts about the container of a symbol used by the refinement and
/// visibility rules.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Parent {
    /// Kind of the container; `None` for a scope that is not a symbol.
    pub kind: Option<SymbolKind>,
    /// Visibility of the container.
    pub exported: Option<bool>,
}

impl Parent {
    pub(crate) fn is_type(self) -> bool {
        matches!(
            self.kind,
            None | Some(
                SymbolKind::Class | SymbolKind::Struct | SymbolKind::Interface | SymbolKind::Enum
            )
        )
    }

    fn is_callable(self) -> bool {
        matches!(self.kind, Some(SymbolKind::Function | SymbolKind::Method))
    }
}

struct Builder<'p, 'a> {
    parsed: &'p ParsedSource<'a>,
    stack: Vec<Open>,
    symbols: Vec<SymbolInfo>,
}

impl<'p, 'a> Builder<'p, 'a> {
    fn new(parsed: &'p ParsedSource<'a>) -> Self {
        Self {
            parsed,
            stack: Vec::new(),
            symbols: Vec::new(),
        }
    }

    fn build<'t>(mut self, raws: impl Iterator<Item = Raw<'t>>) -> Vec<SymbolInfo> {
        for raw in raws {
            self.visit(&raw);
        }
        self.symbols
    }

    fn close_until(&mut self, start: usize, end: usize) {
        while let Some(top) = self.stack.last() {
            let contains = top.start <= start && end <= top.end;
            let same = top.start == start && top.end == end;
            if contains && !same {
                break;
            }
            self.stack.pop();
        }
    }

    fn visit(&mut self, raw: &Raw<'_>) {
        let (start, end) = (raw.node.start_byte(), raw.node.end_byte());
        self.close_until(start, end);
        let name = match raw.name_node {
            Some(node) => normalize_name(&self.parsed.text(node)),
            None => signature(self.parsed, raw.node).unwrap_or_default(),
        };
        if name.is_empty() {
            return;
        }
        let parent = match (&raw.container, self.stack.last()) {
            (Some(_), _) => Some(Parent {
                kind: None,
                exported: None,
            }),
            (None, Some(open)) => Some(Parent {
                kind: open.kind,
                exported: open.exported,
            }),
            (None, None) => None,
        };
        let container = raw
            .container
            .clone()
            .or_else(|| self.stack.last().map(|open| open.qualified.clone()));
        let qualified = match &container {
            Some(container) => format!("{container}.{name}"),
            None => name.clone(),
        };
        let Some(kind) = raw.kind else {
            self.stack.push(Open {
                start,
                end,
                qualified,
                kind: None,
                exported: None,
            });
            return;
        };
        let Some(kind) = refine_kind(kind, &name, parent) else {
            return;
        };
        let explicit = usize::from(raw.container.is_some() && self.stack.is_empty());
        let depth = to_u32(self.stack.len() + explicit);
        let exported = visibility::exported(self.parsed, raw.node, &name, parent);
        let name_node = raw.name_node.unwrap_or(raw.node);
        self.symbols.push(SymbolInfo {
            name,
            kind,
            container,
            depth,
            span: self.parsed.span(raw.node),
            name_span: self.parsed.span(name_node),
            signature: signature(self.parsed, raw.node),
            exported,
        });
        self.stack.push(Open {
            start,
            end,
            qualified,
            kind: Some(kind),
            exported,
        });
    }
}

/// Kind after the container rules of the module documentation; `None` for
/// a local variable, which is not a symbol.
fn refine_kind(kind: SymbolKind, name: &str, parent: Option<Parent>) -> Option<SymbolKind> {
    let in_type = parent.is_some_and(Parent::is_type);
    let in_callable = parent.is_some_and(Parent::is_callable);
    let kind = match kind {
        SymbolKind::Variable | SymbolKind::Constant if in_callable => return None,
        SymbolKind::Function if in_type => SymbolKind::Method,
        SymbolKind::Method if parent.is_none() => SymbolKind::Function,
        SymbolKind::Variable if in_type => SymbolKind::Field,
        other => other,
    };
    Some(match kind {
        SymbolKind::Variable | SymbolKind::Field if is_constant_name(name) => SymbolKind::Constant,
        other => other,
    })
}

/// Upper-case name of at least two characters, such as `MAX_SIZE`.
fn is_constant_name(name: &str) -> bool {
    name.chars().count() >= 2
        && name.chars().any(char::is_uppercase)
        && !name.chars().any(char::is_lowercase)
        && name
            .chars()
            .all(|c| c.is_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Name as shown: white space collapsed, a leading `$` and enclosing quotes
/// removed.
fn normalize_name(text: &str) -> String {
    let collapsed = collapse_whitespace(text);
    let name = collapsed.strip_prefix('$').unwrap_or(&collapsed);
    for quote in ['"', '\''] {
        if let Some(inner) = name
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner.to_owned();
        }
    }
    name.to_owned()
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Declaration line of a definition without its body: the text up to the
/// body (or the first line when the definition has no body), white space
/// collapsed, without a trailing `{` or `:`, shortened to
/// [`SIGNATURE_MAX`] bytes.
fn signature(parsed: &ParsedSource<'_>, node: Node<'_>) -> Option<String> {
    let start = node.start_byte();
    let end = node
        .child_by_field_name("body")
        .map_or(node.end_byte(), |body| body.start_byte())
        .max(start);
    let text = String::from_utf8_lossy(parsed.source.get(start..end).unwrap_or_default());
    let text = if node.child_by_field_name("body").is_some() {
        text.as_ref()
    } else {
        text.lines().next().unwrap_or_default()
    };
    let collapsed = collapse_whitespace(text);
    let trimmed = collapsed
        .trim_end_matches(|c: char| c == '{' || c.is_whitespace())
        .trim_end_matches(':')
        .trim_end();
    if trimmed.is_empty() {
        return None;
    }
    Some(shorten(trimmed))
}

fn shorten(text: &str) -> String {
    if text.len() <= SIGNATURE_MAX {
        return text.to_owned();
    }
    let mut cut = SIGNATURE_MAX - SIGNATURE_ELLIPSIS.len();
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{SIGNATURE_ELLIPSIS}", &text[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbols(language: &str, source: &str) -> Vec<SymbolInfo> {
        symbols_of(
            Registry::builtin(),
            source.as_bytes(),
            &LanguageId::new(language).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn constant_names_are_upper_case_with_two_characters() {
        assert!(is_constant_name("MAX_SIZE"));
        assert!(is_constant_name("V2"));
        assert!(!is_constant_name("X"));
        assert!(!is_constant_name("MaxSize"));
    }

    #[test]
    fn long_signature_is_shortened_on_a_character_boundary() {
        let text = "é".repeat(SIGNATURE_MAX);
        let short = shorten(&text);
        assert!(short.len() <= SIGNATURE_MAX);
        assert!(short.ends_with(SIGNATURE_ELLIPSIS));
    }

    #[test]
    fn names_lose_quotes_and_dollar_sign() {
        assert_eq!(normalize_name("\"key\""), "key");
        assert_eq!(normalize_name("$Name"), "Name");
        assert_eq!(normalize_name("a\n  b"), "a b");
    }

    #[test]
    fn rust_impl_methods_get_the_type_as_container() {
        let found = symbols(
            "rust",
            "pub struct Point { x: i32 }\nimpl Point {\n    pub fn norm(&self) -> i32 { 0 }\n}\n",
        );
        let norm = found.iter().find(|s| s.name == "norm").unwrap();
        assert_eq!(norm.kind, SymbolKind::Method);
        assert_eq!(norm.container.as_deref(), Some("Point"));
        assert_eq!(norm.depth, 1);
        assert_eq!(norm.exported, Some(true));
        assert_eq!(norm.signature.as_deref(), Some("pub fn norm(&self) -> i32"));
    }

    #[test]
    fn go_method_container_comes_from_the_receiver() {
        let found = symbols(
            "go",
            "package p\n\ntype Queue struct{}\n\nfunc (q *Queue) Push(v int) {}\n",
        );
        let push = found.iter().find(|s| s.name == "Push").unwrap();
        assert_eq!(push.kind, SymbolKind::Method);
        assert_eq!(push.container.as_deref(), Some("Queue"));
        assert_eq!(push.exported, Some(true));
    }

    #[test]
    fn local_variables_are_not_symbols() {
        let found = symbols(
            "swift",
            "func run() {\n    let total = 1\n}\nlet LIMIT = 3\n",
        );
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["run", "LIMIT"]);
        assert_eq!(found[1].kind, SymbolKind::Constant);
    }

    #[test]
    fn css_media_rule_is_named_by_its_prelude() {
        let found = symbols("css", "@media screen {\n  .a { color: red; }\n}\n");
        assert_eq!(found[0].name, "@media screen");
        assert_eq!(found[1].name, ".a");
        assert_eq!(found[1].container.as_deref(), Some("@media screen"));
    }

    #[test]
    fn every_kind_name_round_trips() {
        for kind in [
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Class,
            SymbolKind::Struct,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::Variable,
            SymbolKind::Constant,
            SymbolKind::Field,
            SymbolKind::Module,
            SymbolKind::Type,
            SymbolKind::Macro,
        ] {
            assert_eq!(parse_kind(kind_name(kind)), Some(kind));
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
