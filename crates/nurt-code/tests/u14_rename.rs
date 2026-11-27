// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-code/tests/u14_rename.rs
// Responsibility:   Tests of `code.rename`: plan, one transaction, undo, stale plans.
// Dependencies:     nurt_contracts, u14_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of `code.rename`: plan, one transaction, undo, stale plans,
//! alarms, rollback after a new diagnostic error, completeness, resource
//! operations and the syntax fallback.
//!
//! The default project has two Python files: `a.py` defines `total`
//! (line 1, columns 5 to 10) and `b.py` imports it (line 1, columns 15 to
//! 20) and calls it (line 3, columns 7 to 12).

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod u14_support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nurt_code::commands::rename::CodeRename;
use nurt_code::semantic::{DocumentChange, ResourceChange, SemanticErrorKind};
use nurt_contracts::config::{Config, SizeSetting};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::{Effect, RenameEdit, RenameEditKind, Symbol, SymbolSource};
use nurt_contracts::report::{ChangeAction, Notable};
use nurt_contracts::stage::{EffectPlan, RecordStream, StageInput};
use nurt_contracts::value::{Path as RecordPath, Record, RecordKind, Value};

use u14_support::{
    JournalLog, ScriptedProvider, TestTx, args, context, execute, path_value, preview, project,
    read, text_change, write,
};

const A: &str = "def total(values):\n    return sum(values)\n";
const B: &str = "from a import total\n\nprint(total([1, 2]))\n";

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    a: PathBuf,
    b: PathBuf,
}

fn fixture() -> Fixture {
    let (dir, root) = project();
    let a = root.join("a.py");
    let b = root.join("b.py");
    write(&a, A);
    write(&b, B);
    Fixture {
        _dir: dir,
        root,
        a,
        b,
    }
}

fn rename_args(file: &Path, extra: &[(&str, Value)]) -> nurt_contracts::stage::BoundArgs {
    let mut pairs = vec![
        ("to", Value::Text("sum_all".to_owned())),
        ("path", path_value(file)),
        ("line", Value::Int(1)),
        ("column", Value::Int(5)),
    ];
    pairs.extend(extra.iter().cloned());
    args(&pairs)
}

fn syntax() -> (&'static str, Value) {
    ("source", Value::Enum("syntax".to_owned()))
}

fn lsp() -> (&'static str, Value) {
    ("source", Value::Enum("lsp".to_owned()))
}

fn log() -> Arc<Mutex<JournalLog>> {
    Arc::new(Mutex::new(JournalLog::default()))
}

fn server_edits(fixture: &Fixture) -> Vec<DocumentChange> {
    vec![
        text_change(&fixture.a, 1, 5, 10, "sum_all"),
        text_change(&fixture.b, 1, 15, 20, "sum_all"),
        text_change(&fixture.b, 3, 7, 12, "sum_all"),
    ]
}

fn summary(plan: &EffectPlan) -> Record {
    plan.preview
        .iter()
        .find(|record| record.kind == RecordKind::Map)
        .cloned()
        .unwrap()
}

fn edits(plan: &EffectPlan) -> Vec<RenameEdit> {
    plan.preview
        .iter()
        .filter(|record| record.kind == RecordKind::RenameEdit)
        .map(|record| RenameEdit::try_from(record.clone()).unwrap())
        .collect()
}

fn codes(plan: &EffectPlan) -> Vec<Notable> {
    plan.notable.iter().map(|entry| entry.code).collect()
}

#[test]
fn syntax_plan_lists_edits_and_summary_without_changing_files() {
    let fixture = fixture();
    write(
        &fixture.root.join("README.md"),
        "Call total to sum values.\n",
    );
    let ctx = context(Config::default());
    let plan = preview(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        &ctx,
    )
    .unwrap();
    let edits = edits(&plan);
    assert_eq!(edits.len(), 3);
    assert!(
        edits
            .iter()
            .all(|edit| edit.old_text == "total" && edit.new_text == "sum_all")
    );
    assert!(edits.iter().any(|edit| edit.kind == RenameEditKind::Import));
    assert!(edits.iter().all(|edit| edit.source == SymbolSource::Syntax));
    let summary = summary(&plan);
    assert_eq!(summary.get("total"), Some(&Value::Int(3)));
    assert_eq!(summary.get("files"), Some(&Value::Int(2)));
    assert_eq!(
        summary.get("confidence"),
        Some(&Value::Enum("identifier".to_owned()))
    );
    let Some(Value::Text(id)) = summary.get("plan") else {
        panic!("the summary has no plan identifier");
    };
    assert!(id.starts_with("plan-"));
    assert_eq!(plan.changes.len(), 2);
    assert!(plan.bulk);
    assert!(codes(&plan).contains(&Notable::ReferencesLeft));
    assert_eq!(read(&fixture.a), A);
    assert_eq!(read(&fixture.b), B);
}

#[test]
fn apply_changes_every_file_in_one_transaction() {
    let fixture = fixture();
    let ctx = context(Config::default());
    let log = log();
    let outcome = execute(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        StageInput::none(),
        &ctx,
        &log,
    )
    .unwrap();
    assert_eq!(
        read(&fixture.a),
        "def sum_all(values):\n    return sum(values)\n"
    );
    assert_eq!(
        read(&fixture.b),
        "from a import sum_all\n\nprint(sum_all([1, 2]))\n"
    );
    let log = log.lock().unwrap();
    assert_eq!(log.preserve_calls, 1);
    assert!(log.committed);
    assert_eq!(outcome.changes.len(), 2);
    let effect = Effect::try_from(outcome.records[0].clone()).unwrap();
    assert_eq!(effect.kind, "code.rename");
    assert_eq!(effect.summary.unwrap().get("replacements"), Some(&3));
}

#[test]
fn undo_restores_every_file_byte_for_byte() {
    let fixture = fixture();
    let crlf = "from a import total\r\n\r\nprint(total([1, 2]))  # \u{17c}\u{f3}\u{142}w\r\n";
    write(&fixture.b, crlf);
    let before_a = std::fs::read(&fixture.a).unwrap();
    let before_b = std::fs::read(&fixture.b).unwrap();
    let ctx = context(Config::default());
    let log = log();
    execute(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        StageInput::none(),
        &ctx,
        &log,
    )
    .unwrap();
    assert!(read(&fixture.b).contains("sum_all([1, 2]))  # \u{17c}\u{f3}\u{142}w\r\n"));
    log.lock().unwrap().restore();
    assert_eq!(std::fs::read(&fixture.a).unwrap(), before_a);
    assert_eq!(std::fs::read(&fixture.b).unwrap(), before_b);
}

#[test]
fn file_changed_after_the_plan_fails_with_e_stale_and_changes_nothing() {
    let fixture = fixture();
    let ctx = context(Config::default());
    let stage = CodeRename::new(None);
    let arguments = rename_args(&fixture.a, &[syntax()]);
    let plan = preview(&stage, &arguments, &ctx).unwrap();
    write(
        &fixture.b,
        "from a import total\n\nprint(total([1, 2, 3]))\n",
    );
    let mut tx = TestTx::new(log());
    let error = stage
        .as_effect_stage()
        .apply(&arguments, plan, &mut tx, &ctx)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(read(&fixture.a), A);
}

#[test]
fn apply_needs_the_plan_of_the_same_execution() {
    let fixture = fixture();
    let ctx = context(Config::default());
    let stage = CodeRename::new(None);
    let mut tx = TestTx::new(log());
    let error = stage
        .as_effect_stage()
        .apply(
            &rename_args(&fixture.a, &[]),
            EffectPlan::new(),
            &mut tx,
            &ctx,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);
}

#[test]
fn new_name_already_used_fails_with_e_rename_conflict() {
    let fixture = fixture();
    write(
        &fixture.b,
        "from a import total\n\nsum_all = 1\nprint(total([1, 2]))\n",
    );
    let ctx = context(Config::default());
    let error = preview(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        &ctx,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::RenameConflict);
    assert!(error.message.contains("b.py:3"));
}

#[test]
fn edits_outside_the_project_are_dropped_with_an_outside_project_alarm() {
    let fixture = fixture();
    let outside = fixture.root.parent().unwrap().join("other").join("c.py");
    write(&outside, "from a import total\n");
    let mut changes = server_edits(&fixture);
    changes.push(text_change(&outside, 1, 15, 20, "sum_all"));
    let stage = CodeRename::new(Some(Arc::new(ScriptedProvider::renaming(changes))));
    let ctx = context(Config::default());
    let log = log();
    let outcome = execute(
        &stage,
        &rename_args(&fixture.a, &[lsp()]),
        StageInput::none(),
        &ctx,
        &log,
    )
    .unwrap();
    assert!(outcome.notable.contains(&Notable::OutsideProject));
    assert!(log.lock().unwrap().committed);
    assert_eq!(read(&outside), "from a import total\n");
    assert!(read(&fixture.b).contains("sum_all"));
}

#[test]
fn outside_project_alarm_is_shown_by_the_plan() {
    let fixture = fixture();
    let outside = fixture.root.parent().unwrap().join("other").join("c.py");
    write(&outside, "from a import total\n");
    let mut changes = server_edits(&fixture);
    changes.push(text_change(&outside, 1, 15, 20, "sum_all"));
    let stage = CodeRename::new(Some(Arc::new(ScriptedProvider::renaming(changes))));
    let plan = preview(
        &stage,
        &rename_args(&fixture.a, &[lsp()]),
        &context(Config::default()),
    )
    .unwrap();
    assert!(codes(&plan).contains(&Notable::OutsideProject));
    assert_eq!(edits(&plan).len(), 3);
}

#[test]
fn syntax_rename_with_another_definition_fails_with_e_symbol_ambiguous() {
    let fixture = fixture();
    write(&fixture.root.join("c.py"), "def total():\n    return 0\n");
    let ctx = context(Config::default());
    let error = preview(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        &ctx,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SymbolAmbiguous);
    assert!(error.message.contains("2 definitions"));
}

#[test]
fn server_edit_other_than_the_name_is_listed_as_an_alarm() {
    let fixture = fixture();
    let mut changes = server_edits(&fixture);
    changes.push(text_change(&fixture.b, 3, 1, 6, "log"));
    let stage = CodeRename::new(Some(Arc::new(ScriptedProvider::renaming(changes))));
    let plan = preview(
        &stage,
        &rename_args(&fixture.a, &[lsp()]),
        &context(Config::default()),
    )
    .unwrap();
    let Some(Value::List(alarms)) = summary(&plan).get("alarms").cloned() else {
        panic!("the summary has no alarms");
    };
    assert_eq!(alarms.len(), 1);
    assert!(
        matches!(&alarms[0], Value::Text(text) if text.contains("replaces `print` with `log`"))
    );
    assert_eq!(
        summary(&plan).get("source"),
        Some(&Value::Enum("lsp".to_owned()))
    );
}

#[test]
fn new_diagnostic_error_rolls_back_every_file() {
    let fixture = fixture();
    let before_a = std::fs::read(&fixture.a).unwrap();
    let before_b = std::fs::read(&fixture.b).unwrap();
    let mut provider = ScriptedProvider::renaming(server_edits(&fixture));
    provider.error_marker = Some((fixture.b.clone(), "sum_all".to_owned()));
    let stage = CodeRename::new(Some(Arc::new(provider)));
    let ctx = context(Config::default());
    let log = log();
    let error = execute(
        &stage,
        &rename_args(&fixture.a, &[lsp()]),
        StageInput::none(),
        &ctx,
        &log,
    )
    .err()
    .unwrap();
    assert_eq!(error.code(), ErrorCode::RolledBack);
    assert!(error.message.contains("b.py:1:1: name is not defined"));
    assert!(log.lock().unwrap().rolled_back);
    assert_eq!(std::fs::read(&fixture.a).unwrap(), before_a);
    assert_eq!(std::fs::read(&fixture.b).unwrap(), before_b);
}

#[test]
fn errors_present_before_the_rename_do_not_roll_it_back() {
    let fixture = fixture();
    write(&fixture.b, "from a import total\n\nprint(total([1, 2])\n");
    let ctx = context(Config::default());
    let log = log();
    execute(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        StageInput::none(),
        &ctx,
        &log,
    )
    .unwrap();
    assert!(log.lock().unwrap().committed);
    assert_eq!(
        read(&fixture.b),
        "from a import sum_all\n\nprint(sum_all([1, 2])\n"
    );
}

#[test]
fn completeness_check_reports_references_left_in_comments_and_docs() {
    let fixture = fixture();
    write(
        &fixture.a,
        "def total(values):\n    # total adds values\n    return sum(values)\n",
    );
    write(&fixture.root.join("docs.md"), "Use total.\n");
    let ctx = context(Config::default());
    let log = log();
    let outcome = execute(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        StageInput::none(),
        &ctx,
        &log,
    )
    .unwrap();
    assert!(outcome.notable.contains(&Notable::ReferencesLeft));
    assert!(!outcome.notable.contains(&Notable::RenameIncomplete));
    assert!(read(&fixture.a).contains("# total adds values"));
}

#[test]
fn include_comments_renames_occurrences_in_comments_only() {
    let fixture = fixture();
    write(
        &fixture.a,
        "def total(values):\n    # total adds values\n    return sum(values, 'total')\n",
    );
    let ctx = context(Config::default());
    let log = log();
    execute(
        &CodeRename::new(None),
        &rename_args(
            &fixture.a,
            &[syntax(), ("include_comments", Value::Bool(true))],
        ),
        StageInput::none(),
        &ctx,
        &log,
    )
    .unwrap();
    assert_eq!(
        read(&fixture.a),
        "def sum_all(values):\n    # sum_all adds values\n    return sum(values, 'total')\n"
    );
}

#[test]
fn include_strings_with_a_server_adds_string_occurrences() {
    let fixture = fixture();
    write(
        &fixture.b,
        "from a import total\n\nprint(total([1, 2]), \"total\")\n",
    );
    let stage = CodeRename::new(Some(Arc::new(ScriptedProvider::renaming(server_edits(
        &fixture,
    )))));
    let ctx = context(Config::default());
    execute(
        &stage,
        &rename_args(&fixture.a, &[lsp(), ("include_strings", Value::Bool(true))]),
        StageInput::none(),
        &ctx,
        &log(),
    )
    .unwrap();
    assert_eq!(
        read(&fixture.b),
        "from a import sum_all\n\nprint(sum_all([1, 2]), \"sum_all\")\n"
    );
}

#[test]
fn code_left_by_the_server_is_reported_as_rename_incomplete() {
    let fixture = fixture();
    let changes = vec![text_change(&fixture.a, 1, 5, 10, "sum_all")];
    let stage = CodeRename::new(Some(Arc::new(ScriptedProvider::renaming(changes))));
    let ctx = context(Config::default());
    let outcome = execute(
        &stage,
        &rename_args(&fixture.a, &[lsp()]),
        StageInput::none(),
        &ctx,
        &log(),
    )
    .unwrap();
    assert!(outcome.notable.contains(&Notable::RenameIncomplete));
    assert!(outcome.notable.contains(&Notable::ReferencesLeft));
}

#[test]
fn check_references_false_skips_the_completeness_check() {
    let fixture = fixture();
    write(&fixture.root.join("docs.md"), "Use total.\n");
    let ctx = context(Config::default());
    let outcome = execute(
        &CodeRename::new(None),
        &rename_args(
            &fixture.a,
            &[syntax(), ("check_references", Value::Bool(false))],
        ),
        StageInput::none(),
        &ctx,
        &log(),
    )
    .unwrap();
    assert!(outcome.notable.is_empty());
}

#[test]
fn resource_operations_are_applied_and_undone_with_the_text_changes() {
    let fixture = fixture();
    let old = fixture.root.join("old.py");
    let new = fixture.root.join("pkg").join("new.py");
    let created = fixture.root.join("created.py");
    let gone = fixture.root.join("gone.py");
    write(&old, "x = 1\n");
    write(&gone, "y = 2\n");
    std::fs::create_dir_all(new.parent().unwrap()).unwrap();
    let mut changes = server_edits(&fixture);
    changes.push(DocumentChange::Resource(ResourceChange::Rename {
        from: old.clone(),
        to: new.clone(),
        overwrite: false,
        ignore_if_exists: false,
    }));
    changes.push(DocumentChange::Resource(ResourceChange::Create {
        path: created.clone(),
        overwrite: false,
        ignore_if_exists: false,
    }));
    changes.push(text_change(&created, 1, 1, 1, "z = 3\n"));
    changes.push(DocumentChange::Resource(ResourceChange::Delete {
        path: gone.clone(),
        recursive: false,
        ignore_if_not_exists: false,
    }));
    let stage = CodeRename::new(Some(Arc::new(ScriptedProvider::renaming(changes))));
    let ctx = context(Config::default());
    let log = log();
    let outcome = execute(
        &stage,
        &rename_args(&fixture.a, &[lsp()]),
        StageInput::none(),
        &ctx,
        &log,
    )
    .unwrap();
    assert!(!old.exists());
    assert_eq!(read(&new), "x = 1\n");
    assert_eq!(read(&created), "z = 3\n");
    assert!(!gone.exists());
    let actions: Vec<ChangeAction> = outcome.changes.iter().map(|change| change.action).collect();
    for action in [
        ChangeAction::Rename,
        ChangeAction::Create,
        ChangeAction::Delete,
        ChangeAction::Modify,
    ] {
        assert!(actions.contains(&action), "missing {action:?}");
    }
    log.lock().unwrap().restore();
    assert_eq!(read(&old), "x = 1\n");
    assert!(!new.exists());
    assert!(!created.exists());
    assert_eq!(read(&gone), "y = 2\n");
    assert_eq!(read(&fixture.a), A);
}

#[test]
fn renamed_file_with_edits_becomes_a_deletion_and_a_creation() {
    let fixture = fixture();
    let old = fixture.root.join("m.py");
    let new = fixture.root.join("n.py");
    write(&old, "def total(): pass\n");
    write(&fixture.a, "from m import total\n");
    let changes = vec![
        DocumentChange::Resource(ResourceChange::Rename {
            from: old.clone(),
            to: new.clone(),
            overwrite: false,
            ignore_if_exists: false,
        }),
        text_change(&new, 1, 5, 10, "sum_all"),
        text_change(&fixture.a, 1, 15, 20, "sum_all"),
    ];
    let stage = CodeRename::new(Some(Arc::new(ScriptedProvider::renaming(changes))));
    let ctx = context(Config::default());
    let arguments = rename_args(&fixture.a, &[lsp(), ("column", Value::Int(15))]);
    execute(&stage, &arguments, StageInput::none(), &ctx, &log()).unwrap();
    assert!(!old.exists());
    assert_eq!(read(&new), "def sum_all(): pass\n");
    assert_eq!(read(&fixture.a), "from m import sum_all\n");
}

#[test]
fn source_lsp_without_a_provider_fails_with_e_external_missing() {
    let fixture = fixture();
    let error = preview(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[lsp()]),
        &context(Config::default()),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
}

#[test]
fn source_lsp_with_a_failing_server_fails_with_e_external_failed() {
    let fixture = fixture();
    let stage = CodeRename::new(Some(Arc::new(ScriptedProvider::failing(
        SemanticErrorKind::Failed,
    ))));
    let error = preview(
        &stage,
        &rename_args(&fixture.a, &[lsp()]),
        &context(Config::default()),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ExternalFailed);
}

#[test]
fn source_auto_falls_back_to_syntax_with_a_semantic_fallback_note() {
    let fixture = fixture();
    let stage = CodeRename::new(Some(Arc::new(ScriptedProvider::failing(
        SemanticErrorKind::Unavailable,
    ))));
    let plan = preview(
        &stage,
        &rename_args(&fixture.a, &[]),
        &context(Config::default()),
    )
    .unwrap();
    assert!(codes(&plan).contains(&Notable::SemanticFallback));
    assert_eq!(
        summary(&plan).get("source"),
        Some(&Value::Enum("syntax".to_owned()))
    );
    assert_eq!(edits(&plan).len(), 3);
}

#[test]
fn scope_file_changes_only_the_file_and_reports_the_rest() {
    let fixture = fixture();
    let stage = CodeRename::new(Some(Arc::new(ScriptedProvider::renaming(server_edits(
        &fixture,
    )))));
    let ctx = context(Config::default());
    let arguments = rename_args(
        &fixture.a,
        &[lsp(), ("scope", Value::Enum("file".to_owned()))],
    );
    let outcome = execute(&stage, &arguments, StageInput::none(), &ctx, &log()).unwrap();
    assert_eq!(read(&fixture.b), B);
    assert!(read(&fixture.a).starts_with("def sum_all"));
    assert_eq!(outcome.changes.len(), 1);
}

#[test]
fn scope_file_plan_notes_references_in_other_files() {
    let fixture = fixture();
    let stage = CodeRename::new(Some(Arc::new(ScriptedProvider::renaming(server_edits(
        &fixture,
    )))));
    let arguments = rename_args(
        &fixture.a,
        &[lsp(), ("scope", Value::Enum("file".to_owned()))],
    );
    let plan = preview(&stage, &arguments, &context(Config::default())).unwrap();
    assert!(codes(&plan).contains(&Notable::RenameIncomplete));
}

#[test]
fn paths_limit_the_syntax_rename() {
    let fixture = fixture();
    let ctx = context(Config::default());
    let arguments = rename_args(
        &fixture.a,
        &[
            syntax(),
            ("paths", Value::List(vec![path_value(&fixture.a)])),
        ],
    );
    execute(
        &CodeRename::new(None),
        &arguments,
        StageInput::none(),
        &ctx,
        &log(),
    )
    .unwrap();
    assert_eq!(read(&fixture.b), B);
    assert!(read(&fixture.a).starts_with("def sum_all"));
}

#[test]
fn invalid_or_unchanged_new_names_are_rejected() {
    let fixture = fixture();
    let ctx = context(Config::default());
    let stage = CodeRename::new(None);
    let invalid = args(&[
        ("to", Value::Text("1st".to_owned())),
        ("path", path_value(&fixture.a)),
        ("line", Value::Int(1)),
    ]);
    assert_eq!(
        preview(&stage, &invalid, &ctx).unwrap_err().code(),
        ErrorCode::InvalidValue
    );
    let same = args(&[
        ("to", Value::Text("total".to_owned())),
        ("path", path_value(&fixture.a)),
        ("line", Value::Int(1)),
        ("column", Value::Int(5)),
    ]);
    assert_eq!(
        preview(&stage, &same, &ctx).unwrap_err().code(),
        ErrorCode::InvalidValue
    );
    let missing = args(&[
        ("to", Value::Text("x".to_owned())),
        ("path", path_value(&fixture.a)),
    ]);
    assert_eq!(
        preview(&stage, &missing, &ctx).unwrap_err().code(),
        ErrorCode::MissingParam
    );
}

#[test]
fn symbol_by_name_is_found_or_reported() {
    let fixture = fixture();
    let ctx = context(Config::default());
    let stage = CodeRename::new(None);
    let by_name = args(&[
        ("to", Value::Text("sum_all".to_owned())),
        ("path", path_value(&fixture.a)),
        ("symbol", Value::Text("total".to_owned())),
        syntax(),
    ]);
    assert_eq!(edits(&preview(&stage, &by_name, &ctx).unwrap()).len(), 3);
    let unknown = args(&[
        ("to", Value::Text("sum_all".to_owned())),
        ("path", path_value(&fixture.a)),
        ("symbol", Value::Text("missing".to_owned())),
    ]);
    assert_eq!(
        preview(&stage, &unknown, &ctx).unwrap_err().code(),
        ErrorCode::SymbolNotFound
    );
    write(
        &fixture.a,
        "def total():\n    pass\ndef total():\n    pass\n",
    );
    let error = preview(&stage, &by_name, &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::SymbolAmbiguous);
    assert!(error.message.contains("2 definitions"));
}

#[test]
fn line_without_column_uses_the_first_identifier_of_the_line() {
    let fixture = fixture();
    let arguments = args(&[
        ("to", Value::Text("sum_all".to_owned())),
        ("path", path_value(&fixture.a)),
        ("line", Value::Int(1)),
        syntax(),
    ]);
    let plan = preview(
        &CodeRename::new(None),
        &arguments,
        &context(Config::default()),
    )
    .unwrap();
    assert_eq!(
        summary(&plan).get("from"),
        Some(&Value::Text("total".to_owned()))
    );
}

#[test]
fn position_without_an_identifier_fails_with_e_symbol_not_found() {
    let fixture = fixture();
    write(&fixture.a, "def total(values):\n\n    return 1\n");
    let arguments = args(&[
        ("to", Value::Text("sum_all".to_owned())),
        ("path", path_value(&fixture.a)),
        ("line", Value::Int(2)),
        ("column", Value::Int(1)),
    ]);
    let error = preview(
        &CodeRename::new(None),
        &arguments,
        &context(Config::default()),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SymbolNotFound);
}

#[test]
fn stage_form_renames_the_input_symbol() {
    let fixture = fixture();
    let symbol = Symbol {
        name: "total".to_owned(),
        path: RecordPath::new(fixture.a.to_string_lossy()),
        line: 1,
        column: 1,
        end_line: 2,
        language: "python".to_owned(),
        source: SymbolSource::Syntax,
        ..Symbol::default()
    };
    let input = StageInput::records(RecordStream::from_records(vec![Record::from(symbol)]));
    let arguments = args(&[("to", Value::Text("sum_all".to_owned())), syntax()]);
    let ctx = context(Config::default());
    execute(&CodeRename::new(None), &arguments, input, &ctx, &log()).unwrap();
    assert!(read(&fixture.b).contains("print(sum_all("));
}

#[test]
fn typescript_and_javascript_files_are_one_language_family() {
    let (_dir, root) = project();
    let lib = root.join("lib.ts");
    let app = root.join("app.js");
    let notes = root.join("notes.py");
    write(&lib, "export function total(a) { return a; }\n");
    write(&app, "import { total } from './lib';\ntotal(1);\n");
    write(&notes, "total = 0\n");
    let arguments = args(&[
        ("to", Value::Text("sumAll".to_owned())),
        ("path", path_value(&lib)),
        ("symbol", Value::Text("total".to_owned())),
        syntax(),
    ]);
    execute(
        &CodeRename::new(None),
        &arguments,
        StageInput::none(),
        &context(Config::default()),
        &log(),
    )
    .unwrap();
    assert_eq!(read(&app), "import { sumAll } from './lib';\nsumAll(1);\n");
    assert_eq!(read(&notes), "total = 0\n");
}

#[test]
fn file_without_syntax_support_needs_a_server() {
    let (_dir, root) = project();
    let file = root.join("main.zig");
    write(&file, "fn total() void {}\n");
    let error = preview(
        &CodeRename::new(None),
        &rename_args(&file, &[syntax()]),
        &context(Config::default()),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
}

fn config_with_max_files(max: u32) -> Config {
    let mut config = Config::default();
    config.code.index_max_files = max;
    config
}

#[test]
fn syntax_rename_notes_that_the_file_listing_was_cut() {
    let fixture = fixture();
    write(&fixture.root.join("c.py"), "from a import total\n");
    let plan = preview(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        &context(config_with_max_files(2)),
    )
    .unwrap();
    let cut = plan
        .notable
        .iter()
        .find(|entry| entry.code == Notable::RenameIncomplete)
        .expect("a rename_incomplete note");
    assert!(
        cut.detail
            .as_deref()
            .unwrap()
            .contains("code.index_max_files (2)")
    );
    assert_eq!(edits(&plan).len(), 3);
}

#[test]
fn syntax_rename_within_the_file_limit_has_no_listing_note() {
    let fixture = fixture();
    let plan = preview(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        &context(config_with_max_files(2)),
    )
    .unwrap();
    assert!(!codes(&plan).contains(&Notable::RenameIncomplete));
    assert_eq!(edits(&plan).len(), 3);
}

#[cfg(target_os = "linux")]
#[test]
fn syntax_rename_reaches_files_whose_names_are_not_utf8() {
    use std::os::unix::ffi::OsStrExt;

    let fixture = fixture();
    let odd = fixture
        .root
        .join(std::ffi::OsStr::from_bytes(b"caf\xe9.py"));
    std::fs::write(&odd, "from a import total\n").unwrap();
    execute(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        StageInput::none(),
        &context(Config::default()),
        &log(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&odd).unwrap(),
        "from a import sum_all\n"
    );
}

#[test]
fn file_that_is_not_utf8_is_left_alone_by_a_syntax_rename() {
    let fixture = fixture();
    std::fs::write(fixture.root.join("legacy.py"), b"# total \xe9\n").unwrap();
    let outcome = execute(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        StageInput::none(),
        &context(Config::default()),
        &log(),
    )
    .unwrap();
    assert_eq!(outcome.changes.len(), 2);
    assert_eq!(
        std::fs::read(fixture.root.join("legacy.py")).unwrap(),
        b"# total \xe9\n"
    );
}

fn leftover_lines(plan: &EffectPlan) -> Vec<String> {
    let Some(Value::List(items)) = summary(plan).get("leftovers").cloned() else {
        panic!("the summary has no leftovers");
    };
    items
        .into_iter()
        .filter_map(|item| match item {
            Value::Text(text) => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn files_that_cannot_be_read_as_text_are_reported_when_they_hold_the_name() {
    let fixture = fixture();
    std::fs::write(fixture.root.join("legacy.py"), b"# total \xe9\n").unwrap();
    std::fs::write(fixture.root.join("blob.bin"), b"\x00\xff\xfe no match \x00").unwrap();
    let plan = preview(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        &context(Config::default()),
    )
    .unwrap();
    let lines = leftover_lines(&plan);
    assert!(
        lines
            .iter()
            .any(|line| line.ends_with("legacy.py:0 unreadable")),
        "{lines:?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("blob.bin")),
        "{lines:?}"
    );
    assert!(codes(&plan).contains(&Notable::ReferencesLeft));
}

#[test]
fn files_over_the_size_limit_are_reported_when_they_hold_the_name() {
    let fixture = fixture();
    let mut config = Config::default();
    config.code.file_bytes_max = SizeSetting::from_bytes(64);
    write(
        &fixture.root.join("big.py"),
        &"# total of everything\n".repeat(10),
    );
    write(
        &fixture.root.join("big_clean.py"),
        &"# nothing here at all\n".repeat(10),
    );
    let plan = preview(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        &context(config),
    )
    .unwrap();
    let lines = leftover_lines(&plan);
    assert!(
        lines
            .iter()
            .any(|line| line.ends_with("big.py:0 unreadable")),
        "{lines:?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("big_clean.py")),
        "{lines:?}"
    );
}

/// Listing service that lists the given files and reports one directory it
/// cannot read.
struct PartlyReadableListing {
    files: Vec<PathBuf>,
}

impl nurt_contracts::services::FileListService for PartlyReadableListing {
    fn walk<'a>(
        &'a self,
        _spec: &nurt_contracts::services::WalkSpec,
    ) -> nurt_contracts::error::Result<nurt_contracts::services::WalkIter<'a>> {
        let mut items: Vec<nurt_contracts::error::Result<nurt_contracts::services::WalkEntry>> =
            Vec::new();
        items.push(Err(nurt_contracts::error::Error::new(
            ErrorCode::Io,
            "cannot read a directory",
        )));
        for path in &self.files {
            items.push(Ok(nurt_contracts::services::WalkEntry {
                path: path.clone(),
                depth: 1,
                is_dir: false,
            }));
        }
        Ok(Box::new(items.into_iter()))
    }
}

#[test]
fn an_unreadable_directory_does_not_abort_the_rename() {
    use nurt_contracts::services::Services;
    use u14_support::{StdFs, TestClock, TestHost, ToyCode};

    let fixture = fixture();
    let listing = PartlyReadableListing {
        files: vec![fixture.a.clone(), fixture.b.clone()],
    };
    let ctx =
        nurt_contracts::stage::ExecCtx::builder(Arc::new(TestClock), Arc::new(TestHost::new()))
            .fs(Arc::new(StdFs))
            .services(Arc::new(
                Services::new()
                    .with_code(Arc::new(ToyCode))
                    .with_file_list(Arc::new(listing)),
            ))
            .config(Arc::new(Config::default()))
            .build();
    let plan = preview(
        &CodeRename::new(None),
        &rename_args(&fixture.a, &[syntax()]),
        &ctx,
    )
    .unwrap();
    assert_eq!(edits(&plan).len(), 3);
    let note = plan
        .notable
        .iter()
        .find(|entry| entry.code == Notable::SourceUnavailable)
        .expect("a source_unavailable note");
    assert!(note.detail.as_deref().unwrap().contains("1 directories"));
}

trait EffectAccess {
    fn as_effect_stage(&self) -> &dyn nurt_contracts::stage::EffectStage;
}

impl EffectAccess for CodeRename {
    fn as_effect_stage(&self) -> &dyn nurt_contracts::stage::EffectStage {
        nurt_contracts::stage::StageImpl::as_effect(self).unwrap()
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
