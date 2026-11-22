// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Outlines of source files and lookup of symbols by qualified name.
//!
//! An outline is the list of symbols of a file in source order with their
//! nesting depth ([`SymbolInfo::depth`]). A qualified name `Outer.inner`
//! names the symbol `inner` whose container ends with `Outer`; the container
//! of a nested symbol is itself qualified (`Outer.Middle`), so a name
//! `Middle.inner` also selects it.

use nurt_contracts::error::Result;
use nurt_contracts::services::{LanguageId, SymbolInfo};

use crate::registry::Registry;
use crate::symbols::symbols_of;

/// Symbols of source text in source order, no deeper than `depth`
/// (`Some(0)` keeps the top-level symbols only).
///
/// # Errors
///
/// Fails with `E_UNSUPPORTED_LANGUAGE` when the language has no grammar.
///
/// # Examples
///
/// ```
/// use nurt_code::outline::outline;
/// use nurt_code::registry::Registry;
/// use nurt_contracts::services::LanguageId;
///
/// let source = b"class A:\n    def run(self):\n        pass\n";
/// let python = LanguageId::new("python")?;
/// let top = outline(Registry::builtin(), source, &python, Some(0))?;
/// assert_eq!(top.len(), 1);
/// assert_eq!(outline(Registry::builtin(), source, &python, None)?.len(), 2);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn outline(
    registry: &Registry,
    source: &[u8],
    language: &LanguageId,
    depth: Option<u32>,
) -> Result<Vec<SymbolInfo>> {
    let symbols = symbols_of(registry, source, language)?;
    Ok(limit_depth(symbols, depth))
}

/// Symbols no deeper than `depth`; all symbols when `depth` is `None`.
#[must_use]
pub fn limit_depth(symbols: Vec<SymbolInfo>, depth: Option<u32>) -> Vec<SymbolInfo> {
    match depth {
        Some(max) => symbols
            .into_iter()
            .filter(|symbol| symbol.depth <= max)
            .collect(),
        None => symbols,
    }
}

/// Definitions named by a plain or qualified name in source text.
///
/// An empty list means that the name is not defined; more than one entry
/// means that it is ambiguous.
///
/// # Errors
///
/// Fails with `E_UNSUPPORTED_LANGUAGE` when the language has no grammar.
///
/// # Examples
///
/// ```
/// use nurt_code::outline::find;
/// use nurt_code::registry::Registry;
/// use nurt_contracts::services::LanguageId;
///
/// let source = b"class A:\n    def run(self): pass\nclass B:\n    def run(self): pass\n";
/// let python = LanguageId::new("python")?;
/// assert_eq!(find(Registry::builtin(), source, &python, "run")?.len(), 2);
/// assert_eq!(find(Registry::builtin(), source, &python, "B.run")?[0].span.start.line, 4);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn find(
    registry: &Registry,
    source: &[u8],
    language: &LanguageId,
    name: &str,
) -> Result<Vec<SymbolInfo>> {
    let symbols = symbols_of(registry, source, language)?;
    Ok(find_in(&symbols, name).into_iter().cloned().collect())
}

/// Symbols of `symbols` named by a plain or qualified name, in source order.
#[must_use]
pub fn find_in<'s>(symbols: &'s [SymbolInfo], name: &str) -> Vec<&'s SymbolInfo> {
    let name = name.trim();
    symbols
        .iter()
        .filter(|symbol| matches_name(symbol, name))
        .collect()
}

/// Returns `true` when `wanted` names the symbol: its own name, or a suffix
/// of its qualified name that starts at a component boundary.
fn matches_name(symbol: &SymbolInfo, wanted: &str) -> bool {
    if symbol.name == wanted {
        return true;
    }
    let Some(container) = symbol.container.as_deref() else {
        return false;
    };
    let Some(prefix) = wanted
        .strip_suffix(symbol.name.as_str())
        .and_then(|rest| rest.strip_suffix(['.', ':']))
    else {
        return false;
    };
    let prefix = prefix.trim_end_matches(':');
    let normalized = container.replace("::", ".");
    let prefix = prefix.replace("::", ".");
    normalized == prefix
        || normalized
            .strip_suffix(prefix.as_str())
            .is_some_and(|head| head.ends_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_symbols() -> Vec<SymbolInfo> {
        let source = b"mod net {\n    pub struct Client;\n    impl Client {\n        pub fn send(&self) {}\n    }\n}\nfn send() {}\n";
        symbols_of(
            Registry::builtin(),
            source,
            &LanguageId::new("rust").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn qualified_name_selects_the_symbol_of_one_container() {
        let symbols = rust_symbols();
        assert_eq!(find_in(&symbols, "send").len(), 2);
        let method = find_in(&symbols, "Client.send");
        assert_eq!(method.len(), 1);
        assert_eq!(method[0].container.as_deref(), Some("net.Client"));
        assert_eq!(find_in(&symbols, "net.Client.send").len(), 1);
        assert_eq!(find_in(&symbols, "net::Client::send").len(), 1);
        assert!(find_in(&symbols, "ient.send").is_empty());
        assert!(find_in(&symbols, "Server.send").is_empty());
    }

    #[test]
    fn depth_limit_keeps_outer_symbols() {
        let symbols = rust_symbols();
        let top = limit_depth(symbols.clone(), Some(0));
        assert!(top.iter().all(|symbol| symbol.depth == 0));
        assert!(top.len() < symbols.len());
        assert_eq!(limit_depth(symbols.clone(), None).len(), symbols.len());
    }

    #[test]
    fn unknown_language_is_rejected() {
        let error = outline(
            Registry::builtin(),
            b"",
            &LanguageId::new("cobol").unwrap(),
            None,
        )
        .unwrap_err();
        assert_eq!(
            error.code(),
            nurt_contracts::error::ErrorCode::UnsupportedLanguage
        );
    }
}
