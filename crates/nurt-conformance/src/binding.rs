// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-conformance::binding
// Responsibility:   Binding cases `nurt-binding-cases/1`.
// Dependencies:     nurt_contracts, serde, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Binding cases `nurt-binding-cases/1`.
//!
//! The cases describe calls of the application interface that every
//! language binding offers, neutral to the language. This module reads them
//! and maps the calls to `nurt-rpc/1`, which the fake server uses to answer
//! the process backend of a binding.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::error::AbiStatus;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::Error;
use crate::pointer;
use crate::protocol::{self, member, method};
use crate::schemas::SchemaSet;

/// Name and version of the format.
pub const FORMAT: &str = "nurt-binding-cases/1";

/// Calls of the application interface.
pub const CALLS: [&str; 18] = [
    "run", "start", "check", "parse", "print", "correct", "page", "describe", "help", "catalog",
    "schema", "history", "space", "info", "close", "wait", "cancel", "events",
];

/// Calls made on an execution returned by `start`.
pub const EXECUTION_CALLS: [&str; 3] = ["wait", "cancel", "events"];

/// File of binding cases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindingFile {
    /// Format name, [`FORMAT`].
    pub format: String,
    /// Cases of the file.
    pub cases: Vec<BindingCase>,
}

/// Binding case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindingCase {
    /// Identifier, unique among the binding cases.
    pub id: String,
    /// Description of the checked behavior.
    pub summary: String,
    /// Class: `mandatory` or `optional:<feature>`.
    #[serde(default = "mandatory")]
    pub class: String,
    /// Conditions under which the case applies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// Backends the case applies to; all when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backends: Vec<String>,
    /// Files created in a temporary directory before the first step.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fs: Vec<Value>,
    /// Calls of the case.
    pub steps: Vec<BindingStep>,
}

fn mandatory() -> String {
    "mandatory".to_owned()
}

/// Call of a binding case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindingStep {
    /// Name of the call, one of [`CALLS`].
    pub call: String,
    /// For calls on an execution: number of the step that called `start`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<usize>,
    /// Named arguments.
    #[serde(default)]
    pub args: Map<String, Value>,
    /// Expected outcome.
    pub expect: BindingExpect,
}

/// Outcome of a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The call returned normally and reported no language error.
    Ok,
    /// The call returned a language error.
    LanguageError,
    /// The call failed with a call error.
    CallError,
}

/// Expected outcome of a call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindingExpect {
    /// Kind of outcome.
    pub outcome: Outcome,
    /// Name of the call status of a call error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Error code of a language error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Expected values at JSON pointers of the result view.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, Value>,
    /// JSON pointers of the result view that must exist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub present: Vec<String>,
    /// JSON pointers of the result view that must not exist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub absent: Vec<String>,
}

/// Shape of the result view of a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// `{"ok", "report", "text", "error", "next_cursor"}`.
    Response,
    /// `{"value": text}`.
    Text,
    /// `{"value": object}`.
    Object,
    /// `{"value": [events]}`.
    Events,
    /// `{}`.
    Empty,
}

/// Returns the result view of a call.
#[must_use]
pub fn view(call: &str) -> View {
    match call {
        "run" | "page" | "wait" => View::Response,
        "print" | "describe" | "help" => View::Text,
        "events" => View::Events,
        "start" | "cancel" | "close" => View::Empty,
        _ => View::Object,
    }
}

/// Reads every binding file `*.json` of a directory, sorted by name.
///
/// # Errors
///
/// Returns an error when a file cannot be read, does not follow the schema,
/// or repeats an identifier.
pub fn load_dir(dir: &Path, schemas: &SchemaSet) -> Result<Vec<(PathBuf, BindingFile)>, Error> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| Error::io(dir, e))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    let mut seen = BTreeSet::new();
    let mut files = Vec::new();
    for path in paths {
        let text = fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let value: Value =
            serde_json::from_str(&text).map_err(|e| Error::format(&path, e.to_string()))?;
        schemas
            .validate("binding-cases", &value)
            .map_err(|errors| Error::format(&path, errors.join("; ")))?;
        let file: BindingFile =
            serde_json::from_value(value).map_err(|e| Error::format(&path, e.to_string()))?;
        for case in &file.cases {
            check_steps(&path, case)?;
            if !seen.insert(case.id.clone()) {
                return Err(Error::format(
                    &path,
                    format!("case {:?} is defined twice", case.id),
                ));
            }
        }
        files.push((path, file));
    }
    Ok(files)
}

fn check_steps(path: &Path, case: &BindingCase) -> Result<(), Error> {
    for (index, step) in case.steps.iter().enumerate() {
        let Some(execution) = step.execution else {
            continue;
        };
        let starts = execution
            .checked_sub(1)
            .and_then(|i| case.steps.get(i))
            .is_some_and(|target| target.call == "start" && execution <= index);
        if !starts {
            return Err(Error::format(
                path,
                format!(
                    "case {:?}, step {}: execution {execution} does not name an earlier start",
                    case.id,
                    index + 1
                ),
            ));
        }
    }
    Ok(())
}

const REQUEST_MEMBERS: [&str; 5] = ["program", "ast", "cursor", "accept", "variant"];

impl BindingStep {
    /// Returns the method and parameters of `nurt-rpc/1` that the process
    /// backend sends for this call, or `None` for calls without a message
    /// of their own (`close`, `wait`, `cancel`, `events`).
    #[must_use]
    pub fn rpc(&self) -> Option<(&'static str, Value)> {
        let args = &self.args;
        let call = match self.call.as_str() {
            "run" | "start" => (method::RUN, self.request()),
            "check" => (method::CHECK, self.request()),
            "parse" => (method::PARSE, with_default(args, member::VARIANT, "a")),
            "correct" => (method::CORRECT, with_default(args, member::VARIANT, "a")),
            "print" => (method::PRINT, with_default(args, member::VARIANT, "a")),
            "describe" => ("describe", self.describe()),
            "page" => (method::PAGE, plain(args)),
            "help" => ("help", plain(args)),
            "catalog" => ("catalog", plain(args)),
            "schema" => (method::SCHEMA, self.schema()),
            "history" => ("history", plain(args)),
            "space" => ("space", plain(args)),
            "info" => (method::INFO, plain(args)),
            _ => return None,
        };
        Some(call)
    }

    /// Parameters of `describe`: the interface argument `layer`, or `name`
    /// when given, is the `subject` of the protocol.
    fn describe(&self) -> Value {
        let mut params = plain(&self.args);
        if let Some(members) = params.as_object_mut() {
            let layer = members.remove("layer");
            let name = members.remove("name");
            let subject = name.or(layer).unwrap_or_else(|| json!("core"));
            members.insert(member::SUBJECT.to_owned(), subject);
        }
        params
    }

    /// Parameters of `schema`: the interface argument `which` is the `name`
    /// of the protocol.
    fn schema(&self) -> Value {
        let mut params = plain(&self.args);
        if let Some(members) = params.as_object_mut()
            && let Some(which) = members.remove("which")
        {
            members.insert(member::NAME.to_owned(), which);
        }
        params
    }

    fn request(&self) -> Value {
        let mut request = Map::new();
        request.insert(
            member::VERSION.to_owned(),
            json!(nurt_contracts::envelope::ENVELOPE_VERSION),
        );
        for name in REQUEST_MEMBERS {
            if let Some(value) = self.args.get(name) {
                request.insert(name.to_owned(), value.clone());
            }
        }
        if let Some(options) = self.args.get(member::OPTIONS) {
            request.insert(member::OPTIONS.to_owned(), options.clone());
        }
        Value::Object(request)
    }
}

/// Arguments of the interface that are not sent to the server.
const LOCAL_ARGS: [&str; 1] = ["raise_on_error"];

fn plain(args: &Map<String, Value>) -> Value {
    Value::Object(
        args.iter()
            .filter(|(name, _)| !LOCAL_ARGS.contains(&name.as_str()))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

fn with_default(args: &Map<String, Value>, name: &str, default: &str) -> Value {
    let mut params = plain(args);
    if let Some(members) = params.as_object_mut() {
        members.entry(name).or_insert_with(|| json!(default));
    }
    params
}

/// Answer of the server that produces the expected result view.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// Result of the call.
    Result(Value),
    /// JSON-RPC error code and data.
    Error(i64, Value),
}

impl BindingExpect {
    /// Builds the answer of `nurt-rpc/1` whose result view meets this
    /// expectation, for the given call and its view.
    #[must_use]
    pub fn answer(&self, call: &str, view: View) -> Answer {
        if self.outcome == Outcome::CallError {
            let status = self.status.as_deref().and_then(protocol::status_by_name);
            return match status {
                Some(status) => Answer::Error(
                    protocol::status_code(status),
                    json!({member::STATUS: status.name()}),
                ),
                None => Answer::Error(
                    protocol::code::INTERNAL_ERROR,
                    json!({member::STATUS: AbiStatus::Internal.name()}),
                ),
            };
        }
        let mut raw = match view {
            View::Response => self.response(),
            _ => json!({}),
        };
        for (view_pointer, value) in &self.fields {
            if let Some(raw_pointer) = raw_pointer(view, view_pointer) {
                pointer::insert(&mut raw, &raw_pointer, value.clone());
            }
        }
        for view_pointer in &self.present {
            if let Some(raw_pointer) = raw_pointer(view, view_pointer)
                && raw.pointer(&raw_pointer).is_none()
            {
                pointer::insert(&mut raw, &raw_pointer, json!("present"));
            }
        }
        if view == View::Text && raw.get(member::TEXT).is_none() {
            raw[member::TEXT] = json!("");
        }
        if let Some(members) = raw.as_object_mut() {
            for (name, default) in required_members(call) {
                members.entry(name).or_insert(default);
            }
        }
        if view == View::Object && self.outcome == Outcome::LanguageError {
            pointer::insert(
                &mut raw,
                "/error",
                json!({"code": self.code, "message": "error"}),
            );
        }
        Answer::Result(raw)
    }

    fn response(&self) -> Value {
        let mut report = json!({"nurt": "1.0", "status": "ok", "kind": "run"});
        if self.outcome == Outcome::LanguageError {
            report["status"] = json!("failed");
            report["error"] = json!({"code": self.code, "message": "error"});
        }
        json!({"nurt": "1.0", "report": report})
    }
}

/// Members that `nurt-rpc/1` requires in the result of the method of a call
/// and that an expectation need not name, with neutral values.
fn required_members(call: &str) -> Vec<(&'static str, Value)> {
    match call {
        "parse" | "check" => vec![(member::DIAGNOSTICS, json!([]))],
        "describe" => vec![("description", json!(""))],
        "history" => vec![("operations", json!([]))],
        "space" => vec![("volumes", json!([]))],
        _ => Vec::new(),
    }
}

fn raw_pointer(view: View, view_pointer: &str) -> Option<String> {
    let segments = pointer::segments(view_pointer);
    let (first, rest) = segments.split_first()?;
    let rest = pointer::join(rest);
    match (view, first.as_str()) {
        (View::Response, "report") => Some(format!("/report{rest}")),
        (View::Response, "error") => Some(format!("/report/error{rest}")),
        (View::Response, "next_cursor") => Some("/report/next_cursor".to_owned()),
        (View::Response, "text") => Some("/rendered/text".to_owned()),
        (View::Text, "value") => Some(format!("/{}", member::TEXT)),
        (View::Object, "value") => Some(rest),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(call: &str, args: Value, expect: Value) -> BindingStep {
        serde_json::from_value(json!({"call": call, "args": args, "expect": expect})).unwrap()
    }

    #[test]
    fn calls_map_to_protocol_messages() {
        let run = step(
            "run",
            json!({"program": "count", "options": {"limit": 5}, "raise_on_error": true}),
            json!({"outcome": "ok"}),
        );
        assert_eq!(
            run.rpc(),
            Some((
                "run",
                json!({"nurt": "1.0", "program": "count", "options": {"limit": 5}})
            ))
        );
        let parse = step(
            "parse",
            json!({"program": "count"}),
            json!({"outcome": "ok"}),
        );
        assert_eq!(
            parse.rpc().unwrap().1,
            json!({"program": "count", "variant": "a"})
        );
        let describe = step("describe", json!({}), json!({"outcome": "ok"}));
        assert_eq!(describe.rpc().unwrap().1, json!({"subject": "core"}));
        let named = step(
            "describe",
            json!({"layer": "core", "name": "find", "language": "en"}),
            json!({"outcome": "ok"}),
        );
        assert_eq!(
            named.rpc().unwrap().1,
            json!({"subject": "find", "language": "en"})
        );
        let schema = step(
            "schema",
            json!({"which": "report"}),
            json!({"outcome": "ok"}),
        );
        assert_eq!(schema.rpc().unwrap(), ("schema", json!({"name": "report"})));
        for call in [
            "check", "start", "correct", "print", "page", "help", "catalog", "history", "space",
            "info",
        ] {
            assert!(
                step(call, json!({}), json!({"outcome": "ok"}))
                    .rpc()
                    .is_some(),
                "{call}"
            );
        }
        for call in ["close", "wait", "cancel", "events"] {
            assert!(
                step(call, json!({}), json!({"outcome": "ok"}))
                    .rpc()
                    .is_none(),
                "{call}"
            );
        }
    }

    #[test]
    fn views_follow_the_calls() {
        assert_eq!(view("wait"), View::Response);
        assert_eq!(view("help"), View::Text);
        assert_eq!(view("events"), View::Events);
        assert_eq!(view("close"), View::Empty);
        assert_eq!(view("catalog"), View::Object);
    }

    #[test]
    fn answers_meet_the_expectations() {
        let expect: BindingExpect = serde_json::from_value(json!({
            "outcome": "language_error", "code": "E_SYNTAX",
            "fields": {"/report/status": "failed", "/text": "error", "/error/at/line": 1, "/ok": false},
            "present": ["/next_cursor", "/report/status"]
        }))
        .unwrap();
        let Answer::Result(raw) = expect.answer("run", View::Response) else {
            panic!("result expected")
        };
        assert_eq!(raw["report"]["error"]["code"], "E_SYNTAX");
        assert_eq!(raw["report"]["error"]["at"]["line"], 1);
        assert_eq!(raw["rendered"]["text"], "error");
        assert_eq!(raw["report"]["next_cursor"], "present");
        assert_eq!(raw["report"]["status"], "failed");

        let text: BindingExpect =
            serde_json::from_value(json!({"outcome": "ok", "fields": {"/value": "count\n"}}))
                .unwrap();
        assert_eq!(
            text.answer("print", View::Text),
            Answer::Result(json!({"text": "count\n"}))
        );
        let empty: BindingExpect = serde_json::from_value(json!({"outcome": "ok"})).unwrap();
        assert_eq!(
            empty.answer("print", View::Text),
            Answer::Result(json!({"text": ""}))
        );

        let object: BindingExpect = serde_json::from_value(json!({
            "outcome": "language_error", "code": "E_SYNTAX", "fields": {"/value/ast": null}
        }))
        .unwrap();
        let Answer::Result(raw) = object.answer("parse", View::Object) else {
            panic!("result expected")
        };
        assert_eq!(
            raw,
            json!({"ast": null, "diagnostics": [], "error": {"code": "E_SYNTAX", "message": "error"}})
        );

        let call: BindingExpect =
            serde_json::from_value(json!({"outcome": "call_error", "status": "NURT_E_ARG"}))
                .unwrap();
        assert_eq!(
            call.answer("catalog", View::Object),
            Answer::Error(-32003, json!({"status": "NURT_E_ARG"}))
        );
        let unknown: BindingExpect =
            serde_json::from_value(json!({"outcome": "call_error", "status": "NURT_X"})).unwrap();
        assert_eq!(
            unknown.answer("catalog", View::Object),
            Answer::Error(-32603, json!({"status": "NURT_E_INTERNAL"}))
        );
    }

    #[test]
    fn files_are_validated_and_executions_checked() {
        let schemas = SchemaSet::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let valid = json!({"format": FORMAT, "cases": [{"id": "a", "summary": "s", "steps": [
            {"call": "start", "args": {"program": "count"}, "expect": {"outcome": "ok"}},
            {"call": "wait", "execution": 1, "expect": {"outcome": "ok"}}
        ]}]});
        fs::write(dir.path().join("a.json"), valid.to_string()).unwrap();
        fs::write(dir.path().join("notes.txt"), "").unwrap();
        let files = load_dir(dir.path(), &schemas).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1.cases[0].class, "mandatory");

        fs::write(dir.path().join("b.json"), valid.to_string()).unwrap();
        assert!(
            load_dir(dir.path(), &schemas)
                .unwrap_err()
                .to_string()
                .contains("twice")
        );
        fs::remove_file(dir.path().join("b.json")).unwrap();

        let wrong = json!({"format": FORMAT, "cases": [{"id": "c", "summary": "s", "steps": [
            {"call": "wait", "execution": 1, "expect": {"outcome": "ok"}}
        ]}]});
        fs::write(dir.path().join("c.json"), wrong.to_string()).unwrap();
        assert!(
            load_dir(dir.path(), &schemas)
                .unwrap_err()
                .to_string()
                .contains("earlier start")
        );
        fs::write(dir.path().join("c.json"), "{").unwrap();
        assert!(load_dir(dir.path(), &schemas).is_err());
        fs::write(
            dir.path().join("c.json"),
            json!({"format": FORMAT, "cases": []}).to_string(),
        )
        .unwrap();
        assert!(load_dir(dir.path(), &schemas).is_err());
        assert!(load_dir(&dir.path().join("none"), &schemas).is_err());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
