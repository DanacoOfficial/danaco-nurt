// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-report/tests/render_formats.rs
// Responsibility:   Rendering of reports in the four formats, the levels of detail and the response envelope.
// Dependencies:     common, nurt_contracts, serde_json, unicode_width
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Rendering of reports in the four formats, the levels of detail and the
//! response envelope.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use nurt_contracts::envelope::{OutputFormat, Verbosity};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::{Change, ChangeAction, Hunk};
use nurt_contracts::report::{Notable, NotableEntry, RecordsPage, Report, ReportStatus, Timing};
use nurt_contracts::value::{Record, RecordKind, Size, Value};
use nurt_report::{RenderOptions, render, respond};
use serde_json::json;
use unicode_width::UnicodeWidthStr;

use common::{
    UNDO_WORDS, assert_valid, full_error_report, golden_report, golden_reports, manifest_dir,
    options, read_text, response_validator,
};

fn fixture(name: &str) -> String {
    let path = manifest_dir().join(format!("tests/fixtures/render/{name}.txt"));
    read_text(&path).trim_end_matches('\n').to_owned()
}

fn text_of(report: &Report) -> String {
    render(report, &RenderOptions::default())
}

#[test]
fn text_of_a_replacement_follows_the_specified_layout() {
    assert_eq!(text_of(&golden_report("ok-replace")), fixture("ok-replace"));
}

#[test]
fn text_of_a_bulk_operation_follows_the_specified_layout() {
    assert_eq!(
        text_of(&golden_report("ok-bulk-remove")),
        fixture("ok-bulk-remove")
    );
}

#[test]
fn text_of_a_program_run_follows_the_specified_layout() {
    assert_eq!(
        text_of(&golden_report("ok-run-irreversible")),
        fixture("ok-run-irreversible")
    );
}

#[test]
fn text_of_a_background_job_follows_the_specified_layout() {
    assert_eq!(
        text_of(&golden_report("running-job")),
        fixture("running-job")
    );
}

#[test]
fn text_of_a_preview_follows_the_specified_layout() {
    assert_eq!(
        text_of(&golden_report("planned-replace")),
        fixture("planned-replace")
    );
}

#[test]
fn text_of_a_read_follows_the_specified_layout() {
    assert_eq!(text_of(&golden_report("ok-read")), fixture("ok-read"));
}

#[test]
fn text_of_a_failed_check_follows_the_specified_layout() {
    assert_eq!(
        text_of(&golden_report("error-assert")),
        fixture("error-assert")
    );
}

#[test]
fn text_of_a_corrected_program_follows_the_specified_layout() {
    assert_eq!(
        text_of(&golden_report("ok-corrected")),
        fixture("ok-corrected")
    );
}

#[test]
fn text_of_an_error_lists_every_field_and_the_cause() {
    let expected = [
        "error E_ASSERT  stage 3  at 2:5  message of the error",
        "  location /srv/app/data.json:4:9",
        "  found remvoe",
        "  problem not_merged",
        "  help remove",
        "  expected 12",
        "  actual {\"files\":14}",
        "  fix remove \"/srv/app/a.log\"",
        "  sample (1) /srv/app/a.go",
        "  candidate 1 c1  remove \"/srv/app/a.log\"  (closest command)",
        "  next find \"/srv/app\"",
        "  effect applied",
        "  detail path /srv/app",
        "  cause E_IO  read failed",
        "    detail os_error 13",
    ]
    .join("\n");
    assert_eq!(text_of(&full_error_report(ErrorCode::Assert)), expected);
}

fn replacement_with_timing() -> Report {
    let mut report = golden_report("ok-replace");
    report.timing = Some(Timing {
        parse_ms: Some(1),
        read_ms: Some(14),
        ..Timing::default()
    });
    report
}

fn with_verbosity(verbosity: Verbosity) -> RenderOptions {
    RenderOptions {
        verbosity,
        ..RenderOptions::default()
    }
}

#[test]
fn quiet_level_keeps_summary_and_notable_effects_only() {
    let text = render(
        &replacement_with_timing(),
        &with_verbosity(Verbosity::Quiet),
    );
    assert!(text.contains("targets 3 files"));
    assert!(text.contains("many_per_file"));
    assert!(!text.contains("repl  "));
    assert!(!text.contains("diff ("));
    assert!(!text.contains("timing"));
}

#[test]
fn normal_level_shows_changes_and_differences_without_timing() {
    let text = render(
        &replacement_with_timing(),
        &with_verbosity(Verbosity::Normal),
    );
    assert!(text.contains("3 repl"));
    assert!(text.contains("diff (3 hunks)"));
    assert!(!text.contains("timing"));
}

#[test]
fn detailed_level_adds_the_timing_of_the_steps() {
    let text = render(
        &replacement_with_timing(),
        &with_verbosity(Verbosity::Detailed),
    );
    assert!(text.contains("3 repl"));
    assert!(text.contains("diff (3 hunks)"));
    assert!(text.ends_with("timing parse 1ms \u{b7} read 14ms"));
}

fn removal_of(count: usize) -> Report {
    let mut report = Report::new(ReportStatus::Ok, "remove");
    for index in 0..count {
        report.changes.push(Change::new(
            format!("/srv/app/f{index}.tmp").as_str(),
            ChangeAction::Delete,
        ));
    }
    report
}

#[test]
fn changes_beyond_the_page_limit_are_counted_not_listed() {
    let text = text_of(&removal_of(120));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1 + 50 + 1);
    assert!(lines[1].starts_with("  /srv/app/f0.tmp "));
    assert!(lines[1].ends_with("  delete"));
    assert_eq!(lines[51], "  \u{2026} 70 more changes");
}

#[test]
fn the_limit_option_sets_the_page_size() {
    let options = RenderOptions {
        limit: 3,
        ..RenderOptions::default()
    };
    let text = render(&removal_of(5), &options);
    assert_eq!(text.lines().count(), 1 + 3 + 1);
    assert!(text.ends_with("  \u{2026} 2 more changes"));
}

#[test]
fn compact_and_markdown_count_the_changes_beyond_the_page_limit() {
    let report = removal_of(120);
    let compact = render(&report, &options(OutputFormat::Compact));
    assert!(
        compact
            .lines()
            .any(|line| line == "omitted changes=70 hunks=0 rows=0"),
        "{compact}"
    );
    assert_eq!(
        compact
            .lines()
            .filter(|line| line.starts_with("change delete "))
            .count(),
        50
    );
    let markdown = render(&report, &options(OutputFormat::Markdown));
    assert!(markdown.ends_with("70 changes not shown"), "{markdown}");
}

#[test]
fn a_file_name_with_a_line_break_stays_on_one_line_in_text() {
    let mut report = Report::new(ReportStatus::Ok, "remove");
    report
        .changes
        .push(Change::new("/srv/app/a\nb.tmp", ChangeAction::Delete));
    report.notable.push(NotableEntry {
        path: Some("/srv/app/a\nb.tmp".to_owned()),
        ..NotableEntry::new(Notable::SystemPath)
    });
    let text = text_of(&report);
    assert!(text.contains("\n  /srv/app/a\\nb.tmp  delete\n"), "{text}");
    assert!(text.contains("system_path  /srv/app/a\\nb.tmp"), "{text}");
    assert_eq!(text.lines().count(), 4, "{text}");
}

#[test]
fn quiet_level_does_not_mention_the_omitted_changes() {
    let quiet = RenderOptions {
        verbosity: Verbosity::Quiet,
        format: OutputFormat::Compact,
        ..RenderOptions::default()
    };
    let compact = render(&removal_of(120), &quiet);
    assert!(!compact.contains("omitted"), "{compact}");
    let text = render(&removal_of(120), &with_verbosity(Verbosity::Quiet));
    assert!(!text.contains("more changes"), "{text}");
}

#[test]
fn one_change_beyond_the_page_limit_is_counted_in_the_singular() {
    let text = text_of(&removal_of(51));
    assert!(text.ends_with("  \u{2026} 1 more change"), "{text}");
}

fn report_with_output(count: usize) -> Report {
    let lines: Vec<String> = (0..count).map(|index| format!("l{index}")).collect();
    let mut report = Report::new(ReportStatus::Ok, "run");
    report.exec = Some(
        Record::new(RecordKind::Exec)
            .with("stdout", Value::Text(lines.join("\n")))
            .with("exit_code", Value::Int(0)),
    );
    report
}

#[test]
fn long_output_is_cut_to_its_head_and_tail_except_at_the_detailed_level() {
    let report = report_with_output(30);
    let head: Vec<String> = (0..10).map(|index| format!("    l{index}")).collect();
    let tail: Vec<String> = (20..30).map(|index| format!("    l{index}")).collect();
    let cut = format!(
        "  stdout head 10 / tail 10\n{}\n    \u{2026}\n{}",
        head.join("\n"),
        tail.join("\n")
    );
    let normal = render(&report, &with_verbosity(Verbosity::Normal));
    assert!(normal.ends_with(&cut), "{normal}");
    assert!(!normal.contains("l10"));

    let detailed = render(&report, &with_verbosity(Verbosity::Detailed));
    assert!(
        detailed.contains("  stdout\n    l0\n    l1\n"),
        "{detailed}"
    );
    assert!(detailed.ends_with("    l28\n    l29"), "{detailed}");
}

#[test]
fn output_of_twenty_lines_is_not_cut() {
    let text = render(&report_with_output(20), &with_verbosity(Verbosity::Normal));
    assert!(text.contains("    l10\n"), "{text}");
    assert!(!text.contains('\u{2026}'), "{text}");
}

fn entry(name: &str, depth: i64, directory: bool, size: u64) -> Record {
    let mut record = Record::new(RecordKind::Entry)
        .with("name", Value::Text(name.to_owned()))
        .with("depth", Value::Int(depth))
        .with("size", Value::Size(Size::from_bytes(size)));
    if directory {
        record.set("type", Value::Enum("directory".to_owned()));
    }
    record
}

fn listing(rows: Vec<Record>) -> Report {
    let mut report = Report::new(ReportStatus::Ok, "tree");
    report.records = Some(RecordsPage {
        record_type: RecordKind::Entry,
        matched: Some(rows.len() as u64),
        header: None,
        rows,
    });
    report
}

#[test]
fn entries_with_a_depth_are_drawn_as_a_tree_in_text() {
    let report = listing(vec![
        entry("app", 0, true, 0),
        entry("src", 1, true, 0),
        entry("a.go", 2, false, 1127),
        entry("b.go", 2, false, 8192),
        entry("README", 1, false, 12),
    ]);
    let expected = [
        "tree  matched 5 \u{b7} shown 5",
        "app/",
        "|-- src/",
        "|   |-- a.go  1.1kib",
        "|   `-- b.go  8.0kib",
        "`-- README  12b",
    ]
    .join("\n");
    assert_eq!(text_of(&report), expected);
}

fn found(path: &str, depth: i64, size: u64) -> Record {
    let name = path.rsplit('/').next().unwrap_or(path);
    entry(name, depth, false, size).with("path", Value::Path(path.to_owned()))
}

#[test]
fn matches_are_drawn_under_the_directories_of_their_paths() {
    let report = listing(vec![
        found("/srv/app/src/a.go", 2, 1127),
        found("/srv/app/src/sub/b.go", 3, 8192),
    ]);
    let expected = [
        "tree  matched 2 \u{b7} shown 2",
        "`-- src/",
        "    |-- a.go  1.1kib",
        "    `-- sub/",
        "        `-- b.go  8.0kib",
    ]
    .join("\n");
    assert_eq!(text_of(&report), expected);
}

#[test]
fn matches_are_drawn_under_their_directories_in_markdown() {
    let report = listing(vec![found("/srv/app/src/a.go", 2, 1127)]);
    let text = render(&report, &options(OutputFormat::Markdown));
    assert!(
        text.ends_with("```\n`-- src/\n    `-- a.go  1.1kib\n```"),
        "{text}"
    );
}

#[test]
fn a_multi_line_value_stays_inside_its_table_cell_in_text() {
    let row = |name: &str, text: &str| {
        Record::new(RecordKind::Entry)
            .with("name", Value::Text(name.to_owned()))
            .with("text", Value::Text(text.to_owned()))
    };
    let report = listing(vec![row("a.txt", "one\ntwo\r\n"), row("b.txt", "x")]);
    let expected = [
        "tree  matched 2 \u{b7} shown 2",
        "name   text",
        "a.txt  one\\ntwo\\r\\n",
        "b.txt  x",
    ]
    .join("\n");
    assert_eq!(text_of(&report), expected);
}

#[test]
fn a_multi_line_value_stays_on_one_line_in_compact() {
    let report = listing(vec![
        Record::new(RecordKind::Entry).with("text", Value::Text("one\ntwo".to_owned())),
    ]);
    let text = render(&report, &options(OutputFormat::Compact));
    assert!(
        text.lines().any(|line| line == "entry text=\"one\\ntwo\""),
        "{text}"
    );
}

#[test]
fn a_carriage_return_in_a_hunk_line_is_shown_as_an_escape_in_text() {
    let mut report = Report::new(ReportStatus::Ok, "replace");
    report.diff.push(Hunk {
        path: "/srv/app/a.txt".into(),
        lines: vec!["-old\r".to_owned(), "+\tnew\r".to_owned()],
        ..Hunk::default()
    });
    let text = render(&report, &options(OutputFormat::Text));
    assert!(!text.contains('\r'), "{text:?}");
    assert!(text.contains("- old\\r"), "{text}");
    assert!(text.contains("+ \tnew\\r"), "{text}");
    let compact = render(&report, &options(OutputFormat::Compact));
    assert!(!compact.contains('\r'), "{compact:?}");
    assert!(
        compact.contains("diff /srv/app/a.txt:0 -old\\r"),
        "{compact}"
    );
}

#[test]
fn a_fence_in_a_hunk_does_not_close_the_markdown_block() {
    let mut report = Report::new(ReportStatus::Ok, "replace");
    report.diff.push(Hunk {
        path: "/srv/app/README.md".into(),
        lines: vec![
            "+```rust".to_owned(),
            "+fn main() {}".to_owned(),
            "+```".to_owned(),
        ],
        ..Hunk::default()
    });
    let text = render(&report, &options(OutputFormat::Markdown));
    assert!(
        text.contains("````diff\n+```rust\n+fn main() {}\n+```\n````"),
        "{text}"
    );
}

#[test]
fn a_fence_in_the_output_does_not_close_the_markdown_block() {
    let mut report = Report::new(ReportStatus::Ok, "run");
    report.exec = Some(
        Record::new(RecordKind::Exec).with("stdout", Value::Text("```\ncode\n```\n".to_owned())),
    );
    let text = render(&report, &options(OutputFormat::Markdown));
    assert!(
        text.contains("**stdout**\n````\n```\ncode\n```\n````"),
        "{text}"
    );
}

#[test]
fn directories_between_the_root_and_a_deep_match_are_drawn_in_every_format() {
    let report = listing(vec![found("/abs/a/b/c/d.txt", 3, 1)]);
    let expected = [
        "tree  matched 1 \u{b7} shown 1",
        "`-- b/",
        "    `-- c/",
        "        `-- d.txt  1b",
    ]
    .join("\n");
    assert_eq!(text_of(&report), expected);
    let markdown = render(&report, &options(OutputFormat::Markdown));
    assert!(
        markdown.ends_with("```\n`-- b/\n    `-- c/\n        `-- d.txt  1b\n```"),
        "{markdown}"
    );
}

#[test]
fn a_directory_with_a_line_break_in_its_path_is_drawn_on_one_line() {
    let report = listing(vec![found("/abs/a/we\nird/d.txt", 2, 1)]);
    let text = text_of(&report);
    assert!(text.contains("`-- we\\nird/\n"), "{text}");
    assert_eq!(text.lines().count(), 3, "{text}");
}

#[test]
fn a_multi_line_value_stays_inside_its_table_cell_in_markdown() {
    let report = listing(vec![
        Record::new(RecordKind::Content)
            .with("path", Value::Path("/abs/multi.txt".to_owned()))
            .with("text", Value::Text("one\ntwo\r\nthree|four".to_owned())),
    ]);
    let markdown = render(&report, &options(OutputFormat::Markdown));
    assert!(
        markdown.contains("| /abs/multi.txt | one\\ntwo\\r\\nthree\\|four |"),
        "{markdown}"
    );
    assert_eq!(
        markdown
            .lines()
            .filter(|line| line.starts_with('|'))
            .count(),
        3,
        "{markdown}"
    );
}

fn report_naming_a_file_with_a_line_break() -> Report {
    let path = "/abs/we\nird.txt";
    let mut report = Report::new(ReportStatus::Ok, "replace");
    report.changes.push(Change::new(path, ChangeAction::Modify));
    report.diff.push(Hunk {
        path: path.into(),
        new_start: 4,
        lines: vec!["+x".to_owned()],
        ..Hunk::default()
    });
    report.notable.push(NotableEntry {
        path: Some(path.to_owned()),
        detail: Some("sample: /abs/a\nb".to_owned()),
        ..NotableEntry::new(Notable::SystemPath)
    });
    report.result_file = Some(path.to_owned());
    report
}

#[test]
fn a_file_name_with_a_line_break_stays_on_one_line_in_markdown() {
    let text = render(
        &report_naming_a_file_with_a_line_break(),
        &options(OutputFormat::Markdown),
    );
    assert!(text.contains("| `/abs/we\\nird.txt` |"), "{text}");
    assert!(text.contains("`/abs/we\\nird.txt:4`\n```diff"), "{text}");
    assert!(
        text.contains("- alert system_path /abs/we\\nird.txt sample: /abs/a\\nb"),
        "{text}"
    );
    assert!(text.contains("result file `/abs/we\\nird.txt`"), "{text}");
    assert!(!text.contains("we\nird"), "{text}");
}

#[test]
fn a_file_name_with_a_line_break_stays_on_one_line_in_compact_and_text() {
    let report = report_naming_a_file_with_a_line_break();
    let compact = render(&report, &options(OutputFormat::Compact));
    assert!(compact.contains("diff /abs/we\\nird.txt:4 +x"), "{compact}");
    assert!(
        compact.contains("change modify \"/abs/we\\nird.txt\""),
        "{compact}"
    );
    assert!(!compact.contains("we\nird"), "{compact}");
    let text = text_of(&report);
    assert!(text.contains("result file /abs/we\\nird.txt"), "{text}");
    assert!(!text.contains("we\nird"), "{text}");
}

#[test]
fn control_characters_of_a_file_name_are_escaped_in_every_format() {
    let mut report = Report::new(ReportStatus::Ok, "remove");
    report
        .changes
        .push(Change::new("/abs/t\tb\u{7}.txt", ChangeAction::Delete));
    let text = text_of(&report);
    assert!(text.contains("  /abs/t\\tb\\u{7}.txt  delete"), "{text}");
    let markdown = render(&report, &options(OutputFormat::Markdown));
    assert!(
        markdown.contains("| `/abs/t\\tb\\u{7}.txt` |"),
        "{markdown}"
    );
    let compact = render(&report, &options(OutputFormat::Compact));
    assert!(
        compact.contains("change delete \"/abs/t\\tb\\u0007.txt\""),
        "{compact}"
    );
}

#[test]
fn a_path_that_is_not_valid_unicode_keeps_its_raw_form_in_json() {
    let raw = nurt_contracts::value::Bytes::new(b"/abs/bad\xffname.txt".to_vec());
    let mut change = Change::new("/abs/bad\u{fffd}name.txt", ChangeAction::Delete);
    change.raw_path = Some(raw.clone());
    let mut report = Report::new(ReportStatus::Ok, "remove");
    report.changes.push(change);
    report.notable.push(NotableEntry {
        path: Some("/abs/bad\u{fffd}name.txt".to_owned()),
        raw_path: Some(raw.clone()),
        ..NotableEntry::new(Notable::Portability)
    });
    let json: serde_json::Value =
        serde_json::from_str(&render(&report, &options(OutputFormat::Json))).unwrap();
    assert_eq!(json["changes"][0]["path"], "/abs/bad\u{fffd}name.txt");
    assert_eq!(json["changes"][0]["raw_path"], raw.to_base64());
    assert_eq!(json["notable"][0]["raw_path"], raw.to_base64());
    let valid = Change::new("/abs/a.txt", ChangeAction::Delete);
    let json = serde_json::to_value(&valid).unwrap();
    assert!(json.get("raw_path").is_none(), "{json}");
}

#[test]
fn entries_with_a_depth_stay_a_flat_list_in_json() {
    let report = listing(vec![entry("app", 0, true, 0), entry("src", 1, true, 0)]);
    let json = render(&report, &options(OutputFormat::Json));
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rows = value["records"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["fields"]["depth"], json!({"t": "int", "v": 1}));
}

#[test]
fn the_fields_option_keeps_the_named_fields_in_the_given_order() {
    let options = RenderOptions {
        fields: Some(vec!["name".to_owned(), "size".to_owned()]),
        ..RenderOptions::default()
    };
    let expected = [
        "find  matched 148 \u{b7} shown 2 \u{b7} next cursor c_91fe",
        "name  size",
        "a.go  1.1kib",
        "b.go  8.0kib",
    ]
    .join("\n");
    assert_eq!(render(&golden_report("ok-read"), &options), expected);
}

#[test]
fn the_max_chars_option_shortens_text_fields_and_marks_the_record() {
    let mut report = Report::new(ReportStatus::Ok, "run");
    let exec = Record::new(RecordKind::Exec)
        .with("stdout", Value::Text("0123456789abcdef".to_owned()))
        .with("exit_code", Value::Int(0));
    report.exec = Some(exec);
    let options = RenderOptions {
        max_chars: 10,
        ..RenderOptions::default()
    };
    let response = respond(&report, &options);
    let exec = response.report.exec.as_ref().unwrap();
    assert_eq!(
        exec.get("stdout"),
        Some(&Value::Text("0123456789".to_owned()))
    );
    assert_eq!(exec.get("truncated"), Some(&Value::Bool(true)));
    assert!(response.rendered.unwrap().text.contains("    0123456789"));
}

#[test]
fn compact_output_has_one_record_per_line_without_headings() {
    let expected = [
        "report kind=find status=ok",
        "entry path=/srv/app/src/a.go name=a.go type=file size=1.1kib modified=2026-11-21T09:12:00Z",
        "entry path=/srv/app/src/b.go name=b.go type=file size=8.0kib modified=2026-11-20T17:40:12Z",
        "next cursor=c_91fe",
    ]
    .join("\n");
    let text = render(&golden_report("ok-read"), &options(OutputFormat::Compact));
    assert_eq!(text, expected);
}

#[test]
fn compact_output_writes_a_change_on_one_line() {
    let text = render(
        &golden_report("ok-replace"),
        &options(OutputFormat::Compact),
    );
    let expected = "change modify /srv/app/src/a.go size=1127>1131 hash=1a2b3c4d>9f8e7d6c \
                    repl=3 added=3 removed=3";
    assert!(text.lines().any(|line| line == expected), "{text}");
    assert!(text.starts_with("report kind=replace status=ok op=18\n"));
}

#[test]
fn compact_output_writes_an_error_with_its_facts() {
    let text = render(
        &full_error_report(ErrorCode::Assert),
        &options(OutputFormat::Compact),
    );
    assert!(text.contains("error code=E_ASSERT stage=3 message=\"message of the error\""));
    assert!(text.contains("error.expected 12"));
    assert!(text.contains("cause code=E_IO message=\"read failed\""));
}

#[test]
fn markdown_output_writes_a_heading_and_a_table_for_a_read() {
    let expected = [
        "### find ok",
        "",
        "**find** \u{b7} matched 148 \u{b7} shown 2",
        "",
        "| path | name | type | size | modified |",
        "|---|---|---|---|---|",
        "| /srv/app/src/a.go | a.go | file | 1.1kib | 2026-11-21T09:12:00Z |",
        "| /srv/app/src/b.go | b.go | file | 8.0kib | 2026-11-20T17:40:12Z |",
        "",
        "next cursor `c_91fe`",
    ]
    .join("\n");
    let text = render(&golden_report("ok-read"), &options(OutputFormat::Markdown));
    assert_eq!(text, expected);
}

#[test]
fn markdown_output_fences_each_hunk_as_a_diff() {
    let text = render(
        &golden_report("planned-replace"),
        &options(OutputFormat::Markdown),
    );
    assert!(text.starts_with("### preview replace (nothing changed)\n"));
    assert!(text.contains("```diff\n-func oldName(x int) {\n+func newName(x int) {\n```"));
    assert!(text.contains("| `/srv/app/src/a.go` | 3 repl |  |  |"));
}

#[test]
fn json_output_is_the_report_with_a_fixed_order_of_fields() {
    let text = render(&golden_report("ok-replace"), &options(OutputFormat::Json));
    assert!(
        text.starts_with("{\"nurt\":\"1.0\",\"status\":\"ok\",\"kind\":\"replace\",\"op\":18,")
    );
    assert!(!text.contains('\n'));
}

#[test]
fn json_output_of_every_golden_report_equals_the_report_at_detailed_level() {
    for (name, value) in golden_reports() {
        let report: Report = serde_json::from_value(value.clone()).unwrap();
        let options = RenderOptions {
            format: OutputFormat::Json,
            verbosity: Verbosity::Detailed,
            ..RenderOptions::default()
        };
        let rendered: serde_json::Value = serde_json::from_str(&render(&report, &options)).unwrap();
        assert_eq!(rendered, value, "{name}");
    }
}

#[test]
fn every_golden_report_renders_in_all_formats() {
    for (name, value) in golden_reports() {
        let report: Report = serde_json::from_value(value).unwrap();
        for format in OutputFormat::ALL {
            for verbosity in Verbosity::ALL {
                let options = RenderOptions {
                    format: *format,
                    verbosity: *verbosity,
                    ..RenderOptions::default()
                };
                assert!(!render(&report, &options).is_empty(), "{name}: {format:?}");
            }
        }
    }
}

fn words_of(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphabetic())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

fn leaked_words(output: &str, input: &str, words: &[&str]) -> Vec<String> {
    let output = words_of(output);
    let input = words_of(input);
    words
        .iter()
        .filter(|word| {
            output.iter().any(|found| found == **word) && !input.iter().any(|found| found == **word)
        })
        .map(|word| (*word).to_owned())
        .collect()
}

#[test]
fn rendered_results_never_add_words_about_undoing() {
    for (name, value) in golden_reports() {
        let input = value.to_string();
        let report: Report = serde_json::from_value(value).unwrap();
        for format in OutputFormat::ALL {
            let output = render(&report, &options(*format));
            let leaks = leaked_words(&output, &input, &UNDO_WORDS);
            assert!(leaks.is_empty(), "{name}: {format:?}: {leaks:?}");
        }
    }
}

#[test]
fn rendered_text_of_ordinary_effects_has_no_undo_vocabulary() {
    let mut replacement = replacement_with_timing();
    replacement.notable.clear();
    let mut reports = vec![removal_of(3), replacement];
    reports.push(full_error_report(ErrorCode::Stale));
    reports.push(full_error_report(ErrorCode::RolledBack));
    for report in reports {
        for format in OutputFormat::ALL {
            let output = words_of(&render(&report, &options(*format)));
            for word in UNDO_WORDS {
                assert!(
                    !output.iter().any(|found| found == word),
                    "{format:?}: {word}"
                );
            }
        }
    }
}

#[test]
fn rendered_results_do_not_name_the_product() {
    for (name, mut value) in golden_reports() {
        value.as_object_mut().unwrap().remove("nurt");
        let input = value.to_string();
        let report: Report = serde_json::from_value(golden_value_with_version(value)).unwrap();
        for format in [
            OutputFormat::Text,
            OutputFormat::Markdown,
            OutputFormat::Compact,
        ] {
            let output = render(&report, &options(format));
            let leaks = leaked_words(&output, &input, &["nurt"]);
            assert!(leaks.is_empty(), "{name}: {format:?}");
        }
    }
}

fn golden_value_with_version(mut value: serde_json::Value) -> serde_json::Value {
    value["nurt"] = json!("1.0");
    value
}

#[test]
fn the_response_carries_the_report_the_text_and_the_cursor() {
    let response = respond(&golden_report("ok-replace"), &RenderOptions::default());
    assert_eq!(response.nurt, "1.0");
    assert_eq!(response.report.next_cursor.as_deref(), Some("c_5d2e"));
    assert!(response.has_more());
    let rendered = response.rendered.unwrap();
    assert_eq!(rendered.format, OutputFormat::Text);
    assert_eq!(rendered.text, fixture("ok-replace"));
}

#[test]
fn the_response_to_the_json_format_has_no_rendered_text() {
    let response = respond(&golden_report("ok-read"), &options(OutputFormat::Json));
    assert!(response.rendered.is_none());
    assert_eq!(response.report.kind, "find");
}

#[test]
fn every_response_to_a_golden_report_is_valid_against_the_envelope_schema() {
    let validator = response_validator();
    for (name, value) in golden_reports() {
        let report: Report = serde_json::from_value(value).unwrap();
        for format in OutputFormat::ALL {
            let response = respond(&report, &options(*format));
            assert_eq!(response.validate(), Ok(()), "{name}");
            let value = serde_json::to_value(&response).unwrap();
            assert_valid(&validator, &value, &format!("{name}: {format:?}"));
        }
    }
}

#[test]
fn the_response_text_has_no_colors_even_when_the_options_ask_for_them() {
    let options = RenderOptions {
        color: true,
        width: Some(10),
        ..RenderOptions::default()
    };
    let response = respond(&golden_report("ok-replace"), &options);
    let text = response.rendered.unwrap().text;
    assert!(!text.contains('\u{1b}'));
    assert_eq!(text, fixture("ok-replace"));
}

#[test]
fn colors_are_written_only_when_asked() {
    let report = golden_report("ok-replace");
    let plain = render(&report, &RenderOptions::default());
    assert!(!plain.contains('\u{1b}'));
    let options = RenderOptions {
        color: true,
        ..RenderOptions::default()
    };
    let colored = render(&report, &options);
    assert!(colored.starts_with("\u{1b}[32mop 18  replace  ok\u{1b}[0m\n"));
    assert!(colored.contains("\u{1b}[2m  - encoding_preserved"));
}

#[test]
fn the_width_option_shortens_lines_of_the_text_format() {
    let options = RenderOptions {
        width: Some(40),
        ..RenderOptions::default()
    };
    let text = render(&golden_report("ok-replace"), &options);
    assert!(text.lines().all(|line| line.width() <= 40), "{text}");
    assert!(text.contains('\u{2026}'));
}

#[test]
fn a_failed_check_after_the_effect_shows_the_effect_and_the_error() {
    let mut report = golden_report("ok-replace");
    report.error = Some(
        nurt_contracts::error::Error::new(ErrorCode::Assert, "assert failed")
            .with_stage(nurt_contracts::error::Stage::After)
            .with_effect_applied(true),
    );
    let text = text_of(&report);
    assert!(text.starts_with("op 18  replace  ok\n"));
    assert!(text.contains("\nerror E_ASSERT  stage 5a  assert failed\n  effect applied"));
}

fn report_with_result_file() -> Report {
    let mut report = golden_report("ok-replace");
    report.result_file = Some("/srv/results/work 1.txt".to_owned());
    report.timing = Some(Timing {
        parse_ms: Some(1),
        ..Timing::default()
    });
    report
}

#[test]
fn text_names_the_result_file_before_the_timing() {
    let text = render(
        &report_with_result_file(),
        &with_verbosity(Verbosity::Detailed),
    );
    assert!(text.ends_with("result file /srv/results/work 1.txt\ntiming parse 1ms"));
}

#[test]
fn quiet_level_keeps_the_result_file() {
    let text = render(
        &report_with_result_file(),
        &with_verbosity(Verbosity::Quiet),
    );
    assert!(text.ends_with("result file /srv/results/work 1.txt"));
}

#[test]
fn markdown_and_compact_name_the_result_file() {
    let report = report_with_result_file();
    let markdown = RenderOptions {
        format: OutputFormat::Markdown,
        ..RenderOptions::default()
    };
    let compact = RenderOptions {
        format: OutputFormat::Compact,
        ..RenderOptions::default()
    };
    assert!(render(&report, &markdown).contains("result file `/srv/results/work 1.txt`"));
    assert!(
        render(&report, &compact)
            .lines()
            .any(|line| line == "result file=\"/srv/results/work 1.txt\"")
    );
}

#[test]
fn json_carries_the_result_file_and_validates() {
    let report = report_with_result_file();
    let options = RenderOptions {
        format: OutputFormat::Json,
        ..RenderOptions::default()
    };
    let value: serde_json::Value = serde_json::from_str(&render(&report, &options)).unwrap();
    assert_eq!(value["result_file"], "/srv/results/work 1.txt");
    assert!(common::report_validator().is_valid(&value));
}

fn read_report(kind: &str, record_type: RecordKind, rows: Vec<Record>) -> Report {
    let mut report = Report::new(ReportStatus::Ok, kind);
    report.records = Some(RecordsPage {
        record_type,
        matched: Some(rows.len() as u64),
        header: None,
        rows,
    });
    report
}

/// Names in the header row of the table of a text rendering.
fn table_columns(text: &str) -> Vec<String> {
    text.lines()
        .nth(1)
        .unwrap()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

fn operation_row() -> Record {
    Record::new(RecordKind::Operation)
        .with("id", Value::Int(18))
        .with("time", Value::Text("2026-11-21T09:12:00Z".to_owned()))
        .with("kind", Value::Text("replace".to_owned()))
        .with("action", Value::Text("apply".to_owned()))
        .with("summary", Value::Text("3 files".to_owned()))
        .with("state", Value::Enum("available".to_owned()))
        .with("files", Value::Int(3))
        .with("bytes", Value::Size(Size::from_bytes(2048)))
        .with("bulk", Value::Bool(false))
        .with("session", Value::Text("s1".to_owned()))
        .with("ast_hash", Value::Text("sha256:aa".to_owned()))
        .with("manifest_sha256", Value::Text("sha256:bb".to_owned()))
        .with("pinned", Value::Bool(false))
}

#[test]
fn text_of_the_history_shows_the_essential_columns_only() {
    let report = read_report("history", RecordKind::Operation, vec![operation_row()]);
    assert_eq!(
        table_columns(&text_of(&report)),
        ["id", "time", "kind", "state", "files", "bytes", "summary"]
    );
}

#[test]
fn the_history_shows_every_column_in_detailed_text_and_in_json() {
    let report = read_report("history", RecordKind::Operation, vec![operation_row()]);
    let detailed = RenderOptions {
        verbosity: Verbosity::Detailed,
        ..RenderOptions::default()
    };
    assert!(table_columns(&render(&report, &detailed)).contains(&"manifest_sha256".to_owned()));
    let json = RenderOptions {
        format: OutputFormat::Json,
        ..RenderOptions::default()
    };
    let value: serde_json::Value = serde_json::from_str(&render(&report, &json)).unwrap();
    assert!(!value["records"]["rows"][0]["fields"]["manifest_sha256"].is_null());
}

#[test]
fn named_fields_override_the_default_columns() {
    let report = read_report("history", RecordKind::Operation, vec![operation_row()]);
    let options = RenderOptions {
        fields: Some(vec!["id".to_owned(), "pinned".to_owned()]),
        ..RenderOptions::default()
    };
    assert_eq!(table_columns(&render(&report, &options)), ["id", "pinned"]);
}

#[test]
fn text_of_a_search_shows_the_position_and_the_matched_text() {
    let row = Record::new(RecordKind::Match)
        .with("path", Value::Path("/srv/app/a.go".to_owned()))
        .with("no", Value::Int(12))
        .with("column", Value::Int(5))
        .with("start", Value::Int(240))
        .with("end", Value::Int(247))
        .with("text", Value::Text("oldName".to_owned()))
        .with("line", Value::Text("func oldName() {".to_owned()))
        .with("etag", Value::Text("sha256:cc".to_owned()));
    let report = read_report("search", RecordKind::Match, vec![row]);
    assert_eq!(
        table_columns(&text_of(&report)),
        ["path", "no", "column", "text"]
    );
}

fn response_row() -> Record {
    Record::new(RecordKind::Response)
        .with("url", Value::Text("https://example.com/a".to_owned()))
        .with("status", Value::Int(200))
        .with("body", Value::Text("BODYTEXT of the page".to_owned()))
        .with("bytes", Value::Size(Size::from_bytes(20)))
        .with("elapsed", Value::Text("35ms".to_owned()))
        .with("content_type", Value::Text("text/html".to_owned()))
        .with("final_url", Value::Text("https://example.com/a".to_owned()))
}

#[test]
fn text_of_a_response_does_not_print_the_body() {
    let report = read_report("http.request", RecordKind::Response, vec![response_row()]);
    let text = text_of(&report);
    assert_eq!(
        table_columns(&text),
        ["url", "status", "bytes", "content_type", "elapsed"]
    );
    assert!(!text.contains("BODYTEXT"), "{text}");
}

#[test]
fn the_body_of_a_response_is_printed_on_request() {
    let report = read_report("http.request", RecordKind::Response, vec![response_row()]);
    let options = RenderOptions {
        fields: Some(vec!["status".to_owned(), "body".to_owned()]),
        ..RenderOptions::default()
    };
    assert!(render(&report, &options).contains("BODYTEXT of the page"));
    let detailed = RenderOptions {
        verbosity: Verbosity::Detailed,
        ..RenderOptions::default()
    };
    assert!(render(&report, &detailed).contains("BODYTEXT of the page"));
}

#[test]
fn notes_without_a_detail_show_the_message_of_their_code() {
    use nurt_contracts::report::{Notable, NotableEntry};

    let mut report = Report::new(ReportStatus::Ok, "remove");
    report.notable = vec![
        NotableEntry::new(Notable::NothingMatched),
        NotableEntry::new(Notable::DetectionScope),
    ];
    let text = text_of(&report);
    assert!(
        text.contains("nothing_matched  the program had no targets, so nothing was changed"),
        "{text}"
    );
    assert!(
        text.contains(
            "detection_scope  changes made by the program outside the watched directories are not detected"
        ),
        "{text}"
    );
}

#[test]
fn a_note_with_a_detail_keeps_its_own_text() {
    use nurt_contracts::report::{Notable, NotableEntry};

    let mut entry = NotableEntry::new(Notable::DetectionScope);
    entry.detail = Some("changes outside /srv/app are not detected".to_owned());
    let mut report = Report::new(ReportStatus::Ok, "run");
    report.notable = vec![entry];
    let text = text_of(&report);
    assert!(text.contains("detection_scope  changes outside /srv/app are not detected"));
    assert!(!text.contains("outside the watched directories"));
}

fn continued(older: bool) -> Report {
    let mut report = golden_report("ok-read");
    report.continuation = Some(nurt_contracts::continuation::Continuation {
        id: "c3".to_owned(),
        unit: nurt_contracts::continuation::ContinuationUnit::Records,
        shown: 2,
        remaining: 146,
        cut_lines: 0,
        older,
    });
    report
}

#[test]
fn the_text_of_a_shortened_read_ends_with_the_hint_of_the_latest_continuation() {
    let text = render(&continued(false), &RenderOptions::default());
    let first = text.lines().next().unwrap();
    assert!(first.ends_with("shown 2 \u{b7} more: next"), "{first}");
}

#[test]
fn the_hint_names_the_identifier_when_a_newer_continuation_exists() {
    let text = render(&continued(true), &RenderOptions::default());
    assert!(
        text.lines().next().unwrap().ends_with("more: next c3"),
        "{text}"
    );
}

#[test]
fn a_response_without_records_ends_with_the_hint_on_its_own_line() {
    let mut report = Report::new(ReportStatus::Ok, "run");
    report.continuation = continued(false).continuation;
    let text = render(&report, &RenderOptions::default());
    assert_eq!(text.lines().last(), Some("more: next"), "{text}");
}

#[test]
fn compact_and_markdown_write_the_continuation_and_the_hint() {
    let compact = render(&continued(false), &options(OutputFormat::Compact));
    assert!(
        compact.contains("continuation id=c3 unit=records shown=2 remaining=146 cut_lines=0"),
        "{compact}"
    );
    assert_eq!(compact.lines().last(), Some("more: next"));
    let markdown = render(&continued(true), &options(OutputFormat::Markdown));
    assert!(markdown.ends_with("more: next c3"), "{markdown}");
}

#[test]
fn the_continuation_is_a_field_of_the_json_report_without_the_internal_flag() {
    let json = render(&continued(true), &options(OutputFormat::Json));
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        value["continuation"],
        json!({"id": "c3", "unit": "records", "shown": 2, "remaining": 146, "cut_lines": 0})
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
