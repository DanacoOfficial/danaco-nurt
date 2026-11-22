// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Command catalog of Danaco Nurt.
//!
//! The catalog is the registry of every command known to the engine. It loads
//! the command signatures from the `catalog/*.toml` data files, validates them
//! against the catalog contract, resolves command names and group membership,
//! and exposes the stage specifications used by binding, planning and help.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
