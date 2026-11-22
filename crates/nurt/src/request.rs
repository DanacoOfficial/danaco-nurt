// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Request envelope `nurt-request/1` and its options.

use std::time::Duration;

use serde_json::{Map, Value};

/// Version of the request envelope written by this crate.
pub const ENVELOPE_VERSION: &str = "1.0";

/// Notation variant of program text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Variant {
    /// Word notation.
    #[default]
    A,
    /// Call notation.
    B,
    /// Named notation.
    C,
}

impl Variant {
    /// Name of the variant in JSON documents: `a`, `b` or `c`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
            Self::C => "c",
        }
    }
}

/// Format of the rendered result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// Plain text.
    Text,
    /// JSON only; no rendered text.
    Json,
    /// Markdown.
    Markdown,
    /// Compact text with the fewest characters.
    Compact,
}

impl Format {
    /// Name of the format in JSON documents.
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

/// Level of detail of the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Verbosity {
    /// Status and error only.
    Quiet,
    /// The default level.
    Normal,
    /// Every field of the report.
    Detailed,
}

impl Verbosity {
    /// Name of the level in JSON documents.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Normal => "normal",
            Self::Detailed => "detailed",
        }
    }
}

/// Form of differences in the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiffMode {
    /// No differences.
    None,
    /// Counts of changed lines per file.
    Stat,
    /// Unified differences.
    Unified,
}

impl DiffMode {
    /// Name of the form in JSON documents.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Stat => "stat",
            Self::Unified => "unified",
        }
    }
}

/// Language of messages and help.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Language {
    /// English.
    En,
    /// Polish.
    Pl,
}

impl Language {
    /// Language tag in JSON documents.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Pl => "pl",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Input {
    Program(String),
    Ast(Value),
    Cursor(String),
    Accept(String),
}

/// Request envelope (`nurt-request/1`): one input and the call options.
///
/// The input is program text, a syntax tree `nurt-ast/1`, the cursor of a
/// further page or the identifier of a correction candidate. Options that
/// are not set take their values from the configuration of the server.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use nurt::{Format, Request};
///
/// let request = Request::program(r#"find "/home/user/src" glob: "*.go""#)
///     .base("/home/user")
///     .limit(20)
///     .format(Format::Json)
///     .timeout(Duration::from_secs(30));
/// let json = request.to_value();
/// assert_eq!(json["options"]["limit"], 20);
/// assert_eq!(json["options"]["timeout"], "30s");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    input: Input,
    variant: Option<Variant>,
    options: Map<String, Value>,
}

impl Request {
    fn with_input(input: Input) -> Self {
        Self {
            input,
            variant: None,
            options: Map::new(),
        }
    }

    /// Request that runs program text in any notation variant.
    #[must_use]
    pub fn program(text: impl Into<String>) -> Self {
        Self::with_input(Input::Program(text.into()))
    }

    /// Request that runs a syntax tree `nurt-ast/1`.
    #[must_use]
    pub fn ast(document: Value) -> Self {
        Self::with_input(Input::Ast(document))
    }

    /// Request that reads a further page of an earlier result.
    #[must_use]
    pub fn cursor(cursor: impl Into<String>) -> Self {
        Self::with_input(Input::Cursor(cursor.into()))
    }

    /// Request that accepts a candidate of an earlier `E_AMBIGUOUS` error,
    /// for example `c1`.
    #[must_use]
    pub fn accept(candidate: impl Into<String>) -> Self {
        Self::with_input(Input::Accept(candidate.into()))
    }

    /// Notation variant of the program text. The server rejects a variant
    /// given with another input as the language error `E_ARG`.
    #[must_use]
    pub fn variant(mut self, variant: Variant) -> Self {
        self.variant = Some(variant);
        self
    }

    fn option(mut self, name: &str, value: impl Into<Value>) -> Self {
        self.options.insert(name.to_owned(), value.into());
        self
    }

    /// Only compute targets and effect; change nothing.
    #[must_use]
    pub fn preview(self, preview: bool) -> Self {
        self.option("preview", preview)
    }

    /// Absolute directory against which relative paths are resolved.
    #[must_use]
    pub fn base(self, base: impl Into<String>) -> Self {
        self.option("base", base.into())
    }

    /// Number of records or changes per page, from 1 to 5000.
    #[must_use]
    pub fn limit(self, limit: u32) -> Self {
        self.option("limit", limit)
    }

    /// Fields of the result to keep.
    #[must_use]
    pub fn fields<I, S>(self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let fields: Vec<Value> = fields
            .into_iter()
            .map(|field| Value::String(field.into()))
            .collect();
        self.option("fields", fields)
    }

    /// Format of the rendered result.
    #[must_use]
    pub fn format(self, format: Format) -> Self {
        self.option("format", format.as_str())
    }

    /// Level of detail of the report.
    #[must_use]
    pub fn verbosity(self, verbosity: Verbosity) -> Self {
        self.option("verbosity", verbosity.as_str())
    }

    /// Form of differences.
    #[must_use]
    pub fn diff(self, diff: DiffMode) -> Self {
        self.option("diff", diff.as_str())
    }

    /// Context lines of differences.
    #[must_use]
    pub fn diff_context(self, lines: u32) -> Self {
        self.option("diff_context", lines)
    }

    /// Largest number of characters of a single text field; `0` means no
    /// limit.
    #[must_use]
    pub fn max_chars(self, max_chars: u64) -> Self {
        self.option("max_chars", max_chars)
    }

    /// Absolute path of a file that receives the full result.
    #[must_use]
    pub fn save_to(self, path: impl Into<String>) -> Self {
        self.option("save_to", path.into())
    }

    /// Time limit of the whole program, sent in whole seconds or, when the
    /// duration has a fractional second, in milliseconds.
    #[must_use]
    pub fn timeout(self, timeout: Duration) -> Self {
        self.option("timeout", duration_text(timeout))
    }

    /// Session identifier recorded in the journal.
    #[must_use]
    pub fn session(self, session: impl Into<String>) -> Self {
        self.option("session", session.into())
    }

    /// Language of messages and help.
    #[must_use]
    pub fn language(self, language: Language) -> Self {
        self.option("language", language.as_str())
    }

    /// Script mode: several pipelines, each its own operation.
    #[must_use]
    pub fn script(self, script: bool) -> Self {
        self.option("script", script)
    }

    /// The envelope as a JSON value.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut envelope = Map::new();
        envelope.insert("nurt".to_owned(), Value::from(ENVELOPE_VERSION));
        let (field, value) = match &self.input {
            Input::Program(text) => ("program", Value::from(text.as_str())),
            Input::Ast(document) => ("ast", document.clone()),
            Input::Cursor(cursor) => ("cursor", Value::from(cursor.as_str())),
            Input::Accept(candidate) => ("accept", Value::from(candidate.as_str())),
        };
        envelope.insert(field.to_owned(), value);
        if let Some(variant) = self.variant {
            envelope.insert("variant".to_owned(), Value::from(variant.as_str()));
        }
        if !self.options.is_empty() {
            envelope.insert("options".to_owned(), Value::Object(self.options.clone()));
        }
        Value::Object(envelope)
    }

    /// The envelope as JSON text.
    #[must_use]
    pub fn to_json(&self) -> String {
        self.to_value().to_string()
    }
}

fn duration_text(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn minimal_program_has_only_version_and_text() {
        let request = Request::program("find \"/srv/app\"");
        assert_eq!(
            request.to_value(),
            json!({"nurt": "1.0", "program": "find \"/srv/app\""})
        );
    }

    #[test]
    fn every_option_is_written_with_its_wire_name() {
        let request = Request::program("find \"/srv/app\"")
            .variant(Variant::B)
            .preview(true)
            .base("/srv/app")
            .limit(10)
            .fields(["path", "size"])
            .format(Format::Markdown)
            .verbosity(Verbosity::Detailed)
            .diff(DiffMode::Stat)
            .diff_context(5)
            .max_chars(400)
            .save_to("/home/user/out.txt")
            .timeout(Duration::from_millis(1500))
            .session("s-1")
            .language(Language::Pl)
            .script(true);
        assert_eq!(
            request.to_value(),
            json!({
                "nurt": "1.0",
                "program": "find \"/srv/app\"",
                "variant": "b",
                "options": {
                    "preview": true,
                    "base": "/srv/app",
                    "limit": 10,
                    "fields": ["path", "size"],
                    "format": "markdown",
                    "verbosity": "detailed",
                    "diff": "stat",
                    "diff_context": 5,
                    "max_chars": 400,
                    "save_to": "/home/user/out.txt",
                    "timeout": "1500ms",
                    "session": "s-1",
                    "language": "pl",
                    "script": true
                }
            })
        );
    }

    #[test]
    fn later_option_replaces_earlier_value() {
        let request = Request::program("x").limit(5).limit(7);
        assert_eq!(request.to_value()["options"]["limit"], 7);
    }

    #[test]
    fn each_input_uses_its_own_field() {
        assert_eq!(Request::cursor("c_1").to_value()["cursor"], "c_1");
        assert_eq!(Request::accept("c2").to_value()["accept"], "c2");
        let tree = json!({"nurt_ast": "1.0", "pipeline": []});
        assert_eq!(Request::ast(tree.clone()).to_value()["ast"], tree);
    }

    #[test]
    fn whole_seconds_are_written_in_seconds() {
        assert_eq!(duration_text(Duration::from_secs(120)), "120s");
        assert_eq!(duration_text(Duration::from_millis(250)), "250ms");
        assert_eq!(duration_text(Duration::ZERO), "0s");
    }

    #[test]
    fn enumerations_use_lowercase_wire_names() {
        assert_eq!(Variant::default().as_str(), "a");
        assert_eq!(Variant::C.as_str(), "c");
        assert_eq!(Format::Text.as_str(), "text");
        assert_eq!(Format::Json.as_str(), "json");
        assert_eq!(Format::Compact.as_str(), "compact");
        assert_eq!(Verbosity::Quiet.as_str(), "quiet");
        assert_eq!(Verbosity::Normal.as_str(), "normal");
        assert_eq!(DiffMode::None.as_str(), "none");
        assert_eq!(DiffMode::Unified.as_str(), "unified");
        assert_eq!(Language::En.as_str(), "en");
    }

    #[test]
    fn json_text_matches_the_value() {
        let request = Request::program("x").limit(3);
        let parsed: Value = serde_json::from_str(&request.to_json()).unwrap();
        assert_eq!(parsed, request.to_value());
    }
}
