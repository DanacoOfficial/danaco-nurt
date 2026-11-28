// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-core/tests/elevation_journal.rs
// Responsibility:   One journal for the entries of the account and of the elevated operations.
// Dependencies:     nurt_contracts, nurt_fs, nurt_testkit, serde_json, support
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! One journal for the entries of the account and of the elevated operations:
//! one counter, history over both kinds, `undo` of each entry by its number.
//!
//! The process plays the administrator (`with_administrator`), so the
//! backend `self` runs the elevated calls in the same process; the journal is
//! the real one, on the real file system of a temporary directory.

// Tests stop at the first unexpected value by unwrapping or panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::Path;
use std::sync::{Arc, Mutex};

use nurt_contracts::config::Config;
use nurt_contracts::envelope::{Language, Privilege, Request, RequestOptions, Response};
use nurt_contracts::error::Result;
use nurt_contracts::journal::HistoryQuery;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::record::OperationState;
use nurt_contracts::report::ReportStatus;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_core::config::ConfigSources;
use nurt_core::{Engine, EngineConfig};
use nurt_testkit::host::TestHost;
use serde_json::json;
use support::{Programs, Remove, program, remove_stage};

/// `find PATH`: one `entry` record for the file at `PATH`, when it exists.
struct FindOne;

impl StageImpl for FindOne {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "find".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for FindOne {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let mut records = Vec::new();
        if let Some(Value::Path(path)) = args.value("path") {
            let abs = nurt_contracts::fs::AbsPath::new(path.as_str())?;
            if ctx
                .require_fs()?
                .metadata(&abs, nurt_contracts::fs::Follow::NoFollow)
                .is_ok()
            {
                records.push(
                    Record::new(RecordKind::Entry)
                        .with("path", Value::Path(path.clone()))
                        .with("size", Value::Int(1)),
                );
            }
        }
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

fn find_at(path: &Path) -> serde_json::Value {
    json!({
        "t": "stage", "name": "find", "effect": "read", "phase": "source",
        "args": [{"name": "path", "value": {"t": "path", "v": path.to_string_lossy()}}]
    })
}

fn numbered_stage(name: &str, id: i64) -> serde_json::Value {
    json!({
        "t": "stage", "name": name, "effect": "local", "phase": "mutate",
        "args": [
            {"name": "id", "value": {"t": "int", "v": id}},
            {"name": "to", "value": {"t": "null"}, "default": true},
            {"name": "file", "value": {"t": "null"}, "default": true},
            {"name": "since", "value": {"t": "null"}, "default": true},
            {"name": "paths", "value": {"t": "null"}, "default": true},
            {"name": "all_sessions", "value": {"t": "bool", "v": false}, "default": true},
            {"name": "on_conflict", "value": {"t": "enum", "v": "report"}, "default": true}
        ]
    })
}

/// `undo` or `redo` without a number: every selector keeps its default.
fn bare_stage(name: &str) -> serde_json::Value {
    json!({
        "t": "stage", "name": name, "effect": "local", "phase": "mutate",
        "args": [
            {"name": "id", "value": {"t": "null"}, "default": true},
            {"name": "to", "value": {"t": "null"}, "default": true},
            {"name": "file", "value": {"t": "null"}, "default": true},
            {"name": "since", "value": {"t": "null"}, "default": true},
            {"name": "paths", "value": {"t": "null"}, "default": true},
            {"name": "all_sessions", "value": {"t": "bool", "v": false}, "default": true},
            {"name": "on_conflict", "value": {"t": "enum", "v": "report"}, "default": true}
        ]
    })
}

/// Distinguishes the journal directory of every test.
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct World {
    engine: Engine,
    dir: tempfile::TempDir,
    programs: Arc<Mutex<Programs>>,
    journal_name: String,
}

impl Drop for World {
    fn drop(&mut self) {
        // The journal directory is created in the first writable ancestor of
        // the state directory; the name is private to this test.
        for ancestor in self.dir.path().ancestors() {
            let _ = std::fs::remove_dir_all(ancestor.join(&self.journal_name));
        }
    }
}

impl World {
    fn file(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join(name)
    }

    /// Runs `undo ID`; the number is known only after the operations ran.
    fn undo(&self, id: u64) -> Response {
        self.numbered("undo", id)
    }

    /// Runs `redo ID` for the entry of an undo.
    fn redo(&self, id: u64) -> Response {
        self.numbered("redo", id)
    }

    fn numbered(&self, name: &str, id: u64) -> Response {
        let text = format!("{name} {id}");
        {
            let mut programs = self.programs.lock().unwrap();
            let known = std::mem::take(&mut *programs);
            *programs = known.with(
                &text,
                program(numbered_stage(name, i64::try_from(id).unwrap()), Vec::new()),
            );
        }
        self.run(&text, None)
    }

    /// Runs `undo` or `redo` without a number.
    fn bare(&self, name: &str) -> Response {
        {
            let mut programs = self.programs.lock().unwrap();
            let known = std::mem::take(&mut *programs);
            *programs = known.with(name, program(bare_stage(name), Vec::new()));
        }
        self.run(name, None)
    }

    fn run(&self, text: &str, privilege: Option<Privilege>) -> Response {
        let options = RequestOptions {
            privilege,
            ..RequestOptions::default()
        };
        let response = self
            .engine
            .run(&Request::program(text).with_options(options));
        response.validate().unwrap();
        response
    }
}

struct NoCorrection(Arc<Mutex<Programs>>);

impl nurt_core::frontend::Frontend for NoCorrection {
    fn parse(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<nurt_contracts::ast::AstDocument> {
        self.0.lock().unwrap().parse(text, variant, config)
    }
    fn correct(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<nurt_core::frontend::Corrected> {
        self.0.lock().unwrap().correct(text, variant, config)
    }
    fn bind(
        &self,
        document: &nurt_contracts::ast::AstDocument,
        config: &Config,
    ) -> Result<nurt_contracts::bound::BoundDocument> {
        self.0.lock().unwrap().bind(document, config)
    }
    fn print(
        &self,
        document: &nurt_contracts::ast::AstDocument,
        variant: SyntaxVariant,
    ) -> Result<String> {
        self.0.lock().unwrap().print(document, variant)
    }
    fn describe(&self, subject: &str, language: Language) -> Result<serde_json::Value> {
        self.0.lock().unwrap().describe(subject, language)
    }
    fn help(&self, topic: &str, language: Language) -> Result<String> {
        self.0.lock().unwrap().help(topic, language)
    }
    fn compile(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<nurt_core::frontend::Compiled> {
        self.0.lock().unwrap().compile(text, variant, config)
    }
}

fn world() -> World {
    // The journal of a volume is shared by every process of the account on
    // it and numbers its operations per journal directory, so each test keeps
    // a journal directory of its own and removes it at the end. The space
    // reserve is small, so that a disk that other work fills below the default
    // 15 % does not make the journal skip the preservation of a file.
    let journal_name = format!(
        ".nurt-journal-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let embedded = nurt_core::config::toml_table(
        &format!("[journal]\ndir_name = \"{journal_name}\"\n[space]\nreserve = \"1mib\"\n"),
        "test",
    )
    .unwrap();
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    for name in ["x", "y", "z"] {
        std::fs::write(dir.path().join(name), name).unwrap();
    }
    let mut programs = Programs::default();
    for name in ["x", "y", "z"] {
        programs = programs.with(
            &format!("rm {name}"),
            program(find_at(&dir.path().join(name)), vec![remove_stage()]),
        );
    }
    let programs = Arc::new(Mutex::new(programs));
    let config = EngineConfig::new(
        Arc::new(nurt_core::system::SystemClock::new()),
        Arc::new(TestHost::linux()),
    )
    .with_sources(ConfigSources::isolated().with_embedded(embedded))
    .with_state_dir(dir.path().join("state"))
    .with_system_journal()
    .with_fs(nurt_fs::system_fs())
    .with_frontend(Arc::new(NoCorrection(programs.clone())))
    .with_stage(Arc::new(FindOne))
    .with_stage(Arc::new(Remove))
    .with_administrator(true)
    .with_elevation_executable("/opt/nurt/bin/nurt");
    World {
        engine: Engine::open(config).unwrap(),
        dir,
        programs,
        journal_name,
    }
}

fn history(world: &World) -> Vec<nurt_contracts::journal::OperationRecord> {
    world.engine.history(&HistoryQuery::default()).unwrap()
}

#[test]
fn account_and_elevated_entries_share_one_counter() {
    let world = world();
    let first = world.run("rm x", None);
    let second = world.run("rm y", Some(Privilege::Elevated));
    let third = world.run("rm z", None);
    for response in [&first, &second, &third] {
        assert_eq!(response.report.status, ReportStatus::Ok, "{response:?}");
    }
    let base = first.report.op.unwrap();
    let numbers = [first.report.op, second.report.op, third.report.op];
    assert_eq!(
        numbers,
        [Some(base), Some(base + 1), Some(base + 2)],
        "one counter, no gaps"
    );
    assert!(second.report.elevated);
    assert_eq!(second.report.backend.as_deref(), Some("self"));
    assert!(!first.report.elevated && !third.report.elevated);
}

#[test]
fn history_lists_the_elevated_entry_between_the_account_entries() {
    let world = world();
    let _ = world.run("rm x", None);
    let _ = world.run("rm y", Some(Privilege::Elevated));
    let _ = world.run("rm z", None);
    let entries = history(&world);
    let newest: Vec<_> = entries.iter().take(3).collect();
    let ids: Vec<u64> = newest.iter().map(|record| record.id).collect();
    assert_eq!(ids, [ids[2] + 2, ids[2] + 1, ids[2]], "newest first");
    let actions: Vec<Option<&str>> = newest
        .iter()
        .map(|record| record.action.as_deref())
        .collect();
    assert_eq!(actions, [None, Some("elevated:self"), None]);
}

#[test]
fn each_entry_is_undone_by_its_own_number_over_a_mixed_journal() {
    let world = world();
    let x = world.run("rm x", None).report.op.unwrap();
    let y = world
        .run("rm y", Some(Privilege::Elevated))
        .report
        .op
        .unwrap();
    let z = world.run("rm z", None).report.op.unwrap();
    assert!(!world.file("x").exists() && !world.file("y").exists() && !world.file("z").exists());

    let undone = world.undo(y);
    assert_eq!(undone.report.status, ReportStatus::Ok, "{undone:?}");
    assert!(
        undone.report.elevated,
        "the elevated entry is restored through the backend"
    );
    assert!(world.file("y").exists() && !world.file("x").exists() && !world.file("z").exists());

    let undone = world.undo(z);
    assert_eq!(undone.report.status, ReportStatus::Ok, "{undone:?}");
    assert!(
        !undone.report.elevated,
        "an entry of the account stays with the account"
    );
    assert!(world.file("z").exists() && !world.file("x").exists());

    let undone = world.undo(x);
    assert_eq!(undone.report.status, ReportStatus::Ok, "{undone:?}");
    assert!(world.file("x").exists());

    let states: Vec<(u64, OperationState)> = history(&world)
        .iter()
        .filter(|record| [x, y, z].contains(&record.id))
        .map(|record| (record.id, record.state))
        .collect();
    assert!(
        states
            .iter()
            .all(|(_, state)| *state == OperationState::Undone),
        "{states:?}"
    );
}

#[test]
fn bare_undo_of_an_elevated_entry_is_refused_and_names_the_next_step() {
    let world = world();
    let y = world
        .run("rm y", Some(Privilege::Elevated))
        .report
        .op
        .unwrap();
    let refused = world.bare("undo");
    assert_eq!(refused.report.status, ReportStatus::Failed, "{refused:?}");
    let error = refused.report.error.as_ref().unwrap();
    assert_eq!(error.code(), nurt_contracts::error::ErrorCode::Privilege);
    assert_eq!(error.next_steps, [format!("undo {y} privilege: elevated")]);
    assert!(!world.file("y").exists(), "nothing was restored");
    let undone = world.undo(y);
    assert_eq!(undone.report.status, ReportStatus::Ok, "{undone:?}");
    assert!(world.file("y").exists());
}

#[test]
fn bare_undo_of_an_entry_of_the_account_is_not_refused() {
    let world = world();
    let _ = world.run("rm y", Some(Privilege::Elevated));
    let z = world.run("rm z", None).report.op.unwrap();
    let undone = world.bare("undo");
    assert_eq!(undone.report.status, ReportStatus::Ok, "{undone:?}");
    assert!(!undone.report.elevated);
    assert!(world.file("z").exists() && !world.file("y").exists());
    assert!(
        history(&world)
            .iter()
            .any(|record| record.id == z && record.state == OperationState::Undone)
    );
}

#[test]
fn bare_redo_of_the_undo_of_an_elevated_entry_is_refused() {
    let world = world();
    let y = world
        .run("rm y", Some(Privilege::Elevated))
        .report
        .op
        .unwrap();
    let undone = world.undo(y);
    assert_eq!(undone.report.status, ReportStatus::Ok, "{undone:?}");
    let undo_entry = history(&world)
        .iter()
        .find(|record| record.kind == "undo" && record.undo_of == [y])
        .map(|record| record.id)
        .unwrap();
    let refused = world.bare("redo");
    assert_eq!(refused.report.status, ReportStatus::Failed, "{refused:?}");
    let error = refused.report.error.as_ref().unwrap();
    assert_eq!(error.code(), nurt_contracts::error::ErrorCode::Privilege);
    assert_eq!(
        error.next_steps,
        [format!("redo {undo_entry} privilege: elevated")]
    );
    assert!(world.file("y").exists(), "nothing was redone");
}

#[test]
fn redo_by_number_of_the_undo_of_an_elevated_entry_runs_through_the_engine() {
    let world = world();
    let y = world
        .run("rm y", Some(Privilege::Elevated))
        .report
        .op
        .unwrap();
    let undone = world.undo(y);
    assert_eq!(undone.report.status, ReportStatus::Ok, "{undone:?}");
    assert!(world.file("y").exists(), "the undo restored the file");
    let undo_entry = history(&world)
        .iter()
        .find(|record| record.kind == "undo" && record.undo_of == [y])
        .map(|record| record.id)
        .unwrap();
    let redone = world.redo(undo_entry);
    assert_eq!(redone.report.status, ReportStatus::Ok, "{redone:?}");
    assert_eq!(redone.report.elevated, undone.report.elevated);
    assert!(!world.file("y").exists(), "the redo removed the file again");
    assert!(
        history(&world)
            .iter()
            .any(|record| record.id == undo_entry && record.state == OperationState::Undone),
        "{:?}",
        history(&world)
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
