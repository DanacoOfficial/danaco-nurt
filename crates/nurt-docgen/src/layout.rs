// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Layout of the repository as seen by the generator.

use std::path::{Path, PathBuf};

/// Directories of the repository that the generator reads and writes.
#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    /// Creates the layout for a repository root.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Repository root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory of the command catalog.
    #[must_use]
    pub fn catalog(&self) -> PathBuf {
        self.root.join("catalog")
    }

    /// Directory of the command pages.
    #[must_use]
    pub fn commands(&self) -> PathBuf {
        self.root.join("docs").join("commands")
    }

    /// Directory of the general pages, the template and the style sheet.
    #[must_use]
    pub fn site(&self) -> PathBuf {
        self.root.join("docs").join("site")
    }

    /// Directory of the cookbook categories.
    #[must_use]
    pub fn cookbook(&self) -> PathBuf {
        self.root.join("docs").join("cookbook")
    }

    /// Directory of the cheat sheets and their correction tables.
    #[must_use]
    pub fn cheatsheets(&self) -> PathBuf {
        self.root.join("docs").join("cheatsheets")
    }

    /// Directory of the specification.
    #[must_use]
    pub fn spec(&self) -> PathBuf {
        self.root.join("spec")
    }

    /// Directory of the documentation tree.
    #[must_use]
    pub fn docs(&self) -> PathBuf {
        self.root.join("docs")
    }
}
