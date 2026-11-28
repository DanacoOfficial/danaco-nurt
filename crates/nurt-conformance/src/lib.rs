// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-conformance
// Responsibility:   Conformance suite of Danaco Nurt.
// Dependencies:     error
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Conformance suite of Danaco Nurt.
//!
//! The suite checks an implementation of the language through the server
//! protocol `nurt-rpc/1` only, so implementations in any language can be
//! tested. The crate reads the case files of the `conformance/` directory,
//! runs them against a server process and writes the result in the format
//! `nurt-conformance-report/1`.
//!
//! | Module | Content |
//! |---|---|
//! | [`case`] | case format `nurt-conformance-case/1` and loading of a suite |
//! | [`snapshot`] | file system snapshot `nurt-fs-snapshot/1` |
//! | [`compare`] | comparison of expected and actual values, `ast_hash` |
//! | [`normalize`] | normalization of volatile values |
//! | [`schemas`] | published schemas and validation |
//! | [`protocol`] | messages of `nurt-rpc/1` used by the suite |
//! | [`client`] | connection to a server process |
//! | [`execute`] | execution of one case on a connection |
//! | [`runner`] | execution of a suite and the conformance report |
//! | [`report`] | conformance report `nurt-conformance-report/1` |
//! | [`binding`] | binding cases `nurt-binding-cases/1` |
//! | [`fake`] | fake server answering from the cases |
//! | [`fixture`] | local HTTP server, second volume and mount point for cases |

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod binding;
pub mod case;
pub mod client;
pub mod compare;
pub mod execute;
pub mod fake;
pub mod fixture;
pub mod normalize;
pub mod protocol;
pub mod report;
pub mod runner;
pub mod schemas;
pub mod snapshot;

mod error;
mod pointer;

pub use error::Error;

/// Name of the file holding the version of the suite in the case directory.
pub const VERSION_FILE: &str = "VERSION";

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
