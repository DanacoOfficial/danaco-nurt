// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Free space management for the Danaco Nurt journal.
//!
//! The crate measures free space on journal volumes, applies the reserve and
//! soft thresholds, decides per target whether previous content is preserved
//! (admission), and reclaims space held by expired or oldest journal entries.

// Tests that write POSIX paths run on Unix only; their helpers are unused on Windows.
#![cfg_attr(all(test, windows), allow(dead_code, unused_imports))]
#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod admission;
pub(crate) mod cost;
pub(crate) mod heartbeat;
pub(crate) mod measure;
pub(crate) mod reclaim;
pub mod service;
pub(crate) mod thresholds;
