// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Test support for Danaco Nurt.
//!
//! The crate provides a virtual file system, test doubles of the journal, clock
//! and host interfaces, and a runner for golden files. It depends only on the
//! contracts, so every crate of the workspace can use it in its tests.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
