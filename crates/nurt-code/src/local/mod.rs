// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::local
// Responsibility:   Scope-based analysis of one source file.
// Dependencies:     calls, nurt_contracts, testpath, tree_sitter
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Scope-based analysis of one source file.
//!
//! [`LocalAnalysis`] finds every identifier of a file and binds it to its
//! definition without a language server:
//!
//! - Scopes come from the `@local.scope` captures of the scope query of the
//!   grammar; the file itself is the outermost scope.
//! - Definitions are the names of symbols ([`crate::symbols`]), the
//!   `@local.definition` captures (parameters, local variables) and the
//!   `@local.import` captures (names introduced by imports). A symbol is
//!   defined in the scope around its definition; a local name in the
//!   innermost scope holding it.
//! - A name is bound to the nearest definition of the same name in its
//!   scope or an enclosing scope: the last one before the name, or the
//!   first one after it when there is none before (functions and classes
//!   used before their definition).
//! - A name after a member operator (`object.name`) is bound to the member
//!   symbol of that name when the file defines exactly one.
//!
//! The analysis is approximate: it does not know types, so it cannot tell
//! apart members of different types with the same name, and names defined
//! in other files stay unbound. Commands use it when no language server
//! answers.

mod calls;
mod rules;
mod testpath;

use std::collections::HashMap;

use nurt_contracts::error::Result;
use nurt_contracts::services::{
    ByteRange, ClassifiedSpan, LanguageId, ReferenceRole, SpanClass, SymbolInfo, SymbolKind,
    SymbolReference, TextPosition, TextSpan,
};
use tree_sitter::{Node, QueryCursor, StreamingIterator};

use crate::registry::{ParsedSource, Registry, to_u64};
use crate::spans::{class_at, classify_spans};
use crate::symbols::extract;
use crate::symbols::records::qualified_name;

pub use calls::CallSite;
pub use testpath::is_test_path;

/// How an occurrence takes part in the binding of its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OccurrenceKind {
    /// The name of a symbol in its definition.
    Symbol,
    /// A parameter or local variable where it is introduced.
    Local,
    /// A name introduced by an import.
    Import,
    /// Any other occurrence.
    Use,
}

/// Identifier found in the syntax tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// Name as written, without a leading `$`.
    pub name: String,
    /// Region of the name.
    pub span: TextSpan,
    /// Part in the binding of the name.
    pub kind: OccurrenceKind,
    /// The name follows a member operator or qualifies nothing in scope.
    pub member: bool,
    /// Index of the innermost scope holding the occurrence.
    pub scope: usize,
    /// Index of the definition the name is bound to, among the occurrences;
    /// a definition is bound to itself.
    pub binding: Option<usize>,
    /// Index of the symbol named by the occurrence, among the symbols, for
    /// [`OccurrenceKind::Symbol`].
    pub symbol: Option<usize>,
}

impl Occurrence {
    /// Returns `true` for symbol names, local definitions and imports.
    #[must_use]
    pub fn is_definition(&self) -> bool {
        self.kind != OccurrenceKind::Use
    }

    /// Role in the form of the code service.
    #[must_use]
    pub fn role(&self) -> ReferenceRole {
        match self.kind {
            OccurrenceKind::Symbol | OccurrenceKind::Local => ReferenceRole::Definition,
            OccurrenceKind::Import => ReferenceRole::Import,
            OccurrenceKind::Use => ReferenceRole::Usage,
        }
    }
}

/// Scope of the file: a byte range and its enclosing scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    /// Bytes of the scope.
    pub bytes: ByteRange,
    /// Index of the enclosing scope; `None` for the file.
    pub parent: Option<usize>,
}

/// Scopes, definitions, occurrences and calls of one source file.
#[derive(Debug)]
pub struct LocalAnalysis<'a> {
    source: &'a [u8],
    language: LanguageId,
    line_starts: Vec<usize>,
    symbols: Vec<SymbolInfo>,
    /// Index of the symbol whose definition directly contains each symbol.
    symbol_parents: Vec<Option<usize>>,
    scopes: Vec<Scope>,
    occurrences: Vec<Occurrence>,
    /// Byte from which the name of each occurrence is visible: the end of
    /// a sequential declaration for its locals, the name itself otherwise.
    visible_from: Vec<u64>,
    spans: Vec<ClassifiedSpan>,
    calls: Vec<CallSite>,
}

/// Analyses source text.
///
/// # Errors
///
/// Fails with `E_UNSUPPORTED_LANGUAGE` when the language has no grammar.
///
/// # Examples
///
/// ```
/// use nurt_code::local::analyze;
/// use nurt_code::registry::Registry;
/// use nurt_contracts::services::LanguageId;
///
/// let source = b"def area(r):\n    return 3 * r * r\n\nprint(area(2))\n";
/// let analysis = analyze(Registry::builtin(), source, &LanguageId::new("python")?)?;
/// let parameter = analysis.occurrence_at(9).unwrap();
/// assert_eq!(parameter.name, "r");
/// assert_eq!(analysis.bound_to(parameter).len(), 3);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn analyze<'a>(
    registry: &Registry,
    source: &'a [u8],
    language: &LanguageId,
) -> Result<LocalAnalysis<'a>> {
    let parsed = registry.parse(language, source)?;
    Ok(LocalAnalysis::new(&parsed))
}

/// Occurrences of the identifier `name` in source text, including the
/// occurrences in comments and strings, in source order.
///
/// # Errors
///
/// Fails with `E_UNSUPPORTED_LANGUAGE` when the language has no grammar.
///
/// # Examples
///
/// ```
/// use nurt_code::local::references;
/// use nurt_code::registry::Registry;
/// use nurt_contracts::services::{LanguageId, ReferenceRole, SpanClass};
///
/// let source = b"// total of items\nfn total() -> u32 { 0 }\nfn main() { total(); }\n";
/// let found = references(Registry::builtin(), source, &LanguageId::new("rust")?, "total")?;
/// assert_eq!(found.len(), 3);
/// assert_eq!(found[0].class, SpanClass::Comment);
/// assert_eq!(found[1].role, ReferenceRole::Definition);
/// assert_eq!(found[2].container.as_deref(), Some("main"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn references(
    registry: &Registry,
    source: &[u8],
    language: &LanguageId,
    name: &str,
) -> Result<Vec<SymbolReference>> {
    Ok(analyze(registry, source, language)?.references(name))
}

/// Byte offset of a position in `source`; columns count Unicode scalar
/// values from 1. `None` when the line does not exist; a column past the
/// end of the line gives the end of the line.
///
/// ```
/// use nurt_code::local::offset_of;
/// use nurt_contracts::services::TextPosition;
///
/// let source = "a = 1\néüλw = 2\n".as_bytes();
/// assert_eq!(offset_of(source, TextPosition { line: 2, column: 2 }), Some(8));
/// assert_eq!(offset_of(source, TextPosition { line: 9, column: 1 }), None);
/// ```
#[must_use]
pub fn offset_of(source: &[u8], position: TextPosition) -> Option<usize> {
    let line_index = usize::try_from(position.line.checked_sub(1)?).ok()?;
    let start = if line_index == 0 {
        0
    } else {
        source
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .nth(line_index - 1)
            .map(|(at, _)| at + 1)?
    };
    let end = source[start..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(source.len(), |at| start + at);
    let column = usize::try_from(position.column.saturating_sub(1)).ok()?;
    let line = &source[start..end];
    let within = line
        .iter()
        .enumerate()
        .filter(|(_, byte)| (**byte & 0xC0) != 0x80)
        .nth(column)
        .map_or(line.len(), |(at, _)| at);
    Some(start + within)
}

impl<'a> LocalAnalysis<'a> {
    /// Analyses a parsed source.
    #[must_use]
    pub fn new(parsed: &ParsedSource<'a>) -> Self {
        let symbols = extract(parsed);
        let symbol_parents = nesting_parents(&symbols);
        let scopes = collect_scopes(parsed);
        let mut analysis = Self {
            source: parsed.source,
            language: parsed.language.clone(),
            line_starts: line_starts(parsed.source),
            spans: classify_spans(&parsed.tree),
            calls: Vec::new(),
            symbols,
            symbol_parents,
            scopes,
            occurrences: Vec::new(),
            visible_from: Vec::new(),
        };
        analysis.collect_occurrences(parsed);
        analysis.bind();
        analysis.calls = calls::collect(parsed, &analysis.symbols);
        analysis
    }

    /// Language of the source.
    #[must_use]
    pub fn language(&self) -> &LanguageId {
        &self.language
    }

    /// Symbols of the source in source order.
    #[must_use]
    pub fn symbols(&self) -> &[SymbolInfo] {
        &self.symbols
    }

    /// Scopes; index 0 is the file.
    #[must_use]
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    /// Identifier occurrences in source order.
    #[must_use]
    pub fn occurrences(&self) -> &[Occurrence] {
        &self.occurrences
    }

    /// Comments and string literals in source order.
    #[must_use]
    pub fn spans(&self) -> &[ClassifiedSpan] {
        &self.spans
    }

    /// Calls in source order.
    #[must_use]
    pub fn calls(&self) -> &[CallSite] {
        &self.calls
    }

    /// Occurrence that contains the byte `offset` or ends at it.
    #[must_use]
    pub fn occurrence_at(&self, offset: u64) -> Option<&Occurrence> {
        let containing = self
            .occurrences
            .iter()
            .find(|occurrence| occurrence.span.bytes.contains(offset));
        containing.or_else(|| {
            self.occurrences
                .iter()
                .find(|occurrence| occurrence.span.bytes.end == offset)
        })
    }

    /// Definition an occurrence is bound to.
    #[must_use]
    pub fn definition_of(&self, occurrence: &Occurrence) -> Option<&Occurrence> {
        occurrence
            .binding
            .and_then(|index| self.occurrences.get(index))
    }

    /// Occurrences bound to the same definition as `occurrence`, the
    /// definition included, in source order; for an unbound name, the
    /// unbound occurrences of the same name and member kind.
    #[must_use]
    pub fn bound_to(&self, occurrence: &Occurrence) -> Vec<&Occurrence> {
        match occurrence.binding {
            Some(binding) => self
                .occurrences
                .iter()
                .filter(|other| other.binding == Some(binding))
                .collect(),
            None => self
                .occurrences
                .iter()
                .filter(|other| {
                    other.binding.is_none()
                        && other.name == occurrence.name
                        && other.member == occurrence.member
                })
                .collect(),
        }
    }

    /// Qualified name of the innermost symbol whose definition contains
    /// the byte `offset`, other than a symbol named at `offset`.
    #[must_use]
    pub fn container_at(&self, offset: u64) -> Option<String> {
        let qualifies = |symbol: &SymbolInfo| {
            offset < symbol.span.bytes.end && symbol.name_span.bytes.start != offset
        };
        let upper = self
            .symbols
            .partition_point(|symbol| symbol.span.bytes.start <= offset);
        let mut current = upper.checked_sub(1);
        while let Some(index) = current {
            let symbol = &self.symbols[index];
            if qualifies(symbol) {
                // Of several symbols with the same span the first wins.
                let mut found = index;
                let mut up = self.symbol_parents[index];
                while let Some(outer) = up {
                    let candidate = &self.symbols[outer];
                    if candidate.span.bytes != symbol.span.bytes {
                        break;
                    }
                    if qualifies(candidate) {
                        found = outer;
                    }
                    up = self.symbol_parents[outer];
                }
                let symbol = &self.symbols[found];
                return Some(qualified_name(&symbol.name, symbol.container.as_deref()));
            }
            current = self.symbol_parents[index];
        }
        None
    }

    /// Lexical class of the byte at `offset`.
    #[must_use]
    pub fn class_at(&self, offset: u64) -> SpanClass {
        class_at(&self.spans, offset)
    }

    /// Occurrences of the identifier `name` in code, comments and strings,
    /// in source order.
    #[must_use]
    pub fn references(&self, name: &str) -> Vec<SymbolReference> {
        let name = name.strip_prefix('$').unwrap_or(name);
        let mut found: Vec<SymbolReference> = self
            .occurrences
            .iter()
            .filter(|occurrence| occurrence.name == name)
            .map(|occurrence| self.reference(occurrence.span, occurrence.role()))
            .collect();
        for span in &self.spans {
            for start in self.words_in(span.bytes, name) {
                if found.iter().any(|item| item.span.bytes.start == start) {
                    continue;
                }
                let bytes = ByteRange::new(start, start + to_u64(name.len()));
                let mut reference = self.reference(self.text_span(bytes), ReferenceRole::Usage);
                reference.class = span.class;
                found.push(reference);
            }
        }
        found.sort_by_key(|item| item.span.bytes.start);
        found
    }

    fn reference(&self, span: TextSpan, role: ReferenceRole) -> SymbolReference {
        let start = span.bytes.start;
        SymbolReference {
            span,
            role,
            class: self.class_at(start),
            container: self.container_at(start),
            line_text: self.line_text(start),
        }
    }

    /// Start offsets of whole-word occurrences of `word` in a region.
    fn words_in(&self, bytes: ByteRange, word: &str) -> Vec<u64> {
        let start = usize::try_from(bytes.start).unwrap_or(usize::MAX);
        let end = usize::try_from(bytes.end).unwrap_or(usize::MAX);
        let Some(region) = self.source.get(start..end.min(self.source.len())) else {
            return Vec::new();
        };
        let text = String::from_utf8_lossy(region);
        if word.is_empty() || text.len() != region.len() {
            return Vec::new();
        }
        text.match_indices(word)
            .filter(|(at, _)| {
                let before = text[..*at].chars().next_back();
                let after = text[at + word.len()..].chars().next();
                !before.is_some_and(rules::is_identifier_char)
                    && !after.is_some_and(rules::is_identifier_char)
            })
            .map(|(at, _)| to_u64(start + at))
            .collect()
    }

    /// The whole line holding the byte `offset`, without the line ending.
    #[must_use]
    pub fn line_text(&self, offset: u64) -> String {
        let offset = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(self.source.len());
        let line = self.line_starts.partition_point(|start| *start <= offset) - 1;
        let start = self.line_starts[line];
        let end = self
            .line_starts
            .get(line + 1)
            .map_or(self.source.len(), |next| next - 1);
        String::from_utf8_lossy(&self.source[start..end.max(start)])
            .trim_end_matches('\r')
            .to_owned()
    }

    /// Position of the byte `offset`.
    #[must_use]
    pub fn position(&self, offset: u64) -> TextPosition {
        let offset = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(self.source.len());
        let line = self.line_starts.partition_point(|start| *start <= offset) - 1;
        let start = self.line_starts[line];
        let column = self.source[start..offset]
            .iter()
            .filter(|byte| (**byte & 0xC0) != 0x80)
            .count();
        TextPosition {
            line: u32::try_from(line + 1).unwrap_or(u32::MAX),
            column: u32::try_from(column + 1).unwrap_or(u32::MAX),
        }
    }

    fn text_span(&self, bytes: ByteRange) -> TextSpan {
        TextSpan {
            bytes,
            start: self.position(bytes.start),
            end: self.position(bytes.end),
        }
    }

    /// Index of the innermost scope that contains the bytes `start..end`.
    ///
    /// Scopes are sorted by start and nest like syntax nodes, so the last
    /// scope that starts at or before `start` either contains the bytes or
    /// has the innermost containing scope among its ancestors.
    fn innermost_scope(&self, start: u64, end: u64, exclude_same: bool) -> usize {
        let upper = self
            .scopes
            .partition_point(|scope| scope.bytes.start <= start);
        let mut current = upper.checked_sub(1);
        while let Some(index) = current {
            let scope = &self.scopes[index];
            let same = scope.bytes.start == start && scope.bytes.end == end;
            if end <= scope.bytes.end && !(exclude_same && same) {
                return index;
            }
            current = scope.parent;
        }
        0
    }

    fn collect_occurrences(&mut self, parsed: &ParsedSource<'_>) {
        let definitions = self.definition_kinds(parsed);
        let symbol_by_name: HashMap<u64, usize> = self
            .symbols
            .iter()
            .enumerate()
            .map(|(index, symbol)| (symbol.name_span.bytes.start, index))
            .collect();
        let mut cursor = parsed.tree.root_node().walk();
        let mut found = Vec::new();
        let mut visible_from = Vec::new();
        // Nodes from the root to the parent of the current node; `Node::parent`
        // searches from the root on every call.
        let mut ancestors: Vec<Node<'_>> = Vec::new();
        loop {
            let node = cursor.node();
            if rules::is_identifier(node) {
                let start = to_u64(node.start_byte());
                let kind = definitions
                    .get(&start)
                    .copied()
                    .unwrap_or(OccurrenceKind::Use);
                let member = kind == OccurrenceKind::Use
                    && rules::is_member(node, cursor.field_name(), ancestors.last().copied());
                visible_from.push(match kind {
                    OccurrenceKind::Local => {
                        rules::sequential_declaration_end(&ancestors).map_or(start, to_u64)
                    }
                    _ => start,
                });
                found.push(self.occurrence(
                    parsed,
                    node,
                    kind,
                    member,
                    symbol_by_name.get(&start).copied(),
                ));
            }
            if cursor.goto_first_child() {
                ancestors.push(node);
                continue;
            }
            if !next_node(&mut cursor, &mut ancestors) {
                break;
            }
        }
        self.occurrences = found;
        self.visible_from = visible_from;
    }

    fn occurrence(
        &self,
        parsed: &ParsedSource<'_>,
        node: Node<'_>,
        kind: OccurrenceKind,
        member: bool,
        symbol: Option<usize>,
    ) -> Occurrence {
        let span = parsed.span(node);
        let symbol = symbol.filter(|_| kind == OccurrenceKind::Symbol);
        let scope = match symbol.and_then(|index| self.symbols.get(index)) {
            Some(info) => self.innermost_scope(info.span.bytes.start, info.span.bytes.end, true),
            None => self.innermost_scope(span.bytes.start, span.bytes.end, false),
        };
        let text = parsed.text(node);
        Occurrence {
            name: text.strip_prefix('$').unwrap_or(&text).to_owned(),
            span,
            kind,
            member,
            scope,
            binding: None,
            symbol,
        }
    }

    /// Kinds of the definitions of the file by the start of their name;
    /// symbol names take precedence over imports, imports over locals.
    fn definition_kinds(&self, parsed: &ParsedSource<'_>) -> HashMap<u64, OccurrenceKind> {
        let mut kinds: HashMap<u64, OccurrenceKind> = HashMap::new();
        if let Some(query) = parsed.grammar.locals() {
            let names = query.capture_names();
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(query, parsed.tree.root_node(), parsed.source);
            while let Some(found) = matches.next() {
                for capture in found.captures() {
                    let kind = match names.get(capture.index as usize).copied() {
                        Some("local.definition") => OccurrenceKind::Local,
                        Some("local.import") => OccurrenceKind::Import,
                        _ => continue,
                    };
                    let entry = kinds
                        .entry(to_u64(capture.node.start_byte()))
                        .or_insert(kind);
                    if kind == OccurrenceKind::Import {
                        *entry = kind;
                    }
                }
            }
        }
        for symbol in &self.symbols {
            kinds.insert(symbol.name_span.bytes.start, OccurrenceKind::Symbol);
        }
        kinds
    }

    fn bind(&mut self) {
        let keys: Vec<String> = self
            .occurrences
            .iter()
            .map(|occurrence| self.key_of(occurrence))
            .collect();
        let mut by_scope: HashMap<(usize, &str), Vec<usize>> = HashMap::new();
        for (index, occurrence) in self.occurrences.iter().enumerate() {
            if occurrence.is_definition() && !occurrence.member {
                by_scope
                    .entry((occurrence.scope, keys[index].as_str()))
                    .or_default()
                    .push(index);
            }
        }
        let members = self.member_definitions();
        let bindings: Vec<Option<usize>> = (0..self.occurrences.len())
            .map(|index| self.binding_of(index, &keys[index], &by_scope, &members))
            .collect();
        for (occurrence, binding) in self.occurrences.iter_mut().zip(bindings) {
            occurrence.binding = binding;
        }
    }

    fn key_of(&self, occurrence: &Occurrence) -> String {
        let variable = self.language.as_str() == "php"
            && self
                .source
                .get(usize::try_from(occurrence.span.bytes.start.saturating_sub(1)).unwrap_or(0))
                == Some(&b'$');
        if variable {
            format!("${}", occurrence.name)
        } else {
            occurrence.name.clone()
        }
    }

    /// Member symbols by name, with the index of their name occurrence.
    fn member_definitions(&self) -> HashMap<&str, Vec<usize>> {
        let mut members: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, occurrence) in self.occurrences.iter().enumerate() {
            let is_member_symbol = occurrence
                .symbol
                .and_then(|symbol| self.symbols.get(symbol))
                .is_some_and(|info| info.container.is_some() && is_member_kind(info.kind));
            if is_member_symbol {
                members
                    .entry(occurrence.name.as_str())
                    .or_default()
                    .push(index);
            }
        }
        members
    }

    fn binding_of(
        &self,
        index: usize,
        key: &str,
        by_scope: &HashMap<(usize, &str), Vec<usize>>,
        members: &HashMap<&str, Vec<usize>>,
    ) -> Option<usize> {
        let occurrence = &self.occurrences[index];
        if occurrence.is_definition() {
            return Some(index);
        }
        if occurrence.member {
            return match members.get(occurrence.name.as_str()) {
                Some(found) if found.len() == 1 => Some(found[0]),
                _ => None,
            };
        }
        let start = occurrence.span.bytes.start;
        let mut fallback = None;
        let mut scope = Some(occurrence.scope);
        while let Some(current) = scope {
            if let Some(candidates) = by_scope.get(&(current, key)) {
                let visible = candidates.iter().rev().find(|candidate| {
                    self.visible_from
                        .get(**candidate)
                        .is_some_and(|from| *from < start)
                });
                if let Some(found) = visible {
                    return Some(*found);
                }
                fallback = fallback.or(candidates.first().copied());
            }
            scope = self.scopes.get(current).and_then(|found| found.parent);
        }
        fallback
    }
}

fn is_member_kind(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Method | SymbolKind::Field | SymbolKind::Constant
    )
}

/// Moves the cursor to the next node in preorder after the subtree of the
/// current node; `false` at the end of the tree.
pub(crate) fn advance(cursor: &mut tree_sitter::TreeCursor<'_>) -> bool {
    loop {
        if cursor.goto_next_sibling() {
            return true;
        }
        if !cursor.goto_parent() {
            return false;
        }
    }
}

/// Index of the symbol whose definition directly contains each symbol.
///
/// The symbols are sorted by start, larger definitions first, and nest like
/// syntax nodes.
fn nesting_parents(symbols: &[SymbolInfo]) -> Vec<Option<usize>> {
    let mut parents = Vec::with_capacity(symbols.len());
    let mut open: Vec<usize> = Vec::new();
    for (index, symbol) in symbols.iter().enumerate() {
        let bytes = symbol.span.bytes;
        while let Some(top) = open.last() {
            let outer = symbols[*top].span.bytes;
            if outer.start <= bytes.start && bytes.end <= outer.end {
                break;
            }
            open.pop();
        }
        parents.push(open.last().copied());
        open.push(index);
    }
    parents
}

/// Like [`advance`], and drops from `ancestors` every node the cursor leaves.
fn next_node<'t>(cursor: &mut tree_sitter::TreeCursor<'t>, ancestors: &mut Vec<Node<'t>>) -> bool {
    loop {
        if cursor.goto_next_sibling() {
            return true;
        }
        if !cursor.goto_parent() {
            return false;
        }
        ancestors.pop();
    }
}

fn line_starts(source: &[u8]) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            source
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .map(|(at, _)| at + 1),
        )
        .collect()
}

/// Scopes from the scope query, ordered so that every scope follows the
/// scopes that contain it; index 0 is the whole file.
fn collect_scopes(parsed: &ParsedSource<'_>) -> Vec<Scope> {
    let file = ByteRange::new(
        0,
        to_u64(parsed.source.len()).max(to_u64(parsed.tree.root_node().end_byte())),
    );
    let mut ranges: Vec<ByteRange> = Vec::new();
    if let Some(query) = parsed.grammar.locals() {
        let names = query.capture_names();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, parsed.tree.root_node(), parsed.source);
        while let Some(found) = matches.next() {
            for capture in found.captures() {
                if names.get(capture.index as usize).copied() == Some("local.scope") {
                    ranges.push(ByteRange::new(
                        to_u64(capture.node.start_byte()),
                        to_u64(capture.node.end_byte()),
                    ));
                }
            }
        }
    }
    ranges.sort_by_key(|range| (range.start, std::cmp::Reverse(range.end)));
    ranges.dedup();
    let mut scopes = vec![Scope {
        bytes: file,
        parent: None,
    }];
    let mut open: Vec<usize> = vec![0];
    for range in ranges {
        if range == file {
            continue;
        }
        while let Some(&top) = open.last() {
            let outer = scopes[top].bytes;
            if outer.start <= range.start && range.end <= outer.end {
                break;
            }
            open.pop();
        }
        scopes.push(Scope {
            bytes: range,
            parent: Some(open.last().copied().unwrap_or(0)),
        });
        open.push(scopes.len() - 1);
    }
    scopes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analysis<'a>(language: &str, source: &'a str) -> LocalAnalysis<'a> {
        analyze(
            Registry::builtin(),
            source.as_bytes(),
            &LanguageId::new(language).unwrap(),
        )
        .unwrap()
    }

    fn lines_of(occurrences: &[&Occurrence]) -> Vec<u32> {
        occurrences
            .iter()
            .map(|item| item.span.start.line)
            .collect()
    }

    #[test]
    fn shadowing_parameter_is_bound_to_its_own_scope() {
        let source = "x = 1\ndef f(x):\n    return x\nprint(x)\n";
        let analysis = analysis("python", source);
        let offset = offset_of(
            source.as_bytes(),
            TextPosition {
                line: 3,
                column: 12,
            },
        )
        .unwrap();
        let inner = analysis.occurrence_at(to_u64(offset)).unwrap();
        assert_eq!(lines_of(&analysis.bound_to(inner)), [2, 3]);
        let outer = analysis.occurrence_at(0).unwrap();
        assert_eq!(lines_of(&analysis.bound_to(outer)), [1, 4]);
    }

    #[test]
    fn function_used_before_its_definition_is_bound() {
        let source = "function main() { return helper(); }\nfunction helper() { return 1; }\n";
        let analysis = analysis("javascript", source);
        let call = analysis.occurrence_at(25).unwrap();
        assert_eq!(call.name, "helper");
        let definition = analysis.definition_of(call).unwrap();
        assert_eq!(definition.kind, OccurrenceKind::Symbol);
        assert_eq!(definition.span.start.line, 2);
    }

    #[test]
    fn member_names_do_not_bind_to_variables() {
        let source = "package p\n\ntype T struct{ size int }\n\nfunc f(t T, size int) int { return t.size + size }\n";
        let analysis = analysis("go", source);
        let members: Vec<&Occurrence> = analysis
            .occurrences()
            .iter()
            .filter(|item| item.name == "size" && item.member)
            .collect();
        assert_eq!(members.len(), 1);
        let field = analysis.definition_of(members[0]).unwrap();
        assert_eq!(field.span.start.line, 3);
        let plain: Vec<&Occurrence> = analysis
            .occurrences()
            .iter()
            .filter(|item| item.name == "size" && !item.member && item.kind == OccurrenceKind::Use)
            .collect();
        assert_eq!(
            analysis.definition_of(plain[0]).unwrap().kind,
            OccurrenceKind::Local
        );
    }

    #[test]
    fn imports_have_the_import_role() {
        let source = "import os\nfrom pathlib import Path\nPath(os.sep)\n";
        let analysis = analysis("python", source);
        let found = analysis.references("Path");
        assert_eq!(found[0].role, ReferenceRole::Import);
        assert_eq!(found[1].role, ReferenceRole::Usage);
        assert_eq!(found[1].line_text, "Path(os.sep)");
    }

    #[test]
    fn php_variables_are_apart_from_functions() {
        let source = "<?php\nfunction total() { return 1; }\n$total = total();\necho $total;\n";
        let analysis = analysis("php", source);
        let variable = analysis
            .occurrences()
            .iter()
            .find(|item| item.name == "total" && item.span.start.line == 4)
            .unwrap();
        let definition = analysis.definition_of(variable).unwrap();
        assert_eq!(definition.span.start.line, 3);
        assert_eq!(definition.kind, OccurrenceKind::Local);
    }

    #[test]
    fn words_in_strings_are_whole_words_only() {
        let source = "let item = \"item items\";\n";
        let analysis = analysis("typescript", source);
        let found = analysis.references("item");
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].class, SpanClass::String);
    }

    #[test]
    fn positions_and_lines_follow_the_source() {
        let source = "a\néb c\n";
        let analysis = analysis("python", source);
        let offset = offset_of(source.as_bytes(), TextPosition { line: 2, column: 4 }).unwrap();
        assert_eq!(
            analysis.position(to_u64(offset)),
            TextPosition { line: 2, column: 4 }
        );
        assert_eq!(analysis.line_text(to_u64(offset)), "éb c");
        assert_eq!(analysis.line_text(0), "a");
    }

    #[test]
    fn offsets_follow_lines_and_scalar_value_columns() {
        let source = "é = 1\r\nλx\n\nlast".as_bytes();
        let at = |line, column| offset_of(source, TextPosition { line, column });
        assert_eq!(at(1, 1), Some(0));
        assert_eq!(at(1, 2), Some(2));
        assert_eq!(at(2, 2), Some(10));
        assert_eq!(at(2, 3), Some(11));
        assert_eq!(at(2, 50), Some(11));
        assert_eq!(at(3, 1), Some(12));
        assert_eq!(at(4, 3), Some(15));
        assert_eq!(at(4, 99), Some(17));
        assert_eq!(at(5, 1), None);
        assert_eq!(at(0, 1), None);
        assert_eq!(offset_of(b"", TextPosition { line: 1, column: 1 }), Some(0));
        assert_eq!(offset_of(b"", TextPosition { line: 2, column: 1 }), None);
    }

    #[test]
    fn offsets_and_positions_round_trip_on_multibyte_crlf_text() {
        let source = "a = \"é\"\r\nb = \"λλ\"; c = 1\r\n";
        let analysis = analysis("python", source);
        for (line, column) in [(1, 1), (1, 6), (2, 11), (2, 16)] {
            let offset = offset_of(source.as_bytes(), TextPosition { line, column }).unwrap();
            let position = analysis.position(to_u64(offset));
            assert_eq!((position.line, position.column), (line, column));
        }
        let c = source.find("c = 1").unwrap();
        assert_eq!(analysis.line_text(to_u64(c)), "b = \"λλ\"; c = 1");
        let found = analysis.references("c");
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].span.start.line, found[0].span.start.column),
            (2, 11)
        );
    }

    fn brute_force_scope(
        analysis: &LocalAnalysis<'_>,
        start: u64,
        end: u64,
        exclude_same: bool,
    ) -> usize {
        analysis
            .scopes
            .iter()
            .enumerate()
            .filter(|(_, scope)| {
                scope.bytes.start <= start
                    && end <= scope.bytes.end
                    && !(exclude_same && scope.bytes.start == start && scope.bytes.end == end)
            })
            .min_by_key(|(_, scope)| scope.bytes.len())
            .map_or(0, |(index, _)| index)
    }

    #[test]
    fn innermost_scope_agrees_with_a_scan_of_every_scope() {
        let sources = [
            (
                "rust",
                "fn a(x: u32) { let y = x; { let z = y; for i in 0..z { let w = i; } } }\nfn b() { let c = |q| q + 1; c(2); }\nmod m { fn n() { match 1 { k => k, } } }\n",
            ),
            (
                "python",
                "def f(a):\n    def g(b):\n        return [c for c in b]\n    return g(a)\n\nclass K:\n    def m(self): pass\n",
            ),
            (
                "typescript",
                "function f(a: number) { const g = (b: number) => { return b + a; }; if (a) { let d = 1; } }\nclass C { m(x: number) { return x; } }\n",
            ),
        ];
        for (language, source) in sources {
            let analysis = analysis(language, source);
            assert!(analysis.scopes().len() > 3, "{language}");
            for occurrence in analysis.occurrences() {
                let (start, end) = (occurrence.span.bytes.start, occurrence.span.bytes.end);
                for exclude_same in [false, true] {
                    assert_eq!(
                        analysis.innermost_scope(start, end, exclude_same),
                        brute_force_scope(&analysis, start, end, exclude_same),
                        "{language} {} @{start}",
                        occurrence.name
                    );
                }
            }
            for scope in analysis.scopes() {
                let (start, end) = (scope.bytes.start, scope.bytes.end);
                for exclude_same in [false, true] {
                    assert_eq!(
                        analysis.innermost_scope(start, end, exclude_same),
                        brute_force_scope(&analysis, start, end, exclude_same),
                        "{language} scope {start}..{end}"
                    );
                }
            }
        }
    }

    #[test]
    fn php_variables_inside_strings_are_references_in_code() {
        let source = "<?php\n$name = 1;\necho \"hi $name and {$name}\";\n";
        let found = analysis("php", source).references("name");
        let in_code = found
            .iter()
            .filter(|reference| reference.class == SpanClass::Code)
            .count();
        assert_eq!(in_code, 3);
        assert!(
            found
                .iter()
                .all(|reference| reference.class != SpanClass::String)
        );
    }

    #[test]
    fn container_search_agrees_with_a_scan_of_every_symbol() {
        let sources = [
            (
                "rust",
                "mod m { struct S; impl S { fn a(&self) { fn b() {} b(); } fn c(&self) {} } }\nfn d() { m::S.c(); }\nconst K: u32 = 1;\n",
            ),
            (
                "python",
                "class A:\n    x = 1\n    def f(self):\n        def g():\n            return 1\n        return g()\n\nclass B:\n    def h(self): pass\n",
            ),
            (
                "typescript",
                "class C { f = 1; m(x: number) { const g = () => 1; return g(); } }\nfunction top() { return 2; }\n",
            ),
        ];
        for (language, source) in sources {
            let analysis = analysis(language, source);
            assert!(analysis.symbols().len() > 3, "{language}");
            for offset in 0..=source.len() as u64 {
                let expected = analysis
                    .symbols()
                    .iter()
                    .filter(|symbol| {
                        symbol.span.bytes.start <= offset
                            && offset < symbol.span.bytes.end
                            && symbol.name_span.bytes.start != offset
                    })
                    .min_by_key(|symbol| symbol.span.bytes.len())
                    .map(|symbol| qualified_name(&symbol.name, symbol.container.as_deref()));
                assert_eq!(
                    analysis.container_at(offset),
                    expected,
                    "{language} {offset}"
                );
            }
        }
    }

    #[test]
    fn long_member_chains_are_analysed_in_linear_time() {
        let depth = 20_000;
        let source = format!("x{}\n", ".a".repeat(depth));
        let analysis = analysis("python", &source);
        let members = analysis
            .occurrences()
            .iter()
            .filter(|occurrence| occurrence.member)
            .count();
        assert_eq!(members, depth);
    }

    #[test]
    fn scopes_nest_inside_the_file() {
        let analysis = analysis("rust", "fn f() { let a = 1; { let b = a; } }\n");
        assert!(analysis.scopes().len() >= 2);
        assert!(
            analysis.scopes()[1..]
                .iter()
                .all(|scope| scope.parent.is_some())
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
