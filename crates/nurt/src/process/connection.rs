// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt::process::connection
// Responsibility:   One connection to `nurt serve`: JSON-RPC 2.0 messages, one per line.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! One connection to `nurt serve`: JSON-RPC 2.0 messages, one per line.
//!
//! A reader thread receives every message of the server. Responses are
//! matched to their requests by `id`, so several requests may be in flight
//! and answered in any order. Events (`nurt/event`) that name a request are
//! delivered to the execution that started it; the remaining events go to
//! the listener of the client. When the stream ends or a message breaks the
//! protocol, the connection closes and every request in flight fails with
//! the reason.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind as IoErrorKind, Read, Write};
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Listener, Transport, lock};
use crate::error::{Error, ErrorKind, Status};
use crate::response::Event;

/// Name and major version of the protocol.
pub(crate) const PROTOCOL: &str = "nurt-rpc/1";

/// Bits of the interface version below the major number.
const ABI_MAJOR_SHIFT: u32 = 16;

/// Major number of the interface version the client needs.
pub(crate) const ABI_MAJOR: u32 = 1;

/// Interface version the client needs, `(major << 16) | minor`.
pub(crate) const ABI_REQUIRED: u32 = ABI_MAJOR << ABI_MAJOR_SHIFT;

/// Largest message accepted from the server, in bytes.
const MAX_MESSAGE_BYTES: u64 = 64 * 1024 * 1024;

/// Application error codes are `APPLICATION_ERROR_BASE - status`, so
/// `NURT_E_ARG` (3) is `-32003`.
const APPLICATION_ERROR_BASE: i64 = -32000;

/// Lowest application error code of JSON-RPC 2.0.
const APPLICATION_ERROR_LOWEST: i64 = -32099;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

/// Interval between checks whether the server process has ended.
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) type Reply = Result<Value, Error>;

struct Waiter {
    reply: Sender<Reply>,
    events: Option<Sender<Event>>,
}

#[derive(Default)]
struct State {
    pending: HashMap<u64, Waiter>,
    closed: Option<Error>,
}

struct Shared {
    state: Mutex<State>,
    listener: Option<Listener>,
}

impl Shared {
    fn close(&self, reason: Error) {
        let (reason, waiters) = {
            let mut state = lock(&self.state);
            let reason = state.closed.get_or_insert(reason).clone();
            (reason, std::mem::take(&mut state.pending))
        };
        for waiter in waiters.into_values() {
            let _unanswered = waiter.reply.send(Err(reason.clone()));
        }
    }

    fn dispatch(&self, message: Value) {
        match message.get("method").and_then(Value::as_str) {
            Some("nurt/event") => {
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                self.deliver(Event::from_value(params));
            }
            Some(_) => {}
            None => {
                let Some(id) = message.get("id").and_then(request_id) else {
                    return;
                };
                let waiter = lock(&self.state).pending.remove(&id);
                if let Some(waiter) = waiter {
                    let _unanswered = waiter.reply.send(reply_of(&message));
                }
            }
        }
    }

    fn deliver(&self, event: Event) {
        let target = event
            .raw()
            .get("request")
            .and_then(request_id)
            .and_then(|id| {
                lock(&self.state)
                    .pending
                    .get(&id)
                    .and_then(|waiter| waiter.events.clone())
            });
        match target {
            Some(sender) => {
                let _unread = sender.send(event);
            }
            None => {
                if let Some(listener) = &self.listener {
                    listener(event);
                }
            }
        }
    }
}

/// Request sent to the server and not yet answered.
pub(crate) struct Call {
    pub(crate) id: u64,
    pub(crate) reply: Receiver<Reply>,
    pub(crate) events: Receiver<Event>,
}

pub(crate) struct Connection {
    shared: Arc<Shared>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Mutex<Option<Child>>,
    next_id: AtomicU64,
}

impl Connection {
    pub(crate) fn open(transport: Transport, listener: Option<Listener>) -> Result<Self, Error> {
        let Transport {
            reader,
            writer,
            child,
        } = transport;
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            listener,
        });
        let reader_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("nurt-serve-reader".to_owned())
            .spawn(move || read_messages(&reader_shared, reader))
            .map_err(|error| {
                Error::new(
                    ErrorKind::Io,
                    format!("cannot start the reader thread: {error}"),
                )
            })?;
        Ok(Self {
            shared,
            writer: Mutex::new(Some(writer)),
            child: Mutex::new(child),
            next_id: AtomicU64::new(1),
        })
    }

    pub(crate) fn is_open(&self) -> bool {
        lock(&self.shared.state).closed.is_none()
    }

    /// Sends a request; the response arrives in the returned [`Call`].
    pub(crate) fn begin(&self, method: &str, params: Value, events: bool) -> Result<Call, Error> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply_sender, reply) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let event_sender = events.then_some(event_sender);
        {
            let mut state = lock(&self.shared.state);
            if let Some(reason) = &state.closed {
                return Err(reason.clone());
            }
            state.pending.insert(
                id,
                Waiter {
                    reply: reply_sender,
                    events: event_sender,
                },
            );
        }
        if let Err(error) = self.send(id, method, params) {
            self.forget(id);
            return Err(error);
        }
        Ok(Call {
            id,
            reply,
            events: event_receiver,
        })
    }

    /// Sends a request and waits for its response.
    pub(crate) fn call(
        &self,
        method: &str,
        params: Value,
        timeout: Option<Duration>,
    ) -> Result<Value, Error> {
        let call = self.begin(method, params, false)?;
        let Some(limit) = timeout else {
            return receive(&call.reply);
        };
        receive_timeout(&call.reply, limit).unwrap_or_else(|| {
            self.abandon(call.id);
            Err(Error::new(
                ErrorKind::Timeout,
                format!(
                    "no response to `{method}` within {} ms; raise the request timeout",
                    limit.as_millis()
                ),
            ))
        })
    }

    /// Stops waiting for a request and asks the server to cancel it. The
    /// cancellation is not awaited; its response is ignored.
    pub(crate) fn abandon(&self, id: u64) {
        self.forget(id);
        let cancel_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let _ignored = self.send(cancel_id, "cancel", json!({ "request_id": id }));
    }

    fn forget(&self, id: u64) {
        lock(&self.shared.state).pending.remove(&id);
    }

    fn send(&self, id: u64, method: &str, params: Value) -> Result<(), Error> {
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut line = message.to_string().into_bytes();
        line.push(b'\n');
        let mut writer = lock(&self.writer);
        let Some(stream) = writer.as_mut() else {
            return Err(connection_closed());
        };
        stream
            .write_all(&line)
            .and_then(|()| stream.flush())
            .map_err(|error| {
                let error = Error::new(
                    ErrorKind::Disconnected,
                    format!("cannot write to nurt serve: {error}"),
                );
                self.shared.close(error.clone());
                error
            })
    }

    /// Ends the connection: asks the server to shut down when it is still
    /// open, closes its input and waits up to `grace` for the process to
    /// end before killing it.
    ///
    /// Returns an error when the server rejects `shutdown`, exits with a
    /// failure status or has to be killed.
    pub(crate) fn shutdown(&self, grace: Duration) -> Result<(), Error> {
        let mut outcome = Ok(());
        if self.is_open()
            && let Err(error) = self.call("shutdown", json!({}), Some(grace))
            && error.kind() != ErrorKind::Disconnected
        {
            outcome = Err(error);
        }
        lock(&self.writer).take();
        let child = lock(&self.child).take();
        if let Some(child) = child {
            let ended = reap(child, grace);
            if outcome.is_ok() {
                outcome = ended;
            }
        }
        self.shared.close(connection_closed());
        outcome
    }
}

/// Waits for the response of a request.
pub(crate) fn receive(reply: &Receiver<Reply>) -> Reply {
    reply.recv().unwrap_or_else(|_| Err(closed_without_reply()))
}

/// Waits up to `limit` for the response of a request; `None` when it has
/// not arrived.
pub(crate) fn receive_timeout(reply: &Receiver<Reply>, limit: Duration) -> Option<Reply> {
    match reply.recv_timeout(limit) {
        Ok(reply) => Some(reply),
        Err(RecvTimeoutError::Timeout) => None,
        Err(RecvTimeoutError::Disconnected) => Some(Err(closed_without_reply())),
    }
}

fn connection_closed() -> Error {
    Error::new(
        ErrorKind::Disconnected,
        "connection to nurt serve is closed",
    )
}

fn closed_without_reply() -> Error {
    Error::new(
        ErrorKind::Disconnected,
        "connection to nurt serve closed before the response",
    )
}

fn reap(mut child: Child, grace: Duration) -> Result<(), Error> {
    let deadline = Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(Error::new(
                    ErrorKind::Disconnected,
                    format!("nurt serve ended with {status}"),
                ));
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(EXIT_POLL_INTERVAL),
            Ok(None) | Err(_) => break,
        }
    }
    let killed = child.kill();
    let _status = child.wait();
    match killed {
        Ok(()) => Err(Error::new(
            ErrorKind::Disconnected,
            format!(
                "nurt serve did not end within {} ms and was killed",
                grace.as_millis()
            ),
        )),
        Err(error) => Err(Error::new(
            ErrorKind::Io,
            format!("cannot stop nurt serve: {error}"),
        )),
    }
}

fn read_messages(shared: &Shared, reader: Box<dyn Read + Send>) {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    let reason = loop {
        line.clear();
        match (&mut reader)
            .take(MAX_MESSAGE_BYTES + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) => {
                break Error::new(ErrorKind::Disconnected, "nurt serve closed the connection");
            }
            Ok(read) => {
                if line.last() != Some(&b'\n') && read as u64 > MAX_MESSAGE_BYTES {
                    break Error::new(
                        ErrorKind::Protocol,
                        format!("message from nurt serve exceeds {MAX_MESSAGE_BYTES} bytes"),
                    );
                }
                let text = line.trim_ascii();
                if text.is_empty() {
                    continue;
                }
                match serde_json::from_slice::<Value>(text) {
                    Ok(message) => shared.dispatch(message),
                    Err(error) => {
                        break Error::new(
                            ErrorKind::Protocol,
                            format!("nurt serve sent a message that is not valid JSON: {error}"),
                        );
                    }
                }
            }
            Err(error) if error.kind() == IoErrorKind::Interrupted => {}
            Err(error) => {
                break Error::new(
                    ErrorKind::Disconnected,
                    format!("cannot read from nurt serve: {error}"),
                );
            }
        }
    };
    shared.close(reason);
}

fn request_id(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

/// Response of a cancelled request: the server reports `NURT_E_CANCELLED`
/// and carries the response in `data.response`, which the caller receives as
/// a result, as in the other bindings.
fn cancelled_response(error: &Value) -> Option<Value> {
    let cancelled = APPLICATION_ERROR_BASE - i64::from(Status::Cancelled.code());
    if error.get("code").and_then(Value::as_i64) != Some(cancelled) {
        return None;
    }
    let response = error.get("data")?.get("response")?;
    response
        .get("report")?
        .is_object()
        .then(|| response.clone())
}

fn reply_of(message: &Value) -> Reply {
    match message.get("error") {
        Some(error) => cancelled_response(error).map_or_else(|| Err(rpc_error(error)), Ok),
        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
    }
}

/// Call error for a JSON-RPC error object of the server.
pub(crate) fn rpc_error(error: &Value) -> Error {
    let code = error
        .get("code")
        .and_then(Value::as_i64)
        .unwrap_or(INTERNAL_ERROR);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("error without a message");
    let kind = match code {
        INVALID_PARAMS => ErrorKind::InvalidArgument,
        INTERNAL_ERROR => ErrorKind::Internal,
        PARSE_ERROR | INVALID_REQUEST | METHOD_NOT_FOUND => ErrorKind::Protocol,
        APPLICATION_ERROR_LOWEST..APPLICATION_ERROR_BASE => {
            u32::try_from(APPLICATION_ERROR_BASE - code)
                .ok()
                .and_then(Status::from_code)
                .map_or(ErrorKind::Internal, ErrorKind::from_status)
        }
        _ => ErrorKind::Protocol,
    };
    Error::new(kind, format!("nurt serve: {message} (code {code})"))
}

/// Whether the interface version reported by the server, when it reports
/// one, has the major number of [`ABI_REQUIRED`] and at least its minor.
pub(crate) fn is_abi_compatible(abi: Option<u64>) -> bool {
    abi.is_none_or(|abi| {
        let required = u64::from(ABI_REQUIRED);
        abi >> ABI_MAJOR_SHIFT == required >> ABI_MAJOR_SHIFT && abi >= required
    })
}

/// Whether the server protocol named in `initialize` is compatible.
pub(crate) fn is_compatible(protocol: &str) -> bool {
    protocol == PROTOCOL
        || protocol
            .strip_prefix(PROTOCOL)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|minor| !minor.is_empty() && minor.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_codes_map_to_statuses() {
        for status in Status::ALL {
            let code = APPLICATION_ERROR_BASE - i64::from(status.code());
            let error = rpc_error(&json!({"code": code, "message": "failed"}));
            assert_eq!(error.status(), *status, "code {code}");
        }
    }

    #[test]
    fn unknown_application_code_is_internal() {
        let error = rpc_error(&json!({"code": -32050, "message": "failed"}));
        assert_eq!(error.kind(), ErrorKind::Internal);
    }

    #[test]
    fn standard_codes_map_to_kinds() {
        let cases = [
            (PARSE_ERROR, ErrorKind::Protocol),
            (INVALID_REQUEST, ErrorKind::Protocol),
            (METHOD_NOT_FOUND, ErrorKind::Protocol),
            (INVALID_PARAMS, ErrorKind::InvalidArgument),
            (INTERNAL_ERROR, ErrorKind::Internal),
            (-32000, ErrorKind::Protocol),
            (1, ErrorKind::Protocol),
        ];
        for (code, kind) in cases {
            assert_eq!(
                rpc_error(&json!({"code": code})).kind(),
                kind,
                "code {code}"
            );
        }
    }

    #[test]
    fn error_message_names_the_code() {
        let error = rpc_error(&json!({"code": -32003, "message": "request is not an object"}));
        assert_eq!(
            error.message(),
            "nurt serve: request is not an object (code -32003)"
        );
        let error = rpc_error(&json!({}));
        assert_eq!(
            error.message(),
            "nurt serve: error without a message (code -32603)"
        );
    }

    #[test]
    fn request_ids_are_numbers_or_numeric_strings() {
        assert_eq!(request_id(&json!(7)), Some(7));
        assert_eq!(request_id(&json!("8")), Some(8));
        assert_eq!(request_id(&json!("x")), None);
        assert_eq!(request_id(&json!(-1)), None);
        assert_eq!(request_id(&Value::Null), None);
    }

    #[test]
    fn interface_version_needs_the_same_major() {
        assert!(is_abi_compatible(None));
        assert!(is_abi_compatible(Some(1 << 16)));
        assert!(is_abi_compatible(Some((1 << 16) | 3)));
        assert!(!is_abi_compatible(Some(2 << 16)));
        assert!(!is_abi_compatible(Some(0)));
    }

    #[test]
    fn protocol_minor_versions_are_compatible() {
        assert!(is_compatible("nurt-rpc/1"));
        assert!(is_compatible("nurt-rpc/1.3"));
        assert!(!is_compatible("nurt-rpc/2"));
        assert!(!is_compatible("nurt-rpc/10"));
        assert!(!is_compatible(""));
    }

    #[test]
    fn protocol_minor_version_must_be_a_number() {
        assert!(is_compatible("nurt-rpc/1.0"));
        assert!(is_compatible("nurt-rpc/1.12"));
        for protocol in [
            "nurt-rpc/1.",
            "nurt-rpc/1.x",
            "nurt-rpc/1.2.3",
            "nurt-rpc/1.-1",
            "nurt-rpc/1. 2",
            "nurt-rpc/1.\u{663}",
            "Nurt-rpc/1",
            "nurt-rpc/1 ",
        ] {
            assert!(!is_compatible(protocol), "{protocol:?}");
        }
    }

    #[test]
    fn reply_with_an_error_member_fails_even_with_a_result() {
        let error =
            reply_of(&json!({"id": 1, "result": 5, "error": {"code": -32602}})).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidArgument);
    }

    #[test]
    fn non_integer_codes_are_internal_errors() {
        let error = rpc_error(&json!({"code": "x", "message": "m"}));
        assert_eq!(error.kind(), ErrorKind::Internal);
        assert!(error.message().contains("code -32603"));
    }

    #[test]
    fn cancelled_request_replies_with_its_response() {
        let code = APPLICATION_ERROR_BASE - i64::from(Status::Cancelled.code());
        let response = json!({"report": {"status": "failed", "error": {"code": "E_CANCELLED"}}});
        let message = json!({
            "id": 1,
            "error": {"code": code, "message": "cancelled", "data": {"response": response}}
        });
        assert_eq!(reply_of(&message).unwrap(), response);

        let without = json!({"id": 1, "error": {"code": code, "message": "cancelled"}});
        assert_eq!(reply_of(&without).unwrap_err().status(), Status::Cancelled);
        let not_a_response = json!({
            "id": 1,
            "error": {"code": code, "message": "cancelled", "data": {"response": 5}}
        });
        assert!(reply_of(&not_a_response).is_err());
        let other = APPLICATION_ERROR_BASE - i64::from(Status::Io.code());
        let failed = json!({
            "id": 1,
            "error": {"code": other, "message": "x", "data": {"response": response}}
        });
        assert!(reply_of(&failed).is_err());
    }

    #[test]
    fn reply_without_result_is_null() {
        assert_eq!(reply_of(&json!({"id": 1})).unwrap(), Value::Null);
        assert_eq!(reply_of(&json!({"id": 1, "result": 5})).unwrap(), json!(5));
        assert!(reply_of(&json!({"id": 1, "error": {"code": -32603}})).is_err());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
