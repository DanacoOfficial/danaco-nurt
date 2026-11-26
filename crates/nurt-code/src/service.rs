// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The code service of the contract, implemented over the language registry.
//!
//! [`CodeAnalysis`] answers the syntax questions of other command groups:
//! outlines and symbol lookup, enclosing declarations, the lexical class of
//! source regions, syntax errors, structural patterns and their templates,
//! occurrences of a name, function metrics and tokens. Every method parses
//! the bytes passed by the caller and reads no files.
//!
//! ```
//! use nurt_code::service::CodeAnalysis;
//! use nurt_contracts::services::{CodeService, LanguageId};
//!
//! let service = CodeAnalysis::builtin();
//! let rust = LanguageId::new("rust")?;
//! let pattern = service.compile_pattern("$A.unwrap()", &rust)?;
//! let found = service.syntax_search(b"fn f() { open().unwrap(); }", pattern.as_ref())?;
//! assert_eq!(found[0].captures[0].text, "open()");
//! assert_eq!(service.render_template(&found[0], "$A?")?, "open()?");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::path::Path;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{
    ClassifiedSpan, CodeService, FunctionMetrics, LanguageId, LanguageInfo, SymbolInfo,
    SymbolReference, SyntaxIssue, SyntaxMatch, SyntaxPattern, SyntaxToken,
};

use crate::pattern::StructuralPattern;
use crate::registry::Registry;
use crate::{local, spans, symbols, syntax_check};

/// Registry of the languages of a service.
#[derive(Debug, Clone)]
enum Languages {
    /// The built-in grammars, shared by the whole process.
    Builtin(&'static Registry),
    /// A registry with grammars of extensions.
    Shared(Arc<Registry>),
}

/// Syntax analysis of source code with the grammars of a language registry.
#[derive(Debug, Clone)]
pub struct CodeAnalysis {
    languages: Languages,
}

impl CodeAnalysis {
    /// Service over the languages of `registry`.
    #[must_use]
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            languages: Languages::Shared(registry),
        }
    }

    /// Service over the built-in grammars.
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            languages: Languages::Builtin(Registry::builtin()),
        }
    }

    /// Languages the service works with.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        match &self.languages {
            Languages::Builtin(registry) => registry,
            Languages::Shared(registry) => registry,
        }
    }
}

impl Default for CodeAnalysis {
    fn default() -> Self {
        Self::builtin()
    }
}

impl CodeService for CodeAnalysis {
    fn languages(&self) -> Vec<LanguageInfo> {
        self.registry().languages()
    }

    fn language_of(&self, path: Option<&Path>, bytes: &[u8]) -> Option<LanguageId> {
        self.registry().detect(path, bytes)
    }

    fn outline(
        &self,
        bytes: &[u8],
        language: &LanguageId,
        depth: Option<u32>,
    ) -> Result<Vec<SymbolInfo>> {
        let symbols = symbols::symbols_of(self.registry(), bytes, language)?;
        Ok(match depth {
            None => symbols,
            Some(max) => symbols.into_iter().filter(|s| s.depth <= max).collect(),
        })
    }

    fn find_symbol(
        &self,
        bytes: &[u8],
        language: &LanguageId,
        name: &str,
    ) -> Result<Vec<SymbolInfo>> {
        let name = name.trim().replace("::", ".");
        let (container, simple) = split_qualified(&name);
        let symbols = symbols::symbols_of(self.registry(), bytes, language)?;
        Ok(symbols
            .into_iter()
            .filter(|symbol| symbol.name == simple)
            .filter(|symbol| container.is_none_or(|wanted| in_container(symbol, wanted)))
            .collect())
    }

    fn enclosing(
        &self,
        bytes: &[u8],
        language: &LanguageId,
        offset: u64,
    ) -> Result<Option<SymbolInfo>> {
        let parsed = self.registry().parse(language, bytes)?;
        let offset = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        Ok(crate::enclosing::enclosing(&parsed.tree, bytes, offset))
    }

    fn classify_spans(&self, bytes: &[u8], language: &LanguageId) -> Result<Vec<ClassifiedSpan>> {
        let parsed = self.registry().parse(language, bytes)?;
        Ok(spans::classify_spans(&parsed.tree))
    }

    fn check_syntax(&self, bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxIssue>> {
        syntax_check::check_syntax(self.registry(), bytes, language)
    }

    fn compile_pattern(
        &self,
        pattern: &str,
        language: &LanguageId,
    ) -> Result<Box<dyn SyntaxPattern>> {
        let grammar = self.registry().grammar(language)?;
        let compiled = StructuralPattern::compile(pattern, language, grammar.language())?;
        Ok(Box::new(compiled))
    }

    fn syntax_search(&self, bytes: &[u8], pattern: &dyn SyntaxPattern) -> Result<Vec<SyntaxMatch>> {
        let compiled = pattern
            .as_any()
            .downcast_ref::<StructuralPattern>()
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    "the pattern was compiled by another code service; compile it with this service",
                )
            })?;
        compiled.search(bytes)
    }

    fn render_template(&self, found: &SyntaxMatch, template: &str) -> Result<String> {
        Ok(crate::structural::render_template(found, template))
    }

    fn references(
        &self,
        bytes: &[u8],
        language: &LanguageId,
        name: &str,
    ) -> Result<Vec<SymbolReference>> {
        if name.is_empty() {
            return Ok(Vec::new());
        }
        local::references(self.registry(), bytes, language, name)
    }

    fn function_metrics(
        &self,
        bytes: &[u8],
        language: &LanguageId,
    ) -> Result<Vec<FunctionMetrics>> {
        let parsed = self.registry().parse(language, bytes)?;
        Ok(crate::enclosing::function_metrics(&parsed.tree, bytes))
    }

    fn tokens(&self, bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxToken>> {
        let parsed = self.registry().parse(language, bytes)?;
        Ok(spans::tokens(&parsed.tree))
    }
}

/// Splits `Container.name` into the container and the name; a name without
/// a period has no container.
fn split_qualified(name: &str) -> (Option<&str>, &str) {
    match name.rsplit_once('.') {
        Some((container, simple)) if !container.is_empty() && !simple.is_empty() => {
            (Some(container), simple)
        }
        _ => (None, name),
    }
}

/// Returns `true` when the qualified container of `symbol` is `wanted` or
/// ends with it; `::` and `.` separate the components alike.
fn in_container(symbol: &SymbolInfo, wanted: &str) -> bool {
    symbol.container.as_deref().is_some_and(|actual| {
        let actual = actual.replace("::", ".");
        let actual = actual.as_str();
        actual == wanted
            || actual
                .strip_suffix(wanted)
                .is_some_and(|outer| outer.ends_with('.'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_names_split_at_the_last_period() {
        assert_eq!(split_qualified("Stack.push"), (Some("Stack"), "push"));
        assert_eq!(split_qualified("a.B.c"), (Some("a.B"), "c"));
        assert_eq!(split_qualified("push"), (None, "push"));
        assert_eq!(split_qualified(".push"), (None, ".push"));
    }

    #[test]
    fn qualified_containers_match_by_suffix() {
        let service = CodeAnalysis::builtin();
        let python = LanguageId::new("python").unwrap();
        let source = b"class A:\n    class B:\n        def f(self):\n            pass\n";
        assert_eq!(
            service.find_symbol(source, &python, "B.f").unwrap().len(),
            1
        );
        assert_eq!(
            service.find_symbol(source, &python, "A.B.f").unwrap().len(),
            1
        );
        assert!(
            service
                .find_symbol(source, &python, "AB.f")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn double_colon_separates_qualified_names_like_a_period() {
        let service = CodeAnalysis::builtin();
        let rust = LanguageId::new("rust").unwrap();
        let source = b"mod net {\n    pub struct Client;\n    impl Client {\n        pub fn send(&self) {}\n    }\n}\nfn send() {}\n";
        let count = |name: &str| service.find_symbol(source, &rust, name).unwrap().len();
        assert_eq!(count("send"), 2);
        assert_eq!(count("Client::send"), 1);
        assert_eq!(count("net::Client::send"), 1);
        assert_eq!(count(" net.Client::send "), 1);
        assert_eq!(count("Server::send"), 0);
        assert_eq!(count("net::send"), 0);
    }

    #[test]
    fn services_over_shared_and_builtin_registries_agree() {
        let shared = CodeAnalysis::new(Arc::new(Registry::empty()));
        assert!(shared.languages().is_empty());
        assert!(!CodeAnalysis::default().languages().is_empty());
    }
}
