// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::serve::stub
// Responsibility:   Answers from the recorded cases of the library stub.
// Dependencies:     nurt_contracts, nurt_ffi, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Answers from the recorded cases of the library stub.
//!
//! The cases are those of `nurt_ffi::stub`: a call is answered by the case
//! whose key equals the key of the call. The input of a call is built the
//! way the C interface builds it, so one case directory serves both
//! interfaces:
//!
//! | Method | Input of the case | Result |
//! |---|---|---|
//! | `run`, `check` | the request envelope | the first result; `check` with the members of the second |
//! | `parse`, `correct` | `{"program", "variant"}`, variant `a` when not given | `parse`: `{"ast"}` with the members of the second result |
//! | `print` | `{"ast", "variant"}`, variant `a` when not given | `{"text"}` |
//! | `page` | `{"cursor", "limit"}`, limit `0` when not given | the result |
//! | `describe`, `catalog`, `history` | the parameters, or `null` without parameters | `describe`: `{"description"}` |
//! | `help` | the topic, or `null` | `{"text"}` |
//! | `space` | the volume, or `null` | the result |
//!
//! `schema` and `info` are answered without cases. The events of a `run`
//! are those of the case, preceded by `exec.started` (effect `read`) unless
//! the case records its own, and followed by `exec.finished`.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nurt_contracts::ast::AST_VERSION;
use nurt_contracts::bound::BOUND_VERSION;
use nurt_contracts::config::CONFIG_FORMAT;
use nurt_contracts::envelope::{ENVELOPE_VERSION, Request};
use nurt_contracts::error::{AbiStatus, Error, ErrorCode, Stage};
use nurt_contracts::kinds::EffectKind;
use nurt_contracts::report::{REPORT_VERSION, ReportStatus};
use nurt_contracts::stage::{DeliveredEvent, Event, ExecFinished, ExecStarted};
use nurt_contracts::value::Instant;
use nurt_ffi::Function;
use nurt_ffi::stub::{Case, Catalog, CatalogError, case_key, normalize_input};
use serde_json::{Value, json};

use super::backend::{Backend, CallContext, Events, Summary};
use super::call::{self, Call};
use super::protocol::RpcError;

/// Feature reported by `initialize` and `info`.
const STUB_FEATURE: &str = "stub";
/// Report kind of a cancelled case without a recorded response.
const PROGRAM_KIND: &str = "program";

/// The cases of one directory.
pub(crate) struct StubBackend {
    catalog: Catalog,
}

impl StubBackend {
    /// Loads the cases of `dir`.
    pub(crate) fn load(dir: &Path) -> Result<Self, CatalogError> {
        Catalog::load(dir).map(|catalog| Self { catalog })
    }

    fn answer(&self, function: Function, input: Value) -> Result<&Case, RpcError> {
        let input = normalize_input(function, input).map_err(RpcError::invalid_params)?;
        let case = self.catalog.lookup(function, &input).ok_or_else(|| {
            RpcError::status(
                AbiStatus::Internal,
                format!("no stub case answers this call of `{}`", function.name()),
            )
            .with_data("key", case_key(function, &input).into())
            .with_data("input", input.clone())
        })?;
        if let Some(message) = &case.panic {
            return Err(RpcError::status(
                AbiStatus::Internal,
                "the server failed while answering the request",
            )
            .with_data("panic", message.as_str().into()));
        }
        if case.status != AbiStatus::Ok {
            let message = case
                .message
                .clone()
                .unwrap_or_else(|| format!("the stub case `{}` fails", case.name));
            return Err(
                RpcError::status(case.status, message).with_data("case", case.name.as_str().into())
            );
        }
        Ok(case)
    }

    fn first(&self, function: Function, input: Value) -> Result<Value, RpcError> {
        self.answer(function, input).map(|case| output(case, 0))
    }

    fn run(&self, request: &Request, context: &CallContext<'_>) -> Result<Value, RpcError> {
        let case = self.answer(Function::Run, call::to_json(request)?)?;
        let mut sequence = Sequence::new(context.events);
        let records_start = case
            .events
            .first()
            .is_some_and(|event| matches!(event, Event::ExecStarted(_)));
        if !records_start {
            sequence.emit(Event::ExecStarted(ExecStarted {
                ast_hash: None,
                effect: EffectKind::Read,
            }));
        }
        for event in &case.events {
            if context.cancel.is_cancelled() {
                break;
            }
            sequence.emit(event.clone());
        }
        let cancelled = context
            .cancel
            .wait_timeout(Duration::from_millis(case.delay_ms))
            .is_some();
        let response = if cancelled {
            cancelled_response(case)?
        } else {
            output(case, 0)
        };
        sequence.emit(Event::ExecFinished(ExecFinished {
            status: if cancelled {
                ReportStatus::Failed
            } else {
                report_status(&response)
            },
            error: cancelled.then_some(ErrorCode::Cancelled),
        }));
        call::run_result(response)
    }
}

impl Backend for StubBackend {
    fn summary(&self) -> Summary {
        Summary {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            abi: Some(nurt_ffi::nurt_abi_version()),
            features: vec![STUB_FEATURE.to_owned()],
            groups: Vec::new(),
        }
    }

    fn call(&self, call: Call, context: &CallContext<'_>) -> Result<Value, RpcError> {
        match call {
            Call::Run(request) => self.run(&request, context),
            Call::Check(request) => {
                let case = self.answer(Function::Check, call::to_json(&*request)?)?;
                Ok(call::merge(output(case, 0), output(case, 1)))
            }
            Call::Parse(params) => {
                let input = json!({ "program": params.program, "variant": params.variant.unwrap_or_default() });
                let case = self.answer(Function::Parse, input)?;
                Ok(call::merge(
                    call::member("ast", output(case, 0)),
                    output(case, 1),
                ))
            }
            Call::Print(params) => {
                let input =
                    json!({ "ast": params.ast, "variant": params.variant.unwrap_or_default() });
                self.first(Function::Print, input)
                    .map(|text| call::member("text", text))
            }
            Call::Correct(params) => {
                let input = json!({ "program": params.program, "variant": params.variant.unwrap_or_default() });
                self.first(Function::Correct, input)
            }
            Call::Page(params) => {
                let input = json!({ "cursor": params.cursor, "limit": params.limit.unwrap_or(0) });
                self.first(Function::Page, input)
            }
            Call::Describe(params) => {
                let input = optional(call::to_json(&params)?);
                self.first(Function::Describe, input)
                    .map(|description| call::member("description", description))
            }
            Call::Help(params) => self
                .first(Function::Help, params.topic.into())
                .map(|text| call::member("text", text)),
            Call::Schema(params) => call::schema(&params.name),
            Call::Catalog(params) => self.first(Function::Catalog, params),
            Call::History(params) => self.first(Function::History, params),
            Call::Space(params) => self.first(Function::Space, params.volume.into()),
            Call::Rejected { method, error } => call::failure(method, *error),
            Call::Info => Ok(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "formats": {
                    "ast": AST_VERSION,
                    "bound": BOUND_VERSION,
                    "report": REPORT_VERSION,
                    "envelope": ENVELOPE_VERSION,
                    "config": CONFIG_FORMAT,
                },
                "features": [STUB_FEATURE],
                "cases": self.catalog.len(),
            })),
        }
    }
}

/// Numbers and stamps the events of one execution.
struct Sequence<'a> {
    events: &'a Events,
    seq: u64,
}

impl<'a> Sequence<'a> {
    fn new(events: &'a Events) -> Self {
        Self { events, seq: 0 }
    }

    fn emit(&mut self, event: Event) {
        self.seq += 1;
        self.events.send(&DeliveredEvent {
            seq: self.seq,
            time: now(),
            request: self.events.request(),
            event,
        });
    }
}

fn now() -> Instant {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_nanos()).unwrap_or(i64::MAX)
        });
    Instant::from_unix_nanos(nanos)
}

/// Result `index` of a case; text results are JSON strings.
fn output(case: &Case, index: usize) -> Value {
    case.output.get(index).cloned().unwrap_or(Value::Null)
}

/// Parameters as the input of a case: `null` for an empty object.
fn optional(params: Value) -> Value {
    match params {
        Value::Object(object) if object.is_empty() => Value::Null,
        params => params,
    }
}

fn report_status(response: &Value) -> ReportStatus {
    response
        .pointer("/report/status")
        .cloned()
        .and_then(|status| serde_json::from_value(status).ok())
        .unwrap_or(ReportStatus::Failed)
}

fn cancelled_response(case: &Case) -> Result<Value, RpcError> {
    let kind = case
        .output
        .first()
        .and_then(|response| response.pointer("/report/kind"))
        .and_then(Value::as_str)
        .unwrap_or(PROGRAM_KIND);
    let error =
        Error::new(ErrorCode::Cancelled, "the execution was cancelled").with_stage(Stage::Read);
    call::failure(kind, error)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn optional_parameters_become_null() {
        assert_eq!(optional(json!({})), Value::Null);
        assert_eq!(
            optional(json!({"subject": "find"})),
            json!({"subject": "find"})
        );
    }

    #[test]
    fn report_status_defaults_to_failed() {
        assert_eq!(
            report_status(&json!({"report": {"status": "ok"}})),
            ReportStatus::Ok
        );
        assert_eq!(report_status(&json!({})), ReportStatus::Failed);
    }

    #[test]
    fn current_time_is_after_the_epoch() {
        assert!(now() > Instant::UNIX_EPOCH);
    }

    #[test]
    fn a_missing_case_directory_is_reported() {
        let error = StubBackend::load(Path::new("/nonexistent/stub/cases"))
            .err()
            .unwrap();
        assert!(error.path.ends_with("cases"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
