// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::serve::protocol
// Responsibility:   Messages of `nurt-rpc/1` and the codes of their errors.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Messages of `nurt-rpc/1` and the codes of their errors.
//!
//! Every message is a JSON-RPC 2.0 object. Errors of the protocol use the
//! codes reserved by JSON-RPC; errors of the application use one code for
//! each status of the C interface, `-32000 - status`, so that a client maps
//! them to the same handling as the library.

use nurt_contracts::error::AbiStatus;
use serde_json::{Map, Value, json};

/// Name and major version of the protocol, as exchanged in `initialize`.
pub const PROTOCOL: &str = "nurt-rpc/1";

/// The message is not JSON, or its framing is broken.
pub const PARSE_ERROR: i64 = -32700;
/// The message is not a valid request object, or the request is not
/// allowed in the current state of the connection.
pub const INVALID_REQUEST: i64 = -32600;
/// The method does not exist.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// The parameters do not match the method.
pub const INVALID_PARAMS: i64 = -32602;
/// The server failed to process the request.
pub const INTERNAL_ERROR: i64 = -32603;

/// Version of JSON-RPC carried by every message.
pub(crate) const JSONRPC: &str = "2.0";
/// Method of event notifications.
pub(crate) const EVENT_METHOD: &str = "nurt/event";
/// Application code of the status `NURT_OK`; no error carries it.
const APPLICATION_BASE: i64 = -32000;

/// Application error code of a status of the C interface:
/// `-32000 - status`, for example `-32005` for `NURT_E_IO`.
///
/// # Examples
///
/// ```
/// use nurt_cli::serve::application_code;
/// use nurt_contracts::error::AbiStatus;
///
/// assert_eq!(application_code(AbiStatus::Cancelled), -32002);
/// assert_eq!(application_code(AbiStatus::Internal), -32006);
/// ```
#[must_use]
pub const fn application_code(status: AbiStatus) -> i64 {
    APPLICATION_BASE - status.code() as i64
}

/// Status of the C interface for an application error code; `None` for
/// codes outside the application range and for `-32000`.
///
/// # Examples
///
/// ```
/// use nurt_cli::serve::status_for_code;
/// use nurt_contracts::error::AbiStatus;
///
/// assert_eq!(status_for_code(-32003), Some(AbiStatus::Arg));
/// assert_eq!(status_for_code(-32601), None);
/// ```
#[must_use]
pub fn status_for_code(code: i64) -> Option<AbiStatus> {
    AbiStatus::ALL
        .iter()
        .copied()
        .find(|status| *status != AbiStatus::Ok && application_code(*status) == code)
}

/// Error object of a reply.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RpcError {
    pub(crate) code: i64,
    pub(crate) message: String,
    pub(crate) data: Map<String, Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: Map::new(),
        }
    }

    /// `-32700`: the message is not JSON or its framing is broken.
    pub(crate) fn parse(message: impl Into<String>) -> Self {
        Self::new(PARSE_ERROR, message)
    }

    /// `-32600`: the message is not a valid request, or the request is not
    /// allowed now.
    pub(crate) fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(INVALID_REQUEST, message)
    }

    /// `-32601` for `method`, listing the known methods.
    pub(crate) fn method_not_found(method: &str, known: &[&str]) -> Self {
        Self::new(
            METHOD_NOT_FOUND,
            format!("unknown method `{method}`; known: {}", known.join(", ")),
        )
    }

    /// `-32602`: the parameters do not match the method.
    pub(crate) fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message)
    }

    /// `-32603`: the server failed to process the request.
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(INTERNAL_ERROR, message)
    }

    /// Application error with the code of `status`; `data.status` names the
    /// status, for example `NURT_E_IO`.
    pub(crate) fn status(status: AbiStatus, message: impl Into<String>) -> Self {
        Self::new(application_code(status), message).with_data("status", status.name().into())
    }

    /// `NURT_E_CANCELLED` carrying the response of the cancelled request in
    /// `data.response`.
    pub(crate) fn cancelled(response: Value) -> Self {
        Self::status(AbiStatus::Cancelled, "the request was cancelled")
            .with_data("response", response)
    }

    /// Adds the member `name` to `data`.
    pub(crate) fn with_data(mut self, name: &str, value: Value) -> Self {
        self.data.insert(name.to_owned(), value);
        self
    }

    fn to_json(&self) -> Value {
        let mut error = Map::new();
        error.insert("code".to_owned(), Value::from(self.code));
        error.insert("message".to_owned(), Value::from(self.message.as_str()));
        if !self.data.is_empty() {
            error.insert("data".to_owned(), Value::Object(self.data.clone()));
        }
        Value::Object(error)
    }
}

/// Request or notification received from the client.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Message {
    /// Identifier of a request; `None` for a notification.
    pub(crate) id: Option<Value>,
    pub(crate) method: String,
    pub(crate) params: Option<Value>,
}

/// What a received message turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Incoming {
    /// A request or a notification.
    Message(Message),
    /// A reply sent by the client; the server sends no requests and drops it.
    Ignored,
    /// An invalid message, with the error reply to send.
    Rejected(Value),
}

/// Reads one message.
pub(crate) fn parse_message(bytes: &[u8]) -> Incoming {
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(error) => {
            return rejected(
                Value::Null,
                &RpcError::parse(format!("the message is not JSON: {error}")),
            );
        }
    };
    let Value::Object(mut object) = value else {
        let message = if value.is_array() {
            "batch messages are not supported; send one request per message"
        } else {
            "the message must be a JSON object"
        };
        return rejected(Value::Null, &RpcError::invalid_request(message));
    };
    let id = object.remove("id");
    if let Some(id) = &id
        && !(id.is_string() || id.is_number() || id.is_null())
    {
        return rejected(
            Value::Null,
            &RpcError::invalid_request("`id` must be a string, a number or null"),
        );
    }
    match read_request(&object) {
        Ok(Some((method, params))) => Incoming::Message(Message { id, method, params }),
        Ok(None) => Incoming::Ignored,
        Err(error) => rejected(id.unwrap_or(Value::Null), &error),
    }
}

/// Method and parameters of a request object; `None` for a reply.
fn read_request(object: &Map<String, Value>) -> Result<Option<(String, Option<Value>)>, RpcError> {
    if object.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC) {
        return Err(RpcError::invalid_request("`jsonrpc` must be \"2.0\""));
    }
    let Some(method) = object.get("method") else {
        if object.contains_key("result") || object.contains_key("error") {
            return Ok(None);
        }
        return Err(RpcError::invalid_request("`method` is missing"));
    };
    let Some(method) = method.as_str() else {
        return Err(RpcError::invalid_request("`method` must be a string"));
    };
    let params = match object.get("params") {
        None | Some(Value::Null) => None,
        Some(params @ (Value::Object(_) | Value::Array(_))) => Some(params.clone()),
        Some(_) => {
            return Err(RpcError::invalid_request(
                "`params` must be an object or an array",
            ));
        }
    };
    Ok(Some((method.to_owned(), params)))
}

fn rejected(id: Value, error: &RpcError) -> Incoming {
    Incoming::Rejected(error_message(id, error))
}

/// Successful reply to the request `id`.
pub(crate) fn result_message(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": JSONRPC, "id": id, "result": result })
}

/// Error reply to the request `id`.
pub(crate) fn error_message(id: Value, error: &RpcError) -> Value {
    json!({ "jsonrpc": JSONRPC, "id": id, "error": error.to_json() })
}

/// Reply to the request `id` with the outcome of its method.
pub(crate) fn reply(id: Value, outcome: Result<Value, RpcError>) -> Value {
    match outcome {
        Ok(result) => result_message(id, result),
        Err(error) => error_message(id, &error),
    }
}

/// Notification sent by the server.
pub(crate) fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": JSONRPC, "method": method, "params": params })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rejected_error(bytes: &[u8]) -> (Value, Value) {
        match parse_message(bytes) {
            Incoming::Rejected(reply) => (reply["id"].clone(), reply["error"].clone()),
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[test]
    fn every_failure_status_has_its_own_application_code() {
        let expected = [
            (AbiStatus::Timeout, -32001),
            (AbiStatus::Cancelled, -32002),
            (AbiStatus::Arg, -32003),
            (AbiStatus::Abi, -32004),
            (AbiStatus::Io, -32005),
            (AbiStatus::Internal, -32006),
        ];
        for (status, code) in expected {
            assert_eq!(application_code(status), code);
            assert_eq!(status_for_code(code), Some(status));
        }
        assert_eq!(status_for_code(-32000), None);
        assert_eq!(status_for_code(INTERNAL_ERROR), None);
        assert_eq!(AbiStatus::ALL.len(), expected.len() + 1);
    }

    #[test]
    fn status_errors_name_the_status() {
        let error = RpcError::status(AbiStatus::Io, "cannot read").with_data("path", json!("/x"));
        assert_eq!(
            error.to_json(),
            json!({"code": -32005, "message": "cannot read", "data": {"status": "NURT_E_IO", "path": "/x"}})
        );
        assert_eq!(
            RpcError::internal("x").to_json(),
            json!({"code": INTERNAL_ERROR, "message": "x"})
        );
        let cancelled = RpcError::cancelled(json!({"report": {}}));
        assert_eq!(cancelled.code, -32002);
        assert_eq!(cancelled.data["response"], json!({"report": {}}));
    }

    #[test]
    fn requests_and_notifications_are_read() {
        let request = parse_message(br#"{"jsonrpc":"2.0","id":"a","method":"info","params":{}}"#);
        assert_eq!(
            request,
            Incoming::Message(Message {
                id: Some(json!("a")),
                method: "info".to_owned(),
                params: Some(json!({})),
            })
        );
        let notification = parse_message(br#"{"jsonrpc":"2.0","method":"cancel","params":null}"#);
        assert_eq!(
            notification,
            Incoming::Message(Message {
                id: None,
                method: "cancel".to_owned(),
                params: None,
            })
        );
        let null_id = parse_message(br#"{"jsonrpc":"2.0","id":null,"method":"info"}"#);
        assert!(matches!(
            null_id,
            Incoming::Message(Message {
                id: Some(Value::Null),
                ..
            })
        ));
    }

    #[test]
    fn replies_of_the_client_are_ignored() {
        assert_eq!(
            parse_message(br#"{"jsonrpc":"2.0","id":1,"result":null}"#),
            Incoming::Ignored
        );
        assert_eq!(
            parse_message(br#"{"jsonrpc":"2.0","id":1,"error":{"code":1,"message":"x"}}"#),
            Incoming::Ignored
        );
    }

    #[test]
    fn invalid_messages_are_rejected_with_their_code() {
        let (id, error) = rejected_error(b"{not json");
        assert_eq!(
            (id, error["code"].clone()),
            (Value::Null, json!(PARSE_ERROR))
        );
        let (_, error) = rejected_error(br#"[{"jsonrpc":"2.0","id":1,"method":"info"}]"#);
        assert_eq!(error["code"], INVALID_REQUEST);
        assert!(error["message"].as_str().unwrap().contains("batch"));
        let (_, error) = rejected_error(b"7");
        assert_eq!(error["code"], INVALID_REQUEST);
        let (id, error) = rejected_error(br#"{"jsonrpc":"2.0","id":{},"method":"info"}"#);
        assert_eq!(
            (id, error["code"].clone()),
            (Value::Null, json!(INVALID_REQUEST))
        );
        let (id, error) = rejected_error(br#"{"jsonrpc":"1.0","id":4,"method":"info"}"#);
        assert_eq!(
            (id, error["code"].clone()),
            (json!(4), json!(INVALID_REQUEST))
        );
        let (_, error) = rejected_error(br#"{"jsonrpc":"2.0","id":4}"#);
        assert!(error["message"].as_str().unwrap().contains("missing"));
        let (_, error) = rejected_error(br#"{"jsonrpc":"2.0","id":4,"method":7}"#);
        assert!(error["message"].as_str().unwrap().contains("string"));
        let (_, error) = rejected_error(br#"{"jsonrpc":"2.0","id":4,"method":"info","params":3}"#);
        assert!(error["message"].as_str().unwrap().contains("params"));
    }

    #[test]
    fn parameters_may_be_an_array_and_numbers_may_be_fractions() {
        let request =
            parse_message(br#"{"jsonrpc":"2.0","id":1.5,"method":"info","params":[1,2]}"#);
        assert_eq!(
            request,
            Incoming::Message(Message {
                id: Some(json!(1.5)),
                method: "info".to_owned(),
                params: Some(json!([1, 2])),
            })
        );
    }

    #[test]
    fn a_message_without_the_version_is_rejected_with_its_identifier() {
        let (id, error) = rejected_error(br#"{"id":3,"method":"info"}"#);
        assert_eq!(
            (id, error["code"].clone()),
            (json!(3), json!(INVALID_REQUEST))
        );
        let (_, error) = rejected_error(br#"{"jsonrpc":2,"id":3,"result":null}"#);
        assert_eq!(error["code"], INVALID_REQUEST);
    }

    #[test]
    fn invalid_utf8_and_empty_messages_are_parse_errors() {
        for bytes in [&b"\xff\xfe"[..], b"", b"   "] {
            let (id, error) = rejected_error(bytes);
            assert_eq!(
                (id, error["code"].clone()),
                (Value::Null, json!(PARSE_ERROR))
            );
        }
    }

    #[test]
    fn identifiers_of_the_wrong_type_are_rejected_with_a_null_identifier() {
        for id in ["true", "[1]", "{\"a\":1}"] {
            let text = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"info"}}"#);
            let (reply_id, error) = rejected_error(text.as_bytes());
            assert_eq!(reply_id, Value::Null, "{id}");
            assert_eq!(error["code"], INVALID_REQUEST, "{id}");
        }
    }

    #[test]
    fn a_method_name_with_multi_byte_characters_is_kept() {
        let text = "{\"jsonrpc\":\"2.0\",\"method\":\"\u{142}\u{f3}d\u{17a}\"}";
        assert_eq!(
            parse_message(text.as_bytes()),
            Incoming::Message(Message {
                id: None,
                method: "\u{142}\u{f3}d\u{17a}".to_owned(),
                params: None,
            })
        );
    }

    #[test]
    fn the_success_status_has_the_base_code_and_unknown_codes_have_no_status() {
        assert_eq!(application_code(AbiStatus::Ok), -32000);
        assert_eq!(status_for_code(-32007), None);
        assert_eq!(status_for_code(0), None);
        assert_eq!(status_for_code(i64::MIN), None);
        assert_eq!(status_for_code(i64::MAX), None);
    }

    #[test]
    fn errors_with_data_serialize_the_members_in_a_data_object() {
        let error = RpcError::invalid_params("bad").with_data("path", json!("/x"));
        assert_eq!(
            error_message(json!("a"), &error),
            json!({"jsonrpc": "2.0", "id": "a", "error": {"code": INVALID_PARAMS, "message": "bad", "data": {"path": "/x"}}})
        );
        let plain = RpcError::invalid_request("no");
        assert_eq!(
            plain.to_json(),
            json!({"code": INVALID_REQUEST, "message": "no"})
        );
    }

    #[test]
    fn messages_have_the_shape_of_json_rpc() {
        assert_eq!(
            reply(json!(1), Ok(json!(true))),
            json!({"jsonrpc": "2.0", "id": 1, "result": true})
        );
        assert_eq!(
            reply(
                json!(2),
                Err(RpcError::method_not_found("x", &["info", "run"]))
            ),
            json!({"jsonrpc": "2.0", "id": 2, "error": {"code": METHOD_NOT_FOUND, "message": "unknown method `x`; known: info, run"}})
        );
        assert_eq!(
            notification(EVENT_METHOD, json!({"t": "log"})),
            json!({"jsonrpc": "2.0", "method": "nurt/event", "params": {"t": "log"}})
        );
        assert_eq!(RpcError::parse("x").code, PARSE_ERROR);
        assert_eq!(RpcError::invalid_params("x").code, INVALID_PARAMS);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
