// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Format registry and format detection.
//!
//! Every format implements [`DocFormat`] and is registered in a
//! [`Registry`]. The registry detects the format of a file from the
//! `format` parameter, the file name (`.env`, `Cargo.lock`, `pom.xml`), the
//! extension and the content, enforces `documents.max_bytes`, decodes text
//! formats from UTF-8 with or without a byte order mark and from UTF-16, and
//! checks on opening that the document keeps the file byte for byte.
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
use std::path::Path;

use nurt_contracts::config::{DocumentsConfig, YamlVersion};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{
    DocFormatId, DocFormatInfo, IssueSeverity, SyntaxIssue, TextPosition, TextSpan,
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

    /// File names of the format, lower case; a name ending with `*` matches
    /// every name with that prefix (`.env.*`).
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

/// Syntax issue describing an error with optional `line`, `column` and
/// `byte` details.
#[must_use]
pub fn issue_of(error: &Error) -> SyntaxIssue {
    let detail = |name: &str| error.details.get(name).and_then(serde_json::Value::as_u64);
    let position = TextPosition {
        line: detail("line").and_then(|line| u32::try_from(line).ok()).unwrap_or(1),
        column: detail("column").and_then(|column| u32::try_from(column).ok()).unwrap_or(1),
    };
    let byte = detail("byte").unwrap_or_default();
    SyntaxIssue {
        severity: IssueSeverity::Error,
        message: error.message.clone(),
        span: TextSpan {
            bytes: nurt_contracts::services::ByteRange::new(byte, byte),
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
                extensions: format.extensions().iter().map(|ext| (*ext).to_owned()).collect(),
                editable: format.editable(),
            })
            .collect()
    }

    /// Detects the format from the file name and the content.
    ///
    /// A file name of a format wins; then the formats with the extension,
    /// the one whose sniffing is the most certain; then sniffing alone, at
    /// least [`Confidence::Medium`].
    #[must_use]
    pub fn detect(&self, path: Option<&Path>, content: &[u8]) -> Option<DocFormatId> {
        let name = path
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().to_lowercase());
        if let Some(name) = &name {
            if let Some(format) = self.formats.iter().find(|format| matches_name(*format, name)) {
                return Some(format.id());
            }
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
        if let Some(format) = most_certain(&by_extension, content, Confidence::None) {
            return Some(format);
        }
        let all: Vec<&dyn DocFormat> = self.formats.iter().map(AsRef::as_ref).collect();
        most_certain(&all, content, Confidence::Medium)
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
            let known: Vec<String> = self.formats.iter().map(|format| format.id().to_string()).collect();
            return self
                .formats
                .iter()
                .map(|format| format.id())
                .find(|id| id.as_str() == name)
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::UnknownValue,
                        format!("unknown document format `{name}`; use one of: {}", known.join(", ")),
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
    pub fn open(&self, id: &DocFormatId, bytes: &[u8], options: &DocOptions) -> Result<OpenDocument> {
        let format = self.get(id).ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownValue,
                format!("unknown document format `{id}`"),
            )
            .with_found(id.as_str())
        })?;
        open_with(format, bytes, options)
    }
}

/// Opens a document with `format`, as [`Registry::open`] does.
///
/// # Errors
///
/// Fails as described for [`Registry::open`].
pub fn open_with(format: &dyn DocFormat, bytes: &[u8], options: &DocOptions) -> Result<OpenDocument> {
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
    let (encoding, text) = match format.input() {
        InputKind::Utf8Text => TextEncoding::decode(bytes)?,
        InputKind::Bytes => (TextEncoding::Utf8, bytes.to_vec()),
    };
    let handle = format.open(text.clone(), options)?;
    check_round_trip(&text, handle.text())?;
    Ok(OpenDocument { handle, encoding })
}

fn matches_name(format: &dyn DocFormat, name: &str) -> bool {
    format.file_names().iter().any(|pattern| match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *pattern,
    })
}

/// Format with the highest confidence above `floor`; the first registered
/// wins a tie, and a single candidate with no confidence wins when `floor`
/// is [`Confidence::None`].
fn most_certain(formats: &[&dyn DocFormat], content: &[u8], floor: Confidence) -> Option<DocFormatId> {
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
