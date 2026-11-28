// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git
// Responsibility:   Git repository operations for Danaco Nurt.
// Dependencies:     registry
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Git repository operations for Danaco Nurt.
//!
//! The crate implements status, differences, history, blame, staging, commits,
//! branches, tags, checkpoints, work trees, fetch, clone and the remaining
//! repository operations in Rust, without running an external program.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod apply;
pub mod blame;
pub mod branch;
pub mod checkpoint;
pub mod clone;
pub mod commit;
pub mod diff;
pub mod fetch;
pub mod hooks;
pub mod hunk_select;
pub mod log;
pub mod message;
pub mod query;
pub mod refs_tx;
pub mod repo;
pub mod review;
pub mod revwalk;
pub mod show;
pub mod stage;
pub mod status;
pub mod tag;
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;
pub mod worktree;

mod registry;

pub use registry::register;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
