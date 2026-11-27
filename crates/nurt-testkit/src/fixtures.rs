// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Values used repeatedly by tests of effect stages.

use nurt_contracts::bound::{BoundDocument, BoundProgram};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{BeginRequest, Expect, OpKind, TargetAction, TargetPath, TargetSpec};

/// Catalog revision recorded in documents built by [`empty_program`].
pub const TEST_CATALOG_REV: &str = "test";

/// Correction table revision recorded in documents built by
/// [`empty_program`].
pub const TEST_CORRECTION_REV: &str = "test";

/// Bound document without statements, for journal requests whose program
/// does not matter to the test.
#[must_use]
pub fn empty_program() -> BoundDocument {
    BoundDocument::new(
        TEST_CATALOG_REV,
        TEST_CORRECTION_REV,
        BoundProgram {
            statements: Vec::new(),
        },
    )
}

/// Request for a transaction of the catalog command `command`: no session,
/// [`empty_program`], no corrections, not bulk, counted as a step, no
/// preservation budget.
///
/// ```
/// use nurt_testkit::fixtures::begin_request;
///
/// let request = begin_request("remove");
/// assert_eq!(request.kind.command, "remove");
/// assert!(request.counts_as_step);
/// ```
#[must_use]
pub fn begin_request(command: &str) -> BeginRequest {
    BeginRequest {
        kind: OpKind::command(command),
        session: None,
        program: empty_program(),
        corrections: Vec::new(),
        bulk: false,
        counts_as_step: true,
        preserve_budget: None,
    }
}

/// Declaration of a file system target without an expected state.
///
/// ```
/// use nurt_contracts::journal::{Expect, TargetAction};
/// use nurt_testkit::fixtures::fs_target;
/// use nurt_testkit::vfs::test_path;
///
/// let spec = fs_target(&test_path("/data/a.txt").unwrap(), TargetAction::Delete);
/// assert_eq!(spec.expect, Expect::Unchecked);
/// ```
#[must_use]
pub fn fs_target(path: &AbsPath, action: TargetAction) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Fs(path.clone()),
        action,
        expect: Expect::Unchecked,
        size_hint: None,
    }
}

/// Modification time, in nanoseconds since the Unix epoch, that is racy
/// (`nurt_contracts::fs::RACY_WINDOW`) however long the test runs: one hour
/// after the present.
#[must_use]
pub fn racy_time_ns() -> i64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        + std::time::Duration::from_secs(3600);
    i64::try_from(since.as_nanos()).unwrap_or(i64::MAX)
}
