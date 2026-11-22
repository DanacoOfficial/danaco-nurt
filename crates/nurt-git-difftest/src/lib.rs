// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Differential tests of the Danaco Nurt Git commands.
//!
//! Each scenario prepares a repository, runs the same operation with the
//! `git` program on one copy and with the Nurt command on another copy, and
//! compares the normalized repository state. The Nurt operation is then
//! undone through the journal and the copy is compared with the initial
//! state; `git fsck --strict` checks both copies. The `git` program is used
//! only by these tests; scenarios are skipped when it is not on the search
//! path.
//!
//! | Module | Role |
//! |---|---|
//! | [`scenario`] | scenario file format and loading |
//! | [`corpus`] | deterministic corpus of test repositories |
//! | [`normalize`] | normalized repository state read with `git` |
//! | [`compare`] | differences between states |
//! | [`journal`] | recording journal with undo and write audit |
//! | [`bind`] | parameter binding against `catalog/git.toml` and the test host |
//! | [`stage`] | execution of a stage implementation |
//! | [`runner`] | the whole scenario and the summary of a run |

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod bind;
pub mod compare;
pub mod corpus;
pub mod error;
pub mod fsutil;
pub mod git;
pub mod journal;
pub mod normalize;
pub mod runner;
pub mod scenario;
pub mod stage;
pub mod time;
