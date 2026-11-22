// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Control protocol of the service, version 1.
//!
//! Every message is one JSON object on one line, terminated by `\n`, at most
//! [`MAX_LINE`] bytes long without the terminator. Every message carries the
//! protocol version `v`.
//!
//! | Message | Form |
//! |---|---|
//! | request | `{"v":1,"id":<id>,"op":<op>,"body":{…}}`; `body` is optional |
//! | success | `{"v":1,"id":<id>,"ok":true,"result":{…}}` |
//! | failure | `{"v":1,"id":<id>,"ok":false,"error":{"code":…,"message":…}}` |
//! | event | `{"v":1,"event":<name>,"data":{…}}` |
//!
//! `id` is a string or a number chosen by the client and returned unchanged.
//! Operations handled by the host are [`OP_PING`], [`OP_STATUS`],
//! [`OP_RELOAD`] and [`OP_SUBSCRIBE`]. An operation that starts with the
//! name of a hosted module followed by `.` or `/`, for example `schedule.run`
//! or `schedule/ack`, goes to that module ([`module_of`]).
//!
//! Error codes of the protocol itself are lowercase words ([`ERR_BAD_REQUEST`]
//! and the other `ERR_` constants). Errors reported by a module are errors of
//! the language with their `E_` codes and fields.

use std::io::{self, BufRead};
use std::time::Duration;

use nurt_contracts::error::Error;
use serde_json::{Map, Value, json};

/// Version of the protocol spoken by this service.
pub const PROTOCOL_VERSION: u64 = 1;

/// Versions the service accepts.
pub const SUPPORTED_VERSIONS: &[u64] = &[PROTOCOL_VERSION];

/// Longest accepted line, without the terminator.
pub const MAX_LINE: usize = 64 * 1024;

/// A connection that sends nothing for this long is closed, unless it
/// subscribed to events or registered a remote action kind.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Liveness check; the result holds `pid`. Never waits for a module.
pub const OP_PING: &str = "ping";
/// State of the service and its modules.
pub const OP_STATUS: &str = "status";
/// Re-reads the configuration and passes it to every module; a module in
/// state `failed` is started again.
pub const OP_RELOAD: &str = "reload";
/// Subscribes the connection to events addressed to all connections.
pub const OP_SUBSCRIBE: &str = "subscribe";

/// Name of the module an operation is addressed to: the part before the
/// first `.` or `/`.
///
/// ```
/// use nurt_service::ipc::protocol::module_of;
///
/// assert_eq!(module_of("schedule.run"), Some("schedule"));
/// assert_eq!(module_of("schedule/register_action"), Some("schedule"));
/// assert_eq!(module_of("ping"), None);
/// assert_eq!(module_of(".run"), None);
/// ```
#[must_use]
pub fn module_of(op: &str) -> Option<&str> {
    op.split_once(['.', '/'])
        .map(|(module, _)| module)
        .filter(|module| !module.is_empty())
}

/// The request is not a JSON object with `v` and `op`, or `body` is not an
/// object.
pub const ERR_BAD_REQUEST: &str = "bad_request";
/// The operation is unknown; `ops` lists the operations of the host.
pub const ERR_UNKNOWN_OP: &str = "unknown_op";
/// The version is not supported; `supported` lists the accepted versions.
pub const ERR_UNSUPPORTED_VERSION: &str = "unsupported_version";
/// The line exceeds [`MAX_LINE`]; the service closes the connection.
pub const ERR_LINE_TOO_LONG: &str = "line_too_long";
/// The module that handles the operation is not hosted or not running.
pub const ERR_MODULE_UNAVAILABLE: &str = "module_unavailable";

/// Operations handled by the host.
pub const HOST_OPS: &[&str] = &[OP_PING, OP_STATUS, OP_RELOAD, OP_SUBSCRIBE];

/// Request received by the service.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Identifier chosen by the client.
    pub id: Value,
    /// Operation.
    pub op: String,
    /// Body; an empty object when the request has none.
    pub body: Value,
}

/// Error of the protocol, sent back in place of a result.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolError {
    /// Identifier of the request, `null` when it could not be read.
    pub id: Value,
    /// Error object.
    pub error: Value,
}

/// Parses a request line.
///
/// # Errors
///
/// [`ERR_BAD_REQUEST`] for a line that is not a request and
/// [`ERR_UNSUPPORTED_VERSION`] for another version.
///
/// ```
/// use nurt_service::ipc::protocol::parse_request;
///
/// let request = parse_request(br#"{"v":1,"id":3,"op":"ping"}"#).unwrap();
/// assert_eq!(request.op, "ping");
/// let error = parse_request(br#"{"v":2,"id":4,"op":"ping"}"#).unwrap_err();
/// assert_eq!(error.error["code"], "unsupported_version");
/// ```
pub fn parse_request(line: &[u8]) -> Result<Request, Box<ProtocolError>> {
    let Ok(Value::Object(mut object)) = serde_json::from_slice::<Value>(line) else {
        return Err(bad_request(Value::Null, "the line is not a JSON object"));
    };
    let id = object.remove("id").unwrap_or(Value::Null);
    if !matches!(id, Value::Null | Value::String(_) | Value::Number(_)) {
        return Err(bad_request(
            Value::Null,
            "`id` must be a string or a number",
        ));
    }
    let Some(version) = object.get("v").and_then(Value::as_u64) else {
        return Err(bad_request(id, "`v` with the protocol version is missing"));
    };
    if !SUPPORTED_VERSIONS.contains(&version) {
        let error = error_object(
            ERR_UNSUPPORTED_VERSION,
            &format!("protocol version {version} is not supported; use version {PROTOCOL_VERSION}"),
        );
        return Err(Box::new(ProtocolError {
            id,
            error: with_field(error, "supported", json!(SUPPORTED_VERSIONS)),
        }));
    }
    let Some(Value::String(op)) = object.remove("op") else {
        return Err(bad_request(id, "`op` with the operation name is missing"));
    };
    let body = match object.remove("body") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(body @ Value::Object(_)) => body,
        Some(_) => return Err(bad_request(id, "`body` must be a JSON object")),
    };
    Ok(Request { id, op, body })
}

fn bad_request(id: Value, message: &str) -> Box<ProtocolError> {
    Box::new(ProtocolError {
        id,
        error: error_object(ERR_BAD_REQUEST, message),
    })
}

/// Error object of the protocol.
#[must_use]
pub fn error_object(code: &str, message: &str) -> Value {
    json!({ "code": code, "message": message })
}

/// Error object with an extra field.
#[must_use]
pub fn with_field(mut error: Value, name: &str, value: Value) -> Value {
    if let Value::Object(fields) = &mut error {
        fields.insert(name.to_owned(), value);
    }
    error
}

/// Error object for an error of the language reported by a module.
#[must_use]
pub fn language_error(error: &Error) -> Value {
    serde_json::to_value(error).unwrap_or_else(|_| error_object("E_INTERNAL", &error.message))
}

/// Encodes a request line.
#[must_use]
pub fn request_line(id: &Value, op: &str, body: &Value) -> Vec<u8> {
    to_line(&json!({ "v": PROTOCOL_VERSION, "id": id, "op": op, "body": body }))
}

/// Encodes a success line.
#[must_use]
pub fn success_line(id: &Value, result: &Value) -> Vec<u8> {
    to_line(&json!({ "v": PROTOCOL_VERSION, "id": id, "ok": true, "result": result }))
}

/// Encodes a failure line.
#[must_use]
pub fn failure_line(id: &Value, error: &Value) -> Vec<u8> {
    to_line(&json!({ "v": PROTOCOL_VERSION, "id": id, "ok": false, "error": error }))
}

/// Encodes an event line.
#[must_use]
pub fn event_line(event: &str, data: &Value) -> Vec<u8> {
    to_line(&json!({ "v": PROTOCOL_VERSION, "event": event, "data": data }))
}

fn to_line(value: &Value) -> Vec<u8> {
    let mut line = value.to_string().into_bytes();
    line.push(b'\n');
    line
}

/// Outcome of reading one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineRead {
    /// A complete line without its terminator (`\n` or `\r\n`).
    Line(Vec<u8>),
    /// The line exceeds the limit; the rest of it was not read.
    TooLong,
    /// The peer closed the connection before a complete line.
    Eof,
}

/// Reads one line of at most `max` bytes.
///
/// # Errors
///
/// Returns input and output errors of the reader, including the timeout of
/// an idle connection.
///
/// ```
/// use nurt_service::ipc::protocol::{LineRead, read_line};
///
/// let mut input: &[u8] = b"{\"v\":1}\r\nabcdef\n";
/// assert_eq!(read_line(&mut input, 16).unwrap(), LineRead::Line(b"{\"v\":1}".to_vec()));
/// assert_eq!(read_line(&mut input, 3).unwrap(), LineRead::TooLong);
/// ```
pub fn read_line<R: BufRead + ?Sized>(reader: &mut R, max: usize) -> io::Result<LineRead> {
    let mut line = Vec::new();
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            return Ok(LineRead::Eof);
        }
        let (chunk, done) = match available.iter().position(|byte| *byte == b'\n') {
            Some(end) => (&available[..end], Some(end + 1)),
            None => (available, None),
        };
        if line.len() + chunk.len() > max {
            return Ok(LineRead::TooLong);
        }
        line.extend_from_slice(chunk);
        let consumed = done.unwrap_or(chunk.len());
        reader.consume(consumed);
        if done.is_some() {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(LineRead::Line(line));
        }
    }
}

/// Message received by a client.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Response to a request.
    Response {
        /// Identifier of the request.
        id: Value,
        /// Result, or the error object when `ok` was `false`.
        outcome: Result<Value, Value>,
    },
    /// Event published by a module.
    Event {
        /// Event name.
        event: String,
        /// Event data.
        data: Value,
    },
}

/// Parses a line received by a client; `None` when it is neither a
/// response nor an event of a supported version.
#[must_use]
pub fn parse_message(line: &[u8]) -> Option<Message> {
    let Value::Object(mut object) = serde_json::from_slice::<Value>(line).ok()? else {
        return None;
    };
    if !object
        .get("v")
        .and_then(Value::as_u64)
        .is_some_and(|version| SUPPORTED_VERSIONS.contains(&version))
    {
        return None;
    }
    if let Some(Value::String(event)) = object.remove("event") {
        let data = object.remove("data").unwrap_or(Value::Null);
        return Some(Message::Event { event, data });
    }
    let id = object.remove("id").unwrap_or(Value::Null);
    let outcome = match object.get("ok").and_then(Value::as_bool)? {
        true => Ok(object.remove("result").unwrap_or(Value::Null)),
        false => Err(object.remove("error").unwrap_or(Value::Null)),
    };
    Some(Message::Response { id, outcome })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::error::ErrorCode;

    #[test]
    fn requests_with_defects_are_rejected_with_bad_request() {
        for line in [
            &b"not json"[..],
            br#"[1,2]"#,
            br#"{"id":1,"op":"ping"}"#,
            br#"{"v":1,"id":1}"#,
            br#"{"v":1,"id":[1],"op":"ping"}"#,
            br#"{"v":1,"id":1,"op":"ping","body":3}"#,
        ] {
            let error = parse_request(line).unwrap_err();
            assert_eq!(error.error["code"], ERR_BAD_REQUEST, "{line:?}");
        }
        let request = parse_request(br#"{"v":1,"id":"a","op":"x","body":null}"#).unwrap();
        assert_eq!(request.body, json!({}));
    }

    #[test]
    fn messages_roundtrip_between_encoder_and_parser() {
        let id = json!(9);
        let line = success_line(&id, &json!({"pid": 5}));
        assert_eq!(
            parse_message(&line[..line.len() - 1]),
            Some(Message::Response {
                id: id.clone(),
                outcome: Ok(json!({"pid": 5}))
            })
        );
        let error = language_error(&Error::new(ErrorCode::NotFound, "no schedule `s1`"));
        let line = failure_line(&id, &error);
        let Some(Message::Response {
            outcome: Err(error),
            ..
        }) = parse_message(&line)
        else {
            panic!("failure line must parse");
        };
        assert_eq!(error["code"], "E_NOT_FOUND");
        let line = event_line("schedule.finished", &json!({"x": 1}));
        assert!(
            matches!(parse_message(&line), Some(Message::Event { event, .. }) if event == "schedule.finished")
        );
        assert_eq!(parse_message(br#"{"v":7,"event":"x"}"#), None);
        assert_eq!(parse_message(br#"{"v":1,"id":1}"#), None);
        let request = request_line(&json!("r"), "status", &json!({}));
        assert_eq!(
            parse_request(&request[..request.len() - 1]).unwrap().op,
            "status"
        );
    }

    #[test]
    fn read_line_reports_eof_and_split_lines() {
        let mut reader = io::BufReader::with_capacity(4, &b"abcdefgh\npartial"[..]);
        assert_eq!(
            read_line(&mut reader, 64).unwrap(),
            LineRead::Line(b"abcdefgh".to_vec())
        );
        assert_eq!(read_line(&mut reader, 64).unwrap(), LineRead::Eof);
        let mut reader = io::BufReader::with_capacity(4, &b"abcdefgh\n"[..]);
        assert_eq!(read_line(&mut reader, 6).unwrap(), LineRead::TooLong);
    }
}
