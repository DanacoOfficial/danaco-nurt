// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Errors of engine calls.
//!
//! Language errors (an unknown command, a missing file, an invalid envelope)
//! belong to the effect report and use [`nurt_contracts::error::Error`].
//! [`EngineError`] is reserved for calls that cannot produce a report:
//! opening an engine with an invalid configuration or with a state directory
//! that cannot be used.

use std::fmt;
use std::io;
use std::path::Path;

use nurt_contracts::config::ConfigError;
use nurt_contracts::envelope::{EnvelopeError, Request};
use nurt_contracts::error::{AbiStatus, Error, ErrorCode};
use serde_json::json;

/// Error of an engine call that is not a language error.
///
/// # Examples
///
/// ```
/// use nurt_contracts::error::{AbiStatus, ErrorCode};
/// use nurt_core::EngineError;
///
/// let error = EngineError::argument("session identifier must not be empty");
/// assert_eq!(error.status(), AbiStatus::Arg);
/// assert_eq!(error.to_error().code, ErrorCode::Arg);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct EngineError {
    status: AbiStatus,
    code: ErrorCode,
    message: String,
    config: Option<ConfigError>,
}

impl EngineError {
    /// Invalid configuration: a value that does not match its key or a file
    /// that is not valid TOML.
    #[must_use]
    pub fn config(error: ConfigError) -> Self {
        Self {
            status: AbiStatus::Arg,
            code: ErrorCode::Config,
            message: format!("invalid configuration: {error}"),
            config: Some(error),
        }
    }

    /// Invalid argument of the call.
    #[must_use]
    pub fn argument(message: impl Into<String>) -> Self {
        Self {
            status: AbiStatus::Arg,
            code: ErrorCode::Arg,
            message: message.into(),
            config: None,
        }
    }

    /// A path that must be absolute is relative.
    #[must_use]
    pub fn relative_path(what: &str, path: &Path) -> Self {
        Self {
            status: AbiStatus::Arg,
            code: ErrorCode::PathRelative,
            message: format!(
                "{what} must be an absolute path; given `{}`",
                path.display()
            ),
            config: None,
        }
    }

    /// Input or output failure while doing `action` on `path`.
    #[must_use]
    pub fn io(action: &str, path: &Path, error: &io::Error) -> Self {
        Self {
            status: AbiStatus::Io,
            code: ErrorCode::Io,
            message: format!("cannot {action} `{}`: {error}", path.display()),
            config: None,
        }
    }

    /// The command registry of the build cannot be assembled: the embedded
    /// catalog is inconsistent or two stage implementations conflict. The
    /// build is damaged, so the status is `NURT_E_INTERNAL`.
    #[must_use]
    pub fn registry(error: &Error) -> Self {
        Self {
            status: AbiStatus::Internal,
            code: error.code,
            message: format!("cannot assemble the command registry: {}", error.message),
            config: None,
        }
    }

    /// The journal of the process cannot be assembled.
    #[must_use]
    pub fn journal(error: &Error) -> Self {
        Self {
            status: AbiStatus::Internal,
            code: error.code,
            message: format!("cannot assemble the journal: {}", error.message),
            config: None,
        }
    }

    /// Status of the call for the C interface and the protocol server.
    #[must_use]
    pub const fn status(&self) -> AbiStatus {
        self.status
    }

    /// Description of the failure.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Configuration error behind the failure, when there is one.
    #[must_use]
    pub fn config_error(&self) -> Option<&ConfigError> {
        self.config.as_ref()
    }

    /// The same failure as a language error, for an effect report.
    #[must_use]
    pub fn to_error(&self) -> Error {
        let mut error = Error::new(self.code, self.message.clone());
        if let Some(config) = &self.config {
            if let Some(key) = &config.key {
                error = error.with_detail("key", json!(key));
            }
            if let Some(level) = config.level {
                error = error.with_detail("level", json!(level.as_str()));
            }
            if let Some(origin) = &config.origin {
                error = error.with_detail("origin", json!(origin));
            }
        }
        error
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.config
            .as_ref()
            .map(|error| error as &(dyn std::error::Error + 'static))
    }
}

/// Language error for a component that this build of the engine does not
/// contain.
pub(crate) fn unavailable(component: &str) -> Error {
    Error::new(
        ErrorCode::Unsupported,
        format!("{component} is not available in this build"),
    )
}

/// Language error for a request that breaks a rule of the envelope.
///
/// A relative path option is `E_PATH_RELATIVE`; every other rule is `E_ARG`.
/// When several inputs are given and one of them is program text, the fix is
/// that program alone.
pub(crate) fn envelope_error(error: &EnvelopeError, request: &Request) -> Error {
    match error {
        EnvelopeError::RelativePath { option, path } => {
            Error::new(ErrorCode::PathRelative, error.to_string())
                .with_found(path.clone())
                .with_detail("option", json!(option))
        }
        EnvelopeError::ConflictingInputs { fields } => {
            let conflict =
                Error::new(ErrorCode::Arg, error.to_string()).with_detail("fields", json!(fields));
            match &request.program {
                Some(program) => conflict.with_fix(program.clone()),
                None => conflict,
            }
        }
        _ => Error::new(ErrorCode::Arg, error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use nurt_contracts::config::ConfigLevel;
    use nurt_contracts::envelope::RequestOptions;

    use super::*;

    #[test]
    fn config_error_keeps_key_level_and_origin() {
        let error = EngineError::config(ConfigError {
            key: Some("journal.max_ops".to_owned()),
            level: Some(ConfigLevel::User),
            origin: Some("/home/user/.config/nurt/nurt.toml".to_owned()),
            message: "step limit 0 is out of range".to_owned(),
        });
        assert_eq!(error.status(), AbiStatus::Arg);
        assert!(error.message().contains("journal.max_ops"));
        let language = error.to_error();
        assert_eq!(language.code, ErrorCode::Config);
        assert_eq!(language.details["key"], json!("journal.max_ops"));
        assert_eq!(language.details["level"], json!("user"));
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn io_error_names_the_path() {
        let error = EngineError::io(
            "create",
            &PathBuf::from("/var/lib/nurt"),
            &io::Error::from(io::ErrorKind::PermissionDenied),
        );
        assert_eq!(error.status(), AbiStatus::Io);
        assert!(error.to_string().contains("/var/lib/nurt"));
        assert_eq!(error.to_error().code, ErrorCode::Io);
        assert!(error.config_error().is_none());
    }

    #[test]
    fn relative_path_is_path_relative() {
        let error = EngineError::relative_path("state directory", Path::new("state"));
        assert_eq!(error.to_error().code, ErrorCode::PathRelative);
    }

    #[test]
    fn conflicting_inputs_offer_the_program_as_fix() {
        let mut request = Request::program("find \"/srv/app\"");
        request.cursor = Some("c_1".to_owned());
        let Err(envelope) = request.validate() else {
            panic!("request with two inputs passed validation");
        };
        let error = envelope_error(&envelope, &request);
        assert_eq!(error.code, ErrorCode::Arg);
        assert_eq!(error.fix, vec!["find \"/srv/app\"".to_owned()]);
        assert_eq!(error.details["fields"], json!(["program", "cursor"]));
    }

    #[test]
    fn relative_option_is_path_relative() {
        let request = Request::program("find").with_options(RequestOptions {
            save_to: Some("out.json".to_owned()),
            ..RequestOptions::default()
        });
        let Err(envelope) = request.validate() else {
            panic!("relative save_to passed validation");
        };
        let error = envelope_error(&envelope, &request);
        assert_eq!(error.code, ErrorCode::PathRelative);
        assert_eq!(error.found.as_deref(), Some("out.json"));
    }

    #[test]
    fn registry_failure_is_internal_and_keeps_the_code() {
        let error = EngineError::registry(&Error::new(ErrorCode::Catalog, "duplicate command"));
        assert_eq!(error.status(), AbiStatus::Internal);
        assert_eq!(error.to_error().code, ErrorCode::Catalog);
        assert!(error.message().ends_with("duplicate command"));
    }

    #[test]
    fn unavailable_component_is_unsupported() {
        assert_eq!(unavailable("planner").code, ErrorCode::Unsupported);
    }
}
