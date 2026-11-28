// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/journal_ops.rs
// Responsibility:   Command-level file operations, the protocol pass of `apply_ops` and the serialized names of the journal types.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Command-level file operations, the protocol pass of `apply_ops` and the
//! serialized names of the journal types.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::path::PathBuf;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, LinkKind, MetaChange, ProviderKey, ProviderScheme, WriteHandle};
use nurt_contracts::journal::{
    AdmissionDecision, AfterState, Applied, BeforeState, CloneMode, ConflictKind, Eligibility,
    EntryState, ExemptWrite, Expect, ExpiryReason, ExternalChange, FileOp, InactiveReason,
    JournalNotable, LimitsScope, Mutation, MutationTx, OnConflict, OpHandle, OpId, OpKind,
    PreserveMode, Preserved, PurgeSelector, RollbackReason, RollbackReport, ServiceLiveness,
    SessionId, Since, TargetAction, TargetIndex, TargetPath, TargetSpec, UndoSelector, UndoneCause,
    VolumeState, apply_ops,
};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use serde_json::json;

/// Absolute path in the native form of the host system.
fn abs(path: &str) -> AbsPath {
    let text = if cfg!(windows) {
        format!("C:{path}")
    } else {
        path.to_owned()
    };
    AbsPath::new(text).unwrap()
}

/// The language form of `abs(path)`.
fn language(path: &str) -> String {
    abs(path).to_language_string()
}

/// Transaction that answers `preserve` with a configured number of results
/// and records the calls it receives.
struct ScriptedTx {
    preserved: usize,
    fail_preserve: bool,
    fail_apply_at: Option<u32>,
    calls: Vec<String>,
    declared: Vec<TargetSpec>,
    applied: Vec<(TargetIndex, Mutation)>,
}

impl ScriptedTx {
    fn answering(preserved: usize) -> Self {
        Self {
            preserved,
            fail_preserve: false,
            fail_apply_at: None,
            calls: Vec::new(),
            declared: Vec::new(),
            applied: Vec::new(),
        }
    }
}

fn kept(index: usize) -> Preserved {
    Preserved {
        index: TargetIndex(u32::try_from(index).unwrap()),
        volume: None,
        mode: PreserveMode::Exchange,
        bytes_held: 0,
        before: BeforeState::default(),
        irreversible: None,
        notable: Vec::new(),
    }
}

impl MutationTx for ScriptedTx {
    fn op(&self) -> Option<OpId> {
        Some(OpId(1))
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        self.calls.push("preserve".to_owned());
        if self.fail_preserve {
            return Err(Error::new(ErrorCode::NotFound, "the target does not exist"));
        }
        self.declared.extend_from_slice(targets);
        Ok((0..self.preserved).map(kept).collect())
    }

    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        Err(Error::new(
            ErrorCode::StageFailed,
            "staging is not scripted",
        ))
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        self.calls.push("apply".to_owned());
        if self.fail_apply_at == Some(target.0) {
            return Err(Error::new(ErrorCode::Stale, "the file changed"));
        }
        self.applied.push((target, mutation));
        Ok(Applied {
            index: target,
            after: AfterState::default(),
        })
    }

    fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        Ok(Vec::new())
    }

    fn mark_irreversible(&mut self, _marker: IrreversibleEntry) {}

    fn record_exempt(&mut self, _write: ExemptWrite) {}

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        Ok(OpHandle {
            op: Some(OpId(1)),
            kind: OpKind::command("write"),
            bulk: false,
            targets: Vec::new(),
            irreversible: Vec::new(),
            notable: Vec::new(),
            exempt: Vec::new(),
        })
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        Ok(RollbackReport {
            op: None,
            restored: Vec::new(),
            failed: Vec::new(),
        })
    }
}

#[test]
fn apply_ops_declares_every_target_once_and_applies_in_order() {
    let mut tx = ScriptedTx::answering(3);
    let ops = vec![
        FileOp::write(abs("/home/user/a.txt"), b"new".to_vec()),
        FileOp::remove(abs("/home/user/b.txt")),
        FileOp::rename(abs("/home/user/c.txt"), abs("/home/user/d.txt"), false),
    ];
    let outcomes = apply_ops(&mut tx, ops).unwrap();
    assert_eq!(tx.calls, ["preserve", "apply", "apply", "apply"]);
    assert_eq!(tx.declared.len(), 3);
    assert_eq!(outcomes.len(), 3);
    for (position, outcome) in outcomes.iter().enumerate() {
        let index = TargetIndex(u32::try_from(position).unwrap());
        assert_eq!(outcome.preserved.index, index);
        assert_eq!(outcome.applied.index, index);
    }
    let mutations: Vec<&Mutation> = tx.applied.iter().map(|(_, mutation)| mutation).collect();
    assert_eq!(
        mutations,
        [
            &Mutation::WriteBytes(b"new".to_vec()),
            &Mutation::Remove,
            &Mutation::Rename
        ]
    );
}

#[test]
fn apply_ops_without_operations_only_declares() {
    let mut tx = ScriptedTx::answering(0);
    let outcomes = apply_ops(&mut tx, Vec::new()).unwrap();
    assert!(outcomes.is_empty());
    assert_eq!(tx.calls, ["preserve"]);
}

#[test]
fn apply_ops_fails_before_the_first_change_when_targets_are_missing() {
    let mut tx = ScriptedTx::answering(1);
    let ops = vec![
        FileOp::write(abs("/home/user/a.txt"), Vec::new()),
        FileOp::write(abs("/home/user/b.txt"), Vec::new()),
    ];
    let error = apply_ops(&mut tx, ops).unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);
    assert_eq!(error.problem.as_deref(), Some("preserve_count"));
    assert!(error.message.contains("1 of 2"), "{}", error.message);
    assert!(tx.applied.is_empty());
    assert_eq!(tx.calls, ["preserve"]);
}

#[test]
fn apply_ops_fails_before_the_first_change_when_targets_are_surplus() {
    let mut tx = ScriptedTx::answering(3);
    let ops = vec![
        FileOp::write(abs("/home/user/a.txt"), Vec::new()),
        FileOp::write(abs("/home/user/b.txt"), Vec::new()),
    ];
    let error = apply_ops(&mut tx, ops).unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);
    assert!(error.message.contains("3 of 2"), "{}", error.message);
    assert!(tx.applied.is_empty());
}

#[test]
fn apply_ops_returns_the_error_of_preserve_without_applying() {
    let mut tx = ScriptedTx::answering(1);
    tx.fail_preserve = true;
    let ops = vec![FileOp::remove(abs("/home/user/a.txt"))];
    let error = apply_ops(&mut tx, ops).unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert_eq!(tx.calls, ["preserve"]);
}

#[test]
fn apply_ops_stops_at_the_first_failing_mutation() {
    let mut tx = ScriptedTx::answering(3);
    tx.fail_apply_at = Some(1);
    let ops = vec![
        FileOp::remove(abs("/home/user/a.txt")),
        FileOp::remove(abs("/home/user/b.txt")),
        FileOp::remove(abs("/home/user/c.txt")),
    ];
    let error = apply_ops(&mut tx, ops).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(tx.calls, ["preserve", "apply", "apply"]);
    assert_eq!(tx.applied.len(), 1);
}

#[test]
fn file_op_constructors_pair_the_declared_action_with_the_mutation() {
    let path = abs("/home/user/a.txt");
    let other = abs("/home/user/b.txt");
    let link = PathBuf::from("target");
    let cases: Vec<(FileOp, TargetAction, Mutation, Expect)> = vec![
        (
            FileOp::write(path.clone(), b"x".to_vec()),
            TargetAction::Modify,
            Mutation::WriteBytes(b"x".to_vec()),
            Expect::Unchecked,
        ),
        (
            FileOp::create(path.clone(), b"x".to_vec()),
            TargetAction::Create,
            Mutation::WriteBytes(b"x".to_vec()),
            Expect::Absent,
        ),
        (
            FileOp::append(path.clone(), b"y".to_vec()),
            TargetAction::Append,
            Mutation::AppendBytes(b"y".to_vec()),
            Expect::Unchecked,
        ),
        (
            FileOp::write_at(path.clone(), 7, b"z".to_vec()),
            TargetAction::Modify,
            Mutation::WriteAt {
                offset: 7,
                bytes: b"z".to_vec(),
            },
            Expect::Unchecked,
        ),
        (
            FileOp::set_length(path.clone(), 0),
            TargetAction::Truncate,
            Mutation::SetLength(0),
            Expect::Unchecked,
        ),
        (
            FileOp::remove(path.clone()),
            TargetAction::Delete,
            Mutation::Remove,
            Expect::Unchecked,
        ),
        (
            FileOp::copy(other.clone(), path.clone(), CloneMode::Never),
            TargetAction::Create,
            Mutation::CopyFrom {
                source: other.clone(),
                clone: CloneMode::Never,
            },
            Expect::Absent,
        ),
        (
            FileOp::copy_over(other.clone(), path.clone(), CloneMode::Auto),
            TargetAction::Modify,
            Mutation::CopyFrom {
                source: other.clone(),
                clone: CloneMode::Auto,
            },
            Expect::Unchecked,
        ),
        (
            FileOp::create_dir(path.clone(), 0o755),
            TargetAction::Create,
            Mutation::CreateDir { mode: 0o755 },
            Expect::Absent,
        ),
        (
            FileOp::create_link(path.clone(), link.clone(), LinkKind::SymlinkFile),
            TargetAction::Link,
            Mutation::CreateLink {
                target: link,
                kind: LinkKind::SymlinkFile,
            },
            Expect::Unchecked,
        ),
        (
            FileOp::create_hard_link(path.clone(), other.clone()),
            TargetAction::Link,
            Mutation::CreateHardLink { existing: other },
            Expect::Unchecked,
        ),
        (
            FileOp::set_meta(path.clone(), MetaChange::default()),
            TargetAction::Attributes,
            Mutation::SetMeta(MetaChange::default()),
            Expect::Unchecked,
        ),
    ];
    for (op, action, mutation, expect) in cases {
        assert_eq!(op.target.path, TargetPath::Fs(path.clone()), "{action:?}");
        assert_eq!(op.target.action, action);
        assert_eq!(op.mutation, mutation);
        assert_eq!(op.target.expect, expect, "{action:?}");
        assert_eq!(op.target.size_hint, None);
    }
}

#[test]
fn rename_declares_the_source_and_names_the_destination_in_the_action() {
    let op = FileOp::rename(abs("/d/a.txt"), abs("/d/b.txt"), true);
    assert_eq!(op.target.path, TargetPath::Fs(abs("/d/a.txt")));
    assert_eq!(
        op.target.action,
        TargetAction::Rename {
            to: abs("/d/b.txt"),
            replace: true
        }
    );
    assert_eq!(op.mutation, Mutation::Rename);
}

#[test]
fn expect_and_size_hint_set_the_declaration() {
    let op = FileOp::write(abs("/home/user/a.txt"), Vec::new())
        .expect(Expect::Absent)
        .size_hint(42);
    assert_eq!(op.target.expect, Expect::Absent);
    assert_eq!(op.target.size_hint, Some(42));
}

#[test]
fn entry_states_use_the_names_of_the_specification() {
    let cases = [
        (EntryState::Prepared, "prepared"),
        (EntryState::Committed, "committed"),
        (EntryState::RollingBack, "rolling_back"),
        (EntryState::Partial, "partial"),
        (EntryState::Undone, "undone"),
        (EntryState::Expiring, "expiring"),
        (EntryState::Expired, "expired"),
        (EntryState::Corrupt, "corrupt"),
    ];
    for (state, name) in cases {
        assert_eq!(serde_json::to_value(state).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<EntryState>(json!(name)).unwrap(),
            state
        );
    }
}

#[test]
fn causes_and_reasons_use_the_names_of_the_specification() {
    let causes = [
        (UndoneCause::Undo, "undo"),
        (UndoneCause::RolledBack, "rolled_back"),
        (UndoneCause::Crash, "crash"),
        (UndoneCause::VerifyFailed, "verify_failed"),
        (UndoneCause::ValidateFailed, "validate_failed"),
    ];
    for (cause, name) in causes {
        assert_eq!(serde_json::to_value(cause).unwrap(), json!(name));
    }
    let reasons = [
        (ExpiryReason::Ops, "ops"),
        (ExpiryReason::Time, "time"),
        (ExpiryReason::Size, "size"),
        (ExpiryReason::Space, "space"),
        (ExpiryReason::Purge, "purge"),
    ];
    for (reason, name) in reasons {
        assert_eq!(serde_json::to_value(reason).unwrap(), json!(name));
    }
    let rollbacks = [
        (RollbackReason::TargetFailed, "target_failed"),
        (RollbackReason::Cancelled, "cancelled"),
        (RollbackReason::VerifyFailed, "verify_failed"),
        (RollbackReason::ValidateFailed, "validate_failed"),
    ];
    for (reason, name) in rollbacks {
        assert_eq!(serde_json::to_value(reason).unwrap(), json!(name));
    }
}

#[test]
fn inactive_reasons_have_names_and_irreversible_markers() {
    let cases = [
        (
            InactiveReason::ServiceDown,
            "service_down",
            Irreversible::JournalInactive,
        ),
        (
            InactiveReason::NetworkVolume,
            "network_volume",
            Irreversible::NetworkVolume,
        ),
        (
            InactiveReason::RemovableVolume,
            "removable_volume",
            Irreversible::RemovableVolume,
        ),
        (
            InactiveReason::MemoryVolume,
            "memory_volume",
            Irreversible::JournalInactive,
        ),
        (
            InactiveReason::BelowReserve,
            "below_reserve",
            Irreversible::NoSpaceForCopy,
        ),
        (
            InactiveReason::NoAnchor,
            "no_anchor",
            Irreversible::JournalInactive,
        ),
        (
            InactiveReason::Disabled,
            "disabled",
            Irreversible::JournalInactive,
        ),
        (
            InactiveReason::NotDurable,
            "not_durable",
            Irreversible::JournalInactive,
        ),
    ];
    for (reason, name, irreversible) in cases {
        assert_eq!(reason.as_str(), name);
        assert_eq!(serde_json::to_value(reason).unwrap(), json!(name));
        assert_eq!(reason.irreversible(), irreversible, "{name}");
    }
}

#[test]
fn undo_selectors_serialize_in_snake_case() {
    assert_eq!(
        serde_json::to_value(UndoSelector::Last).unwrap(),
        json!("last")
    );
    assert_eq!(
        serde_json::to_value(UndoSelector::Op(OpId(7))).unwrap(),
        json!({"op": 7})
    );
    assert_eq!(
        serde_json::to_value(UndoSelector::To {
            after: OpId(3),
            all_sessions: false
        })
        .unwrap(),
        json!({"to": {"after": 3, "all_sessions": false}})
    );
    let file = UndoSelector::File {
        path: abs("/home/user/a.txt"),
        since: Some(Since::Op(OpId(2))),
    };
    assert_eq!(
        serde_json::to_value(&file).unwrap(),
        json!({"file": {"path": language("/home/user/a.txt"), "since": {"op": 2}}})
    );
    assert_eq!(
        serde_json::from_value::<UndoSelector>(serde_json::to_value(&file).unwrap()).unwrap(),
        file
    );
    let paths = UndoSelector::Paths {
        op: OpId(4),
        paths: vec![TargetPath::Fs(abs("/home/user/a.txt"))],
    };
    assert_eq!(
        serde_json::to_value(&paths).unwrap(),
        json!({"paths": {"op": 4, "paths": [{"kind": "fs", "at": language("/home/user/a.txt")}]}})
    );
}

#[test]
fn conflict_behaviour_and_purge_selectors_serialize_in_snake_case() {
    assert_eq!(OnConflict::default(), OnConflict::Report);
    assert_eq!(
        serde_json::to_value(OnConflict::IncludeLater).unwrap(),
        json!("include_later")
    );
    assert_eq!(
        serde_json::to_value(PurgeSelector::All).unwrap(),
        json!("all")
    );
    assert_eq!(
        serde_json::to_value(PurgeSelector::UpTo(OpId(5))).unwrap(),
        json!({"up_to": 5})
    );
    assert_eq!(
        serde_json::to_value(PurgeSelector::Ids(vec![OpId(1), OpId(9)])).unwrap(),
        json!({"ids": [1, 9]})
    );
    assert_eq!(
        serde_json::to_value(LimitsScope::User).unwrap(),
        json!("user")
    );
    assert_eq!(
        serde_json::to_value(LimitsScope::Session(SessionId("s-1".to_owned()))).unwrap(),
        json!({"session": "s-1"})
    );
}

#[test]
fn conflicts_and_notable_effects_carry_a_kind_member() {
    assert_eq!(
        serde_json::to_value(ConflictKind::Modified).unwrap(),
        json!({"kind": "modified"})
    );
    assert_eq!(
        serde_json::to_value(ConflictKind::LaterEntry { op: OpId(9) }).unwrap(),
        json!({"kind": "later_entry", "op": 9})
    );
    assert_eq!(
        serde_json::to_value(JournalNotable::JournalOversize { size: 10, limit: 5 }).unwrap(),
        json!({"kind": "journal_oversize", "size": 10, "limit": 5})
    );
    assert_eq!(
        serde_json::to_value(JournalNotable::HardlinkSetPreserved {
            path: TargetPath::Fs(abs("/home/user/a.txt"))
        })
        .unwrap(),
        json!({
            "kind": "hardlink_set_preserved",
            "path": {"kind": "fs", "at": language("/home/user/a.txt")}
        })
    );
}

#[test]
fn admission_types_serialize_in_snake_case() {
    assert_eq!(
        serde_json::to_value(AdmissionDecision::KeepAfterReclaim {
            released: vec![OpId(1)]
        })
        .unwrap(),
        json!({"keep_after_reclaim": {"released": [1]}})
    );
    assert_eq!(
        serde_json::to_value(Eligibility::Eligible).unwrap(),
        json!("eligible")
    );
    assert_eq!(
        serde_json::to_value(Eligibility::Inactive(InactiveReason::Disabled)).unwrap(),
        json!({"inactive": "disabled"})
    );
    let states = [
        (VolumeState::Normal, "normal"),
        (VolumeState::Soft, "soft"),
        (VolumeState::Hard, "hard"),
        (VolumeState::Emergency, "emergency"),
    ];
    for (state, name) in states {
        assert_eq!(serde_json::to_value(state).unwrap(), json!(name));
    }
    assert_eq!(
        serde_json::to_value(ServiceLiveness::Down).unwrap(),
        json!("down")
    );
}

#[test]
fn operation_kinds_round_trip_with_their_action() {
    let kind = OpKind::with_action("git.stash", "push");
    let value = serde_json::to_value(&kind).unwrap();
    assert_eq!(value, json!({"command": "git.stash", "action": "push"}));
    assert_eq!(serde_json::from_value::<OpKind>(value).unwrap(), kind);
    let bare: OpKind = serde_json::from_value(json!({"command": "remove"})).unwrap();
    assert_eq!(bare, OpKind::command("remove"));
}

#[test]
fn target_paths_order_file_system_entries_before_provider_keys() {
    let key = ProviderKey {
        scheme: ProviderScheme("registry".to_owned()),
        segments: vec!["HKCU".to_owned(), "Software".to_owned()],
        value: None,
    };
    let provider = TargetPath::Provider(key);
    let first = TargetPath::Fs(abs("/home/user/a.txt"));
    let second = TargetPath::Fs(abs("/home/user/b.txt"));
    let mut paths = vec![provider.clone(), second.clone(), first.clone()];
    paths.sort();
    assert_eq!(paths, [first, second, provider.clone()]);
    assert_eq!(
        serde_json::to_value(&provider).unwrap(),
        json!({
            "kind": "provider",
            "at": {"scheme": "registry", "segments": ["HKCU", "Software"], "value": null}
        })
    );
}

#[test]
fn mutations_serialize_in_snake_case() {
    assert_eq!(
        serde_json::to_value(Mutation::CommitStaged).unwrap(),
        json!("commit_staged")
    );
    assert_eq!(
        serde_json::to_value(Mutation::SetLength(5)).unwrap(),
        json!({"set_length": 5})
    );
    assert_eq!(
        serde_json::to_value(Mutation::WriteAt {
            offset: 2,
            bytes: vec![1, 2]
        })
        .unwrap(),
        json!({"write_at": {"offset": 2, "bytes": [1, 2]}})
    );
    assert_eq!(
        serde_json::to_value(TargetAction::CreateTree).unwrap(),
        json!({"action": "create_tree"})
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
