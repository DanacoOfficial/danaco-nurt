// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-difftest
// Responsibility:   Differential tests of the Danaco Nurt Git commands.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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
//! | [`output`] | comparison of output records with the output of `git` |
//! | [`process`] | process service of the Nurt side for repository hooks |
//! | [`server`] | test server of the smart HTTP transport |
//! | [`ssh`] | test server of the SSH transport |

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
pub mod output;
pub mod process;
pub mod runner;
pub mod scenario;
pub mod server;
pub mod ssh;
pub mod stage;
pub mod time;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
