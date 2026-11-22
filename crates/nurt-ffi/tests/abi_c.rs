// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface test in C: `tests/abi/c/abi_test.c` is compiled with `clang`
//! against `nurt.h`, linked with the shared library and run. The test is
//! skipped when `clang` is not in `PATH`.

#![cfg(all(feature = "stub", unix))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test fails at the first unexpected result"
)]

mod common;

use std::process::Command;

#[test]
fn c_program_passes_the_status_matrix_and_interface_checks() {
    let Some(clang) = common::find_program("clang") else {
        return;
    };
    let work = tempfile::tempdir().unwrap();
    let program = work.path().join("abi_test");
    let source = common::manifest_dir().join("tests/abi/c/abi_test.c");
    common::compile_c(&clang, &[source], &program, &[]);
    let output = common::run(
        Command::new(&program)
            .env_remove("NURT_STUB_DIR")
            .env_remove("NURT_STUB_PANIC"),
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
}
