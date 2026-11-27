// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the conversion of `WorkspaceEdit` answers to workspace changes.

// Tests that write POSIX paths run on Unix only; their helpers are unused on Windows.
#![cfg_attr(windows, allow(dead_code, unused_imports))]
// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use nurt_code::semantic::{DocumentChange, ResourceChange, SemanticErrorKind, TextChange};
use nurt_contracts::services::TextPosition;
use nurt_lspclient::rename::{PositionUnit, workspace_change};
use serde_json::{Value, json};

/// Line with a character outside the Basic Multilingual Plane, which takes
/// two UTF-16 code units and four UTF-8 code units: `old` starts at UTF-16
/// offset 9, UTF-8 offset 11 and character 8.
const WIDE: &str = "let \u{1f600} = old;\n";

fn range(line: u32, start: u32, end: u32) -> Value {
    json!({"start": {"line": line, "character": start}, "end": {"line": line, "character": end}})
}

fn texts(change: &[DocumentChange]) -> Vec<&TextChange> {
    change
        .iter()
        .filter_map(|change| match change {
            DocumentChange::Text(text) => Some(text),
            DocumentChange::Resource(_) => None,
        })
        .collect()
}

fn at(line: u32, column: u32) -> TextPosition {
    TextPosition { line, column }
}

fn only(path: &'static str, text: &'static str) -> impl Fn(&Path) -> Option<String> {
    move |asked: &Path| (asked == Path::new(path)).then(|| text.to_owned())
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn utf16_offsets_become_character_columns() {
    let edit =
        json!({"changes": {"file:///w/a.ts": [{"range": range(0, 9, 12), "newText": "total"}]}});
    let change = workspace_change(&edit, PositionUnit::Utf16, &only("/w/a.ts", WIDE)).unwrap();
    let edits = texts(&change.changes);
    assert_eq!(edits[0].range.start, at(1, 9));
    assert_eq!(edits[0].range.end, at(1, 12));
    assert_eq!(edits[0].range.path, PathBuf::from("/w/a.ts"));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn utf8_and_utf32_offsets_are_converted_by_their_unit() {
    let utf8 =
        json!({"changes": {"file:///w/a.ts": [{"range": range(0, 11, 14), "newText": "x"}]}});
    let change = workspace_change(&utf8, PositionUnit::Utf8, &only("/w/a.ts", WIDE)).unwrap();
    assert_eq!(texts(&change.changes)[0].range.start, at(1, 9));
    let utf32 =
        json!({"changes": {"file:///w/a.ts": [{"range": range(0, 8, 11), "newText": "x"}]}});
    let change = workspace_change(&utf32, PositionUnit::Utf32, &only("/w/a.ts", WIDE)).unwrap();
    assert_eq!(texts(&change.changes)[0].range.start, at(1, 9));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn changes_map_is_ordered_by_path_and_position() {
    let edit = json!({"changes": {
        "file:///w/b.py": [
            {"range": range(2, 0, 3), "newText": "y"},
            {"range": range(0, 0, 3), "newText": "y"}
        ],
        "file:///w/a.py": [{"range": range(0, 4, 7), "newText": "y"}]
    }});
    let change = workspace_change(&edit, PositionUnit::Utf16, &|_: &Path| None).unwrap();
    let order: Vec<(PathBuf, u32)> = texts(&change.changes)
        .iter()
        .map(|text| (text.range.path.clone(), text.range.start.line))
        .collect();
    assert_eq!(
        order,
        vec![
            (PathBuf::from("/w/a.py"), 1),
            (PathBuf::from("/w/b.py"), 1),
            (PathBuf::from("/w/b.py"), 3)
        ]
    );
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn unreadable_document_keeps_the_protocol_offsets() {
    let edit = json!({"changes": {"file:///w/a.py": [{"range": range(4, 2, 5), "newText": "y"}]}});
    let change = workspace_change(&edit, PositionUnit::Utf16, &|_: &Path| None).unwrap();
    assert_eq!(texts(&change.changes)[0].range.start, at(5, 3));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn document_changes_keep_the_order_of_the_server() {
    let edit = json!({"documentChanges": [
        {"kind": "rename", "oldUri": "file:///w/old.ts", "newUri": "file:///w/new.ts", "options": {"overwrite": true}},
        {"textDocument": {"uri": "file:///w/new.ts", "version": 1}, "edits": [{"range": range(0, 9, 12), "newText": "total"}]},
        {"kind": "create", "uri": "file:///w/created.ts", "options": {"ignoreIfExists": true}},
        {"textDocument": {"uri": "file:///w/created.ts", "version": null}, "edits": [{"range": range(0, 0, 0), "newText": "x"}]},
        {"kind": "delete", "uri": "file:///w/gone", "options": {"recursive": true}}
    ]});
    let change = workspace_change(&edit, PositionUnit::Utf16, &only("/w/old.ts", WIDE)).unwrap();
    assert_eq!(change.changes.len(), 5);
    assert_eq!(
        change.changes[0],
        DocumentChange::Resource(ResourceChange::Rename {
            from: PathBuf::from("/w/old.ts"),
            to: PathBuf::from("/w/new.ts"),
            overwrite: true,
            ignore_if_exists: false,
        })
    );
    let edits = texts(&change.changes);
    assert_eq!(
        edits[0].range.start,
        at(1, 9),
        "edits of a renamed file read the old content"
    );
    assert_eq!(
        edits[1].range.start,
        at(1, 1),
        "edits of a created file start from empty content"
    );
    assert_eq!(
        change.changes[2],
        DocumentChange::Resource(ResourceChange::Create {
            path: PathBuf::from("/w/created.ts"),
            overwrite: false,
            ignore_if_exists: true,
        })
    );
    assert_eq!(
        change.changes[4],
        DocumentChange::Resource(ResourceChange::Delete {
            path: PathBuf::from("/w/gone"),
            recursive: true,
            ignore_if_not_exists: false,
        })
    );
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn text_document_edits_with_annotations_are_converted() {
    let edit = json!({
        "documentChanges": [{"textDocument": {"uri": "file:///w/a.py", "version": 3}, "edits": [
            {"range": range(0, 0, 3), "newText": "new", "annotationId": "rename"}
        ]}],
        "changeAnnotations": {"rename": {"label": "Rename"}}
    });
    let change =
        workspace_change(&edit, PositionUnit::Utf16, &only("/w/a.py", "old = 1\n")).unwrap();
    let edits = texts(&change.changes);
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].new_text, "new");
    assert_eq!(edits[0].range.end, at(1, 4));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn offsets_past_the_line_or_the_text_are_clamped() {
    let text = "ab\r\ncd\n";
    let past_line =
        json!({"changes": {"file:///w/a.py": [{"range": range(0, 1, 40), "newText": ""}]}});
    let change = workspace_change(&past_line, PositionUnit::Utf16, &only("/w/a.py", text)).unwrap();
    assert_eq!(
        texts(&change.changes)[0].range.end,
        at(1, 3),
        "the carriage return is not a character of the line"
    );
    let past_text =
        json!({"changes": {"file:///w/a.py": [{"range": range(0, 0, 0), "newText": ""}]}});
    let mut value = past_text;
    value["changes"]["file:///w/a.py"][0]["range"]["end"] = json!({"line": 9, "character": 0});
    let change = workspace_change(&value, PositionUnit::Utf16, &only("/w/a.py", text)).unwrap();
    assert_eq!(texts(&change.changes)[0].range.end, at(3, 1));
}

#[test]
fn null_answer_is_an_empty_change() {
    let change = workspace_change(&Value::Null, PositionUnit::Utf16, &|_: &Path| None).unwrap();
    assert!(change.changes.is_empty());
}

#[test]
fn edit_for_a_non_file_document_fails() {
    let edit =
        json!({"changes": {"untitled:Untitled-1": [{"range": range(0, 0, 1), "newText": "x"}]}});
    let error = workspace_change(&edit, PositionUnit::Utf16, &|_: &Path| None).unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::Failed);
    assert!(error.message.contains("not a local file"));
}

#[test]
fn malformed_answer_fails() {
    let error = workspace_change(&json!({"changes": 3}), PositionUnit::Utf16, &|_: &Path| {
        None
    })
    .unwrap_err();
    assert_eq!(error.kind, SemanticErrorKind::Failed);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn text_with_carriage_returns_and_no_line_feed_is_split_at_the_carriage_returns() {
    let edit =
        json!({"changes": {"file:///work/a.txt": [{"range": range(2, 1, 2), "newText": "x"}]}});
    let text = only("/work/a.txt", "one\rtwo\rthree\r");
    let change = workspace_change(&edit, PositionUnit::Utf16, &text).unwrap();
    let first = texts(&change.changes)[0];
    assert_eq!(first.range.start, at(3, 2));
    assert_eq!(first.range.end, at(3, 3));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn line_feeds_keep_priority_over_carriage_returns() {
    let edit =
        json!({"changes": {"file:///work/a.txt": [{"range": range(1, 0, 1), "newText": "x"}]}});
    let text = only("/work/a.txt", "a\rb\nc\r\nd\n");
    let change = workspace_change(&edit, PositionUnit::Utf16, &text).unwrap();
    assert_eq!(texts(&change.changes)[0].range.start, at(2, 1));
}
