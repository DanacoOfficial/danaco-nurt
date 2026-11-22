// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Diagnostics of catalog reading and validation.

use std::fmt;

/// Severity of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    /// The catalog violates the format.
    Error,
    /// The catalog is valid; the finding deserves attention.
    Warning,
}

impl Severity {
    /// Name used in reports.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

/// Kind of a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagnosticCode {
    /// The catalog cannot be read as a whole.
    CatalogInvalid,
    /// The catalog has a major format version the reader does not implement.
    CatalogFormat,
    /// An object, member or file of the catalog violates the format and is
    /// skipped.
    EntryInvalid,
    /// An object has a member that the format does not define.
    UnknownField,
    /// A guide has no `version` in its front matter.
    GuideVersionMissing,
    /// A guide disagrees with the entries that name it.
    GuideMismatch,
    /// A text value contained control characters, which were removed.
    ControlCharacter,
}

impl DiagnosticCode {
    /// Stable name of the code.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CatalogInvalid => "catalog_invalid",
            Self::CatalogFormat => "catalog_format",
            Self::EntryInvalid => "catalog_entry_invalid",
            Self::UnknownField => "catalog_unknown_field",
            Self::GuideVersionMissing => "guide_version_missing",
            Self::GuideMismatch => "guide_mismatch",
            Self::ControlCharacter => "control_character",
        }
    }

    /// Severity of the code when a catalog is validated; `newer_minor` is
    /// set when the catalog declares a later revision than the reader
    /// implements.
    #[must_use]
    pub fn strict_severity(self, newer_minor: bool) -> Severity {
        match self {
            Self::CatalogInvalid | Self::CatalogFormat | Self::EntryInvalid => Severity::Error,
            Self::UnknownField if !newer_minor => Severity::Error,
            Self::UnknownField
            | Self::GuideVersionMissing
            | Self::GuideMismatch
            | Self::ControlCharacter => Severity::Warning,
        }
    }
}

/// One finding about a catalog, located by file, line and member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity.
    pub severity: Severity,
    /// Kind.
    pub code: DiagnosticCode,
    /// File relative to the catalog root, `/` separated.
    pub file: String,
    /// Line in the file, counted from 1, when known.
    pub line: Option<u64>,
    /// Path of the member, for example `programs[3].aliases[1].kind`.
    pub field: Option<String>,
    /// Name or identifier of the object concerned.
    pub name: Option<String>,
    /// What is wrong and how to correct it.
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
        }
        if let Some(field) = &self.field {
            write!(f, " {field}")?;
        }
        write!(
            f,
            ": {} [{}]: {}",
            self.severity.as_str(),
            self.code.as_str(),
            self.message
        )
    }
}

/// Collector of diagnostics with the severity rules of the reading mode.
#[derive(Debug, Clone, Default)]
pub(crate) struct Diagnostics {
    strict: bool,
    newer_minor: bool,
    items: Vec<Diagnostic>,
}

/// Location of a finding.
#[derive(Debug, Clone, Default)]
pub(crate) struct At {
    pub(crate) file: String,
    pub(crate) line: Option<u64>,
    pub(crate) field: Option<String>,
    pub(crate) name: Option<String>,
}

impl At {
    /// Location in `file` at `line`.
    pub(crate) fn new(file: &str, line: Option<u64>) -> Self {
        Self {
            file: file.to_owned(),
            line,
            field: None,
            name: None,
        }
    }

    /// Sets the member.
    pub(crate) fn field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    /// Sets the object name.
    pub(crate) fn name(mut self, name: Option<&str>) -> Self {
        self.name = name.map(str::to_owned);
        self
    }
}

impl Diagnostics {
    /// Collector for reading (`strict == false`) or validation.
    pub(crate) fn new(strict: bool) -> Self {
        Self {
            strict,
            newer_minor: false,
            items: Vec::new(),
        }
    }

    /// Records that the catalog declares a later revision than implemented.
    pub(crate) fn set_newer_minor(&mut self) {
        self.newer_minor = true;
    }

    /// Returns `true` when findings of `code` are collected: a reader does
    /// not report what it repairs or ignores without loss.
    pub(crate) fn wants(&self, code: DiagnosticCode) -> bool {
        self.strict
            || !matches!(
                code,
                DiagnosticCode::GuideMismatch | DiagnosticCode::ControlCharacter
            )
    }

    /// Adds a finding.
    pub(crate) fn push(&mut self, code: DiagnosticCode, at: At, message: impl Into<String>) {
        if !self.wants(code) {
            return;
        }
        let severity = if self.strict {
            code.strict_severity(self.newer_minor)
        } else {
            Severity::Warning
        };
        self.items.push(Diagnostic {
            severity,
            code,
            file: at.file,
            line: at.line,
            field: at.field,
            name: at.name,
            message: message.into(),
        });
    }

    /// The findings in the order they were made.
    pub(crate) fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }
}
