// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Pipeline executor of Danaco Nurt.
//!
//! The executor runs execution plans as streams of records with parallelism,
//! cancellation, entity tags, assertions and verification, and implements the
//! record stages `filter`, `select`, `sort`, `first`, `last`, `skip`,
//! `count`, `assert`, `verify`, `group`, `summarize`, `distinct`, `parse`
//! and `split`.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod stages;
