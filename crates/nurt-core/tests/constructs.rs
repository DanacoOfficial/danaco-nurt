// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-core/tests/constructs.rs
// Responsibility:   Control constructs run end to end: the language front end, the planner, the executor, the journal and `undo`, over a real directory.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Control constructs run end to end: the language front end, the planner,
//! the executor, the journal and `undo`, over a real directory.
//!
//! Every construct is one journal entry, all or nothing, previewed without a
//! change and checked before any effect.

// Tests stop at the first unexpected value by unwrapping or panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use nurt_contracts::envelope::{Request, RequestOptions, Response};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::report::ReportStatus;
use nurt_core::config::ConfigSources;
use nurt_core::{Engine, EngineConfig, LanguageFrontend};
use serde_json::json;

fn engine(state: &Path) -> Engine {
    let frontend = LanguageFrontend::new().expect("embedded front end");
    Engine::open(
        EngineConfig::system()
            .with_sources(ConfigSources::isolated())
            .with_state_dir(state)
            .with_frontend(Arc::new(frontend)),
    )
    .expect("engine opens in a temporary directory")
}

/// A directory with an engine over it.
struct World {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    engine: Engine,
}

fn world() -> World {
    let temp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let root = temp.path().canonicalize().unwrap().join("work");
    fs::create_dir_all(&root).unwrap();
    let engine = engine(&temp.path().join("state"));
    World {
        _temp: temp,
        root,
        engine,
    }
}

impl World {
    /// A path of the work directory as program text.
    fn at(&self, name: &str) -> String {
        let path = self.root.join(name);
        path.to_string_lossy().replace('\\', "/")
    }

    fn put(&self, name: &str, text: &str) {
        let path = self.root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.root.join(name)).ok()
    }

    /// Content of every file below the work directory.
    fn snapshot(&self) -> BTreeMap<String, String> {
        let mut found = BTreeMap::new();
        let mut pending = vec![self.root.clone()];
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let name = path
                        .strip_prefix(&self.root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    found.insert(name, fs::read_to_string(&path).unwrap_or_default());
                }
            }
        }
        found
    }

    fn run(&self, program: &str) -> Response {
        let response = self.engine.run(&Request::program(program));
        response.validate().unwrap();
        response
    }

    fn preview(&self, program: &str) -> Response {
        let options = RequestOptions {
            preview: Some(true),
            ..RequestOptions::default()
        };
        let response = self
            .engine
            .run(&Request::program(program).with_options(options));
        response.validate().unwrap();
        response
    }

    fn with_params(&self, program: &str, params: serde_json::Value) -> Response {
        let mut request = Request::program(program);
        request.params = params.as_object().cloned();
        let response = self.engine.run(&request);
        response.validate().unwrap();
        response
    }
}

fn code(response: &Response) -> Option<ErrorCode> {
    response.report.error.as_ref().map(|error| error.code)
}

/// Volume, mount point and write access of a directory and of each of its
/// ancestors, for the message of a failed precondition.
fn volume_chain(root: &nurt_contracts::fs::AbsPath) -> String {
    let fs = nurt_fs::system_fs();
    let mut lines = String::new();
    let mut current = Some(root.clone());
    while let Some(dir) = current {
        let volume = fs.volume(&dir).map(|volume| {
            format!(
                "mount {} dev {} kind {:?} fs {} read_only {}",
                volume.mount, volume.id.dev, volume.kind, volume.fs_type, volume.read_only
            )
        });
        lines.push_str(&format!(
            "\n  {dir}: {volume:?}, writable {:?}",
            fs.can_write_dir(&dir)
        ));
        current = dir.parent();
    }
    lines
}

#[test]
fn the_directory_of_the_tests_is_on_a_volume_that_keeps_a_journal() {
    let world = world();
    let text = world.root.to_string_lossy();
    let root = nurt_contracts::fs::AbsPath::new(text.trim_start_matches(r"\\?\")).unwrap();
    let written = world.run(&format!(
        "write \"{a}\" \"1\" mode: overwrite\n",
        a = world.at("a")
    ));
    assert_ok(&written);
    let volumes = world.engine.space(Some(&root)).unwrap_or_else(|error| {
        panic!(
            "{:?}\nirreversible: {:?}\nnotable: {:?}\nvolumes:{}",
            error.data().message,
            written.report.irreversible,
            written.report.notable,
            volume_chain(&root)
        )
    });
    assert!(!volumes.is_empty(), "no volume is reported for {root}");
    for volume in &volumes {
        assert_eq!(
            volume.inactive,
            None,
            "the journal of {} ({}, free {}, total {}, reserve {}) is inactive\nvolumes:{}",
            volume.mount,
            volume.filesystem,
            volume.free,
            volume.total,
            volume.reserve,
            volume_chain(&root)
        );
    }
}

fn assert_ok(response: &Response) {
    assert_eq!(
        response.report.status,
        ReportStatus::Ok,
        "{:?}",
        response.report.error
    );
}

#[test]
fn when_runs_the_first_branch_whose_condition_holds() {
    let world = world();
    world.put("flag", "");
    let response = world.run(&format!(
        "when exists \"{flag}\" {{\n    write \"{out}\" \"first\" mode: overwrite\n}} otherwise {{\n    write \"{out}\" \"second\" mode: overwrite\n}}\n",
        flag = world.at("flag"),
        out = world.at("out"),
    ));
    assert_ok(&response);
    assert_eq!(world.read("out").as_deref(), Some("first"));
    assert_eq!(response.report.kind, "when");
    let branch = &response.report.parts[0];
    assert_eq!(branch.kind, "branch");
    assert_eq!(branch.condition, Some(true));
    assert_eq!(branch.branch, Some(1));
    assert!(response.report.op.is_some());
}

#[test]
fn when_falls_through_otherwise_when_to_otherwise() {
    let world = world();
    let program = |marker: &str| {
        format!(
            "when exists \"{a}\" {{\n    write \"{out}\" \"a\" mode: overwrite\n}} otherwise when exists \"{b}\" {{\n    write \"{out}\" \"b\" mode: overwrite\n}} otherwise {{\n    write \"{out}\" \"none\" mode: overwrite\n}}\n",
            a = world.at("a"),
            b = world.at(marker),
            out = world.at("out"),
        )
    };
    world.put("b", "");
    let second = world.run(&program("b"));
    assert_ok(&second);
    assert_eq!(world.read("out").as_deref(), Some("b"));
    assert_eq!(second.report.parts[0].branch, Some(2));
    fs::remove_file(world.root.join("b")).unwrap();
    fs::remove_file(world.root.join("out")).unwrap();
    let none = world.run(&program("b"));
    assert_ok(&none);
    assert_eq!(world.read("out").as_deref(), Some("none"));
    assert_eq!(none.report.parts[0].branch, Some(3));
}

#[test]
fn a_condition_that_is_not_bool_fails_before_any_effect() {
    let world = world();
    let response = world.run(&format!(
        "when 5 {{\n    write \"{}\" \"x\" mode: overwrite\n}}\n",
        world.at("out")
    ));
    assert_eq!(response.report.status, ReportStatus::Failed);
    assert_eq!(code(&response), Some(ErrorCode::Type));
    assert!(world.read("out").is_none());
}

#[test]
fn the_condition_reads_a_pipeline_in_parentheses() {
    let world = world();
    world.put("logs/a.log", "1");
    world.put("logs/b.log", "2");
    let response = world.run(&format!(
        "when (find \"{logs}\" glob: \"*.log\" |> count) > 1 {{\n    write \"{out}\" \"many\" mode: overwrite\n}} otherwise {{\n    write \"{out}\" \"few\" mode: overwrite\n}}\n",
        logs = world.at("logs"),
        out = world.at("out"),
    ));
    assert_ok(&response);
    assert_eq!(world.read("out").as_deref(), Some("many"));
}

#[test]
fn each_runs_the_block_per_record_in_one_entry_and_one_undo_reverses_it() {
    let world = world();
    for name in ["a", "b", "c"] {
        world.put(&format!("src/{name}.log"), name);
    }
    let before = world.snapshot();
    let response = world.run(&format!(
        "each (find \"{src}\" glob: \"*.log\") as f {{\n    copy f.path into: \"{dst}\"\n}}\n",
        src = world.at("src"),
        dst = world.at("dst"),
    ));
    assert_ok(&response);
    assert_eq!(response.report.kind, "each");
    for name in ["a", "b", "c"] {
        assert_eq!(
            world.read(&format!("dst/{name}.log")).as_deref(),
            Some(name)
        );
    }
    let iterations = response
        .report
        .parts
        .iter()
        .find(|part| part.kind == "loop")
        .and_then(|part| part.iterations);
    assert_eq!(iterations, Some(3));
    let undone = world.run("undo");
    assert_ok(&undone);
    assert_eq!(world.snapshot(), before);
}

#[test]
fn a_failure_in_the_third_iteration_restores_the_state_before() {
    let world = world();
    for name in ["a", "b", "c"] {
        world.put(&format!("src/{name}.log"), name);
    }
    world.put("dst/c.log", "old");
    let before = world.snapshot();
    let response = world.run(&format!(
        "each (find \"{src}\" glob: \"*.log\") as f {{\n    copy f.path into: \"{dst}\" on_conflict: error\n}}\n",
        src = world.at("src"),
        dst = world.at("dst"),
    ));
    assert_ne!(response.report.status, ReportStatus::Ok);
    assert_eq!(world.snapshot(), before);
    let failed = response.report.parts.iter().filter(|part| !part.ok).count();
    assert!(failed >= 1);
}

#[test]
fn a_failure_in_the_third_of_five_steps_restores_the_state_before() {
    let world = world();
    world.put("keep", "k");
    let before = world.snapshot();
    let response = world.run(&format!(
        "steps {{\n    write \"{a}\" \"1\" mode: overwrite\n    write \"{b}\" \"2\" mode: overwrite\n    copy \"{missing}\" into: \"{dst}\"\n    write \"{d}\" \"4\" mode: overwrite\n    write \"{e}\" \"5\" mode: overwrite\n}}\n",
        a = world.at("a"),
        b = world.at("b"),
        missing = world.at("missing"),
        dst = world.at("dst"),
        d = world.at("d"),
        e = world.at("e"),
    ));
    assert_ne!(response.report.status, ReportStatus::Ok);
    assert_eq!(world.snapshot(), before);
}

#[test]
fn steps_is_one_entry_and_one_undo_reverses_every_step() {
    let world = world();
    world.put("old.conf", "x");
    let before = world.snapshot();
    let response = world.run(&format!(
        "steps {{\n    write \"{a}\" \"1\" mode: overwrite\n    write \"{b}\" \"2\" mode: overwrite\n    move \"{old}\" to: \"{moved}\"\n}}\n",
        a = world.at("a"),
        b = world.at("b"),
        old = world.at("old.conf"),
        moved = world.at("new.conf"),
    ));
    assert_ok(&response);
    assert!(world.read("a").is_some() && world.read("new.conf").is_some());
    assert_eq!(
        response
            .report
            .parts
            .iter()
            .filter(|part| part.kind == "step")
            .count(),
        3
    );
    assert_ok(&world.run("undo"));
    assert_eq!(world.snapshot(), before);
}

#[test]
fn preview_changes_nothing_and_lists_the_effects() {
    let world = world();
    for name in ["a", "b"] {
        world.put(&format!("src/{name}.log"), name);
    }
    let before = world.snapshot();
    let response = world.preview(&format!(
        "each (find \"{src}\" glob: \"*.log\") as f {{\n    copy f.path into: \"{dst}\"\n}}\n",
        src = world.at("src"),
        dst = world.at("dst"),
    ));
    assert_eq!(response.report.status, ReportStatus::Planned);
    assert_eq!(world.snapshot(), before);
    let planned: Vec<&str> = response
        .report
        .changes
        .iter()
        .map(|change| change.path.as_str())
        .collect();
    assert!(
        planned.iter().any(|path| path.ends_with("dst/a.log")),
        "{planned:?}"
    );
    assert!(
        planned.iter().any(|path| path.ends_with("dst/b.log")),
        "{planned:?}"
    );
}

#[test]
fn a_stage_written_with_preview_plans_without_applying() {
    let world = world();
    world.put("a.txt", "x");
    let before = world.snapshot();
    let program = format!(
        "params {{\n    dry: bool = false\n}}\nsteps {{\n    remove \"{a}\" preview: @dry\n}}\n",
        a = world.at("a.txt")
    );
    let entries = |world: &World| world.engine.history(&Default::default()).unwrap().len();
    let entries_before = entries(&world);
    let dry = world.with_params(&program, json!({"dry": true}));
    assert_eq!(world.snapshot(), before);
    assert_eq!(
        entries(&world),
        entries_before,
        "an empty entry is not written"
    );
    assert!(dry.report.changes.is_empty(), "{:?}", dry.report.changes);
    assert!(dry.report.op.is_none());
    assert!(
        dry.report
            .parts
            .iter()
            .any(|part| part.note.as_deref() == Some("planned, not applied"))
    );
    let real = world.with_params(&program, json!({}));
    assert_ok(&real);
    assert!(world.read("a.txt").is_none());
}

#[test]
fn preview_written_without_a_value_means_true() {
    let world = world();
    world.put("a.txt", "x");
    let response = world.run(&format!("remove \"{}\" preview", world.at("a.txt")));
    assert_eq!(response.report.status, ReportStatus::Planned);
    assert!(world.read("a.txt").is_some());
}

#[test]
fn while_ends_at_its_limit_and_says_so() {
    let world = world();
    world.put("marker", "");
    let response = world.run(&format!(
        "while exists \"{marker}\" limit: 3 {{\n    write \"{log}\" \"x\" mode: append\n}}\n",
        marker = world.at("marker"),
        log = world.at("log"),
    ));
    assert_ok(&response);
    assert_eq!(world.read("log").as_deref(), Some("xxx"));
    let part = response
        .report
        .parts
        .iter()
        .find(|part| part.kind == "loop")
        .unwrap();
    assert_eq!(part.iterations, Some(3));
    assert!(part.note.is_some());
}

#[test]
fn repeat_until_ends_with_a_limit_error_when_the_condition_is_never_met() {
    let world = world();
    let response = world.run(&format!(
        "repeat until: exists \"{never}\" every: 1ms limit: 3 {{\n    write \"{log}\" \"x\" mode: append\n}}\n",
        never = world.at("never"),
        log = world.at("log"),
    ));
    assert_ne!(response.report.status, ReportStatus::Ok);
    assert!(
        world.read("log").is_none(),
        "the loop is one all or nothing entry"
    );
}

#[test]
fn repeat_until_stops_when_the_condition_holds() {
    let world = world();
    world.put("marker", "");
    let response = world.run(&format!(
        "repeat until: not exists \"{marker}\" limit: 5 {{\n    remove \"{marker}\"\n}}\n",
        marker = world.at("marker"),
    ));
    assert_ok(&response);
    assert!(world.read("marker").is_none());
}

#[test]
fn break_ends_the_loop_and_continue_skips_the_rest_of_the_block() {
    let world = world();
    for name in ["a", "b", "c"] {
        world.put(&format!("src/{name}.log"), name);
    }
    let response = world.run(&format!(
        "each (find \"{src}\" glob: \"*.log\" sort: path) as f {{\n    when f.name == \"a.log\" {{\n        continue\n    }}\n    when f.name == \"c.log\" {{\n        break\n    }}\n    copy f.path into: \"{dst}\"\n}}\n",
        src = world.at("src"),
        dst = world.at("dst"),
    ));
    assert_ok(&response);
    assert!(world.read("dst/a.log").is_none());
    assert_eq!(world.read("dst/b.log").as_deref(), Some("b"));
    assert!(world.read("dst/c.log").is_none());
}

#[test]
fn break_outside_a_loop_is_rejected_before_any_effect() {
    let world = world();
    let response = world.run(&format!(
        "steps {{\n    write \"{}\" \"x\" mode: overwrite\n    break\n}}\n",
        world.at("out")
    ));
    assert_eq!(response.report.status, ReportStatus::Failed);
    assert!(world.read("out").is_none());
}

#[test]
fn a_function_has_defaults_and_its_own_scope() {
    let world = world();
    let response = world.run(&format!(
        "define put(dir: text, name: text = \"x.txt\") {{\n    write @dir + \"/\" + @name \"hi\" mode: overwrite\n}}\nput(\"{dir}\")\nput(\"{dir}\", name: \"y.txt\")\n",
        dir = world.at("out"),
    ));
    assert_ok(&response);
    assert_eq!(world.read("out/x.txt").as_deref(), Some("hi"));
    assert_eq!(world.read("out/y.txt").as_deref(), Some("hi"));
}

#[test]
fn a_function_does_not_see_the_names_of_its_caller() {
    let world = world();
    let response = world.run(&format!(
        "let outer = find \"{}\"\ndefine f() {{\n    write \"/never\" \"x\" mode: overwrite\n    @outer |> count\n}}\nf()\n",
        world.at("")
    ));
    assert_eq!(response.report.status, ReportStatus::Failed);
    assert_eq!(code(&response), Some(ErrorCode::UnknownName));
}

#[test]
fn a_call_with_a_wrong_argument_fails_before_any_effect() {
    let world = world();
    let program = |call: &str| {
        format!(
            "define put(dir: text, times: int = 1) {{\n    write @dir \"hi\" mode: overwrite\n}}\n{call}\n"
        )
    };
    let missing = world.run(&program("put()"));
    assert_eq!(code(&missing), Some(ErrorCode::MissingParam));
    let unknown = world.run(&program("put(\"x\", tims: 2)"));
    assert_eq!(code(&unknown), Some(ErrorCode::UnknownParam));
    let wrong = world.run(&program("put(\"x\", times: \"many\")"));
    assert_eq!(code(&wrong), Some(ErrorCode::Type));
    let many = world.run(&program("put(\"x\", 1, 2)"));
    assert_eq!(code(&many), Some(ErrorCode::Arity));
}

#[test]
fn recursion_stops_at_the_depth_limit() {
    let world = world();
    let response = world.run("define again() {\n    again()\n}\nagain()\n");
    assert_eq!(code(&response), Some(ErrorCode::Limit));
}

#[test]
fn script_parameters_are_checked_before_any_effect() {
    let world = world();
    let program = format!(
        "params {{\n    name: text\n    times: int = 2\n    dry: bool = false\n}}\nwhen @dry {{\n    write \"{out}\" \"dry\" mode: overwrite\n}} otherwise {{\n    write \"{out}\" @name mode: overwrite\n}}\n",
        out = world.at("out"),
    );
    let missing = world.with_params(&program, json!({}));
    assert_eq!(code(&missing), Some(ErrorCode::MissingParam));
    let unknown = world.with_params(&program, json!({"name": "x", "nam": "y"}));
    assert_eq!(code(&unknown), Some(ErrorCode::UnknownParam));
    let wrong = world.with_params(&program, json!({"name": "x", "times": "often"}));
    assert!(
        matches!(
            code(&wrong),
            Some(ErrorCode::Type | ErrorCode::InvalidValue)
        ),
        "{:?}",
        wrong.report.error
    );
    let declaration = &wrong.report.error.as_ref().unwrap().details["declaration"];
    assert!(
        declaration.to_string().contains("times: int = 2"),
        "{declaration}"
    );
    assert!(world.read("out").is_none());
    let given = world.with_params(&program, json!({"name": "hello", "times": "3"}));
    assert_ok(&given);
    assert_eq!(world.read("out").as_deref(), Some("hello"));
    let dry = world.with_params(&program, json!({"name": "hello", "dry": true}));
    assert_ok(&dry);
    assert_eq!(world.read("out").as_deref(), Some("dry"));
}

#[test]
fn a_program_without_parameters_takes_no_values() {
    let world = world();
    let response = world.with_params(&format!("find \"{}\"", world.at("")), json!({"x": 1}));
    assert_eq!(code(&response), Some(ErrorCode::UnknownParam));
}

#[test]
fn atomic_false_keeps_the_completed_iterations_and_names_the_failed_one() {
    let world = world();
    for name in ["a", "b", "c"] {
        world.put(&format!("src/{name}.log"), name);
    }
    world.put("dst/c.log", "old");
    let response = world.run(&format!(
        "each (find \"{src}\" glob: \"*.log\") as f atomic: false {{\n    copy f.path into: \"{dst}\" on_conflict: error\n}}\n",
        src = world.at("src"),
        dst = world.at("dst"),
    ));
    assert_eq!(
        response.report.status,
        ReportStatus::Partial,
        "{:?}",
        response.report.error
    );
    assert_eq!(world.read("dst/a.log").as_deref(), Some("a"));
    assert_eq!(world.read("dst/b.log").as_deref(), Some("b"));
    assert_eq!(world.read("dst/c.log").as_deref(), Some("old"));
    let failed: Vec<_> = response
        .report
        .parts
        .iter()
        .filter(|part| !part.ok && part.kind == "iteration")
        .collect();
    assert_eq!(failed.len(), 1);
    let error = response.report.error.as_ref().unwrap();
    assert_eq!(error.details.get("iteration"), Some(&json!(3)));
    assert!(response.report.op.is_some());
    assert_eq!(
        response
            .report
            .parts
            .iter()
            .filter_map(|part| part.op)
            .count(),
        0,
        "the kept iterations are in the one entry of the loop"
    );
    assert_eq!(
        world.engine.history(&Default::default()).unwrap().len(),
        1,
        "the whole loop is one journal entry"
    );
    assert_ok(&world.run("undo"));
    assert!(world.read("dst/a.log").is_none() && world.read("dst/b.log").is_none());
    assert_eq!(world.read("dst/c.log").as_deref(), Some("old"));
}

#[test]
fn a_parallel_each_visits_every_record() {
    let world = world();
    for name in ["a", "b"] {
        world.put(&format!("src/{name}.log"), name);
    }
    let response = world.run(&format!(
        "each (find \"{src}\" glob: \"*.log\") as f parallel: 2 {{\n    copy f.path into: \"{dst}\"\n}}\n",
        src = world.at("src"),
        dst = world.at("dst"),
    ));
    assert_ok(&response);
    assert!(world.read("dst/a.log").is_some() && world.read("dst/b.log").is_some());
}

#[test]
fn a_function_result_is_the_value_of_its_last_statement() {
    let world = world();
    world.put("logs/a.log", "1");
    world.put("logs/b.log", "2");
    let response = world.run(&format!(
        "define list_logs(dir: path) {{\n    find @dir glob: \"*.log\" |> count\n}}\nlet n = list_logs(\"{}\")\n@n\n",
        world.at("logs")
    ));
    assert_ok(&response);
    let page = response.report.records.as_ref().unwrap();
    assert_eq!(
        page.rows[0].get("value"),
        Some(&nurt_contracts::value::Value::from(2_u64))
    );
}

#[test]
fn the_preview_of_every_construct_changes_nothing() {
    let world = world();
    world.put("marker", "");
    world.put("src/a.log", "a");
    let before = world.snapshot();
    let at = |name: &str| world.at(name);
    let programs = [
        format!(
            "when exists \"{}\" {{\n    write \"{}\" \"x\" mode: overwrite\n}}\n",
            at("marker"),
            at("out")
        ),
        format!(
            "steps {{\n    write \"{}\" \"1\" mode: overwrite\n    write \"{}\" \"2\" mode: overwrite\n}}\n",
            at("a"),
            at("b")
        ),
        format!(
            "while exists \"{}\" limit: 3 {{\n    write \"{}\" \"x\" mode: append\n}}\n",
            at("marker"),
            at("log")
        ),
        format!(
            "repeat until: exists \"{}\" limit: 3 {{\n    write \"{}\" \"x\" mode: append\n}}\n",
            at("never"),
            at("log")
        ),
        format!(
            "define put(file: text) {{\n    write @file \"x\" mode: overwrite\n}}\nput(\"{}\")\n",
            at("viaf")
        ),
        format!(
            "each (find \"{}\" glob: \"*.log\") as f {{\n    remove f.path\n}}\n",
            at("src")
        ),
    ];
    for program in &programs {
        let response = world.preview(program);
        assert_eq!(
            response.report.status,
            ReportStatus::Planned,
            "{program}\n{:?}",
            response.report.error
        );
        assert!(!response.report.changes.is_empty(), "{program}");
        assert_eq!(world.snapshot(), before, "{program}");
    }
}

#[test]
fn a_part_that_cannot_be_rolled_back_is_listed_and_named_after_a_failure() {
    let world = world();
    let args = if cfg!(windows) {
        "[\"cmd\", \"/c\", \"exit 0\"]"
    } else {
        "[\"sh\", \"-c\", \"exit 0\"]"
    };
    let response = world.run(&format!(
        "steps {{\n    write \"{a}\" \"1\" mode: overwrite\n    run {args} cwd: \"{cwd}\"\n    copy \"{missing}\" into: \"{dst}\"\n}}\n",
        a = world.at("a"),
        cwd = world.at(""),
        missing = world.at("missing"),
        dst = world.at("dst"),
    ));
    assert_eq!(response.report.status, ReportStatus::RolledBack);
    assert!(world.read("a").is_none(), "the write is rolled back");
    let error = response.report.error.as_ref().unwrap();
    assert_eq!(error.code, ErrorCode::RolledBack);
    let unrolled = error.details["not_rolled_back"].as_array().unwrap();
    assert!(
        unrolled
            .iter()
            .any(|entry| entry.to_string().contains("`run`")),
        "{unrolled:?}"
    );
    let part = |command: &str| {
        response
            .report
            .parts
            .iter()
            .find(|part| part.command.as_deref() == Some(command))
            .unwrap()
    };
    assert!(part("run").irreversible && !part("run").rolled_back);
    assert!(part("write").rolled_back && !part("write").irreversible);
    assert!(
        response
            .report
            .irreversible
            .iter()
            .any(|entry| entry.detail.as_deref().is_some_and(|d| d.contains("`run`"))),
        "{:?}",
        response.report.irreversible
    );
}

#[test]
fn irreversible_parts_are_listed_before_the_change_in_the_preview() {
    let world = world();
    let args = if cfg!(windows) {
        "[\"cmd\", \"/c\", \"exit 0\"]"
    } else {
        "[\"sh\", \"-c\", \"exit 0\"]"
    };
    let before = world.snapshot();
    let response = world.preview(&format!(
        "steps {{\n    write \"{a}\" \"1\" mode: overwrite\n    run {args} cwd: \"{cwd}\"\n}}\n",
        a = world.at("a"),
        cwd = world.at(""),
    ));
    assert_eq!(response.report.status, ReportStatus::Planned);
    assert_eq!(world.snapshot(), before);
    assert!(
        response
            .report
            .irreversible
            .iter()
            .any(|entry| entry.detail.as_deref().is_some_and(|d| d.contains("`run`"))),
        "{:?}",
        response.report.irreversible
    );
}

#[test]
fn a_failure_in_the_fiftieth_of_a_hundred_iterations_restores_the_state_before() {
    let world = world();
    for number in 1..=100 {
        world.put(&format!("src/f{number:03}.log"), &number.to_string());
    }
    world.put("dst/f050.log", "old");
    let before = world.snapshot();
    let response = world.run(&format!(
        "each (find \"{src}\" glob: \"*.log\") as f {{\n    copy f.path into: \"{dst}\" on_conflict: error\n}}\n",
        src = world.at("src"),
        dst = world.at("dst"),
    ));
    assert_ne!(response.report.status, ReportStatus::Ok);
    assert_eq!(world.snapshot(), before);
    let failed: Vec<_> = response
        .report
        .parts
        .iter()
        .filter(|part| !part.ok)
        .collect();
    assert_eq!(failed.len(), 1);
    assert!(failed[0].at.contains("iteration[50]"), "{}", failed[0].at);
}

#[test]
fn a_failure_inside_a_branch_restores_the_state_before() {
    let world = world();
    world.put("flag", "");
    let before = world.snapshot();
    let response = world.run(&format!(
        "when exists \"{flag}\" {{\n    write \"{a}\" \"1\" mode: overwrite\n    copy \"{missing}\" into: \"{dst}\"\n}}\n",
        flag = world.at("flag"),
        a = world.at("a"),
        missing = world.at("missing"),
        dst = world.at("dst"),
    ));
    assert_ne!(response.report.status, ReportStatus::Ok);
    assert_eq!(world.snapshot(), before);
}

#[test]
fn constructs_nest_each_in_steps_in_when() {
    let world = world();
    world.put("flag", "");
    for name in ["a", "b"] {
        world.put(&format!("src/{name}.log"), name);
    }
    let before = world.snapshot();
    let response = world.run(&format!(
        "when exists \"{flag}\" {{\n    steps {{\n        write \"{marker}\" \"done\" mode: overwrite\n        each (find \"{src}\" glob: \"*.log\") as f {{\n            copy f.path into: \"{dst}\"\n        }}\n    }}\n}}\n",
        flag = world.at("flag"),
        marker = world.at("marker"),
        src = world.at("src"),
        dst = world.at("dst"),
    ));
    assert_ok(&response);
    assert_eq!(world.read("marker").as_deref(), Some("done"));
    assert_eq!(world.read("dst/a.log").as_deref(), Some("a"));
    assert_eq!(response.report.kind, "when");
    assert!(response.report.op.is_some());
    assert_ok(&world.run("undo"));
    assert_eq!(world.snapshot(), before);
}

#[test]
fn parallel_runs_disjoint_iterations_together_in_one_entry() {
    let world = world();
    for number in 1..=6 {
        world.put(&format!("src/f{number}.log"), &number.to_string());
    }
    world.put("dst/keep", "k");
    let before = world.snapshot();
    let response = world.run(&format!(
        "each (find \"{src}\" glob: \"*.log\") as f parallel: 3 {{\n    copy f.path into: \"{dst}\"\n}}\n",
        src = world.at("src"),
        dst = world.at("dst"),
    ));
    assert_ok(&response);
    for number in 1..=6 {
        assert_eq!(
            world.read(&format!("dst/f{number}.log")),
            Some(number.to_string())
        );
    }
    let note = response
        .report
        .parts
        .iter()
        .find(|part| part.kind == "loop")
        .and_then(|part| part.note.clone())
        .unwrap_or_default();
    assert!(note.contains("at a time"), "{note}");
    assert_ok(&world.run("undo"));
    assert_eq!(world.snapshot(), before);
}

#[test]
fn parallel_falls_back_to_order_when_the_targets_overlap_and_says_so() {
    let world = world();
    for name in ["a", "b"] {
        world.put(&format!("src/{name}.log"), name);
    }
    let response = world.run(&format!(
        "each (find \"{src}\" glob: \"*.log\") as f parallel: 2 {{\n    write \"{out}\" f.name mode: append\n}}\n",
        src = world.at("src"),
        out = world.at("out"),
    ));
    assert_ok(&response);
    assert_eq!(world.read("out").as_deref(), Some("a.logb.log"));
    let note = response
        .report
        .parts
        .iter()
        .find(|part| part.kind == "loop")
        .and_then(|part| part.note.clone())
        .unwrap_or_default();
    assert!(note.contains("overlap"), "{note}");
}

#[test]
fn a_misspelt_command_inside_a_block_is_corrected_like_at_top_level() {
    let world = world();
    world.put("a.txt", "x");
    let reading = world.run(&format!("steps {{\n    lsit \"{}\"\n}}\n", world.at("")));
    assert_ok(&reading);
    assert!(
        !reading.report.corrections.is_empty(),
        "{:?}",
        reading.report
    );
    let changing = world.run(&format!(
        "steps {{\n    remvoe \"{}\"\n}}\n",
        world.at("a.txt")
    ));
    assert_eq!(code(&changing), Some(ErrorCode::Ambiguous));
    assert!(world.read("a.txt").is_some());
}

#[test]
fn an_explicit_limit_replaces_the_default_limit_of_the_other_dimension() {
    let world = world();
    world.put("marker", "");
    let response = world.run(&format!(
        "while exists \"{marker}\" limit: 25 {{\n    write \"{log}\" \"x\" mode: append\n}}\n",
        marker = world.at("marker"),
        log = world.at("log"),
    ));
    assert_ok(&response);
    assert_eq!(world.read("log").map(|text| text.len()), Some(25));
    let timed = world.run(&format!(
        "repeat until: exists \"{never}\" every: 1ms limit: 400ms {{\n    write \"{log}\" \"y\" mode: append\n}}\n",
        never = world.at("never"),
        log = world.at("log2"),
    ));
    let cause = timed.report.error.as_ref().unwrap().cause.as_ref().unwrap();
    assert_eq!(cause.code, ErrorCode::Limit);
    let iterations = cause.details["iterations"].as_u64().unwrap();
    assert!(iterations > 1, "{iterations}");
}

#[test]
fn a_condition_that_cannot_be_bool_is_refused_before_any_effect() {
    let world = world();
    world.put("src/a.log", "a");
    for condition in [
        format!("(find \"{}\" |> count)", world.at("src")),
        "exit_code".to_owned(),
    ] {
        let response = world.run(&format!(
            "steps {{\n    write \"{out}\" \"x\" mode: overwrite\n    when {condition} {{\n        remove \"{out}\"\n    }}\n}}\n",
            out = world.at("out"),
        ));
        assert_eq!(code(&response), Some(ErrorCode::Type), "{condition}");
        assert!(world.read("out").is_none(), "{condition}");
    }
}

#[test]
fn a_function_that_changes_cannot_be_called_inside_a_condition() {
    let world = world();
    let response = world.run(&format!(
        "define clean() {{\n    write \"{out}\" \"x\" mode: overwrite\n}}\nwhen (clean() |> count) > 0 {{\n    find \"{dir}\"\n}}\n",
        out = world.at("out"),
        dir = world.at(""),
    ));
    assert_eq!(code(&response), Some(ErrorCode::Type));
    assert!(world.read("out").is_none());
}

#[test]
fn define_inside_a_block_is_refused_before_any_effect() {
    let world = world();
    let response = world.run(&format!(
        "steps {{\n    write \"{out}\" \"x\" mode: overwrite\n    when exists \"{no}\" {{\n        define f() {{\n            find \"{dir}\"\n        }}\n    }}\n}}\nf()\n",
        out = world.at("out"),
        no = world.at("nothing"),
        dir = world.at(""),
    ));
    assert_eq!(code(&response), Some(ErrorCode::Arg));
    assert!(world.read("out").is_none());
}

#[test]
fn a_function_does_not_see_the_exit_code_of_its_caller() {
    let world = world();
    let args = if cfg!(windows) {
        "[\"cmd\", \"/c\", \"exit 0\"]"
    } else {
        "[\"sh\", \"-c\", \"exit 0\"]"
    };
    let response = world.run(&format!(
        "define probe() {{\n    when exit_code == 0 {{\n        write \"{out}\" \"seen\" mode: overwrite\n    }}\n}}\nsteps {{\n    run {args} cwd: \"{cwd}\"\n    probe()\n}}\n",
        out = world.at("out"),
        cwd = world.at(""),
    ));
    assert_ne!(
        response.report.status,
        ReportStatus::Ok,
        "the name is not visible"
    );
    assert!(world.read("out").is_none());
}

#[test]
fn nested_atomic_false_loops_keep_the_outer_savepoint() {
    let world = world();
    for name in ["a", "b"] {
        world.put(&format!("src/{name}.log"), name);
        world.put(&format!("in/{name}.txt"), name);
    }
    let before = world.snapshot();
    let response = world.run(&format!(
        "each (find \"{src}\" glob: \"*.log\") as a atomic: false {{\n    write \"{mark}\" a.name mode: overwrite\n    each (find \"{inner}\" glob: \"*.txt\") as b atomic: false {{\n        copy b.path into: \"{dst}\"\n    }}\n    copy \"{missing}\" into: \"{dst}\"\n}}\n",
        src = world.at("src"),
        mark = world.at("mark"),
        inner = world.at("in"),
        dst = world.at("dst"),
        missing = world.at("missing"),
    ));
    assert_ne!(response.report.status, ReportStatus::Ok);
    assert_eq!(
        world.snapshot(),
        before,
        "the failed outer iteration is wholly restored"
    );
}

#[test]
fn a_rolled_back_move_is_restored_once_and_the_loop_keeps_the_earlier_iterations() {
    let world = world();
    for name in ["a", "b", "c"] {
        world.put(&format!("src/{name}.log"), name);
    }
    world.put("dst/c.log", "old");
    let response = world.run(&format!(
        "each (find \"{src}\" glob: \"*.log\" sort: path) as f atomic: false {{\n    move f.path into: \"{dst}\" on_conflict: error\n}}\n",
        src = world.at("src"),
        dst = world.at("dst"),
    ));
    assert_eq!(
        response.report.status,
        ReportStatus::Partial,
        "{:?}",
        response.report.error
    );
    assert_eq!(world.read("dst/a.log").as_deref(), Some("a"));
    assert_eq!(world.read("dst/b.log").as_deref(), Some("b"));
    assert_eq!(world.read("src/c.log").as_deref(), Some("c"));
    assert_eq!(world.read("dst/c.log").as_deref(), Some("old"));
    assert_ok(&world.run("undo"));
    assert_eq!(world.read("src/a.log").as_deref(), Some("a"));
    assert_eq!(world.read("src/b.log").as_deref(), Some("b"));
    assert!(world.read("dst/a.log").is_none());
}

#[test]
fn a_loop_that_is_not_atomic_followed_by_a_failing_step_is_restored_whole() {
    let world = world();
    for name in ["a", "b"] {
        world.put(&format!("src/{name}.log"), name);
    }
    let before = world.snapshot();
    let response = world.run(&format!(
        "steps {{\n    each (find \"{src}\" glob: \"*.log\") as f atomic: false {{\n        move f.path into: \"{dst}\"\n    }}\n    copy \"{missing}\" into: \"{dst}\"\n}}\n",
        src = world.at("src"),
        dst = world.at("dst"),
        missing = world.at("missing"),
    ));
    assert_eq!(
        response.report.status,
        ReportStatus::RolledBack,
        "{:?}",
        response.report.error
    );
    assert_eq!(world.snapshot(), before);
}

#[test]
fn a_field_of_the_loop_record_that_is_not_bool_is_refused_before_any_effect() {
    let world = world();
    world.put("src/a.log", "a");
    let response = world.run(&format!(
        "steps {{\n    write \"{out}\" \"x\" mode: overwrite\n    each (find \"{src}\") as f {{\n        when f.size {{\n            remove f.path\n        }}\n    }}\n}}\n",
        out = world.at("out"),
        src = world.at("src"),
    ));
    assert_eq!(code(&response), Some(ErrorCode::Type));
    assert!(world.read("out").is_none());
}

#[test]
fn an_effect_picked_by_a_value_that_is_not_a_literal_is_refused_in_a_condition() {
    let world = world();
    let response = world.run(
        "params {\n    m: text = \"post\"\n}\nwhen ((http.request \"https://example.invalid/\" method: @m) |> first 1).status == 200 {\n    find \"/\"\n}\n",
    );
    assert_eq!(
        code(&response),
        Some(ErrorCode::Type),
        "{:?}",
        response.report.error
    );
}

fn field_condition_is_refused_before_any_effect(condition: &str) {
    let world = world();
    world.put("src/a.log", "a");
    let response = world.run(&format!(
        "steps {{\n    write \"{out}\" \"x\" mode: overwrite\n    each (find \"{src}\") as f {{\n        when {condition} {{\n            remove f.path\n        }}\n    }}\n}}\n",
        out = world.at("out"),
        src = world.at("src"),
    ));
    assert_eq!(
        code(&response),
        Some(ErrorCode::Type),
        "{condition}: {:?}",
        response.report.error
    );
    assert!(world.read("out").is_none(), "{condition}");
    assert!(world.read("src/a.log").is_some(), "{condition}");
}

#[test]
fn every_misuse_of_a_field_of_a_known_type_is_refused_before_any_effect() {
    for condition in [
        "f.size ~ \"x\"",
        "f.size > 1",
        "f.name ~ \"txt\"",
        "f.size ~ /x/",
        "f.size !~ \"x\"",
        "f.name < 3",
        "f.name >= 3",
        "f.size == \"big\" and f.name == 3",
        "f.name == 3",
        "f.size == true",
        "f.size and f.name == \"a\"",
        "f.name or true",
        "not f.name",
        "1 in f.size",
        "f.path - 1 > 0",
        "f.name * 2 > 0",
        "f.name == f.size and f.size < f.name and 1 + f.path > 0",
        "f.modified * 2 > 0",
    ] {
        field_condition_is_refused_before_any_effect(condition);
    }
}

#[test]
fn a_field_of_a_known_type_used_as_its_type_is_accepted() {
    let world = world();
    world.put("src/a.log", "a");
    world.put("src/b.txt", "bb");
    let response = world.run(&format!(
        "each (find \"{src}\" sort: path) as f {{\n    when f.size > 1b and f.name ~ /txt/ and f.name != \"x\" {{\n        remove f.path\n    }}\n}}\n",
        src = world.at("src"),
    ));
    assert_eq!(
        response.report.status,
        ReportStatus::Ok,
        "{:?}",
        response.report.error
    );
    assert!(world.read("src/b.txt").is_none());
    assert!(world.read("src/a.log").is_some());
}

#[test]
fn a_field_without_a_declared_type_is_checked_when_the_condition_is_evaluated() {
    let world = world();
    world.put("a.json", "{\"v\": 1}");
    let response = world.run(&format!(
        "each (read \"{src}\" key: .v |> first 1) as r {{\n    when r.value ~ \"x\" {{\n        find \"/\"\n    }}\n}}\n",
        src = world.at("a.json"),
    ));
    let error = response.report.error.as_ref();
    assert!(
        error.is_none_or(|error| !error.message.contains("takes text")),
        "{error:?}"
    );
}

#[test]
fn a_stage_that_may_affect_and_takes_its_action_from_a_value_is_refused_in_a_condition() {
    for stage in [
        "git.branch repo: \"/tmp\" delete: @a",
        "sql \"/tmp/x.db\" \"select 1\" write: @w",
        "http.request \"https://example.invalid/\" method: @a",
    ] {
        let world = world();
        let response = world.run(&format!(
            "params {{\n    a: text = \"x\"\n    w: bool = true\n}}\nwhen ((({stage}) |> count)) > 0 {{\n    find \"/\"\n}}\n",
        ));
        assert_eq!(
            code(&response),
            Some(ErrorCode::Type),
            "{stage}: {:?}",
            response.report.error
        );
    }
}

#[test]
fn parallel_treats_a_path_and_its_descendant_as_overlapping_between_other_names() {
    let world = world();
    for name in ["1", "2", "3"] {
        world.put(&format!("src/{name}.log"), name);
    }
    world.put("t/a/old", "o");
    world.put("t/a.log", "l");
    let response = world.run(&format!(
        "each (find \"{src}\" glob: \"*.log\" sort: path) as f parallel: 3 {{\n    when f.name == \"1.log\" {{\n        remove \"{a}\"\n    }}\n    when f.name == \"2.log\" {{\n        write \"{log}\" \"n\" mode: overwrite\n    }}\n    when f.name == \"3.log\" {{\n        write \"{x}\" \"n\" mode: overwrite\n    }}\n}}\n",
        src = world.at("src"),
        a = world.at("t/a"),
        log = world.at("t/a.log"),
        x = world.at("t/a/x"),
    ));
    let note = response
        .report
        .parts
        .iter()
        .find(|part| part.kind == "loop")
        .and_then(|part| part.note.clone())
        .unwrap_or_default();
    assert!(
        note.contains("overlap"),
        "{note}\n{:?}",
        response.report.error
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
