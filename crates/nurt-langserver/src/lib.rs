// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Language server for Danaco Nurt program files.
//!
//! The server implements the Language Server Protocol 3.17 over JSON-RPC.
//! It keeps the open program files in memory, publishes the error of the
//! parser as a diagnostic with its code, position and corrected programs,
//! formats a document into its canonical text, and completes and describes
//! command names, parameter labels, enumeration values and `@` references
//! from the command catalog.
//!
//! Positions are exchanged in UTF-32 when the client offers it, because the
//! columns of syntax trees count Unicode scalar values, and in UTF-16
//! otherwise.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod analysis;
mod assist;
mod catalog;
mod context;
mod document;
mod options;
mod position;
mod server;

pub use options::Options;
pub use server::{ServerError, run_stdio, serve};
