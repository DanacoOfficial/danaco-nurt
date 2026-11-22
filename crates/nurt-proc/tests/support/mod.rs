// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Location of the `proc_fixture` test program.

use std::path::PathBuf;

/// Path of the `proc_fixture` example, which `cargo test` builds next to the
/// test binaries.
pub(crate) fn fixture_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let target_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(PathBuf::from)
        .unwrap_or_default();
    target_dir
        .join("examples")
        .join(format!("proc_fixture{}", std::env::consts::EXE_SUFFIX))
}
