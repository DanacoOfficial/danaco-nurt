// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Values shared by the tests of the common building blocks.

use nurt_contracts::fs::AbsPath;
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::test_path;

/// Absolute path of the virtual file system for a path written with `/`.
pub(crate) fn abs(text: &str) -> AbsPath {
    test_path(text).expect("test paths are absolute")
}

/// Test context whose file system holds one file.
pub(crate) fn with_file(path: &str, content: &[u8]) -> (TestContext, AbsPath) {
    let test = TestContext::new();
    let path = abs(path);
    test.fs()
        .write_file(&path, content)
        .expect("the virtual file system accepts the file");
    (test, path)
}
