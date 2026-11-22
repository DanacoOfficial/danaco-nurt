// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Language registry and language detection.
//!
//! A [`Registry`] holds the languages of one or more [`GrammarProvider`]s:
//! the built-in grammars ([`crate::grammars::BuiltinGrammars`]) and grammars
//! supplied by extensions. It detects the language of a file, parses source
//! text and keeps the compiled queries of every grammar.
//!
//! # Queries of a grammar
//!
//! The symbol query ([`Grammar::tags`]) marks every definition with a
//! capture `@definition.<kind>`, where `<kind>` is a value of the `kind`
//! parameter of `code.symbols` (`function`, `method`, `class`, `struct`,
//! `interface`, `enum`, `variable`, `constant`, `field`, `module`, `type`,
//! `macro`), and its name with `@name`. A capture `@container.name` names the
//! container of a definition written outside it (a Go method receiver, a
//! C++ `Type::method`). A capture `@scope` with `@name` marks a container
//! that is not a symbol itself, such as a Rust `impl` block. When a pattern
//! matches the same definition as an earlier pattern of the query, the
//! earlier pattern wins.
//!
//! The scope query ([`Grammar::locals`]) marks scopes with `@local.scope`,
//! local definitions with `@local.definition` and names introduced by
//! imports with `@local.import`.

mod detect;

use std::borrow::Cow;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{ByteRange, LanguageId, LanguageInfo, TextPosition, TextSpan};
use tree_sitter::{Language, Node, Parser, Point, Query, Tree};

use crate::grammars::BuiltinGrammars;

pub use detect::{detect_language, interpreter_of_shebang, modeline_language};

/// Description of a language: identifier, names and file associations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageSpec {
    /// Identifier.
    pub id: LanguageId,
    /// Display name.
    pub name: String,
    /// File name extensions without the leading period, in lower case.
    pub extensions: Vec<String>,
    /// Whole file names (`Gemfile`, `Cargo.lock`).
    pub file_names: Vec<String>,
    /// Interpreter names of shebang lines (`python`, `node`), without
    /// version suffixes.
    pub interpreters: Vec<String>,
    /// Other names of the language in editor modelines and in the
    /// `language` parameter (`py`, `ts`, `sh`), in lower case.
    pub aliases: Vec<String>,
    /// Version of the grammar.
    pub grammar_version: String,
    /// A programming language, as opposed to a data or markup format;
    /// repository-wide symbol searches cover programming languages unless a
    /// language is named.
    pub programming: bool,
}

/// Grammar of one language and its queries.
#[derive(Clone)]
pub struct Grammar {
    /// Compiled tree-sitter grammar.
    pub language: Language,
    /// Symbol query, described in the module documentation.
    pub tags: Option<Cow<'static, str>>,
    /// Scope query, described in the module documentation.
    pub locals: Option<Cow<'static, str>>,
}

impl fmt::Debug for Grammar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Grammar")
            .field("abi_version", &self.language.abi_version())
            .field("tags", &self.tags.is_some())
            .field("locals", &self.locals.is_some())
            .finish()
    }
}

/// Source of grammars: the built-in set or an extension.
pub trait GrammarProvider: Send + Sync {
    /// Languages of the provider.
    fn languages(&self) -> Vec<LanguageSpec>;

    /// Grammar of a language listed by [`GrammarProvider::languages`];
    /// `None` when the provider cannot load it.
    fn grammar(&self, id: &LanguageId) -> Option<Grammar>;
}

/// Grammar with compiled queries, shared by every analysis of the language.
pub struct LoadedGrammar {
    language: Language,
    tags: Option<Query>,
    locals: Option<Query>,
}

impl LoadedGrammar {
    /// Compiled tree-sitter grammar.
    #[must_use]
    pub fn language(&self) -> &Language {
        &self.language
    }

    /// Compiled symbol query.
    #[must_use]
    pub fn tags(&self) -> Option<&Query> {
        self.tags.as_ref()
    }

    /// Compiled scope query.
    #[must_use]
    pub fn locals(&self) -> Option<&Query> {
        self.locals.as_ref()
    }
}

impl fmt::Debug for LoadedGrammar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadedGrammar")
            .field("tags", &self.tags.is_some())
            .field("locals", &self.locals.is_some())
            .finish()
    }
}

type LoadResult = std::result::Result<Arc<LoadedGrammar>, (ErrorCode, String)>;

struct Entry {
    spec: LanguageSpec,
    provider: Arc<dyn GrammarProvider>,
    loaded: OnceLock<LoadResult>,
}

/// Parsed source text with its language.
pub struct ParsedSource<'a> {
    /// Syntax tree.
    pub tree: Tree,
    /// Grammar and queries of the language.
    pub grammar: Arc<LoadedGrammar>,
    /// The parsed text.
    pub source: &'a [u8],
    /// Language.
    pub language: LanguageId,
}

impl fmt::Debug for ParsedSource<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedSource")
            .field("language", &self.language)
            .field("bytes", &self.source.len())
            .finish_non_exhaustive()
    }
}

impl ParsedSource<'_> {
    /// Text of a node; invalid UTF-8 is replaced.
    #[must_use]
    pub fn text(&self, node: Node<'_>) -> Cow<'_, str> {
        let bytes = self
            .source
            .get(node.start_byte()..node.end_byte())
            .unwrap_or_default();
        String::from_utf8_lossy(bytes)
    }

    /// Region of a node.
    #[must_use]
    pub fn span(&self, node: Node<'_>) -> TextSpan {
        TextSpan {
            bytes: ByteRange::new(to_u64(node.start_byte()), to_u64(node.end_byte())),
            start: self.position(node.start_byte(), node.start_position()),
            end: self.position(node.end_byte(), node.end_position()),
        }
    }

    /// Position of a byte offset whose tree-sitter point is `point`: the
    /// column counts Unicode scalar values from the start of the line.
    #[must_use]
    pub fn position(&self, byte: usize, point: Point) -> TextPosition {
        let line_start = byte.saturating_sub(point.column);
        TextPosition {
            line: to_u32(point.row).saturating_add(1),
            column: to_u32(scalar_count(self.source.get(line_start..byte).unwrap_or_default()))
                .saturating_add(1),
        }
    }

    /// The whole line holding the byte offset, without the line ending.
    #[must_use]
    pub fn line_text(&self, byte: usize) -> String {
        let byte = byte.min(self.source.len());
        let start = self.source[..byte]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |at| at + 1);
        let end = self.source[byte..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(self.source.len(), |at| byte + at);
        let line = String::from_utf8_lossy(&self.source[start..end]);
        line.trim_end_matches('\r').to_owned()
    }
}

/// Number of Unicode scalar values in UTF-8 text; bytes of invalid
/// sequences count as one value each lead byte.
fn scalar_count(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| (b & 0xC0) != 0x80).count()
}

pub(crate) fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

pub(crate) fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Languages known to the code group.
pub struct Registry {
    entries: Vec<Entry>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|entry| entry.spec.id.as_str()))
            .finish()
    }
}

impl Registry {
    /// Registry without languages.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Registry of the built-in grammars, shared by the whole process.
    ///
    /// ```
    /// use nurt_code::registry::Registry;
    /// use nurt_contracts::services::LanguageId;
    ///
    /// let registry = Registry::builtin();
    /// let rust = LanguageId::new("rust")?;
    /// assert!(registry.spec(&rust).is_some());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn builtin() -> Arc<Registry> {
        static BUILTIN: OnceLock<Arc<Registry>> = OnceLock::new();
        Arc::clone(BUILTIN.get_or_init(|| {
            let mut registry = Registry::empty();
            let provider: Arc<dyn GrammarProvider> = Arc::new(BuiltinGrammars);
            let specs = provider.languages();
            registry.push_specs(specs, &provider);
            Arc::new(registry)
        }))
    }

    /// Registry of the built-in grammars and the grammars of `provider`.
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` when the provider lists a language that
    /// is already registered.
    pub fn with_provider(provider: Arc<dyn GrammarProvider>) -> Result<Self> {
        let mut registry = Registry::empty();
        let builtin: Arc<dyn GrammarProvider> = Arc::new(BuiltinGrammars);
        let specs = builtin.languages();
        registry.push_specs(specs, &builtin);
        registry.add(provider)?;
        Ok(registry)
    }

    /// Adds the languages of `provider`.
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` when the provider lists a language that
    /// is already registered; no language of the provider is added then.
    pub fn add(&mut self, provider: Arc<dyn GrammarProvider>) -> Result<()> {
        let specs = provider.languages();
        if let Some(duplicate) = specs.iter().find(|spec| self.spec(&spec.id).is_some()) {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "language `{}` is already registered; choose another identifier",
                    duplicate.id
                ),
            ));
        }
        self.push_specs(specs, &provider);
        Ok(())
    }

    fn push_specs(&mut self, specs: Vec<LanguageSpec>, provider: &Arc<dyn GrammarProvider>) {
        self.entries.extend(specs.into_iter().map(|spec| Entry {
            spec,
            provider: Arc::clone(provider),
            loaded: OnceLock::new(),
        }));
    }

    /// Descriptions of every language, in registration order.
    pub fn specs(&self) -> impl Iterator<Item = &LanguageSpec> {
        self.entries.iter().map(|entry| &entry.spec)
    }

    /// Description of a language.
    #[must_use]
    pub fn spec(&self, id: &LanguageId) -> Option<&LanguageSpec> {
        self.entry(id).map(|entry| &entry.spec)
    }

    /// Languages in the form of the code service.
    #[must_use]
    pub fn languages(&self) -> Vec<LanguageInfo> {
        self.specs()
            .map(|spec| LanguageInfo {
                id: spec.id.clone(),
                name: spec.name.clone(),
                extensions: spec.extensions.clone(),
                grammar_version: Some(spec.grammar_version.clone()),
            })
            .collect()
    }

    /// Language named by an identifier or an alias, compared without regard
    /// to case.
    ///
    /// ```
    /// use nurt_code::registry::Registry;
    ///
    /// let registry = Registry::builtin();
    /// assert_eq!(registry.resolve_name("Py").map(|id| id.to_string()), Some("python".to_owned()));
    /// assert!(registry.resolve_name("cobol").is_none());
    /// ```
    #[must_use]
    pub fn resolve_name(&self, name: &str) -> Option<LanguageId> {
        let wanted = name.trim().to_ascii_lowercase();
        self.specs()
            .find(|spec| spec.id.as_str() == wanted || spec.aliases.iter().any(|a| *a == wanted))
            .map(|spec| spec.id.clone())
    }

    /// Language of a file, from the modeline, the file name, the extension
    /// and the shebang line, in this order.
    #[must_use]
    pub fn detect(&self, path: Option<&Path>, bytes: &[u8]) -> Option<LanguageId> {
        detect_language(self, path, bytes)
    }

    /// Language of a file from its name alone.
    #[must_use]
    pub fn detect_by_name(&self, path: &Path) -> Option<LanguageId> {
        detect::by_name(self, path)
    }

    /// Grammar of a language with compiled queries, loaded on first use.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNSUPPORTED_LANGUAGE` when the language is unknown or
    /// its grammar cannot be loaded, and with `E_INTERNAL` when a query of
    /// the grammar does not compile.
    pub fn grammar(&self, id: &LanguageId) -> Result<Arc<LoadedGrammar>> {
        let entry = self.entry(id).ok_or_else(|| unsupported(id))?;
        let loaded = entry.loaded.get_or_init(|| load(entry));
        loaded
            .as_ref()
            .map(Arc::clone)
            .map_err(|(code, message)| Error::new(*code, message.clone()))
    }

    /// Parses source text.
    ///
    /// # Errors
    ///
    /// Fails like [`Registry::grammar`], and with `E_INTERNAL` when the
    /// parser rejects the grammar.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_code::registry::Registry;
    /// use nurt_contracts::services::LanguageId;
    ///
    /// let registry = Registry::builtin();
    /// let parsed = registry.parse(&LanguageId::new("go")?, b"package main\n")?;
    /// assert_eq!(parsed.tree.root_node().kind(), "source_file");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse<'a>(&self, id: &LanguageId, source: &'a [u8]) -> Result<ParsedSource<'a>> {
        let grammar = self.grammar(id)?;
        let mut parser = Parser::new();
        parser.set_language(grammar.language()).map_err(|error| {
            Error::new(
                ErrorCode::Internal,
                format!("grammar of `{id}` is not compatible with the parser: {error}"),
            )
        })?;
        let tree = parser.parse(source, None).ok_or_else(|| {
            Error::new(
                ErrorCode::Internal,
                format!("parser of `{id}` returned no tree"),
            )
        })?;
        Ok(ParsedSource {
            tree,
            grammar,
            source,
            language: id.clone(),
        })
    }

    fn entry(&self, id: &LanguageId) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.spec.id == *id)
    }
}

/// Error for a language without a grammar.
#[must_use]
pub fn unsupported(id: &LanguageId) -> Error {
    Error::new(
        ErrorCode::UnsupportedLanguage,
        format!("no grammar for language `{id}`; list the languages with `code.languages`"),
    )
}

fn load(entry: &Entry) -> LoadResult {
    let id = &entry.spec.id;
    let grammar = entry.provider.grammar(id).ok_or_else(|| {
        (
            ErrorCode::UnsupportedLanguage,
            format!("grammar of `{id}` cannot be loaded; list the languages with `code.languages`"),
        )
    })?;
    let compile = |kind: &str, source: &Option<Cow<'static, str>>| {
        source
            .as_deref()
            .map(|text| {
                Query::new(&grammar.language, text).map_err(|error| {
                    (
                        ErrorCode::Internal,
                        format!("{kind} query of `{id}` does not compile: {error}"),
                    )
                })
            })
            .transpose()
    };
    let tags = compile("symbol", &grammar.tags)?;
    let locals = compile("scope", &grammar.locals)?;
    Ok(Arc::new(LoadedGrammar {
        language: grammar.language,
        tags,
        locals,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Extra;

    impl GrammarProvider for Extra {
        fn languages(&self) -> Vec<LanguageSpec> {
            let id = LanguageId::new("extra").unwrap_or_else(|_| unreachable!());
            vec![LanguageSpec {
                id,
                name: "Extra".to_owned(),
                extensions: vec!["extra".to_owned()],
                file_names: Vec::new(),
                interpreters: Vec::new(),
                aliases: Vec::new(),
                grammar_version: "1.0.0".to_owned(),
                programming: true,
            }]
        }

        fn grammar(&self, _id: &LanguageId) -> Option<Grammar> {
            None
        }
    }

    struct Duplicate;

    impl GrammarProvider for Duplicate {
        fn languages(&self) -> Vec<LanguageSpec> {
            let mut specs = Extra.languages();
            specs[0].id = LanguageId::new("rust").unwrap();
            specs
        }

        fn grammar(&self, _id: &LanguageId) -> Option<Grammar> {
            None
        }
    }

    #[test]
    fn provider_languages_are_listed_and_unloadable_grammar_is_unsupported() {
        let registry = Registry::with_provider(Arc::new(Extra)).unwrap();
        let id = LanguageId::new("extra").unwrap();
        assert!(registry.spec(&id).is_some());
        assert_eq!(
            registry.detect_by_name(Path::new("a.extra")),
            Some(id.clone())
        );
        let error = registry.grammar(&id).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
    }

    #[test]
    fn duplicate_language_is_rejected() {
        let error = Registry::with_provider(Arc::new(Duplicate)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn unknown_language_fails_with_unsupported_language() {
        let registry = Registry::builtin();
        let error = registry
            .parse(&LanguageId::new("cobol").unwrap(), b"")
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
    }

    #[test]
    fn positions_count_unicode_scalar_values() {
        let registry = Registry::builtin();
        let source = "x = \"żółw\"; y = 1\n".as_bytes();
        let parsed = registry
            .parse(&LanguageId::new("python").unwrap(), source)
            .unwrap();
        let offset = source.len() - 6;
        let position = parsed.position(
            offset,
            Point {
                row: 0,
                column: offset,
            },
        );
        assert_eq!(position.line, 1);
        assert_eq!(position.column, 13);
        assert_eq!(parsed.line_text(3), "x = \"żółw\"; y = 1");
    }
}
