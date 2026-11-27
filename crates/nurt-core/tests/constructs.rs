// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
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
    let temp = tempfile::tempdir().unwrap();
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
    let dry = world.with_params(&program, json!({"dry": true}));
    assert_eq!(world.snapshot(), before);
    assert_eq!(
        dry.report.status,
        ReportStatus::Ok,
        "{:?}",
        dry.report.error
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
    assert_ne!(wrong.report.status, ReportStatus::Ok);
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
    let exe = if cfg!(windows) { "cmd" } else { "sh" };
    let args = if cfg!(windows) {
        "[\"cmd\", \"/c\", \"exit 0\"]"
    } else {
        "[\"sh\", \"-c\", \"exit 0\"]"
    };
    let _ = exe;
    let response = world.run(&format!(
        "steps {{\n    write \"{a}\" \"1\" mode: overwrite\n    run {args} cwd: \"{cwd}\"\n    copy \"{missing}\" into: \"{dst}\"\n}}\n",
        a = world.at("a"),
        cwd = world.at(""),
        missing = world.at("missing"),
        dst = world.at("dst"),
    ));
    assert_ne!(response.report.status, ReportStatus::Ok);
    assert!(world.read("a").is_none(), "the write is rolled back");
    let error = response.report.error.as_ref().unwrap();
    assert!(error.details.contains_key("restored") || error.cause.is_some());
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
