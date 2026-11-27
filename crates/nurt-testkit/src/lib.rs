// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test support for Danaco Nurt.
//!
//! The crate provides deterministic doubles of the interfaces a stage uses
//! and a runner for golden files. It depends only on the contracts, so every
//! crate of the workspace can use it in its tests.
//!
//! | Module | Content |
//! |---|---|
//! | [`vfs`] | in-memory file system implementing `Fs` and `FsWrite`, with fault injection |
//! | [`journal`] | pass-through and restoring journals with a register of transactions, test admission |
//! | [`clock`] | virtual clock driven by the test, local time by time zone rules |
//! | [`host`] | host facts and a private environment |
//! | [`services`] | service doubles, empty and with data, including scripted programs |
//! | [`ctx`] | execution contexts assembled from the doubles |
//! | [`snapshot`] | file tree snapshots in the `nurt-fs-snapshot/1` format |
//! | [`golden`] | golden file and case directory runners with JSON comparison and blessing |
//! | [`fs_contract`] | conformance checks of `Fs` and `FsWrite` implementations |
//! | [`fixtures`] | journal requests and target declarations |
//! | [`hash`] | SHA-256 of content |
//! | [`paths`] | absolute paths of the tests on the current system |

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod clock;
pub mod ctx;
pub mod fixtures;
pub mod fs_contract;
pub mod golden;
pub mod hash;
pub mod host;
pub mod journal;
pub mod paths;
pub mod services;
pub mod snapshot;
pub mod vfs;
