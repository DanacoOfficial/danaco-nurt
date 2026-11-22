// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

use std::path::PathBuf;

/// Error of loading the suite or of running it.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A file or directory could not be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// Path of the file or directory.
        path: PathBuf,
        /// Error of the operating system.
        #[source]
        source: std::io::Error,
    },
    /// A file of the suite does not follow its format.
    #[error("{}: {message}", path.display())]
    Format {
        /// Path of the file.
        path: PathBuf,
        /// Description of the problem.
        message: String,
    },
    /// An argument of the invocation is not valid.
    #[error("{0}")]
    Invalid(String),
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn format(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::Format {
            path: path.into(),
            message: message.into(),
        }
    }
}
