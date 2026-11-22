// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Messages of the server protocol `nurt-rpc/1` used by the suite.
//!
//! Every name of a method, parameter and result member that the suite relies
//! on is defined here.

use nurt_contracts::envelope::ENVELOPE_VERSION;
use nurt_contracts::error::AbiStatus;
use serde_json::{Map, Value, json};

/// Name and version of the protocol.
pub const PROTOCOL: &str = "nurt-rpc/1";

/// Version of the JSON-RPC protocol.
pub const JSONRPC: &str = "2.0";

/// Method names of `nurt-rpc/1`.
pub mod method {
    /// Opens the session and negotiates versions.
    pub const INITIALIZE: &str = "initialize";
    /// Describes the implementation.
    pub const INFO: &str = "info";
    /// Parses program text into a surface tree.
    pub const PARSE: &str = "parse";
    /// Prints a surface tree as program text.
    pub const PRINT: &str = "print";
    /// Binds a request without executing it.
    pub const CHECK: &str = "check";
    /// Corrects program text.
    pub const CORRECT: &str = "correct";
    /// Executes a request.
    pub const RUN: &str = "run";
    /// Returns a further page of a result.
    pub const PAGE: &str = "page";
    /// Returns a published schema.
    pub const SCHEMA: &str = "schema";
    /// Cancels a request in progress or a background job.
    pub const CANCEL: &str = "cancel";
    /// Ends the session.
    pub const SHUTDOWN: &str = "shutdown";
    /// Notification carrying an event.
    pub const EVENT: &str = "nurt/event";
    /// Moves the clock of an implementation with the feature `testkit`.
    pub const CLOCK_ADVANCE: &str = "testkit.clock.advance";
    /// Prefix of the methods of the feature `testkit`. An implementation
    /// may offer only some of them; a case whose call of such a method is
    /// answered with [`super::code::METHOD_NOT_FOUND`] is skipped.
    pub const TESTKIT_PREFIX: &str = "testkit.";
}

/// Members of parameters and results.
pub mod member {
    /// Surface tree in the result of `parse` and in the parameters of `print`.
    pub const AST: &str = "ast";
    /// Bound tree in the result of `check`.
    pub const BOUND: &str = "bound";
    /// Hash of the bound tree in the result of `check`.
    pub const AST_HASH: &str = "ast_hash";
    /// Program text in the result of `print`.
    pub const TEXT: &str = "text";
    /// Program text in parameters.
    pub const PROGRAM: &str = "program";
    /// Syntax variant in parameters.
    pub const VARIANT: &str = "variant";
    /// Request options.
    pub const OPTIONS: &str = "options";
    /// Working directory option of a request.
    pub const BASE: &str = "base";
    /// Version member of documents.
    pub const VERSION: &str = "nurt";
    /// Language error in the results of `parse` and `check`.
    pub const ERROR: &str = "error";
    /// Feature list in the result of `initialize`.
    pub const FEATURES: &str = "features";
    /// Language version in the result of `initialize`.
    pub const LANGUAGE: &str = "language";
    /// Diagnostics in the results of `parse` and `check`.
    pub const DIAGNOSTICS: &str = "diagnostics";
    /// Name of the schema in the parameters of `schema`.
    pub const NAME: &str = "name";
    /// Subject in the parameters of `describe`.
    pub const SUBJECT: &str = "subject";
    /// Request identifier in the parameters of `cancel`.
    pub const REQUEST_ID: &str = "request_id";
    /// Whether a request was cancelled, in the result of `cancel`.
    pub const CANCELLED: &str = "cancelled";
    /// Identifier of the request as text, in an event.
    pub const REQUEST: &str = "request";
    /// Name of the call status in the data of an error.
    pub const STATUS: &str = "status";
    /// Duration in the parameters of `testkit.clock.advance`.
    pub const BY: &str = "by";
    /// Time of the clock in the result of `testkit.clock.advance`.
    pub const NOW: &str = "now";
}

/// JSON-RPC error codes.
pub mod code {
    /// The message is not valid JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// The message is not a valid request.
    pub const INVALID_REQUEST: i64 = -32600;
    /// The method does not exist.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// The parameters are not valid.
    pub const INVALID_PARAMS: i64 = -32602;
    /// Internal error of the server.
    pub const INTERNAL_ERROR: i64 = -32603;
    /// Base of the codes of the call statuses `NURT_E_*`: the code is this
    /// value minus the status number, between `-32001` and `-32099`.
    pub const STATUS_BASE: i64 = -32000;
}

/// Syntax variants of program text.
pub const VARIANTS: [&str; 3] = ["a", "b", VARIANT_C];

/// Syntax variant that writes every argument by name.
pub const VARIANT_C: &str = "c";

/// Default syntax variant.
pub const DEFAULT_VARIANT: &str = "a";

/// Returns the JSON-RPC error code of a call status other than `NURT_OK`,
/// which is never reported as an error.
#[must_use]
pub fn status_code(status: AbiStatus) -> i64 {
    code::STATUS_BASE - i64::from(status.code())
}

/// Returns the call status of `nurt.h` with the given name; `NURT_OK` is not
/// an error status and is not returned.
#[must_use]
pub fn status_by_name(name: &str) -> Option<AbiStatus> {
    AbiStatus::ALL
        .iter()
        .copied()
        .find(|status| *status != AbiStatus::Ok && status.name() == name)
}

/// Returns the identifier of a request as the text that an event carries in
/// `request`: a string as it is, a number in decimal, `null` for no
/// identifier.
#[must_use]
pub fn request_text(id: &Value) -> Value {
    match id {
        Value::String(_) | Value::Null => id.clone(),
        other => Value::String(other.to_string()),
    }
}

/// Returns the parameters of `initialize`.
#[must_use]
pub fn initialize_params(client: &str, version: &str, events: bool) -> Value {
    json!({
        "protocol": PROTOCOL,
        "client": {"name": client, "version": version},
        "capabilities": {"events": events},
    })
}

/// Returns a request `nurt-request/1` for program text.
///
/// The variant is written only when it differs from the default; the options
/// only when they are not empty.
#[must_use]
pub fn program_request(program: &str, variant: &str, options: Option<&Value>) -> Value {
    let mut request = Map::new();
    request.insert(member::VERSION.to_owned(), json!(ENVELOPE_VERSION));
    request.insert(member::PROGRAM.to_owned(), json!(program));
    if variant != DEFAULT_VARIANT {
        request.insert(member::VARIANT.to_owned(), json!(variant));
    }
    if let Some(options) = options.filter(|o| o.as_object().is_some_and(|m| !m.is_empty())) {
        request.insert(member::OPTIONS.to_owned(), options.clone());
    }
    Value::Object(request)
}

/// Returns the parameters of `parse` and `correct`.
#[must_use]
pub fn text_params(program: &str, variant: &str) -> Value {
    json!({member::PROGRAM: program, member::VARIANT: variant})
}

/// Returns the parameters of `print`.
#[must_use]
pub fn print_params(ast: &Value, variant: &str) -> Value {
    json!({member::AST: ast, member::VARIANT: variant})
}

/// Whether the parameters of the method are a request `nurt-request/1`
/// whose options receive the working directory.
#[must_use]
pub fn takes_request(method: &str) -> bool {
    method == method::RUN || method == method::CHECK
}

/// Sets `options.base` of a request unless the options name it already. A
/// `null` base marks a request that is sent without one and is removed.
pub fn set_base(request: &mut Value, base: &str) {
    let Some(members) = request.as_object_mut() else {
        return;
    };
    let options = members
        .entry(member::OPTIONS)
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(options) = options.as_object_mut() {
        if options.get(member::BASE).is_some_and(Value::is_null) {
            options.remove(member::BASE);
        } else {
            options
                .entry(member::BASE)
                .or_insert_with(|| Value::String(base.to_owned()));
        }
    }
}

/// Builds a JSON-RPC request.
#[must_use]
pub fn request(id: u64, method: &str, params: &Value) -> Value {
    json!({"jsonrpc": JSONRPC, "id": id, "method": method, "params": params})
}

/// Builds a successful JSON-RPC response.
#[must_use]
pub fn response(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": JSONRPC, "id": id, "result": result})
}

/// Builds a JSON-RPC error response. `data` is left out when it is `null`,
/// because the protocol defines it as an object.
#[must_use]
pub fn error_response(id: &Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code": code, "message": message});
    if let Some(data) = data.filter(|data| !data.is_null()) {
        error["data"] = data;
    }
    json!({"jsonrpc": JSONRPC, "id": id, "error": error})
}

/// Builds a JSON-RPC notification.
#[must_use]
pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": JSONRPC, "method": method, "params": params})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schemas::SchemaSet;

    #[test]
    fn status_codes_follow_the_status_numbers() {
        assert_eq!(status_code(AbiStatus::Arg), -32003);
        assert_eq!(status_by_name("NURT_E_IO"), Some(AbiStatus::Io));
        assert_eq!(status_by_name("NURT_E_NONE"), None);
        assert_eq!(status_by_name("NURT_OK"), None);
    }

    #[test]
    fn status_errors_are_valid_replies_of_the_protocol_schema() {
        let schemas = SchemaSet::shared().unwrap();
        for status in AbiStatus::ALL
            .iter()
            .copied()
            .filter(|s| *s != AbiStatus::Ok)
        {
            let code = status_code(status);
            assert!((-32099..=-32001).contains(&code), "{status:?}");
            let reply =
                error_response(&json!(1), code, "m", Some(json!({"status": status.name()})));
            assert_eq!(schemas.validate_message(&reply), Ok(()), "{status:?}");
        }
        let unknown = error_response(&json!(1), -32003, "m", Some(json!({"status": "NURT_X"})));
        assert!(schemas.validate_message(&unknown).is_err());
        let no_data = error_response(&json!(1), code::INTERNAL_ERROR, "m", Some(Value::Null));
        assert_eq!(schemas.validate_message(&no_data), Ok(()));
    }

    #[test]
    fn requests_of_every_method_follow_the_protocol_schema() {
        let schemas = SchemaSet::shared().unwrap();
        let program = program_request("count", "a", None);
        let tree: Value = serde_json::from_str(include_str!(
            "../../../spec/golden/ast/example-01-largest-go-files.json"
        ))
        .unwrap();
        let calls = [
            (method::INITIALIZE, initialize_params("c", "1", true)),
            (method::RUN, program.clone()),
            (method::CHECK, program),
            (method::PARSE, text_params("count", "b")),
            (method::CORRECT, text_params("count", "c")),
            (method::PRINT, print_params(&tree, "a")),
            (method::PAGE, json!({"cursor": "c", "limit": 5})),
            (method::SCHEMA, json!({member::NAME: "report"})),
            (method::CANCEL, json!({member::REQUEST_ID: 2})),
            (method::INFO, json!({})),
            (method::SHUTDOWN, json!({})),
        ];
        for (name, params) in calls {
            let message = request(7, name, &params);
            assert_eq!(schemas.validate_message(&message), Ok(()), "{name}");
        }
        let unknown = request(7, "nurt.unknown", &json!({}));
        assert!(schemas.validate_message(&unknown).is_err());
    }

    #[test]
    fn request_identifiers_become_event_text() {
        assert_eq!(request_text(&json!(5)), json!("5"));
        assert_eq!(request_text(&json!("r")), json!("r"));
        assert_eq!(request_text(&Value::Null), Value::Null);
    }

    #[test]
    fn program_requests_omit_defaults() {
        assert_eq!(
            program_request("count", "a", Some(&json!({}))),
            json!({"nurt": "1.0", "program": "count"})
        );
        let mut request = program_request("count", "b", Some(&json!({"limit": 5})));
        set_base(&mut request, "/w");
        set_base(&mut request, "/other");
        assert_eq!(
            request,
            json!({"nurt": "1.0", "program": "count", "variant": "b", "options": {"limit": 5, "base": "/w"}})
        );
        let mut bare = json!({"nurt": "1.0", "cursor": "c"});
        set_base(&mut bare, "/w");
        assert_eq!(bare["options"]["base"], "/w");
        let mut unset = json!({"nurt": "1.0", "program": "x", "options": {"base": null}});
        set_base(&mut unset, "/w");
        assert_eq!(unset["options"], json!({}));
        let mut scalar = json!(1);
        set_base(&mut scalar, "/w");
        assert_eq!(scalar, json!(1));
    }

    #[test]
    fn messages_have_the_json_rpc_shape() {
        assert_eq!(request(3, "info", &json!({}))["jsonrpc"], "2.0");
        assert_eq!(response(&json!(3), json!({}))["id"], 3);
        let error = error_response(
            &json!(1),
            code::INVALID_PARAMS,
            "bad",
            Some(json!({"x": 1})),
        );
        assert_eq!(error["error"]["data"]["x"], 1);
        assert!(
            error_response(&json!(1), -1, "m", None)["error"]
                .get("data")
                .is_none()
        );
        assert_eq!(
            notification(method::EVENT, json!({}))["method"],
            "nurt/event"
        );
        assert!(takes_request("run") && takes_request("check") && !takes_request("parse"));
        assert_eq!(text_params("x", "c")["variant"], "c");
        assert_eq!(print_params(&json!({}), "a")["variant"], "a");
        assert_eq!(
            initialize_params("c", "1", true)["capabilities"]["events"],
            true
        );
    }
}
