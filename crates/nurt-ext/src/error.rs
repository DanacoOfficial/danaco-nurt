// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-ext::error
// Responsibility:   Construction of contract errors for the extension interface.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Construction of contract errors for the extension interface.

use std::io;
use std::path::Path;

use nurt_contracts::error::{Error, ErrorCode};

use crate::manifest::{MANIFEST_FILE, ManifestError};

/// Error with a code and a message.
pub(crate) fn error(code: ErrorCode, message: impl Into<String>) -> Error {
    Error::new(code, message)
}

/// Error of a file operation on `path`, with the path in the details.
pub(crate) fn io_error(action: &str, path: &Path, source: &io::Error) -> Error {
    let code = match source.kind() {
        io::ErrorKind::NotFound => ErrorCode::NotFound,
        io::ErrorKind::PermissionDenied => ErrorCode::Permission,
        io::ErrorKind::AlreadyExists => ErrorCode::Exists,
        _ => ErrorCode::Io,
    };
    let shown = path.display().to_string();
    Error::new(code, format!("cannot {action} {shown}: {source}"))
        .with_detail("path", serde_json::Value::from(shown))
}

/// Error of a command of an extension.
pub(crate) fn stage_failed(command: &str, message: impl std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::StageFailed,
        format!("`{command}` failed: {message}"),
    )
    .with_detail("command", serde_json::Value::from(command))
}

impl From<ManifestError> for Error {
    fn from(problem: ManifestError) -> Self {
        Error::new(ErrorCode::Catalog, problem.to_string())
            .with_detail("file", serde_json::Value::from(MANIFEST_FILE))
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
