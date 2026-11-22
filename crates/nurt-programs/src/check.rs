// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Validation of program catalogs in the format `nurt-program-catalog/1`.
//!
//! [`validate_catalog`] reads a catalog in the strict mode of the reader and
//! reads every guide in full, so that every violation of the format is
//! reported with its file, line and member. The command line program exposes
//! it as `nurt program check <dir>`.

use std::fmt;
use std::path::Path;

use crate::catalog::{
    CatalogErrorKind, Diagnostic, DiagnosticCode, Limits, ReadOptions, Severity, read_catalog,
};

/// Counts of the objects of a valid catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CatalogSummary {
    /// Declared identifier.
    pub id: Option<String>,
    /// Programs that satisfy the format.
    pub programs: usize,
    /// Categories and subcategories that satisfy the format.
    pub categories: usize,
    /// Standalone guides that satisfy the format.
    pub guides: usize,
    /// Guide files read, including translations.
    pub guide_files: usize,
    /// Included parts read.
    pub parts: usize,
    /// Localized overlays read.
    pub overlays: usize,
}

/// Result of the validation of a catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckReport {
    /// Counts, absent when the catalog cannot be read as a whole.
    pub summary: Option<CatalogSummary>,
    /// Findings in the order they were made.
    pub diagnostics: Vec<Diagnostic>,
}

impl CheckReport {
    /// Returns `true` when no finding is an error.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors() == 0
    }

    /// Number of findings with the severity [`Severity::Error`].
    #[must_use]
    pub fn errors(&self) -> usize {
        self.count(Severity::Error)
    }

    /// Number of findings with the severity [`Severity::Warning`].
    #[must_use]
    pub fn warnings(&self) -> usize {
        self.count(Severity::Warning)
    }

    fn count(&self, severity: Severity) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == severity)
            .count()
    }
}

impl fmt::Display for CheckReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for diagnostic in &self.diagnostics {
            writeln!(f, "{diagnostic}")?;
        }
        write!(f, "{} errors, {} warnings", self.errors(), self.warnings())
    }
}

/// Validates the catalog in the directory `path` with the limits of the
/// format.
///
/// # Examples
///
/// ```
/// let dir = tempfile::tempdir().unwrap();
/// std::fs::write(
///     dir.path().join("catalog.json"),
///     r#"{"format": "nurt-program-catalog/1", "categories": [], "programs": []}"#,
/// )
/// .unwrap();
/// let report = nurt_programs::check::validate_catalog(dir.path());
/// assert!(report.is_valid());
/// ```
#[must_use]
pub fn validate_catalog(path: &Path) -> CheckReport {
    validate_catalog_with(path, &Limits::default())
}

/// Validates the catalog in the directory `path` with `limits`.
#[must_use]
pub fn validate_catalog_with(path: &Path, limits: &Limits) -> CheckReport {
    let options = ReadOptions {
        limits: *limits,
        strict: true,
    };
    let loaded = match read_catalog(path, &options) {
        Ok(loaded) => loaded,
        Err(error) => {
            let code = if error.kind == CatalogErrorKind::Format {
                DiagnosticCode::CatalogFormat
            } else {
                DiagnosticCode::CatalogInvalid
            };
            return CheckReport {
                summary: None,
                diagnostics: vec![Diagnostic {
                    severity: Severity::Error,
                    code,
                    file: error.file,
                    line: error.line,
                    field: None,
                    name: None,
                    message: error.message,
                }],
            };
        }
    };
    let catalog = loaded.catalog;
    let mut diagnostics = loaded.diagnostics;
    for path in catalog.guide_headers.keys() {
        if let Err(message) = catalog.read_guide(path, limits) {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: DiagnosticCode::EntryInvalid,
                file: path.clone(),
                line: None,
                field: None,
                name: None,
                message,
            });
        }
    }
    CheckReport {
        summary: Some(CatalogSummary {
            id: catalog.id.clone(),
            programs: catalog.programs.len(),
            categories: catalog.categories.len(),
            guides: catalog.guides.len(),
            guide_files: catalog.guide_headers.len(),
            parts: catalog.parts.len(),
            overlays: catalog.overlays.len(),
        }),
        diagnostics,
    }
}
