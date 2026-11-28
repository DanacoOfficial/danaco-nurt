// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::revwalk
// Responsibility:   History traversal limited to paths, with history simplification and rename following.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! History traversal limited to paths, with history simplification and rename following.
//!
//! The traversal reproduces the default order and filtering of `git log`:
//! commits are visited newest first by committer time, ties in the order
//! they were reached; commits reachable from an excluded revision are left
//! out; with paths, commits that do not change them are hidden and a merge
//! that equals one of its parents within the paths is followed through that
//! parent only.

pub(crate) mod args;
pub(crate) mod changes;
pub(crate) mod combined;
pub(crate) mod graph;
pub(crate) mod lines;
pub(crate) mod moves;
pub(crate) mod paths;
pub(crate) mod record;
pub(crate) mod repo;
pub(crate) mod spec;
pub(crate) mod walk;

#[cfg(test)]
pub(crate) mod fixture;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
