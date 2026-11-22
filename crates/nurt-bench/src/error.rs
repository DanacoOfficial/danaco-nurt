// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Error type of the benchmark tools.

use std::io;
use std::path::{Path, PathBuf};

/// Result of the benchmark tools.
pub type Result<T> = std::result::Result<T, BenchError>;

/// Error of the benchmark tools.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BenchError {
    /// A file or directory could not be read or written.
    #[error("cannot access {path}: {source}")]
    Io {
        /// Path of the file or directory.
        path: PathBuf,
        /// Underlying error.
        source: io::Error,
    },
    /// A file does not match its format.
    #[error("invalid {what} in {path}: {message}")]
    Format {
        /// Path of the file.
        path: PathBuf,
        /// Name of the format.
        what: &'static str,
        /// Description of the problem.
        message: String,
    },
    /// A value given on the command line or in a configuration is invalid.
    #[error("invalid argument: {0}")]
    Argument(String),
    /// The environment variable that holds the API key is not set.
    #[error("environment variable {0} is not set; export the API key of the provider")]
    MissingKey(String),
    /// A model provider returned an error or an unreadable response.
    #[error("model provider {provider} failed: {message}")]
    Provider {
        /// Provider name.
        provider: String,
        /// Description of the failure.
        message: String,
        /// Whether a later attempt can succeed (rate limit, server error,
        /// connection failure).
        retryable: bool,
    },
    /// The `nurt serve` process failed or broke the protocol.
    #[error("nurt serve failed: {0}")]
    Serve(String),
    /// A process of a tool, a setup script or a probe could not be run.
    #[error("process failed: {0}")]
    Process(String),
    /// The oracle could not evaluate a run.
    #[error("oracle failed: {0}")]
    Oracle(String),
}

impl BenchError {
    /// Creates an [`BenchError::Io`] error for a path.
    #[must_use]
    pub fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    /// Creates a [`BenchError::Format`] error for a path.
    #[must_use]
    pub fn format(path: &Path, what: &'static str, message: impl Into<String>) -> Self {
        Self::Format {
            path: path.to_path_buf(),
            what,
            message: message.into(),
        }
    }

    /// Whether a repeated attempt of the failed operation can succeed.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Provider {
                retryable: true,
                ..
            }
        )
    }
}
