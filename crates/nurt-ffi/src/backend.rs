// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Operations of the engine reached through the C interface.

use nurt_contracts::stage::{CancelToken, EventSink};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::CallError;

/// Operation of the engine behind an exported function.
///
/// The arguments of the C function are passed to the engine as one JSON
/// value, the input of the operation:
///
/// | Operation | Input |
/// |---|---|
/// | `engine_config_get`, `ext_list` | `null` |
/// | `parse`, `correct` | `{"program": text, "variant": "a" \| "b" \| "c"}` |
/// | `print` | `{"ast": object, "variant": "a" \| "b" \| "c"}` |
/// | `check`, `run` | the request envelope `nurt-request/1` as serialised by the contract |
/// | `page` | `{"cursor": text, "limit": number}` |
/// | `catalog`, `describe`, `history` | the JSON object argument or `null` |
/// | `help`, `space` | the text argument or `null` |
/// | `session_end`, `ext_load` | the text argument |
///
/// `run` serves both `nurt_run_json` and `nurt_exec_start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Function {
    /// `nurt_engine_config_get`.
    EngineConfigGet,
    /// `nurt_parse`.
    Parse,
    /// `nurt_print`.
    Print,
    /// `nurt_correct`.
    Correct,
    /// `nurt_check`.
    Check,
    /// `nurt_run_json` and `nurt_exec_start`.
    Run,
    /// `nurt_page`.
    Page,
    /// `nurt_catalog`.
    Catalog,
    /// `nurt_describe`.
    Describe,
    /// `nurt_help`.
    Help,
    /// `nurt_history`.
    History,
    /// `nurt_space`.
    Space,
    /// `nurt_session_end`.
    SessionEnd,
    /// `nurt_ext_load`.
    ExtLoad,
    /// `nurt_ext_list`.
    ExtList,
}

/// Form of one result of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OutputKind {
    /// A JSON document.
    Json,
    /// UTF-8 text.
    Text,
}

impl Function {
    /// Every operation in the order of `nurt.h`.
    pub const ALL: &'static [Self] = &[
        Self::EngineConfigGet,
        Self::Parse,
        Self::Print,
        Self::Correct,
        Self::Check,
        Self::Run,
        Self::Page,
        Self::Catalog,
        Self::Describe,
        Self::Help,
        Self::History,
        Self::Space,
        Self::SessionEnd,
        Self::ExtLoad,
        Self::ExtList,
    ];

    /// Name of the operation as written in JSON, for example `ext_load`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::EngineConfigGet => "engine_config_get",
            Self::Parse => "parse",
            Self::Print => "print",
            Self::Correct => "correct",
            Self::Check => "check",
            Self::Run => "run",
            Self::Page => "page",
            Self::Catalog => "catalog",
            Self::Describe => "describe",
            Self::Help => "help",
            Self::History => "history",
            Self::Space => "space",
            Self::SessionEnd => "session_end",
            Self::ExtLoad => "ext_load",
            Self::ExtList => "ext_list",
        }
    }

    /// Results of the operation, in the order of the output buffers of the
    /// C function.
    #[must_use]
    pub const fn outputs(self) -> &'static [OutputKind] {
        match self {
            Self::Parse | Self::Check => &[OutputKind::Json, OutputKind::Json],
            Self::Print | Self::Describe | Self::Help => &[OutputKind::Text],
            Self::SessionEnd | Self::ExtLoad => &[],
            Self::EngineConfigGet
            | Self::Correct
            | Self::Run
            | Self::Page
            | Self::Catalog
            | Self::History
            | Self::Space
            | Self::ExtList => &[OutputKind::Json],
        }
    }
}

/// Response of an execution and whether it ended by cancellation.
#[cfg_attr(
    not(feature = "stub"),
    allow(
        dead_code,
        reason = "constructed by the engine backend, which is connected with the engine API"
    )
)]
pub(crate) struct RunOutcome {
    pub(crate) response: Vec<u8>,
    pub(crate) cancelled: bool,
}

/// Engine behind an open `nurt_engine` handle.
pub(crate) trait Backend: Send + Sync {
    /// Runs an operation other than [`Function::Run`] and returns its
    /// results in the order of [`Function::outputs`].
    fn call(&self, function: Function, input: &Value) -> Result<Vec<Vec<u8>>, CallError>;

    /// Runs a request envelope, emitting events and observing cancellation.
    fn run(
        &self,
        request: &Value,
        events: &dyn EventSink,
        cancel: &CancelToken,
    ) -> Result<RunOutcome, CallError>;
}

/// Opens the engine for a validated configuration.
#[cfg(feature = "stub")]
pub(crate) fn open(config: &Value) -> Result<Box<dyn Backend>, CallError> {
    crate::stub::StubBackend::open(config).map(|backend| Box::new(backend) as Box<dyn Backend>)
}

/// Opens the engine for a validated configuration.
#[cfg(not(feature = "stub"))]
pub(crate) fn open(config: &Value) -> Result<Box<dyn Backend>, CallError> {
    let _ = config;
    Err(CallError::internal(
        "the engine is not available in this build of the library",
    ))
}

#[cfg(test)]
mod tests {
    use super::{Function, OutputKind};

    #[test]
    fn names_round_trip_through_json() {
        for function in Function::ALL {
            let json = serde_json::to_value(function).unwrap();
            assert_eq!(json, function.name());
            assert_eq!(serde_json::from_value::<Function>(json).unwrap(), *function);
        }
    }

    #[test]
    fn functions_with_two_buffers_return_two_documents() {
        assert_eq!(
            Function::Parse.outputs(),
            &[OutputKind::Json, OutputKind::Json]
        );
        assert_eq!(Function::Check.outputs().len(), 2);
        assert!(Function::SessionEnd.outputs().is_empty());
        assert_eq!(Function::Help.outputs(), &[OutputKind::Text]);
    }
}
