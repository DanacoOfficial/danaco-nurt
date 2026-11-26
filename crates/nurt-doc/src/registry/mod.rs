// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Format registry and format detection.
//!
//! Every format implements [`DocFormat`] and is registered in a
//! [`Registry`]. The registry detects the format of a file from the
//! `format` parameter, the file name (`.env`, `Cargo.lock`, `pom.xml`,
//! `.vscode/settings.json`), the extension and the content, enforces
//! `documents.max_bytes`, decodes text formats from UTF-8 with or without a
//! byte order mark and from UTF-16, and checks on opening that the document
//! keeps the file byte for byte.
//!
//! Each module of [`crate::formats`] provides a registration function
//! `pub fn formats() -> Vec<Box<dyn DocFormat>>` listed in
//! `formats::BUILTIN`; [`Registry::builtin`] registers all of them.
//!
//! ```
//! use nurt_doc::registry::{DocOptions, Registry};
//!
//! let registry = Registry::new();
//! assert!(registry.detect(None, b"{}").is_none());
//! assert_eq!(DocOptions::default().max_bytes, 256 * 1024 * 1024);
//! ```

mod encoding;

use std::fmt;
use std::path::{Component, Path};

use nurt_contracts::config::{DocumentsConfig, YamlVersion};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{
    ByteRange, DocFormatId, DocFormatInfo, IssueSeverity, SyntaxIssue, TextPosition, TextSpan,
};

pub use encoding::TextEncoding;

use crate::edit::guard::check_round_trip;
use crate::model::DocHandle;

/// Largest document read by default (`documents.max_bytes`).
const DEFAULT_MAX_BYTES: u64 = 256 * 1024 * 1024;

/// How certain a format is that a text is one of its documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Confidence {
    /// Not a document of the format.
    #[default]
    None,
    /// Could be a document of the format (for example a text of `key=value`
    /// lines).
    Low,
    /// Probably a document of the format (for example a text starting with
    /// `{`).
    Medium,
    /// A document of the format (for example a text starting with `<?xml`
    /// or `bplist00`).
    High,
}

/// What a format receives when a document is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InputKind {
    /// UTF-8 text without a byte order mark; the registry decodes UTF-8
    /// with a mark and UTF-16 and encodes the result back.
    #[default]
    Utf8Text,
    /// The bytes of the file (binary formats and formats that declare their
    /// own encoding, such as XML and property files).
    Bytes,
}

/// Options of opening documents, from the `documents.*` configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocOptions {
    /// Largest document read, in bytes (`documents.max_bytes`).
    pub max_bytes: u64,
    /// YAML version of documents without a `%YAML` directive
    /// (`documents.yaml_version`).
    pub yaml_version: YamlVersion,
}

impl DocOptions {
    /// Options from the `[documents]` configuration section.
    ///
    /// External XML entities are never resolved: the setting
    /// `documents.xml_external_entities` is not carried into the options.
    #[must_use]
    pub fn from_config(config: &DocumentsConfig) -> Self {
        Self {
            max_bytes: config.max_bytes.bytes(),
            yaml_version: config.yaml_version,
        }
    }

    /// Returns `false`: external entities of XML documents are never
    /// resolved, whatever the configuration says.
    #[must_use]
    pub const fn xml_external_entities(&self) -> bool {
        false
    }
}

impl Default for DocOptions {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            yaml_version: YamlVersion::V1_2,
        }
    }
}

/// Structured document format.
pub trait DocFormat: Send + Sync {
    /// Identifier (`json`, `yaml`).
    fn id(&self) -> DocFormatId;

    /// File name extensions without the leading period, lower case.
    fn extensions(&self) -> &[&'static str];

    /// File name patterns of the format, lower case.
    ///
    /// A pattern is an exact name (`cargo.lock`), a name with one `*`
    /// matching any run of characters (`.env.*`), or names separated by `/`
    /// matched against the last components of the path (`.vscode/*.json`).
    /// When patterns of several formats match, the one with the larger
    /// number of components wins.
    fn file_names(&self) -> &[&'static str] {
        &[]
    }

    /// Certainty that `content` (the start of a file, possibly the whole
    /// file) is a document of the format.
    fn sniff(&self, content: &[u8]) -> Confidence;

    /// What [`DocFormat::open`] receives.
    fn input(&self) -> InputKind {
        InputKind::Utf8Text
    }

    /// Returns `true` when documents of the format can be changed with the
    /// layout kept.
    fn editable(&self) -> bool {
        true
    }

    /// Opens a document.
    ///
    /// # Errors
    ///
    /// Fails with `E_DOC_PARSE` when the text is not a valid document of the
    /// format.
    fn open(&self, content: Vec<u8>, options: &DocOptions) -> Result<Box<dyn DocHandle>>;

    /// Opens a document of the file at `path`.
    ///
    /// A format whose dialect follows from the file name (INI profiles of
    /// `.service` units, `.gitconfig` or `php.ini`) overrides it, so that the
    /// dialect does not depend on the content and stays the same after a
    /// change. The default ignores the path and calls [`DocFormat::open`].
    ///
    /// # Errors
    ///
    /// Fails as described for [`DocFormat::open`].
    fn open_at(
        &self,
        _path: Option<&Path>,
        content: Vec<u8>,
        options: &DocOptions,
    ) -> Result<Box<dyn DocHandle>> {
        self.open(content, options)
    }

    /// Syntax problems of `content`; an empty list when it parses.
    ///
    /// The default opens the document and reports the error of a failed
    /// opening as one issue at the position given in its details.
    fn check_syntax(&self, content: &[u8], options: &DocOptions) -> Vec<SyntaxIssue> {
        match self.open(content.to_vec(), options) {
            Ok(_) => Vec::new(),
            Err(error) => vec![issue_of(&error)],
        }
    }
}

/// Registration function of a module of [`crate::formats`]: the formats the
/// module provides, in detection priority order.
pub type FormatsFn = fn() -> Vec<Box<dyn DocFormat>>;

/// Syntax issue describing an error with optional `line`, `column` and
/// `byte` details.
#[must_use]
pub fn issue_of(error: &Error) -> SyntaxIssue {
    let detail = |name: &str| error.details.get(name).and_then(serde_json::Value::as_u64);
    let position = TextPosition {
        line: detail("line")
            .and_then(|line| u32::try_from(line).ok())
            .unwrap_or(1),
        column: detail("column")
            .and_then(|column| u32::try_from(column).ok())
            .unwrap_or(1),
    };
    let byte = detail("byte").unwrap_or_default();
    SyntaxIssue {
        severity: IssueSeverity::Error,
        message: error.message.clone(),
        span: TextSpan {
            bytes: ByteRange::new(byte, byte),
            start: position,
            end: position,
        },
        kind: Some(error.code.as_str().to_ascii_lowercase()),
    }
}

/// Document opened through a [`Registry`]: the handle and the encoding of
/// the file.
pub struct OpenDocument {
    /// The document.
    pub handle: Box<dyn DocHandle>,
    /// Encoding of the file; [`TextEncoding::Utf8`] for formats that read
    /// bytes.
    pub encoding: TextEncoding,
}

impl fmt::Debug for OpenDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenDocument")
            .field("format", &self.handle.format())
            .field("encoding", &self.encoding)
            .finish_non_exhaustive()
    }
}

impl OpenDocument {
    /// Bytes of the file after the changes made through the handle, in the
    /// encoding of the file.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        self.encoding.encode(self.handle.text())
    }
}

/// Set of known formats.
#[derive(Default)]
pub struct Registry {
    formats: Vec<Box<dyn DocFormat>>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.formats.iter().map(|format| format.id()))
            .finish()
    }
}

impl Registry {
    /// Registry without formats.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registry with every built-in format, in the order of
    /// `formats::BUILTIN`.
    #[must_use]
    pub fn builtin() -> Self {
        Self::from_functions(crate::formats::BUILTIN)
    }

    /// Registry with the formats returned by the registration functions,
    /// in order.
    #[must_use]
    pub fn from_functions(functions: &[FormatsFn]) -> Self {
        Self::with(functions.iter().flat_map(|formats| formats()))
    }

    /// Registry with the given formats.
    #[must_use]
    pub fn with(formats: impl IntoIterator<Item = Box<dyn DocFormat>>) -> Self {
        let mut registry = Self::new();
        for format in formats {
            registry.register(format);
        }
        registry
    }

    /// Adds a format; a format with the same identifier is replaced.
    pub fn register(&mut self, format: Box<dyn DocFormat>) {
        let id = format.id();
        self.formats.retain(|existing| existing.id() != id);
        self.formats.push(format);
    }

    /// The format with the identifier.
    #[must_use]
    pub fn get(&self, id: &DocFormatId) -> Option<&dyn DocFormat> {
        self.formats
            .iter()
            .find(|format| format.id() == *id)
            .map(AsRef::as_ref)
    }

    /// Description of every format, in registration order.
    #[must_use]
    pub fn infos(&self) -> Vec<DocFormatInfo> {
        self.formats
            .iter()
            .map(|format| DocFormatInfo {
                id: format.id(),
                extensions: format
                    .extensions()
                    .iter()
                    .map(|ext| (*ext).to_owned())
                    .collect(),
                editable: format.editable(),
            })
            .collect()
    }

    /// Detects the format from the path and the content.
    ///
    /// A file name pattern wins, the pattern with the larger number of path
    /// components first; then the formats with the extension, the one whose
    /// sniffing is the surest; then sniffing alone, at least
    /// [`Confidence::Medium`]. The first registered format wins a tie.
    #[must_use]
    pub fn detect(&self, path: Option<&Path>, content: &[u8]) -> Option<DocFormatId> {
        if let Some(id) = path.and_then(|path| self.by_name(path)) {
            return Some(id);
        }
        let extension = path
            .and_then(Path::extension)
            .map(|ext| ext.to_string_lossy().to_lowercase());
        let by_extension: Vec<&dyn DocFormat> = self
            .formats
            .iter()
            .filter(|format| {
                extension
                    .as_deref()
                    .is_some_and(|ext| format.extensions().contains(&ext))
            })
            .map(AsRef::as_ref)
            .collect();
        if let Some(format) = best_sniffed(&by_extension, content, Confidence::None) {
            return Some(format);
        }
        let all: Vec<&dyn DocFormat> = self.formats.iter().map(AsRef::as_ref).collect();
        best_sniffed(&all, content, Confidence::Medium)
    }

    fn by_name(&self, path: &Path) -> Option<DocFormatId> {
        let components = components_of(path);
        let mut best: Option<(usize, DocFormatId)> = None;
        for format in &self.formats {
            let Some(width) = name_match(format.as_ref(), &components) else {
                continue;
            };
            if best.as_ref().is_none_or(|(top, _)| width > *top) {
                best = Some((width, format.id()));
            }
        }
        best.map(|(_, id)| id)
    }

    /// Format named by the `format` parameter, or detected when the
    /// parameter is absent.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_VALUE` for a format name that is not
    /// registered, and with `E_FORMAT` when no format is detected.
    pub fn resolve_format(
        &self,
        requested: Option<&str>,
        path: Option<&Path>,
        content: &[u8],
    ) -> Result<DocFormatId> {
        if let Some(name) = requested {
            let known: Vec<String> = self
                .formats
                .iter()
                .map(|format| format.id().to_string())
                .collect();
            return self
                .formats
                .iter()
                .map(|format| format.id())
                .find(|id| id.as_str() == name)
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::UnknownValue,
                        format!(
                            "unknown document format `{name}`; use one of: {}",
                            known.join(", ")
                        ),
                    )
                    .with_found(name)
                });
        }
        self.detect(path, content).ok_or_else(|| {
            Error::new(
                ErrorCode::Format,
                "the format of the document is not recognized; set the `format` parameter",
            )
        })
    }

    /// Opens a document of the format `id` from the bytes of a file.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_VALUE` for an unknown format, `E_LIMIT` when the
    /// file is larger than `max_bytes`, `E_ENCODING` for text that is not
    /// valid UTF-8 or UTF-16, the errors of [`DocFormat::open`], and
    /// `E_UNSUPPORTED_CONSTRUCT` naming `doc.round_trip_loss` when the
    /// opened document would not keep the file byte for byte.
    pub fn open(
        &self,
        id: &DocFormatId,
        bytes: &[u8],
        options: &DocOptions,
    ) -> Result<OpenDocument> {
        self.open_at(id, None, bytes, options)
    }

    /// Opens a document of the format `id` from the bytes of the file at
    /// `path`, as [`Registry::open`] does, and passes the path to
    /// [`DocFormat::open_at`].
    ///
    /// # Errors
    ///
    /// Fails as described for [`Registry::open`].
    pub fn open_at(
        &self,
        id: &DocFormatId,
        path: Option<&Path>,
        bytes: &[u8],
        options: &DocOptions,
    ) -> Result<OpenDocument> {
        let format = self.get(id).ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownValue,
                format!("unknown document format `{id}`"),
            )
            .with_found(id.as_str())
        })?;
        open_with_path(format, path, bytes, options)
    }
}

/// Opens a document with `format`, as [`Registry::open`] does.
///
/// # Errors
///
/// Fails as described for [`Registry::open`].
pub fn open_with(
    format: &dyn DocFormat,
    bytes: &[u8],
    options: &DocOptions,
) -> Result<OpenDocument> {
    open_with_path(format, None, bytes, options)
}

/// Opens a document of the file at `path` with `format`, as
/// [`Registry::open_at`] does.
///
/// # Errors
///
/// Fails as described for [`Registry::open`].
pub fn open_with_path(
    format: &dyn DocFormat,
    path: Option<&Path>,
    bytes: &[u8],
    options: &DocOptions,
) -> Result<OpenDocument> {
    check_size(bytes, options)?;
    let (encoding, text) = match format.input() {
        InputKind::Utf8Text => TextEncoding::decode(bytes)?,
        InputKind::Bytes => (TextEncoding::Utf8, bytes.to_vec()),
    };
    // For UTF-8 without a byte order mark and for formats that read bytes
    // the text equals the file, so no copy is kept for the identity check.
    let decoded = (encoding != TextEncoding::Utf8).then(|| text.clone());
    let handle = format.open_at(path, text, options)?;
    check_round_trip(decoded.as_deref().unwrap_or(bytes), handle.text())?;
    Ok(OpenDocument { handle, encoding })
}

/// Checks `bytes` against `documents.max_bytes`.
///
/// # Errors
///
/// Fails with `E_LIMIT` when the document is larger than the limit.
pub(crate) fn check_size(bytes: &[u8], options: &DocOptions) -> Result<()> {
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if size > options.max_bytes {
        return Err(Error::new(
            ErrorCode::Limit,
            format!(
                "the document has {size} bytes, more than documents.max_bytes ({}); raise the limit or read the file as text",
                options.max_bytes
            ),
        )
        .with_detail("limit", options.max_bytes.into()));
    }
    Ok(())
}

/// Returns `true` when the name or the extension of `path` belongs to
/// `format`.
pub(crate) fn matches_file(format: &dyn DocFormat, path: &Path) -> bool {
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    format.extensions().contains(&extension.as_str())
        || name_match(format, &components_of(path)).is_some()
}

/// Lower-case names of the components of `path`, root and `.` excluded.
fn components_of(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect()
}

/// Number of components of the widest file name pattern of `format` that
/// matches the end of `components`.
fn name_match(format: &dyn DocFormat, components: &[String]) -> Option<usize> {
    format
        .file_names()
        .iter()
        .filter_map(|pattern| {
            let parts: Vec<&str> = pattern.split('/').collect();
            let first = components.len().checked_sub(parts.len())?;
            parts
                .iter()
                .zip(&components[first..])
                .all(|(part, name)| glob(part, name))
                .then_some(parts.len())
        })
        .max()
}

/// Matches `name` against a pattern with at most one `*`.
fn glob(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        Some((prefix, suffix)) => {
            name.len() >= prefix.len() + suffix.len()
                && name.starts_with(prefix)
                && name.ends_with(suffix)
        }
        None => pattern == name,
    }
}

/// Format with the highest confidence at or above `floor`; the first
/// registered wins a tie, and a single candidate with no confidence wins
/// when `floor` is [`Confidence::None`].
fn best_sniffed(
    formats: &[&dyn DocFormat],
    content: &[u8],
    floor: Confidence,
) -> Option<DocFormatId> {
    if floor == Confidence::None && formats.len() == 1 {
        return formats.first().map(|format| format.id());
    }
    let mut best: Option<(Confidence, DocFormatId)> = None;
    for format in formats {
        let confidence = format.sniff(content);
        if confidence >= floor && best.as_ref().is_none_or(|(top, _)| confidence > *top) {
            best = Some((confidence, format.id()));
        }
    }
    best.map(|(_, id)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub {
        id: &'static str,
        extensions: &'static [&'static str],
        names: &'static [&'static str],
        confidence: Confidence,
    }

    impl DocFormat for Stub {
        fn id(&self) -> DocFormatId {
            DocFormatId::new(self.id).unwrap()
        }

        fn extensions(&self) -> &[&'static str] {
            self.extensions
        }

        fn file_names(&self) -> &[&'static str] {
            self.names
        }

        fn sniff(&self, _content: &[u8]) -> Confidence {
            self.confidence
        }

        fn open(&self, _content: Vec<u8>, _options: &DocOptions) -> Result<Box<dyn DocHandle>> {
            Err(Error::new(ErrorCode::DocParse, "the stub opens nothing"))
        }
    }

    fn stub(
        id: &'static str,
        extensions: &'static [&'static str],
        names: &'static [&'static str],
        confidence: Confidence,
    ) -> Box<dyn DocFormat> {
        Box::new(Stub {
            id,
            extensions,
            names,
            confidence,
        })
    }

    fn id(name: &str) -> Option<DocFormatId> {
        Some(DocFormatId::new(name).unwrap())
    }

    #[test]
    fn glob_matches_exact_names_and_one_wildcard() {
        assert!(glob("cargo.lock", "cargo.lock"));
        assert!(!glob("cargo.lock", "cargo.lock2"));
        assert!(glob(".env.*", ".env.local"));
        assert!(glob(".env.*", ".env."));
        assert!(!glob(".env.*", ".env"));
        assert!(glob("*.json", "a.json"));
        assert!(!glob("a*a", "a"));
        assert!(glob("a*a", "aa"));
    }

    #[test]
    fn file_name_patterns_win_by_the_number_of_components() {
        let registry = Registry::with([
            stub("json", &["json"], &[], Confidence::Medium),
            stub("vscode", &[], &[".vscode/*.json"], Confidence::None),
        ]);
        let path = Path::new("/home/u/project/.VSCode/Settings.json");
        assert_eq!(registry.detect(Some(path), b"{}"), id("vscode"));
        let other = Path::new("/home/u/project/a.json");
        assert_eq!(registry.detect(Some(other), b"{}"), id("json"));
    }

    #[test]
    fn extension_candidates_are_ranked_by_content() {
        let registry = Registry::with([
            stub("one", &["cfg"], &[], Confidence::Low),
            stub("two", &["cfg"], &[], Confidence::High),
            stub("three", &["cfg"], &[], Confidence::High),
        ]);
        let path = Path::new("x.CFG");
        assert_eq!(registry.detect(Some(path), b""), id("two"));
        let alone = Registry::with([stub("one", &["cfg"], &[], Confidence::None)]);
        assert_eq!(alone.detect(Some(path), b""), id("one"));
    }

    #[test]
    fn content_alone_needs_at_least_medium_confidence() {
        let low = Registry::with([stub("low", &[], &[], Confidence::Low)]);
        assert_eq!(low.detect(None, b"x"), None);
        let medium = Registry::with([stub("medium", &[], &[], Confidence::Medium)]);
        assert_eq!(medium.detect(None, b"x"), id("medium"));
    }

    #[test]
    fn resolving_a_format_reports_unknown_and_undetected() {
        let registry = Registry::with([stub("one", &["cfg"], &[], Confidence::High)]);
        let error = registry.resolve_format(Some("two"), None, b"").unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownValue);
        assert!(error.message.contains("one"), "{}", error.message);
        let error = Registry::new().resolve_format(None, None, b"").unwrap_err();
        assert_eq!(error.code, ErrorCode::Format);
        let found = registry.resolve_format(Some("one"), None, b"").unwrap();
        assert_eq!(Some(found), id("one"));
    }

    #[test]
    fn registering_an_identifier_again_replaces_the_format() {
        let mut registry = Registry::new();
        registry.register(stub("one", &["a"], &[], Confidence::None));
        registry.register(stub("one", &["b"], &[], Confidence::None));
        let infos = registry.infos();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].extensions, ["b"]);
    }

    #[test]
    fn documents_larger_than_the_limit_are_refused_before_opening() {
        let format = stub("one", &[], &[], Confidence::None);
        let options = DocOptions {
            max_bytes: 4,
            ..DocOptions::default()
        };
        let error = open_with(format.as_ref(), b"12345", &options).unwrap_err();
        assert_eq!(error.code, ErrorCode::Limit);
        let error = open_with(format.as_ref(), b"1234", &options).unwrap_err();
        assert_eq!(error.code, ErrorCode::DocParse);
    }

    #[test]
    fn invalid_text_is_reported_as_an_encoding_error() {
        let format = stub("one", &[], &[], Confidence::None);
        let error = open_with(format.as_ref(), b"a\xFF", &DocOptions::default()).unwrap_err();
        assert_eq!(error.code, ErrorCode::Encoding);
    }

    #[test]
    fn issue_of_reads_the_position_from_the_details() {
        let error = Error::new(ErrorCode::DocParse, "bad")
            .with_detail("line", 3.into())
            .with_detail("column", 5.into())
            .with_detail("byte", 17.into());
        let issue = issue_of(&error);
        assert_eq!((issue.span.start.line, issue.span.start.column), (3, 5));
        assert_eq!(issue.span.bytes, ByteRange::new(17, 17));
        let plain = issue_of(&Error::new(ErrorCode::DocParse, "bad"));
        assert_eq!((plain.span.start.line, plain.span.start.column), (1, 1));
    }

    #[test]
    fn external_xml_entities_are_never_enabled() {
        assert!(!DocOptions::default().xml_external_entities());
    }
}
