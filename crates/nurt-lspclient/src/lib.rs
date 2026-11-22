// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Language Server Protocol client for Danaco Nurt.
//!
//! The client starts language servers on demand, keeps one server per server
//! and project root, synchronizes open documents with the file system and
//! implements [`nurt_code::semantic::SemanticProvider`].

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod config;
pub mod diagnostics;
pub mod jsonrpc;
pub mod pool;
pub mod progress;
pub mod provider;
pub mod rename;
pub mod server;
pub mod sync;
pub mod transport;
