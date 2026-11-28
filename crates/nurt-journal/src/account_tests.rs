// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-journal.account_tests
// Responsibility:   Write, undo and redo of an entry through the descriptor mode of the file system of an account journal.
// Dependencies:     nurt_contracts, nurt_fs, store, tx, undo
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Write, undo and redo of an entry through the descriptor mode of the file
//! system of an account journal (`AccountFs::with_descriptors`).
//!
//! The elevated process keeps the journal of an ordinary account through
//! this file system. The tests run the whole cycle of a real journal over it:
//! a write of a file with permissions and an extended attribute, `undo` and
//! `redo`. The account is the user that runs the tests, so the cycle needs no
//! administrator and runs the same calls as for another account; only the
//! `fchown` to a foreign account is not exercised here.

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use nurt_contracts::bound::{BoundDocument, BoundProgram};
use nurt_contracts::fs::{AbsPath, Follow, FsWrite, MetaChange, Xattr};
use nurt_contracts::journal::{
    BeginRequest, Mutation, OnConflict, OpId, OpKind, RedoRequest, TargetAction, TargetIndex,
    TargetPath, TargetSpec, UndoRequest, UndoSelector,
};
use nurt_fs::account::{AccountFs, AccountIdentity};

use crate::store::JournalCtx;
use crate::store::test_support::Fixture;
use crate::tx::begin;
use crate::undo::{redo, undo};

/// Name of the extended attribute of the tests.
const ATTRIBUTE: &str = "user.nurt-account-test";

fn program() -> BoundDocument {
    BoundDocument::new(
        "cat-1",
        "corr-1",
        BoundProgram {
            statements: Vec::new(),
        },
    )
}

fn request(command: &str) -> BeginRequest {
    BeginRequest {
        kind: OpKind::command(command),
        session: None,
        program: program(),
        corrections: Vec::new(),
        bulk: false,
        counts_as_step: true,
        preserve_budget: None,
    }
}

fn spec(path: &AbsPath, action: TargetAction) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Fs(path.clone()),
        action,
        expect: nurt_contracts::journal::Expect::Unchecked,
        size_hint: None,
    }
}

/// Context whose file system is the descriptor mode of the account that owns
/// the state directory of the fixture.
fn account_ctx(fixture: &Fixture) -> JournalCtx {
    let owner = std::fs::metadata(fixture.state_dir.as_path()).unwrap();
    let identity = AccountIdentity {
        uid: std::os::unix::fs::MetadataExt::uid(&owner),
        gid: std::os::unix::fs::MetadataExt::gid(&owner),
        state_dir: fixture.state_dir.clone(),
    };
    let inner: Arc<dyn FsWrite> = Arc::new(fixture.fs.clone());
    let mut ctx = fixture.ctx();
    ctx.fs = Arc::new(AccountFs::new(inner, identity).with_descriptors());
    ctx
}

fn mode_of(path: &AbsPath) -> u32 {
    std::fs::metadata(path.as_path())
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

fn attribute_of(path: &AbsPath) -> Option<Vec<u8>> {
    nurt_fs::system_fs()
        .extended_metadata(path, Follow::NoFollow)
        .unwrap()
        .xattrs
        .into_iter()
        .find(|attribute| attribute.name == ATTRIBUTE)
        .map(|attribute| attribute.value)
}

/// Gives `path` the mode and the attribute; `false` when the volume keeps no
/// user attributes, in which case the test has nothing to check.
fn mark(path: &AbsPath, mode: u32, value: &[u8]) -> bool {
    let change = MetaChange {
        mode: Some(mode),
        set_xattrs: vec![Xattr {
            name: ATTRIBUTE.into(),
            value: value.to_vec(),
        }],
        ..MetaChange::default()
    };
    nurt_fs::system_fs()
        .set_meta(&journal_scope(), path, &change)
        .is_ok()
}

#[allow(clippy::disallowed_methods, reason = "tests stand in for the journal")]
fn journal_scope() -> nurt_contracts::fs::TxScope {
    nurt_contracts::fs::TxScope::new_for_journal(
        nurt_contracts::journal::OpId(1),
        nurt_contracts::fs::ScopePurpose::Maintenance,
    )
}

/// Checks the attribute of `path` when the file system of the scenario keeps
/// attributes.
fn expect_attribute(path: &AbsPath, value: &[u8], kept: bool) {
    if kept {
        assert_eq!(attribute_of(path).as_deref(), Some(value));
    }
}

fn undo_entry(ctx: &JournalCtx, op: OpId) -> OpId {
    undo(
        ctx,
        &UndoRequest {
            selector: UndoSelector::Op(op),
            on_conflict: OnConflict::Report,
            session: None,
            program: program(),
        },
    )
    .unwrap()
    .entry
}

fn redo_entry(ctx: &JournalCtx, undone: OpId) {
    redo(
        ctx,
        &RedoRequest {
            undo_entry: Some(undone),
            session: None,
            program: program(),
        },
    )
    .unwrap();
}

/// One cycle: write, undo, redo of a file at `relative`, then a change of the
/// metadata alone with its undo and redo, checking the content, the mode and
/// the attribute after each step. The attribute is checked when `attributes`
/// holds: the file system under the targets outside the account is the test
/// double of the journal tests, which keeps none.
fn cycle(relative: &str, attributes: bool) {
    let fixture = Fixture::new();
    let ctx = account_ctx(&fixture);
    let path = fixture.fs.path(relative);
    std::fs::create_dir_all(path.as_path().parent().unwrap()).unwrap();
    fixture.write(&path, b"old content");
    if !mark(&path, 0o640, b"before") {
        return;
    }

    let mut tx = begin(&ctx, request("write"));
    tx.preserve(&[spec(&path, TargetAction::Modify)]).unwrap();
    tx.apply(
        TargetIndex(0),
        Mutation::WriteBytes(b"new content".to_vec()),
    )
    .unwrap();
    let op = tx.commit().unwrap().op.unwrap();
    assert_eq!(fixture.read(&path), b"new content");
    assert_eq!(mode_of(&path), 0o640);
    expect_attribute(&path, b"before", attributes);

    let undone = undo_entry(&ctx, op);
    assert_eq!(fixture.read(&path), b"old content");
    assert_eq!(mode_of(&path), 0o640);
    expect_attribute(&path, b"before", attributes);

    redo_entry(&ctx, undone);
    assert_eq!(fixture.read(&path), b"new content");
    assert_eq!(mode_of(&path), 0o640);
    expect_attribute(&path, b"before", attributes);

    let mut tx = begin(&ctx, request("attributes"));
    tx.preserve(&[spec(&path, TargetAction::Attributes)])
        .unwrap();
    let change = MetaChange {
        mode: Some(0o644),
        set_xattrs: if attributes {
            vec![Xattr {
                name: ATTRIBUTE.into(),
                value: b"after".to_vec(),
            }]
        } else {
            Vec::new()
        },
        ..MetaChange::default()
    };
    tx.apply(TargetIndex(0), Mutation::SetMeta(change)).unwrap();
    let meta_op = tx.commit().unwrap().op.unwrap();
    assert_eq!(mode_of(&path), 0o644);
    expect_attribute(&path, b"after", attributes);

    let undone_meta = undo_entry(&ctx, meta_op);
    assert_eq!(mode_of(&path), 0o640);
    expect_attribute(&path, b"before", attributes);
    assert_eq!(fixture.read(&path), b"new content");

    redo_entry(&ctx, undone_meta);
    assert_eq!(mode_of(&path), 0o644);
    expect_attribute(&path, b"after", attributes);
    assert_eq!(fixture.read(&path), b"new content");
}
#[test]
fn an_elevated_write_of_a_target_below_the_state_directory_undoes_and_redoes() {
    cycle("vol-a/state/data/file.txt", true);
}

#[test]
fn an_elevated_write_of_a_target_outside_the_account_undoes_and_redoes() {
    cycle("vol-a/outside/file.txt", false);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
