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
//! | [`journal`] | pass-through journal `NoopJournal`, `RecordingJournal`, test admission |
//! | [`clock`] | virtual clock driven by the test |
//! | [`host`] | host facts and a private environment |
//! | [`services`] | service doubles for document, code, program and listing services |
//! | [`ctx`] | execution contexts assembled from the doubles |
//! | [`golden`] | golden file runner with JSON value comparison and blessing |
//! | [`fixtures`] | journal requests and target declarations |
//! | [`hash`] | SHA-256 of content |

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod clock;
pub mod ctx;
pub mod fixtures;
pub mod golden;
pub mod hash;
pub mod host;
pub mod journal;
pub mod services;
pub mod vfs;
