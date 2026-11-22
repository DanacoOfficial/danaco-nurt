// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test double of the engine that answers from recorded cases.
//!
//! The module is compiled with the cargo feature `stub`, which is never
//! enabled in a release build. It lets embedding applications, protocol
//! servers and their tests exercise the whole C interface without a real
//! engine.
//!
//! A case is a JSON file `*.json` in the case directory: the directory
//! named by the variable [`CASES_DIR_VAR`], or the directory
//! `tests/fixtures/stub` of this crate. A case names the operation
//! ([`Function`]), its input as described there, and the answer:
//!
//! ```json
//! {
//!   "description": "Read request answered with one page of entries.",
//!   "function": "run",
//!   "input": {"nurt": "1.0", "program": "find \"/srv/app\""},
//!   "output": [{"nurt": "1.0", "report": {"nurt": "1.0", "status": "ok", "kind": "find"}}],
//!   "events": [{"t": "log", "level": "info", "message": "scan started", "source": null}],
//!   "delay_ms": 0
//! }
//! ```
//!
//! | Member | Meaning |
//! |---|---|
//! | `status` | status of the call, `ok` by default; any other value fails the call with `message` |
//! | `output` | results in the order of [`Function::outputs`]; text results are JSON strings |
//! | `events` | events of an execution (`run` only), emitted in order |
//! | `delay_ms` | time an execution (`run` only) takes after its events; cancellation ends it early |
//! | `panic` | message of a panic raised inside the operation |
//!
//! A call is answered by the case whose key equals the key of the call: the
//! SHA-256 digest of the JSON canonical form (RFC 8785) of
//! `{"function": ..., "input": ...}` ([`case_key`]). Request envelopes are
//! read and written back through the contract type before the digest is
//! taken, so equivalent envelopes match the same case. A call without a
//! matching case fails with `NURT_E_INTERNAL`, and `nurt_last_error` names
//! the key.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::envelope::{Request, Response};
use nurt_contracts::error::{AbiStatus, Error, ErrorCode, Stage};
use nurt_contracts::stage::{CancelToken, Event, EventSink};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::backend::{Backend, Function, OutputKind, RunOutcome};
use crate::error::{CallError, NurtStatus};
use crate::response;

/// Variable naming the case directory.
pub const CASES_DIR_VAR: &str = "NURT_STUB_DIR";

/// Variable with a comma-separated list of exported functions, for example
/// `nurt_info,nurt_exec_wait`, that panic inside their boundary. Used by
/// tests of the panic boundary in other languages.
pub const PANIC_VAR: &str = "NURT_STUB_PANIC";

const BUILT_IN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/stub");

thread_local! {
    static CASES_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static INJECTED_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Case directory used by engines opened on the calling thread: the
/// directory set with [`set_thread_cases_dir`], the variable
/// [`CASES_DIR_VAR`], or the built-in directory, in this order.
#[must_use]
pub fn cases_dir() -> PathBuf {
    if let Some(dir) = CASES_DIR.with(|dir| dir.borrow().clone()) {
        return dir;
    }
    std::env::var_os(CASES_DIR_VAR).map_or_else(|| PathBuf::from(BUILT_IN_DIR), PathBuf::from)
}

/// Sets or, with `None`, clears the case directory of the calling thread.
pub fn set_thread_cases_dir(dir: Option<PathBuf>) {
    CASES_DIR.with(|current| *current.borrow_mut() = dir);
}

/// Makes the exported function `function` (for example `nurt_info`) panic
/// inside its boundary when called from the calling thread; `None` stops
/// it.
pub fn set_thread_panic(function: Option<&str>) {
    INJECTED_PANIC.with(|current| *current.borrow_mut() = function.map(str::to_owned));
}

/// Panics when a panic of `function` was requested for the calling thread
/// or through [`PANIC_VAR`].
#[expect(
    clippy::panic,
    reason = "the stub raises the panic that the boundary tests expect"
)]
pub(crate) fn check_injected_panic(function: &str) {
    let on_thread = INJECTED_PANIC.with(|current| current.borrow().as_deref() == Some(function));
    let in_environment = std::env::var(PANIC_VAR)
        .is_ok_and(|names| names.split(',').any(|name| name.trim() == function));
    if on_thread || in_environment {
        panic!("injected panic in {function}");
    }
}

/// Key of a call. The key is the lowercase hexadecimal SHA-256 digest of
/// the canonical JSON form of `{"function": ..., "input": ...}`.
///
/// # Examples
///
/// ```
/// use nurt_ffi::Function;
/// use nurt_ffi::stub::case_key;
///
/// let key = case_key(Function::Help, &serde_json::Value::Null);
/// assert_eq!(key.len(), 64);
/// assert_eq!(key, case_key(Function::Help, &serde_json::Value::Null));
/// assert_ne!(key, case_key(Function::Describe, &serde_json::Value::Null));
/// ```
#[must_use]
pub fn case_key(function: Function, input: &Value) -> String {
    let call = json!({ "function": function, "input": input });
    // Canonicalisation fails only for numbers outside the range of JSON,
    // which a `Value` cannot hold.
    let canonical = serde_json_canonicalizer::to_vec(&call).unwrap_or_default();
    let digest = Sha256::digest(&canonical);
    digest
        .iter()
        .fold(String::with_capacity(64), |mut key, byte| {
            let _ = write!(key, "{byte:02x}");
            key
        })
}

/// Input of a call in the form used for its key. Request envelopes are
/// read and written back through [`Request`].
///
/// # Errors
///
/// Returns a message when the input of `check` or `run` is not a request
/// envelope.
pub fn normalize_input(function: Function, input: Value) -> Result<Value, String> {
    match function {
        Function::Check | Function::Run => {
            let request: Request = serde_json::from_value(input)
                .map_err(|error| format!("the input is not a request envelope: {error}"))?;
            serde_json::to_value(&request).map_err(|error| error.to_string())
        }
        _ => Ok(input),
    }
}

fn ok_status() -> AbiStatus {
    AbiStatus::Ok
}

/// Recorded answer to one call.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Case {
    /// File name without the extension.
    #[serde(skip)]
    pub name: String,
    /// What the case represents.
    pub description: String,
    /// Operation answered by the case.
    pub function: Function,
    /// Input of the operation.
    #[serde(default)]
    pub input: Value,
    /// Status of the call.
    #[serde(default = "ok_status")]
    pub status: AbiStatus,
    /// Message of a failing call.
    #[serde(default)]
    pub message: Option<String>,
    /// Results of a successful call.
    #[serde(default)]
    pub output: Vec<Value>,
    /// Events of an execution.
    #[serde(default)]
    pub events: Vec<Event>,
    /// Duration of an execution after its events, in milliseconds.
    #[serde(default)]
    pub delay_ms: u64,
    /// Message of a panic raised by the operation.
    #[serde(default)]
    pub panic: Option<String>,
}

impl Case {
    fn read(path: &Path) -> Result<Self, CatalogError> {
        let text = fs::read_to_string(path)
            .map_err(|error| CatalogError::new(path, format!("cannot read the case: {error}")))?;
        let mut case: Self = serde_json::from_str(&text)
            .map_err(|error| CatalogError::new(path, format!("invalid case: {error}")))?;
        case.name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        case.input = normalize_input(case.function, std::mem::take(&mut case.input))
            .map_err(|message| CatalogError::new(path, message))?;
        case.check()
            .map_err(|message| CatalogError::new(path, message))?;
        Ok(case)
    }

    fn check(&self) -> Result<(), String> {
        if self.function != Function::Run && (!self.events.is_empty() || self.delay_ms > 0) {
            return Err("only cases of `run` have `events` and `delay_ms`".to_owned());
        }
        if self.status != AbiStatus::Ok || self.panic.is_some() {
            return Ok(());
        }
        let kinds = self.function.outputs();
        if self.output.len() != kinds.len() {
            return Err(format!(
                "`{}` has {} results; the case gives {}",
                self.function.name(),
                kinds.len(),
                self.output.len()
            ));
        }
        if kinds
            .iter()
            .zip(&self.output)
            .any(|(kind, value)| *kind == OutputKind::Text && !value.is_string())
        {
            return Err("text results are written as JSON strings".to_owned());
        }
        if self.function == Function::Run {
            let response: Response =
                serde_json::from_value(self.output[0].clone()).map_err(|error| {
                    format!("the result is not a response nurt-response/1: {error}")
                })?;
            response
                .validate()
                .map_err(|error| format!("the response is not valid: {error}"))?;
        }
        Ok(())
    }

    fn outputs(&self) -> Vec<Vec<u8>> {
        self.function
            .outputs()
            .iter()
            .zip(&self.output)
            .map(|(kind, value)| match (kind, value) {
                (OutputKind::Text, Value::String(text)) => text.clone().into_bytes(),
                _ => serde_json::to_vec(value).unwrap_or_default(),
            })
            .collect()
    }

    #[expect(
        clippy::panic,
        reason = "a case may request a panic to test the boundary"
    )]
    fn answer(&self) -> Result<(), CallError> {
        if let Some(message) = &self.panic {
            panic!("{message}");
        }
        if self.status == AbiStatus::Ok {
            return Ok(());
        }
        let message = self
            .message
            .clone()
            .unwrap_or_else(|| format!("the stub case `{}` fails", self.name));
        Err(CallError::new(NurtStatus::from(self.status), message)
            .with_detail("case", self.name.as_str()))
    }

    fn cancelled_response(&self) -> Vec<u8> {
        let kind = self
            .output
            .first()
            .and_then(|response| response["report"]["kind"].as_str())
            .unwrap_or(response::REQUEST_KIND);
        let error =
            Error::new(ErrorCode::Cancelled, "the execution was cancelled").with_stage(Stage::Read);
        response::failure(kind, error)
    }
}

/// Failure to load a case directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError {
    /// File or directory concerned.
    pub path: PathBuf,
    /// What is wrong with it.
    pub message: String,
}

impl CatalogError {
    fn new(path: &Path, message: impl Into<String>) -> Self {
        Self {
            path: path.to_owned(),
            message: message.into(),
        }
    }
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for CatalogError {}

/// Cases of a directory, indexed by [`case_key`].
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    cases: HashMap<String, Case>,
}

impl Catalog {
    /// Loads every file of `dir` with the extension `json`.
    ///
    /// # Errors
    ///
    /// Fails when the directory or a case cannot be read, when a case is
    /// not valid, or when two cases answer the same call.
    pub fn load(dir: &Path) -> Result<Self, CatalogError> {
        let entries = fs::read_dir(dir).map_err(|error| {
            CatalogError::new(dir, format!("cannot read the case directory: {error}"))
        })?;
        let mut paths = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|error| {
                    CatalogError::new(dir, format!("cannot read the case directory: {error}"))
                })?
                .path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                paths.push(path);
            }
        }
        paths.sort();
        let mut cases = HashMap::with_capacity(paths.len());
        for path in paths {
            let case = Case::read(&path)?;
            let key = case_key(case.function, &case.input);
            if let Some(previous) = cases.insert(key, case) {
                return Err(CatalogError::new(
                    &path,
                    format!("the case answers the same call as `{}`", previous.name),
                ));
            }
        }
        Ok(Self { cases })
    }

    /// Case answering a call; `input` is in the form of [`normalize_input`].
    #[must_use]
    pub fn lookup(&self, function: Function, input: &Value) -> Option<&Case> {
        self.cases.get(&case_key(function, input))
    }

    /// Every case, in no particular order.
    pub fn cases(&self) -> impl Iterator<Item = &Case> {
        self.cases.values()
    }

    /// Number of cases.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cases.len()
    }

    /// Whether there is no case.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cases.is_empty()
    }
}

/// Engine replaced by the cases of a [`Catalog`].
pub(crate) struct StubBackend {
    catalog: Catalog,
}

impl StubBackend {
    /// Loads the cases of [`cases_dir`]; the configuration is accepted
    /// after validation and otherwise ignored.
    pub(crate) fn open(config: &Value) -> Result<Self, CallError> {
        let _ = config;
        let dir = cases_dir();
        Catalog::load(&dir)
            .map(|catalog| Self { catalog })
            .map_err(|error| {
                CallError::new(NurtStatus::Io, error.to_string())
                    .with_detail("path", error.path.display().to_string())
            })
    }

    fn find(&self, function: Function, input: &Value) -> Result<&Case, CallError> {
        let case = self.catalog.lookup(function, input).ok_or_else(|| {
            CallError::internal(format!(
                "no stub case answers this call of `{}`",
                function.name()
            ))
            .with_detail("key", case_key(function, input))
            .with_detail("input", input.clone())
        })?;
        case.answer()?;
        Ok(case)
    }
}

impl Backend for StubBackend {
    fn call(&self, function: Function, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        self.find(function, input).map(Case::outputs)
    }

    fn run(
        &self,
        request: &Value,
        events: &dyn EventSink,
        cancel: &CancelToken,
    ) -> Result<RunOutcome, CallError> {
        let case = self.find(Function::Run, request)?;
        for event in &case.events {
            if cancel.is_cancelled() {
                break;
            }
            events.emit(event.clone());
        }
        if cancel
            .wait_timeout(Duration::from_millis(case.delay_ms))
            .is_some()
        {
            return Ok(RunOutcome {
                response: case.cancelled_response(),
                cancelled: true,
            });
        }
        Ok(RunOutcome {
            response: case.outputs().into_iter().next().unwrap_or_default(),
            cancelled: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use nurt_contracts::envelope::Response;
    use nurt_contracts::stage::{CancelToken, DiscardEvents};
    use serde_json::json;

    use super::{Catalog, Function, StubBackend, case_key, cases_dir, normalize_input};
    use crate::backend::Backend;

    #[test]
    fn built_in_cases_load_and_are_valid() {
        let catalog = Catalog::load(&cases_dir()).unwrap();
        assert!(!catalog.is_empty());
        for function in Function::ALL {
            assert!(
                catalog.cases().any(|case| case.function == *function),
                "no case for {}",
                function.name()
            );
        }
    }

    #[test]
    fn equivalent_envelopes_have_the_same_key() {
        let compact = normalize_input(
            Function::Run,
            json!({"nurt": "1.0", "program": "find \"/srv/app\""}),
        )
        .unwrap();
        let explicit = normalize_input(
            Function::Run,
            json!({"program": "find \"/srv/app\"", "options": {}, "nurt": "1.0"}),
        )
        .unwrap();
        assert_eq!(
            case_key(Function::Run, &compact),
            case_key(Function::Run, &explicit)
        );
        assert!(normalize_input(Function::Check, json!({"program": 1})).is_err());
    }

    #[test]
    fn rejects_cases_with_wrong_results_and_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let wrong = dir.path().join("wrong.json");
        fs::write(
            &wrong,
            r#"{"description": "two results", "function": "help", "output": ["a", "b"]}"#,
        )
        .unwrap();
        let error = Catalog::load(dir.path()).unwrap_err();
        assert_eq!(error.path, wrong);
        fs::write(
            &wrong,
            r#"{"description": "text as object", "function": "help", "output": [{}]}"#,
        )
        .unwrap();
        assert!(Catalog::load(dir.path()).is_err());
        fs::write(
            &wrong,
            r#"{"description": "events", "function": "help", "output": ["a"], "delay_ms": 5}"#,
        )
        .unwrap();
        assert!(Catalog::load(dir.path()).is_err());
        fs::write(
            &wrong,
            r#"{"description": "help", "function": "help", "output": ["a"]}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("same.json"),
            r#"{"description": "help", "function": "help", "output": ["b"]}"#,
        )
        .unwrap();
        let error = Catalog::load(dir.path()).unwrap_err();
        assert!(error.to_string().contains("same call"));
        assert!(Catalog::load(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn rejects_run_results_that_are_not_responses() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("bad.json"),
            r#"{"description": "bad", "function": "run", "input": {"nurt": "1.0", "program": "x"}, "output": [{"report": 1}]}"#,
        )
        .unwrap();
        assert!(
            Catalog::load(dir.path())
                .unwrap_err()
                .message
                .contains("nurt-response/1")
        );
    }

    #[test]
    fn cancelled_run_reports_e_cancelled_with_the_kind_of_the_case() {
        let backend = StubBackend::open(&serde_json::Value::Null).unwrap();
        let input = normalize_input(
            Function::Run,
            json!({"nurt": "1.0", "program": "find \"/srv/archive\""}),
        )
        .unwrap();
        let cancel = CancelToken::new();
        cancel.cancel(nurt_contracts::stage::CancelReason::Requested);
        let outcome = backend.run(&input, &DiscardEvents, &cancel).unwrap();
        assert!(outcome.cancelled);
        let response: Response = serde_json::from_slice(&outcome.response).unwrap();
        response.validate().unwrap();
        assert_eq!(response.report.kind, "find");
        assert_eq!(response.report.error.unwrap().code.as_str(), "E_CANCELLED");
    }
}
