// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The client [`Nurt`], its [`Builder`] and asynchronous [`Execution`]s.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::error::{Error, ErrorKind};
use crate::process::{self, Call, Connection, Listener, PROTOCOL, Reply, Supervisor, Transport};
use crate::request::{Language, Request, Variant};
use crate::response::{Event, Response};

/// Time the server is given to finish its operations when the client is
/// closed, before the process is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Client of a Danaco Nurt server.
///
/// `Nurt` is [`Send`] and [`Sync`]: one client may be shared by several
/// threads, and their requests are in flight at the same time. When the
/// server process ends, the requests in flight fail with
/// [`ErrorKind::Disconnected`] and the next request starts a new process.
///
/// Dropping the client shuts the server down; [`Nurt::close`] does the same
/// and reports how the server ended.
///
/// # Examples
///
/// ```no_run
/// use nurt::{Nurt, Request};
///
/// let nurt = Nurt::builder().spawn()?;
/// let response = nurt.run(&Request::program(r#"find "/home/user/src" glob: "*.go""#))?;
/// match response.error() {
///     None => assert!(response.is_ok()),
///     Some(error) => assert!(error.code().starts_with("E_")),
/// }
/// nurt.close()?;
/// # Ok::<(), nurt::Error>(())
/// ```
pub struct Nurt {
    supervisor: Supervisor,
}

impl Nurt {
    /// Builder of a client.
    #[must_use]
    pub fn builder() -> Builder {
        Builder::default()
    }

    /// Runs a request and waits for its response.
    ///
    /// A language error is part of the response; see [`Response::error`].
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered, for
    /// example [`ErrorKind::Disconnected`] when the server ends or
    /// [`ErrorKind::Timeout`] when the request timeout passes.
    pub fn run(&self, request: &Request) -> Result<Response, Error> {
        Response::from_value(self.supervisor.call("run", request.to_value())?)
    }

    /// Starts a request and returns at once; the response and the events of
    /// the execution are read from the returned [`Execution`].
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use nurt::{Nurt, Request};
    ///
    /// let nurt = Nurt::builder().spawn()?;
    /// let mut execution = nurt.start(&Request::program(r#"test "/home/user/project""#))?;
    /// for event in execution.events() {
    ///     if event.kind() == "job.progress" {
    ///         let _progress = event.raw();
    ///     }
    /// }
    /// let response = execution.wait()?;
    /// assert!(response.status().is_some());
    /// # Ok::<(), nurt::Error>(())
    /// ```
    pub fn start(&self, request: &Request) -> Result<Execution, Error> {
        let connection = self.supervisor.connection()?;
        let call = connection.begin("run", request.to_value(), true)?;
        Ok(Execution::new(connection, call))
    }

    /// Binds a request and classifies its effect without running it
    /// (`nurt-check/1`): bound tree, effects, paths, targets, external
    /// programs, network addresses and corrections.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered.
    pub fn check(&self, request: &Request) -> Result<Value, Error> {
        self.supervisor.call("check", request.to_value())
    }

    /// Parses program text into a syntax tree `nurt-ast/1` with diagnostics.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered.
    pub fn parse(&self, program: &str, variant: Variant) -> Result<Value, Error> {
        self.supervisor.call(
            "parse",
            json!({"program": program, "variant": variant.as_str()}),
        )
    }

    /// Prints a syntax tree `nurt-ast/1` as the canonical text of a variant.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered,
    /// and [`ErrorKind::Protocol`] when the result is not text.
    pub fn print(&self, ast: &Value, variant: Variant) -> Result<String, Error> {
        let result = self
            .supervisor
            .call("print", json!({"ast": ast, "variant": variant.as_str()}))?;
        text_result("print", result)
    }

    /// Corrects the written form of a program without running it.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered.
    pub fn correct(&self, program: &str, variant: Variant) -> Result<Value, Error> {
        self.supervisor.call(
            "correct",
            json!({"program": program, "variant": variant.as_str()}),
        )
    }

    /// Reads a further page of a result; `limit` `None` keeps the limit of
    /// the original request.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered.
    /// An expired cursor is a language error in the response.
    pub fn page(&self, cursor: &str, limit: Option<u32>) -> Result<Response, Error> {
        let mut params = Map::new();
        params.insert("cursor".to_owned(), Value::from(cursor));
        if let Some(limit) = limit {
            params.insert("limit".to_owned(), Value::from(limit));
        }
        Response::from_value(self.supervisor.call("page", Value::Object(params))?)
    }

    /// Description of the language for tool integrations, as text.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered,
    /// and [`ErrorKind::Protocol`] when the result is not text.
    pub fn describe(
        &self,
        target: Describe<'_>,
        language: Option<Language>,
    ) -> Result<String, Error> {
        let mut params = Map::new();
        match target {
            Describe::Core => {
                params.insert("layer".to_owned(), Value::from("core"));
            }
            Describe::Group(name) => {
                params.insert("layer".to_owned(), Value::from("group"));
                params.insert("name".to_owned(), Value::from(name));
            }
        }
        if let Some(language) = language {
            params.insert("language".to_owned(), Value::from(language.as_str()));
        }
        let result = self.supervisor.call("describe", Value::Object(params))?;
        text_result("describe", result)
    }

    /// Built-in help on a topic, as text; `None` returns the index of
    /// topics.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered,
    /// and [`ErrorKind::Protocol`] when the result is not text.
    pub fn help(&self, topic: Option<&str>) -> Result<String, Error> {
        let params = topic.map_or_else(|| json!({}), |topic| json!({"topic": topic}));
        let result = self.supervisor.call("help", params)?;
        text_result("help", result)
    }

    /// Command catalog `nurt-catalog/1`; `what` selects a group or a
    /// command, `None` returns the whole catalog.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered.
    pub fn catalog(&self, what: Option<&Value>) -> Result<Value, Error> {
        self.supervisor
            .call("catalog", what.cloned().unwrap_or_else(|| json!({})))
    }

    /// Published JSON schema by name, for example `request` or `report`.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered;
    /// an unknown name is [`ErrorKind::InvalidArgument`].
    pub fn schema(&self, which: &str) -> Result<Value, Error> {
        self.supervisor.call("schema", json!({ "which": which }))
    }

    /// Journal history; `query` holds the parameters of the command
    /// `history`.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered.
    pub fn history(&self, query: Option<&Value>) -> Result<Value, Error> {
        self.supervisor
            .call("history", query.cloned().unwrap_or_else(|| json!({})))
    }

    /// Free space and journal usage of the volume holding `volume`, or of
    /// every volume.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered.
    pub fn space(&self, volume: Option<&str>) -> Result<Value, Error> {
        let params = volume.map_or_else(|| json!({}), |volume| json!({ "volume": volume }));
        self.supervisor.call("space", params)
    }

    /// Version, interface and format versions, catalog revision and
    /// features of the server.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered.
    pub fn info(&self) -> Result<Value, Error> {
        self.supervisor.call("info", json!({}))
    }

    /// Cancels a background job.
    ///
    /// # Errors
    ///
    /// Returns a call error when the request cannot be sent or answered.
    pub fn cancel_job(&self, job_id: &str) -> Result<(), Error> {
        self.supervisor
            .call("cancel", json!({ "job_id": job_id }))
            .map(drop)
    }

    /// Shuts the server down: it finishes its operations and ends.
    ///
    /// # Errors
    ///
    /// Returns an error when the server rejects the shutdown, ends with a
    /// failure status or does not end within five seconds and is killed.
    pub fn close(self) -> Result<(), Error> {
        self.supervisor.shutdown(SHUTDOWN_GRACE)
    }
}

impl Drop for Nurt {
    fn drop(&mut self) {
        let _ended = self.supervisor.shutdown(SHUTDOWN_GRACE);
    }
}

impl fmt::Debug for Nurt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Nurt").finish_non_exhaustive()
    }
}

fn text_result(method: &str, result: Value) -> Result<String, Error> {
    match result {
        Value::String(text) => Ok(text),
        _ => Err(Error::new(
            ErrorKind::Protocol,
            format!("result of `{method}` is not a string"),
        )),
    }
}

/// Layer of the language description returned by [`Nurt::describe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Describe<'a> {
    /// Rules, syntax, core commands, records and examples.
    Core,
    /// One group of commands, for example `documents` or `git`.
    Group(&'a str),
}

/// Builder of a [`Nurt`] client.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
///
/// use nurt::{Language, Nurt};
///
/// let nurt = Nurt::builder()
///     .executable("/home/user/.cargo/bin/nurt")
///     .language(Language::En)
///     .session("review-42")
///     .request_timeout(Duration::from_secs(300))
///     .on_event(|event| {
///         if event.kind() == "journal.committed" {
///             let _change = event.raw();
///         }
///     })
///     .spawn()?;
/// # Ok::<(), nurt::Error>(())
/// ```
#[derive(Default)]
pub struct Builder {
    executable: Option<PathBuf>,
    language: Option<Language>,
    session: Option<String>,
    config: Option<Value>,
    request_timeout: Option<Duration>,
    listener: Option<Listener>,
}

impl Builder {
    /// Path of the `nurt` executable. Without it the executable named by
    /// `NURT_EXECUTABLE` is used, and without that variable the first
    /// `nurt` found in `PATH`.
    #[must_use]
    pub fn executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.executable = Some(path.into());
        self
    }

    /// Default language of messages and help.
    #[must_use]
    pub fn language(mut self, language: Language) -> Self {
        self.language = Some(language);
        self
    }

    /// Default session identifier of requests.
    #[must_use]
    pub fn session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    /// Configuration `nurt-config/1` applied over the configuration files
    /// of the server.
    #[must_use]
    pub fn config(mut self, config: Value) -> Self {
        self.config = Some(config);
        self
    }

    /// Time limit of each response. Without it the client waits until the
    /// server answers or ends. A request that passes the limit fails with
    /// [`ErrorKind::Timeout`] and is cancelled on the server.
    #[must_use]
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = Some(timeout);
        self
    }

    /// Receiver of the events that belong to no execution started with
    /// [`Nurt::start`]: background jobs, journal changes, free space,
    /// schedules and log messages.
    ///
    /// The receiver is called on the thread that reads the messages of the
    /// server; it must return quickly and must not wait for a response.
    #[must_use]
    pub fn on_event<F>(mut self, listener: F) -> Self
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        self.listener = Some(Arc::new(listener));
        self
    }

    /// Starts `nurt serve --stdio` and initializes the client.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::NotFound`] with the checked locations when the
    /// executable is not found, [`ErrorKind::Incompatible`] when the server
    /// uses another major version of the protocol, and other call errors
    /// when the process cannot be started or initialized.
    pub fn spawn(self) -> Result<Nurt, Error> {
        let executable = process::locate(self.executable.as_deref())?;
        self.connect_with(move || Transport::spawn(&executable))
    }

    /// Initializes the client over streams opened by `connect`, for example
    /// a socket of a long-running server. `connect` is called again to
    /// replace a connection that has closed.
    ///
    /// # Errors
    ///
    /// Returns the error of `connect`, [`ErrorKind::Incompatible`] when the
    /// server uses another major version of the protocol, and other call
    /// errors when the connection cannot be initialized.
    pub fn connect_with<F>(self, connect: F) -> Result<Nurt, Error>
    where
        F: Fn() -> io::Result<Transport> + Send + Sync + 'static,
    {
        let initialize = self.initialize_params();
        let supervisor = Supervisor::start(
            Box::new(connect),
            initialize,
            self.listener,
            self.request_timeout,
        )?;
        Ok(Nurt { supervisor })
    }

    fn initialize_params(&self) -> Value {
        let mut params = Map::new();
        params.insert("protocol".to_owned(), Value::from(PROTOCOL));
        params.insert(
            "client".to_owned(),
            json!({"name": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION")}),
        );
        params.insert("capabilities".to_owned(), json!({"events": true}));
        if let Some(language) = self.language {
            params.insert("language".to_owned(), Value::from(language.as_str()));
        }
        if let Some(session) = &self.session {
            params.insert("session".to_owned(), Value::from(session.as_str()));
        }
        if let Some(config) = &self.config {
            params.insert("config_overrides".to_owned(), config.clone());
        }
        Value::Object(params)
    }
}

impl fmt::Debug for Builder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Builder")
            .field("executable", &self.executable)
            .field("language", &self.language)
            .field("session", &self.session)
            .field("config", &self.config)
            .field("request_timeout", &self.request_timeout)
            .field("listener", &self.listener.is_some())
            .finish()
    }
}

/// Request started with [`Nurt::start`].
///
/// Events of the execution are read with [`Execution::events`] or
/// [`Execution::try_event`]; the response with [`Execution::wait`] or
/// [`Execution::wait_timeout`]. Dropping an execution that has not finished
/// cancels it.
pub struct Execution {
    connection: Arc<Connection>,
    id: u64,
    reply: Receiver<Reply>,
    events: Receiver<Event>,
    outcome: Option<Result<Response, Error>>,
}

impl Execution {
    fn new(connection: Arc<Connection>, call: Call) -> Self {
        let Call { id, reply, events } = call;
        Self {
            connection,
            id,
            reply,
            events,
            outcome: None,
        }
    }

    /// Events of the execution in the order of the server. The iterator
    /// blocks until the next event and ends when the execution has
    /// finished.
    pub fn events(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.iter()
    }

    /// The next event of the execution, when one has arrived.
    #[must_use]
    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Waits for the response. Further calls return the same result.
    ///
    /// # Errors
    ///
    /// Returns a call error when the server ends before the response or
    /// rejects the request.
    pub fn wait(&mut self) -> Result<Response, Error> {
        if let Some(outcome) = &self.outcome {
            return outcome.clone();
        }
        self.settle(process::receive(&self.reply))
    }

    /// Waits up to `timeout` for the response; `None` when the execution is
    /// still running.
    ///
    /// # Errors
    ///
    /// Returns a call error when the server ends before the response or
    /// rejects the request.
    pub fn wait_timeout(&mut self, timeout: Duration) -> Result<Option<Response>, Error> {
        if let Some(outcome) = &self.outcome {
            return outcome.clone().map(Some);
        }
        match process::receive_timeout(&self.reply, timeout) {
            Some(reply) => self.settle(reply).map(Some),
            None => Ok(None),
        }
    }

    fn settle(&mut self, reply: Reply) -> Result<Response, Error> {
        let outcome = reply.and_then(Response::from_value);
        self.outcome = Some(outcome.clone());
        outcome
    }

    /// Asks the server to cancel the execution. The response then reports
    /// the cancelled execution; changes already committed stay in the
    /// journal.
    ///
    /// # Errors
    ///
    /// Returns a call error when the cancellation cannot be sent or
    /// answered.
    pub fn cancel(&self) -> Result<(), Error> {
        self.connection
            .call("cancel", json!({ "request_id": self.id }), None)
            .map(drop)
    }
}

impl Drop for Execution {
    fn drop(&mut self) {
        if self.outcome.is_none() && self.reply.try_recv().is_err() {
            self.connection.abandon(self.id);
        }
    }
}

impl fmt::Debug for Execution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Execution")
            .field("id", &self.id)
            .field("finished", &self.outcome.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn client_is_shared_between_threads() {
        assert_send_sync::<Nurt>();
        assert_send_sync::<Builder>();
    }

    #[test]
    fn initialize_carries_protocol_client_and_settings() {
        let params = Nurt::builder()
            .language(Language::Pl)
            .session("s-9")
            .config(json!({"journal": {"enabled": true}}))
            .initialize_params();
        assert_eq!(
            params,
            json!({
                "protocol": "nurt-rpc/1",
                "client": {"name": "nurt", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"events": true},
                "language": "pl",
                "session": "s-9",
                "config_overrides": {"journal": {"enabled": true}}
            })
        );
    }

    #[test]
    fn builder_debug_hides_the_listener() {
        let text = format!("{:?}", Nurt::builder().on_event(drop));
        assert!(text.contains("listener: true"));
    }

    #[test]
    fn text_result_rejects_other_values() {
        assert_eq!(text_result("help", json!("topics")).unwrap(), "topics");
        let error = text_result("help", json!({"text": "topics"})).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Protocol);
    }
}
