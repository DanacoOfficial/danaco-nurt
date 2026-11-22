// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Request envelope `nurt-request/1` and response `nurt-response/1`.
//!
//! A [`Request`] carries exactly one input and the call options shared by
//! every command ([`RequestOptions`]). The input is program text, a syntax
//! tree, the cursor of a further page or the identifier of a correction
//! candidate to accept. The same envelope is accepted by the C interface
//! (`nurt_run_json`), by `nurt serve`, by the `nurt` tool of the integrations
//! and by instances on remote devices. The result is a [`Response`] carrying the
//! effect report and, for text formats, the rendered text.
//!
//! Both documents reject unknown fields: a misspelt option must never be
//! ignored silently. An envelope that is not valid JSON or does not match
//! the schema is a call error of the interface; an envelope that matches the
//! schema but breaks a rule checked by [`Request::validate`] (for example two
//! inputs at once) is reported as the language error `E_ARG` in the report.
//!
//! # Examples
//!
//! ```
//! use nurt_contracts::envelope::{Request, RequestInput};
//!
//! let request: Request = serde_json::from_str(
//!     r#"{"nurt": "1.0", "program": "find \"/srv/app\"", "options": {"limit": 20}}"#,
//! )
//! .unwrap();
//! assert!(matches!(request.input(), Ok(RequestInput::Program { .. })));
//! assert_eq!(request.options.limit(), 20);
//! ```

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ast::AstDocument;
use crate::kinds::SyntaxVariant;
use crate::report::{Report, ReportViolation, is_supported_version};

/// Name of the request format.
pub const REQUEST_FORMAT: &str = "nurt-request/1";

/// Name of the response format.
pub const RESPONSE_FORMAT: &str = "nurt-response/1";

/// Version written in the `nurt` field of requests and responses produced
/// by this version of the contract.
pub const ENVELOPE_VERSION: &str = "1.0";

/// Stable identifier (`$id`) of the published request schema, which also
/// describes the response in `$defs/response`.
pub const REQUEST_SCHEMA_ID: &str =
    "https://nurt.danaco-group.pl/schema/nurt-request-1.schema.json";

/// Default number of records or changes on one page.
pub const DEFAULT_LIMIT: u32 = 50;

/// Largest accepted number of records or changes on one page.
pub const MAX_LIMIT: u32 = 5000;

/// Default number of context lines in differences.
pub const DEFAULT_DIFF_CONTEXT: u32 = 3;

/// Default time limit of a whole program.
pub const DEFAULT_TIMEOUT: &str = "120s";

/// Format of the rendered result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OutputFormat {
    /// Plain text with aligned tables.
    #[default]
    Text,
    /// The report as JSON only; nothing is rendered.
    Json,
    /// Markdown with tables.
    Markdown,
    /// One record per line, without headings.
    Compact,
}

impl OutputFormat {
    /// Every format, in declaration order.
    pub const ALL: &'static [Self] = &[Self::Text, Self::Json, Self::Markdown, Self::Compact];

    /// Returns the stable code of the format, as written in JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
            Self::Markdown => "markdown",
            Self::Compact => "compact",
        }
    }
}

/// Level of detail of the effect report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Verbosity {
    /// Status, operation number, targets, summary, notable and irreversible
    /// effects; no list of changes and no differences.
    Quiet,
    /// Summary, changes up to the page limit, a sample of differences and the
    /// bulk summary of bulk operations.
    #[default]
    Normal,
    /// All changes and differences, paged, and the durations of the
    /// processing steps.
    Detailed,
}

impl Verbosity {
    /// Every level, in declaration order.
    pub const ALL: &'static [Self] = &[Self::Quiet, Self::Normal, Self::Detailed];

    /// Returns the stable code of the level, as written in JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Normal => "normal",
            Self::Detailed => "detailed",
        }
    }
}

/// Form of differences in the report of a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DiffMode {
    /// No differences.
    None,
    /// Numbers of added and removed lines only.
    Stat,
    /// Unified differences.
    #[default]
    Unified,
}

impl DiffMode {
    /// Every mode, in declaration order.
    pub const ALL: &'static [Self] = &[Self::None, Self::Stat, Self::Unified];

    /// Returns the stable code of the mode, as written in JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Stat => "stat",
            Self::Unified => "unified",
        }
    }
}

/// Language of messages, help and hints; codes, fields and keywords are
/// always English.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Language {
    /// English.
    #[default]
    En,
    /// Polish.
    Pl,
}

impl Language {
    /// Every language, in declaration order.
    pub const ALL: &'static [Self] = &[Self::En, Self::Pl];

    /// Returns the stable code of the language, as written in JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Pl => "pl",
        }
    }
}

/// Call options shared by every command.
///
/// An absent option takes its value from the configuration, and without a
/// configured value from the built-in default returned by the accessor
/// methods. Options given in the call override the configuration.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestOptions {
    /// Only compute targets and effect; change nothing. Default `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<bool>,
    /// Absolute directory against which relative paths are resolved; it is
    /// not a current directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Number of records or changes per page, from 1 to [`MAX_LIMIT`].
    /// Default [`DEFAULT_LIMIT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 5000))]
    pub limit: Option<u32>,
    /// Fields of the result to keep, like a final `select`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
    /// Format of the rendered result. Default `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<OutputFormat>,
    /// Level of detail of the report. Default `normal`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<Verbosity>,
    /// Form of differences. Default `unified`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<DiffMode>,
    /// Context lines of differences. Default [`DEFAULT_DIFF_CONTEXT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_context: Option<u32>,
    /// Largest number of characters of a single text field of the result;
    /// `0` means no limit. Shortened fields are marked `truncated: true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<u64>,
    /// Absolute path of a file that receives the full result (UTF-8) instead
    /// of the response; the report then gives `result_file`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_to: Option<String>,
    /// Time limit of the whole program, for example `120s` or `1h30m`.
    /// Default [`DEFAULT_TIMEOUT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
    /// Session identifier recorded in the journal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Language of messages and help. Default `en`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
    /// Script mode: several pipelines, each its own operation. Default
    /// `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<bool>,
}

impl RequestOptions {
    /// Whether no option is given.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Effective `preview`.
    #[must_use]
    pub fn preview(&self) -> bool {
        self.preview.unwrap_or(false)
    }

    /// Effective `limit`.
    #[must_use]
    pub fn limit(&self) -> u32 {
        self.limit.unwrap_or(DEFAULT_LIMIT)
    }

    /// Effective `format`.
    #[must_use]
    pub fn format(&self) -> OutputFormat {
        self.format.unwrap_or_default()
    }

    /// Effective `verbosity`.
    #[must_use]
    pub fn verbosity(&self) -> Verbosity {
        self.verbosity.unwrap_or_default()
    }

    /// Effective `diff`.
    #[must_use]
    pub fn diff(&self) -> DiffMode {
        self.diff.unwrap_or_default()
    }

    /// Effective `diff_context`.
    #[must_use]
    pub fn diff_context(&self) -> u32 {
        self.diff_context.unwrap_or(DEFAULT_DIFF_CONTEXT)
    }

    /// Effective `max_chars`; `0` means no limit.
    #[must_use]
    pub fn max_chars(&self) -> u64 {
        self.max_chars.unwrap_or(0)
    }

    /// Effective `script`.
    #[must_use]
    pub fn script(&self) -> bool {
        self.script.unwrap_or(false)
    }

    /// Checks the options.
    ///
    /// # Errors
    ///
    /// Returns the first option that breaks its rule.
    pub fn validate(&self) -> Result<(), EnvelopeError> {
        if let Some(limit) = self.limit.filter(|limit| !(1..=MAX_LIMIT).contains(limit)) {
            return Err(EnvelopeError::LimitOutOfRange { limit });
        }
        if let Some(base) = self.base.as_deref().filter(|base| !is_absolute_path(base)) {
            return Err(EnvelopeError::RelativePath {
                option: "base",
                path: base.to_owned(),
            });
        }
        if let Some(path) = self
            .save_to
            .as_deref()
            .filter(|path| !is_absolute_path(path))
        {
            return Err(EnvelopeError::RelativePath {
                option: "save_to",
                path: path.to_owned(),
            });
        }
        if let Some(timeout) = self
            .timeout
            .as_deref()
            .filter(|timeout| !is_duration(timeout))
        {
            return Err(EnvelopeError::InvalidTimeout {
                timeout: timeout.to_owned(),
            });
        }
        if self.session.as_deref() == Some("") {
            return Err(EnvelopeError::EmptyField { field: "session" });
        }
        if self
            .fields
            .as_ref()
            .is_some_and(|fields| fields.iter().any(String::is_empty))
        {
            return Err(EnvelopeError::EmptyField { field: "fields" });
        }
        Ok(())
    }
}

/// Request envelope (`nurt-request/1`).
///
/// Exactly one of `program`, `ast`, `cursor` and `accept` is present;
/// `variant` is allowed only with `program`. [`Request::input`] returns the
/// input and enforces the rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Format version, `MAJOR.MINOR`; see [`ENVELOPE_VERSION`].
    pub nurt: String,
    /// Program text in any notation variant; the instance parses and
    /// corrects it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// Notation variant of `program`. Default: the configured variant, with
    /// the other variants accepted when tolerant parsing is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<SyntaxVariant>,
    /// Surface syntax tree `nurt-ast/1`, equivalent to `program`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ast: Option<Box<AstDocument>>,
    /// Cursor of a further page of an earlier result; reading a page never
    /// runs a changing program again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Identifier of a candidate from an earlier `E_AMBIGUOUS` error, for
    /// example `c1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<String>,
    /// Call options.
    #[serde(default, skip_serializing_if = "RequestOptions::is_empty")]
    pub options: RequestOptions,
}

/// The single input of a [`Request`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum RequestInput<'a> {
    /// Program text.
    Program {
        /// The text.
        text: &'a str,
        /// Notation variant, when given.
        variant: Option<SyntaxVariant>,
    },
    /// Surface syntax tree.
    Ast(&'a AstDocument),
    /// Cursor of a further page.
    Cursor(&'a str),
    /// Identifier of a correction candidate to accept.
    Accept(&'a str),
}

impl Request {
    fn with_input(
        program: Option<String>,
        ast: Option<Box<AstDocument>>,
        cursor: Option<String>,
        accept: Option<String>,
    ) -> Self {
        Self {
            nurt: ENVELOPE_VERSION.to_owned(),
            program,
            variant: None,
            ast,
            cursor,
            accept,
            options: RequestOptions::default(),
        }
    }

    /// Creates a request that runs program text.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::envelope::Request;
    ///
    /// let request = Request::program("find \"/srv/app\"");
    /// assert!(request.validate().is_ok());
    /// ```
    #[must_use]
    pub fn program(text: impl Into<String>) -> Self {
        Self::with_input(Some(text.into()), None, None, None)
    }

    /// Creates a request that runs a surface syntax tree.
    #[must_use]
    pub fn ast(document: AstDocument) -> Self {
        Self::with_input(None, Some(Box::new(document)), None, None)
    }

    /// Creates a request that reads a further page of an earlier result.
    #[must_use]
    pub fn cursor(cursor: impl Into<String>) -> Self {
        Self::with_input(None, None, Some(cursor.into()), None)
    }

    /// Creates a request that accepts a correction candidate.
    #[must_use]
    pub fn accept(candidate: impl Into<String>) -> Self {
        Self::with_input(None, None, None, Some(candidate.into()))
    }

    /// Returns the request with the given options.
    #[must_use]
    pub fn with_options(mut self, options: RequestOptions) -> Self {
        self.options = options;
        self
    }

    /// Returns the single input of the request.
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError::MissingInput`] when no input is given,
    /// [`EnvelopeError::ConflictingInputs`] when several are given,
    /// [`EnvelopeError::VariantWithoutProgram`] when `variant` accompanies
    /// another input and [`EnvelopeError::EmptyField`] for an empty cursor
    /// or candidate identifier.
    pub fn input(&self) -> Result<RequestInput<'_>, EnvelopeError> {
        let given: Vec<&'static str> = [
            ("program", self.program.is_some()),
            ("ast", self.ast.is_some()),
            ("cursor", self.cursor.is_some()),
            ("accept", self.accept.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, present)| present.then_some(name))
        .collect();
        if given.len() > 1 {
            return Err(EnvelopeError::ConflictingInputs { fields: given });
        }
        if self.variant.is_some() && self.program.is_none() {
            return Err(EnvelopeError::VariantWithoutProgram);
        }
        if let Some(text) = self.program.as_deref() {
            return Ok(RequestInput::Program {
                text,
                variant: self.variant,
            });
        }
        if let Some(document) = self.ast.as_deref() {
            return Ok(RequestInput::Ast(document));
        }
        if let Some(cursor) = self.cursor.as_deref() {
            return non_empty(cursor, "cursor").map(RequestInput::Cursor);
        }
        if let Some(candidate) = self.accept.as_deref() {
            return non_empty(candidate, "accept").map(RequestInput::Accept);
        }
        Err(EnvelopeError::MissingInput)
    }

    /// Checks the version, the input and the options.
    ///
    /// # Errors
    ///
    /// Returns the first rule that the request breaks.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::envelope::{EnvelopeError, Request};
    ///
    /// let mut request = Request::program("find \"/srv/app\"");
    /// request.cursor = Some("c_91fe".to_owned());
    /// assert!(matches!(
    ///     request.validate(),
    ///     Err(EnvelopeError::ConflictingInputs { .. })
    /// ));
    /// ```
    pub fn validate(&self) -> Result<(), EnvelopeError> {
        if !is_supported_version(&self.nurt) {
            return Err(EnvelopeError::UnsupportedVersion {
                found: self.nurt.clone(),
            });
        }
        self.input()?;
        self.options.validate()
    }
}

fn non_empty<'a>(value: &'a str, field: &'static str) -> Result<&'a str, EnvelopeError> {
    if value.is_empty() {
        Err(EnvelopeError::EmptyField { field })
    } else {
        Ok(value)
    }
}

/// Text rendered from the report in the requested format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Rendered {
    /// Format of the text; never `json`.
    pub format: OutputFormat,
    /// The rendered text (UTF-8).
    pub text: String,
}

/// Response to a request (`nurt-response/1`).
///
/// The report is always present. The rendered text is present for the
/// formats `text`, `markdown` and `compact`; for `json` the report itself is
/// the result. Whether a further page exists follows from
/// `report.next_cursor` ([`Response::has_more`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Response {
    /// Format version, `MAJOR.MINOR`; see [`ENVELOPE_VERSION`].
    pub nurt: String,
    /// Effect report.
    pub report: Report,
    /// Text rendered from the report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered: Option<Rendered>,
}

impl Response {
    /// Creates a response without rendered text.
    #[must_use]
    pub fn new(report: Report) -> Self {
        Self {
            nurt: ENVELOPE_VERSION.to_owned(),
            report,
            rendered: None,
        }
    }

    /// Whether there is a further page of the result.
    #[must_use]
    pub fn has_more(&self) -> bool {
        self.report.has_more()
    }

    /// Checks the version, the rendered text and the report.
    ///
    /// # Errors
    ///
    /// Returns the first rule that the response breaks.
    pub fn validate(&self) -> Result<(), EnvelopeError> {
        if !is_supported_version(&self.nurt) {
            return Err(EnvelopeError::UnsupportedVersion {
                found: self.nurt.clone(),
            });
        }
        if self
            .rendered
            .as_ref()
            .is_some_and(|rendered| rendered.format == OutputFormat::Json)
        {
            return Err(EnvelopeError::RenderedJson);
        }
        self.report.validate().map_err(EnvelopeError::Report)
    }
}

/// A rule of the envelope broken by a [`Request`] or [`Response`].
///
/// The executor reports a broken request rule as the language error `E_ARG`
/// with the message of this error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvelopeError {
    /// The `nurt` field does not hold a supported version.
    UnsupportedVersion {
        /// Version found in the envelope.
        found: String,
    },
    /// None of `program`, `ast`, `cursor` and `accept` is given.
    MissingInput,
    /// More than one of `program`, `ast`, `cursor` and `accept` is given.
    ConflictingInputs {
        /// Names of the inputs given.
        fields: Vec<&'static str>,
    },
    /// `variant` is given without `program`.
    VariantWithoutProgram,
    /// A field that must not be empty is empty.
    EmptyField {
        /// Name of the field.
        field: &'static str,
    },
    /// `limit` lies outside 1 to [`MAX_LIMIT`].
    LimitOutOfRange {
        /// Value given.
        limit: u32,
    },
    /// A path option is not absolute.
    RelativePath {
        /// Name of the option.
        option: &'static str,
        /// Value given.
        path: String,
    },
    /// `timeout` is not a duration such as `30s`, `5m` or `1h30m`.
    InvalidTimeout {
        /// Value given.
        timeout: String,
    },
    /// A response renders text in the format `json`.
    RenderedJson,
    /// The report of a response breaks a rule of the report format.
    Report(ReportViolation),
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { found } => {
                write!(
                    f,
                    "unsupported envelope version `{found}`; expected `1.<minor>`"
                )
            }
            Self::MissingInput => f.write_str("give one of `program`, `ast`, `cursor` or `accept`"),
            Self::ConflictingInputs { fields } => write!(
                f,
                "give only one of `program`, `ast`, `cursor` or `accept`; given: {}",
                fields.join(", ")
            ),
            Self::VariantWithoutProgram => f.write_str("`variant` applies only to `program`"),
            Self::EmptyField { field } => write!(f, "`{field}` must not be empty"),
            Self::LimitOutOfRange { limit } => {
                write!(f, "`limit` is {limit}; expected 1 to {MAX_LIMIT}")
            }
            Self::RelativePath { option, path } => {
                write!(f, "`{option}` must be an absolute path; given `{path}`")
            }
            Self::InvalidTimeout { timeout } => write!(
                f,
                "`timeout` must be a duration such as `30s` or `1h30m`; given `{timeout}`"
            ),
            Self::RenderedJson => f.write_str("the format `json` is not rendered as text"),
            Self::Report(violation) => write!(f, "invalid report: {violation}"),
        }
    }
}

impl std::error::Error for EnvelopeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Report(violation) => Some(violation),
            _ => None,
        }
    }
}

/// Whether `path` is absolute in the canonical path notation.
///
/// Accepted forms: `/srv/app`, a drive letter with a separator (`C:/proj`,
/// `C:\proj`), a UNC path (`//server/share`, `\\server\share`) and a
/// provider path (`registry:/HKCU/Software`).
///
/// # Examples
///
/// ```
/// use nurt_contracts::envelope::is_absolute_path;
///
/// assert!(is_absolute_path("/home/user"));
/// assert!(is_absolute_path("C:/Users/user"));
/// assert!(!is_absolute_path("notes.txt"));
/// ```
#[must_use]
pub fn is_absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    match bytes {
        [b'/', ..] | [b'\\', b'\\', ..] => true,
        [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic() => true,
        _ => is_provider_path(path),
    }
}

fn is_provider_path(path: &str) -> bool {
    path.split_once(":/").is_some_and(|(provider, _)| {
        provider.len() > 1
            && provider
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    })
}

/// Whether `text` is a duration: one or more pairs of a number and a unit
/// `ms`, `s`, `m`, `h` or `d`, for example `250ms`, `30s` or `1h30m`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::envelope::is_duration;
///
/// assert!(is_duration("120s"));
/// assert!(is_duration("1h30m"));
/// assert!(!is_duration("120"));
/// assert!(!is_duration("s"));
/// ```
#[must_use]
pub fn is_duration(text: &str) -> bool {
    let mut rest = text.as_bytes();
    if rest.is_empty() {
        return false;
    }
    while !rest.is_empty() {
        let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 {
            return false;
        }
        rest = &rest[digits..];
        let unit = match rest {
            [b'm', b's', ..] => 2,
            [b's' | b'm' | b'h' | b'd', ..] => 1,
            _ => return false,
        };
        rest = &rest[unit..];
    }
    true
}
