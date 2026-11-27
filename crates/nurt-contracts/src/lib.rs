// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-contracts
// Responsibility:   Frozen contracts of Danaco Nurt.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Frozen contracts of Danaco Nurt.
//!
//! This crate defines the interfaces shared by all crates of the workspace and
//! by external implementations of the language. Every type here is part of a
//! versioned, documented contract; changes within a major version only add
//! optional fields or new variants of enumerations marked as non-exhaustive.
//!
//! | Module | Contract |
//! |---|---|
//! | [`kinds`] | command categories, reversibility, effect kinds, implementation markers, syntax variants |
//! | [`value`] | values, units and ranges of the language |
//! | [`record`] | typed records that flow through pipelines |
//! | [`error`] | stable error codes and the error structure |
//! | [`ast`] | surface syntax tree `nurt-ast/1` |
//! | [`bound`] | bound syntax tree `nurt-bound/1` |
//! | [`spec`] | command and parameter specifications loaded from `catalog/*.toml` |
//! | [`stage`] | stage implementation interface, record streams and execution context |
//! | [`env`](mod@env) | clock and host interfaces |
//! | [`report`] | effect report `nurt-report/1` |
//! | [`envelope`] | request envelope `nurt-request/1` and response |
//! | [`journal`] | journal store, mutation transactions and admission |
//! | [`fs`] | file system interface |
//! | [`config`] | configuration schema |
//! | [`keypath`] | key paths into structured documents |
//! | [`services`] | services shared between command crates (documents, code, processes, file listing) |
//! | [`schema`] | published JSON schemas |
//! | [`product`] | product name, canonical description and version, help and banner texts |

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod ast;
pub mod bound;
pub mod config;
pub mod continuation;
pub mod env;
pub mod envelope;
pub mod error;
pub mod fs;
pub mod journal;
pub mod keypath;
pub mod kinds;
pub mod product;
pub mod record;
pub mod report;
pub mod schema;
pub mod services;
pub mod spec;
pub mod sql_text;
pub mod stage;
pub mod value;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
