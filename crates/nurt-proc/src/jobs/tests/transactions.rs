// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The journal transaction of a run: the bound program of its entry and the
//! transaction used without a journal store.

use nurt_contracts::bound::{BoundDocument, BoundExpr, BoundHead, BoundStatement};
use nurt_contracts::fs::{AbsPath, FileKind};
use nurt_contracts::journal::{
    Expect, ExternalChange, ExternalChangeKind, Mutation, PreserveMode, RollbackReason,
    TargetAction, TargetIndex, TargetPath, TargetSpec,
};

use super::super::journal::{begin, bound_program};
use super::*;

fn target(path: &Path) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Fs(AbsPath::new(path.to_path_buf()).unwrap()),
        action: TargetAction::Modify,
        expect: Expect::default(),
        size_hint: None,
    }
}

fn change(path: &Path, kind: ExternalChangeKind, declared: Option<u32>) -> ExternalChange {
    ExternalChange {
        path: AbsPath::new(path.to_path_buf()).unwrap(),
        kind,
        declared: declared.map(TargetIndex),
    }
}

/// Command words of the single stage of a bound program.
fn stage_command(document: &BoundDocument) -> (String, Vec<String>) {
    let BoundStatement::Pipeline(pipeline) = &document.program.statements[0] else {
        panic!("the program holds no pipeline");
    };
    let BoundHead::Stage(stage) = &pipeline.head else {
        panic!("the pipeline does not start with a stage");
    };
    assert!(stage.waits);
    assert_eq!(stage.args[0].name, "command");
    let BoundExpr::List { items } = &stage.args[0].value else {
        panic!("the command is not a list");
    };
    let words = items
        .iter()
        .map(|item| match item {
            BoundExpr::Text { v } => v.clone(),
            other => panic!("the command holds {other:?}"),
        })
        .collect();
    (stage.name.clone(), words)
}

#[test]
fn a_run_without_a_journal_keeps_no_previous_content() {
    let dir = tempfile::tempdir().unwrap();
    let mut tx = Unjournaled::new(OpKind::command("run"));
    assert_eq!(tx.op(), None);
    let preserved = tx
        .preserve(&[target(&dir.path().join("a")), target(&dir.path().join("b"))])
        .unwrap();
    let indexes: Vec<TargetIndex> = preserved.iter().map(|entry| entry.index).collect();
    assert_eq!(indexes, [TargetIndex(0), TargetIndex(1)]);
    assert!(
        preserved
            .iter()
            .all(|entry| entry.mode == PreserveMode::None
                && entry.bytes_held == 0
                && entry.irreversible == Some(Irreversible::JournalInactive))
    );
    let staged = tx.stage(TargetIndex(0)).err().map(|error| error.code);
    assert_eq!(staged, Some(ErrorCode::StageFailed));
    let applied = tx
        .apply(TargetIndex(1), Mutation::WriteBytes(b"new".to_vec()))
        .unwrap_err();
    assert_eq!(applied.code, ErrorCode::StageFailed);
    let report = Box::new(tx).rollback(RollbackReason::Cancelled).unwrap();
    assert_eq!(report.op, None);
    assert!(report.restored.is_empty() && report.failed.is_empty());
}

#[test]
fn observed_changes_carry_their_state_after_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("out.txt");
    fs::write(&file, b"done").unwrap();
    let folder = dir.path().join("build");
    fs::create_dir(&folder).unwrap();
    let gone = dir.path().join("gone.txt");
    let vanished = dir.path().join("vanished.txt");
    let mut tx = Unjournaled::new(OpKind::command("run"));
    tx.preserve(&[target(&file)]).unwrap();
    let applied = tx
        .observe_external(&[
            change(&file, ExternalChangeKind::Modified, Some(0)),
            change(&folder, ExternalChangeKind::Created, None),
            change(&gone, ExternalChangeKind::Deleted, None),
            change(&vanished, ExternalChangeKind::Created, None),
        ])
        .unwrap();
    let indexes: Vec<u32> = applied.iter().map(|entry| entry.index.0).collect();
    assert_eq!(indexes, [0, 2, 3, 4]);
    assert_eq!(
        (applied[0].after.kind, applied[0].after.size),
        (Some(FileKind::File), Some(4))
    );
    assert_eq!(applied[1].after.kind, Some(FileKind::Directory));
    assert!(!applied[2].after.exists);
    assert!(!applied[3].after.exists);
    tx.mark_irreversible(nurt_contracts::report::IrreversibleEntry::new(
        Irreversible::ProcessStopped,
    ));
    let handle = Box::new(tx).commit().unwrap();
    assert_eq!(handle.op, None);
    let reasons: Vec<Irreversible> = handle
        .irreversible
        .iter()
        .map(|entry| entry.reason)
        .collect();
    assert_eq!(
        reasons,
        [
            Irreversible::JournalInactive,
            Irreversible::JournalInactive,
            Irreversible::JournalInactive,
            Irreversible::JournalInactive,
            Irreversible::ProcessStopped,
        ]
    );
    assert_eq!(
        handle.irreversible[0].path.as_deref(),
        Some(&*file.to_string_lossy())
    );
}

#[cfg(unix)]
#[test]
fn an_observed_link_is_reported_as_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("current");
    std::os::unix::fs::symlink(dir.path().join("missing-target"), &link).unwrap();
    let mut tx = Unjournaled::new(OpKind::command("run"));
    let applied = tx
        .observe_external(&[change(&link, ExternalChangeKind::Created, None)])
        .unwrap();
    assert_eq!(applied[0].after.kind, Some(FileKind::Symlink));
    assert!(applied[0].after.exists);
}

#[test]
fn the_journal_entry_names_the_command_of_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let request = request(dir.path(), 0, 0, 1);
    let document = bound_program(&ctx, "retry", &request);
    let (name, words) = stage_command(&document);
    assert_eq!(name, "retry");
    assert_eq!(words, request.command);
}

#[test]
fn the_journal_entry_keeps_the_program_given_by_the_executor() {
    let dir = tempfile::tempdir().unwrap();
    let (plain, _) = context(dir.path());
    let given = bound_program(&plain, "run", &request(dir.path(), 5, 0, 0));
    let mut extensions = Extensions::new();
    extensions.insert::<BoundDocument>(Arc::new(given.clone()));
    let ctx = ExecCtx::builder(
        Arc::new(SystemClock {
            origin: Instant::now(),
        }),
        Arc::new(TestHost {
            info: plain.host().info().clone(),
        }),
    )
    .state_dir(dir.path().to_path_buf())
    .extensions(extensions)
    .build();
    let document = bound_program(&ctx, "retry", &request(dir.path(), 0, 0, 0));
    assert_eq!(document, given);
    assert_eq!(stage_command(&document).0, "run");
}

#[test]
fn without_a_journal_store_a_run_marks_its_changes_irreversible() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let request = request(dir.path(), 0, 0, 0);
    let document = bound_program(&ctx, "run", &request);
    let mut tx = begin(&ctx, "run", document, &request);
    assert_eq!(tx.op(), None);
    let created = dir.path().join("report.txt");
    fs::write(&created, b"report").unwrap();
    tx.observe_external(&[change(&created, ExternalChangeKind::Created, None)])
        .unwrap();
    let handle = tx.commit().unwrap();
    assert_eq!(handle.kind, OpKind::command("run"));
    assert_eq!(handle.irreversible.len(), 1);
    assert_eq!(handle.irreversible[0].reason, Irreversible::JournalInactive);
}
