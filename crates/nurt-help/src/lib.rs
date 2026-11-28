// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-help
// Responsibility:   Built-in help and documentation layers of Danaco Nurt.
// Dependencies:     text
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Built-in help and documentation layers of Danaco Nurt.
//!
//! The crate renders the `help` command, the help references attached to
//! errors and the description layers for tool integrations, all generated
//! from the command catalog and the documentation pages. It also implements
//! the commands `help`, `describe` and `evaluate`.
//!
//! - [`help::render`] answers a [`help::HelpQuery`].
//! - [`describe::layer`] generates the core, group, guide and reference
//!   layers in English or Polish and fits them to a token budget.
//! - [`errors::help_reference`] and [`help::group_note`] produce the lines
//!   that point a result at the help.
//! - [`translate::localize`] puts the messages and notes of a report into
//!   Polish.
//! - [`consistency::check_program`] checks the names in a program text
//!   against the catalog.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod catalog;
pub mod commands;
pub mod consistency;
pub mod describe;
pub mod docs;
pub mod errors;
pub mod help;
mod lookup;
pub mod reference;
pub mod signature;
mod text;
pub mod tokens;
pub mod translate;
pub mod variant;

pub use text::{TAGLINE, first_line};

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
