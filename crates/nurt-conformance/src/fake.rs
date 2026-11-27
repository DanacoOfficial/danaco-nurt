// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-conformance::fake
// Responsibility:   Fake server that answers `nurt-rpc/1` from the cases.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Fake server that answers `nurt-rpc/1` from the cases.
//!
//! A request is matched to a call of a case by its method and its
//! parameters; the answer is built from the expectations of that call. The
//! server serves the tests of the runner and of the language bindings while
//! no implementation is available. A deviation selected with [`Break`] shows
//! that the runner detects a non-conforming server.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::binding::{self, Answer as BindingAnswer, BindingCase, BindingFile, View};
use crate::case::{Case, Class, Condition, FsExpectation, Golden, Plan, Step, Suite};
use crate::compare::ast_hash;
use crate::normalize::{BASE_PLACEHOLDER, HTTP_BASE_PLACEHOLDER, SECOND_VOLUME_PLACEHOLDER};
use crate::pointer::{self, Bindings};
use crate::protocol::{self, code, member, method};
use crate::schemas::SchemaSet;
use crate::snapshot::Snapshot;

/// Environment variable naming the case a server process is started for.
pub const CASE_ENV: &str = "NURT_CONFORMANCE_CASE";

/// Environment variable with the address of the local HTTP server of the
/// case, set by the runner for cases that require `fixture:http`.
pub const HTTP_BASE_ENV: &str = "NURT_CONFORMANCE_CASE_HTTP_BASE";

/// Environment variable with the directory of the case on the second
/// volume, set by the runner for cases that require
/// `fixture:second-volume`.
pub const SECOND_VOLUME_ENV: &str = "NURT_CONFORMANCE_CASE_SECOND_VOLUME";

/// Environment variable naming the directory of cases of `serve --stdio`.
pub const CASES_ENV: &str = "NURT_CONFORMANCE_CASES";

/// Environment variable naming the deviation of `serve --stdio`.
pub const BREAK_ENV: &str = "NURT_CONFORMANCE_BREAK";

/// File created in the working directory by [`Break::Effect`].
pub const EFFECT_FILE: &str = ".nurt-conformance-effect";

/// Directory of the binding cases below the case directory.
pub const BINDINGS_DIR: &str = "bindings";

/// Name of the server in the result of `info`.
const SERVER_NAME: &str = "nurt-conformance fake server";

/// Version of the language served.
const LANGUAGE: &str = "1.0";

/// Deviation of the fake server from the expected behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Break {
    /// `parse` returns a tree with a changed command name.
    Tree,
    /// `check` returns a bound tree with a changed argument.
    Bound,
    /// `check` returns a wrong `ast_hash`.
    Hash,
    /// Language errors carry another code.
    ErrorCode,
    /// `run` reports another status.
    Report,
    /// `schema` returns a document that is not a valid schema.
    Schema,
    /// `check` and `run` create a file in the working directory.
    Effect,
    /// Results are sent with a wrong identifier.
    Protocol,
    /// The process exits after `initialize`.
    Crash,
    /// The server stops answering after `initialize`.
    Hang,
    /// The last expected event of every call is not sent.
    Events,
    /// Results carry the members that the cases require to be absent.
    Absent,
    /// `testkit.clock.advance` is answered as an unknown method, as by an
    /// implementation without a test clock; the cases that call it are
    /// skipped, not failed.
    NoClock,
}

impl Break {
    /// Every deviation.
    pub const ALL: [Break; 13] = [
        Self::Tree,
        Self::Bound,
        Self::Hash,
        Self::ErrorCode,
        Self::Report,
        Self::Schema,
        Self::Effect,
        Self::Protocol,
        Self::Crash,
        Self::Hang,
        Self::Events,
        Self::Absent,
        Self::NoClock,
    ];

    /// Name of the deviation in `--break`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tree => "tree",
            Self::Bound => "bound",
            Self::Hash => "hash",
            Self::ErrorCode => "error-code",
            Self::Report => "report",
            Self::Schema => "schema",
            Self::Effect => "effect",
            Self::Protocol => "protocol",
            Self::Crash => "crash",
            Self::Hang => "hang",
            Self::Events => "events",
            Self::Absent => "absent",
            Self::NoClock => "no-clock",
        }
    }

    /// Returns the deviation with the given name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.name() == name)
    }
}

/// Prepared answer to a request.
#[derive(Debug, Clone)]
struct Answer {
    result: Result<Value, (i64, Value)>,
    effect: Option<Snapshot>,
    second_effect: Option<Snapshot>,
    events: Vec<Value>,
    absent: Vec<String>,
    start: bool,
    hold: bool,
}

impl Answer {
    fn result(value: Value) -> Self {
        Self {
            result: Ok(value),
            effect: None,
            second_effect: None,
            events: Vec::new(),
            absent: Vec::new(),
            start: false,
            hold: false,
        }
    }
}

/// Fake server.
#[derive(Debug)]
pub struct FakeServer {
    answers: BTreeMap<String, Answer>,
    features: BTreeSet<String>,
    schemas: SchemaSet,
    deviation: Option<Break>,
    events: bool,
    hanging: bool,
    held: Vec<(Value, Value)>,
    seq: u64,
    clock: jiff::Timestamp,
    http_base: Option<String>,
    second_volume: Option<PathBuf>,
}

/// Messages to send after a request and whether the server ends.
#[derive(Debug, Default, PartialEq)]
pub struct Reply {
    /// Messages, in order.
    pub messages: Vec<Value>,
    /// Whether the server ends after sending them.
    pub exit: bool,
}

impl FakeServer {
    /// Prepares the answers of the given cases and binding cases.
    ///
    /// With `case_id` only the calls of that case are answered.
    ///
    /// # Errors
    ///
    /// Returns an error when the schemas cannot be compiled.
    pub fn new(
        suite: &Suite,
        bindings: &[BindingFile],
        case_id: Option<&str>,
        deviation: Option<Break>,
    ) -> Result<Self, crate::Error> {
        Ok(Self::prepare(
            SchemaSet::new()?,
            suite,
            bindings,
            case_id,
            deviation,
        ))
    }

    /// Loads the cases below `cases` and the binding cases of its directory
    /// `bindings`, and prepares their answers.
    ///
    /// With `case_id` only that case is loaded and answered: the other case
    /// files are not parsed, which keeps the start of a server process for
    /// one case short.
    ///
    /// # Errors
    ///
    /// Returns an error when the schemas cannot be compiled, or a case file
    /// or a binding case file cannot be read or does not follow its format.
    pub fn load(
        cases: &Path,
        case_id: Option<&str>,
        deviation: Option<Break>,
    ) -> Result<Self, crate::Error> {
        let schemas = SchemaSet::new()?;
        let suite = match case_id {
            Some(id) => Suite::load_case(cases, &schemas, id)?,
            None => Suite::load(cases, &schemas)?,
        };
        let dir = cases.join(BINDINGS_DIR);
        let bindings = if dir.is_dir() {
            binding::load_dir(&dir, &schemas)?
                .into_iter()
                .map(|(_, file)| file)
                .collect()
        } else {
            Vec::new()
        };
        Ok(Self::prepare(
            schemas, &suite, &bindings, case_id, deviation,
        ))
    }

    fn prepare(
        schemas: SchemaSet,
        suite: &Suite,
        bindings: &[BindingFile],
        case_id: Option<&str>,
        deviation: Option<Break>,
    ) -> Self {
        let mut server = Self {
            answers: BTreeMap::new(),
            features: BTreeSet::new(),
            schemas,
            deviation,
            events: false,
            hanging: false,
            held: Vec::new(),
            seq: 0,
            clock: jiff::Timestamp::from_second(CLOCK_START).unwrap_or_default(),
            http_base: None,
            second_volume: None,
        };
        for case in suite
            .cases
            .iter()
            .filter(|c| case_id.is_none_or(|id| c.id == id))
        {
            server.add_case(case);
        }
        let binding_cases = bindings.iter().flat_map(|file| &file.cases);
        for case in binding_cases.filter(|c| case_id.is_none_or(|id| c.id == id)) {
            server.add_binding_case(case);
        }
        server
    }

    /// Sets the address of the local HTTP server and the directory on the
    /// second volume of the case. Both are rewritten to their placeholders
    /// in requests before the requests are matched, and the expected state
    /// of the second volume is created in the directory.
    #[must_use]
    pub fn with_fixtures(
        mut self,
        http_base: Option<String>,
        second_volume: Option<PathBuf>,
    ) -> Self {
        self.http_base = http_base;
        self.second_volume = second_volume;
        self
    }

    /// Number of prepared answers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.answers.len()
    }

    /// Whether no answer is prepared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.answers.is_empty()
    }

    fn add(&mut self, method_name: &str, params: &Value, answer: Answer) {
        self.answers
            .entry(key(method_name, params))
            .or_insert(answer);
    }

    fn add_features(&mut self, class: &Class, requires: &[Condition]) {
        if let Class::Optional(feature) = class {
            self.features.insert(feature.clone());
        }
        for condition in requires {
            if let Condition::Feature(feature) = condition {
                self.features.insert(feature.clone());
            }
        }
    }

    fn add_case(&mut self, case: &Case) {
        self.add_features(&case.class, &case.requires);
        match &case.plan {
            Plan::Steps(steps) => {
                let mut results = Recorded(Vec::new());
                for step in steps {
                    let answer = step_answer(step);
                    let result = answer.result.clone().unwrap_or(Value::Null);
                    if let Ok(params) = pointer::substitute(&step.params, &results) {
                        self.add(&step.method, &params, answer);
                    }
                    results.0.push(result);
                }
            }
            Plan::Golden(golden) => self.add_golden(golden),
        }
    }

    fn add_golden(&mut self, golden: &Golden) {
        let checked = golden.bound.as_ref().map(|bound| {
            let hash = golden
                .bound_hash
                .clone()
                .unwrap_or_else(|| ast_hash(bound).unwrap_or_default());
            Answer::result(json!({
                member::BOUND: bound,
                member::AST_HASH: hash,
                member::DIAGNOSTICS: [],
            }))
        });
        let mut texts: BTreeMap<String, String> = golden.texts.iter().cloned().collect();
        if texts.is_empty() {
            let hash = ast_hash(&golden.ast).unwrap_or_default();
            for variant in protocol::VARIANTS {
                texts.insert(variant.to_owned(), format!("print {variant} {hash}"));
            }
        }
        for (variant, text) in &texts {
            let tree = golden.printed_tree(variant);
            self.add(
                method::PARSE,
                &protocol::text_params(text, variant),
                Answer::result(json!({member::AST: tree, member::DIAGNOSTICS: []})),
            );
            self.add(
                method::PRINT,
                &protocol::print_params(tree, variant),
                Answer::result(json!({member::TEXT: text})),
            );
            if let Some(checked) = &checked {
                let request = protocol::program_request(text, variant, None);
                self.add(method::CHECK, &request, checked.clone());
            }
        }
    }

    fn add_binding_case(&mut self, case: &BindingCase) {
        let class = Class::parse(&case.class).unwrap_or(Class::Mandatory);
        let requires: Vec<Condition> = case
            .requires
            .iter()
            .filter_map(|c| Condition::parse(c))
            .collect();
        self.add_features(&class, &requires);
        let resolved = BindingFields(case);
        for (index, step) in case.steps.iter().enumerate() {
            let Some((method_name, params)) = step.rpc() else {
                continue;
            };
            let Ok(params) = pointer::substitute(&params, &resolved) else {
                continue;
            };
            let number = index + 1;
            let on_execution = |call: &str| {
                case.steps
                    .iter()
                    .find(|s| s.call == call && s.execution == Some(number))
            };
            let answer = if step.call == "start" {
                let expect =
                    on_execution("wait").map(|wait| wait.expect.answer("wait", View::Response));
                Answer {
                    start: true,
                    hold: on_execution("cancel").is_some(),
                    ..binding_answer(
                        expect.unwrap_or_else(|| default_response("wait", View::Response)),
                    )
                }
            } else {
                binding_answer(step.expect.answer(&step.call, binding::view(&step.call)))
            };
            self.add(method_name, &params, answer);
        }
    }

    /// Answers one line of input.
    pub fn handle(&mut self, line: &str) -> Reply {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            let error = protocol::error_response(
                &Value::Null,
                code::PARSE_ERROR,
                "the message is not JSON",
                None,
            );
            return self.reply(vec![error]);
        };
        let Some(id) = message.get("id").cloned() else {
            return Reply::default();
        };
        let Some(method_name) = message.get("method").and_then(Value::as_str) else {
            let error = protocol::error_response(
                &id,
                code::INVALID_REQUEST,
                "the request has no method",
                None,
            );
            return self.reply(vec![error]);
        };
        if self.hanging {
            return Reply::default();
        }
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        match method_name {
            method::INITIALIZE => self.initialize(&id, &params),
            method::INFO => self.reply(vec![protocol::response(&id, self.info())]),
            method::SHUTDOWN => Reply {
                exit: true,
                ..self.reply(vec![protocol::response(&id, Value::Null)])
            },
            method::SCHEMA => self.schema(&id, &params),
            method::CANCEL => self.cancel(&id, &params),
            method::CLOCK_ADVANCE => self.advance_clock(&id, &params),
            _ => self.answer(&id, method_name, &params),
        }
    }

    fn advance_clock(&mut self, id: &Value, params: &Value) -> Reply {
        if self.deviation == Some(Break::NoClock) {
            let error =
                protocol::error_response(id, code::METHOD_NOT_FOUND, "unknown method", None);
            return self.reply(vec![error]);
        }
        let by = params
            .get(member::BY)
            .and_then(Value::as_str)
            .unwrap_or_default();
        let advanced = crate::runner::parse_duration(by)
            .ok()
            .and_then(|duration| jiff::SignedDuration::try_from(duration).ok())
            .and_then(|duration| self.clock.checked_add(duration).ok());
        let message = match advanced {
            Some(now) => {
                self.clock = now;
                protocol::response(id, json!({member::NOW: now.to_string()}))
            }
            None => protocol::error_response(
                id,
                code::INVALID_PARAMS,
                &format!("{by:?} is not a duration such as 2h"),
                None,
            ),
        };
        self.reply(vec![message])
    }

    fn reply(&self, mut messages: Vec<Value>) -> Reply {
        if self.deviation == Some(Break::Protocol) {
            for message in &mut messages {
                if let Some(id) = message.get_mut("id").filter(|id| !id.is_null()) {
                    *id = json!(id.as_u64().map_or(u64::MAX, |n| n + MISMATCH));
                }
            }
        }
        Reply {
            messages,
            exit: false,
        }
    }

    fn info(&self) -> Value {
        json!({
            "name": SERVER_NAME,
            "version": env!("CARGO_PKG_VERSION"),
            "language": LANGUAGE,
            "features": self.features,
        })
    }

    fn initialize(&mut self, id: &Value, params: &Value) -> Reply {
        self.events = params
            .pointer("/capabilities/events")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let result = json!({
            "protocol": protocol::PROTOCOL,
            "version": env!("CARGO_PKG_VERSION"),
            "abi": null,
            "language": LANGUAGE,
            "ast": LANGUAGE,
            "report": LANGUAGE,
            "catalog_rev": null,
            "features": self.features,
            "groups": [],
            "limits": {"max_inflight": 1, "max_message_bytes": MAX_MESSAGE_BYTES},
        });
        let mut reply = self.reply(vec![protocol::response(id, result)]);
        match self.deviation {
            Some(Break::Crash) => reply.exit = true,
            Some(Break::Hang) => self.hanging = true,
            _ => {}
        }
        reply
    }

    fn schema(&self, id: &Value, params: &Value) -> Reply {
        if self.deviation == Some(Break::Schema) {
            return self.reply(vec![protocol::response(id, json!({"type": 12}))]);
        }
        let which = params
            .get(member::NAME)
            .and_then(Value::as_str)
            .unwrap_or_default();
        let message = match self.schemas.document(which) {
            Some(document) => protocol::response(id, document.clone()),
            None => protocol::error_response(
                id,
                code::INVALID_PARAMS,
                &format!("unknown schema {which:?}"),
                None,
            ),
        };
        self.reply(vec![message])
    }

    fn cancel(&mut self, id: &Value, params: &Value) -> Reply {
        let target = params
            .get(member::REQUEST_ID)
            .cloned()
            .unwrap_or(Value::Null);
        let held = self
            .held
            .iter()
            .position(|(held, _)| *held == target)
            .map(|index| self.held.remove(index).1);
        let mut messages = vec![protocol::response(
            id,
            json!({member::CANCELLED: held.is_some()}),
        )];
        messages.extend(held);
        self.reply(messages)
    }

    fn answer(&mut self, id: &Value, method_name: &str, params: &Value) -> Reply {
        let base = params
            .pointer(&format!("/{}/{}", member::OPTIONS, member::BASE))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut normalized = params.clone();
        if let Some(base) = &base {
            replace_base(&mut normalized, base, BASE_PLACEHOLDER);
        }
        if let Some(second) = &self.second_volume {
            let second = second.to_string_lossy();
            replace_base(&mut normalized, &second, SECOND_VOLUME_PLACEHOLDER);
        }
        if let Some(http_base) = &self.http_base {
            replace_base(&mut normalized, http_base, HTTP_BASE_PLACEHOLDER);
        }
        let Some(answer) = self.answers.get(&key(method_name, &normalized)).cloned() else {
            let error = protocol::error_response(
                id,
                code::INVALID_PARAMS,
                "no case matches the request",
                Some(json!({"key": key(method_name, &normalized)})),
            );
            return self.reply(vec![error]);
        };
        if let Some(base) = &base {
            self.apply_effects(method_name, &answer, Path::new(base));
        }
        if let (Some(snapshot), Some(dir)) = (&answer.second_effect, &self.second_volume) {
            apply_snapshot(snapshot, dir);
        }
        let mut events = if answer.start {
            vec![json!({"t": "exec.started"}), json!({"t": "exec.finished"})]
        } else {
            answer.events.clone()
        };
        if self.deviation == Some(Break::Events) {
            events.pop();
        }
        let message = match answer.result {
            Ok(mut result) => {
                if self.deviation == Some(Break::Absent) {
                    insert_absent(&mut result, &answer.absent);
                }
                protocol::response(id, self.deviate(method_name, result))
            }
            Err((code, data)) => protocol::error_response(id, code, "call error", Some(data)),
        };
        let mut messages = Vec::new();
        self.seq = 0;
        if self.events {
            for event in events {
                messages.push(self.event(id, event));
            }
        }
        if answer.hold {
            self.held.push((id.clone(), message));
        } else {
            messages.push(message);
        }
        self.reply(messages)
    }

    /// Completes an event with its sequence number within the request, the
    /// time of the clock, the identifier of the request as text and, for
    /// `exec.finished`, the status `ok` unless the event names one.
    fn event(&mut self, request_id: &Value, mut event: Value) -> Value {
        self.seq += 1;
        if let Value::Object(members) = &mut event {
            members.insert("seq".to_owned(), json!(self.seq));
            members.insert("time".to_owned(), json!(self.clock.to_string()));
            members
                .entry(member::REQUEST)
                .or_insert_with(|| protocol::request_text(request_id));
            if members.get("t") == Some(&json!("exec.finished")) {
                members.entry(member::STATUS).or_insert_with(|| json!("ok"));
            }
        }
        protocol::notification(method::EVENT, event)
    }

    fn apply_effects(&self, method_name: &str, answer: &Answer, base: &Path) {
        if let Some(snapshot) = &answer.effect {
            apply_snapshot(snapshot, base);
        }
        if self.deviation == Some(Break::Effect) && protocol::takes_request(method_name) {
            // A failure to create the file makes the case pass, which the
            // tests of the deviation detect.
            let _ = std::fs::write(base.join(EFFECT_FILE), "");
        }
    }

    fn deviate(&self, method_name: &str, mut result: Value) -> Value {
        match (self.deviation, method_name) {
            (Some(Break::Tree), method::PARSE) => {
                if let Some(tree) = result.get_mut(member::AST) {
                    rename_first_stage(tree);
                }
            }
            (Some(Break::Bound), method::CHECK) => {
                if let Some(tree) = result.get_mut(member::BOUND) {
                    change_first_argument(tree);
                }
            }
            (Some(Break::Hash), method::CHECK) => {
                if let Some(hash) = result.get_mut(member::AST_HASH) {
                    *hash = json!(format!("sha256:{}", "0".repeat(HASH_DIGITS)));
                }
            }
            (Some(Break::ErrorCode), _) => replace_error_codes(&mut result),
            (Some(Break::Report), method::RUN | method::PAGE) => {
                if let Some(status) = result.pointer_mut("/report/status") {
                    let changed = if status == "ok" { "failed" } else { "ok" };
                    *status = json!(changed);
                }
            }
            _ => {}
        }
        result
    }

    /// Answers lines of `input` on `output` until the end of the input or
    /// `shutdown`. Requests held for cancellation are answered at the end
    /// of the input.
    ///
    /// # Errors
    ///
    /// Returns an error when the input cannot be read or the output written.
    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = self.handle(&line);
            for message in &reply.messages {
                writeln!(output, "{message}")?;
            }
            output.flush()?;
            if reply.exit {
                return Ok(());
            }
        }
        for (_, message) in std::mem::take(&mut self.held) {
            writeln!(output, "{message}")?;
        }
        output.flush()
    }
}

/// Offset added to response identifiers by [`Break::Protocol`].
const MISMATCH: u64 = 1000;

/// Number of hexadecimal digits of a SHA-256 digest.
const HASH_DIGITS: usize = 64;

/// Largest message accepted, as announced in `initialize`.
const MAX_MESSAGE_BYTES: u64 = 16 * 1024 * 1024;

/// Start of the clock of the fake server, 2026-01-01T00:00:00Z, in seconds
/// since the Unix epoch. The clock moves only with `testkit.clock.advance`.
const CLOCK_START: i64 = 1_767_225_600;

/// Placeholder of the environment with the given name; the fake server
/// matches requests after rewriting the actual values to placeholders.
fn placeholder(name: &str) -> String {
    format!("${{{name}}}")
}

/// Results of the earlier steps of a case, for placeholders.
struct Recorded(Vec<Value>);

impl Bindings for Recorded {
    fn variable(&self, name: &str) -> Option<String> {
        Some(placeholder(name))
    }

    fn step(&self, number: usize, pointer: &str) -> Option<Value> {
        self.0
            .get(number.checked_sub(1)?)?
            .pointer(pointer)
            .cloned()
    }
}

/// Expected fields of the earlier steps of a binding case, for placeholders.
struct BindingFields<'a>(&'a BindingCase);

impl Bindings for BindingFields<'_> {
    fn variable(&self, name: &str) -> Option<String> {
        Some(placeholder(name))
    }

    fn step(&self, number: usize, pointer: &str) -> Option<Value> {
        let step = self.0.steps.get(number.checked_sub(1)?)?;
        step.expect.fields.get(pointer).cloned()
    }
}

fn key(method_name: &str, params: &Value) -> String {
    let mut params = params.clone();
    if let Some(members) = params.as_object_mut() {
        members.remove(member::VERSION);
        if let Some(Value::Object(options)) = members.get_mut(member::OPTIONS) {
            options.remove(member::BASE);
            if options.is_empty() {
                members.remove(member::OPTIONS);
            }
        }
    }
    let canonical = serde_json_canonicalizer::to_string(&params).unwrap_or_default();
    format!("{method_name} {canonical}")
}

fn step_answer(step: &Step) -> Answer {
    let snapshot_of = |expectation: Option<&FsExpectation>| match expectation {
        Some(FsExpectation::Snapshot(snapshot)) => Some(snapshot.clone()),
        _ => None,
    };
    let answer = Answer {
        effect: snapshot_of(step.fs_after.as_ref()),
        second_effect: snapshot_of(step.second_volume_after.as_ref()),
        events: step
            .events
            .as_ref()
            .map(|expected| expected.events.clone())
            .unwrap_or_default(),
        absent: step.absent.clone(),
        ..Answer::result(Value::Null)
    };
    if let Some(error) = step.expect_error {
        return Answer {
            result: Err((error, Value::Null)),
            ..answer
        };
    }
    let mut result = json!({});
    for expectation in &step.expectations {
        pointer::insert(&mut result, &expectation.at, expectation.value.clone());
    }
    if step.method == method::RUN || step.method == method::PAGE {
        complete_response(&mut result, &step.params);
    }
    for pattern in &step.absent {
        pointer::remove_matches(&mut result, &pointer::segments(pattern));
    }
    Answer {
        result: Ok(result),
        ..answer
    }
}

/// Stores `true` at every pointer without `*` of `absent`.
fn insert_absent(result: &mut Value, absent: &[String]) {
    for pattern in absent {
        let segments = pointer::segments(pattern);
        if !segments.iter().any(|segment| segment == pointer::WILDCARD) {
            pointer::insert(result, pattern, json!(true));
        }
    }
}

/// Creates the entries of a snapshot in `dir`; an exhaustive snapshot
/// replaces the content of the directory.
fn apply_snapshot(snapshot: &Snapshot, dir: &Path) {
    // Failures to create the effect make the case fail in the runner, which
    // compares the directory with the same snapshot.
    if snapshot.exhaustive {
        let _ = clear_directory(dir);
    }
    let _ = snapshot.materialize(dir);
}

fn complete_response(result: &mut Value, params: &Value) {
    let kind = params
        .get(member::PROGRAM)
        .and_then(Value::as_str)
        .and_then(|program| program.split_whitespace().next())
        .unwrap_or(method::RUN)
        .to_owned();
    let defaults =
        json!({"nurt": LANGUAGE, "report": {"nurt": LANGUAGE, "status": "ok", "kind": kind}});
    let mut completed = defaults;
    pointer::merge(&mut completed, result.take());
    *result = completed;
}

fn binding_answer(answer: BindingAnswer) -> Answer {
    match answer {
        BindingAnswer::Result(value) => Answer::result(value),
        BindingAnswer::Error(code, data) => Answer {
            result: Err((code, data)),
            ..Answer::result(Value::Null)
        },
    }
}

fn default_response(call: &str, view: View) -> BindingAnswer {
    let expect = binding::BindingExpect {
        outcome: binding::Outcome::Ok,
        status: None,
        code: None,
        fields: BTreeMap::new(),
        present: Vec::new(),
        absent: Vec::new(),
    };
    expect.answer(call, view)
}

fn clear_directory(dir: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() && !path.is_symlink() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

fn find_object<'a>(
    value: &'a mut Value,
    test: &dyn Fn(&Map<String, Value>) -> bool,
) -> Option<&'a mut Map<String, Value>> {
    if matches!(value, Value::Object(members) if test(members)) {
        return value.as_object_mut();
    }
    match value {
        Value::Object(members) => members.values_mut().find_map(|v| find_object(v, test)),
        Value::Array(items) => items.iter_mut().find_map(|v| find_object(v, test)),
        _ => None,
    }
}

fn rename_first_stage(tree: &mut Value) {
    let is_stage =
        |m: &Map<String, Value>| m.get("t") == Some(&json!("stage")) && m.contains_key("name");
    if let Some(stage) = find_object(tree, &is_stage)
        && let Some(Value::String(name)) = stage.get_mut("name")
    {
        name.push('x');
    }
}

/// Replaces every occurrence of `base`, in its native form and with `/`
/// separators, by `placeholder` in every text of `value`.
fn replace_base(value: &mut Value, base: &str, placeholder: &str) {
    let trimmed = base.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return;
    }
    let forms = [trimmed.to_owned(), trimmed.replace('\\', "/")];
    match value {
        Value::String(text) => {
            for form in &forms {
                if text.contains(form.as_str()) {
                    *text = text.replace(form.as_str(), placeholder);
                }
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| replace_base(item, base, placeholder)),
        Value::Object(members) => members
            .values_mut()
            .for_each(|member| replace_base(member, base, placeholder)),
        _ => {}
    }
}

fn change_first_argument(tree: &mut Value) {
    let has_args = |m: &Map<String, Value>| {
        m.get("args")
            .and_then(Value::as_array)
            .is_some_and(|args| !args.is_empty())
    };
    if let Some(stage) = find_object(tree, &has_args)
        && let Some(Value::Array(args)) = stage.get_mut("args")
        && let Some(Value::Object(first)) = args.first_mut()
    {
        first.insert("value".to_owned(), json!({"t": "null"}));
        first.remove("default");
        return;
    }
    rename_first_stage(tree);
}

fn replace_error_codes(value: &mut Value) {
    match value {
        Value::Object(members) => {
            for (name, member) in members.iter_mut() {
                match member {
                    Value::String(code) if name == "code" && code.starts_with("E_") => {
                        let replaced = if code == "E_INTERNAL" {
                            "E_IO"
                        } else {
                            "E_INTERNAL"
                        };
                        *code = replaced.to_owned();
                    }
                    other => replace_error_codes(other),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(replace_error_codes),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::case::{Expectation, Kind};
    use crate::compare::Match;

    fn case(id: &str, plan: Plan) -> Case {
        Case {
            id: id.to_owned(),
            kind: Kind::Exec,
            class: Class::Optional("testkit".to_owned()),
            language: "1.0".to_owned(),
            requires: vec![Condition::Feature("events".to_owned())],
            summary: "s".to_owned(),
            variant: "a".to_owned(),
            code: None,
            volatile: Vec::new(),
            config: None,
            fs_before: None,
            second_volume_before: None,
            http_files: None,
            plan,
            source: "case.toml".into(),
        }
    }

    fn step(method_name: &str, params: Value, expect: Option<Value>) -> Step {
        Step {
            method: method_name.to_owned(),
            params,
            expectations: expect
                .into_iter()
                .map(|value| Expectation {
                    at: String::new(),
                    value,
                    matching: Match::Subset,
                })
                .collect(),
            ..Step::default()
        }
    }

    #[test]
    fn the_clock_moves_only_when_advanced() {
        let run = Step {
            events: Some(crate::case::EventExpectation {
                events: vec![json!({"t": "exec.started"})],
                order: crate::case::EventOrder::Ordered,
            }),
            ..step("run", json!({"nurt": "1.0", "program": "count"}), None)
        };
        let mut fake = server(vec![case("journal/clock", Plan::Steps(vec![run]))], None);
        fake.handle(&request(
            1,
            "initialize",
            protocol::initialize_params("t", "1", true),
        ));
        let advanced = fake.handle(&request(2, "testkit.clock.advance", json!({"by": "2h"})));
        assert_eq!(result_of(&advanced)["now"], "2026-01-01T02:00:00Z");
        let again = fake.handle(&request(3, "testkit.clock.advance", json!({"by": "30m"})));
        assert_eq!(result_of(&again)["now"], "2026-01-01T02:30:00Z");
        let invalid = fake.handle(&request(4, "testkit.clock.advance", json!({"by": "soon"})));
        assert_eq!(invalid.messages[0]["error"]["code"], code::INVALID_PARAMS);
        let ran = fake.handle(&request(
            5,
            "run",
            json!({"nurt": "1.0", "program": "count"}),
        ));
        assert_eq!(ran.messages[0]["params"]["time"], "2026-01-01T02:30:00Z");
        assert_eq!(ran.messages[0]["params"]["request"], "5");

        let mut fake = server(Vec::new(), Some(Break::NoClock));
        let missing = fake.handle(&request(1, "testkit.clock.advance", json!({"by": "2h"})));
        assert_eq!(missing.messages[0]["error"]["code"], code::METHOD_NOT_FOUND);
    }

    #[test]
    fn expected_events_are_sent_before_the_response() {
        let started = json!({"t": "exec.started", "request": "r"});
        let finished = json!({"t": "exec.finished", "status": "ok"});
        let run = Step {
            events: Some(crate::case::EventExpectation {
                events: vec![started, finished],
                order: crate::case::EventOrder::Exact,
            }),
            ..step("run", json!({"nurt": "1.0", "program": "count"}), None)
        };
        let cases = vec![case("exec/events", Plan::Steps(vec![run]))];
        let call = request(2, "run", json!({"nurt": "1.0", "program": "count"}));
        let mut fake = server(cases.clone(), None);
        fake.handle(&request(
            1,
            "initialize",
            protocol::initialize_params("t", "1", true),
        ));
        let reply = fake.handle(&call);
        let schemas = SchemaSet::shared().unwrap();
        for message in &reply.messages {
            assert_eq!(schemas.validate_message(message), Ok(()), "{message}");
        }
        let methods: Vec<&Value> = reply.messages.iter().map(|m| &m["method"]).collect();
        assert_eq!(
            methods,
            [&json!("nurt/event"), &json!("nurt/event"), &Value::Null]
        );
        assert_eq!(reply.messages[0]["params"]["request"], "r");
        assert_eq!(reply.messages[1]["params"]["seq"], 2);
        assert_eq!(reply.messages[1]["params"]["status"], "ok");

        let mut silent = server(cases.clone(), None);
        silent.handle(&request(
            1,
            "initialize",
            protocol::initialize_params("t", "1", false),
        ));
        assert_eq!(silent.handle(&call).messages.len(), 1);

        let mut broken = server(cases, Some(Break::Events));
        broken.handle(&request(
            1,
            "initialize",
            protocol::initialize_params("t", "1", true),
        ));
        let reply = broken.handle(&call);
        assert_eq!(reply.messages.len(), 2);
        assert_eq!(reply.messages[0]["params"]["t"], "exec.started");
    }

    #[test]
    fn absent_members_are_left_out_of_the_answers() {
        let run = Step {
            absent: vec![
                "/report/kind".to_owned(),
                "/report/changes/*/rows_changed".to_owned(),
            ],
            ..step(
                "run",
                json!({"nurt": "1.0", "program": "count"}),
                Some(json!({"report": {"changes": [{"path": "a", "rows_changed": 1}]}})),
            )
        };
        let cases = vec![case("exec/absent", Plan::Steps(vec![run]))];
        let call = request(2, "run", json!({"nurt": "1.0", "program": "count"}));
        let mut fake = server(cases.clone(), None);
        let report = result_of(&fake.handle(&call))["report"].clone();
        assert_eq!(report.get("kind"), None);
        assert_eq!(report["changes"], json!([{"path": "a"}]));
        let mut broken = server(cases, Some(Break::Absent));
        let report = result_of(&broken.handle(&call))["report"].clone();
        assert_eq!(report["kind"], true);
        assert_eq!(report["changes"], json!([{"path": "a"}]));
    }

    #[test]
    fn fixture_addresses_are_matched_and_the_second_volume_receives_effects() {
        let mut run = step(
            "run",
            json!({"nurt": "1.0", "program": "copy \"${http_base}/files/a\" \"${second_volume}/a\""}),
            Some(json!({"report": {"target": "${second_volume}/a"}})),
        );
        run.second_volume_after = Some(FsExpectation::Snapshot(
            Snapshot::from_json(
                &json!({"entries": [{"path": "a", "type": "file", "content_utf8": "a"}]}),
            )
            .unwrap(),
        ));
        let volume = tempfile::tempdir().unwrap();
        std::fs::write(volume.path().join("old"), "o").unwrap();
        let http = "http://127.0.0.1:4555";
        let mut fake = server(vec![case("exec/copy", Plan::Steps(vec![run]))], None)
            .with_fixtures(Some(http.to_owned()), Some(volume.path().to_path_buf()));
        let volume_text = volume.path().to_string_lossy().into_owned();
        let reply = fake.handle(&request(
            2,
            "run",
            json!({"nurt": "1.0", "program": format!("copy \"{http}/files/a\" \"{volume_text}/a\"")}),
        ));
        assert_eq!(result_of(&reply)["report"]["target"], "${second_volume}/a");
        assert_eq!(
            std::fs::read_to_string(volume.path().join("a")).unwrap(),
            "a"
        );
        assert!(!volume.path().join("old").exists());
    }

    fn suite(cases: Vec<Case>) -> Suite {
        Suite {
            root: ".".into(),
            version: "1.0.0".to_owned(),
            digest: String::new(),
            cases,
        }
    }

    fn request(id: u64, method_name: &str, params: Value) -> String {
        protocol::request(id, method_name, &params).to_string()
    }

    fn server(cases: Vec<Case>, deviation: Option<Break>) -> FakeServer {
        FakeServer::new(&suite(cases), &[], None, deviation).unwrap()
    }

    fn result_of(reply: &Reply) -> &Value {
        &reply.messages.last().unwrap()["result"]
    }

    #[test]
    fn break_names_round_trip() {
        for deviation in Break::ALL {
            assert_eq!(Break::from_name(deviation.name()), Some(deviation));
        }
        assert_eq!(Break::from_name("none"), None);
    }

    #[test]
    fn native_answers_follow_the_protocol_schema() {
        let schemas = SchemaSet::shared().unwrap();
        let mut fake = server(Vec::new(), None);
        let calls = [
            ("initialize", protocol::initialize_params("t", "1", true)),
            ("info", json!({})),
            ("schema", json!({"name": "rpc"})),
            ("cancel", json!({"request_id": 9})),
            ("shutdown", json!({})),
        ];
        for (id, (name, params)) in calls.into_iter().enumerate() {
            let reply = fake.handle(&request(id as u64 + 1, name, params));
            for message in &reply.messages {
                assert_eq!(schemas.validate_message(message), Ok(()), "{name}");
            }
            assert_eq!(
                schemas.validate_result(name, result_of(&reply)),
                Ok(()),
                "{name}"
            );
        }
    }

    #[test]
    fn native_methods_answer_without_cases() {
        let mut fake = server(Vec::new(), None);
        assert!(fake.is_empty());
        let init = fake.handle(&request(
            1,
            "initialize",
            protocol::initialize_params("t", "1", true),
        ));
        assert_eq!(result_of(&init)["protocol"], "nurt-rpc/1");
        assert_eq!(
            result_of(&fake.handle(&request(2, "info", json!({}))))["name"],
            SERVER_NAME
        );
        let schema = fake.handle(&request(3, "schema", json!({"name": "report"})));
        assert_eq!(result_of(&schema)["title"], "nurt-report/1");
        let unknown = fake.handle(&request(4, "schema", json!({"name": "none"})));
        assert_eq!(unknown.messages[0]["error"]["code"], code::INVALID_PARAMS);
        let unmatched = fake.handle(&request(5, "run", json!({"nurt": "1.0", "program": "x"})));
        assert_eq!(
            unmatched.messages[0]["error"]["message"],
            "no case matches the request"
        );
        assert_eq!(
            fake.handle(&request(6, "cancel", json!({"request_id": 9})))
                .messages
                .len(),
            1
        );
        assert_eq!(
            fake.handle("{").messages[0]["error"]["code"],
            code::PARSE_ERROR
        );
        assert_eq!(
            fake.handle(r#"{"id": 7}"#).messages[0]["error"]["code"],
            code::INVALID_REQUEST
        );
        assert_eq!(fake.handle(r#"{"method": "nurt/event"}"#), Reply::default());
        let shutdown = fake.handle(&request(8, "shutdown", json!({})));
        assert!(shutdown.exit);
    }

    #[test]
    fn steps_are_answered_from_expectations_with_placeholders_and_base() {
        let first = step(
            "run",
            json!({"nurt": "1.0", "program": "find \"${base}/a\""}),
            Some(json!({"report": {"next_cursor": "c1", "error": {"code": "E_NOT_FOUND"}}})),
        );
        let second = step(
            "page",
            json!({"cursor": "${step1/report/next_cursor}"}),
            Some(json!({})),
        );
        let mut third = step("check", json!({"nurt": "1.0", "program": "count"}), None);
        third.expect_error = Some(-32602);
        let mut fake = server(
            vec![case("exec/a", Plan::Steps(vec![first, second, third]))],
            None,
        );
        assert_eq!(fake.len(), 3);
        let init = fake.handle(&request(1, "initialize", json!({})));
        assert_eq!(result_of(&init)["features"], json!(["events", "testkit"]));
        let base = tempfile::tempdir().unwrap();
        let base_text = base.path().to_string_lossy().replace('\\', "/");
        let run = fake.handle(&request(
            2,
            "run",
            json!({"nurt": "1.0", "program": format!("find \"{base_text}/a\""), "options": {"base": base_text}}),
        ));
        let report = &result_of(&run)["report"];
        assert_eq!(report["kind"], "find");
        assert_eq!(report["status"], "ok");
        let page = fake.handle(&request(3, "page", json!({"cursor": "c1"})));
        assert_eq!(result_of(&page)["report"]["kind"], "run");
        let check = fake.handle(&request(
            4,
            "check",
            json!({"nurt": "1.0", "program": "count"}),
        ));
        assert_eq!(check.messages[0]["error"]["code"], -32602);
    }

    #[test]
    fn deviations_change_the_answers() {
        let tree = json!({"nurt": "1.0", "program": {"statements": [{"t": "pipeline",
            "head": {"t": "stage", "name": "count", "args": [{"name": "x", "value": {"t": "int", "v": 1}, "default": true}]},
            "stages": []}]}});
        let c_tree = json!({"nurt": "1.0", "program": {"statements": []}});
        let golden = Golden {
            texts: vec![
                ("a".to_owned(), "count".to_owned()),
                ("c".to_owned(), "count named".to_owned()),
            ],
            ast: tree.clone(),
            ast_hash: None,
            bound: Some(tree.clone()),
            bound_hash: None,
            c_ast: Some(c_tree.clone()),
            c_ast_hash: None,
        };
        let run = step(
            "run",
            json!({"nurt": "1.0", "program": "count"}),
            Some(json!({"report": {"error": {"code": "E_INTERNAL"}}})),
        );
        let cases = vec![
            case("golden/a", Plan::Golden(Box::new(golden))),
            case("exec/b", Plan::Steps(vec![run])),
        ];
        let parse = request(2, "parse", json!({"program": "count", "variant": "a"}));
        let check = request(3, "check", json!({"nurt": "1.0", "program": "count"}));
        let run = request(4, "run", json!({"nurt": "1.0", "program": "count"}));

        let mut fake = server(cases.clone(), None);
        let parse_c = request(
            7,
            "parse",
            json!({"program": "count named", "variant": "c"}),
        );
        assert_eq!(result_of(&fake.handle(&parse_c))["ast"], c_tree);
        let print_c = request(8, "print", json!({"ast": c_tree, "variant": "c"}));
        assert_eq!(result_of(&fake.handle(&print_c))["text"], "count named");

        let mut fake = server(cases.clone(), Some(Break::Tree));
        let reply = fake.handle(&parse);
        let name = &result_of(&reply)["ast"]["program"]["statements"][0]["head"]["name"];
        assert_eq!(name, "countx");
        let mut fake = server(cases.clone(), Some(Break::Bound));
        let reply = fake.handle(&check);
        let arg = &result_of(&reply)["bound"]["program"]["statements"][0]["head"]["args"][0];
        assert_eq!(arg, &json!({"name": "x", "value": {"t": "null"}}));
        let mut fake = server(cases.clone(), Some(Break::Hash));
        assert_eq!(
            result_of(&fake.handle(&check))["ast_hash"],
            format!("sha256:{}", "0".repeat(64))
        );
        let mut fake = server(cases.clone(), Some(Break::ErrorCode));
        assert_eq!(
            result_of(&fake.handle(&run))["report"]["error"]["code"],
            "E_IO"
        );
        let mut fake = server(cases.clone(), Some(Break::Report));
        assert_eq!(result_of(&fake.handle(&run))["report"]["status"], "failed");
        let mut fake = server(cases.clone(), Some(Break::Schema));
        assert_eq!(
            result_of(&fake.handle(&request(5, "schema", json!({"name": "ast"})))),
            &json!({"type": 12})
        );
        let mut fake = server(cases.clone(), Some(Break::Protocol));
        assert_eq!(fake.handle(&parse).messages[0]["id"], 1002);
        let mut fake = server(cases.clone(), Some(Break::Crash));
        assert!(fake.handle(&request(1, "initialize", json!({}))).exit);
        let mut fake = server(cases.clone(), Some(Break::Hang));
        fake.handle(&request(1, "initialize", json!({})));
        assert_eq!(fake.handle(&parse), Reply::default());

        let base = tempfile::tempdir().unwrap();
        let mut fake = server(cases, Some(Break::Effect));
        let base_text = base.path().to_string_lossy().into_owned();
        fake.handle(&request(
            6,
            "check",
            json!({"nurt": "1.0", "program": "count", "options": {"base": base_text}}),
        ));
        assert!(base.path().join(EFFECT_FILE).exists());
    }

    #[test]
    fn helpers_cover_trees_without_arguments() {
        let mut tree = json!({"t": "stage", "name": "count", "args": []});
        change_first_argument(&mut tree);
        assert_eq!(tree["name"], "countx");
        let mut codes = json!([{"code": "E_IO"}, {"code": 3}]);
        replace_error_codes(&mut codes);
        assert_eq!(codes, json!([{"code": "E_INTERNAL"}, {"code": 3}]));
    }

    /// Schema generated from the response type of the contracts.
    const RESPONSE_SCHEMA: &str =
        include_str!("../../../spec/golden/request/schema/response.generated.json");

    #[test]
    fn answers_with_the_report_members_of_the_contracts_are_valid_responses() {
        let report = json!({
            "status": "failed",
            "changes": [{"path": "/w/data.db", "action": "modify", "rows_changed": 3}],
            "notable": [
                {"code": "tls_verification_disabled", "severity": "warning"},
                {"code": "link_loop_skipped", "severity": "notice", "count": 1},
                {"code": "pack_without_deltas", "severity": "info"}
            ],
            "records": {"type": "git_trash_entry", "rows": []},
            "error": {"code": "E_OUT_OF_RANGE", "message": "line 7 is after the end", "at": {"stage": "2"}}
        });
        let git =
            json!({"status": "failed", "error": {"code": "E_GIT_CONFIG", "message": "bad value"}});
        let cases = vec![
            case(
                "errors/range",
                Plan::Steps(vec![step(
                    "run",
                    json!({"nurt": "1.0", "program": "read"}),
                    Some(json!({"report": report})),
                )]),
            ),
            case(
                "errors/git",
                Plan::Steps(vec![step(
                    "run",
                    json!({"nurt": "1.0", "program": "git.status"}),
                    Some(json!({"report": git})),
                )]),
            ),
        ];
        let response: Value = serde_json::from_str(RESPONSE_SCHEMA).unwrap();
        let response = jsonschema::validator_for(&response).unwrap();
        let schemas = SchemaSet::new().unwrap();
        for deviation in [None, Some(Break::ErrorCode)] {
            let mut fake = server(cases.clone(), deviation);
            for (id, program) in [(2, "read"), (3, "git.status")] {
                let reply = fake.handle(&request(
                    id,
                    "run",
                    json!({"nurt": "1.0", "program": program}),
                ));
                let result = result_of(&reply);
                let errors: Vec<String> = response
                    .iter_errors(result)
                    .map(|e| e.to_string())
                    .collect();
                assert!(errors.is_empty(), "{program}: {errors:?}");
                assert_eq!(schemas.validate("report", &result["report"]), Ok(()));
                assert_eq!(result["report"]["kind"], program);
            }
        }
        let mut fake = server(cases, None);
        let reply = fake.handle(&request(
            4,
            "run",
            json!({"nurt": "1.0", "program": "read"}),
        ));
        assert_eq!(result_of(&reply)["report"]["changes"][0]["rows_changed"], 3);
        assert_eq!(
            result_of(&reply)["report"]["error"]["code"],
            "E_OUT_OF_RANGE"
        );
    }

    #[test]
    fn a_server_for_one_case_loads_and_answers_only_that_case() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        std::fs::write(r.join(crate::VERSION_FILE), "1.0.0\n").unwrap();
        std::fs::write(
            r.join("all.cases.toml"),
            "format = \"nurt-conformance-cases/1\"\n\n\
             [[case]]\nid = \"exec/a\"\nkind = \"exec\"\nclass = \"optional:testkit\"\nsummary = \"s\"\nprogram = \"count\"\nexpect = {}\n\n\
             [[case]]\nid = \"exec/b\"\nkind = \"exec\"\nsummary = \"s\"\nprogram = \"find\"\nexpect = {}\n",
        )
        .unwrap();
        std::fs::create_dir(r.join("broken")).unwrap();
        std::fs::write(r.join("broken").join("case.toml"), "id = [").unwrap();
        std::fs::create_dir(r.join(BINDINGS_DIR)).unwrap();
        std::fs::write(
            r.join(BINDINGS_DIR).join("api.json"),
            r#"{"format": "nurt-binding-cases/1", "cases": [{"id": "run.plain", "summary": "s", "steps": [
                {"call": "run", "args": {"program": "sort"}, "expect": {"outcome": "ok"}}]}]}"#,
        )
        .unwrap();

        let mut one = FakeServer::load(r, Some("exec/a"), None).unwrap();
        assert_eq!(one.len(), 1);
        let init = one.handle(&request(1, "initialize", json!({})));
        assert_eq!(result_of(&init)["features"], json!(["testkit"]));
        let find = one.handle(&request(
            2,
            "run",
            json!({"nurt": "1.0", "program": "find"}),
        ));
        assert_eq!(
            find.messages[0]["error"]["message"],
            "no case matches the request"
        );
        let binding = FakeServer::load(r, Some("run.plain"), None).unwrap();
        assert_eq!(binding.len(), 1);
        assert!(
            FakeServer::load(r, Some("exec/none"), None)
                .unwrap()
                .is_empty()
        );

        let error = FakeServer::load(r, None, None).unwrap_err();
        assert!(error.to_string().contains("broken"), "{error}");
        std::fs::remove_dir_all(r.join("broken")).unwrap();
        assert_eq!(FakeServer::load(r, None, None).unwrap().len(), 3);
        std::fs::remove_dir_all(r.join(BINDINGS_DIR)).unwrap();
        assert_eq!(FakeServer::load(r, None, None).unwrap().len(), 2);
    }

    #[test]
    fn file_system_effects_and_serving_lines() {
        let snapshot = Snapshot::from_json(
            &json!({"entries": [{"path": "new.txt", "type": "file", "content_utf8": "n"}]}),
        )
        .unwrap();
        let mut run = step("run", json!({"nurt": "1.0", "program": "write"}), None);
        run.fs_after = Some(FsExpectation::Snapshot(snapshot));
        let mut fake = server(vec![case("exec/w", Plan::Steps(vec![run]))], None);
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir(base.path().join("old")).unwrap();
        std::fs::write(base.path().join("old.txt"), "o").unwrap();
        let base_text = base.path().to_string_lossy().into_owned();
        let input = format!(
            "{}\n\n{}\n{}\n",
            request(1, "initialize", json!({})),
            request(
                2,
                "run",
                json!({"nurt": "1.0", "program": "write", "options": {"base": base_text}})
            ),
            request(3, "shutdown", json!({})),
        );
        let mut output = Vec::new();
        fake.serve(input.as_bytes(), &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap().lines().count(), 3);
        assert!(base.path().join("new.txt").exists());
        assert!(!base.path().join("old.txt").exists());
        assert!(!base.path().join("old").exists());
    }

    #[test]
    fn binding_cases_start_hold_cancel_and_emit_events() {
        let file: BindingFile = serde_json::from_value(json!({
            "format": "nurt-binding-cases/1",
            "cases": [{"id": "start.cancel", "summary": "s", "class": "optional:events", "requires": ["feature:jobs"], "steps": [
                {"call": "start", "args": {"program": "sleep"}, "expect": {"outcome": "ok"}},
                {"call": "cancel", "execution": 1, "expect": {"outcome": "ok"}},
                {"call": "wait", "execution": 1, "expect": {"outcome": "language_error", "code": "E_CANCELLED"}},
                {"call": "events", "execution": 1, "expect": {"outcome": "ok"}},
                {"call": "page", "args": {"cursor": "${step3/next_cursor}"}, "expect": {"outcome": "ok"}}
            ]}, {"id": "start.plain", "summary": "s", "steps": [
                {"call": "start", "args": {"program": "count"}, "expect": {"outcome": "ok"}},
                {"call": "close", "expect": {"outcome": "ok"}}
            ]}]
        }))
        .unwrap();
        let mut fake = FakeServer::new(&suite(Vec::new()), &[file], None, None).unwrap();
        assert_eq!(fake.len(), 2);
        fake.handle(&request(
            1,
            "initialize",
            protocol::initialize_params("t", "1", true),
        ));
        let started = fake.handle(&request(
            2,
            "run",
            json!({"nurt": "1.0", "program": "sleep"}),
        ));
        let kinds: Vec<&Value> = started.messages.iter().map(|m| &m["params"]["t"]).collect();
        assert_eq!(kinds, [&json!("exec.started"), &json!("exec.finished")]);
        let cancelled = fake.handle(&request(3, "cancel", json!({"request_id": 2})));
        assert_eq!(cancelled.messages.len(), 2);
        assert_eq!(
            cancelled.messages[1]["result"]["report"]["error"]["code"],
            "E_CANCELLED"
        );

        let plain = fake.handle(&request(
            4,
            "run",
            json!({"nurt": "1.0", "program": "count"}),
        ));
        assert_eq!(
            plain.messages.last().unwrap()["result"]["report"]["status"],
            "ok"
        );

        fake.handle(&request(
            5,
            "run",
            json!({"nurt": "1.0", "program": "sleep"}),
        ));
        let mut output = Vec::new();
        fake.serve(&b""[..], &mut output).unwrap();
        assert!(String::from_utf8(output).unwrap().contains("E_CANCELLED"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
