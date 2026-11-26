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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_files_with_the_extension_sorted_by_name() {
        let dir = tempfile::tempdir().expect("temporary directory");
        for name in ["b.md", "a.md", "c.txt", "\u{17c}.md"] {
            std::fs::write(dir.path().join(name), "x").expect("file");
        }
        std::fs::create_dir(dir.path().join("d.md")).expect("directory");
        let names: Vec<String> = list_files(dir.path(), "md")
            .expect("listing")
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.md", "b.md", "\u{17c}.md"]);
    }

    #[test]
    fn write_text_creates_parent_directories_and_keeps_multi_byte_text() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("a").join("b").join("page.html");
        write_text(&path, "za\u{17c}\u{f3}\u{142}\u{107}\r\n").expect("written");
        assert_eq!(read_text(&path).expect("read"), "za\u{17c}\u{f3}\u{142}\u{107}\r\n");
    }

    #[test]
    fn missing_files_and_directories_are_reported_with_their_path() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let missing = dir.path().join("missing");
        let error = read_text(&missing).expect_err("missing file");
        assert!(matches!(error, DocgenError::Io { .. }));
        assert!(error.to_string().starts_with(&missing.display().to_string()));
        assert!(std::error::Error::source(&error).is_some());
        assert!(list_files(&missing, "md").is_err());
    }

    #[test]
    fn describes_invalid_inputs_and_failed_checks() {
        let invalid = DocgenError::invalid("table", "bad");
        assert_eq!(invalid.to_string(), "table: bad");
        assert!(std::error::Error::source(&invalid).is_none());
        assert_eq!(
            DocgenError::Checks { count: 3 }.to_string(),
            "3 documentation problems found"
        );
    }
}
