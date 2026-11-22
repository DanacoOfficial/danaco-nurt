// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Differential tests of the Danaco Nurt Git commands.
//!
//! Each scenario prepares a repository, runs the same operation with the
//! `git` program on one copy and with the Nurt command on another copy, and
//! compares the normalized repository state. The `git` program is used only
//! by these tests; scenarios are skipped when it is not on the search path.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
