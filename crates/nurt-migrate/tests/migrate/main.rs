// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-migrate/tests/migrate/main.rs
// Responsibility:   Tests of script migration against a translator with fixed replies.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of script migration against a translator with fixed replies.

// Shared test helpers outside `#[test]` functions report failures by panicking.
#![allow(clippy::panic)]

mod behavior;
mod binding;
mod constructs;
mod foreign_text;
mod golden;
mod language;
mod mock;
mod scripts;
mod tables;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
