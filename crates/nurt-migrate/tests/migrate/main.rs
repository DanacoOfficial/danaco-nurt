// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of script migration against a translator with fixed replies.

// Shared test helpers outside `#[test]` functions report failures by panicking.
#![allow(clippy::panic)]

mod behavior;
mod binding;
mod foreign_text;
mod golden;
mod language;
mod mock;
mod scripts;
mod tables;
