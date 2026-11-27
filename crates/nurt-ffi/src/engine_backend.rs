// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Backend serving the C interface from the engine of `nurt-core`.
//!
//! The engine is opened with the clock and the host of the process and with
//! the configuration of the platform files, `NURT_CONFIG` and the `NURT_*`
//! variables; the configuration passed to `nurt_engine_open` is applied at
//! the level of the `NURT_CONFIG` file, after that file.
//!
//! A language error never changes the status of a call with a JSON result:
//! a single document becomes `{"error": ...}`, and for `nurt_parse` and
//! `nurt_check` the first document is `null` and the diagnostics are
//! `{"diagnostics": [], "error": ...}`. Text results (`nurt_print`,
//! `nurt_describe`, `nurt_help`) and the functions without a result have no
//! place for the error: the call fails with the status of its code
//! ([`ErrorCode::abi`]) when the function may return that status, otherwise
//! with `NURT_E_INTERNAL`, and `nurt_last_error` holds the error under
//! `detail.error`.

use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::ast::AstDocument;
use nurt_contracts::env::Clock;
use nurt_contracts::envelope::{Language, Request, RequestOptions, Response};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::HistoryQuery;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::spec::CORE_GROUP;
use nurt_core::config::ConfigSources;
use nurt_core::session::SessionId;
use nurt_core::{CatalogQuery, Engine, EngineConfig, EngineError, EventListener, ExecHandle};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::backend::{Backend, Function, Run, to_json};
use crate::error::{CallError, NurtStatus};
use crate::system::{SystemClock, SystemHost};

/// Topic of `nurt_help` without a topic: the index of topics.
const HELP_INDEX: &str = "";

/// Selection of `nurt_describe`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DescribeQuery {
    #[serde(default)]
    layer: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    language: Option<Language>,
}

/// Engine of `nurt-core` behind a `nurt_engine` handle.
pub(crate) struct EngineBackend {
    engine: Engine,
    clock: Arc<dyn Clock>,
}

impl EngineBackend {
    /// Opens the engine of the process; `config` is `null` or an object
    /// `nurt-config/1`.
    pub(crate) fn open(config: &Value) -> Result<Self, CallError> {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let mut sources = ConfigSources::new();
        if let Value::Object(values) = config {
            sources = sources.with_embedded(values.clone());
        }
        let engine = Engine::open(
            EngineConfig::new(Arc::clone(&clock), Arc::new(SystemHost::detect()))
                .with_sources(sources),
        )
        .map_err(|error| open_failure(&error))?;
        Ok(Self::new(engine, clock))
    }

    /// Serves an open engine whose clock is `clock`.
    pub(crate) fn new(engine: Engine, clock: Arc<dyn Clock>) -> Self {
        Self { engine, clock }
    }

    fn parse(&self, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        let (text, variant) = program_input(Function::Parse, input)?;
        match self.engine.parse(text, Some(variant)) {
            Ok(ast) => Ok(vec![to_json(&ast)?, to_json(&no_diagnostics())?]),
            Err(error) => without_document(&error),
        }
    }

    fn print(&self, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        let variant = variant_input(Function::Print, input)?;
        let ast = AstDocument::deserialize(input.get("ast").unwrap_or(&Value::Null)).map_err(
            |error| {
                CallError::arg(format!(
                    "`ast_json` is not a syntax tree nurt-ast/1: {error}"
                ))
                .with_detail("argument", "ast_json")
            },
        )?;
        self.engine
            .print(&ast, Some(variant))
            .map(|text| vec![text.into_bytes()])
            .map_err(|error| failure(Function::Print, &error))
    }

    fn correct(&self, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        let (text, variant) = program_input(Function::Correct, input)?;
        let result = match self.engine.correct(text, Some(variant)) {
            Ok(corrected) => json!({
                "program": corrected.ran.as_deref().unwrap_or(text),
                "ast": corrected.ast,
                "corrections": corrected.corrections,
            }),
            Err(error) => with_error(&error),
        };
        Ok(vec![to_json(&result)?])
    }

    fn check(&self, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        let request = request_input(Function::Check, input)?;
        match self.engine.check(&request) {
            Ok(report) => Ok(vec![to_json(&report)?, to_json(&no_diagnostics())?]),
            Err(error) => without_document(&error),
        }
    }

    fn page(&self, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        let cursor = input
            .get("cursor")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed(Function::Page))?;
        let options = RequestOptions {
            limit: input
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|limit| u32::try_from(limit).ok())
                .filter(|limit| *limit > 0),
            ..RequestOptions::default()
        };
        Ok(vec![to_json(&self.engine.page(cursor, &options))?])
    }

    fn catalog(&self, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        let query = catalog_query(input)?;
        let result = self
            .engine
            .catalog_query(&query)
            .unwrap_or_else(|error| with_error(&error));
        Ok(vec![to_json(&result)?])
    }

    fn describe(&self, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        let query = if input.is_null() {
            DescribeQuery::default()
        } else {
            DescribeQuery::deserialize(input).map_err(|error| {
                CallError::arg(format!(
                    "`what_json` is not a description query with `layer`, `name` and `language`: {error}"
                ))
                .with_detail("argument", "what_json")
            })?
        };
        let subject = query.name.or(query.layer);
        let description = self
            .engine
            .describe(subject.as_deref().unwrap_or(CORE_GROUP), query.language)
            .map_err(|error| failure(Function::Describe, &error))?;
        let text = match description {
            Value::String(text) => text,
            other => other.to_string(),
        };
        Ok(vec![text.into_bytes()])
    }

    fn help(&self, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        let topic = input.as_str().unwrap_or(HELP_INDEX);
        self.engine
            .help(topic, None)
            .map(|text| vec![text.into_bytes()])
            .map_err(|error| failure(Function::Help, &error))
    }

    fn history(&self, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        let query = history_query(input)?;
        let result = match self.engine.history(&query) {
            Ok(operations) => json!({ "operations": operations }),
            Err(error) => with_error(&error),
        };
        Ok(vec![to_json(&result)?])
    }

    fn space(&self, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        let volume = match input.as_str() {
            None => None,
            Some(text) => match AbsPath::new(text) {
                Ok(path) => Some(path),
                Err(_) => {
                    let error = Error::new(
                        ErrorCode::PathRelative,
                        format!("`volume` must be an absolute path; given `{text}`"),
                    )
                    .with_found(text);
                    return Ok(vec![to_json(&with_error(&error))?]);
                }
            },
        };
        let result = match self.engine.space(volume.as_ref()) {
            Ok(volumes) => json!({ "volumes": volumes }),
            Err(error) => with_error(&error),
        };
        Ok(vec![to_json(&result)?])
    }

    fn session_end(&self, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        let text = text_input(Function::SessionEnd, input)?;
        let id = SessionId::new(text).map_err(|error| failure(Function::SessionEnd, &error))?;
        let now = self.clock.now();
        let sessions = self.engine.sessions();
        if sessions.get(&id, now).is_some() {
            sessions.set_settings(&id, Map::new(), now);
        }
        Ok(Vec::new())
    }

    fn ext_load(input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        let path = text_input(Function::ExtLoad, input)?;
        let error = Error::new(
            ErrorCode::Unsupported,
            "extensions are not available in this build; the engine has no extension host",
        )
        .with_found(path);
        Err(failure(Function::ExtLoad, &error))
    }
}

impl Backend for EngineBackend {
    fn call(&self, function: Function, input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
        match function {
            Function::EngineConfigGet => Ok(vec![to_json(self.engine.config())?]),
            Function::Parse => self.parse(input),
            Function::Print => self.print(input),
            Function::Correct => self.correct(input),
            Function::Check => self.check(input),
            Function::Run => {
                let request = request_input(Function::Run, input)?;
                Ok(vec![to_json(&self.run(&request)?)?])
            }
            Function::Page => self.page(input),
            Function::Catalog => self.catalog(input),
            Function::Describe => self.describe(input),
            Function::Help => self.help(input),
            Function::History => self.history(input),
            Function::Space => self.space(input),
            Function::SessionEnd => self.session_end(input),
            Function::ExtLoad => Self::ext_load(input),
            Function::ExtList => Ok(vec![to_json(&json!({ "extensions": [] }))?]),
        }
    }

    fn run(&self, request: &Request) -> Result<Response, CallError> {
        Ok(self.engine.run(request))
    }

    fn start(
        &self,
        request: &Request,
        listener: Arc<dyn EventListener>,
    ) -> Result<Arc<dyn Run>, CallError> {
        Ok(Arc::new(EngineRun(self.engine.start(request, listener))))
    }
}

/// Execution started by the engine.
struct EngineRun(ExecHandle);

impl Run for EngineRun {
    fn request_id(&self) -> &str {
        self.0.request_id()
    }

    fn cancel(&self) {
        self.0.cancel();
    }

    fn result(&self, timeout: Option<Duration>) -> Option<Result<Response, CallError>> {
        self.0.wait(timeout).map(Ok)
    }
}

fn no_diagnostics() -> Value {
    json!({ "diagnostics": [] })
}

fn with_error(error: &Error) -> Value {
    json!({ "error": error })
}

/// Results of `nurt_parse` and `nurt_check` for a language error.
fn without_document(error: &Error) -> Result<Vec<Vec<u8>>, CallError> {
    Ok(vec![
        b"null".to_vec(),
        to_json(&json!({ "diagnostics": [], "error": error }))?,
    ])
}

/// Failure of a call whose result has no place for the language error
/// `error`.
fn failure(function: Function, error: &Error) -> CallError {
    let allowed: &[NurtStatus] = match function {
        Function::SessionEnd => &[NurtStatus::Arg, NurtStatus::Io],
        Function::ExtLoad => &[NurtStatus::Arg, NurtStatus::Abi, NurtStatus::Io],
        _ => &[NurtStatus::Arg],
    };
    let status = NurtStatus::from(error.code.abi());
    let status = if allowed.contains(&status) {
        status
    } else {
        NurtStatus::Internal
    };
    CallError::new(status, error.message.clone())
        .with_detail("error", serde_json::to_value(error).unwrap_or(Value::Null))
}

fn open_failure(error: &EngineError) -> CallError {
    CallError::new(NurtStatus::from(error.status()), error.message()).with_detail(
        "error",
        serde_json::to_value(error.to_error()).unwrap_or(Value::Null),
    )
}

/// Failure for an input that the exported function did not build as
/// documented by [`Function`].
fn malformed(function: Function) -> CallError {
    CallError::internal(format!("the input of `{}` is not valid", function.name()))
}

fn text_input(function: Function, input: &Value) -> Result<&str, CallError> {
    input.as_str().ok_or_else(|| malformed(function))
}

fn variant_input(function: Function, input: &Value) -> Result<SyntaxVariant, CallError> {
    input
        .get("variant")
        .and_then(|variant| SyntaxVariant::deserialize(variant).ok())
        .ok_or_else(|| malformed(function))
}

fn program_input(function: Function, input: &Value) -> Result<(&str, SyntaxVariant), CallError> {
    let text = input
        .get("program")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed(function))?;
    Ok((text, variant_input(function, input)?))
}

/// Selection of `nurt_catalog`: `null` or an object with `group` or with
/// `command`.
fn catalog_query(input: &Value) -> Result<CatalogQuery<'_>, CallError> {
    let invalid = |message: String| {
        CallError::arg(format!("`what_json` is not a catalog selection: {message}"))
            .with_detail("argument", "what_json")
    };
    let object = match input {
        Value::Null => return Ok(CatalogQuery::Summary),
        Value::Object(object) => object,
        _ => return Err(invalid("expected an object or null".to_owned())),
    };
    if let Some(key) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "group" | "command"))
    {
        return Err(invalid(format!(
            "unknown member `{key}`; expected `group` or `command`"
        )));
    }
    let name = |key: &str| match object.get(key) {
        Some(Value::String(name)) if !name.is_empty() => Ok(Some(name.as_str())),
        Some(_) => Err(invalid(format!("`{key}` must be a non-empty string"))),
        None => Ok(None),
    };
    match (name("group")?, name("command")?) {
        (None, None) => Ok(CatalogQuery::Summary),
        (Some(group), None) => Ok(CatalogQuery::Group(group)),
        (None, Some(command)) => Ok(CatalogQuery::Command(command)),
        (Some(_), Some(_)) => Err(invalid("`group` and `command` are exclusive".to_owned())),
    }
}

/// Query of `nurt_history`: the members given by the caller over the
/// defaults of the command `history`.
fn history_query(input: &Value) -> Result<HistoryQuery, CallError> {
    let invalid = |message: String| CallError::arg(message).with_detail("argument", "query_json");
    let Value::Object(given) = input else {
        return Ok(HistoryQuery::default());
    };
    let mut query = serde_json::to_value(HistoryQuery::default())
        .map_err(|error| CallError::internal(format!("cannot build the query: {error}")))?;
    if let Value::Object(fields) = &mut query {
        for (key, value) in given {
            if !fields.contains_key(key) {
                return Err(invalid(format!(
                    "`query_json` has the unknown member `{key}`; use the parameters of the command `history`"
                )));
            }
            fields.insert(key.clone(), value.clone());
        }
    }
    HistoryQuery::deserialize(&query).map_err(|error| {
        invalid(format!(
            "`query_json` is not a query of the command `history`: {error}"
        ))
    })
}

fn request_input(function: Function, input: &Value) -> Result<Request, CallError> {
    Request::deserialize(input).map_err(|_| malformed(function))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, Mutex};

    use nurt_contracts::ast::AstDocument;
    use nurt_contracts::env::Clock;
    use nurt_contracts::envelope::Request;
    use nurt_contracts::error::{Error, ErrorCode};
    use nurt_core::config::ConfigSources;
    use nurt_core::fixtures::FixtureFrontend;
    use nurt_core::session::SessionId;
    use nurt_core::{DeliveredEvent, Engine, EngineConfig, EventListener};
    use nurt_testkit::clock::VirtualClock;
    use nurt_testkit::fixtures::empty_program;
    use nurt_testkit::host::TestHost;
    use serde_json::{Map, Value, json};

    use super::{EngineBackend, failure};
    use crate::backend::{Backend, Function};
    use crate::error::{CallError, NurtStatus};

    fn ast() -> AstDocument {
        serde_json::from_value(json!({"nurt": "1.0", "program": {"statements": []}})).unwrap()
    }

    fn backend(frontend: FixtureFrontend) -> EngineBackend {
        let clock: Arc<dyn Clock> = Arc::new(VirtualClock::new());
        let engine = Engine::open(
            EngineConfig::new(Arc::clone(&clock), Arc::new(TestHost::linux()))
                .with_sources(ConfigSources::isolated())
                .without_state_dir()
                .with_frontend(Arc::new(frontend)),
        )
        .unwrap();
        EngineBackend::new(engine, clock)
    }

    fn listing() -> EngineBackend {
        backend(FixtureFrontend::new().with_trees("list", ast(), empty_program()))
    }

    fn json_outputs(outputs: &[Vec<u8>]) -> Vec<Value> {
        outputs
            .iter()
            .map(|bytes| serde_json::from_slice(bytes).unwrap())
            .collect()
    }

    fn call(backend: &EngineBackend, function: Function, input: Value) -> Vec<Value> {
        json_outputs(&backend.call(function, &input).unwrap())
    }

    fn failed(backend: &EngineBackend, function: Function, input: Value) -> CallError {
        backend.call(function, &input).unwrap_err()
    }

    fn program(text: &str) -> Value {
        json!({ "program": text, "variant": "a" })
    }

    #[test]
    fn parse_returns_the_tree_or_the_error_in_the_diagnostics() {
        let backend = listing();
        let outputs = call(&backend, Function::Parse, program("list"));
        assert_eq!(outputs[0], serde_json::to_value(ast()).unwrap());
        assert_eq!(outputs[1], json!({"diagnostics": []}));
        let outputs = call(&backend, Function::Parse, program("unknown"));
        assert_eq!(outputs[0], Value::Null);
        assert_eq!(outputs[1]["diagnostics"], json!([]));
        assert_eq!(outputs[1]["error"]["code"], "E_UNSUPPORTED");
    }

    #[test]
    fn print_returns_text_and_fails_with_the_status_of_the_error() {
        let listing = listing();
        let input = json!({"ast": ast(), "variant": "b"});
        let outputs = listing.call(Function::Print, &input).unwrap();
        assert_eq!(outputs, [b"list".to_vec()]);
        let empty = backend(FixtureFrontend::new());
        let error = failed(&empty, Function::Print, input);
        assert_eq!(error.status(), NurtStatus::Arg);
        let invalid = failed(
            &listing,
            Function::Print,
            json!({"ast": [1], "variant": "a"}),
        );
        assert_eq!(invalid.status(), NurtStatus::Arg);
    }

    #[test]
    fn correct_returns_the_program_with_its_corrections() {
        let backend = listing();
        let outputs = call(&backend, Function::Correct, program("list"));
        assert_eq!(outputs[0]["program"], "list");
        assert_eq!(outputs[0]["corrections"], json!([]));
        let outputs = call(&backend, Function::Correct, program("fnd"));
        assert_eq!(outputs[0]["error"]["code"], "E_UNSUPPORTED");
    }

    #[test]
    fn check_and_run_report_language_errors_in_their_results() {
        let backend = listing();
        let request = json!({"nurt": "1.0", "program": "list"});
        let outputs = call(&backend, Function::Check, request.clone());
        assert_eq!(outputs[0]["format"], "nurt-check/1");
        let unknown = json!({"nurt": "1.0", "program": "unknown"});
        let outputs = call(&backend, Function::Check, unknown);
        assert_eq!(outputs[0], Value::Null);
        assert_eq!(outputs[1]["error"]["code"], "E_UNSUPPORTED");
        let outputs = call(&backend, Function::Run, request);
        assert_eq!(outputs[0]["report"]["status"], "failed");
        assert_eq!(outputs[0]["report"]["error"]["code"], "E_UNSUPPORTED");
        let outputs = call(
            &backend,
            Function::Page,
            json!({"cursor": "c_unknown", "limit": 5}),
        );
        assert_eq!(outputs[0]["report"]["status"], "failed");
    }

    /// Checks the members of `document` against the required members and
    /// the simple types of the schema `nurt-check/1`.
    fn assert_check_document(document: &Value) {
        let schema: Value = serde_json::from_str(nurt_contracts::schema::CHECK).unwrap();
        let object = document.as_object().unwrap();
        for name in schema["required"].as_array().unwrap() {
            let name = name.as_str().unwrap();
            assert!(object.contains_key(name), "missing member `{name}`");
        }
        for (name, property) in schema["properties"].as_object().unwrap() {
            let Some(kind) = property["type"].as_str() else {
                continue;
            };
            let member = &object[name];
            let matches = match kind {
                "string" => member.is_string(),
                "integer" => member.is_u64(),
                "boolean" => member.is_boolean(),
                "array" => member.is_array(),
                other => panic!("unexpected type `{other}` of `{name}`"),
            };
            assert!(
                matches,
                "member `{name}` is not of the type {kind}: {member}"
            );
        }
        assert_eq!(object["format"], "nurt-check/1");
    }

    #[test]
    fn check_result_has_the_members_of_the_check_document() {
        let outputs = call(
            &listing(),
            Function::Check,
            json!({"nurt": "1.0", "program": "list"}),
        );
        assert_check_document(&outputs[0]);
        assert!(outputs[0]["bound"].is_object());
        assert_eq!(outputs[1], json!({"diagnostics": []}));
    }

    #[test]
    fn catalog_applies_the_selection_of_a_group_or_a_command() {
        let backend = listing();
        let summary = call(&backend, Function::Catalog, Value::Null);
        assert!(summary[0]["groups"].is_array());
        assert_eq!(call(&backend, Function::Catalog, json!({})), summary);
        let group = summary[0]["groups"][0]["name"].as_str().unwrap().to_owned();
        let listed = call(&backend, Function::Catalog, json!({"group": group}));
        assert_eq!(listed[0]["group"]["name"], json!(group));
        let command = listed[0]["commands"][0]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        let specified = call(&backend, Function::Catalog, json!({"command": command}));
        assert_eq!(specified[0]["command"]["name"], json!(command));
        let unknown = call(
            &backend,
            Function::Catalog,
            json!({"group": "no-such-group"}),
        );
        assert_eq!(unknown[0]["error"]["code"], "E_UNKNOWN_NAME");
        for input in [
            json!("find"),
            json!({"group": 1}),
            json!({"group": "a", "command": "b"}),
            json!({"what": "a"}),
        ] {
            assert_eq!(
                failed(&backend, Function::Catalog, input).status(),
                NurtStatus::Arg
            );
        }
    }

    #[test]
    fn descriptions_and_help_fail_without_a_front_end_that_has_them() {
        let backend = listing();
        for input in [
            Value::Null,
            json!({"layer": "group", "name": "documents", "language": "pl"}),
        ] {
            let error = failed(&backend, Function::Describe, input);
            assert_eq!(error.status(), NurtStatus::Arg);
        }
        let invalid = failed(&backend, Function::Describe, json!({"colour": 1}));
        assert_eq!(invalid.status(), NurtStatus::Arg);
        for topic in [Value::Null, json!("find")] {
            assert_eq!(
                failed(&backend, Function::Help, topic).status(),
                NurtStatus::Arg
            );
        }
    }

    #[test]
    fn journal_queries_report_the_missing_journal_in_the_result() {
        let backend = listing();
        for (function, input) in [
            (Function::History, Value::Null),
            (Function::History, json!({"limit": 5})),
            (Function::Space, Value::Null),
            (Function::Space, json!("/srv")),
        ] {
            let outputs = call(&backend, function, input);
            assert_eq!(outputs[0]["error"]["code"], "E_UNSUPPORTED");
        }
        let outputs = call(&backend, Function::Space, json!("srv"));
        assert_eq!(outputs[0]["error"]["code"], "E_PATH_RELATIVE");
        let outputs = call(&backend, Function::Catalog, Value::Null);
        assert!(outputs[0].get("error").is_none());
        for query in [json!({"colour": 1}), json!({"limit": "five"})] {
            let invalid = failed(&backend, Function::History, query);
            assert_eq!(invalid.status(), NurtStatus::Arg);
        }
    }

    #[test]
    fn ending_a_session_clears_its_settings() {
        let backend = listing();
        let id = SessionId::new("s-1").unwrap();
        let now = backend.clock.now();
        let mut settings = Map::new();
        settings.insert("journal.max_ops".to_owned(), json!(5));
        backend.engine.sessions().set_settings(&id, settings, now);
        assert!(call(&backend, Function::SessionEnd, json!("s-1")).is_empty());
        let session = backend.engine.sessions().get(&id, now).unwrap();
        assert!(session.settings().is_empty());
        assert!(call(&backend, Function::SessionEnd, json!("s-unknown")).is_empty());
        let invalid = failed(&backend, Function::SessionEnd, json!(""));
        assert_eq!(invalid.status(), NurtStatus::Arg);
    }

    #[test]
    fn configuration_and_extensions_are_reported() {
        let backend = listing();
        let outputs = call(&backend, Function::EngineConfigGet, Value::Null);
        assert!(outputs[0]["journal"].is_object());
        let outputs = call(&backend, Function::ExtList, Value::Null);
        assert_eq!(outputs[0], json!({"extensions": []}));
        let error = failed(
            &backend,
            Function::ExtLoad,
            json!("/opt/nurt/ext/sample/nurt-ext.toml"),
        );
        assert_eq!(error.status(), NurtStatus::Arg);
    }

    #[test]
    fn inputs_not_built_by_the_interface_are_internal_failures() {
        let backend = listing();
        for (function, input) in [
            (Function::Parse, Value::Null),
            (Function::Print, json!({"ast": {}})),
            (Function::Check, json!([1])),
            (Function::Run, Value::Null),
            (Function::Page, json!({"limit": 1})),
            (Function::SessionEnd, Value::Null),
            (Function::ExtLoad, json!(1)),
        ] {
            assert_eq!(
                failed(&backend, function, input).status(),
                NurtStatus::Internal,
                "{}",
                function.name()
            );
        }
    }

    #[test]
    fn statuses_without_a_place_in_the_function_become_internal() {
        let io = Error::new(ErrorCode::Io, "cannot read");
        assert_eq!(failure(Function::SessionEnd, &io).status(), NurtStatus::Io);
        assert_eq!(failure(Function::ExtLoad, &io).status(), NurtStatus::Io);
        assert_eq!(failure(Function::Print, &io).status(), NurtStatus::Internal);
        let internal = Error::new(ErrorCode::Internal, "broken");
        assert_eq!(
            failure(Function::Help, &internal).status(),
            NurtStatus::Internal
        );
    }

    #[derive(Default)]
    struct Collect(Mutex<Vec<DeliveredEvent>>);

    impl EventListener for Collect {
        fn deliver(&self, event: DeliveredEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn started_requests_deliver_framed_events_and_their_response() {
        let backend = listing();
        let listener = Arc::new(Collect::default());
        let run = backend
            .start(&Request::program("list"), listener.clone())
            .unwrap();
        let response = run.result(None).unwrap().unwrap();
        assert_eq!(response.report.error.unwrap().code, ErrorCode::Unsupported);
        run.cancel();
        assert!(!run.request_id().is_empty());
        let events = listener.0.lock().unwrap();
        let seqs: Vec<u64> = events.iter().map(|event| event.seq).collect();
        assert_eq!(seqs, [1, 2]);
        assert!(
            events
                .iter()
                .all(|event| event.request_id == run.request_id())
        );
    }

    #[test]
    fn opening_reads_the_configuration_of_the_caller() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let config = json!({"state": {"dir": state.to_string_lossy()}});
        let backend = EngineBackend::open(&config).unwrap();
        assert!(state.is_dir());
        assert!(backend.engine.state_dir().is_some());
        let relative = EngineBackend::open(&json!({"state": {"dir": "state"}}));
        assert_eq!(relative.err().unwrap().status(), NurtStatus::Arg);
        let file = temp.path().join("file");
        fs::write(&file, b"").unwrap();
        let blocked = json!({"state": {"dir": file.join("state").to_string_lossy()}});
        let error = EngineBackend::open(&blocked).err().unwrap();
        assert_eq!(error.status(), NurtStatus::Io);
    }
}
