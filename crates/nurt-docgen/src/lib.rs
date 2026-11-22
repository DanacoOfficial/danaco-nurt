// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Generator of the Danaco Nurt documentation.
//!
//! The generator builds the documentation site (static HTML), manual pages and
//! Markdown from one source: the command catalog (`catalog/*.toml`), the
//! command pages (`docs/commands/*.md`), the general pages (`docs/site/`), the
//! specification (`spec/`), the cookbook (`docs/cookbook/`) and the correction
//! tables behind the cheat sheets (`docs/cheatsheets/`). It also checks that
//! the documentation agrees with the catalog.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod catalog;
pub mod cheatsheet;
pub mod check;
pub mod cli;
pub mod consistency;
pub mod cookbook;
pub mod error;
pub mod examples;
pub mod html;
pub mod layout;
pub mod lexer;
pub mod man;
pub mod manpages;
pub mod markdown;
pub mod page;
pub mod reference;
pub mod site;
pub mod tasks;

pub use error::{DocgenError, Result};
