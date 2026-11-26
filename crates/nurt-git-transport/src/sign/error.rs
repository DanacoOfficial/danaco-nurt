// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Errors of signing.

use std::path::Path;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::SignatureFormat;
use serde_json::Value;

pub(crate) const NO_KEY: &str = "no_key";
pub(crate) const AGENT_UNAVAILABLE: &str = "agent_unavailable";
pub(crate) const KEY_LOCKED: &str = "key_locked";
pub(crate) const UNSUPPORTED_FORMAT: &str = "unsupported_format";
pub(crate) const PROGRAM_UNAVAILABLE: &str = "program_unavailable";
pub(crate) const PROGRAM_FAILED: &str = "program_failed";

pub(crate) fn sign_error(problem: &str, message: impl Into<String>) -> Error {
    Error::new(ErrorCode::GitSign, message).with_problem(problem)
}

pub(crate) fn unsupported_format(format: SignatureFormat) -> Error {
    sign_error(
        UNSUPPORTED_FORMAT,
        format!(
            "signature format `{format}` is not supported; set `gpg.format` to `openpgp` or `ssh`"
        ),
    )
    .with_detail("format", Value::String(format.as_str().to_owned()))
}

pub(crate) fn no_key(message: impl Into<String>) -> Error {
    sign_error(NO_KEY, message)
}

pub(crate) fn agent_unavailable(message: impl Into<String>) -> Error {
    sign_error(AGENT_UNAVAILABLE, message)
}

pub(crate) fn program_unavailable(message: impl Into<String>) -> Error {
    sign_error(PROGRAM_UNAVAILABLE, message)
}

pub(crate) fn program_failed(message: impl Into<String>) -> Error {
    sign_error(PROGRAM_FAILED, message)
}

pub(crate) fn key_locked(message: impl Into<String>) -> Error {
    sign_error(KEY_LOCKED, message)
}

pub(crate) fn with_path(error: Error, name: &str, path: &Path) -> Error {
    error.with_detail(name, Value::String(path.display().to_string()))
}

pub(crate) fn io_error(path: &Path, source: &std::io::Error) -> Error {
    let code = match source.kind() {
        std::io::ErrorKind::PermissionDenied => ErrorCode::Permission,
        std::io::ErrorKind::NotFound => ErrorCode::NotFound,
        _ => ErrorCode::Io,
    };
    Error::new(code, format!("cannot read `{}`: {source}", path.display()))
        .with_detail("path", Value::String(path.display().to_string()))
}

pub(crate) fn is_problem(error: &Error, problem: &str) -> bool {
    error.code() == ErrorCode::GitSign && error.data().problem.as_deref() == Some(problem)
}
