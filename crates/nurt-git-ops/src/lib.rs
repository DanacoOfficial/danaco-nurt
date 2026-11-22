// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Working tree changing Git operations of Danaco Nurt.
//!
//! The crate implements `switch`, `restore`, `reset`, `merge`, `rebase`,
//! `stash`, `pull` and `push` in Rust on top of `gix`, without running an
//! external program. Every change of a working tree file goes through a
//! journal mutation transaction, so the exact previous content is preserved
//! and each operation can be undone individually.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
