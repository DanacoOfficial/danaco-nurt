// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/coverage/procedure.rs
// Responsibility:   Coverage procedure of one table row, run by a stand-in command that drives.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Coverage procedure of one table row, run by a stand-in command that drives
//! the transaction as the command of the row does: it declares one target per
//! preservation mode of the row, preserves, applies the changes and commits.
//!
//! The procedure checks that undo restores the tree exactly and that redo
//! restores the state after the operation, that every target is preserved by
//! a mode of the row, that a fully covered row reports nothing irreversible
//! and that other rows report exactly their reasons, and that a read changes
//! nothing and opens no transaction. Mutations outside a transaction are
//! detected by the auditing file system of the fixture.

use nurt_contracts::fs::{
    AbsPath, LinkKind, MetaChange, ProviderKey, ProviderScheme, ProviderValue,
};
use nurt_contracts::journal::{
    Expect, ExternalChange, ExternalChangeKind, JournalNotable, Mutation, MutationTx, OpHandle,
    PreserveMode, TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};

use crate::cases::{Case, Rows, Variant};
use crate::harness::clock::START;
use crate::harness::fixture::{Fixture, FixtureOptions, VolumeSpec, path_below};
use crate::harness::request;
use crate::table::{self, Coverage, CoverageTable, Row};

/// Modes that write new content and keep the old one under another name.
const CONTENT_MODES: [PreserveMode; 5] = [
    PreserveMode::Exchange,
    PreserveMode::ReplaceBackup,
    PreserveMode::Hardlink,
    PreserveMode::Clone,
    PreserveMode::Copy,
];

/// Capacity of the volume of cases without space for a copy.
const SMALL_VOLUME: u64 = 64 << 20;

/// Size of the file whose copy does not fit on the small volume.
const LARGE_FILE: u64 = 48 << 20;

/// Zero bytes added by an extension of a file.
const EXTENSION_LEN: usize = 4096;

/// Provider scheme registered by the fixture for value targets.
const TEST_PROVIDER: &str = "test";

/// Runs the procedure for every row of a table case.
///
/// # Panics
///
/// Panics when a check fails or the case does not name table rows.
pub(crate) fn round_trip(case: &Case) {
    let table = table::load();
    let Rows::Table(keys) = case.rows else {
        panic!("{}: the round trip needs table rows", case.name);
    };
    for &(command, action) in keys {
        let row = table
            .find(command, action)
            .unwrap_or_else(|| panic!("{}: no row {command} {action:?}", case.name));
        let fixture = Fixture::open(options(case.variant));
        match row.coverage {
            Coverage::NoEffect => read_changes_nothing(&fixture),
            Coverage::Full | Coverage::Partial => {
                change_round_trips(&fixture, &table, row, case.variant);
            }
            Coverage::Uncovered => effect_is_marked(&fixture, &table, row),
        }
    }
}

fn options(variant: Variant) -> FixtureOptions {
    let mut options = FixtureOptions::default();
    match variant {
        Variant::NoSpace => {
            options.volumes = vec![VolumeSpec {
                capacity: SMALL_VOLUME,
                used: SMALL_VOLUME - LARGE_FILE - (LARGE_FILE / 4),
            }];
        }
        Variant::SecondVolume => options.volumes.push(VolumeSpec::STANDARD),
        Variant::LowerOverlayLayer => options.lower_layer = true,
        _ => {}
    }
    options
}

/// One declared target with its change.
struct Step {
    spec: TargetSpec,
    change: Change,
}

enum Change {
    Apply(Mutation),
    External(AbsPath),
}

fn target(path: &AbsPath, action: TargetAction) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Fs(path.clone()),
        action,
        expect: Expect::Unchecked,
        size_hint: None,
    }
}

fn applied(path: &AbsPath, action: TargetAction, mutation: Mutation) -> Step {
    Step {
        spec: target(path, action),
        change: Change::Apply(mutation),
    }
}

fn has(row: &Row, mode: PreserveMode) -> bool {
    row.preserve.contains(&mode)
}

fn keeps_content(row: &Row) -> bool {
    CONTENT_MODES.iter().any(|mode| has(row, *mode))
}

/// A program or library writes the targets itself: the row keeps content by
/// clone or copy and may lose it.
fn written_externally(row: &Row) -> bool {
    has(row, PreserveMode::None) && (has(row, PreserveMode::Clone) || has(row, PreserveMode::Copy))
}

fn count(variant: Variant) -> usize {
    match variant {
        Variant::Bulk(files) => files,
        _ => 1,
    }
}

fn plan(fixture: &Fixture, row: &Row, variant: Variant) -> Vec<Step> {
    let mut steps = Vec::new();
    let action = row.action.as_deref();
    if has(row, PreserveMode::Created) && !written_externally(row) {
        steps.extend(created_steps(fixture, action, variant));
    }
    if written_externally(row) {
        let size = if variant == Variant::NoSpace {
            LARGE_FILE
        } else {
            1
        };
        let path = fixture.seed_sparse("work/external.bin", size);
        steps.push(Step {
            spec: target(&path, TargetAction::ExternalWrite),
            change: Change::External(path),
        });
    } else if keeps_content(row) || has(row, PreserveMode::Ref) {
        steps.extend(content_steps(fixture, action, variant));
    }
    if has(row, PreserveMode::Append) {
        let path = fixture.seed_file("work/append.txt", b"line 1\n");
        steps.push(if action == Some("extend") {
            applied(
                &path,
                TargetAction::Append,
                Mutation::AppendBytes(vec![0; EXTENSION_LEN]),
            )
        } else {
            applied(
                &path,
                TargetAction::Append,
                Mutation::AppendBytes(b"line 2\n".to_vec()),
            )
        });
    }
    if has(row, PreserveMode::Moved) {
        let path = fixture.seed_tree("work/removed", count(variant).max(2));
        steps.push(applied(&path, TargetAction::Delete, Mutation::Remove));
    }
    if has(row, PreserveMode::Rename) {
        steps.push(rename_step(fixture, row, variant));
    }
    if has(row, PreserveMode::Meta) {
        let path = fixture.seed_file("work/meta.txt", b"metadata\n");
        let change = MetaChange {
            modified_ns: Some(i64::try_from(START.unix_nanos()).unwrap_or(i64::MAX)),
            ..MetaChange::default()
        };
        steps.push(applied(
            &path,
            TargetAction::Attributes,
            Mutation::SetMeta(change),
        ));
    }
    if has(row, PreserveMode::Value) {
        let key = ProviderKey {
            scheme: ProviderScheme(TEST_PROVIDER.to_owned()),
            segments: vec!["settings".to_owned()],
            value: Some("level".to_owned()),
        };
        steps.push(Step {
            spec: TargetSpec {
                path: TargetPath::Provider(key),
                action: TargetAction::Modify,
                expect: Expect::Unchecked,
                size_hint: None,
            },
            change: Change::Apply(Mutation::ProviderSet(ProviderValue::U32(2))),
        });
    }
    steps
}

fn created_steps(fixture: &Fixture, action: Option<&str>, variant: Variant) -> Vec<Step> {
    let source = fixture.seed_file("work/source.txt", b"source\n");
    (0..count(variant))
        .map(|index| {
            let path = fixture.path(&format!("work/created-{index}"));
            match (action, variant) {
                (Some("link"), Variant::HardLink) => applied(
                    &path,
                    TargetAction::Link,
                    Mutation::CreateHardLink {
                        existing: source.clone(),
                    },
                ),
                (Some("link"), _) => applied(
                    &path,
                    TargetAction::Link,
                    Mutation::CreateLink {
                        target: "source.txt".into(),
                        kind: LinkKind::SymlinkFile,
                    },
                ),
                _ => applied(
                    &path,
                    TargetAction::Create,
                    Mutation::WriteBytes(b"created\n".to_vec()),
                ),
            }
        })
        .collect()
}

fn content_steps(fixture: &Fixture, action: Option<&str>, variant: Variant) -> Vec<Step> {
    (0..count(variant))
        .map(|index| {
            let path =
                fixture.seed_file(&format!("work/content-{index}.txt"), b"previous content\n");
            match action {
                Some("truncate") => applied(&path, TargetAction::Truncate, Mutation::SetLength(4)),
                Some("data") => applied(
                    &path,
                    TargetAction::Modify,
                    Mutation::WriteAt {
                        offset: 2,
                        bytes: b"XY".to_vec(),
                    },
                ),
                _ => applied(
                    &path,
                    TargetAction::Modify,
                    Mutation::WriteBytes(b"new content\n".to_vec()),
                ),
            }
        })
        .collect()
}

fn rename_step(fixture: &Fixture, row: &Row, variant: Variant) -> Step {
    let source = if matches!(variant, Variant::Bulk(_)) {
        fixture.seed_tree("work/tree", count(variant))
    } else {
        fixture.seed_file("work/old-name.txt", b"renamed\n")
    };
    let replace = has(row, PreserveMode::Moved);
    let to = if variant == Variant::SecondVolume {
        path_below(fixture.volume(1), "moved.txt")
    } else if replace {
        fixture.seed_file("work/new-name.txt", b"overwritten destination\n")
    } else {
        fixture.path("work/new-name.txt")
    };
    applied(
        &source,
        TargetAction::Rename { to, replace },
        Mutation::Rename,
    )
}

/// Runs the plan in one transaction and commits it.
fn execute(fixture: &Fixture, row: &Row, steps: Vec<Step>, markers: &[Irreversible]) -> OpHandle {
    let mut begin = request::begin(&row.command, row.action.as_deref());
    begin.bulk = steps.len() > 1;
    let mut tx: Box<dyn MutationTx + '_> = fixture.journal.begin(begin);
    let specs: Vec<TargetSpec> = steps.iter().map(|step| step.spec.clone()).collect();
    let preserved = tx.preserve(&specs).expect("targets are preserved");
    assert_eq!(
        preserved.len(),
        specs.len(),
        "one preservation result per target"
    );
    let mut external = Vec::new();
    for (index, step) in steps.into_iter().enumerate() {
        let index = TargetIndex(u32::try_from(index).expect("target index fits"));
        match step.change {
            Change::Apply(mutation) => {
                tx.apply(index, mutation).expect("change is applied");
            }
            Change::External(path) => {
                fixture.write_outside(&path, b"written by the program\n");
                external.push(ExternalChange {
                    path,
                    kind: ExternalChangeKind::Modified,
                    declared: Some(index),
                });
            }
        }
    }
    if !external.is_empty() {
        tx.observe_external(&external)
            .expect("external changes are recorded");
    }
    for reason in markers {
        tx.mark_irreversible(IrreversibleEntry::new(*reason));
    }
    tx.commit().expect("transaction commits")
}

fn push_unique<T: PartialEq>(items: &mut Vec<T>, item: T) {
    if !items.contains(&item) {
        items.push(item);
    }
}

/// Modes the journal may record for a row: the modes of the row, the whole
/// overwrite chain when the row lists one of its modes, the file modes for
/// references (the file system layer sees reference files), and created for
/// values that did not exist.
fn accepted_modes(row: &Row) -> Vec<PreserveMode> {
    let mut accepted = row.preserve.clone();
    if keeps_content(row) || has(row, PreserveMode::Ref) {
        for mode in CONTENT_MODES {
            push_unique(&mut accepted, mode);
        }
    }
    if has(row, PreserveMode::Ref) || has(row, PreserveMode::Value) {
        push_unique(&mut accepted, PreserveMode::Created);
    }
    accepted
}

fn check_modes(handle: &OpHandle, row: &Row, table: &CoverageTable) {
    let accepted = accepted_modes(row);
    for outcome in &handle.targets {
        if outcome.mode == PreserveMode::None {
            let reason = outcome.irreversible.unwrap_or_else(|| {
                panic!(
                    "{}: target {:?} kept nothing without a reason",
                    row.label(),
                    outcome.path
                )
            });
            assert!(
                table.generic_irreversible.contains(&reason) || row.irreversible.contains(&reason),
                "{}: reason {} is not a reason of the row",
                row.label(),
                reason.as_str()
            );
            continue;
        }
        assert!(
            accepted.contains(&outcome.mode),
            "{}: mode {} is not a mode of the row",
            row.label(),
            outcome.mode.as_str()
        );
    }
}

fn reasons(handle: &OpHandle) -> Vec<Irreversible> {
    let mut reasons = Vec::new();
    let reported = handle.irreversible.iter().map(|entry| entry.reason).chain(
        handle
            .targets
            .iter()
            .filter_map(|outcome| outcome.irreversible),
    );
    for reason in reported {
        push_unique(&mut reasons, reason);
    }
    reasons
}

fn change_round_trips(fixture: &Fixture, table: &CoverageTable, row: &Row, variant: Variant) {
    let steps = plan(fixture, row, variant);
    let before = fixture.snapshot();
    let markers = match variant {
        Variant::PreserveLimit | Variant::PreserveBudget => {
            vec![Irreversible::PreviousContentNotKept]
        }
        _ => Vec::new(),
    };
    let handle = execute(fixture, row, steps, &markers);
    let after = fixture.snapshot();
    check_modes(&handle, row, table);
    if variant == Variant::LowerOverlayLayer {
        let copied = handle.targets.iter().any(|outcome| {
            outcome.mode == PreserveMode::Copy
                && outcome
                    .notable
                    .iter()
                    .any(|notable| matches!(notable, JournalNotable::CrossDeviceCopy { .. }))
        });
        assert!(
            copied,
            "{}: a lower layer directory is kept by copy",
            row.label()
        );
    }
    let reported = reasons(&handle);
    match (row.coverage, variant) {
        (Coverage::Full, _) | (Coverage::Partial, Variant::Plain | Variant::Bulk(_)) => {
            assert!(
                reported.is_empty(),
                "{}: unexpected reasons {reported:?}",
                row.label()
            );
        }
        (_, Variant::NoSpace) => assert!(
            reported.contains(&Irreversible::NoSpaceForCopy)
                || reported.contains(&Irreversible::Database),
            "{}: lack of space is not reported",
            row.label()
        ),
        (_, Variant::PreserveLimit | Variant::PreserveBudget) => assert!(
            reported.contains(&Irreversible::PreviousContentNotKept),
            "{}: unpreserved content is not reported",
            row.label()
        ),
        _ => {}
    }
    if !reported.is_empty() {
        return;
    }
    let undone = fixture
        .journal
        .undo(&request::undo_last())
        .expect("operation is undone");
    assert_eq!(
        undone.sources,
        vec![handle.op.expect("operation has a number")]
    );
    let restored = fixture.snapshot();
    if variant == Variant::HardLink {
        before.without_link_counts().assert_same(
            &restored.without_link_counts(),
            &format!("{} after undo", row.label()),
        );
    } else {
        before.assert_same(&restored, &format!("{} after undo", row.label()));
    }
    fixture
        .journal
        .redo(&request::redo(Some(undone.entry)))
        .expect("undo is redone");
    let redone = fixture.snapshot();
    let context = format!("{} after redo", row.label());
    if variant == Variant::HardLink {
        after
            .without_link_counts()
            .assert_same(&redone.without_link_counts(), &context);
    } else {
        after.assert_same(&redone, &context);
    }
}

fn effect_is_marked(fixture: &Fixture, table: &CoverageTable, row: &Row) {
    let mut steps = Vec::new();
    if has(row, PreserveMode::Ref) {
        steps.extend(content_steps(fixture, None, Variant::Plain));
    }
    let handle = execute(fixture, row, steps, &row.irreversible);
    check_modes(&handle, row, table);
    let reported = reasons(&handle);
    for reason in &row.irreversible {
        assert!(
            reported.contains(reason),
            "{}: reason {} is not reported",
            row.label(),
            reason.as_str()
        );
    }
    assert!(
        handle.op.is_some(),
        "{}: an irreversible effect is recorded",
        row.label()
    );
}

fn read_changes_nothing(fixture: &Fixture) {
    let path = fixture.seed_file("work/read.txt", b"read only\n");
    let before = fixture.snapshot();
    let entries = fixture.history().len();
    fixture.fs.content_hash(&path).expect("file is readable");
    let listed = fixture
        .fs
        .read_dir(fixture.volume(0))
        .expect("root is listable")
        .count();
    assert!(listed > 0, "the seeded tree is listed");
    assert_eq!(fixture.history().len(), entries, "a read records no entry");
    before.assert_same(&fixture.snapshot(), "after a read");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
