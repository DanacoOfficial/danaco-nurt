// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Engine over a virtual file system for the pipeline tests of the
//! interfaces.
//!
//! The world holds the files `/data/a.log`, `/data/b.log` and `/data/c.log`
//! of 10, 200 and 300 bytes, a journal that keeps the committed operations
//! and a language front end that knows the programs of [`PROGRAMS`]. The
//! stages `find` and `remove` work on the virtual file system; the record
//! stages are those of the engine. The module is compiled with the Cargo
//! feature `e2e`. With the environment variable [`ENGINE_ENV`] set,
//! `nurt_engine_open` returns this engine instead of the engine of the
//! process.

use std::sync::Arc;

use nurt_contracts::bound::BoundDocument;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Follow};
use nurt_contracts::journal::{FileOp, MutationTx, apply_ops};
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_core::config::ConfigSources;
use nurt_core::fixtures::FixtureFrontend;
use nurt_core::{Engine, EngineConfig};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use nurt_testkit::journal::RecordingJournal;
use nurt_testkit::vfs::{VirtualFs, test_path};
use serde_json::json;

#[cfg(not(feature = "stub"))]
use crate::engine_backend::EngineBackend;
#[cfg(not(feature = "stub"))]
use crate::error::CallError;

/// Environment variable that selects this engine in `nurt_engine_open`.
pub const ENGINE_ENV: &str = "NURT_E2E_ENGINE";

/// Directory searched by the stage `find`.
pub const DATA_DIR: &str = "/data";

/// Files of the world and their sizes in bytes.
pub const FILES: [(&str, i64); 3] = [
    ("/data/a.log", 10),
    ("/data/b.log", 200),
    ("/data/c.log", 300),
];

/// Program that lists the files.
pub const LIST: &str = "find /data";
/// Program that counts the files larger than 150 bytes.
pub const COUNT_LARGE: &str = "find /data |> filter size > 150 |> count";
/// Program that removes every file.
pub const REMOVE_ALL: &str = "find /data |> remove";
/// Program that removes the files larger than 150 bytes.
pub const REMOVE_LARGE: &str = "find /data |> filter size > 150 |> remove";

/// Programs known to the front end of the world.
pub const PROGRAMS: [&str; 4] = [LIST, COUNT_LARGE, REMOVE_ALL, REMOVE_LARGE];

/// Engine together with the components it runs on.
pub struct World {
    /// The engine.
    pub engine: Engine,
    /// Virtual file system of the engine.
    pub fs: Arc<VirtualFs>,
    /// Journal of the engine.
    pub journal: Arc<RecordingJournal>,
    /// Clock of the engine.
    pub clock: Arc<VirtualClock>,
}

impl World {
    /// Whether the virtual file system holds the file `path`.
    #[must_use]
    pub fn exists(&self, path: &str) -> bool {
        test_path(path).is_ok_and(|path| self.fs.exists(&path))
    }
}

/// Opens an engine over a fresh virtual file system with [`FILES`].
///
/// # Errors
///
/// Returns the error of the file system or of the engine.
pub fn world() -> Result<World> {
    let clock = Arc::new(VirtualClock::new());
    let fs = Arc::new(VirtualFs::new(clock.clone()));
    fs.create_dir_all(&test_path(DATA_DIR)?)?;
    for (path, size) in FILES {
        let length = usize::try_from(size).map_err(|_| internal("negative file size"))?;
        fs.write_file(&test_path(path)?, &vec![b'x'; length])?;
    }
    let journal = Arc::new(RecordingJournal::keeping(fs.clone()));
    let engine = Engine::open(
        EngineConfig::new(clock.clone(), Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir()
            .with_fs(fs.clone())
            .with_journal(journal.clone())
            .with_frontend(Arc::new(frontend()?))
            .with_stage(Arc::new(Find))
            .with_stage(Arc::new(Remove)),
    )
    .map_err(|error| error.to_error())?;
    Ok(World {
        engine,
        fs,
        journal,
        clock,
    })
}

/// Opens the backend of the world for `nurt_engine_open`.
#[cfg(not(feature = "stub"))]
pub(crate) fn open() -> std::result::Result<EngineBackend, CallError> {
    let World { engine, clock, .. } =
        world().map_err(|error| CallError::internal(error.to_string()))?;
    Ok(EngineBackend::new(engine, clock))
}

fn internal(message: &str) -> Error {
    Error::new(ErrorCode::Internal, message)
}

fn frontend() -> Result<FixtureFrontend> {
    Ok(FixtureFrontend::new()
        .with_bound(LIST, program(find_stage(), Vec::new())?)
        .with_bound(
            COUNT_LARGE,
            program(find_stage(), vec![filter_stage(), count_stage()])?,
        )
        .with_bound(REMOVE_ALL, program(find_stage(), vec![remove_stage()])?)
        .with_bound(
            REMOVE_LARGE,
            program(find_stage(), vec![filter_stage(), remove_stage()])?,
        ))
}

fn find_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "find", "effect": "read", "phase": "source",
        "args": [{"name": "path", "value": {"t": "path", "v": DATA_DIR}}]
    })
}

fn filter_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "filter", "effect": "read", "phase": "transform",
        "args": [{"name": "condition", "value": {
            "t": "bin", "op": ">",
            "l": {"t": "field", "name": "size"},
            "r": {"t": "int", "v": 150}
        }}]
    })
}

fn count_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "count", "effect": "read", "phase": "transform", "args": []
    })
}

fn remove_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "remove", "effect": "local", "phase": "mutate", "args": []
    })
}

fn program(head: serde_json::Value, stages: Vec<serde_json::Value>) -> Result<BoundDocument> {
    serde_json::from_value(json!({
        "nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
        "program": {"statements": [{"t": "pipeline", "head": head, "stages": stages}]}
    }))
    .map_err(|error| internal(&format!("invalid program tree: {error}")))
}

/// Stage `find`: one `entry` record for every file of [`FILES`] that exists.
struct Find;

impl StageImpl for Find {
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

impl ReadStage for Find {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let fs = ctx.require_fs()?;
        let mut records = Vec::new();
        for (path, _) in FILES {
            let abs = AbsPath::new(path)?;
            if let Ok(stat) = fs.metadata(&abs, Follow::NoFollow) {
                records.push(
                    Record::new(RecordKind::Entry)
                        .with("path", Value::Path(path.to_owned()))
                        .with(
                            "size",
                            Value::Int(i64::try_from(stat.size).unwrap_or(i64::MAX)),
                        ),
                );
            }
        }
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

/// Stage `remove`: deletes the files of the input records.
struct Remove;

struct Targets(Vec<AbsPath>);

impl StageImpl for Remove {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "remove".to_owned(),
            source: false,
            input: InputKinds::Only(vec![RecordKind::Entry]),
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Remove {
    fn plan<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let mut targets = Vec::new();
        let mut changes = Vec::new();
        for item in input.into_records()? {
            let record = item?;
            let text = record
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            targets.push(AbsPath::new(&text)?);
            changes.push(Change::new(text.as_str(), ChangeAction::Delete));
        }
        let mut plan = EffectPlan::new();
        plan.bulk = targets.len() > 1;
        plan.changes = changes;
        Ok(plan.with_state(Targets(targets)))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let Targets(targets) = plan
            .take_state::<Targets>()
            .ok_or_else(|| internal("the plan has no targets"))?;
        apply_ops(tx, targets.into_iter().map(FileOp::remove).collect())?;
        let effect = Record::new(RecordKind::Effect)
            .with("status", Value::Enum("ok".to_owned()))
            .with("kind", Value::Text("remove".to_owned()));
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(effect)));
        outcome.changes = plan.changes;
        Ok(outcome)
    }
}
