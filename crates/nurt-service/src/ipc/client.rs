// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Client side of the control channel.

use std::collections::VecDeque;
use std::io::{self, BufReader, Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use nurt_contracts::error::{Error, ErrorCode};
use nurt_sched::module::{ModuleRequest, ServiceLink};
use serde_json::{Value, json};

use super::protocol::{self, LineRead, Message};
use crate::paths::ServicePaths;
use crate::status::{self, Liveness};

/// Connection to the control channel of a running service.
pub struct ControlClient {
    reader: BufReader<Box<dyn Read + Send>>,
    writer: Box<dyn Write + Send>,
    next_id: u64,
    events: VecDeque<(String, Value)>,
}

impl std::fmt::Debug for ControlClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlClient")
            .field("next_id", &self.next_id)
            .field("queued_events", &self.events.len())
            .finish_non_exhaustive()
    }
}

impl ControlClient {
    /// Connects to the service of the state directory.
    ///
    /// # Errors
    ///
    /// Fails when no service listens.
    pub fn connect(paths: &ServicePaths) -> io::Result<Self> {
        let (reader, writer) = super::connect(paths)?;
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
            next_id: 1,
            events: VecDeque::new(),
        })
    }

    /// Sends a request and waits for its response; events received in the
    /// meantime are kept for [`next_event`](Self::next_event).
    ///
    /// Returns the result, or the error object of the response.
    ///
    /// # Errors
    ///
    /// Fails when the connection breaks.
    pub fn call(&mut self, op: &str, body: Value) -> io::Result<Result<Value, Value>> {
        let id = json!(self.next_id);
        self.next_id += 1;
        self.send_line(&protocol::request_line(&id, op, &body))?;
        loop {
            match self.read_message()? {
                Some(Message::Response {
                    id: answered,
                    outcome,
                }) if answered == id => return Ok(outcome),
                Some(Message::Response { .. }) | None => {}
                Some(Message::Event { event, data }) => self.events.push_back((event, data)),
            }
        }
    }

    /// Next event published to this connection.
    ///
    /// # Errors
    ///
    /// Fails when the connection breaks.
    pub fn next_event(&mut self) -> io::Result<(String, Value)> {
        if let Some(event) = self.events.pop_front() {
            return Ok(event);
        }
        loop {
            if let Some(Message::Event { event, data }) = self.read_message()? {
                return Ok((event, data));
            }
        }
    }

    /// Sends one raw line; the terminator is added when missing.
    ///
    /// # Errors
    ///
    /// Fails when the connection breaks.
    pub fn send_line(&mut self, line: &[u8]) -> io::Result<()> {
        self.writer.write_all(line)?;
        if line.last() != Some(&b'\n') {
            self.writer.write_all(b"\n")?;
        }
        self.writer.flush()
    }

    /// Reads one message; `None` for a line that is not a message.
    ///
    /// # Errors
    ///
    /// `UnexpectedEof` when the service closed the connection.
    pub fn read_message(&mut self) -> io::Result<Option<Message>> {
        match protocol::read_line(&mut self.reader, usize::MAX)? {
            LineRead::Line(line) => Ok(protocol::parse_message(&line)),
            LineRead::TooLong | LineRead::Eof => Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
        }
    }
}

/// Sends `ping` and returns the process identifier of the service and the
/// round trip time; fails when no answer arrives within `timeout`.
///
/// # Errors
///
/// `TimedOut` when the time limit elapsed, other errors when no service
/// listens.
pub fn ping(paths: &ServicePaths, timeout: Duration) -> io::Result<(u32, Duration)> {
    let paths = paths.clone();
    let (sender, receiver) = mpsc::channel();
    let started = Instant::now();
    thread::Builder::new()
        .name("nurt-service-ping".to_owned())
        .spawn(move || {
            let outcome = ControlClient::connect(&paths)
                .and_then(|mut client| client.call(protocol::OP_PING, json!({})));
            let _ = sender.send(outcome);
        })?;
    let outcome = receiver
        .recv_timeout(timeout)
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
    let elapsed = started.elapsed();
    let pid = outcome
        .ok()
        .and_then(|result| result.get("pid").and_then(Value::as_u64))
        .and_then(|pid| u32::try_from(pid).ok())
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
    Ok((pid, elapsed))
}

/// Link from a console process to the modules of the service, registered by
/// the embedding application in the execution context.
#[derive(Debug, Clone)]
pub struct ControlLink {
    paths: ServicePaths,
    max_age: Duration,
    ping_timeout: Duration,
}

impl ControlLink {
    /// Link to the service of the state directory with the liveness limits
    /// `space.heartbeat_max_age_ms` and the `ping` time limit.
    #[must_use]
    pub fn new(paths: ServicePaths, max_age: Duration, ping_timeout: Duration) -> Self {
        Self {
            paths,
            max_age,
            ping_timeout,
        }
    }
}

impl ServiceLink for ControlLink {
    fn running(&self) -> bool {
        matches!(
            status::liveness(
                &self.paths,
                status::wall_now(),
                self.max_age,
                self.ping_timeout
            ),
            Liveness::Active { .. }
        )
    }

    fn request(&self, request: ModuleRequest) -> nurt_contracts::error::Result<Value> {
        let mut client = ControlClient::connect(&self.paths).map_err(|error| {
            Error::new(
                ErrorCode::ServiceNotRunning,
                format!("the background service is not running ({error}); install it with `nurt service install`"),
            )
        })?;
        let outcome = client.call(&request.op, request.body).map_err(|error| {
            Error::new(
                ErrorCode::Service,
                format!("the background service closed the connection: {error}"),
            )
        })?;
        outcome.map_err(|error| response_error(&error))
    }
}

/// Converts an error object of a response to an error of the language.
#[must_use]
pub fn response_error(error: &Value) -> Error {
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if code.starts_with("E_")
        && let Ok(error) = serde_json::from_value::<Error>(error.clone())
    {
        return error;
    }
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("the background service rejected the request");
    Error::new(ErrorCode::Service, message.to_owned()).with_problem(code.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_errors_keep_language_codes() {
        let error = response_error(&json!({"code": "E_NOT_FOUND", "message": "no schedule `s1`"}));
        assert_eq!(error.code, ErrorCode::NotFound);
        let error = response_error(
            &json!({"code": "module_unavailable", "message": "the module `schedule` is failed"}),
        );
        assert_eq!(error.code, ErrorCode::Service);
        assert_eq!(error.problem.as_deref(), Some("module_unavailable"));
        assert_eq!(response_error(&Value::Null).code, ErrorCode::Service);
    }

    #[test]
    fn link_without_service_reports_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let link = ControlLink::new(
            ServicePaths::new(dir.path()),
            Duration::from_millis(2_000),
            Duration::from_millis(50),
        );
        assert!(!link.running());
        let error = link
            .request(ModuleRequest::new("schedule.status", json!({})))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ServiceNotRunning);
    }
}
