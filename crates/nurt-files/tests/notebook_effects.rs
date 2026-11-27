// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/notebook_effects.rs
// Responsibility:   Journal coverage, entity tag checks, lossless writes and format errors of the `notebook.*` commands.
// Dependencies:     notebook_support, nurt_contracts, nurt_testkit, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Journal coverage, entity tag checks, lossless writes and format errors of
//! the `notebook.*` commands.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

mod notebook_support;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::{Follow, Fs};
use nurt_contracts::journal::{JournalStore, TargetIndex, TargetPath};
use nurt_contracts::record::ChangeAction;
use nurt_contracts::stage::{ArgValue, StageInput};
use nurt_contracts::value::{Regex, Size, Value};
use nurt_files::common::etag::etag_text;
use nurt_testkit::fixtures::begin_request;
use serde_json::json;

use notebook_support::{
    INTRO, LOAD, PLOT, SALES, abs, applied, args, choice, declared, journal_untouched, language,
    map, notebook, plan, read, run, sales, stage, text, with_notebook,
};

/// Arguments of one call of every command that changes a file.
fn every_change() -> Vec<nurt_contracts::stage::BoundArgs> {
    vec![
        args(
            "notebook.insert",
            SALES,
            &[("cell_type", choice("code")), ("source", text("x = 1"))],
        ),
        args(
            "notebook.replace",
            SALES,
            &[("index", Value::Int(0)), ("source", text("# Revenue"))],
        ),
        args(
            "notebook.move",
            SALES,
            &[("from", Value::Int(0)), ("to", Value::Int(2))],
        ),
        args(
            "notebook.copy",
            SALES,
            &[("from", Value::Int(2)), ("to", Value::Int(0))],
        ),
        args("notebook.remove", SALES, &[("id", text("load"))]),
        args("notebook.clear_outputs", SALES, &[]),
        args("notebook.normalize", SALES, &[]),
        args(
            "notebook.ops",
            SALES,
            &[(
                "changes",
                Value::List(vec![map(&[
                    ("action", text("remove")),
                    ("index", Value::Int(1)),
                ])]),
            )],
        ),
        args(
            "notebook.export",
            SALES,
            &[
                ("to", Value::Path(language("/home/user/sales.py"))),
                ("format", choice("script")),
            ],
        ),
    ]
}

#[test]
fn every_change_goes_through_the_transaction() {
    for args in every_change() {
        let command = args.command().to_owned();
        let original = if command == "notebook.normalize" {
            notebook(&[INTRO, LOAD, PLOT]).replace("\"nbformat_minor\": 5", "\"nbformat_minor\": 4")
        } else {
            sales()
        };
        let test = with_notebook(SALES, &original);
        let effect = run(&test, &args).expect("the command runs");
        let target = if command == "notebook.export" {
            abs("/home/user/sales.py")
        } else {
            abs(SALES)
        };
        assert_eq!(declared(&test), [TargetPath::Fs(target)], "{command}");
        assert_eq!(applied(&test), [TargetIndex(0)], "{command}");
        assert_eq!(effect.targets, Some(1), "{command}");
        let changes = effect.changes.expect("the effect lists its changes");
        let expected_action = if command == "notebook.export" {
            ChangeAction::Create
        } else {
            ChangeAction::Modify
        };
        assert_eq!(changes[0].action, expected_action, "{command}");
        assert_eq!(effect.kind, command);
    }
}

#[test]
fn the_plan_reports_the_change_without_writing() {
    let test = with_notebook(SALES, &sales());
    let changes = plan(
        &test,
        &args("notebook.remove", SALES, &[("index", Value::Int(2))]),
    )
    .expect("the plan runs");
    assert_eq!(changes.len(), 1);
    let change = &changes[0];
    assert_eq!(change.action, ChangeAction::Modify);
    assert_eq!(
        change.size_before,
        Some(Size::from_bytes(sales().len() as u64))
    );
    assert_eq!(
        change.size_after,
        Some(Size::from_bytes(notebook(&[INTRO, LOAD]).len() as u64))
    );
    assert_eq!(change.added, Some(0));
    assert!(change.removed.is_some_and(|removed| removed > 0));
    let diff = change
        .diff
        .as_ref()
        .expect("the plan carries the difference");
    assert!(
        diff.iter()
            .flat_map(|hunk| &hunk.lines)
            .any(|line| line.contains("\"id\": \"plot\""))
    );
    assert_eq!(read(&test, SALES), sales());
    assert!(journal_untouched(&test));
}

#[test]
fn if_match_with_an_old_tag_fails_with_stale() {
    let test = with_notebook(SALES, &sales());
    let fs = test.fs();
    let old = fs
        .metadata(&abs(SALES), Follow::NoFollow)
        .expect("the notebook exists")
        .etag();
    fs.write_file(&abs(SALES), notebook(&[INTRO, LOAD]).as_bytes())
        .expect("the virtual file system accepts the file");
    let mut call = args("notebook.remove", SALES, &[("index", Value::Int(0))]);
    call.insert("if_match", ArgValue::Value(text(&etag_text(&old))), false);
    let error = run(&test, &call).expect_err("the tag is old");
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(read(&test, SALES), notebook(&[INTRO, LOAD]));
    assert!(journal_untouched(&test));
}

#[test]
fn if_match_with_the_current_tag_passes() {
    let test = with_notebook(SALES, &sales());
    let current = test
        .fs()
        .metadata(&abs(SALES), Follow::NoFollow)
        .expect("the notebook exists")
        .etag();
    let mut call = args("notebook.remove", SALES, &[("index", Value::Int(0))]);
    call.insert(
        "if_match",
        ArgValue::Value(text(&etag_text(&current))),
        false,
    );
    run(&test, &call).expect("the tag is current");
    assert_eq!(read(&test, SALES), notebook(&[LOAD, PLOT]));
}

#[test]
fn a_notebook_changed_after_the_plan_is_not_overwritten() {
    let test = with_notebook(SALES, &sales());
    let call = args("notebook.remove", SALES, &[("index", Value::Int(0))]);
    let stage = stage("notebook.remove");
    let effect = stage.as_effect().expect("remove changes files");
    let ctx = test.ctx();
    let plan = effect
        .plan(&call, StageInput::none(), &ctx)
        .expect("the plan runs");
    let changed = notebook(&[PLOT]);
    test.fs()
        .write_file(&abs(SALES), changed.as_bytes())
        .expect("the virtual file system accepts the file");
    let mut tx = test.journal().begin(begin_request("notebook.remove"));
    let error = effect
        .apply(&call, plan, tx.as_mut(), &ctx)
        .expect_err("the notebook changed after the plan");
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(read(&test, SALES), changed);
    assert!(applied(&test).is_empty());
}

#[test]
fn cells_carry_the_tag_accepted_by_if_match() {
    let test = with_notebook(SALES, &sales());
    let cells = notebook_support::cells(&test, &args("notebook.cells", SALES, &[]))
        .expect("the cells are listed");
    let mut call = args("notebook.remove", SALES, &[("index", Value::Int(0))]);
    call.insert("if_match", ArgValue::Value(text(&cells[0].etag)), false);
    run(&test, &call).expect("the tag of the listing is current");
}

#[test]
fn unchanged_cells_keep_their_bytes_in_every_layout() {
    let cases: [(&str, &str, &str, &str); 4] = [
        (
            "two spaces, sources as text",
            "{\n  \"cells\": [\n    {\n      \"cell_type\": \"code\",\n      \"execution_count\": null,\n      \"metadata\": {},\n      \"outputs\": [],\n      \"source\": \"a = 1\\nb = 2\"\n    },\n    {\n      \"cell_type\": \"markdown\",\n      \"metadata\": {},\n      \"source\": \"Text\"\n    }\n  ],\n  \"metadata\": {},\n  \"nbformat\": 4,\n  \"nbformat_minor\": 4\n}\n",
            "\"a = 1\\nb = 2\"",
            "\"a = 10\\nb = 2\"",
        ),
        (
            "four spaces, sources as lists",
            "{\n    \"cells\": [\n        {\n            \"cell_type\": \"code\",\n            \"execution_count\": null,\n            \"metadata\": {},\n            \"outputs\": [],\n            \"source\": [\n                \"a = 1\\n\",\n                \"b = 2\"\n            ]\n        },\n        {\n            \"cell_type\": \"markdown\",\n            \"metadata\": {},\n            \"source\": [\n                \"Text\"\n            ]\n        }\n    ],\n    \"metadata\": {},\n    \"nbformat\": 4,\n    \"nbformat_minor\": 4\n}",
            "\"a = 1\\n\"",
            "\"a = 10\\n\"",
        ),
        (
            "one line",
            "{\"cells\": [{\"cell_type\": \"code\", \"execution_count\": null, \"metadata\": {}, \"outputs\": [], \"source\": [\"a = 1\\n\", \"b = 2\"]}, {\"cell_type\": \"markdown\", \"metadata\": {}, \"source\": [\"Text\"]}], \"metadata\": {}, \"nbformat\": 4, \"nbformat_minor\": 4}",
            "\"a = 1\\n\"",
            "\"a = 10\\n\"",
        ),
        (
            "Jupyter layout with images",
            "",
            "\"import pandas as pd\\n\"",
            "\"import polars as pd\\n\"",
        ),
    ];
    for (layout, original, before, after) in cases {
        let original = if original.is_empty() {
            sales()
        } else {
            original.to_owned()
        };
        let source = if layout.starts_with("Jupyter") {
            "import polars as pd\ndf = pd.read_csv(\"sales.csv\")\nprint(len(df), \"rows\")"
        } else {
            "a = 10\nb = 2"
        };
        let index = if layout.starts_with("Jupyter") { 1 } else { 0 };
        let test = with_notebook(SALES, &original);
        run(
            &test,
            &args(
                "notebook.replace",
                SALES,
                &[("index", Value::Int(index)), ("source", text(source))],
            ),
        )
        .expect("the replacement runs");
        assert_eq!(
            read(&test, SALES),
            original.replacen(before, after, 1),
            "{layout}"
        );
    }
}

#[test]
fn an_image_output_survives_edits_of_other_cells() {
    let test = with_notebook(SALES, &sales());
    run(
        &test,
        &args(
            "notebook.insert",
            SALES,
            &[
                ("index", Value::Int(0)),
                ("cell_type", choice("raw")),
                ("source", text("draft")),
            ],
        ),
    )
    .expect("the insertion runs");
    run(
        &test,
        &args("notebook.remove", SALES, &[("index", Value::Int(0))]),
    )
    .expect("the removal runs");
    assert_eq!(read(&test, SALES), sales());
}

#[test]
fn a_notebook_without_changes_is_not_written() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args(
            "notebook.move",
            SALES,
            &[("from", Value::Int(1)), ("to", Value::Int(1))],
        ),
    )
    .expect("the move runs");
    assert_eq!(effect.targets, Some(0));
    assert_eq!(effect.changes.map(|changes| changes.len()), Some(0));
    assert!(journal_untouched(&test));
}

#[test]
fn old_format_versions_fail_with_format() {
    let test = with_notebook(
        SALES,
        "{\"nbformat\": 3, \"nbformat_minor\": 0, \"worksheets\": []}",
    );
    let error = notebook_support::cells(&test, &args("notebook.cells", SALES, &[]))
        .expect_err("nbformat 3 is not supported");
    assert_eq!(error.code(), ErrorCode::Format);
    assert_eq!(error.expected, Some(json!(4)));
    assert_eq!(error.actual, Some(json!(3)));
    assert!(error.message.contains("nbconvert"), "{}", error.message);
    assert_eq!(
        error
            .location
            .as_ref()
            .map(|location| location.path.clone()),
        Some(language(SALES))
    );
}

#[test]
fn broken_json_fails_with_format_and_its_position() {
    let test = with_notebook(SALES, "{\n \"cells\": [\n  {,\n ]\n}\n");
    let error =
        run(&test, &args("notebook.clear_outputs", SALES, &[])).expect_err("the file is not JSON");
    assert_eq!(error.code(), ErrorCode::Format);
    let location = error.location.clone().expect("the error has a location");
    assert_eq!(location.line, Some(3));
    assert!(journal_untouched(&test));
}

#[test]
fn files_that_are_not_utf8_fail_with_format() {
    let test = notebook_support::with_notebook(SALES, "");
    test.fs()
        .write_file(&abs(SALES), b"{\"cells\": [], \"x\": \"\xff\"}")
        .expect("the virtual file system accepts the file");
    let error = notebook_support::cells(&test, &args("notebook.cells", SALES, &[]))
        .expect_err("the file is not UTF-8");
    assert_eq!(error.code(), ErrorCode::Format);
    assert!(
        error
            .location
            .as_ref()
            .is_some_and(|location| location.byte.is_some())
    );
}

#[test]
fn an_unknown_identifier_suggests_close_ones() {
    let test = with_notebook(SALES, &sales());
    let error = run(
        &test,
        &args("notebook.remove", SALES, &[("id", text("lod"))]),
    )
    .expect_err("no cell has the identifier");
    assert_eq!(error.code(), ErrorCode::Arg);
    assert!(error.message.contains("load"), "{}", error.message);
    assert_eq!(error.found.as_deref(), Some("lod"));
}

#[test]
fn a_missing_match_fails_with_no_match() {
    let test = with_notebook(SALES, &sales());
    let error = run(
        &test,
        &args(
            "notebook.replace",
            SALES,
            &[("find", text("polars")), ("with", text("pandas"))],
        ),
    )
    .expect_err("no cell contains the text");
    assert_eq!(error.code(), ErrorCode::NoMatch);
    assert_eq!(read(&test, SALES), sales());
}

#[test]
fn an_invalid_replacement_template_changes_nothing() {
    let test = with_notebook(SALES, &sales());
    let error = run(
        &test,
        &args(
            "notebook.replace",
            SALES,
            &[
                (
                    "find",
                    Value::Regex(Regex::new(r"read_(\w+)", "").expect("valid flags")),
                ),
                ("with", text("load_$2")),
            ],
        ),
    )
    .expect_err("group 2 does not exist");
    assert_eq!(error.code(), ErrorCode::Regex);
    assert_eq!(read(&test, SALES), sales());
}

#[test]
fn ops_accept_regular_expressions_with_templates() {
    let test = with_notebook(SALES, &sales());
    let changes = Value::List(vec![map(&[
        ("action", text("replace")),
        ("id", text("load")),
        (
            "find",
            Value::Regex(Regex::new(r"read_(\w+)", "").expect("valid flags")),
        ),
        ("with", text("load_$1")),
    ])]);
    run(&test, &args("notebook.ops", SALES, &[("changes", changes)])).expect("the change runs");
    let load = LOAD.replace("pd.read_csv", "pd.load_csv");
    assert_eq!(read(&test, SALES), notebook(&[INTRO, &load, PLOT]));
}

#[test]
fn export_does_not_overwrite_a_file() {
    let test = with_notebook(SALES, &sales());
    test.fs()
        .write_file(&abs("/home/user/sales.py"), b"keep\n")
        .expect("the virtual file system accepts the file");
    let error = run(
        &test,
        &args(
            "notebook.export",
            SALES,
            &[
                ("to", Value::Path(language("/home/user/sales.py"))),
                ("format", choice("script")),
            ],
        ),
    )
    .expect_err("the target exists");
    assert_eq!(error.code(), ErrorCode::Exists);
    assert_eq!(read(&test, "/home/user/sales.py"), "keep\n");
    assert!(journal_untouched(&test));
}

#[test]
fn every_command_is_registered_once() {
    let commands: Vec<String> = nurt_files::stages()
        .into_iter()
        .map(|stage| stage.descriptor().command)
        .filter(|command| command.starts_with("notebook."))
        .collect();
    assert_eq!(
        commands,
        [
            "notebook.cells",
            "notebook.insert",
            "notebook.replace",
            "notebook.move",
            "notebook.copy",
            "notebook.remove",
            "notebook.clear_outputs",
            "notebook.normalize",
            "notebook.ops",
            "notebook.export",
        ]
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
