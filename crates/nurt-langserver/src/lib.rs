// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-langserver
// Responsibility:   Language server for Danaco Nurt program files.
// Dependencies:     options, server
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Language server for Danaco Nurt program files.
//!
//! The server implements the Language Server Protocol 3.17 over JSON-RPC.
//! It keeps the open program files in memory and publishes their
//! diagnostics: the corrections of notation with their rule, class and
//! replaced text, and the errors of reading and binding with their code,
//! position, ready fixes and candidates. Quick fixes apply a correction, a
//! fix or a candidate. The server formats a document into its canonical
//! text, completes command names, parameter labels, enumeration values,
//! fields of the input records and `@` references, shows the signature of
//! the command being written, describes commands, parameters and error codes
//! on hover and classifies the tokens of a document for highlighting.
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
mod fields;
mod options;
mod position;
mod semantic;
mod server;
mod signature;

pub use options::Options;
pub use server::{ServerError, run_stdio, serve};

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
