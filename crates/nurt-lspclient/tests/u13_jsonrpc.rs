// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the JSON-RPC 2.0 message model: construction, classification,
//! JSON form and display of identifiers and errors.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_lspclient::jsonrpc::{
    CONTENT_MODIFIED, METHOD_NOT_FOUND, Message, REQUEST_CANCELLED, RequestId, RpcError,
};
use serde_json::{Value, json};

#[test]
fn request_identifiers_display_as_number_or_text() {
    assert_eq!(RequestId::Number(42).to_string(), "42");
    assert_eq!(RequestId::Text("cfg-1".to_owned()).to_string(), "cfg-1");
    assert_eq!(json!(RequestId::Number(7)), json!(7));
    assert_eq!(json!(RequestId::Text("a".to_owned())), json!("a"));
}

#[test]
fn rpc_error_displays_its_message_and_code() {
    let error = RpcError::new(METHOD_NOT_FOUND, "unsupported method textDocument/rename");
    assert_eq!(error.data, None);
    assert_eq!(
        error.to_string(),
        "unsupported method textDocument/rename (code -32601)"
    );
    assert_eq!(
        json!(error),
        json!({"code": -32601, "message": "unsupported method textDocument/rename"})
    );
}

#[test]
fn error_codes_have_the_protocol_values() {
    assert_eq!(METHOD_NOT_FOUND, -32601);
    assert_eq!(REQUEST_CANCELLED, -32800);
    assert_eq!(CONTENT_MODIFIED, -32801);
}

#[test]
fn notification_has_no_identifier_in_its_json_form() {
    let note = Message::notification("exit", Value::Null);
    assert_eq!(
        note.to_value(),
        json!({"jsonrpc": "2.0", "method": "exit", "params": null})
    );
    assert_eq!(Message::from_value(note.to_value()).unwrap(), note);
}

#[test]
fn successful_response_round_trips_through_its_json_form() {
    let response = Message::response(RequestId::Number(5), json!([1, 2]));
    let value = response.to_value();
    assert_eq!(value, json!({"jsonrpc": "2.0", "id": 5, "result": [1, 2]}));
    assert_eq!(Message::from_value(value).unwrap(), response);
}

#[test]
fn error_response_round_trips_with_its_data() {
    let mut error = RpcError::new(CONTENT_MODIFIED, "content modified");
    error.data = Some(json!({"retry": true}));
    let response = Message::Response {
        id: RequestId::Text("r-1".to_owned()),
        result: Err(error),
    };
    let value = response.to_value();
    assert_eq!(
        value,
        json!({"jsonrpc": "2.0", "id": "r-1", "error": {"code": -32801, "message": "content modified", "data": {"retry": true}}})
    );
    assert_eq!(Message::from_value(value).unwrap(), response);
}

#[test]
fn null_result_is_a_successful_response() {
    let message = Message::from_value(json!({"jsonrpc": "2.0", "id": 1, "result": null})).unwrap();
    assert_eq!(
        message,
        Message::response(RequestId::Number(1), Value::Null)
    );
}

#[test]
fn null_identifier_with_a_method_is_a_notification() {
    let message =
        Message::from_value(json!({"jsonrpc": "2.0", "id": null, "method": "$/progress"})).unwrap();
    assert_eq!(message, Message::notification("$/progress", Value::Null));
}

#[test]
fn response_with_a_malformed_error_member_is_rejected() {
    let error = Message::from_value(json!({"id": 3, "error": {"message": "no code"}})).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid JSON-RPC message: the error member has no code or message"
    );
}

#[test]
fn rejected_messages_name_the_reason() {
    let cases = [
        (json!("text"), "the message is not an object"),
        (json!({"method": 1}), "the method is not text"),
        (
            json!({"id": [1], "result": 1}),
            "the identifier is neither a number nor text",
        ),
        (json!({"result": 1}), "a response has no identifier"),
        (json!({"id": 1}), "a response has neither result nor error"),
    ];
    for (value, reason) in cases {
        let error = Message::from_value(value).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("invalid JSON-RPC message: {reason}")
        );
    }
}
