// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Installed programs and external program catalogs of Danaco Nurt.
//!
//! The crate discovers programs installed on the computer from the search
//! path, the databases of package managers (dpkg, rpm, pacman, Homebrew,
//! winget, Scoop, Chocolatey) and the application registries of Windows and
//! macOS. Descriptions come from manual pages, package metadata and the help
//! output of a program. External catalogs add descriptions, categories and
//! usage notes in an open, documented format.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
