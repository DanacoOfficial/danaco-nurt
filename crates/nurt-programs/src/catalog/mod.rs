// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reader of the format `nurt-program-catalog/1`: index, guides and localized overlays.
//!
//! [`read_catalog`] loads a catalog directory into a [`Catalog`]. Objects
//! that violate the format are skipped and reported as [`Diagnostic`]s;
//! faults that make the whole catalog unusable are returned as
//! [`CatalogError`]. A catalog is data: the reader opens only files named by
//! the catalog inside its root and never executes or fetches anything.

mod diag;
mod entries;
mod fields;
pub mod guide;
mod index;
mod json;
mod overlay;
mod paths;
mod types;

pub use diag::{Diagnostic, DiagnosticCode, Severity};
pub use index::{CatalogError, CatalogErrorKind, LoadedCatalog, read_catalog};
pub use paths::{MAX_RELPATH_CHARS, PathError, check_relpath};
pub use types::{
    Alias, Catalog, Category, CategoryText, DEFAULT_LANGUAGE, FORMAT, FORMAT_MAJOR, FORMAT_MINOR,
    FORMAT_PREFIX, GUIDE_EXTENSION, GuideHeader, GuideText, I18N_DIR, INDEX_FILE, Limits,
    NotForTask, Origin, Overlay, Program, ProgramText, ReadOptions, Section, StandaloneGuide,
    is_valid_catalog_id, is_valid_category_id, is_valid_date, is_valid_language, is_valid_name,
};

use guide::GuideFile;
use paths::Root;

impl Catalog {
    /// Reads the guide at `path`, relative to the root, in full.
    ///
    /// # Errors
    ///
    /// Fails when the path leaves the root, the file is missing, exceeds
    /// [`Limits::guide_bytes`], is not UTF-8 or has no valid front matter.
    pub fn read_guide(&self, path: &str, limits: &Limits) -> Result<GuideFile, String> {
        let root = self.open_root()?;
        let file = root.resolve(path).map_err(|error| error.to_string())?;
        guide::read_full(&file, limits)
    }

    /// Files of the material of the guide at `path`, relative to the root;
    /// empty when the guide has no material directory.
    #[must_use]
    pub fn guide_material(&self, path: &str, limits: &Limits) -> Vec<String> {
        self.open_root()
            .map(|root| guide::material(&root, path, limits))
            .unwrap_or_default()
    }

    fn open_root(&self) -> Result<Root, String> {
        Root::open(&self.root).map_err(|error| format!("catalog directory cannot be opened: {error}"))
    }
}
