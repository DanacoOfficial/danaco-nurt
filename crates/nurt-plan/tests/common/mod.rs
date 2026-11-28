// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-plan/tests/common/mod.rs
// Responsibility:   Builders of bound programs, the golden files and a fixed host shared by the integration tests.
// Dependencies:     nurt_catalog, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Builders of bound programs, the golden files and a fixed host shared by
//! the integration tests.

#![allow(
    dead_code,
    reason = "each test binary uses its own part of the shared helpers"
)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail the test by panicking"
)]

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;

use nurt_catalog::Registry;
use nurt_contracts::bound::{
    BoundArg, BoundDocument, BoundExpr, BoundHead, BoundLet, BoundPipeline, BoundProgram, BoundRef,
    BoundStage, BoundStatement, Phase,
};
use nurt_contracts::env::{Host, HostInfo, Os, PathStyle};
use nurt_contracts::kinds::EffectKind;
use nurt_plan::{PipelinePlan, Plan, PlanStatement};

/// Directory of the golden bound trees.
pub(crate) fn golden_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../spec/golden/bound")
}

/// Reads a golden bound tree by its relative name without `.json`.
pub(crate) fn golden(name: &str) -> BoundDocument {
    let path = golden_dir().join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

pub(crate) fn registry() -> Registry {
    Registry::load_embedded().expect("the embedded catalog loads")
}

pub(crate) fn stage(
    name: &str,
    effect: EffectKind,
    phase: Phase,
    args: Vec<BoundArg>,
) -> BoundStage {
    BoundStage {
        name: name.to_owned(),
        effect,
        phase,
        waits: false,
        args,
        span: None,
    }
}

pub(crate) fn source(name: &str, args: Vec<BoundArg>) -> BoundStage {
    stage(name, EffectKind::Read, Phase::Source, args)
}

pub(crate) fn transform(name: &str, args: Vec<BoundArg>) -> BoundStage {
    stage(name, EffectKind::Read, Phase::Transform, args)
}

pub(crate) fn checking(name: &str, args: Vec<BoundArg>) -> BoundStage {
    stage(name, EffectKind::Read, Phase::Check, args)
}

pub(crate) fn mutate(name: &str, args: Vec<BoundArg>) -> BoundStage {
    stage(name, EffectKind::Local, Phase::Mutate, args)
}

pub(crate) fn after(name: &str, args: Vec<BoundArg>) -> BoundStage {
    stage(name, EffectKind::Read, Phase::After, args)
}

pub(crate) fn run(command: &[&str]) -> BoundStage {
    let items = command.iter().map(|part| text(part)).collect();
    stage(
        "run",
        EffectKind::Process,
        Phase::Run,
        vec![given("command", BoundExpr::List { items })],
    )
}

pub(crate) fn verify(check: BoundPipeline) -> BoundStage {
    stage(
        "verify",
        EffectKind::Read,
        Phase::Verify,
        vec![given("check", BoundExpr::Sub { p: check })],
    )
}

pub(crate) fn find(dir: &str) -> BoundStage {
    source("find", vec![given("path", path(dir))])
}

pub(crate) fn pipeline(head: BoundStage, stages: Vec<BoundStage>) -> BoundPipeline {
    BoundPipeline {
        head: BoundHead::Stage(head),
        stages,
        span: None,
    }
}

pub(crate) fn from_ref(name: &str, stages: Vec<BoundStage>) -> BoundPipeline {
    BoundPipeline {
        head: BoundHead::Ref(BoundRef {
            name: name.to_owned(),
            fields: Vec::new(),
        }),
        stages,
        span: None,
    }
}

pub(crate) fn operation(pipeline: BoundPipeline) -> BoundStatement {
    BoundStatement::Pipeline(pipeline)
}

pub(crate) fn binding(name: &str, value: BoundPipeline) -> BoundStatement {
    BoundStatement::Let(BoundLet {
        name: name.to_owned(),
        value,
        span: None,
    })
}

pub(crate) fn document(statements: Vec<BoundStatement>) -> BoundDocument {
    BoundDocument::new("cat-1", "corr-1", BoundProgram { statements })
}

pub(crate) fn single(pipeline: BoundPipeline) -> BoundDocument {
    document(vec![operation(pipeline)])
}

pub(crate) fn given(name: &str, value: BoundExpr) -> BoundArg {
    BoundArg::given(name, value)
}

pub(crate) fn path(value: &str) -> BoundExpr {
    BoundExpr::Path {
        v: value.to_owned(),
    }
}

pub(crate) fn text(value: &str) -> BoundExpr {
    BoundExpr::Text {
        v: value.to_owned(),
    }
}

pub(crate) fn int(value: i64) -> BoundExpr {
    BoundExpr::Int { v: value }
}

pub(crate) fn field(name: &str) -> BoundExpr {
    BoundExpr::Field {
        name: name.to_owned(),
        of: None,
    }
}

pub(crate) fn range(lo: i64, hi: i64) -> BoundExpr {
    BoundExpr::Range {
        lo: Some(lo),
        hi: Some(hi),
    }
}

pub(crate) fn greater(name: &str, value: BoundExpr) -> BoundExpr {
    BoundExpr::Bin {
        op: nurt_contracts::ast::BinOp::Gt,
        l: Box::new(field(name)),
        r: Box::new(value),
    }
}

/// Steps and commands of a pipeline, for example `2:find 4:remove`; the
/// pipeline of `verify` follows in braces.
pub(crate) fn outline(pipeline: &PipelinePlan) -> String {
    let mut parts = Vec::new();
    if let Some(head) = &pipeline.head {
        parts.push(format!("@{}", head.name));
    }
    for planned in &pipeline.stages {
        let mut part = format!("{}:{}", planned.step.as_str(), planned.command());
        if let Some(check) = planned.check.as_deref() {
            write!(part, "{{{}}}", outline(check)).unwrap();
        }
        parts.push(part);
    }
    parts.join(" ")
}

/// Outline of every statement of a plan; named results start with
/// `let NAME = `.
pub(crate) fn plan_outline(plan: &Plan) -> Vec<String> {
    plan.statements
        .iter()
        .map(|statement| match statement {
            PlanStatement::Let(binding) => {
                format!("let {} = {}", binding.name, outline(&binding.pipeline))
            }
            PlanStatement::Construct(construct) => format!("construct {}", construct.name()),
            other => other.pipeline().map_or_else(String::new, outline),
        })
        .collect()
}

/// Host with fixed facts and an empty environment.
pub(crate) struct FixedHost(HostInfo);

impl Host for FixedHost {
    fn info(&self) -> &HostInfo {
        &self.0
    }

    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

pub(crate) fn host() -> FixedHost {
    FixedHost(HostInfo {
        os: Os::Linux,
        os_version: "6.8.0".to_owned(),
        architecture: "x86_64".to_owned(),
        hostname: "build".to_owned(),
        user: "user".to_owned(),
        home: PathBuf::from("/home/user"),
        temp: PathBuf::from("/tmp"),
        path_style: PathStyle::Posix,
        timezone: "UTC".to_owned(),
        locale: None,
        cpus: 2,
    })
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
