// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Settings and operations shared by the retention scenarios, and the checks
//! that end every scenario.

use std::time::Duration;

use nurt_contracts::config::{AgeLimit, Oversize, SizeLimit, StepLimit};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    Expect, Mutation, MutationTx, OpHandle, OpId, OpState, SweepReport, TargetAction, TargetIndex,
    TargetPath, TargetSpec,
};

use crate::harness::fixture::{Fixture, FixtureOptions};
use crate::harness::request;
use crate::leak;

/// Step limit of the scenarios.
pub(crate) const STEPS: u32 = 30;
/// Age limit of the scenarios.
pub(crate) const AGE: Duration = Duration::from_secs(30 * 60);
/// One minute.
pub(crate) const MINUTE: Duration = Duration::from_secs(60);
/// One second.
pub(crate) const SECOND: Duration = Duration::from_secs(1);

/// Options of the scenarios: 30 steps, 30 minutes, 20 GiB, oversized
/// operations skipped, a 500 GB volume and a running space service.
pub(crate) fn options() -> FixtureOptions {
    let mut options = FixtureOptions::default();
    options.journal.max_ops = StepLimit::Limited(STEPS);
    options.journal.max_age = "30m".parse::<AgeLimit>().expect("age limit is valid");
    options.journal.max_size = "20gib".parse::<SizeLimit>().expect("size limit is valid");
    options.journal.oversize = Oversize::Skip;
    options
}

/// Options with every limit of one dimension removed.
pub(crate) fn unlimited(steps: bool, age: bool, size: bool) -> FixtureOptions {
    let mut options = options();
    if steps {
        options.journal.max_ops = StepLimit::Unlimited;
    }
    if age {
        options.journal.max_age = AgeLimit::Unlimited;
    }
    if size {
        options.journal.max_size = SizeLimit::Unlimited;
    }
    options
}

fn target(path: &nurt_contracts::fs::AbsPath, action: TargetAction) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Fs(path.clone()),
        action,
        expect: Expect::Unchecked,
        size_hint: None,
    }
}

/// Overwrites the small file `work/<index>.txt`; one step.
pub(crate) fn small_write(fixture: &Fixture, index: usize) -> OpId {
    let path = fixture.seed_file(&format!("work/{index}.txt"), b"before\n");
    let mut tx: Box<dyn MutationTx + '_> = fixture
        .journal
        .begin(request::begin("write", Some("overwrite")));
    tx.preserve(&[target(&path, TargetAction::Modify)])
        .expect("target is preserved");
    tx.apply(TargetIndex(0), Mutation::WriteBytes(b"after\n".to_vec()))
        .expect("change is applied");
    let op = tx
        .commit()
        .expect("transaction commits")
        .op
        .expect("a committed change has a number");
    retention_pass(fixture);
    op
}

/// Retention pass that the space service runs after changes; returns the
/// entries it expired.
pub(crate) fn retention_pass(fixture: &Fixture) -> SweepReport {
    fixture.journal.sweep().expect("retention pass runs")
}

/// Replaces the content of a sparse file of `size` bytes, so that its whole
/// previous content must be kept.
pub(crate) fn large_write(fixture: &Fixture, name: &str, size: u64) -> OpHandle {
    let path = fixture.seed_sparse(&format!("work/{name}"), size);
    let mut tx: Box<dyn MutationTx + '_> =
        fixture.journal.begin(request::begin("write", Some("data")));
    tx.preserve(&[TargetSpec {
        size_hint: Some(size),
        ..target(&path, TargetAction::Modify)
    }])
    .expect("target is preserved");
    tx.apply(TargetIndex(0), Mutation::WriteBytes(b"changed".to_vec()))
        .expect("change is applied");
    let handle = tx.commit().expect("transaction commits");
    retention_pass(fixture);
    handle
}

/// Removes a sparse file of `size` bytes; the journal keeps it by rename.
pub(crate) fn large_remove(fixture: &Fixture, name: &str, size: u64) -> OpId {
    large_remove_swept(fixture, name, size).0
}

/// Like [`large_remove`], also returning the entries the retention pass after
/// the removal expired.
pub(crate) fn large_remove_swept(fixture: &Fixture, name: &str, size: u64) -> (OpId, SweepReport) {
    let path = fixture.seed_sparse(&format!("work/{name}"), size);
    remove_path_swept(fixture, &path)
}

/// Removes an existing file; the journal keeps it by rename.
pub(crate) fn remove_path(fixture: &Fixture, path: &AbsPath) -> OpId {
    remove_path_swept(fixture, path).0
}

/// Like [`remove_path`], also returning the entries the retention pass after
/// the removal expired.
pub(crate) fn remove_path_swept(fixture: &Fixture, path: &AbsPath) -> (OpId, SweepReport) {
    let mut tx: Box<dyn MutationTx + '_> = fixture.journal.begin(request::begin("remove", None));
    tx.preserve(&[target(path, TargetAction::Delete)])
        .expect("target is preserved");
    tx.apply(TargetIndex(0), Mutation::Remove)
        .expect("change is applied");
    let op = tx
        .commit()
        .expect("transaction commits")
        .op
        .expect("a committed change has a number");
    (op, retention_pass(fixture))
}

/// State of an entry in `history`.
pub(crate) fn state(fixture: &Fixture, op: OpId) -> OpState {
    fixture.record(op).state
}

/// Asserts the state of every listed entry.
pub(crate) fn assert_states(fixture: &Fixture, ops: &[OpId], expected: OpState) {
    for op in ops {
        assert_eq!(state(fixture, *op), expected, "state of entry {op:?}");
    }
}

/// Checks that `journal.status` agrees with `history`: the counts of
/// available and expired entries and the oldest available entry. `history`
/// covers every journal volume, including the one that holds the user
/// configuration file, so the states of all volumes are summed.
pub(crate) fn status_matches_history(fixture: &Fixture) {
    let states = fixture
        .journal
        .status(None)
        .expect("journal status is readable");
    let status = Totals {
        entries_available: states.iter().map(|state| state.entries_available).sum(),
        entries_expired: states.iter().map(|state| state.entries_expired).sum(),
        bytes_held: states.iter().map(|state| state.bytes_held).sum(),
        oldest: states
            .iter()
            .filter_map(|state| state.oldest.as_ref().map(|entry| entry.op))
            .min(),
    };
    let records = fixture.history();
    let available: Vec<_> = records
        .iter()
        .filter(|record| matches!(record.state, OpState::Available | OpState::Partial))
        .collect();
    let expired = records
        .iter()
        .filter(|record| record.state == OpState::Expired)
        .count();
    assert_eq!(
        status.entries_available,
        u64::try_from(available.len()).expect("count fits"),
        "available entries"
    );
    assert_eq!(
        status.entries_expired,
        u64::try_from(expired).expect("count fits"),
        "expired entries: {:?}",
        records
            .iter()
            .map(|record| (record.id, record.kind.as_str(), record.state))
            .collect::<Vec<_>>()
    );
    let oldest = available.iter().map(|record| OpId(record.id)).min();
    assert_eq!(status.oldest, oldest, "oldest available entry");
    if available.is_empty() {
        assert_eq!(status.bytes_held, 0, "no bytes held without entries");
    }
}

/// Counts of `journal.status` summed over the journal volumes.
struct Totals {
    entries_available: u64,
    entries_expired: u64,
    bytes_held: u64,
    oldest: Option<OpId>,
}

/// Ends a scenario: `journal.status` agrees with `history`, and no result
/// carries an expiry time.
pub(crate) fn finish(fixture: &Fixture) {
    status_matches_history(fixture);
    leak::history_carries_no_expiry(fixture);
}
