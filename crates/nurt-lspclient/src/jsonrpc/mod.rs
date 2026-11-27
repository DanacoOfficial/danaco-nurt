// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-lspclient::jsonrpc
// Responsibility:   JSON-RPC 2.0 messages.
// Dependencies:     serde, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! JSON-RPC 2.0 messages.
//!
//! A message is classified by its members: `method` with `id` is a request,
//! `method` without `id` is a notification, `id` with `result` or `error` is
//! a response. Language servers send requests to the client as well, so the
//! classification does not depend on the direction.
//!
//! # Examples
//!
//! ```
//! use nurt_lspclient::jsonrpc::{Message, RequestId};
//! use serde_json::json;
//!
//! let message = Message::from_value(json!({"jsonrpc": "2.0", "id": 3, "result": null}))?;
//! assert_eq!(message, Message::Response { id: RequestId::Number(3), result: Ok(json!(null)) });
//! # Ok::<(), nurt_lspclient::jsonrpc::MessageError>(())
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Error code of a request that names an unknown method.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// Error code of a request cancelled by the client.
pub const REQUEST_CANCELLED: i64 = -32800;
/// Error code of a request whose result is out of date because the content
/// changed.
pub const CONTENT_MODIFIED: i64 = -32801;

/// Identifier of a request.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    /// Numeric identifier.
    Number(i64),
    /// Text identifier.
    Text(String),
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(number) => write!(f, "{number}"),
            Self::Text(text) => f.write_str(text),
        }
    }
}

/// Error member of a response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    /// Error code.
    pub code: i64,
    /// Message of the server.
    pub message: String,
    /// Additional data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    /// Error with the given code and message.
    #[must_use]
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)
    }
}

/// One JSON-RPC message.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Request that expects a response.
    Request {
        /// Identifier echoed by the response.
        id: RequestId,
        /// Method name.
        method: String,
        /// Parameters; `null` when absent.
        params: Value,
    },
    /// Notification without a response.
    Notification {
        /// Method name.
        method: String,
        /// Parameters; `null` when absent.
        params: Value,
    },
    /// Response to a request.
    Response {
        /// Identifier of the request.
        id: RequestId,
        /// Result or error.
        result: Result<Value, RpcError>,
    },
}

/// Message that does not follow JSON-RPC 2.0.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid JSON-RPC message: {reason}")]
pub struct MessageError {
    reason: &'static str,
}

impl MessageError {
    const fn new(reason: &'static str) -> Self {
        Self { reason }
    }
}

impl Message {
    /// Request with the given identifier.
    #[must_use]
    pub fn request(id: RequestId, method: impl Into<String>, params: Value) -> Self {
        Self::Request {
            id,
            method: method.into(),
            params,
        }
    }

    /// Notification.
    #[must_use]
    pub fn notification(method: impl Into<String>, params: Value) -> Self {
        Self::Notification {
            method: method.into(),
            params,
        }
    }

    /// Successful response.
    #[must_use]
    pub fn response(id: RequestId, result: Value) -> Self {
        Self::Response {
            id,
            result: Ok(result),
        }
    }

    /// Classifies a decoded JSON value.
    ///
    /// # Errors
    ///
    /// Fails when the value is not an object, the method is not text, the
    /// identifier is neither a number nor text, or a response has neither
    /// `result` nor `error`.
    pub fn from_value(value: Value) -> Result<Self, MessageError> {
        let Value::Object(mut object) = value else {
            return Err(MessageError::new("the message is not an object"));
        };
        let id =
            match object.remove("id") {
                None | Some(Value::Null) => None,
                Some(raw) => Some(serde_json::from_value::<RequestId>(raw).map_err(|_| {
                    MessageError::new("the identifier is neither a number nor text")
                })?),
            };
        match object.remove("method") {
            Some(Value::String(method)) => {
                let params = object.remove("params").unwrap_or(Value::Null);
                Ok(match id {
                    Some(id) => Self::Request { id, method, params },
                    None => Self::Notification { method, params },
                })
            }
            Some(_) => Err(MessageError::new("the method is not text")),
            None => Self::response_from(id, object),
        }
    }

    fn response_from(
        id: Option<RequestId>,
        mut object: Map<String, Value>,
    ) -> Result<Self, MessageError> {
        let id = id.ok_or(MessageError::new("a response has no identifier"))?;
        if let Some(error) = object.remove("error") {
            let error = serde_json::from_value::<RpcError>(error)
                .map_err(|_| MessageError::new("the error member has no code or message"))?;
            return Ok(Self::Response {
                id,
                result: Err(error),
            });
        }
        match object.remove("result") {
            Some(result) => Ok(Self::Response {
                id,
                result: Ok(result),
            }),
            None => Err(MessageError::new("a response has neither result nor error")),
        }
    }

    /// JSON form with the `jsonrpc` member.
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Request { id, method, params } => {
                json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
            }
            Self::Notification { method, params } => {
                json!({"jsonrpc": "2.0", "method": method, "params": params})
            }
            Self::Response {
                id,
                result: Ok(result),
            } => {
                json!({"jsonrpc": "2.0", "id": id, "result": result})
            }
            Self::Response {
                id,
                result: Err(error),
            } => {
                json!({"jsonrpc": "2.0", "id": id, "error": error})
            }
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
