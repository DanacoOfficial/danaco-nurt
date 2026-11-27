// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-contracts::services
// Responsibility:   Services shared between command crates of different domains.
// Dependencies:     schemars, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Services shared between command crates of different domains.
//!
//! Command crates depend only on the contracts. When a command of one domain
//! needs a capability implemented in another domain (reading a key of a
//! structured document while reading a file, outlining source code, running a
//! language server, listing files with the exclusions of `find`), it calls a
//! service trait defined here. The engine registers the implementations in a
//! [`Services`] value carried by the execution context
//! ([`ExecCtx`](crate::stage::ExecCtx)).
//!
//! A missing service is not an error: the group that provides it is not
//! loaded, and the calling command applies its documented fallback (for
//! example `read view: outline` returns the full content and reports that the
//! outline was unavailable).
//!
//! | Trait | Implemented by | Used by |
//! |---|---|---|
//! | [`DocService`] | structured documents | `read key:`, `write format:`, `validate`, test and analysis commands |
//! | [`CodeService`] | source code analysis | `read symbol:`, `read view:`, `search mode: syntax`, `search scope:`, `validate`, reference checks, `git.diff symbol:`, `analyze.check` |
//! | [`ProcessHost`] | program execution | language server client, git hooks, service and package managers |
//! | [`FileListService`] | file commands | repository-wide code commands |
//! | [`GitQueryService`] | repository commands | `code.profile`, `code.diagnostics changed:`, `info details: git`, `analyze.* changed_only` |
//! | [`TextCodecService`] | text commands | document and code commands that decode or encode files |
//! | [`FileHolderService`] | system commands | file layer (`E_IN_USE` with holders), `open_files` |
//! | [`ReviewRuleProvider`] | engine or extension | `git.review checks: [secrets]` |
//!
//! All service methods are synchronous and free of hidden global state; the
//! document and code services work on bytes passed by the caller, so they can
//! check content that is not yet written.

use std::any::Any;
use std::borrow::Cow;
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::env::WallTime;
use crate::error::Result;
use crate::keypath::KeyPath;
use crate::stage::CancelToken;
use crate::value::{Record, Value};

/// Error returned when a format or language identifier is malformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidServiceId {
    /// The rejected text.
    pub value: String,
}

impl fmt::Display for InvalidServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a valid identifier: expected a lowercase ASCII letter followed by \
             lowercase letters, digits, `_`, `-` or `+`",
            self.value
        )
    }
}

impl std::error::Error for InvalidServiceId {}

/// Longest format or language identifier, in bytes.
const MAX_ID_LEN: usize = 64;

const fn is_valid_id(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_ID_LEN || !bytes[0].is_ascii_lowercase() {
        return false;
    }
    let mut i = 1;
    while i < bytes.len() {
        let b = bytes[i];
        if !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-' || b == b'+') {
            return false;
        }
        i += 1;
    }
    true
}

macro_rules! service_id {
    ($(#[$meta:meta])* $name:ident, $schema_name:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Cow<'static, str>);

        impl $name {
            /// Checks and wraps an identifier: a lowercase ASCII letter
            /// followed by up to 63 lowercase letters, digits, `_`, `-` or
            /// `+`.
            ///
            /// # Errors
            ///
            /// Returns [`InvalidServiceId`] when the text does not have this
            /// form.
            pub fn new(id: impl Into<String>) -> std::result::Result<Self, InvalidServiceId> {
                let id = id.into();
                if is_valid_id(&id) {
                    Ok(Self(Cow::Owned(id)))
                } else {
                    Err(InvalidServiceId { value: id })
                }
            }

            /// The identifier text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = InvalidServiceId;

            fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
                Self::new(text)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                Self::new(text).map_err(D::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed($schema_name)
            }

            fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "string",
                    "pattern": "^[a-z][a-z0-9_+-]{0,63}$"
                })
            }
        }
    };
}

service_id!(
    /// Identifier of a structured document format (`json`, `yaml`, …).
    ///
    /// The set is open: extensions register further formats. The constants
    /// name the formats built into the documents group.
    ///
    /// ```
    /// use nurt_contracts::services::DocFormatId;
    ///
    /// assert_eq!(DocFormatId::YAML.as_str(), "yaml");
    /// assert_eq!("yaml".parse::<DocFormatId>()?, DocFormatId::YAML);
    /// assert!(DocFormatId::new("YAML").is_err());
    /// # Ok::<(), nurt_contracts::services::InvalidServiceId>(())
    /// ```
    DocFormatId,
    "DocFormatId"
);

service_id!(
    /// Identifier of a programming language (`rust`, `python`, `typescript`,
    /// …), as listed by [`CodeService::languages`].
    ///
    /// The set is open: grammars are added by the code group and by
    /// extensions.
    ///
    /// ```
    /// use nurt_contracts::services::LanguageId;
    ///
    /// let rust = LanguageId::new("rust")?;
    /// assert_eq!(rust.to_string(), "rust");
    /// # Ok::<(), nurt_contracts::services::InvalidServiceId>(())
    /// ```
    LanguageId,
    "LanguageId"
);

impl DocFormatId {
    /// JSON.
    pub const JSON: Self = Self(Cow::Borrowed("json"));
    /// JSON with comments and trailing commas.
    pub const JSONC: Self = Self(Cow::Borrowed("jsonc"));
    /// YAML (1.2 by default).
    pub const YAML: Self = Self(Cow::Borrowed("yaml"));
    /// TOML.
    pub const TOML: Self = Self(Cow::Borrowed("toml"));
    /// XML.
    pub const XML: Self = Self(Cow::Borrowed("xml"));
    /// INI.
    pub const INI: Self = Self(Cow::Borrowed("ini"));
    /// Comma-separated values.
    pub const CSV: Self = Self(Cow::Borrowed("csv"));
    /// Tab-separated values.
    pub const TSV: Self = Self(Cow::Borrowed("tsv"));
    /// Java properties.
    pub const PROPERTIES: Self = Self(Cow::Borrowed("properties"));
    /// Environment files (`.env`).
    pub const ENV: Self = Self(Cow::Borrowed("env"));
    /// Property lists (XML and binary).
    pub const PLIST: Self = Self(Cow::Borrowed("plist"));

    /// Formats built into the documents group.
    pub const BUILT_IN: [Self; 11] = [
        Self::JSON,
        Self::JSONC,
        Self::YAML,
        Self::TOML,
        Self::XML,
        Self::INI,
        Self::CSV,
        Self::TSV,
        Self::PROPERTIES,
        Self::ENV,
        Self::PLIST,
    ];
}

/// Byte range `start..end` in a document (zero-based, end exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
pub struct ByteRange {
    /// Offset of the first byte.
    pub start: u64,
    /// Offset after the last byte.
    pub end: u64,
}

impl ByteRange {
    /// Range `start..end`.
    pub const fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    /// Number of bytes in the range (zero when `end <= start`).
    pub const fn len(self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// Returns `true` for an empty range.
    pub const fn is_empty(self) -> bool {
        self.end <= self.start
    }

    /// Returns `true` when `offset` lies in the range.
    pub const fn contains(self, offset: u64) -> bool {
        self.start <= offset && offset < self.end
    }
}

/// Position in a text: one-based line and one-based column counted in
/// Unicode scalar values, the convention of records such as `match` and
/// `symbol`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct TextPosition {
    /// Line, starting at 1.
    pub line: u32,
    /// Column, starting at 1.
    pub column: u32,
}

/// Region of a document as bytes and as start and end positions.
///
/// `end` is the position after the last character, so an empty span has
/// `start == end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct TextSpan {
    /// Bytes of the region.
    pub bytes: ByteRange,
    /// Position of the first character.
    pub start: TextPosition,
    /// Position after the last character.
    pub end: TextPosition,
}

/// Severity of a syntax issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IssueSeverity {
    /// The content does not parse.
    Error,
    /// The content parses, but a construct is suspicious (for example a
    /// duplicate key).
    Warning,
}

/// Syntax problem found in a document or in source code.
///
/// Callers that validate a change compare the issues of the content before
/// and after the change; only new issues reject it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyntaxIssue {
    /// Severity.
    pub severity: IssueSeverity,
    /// Description in English.
    pub message: String,
    /// Region of the problem.
    pub span: TextSpan,
    /// Short machine-readable kind, for example `missing`, `unexpected` or
    /// `duplicate_key`.
    pub kind: Option<String>,
}

/// Structured document format known to a [`DocService`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DocFormatInfo {
    /// Identifier.
    pub id: DocFormatId,
    /// File name extensions without the leading period (`json`, `yml`).
    pub extensions: Vec<String>,
    /// Keys can be changed with the layout preserved.
    pub editable: bool,
}

/// Value read from a structured document at one key path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DocItem {
    /// Concrete key path of the value, in canonical form.
    pub key: KeyPath,
    /// The value; maps keep the order of the document.
    pub value: Value,
    /// Region of the value in the document.
    pub span: Option<TextSpan>,
    /// Comment attached to the value, without comment markers.
    pub comment: Option<String>,
    /// Zero-based document index in a multi-document stream.
    pub document: Option<u32>,
    /// Anchor defined on the value (YAML).
    pub anchor: Option<String>,
    /// Anchor this value is an alias of (YAML).
    pub alias_of: Option<String>,
    /// Explicit tag of the value (YAML).
    pub tag: Option<String>,
}

/// Options of [`DocService::serialize_records`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SerializeOptions {
    /// Header row for tabular formats.
    pub header: bool,
    /// Field delimiter for tabular formats; `None` uses the format default.
    pub delimiter: Option<char>,
    /// Indented output where the format allows it.
    pub pretty: bool,
}

impl Default for SerializeOptions {
    fn default() -> Self {
        Self {
            header: true,
            delimiter: None,
            pretty: true,
        }
    }
}

/// Structured documents: format detection, reading by key, syntax checks and
/// serialization of records.
///
/// Implemented by the documents group. Changing documents is not part of this
/// service: commands that change documents belong to the documents group
/// itself and are dispatched by record type.
pub trait DocService: Send + Sync {
    /// Formats this service understands.
    fn formats(&self) -> Vec<DocFormatInfo>;

    /// Detects the format from the file name (when known) and the content;
    /// `None` when the content is not a structured document of a known
    /// format.
    fn detect_format(&self, path: Option<&Path>, bytes: &[u8]) -> Option<DocFormatId>;

    /// Reads the values selected by `key`.
    ///
    /// `depth` limits how many levels of nested maps and lists are expanded
    /// in each value; `None` expands all levels. A key that selects nothing
    /// returns an empty list; the caller decides whether that is an error.
    fn read_keys(
        &self,
        bytes: &[u8],
        format: &DocFormatId,
        key: &KeyPath,
        depth: Option<u32>,
    ) -> Result<Vec<DocItem>>;

    /// Checks the syntax of the content; an empty list means the content
    /// parses.
    ///
    /// # Errors
    ///
    /// Fails only when the format is not supported.
    fn check_syntax(&self, bytes: &[u8], format: &DocFormatId) -> Result<Vec<SyntaxIssue>>;

    /// Writes records as a document of the format (`write format:`).
    fn serialize_records(
        &self,
        records: &[Record],
        format: &DocFormatId,
        options: &SerializeOptions,
    ) -> Result<Vec<u8>>;
}

/// Programming language known to a [`CodeService`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LanguageInfo {
    /// Identifier.
    pub id: LanguageId,
    /// Display name.
    pub name: String,
    /// File name extensions without the leading period.
    pub extensions: Vec<String>,
    /// Version of the grammar.
    pub grammar_version: Option<String>,
}

/// Kind of a symbol, matching the `kind` parameter of `code.symbols`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SymbolKind {
    /// Free function.
    Function,
    /// Function bound to a type.
    Method,
    /// Class.
    Class,
    /// Structure or record type.
    Struct,
    /// Interface, trait or protocol.
    Interface,
    /// Enumeration.
    Enum,
    /// Variable.
    Variable,
    /// Constant.
    Constant,
    /// Field or property of a type.
    Field,
    /// Module, namespace or package.
    Module,
    /// Type alias or other type definition.
    Type,
    /// Macro.
    Macro,
}

/// Symbol defined in source code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SymbolInfo {
    /// Name as written.
    pub name: String,
    /// Kind.
    pub kind: SymbolKind,
    /// Qualified name of the enclosing symbol (`Container.inner`), if any.
    pub container: Option<String>,
    /// Nesting level in the outline, 0 for top-level symbols.
    pub depth: u32,
    /// Whole definition, including its body.
    pub span: TextSpan,
    /// The name inside the definition.
    pub name_span: TextSpan,
    /// Signature line without the body.
    pub signature: Option<String>,
    /// Visibility outside the module; `None` when the language does not
    /// define visibility.
    pub exported: Option<bool>,
}

/// Lexical class of a region of source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SpanClass {
    /// Code outside comments and string literals.
    Code,
    /// Comment, including documentation comments.
    Comment,
    /// String or character literal.
    String,
}

/// Region of source code with its lexical class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClassifiedSpan {
    /// Bytes of the region.
    pub bytes: ByteRange,
    /// Lexical class.
    pub class: SpanClass,
}

/// Compiled structural search pattern of [`CodeService::compile_pattern`].
///
/// The pattern is opaque to callers; only the service that compiled it can
/// run it, recovering its own type through [`SyntaxPattern::as_any`].
pub trait SyntaxPattern: fmt::Debug + Send + Sync {
    /// Language of the pattern.
    fn language(&self) -> &LanguageId;

    /// Pattern text as written.
    fn source(&self) -> &str;

    /// Names of the metavariables (`X` for `$X`, `ARGS` for `$$$ARGS`).
    fn metavariables(&self) -> Vec<String>;

    /// The concrete pattern type, for the service that created it.
    fn as_any(&self) -> &dyn Any;
}

/// Text captured by a metavariable of a structural pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyntaxCapture {
    /// Metavariable name without `$`.
    pub name: String,
    /// Captured nodes; one for `$X`, zero or more for `$$$X`.
    pub spans: Vec<TextSpan>,
    /// Captured text, from the first to the last node.
    pub text: String,
}

/// Match of a structural pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyntaxMatch {
    /// Region of the matched node.
    pub span: TextSpan,
    /// Grammar node kind of the matched node.
    pub node_kind: String,
    /// Matched text.
    pub text: String,
    /// Captures of the metavariables, in pattern order.
    pub captures: Vec<SyntaxCapture>,
}

/// Role of a reference to a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReferenceRole {
    /// The definition of the symbol.
    Definition,
    /// An import or use declaration.
    Import,
    /// Any other occurrence.
    Usage,
}

/// Occurrence of a name in source code found by syntax analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SymbolReference {
    /// Region of the name.
    pub span: TextSpan,
    /// Role of the occurrence.
    pub role: ReferenceRole,
    /// Lexical class; occurrences in comments and strings are reported so
    /// callers can count references left behind.
    pub class: SpanClass,
    /// Qualified name of the enclosing symbol.
    pub container: Option<String>,
    /// The whole line holding the occurrence, without the line ending.
    pub line_text: String,
}

/// Size and complexity of one function, for analysis commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FunctionMetrics {
    /// Name of the function or method.
    pub name: String,
    /// Qualified name of the enclosing symbol.
    pub container: Option<String>,
    /// Whole definition.
    pub span: TextSpan,
    /// Lines of the definition.
    pub lines: u32,
    /// Number of parameters.
    pub parameters: u32,
    /// Cyclomatic complexity (1 plus the number of decision points).
    pub cyclomatic: u32,
    /// Deepest nesting of blocks inside the body.
    pub max_nesting: u32,
}

/// Token of source code, for duplicate detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyntaxToken {
    /// Bytes of the token.
    pub bytes: ByteRange,
    /// Grammar node kind of the token (identifiers and literals keep their
    /// kind, so callers can normalize them).
    pub kind: String,
    /// Lexical class.
    pub class: SpanClass,
}

/// Syntax analysis of source code (tree-sitter grammars).
///
/// Implemented by the code group. Every method works on the bytes passed by
/// the caller and reads no files. A language without a grammar fails with
/// `E_UNSUPPORTED_LANGUAGE`; a pattern that does not parse fails with
/// `E_PATTERN_SYNTAX`.
pub trait CodeService: Send + Sync {
    /// Languages with a grammar.
    fn languages(&self) -> Vec<LanguageInfo>;

    /// Detects the language from the file name (when known) and the content
    /// (for example a shebang line); `None` when no grammar applies.
    fn language_of(&self, path: Option<&Path>, bytes: &[u8]) -> Option<LanguageId>;

    /// Symbols of the document in source order; `depth` limits the nesting
    /// level (`Some(0)` returns top-level symbols only).
    fn outline(
        &self,
        bytes: &[u8],
        language: &LanguageId,
        depth: Option<u32>,
    ) -> Result<Vec<SymbolInfo>>;

    /// Definitions named `name`; a qualified name (`Container.name`) narrows
    /// the container. An empty list means no definition; several entries
    /// mean the name is ambiguous.
    fn find_symbol(
        &self,
        bytes: &[u8],
        language: &LanguageId,
        name: &str,
    ) -> Result<Vec<SymbolInfo>>;

    /// Innermost symbol whose definition contains the byte `offset`.
    fn enclosing(
        &self,
        bytes: &[u8],
        language: &LanguageId,
        offset: u64,
    ) -> Result<Option<SymbolInfo>>;

    /// Comments and string literals of the document; regions not listed are
    /// code. The list is in source order and regions do not overlap.
    fn classify_spans(&self, bytes: &[u8], language: &LanguageId) -> Result<Vec<ClassifiedSpan>>;

    /// Syntax errors of the document; an empty list means the content parses.
    fn check_syntax(&self, bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxIssue>>;

    /// Compiles a structural pattern written in the syntax of the language,
    /// with metavariables `$X` (one node) and `$$$X` (zero or more nodes).
    fn compile_pattern(
        &self,
        pattern: &str,
        language: &LanguageId,
    ) -> Result<Box<dyn SyntaxPattern>>;

    /// Matches of a compiled pattern, in source order.
    fn syntax_search(&self, bytes: &[u8], pattern: &dyn SyntaxPattern) -> Result<Vec<SyntaxMatch>>;

    /// Replacement text for a match: the template with `$X` and `$$$X`
    /// replaced by the captured text.
    fn render_template(&self, found: &SyntaxMatch, template: &str) -> Result<String>;

    /// Occurrences of the identifier `name` in the document, including
    /// occurrences in comments and strings (marked by their class).
    fn references(
        &self,
        bytes: &[u8],
        language: &LanguageId,
        name: &str,
    ) -> Result<Vec<SymbolReference>>;

    /// Size and complexity of every function and method.
    fn function_metrics(&self, bytes: &[u8], language: &LanguageId)
    -> Result<Vec<FunctionMetrics>>;

    /// Tokens of the document in source order, without whitespace.
    fn tokens(&self, bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxToken>>;
}

/// Environment variable set or removed for a child process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvVar {
    /// Variable name.
    pub name: String,
    /// New value; `None` removes the variable.
    pub value: Option<String>,
}

/// Standard input of a program run by [`ProcessHost::run_program`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProgramInput {
    /// Empty input (`/dev/null`, `NUL`).
    #[default]
    Null,
    /// The given bytes, then end of input.
    Bytes(Vec<u8>),
}

/// Program to run to completion.
///
/// Programs are resolved and started by the same rules as the `run` command:
/// arguments are passed as a list without a shell, the program is looked up
/// in `PATH` (with `PATHEXT` on Windows), batch files are started through the
/// command interpreter with every argument quoted, there is no terminal, and
/// the whole process tree is stopped on timeout or cancellation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProgramSpec {
    /// Program name or path.
    pub program: String,
    /// Arguments, without the program name.
    pub args: Vec<String>,
    /// Working directory.
    pub cwd: PathBuf,
    /// Variables changed on top of the inherited environment.
    pub env: Vec<EnvVar>,
    /// Inherit the environment of the console process; `false` starts from
    /// an empty environment.
    pub inherit_env: bool,
    /// Standard input.
    pub stdin: ProgramInput,
    /// Time limit in milliseconds; `None` uses the configured default.
    pub timeout_ms: Option<u64>,
    /// Bytes kept from each of standard output and standard error; `None`
    /// uses the configured default.
    pub output_limit: Option<u64>,
    /// Report files changed under the working directory while the program
    /// ran.
    pub detect_changes: bool,
}

impl ProgramSpec {
    /// Specification with default settings: inherited environment, empty
    /// input, configured limits and no change detection.
    pub fn new(program: impl Into<String>, args: Vec<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args,
            cwd: cwd.into(),
            env: Vec::new(),
            inherit_env: true,
            stdin: ProgramInput::Null,
            timeout_ms: None,
            output_limit: None,
            detect_changes: false,
        }
    }
}

/// Result of [`ProcessHost::run_program`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ProgramOutcome {
    /// Resolved program file.
    pub program_path: PathBuf,
    /// Interpreter used to start the program (for example `cmd.exe` for
    /// batch files).
    pub launcher: Option<String>,
    /// Exit code; `None` when the process ended by a signal or was stopped.
    pub exit_code: Option<i32>,
    /// Signal that ended the process (Unix).
    pub signal: Option<i32>,
    /// The time limit was reached and the process tree was stopped.
    pub timed_out: bool,
    /// The operation was cancelled and the process tree was stopped.
    pub cancelled: bool,
    /// Standard output, up to the output limit.
    pub stdout: Vec<u8>,
    /// Standard error, up to the output limit.
    pub stderr: Vec<u8>,
    /// Standard output exceeded the output limit.
    pub stdout_truncated: bool,
    /// Standard error exceeded the output limit.
    pub stderr_truncated: bool,
    /// Wall time from start to exit, in milliseconds.
    pub duration_ms: u64,
    /// Files created, changed or removed under the working directory, when
    /// change detection was requested.
    pub changed_paths: Vec<PathBuf>,
}

/// Long-running helper process (for example a language server) started by
/// [`ProcessHost::spawn_service`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ServiceSpawnSpec {
    /// Program name or path, resolved as by [`ProgramSpec::program`].
    pub program: String,
    /// Arguments, without the program name.
    pub args: Vec<String>,
    /// Working directory.
    pub cwd: PathBuf,
    /// Variables changed on top of the inherited environment.
    pub env: Vec<EnvVar>,
    /// Inherit the environment of the console process.
    pub inherit_env: bool,
    /// Private temporary directory for the process (`TMPDIR`, `TEMP`).
    pub temp_dir: Option<PathBuf>,
}

/// How a child process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct ChildExit {
    /// Exit code; `None` when ended by a signal.
    pub exit_code: Option<i32>,
    /// Signal that ended the process (Unix).
    pub signal: Option<i32>,
}

/// Running helper process with piped standard streams.
///
/// The process runs in its own process group (Unix) or job object (Windows);
/// [`ServiceChild::kill_tree`] stops it with all its descendants, and dropping
/// the handle does the same. The handle is owned by one user at a time, so
/// the trait requires `Send` but not `Sync`.
pub trait ServiceChild: Send {
    /// Process identifier.
    fn pid(&self) -> u32;

    /// Wall-clock time the process was started.
    fn started(&self) -> WallTime;

    /// Writer to standard input; `None` once taken.
    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>>;

    /// Reader of standard output; `None` once taken.
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>>;

    /// Reader of standard error; `None` once taken.
    fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>>;

    /// Exit status if the process has ended, without waiting.
    fn try_wait(&mut self) -> Result<Option<ChildExit>>;

    /// Waits up to `timeout` for the process to end.
    fn wait_timeout(&mut self, timeout: Duration) -> Result<Option<ChildExit>>;

    /// Stops the process and all of its descendants.
    fn kill_tree(&mut self) -> Result<()>;
}

/// Program execution for commands outside the run group.
///
/// Implemented by the run group, so every program started by the console
/// follows one set of rules for resolution, quoting, environment, process
/// groups, limits and cancellation.
pub trait ProcessHost: Send + Sync {
    /// Resolves a program name to a file as `run` would, searching `PATH`
    /// of the console process.
    ///
    /// # Errors
    ///
    /// Fails with `E_EXTERNAL_MISSING` when the program is not found.
    fn resolve_program(&self, program: &str, cwd: &Path) -> Result<PathBuf>;

    /// Runs a program to completion and captures its output.
    ///
    /// A non-zero exit code is a normal outcome. Cancellation stops the
    /// process tree and returns an outcome with `cancelled` set.
    ///
    /// # Errors
    ///
    /// Fails only when the program cannot be started.
    fn run_program(&self, spec: &ProgramSpec, cancel: &CancelToken) -> Result<ProgramOutcome>;

    /// Starts a helper process with piped standard streams.
    fn spawn_service(&self, spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>>;
}

/// Named group of excluded directories, matching `exclude_preset` of `find`
/// and `search`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExcludePreset {
    /// Version control metadata (`.git`, `.hg`, `.svn`).
    Vcs,
    /// Installed dependencies (`node_modules`, `venv`, `.venv`, `vendor`).
    Deps,
    /// Build output (`target`, `build`, `dist`).
    Build,
    /// Caches (`__pycache__`, `.cache`, `.pytest_cache`).
    Cache,
}

/// Kind of entries returned by [`FileListService::walk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WalkKind {
    /// Files only.
    #[default]
    File,
    /// Directories only.
    Directory,
    /// Files and directories.
    Any,
}

/// File listing request, with the parameters and defaults of `find`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WalkSpec {
    /// Directory to list.
    pub root: PathBuf,
    /// Glob patterns relative to the root; an entry matching any of them is
    /// listed.
    pub globs: Vec<String>,
    /// Kind of entries.
    pub kind: WalkKind,
    /// Levels below the root; `None` is unlimited.
    pub max_depth: Option<u32>,
    /// Include hidden entries.
    pub hidden: bool,
    /// Follow symbolic links to directories.
    pub follow_links: bool,
    /// Apply `.gitignore`, `.ignore` and the global exclude file.
    pub gitignore: bool,
    /// Additional exclusions in gitignore syntax (`!pattern` re-includes).
    pub exclude: Vec<String>,
    /// Named exclusion groups.
    pub exclude_presets: Vec<ExcludePreset>,
    /// Match globs case-sensitively.
    pub case_sensitive: bool,
}

impl WalkSpec {
    /// Request with the defaults of `find`: all files (`**`), unlimited
    /// depth, hidden entries included, links not followed, no ignore files
    /// and no exclusions.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            globs: vec!["**".to_owned()],
            kind: WalkKind::File,
            max_depth: None,
            hidden: true,
            follow_links: false,
            gitignore: false,
            exclude: Vec::new(),
            exclude_presets: Vec::new(),
            case_sensitive: true,
        }
    }

    /// Request for repository-wide source analysis: ignore files applied and
    /// version control, dependency and cache directories excluded.
    pub fn source_tree(root: impl Into<PathBuf>) -> Self {
        Self {
            gitignore: true,
            exclude_presets: vec![
                ExcludePreset::Vcs,
                ExcludePreset::Deps,
                ExcludePreset::Cache,
            ],
            ..Self::new(root)
        }
    }
}

/// Entry listed by [`FileListService::walk`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WalkEntry {
    /// Absolute path.
    pub path: PathBuf,
    /// Levels below the root (1 for direct children).
    pub depth: u32,
    /// The entry is a directory.
    pub is_dir: bool,
}

/// Iterator over listed entries; an unreadable directory yields an error item
/// and the listing continues.
pub type WalkIter<'a> = Box<dyn Iterator<Item = Result<WalkEntry>> + Send + 'a>;

/// File listing with the exclusion rules of `find`.
///
/// Implemented by the file commands, so every command that walks a tree
/// excludes the same directories.
pub trait FileListService: Send + Sync {
    /// Lists entries under `spec.root`, sorted by path within each
    /// directory.
    ///
    /// # Errors
    ///
    /// Fails when the root cannot be read; an unreadable directory below the
    /// root is reported as an error item of the iterator.
    fn walk<'a>(&'a self, spec: &WalkSpec) -> Result<WalkIter<'a>>;
}

/// State of a path in one area of a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GitChange {
    /// No change.
    Unmodified,
    /// Added.
    Added,
    /// Content changed.
    Modified,
    /// Removed.
    Deleted,
    /// Renamed.
    Renamed,
    /// Copied.
    Copied,
    /// File type changed (for example file to symbolic link).
    TypeChanged,
    /// Not tracked.
    Untracked,
    /// Ignored.
    Ignored,
    /// Unresolved merge conflict.
    Conflicted,
}

/// Status of one path in a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitPathStatus {
    /// Path relative to the repository root, with `/` separators.
    pub path: String,
    /// Change staged in the index.
    pub index: GitChange,
    /// Change in the working tree relative to the index.
    pub worktree: GitChange,
    /// Previous path of a rename or copy.
    pub original_path: Option<String>,
}

/// Change history of one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileHistoryStat {
    /// Path relative to the repository root, with `/` separators.
    pub path: String,
    /// Commits that changed the file in the period.
    pub commits: u32,
    /// Distinct authors of those commits.
    pub authors: u32,
    /// Time of the newest of those commits.
    pub last_change: Option<WallTime>,
}

/// Read-only repository queries for commands outside the git group.
pub trait GitQueryService: Send + Sync {
    /// Root of the working tree containing `path`; `None` outside a
    /// repository.
    fn repo_root(&self, path: &Path) -> Result<Option<PathBuf>>;

    /// Status of the given paths (relative to `root`); an empty list returns
    /// every path that is not unmodified.
    fn status(&self, root: &Path, paths: &[String]) -> Result<Vec<GitPathStatus>>;

    /// Paths changed in the working tree and index relative to `against`
    /// (a revision; `None` is `HEAD`), including untracked files.
    fn changed_files(&self, root: &Path, against: Option<&str>) -> Result<Vec<String>>;

    /// Paths tracked in the index.
    fn tracked_files(&self, root: &Path) -> Result<Vec<String>>;

    /// Returns `true` when the ignore rules of the repository exclude
    /// `path`.
    fn is_ignored(&self, root: &Path, path: &str) -> Result<bool>;

    /// Files with the largest number of commits in the last `since_days`
    /// days, in descending order, no more than `limit` entries.
    fn file_history(
        &self,
        root: &Path,
        since_days: u32,
        limit: u32,
    ) -> Result<Vec<FileHistoryStat>>;
}

/// Line ending convention of a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LineEnding {
    /// `\n`.
    Lf,
    /// `\r\n`; written `crlf`, as in the record enumeration.
    #[serde(rename = "crlf")]
    CrLf,
    /// `\r`.
    Cr,
    /// More than one convention.
    Mixed,
    /// No line ending.
    None,
}

/// Character encoding of a text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct TextEncoding {
    /// Encoding label in WHATWG form, lowercase (`utf-8`, `utf-16le`,
    /// `windows-1252`).
    pub label: String,
    /// The text starts with a byte order mark.
    pub bom: bool,
}

impl TextEncoding {
    /// UTF-8 without a byte order mark.
    pub fn utf8() -> Self {
        Self {
            label: "utf-8".to_owned(),
            bom: false,
        }
    }
}

/// Result of [`TextCodecService::detect`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextDetection {
    /// The content is binary (not text in any supported encoding).
    pub binary: bool,
    /// Detected encoding; `None` for binary content.
    pub encoding: Option<TextEncoding>,
    /// Line ending convention.
    pub line_ending: LineEnding,
}

/// Text decoding and encoding with the rules of the text commands.
///
/// Decoding followed by encoding with the same [`TextEncoding`] returns the
/// original bytes; content for which that cannot hold fails with
/// `E_ENCODING` instead of being changed silently.
pub trait TextCodecService: Send + Sync {
    /// Detects whether the content is text, its encoding and line endings.
    fn detect(&self, bytes: &[u8]) -> TextDetection;

    /// Decodes text, without the byte order mark.
    fn decode(&self, bytes: &[u8], encoding: &TextEncoding) -> Result<String>;

    /// Encodes text, writing a byte order mark when `encoding.bom` is set.
    fn encode(&self, text: &str, encoding: &TextEncoding) -> Result<Vec<u8>>;
}

/// Process that holds a file open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileHolder {
    /// Process identifier.
    pub pid: u32,
    /// Process or application name.
    pub name: String,
    /// Kind reported by the system (for example `service` or `application`).
    pub kind: Option<String>,
}

/// Processes holding files open, for `E_IN_USE` and `open_files`.
pub trait FileHolderService: Send + Sync {
    /// Processes that hold `path` open; an empty list when none is known.
    fn holders(&self, path: &Path) -> Result<Vec<FileHolder>>;
}

/// Severity of a review rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RuleSeverity {
    /// The finding blocks the review.
    Error,
    /// The finding is reported.
    Warning,
    /// The finding is informational.
    Info,
}

/// Content rule applied by `git.review`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReviewRule {
    /// Stable identifier (`aws_access_key`).
    pub id: String,
    /// Regular expression matched against added lines.
    pub pattern: String,
    /// Severity of a match.
    pub severity: RuleSeverity,
    /// Description in English.
    pub summary: String,
}

/// Additional rules for `git.review`, supplied by the engine or extensions.
pub trait ReviewRuleProvider: Send + Sync {
    /// Rules added to the built-in rules of `git.review`.
    fn rules(&self) -> Vec<ReviewRule>;
}

/// Service of the registry, for diagnostics and fallback notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServiceKind {
    /// [`DocService`].
    Doc,
    /// [`CodeService`].
    Code,
    /// [`ProcessHost`].
    ProcessHost,
    /// [`FileListService`].
    FileList,
    /// [`GitQueryService`].
    GitQuery,
    /// [`TextCodecService`].
    TextCodec,
    /// [`FileHolderService`].
    FileHolders,
    /// [`ReviewRuleProvider`].
    ReviewRules,
}

impl ServiceKind {
    /// Every service kind, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Doc,
        Self::Code,
        Self::ProcessHost,
        Self::FileList,
        Self::GitQuery,
        Self::TextCodec,
        Self::FileHolders,
        Self::ReviewRules,
    ];

    /// Stable name (`doc`, `code`, `process_host`, …).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Doc => "doc",
            Self::Code => "code",
            Self::ProcessHost => "process_host",
            Self::FileList => "file_list",
            Self::GitQuery => "git_query",
            Self::TextCodec => "text_codec",
            Self::FileHolders => "file_holders",
            Self::ReviewRules => "review_rules",
        }
    }
}

impl fmt::Display for ServiceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Registry of the services available to commands.
///
/// The engine builds it once and the execution context exposes it; each
/// accessor returns `None` when the providing group is not loaded. Cloning is
/// cheap (shared references).
///
/// ```
/// use nurt_contracts::services::{ServiceKind, Services};
///
/// let services = Services::new();
/// assert!(services.doc().is_none());
/// assert!(services.available().is_empty());
/// assert!(!services.has(ServiceKind::Code));
/// ```
#[derive(Clone, Default)]
pub struct Services {
    doc: Option<Arc<dyn DocService>>,
    code: Option<Arc<dyn CodeService>>,
    process_host: Option<Arc<dyn ProcessHost>>,
    file_list: Option<Arc<dyn FileListService>>,
    git_query: Option<Arc<dyn GitQueryService>>,
    text_codec: Option<Arc<dyn TextCodecService>>,
    file_holders: Option<Arc<dyn FileHolderService>>,
    review_rules: Option<Arc<dyn ReviewRuleProvider>>,
}

impl Services {
    /// Registry without services.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the document service.
    #[must_use]
    pub fn with_doc(mut self, service: Arc<dyn DocService>) -> Self {
        self.doc = Some(service);
        self
    }

    /// Registers the code service.
    #[must_use]
    pub fn with_code(mut self, service: Arc<dyn CodeService>) -> Self {
        self.code = Some(service);
        self
    }

    /// Registers the process host.
    #[must_use]
    pub fn with_process_host(mut self, service: Arc<dyn ProcessHost>) -> Self {
        self.process_host = Some(service);
        self
    }

    /// Registers the file listing service.
    #[must_use]
    pub fn with_file_list(mut self, service: Arc<dyn FileListService>) -> Self {
        self.file_list = Some(service);
        self
    }

    /// Registers the repository query service.
    #[must_use]
    pub fn with_git_query(mut self, service: Arc<dyn GitQueryService>) -> Self {
        self.git_query = Some(service);
        self
    }

    /// Registers the text codec.
    #[must_use]
    pub fn with_text_codec(mut self, service: Arc<dyn TextCodecService>) -> Self {
        self.text_codec = Some(service);
        self
    }

    /// Registers the file holder service.
    #[must_use]
    pub fn with_file_holders(mut self, service: Arc<dyn FileHolderService>) -> Self {
        self.file_holders = Some(service);
        self
    }

    /// Registers the review rule provider.
    #[must_use]
    pub fn with_review_rules(mut self, service: Arc<dyn ReviewRuleProvider>) -> Self {
        self.review_rules = Some(service);
        self
    }

    /// Document service.
    pub fn doc(&self) -> Option<&dyn DocService> {
        self.doc.as_deref()
    }

    /// Code service.
    pub fn code(&self) -> Option<&dyn CodeService> {
        self.code.as_deref()
    }

    /// Process host.
    pub fn process_host(&self) -> Option<&dyn ProcessHost> {
        self.process_host.as_deref()
    }

    /// Process host as a shared handle, for objects that outlive the borrow
    /// of the services.
    pub fn process_host_shared(&self) -> Option<Arc<dyn ProcessHost>> {
        self.process_host.clone()
    }

    /// File listing service.
    pub fn file_list(&self) -> Option<&dyn FileListService> {
        self.file_list.as_deref()
    }

    /// Repository query service.
    pub fn git_query(&self) -> Option<&dyn GitQueryService> {
        self.git_query.as_deref()
    }

    /// Text codec.
    pub fn text_codec(&self) -> Option<&dyn TextCodecService> {
        self.text_codec.as_deref()
    }

    /// File holder service.
    pub fn file_holders(&self) -> Option<&dyn FileHolderService> {
        self.file_holders.as_deref()
    }

    /// Review rule provider.
    pub fn review_rules(&self) -> Option<&dyn ReviewRuleProvider> {
        self.review_rules.as_deref()
    }

    /// Returns `true` when the service is registered.
    pub fn has(&self, kind: ServiceKind) -> bool {
        match kind {
            ServiceKind::Doc => self.doc.is_some(),
            ServiceKind::Code => self.code.is_some(),
            ServiceKind::ProcessHost => self.process_host.is_some(),
            ServiceKind::FileList => self.file_list.is_some(),
            ServiceKind::GitQuery => self.git_query.is_some(),
            ServiceKind::TextCodec => self.text_codec.is_some(),
            ServiceKind::FileHolders => self.file_holders.is_some(),
            ServiceKind::ReviewRules => self.review_rules.is_some(),
        }
    }

    /// Registered services, in declaration order.
    pub fn available(&self) -> Vec<ServiceKind> {
        ServiceKind::ALL
            .into_iter()
            .filter(|kind| self.has(*kind))
            .collect()
    }
}

impl fmt::Debug for Services {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Services")
            .field("available", &self.available())
            .finish()
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
