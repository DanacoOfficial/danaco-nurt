// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rendering of reports in the four formats, the levels of detail and the
//! response envelope.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use nurt_contracts::envelope::{OutputFormat, Verbosity};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::report::{RecordsPage, Report, ReportStatus, Timing};
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
