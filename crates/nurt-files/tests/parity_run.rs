// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Runs the parity scenarios on the stages of the crate and tests the runner.
//!
//! Every scenario of `tests/parity/files` either agrees with the stages, is
//! listed in `divergences.toml` with the observed difference, or is listed
//! in `pending.toml` with the reason it does not run. A scenario pending
//! because its command is not merged runs as soon as the stage of the
//! command is registered.

// Test code unwraps results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_parity;
mod support_parity_run;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::stage::ArgValue;
use nurt_contracts::value::{Range, Value};
use nurt_testkit::ctx::TestContext;
use serde_json::{Value as Json, json};
use support_parity::{AfterEntry, FsEntry, Model};
use support_parity_run::bind::{BindCx, BindFailure, Catalog, UNKNOWN_ETAG, bind, split_top};
use support_parity_run::program::{Call, Lit, parse_script};
use support_parity_run::report::{Comparison, Item, Report, check_field, resolve, same};
use support_parity_run::tree::{self, Node};
use support_parity_run::{NOT_MERGED, Outcome, Runner};

/// Longest difference quoted in a suggested `divergences.toml` entry.
const DETAIL_LIMIT: usize = 600;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity")
}

fn catalog_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog")
}

fn quoted(text: &str) -> String {
    toml::Value::String(text.to_owned()).to_string()
}

fn first_command(program: &str) -> String {
    parse_script(program)
        .ok()
        .and_then(|pipelines| {
            pipelines
                .first()
                .and_then(|p| p.last())
                .map(|c| c.command.clone())
        })
        .unwrap_or_default()
}

#[test]
fn scenarios_agree_with_the_stages_or_are_listed() {
    let model = Model::load(&data_dir()).unwrap();
    let runner = Runner::new(&catalog_dir()).unwrap();
    let pending: BTreeMap<&str, &str> = model
        .pending_scenarios
        .iter()
        .map(|entry| (entry.name.as_str(), entry.reason.as_str()))
        .collect();
    let diverging: BTreeSet<&str> = model.divergences.iter().map(|d| d.name.as_str()).collect();
    let mut problems = Vec::new();
    let (mut agree, mut diverge, mut skipped) = (0_usize, 0_usize, 0_usize);
    for scenario in model.scenarios() {
        let name = scenario.name.as_str();
        let listed = pending.get(name).copied();
        if let Some(reason) = listed
            && !reason.starts_with(NOT_MERGED)
        {
            skipped += 1;
            eprintln!("skipped: {name}: {reason}");
            continue;
        }
        match runner.run(&model, scenario) {
            Outcome::Pending(reason) => {
                skipped += 1;
                if listed.is_none() {
                    problems.push(format!(
                        "add to pending.toml:\n[[scenario]]\nname = {}\nreason = {}\n",
                        quoted(name),
                        quoted(&reason)
                    ));
                } else {
                    eprintln!("skipped: {name}: {reason}");
                }
            }
            Outcome::Agree => {
                agree += 1;
                if diverging.contains(name) {
                    problems.push(format!(
                        "scenario {name:?} agrees; remove it from divergences.toml"
                    ));
                }
                if listed.is_some() {
                    eprintln!(
                        "scenario {name:?} runs and agrees; its pending entry can be removed"
                    );
                }
            }
            Outcome::Diverge(differences) => {
                diverge += 1;
                if !diverging.contains(name) {
                    let detail: String =
                        differences.join("; ").chars().take(DETAIL_LIMIT).collect();
                    problems.push(format!(
                        "add to divergences.toml:\n[[divergence]]\nname = {}\ncommand = {}\ndetail = {}\n",
                        quoted(name),
                        quoted(&first_command(&scenario.nurt_program)),
                        quoted(&detail)
                    ));
                }
            }
        }
    }
    eprintln!("parity scenarios: {agree} agree, {diverge} diverge, {skipped} pending");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn parses_values_pipelines_and_scripts() {
    let script = parse_script(
        "read \"{root}/a.ts\" lines: [1..2, 3] line_numbers: true \
         |> replace \"a\\n\\u{142}\" /x\\/y/i; undo 1 known: { \"{root}/a\": \"<sha>\", depth: -2 }",
    )
    .unwrap();
    assert_eq!(script.len(), 2);
    let read = &script[0][0];
    assert_eq!(read.command, "read");
    assert_eq!(read.positional, [Lit::Str("{root}/a.ts".into())]);
    assert_eq!(
        read.named,
        [
            (
                "lines".into(),
                Lit::List(vec![Lit::Range(Some(1), Some(2)), Lit::Int(3)])
            ),
            ("line_numbers".into(), Lit::Bool(true)),
        ]
    );
    let replace = &script[0][1];
    assert_eq!(
        replace.positional,
        [
            Lit::Str("a\n\u{142}".into()),
            Lit::Regex("x/y".into(), "i".into())
        ]
    );
    let undo = &script[1][0];
    assert_eq!(undo.positional, [Lit::Int(1)]);
    assert_eq!(
        undo.named,
        [(
            "known".into(),
            Lit::Map(vec![
                ("{root}/a".into(), Lit::Str("<sha>".into())),
                ("depth".into(), Lit::Int(-2)),
            ])
        )]
    );
}

#[test]
fn keeps_unknown_tokens_as_raw_text() {
    let script =
        parse_script("find \"{root}\" |> filter modified < 2021-01-01 |> select path size")
            .unwrap();
    let filter = &script[0][1];
    assert_eq!(
        filter.positional,
        [
            Lit::Ident("modified".into()),
            Lit::Raw("<".into()),
            Lit::Raw("2021-01-01".into())
        ]
    );
    assert_eq!(script[0][2].positional.len(), 2);
    assert!(parse_script("read \"unterminated").is_err());
    assert!(parse_script("").is_err());
}

#[test]
fn splits_types_outside_brackets() {
    assert_eq!(
        split_top("text | regex | list<text | regex>", '|'),
        ["text", "regex", "list<text | regex>"]
    );
    assert_eq!(split_top("enum(a|b)", '|'), ["enum(a|b)"]);
}

fn catalog() -> Catalog {
    Catalog::load(&catalog_dir()).unwrap()
}

fn bind_one(program: &str, source: bool) -> Result<support_parity_run::bind::Bound, BindFailure> {
    let catalog = catalog();
    let call: Call = parse_script(program).unwrap().remove(0).remove(0);
    let spec = catalog.command(&call.command).unwrap().clone();
    let etag_of = |path: &str| format!("etag of {path}");
    let cx = BindCx {
        root: "/work",
        etag_of: &etag_of,
    };
    bind(&call, &spec, source, &cx)
}

#[test]
fn binds_positional_and_named_values_to_catalog_types() {
    let bound = bind_one(
        "read \"{root}/a.txt\" \"{root}/b.txt\" lines: 2..3 view: header",
        true,
    )
    .unwrap();
    assert_eq!(
        bound.args.get("path"),
        Some(&ArgValue::Value(Value::List(vec![
            Value::Path("/work/a.txt".into()),
            Value::Path("/work/b.txt".into())
        ])))
    );
    assert_eq!(
        bound.args.get("lines"),
        Some(&ArgValue::Value(Value::Range(
            Range::new(Some(2), Some(3)).unwrap()
        )))
    );
    assert_eq!(
        bound.args.value("view"),
        Some(&Value::Enum("header".into()))
    );
    let search = bind_one("search /a+/ \"{root}\" \"{root}/b\" preview: true", true).unwrap();
    assert!(matches!(
        search.args.value("pattern"),
        Some(Value::Regex(_))
    ));
    assert_eq!(
        search.args.value("path"),
        Some(&Value::List(vec![
            Value::Path("/work".into()),
            Value::Path("/work/b".into())
        ]))
    );
    assert_eq!(search.options.get("preview"), Some(&Json::Bool(true)));
}

#[test]
fn resolves_entity_tag_placeholders_from_the_path() {
    let bound = bind_one(
        "read \"{root}/a.txt\" unchanged_since: \"<etag>\" known: { \"{root}/b.txt\": \"<sha>\" }",
        true,
    )
    .unwrap();
    assert_eq!(
        bound.args.value("unchanged_since"),
        Some(&Value::Text("etag of /work/a.txt".into()))
    );
    let Some(Value::Map(known)) = bound.args.value("known") else {
        panic!("known is not a map");
    };
    assert_eq!(
        known.get("/work/b.txt"),
        Some(&Value::Text("etag of /work/b.txt".into()))
    );
    let unknown = bind_one(
        "read \"{root}/a.txt\" unchanged_since: \"<unknown etag>\"",
        true,
    )
    .unwrap();
    assert_eq!(
        unknown.args.value("unchanged_since"),
        Some(&Value::Text(UNKNOWN_ETAG.into()))
    );
}

#[test]
fn reports_values_the_engine_rejects_as_errors() {
    let missing = bind_one("list", true).unwrap_err();
    assert!(matches!(missing, BindFailure::Error(ref e) if e.code() == ErrorCode::MissingParam));
    assert!(bind_one("list", false).is_ok());
    let unknown = bind_one("find \"{root}\" sort: sideways", true).unwrap_err();
    assert!(matches!(unknown, BindFailure::Error(ref e) if e.code() == ErrorCode::UnknownValue));
}

#[test]
fn makes_parameters_outside_the_catalog_pending() {
    let unknown = bind_one("read \"{root}/a\" with: \"x\"", true).unwrap_err();
    assert!(
        matches!(unknown, BindFailure::Pending(ref r) if r.contains("not in the catalog of `read`"))
    );
    let executor = bind_one("find \"{root}\" limit: 3", true).unwrap_err();
    assert!(
        matches!(executor, BindFailure::Pending(ref r) if r.contains("applied by the executor"))
    );
}

#[test]
fn generators_produce_the_documented_content() {
    let png = tree::generate("png:640x480").unwrap();
    assert_eq!(&png[1..4], b"PNG");
    assert_eq!(&png[16..24], &[0, 0, 2, 128, 0, 0, 1, 224]);
    let webp = tree::generate("webp:VP8X,320x240").unwrap();
    assert_eq!(&webp[..4], b"RIFF");
    assert_eq!(&webp[8..16], b"WEBPVP8X");
    assert_eq!(&webp[24..30], &[63, 1, 0, 239, 0, 0]);
    let decodable = tree::generate("webp_image:32x16").unwrap();
    assert_eq!(&decodable[..4], b"RIFF");
    assert_eq!(&decodable[8..12], b"WEBP");
    let decoded = image::load_from_memory(&decodable).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (32, 16));
    for variant in ["VP8", "VP8L"] {
        let image = tree::webp(variant, 2, 2).unwrap();
        let declared = u32::from_le_bytes(image[4..8].try_into().unwrap());
        assert_eq!(declared as usize, image.len() - 8, "{variant}");
    }
    let pdf = String::from_utf8(tree::generate("pdf:page (one)").unwrap()).unwrap();
    assert!(
        pdf.starts_with("%PDF-1.4")
            && pdf.contains("(page \\(one\\)) Tj")
            && pdf.ends_with("%%EOF\n")
    );
    let source = String::from_utf8(tree::generate("big_source:80").unwrap()).unwrap();
    assert_eq!(source.lines().count(), 401);
    assert_eq!(
        source.lines().nth(396),
        Some("export function step79(value: number): number {")
    );
}

#[test]
fn generators_produce_text_bytes_and_notebooks() {
    assert_eq!(tree::generate("repeat:ab*3").unwrap(), b"ababab");
    assert_eq!(tree::generate("bytes:00ff").unwrap(), [0x00, 0xff]);
    assert!(tree::generate("bytes:0").is_err());
    for kind in ["cells4", "cells2", "flat", "missing_ids", "empty"] {
        let text = tree::generate(&format!("notebook:{kind}")).unwrap();
        let document: Json = serde_json::from_slice(&text).unwrap();
        assert_eq!(document["nbformat"], json!(4), "{kind}");
    }
    let bom = tree::generate("notebook:standard_crlf_bom").unwrap();
    assert!(bom.starts_with(&[0xEF, 0xBB, 0xBF]) && bom.windows(2).any(|w| w == b"\r\n"));
    assert_eq!(
        tree::parse_utc("2020-01-01T00:00:00Z").map(|t| t.0),
        Some(1_577_836_800_000_000_000)
    );
}

#[test]
fn trees_are_written_and_compared() {
    let model = Model::load(&data_dir()).unwrap();
    let test = TestContext::new();
    let entries = vec![
        FsEntry {
            path: Some("a/b.txt".into()),
            content: Some("caf\u{e9}\n".into()),
            encoding: Some("latin-1".into()),
            mode: Some("0751".into()),
            ..FsEntry::default()
        },
        FsEntry {
            path: Some("link.txt".into()),
            symlink_to: Some("a/b.txt".into()),
            ..FsEntry::default()
        },
    ];
    tree::populate(&test, &model, &entries).unwrap();
    let before = tree::snapshot(&test).unwrap();
    assert_eq!(
        before.get("a/b.txt"),
        Some(&Node::File(b"caf\xe9\n".to_vec(), 0o751))
    );
    assert_eq!(before.get("link.txt"), Some(&Node::Link("a/b.txt".into())));
    let content = AfterEntry {
        path: "a/b.txt".into(),
        content: Some("caf\u{e9}\n".into()),
        encoding: Some("latin-1".into()),
        mode: Some("0751".into()),
        ..AfterEntry::default()
    };
    assert_eq!(tree::check_after(&model, &content, &before), Ok(()));
    let excludes = AfterEntry {
        path: "gone.txt".into(),
        excludes: Some("x".into()),
        ..AfterEntry::default()
    };
    assert_eq!(tree::check_after(&model, &excludes, &before), Ok(()));
    let mut after = before.clone();
    after.remove("link.txt");
    after.insert("new.txt".into(), Node::File(Vec::new(), 0o644));
    assert_eq!(
        tree::tree_differences(&before, &after),
        ["link.txt was removed", "new.txt was created"]
    );
}

fn report_with(items: Vec<Item>) -> Report {
    Report {
        status: "ok".into(),
        items,
        ..Report::default()
    }
}

#[test]
fn field_paths_of_the_scenarios_map_onto_the_report() {
    let report = report_with(vec![
        Item::Record("entry".into(), json!({ "path": "a.txt", "size": 4 })),
        Item::Record("entry".into(), json!({ "path": "b.txt", "size": 7 })),
        Item::Record("match".into(), json!({ "no": 3 })),
    ]);
    assert_eq!(
        resolve(&report, "entries[*].path").unwrap(),
        json!(["a.txt", "b.txt"])
    );
    assert_eq!(resolve(&report, "entries[1].size").unwrap(), json!(7));
    assert_eq!(resolve(&report, "entry.path").unwrap(), json!("a.txt"));
    assert_eq!(resolve(&report, "matches.count").unwrap(), json!(1));
    assert_eq!(resolve(&report, "total").unwrap(), json!(3));
    assert_eq!(resolve(&report, "total_matches").unwrap(), json!(1));
    assert_eq!(resolve(&report, "effect.status").unwrap(), json!("applied"));
    assert!(resolve(&report, "next_cursor").is_err());
    assert!(resolve(&report, "undoable").is_err());
    assert!(resolve(&report, "effect.diff.added").is_err());
    let planned = Report {
        status: "planned".into(),
        effect: true,
        changes: vec![json!({ "path": "a.txt", "added": 2 })],
        ..Report::default()
    };
    assert_eq!(resolve(&planned, "preview").unwrap(), json!(true));
    assert_eq!(
        resolve(&planned, "effect.status").unwrap(),
        json!("previewed")
    );
    assert_eq!(resolve(&planned, "effect.change.added").unwrap(), json!(2));
    assert_eq!(resolve(&planned, "targets").unwrap(), json!(1));
}

#[test]
fn field_paths_look_into_a_list_of_one_and_stay_null_for_a_missing_member() {
    let report = Report {
        status: "ok".into(),
        effect: true,
        changes: vec![json!({ "path": "a.txt", "files": [{ "name": "x" }, { "name": "y" }] })],
        ..Report::default()
    };
    assert_eq!(
        resolve(&report, "effect.change.files[1].name").unwrap(),
        json!("y")
    );
    assert_eq!(
        resolve(&report, "effect.change.files[*].name").unwrap(),
        json!(["x", "y"])
    );
    assert_eq!(
        resolve(&report, "effect.change.files[7].name").unwrap(),
        Json::Null
    );
    let single = Report {
        changes: vec![json!({ "files": [{ "name": "x" }] })],
        ..report.clone()
    };
    assert_eq!(
        resolve(&single, "effect.change.files.name").unwrap(),
        json!("x")
    );
    assert_eq!(
        resolve(&single, "effect.change.missing").unwrap(),
        Json::Null
    );
    assert!(resolve(&single, "[0]").is_err());
    assert!(resolve(&single, "entries[1").is_err());
    assert!(resolve(&single, "entries[x]").is_err());
}

#[test]
fn a_missing_member_equals_only_an_empty_expectation() {
    assert!(same(&json!(""), &Json::Null));
    assert!(same(&json!([]), &Json::Null));
    assert!(same(&json!(0), &Json::Null));
    assert!(!same(&json!("a"), &Json::Null));
    assert!(!same(&json!(true), &Json::Null));
    assert!(!same(&json!(3), &Json::Null));
    assert!(!same(&json!({ "a": 1 }), &json!({ "a": 1, "b": 2 })));
    assert!(!same(&json!(2), &json!(["a"])));
}

#[test]
fn a_field_of_a_record_that_does_not_exist_is_not_an_empty_value() {
    let none = Report {
        status: "ok".into(),
        ..Report::default()
    };
    // No entry exists, so its member is not "empty": it is absent.
    assert!(check_field(&none, "entries[0].path", Comparison::Value, &json!("")).is_err());
    assert!(check_field(&none, "entries[0].size", Comparison::Value, &json!(0)).is_err());
    assert!(check_field(&none, "effect.change.added", Comparison::Value, &json!(0)).is_err());
    let one = report_with(vec![Item::Record("entry".into(), json!({ "path": "a" }))]);
    // The entry exists and omits the member, as the report omits empty ones.
    assert_eq!(
        check_field(&one, "entries[0].size", Comparison::Value, &json!(0)),
        Ok(())
    );
    assert!(check_field(&one, "entries[0].size", Comparison::Value, &json!(5)).is_err());
}

#[test]
fn excluding_a_value_needs_a_field_that_exists() {
    let listing = report_with(vec![Item::Record("entry".into(), json!({ "path": "a" }))]);
    assert_eq!(
        check_field(
            &listing,
            "entries[*].path",
            Comparison::Excludes,
            &json!("b")
        ),
        Ok(())
    );
    assert!(
        check_field(
            &listing,
            "entries[*].path",
            Comparison::Excludes,
            &json!("a")
        )
        .is_err()
    );
    let failed = Report {
        status: "failed".into(),
        ..Report::default()
    };
    assert!(
        check_field(&failed, "error.code", Comparison::Excludes, &json!("E_X")).is_err(),
        "no error exists, so a misspelled path cannot pass"
    );
}

#[test]
fn error_codes_of_the_previous_vocabulary_stand_only_for_their_listed_codes() {
    let failed = Report {
        status: "failed".into(),
        error: Some(json!({ "code": "E_INVALID_VALUE", "message": "bad value" })),
        ..Report::default()
    };
    assert_eq!(
        check_field(
            &failed,
            "error.code",
            Comparison::Value,
            &json!("E_ARGUMENT")
        ),
        Ok(())
    );
    for other in [
        "E_TYPE",
        "E_ARITY",
        "E_PARAM_CONFLICT",
        "E_UNKNOWN_VALUE",
        "E_IO",
    ] {
        let other = Report {
            error: Some(json!({ "code": other, "message": "other" })),
            ..failed.clone()
        };
        assert!(
            check_field(
                &other,
                "error.code",
                Comparison::Value,
                &json!("E_ARGUMENT")
            )
            .is_err(),
            "{other:?} stands for E_ARGUMENT"
        );
    }
    assert!(check_field(&failed, "error.code", Comparison::Value, &json!("E_RANGE")).is_err());
    assert!(check_field(&failed, "error.code", Comparison::Value, &json!("E_SYNTAX")).is_err());
}

#[test]
fn comparisons_follow_the_report_conventions() {
    assert!(same(&json!("8..8"), &json!("8")));
    assert!(same(&json!("1..3,15..15"), &json!("1..3,15")));
    assert!(!same(&json!("1..3"), &json!("1")));
    assert!(same(&json!(false), &Json::Null));
    assert!(same(&json!(2), &json!(["a", "b"])));
    assert!(same(&json!(1), &json!(1.0)));
    assert_failed_report_comparisons();
    let listing = report_with(vec![Item::Record(
        "entry".into(),
        json!({ "path": "src/a.ts" }),
    )]);
    assert_listing_comparisons(listing);
}

fn assert_failed_report_comparisons() {
    let failed = Report {
        status: "failed".into(),
        error: Some(json!({ "code": "E_OUT_OF_RANGE", "message": "line 99 is past the end" })),
        notable: vec!["nothing_matched".into()],
        ..Report::default()
    };
    assert_eq!(
        check_field(&failed, "error.code", Comparison::Value, &json!("E_RANGE")),
        Ok(())
    );
    assert!(check_field(&failed, "error.code", Comparison::Value, &json!("E_STALE")).is_err());
    assert_eq!(
        check_field(
            &failed,
            "notable",
            Comparison::Value,
            &json!("nothing_matched")
        ),
        Ok(())
    );
}

#[test]
fn a_failure_without_a_rollback_is_not_a_rollback() {
    let failed = Report {
        status: "failed".into(),
        error: Some(json!({ "code": "E_OUT_OF_RANGE", "message": "line 99 is past the end" })),
        ..Report::default()
    };
    let status = |report: &Report, word: &str| {
        check_field(report, "effect.status", Comparison::Value, &json!(word))
    };
    assert_eq!(status(&failed, "failed"), Ok(()));
    assert!(status(&failed, "rolled_back").is_err());
    let reverted = Report {
        status: "rolled_back".into(),
        ..failed.clone()
    };
    assert_eq!(status(&reverted, "rolled_back"), Ok(()));
    assert!(status(&reverted, "failed").is_err());
}

fn assert_listing_comparisons(listing: Report) {
    assert_eq!(
        check_field(
            &listing,
            "entries[*].path",
            Comparison::Contains,
            &json!("src/a.ts")
        ),
        Ok(())
    );
    assert_eq!(
        check_field(
            &listing,
            "entries[*].path",
            Comparison::Excludes,
            &json!("b.ts")
        ),
        Ok(())
    );
}

#[test]
fn stream_errors_without_records_become_the_program_error() {
    let mut report = report_with(vec![Item::Error(json!({ "code": "E_NOT_FOUND" }))]);
    report.promote_stream_error();
    assert_eq!(
        resolve(&report, "error.code").unwrap(),
        json!("E_NOT_FOUND")
    );
    let mut mixed = report_with(vec![
        Item::Record("content".into(), json!({ "unchanged": true })),
        Item::Error(json!({ "code": "E_NOT_FOUND" })),
    ]);
    mixed.promote_stream_error();
    assert!(mixed.error.is_none());
    assert_eq!(
        resolve(&mixed, "content[1].error.code").unwrap(),
        json!("E_NOT_FOUND")
    );
}

#[test]
fn runner_registers_the_stages_and_probes_the_reader() {
    let runner = Runner::new(&catalog_dir()).unwrap();
    assert!(runner.registered("read"));
    assert!(runner.registered("replace"));
    assert_eq!(
        runner.registered("archive.list"),
        nurt_files::stages()
            .iter()
            .any(|s| s.descriptor().command == "archive.list")
    );
    let model = Model::load(&data_dir()).unwrap();
    let scenario = model.scenarios().next().unwrap().clone();
    let mut unknown = scenario.clone();
    unknown.nurt_program = "replace_all \"{root}/a\" \"b\"".into();
    assert_eq!(
        runner.run(&model, &unknown),
        Outcome::Pending("command not in the catalog: replace_all".into())
    );
    let mut image = scenario;
    image.nurt_program = "read \"{root}/logo.png\"".into();
    if !runner.reads_non_text() {
        assert_eq!(
            runner.run(&model, &image),
            Outcome::Pending(format!("{NOT_MERGED}: read of non-text files"))
        );
    }
}
