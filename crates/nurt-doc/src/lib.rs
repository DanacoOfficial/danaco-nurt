// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Structured documents for Danaco Nurt.
//!
//! The crate reads and edits JSON, JSONC, TOML, YAML, XML, INI, CSV, property
//! lists and environment files while preserving layout, comments and
//! formatting of unchanged content, queries documents with `jq` filters and
//! runs SQL statements on SQLite databases.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod commands;
pub mod constructs;
pub mod edit;
pub mod formats;
pub mod jq;
pub mod model;
pub mod registry;
pub mod service;
pub mod sql;
pub mod style;
#[cfg(feature = "testing")]
pub mod testing;
pub mod value;
