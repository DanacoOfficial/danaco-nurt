// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt::embedded
// Responsibility:   Engine of Danaco Nurt running inside the calling process.
// Dependencies:     nurt_contracts, nurt_core, serde, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Engine of Danaco Nurt running inside the calling process.

mod system;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use nurt_contracts::envelope as contract;
use nurt_contracts::error::AbiStatus;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::HistoryQuery;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_core::config::ConfigSources;
use nurt_core::events::{DeliveredEvent, EventListener};
use nurt_core::{Engine, EngineConfig, EngineError, ExecHandle};
use serde::Serialize;
use serde_json::Value;

use self::system::{SystemClock, SystemHost};
use crate::error::{Error, ErrorKind};
use crate::request::{Language, Request, Variant};
use crate::response::{Event, Response};

/// Where an embedded engine keeps cursors and correction candidates.
#[derive(Debug, Clone, Default)]
enum StateLocation {
    #[default]
    Configured,
    Directory(PathBuf),
    Memory,
}

/// Danaco Nurt engine running inside the calling process.
///
/// `Embedded` offers the calls of [`Nurt`](crate::Nurt) without a server
/// process. Cloning is cheap and clones share all state; the engine is
/// [`Send`] and [`Sync`], and requests may run on many threads at the same
/// time.
///
/// Language errors are part of a [`Response`]. A call error ([`Error`]) means
/// that the call itself failed, for example because an argument was invalid
/// or the state directory could not be created.
///
/// # Examples
///
/// ```
/// use nurt::{Embedded, Request};
///
/// let embedded = Embedded::builder()
///     .isolated_configuration()
///     .without_state_dir()
///     .open()?;
/// let response = embedded.run(&Request::program(r#"find "/home/user/src""#))?;
/// assert!(response.status().is_some());
/// assert!(embedded.schema("request")?.is_string());
/// # Ok::<(), nurt::Error>(())
/// ```
#[derive(Clone)]
pub struct Embedded {
    engine: Engine,
}

impl Embedded {
    /// Builder of an embedded engine.
    #[must_use]
    pub fn builder() -> EmbeddedBuilder {
        EmbeddedBuilder::default()
    }

    /// Runs a request and waits for its response.
    ///
    /// A language error is part of the response; see [`Response::error`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidArgument`] when the request cannot be
    /// converted to an envelope and [`ErrorKind::Internal`] when the response
    /// cannot be represented.
    pub fn run(&self, request: &Request) -> Result<Response, Error> {
        response_of(&self.engine.run(&envelope_of(request)?))
    }

    /// Starts a request on its own thread and returns at once; the response
    /// and the events of the execution are read from the returned
    /// [`EmbeddedExecution`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidArgument`] when the request cannot be
    /// converted to an envelope.
    pub fn start(&self, request: &Request) -> Result<EmbeddedExecution, Error> {
        let (sender, events) = mpsc::channel();
        let handle = self
            .engine
            .start(&envelope_of(request)?, Arc::new(ChannelListener { sender }));
        Ok(EmbeddedExecution { handle, events })
    }

    /// Binds a request and classifies its effect without running it
    /// (`nurt-check/1`).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Language`] with the code of the language error
    /// when the request cannot be bound or planned.
    pub fn check(&self, request: &Request) -> Result<Value, Error> {
        let report = self
            .engine
            .check(&envelope_of(request)?)
            .map_err(language_error)?;
        value_of(&report)
    }

    /// Parses program text into a syntax tree `nurt-ast/1`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Language`] with the lexical or syntax error of the
    /// text.
    pub fn parse(&self, program: &str, variant: Variant) -> Result<Value, Error> {
        let tree = self
            .engine
            .parse(program, Some(variant_of(variant)))
            .map_err(language_error)?;
        value_of(&tree)
    }

    /// Prints a syntax tree `nurt-ast/1` as the canonical text of a variant.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidArgument`] when `ast` is not a syntax tree
    /// and [`ErrorKind::Language`] when it cannot be printed.
    pub fn print(&self, ast: &Value, variant: Variant) -> Result<String, Error> {
        let tree = serde_json::from_value(ast.clone()).map_err(|error| {
            Error::new(
                ErrorKind::InvalidArgument,
                format!("not a syntax tree: {error}"),
            )
        })?;
        self.engine
            .print(&tree, Some(variant_of(variant)))
            .map_err(language_error)
    }

    /// Corrects the written form of a program without running it.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Language`] with the syntax error, or with
    /// `E_AMBIGUOUS` or `E_UNCORRECTABLE`.
    pub fn correct(&self, program: &str, variant: Variant) -> Result<Value, Error> {
        let corrected = self
            .engine
            .correct(program, Some(variant_of(variant)))
            .map_err(language_error)?;
        value_of(&corrected)
    }

    /// Reads a further page of a result; `limit` `None` keeps the limit of
    /// the original request.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Internal`] when the response cannot be
    /// represented. An expired cursor is a language error in the response.
    pub fn page(&self, cursor: &str, limit: Option<u32>) -> Result<Response, Error> {
        let options = contract::RequestOptions {
            limit,
            ..contract::RequestOptions::default()
        };
        response_of(&self.engine.page(cursor, &options))
    }

    /// Description of a command, a group of commands or the language
    /// (`core`) as JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Language`] for an unknown subject.
    pub fn describe(&self, subject: &str, language: Option<Language>) -> Result<Value, Error> {
        self.engine
            .describe(subject, language.map(language_of))
            .map_err(language_error)
    }

    /// Built-in help on a topic, as text.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Language`] for an unknown topic.
    pub fn help(&self, topic: &str, language: Option<Language>) -> Result<String, Error> {
        self.engine
            .help(topic, language.map(language_of))
            .map_err(language_error)
    }

    /// Command catalog `nurt-catalog/1`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Language`] with `E_UNSUPPORTED` while no command
    /// catalog is connected to the engine.
    pub fn catalog(&self) -> Result<Value, Error> {
        self.engine.catalog().map_err(language_error)
    }

    /// Published JSON schema by name: `ast`, `bound`, `report`, `request`,
    /// `config`, `catalog` or `value`. The schema is returned as a JSON
    /// string holding the schema document.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Language`] with `E_UNKNOWN_VALUE` for another
    /// name.
    pub fn schema(&self, which: &str) -> Result<Value, Error> {
        self.engine
            .schema(which)
            .map(Value::from)
            .map_err(language_error)
    }

    /// Journal history; `query` holds the parameters of the command
    /// `history`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidArgument`] when `query` is not valid and
    /// [`ErrorKind::Language`] with `E_UNSUPPORTED` when the engine has no
    /// journal.
    pub fn history(&self, query: Option<&Value>) -> Result<Value, Error> {
        let query = match query {
            None => HistoryQuery::default(),
            Some(query) => serde_json::from_value(query.clone()).map_err(|error| {
                Error::new(
                    ErrorKind::InvalidArgument,
                    format!("invalid history query: {error}"),
                )
            })?,
        };
        let records = self.engine.history(&query).map_err(language_error)?;
        value_of(&records)
    }

    /// Space used by the journal on the volume holding `volume`, or on every
    /// volume.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidArgument`] when `volume` is not an
    /// absolute path and [`ErrorKind::Language`] with `E_UNSUPPORTED` when
    /// the engine has no journal.
    pub fn space(&self, volume: Option<&str>) -> Result<Value, Error> {
        let volume = volume
            .map(|volume| {
                AbsPath::new(volume).map_err(|error| {
                    Error::new(
                        ErrorKind::InvalidArgument,
                        format!("invalid volume `{volume}`: {error}"),
                    )
                })
            })
            .transpose()?;
        let spaces = self.engine.space(volume.as_ref()).map_err(language_error)?;
        value_of(&spaces)
    }

    /// Version, format versions, features and state directory of the engine.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Internal`] when the description cannot be
    /// represented.
    pub fn info(&self) -> Result<Value, Error> {
        value_of(&self.engine.info())
    }
}

impl fmt::Debug for Embedded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Embedded").finish_non_exhaustive()
    }
}

/// Builder of an [`Embedded`] engine.
///
/// # Examples
///
/// ```
/// use nurt::Embedded;
/// use serde_json::json;
///
/// let embedded = Embedded::builder()
///     .isolated_configuration()
///     .config(json!({"journal": {"max_ops": 50}}))
///     .session("review-42")
///     .without_state_dir()
///     .open()?;
/// let info = embedded.info()?;
/// assert_eq!(info["state_dir"], serde_json::Value::Null);
/// # Ok::<(), nurt::Error>(())
/// ```
#[derive(Debug, Default)]
pub struct EmbeddedBuilder {
    session: Option<String>,
    config: Option<Value>,
    isolated: bool,
    state: StateLocation,
}

impl EmbeddedBuilder {
    /// Default session identifier of requests that do not name one.
    #[must_use]
    pub fn session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    /// Configuration `nurt-config/1` applied over the configuration files.
    #[must_use]
    pub fn config(mut self, config: Value) -> Self {
        self.config = Some(config);
        self
    }

    /// Reads neither the configuration files nor the `NURT_*` variables, so
    /// that only the values given to [`config`](Self::config) apply.
    #[must_use]
    pub fn isolated_configuration(mut self) -> Self {
        self.isolated = true;
        self
    }

    /// State directory, instead of `state.dir` and the platform default.
    #[must_use]
    pub fn state_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.state = StateLocation::Directory(dir.into());
        self
    }

    /// No state directory: cursors and candidates are kept in memory only.
    #[must_use]
    pub fn without_state_dir(mut self) -> Self {
        self.state = StateLocation::Memory;
        self
    }

    /// Opens the engine.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidArgument`] when the configuration, the
    /// session identifier or the state directory is invalid and
    /// [`ErrorKind::Io`] when the state directory cannot be created.
    pub fn open(self) -> Result<Embedded, Error> {
        let mut sources = if self.isolated {
            ConfigSources::isolated()
        } else {
            ConfigSources::new()
        };
        match self.config {
            None => {}
            Some(Value::Object(values)) => sources = sources.with_overrides(values),
            Some(other) => {
                return Err(Error::new(
                    ErrorKind::InvalidArgument,
                    format!(
                        "configuration must be a JSON object; given {}",
                        kind_of(&other)
                    ),
                ));
            }
        }
        let mut config =
            EngineConfig::new(Arc::new(SystemClock::new()), Arc::new(SystemHost::detect()))
                .with_sources(sources);
        if let Some(session) = self.session {
            config = config.with_session(session);
        }
        config = match self.state {
            StateLocation::Configured => config,
            StateLocation::Directory(dir) => config.with_state_dir(dir),
            StateLocation::Memory => config.without_state_dir(),
        };
        Engine::open(config)
            .map(|engine| Embedded { engine })
            .map_err(engine_error)
    }
}

/// Request started with [`Embedded::start`].
///
/// Events of the execution are read with [`EmbeddedExecution::events`] or
/// [`EmbeddedExecution::try_event`]; the response with
/// [`EmbeddedExecution::wait`] or [`EmbeddedExecution::wait_timeout`].
/// Dropping an execution that has not finished cancels it.
pub struct EmbeddedExecution {
    handle: ExecHandle,
    events: Receiver<Event>,
}

impl EmbeddedExecution {
    /// Identifier of the request, as carried by its events.
    #[must_use]
    pub fn request_id(&self) -> &str {
        self.handle.request_id()
    }

    /// Events of the execution, from `exec.started` to `exec.finished`. The
    /// iterator blocks until the next event and ends when the execution has
    /// finished.
    pub fn events(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.iter()
    }

    /// The next event of the execution, when one has arrived.
    #[must_use]
    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Whether the response is available.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// Waits for the response. Further calls return the same response.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Internal`] when the response cannot be
    /// represented.
    pub fn wait(&self) -> Result<Response, Error> {
        match self.handle.wait(None) {
            Some(response) => response_of(&response),
            None => Err(Error::new(
                ErrorKind::Internal,
                "execution ended without a response",
            )),
        }
    }

    /// Waits up to `timeout` for the response; `None` when the execution is
    /// still running.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Internal`] when the response cannot be
    /// represented.
    pub fn wait_timeout(&self, timeout: Duration) -> Result<Option<Response>, Error> {
        self.handle
            .wait(Some(timeout))
            .map(|response| response_of(&response))
            .transpose()
    }

    /// Asks the engine to cancel the execution. The response then reports
    /// `E_CANCELLED`. Cancelling a finished execution has no effect.
    pub fn cancel(&self) {
        self.handle.cancel();
    }
}

impl Drop for EmbeddedExecution {
    fn drop(&mut self) {
        if !self.handle.is_finished() {
            self.handle.cancel();
        }
    }
}

impl fmt::Debug for EmbeddedExecution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmbeddedExecution")
            .field("request_id", &self.handle.request_id())
            .field("finished", &self.handle.is_finished())
            .finish_non_exhaustive()
    }
}

struct ChannelListener {
    sender: Sender<Event>,
}

impl EventListener for ChannelListener {
    fn deliver(&self, event: DeliveredEvent) {
        if let Ok(value) = serde_json::to_value(&event) {
            // The receiver is gone when the execution was dropped; the event
            // has no other consumer.
            let _gone = self.sender.send(Event::from_value(value));
        }
    }
}

fn envelope_of(request: &Request) -> Result<contract::Request, Error> {
    serde_json::from_value(request.to_value()).map_err(|error| {
        Error::new(
            ErrorKind::InvalidArgument,
            format!("request is not a valid envelope: {error}"),
        )
    })
}

fn response_of(response: &contract::Response) -> Result<Response, Error> {
    Response::from_value(value_of(response)?)
}

fn value_of<T: Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|error| {
        Error::new(
            ErrorKind::Internal,
            format!("result cannot be represented as JSON: {error}"),
        )
    })
}

fn language_error(error: nurt_contracts::error::Error) -> Error {
    Error::language(error.code.as_str(), error.message.clone())
}

fn engine_error(error: EngineError) -> Error {
    let kind = match error.status() {
        AbiStatus::Arg => ErrorKind::InvalidArgument,
        AbiStatus::Io => ErrorKind::Io,
        _ => ErrorKind::Internal,
    };
    Error::new(kind, error.message())
}

fn variant_of(variant: Variant) -> SyntaxVariant {
    match variant {
        Variant::B => SyntaxVariant::B,
        Variant::C => SyntaxVariant::C,
        _ => SyntaxVariant::A,
    }
}

fn language_of(language: Language) -> contract::Language {
    match language {
        Language::Pl => contract::Language::Pl,
        _ => contract::Language::En,
    }
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn engine_is_shared_between_threads() {
        assert_send_sync::<Embedded>();
    }

    #[test]
    fn variants_map_to_the_notations_of_the_language() {
        assert_eq!(variant_of(Variant::A), SyntaxVariant::A);
        assert_eq!(variant_of(Variant::B), SyntaxVariant::B);
        assert_eq!(variant_of(Variant::C), SyntaxVariant::C);
    }

    #[test]
    fn languages_map_to_the_languages_of_the_envelope() {
        assert_eq!(language_of(Language::En), contract::Language::En);
        assert_eq!(language_of(Language::Pl), contract::Language::Pl);
    }

    #[test]
    fn configuration_that_is_not_an_object_is_rejected() {
        let error = Embedded::builder()
            .isolated_configuration()
            .without_state_dir()
            .config(serde_json::json!([1]))
            .open()
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidArgument);
        assert!(error.message().contains("an array"));
    }

    #[test]
    fn engine_errors_keep_their_status() {
        let argument = engine_error(EngineError::argument("bad"));
        assert_eq!(argument.kind(), ErrorKind::InvalidArgument);
        let io = engine_error(EngineError::io(
            "create",
            std::path::Path::new("/state"),
            &std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        ));
        assert_eq!(io.kind(), ErrorKind::Io);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
