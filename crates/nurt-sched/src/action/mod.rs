// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Actions of schedules.
//!
//! The built-in kind [`PIPELINE_KIND`] runs a pipeline of the language as an
//! operation of the session `schedule:<id>`. Other kinds are provided by a
//! [`ScheduleAction`] registered in an [`ActionRegistry`] (local kinds) or by
//! a client of the service that registers the kind over the control channel
//! (remote kinds, handled by the schedule module).
//!
//! Every definition carries an integrity hash of its action, `ast_hash`. For
//! a pipeline it is the hash of the bound tree defined by the contracts
//! (`sha256:` and the SHA-256 of the canonical JSON of the tree without source
//! ranges); for another kind it is the same hash of `{"kind", "args"}`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use nurt_contracts::ast::strip_for_hash;
use nurt_contracts::bound::{BoundArg, BoundExpr, BoundHead, BoundPipeline, BoundStage};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::stage::CancelToken;
use nurt_contracts::value::{Record, RecordKind, Value};
use sha2::{Digest, Sha256};

use crate::errors::{failed, invalid};
use crate::store::{Definition, RunState};

/// Kind of an action that runs a pipeline of the language.
pub const PIPELINE_KIND: &str = "pipeline";

const HASH_PREFIX: &str = "sha256:";
const PIPE: &str = " |> ";
const EXIT_OK: i64 = 0;
const EXIT_TIMED_OUT: i64 = -1;
const VOLATILE_FIELDS: &[&str] = &["started", "elapsed", "finished", "pid", "cursor"];

/// Facts about one run, given to a [`ScheduleAction`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRun {
    /// Schedule identifier.
    pub schedule: String,
    /// Run number.
    pub run: u64,
    /// Label of the schedule.
    pub label: String,
}

/// Result of a run of an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Outcome.
    pub state: RunState,
    /// Exit code of the program of the action.
    pub exit_code: Option<i64>,
    /// Short description of the outcome.
    pub message: Option<String>,
    /// Journal operation of the run.
    pub operation: Option<u64>,
    /// Job of the program of the action.
    pub job: Option<String>,
    /// File holding the full output.
    pub output_file: Option<String>,
    /// SHA-256 of the output, lowercase hex.
    pub output_sha256: Option<String>,
}

impl Outcome {
    /// An outcome with a state and a message and nothing else.
    #[must_use]
    pub fn new(state: RunState, message: impl Into<String>) -> Self {
        Self {
            state,
            exit_code: None,
            message: Some(message.into()),
            operation: None,
            job: None,
            output_file: None,
            output_sha256: None,
        }
    }

    /// Outcome of a pipeline that ended with an error.
    #[must_use]
    pub fn from_error(error: &Error) -> Self {
        let state = match error.code() {
            ErrorCode::Timeout => RunState::Timeout,
            ErrorCode::Cancelled => RunState::Stopped,
            _ => RunState::Failed,
        };
        let mut outcome = Self::new(state, error.message.clone());
        if state == RunState::Timeout {
            outcome.exit_code = Some(EXIT_TIMED_OUT);
        }
        outcome
    }

    /// Outcome of a pipeline from its output records.
    ///
    /// An `exec` record gives the exit code and the output whose hash is
    /// [`Outcome::output_sha256`]; without one the hash covers the JSON of
    /// the records without the fields that change between equal runs (times,
    /// process identifiers).
    #[must_use]
    pub fn from_output(records: &[Record]) -> Self {
        let exec = records
            .iter()
            .find(|record| record.kind == RecordKind::Exec);
        let mut outcome = Self::new(RunState::Ok, "the action ended");
        outcome.operation = records
            .iter()
            .filter(|record| record.kind == RecordKind::Effect)
            .find_map(|record| int_field(record, "op"));
        outcome.job = records.iter().find_map(|record| match record.kind {
            RecordKind::Job => text_field(record, "id"),
            _ => text_field(record, "job"),
        });
        outcome.output_file = records
            .iter()
            .find_map(|record| text_field(record, "output_file"));
        match exec {
            Some(exec) => {
                let timed_out = matches!(exec.get("timed_out"), Some(Value::Bool(true)));
                let code = int_field(exec, "exit_code").map(|code| code.cast_signed());
                outcome.exit_code = if timed_out {
                    Some(EXIT_TIMED_OUT)
                } else {
                    code
                };
                outcome.state = match (timed_out, code) {
                    (true, _) => RunState::Timeout,
                    (false, Some(EXIT_OK) | None) => RunState::Ok,
                    (false, Some(_)) => RunState::Exit,
                };
                outcome.message = Some(match outcome.exit_code {
                    Some(code) => format!("the program ended with exit code {code}"),
                    None => "the program ended".to_owned(),
                });
                let stdout = text_field(exec, "stdout").unwrap_or_default();
                let stderr = text_field(exec, "stderr").unwrap_or_default();
                outcome.output_sha256 =
                    Some(sha256_hex(&[stdout.as_bytes(), b"\0", stderr.as_bytes()]));
            }
            None => {
                let stable: Vec<serde_json::Value> = records.iter().map(stable_json).collect();
                let bytes = serde_json::to_vec(&stable).unwrap_or_default();
                outcome.output_sha256 = Some(sha256_hex(&[&bytes]));
            }
        }
        outcome
    }

    /// Digest of the state, exit code and output, compared between scheduled
    /// runs to set `changed`.
    #[must_use]
    pub fn digest(&self) -> String {
        let exit = self
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_default();
        sha256_hex(&[
            self.state.as_str().as_bytes(),
            b"\0",
            exit.as_bytes(),
            b"\0",
            self.output_sha256.as_deref().unwrap_or_default().as_bytes(),
        ])
    }
}

/// An action kind other than a pipeline, provided by the embedding
/// application or an extension.
pub trait ScheduleAction: Send + Sync {
    /// Kind name, for example `session_message`.
    fn kind(&self) -> &str;

    /// Checks the arguments given to `schedule.create action_args:`.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALID_VALUE` with what to correct.
    fn validate(&self, args: &serde_json::Value) -> Result<(), Error>;

    /// Short text of the action shown in `schedule.list`.
    fn describe(&self, args: &serde_json::Value) -> String;

    /// Runs the action; returns when it ends or `cancel` is cancelled.
    fn execute(&self, run: &ActionRun, args: &serde_json::Value, cancel: &CancelToken) -> Outcome;
}

/// Local action kinds by name.
#[derive(Clone, Default)]
pub struct ActionRegistry {
    actions: BTreeMap<String, Arc<dyn ScheduleAction>>,
}

impl ActionRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an action kind; replaces an earlier one of the same name.
    pub fn register(&mut self, action: Arc<dyn ScheduleAction>) {
        self.actions.insert(action.kind().to_owned(), action);
    }

    /// The action of `kind`.
    #[must_use]
    pub fn get(&self, kind: &str) -> Option<Arc<dyn ScheduleAction>> {
        self.actions.get(kind).cloned()
    }

    /// Registered kinds in name order.
    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.actions.keys().map(String::as_str)
    }
}

impl fmt::Debug for ActionRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.actions.keys()).finish()
    }
}

/// Integrity hash of a bound pipeline.
///
/// # Errors
///
/// Returns `E_STAGE_FAILED` when the pipeline cannot be serialized.
pub fn pipeline_hash(pipeline: &BoundPipeline) -> Result<String, Error> {
    let json = serde_json::to_value(pipeline)
        .map_err(|error| failed(format!("the action cannot be serialized: {error}")))?;
    canonical_hash(&strip_for_hash(&json))
}

/// Integrity hash of an action of a registered kind.
///
/// # Errors
///
/// Returns `E_STAGE_FAILED` when the arguments cannot be serialized.
pub fn args_hash(kind: &str, args: Option<&serde_json::Value>) -> Result<String, Error> {
    let json = serde_json::json!({ "kind": kind, "args": args });
    canonical_hash(&json)
}

/// Integrity hash computed from the action of a definition.
///
/// # Errors
///
/// Returns `E_INVALID_VALUE` when a pipeline definition holds no pipeline.
pub fn definition_hash(definition: &Definition) -> Result<String, Error> {
    if definition.action_kind == PIPELINE_KIND {
        let pipeline = definition
            .pipeline
            .as_ref()
            .ok_or_else(|| invalid(format!("schedule `{}` has no pipeline", definition.id)))?;
        pipeline_hash(pipeline)
    } else {
        args_hash(&definition.action_kind, definition.action_args.as_ref())
    }
}

/// Program text of a bound pipeline, without the values the binder took
/// from the catalog defaults.
///
/// ```
/// use nurt_contracts::bound::{BoundArg, BoundExpr, BoundHead, BoundPipeline, BoundStage, Phase};
/// use nurt_contracts::kinds::EffectKind;
/// use nurt_sched::action::pipeline_text;
///
/// let stage = BoundStage {
///     name: "run".to_owned(),
///     effect: EffectKind::Process,
///     phase: Phase::Run,
///     waits: false,
///     args: vec![
///         BoundArg::given("command", BoundExpr::List {
///             items: vec![BoundExpr::Text { v: "true".to_owned() }],
///         }),
///         BoundArg::given("cwd", BoundExpr::Path { v: "/tmp".to_owned() }),
///     ],
///     span: None,
/// };
/// let pipeline = BoundPipeline { head: BoundHead::Stage(stage), stages: Vec::new(), span: None };
/// assert_eq!(pipeline_text(&pipeline), r#"run command: ["true"] cwd: "/tmp""#);
/// ```
#[must_use]
pub fn pipeline_text(pipeline: &BoundPipeline) -> String {
    let head = match &pipeline.head {
        BoundHead::Stage(stage) => stage_text(stage),
        BoundHead::Ref(reference) => reference_text(&reference.name, &reference.fields),
        _ => String::new(),
    };
    std::iter::once(head)
        .chain(pipeline.stages.iter().map(stage_text))
        .collect::<Vec<_>>()
        .join(PIPE)
}

fn stage_text(stage: &BoundStage) -> String {
    let mut text = stage.name.clone();
    for arg in stage.args.iter().filter(|arg| shown(arg)) {
        text.push(' ');
        text.push_str(&arg.name);
        text.push_str(": ");
        text.push_str(&expr_text(&arg.value));
    }
    text
}

fn shown(arg: &BoundArg) -> bool {
    !arg.default && !matches!(arg.value, BoundExpr::Null)
}

fn reference_text(name: &str, fields: &[String]) -> String {
    std::iter::once(format!("@{name}"))
        .chain(fields.iter().cloned())
        .collect::<Vec<_>>()
        .join(".")
}

fn expr_text(expr: &BoundExpr) -> String {
    match expr {
        BoundExpr::Null => "null".to_owned(),
        BoundExpr::Bool { v } => v.to_string(),
        BoundExpr::Int { v } => v.to_string(),
        BoundExpr::Num { v } => v.to_string(),
        BoundExpr::Text { v }
        | BoundExpr::Path { v }
        | BoundExpr::Glob { v }
        | BoundExpr::Key { v }
        | BoundExpr::Instant { v } => quoted(v),
        BoundExpr::Enum { v } => v.clone(),
        BoundExpr::Size { v } => format!("{v}B"),
        BoundExpr::Dur { v } => format!("{v}ms"),
        BoundExpr::List { items } => {
            let items: Vec<String> = items.iter().map(expr_text).collect();
            format!("[{}]", items.join(", "))
        }
        BoundExpr::Map { entries } => {
            let entries: Vec<String> = entries
                .iter()
                .map(|entry| format!("{}: {}", entry.k, expr_text(&entry.v)))
                .collect();
            format!("{{{}}}", entries.join(", "))
        }
        BoundExpr::Ref(reference) => reference_text(&reference.name, &reference.fields),
        BoundExpr::Sub { p } => format!("({})", pipeline_text(p)),
        BoundExpr::Field { name, of: None } => name.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""))
}

fn canonical_hash(json: &serde_json::Value) -> Result<String, Error> {
    let canonical = serde_json_canonicalizer::to_vec(json)
        .map_err(|error| failed(format!("the action cannot be serialized: {error}")))?;
    Ok(format!("{HASH_PREFIX}{}", sha256_hex(&[&canonical])))
}

fn sha256_hex(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hex::encode(hasher.finalize())
}

fn int_field(record: &Record, name: &str) -> Option<u64> {
    match record.get(name) {
        Some(Value::Int(number)) => u64::try_from(*number).ok().or(Some(number.cast_unsigned())),
        _ => None,
    }
}

fn text_field(record: &Record, name: &str) -> Option<String> {
    match record.get(name) {
        Some(Value::Text(text) | Value::Path(text)) => Some(text.clone()),
        _ => None,
    }
}

fn stable_json(record: &Record) -> serde_json::Value {
    let mut json = serde_json::to_value(record).unwrap_or_default();
    strip_volatile(&mut json);
    json
}

fn strip_volatile(json: &mut serde_json::Value) {
    match json {
        serde_json::Value::Object(members) => {
            members.retain(|name, _| !VOLATILE_FIELDS.contains(&name.as_str()));
            members.values_mut().for_each(strip_volatile);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(strip_volatile),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec(stdout: &str, exit_code: i64) -> Record {
        Record::new(RecordKind::Exec)
            .with("exit_code", Value::Int(exit_code))
            .with("timed_out", Value::Bool(false))
            .with("stdout", Value::Text(stdout.to_owned()))
            .with("stderr", Value::Text(String::new()))
    }

    #[test]
    fn equal_output_gives_equal_digest() {
        let first = Outcome::from_output(&[exec("done\n", 0)]);
        let second = Outcome::from_output(&[exec("done\n", 0)]);
        let third = Outcome::from_output(&[exec("other\n", 0)]);
        assert_eq!(first.output_sha256, second.output_sha256);
        assert_eq!(first.digest(), second.digest());
        assert_ne!(first.digest(), third.digest());
        assert_eq!(first.state, RunState::Ok);
    }

    #[test]
    fn non_zero_exit_code_is_exit_state() {
        let outcome = Outcome::from_output(&[exec("", 3)]);
        assert_eq!(outcome.state, RunState::Exit);
        assert_eq!(outcome.exit_code, Some(3));
    }

    #[test]
    fn timeout_error_maps_to_timeout_state() {
        let outcome = Outcome::from_error(&Error::new(ErrorCode::Timeout, "time limit"));
        assert_eq!(outcome.state, RunState::Timeout);
        assert_eq!(outcome.exit_code, Some(-1));
        let stopped = Outcome::from_error(&Error::new(ErrorCode::Cancelled, "cancelled"));
        assert_eq!(stopped.state, RunState::Stopped);
    }

    #[test]
    fn args_hash_depends_on_arguments() {
        let a = args_hash("session_message", Some(&serde_json::json!({"text": "a"}))).unwrap();
        let b = args_hash("session_message", Some(&serde_json::json!({"text": "b"}))).unwrap();
        assert!(a.starts_with("sha256:"));
        assert_ne!(a, b);
    }
}
