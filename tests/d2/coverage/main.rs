// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Journal coverage tests of catalog commands.
//!
//! The coverage table `tables/journal-coverage.toml` states for every command
//! and action of the catalog how the journal preserves the previous state.
//! These tests check the table against the catalog and run the undo and redo
//! round trip of every row.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions report failures by panicking"
)]

mod cases;
mod catalog;
mod consistency;
#[allow(
    dead_code,
    reason = "the harness is shared by several test targets, each of which uses a subset"
)]
mod harness;
mod journal_rows;
mod procedure;
mod table;
