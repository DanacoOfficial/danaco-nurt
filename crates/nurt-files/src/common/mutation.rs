// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::common::mutation
// Responsibility:   Targets, mutations and `change` records of effect stages.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Targets, mutations and `change` records of effect stages.
//!
//! An effect stage builds its file operations in the plan step, reports them
//! with [`planned_change`], and in the apply step runs them through the
//! transaction with [`apply_changes`], which declares and preserves every
//! target before the first change and returns the `change` record of each
//! target. Stages that write streamed or encoded content declare their
//! targets with [`fs_target`] and [`preserve_targets`] and apply the
//! mutations themselves, for example with
//! [`write_text_preserving`](super::text::write_text_preserving).

use nurt_contracts::error::Result;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    Applied, Expect, FileOp, MutationTx, Preserved, TargetAction, TargetIndex, TargetPath,
    TargetSpec, apply_ops,
};
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::value::{self, Size};

use super::path::path_field;

/// Declaration of a file system target.
#[must_use]
pub fn fs_target(path: &AbsPath, action: TargetAction, expect: Expect) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Fs(path.clone()),
        action,
        expect,
        size_hint: None,
    }
}

/// Declares and preserves targets and returns their indexes, in order.
///
/// # Errors
///
/// Fails with the errors of [`MutationTx::preserve`], for example
/// `E_STALE`, `E_NOT_FOUND` or `E_EXISTS`.
pub fn preserve_targets(
    tx: &mut dyn MutationTx,
    targets: &[TargetSpec],
) -> Result<Vec<TargetIndex>> {
    Ok(tx
        .preserve(targets)?
        .into_iter()
        .map(|preserved| preserved.index)
        .collect())
}

/// Runs file operations as one protocol pass and returns the `change`
/// record of every target, in order.
///
/// # Errors
///
/// Fails with the errors of [`apply_ops`]; the journal then rolls the
/// transaction back.
pub fn apply_changes(tx: &mut dyn MutationTx, ops: Vec<FileOp>) -> Result<Vec<Change>> {
    let targets: Vec<TargetSpec> = ops.iter().map(|op| op.target.clone()).collect();
    let outcomes = apply_ops(tx, ops)?;
    Ok(targets
        .iter()
        .zip(outcomes)
        .map(|(target, outcome)| applied_change(target, &outcome.preserved, &outcome.applied))
        .collect())
}

/// `change` record of a planned target: the path, the action, the
/// destination of a rename and the known size.
///
/// The path is the declared target; for a rename it is the path after the
/// change, as in every other record of the report, `from` is the source (the
/// report requires it) and `to` repeats the path.
#[must_use]
pub fn planned_change(target: &TargetSpec) -> Change {
    let mut change = Change::new(target_field(&target.path), change_action(&target.action));
    if let TargetPath::Fs(path) = &target.path {
        change.raw_path = path.raw_bytes();
    }
    if let TargetAction::Rename { to, .. } = &target.action {
        change.from = Some(change.path.clone());
        change.raw_from = change.raw_path.clone();
        change.path = path_field(to);
        change.raw_path = to.raw_bytes();
        change.to = Some(path_field(to));
        change.raw_to = to.raw_bytes();
    }
    change.size_before = target.size_hint.map(Size::from_bytes);
    change
}

/// `change` record of an applied target: [`planned_change`] with the sizes
/// and hashes before and after, the way the previous state was kept and the
/// reason when it was not.
#[must_use]
pub fn applied_change(target: &TargetSpec, preserved: &Preserved, applied: &Applied) -> Change {
    let before = &preserved.before;
    let after = &applied.after;
    Change {
        size_before: before
            .exists
            .then_some(before.size)
            .flatten()
            .map(Size::from_bytes),
        size_after: after
            .exists
            .then_some(after.size)
            .flatten()
            .map(Size::from_bytes),
        hash_before: before.hash.map(|hash| hash.to_string()),
        hash_after: after.hash.map(|hash| hash.to_string()),
        store: Some(preserved.mode.as_str().to_owned()),
        irreversible: preserved.irreversible,
        ..planned_change(target)
    }
}

fn change_action(action: &TargetAction) -> ChangeAction {
    match action {
        TargetAction::Create | TargetAction::CreateTree => ChangeAction::Create,
        TargetAction::Delete => ChangeAction::Delete,
        TargetAction::Rename { .. } => ChangeAction::Rename,
        TargetAction::Attributes => ChangeAction::Attributes,
        TargetAction::Link => ChangeAction::Link,
        TargetAction::Truncate => ChangeAction::Truncate,
        _ => ChangeAction::Modify,
    }
}

fn target_field(path: &TargetPath) -> value::Path {
    match path {
        TargetPath::Fs(path) => path_field(path),
        TargetPath::Provider(key) => {
            let mut text = format!("{}:/{}", key.scheme.0, key.segments.join("/"));
            if let Some(name) = &key.value {
                text.push('/');
                text.push_str(name);
            }
            value::Path::new(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::fs::{ContentHash, ProviderKey, ProviderScheme};
    use nurt_contracts::journal::{AfterState, BeforeState, JournalStore, PreserveMode};
    use nurt_contracts::report::Irreversible;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::journal::JournalCall;

    use super::*;
    use crate::common::etag::observe;
    use crate::common::fixture::abs;

    fn declared(test: &TestContext) -> Vec<TargetPath> {
        test.journal()
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                JournalCall::Preserve { targets, .. } => Some(targets),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn applied_ok(test: &TestContext) -> Vec<TargetIndex> {
        test.journal()
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                JournalCall::Apply {
                    index, ok: true, ..
                } => Some(index),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_change_goes_through_the_transaction() {
        let test = TestContext::new();
        let kept = abs("/data/kept.txt");
        let gone = abs("/data/gone.txt");
        let created = abs("/data/new.txt");
        test.fs().write_file(&kept, b"old").unwrap();
        test.fs().write_file(&gone, b"x").unwrap();
        let observed = observe(test.fs().as_ref(), &kept).unwrap();

        let ops = vec![
            FileOp::write(kept.clone(), b"newer".to_vec()).expect(observed.expect()),
            FileOp::remove(gone.clone()),
            FileOp::create(created.clone(), b"n".to_vec()),
        ];
        let mut tx = test.journal().begin(begin_request("write"));
        let changes = apply_changes(tx.as_mut(), ops).unwrap();
        tx.commit().unwrap();

        assert_eq!(
            declared(&test),
            [&kept, &gone, &created].map(|path| TargetPath::Fs(path.clone()))
        );
        assert_eq!(
            applied_ok(&test),
            [TargetIndex(0), TargetIndex(1), TargetIndex(2)]
        );
        let actions: Vec<ChangeAction> = changes.iter().map(|change| change.action).collect();
        assert_eq!(
            actions,
            [
                ChangeAction::Modify,
                ChangeAction::Delete,
                ChangeAction::Create
            ]
        );
        assert_eq!(changes[0].size_before, Some(Size::from_bytes(3)));
        assert_eq!(changes[0].size_after, Some(Size::from_bytes(5)));
        assert_eq!(changes[1].size_after, None);
        assert_eq!(changes[2].size_before, None);
        assert_eq!(changes[0].store.as_deref(), Some("none"));
        assert_eq!(test.fs().read_file(&kept).unwrap(), b"newer");
        assert!(!test.fs().exists(&gone));
        assert_eq!(test.fs().read_file(&created).unwrap(), b"n");
    }

    #[test]
    fn a_target_changed_after_the_plan_fails_with_stale() {
        let test = TestContext::new();
        let path = abs("/data/a.txt");
        test.fs().write_file(&path, b"old").unwrap();
        let observed = observe(test.fs().as_ref(), &path).unwrap();
        test.fs().write_file(&path, b"changed").unwrap();

        let ops = vec![FileOp::write(path.clone(), b"new".to_vec()).expect(observed.expect())];
        let mut tx = test.journal().begin(begin_request("write"));
        let error = apply_changes(tx.as_mut(), ops).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"changed");
        assert!(applied_ok(&test).is_empty());
    }

    #[test]
    fn a_target_created_after_the_plan_fails_with_stale() {
        let test = TestContext::new();
        let path = abs("/data/a.txt");
        let observed = observe(test.fs().as_ref(), &path).unwrap();
        test.fs().write_file(&path, b"other").unwrap();
        let ops = vec![FileOp::write(path.clone(), b"mine".to_vec()).expect(observed.expect())];
        let mut tx = test.journal().begin(begin_request("write"));
        let error = apply_changes(tx.as_mut(), ops).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"other");
    }

    #[test]
    fn preserved_targets_are_returned_in_order() {
        let test = TestContext::new();
        let first = abs("/data/a.txt");
        let second = abs("/data/b.txt");
        test.fs().write_file(&first, b"a").unwrap();
        let mut tx = test.journal().begin(begin_request("copy"));
        let indexes = preserve_targets(
            tx.as_mut(),
            &[
                fs_target(&first, TargetAction::Modify, Expect::Unchecked),
                fs_target(&second, TargetAction::Create, Expect::Absent),
            ],
        )
        .unwrap();
        assert_eq!(indexes, [TargetIndex(0), TargetIndex(1)]);
        tx.commit().unwrap();
        assert_eq!(
            declared(&test),
            [TargetPath::Fs(first), TargetPath::Fs(second)]
        );
    }

    #[test]
    fn planned_changes_name_the_action_and_destination() {
        let from = abs("/data/a.txt");
        let to = abs("/data/b.txt");
        let rename = FileOp::rename(from.clone(), to.clone(), false).size_hint(9);
        let change = planned_change(&rename.target);
        assert_eq!(change.action, ChangeAction::Rename);
        assert_eq!(change.path.as_str(), to.to_language_string());
        assert_eq!(
            change.from.as_ref().map(|path| path.as_str().to_owned()),
            Some(from.to_language_string()),
            "the report requires `from` on a rename"
        );
        assert_eq!(
            change.to.map(|path| path.as_str().to_owned()),
            Some(to.to_language_string())
        );
        assert_eq!(change.size_before, Some(Size::from_bytes(9)));

        let cases = [
            (TargetAction::CreateTree, ChangeAction::Create),
            (TargetAction::Append, ChangeAction::Modify),
            (TargetAction::Replace, ChangeAction::Modify),
            (TargetAction::ExternalWrite, ChangeAction::Modify),
            (TargetAction::Attributes, ChangeAction::Attributes),
            (TargetAction::Link, ChangeAction::Link),
            (TargetAction::Truncate, ChangeAction::Truncate),
        ];
        for (action, expected) in cases {
            let change = planned_change(&fs_target(&from, action, Expect::Unchecked));
            assert_eq!(change.action, expected);
            assert_eq!(change.to, None);
        }
    }

    #[test]
    fn applied_changes_report_hashes_and_irreversibility() {
        let path = abs("/data/a.txt");
        let target = fs_target(&path, TargetAction::Modify, Expect::Unchecked);
        let hash = ContentHash([1; 32]);
        let preserved = Preserved {
            index: TargetIndex(0),
            volume: None,
            mode: PreserveMode::None,
            bytes_held: 0,
            before: BeforeState {
                exists: true,
                size: Some(4),
                hash: Some(hash),
                ..BeforeState::default()
            },
            irreversible: Some(Irreversible::JournalInactive),
            notable: Vec::new(),
        };
        let applied = Applied {
            index: TargetIndex(0),
            after: AfterState {
                exists: true,
                size: Some(2),
                hash: Some(hash),
                ..AfterState::default()
            },
        };
        let change = applied_change(&target, &preserved, &applied);
        assert_eq!(change.hash_before, Some(hash.to_string()));
        assert_eq!(change.hash_after, Some(hash.to_string()));
        assert_eq!(change.irreversible, Some(Irreversible::JournalInactive));
        assert_eq!(change.size_before, Some(Size::from_bytes(4)));
        assert_eq!(change.size_after, Some(Size::from_bytes(2)));
    }

    #[test]
    fn provider_targets_are_named_by_scheme_and_key() {
        let key = ProviderKey {
            scheme: ProviderScheme("registry".to_owned()),
            segments: vec!["HKCU".to_owned(), "Software".to_owned()],
            value: Some("Theme".to_owned()),
        };
        let target = TargetSpec {
            path: TargetPath::Provider(key.clone()),
            action: TargetAction::Modify,
            expect: Expect::Unchecked,
            size_hint: None,
        };
        assert_eq!(
            planned_change(&target).path.as_str(),
            "registry:/HKCU/Software/Theme"
        );
        let key_only = TargetSpec {
            path: TargetPath::Provider(ProviderKey { value: None, ..key }),
            ..target
        };
        assert_eq!(
            planned_change(&key_only).path.as_str(),
            "registry:/HKCU/Software"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
