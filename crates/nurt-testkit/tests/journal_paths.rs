// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Mutations, expectations, restoring and failure paths of the pass-through
//! and keeping journals, and the fixed-answer admission.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]
#![allow(clippy::disallowed_methods, reason = "tests stand in for the journal")]
#![allow(
    clippy::cognitive_complexity,
    reason = "one test walks every store query of two journals"
)]

use std::io::Write;
use std::sync::Arc;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::{
    AbsPath, FileKind, Follow, Fs, FsErrorKind, LinkKind, MetaChange, ProviderKey, ProviderScheme,
    ProviderValue, VolumeId,
};
use nurt_contracts::journal::{
    Admission, AdmissionDecision, AdmissionRequest, CloneMode, Eligibility, ExemptWrite, Expect,
    ExternalChange, ExternalChangeKind, HistoryQuery, InactiveReason, JournalStore, LimitsRequest,
    LimitsScope, Mutation, OpId, PinRequest, PreserveMode, PurgeRequest, PurgeSelector,
    RedoRequest, RollbackReason, ServiceLiveness, TargetAction, TargetIndex, TargetPath,
    TargetSpec, UndoRequest, UndoSelector, VolumeState, VolumeStatus,
};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::fixtures::{begin_request, empty_program, fs_target};
use nurt_testkit::journal::{JournalCall, NoopJournal, RecordingJournal, TestAdmission, TxState};
use nurt_testkit::vfs::{VirtualFs, VolumeConfig, test_path};

fn path(text: &str) -> AbsPath {
    test_path(text).unwrap()
}

fn keeping() -> (Arc<VirtualFs>, RecordingJournal) {
    let fs = Arc::new(VirtualFs::default());
    let journal = RecordingJournal::keeping(fs.clone());
    (fs, journal)
}

fn plain() -> (Arc<VirtualFs>, RecordingJournal) {
    let fs = Arc::new(VirtualFs::default());
    let journal = RecordingJournal::new(fs.clone());
    (fs, journal)
}

fn spec(file: &AbsPath, action: TargetAction, expect: Expect) -> TargetSpec {
    TargetSpec {
        expect,
        ..fs_target(file, action)
    }
}

fn provider(value: Option<&str>) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Provider(ProviderKey {
            scheme: ProviderScheme("registry".to_owned()),
            segments: vec!["hive".to_owned(), "key".to_owned()],
            value: value.map(str::to_owned),
        }),
        action: TargetAction::Modify,
        expect: Expect::Unchecked,
        size_hint: None,
    }
}

fn undo_request() -> UndoRequest {
    UndoRequest {
        selector: UndoSelector::Op(OpId(1)),
        on_conflict: Default::default(),
        session: None,
        program: empty_program(),
    }
}

#[test]
fn every_store_query_of_both_journals_is_answered_and_recorded() {
    let fs = Arc::new(VirtualFs::default());
    let noop = NoopJournal::new(fs.clone());
    let recording = RecordingJournal::new(fs);
    let limits = LimitsRequest {
        steps: None,
        age: None,
        size: None,
        scope: LimitsScope::User,
        volume: None,
        preview: true,
    };
    let pin = PinRequest {
        ids: vec![OpId(3)],
        release: false,
        until: None,
    };
    let purge = PurgeRequest {
        volume: None,
        selector: PurgeSelector::All,
    };
    let redo = RedoRequest {
        undo_entry: None,
        session: None,
        program: empty_program(),
    };
    macro_rules! exercise {
        ($journal:expr) => {{
            let journal = &$journal;
            assert!(journal.recover().is_ok());
            assert!(
                journal
                    .history(&HistoryQuery::default())
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                journal.details(OpId(1)).unwrap_err().code(),
                ErrorCode::JournalNoop
            );
            assert_eq!(
                journal.undo(&undo_request()).unwrap_err().code(),
                ErrorCode::JournalNoop
            );
            assert_eq!(
                journal.redo(&redo).unwrap_err().code(),
                ErrorCode::JournalNoop
            );
            assert_eq!(journal.pin(&pin).unwrap().not_available, [OpId(3)]);
            assert_eq!(journal.purge(&purge).unwrap().bytes, 0);
            assert!(journal.status(None).unwrap().is_empty());
            assert!(!journal.limits(&limits).unwrap().applied);
            assert!(journal.space(None).unwrap().is_empty());
            assert!(journal.sweep().is_ok());
            assert_eq!(
                journal
                    .rollback_committed(OpId(1), RollbackReason::Cancelled)
                    .unwrap_err()
                    .code(),
                ErrorCode::JournalNoop
            );
        }};
    }
    exercise!(noop);
    exercise!(recording);
    assert_eq!(recording.calls().len(), 12);
    recording.clear();
    assert!(recording.calls().is_empty());
}

#[test]
fn keeping_journal_reports_an_unknown_operation() {
    let (_fs, journal) = keeping();
    let error = journal
        .rollback_committed(OpId(9), RollbackReason::Cancelled)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);
}

#[test]
fn rollback_restores_directories_links_and_attributes() {
    let (fs, journal) = keeping();
    let dir = path("/w/dir");
    let inner = path("/w/dir/sub/a.txt");
    let link = path("/w/link");
    let file = path("/w/file.txt");
    fs.write_file(&inner, b"inner").unwrap();
    fs.symlink("dir", &link).unwrap();
    fs.write_file(&file, b"keep").unwrap();
    fs.set_mode(&file, 0o640).unwrap();

    let mut tx = journal.begin(begin_request("clean"));
    let preserved = tx
        .preserve(&[
            fs_target(&dir, TargetAction::Delete),
            fs_target(&link, TargetAction::Delete),
            fs_target(&file, TargetAction::Attributes),
        ])
        .unwrap();
    assert_eq!(preserved[0].mode, PreserveMode::Copy);
    assert_eq!(preserved[2].mode, PreserveMode::Meta);
    tx.apply(TargetIndex(0), Mutation::Remove).unwrap();
    tx.apply(TargetIndex(1), Mutation::Remove).unwrap();
    tx.apply(
        TargetIndex(2),
        Mutation::SetMeta(MetaChange {
            mode: Some(0o600),
            ..MetaChange::default()
        }),
    )
    .unwrap();
    assert!(!fs.exists(&dir));
    let report = tx.rollback(RollbackReason::TargetFailed).unwrap();
    assert!(report.failed.is_empty());
    assert_eq!(fs.read_file(&inner).unwrap(), b"inner");
    assert_eq!(fs.read_link(&link).unwrap(), std::path::Path::new("dir"));
    assert_eq!(fs.metadata(&file, Follow::NoFollow).unwrap().mode, 0o640);
    let record = &journal.transactions()[0];
    assert_eq!(record.state, TxState::RolledBack);
    assert_eq!(record.restored.len(), 3);
}

#[test]
fn special_files_are_not_kept_and_cannot_be_restored() {
    let (fs, journal) = keeping();
    let fifo = path("/w/pipe");
    fs.create_dir_all(&path("/w")).unwrap();
    fs.create_special(&fifo).unwrap();
    let mut tx = journal.begin(begin_request("clean"));
    let preserved = tx
        .preserve(&[fs_target(&fifo, TargetAction::Delete)])
        .unwrap();
    assert_eq!(preserved[0].mode, PreserveMode::None);
    assert_eq!(
        preserved[0].irreversible,
        Some(Irreversible::PreviousContentNotKept)
    );
    tx.apply(TargetIndex(0), Mutation::Remove).unwrap();
    let report = tx.rollback(RollbackReason::Cancelled).unwrap();
    assert_eq!(report.failed.len(), 1);
}

#[test]
fn rename_append_and_create_modes_are_kept() {
    let (fs, journal) = keeping();
    let from = path("/w/from.txt");
    let to = path("/w/to.txt");
    let log = path("/w/log.txt");
    let fresh = path("/w/fresh.txt");
    fs.write_file(&from, b"from").unwrap();
    fs.write_file(&to, b"to").unwrap();
    fs.write_file(&log, b"log").unwrap();
    let mut tx = journal.begin(begin_request("mixed"));
    let preserved = tx
        .preserve(&[
            fs_target(
                &from,
                TargetAction::Rename {
                    to: to.clone(),
                    replace: true,
                },
            ),
            fs_target(&log, TargetAction::Append),
            fs_target(&fresh, TargetAction::Create),
        ])
        .unwrap();
    assert_eq!(preserved[0].mode, PreserveMode::Rename);
    assert_eq!(preserved[1].mode, PreserveMode::Append);
    assert_eq!(preserved[2].mode, PreserveMode::Created);
    tx.apply(TargetIndex(0), Mutation::Rename).unwrap();
    tx.apply(TargetIndex(1), Mutation::AppendBytes(b"+".to_vec()))
        .unwrap();
    tx.apply(TargetIndex(2), Mutation::WriteBytes(b"new".to_vec()))
        .unwrap();
    let handle = tx.commit().unwrap();
    let op = handle.op.unwrap();
    assert_eq!(fs.read_file(&to).unwrap(), b"from");
    let undone = journal
        .rollback_committed(op, RollbackReason::VerifyFailed)
        .unwrap();
    assert_eq!(undone.restored.len(), 3);
    assert_eq!(fs.read_file(&from).unwrap(), b"from");
    assert_eq!(fs.read_file(&to).unwrap(), b"to");
    assert_eq!(fs.read_file(&log).unwrap(), b"log");
    assert!(!fs.exists(&fresh));
}

#[test]
fn declarations_check_existence_and_expectations() {
    let (fs, journal) = plain();
    let file = path("/w/a.txt");
    fs.write_file(&file, b"x").unwrap();
    let missing = path("/w/missing.txt");
    let etag = fs.etag(&file, Follow::NoFollow).unwrap();
    let hash = fs.content_hash(&file).unwrap();
    let wrong_hash = fs.content_hash(&file).map(|mut h| {
        h.0[0] ^= 1;
        h
    });

    for rejected in [
        spec(&file, TargetAction::Create, Expect::Unchecked),
        spec(&file, TargetAction::CreateTree, Expect::Unchecked),
        spec(&file, TargetAction::Link, Expect::Unchecked),
        spec(&missing, TargetAction::Delete, Expect::Unchecked),
        spec(&missing, TargetAction::Replace, Expect::Unchecked),
        spec(&missing, TargetAction::Attributes, Expect::Unchecked),
        spec(&missing, TargetAction::Truncate, Expect::Unchecked),
        spec(&file, TargetAction::Modify, Expect::Absent),
        spec(&missing, TargetAction::Modify, Expect::Etag(etag.clone())),
        spec(&missing, TargetAction::Modify, Expect::Hash(hash)),
        spec(
            &file,
            TargetAction::Modify,
            Expect::Hash(wrong_hash.unwrap()),
        ),
    ] {
        let mut tx = journal.begin(begin_request("check"));
        assert!(tx.preserve(&[rejected]).is_err());
    }
    let stale = {
        let mut tx = journal.begin(begin_request("check"));
        tx.preserve(&[spec(
            &missing,
            TargetAction::Modify,
            Expect::Etag(etag.clone()),
        )])
        .unwrap_err()
    };
    assert_eq!(stale.code(), ErrorCode::Stale);

    for accepted in [
        spec(&file, TargetAction::Modify, Expect::Etag(etag)),
        spec(&file, TargetAction::Modify, Expect::Hash(hash)),
        spec(&missing, TargetAction::Create, Expect::Absent),
    ] {
        let mut tx = journal.begin(begin_request("check"));
        assert!(tx.preserve(&[accepted]).is_ok());
    }
}

#[test]
fn misused_transactions_fail_with_stage_failed_and_stay_rolled_back() {
    let (fs, journal) = plain();
    let file = path("/w/a.txt");
    fs.write_file(&file, b"x").unwrap();

    let mut tx = journal.begin(begin_request("misuse"));
    tx.preserve(&[fs_target(&file, TargetAction::Delete), provider(Some("v"))])
        .unwrap();
    let undeclared = tx.apply(TargetIndex(7), Mutation::Remove).unwrap_err();
    assert_eq!(undeclared.code(), ErrorCode::StageFailed);
    let after = tx.apply(TargetIndex(0), Mutation::Remove).unwrap_err();
    assert_eq!(after.code(), ErrorCode::RolledBack);
    assert_eq!(tx.preserve(&[]).unwrap_err().code(), ErrorCode::RolledBack);
    assert_eq!(
        tx.stage(TargetIndex(0)).err().unwrap().code(),
        ErrorCode::RolledBack
    );
    assert!(tx.observe_external(&[]).is_err());
    tx.mark_irreversible(IrreversibleEntry {
        reason: Irreversible::Network,
        path: None,
        count: None,
        detail: None,
    });
    tx.record_exempt(ExemptWrite::ObjectStore {
        repo: path("/repo"),
    });
    assert!(tx.commit().is_err());

    let mut tx = journal.begin(begin_request("misuse"));
    tx.preserve(&[fs_target(&file, TargetAction::Delete)])
        .unwrap();
    let mismatch = tx
        .apply(TargetIndex(0), Mutation::WriteBytes(Vec::new()))
        .unwrap_err();
    assert_eq!(mismatch.code(), ErrorCode::StageFailed);

    let mut tx = journal.begin(begin_request("misuse"));
    tx.preserve(&[fs_target(&file, TargetAction::Delete)])
        .unwrap();
    assert!(tx.stage(TargetIndex(0)).is_err());

    let mut tx = journal.begin(begin_request("misuse"));
    tx.preserve(&[fs_target(&file, TargetAction::Modify)])
        .unwrap();
    let no_stage = tx
        .apply(TargetIndex(0), Mutation::CommitStaged)
        .unwrap_err();
    assert_eq!(no_stage.code(), ErrorCode::StageFailed);

    let mut tx = journal.begin(begin_request("misuse"));
    assert!(tx.stage(TargetIndex(3)).is_err());

    let mut tx = journal.begin(begin_request("misuse"));
    tx.preserve(&[fs_target(&file, TargetAction::Modify)])
        .unwrap();
    tx.apply(TargetIndex(0), Mutation::SetLength(0)).unwrap();
    let late = tx.preserve(&[]).unwrap_err();
    assert_eq!(late.code(), ErrorCode::StageFailed);
}

#[test]
fn provider_targets_have_no_provider_in_the_pass_through_journal() {
    let (_fs, journal) = plain();
    let mut tx = journal.begin(begin_request("registry"));
    let preserved = tx.preserve(&[provider(None)]).unwrap();
    assert_eq!(preserved[0].mode, PreserveMode::None);
    let error = tx
        .apply(
            TargetIndex(0),
            Mutation::ProviderSet(ProviderValue::Text("v".to_owned())),
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Unsupported);
    let mut tx = journal.begin(begin_request("registry"));
    tx.preserve(&[provider(Some("value"))]).unwrap();
    assert!(tx.stage(TargetIndex(0)).is_err());
}

#[test]
fn staged_content_replaces_the_target_with_its_metadata() {
    let (fs, journal) = plain();
    let file = path("/w/a.txt");
    fs.write_file(&file, b"old").unwrap();
    fs.set_mode(&file, 0o640).unwrap();
    let mut tx = journal.begin(begin_request("stage"));
    tx.preserve(&[fs_target(&file, TargetAction::Modify)])
        .unwrap();
    let mut first = tx.stage(TargetIndex(0)).unwrap();
    first.write_all(b"first").unwrap();
    drop(first);
    let mut second = tx.stage(TargetIndex(0)).unwrap();
    second.write_all(b"second").unwrap();
    drop(second);
    tx.apply(TargetIndex(0), Mutation::CommitStaged).unwrap();
    tx.commit().unwrap();
    assert_eq!(fs.read_file(&file).unwrap(), b"second");
    assert_eq!(fs.metadata(&file, Follow::NoFollow).unwrap().mode, 0o640);
}

#[test]
fn every_file_mutation_is_applied_to_the_virtual_file_system() {
    let (fs, journal) = plain();
    let data = path("/w/data.bin");
    let copy = path("/w/copy.bin");
    let dir = path("/w/tree/a/b");
    let link = path("/w/link");
    let hard = path("/w/hard");
    let moved = path("/w/moved.bin");
    fs.write_file(&data, b"0123456789").unwrap();
    fs.write_file(&path("/w/other"), b"o").unwrap();

    let mut tx = journal.begin(begin_request("all"));
    tx.preserve(&[
        fs_target(&data, TargetAction::Modify),
        fs_target(&copy, TargetAction::Create),
        fs_target(&dir, TargetAction::CreateTree),
        fs_target(&link, TargetAction::Link),
        fs_target(&hard, TargetAction::Link),
        fs_target(&data, TargetAction::Truncate),
        fs_target(
            &path("/w/other"),
            TargetAction::Rename {
                to: moved.clone(),
                replace: false,
            },
        ),
    ])
    .unwrap();
    fs.create_dir_all(&path("/w/tree/a")).unwrap();
    tx.apply(
        TargetIndex(0),
        Mutation::WriteAt {
            offset: 2,
            bytes: b"XY".to_vec(),
        },
    )
    .unwrap();
    tx.apply(
        TargetIndex(1),
        Mutation::CopyFrom {
            source: data.clone(),
            clone: CloneMode::Auto,
        },
    )
    .unwrap();
    tx.apply(TargetIndex(2), Mutation::CreateDir { mode: 0o755 })
        .unwrap();
    tx.apply(
        TargetIndex(3),
        Mutation::CreateLink {
            target: "data.bin".into(),
            kind: LinkKind::SymlinkFile,
        },
    )
    .unwrap();
    tx.apply(
        TargetIndex(4),
        Mutation::CreateHardLink {
            existing: data.clone(),
        },
    )
    .unwrap();
    tx.apply(TargetIndex(5), Mutation::SetLength(4)).unwrap();
    tx.apply(TargetIndex(6), Mutation::Rename).unwrap();
    tx.commit().unwrap();
    assert_eq!(fs.read_file(&data).unwrap(), b"01XY");
    assert_eq!(fs.read_file(&copy).unwrap(), b"01XY456789");
    assert_eq!(fs.read_file(&hard).unwrap(), b"01XY");
    assert!(fs.exists(&moved));
    assert_eq!(
        fs.metadata(&link, Follow::NoFollow).unwrap().kind,
        FileKind::Symlink
    );
}

#[test]
fn failing_mutations_roll_the_transaction_back() {
    let (fs, journal) = plain();
    let source = path("/w/source");
    let target = path("/w/target");
    fs.write_file(&source, b"s").unwrap();

    let mut tx = journal.begin(begin_request("clone"));
    tx.preserve(&[fs_target(&target, TargetAction::Create)])
        .unwrap();
    let error = tx
        .apply(
            TargetIndex(0),
            Mutation::CopyFrom {
                source: source.clone(),
                clone: CloneMode::Always,
            },
        )
        .unwrap_err();
    assert!(!error.to_string().is_empty());
    assert!(!fs.exists(&target));

    let mut tx = journal.begin(begin_request("link"));
    tx.preserve(&[fs_target(&source, TargetAction::Modify)])
        .unwrap();
    assert!(tx.apply(TargetIndex(0), Mutation::SetLength(1)).is_ok());
    let mut tx = journal.begin(begin_request("missing"));
    tx.preserve(&[fs_target(&path("/w/none"), TargetAction::Create)])
        .unwrap();
    assert!(
        tx.apply(
            TargetIndex(0),
            Mutation::CopyFrom {
                source: path("/w/none-source"),
                clone: CloneMode::Auto,
            },
        )
        .is_err()
    );
}

#[test]
fn renames_across_volumes_copy_files_and_refuse_directories() {
    let clock = Arc::new(VirtualClock::new());
    let fs = Arc::new(VirtualFs::new(clock));
    let mount = path("/mnt");
    fs.create_dir_all(&mount).unwrap();
    fs.add_volume(&mount, VolumeConfig::default()).unwrap();
    let journal = RecordingJournal::new(fs.clone());
    let from = path("/w/a.txt");
    let existing = path("/mnt/b.txt");
    let dir = path("/w/dir");
    fs.write_file(&from, b"payload").unwrap();
    fs.write_file(&dir.join_name("x").unwrap(), b"x").unwrap();
    fs.write_file(&existing, b"present").unwrap();

    let rename = |from: &AbsPath, to: &AbsPath, replace: bool| {
        let mut tx = journal.begin(begin_request("mv"));
        tx.preserve(&[fs_target(
            from,
            TargetAction::Rename {
                to: to.clone(),
                replace,
            },
        )])
        .unwrap();
        tx.apply(TargetIndex(0), Mutation::Rename)
    };
    assert!(rename(&from, &existing, false).is_err());
    assert!(rename(&dir, &path("/mnt/dir"), true).is_err());
    assert!(rename(&from, &existing, true).is_ok());
    assert_eq!(fs.read_file(&existing).unwrap(), b"payload");
    assert!(!fs.exists(&from));
}

#[test]
fn a_full_volume_reports_no_space_for_staged_writes() {
    let fs = Arc::new(VirtualFs::with_root(
        Arc::new(VirtualClock::new()),
        VolumeConfig {
            capacity: Some(64),
            ..VolumeConfig::default()
        },
    ));
    let journal = RecordingJournal::new(fs.clone());
    let file = path("/w/a.txt");
    fs.write_file(&file, b"x").unwrap();
    let mut tx = journal.begin(begin_request("big"));
    tx.preserve(&[fs_target(&file, TargetAction::Modify)])
        .unwrap();
    let error = tx
        .apply(TargetIndex(0), Mutation::WriteBytes(vec![0; 4096]))
        .unwrap_err();
    assert!(
        error.to_string().contains("space") || error.to_string().contains("io"),
        "{error}"
    );
}

#[test]
fn observed_external_changes_are_registered() {
    let (fs, journal) = plain();
    let made = path("/w/made.txt");
    let gone = path("/w/gone.txt");
    fs.write_file(&made, b"m").unwrap();
    let mut tx = journal.begin(begin_request("tool"));
    let applied = tx
        .observe_external(&[
            ExternalChange {
                path: made.clone(),
                kind: ExternalChangeKind::Created,
                declared: None,
            },
            ExternalChange {
                path: gone,
                kind: ExternalChangeKind::Deleted,
                declared: None,
            },
        ])
        .unwrap();
    assert_eq!(applied.len(), 2);
    assert!(applied[0].after.exists);
    assert!(!applied[1].after.exists);
    tx.mark_irreversible(IrreversibleEntry {
        reason: Irreversible::ExternalEffect,
        path: None,
        count: Some(2),
        detail: None,
    });
    tx.record_exempt(ExemptWrite::ObjectStore {
        repo: path("/repo"),
    });
    let handle = tx.commit().unwrap();
    assert_eq!(handle.targets.len(), 2);
    assert_eq!(handle.exempt.len(), 1);
    assert!(
        journal
            .calls()
            .iter()
            .any(|call| matches!(call, JournalCall::RecordExempt { .. }))
    );

    let mut tx = journal.begin(begin_request("tool"));
    tx.preserve(&[
        fs_target(&made, TargetAction::Modify),
        fs_target(&made, TargetAction::ExternalWrite),
    ])
    .unwrap();
    let wrong = tx.observe_external(&[ExternalChange {
        path: made.clone(),
        kind: ExternalChangeKind::Modified,
        declared: Some(TargetIndex(0)),
    }]);
    assert_eq!(wrong.unwrap_err().code(), ErrorCode::StageFailed);

    let mut tx = journal.begin(begin_request("tool"));
    tx.preserve(&[fs_target(&made, TargetAction::ExternalWrite)])
        .unwrap();
    let declared = tx.observe_external(&[ExternalChange {
        path: made,
        kind: ExternalChangeKind::Modified,
        declared: Some(TargetIndex(0)),
    }]);
    assert_eq!(declared.unwrap().len(), 1);
}

#[test]
fn failed_rollbacks_of_kept_entries_are_reported() {
    let (fs, journal) = keeping();
    let file = path("/w/a.txt");
    fs.write_file(&file, b"old").unwrap();
    journal.fail_rollback_of(TargetPath::Fs(file.clone()));
    journal.fail_rollback_of(TargetPath::Fs(file.clone()));
    let mut tx = journal.begin(begin_request("fail"));
    tx.preserve(&[fs_target(&file, TargetAction::Modify)])
        .unwrap();
    tx.apply(TargetIndex(0), Mutation::WriteBytes(b"new".to_vec()))
        .unwrap();
    let report = tx.rollback(RollbackReason::TargetFailed).unwrap();
    assert_eq!(report.failed.len(), 1);
    let again = journal.transactions();
    assert_eq!(again[0].not_restored.len(), 1);
}

#[test]
fn a_failing_apply_on_a_kept_journal_reports_the_failed_rollback() {
    let (fs, journal) = keeping();
    let file = path("/w/a.txt");
    fs.write_file(&file, b"old").unwrap();
    journal.fail_rollback_of(TargetPath::Fs(file.clone()));
    let mut tx = journal.begin(begin_request("fail"));
    tx.preserve(&[fs_target(&file, TargetAction::Modify)])
        .unwrap();
    fs.inject(nurt_testkit::vfs::Fault::error(
        &file,
        "rename",
        FsErrorKind::PermissionDenied,
    ));
    let error = tx
        .apply(TargetIndex(0), Mutation::WriteBytes(b"new".to_vec()))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::RollbackFailed);
}

#[test]
fn admission_answers_are_fixed_and_recorded() {
    let fs = VirtualFs::default();
    fs.write_file(&path("/w/a"), b"x").unwrap();
    let info = fs.volume(&path("/w/a")).unwrap();
    let admission = TestAdmission::default()
        .with_eligibility(Eligibility::Inactive(InactiveReason::ServiceDown))
        .with_status(VolumeStatus {
            state: VolumeState::Hard,
            thresholds: nurt_contracts::journal::Thresholds {
                reserve: 1,
                soft: 2,
                margin: 3,
                hysteresis: 4,
            },
        })
        .with_liveness(ServiceLiveness::Down);
    assert_eq!(
        admission.eligibility(&info),
        Eligibility::Inactive(InactiveReason::ServiceDown)
    );
    assert_eq!(admission.liveness(), ServiceLiveness::Down);
    assert_eq!(admission.volume_status(&info.id).state, VolumeState::Hard);
    let volume = VolumeId {
        dev: 1,
        uuid: None,
        subvolume: 0,
    };
    admission.after_operation(&volume);
    assert_eq!(admission.notified(), std::slice::from_ref(&volume));
    let request = AdmissionRequest {
        volume,
        extra: 1,
        op_size: 1,
        bytes_held: 0,
        size_budget: None,
        oversize_keep: false,
        op_size_limit: None,
    };
    assert_eq!(admission.admit(&request), AdmissionDecision::Keep);
    assert_eq!(admission.requests().len(), 1);
}
