// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Migration report in text and JSON form.
//!
//! # JSON form
//!
//! The JSON form has the format name [`REPORT_FORMAT`] and the members below,
//! in this order. Optional members are omitted when absent.
//!
//! | Member | Type | Content |
//! |---|---|---|
//! | `format` | text | `nurt-migrate-report/1` |
//! | `language` | text | `bash` or `powershell` |
//! | `script` | text | script path written into the fallback program |
//! | `summary` | object | `statements`, `translated`, `assumption`, `impossible`, `error`: counts of items |
//! | `items` | array | one object per statement, in source order |
//! | `fallback` | text, optional | program that runs the whole script unchanged; present when a statement is not translated |
//! | `base` | text, optional | absolute directory proposed as the invocation parameter `base` |
//!
//! An item has the members `line` and `column` (1-based; the column counts
//! Unicode scalar values), `source` (statement text), `status`
//! (`translated`, `assumption`, `impossible` or `error`), `rules` (rule
//! identifiers such as `B15`), `assumptions` (texts), the optional `code`
//! (error code such as `E_UNCORRECTABLE`), `reason` (see [`Reason`]) and
//! `problem`, the array `fix` (programs) and the optional `output` (the
//! translated program, also present for a translation that fails to bind).
//!
//! # Examples
//!
//! ```
//! use nurt_migrate::{MigrationReport, REPORT_FORMAT, ScriptLanguage};
//!
//! let report = MigrationReport::new(ScriptLanguage::Bash, "/srv/deploy.sh");
//! let json: serde_json::Value = serde_json::from_str(&report.to_json())?;
//! assert_eq!(json["format"], REPORT_FORMAT);
//! assert_eq!(json["summary"]["statements"], 0);
//! # Ok::<(), serde_json::Error>(())
//! ```

use std::fmt::Write as _;

use nurt_contracts::error::ErrorCode;
use serde_json::{Map, Value, json};

use crate::language::ScriptLanguage;

/// Name and version of the JSON form of the report.
pub const REPORT_FORMAT: &str = "nurt-migrate-report/1";

/// Indentation of detail lines in the text form.
const DETAIL_INDENT: &str = "  ";

/// Outcome of one statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Status {
    /// Translated with certain rules only.
    Translated,
    /// Translated with at least one probable rule or stated assumption.
    Assumption,
    /// Not translated: the statement has no equivalent.
    Impossible,
    /// Not translated: the script has a syntax error, or the translated
    /// program does not parse or bind.
    Error,
}

impl Status {
    /// Returns the name used in reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Translated => "translated",
            Self::Assumption => "assumption",
            Self::Impossible => "impossible",
            Self::Error => "error",
        }
    }

    /// Returns `true` when the statement was translated.
    #[must_use]
    pub const fn is_translated(self) -> bool {
        matches!(self, Self::Translated | Self::Assumption)
    }
}

/// Reason why a statement was not translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Reason {
    /// A loop (`for`, `while`, `until`, `foreach`, `do`).
    Loop,
    /// A conditional statement (`if`, `case`, `switch`).
    Condition,
    /// A definition of a function, class or enumeration.
    Function,
    /// A command group, subshell or block statement (`( ... )`, `{ ... }`, `try`).
    Block,
    /// Evaluation of text as code (`eval`, `Invoke-Expression`).
    Eval,
    /// Loading of another script into the current shell (`source`, `.`).
    Source,
    /// Script parameters (`param( ... )`).
    Parameters,
    /// Flow control (`exit`, `return`, `break`, `throw`).
    Control,
    /// An alternative branch joined with `||`.
    OrList,
    /// A here-document other than the content of a written file.
    HereDocument,
    /// Dependence on the current directory.
    CurrentDirectory,
    /// A command or parameter without a rule in the habit tables.
    Rule,
    /// A syntax error in the script.
    Syntax,
    /// A translated program that does not parse or bind.
    Translation,
}

impl Reason {
    /// Returns the name used in reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loop => "loop",
            Self::Condition => "condition",
            Self::Function => "function",
            Self::Block => "block",
            Self::Eval => "eval",
            Self::Source => "source",
            Self::Parameters => "parameters",
            Self::Control => "control",
            Self::OrList => "or_list",
            Self::HereDocument => "here_document",
            Self::CurrentDirectory => "current_directory",
            Self::Rule => "rule",
            Self::Syntax => "syntax",
            Self::Translation => "translation",
        }
    }
}

/// Report entry for one statement.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Item {
    /// Line of the statement start, from 1.
    pub line: u32,
    /// Column of the statement start, from 1, in Unicode scalar values.
    pub column: u32,
    /// Source text of the statement.
    pub source: String,
    /// Outcome.
    pub status: Status,
    /// Identifiers of the rules applied, without repetitions.
    pub rules: Vec<String>,
    /// Assumptions of the translation.
    pub assumptions: Vec<String>,
    /// Error code of a statement that was not translated.
    pub code: Option<ErrorCode>,
    /// Reason of a statement that was not translated.
    pub reason: Option<Reason>,
    /// Explanation of a statement that was not translated.
    pub problem: Option<String>,
    /// Programs that replace the statement, best first.
    pub fix: Vec<String>,
    /// Translated program in canonical form.
    pub output: Option<String>,
}

impl Item {
    pub(crate) fn new(line: u32, column: u32, source: &str, status: Status) -> Self {
        Self {
            line,
            column,
            source: source.to_owned(),
            status,
            rules: Vec::new(),
            assumptions: Vec::new(),
            code: None,
            reason: None,
            problem: None,
            fix: Vec::new(),
            output: None,
        }
    }

    fn to_json(&self) -> Value {
        let mut item = Map::new();
        item.insert("line".to_owned(), json!(self.line));
        item.insert("column".to_owned(), json!(self.column));
        item.insert("source".to_owned(), json!(self.source));
        item.insert("status".to_owned(), json!(self.status.as_str()));
        item.insert("rules".to_owned(), json!(self.rules));
        item.insert("assumptions".to_owned(), json!(self.assumptions));
        if let Some(code) = self.code {
            item.insert("code".to_owned(), json!(code.as_str()));
        }
        if let Some(reason) = self.reason {
            item.insert("reason".to_owned(), json!(reason.as_str()));
        }
        if let Some(problem) = &self.problem {
            item.insert("problem".to_owned(), json!(problem));
        }
        item.insert("fix".to_owned(), json!(self.fix));
        if let Some(output) = &self.output {
            item.insert("output".to_owned(), json!(output));
        }
        Value::Object(item)
    }

    fn write_text(&self, text: &mut String) {
        let _ = write!(
            text,
            "{}:{} {}",
            self.line,
            self.column,
            self.status.as_str()
        );
        if let Some(code) = self.code {
            let _ = write!(text, ": {}", code.as_str());
            if let Some(reason) = self.reason {
                let _ = write!(text, " ({})", reason.as_str());
            }
        } else if !self.rules.is_empty() {
            let _ = write!(text, ": {}", self.rules.join(", "));
        }
        text.push('\n');
        write_detail(text, "source", &self.source);
        for assumption in &self.assumptions {
            write_detail(text, "assumption", assumption);
        }
        if let Some(problem) = &self.problem {
            write_detail(text, "problem", problem);
        }
        for fix in &self.fix {
            write_detail(text, "fix", fix);
        }
        if let Some(output) = &self.output {
            write_detail(text, "output", output);
        }
    }
}

/// Counts of report items by status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Summary {
    /// All statements.
    pub statements: usize,
    /// Statements translated with certain rules only.
    pub translated: usize,
    /// Statements translated with an assumption.
    pub assumption: usize,
    /// Statements without an equivalent.
    pub impossible: usize,
    /// Statements with a syntax error or a translation that does not bind.
    pub error: usize,
}

/// Report of a migration: one item per statement and proposals for the
/// statements that were not translated.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MigrationReport {
    /// Language of the script.
    pub language: ScriptLanguage,
    /// Script path written into the fallback program.
    pub script: String,
    /// Items in source order.
    pub items: Vec<Item>,
    /// Program that runs the whole script unchanged, present when a statement
    /// was not translated.
    pub fallback: Option<String>,
    /// Absolute directory proposed as the invocation parameter `base`, taken
    /// from the only absolute target of the untranslated directory changes.
    pub base: Option<String>,
}

impl MigrationReport {
    /// Creates an empty report.
    #[must_use]
    pub fn new(language: ScriptLanguage, script: &str) -> Self {
        Self {
            language,
            script: script.to_owned(),
            items: Vec::new(),
            fallback: None,
            base: None,
        }
    }

    /// Counts the items by status.
    #[must_use]
    pub fn summary(&self) -> Summary {
        let mut summary = Summary {
            statements: self.items.len(),
            ..Summary::default()
        };
        for item in &self.items {
            match item.status {
                Status::Translated => summary.translated += 1,
                Status::Assumption => summary.assumption += 1,
                Status::Impossible => summary.impossible += 1,
                Status::Error => summary.error += 1,
            }
        }
        summary
    }

    /// Returns the JSON form, indented with two spaces and ending with a line
    /// feed.
    #[must_use]
    pub fn to_json(&self) -> String {
        let summary = self.summary();
        let mut report = Map::new();
        report.insert("format".to_owned(), json!(REPORT_FORMAT));
        report.insert("language".to_owned(), json!(self.language.as_str()));
        report.insert("script".to_owned(), json!(self.script));
        report.insert(
            "summary".to_owned(),
            json!({
                "statements": summary.statements,
                "translated": summary.translated,
                "assumption": summary.assumption,
                "impossible": summary.impossible,
                "error": summary.error,
            }),
        );
        report.insert(
            "items".to_owned(),
            Value::Array(self.items.iter().map(Item::to_json).collect()),
        );
        if let Some(fallback) = &self.fallback {
            report.insert("fallback".to_owned(), json!(fallback));
        }
        if let Some(base) = &self.base {
            report.insert("base".to_owned(), json!(base));
        }
        format!("{:#}\n", Value::Object(report))
    }

    /// Returns the text form: a heading, the counts, one entry per statement
    /// and the proposals.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_migrate::{MigrationReport, ScriptLanguage};
    ///
    /// let report = MigrationReport::new(ScriptLanguage::PowerShell, "/srv/setup.ps1");
    /// assert_eq!(
    ///     report.to_text(),
    ///     "script /srv/setup.ps1 (powershell)\n\
    ///      statements 0: translated 0, assumption 0, impossible 0, error 0\n",
    /// );
    /// ```
    #[must_use]
    pub fn to_text(&self) -> String {
        let summary = self.summary();
        let mut text = format!("script {} ({})\n", self.script, self.language);
        let _ = writeln!(
            text,
            "statements {}: translated {}, assumption {}, impossible {}, error {}",
            summary.statements,
            summary.translated,
            summary.assumption,
            summary.impossible,
            summary.error,
        );
        for item in &self.items {
            item.write_text(&mut text);
        }
        if let Some(fallback) = &self.fallback {
            let _ = writeln!(text, "fallback: {fallback}");
        }
        if let Some(base) = &self.base {
            let _ = writeln!(text, "base: {}", json!(base));
        }
        text
    }
}

/// Writes a labelled detail line; continuation lines of a multi-line value
/// are indented under the value.
fn write_detail(text: &mut String, label: &str, value: &str) {
    let indent = " ".repeat(DETAIL_INDENT.len() + label.len() + 2);
    for (index, line) in value.lines().enumerate() {
        if index == 0 {
            let _ = writeln!(text, "{DETAIL_INDENT}{label}: {line}");
        } else {
            let _ = writeln!(text, "{indent}{line}");
        }
    }
}
