// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-mcp::backend
// Responsibility:   Interface between the server and the engine.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Interface between the server and the engine.
//!
//! The server reaches the engine only through [`Backend`]. Every method is
//! synchronous and may block; the server calls it from a blocking thread, so
//! an implementation needs no asynchronous runtime.

use std::fmt;
use std::sync::Arc;

use nurt_contracts::envelope::{Language, Request, Response};
use nurt_contracts::error::{AbiStatus, Error};
use nurt_contracts::stage::{CancelToken, EventSink};
use serde::{Deserialize, Serialize};

/// Engine operations used by the server.
///
/// Language errors of a program are part of the [`Response`]; a
/// [`CallError`] reports only a failure of the call itself, with the same
/// meaning as a non-zero `nurt_status` of the C interface.
pub trait Backend: Send + Sync + 'static {
    /// Returns the version and the limits of the engine.
    fn info(&self) -> BackendInfo;

    /// Returns the revision of the command catalog the descriptions come
    /// from, for the metadata of the tool definition; `None` when the backend
    /// has no catalog revision.
    fn catalog_rev(&self) -> Option<String> {
        None
    }

    /// Runs a request and returns the response.
    ///
    /// Progress, output of launched programs and log messages are emitted to
    /// `events` while the request runs. The engine stops the execution when
    /// `cancel` is cancelled and reports the cancellation in the response.
    ///
    /// # Errors
    ///
    /// Returns a [`CallError`] when the request cannot be handled at all.
    fn run(
        &self,
        request: &Request,
        events: Arc<dyn EventSink>,
        cancel: CancelToken,
    ) -> Result<Response, CallError>;

    /// Returns built-in help as text.
    ///
    /// # Errors
    ///
    /// Returns a [`CallError`] when the help cannot be produced.
    fn help(&self, query: &HelpQuery) -> Result<String, CallError>;

    /// Returns a description layer for models as text.
    ///
    /// # Errors
    ///
    /// Returns a [`CallError`], with status [`AbiStatus::Arg`] for an
    /// unknown group.
    fn describe(&self, subject: &DescribeSubject) -> Result<String, CallError>;

    /// Returns a published JSON schema as text.
    ///
    /// # Errors
    ///
    /// Returns a [`CallError`] when the schema cannot be produced.
    fn schema(&self, name: SchemaName) -> Result<String, CallError>;

    /// Binds and plans a request and reports what running it would touch,
    /// without running it.
    ///
    /// # Errors
    ///
    /// Returns a [`CallError`] when the request cannot be handled at all; a
    /// language error of the program is the [`CheckOutcome::Failed`] value.
    fn check(&self, request: &Request) -> Result<CheckOutcome, CallError>;
}

/// Result of a check without effect.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum CheckOutcome {
    /// The check document `nurt-check/1` with the member `bound`.
    Report(serde_json::Value),
    /// The program was rejected before the check completed.
    Failed(Error),
}

/// Version and limits of the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendInfo {
    /// Version of the engine, for example `0.1.0`.
    pub version: String,
    /// Largest number of calls handled at the same time; at least 1.
    pub max_inflight: usize,
}

/// Failure of a call to the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallError {
    /// Status of the call, never [`AbiStatus::Ok`].
    pub status: AbiStatus,
    /// Description of the failure.
    pub message: String,
}

impl CallError {
    /// Creates a call error.
    #[must_use]
    pub fn new(status: AbiStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.status.name(), self.message)
    }
}

impl std::error::Error for CallError {}

/// Kind of a help topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HelpKind {
    /// A command.
    Command,
    /// A group of commands.
    Group,
    /// An error code.
    Error,
    /// A domain guide.
    Skill,
    /// The syntax of the language.
    Syntax,
    /// The corrections applied to program text.
    Corrections,
}

impl HelpKind {
    /// Every kind, in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Command,
        Self::Group,
        Self::Error,
        Self::Skill,
        Self::Syntax,
        Self::Corrections,
    ];

    /// Returns the name of the kind, as written in JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Group => "group",
            Self::Error => "error",
            Self::Skill => "skill",
            Self::Syntax => "syntax",
            Self::Corrections => "corrections",
        }
    }
}

/// Query of built-in help.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelpQuery {
    /// Topic: a command, group, error code, `syntax` or `corrections`;
    /// absent for the index of topics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// Kind of the topic, when the name alone is ambiguous.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<HelpKind>,
    /// Number of examples to include.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub examples: Option<u32>,
    /// Language of the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
}

/// Description layer requested from the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DescribeSubject {
    /// The core layer, used as the description of the `nurt` tool.
    Core,
    /// The layer of one group of commands.
    Group(String),
}

/// Name of a published JSON schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SchemaName {
    /// Surface syntax tree `nurt-ast/1`.
    Ast,
    /// Bound syntax tree `nurt-bound/1`.
    Bound,
    /// Effect report `nurt-report/1`.
    Report,
    /// Request envelope `nurt-request/1`.
    Request,
    /// Configuration `nurt-config/1`.
    Config,
    /// Neutral tool definition `nurt-tool/1`.
    Tool,
}

impl SchemaName {
    /// Every schema name, in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Ast,
        Self::Bound,
        Self::Report,
        Self::Request,
        Self::Config,
        Self::Tool,
    ];

    /// Returns the name used in `nurt schema <name>`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ast => "ast",
            Self::Bound => "bound",
            Self::Report => "report",
            Self::Request => "request",
            Self::Config => "config",
            Self::Tool => "tool",
        }
    }

    /// Returns the schema with the given name, or `None` for an unknown name.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_mcp::SchemaName;
    ///
    /// assert_eq!(SchemaName::from_name("report"), Some(SchemaName::Report));
    /// assert_eq!(SchemaName::from_name("catalog"), None);
    /// ```
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|schema| schema.as_str() == name)
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::AbiStatus;

    use super::{CallError, HelpKind, HelpQuery, SchemaName};

    #[test]
    fn schema_names_round_trip() {
        for name in SchemaName::ALL {
            assert_eq!(SchemaName::from_name(name.as_str()), Some(*name));
        }
        assert_eq!(SchemaName::from_name("Report"), None);
    }

    #[test]
    fn help_kinds_match_their_json_names() {
        for kind in HelpKind::ALL {
            let json = serde_json::to_value(kind).unwrap_or_default();
            assert_eq!(json, kind.as_str());
        }
    }

    #[test]
    fn help_query_rejects_unknown_fields() {
        let parsed = serde_json::from_str::<HelpQuery>(r#"{"topic": "find", "depth": 2}"#);
        assert!(parsed.is_err());
    }

    #[test]
    fn call_error_names_the_status() {
        let error = CallError::new(AbiStatus::Io, "journal is not writable");
        assert_eq!(error.to_string(), "NURT_E_IO: journal is not writable");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
