// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion of symbols to `symbol` records.
//!
//! A `symbol` record places `line` and `column` at the name of the symbol and
//! `end_line` and `end_column` at the end of the whole definition, so the
//! position of a record can be passed to the positional commands
//! (`code.definition`, `code.references`) and the record still covers the
//! body.

use std::path::Path;

use nurt_contracts::fs::AbsPath;
use nurt_contracts::record::{Symbol, SymbolKind as RecordKind, SymbolSource};
use nurt_contracts::services::{LanguageId, SymbolInfo, SymbolKind};
use nurt_contracts::value::Path as RecordPath;

use crate::semantic::SemanticSymbol;

/// Kind of a `symbol` record for a kind of syntax analysis.
///
/// ```
/// use nurt_code::symbols::records::record_kind;
/// use nurt_contracts::record::SymbolKind as RecordKind;
/// use nurt_contracts::services::SymbolKind;
///
/// assert_eq!(record_kind(SymbolKind::Method), RecordKind::Method);
/// ```
#[must_use]
pub fn record_kind(kind: SymbolKind) -> RecordKind {
    match kind {
        SymbolKind::Function => RecordKind::Function,
        SymbolKind::Method => RecordKind::Method,
        SymbolKind::Class => RecordKind::Class,
        SymbolKind::Struct => RecordKind::Struct,
        SymbolKind::Interface => RecordKind::Interface,
        SymbolKind::Enum => RecordKind::Enum,
        SymbolKind::Variable => RecordKind::Variable,
        SymbolKind::Constant => RecordKind::Constant,
        SymbolKind::Field => RecordKind::Field,
        SymbolKind::Module => RecordKind::Module,
        SymbolKind::Type => RecordKind::Type,
        SymbolKind::Macro => RecordKind::Macro,
        _ => RecordKind::Other,
    }
}

/// Kind of syntax analysis for a record kind; `None` for
/// [`RecordKind::Other`].
#[must_use]
pub fn service_kind(kind: RecordKind) -> Option<SymbolKind> {
    Some(match kind {
        RecordKind::Function => SymbolKind::Function,
        RecordKind::Method => SymbolKind::Method,
        RecordKind::Class => SymbolKind::Class,
        RecordKind::Struct => SymbolKind::Struct,
        RecordKind::Interface => SymbolKind::Interface,
        RecordKind::Enum => SymbolKind::Enum,
        RecordKind::Variable => SymbolKind::Variable,
        RecordKind::Constant => SymbolKind::Constant,
        RecordKind::Field => SymbolKind::Field,
        RecordKind::Module => SymbolKind::Module,
        RecordKind::Type => SymbolKind::Type,
        RecordKind::Macro => SymbolKind::Macro,
        _ => return None,
    })
}

/// Path in the form of the language, for the `path` field of records;
/// relative paths are kept as written.
///
/// ```
/// use std::path::Path;
/// use nurt_code::symbols::records::path_text;
///
/// assert_eq!(path_text(Path::new("src/main.rs")).as_str(), "src/main.rs");
/// ```
#[must_use]
pub fn path_text(path: &Path) -> RecordPath {
    let text = AbsPath::new(path).map_or_else(
        |_| path.to_string_lossy().replace('\\', "/"),
        |absolute| absolute.to_language_string(),
    );
    RecordPath::new(text)
}

/// Facts about the file a symbol comes from.
#[derive(Debug, Clone, Copy)]
pub struct FileFacts<'a> {
    /// Path of the file.
    pub path: &'a Path,
    /// Language of the file.
    pub language: &'a LanguageId,
    /// Entity tag of the content (`sha256:…`).
    pub etag: &'a str,
}

/// Optional fields of a `symbol` record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordFields {
    /// Fill `signature`.
    pub signature: bool,
    /// Number of references, for `references`.
    pub references: Option<u64>,
}

/// `symbol` record of a symbol found by syntax analysis.
///
/// ```
/// use std::path::Path;
/// use nurt_code::registry::Registry;
/// use nurt_code::symbols::records::{FileFacts, RecordFields, symbol_record};
/// use nurt_code::symbols::symbols_of;
/// use nurt_contracts::services::LanguageId;
///
/// let go = LanguageId::new("go")?;
/// let symbols = symbols_of(Registry::builtin(), b"package p\n\nfunc Run() {}\n", &go)?;
/// let facts = FileFacts { path: Path::new("/src/p.go"), language: &go, etag: "sha256:00" };
/// let record = symbol_record(&symbols[0], &facts, RecordFields { signature: true, references: None });
/// assert_eq!((record.line, record.column, record.end_line), (3, 6, 3));
/// assert_eq!(record.signature.as_deref(), Some("func Run()"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn symbol_record(info: &SymbolInfo, file: &FileFacts<'_>, fields: RecordFields) -> Symbol {
    Symbol {
        name: info.name.clone(),
        kind: record_kind(info.kind),
        path: path_text(file.path),
        line: u64::from(info.name_span.start.line),
        column: u64::from(info.name_span.start.column),
        end_line: u64::from(info.span.end.line),
        language: file.language.to_string(),
        source: SymbolSource::Syntax,
        etag: file.etag.to_owned(),
        end_column: Some(u64::from(info.span.end.column)),
        container: info.container.clone(),
        signature: if fields.signature {
            info.signature.clone()
        } else {
            None
        },
        text: None,
        exported: info.exported,
        references: fields.references,
    }
}

/// `symbol` record of a symbol reported by a language server.
///
/// `language` is the language of the file; `etag` is empty when the file
/// was not read.
#[must_use]
pub fn semantic_record(
    symbol: &SemanticSymbol,
    language: &str,
    etag: &str,
    fields: RecordFields,
) -> Symbol {
    Symbol {
        name: symbol.name.clone(),
        kind: symbol.kind,
        path: path_text(&symbol.location.path),
        line: u64::from(symbol.name_position.line),
        column: u64::from(symbol.name_position.column),
        end_line: u64::from(symbol.location.end.line),
        language: language.to_owned(),
        source: SymbolSource::Lsp,
        etag: etag.to_owned(),
        end_column: Some(u64::from(symbol.location.end.column)),
        container: symbol.container.clone(),
        signature: if fields.signature {
            symbol.detail.clone()
        } else {
            None
        },
        text: None,
        exported: None,
        references: fields.references,
    }
}

/// Qualified name of a symbol: `Container.name`, or the name alone.
///
/// ```
/// use nurt_code::symbols::records::qualified_name;
///
/// assert_eq!(qualified_name("push", Some("Stack")), "Stack.push");
/// assert_eq!(qualified_name("main", None), "main");
/// ```
#[must_use]
pub fn qualified_name(name: &str, container: Option<&str>) -> String {
    match container {
        Some(container) => format!("{container}.{name}"),
        None => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use nurt_contracts::services::TextPosition;

    use super::*;
    use crate::semantic::Location;

    #[test]
    fn every_service_kind_maps_back_from_its_record_kind() {
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
            assert_eq!(service_kind(record_kind(kind)), Some(kind));
        }
        assert_eq!(service_kind(RecordKind::Other), None);
    }

    #[test]
    fn semantic_record_takes_the_name_position_and_the_detail() {
        let position = |line, column| TextPosition { line, column };
        let symbol = SemanticSymbol {
            name: "run".to_owned(),
            kind: RecordKind::Function,
            container: None,
            depth: 0,
            location: Location {
                path: PathBuf::from("/src/a.py"),
                start: position(1, 1),
                end: position(3, 9),
            },
            name_position: position(1, 5),
            detail: Some("def run()".to_owned()),
        };
        let with = semantic_record(
            &symbol,
            "python",
            "",
            RecordFields {
                signature: true,
                references: Some(2),
            },
        );
        assert_eq!((with.line, with.column, with.end_line), (1, 5, 3));
        assert_eq!(with.source, SymbolSource::Lsp);
        assert_eq!(with.signature.as_deref(), Some("def run()"));
        assert_eq!(with.references, Some(2));
        let without = semantic_record(&symbol, "python", "", RecordFields::default());
        assert_eq!(without.signature, None);
    }
}
