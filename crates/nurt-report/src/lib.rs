// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Effect report of Danaco Nurt.
//!
//! The crate builds `nurt-report/1` reports from planned and executed effects
//! and renders them as text, JSON, Markdown and compact output. It applies the
//! rules for notable effects, irreversibility and bulk summaries.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod assemble;
pub mod rules;
