// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Structural patterns with metavariables.
//!
//! A pattern is source code of the target language in which metavariables
//! stand for syntax nodes:
//!
//! | Metavariable | Matches |
//! |---|---|
//! | `$NAME` | one named node, captured as `NAME` |
//! | `$$NAME` | one node, named or not (for example an operator), captured as `NAME` |
//! | `$$$NAME` | zero or more sibling nodes, captured as `NAME` |
//! | `$_` | one named node, not captured |
//! | `$$$` | zero or more sibling nodes, not captured |
//!
//! Names consist of upper-case ASCII letters, digits and `_` and start with a
//! letter. A name used twice matches only nodes with the same structure, so
//! `$A == $A` finds comparisons of a value with itself. Whitespace and
//! comments are not significant; the kind and the text of every other leaf
//! must agree.
//!
//! The pattern is parsed with the grammar of the target language. A pattern
//! that does not parse fails with `E_PATTERN_SYNTAX` and the position of the
//! first syntax error. Matching uses `ast-grep-core`.
//!
//! ```
//! use nurt_code::pattern::StructuralPattern;
//! use nurt_contracts::services::LanguageId;
//!
//! let rust = LanguageId::new("rust")?;
//! let grammar = tree_sitter::Language::new(tree_sitter_rust::LANGUAGE);
//! let pattern = StructuralPattern::compile("$A.unwrap()", &rust, &grammar)?;
//! let found = pattern.search(b"fn f() { let x = load().unwrap(); }")?;
//! assert_eq!(found.len(), 1);
//! assert_eq!(found[0].captures[0].text, "load()");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::any::Any;
use std::borrow::Cow;
use std::fmt;

use ast_grep_core::matcher::{PatternBuilder, PatternNode};
use ast_grep_core::meta_var::MetaVariable;
use ast_grep_core::tree_sitter::{LanguageExt, StrDoc, TSLanguage};
use ast_grep_core::{AstGrep, Language, Node, Pattern, PatternError};
use indexmap::IndexMap;
use nurt_contracts::error::{At, Error, ErrorCode, Result};
use nurt_contracts::record::Node as NodeRecord;
use nurt_contracts::services::{LanguageId, SyntaxCapture, SyntaxMatch, SyntaxPattern};
use nurt_contracts::value::Path as PathValue;

use crate::spans::LineIndex;

/// Sigil of metavariables as written by the user.
const SIGIL: char = '$';

/// Replacement of the sigil for grammars in which `$` cannot start an
/// identifier; the micro sign is a letter in the identifier rules of these
/// languages.
const MICRO_SIGN: char = '\u{b5}';

/// Languages in which `$NAME` already parses as an identifier, a variable
/// or plain text.
const DOLLAR_LANGUAGES: [&str; 10] = [
    "javascript",
    "typescript",
    "tsx",
    "java",
    "bash",
    "powershell",
    "json",
    "jsonc",
    "yaml",
    "markdown",
];

/// Languages whose names accept `_` but not the micro sign.
const UNDERSCORE_LANGUAGES: [&str; 3] = ["css", "toml", "xml"];

/// Text placed before a PHP pattern, which parses as code only after the
/// opening tag.
const PHP_OPEN_TAG: &str = "<?php\n";

/// Letter used for metavariables in HTML, where names may contain neither
/// `$` nor the micro sign in every position.
const HTML_SIGIL: char = 'z';

/// Grammar of a pattern, in the form `ast-grep-core` works with.
#[derive(Clone)]
struct PatternLanguage {
    grammar: TSLanguage,
    expando: char,
}

impl Language for PatternLanguage {
    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
        substitute(query, self.expando)
    }

    fn meta_var_char(&self) -> char {
        SIGIL
    }

    fn expando_char(&self) -> char {
        self.expando
    }

    fn kind_to_id(&self, kind: &str) -> u16 {
        self.grammar.id_for_node_kind(kind, true)
    }

    fn field_to_id(&self, field: &str) -> Option<u16> {
        self.grammar.field_id_for_name(field).map(|id| id.get())
    }

    fn build_pattern(&self, builder: &PatternBuilder<'_>) -> std::result::Result<Pattern, PatternError> {
        builder.build(|source| StrDoc::try_new(source, self.clone()))
    }
}

impl LanguageExt for PatternLanguage {
    fn get_ts_language(&self) -> TSLanguage {
        self.grammar.clone()
    }
}

/// Character that stands for `$` in the parsed form of a pattern.
fn expando_for(language: &LanguageId) -> char {
    let id = language.as_str();
    if DOLLAR_LANGUAGES.contains(&id) {
        SIGIL
    } else if UNDERSCORE_LANGUAGES.contains(&id) {
        '_'
    } else if id == "html" {
        HTML_SIGIL
    } else {
        MICRO_SIGN
    }
}

/// Replaces the sigil of every metavariable by `expando`; a `$` that does
/// not start a metavariable is kept. Each sigil is replaced by exactly one
/// character, so character positions of the result are positions in the
/// pattern as written.
fn substitute(pattern: &str, expando: char) -> Cow<'_, str> {
    if expando == SIGIL || !pattern.contains(SIGIL) {
        return Cow::Borrowed(pattern);
    }
    let mut out = String::with_capacity(pattern.len());
    let mut dollars = 0_usize;
    for c in pattern.chars() {
        if c == SIGIL {
            dollars += 1;
            continue;
        }
        let starts_name = c.is_ascii_uppercase() || c == '_';
        let sigil = if starts_name || dollars == 3 {
            expando
        } else {
            SIGIL
        };
        out.extend(std::iter::repeat_n(sigil, dollars));
        dollars = 0;
        out.push(c);
    }
    let sigil = if dollars == 3 { expando } else { SIGIL };
    out.extend(std::iter::repeat_n(sigil, dollars));
    Cow::Owned(out)
}

/// Metavariable of a compiled pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Metavariable {
    name: String,
    multiple: bool,
}

/// Compiled structural pattern of one language.
#[derive(Clone)]
pub struct StructuralPattern {
    language: LanguageId,
    source: String,
    metavariables: Vec<Metavariable>,
    pattern: Pattern,
    grammar: PatternLanguage,
}

impl fmt::Debug for StructuralPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StructuralPattern")
            .field("language", &self.language)
            .field("source", &self.source)
            .field("metavariables", &self.metavariables)
            .finish_non_exhaustive()
    }
}

impl StructuralPattern {
    /// Compiles a pattern written in the syntax of `language`, parsed with
    /// `grammar`.
    ///
    /// The pattern is one syntax node of the language, for example an
    /// expression, a statement or a declaration, with metavariables in place
    /// of nodes. A pattern with several top-level nodes, an empty pattern, a
    /// pattern that is only `$$$NAME` and a pattern with a syntax error fail
    /// with `E_PATTERN_SYNTAX`; the error has the language in its details
    /// and the position of the problem in `at`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_code::pattern::StructuralPattern;
    /// use nurt_contracts::services::LanguageId;
    ///
    /// let python = LanguageId::new("python")?;
    /// let grammar = tree_sitter::Language::new(tree_sitter_python::LANGUAGE);
    /// let error = StructuralPattern::compile("print($A", &python, &grammar).unwrap_err();
    /// assert_eq!(error.code.as_str(), "E_PATTERN_SYNTAX");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn compile(pattern: &str, language: &LanguageId, grammar: &TSLanguage) -> Result<Self> {
        if pattern.trim().is_empty() {
            return Err(pattern_error(
                language,
                "pattern is empty; write a construct of the language with metavariables such as `$X`",
                pattern,
                0,
            ));
        }
        let lang = PatternLanguage {
            grammar: grammar.clone(),
            expando: expando_for(language),
        };
        let processed = substitute(pattern, lang.expando);
        let prefix = if language.as_str() == "php" && !pattern.trim_start().starts_with("<?php") {
            PHP_OPEN_TAG
        } else {
            ""
        };
        let full = format!("{prefix}{processed}");
        let root = AstGrep::<StrDoc<PatternLanguage>>::try_new(&full, lang.clone())
            .map_err(|message| internal_parse_error(language, &message))?;
        if let Some(offset) = first_error(root.root().get_inner_node()) {
            let at = full[..offset.min(full.len())].chars().count() - prefix.chars().count();
            return Err(pattern_error(
                language,
                &format!(
                    "pattern is not valid {language} code; correct the syntax at the reported position"
                ),
                pattern,
                at,
            ));
        }
        let node = pattern_root(&root, prefix.len(), language, pattern)?;
        let compiled = Pattern::from(node);
        if let PatternNode::MetaVar {
            meta_var: MetaVariable::MultiCapture(_) | MetaVariable::Multiple,
        } = &compiled.node
        {
            return Err(pattern_error(
                language,
                "a pattern cannot consist of `$$$` alone; use `$X` or place `$$$X` inside a larger construct",
                pattern,
                0,
            ));
        }
        let mut metavariables = Vec::new();
        collect_metavariables(&compiled.node, &mut metavariables);
        Ok(Self {
            language: language.clone(),
            source: pattern.to_owned(),
            metavariables,
            pattern: compiled,
            grammar: lang,
        })
    }

    /// Matches of the pattern in UTF-8 source code, in source order.
    ///
    /// A match inside another match is reported as well.
    ///
    /// # Errors
    ///
    /// Fails with `E_ENCODING` when the source is not valid UTF-8 and with
    /// `E_INTERNAL` when the parser produces no tree.
    pub fn search(&self, source: &[u8]) -> Result<Vec<SyntaxMatch>> {
        let text = std::str::from_utf8(source).map_err(|error| {
            Error::new(
                ErrorCode::Encoding,
                format!(
                    "source is not valid UTF-8 at byte {}; convert the file to UTF-8 for a syntax search",
                    error.valid_up_to()
                ),
            )
        })?;
        let root = AstGrep::<StrDoc<PatternLanguage>>::try_new(text, self.grammar.clone())
            .map_err(|message| internal_parse_error(&self.language, &message))?;
        let lines = LineIndex::new(source);
        let top = root.root();
        let found = top
            .find_all(&self.pattern)
            .map(|found| {
                let node = found.get_node();
                let captures = self
                    .metavariables
                    .iter()
                    .filter_map(|var| capture(found.get_env(), var, &lines, source))
                    .collect();
                SyntaxMatch {
                    span: lines.span(node.range().start, node.range().end),
                    node_kind: node.kind().into_owned(),
                    text: node.text().into_owned(),
                    captures,
                }
            })
            .collect();
        Ok(found)
    }
}

impl SyntaxPattern for StructuralPattern {
    fn language(&self) -> &LanguageId {
        &self.language
    }

    fn source(&self) -> &str {
        &self.source
    }

    fn metavariables(&self) -> Vec<String> {
        self.metavariables
            .iter()
            .map(|var| var.name.clone())
            .collect()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

type PatternDoc = StrDoc<PatternLanguage>;

fn capture(
    env: &ast_grep_core::meta_var::MetaVarEnv<'_, PatternDoc>,
    var: &Metavariable,
    lines: &LineIndex<'_>,
    source: &[u8],
) -> Option<SyntaxCapture> {
    let nodes: Vec<Node<'_, PatternDoc>> = if var.multiple {
        env.get_multiple_matches(&var.name)
    } else {
        vec![env.get_match(&var.name)?.clone()]
    };
    let (start, end) = match (nodes.first(), nodes.last()) {
        (Some(first), Some(last)) => (first.range().start, last.range().end),
        _ => (0, 0),
    };
    let text = source
        .get(start..end)
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let spans = nodes
        .iter()
        .filter(|node| node.is_named() || !var.multiple)
        .map(|node| lines.span(node.range().start, node.range().end))
        .collect();
    Some(SyntaxCapture {
        name: var.name.clone(),
        spans,
        text,
    })
}

/// Node of the parsed pattern that becomes the pattern: the only top-level
/// node, descended through nodes with a single child.
fn pattern_root<'r>(
    root: &'r AstGrep<PatternDoc>,
    prefix_len: usize,
    language: &LanguageId,
    pattern: &str,
) -> Result<Node<'r, PatternDoc>> {
    let top = root.root();
    let content: Vec<Node<'r, PatternDoc>> = top
        .children()
        .filter(|child| child.range().start >= prefix_len && !child.is_missing())
        .filter(|child| !child.kind().is_empty())
        .collect();
    let mut node = match content.as_slice() {
        [] => {
            return Err(pattern_error(
                language,
                "pattern contains no syntax node; write a construct of the language",
                pattern,
                0,
            ));
        }
        [only] => only.clone(),
        [_, second, ..] => {
            let offset = second.range().start - prefix_len;
            let at = root.get_text()[prefix_len..prefix_len + offset]
                .chars()
                .count();
            return Err(pattern_error(
                language,
                "pattern has several top-level nodes; write a single construct or enclose the nodes in one",
                pattern,
                at,
            ));
        }
    };
    while is_single(&node) {
        let Some(child) = node.child(0) else {
            break;
        };
        node = child;
    }
    Ok(node)
}

/// A node with one child, or with one child followed by a missing or empty
/// node the grammar inserts after some statements.
fn is_single(node: &Node<'_, PatternDoc>) -> bool {
    match node.children().len() {
        1 => true,
        2 => node
            .child(1)
            .is_some_and(|second| second.is_missing() || second.kind().is_empty()),
        _ => false,
    }
}

/// Byte offset of the first `ERROR` node in document order.
fn first_error(node: tree_sitter::Node<'_>) -> Option<usize> {
    if !node.has_error() {
        return None;
    }
    if node.is_error() {
        return Some(node.start_byte());
    }
    let mut cursor = node.walk();
    let children: Vec<tree_sitter::Node<'_>> = node.children(&mut cursor).collect();
    children.into_iter().find_map(first_error)
}

fn collect_metavariables(node: &PatternNode, out: &mut Vec<Metavariable>) {
    match node {
        PatternNode::MetaVar { meta_var } => {
            let var = match meta_var {
                MetaVariable::Capture(name, _) => Metavariable {
                    name: name.clone(),
                    multiple: false,
                },
                MetaVariable::MultiCapture(name) => Metavariable {
                    name: name.clone(),
                    multiple: true,
                },
                MetaVariable::Dropped(_) | MetaVariable::Multiple => return,
            };
            if !out.iter().any(|known| known.name == var.name) {
                out.push(var);
            }
        }
        PatternNode::Terminal { .. } => {}
        PatternNode::Internal { children, .. } => {
            for child in children {
                collect_metavariables(child, out);
            }
        }
    }
}

/// `E_PATTERN_SYNTAX` at the character `at` of the pattern.
fn pattern_error(language: &LanguageId, message: &str, pattern: &str, at: usize) -> Error {
    let (mut line, mut column) = (1_u32, 1_u32);
    for c in pattern.chars().take(at) {
        if c == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    let found: String = pattern.chars().skip(at).take_while(|c| *c != '\n').collect();
    Error::new(ErrorCode::PatternSyntax, message)
        .with_at(At::new(line, column, 1))
        .with_found(found)
        .with_detail("language", serde_json::json!(language.as_str()))
}

fn internal_parse_error(language: &LanguageId, message: &str) -> Error {
    Error::new(
        ErrorCode::Internal,
        format!("parser of `{language}` failed: {message}"),
    )
}

/// `node` record of a match.
///
/// `path` is the file in canonical form and `etag` its entity tag at the
/// time of the search.
///
/// # Examples
///
/// ```
/// use nurt_code::pattern::{StructuralPattern, node_record};
/// use nurt_contracts::services::LanguageId;
///
/// let go = LanguageId::new("go")?;
/// let grammar = tree_sitter::Language::new(tree_sitter_go::LANGUAGE);
/// let pattern = StructuralPattern::compile("fmt.Println($$$ARGS)", &go, &grammar)?;
/// let found = pattern.search(b"package main\nfunc main() { fmt.Println(1, 2) }\n")?;
/// let node = node_record("/src/main.go", &go, "e1", &found[0]);
/// assert_eq!(node.line, 2);
/// assert_eq!(node.captures.unwrap()["ARGS"], "1, 2");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn node_record(path: &str, language: &LanguageId, etag: &str, found: &SyntaxMatch) -> NodeRecord {
    let captures: IndexMap<String, String> = found
        .captures
        .iter()
        .map(|capture| (capture.name.clone(), capture.text.clone()))
        .collect();
    NodeRecord {
        path: PathValue::from(path),
        language: language.to_string(),
        kind: found.node_kind.clone(),
        start: found.span.bytes.start,
        end: found.span.bytes.end,
        line: u64::from(found.span.start.line),
        column: u64::from(found.span.start.column),
        end_line: u64::from(found.span.end.line),
        text: found.text.clone(),
        etag: etag.to_owned(),
        end_column: Some(u64::from(found.span.end.column)),
        captures: (!captures.is_empty()).then_some(captures),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitution_replaces_only_metavariable_sigils() {
        assert_eq!(substitute("$A + $b", MICRO_SIGN), "\u{b5}A + $b");
        assert_eq!(substitute("f($$$ARGS)", MICRO_SIGN), "f(\u{b5}\u{b5}\u{b5}ARGS)");
        assert_eq!(substitute("$$$", MICRO_SIGN), "\u{b5}\u{b5}\u{b5}");
        assert_eq!(substitute("$$OP", MICRO_SIGN), "\u{b5}\u{b5}OP");
        assert_eq!(substitute("cost $5", MICRO_SIGN), "cost $5");
        assert_eq!(substitute("$A", SIGIL), "$A");
    }

    #[test]
    fn sigil_follows_the_identifier_rules_of_the_language() {
        let id = |name: &str| LanguageId::new(name).unwrap();
        assert_eq!(expando_for(&id("javascript")), SIGIL);
        assert_eq!(expando_for(&id("rust")), MICRO_SIGN);
        assert_eq!(expando_for(&id("css")), '_');
        assert_eq!(expando_for(&id("html")), HTML_SIGIL);
    }

    #[test]
    fn error_positions_count_lines_and_characters() {
        let id = LanguageId::new("rust").unwrap();
        let error = pattern_error(&id, "m", "a\nb\u{e9}c", 4);
        assert_eq!(error.at, Some(At::new(2, 3, 1)));
        assert_eq!(error.found.as_deref(), Some("c"));
    }
}
