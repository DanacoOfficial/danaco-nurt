// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Error type of the documentation generator.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Error of a generator operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum DocgenError {
    /// A file or directory could not be read or written.
    Io {
        /// Path of the file or directory.
        path: PathBuf,
        /// Underlying error.
        source: io::Error,
    },
    /// An input file has invalid content.
    Invalid {
        /// Path of the file, or a description of the input.
        input: String,
        /// Description of the problem.
        message: String,
    },
    /// The inputs are consistent in form but fail the documentation checks.
    Checks {
        /// Number of problems found.
        count: usize,
    },
}

impl DocgenError {
    /// Creates an error for a file that could not be accessed.
    pub fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    /// Creates an error for an input with invalid content.
    pub fn invalid(input: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Invalid {
            input: input.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for DocgenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Invalid { input, message } => write!(f, "{input}: {message}"),
            Self::Checks { count } => write!(f, "{count} documentation problems found"),
        }
    }
}

impl std::error::Error for DocgenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Result of a generator operation.
pub type Result<T> = std::result::Result<T, DocgenError>;

/// Reads a UTF-8 text file.
///
/// # Errors
///
/// Returns [`DocgenError::Io`] when the file cannot be read.
pub fn read_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|source| DocgenError::io(path, source))
}

/// Writes a text file, creating the parent directories.
///
/// # Errors
///
/// Returns [`DocgenError::Io`] when a directory or the file cannot be created.
pub fn write_text(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| DocgenError::io(parent, source))?;
    }
    std::fs::write(path, content).map_err(|source| DocgenError::io(path, source))
}

/// Lists the files with the given extension directly in a directory, sorted by name.
///
/// # Errors
///
/// Returns [`DocgenError::Io`] when the directory cannot be read.
pub fn list_files(dir: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    let entries = std::fs::read_dir(dir).map_err(|source| DocgenError::io(dir, source))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| DocgenError::io(dir, source))?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|found| found == extension) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}
