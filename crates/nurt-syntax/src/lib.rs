// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Syntax of Danaco Nurt programs.
//!
//! The crate contains the lexer, the parsers of the three notation variants,
//! the canonical printer used by `nurt fmt`, the JSON and CBOR codecs of the
//! `nurt-ast/1` syntax tree, the canonical tree hash and diagnostics with
//! source positions.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod cbor;
pub mod error;
pub mod fmt;
pub mod hash;
pub mod json;
pub mod lex;
pub mod parse;
pub mod print;
pub mod schema;
pub mod validate;
