// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rendered definitions against their approved snapshots.
//!
//! The snapshots in `tests/golden/` show the exact output of every format
//! for the options of `common::options`. Set `NURT_BLESS=1` to rewrite them
//! after an intended change.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

// Each test crate uses a different part of the shared helpers.
#[allow(dead_code)]
mod common;

use std::fs;

use nurt_tooldef::{ToolFormat, render};

#[test]
fn rendered_definitions_equal_their_snapshots() {
    let bless = std::env::var_os("NURT_BLESS").is_some_and(|value| value == "1");
    let directory = common::crate_dir().join("tests/golden");
    for format in ToolFormat::ALL {
        let mut rendered = render(*format, &common::options()).unwrap();
        rendered.push('\n');
        let path = directory.join(format!("{}.json", format.name()));
        if bless {
            fs::create_dir_all(&directory).unwrap();
            fs::write(&path, &rendered).unwrap();
        }
        let approved = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "{}: {error}; run the tests with NURT_BLESS=1 to create it",
                path.display()
            )
        });
        assert_eq!(
            rendered,
            approved,
            "{format} differs from {}",
            path.display()
        );
    }
}
