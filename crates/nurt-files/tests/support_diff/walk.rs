// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/support_diff/walk.rs
// Responsibility:   Directory walks on the real file system for differential and performance tests.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Directory walks on the real file system for differential and performance tests.

// Each test target links a different subset of the helpers; failures abort the test.
#![allow(dead_code, clippy::expect_used)]

use std::ops::ControlFlow;
use std::path::Path;

use nurt_contracts::fs::AbsPath;
use nurt_files::walk::{WalkItem, WalkOptions, Walker};

use crate::support_fs::DiskFs;

/// Relative paths delivered by a walk of `root`, in walk order.
pub(crate) fn walk_relative(root: &Path, options: WalkOptions) -> Vec<String> {
    let mut paths = Vec::new();
    walk_each(root, options, |path| paths.push(path));
    paths
}

/// Number of entries delivered by a walk of `root`.
pub(crate) fn walk_count(root: &Path, options: WalkOptions) -> u64 {
    let mut count = 0u64;
    walk_each(root, options, |_| count += 1);
    count
}

fn walk_each(root: &Path, options: WalkOptions, mut each: impl FnMut(String)) {
    let walker = Walker::new(options).expect("walk options are valid");
    let root = AbsPath::new(root).expect("corpus root is absolute");
    walker
        .run(&DiskFs, std::slice::from_ref(&root), &|| Ok(()), |item| {
            if let WalkItem::Entry(entry) = item {
                each(entry.relative_string());
            }
            ControlFlow::Continue(())
        })
        .expect("walk succeeds");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
