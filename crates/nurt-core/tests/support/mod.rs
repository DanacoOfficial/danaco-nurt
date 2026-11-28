// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-core/tests/support/mod.rs
// Responsibility:   Test stages, front end and engine set-up shared by the end-to-end tests.
// Dependencies:     nurt_contracts, nurt_testkit, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Test stages, front end and engine set-up shared by the end-to-end tests.

// Every test crate uses a part of the items.
#![allow(dead_code)]
// Tests stop at the first unexpected value by unwrapping.
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::sync::Arc;

use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::config::Config;
use nurt_contracts::envelope::Language;
use nurt_contracts::envelope::{Request, RequestOptions, Response, Verbosity};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{FileOp, MutationTx, apply_ops};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_core::config::ConfigSources;
use nurt_core::frontend::{Compiled, Corrected, Frontend};
use nurt_core::{Engine, EngineConfig};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use nurt_testkit::journal::RecordingJournal;
use nurt_testkit::vfs::{VirtualFs, test_path};
use serde_json::json;

pub(crate) const FILES: [(&str, i64); 3] = [
    ("/data/a.log", 10),
    ("/data/b.log", 200),
    ("/data/c.log", 300),
];

/// `find`: one `entry` record for every file of the virtual file system under
/// the directory `/data`.
pub(crate) struct Find;

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
            let abs = AbsPath::for_test(path);
            let size = fs
                .metadata(&abs, nurt_contracts::fs::Follow::NoFollow)
                .ok()
                .map(|stat| i64::try_from(stat.size).unwrap_or(i64::MAX));
            if let Some(size) = size {
                records.push(
                    Record::new(RecordKind::Entry)
                        .with("path", Value::Path(abs.to_language_string()))
                        .with("size", Value::Int(size)),
                );
            }
        }
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

/// `remove`: deletes the files of the input records.
pub(crate) struct Remove;

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
            .ok_or_else(|| Error::new(ErrorCode::Internal, "the plan has no targets"))?;
        apply_ops(tx, targets.into_iter().map(FileOp::remove).collect())?;
        let effect = Record::new(RecordKind::Effect)
            .with("status", Value::Enum("ok".to_owned()))
            .with("kind", Value::Text("remove".to_owned()));
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(effect)));
        outcome.changes = plan.changes;
        Ok(outcome)
    }
}

/// Front end that binds the program texts it was given to fixed trees.
#[derive(Default)]
pub(crate) struct Programs(HashMap<String, BoundDocument>);

impl Programs {
    pub(crate) fn with(mut self, text: &str, bound: BoundDocument) -> Self {
        self.0.insert(text.to_owned(), bound);
        self
    }
}

fn no_language() -> Error {
    Error::new(ErrorCode::Unsupported, "the test front end only compiles")
}

impl Frontend for Programs {
    fn parse(&self, _: &str, _: Option<SyntaxVariant>, _: &Config) -> Result<AstDocument> {
        Err(no_language())
    }

    fn correct(&self, _: &str, _: Option<SyntaxVariant>, _: &Config) -> Result<Corrected> {
        Err(no_language())
    }

    fn bind(&self, _: &AstDocument, _: &Config) -> Result<BoundDocument> {
        Err(no_language())
    }

    fn print(&self, _: &AstDocument, _: SyntaxVariant) -> Result<String> {
        Err(no_language())
    }

    fn describe(&self, _: &str, _: Language) -> Result<serde_json::Value> {
        Err(no_language())
    }

    fn help(&self, _: &str, _: Language) -> Result<String> {
        Err(no_language())
    }

    fn compile(&self, text: &str, _: Option<SyntaxVariant>, _: &Config) -> Result<Compiled> {
        let bound =
            self.0.get(text).cloned().ok_or_else(|| {
                Error::new(ErrorCode::Syntax, format!("unknown program `{text}`"))
            })?;
        Ok(Compiled {
            bound,
            corrections: Vec::new(),
            ran: None,
            ast_hash: Some(PROGRAM_HASH.to_owned()),
        })
    }
}

pub(crate) const PROGRAM_HASH: &str =
    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

pub(crate) fn find_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "find", "effect": "read", "phase": "source",
        "args": [{"name": "path", "value": {"t": "path", "v": nurt_testkit::paths::abs("/data")}}]
    })
}

pub(crate) fn filter_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "filter", "effect": "read", "phase": "transform",
        "args": [{"name": "condition", "value": {
            "t": "bin", "op": ">",
            "l": {"t": "field", "name": "size"},
            "r": {"t": "int", "v": 150}
        }}]
    })
}

pub(crate) fn count_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "count", "effect": "read", "phase": "transform", "args": []
    })
}

pub(crate) fn remove_stage() -> serde_json::Value {
    json!({
        "t": "stage", "name": "remove", "effect": "local", "phase": "mutate", "args": []
    })
}

pub(crate) fn program(head: serde_json::Value, stages: Vec<serde_json::Value>) -> BoundDocument {
    serde_json::from_value(json!({
        "nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
        "program": {"statements": [{"t": "pipeline", "head": head, "stages": stages}]}
    }))
    .unwrap()
}

pub(crate) struct World {
    pub(crate) engine: Engine,
    pub(crate) vfs: Arc<VirtualFs>,
    pub(crate) journal: Arc<RecordingJournal>,
}

/// Programs shared by the tests of the engine.
pub(crate) fn standard_programs() -> Programs {
    Programs::default()
        .with(
            "find /data |> filter size > 150 |> count",
            program(find_stage(), vec![filter_stage(), count_stage()]),
        )
        .with(
            "find /data |> remove",
            program(find_stage(), vec![remove_stage()]),
        )
        .with(
            "find /data |> filter size > 150 |> remove",
            program(find_stage(), vec![filter_stage(), remove_stage()]),
        )
        .with("find /data", program(find_stage(), Vec::new()))
}

pub(crate) fn world() -> World {
    world_with(standard_programs())
}

/// Engine on a virtual file system holding the three files of `FILES`.
pub(crate) fn world_with(frontend: Programs) -> World {
    let clock = Arc::new(VirtualClock::new());
    let vfs = Arc::new(VirtualFs::new(clock.clone()));
    vfs.create_dir_all(&test_path("/data").unwrap()).unwrap();
    for (path, size) in FILES {
        let content = vec![b'x'; usize::try_from(size).unwrap()];
        vfs.write_file(&test_path(path).unwrap(), &content).unwrap();
    }
    let journal = Arc::new(RecordingJournal::keeping(vfs.clone()));
    let engine = Engine::open(
        EngineConfig::new(clock, Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir()
            .with_fs(vfs.clone())
            .with_journal(journal.clone())
            .with_frontend(Arc::new(frontend))
            .with_stage(Arc::new(Find))
            .with_stage(Arc::new(Remove)),
    )
    .unwrap();
    World {
        engine,
        vfs,
        journal,
    }
}

pub(crate) fn exists(world: &World, path: &str) -> bool {
    world.vfs.exists(&test_path(path).unwrap())
}

pub(crate) fn options(preview: bool) -> RequestOptions {
    RequestOptions {
        preview: Some(preview),
        verbosity: Some(Verbosity::Detailed),
        ..RequestOptions::default()
    }
}

pub(crate) fn run(world: &World, text: &str, preview: bool) -> Response {
    let response = world
        .engine
        .run(&Request::program(text).with_options(options(preview)));
    response.validate().unwrap();
    response
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
