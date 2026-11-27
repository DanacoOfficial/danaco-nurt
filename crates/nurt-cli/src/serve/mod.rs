// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::serve
// Responsibility:   Protocol server started by `nurt serve`.
// Dependencies:     framing, protocol, server
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Protocol server started by `nurt serve`.
//!
//! The server implements the `nurt-rpc/1` protocol: JSON-RPC 2.0 messages
//! framed as newline-delimited JSON (or with `Content-Length` headers) on a
//! pair of byte streams. Requests are executed through the engine API, each
//! on its own thread, so several requests may be in flight and their replies
//! may arrive in any order. Standard output carries protocol messages only.
//!
//! [`run`] serves one connection until `shutdown` or the end of the input.
//! With the feature `stub`, [`ServeOptions::stub`] answers from the recorded
//! cases of the library stub instead of an engine.

mod backend;
mod call;
mod framing;
mod protocol;
mod server;
#[cfg(feature = "stub")]
mod stub;

pub use framing::Framing;
pub use protocol::{
    INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, PROTOCOL,
    application_code, status_for_code,
};
pub use server::{DEFAULT_MAX_INFLIGHT, DEFAULT_MAX_MESSAGE_BYTES, ServeOptions, run};

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
