// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-ffi/tests/symbols.rs
// Responsibility:   The list `nurt.symbols` matches the header and the exports of the shared library.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The list `nurt.symbols` matches the header and the exports of the shared
//! library.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test fails at the first unexpected result"
)]

mod common;

#[test]
fn symbol_list_matches_the_header() {
    let header = common::header_functions();
    let listed = common::listed_symbols();
    assert_eq!(listed.len(), 29);
    assert_eq!(header, listed);
}

#[test]
fn status_table_of_the_header_names_declared_functions() {
    let declared = common::header_functions();
    for function in common::header_matrix().keys() {
        assert!(declared.contains(function), "{function} is not declared");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn shared_library_exports_exactly_the_listed_symbols() {
    use std::collections::BTreeSet;
    use std::process::Command;

    let Some(nm) = common::find_program("nm") else {
        return;
    };
    let library = common::shared_library();
    let output = common::run(
        Command::new(nm)
            .args(["-D", "--defined-only"])
            .arg(&library),
    );
    let exported: BTreeSet<String> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace().skip(1);
            let kind = fields.next()?;
            let name = fields.next()?;
            (kind == "T" && name.starts_with("nurt_")).then(|| name.to_owned())
        })
        .collect();
    assert_eq!(exported, common::listed_symbols());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
