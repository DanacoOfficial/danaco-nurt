// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface tests in Python: `tests/abi/python/abi_test.py` (feature
//! `stub`) or `tests/abi/python/engine_test.py` (engine of `nurt-core`)
//! loads the shared library with `ctypes`. The tests are skipped when
//! `python3` is not in `PATH`.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test fails at the first unexpected result"
)]

mod common;

use std::process::Command;

#[cfg(feature = "stub")]
#[test]
fn python_script_passes_the_status_matrix_and_interface_checks() {
    let Some(python) = common::find_program("python3") else {
        return;
    };
    let script = common::manifest_dir().join("tests/abi/python/abi_test.py");
    let output = common::run(
        Command::new(python)
            .arg("-I")
            .arg(script)
            .arg(common::shared_library())
            .env_remove("NURT_STUB_DIR")
            .env_remove("NURT_STUB_PANIC"),
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
}

#[cfg(not(feature = "stub"))]
#[test]
fn python_script_passes_the_interface_checks_on_the_engine() {
    let Some(python) = common::find_program("python3") else {
        return;
    };
    let work = tempfile::tempdir().unwrap();
    let script = common::manifest_dir().join("tests/abi/python/engine_test.py");
    let output = common::run_isolated(
        Command::new(python)
            .arg("-I")
            .arg(script)
            .arg(common::shared_library()),
        work.path(),
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
}
