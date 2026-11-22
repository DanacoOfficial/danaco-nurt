// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Installed programs and external program catalogs for Danaco Nurt.
//!
//! The crate implements the `program` command group. Providers read the
//! search path, package databases (dpkg, rpm, pacman, Homebrew, winget,
//! Scoop, Chocolatey), registered applications, manual pages and external
//! catalogs in the format `nurt-program-catalog/1`. The index merges their
//! entries into program cards, keeps them in a cache and ranks them for
//! `program.search`. Providers only read files and registry keys; the only
//! program ever started is the one whose `--help` is requested explicitly.

#![deny(missing_docs)]
#![deny(unsafe_code)]

pub mod catalog;
pub mod check;
pub mod help;
pub mod index;
pub mod model;
pub mod search;
pub mod source;
pub mod sources;
pub mod stages;
pub mod taxonomy;
pub mod text;
#[cfg(windows)]
pub mod win;
