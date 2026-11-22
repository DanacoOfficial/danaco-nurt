// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Protocol server started by `nurt serve`.
//!
//! The server implements the `nurt-rpc/1` protocol: JSON-RPC 2.0 messages
//! framed as newline-delimited JSON on standard input and output. Requests
//! are executed through the engine API.
