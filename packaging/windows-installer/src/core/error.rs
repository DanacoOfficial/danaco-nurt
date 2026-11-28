// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/error.rs
// Responsibility:   Error type of the setup program: a stable code for the catalog of the window and a detail for the log.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Error type of the setup program.
//!
//! The code selects the text of the catalog of the window; the detail is data
//! (a path, a system message) that the window shows under the text and that
//! the log keeps.

use std::fmt;
use std::io;
use std::path::Path;

use serde::Serialize;

/// Failure of a setup action.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SetupError {
    /// Stable identifier of the failure; a key of the error catalog of the window.
    pub(crate) code: &'static str,
    /// Data of the failure: path, system message.
    pub(crate) detail: String,
}

/// Result of a setup action.
pub(crate) type SetupResult<T> = Result<T, SetupError>;

impl SetupError {
    /// Creates an error with the given code and detail.
    pub(crate) fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    /// Creates an error from an I/O failure on a path; the code follows the cause.
    pub(crate) fn io(path: &Path, error: &io::Error) -> Self {
        let code = match error.raw_os_error() {
            Some(5) => "access-denied",
            Some(32 | 33) => "in-use",
            Some(39 | 112) => "no-space",
            Some(123 | 161) => "invalid-path",
            _ => match error.kind() {
                io::ErrorKind::PermissionDenied => "access-denied",
                io::ErrorKind::StorageFull => "no-space",
                _ => "io",
            },
        };
        Self {
            code,
            detail: format!("{}: {error}", path.display()),
        }
    }
}

impl fmt::Display for SetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for SetupError {}

impl From<SetupError> for String {
    fn from(error: SetupError) -> Self {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_codes_follow_the_cause() {
        let path = Path::new("C:\\x");
        assert_eq!(
            SetupError::io(path, &io::Error::from_raw_os_error(5)).code,
            "access-denied"
        );
        assert_eq!(
            SetupError::io(path, &io::Error::from_raw_os_error(32)).code,
            "in-use"
        );
        assert_eq!(
            SetupError::io(path, &io::Error::from_raw_os_error(112)).code,
            "no-space"
        );
        assert_eq!(
            SetupError::io(path, &io::Error::from_raw_os_error(2)).code,
            "io"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
