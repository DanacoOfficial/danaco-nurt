// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-plan::check
// Responsibility:   Check without effect: what a plan would touch, known before any step runs.
// Dependencies:     indexmap, nurt_catalog, nurt_contracts, schemars, serde
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Check without effect: what a plan would touch, known before any step
//! runs.
//!
//! The summary combines facts read from the literals of the bound arguments
//! (paths, network addresses, launched programs) with the facts reported by
//! the static check of every registered stage implementation
//! ([`StageImpl::check`]). Arguments computed per record are reported as
//! dynamic targets.

use std::sync::Arc;

use indexmap::IndexMap;
use nurt_catalog::Registry;
use nurt_contracts::bound::{BoundExpr, BoundStage};
use nurt_contracts::config::Config;
use nurt_contracts::env::Host;
use nurt_contracts::error::{Error, Result};
use nurt_contracts::kinds::{Category, EffectKind, Implementation, Reversibility};
use nurt_contracts::report::Correction;
use nurt_contracts::spec::{ScalarType, StageSpec};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CheckCtx, ExecStep, InputKinds, NetworkTarget, StageImpl, StaticInfo,
};
use nurt_contracts::value::{Duration, Fields, Instant, Range, RecordKind, Regex, Size, Value};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::address::parse_address;
use crate::model::{Plan, PlannedStage};

/// Name of the format of the check result.
pub const CHECK_FORMAT: &str = "nurt-check/1";

/// Program that runs batch files (`.bat`, `.cmd`) on Windows.
const BATCH_LAUNCHER: &str = "cmd.exe";

/// Extensions of batch files, compared without regard to case.
const BATCH_EXTENSIONS: &[&str] = &[".bat", ".cmd"];

/// Prefix of the commands of the network group whose address is `url`.
const HTTP_PREFIX: &str = "http.";

/// Parameter that holds the address of an HTTP request or a clone.
const URL_PARAM: &str = "url";

/// Parameter that holds the remote of a git transfer.
const REMOTE_PARAM: &str = "remote";

/// Git commands that contact a remote, with the parameter naming it.
const GIT_TRANSFERS: &[(&str, &str)] = &[
    ("git.clone", URL_PARAM),
    ("git.fetch", REMOTE_PARAM),
    ("git.pull", REMOTE_PARAM),
    ("git.push", REMOTE_PARAM),
];

/// Parameter of a running command that holds the command line.
const COMMAND_PARAM: &str = "command";

/// Where a reported target comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TargetSource {
    /// A literal of the program.
    Literal,
    /// An expression evaluated per record; the target is known only while
    /// the program runs.
    Record,
    /// A git remote resolved from the repository configuration while the
    /// program runs.
    Remote,
    /// The command catalog: a program the command always uses.
    Catalog,
    /// The static check of the stage implementation.
    Implementation,
}

/// Network endpoint a stage will contact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkUse {
    /// Command that contacts the endpoint.
    pub command: String,
    /// Parameter that holds the address; absent for targets reported by the
    /// implementation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    /// Where the target comes from.
    pub source: TargetSource,
    /// The endpoint, when known before execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<NetworkTarget>,
    /// Name of the git remote, for targets resolved from the repository;
    /// absent for the default remote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

/// External program a stage will start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProgramUse {
    /// Command that starts the program.
    pub command: String,
    /// Program as written; absent when it is computed per record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// Why the command uses an external program.
    pub implementation: Implementation,
    /// Program that runs the program on Windows, for batch files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher: Option<String>,
    /// Where the program comes from.
    pub source: TargetSource,
}

/// Facts specific to one command, reported by its static check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StageDetails {
    /// Command.
    pub command: String,
    /// Index of the stage in its pipeline.
    pub index: u32,
    /// Facts by name.
    pub details: IndexMap<String, Value>,
}

/// Result of the check without effect (`nurt-check/1`).
///
/// The check runs after binding and planning and before any execution
/// step; it neither reads nor changes the world.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CheckSummary {
    /// Format name, [`CHECK_FORMAT`].
    pub format: String,
    /// Strongest effect of the plan.
    pub effect: EffectKind,
    /// Weakest reversibility of the plan.
    pub reversibility: Reversibility,
    /// Number of operations; more than one for a script.
    pub operations: u32,
    /// A stage blocks the call by waiting.
    pub waits: bool,
    /// Paths named by literals of the program, in order of appearance.
    pub paths: Vec<String>,
    /// Network endpoints the plan will contact.
    pub network: Vec<NetworkUse>,
    /// External programs the plan will start.
    pub programs: Vec<ProgramUse>,
    /// Command groups the plan uses, in order of first use.
    pub groups: Vec<String>,
    /// Corrections applied to the program text before binding.
    pub corrections: Vec<Correction>,
    /// Facts specific to commands, from their static checks.
    pub details: Vec<StageDetails>,
}

fn push_unique<T: PartialEq>(items: &mut Vec<T>, item: T) {
    if !items.contains(&item) {
        items.push(item);
    }
}

fn launcher_for(program: &str) -> Option<String> {
    let lower = program.to_ascii_lowercase();
    BATCH_EXTENSIONS
        .iter()
        .any(|extension| lower.ends_with(extension))
        .then(|| BATCH_LAUNCHER.to_owned())
}

fn is_given(stage: &BoundStage, name: &str) -> bool {
    stage
        .args
        .iter()
        .any(|arg| arg.name == name && !arg.default && arg.value != BoundExpr::Null)
}

fn arg<'a>(stage: &'a BoundStage, name: &str) -> Option<&'a BoundExpr> {
    stage
        .args
        .iter()
        .find(|arg| arg.name == name)
        .map(|arg| &arg.value)
}

fn collect_paths(value: &BoundExpr, paths: &mut Vec<String>) {
    match value {
        BoundExpr::Path { v } => push_unique(paths, v.clone()),
        BoundExpr::List { items } => {
            for item in items {
                collect_paths(item, paths);
            }
        }
        _ => {}
    }
}

/// Value of a literal argument for the static check; expressions are left
/// for the evaluator of the executor. An instant without a zone depends on
/// the time zone of the run and stays an expression.
fn static_value(value: &BoundExpr) -> Option<Value> {
    match value {
        BoundExpr::Bool { v } => Some(Value::Bool(*v)),
        BoundExpr::Int { v } => Some(Value::Int(*v)),
        BoundExpr::Num { v } => Some(Value::Number(*v)),
        BoundExpr::Text { v } => Some(Value::Text(v.clone())),
        BoundExpr::Path { v } => Some(Value::Path(v.clone())),
        BoundExpr::Glob { v } => Some(Value::Glob(v.clone())),
        BoundExpr::Key { v } => Some(Value::Key(v.clone())),
        BoundExpr::Enum { v } => Some(Value::Enum(v.clone())),
        BoundExpr::Instant { v } => v.parse::<Instant>().ok().map(Value::Instant),
        BoundExpr::Size { v } => Some(Value::Size(Size::from_bytes(*v))),
        BoundExpr::Dur { v } => i64::try_from(*v)
            .ok()
            .map(|millis| Value::Duration(Duration::from_millis(millis))),
        BoundExpr::Regex { v, flags } => Regex::new(v, flags).ok().map(Value::Regex),
        BoundExpr::Range { lo, hi } => Range::new(*lo, *hi).ok().map(Value::Range),
        BoundExpr::List { items } => items
            .iter()
            .map(static_value)
            .collect::<Option<Vec<_>>>()
            .map(Value::List),
        BoundExpr::Map { entries } => entries
            .iter()
            .map(|entry| static_value(&entry.v).map(|value| (entry.k.clone(), value)))
            .collect::<Option<Fields>>()
            .map(Value::Map),
        _ => None,
    }
}

fn static_arg(value: &BoundExpr) -> ArgValue {
    match value {
        BoundExpr::Null => ArgValue::Absent,
        BoundExpr::Sub { p } => ArgValue::Pipeline(p.clone()),
        other => static_value(other).map_or_else(|| ArgValue::Expr(other.clone()), ArgValue::Value),
    }
}

/// Bound arguments of a stage as seen by its static check.
pub(crate) fn static_args(stage: &BoundStage) -> BoundArgs {
    let mut args = BoundArgs::new(stage.name.clone());
    for arg in &stage.args {
        args.insert(arg.name.clone(), static_arg(&arg.value), arg.default);
    }
    args
}

/// Implementation whose static check applies to the stage.
pub(crate) fn implementation_for(
    registry: &Registry,
    command: &str,
    head: bool,
) -> Option<Arc<dyn StageImpl>> {
    if head && let Ok(found) = registry.resolve_source(command) {
        return Some(found);
    }
    registry
        .implementations(command)
        .find_map(|descriptor| match &descriptor.input {
            InputKinds::Only(kinds) => kinds
                .first()
                .and_then(|kind| registry.resolve(command, *kind).ok()),
            InputKinds::Any => registry.resolve(command, RecordKind::Entry).ok(),
            _ => registry.resolve_source(command).ok(),
        })
}

fn network_param(command: &str) -> Option<&'static str> {
    if command.starts_with(HTTP_PREFIX) {
        return Some(URL_PARAM);
    }
    GIT_TRANSFERS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, param)| *param)
}

struct Collector<'a> {
    registry: &'a Registry,
    host: &'a dyn Host,
    config: &'a Config,
    summary: CheckSummary,
}

impl Collector<'_> {
    fn stage(&mut self, planned: &PlannedStage) -> Result<()> {
        let stage = &planned.stage;
        let spec = self
            .registry
            .require_command(&stage.name)
            .map_err(|error| error.with_stage(ExecStep::Bind))?;
        self.summary.waits |= stage.waits || spec.waits;
        self.group(&stage.name);
        self.paths(stage, spec);
        self.network(stage);
        self.programs(stage, spec);
        self.implementation(planned, spec)
    }

    fn group(&mut self, command: &str) {
        let group = self
            .registry
            .groups()
            .find(|group| group.commands.iter().any(|spec| spec.name == command));
        if let Some(group) = group {
            push_unique(&mut self.summary.groups, group.name.clone());
        }
    }

    fn paths(&mut self, stage: &BoundStage, spec: &StageSpec) {
        for arg in &stage.args {
            let names_paths = spec
                .param(&arg.name)
                .is_none_or(|param| param.ty.contains_scalar(ScalarType::Path));
            if names_paths {
                collect_paths(&arg.value, &mut self.summary.paths);
            }
        }
    }

    fn network(&mut self, stage: &BoundStage) {
        let Some(param) = network_param(&stage.name) else {
            return;
        };
        let value = arg(stage, param).unwrap_or(&BoundExpr::Null);
        let items = match value {
            BoundExpr::List { items } => items.as_slice(),
            single => std::slice::from_ref(single),
        };
        for item in items {
            let found = network_use(&stage.name, param, item);
            push_unique(&mut self.summary.network, found);
        }
    }

    fn programs(&mut self, stage: &BoundStage, spec: &StageSpec) {
        if spec.category == Category::Run
            && let Some(command) = arg(stage, COMMAND_PARAM)
        {
            self.command_line(stage, spec, command);
        }
        let Some(external) = &spec.external else {
            return;
        };
        if external
            .when
            .as_deref()
            .is_some_and(|when| !is_given(stage, when))
        {
            return;
        }
        for program in &external.programs {
            push_unique(
                &mut self.summary.programs,
                ProgramUse {
                    command: stage.name.clone(),
                    program: Some(program.clone()),
                    implementation: external.kind,
                    launcher: None,
                    source: TargetSource::Catalog,
                },
            );
        }
    }

    fn command_line(&mut self, stage: &BoundStage, spec: &StageSpec, command: &BoundExpr) {
        let first = match command {
            BoundExpr::List { items } => items.first(),
            BoundExpr::Null => None,
            single => Some(single),
        };
        let Some(first) = first else {
            return;
        };
        let (program, source) = match first {
            BoundExpr::Text { v } | BoundExpr::Path { v } => {
                (Some(v.clone()), TargetSource::Literal)
            }
            _ => (None, TargetSource::Record),
        };
        push_unique(
            &mut self.summary.programs,
            ProgramUse {
                command: stage.name.clone(),
                launcher: program.as_deref().and_then(launcher_for),
                program,
                implementation: spec.implementation,
                source,
            },
        );
    }

    fn implementation(&mut self, planned: &PlannedStage, spec: &StageSpec) -> Result<()> {
        let Some(implementation) =
            implementation_for(self.registry, planned.command(), planned.index == 0)
        else {
            return Ok(());
        };
        let ctx = CheckCtx::new(spec, self.host, self.config);
        let info = implementation
            .check(&static_args(&planned.stage), &ctx)
            .map_err(with_bind_stage)?;
        self.merge(planned, info);
        Ok(())
    }

    fn merge(&mut self, planned: &PlannedStage, info: StaticInfo) {
        let command = planned.command();
        self.summary.waits |= info.waits;
        for target in info.network {
            let found = NetworkUse {
                command: command.to_owned(),
                param: None,
                source: TargetSource::Implementation,
                target: Some(target),
                remote: None,
            };
            push_unique(&mut self.summary.network, found);
        }
        for program in info.programs {
            let found = ProgramUse {
                command: command.to_owned(),
                launcher: launcher_for(&program.program),
                program: Some(program.program),
                implementation: program.implementation,
                source: TargetSource::Implementation,
            };
            push_unique(&mut self.summary.programs, found);
        }
        for path in info.paths {
            push_unique(&mut self.summary.paths, path);
        }
        if !info.details.is_empty() {
            self.summary.details.push(StageDetails {
                command: command.to_owned(),
                index: planned.index,
                details: info.details,
            });
        }
    }
}

fn with_bind_stage(error: Error) -> Error {
    if error.stage().is_some() {
        error
    } else {
        error.with_stage(ExecStep::Bind)
    }
}

fn network_use(command: &str, param: &str, value: &BoundExpr) -> NetworkUse {
    let mut found = NetworkUse {
        command: command.to_owned(),
        param: Some(param.to_owned()),
        source: TargetSource::Literal,
        target: None,
        remote: None,
    };
    match value {
        BoundExpr::Text { v } | BoundExpr::Path { v } => match parse_address(v) {
            Some(target) => found.target = Some(target),
            None if param == REMOTE_PARAM => {
                found.source = TargetSource::Remote;
                found.remote = Some(v.clone());
            }
            None => {}
        },
        BoundExpr::Null if param == REMOTE_PARAM => found.source = TargetSource::Remote,
        BoundExpr::Null => {}
        _ => found.source = TargetSource::Record,
    }
    found
}

/// Computes the result of the check without effect.
pub(crate) fn summarize(
    plan: &Plan,
    registry: &Registry,
    host: &dyn Host,
    config: &Config,
) -> Result<CheckSummary> {
    let operations = u32::try_from(plan.operations().count()).unwrap_or(u32::MAX);
    let mut collector = Collector {
        registry,
        host,
        config,
        summary: CheckSummary {
            format: CHECK_FORMAT.to_owned(),
            effect: plan.effect,
            reversibility: plan.reversibility,
            operations,
            waits: false,
            paths: Vec::new(),
            network: Vec::new(),
            programs: Vec::new(),
            groups: Vec::new(),
            corrections: plan.corrections.clone(),
            details: Vec::new(),
        },
    };
    for stage in plan.stages() {
        collector.stage(stage)?;
    }
    Ok(collector.summary)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests fail by panicking"
    )]

    use nurt_contracts::bound::BoundMapEntry;

    use super::*;

    fn text(v: &str) -> BoundExpr {
        BoundExpr::Text { v: v.to_owned() }
    }

    #[test]
    fn batch_files_get_launcher() {
        assert_eq!(
            launcher_for("C:/tools/Build.CMD").as_deref(),
            Some("cmd.exe")
        );
        assert_eq!(launcher_for("setup.bat").as_deref(), Some("cmd.exe"));
        assert_eq!(launcher_for("go"), None);
    }

    #[test]
    fn network_use_classifies_address_sources() {
        let literal = network_use("http.request", URL_PARAM, &text("http://example.com/a"));
        assert_eq!(literal.source, TargetSource::Literal);
        assert_eq!(literal.target.map(|target| target.port), Some(Some(80)));
        let remote = network_use("git.push", REMOTE_PARAM, &text("origin"));
        assert_eq!(remote.source, TargetSource::Remote);
        assert_eq!(remote.remote.as_deref(), Some("origin"));
        let default_remote = network_use("git.fetch", REMOTE_PARAM, &BoundExpr::Null);
        assert_eq!(default_remote.source, TargetSource::Remote);
        assert!(default_remote.remote.is_none());
        let record = BoundExpr::Field {
            name: "url".to_owned(),
            of: None,
        };
        assert_eq!(
            network_use("http.request", URL_PARAM, &record).source,
            TargetSource::Record
        );
        let unknown = network_use("http.request", URL_PARAM, &text("not an address"));
        assert_eq!(unknown.source, TargetSource::Literal);
        assert!(unknown.target.is_none());
    }

    #[test]
    fn static_arguments_keep_literals_and_expressions() {
        assert_eq!(static_arg(&BoundExpr::Null), ArgValue::Absent);
        assert_eq!(
            static_arg(&BoundExpr::Int { v: 3 }),
            ArgValue::Value(Value::Int(3))
        );
        let list = BoundExpr::List {
            items: vec![
                text("go"),
                BoundExpr::Bool { v: true },
                BoundExpr::Num { v: 1.5 },
            ],
        };
        assert_eq!(
            static_arg(&list),
            ArgValue::Value(Value::List(vec![
                Value::Text("go".to_owned()),
                Value::Bool(true),
                Value::Number(1.5)
            ]))
        );
        let field = BoundExpr::Field {
            name: "size".to_owned(),
            of: None,
        };
        assert_eq!(static_arg(&field), ArgValue::Expr(field.clone()));
        let mixed = BoundExpr::List { items: vec![field] };
        assert_eq!(static_arg(&mixed), ArgValue::Expr(mixed.clone()));
        for literal in [
            BoundExpr::Path { v: "/a".to_owned() },
            BoundExpr::Glob { v: "*".to_owned() },
            BoundExpr::Key {
                v: "a.b".to_owned(),
            },
            BoundExpr::Enum {
                v: "asc".to_owned(),
            },
        ] {
            assert!(matches!(static_arg(&literal), ArgValue::Value(_)));
        }
    }

    #[test]
    fn measure_time_pattern_and_map_literals_become_values() {
        assert_eq!(
            static_arg(&BoundExpr::Size { v: 10 }),
            ArgValue::Value(Value::Size(Size::from_bytes(10)))
        );
        assert_eq!(
            static_arg(&BoundExpr::Dur { v: 500 }),
            ArgValue::Value(Value::Duration(Duration::from_millis(500)))
        );
        assert!(matches!(
            static_arg(&BoundExpr::Regex {
                v: "a+".to_owned(),
                flags: String::new()
            }),
            ArgValue::Value(Value::Regex(_))
        ));
        assert!(matches!(
            static_arg(&BoundExpr::Instant {
                v: "2026-11-21T09:12:00Z".to_owned()
            }),
            ArgValue::Value(Value::Instant(_))
        ));
        let local = BoundExpr::Instant {
            v: "2026-11-21T09:12:00".to_owned(),
        };
        assert_eq!(static_arg(&local), ArgValue::Expr(local.clone()));
        let map = BoundExpr::Map {
            entries: vec![BoundMapEntry {
                k: "action".to_owned(),
                v: text("remove"),
            }],
        };
        let ArgValue::Value(Value::Map(fields)) = static_arg(&map) else {
            panic!("a map of literals is a value");
        };
        assert_eq!(
            fields.get("action"),
            Some(&Value::Text("remove".to_owned()))
        );
    }

    #[test]
    fn range_literals_become_range_values() {
        assert_eq!(
            static_arg(&BoundExpr::Range {
                lo: Some(12),
                hi: Some(12)
            }),
            ArgValue::Value(Value::Range(Range::new(Some(12), Some(12)).unwrap()))
        );
        let open = BoundExpr::Range {
            lo: Some(3),
            hi: None,
        };
        assert!(matches!(
            static_arg(&open),
            ArgValue::Value(Value::Range(_))
        ));
        let inverted = BoundExpr::Range {
            lo: Some(5),
            hi: Some(1),
        };
        assert!(matches!(static_arg(&inverted), ArgValue::Expr(_)));
    }

    #[test]
    fn paths_are_collected_from_lists_once() {
        let mut paths = Vec::new();
        let value = BoundExpr::List {
            items: vec![
                BoundExpr::Path { v: "/a".to_owned() },
                BoundExpr::Path { v: "/b".to_owned() },
                BoundExpr::Path { v: "/a".to_owned() },
                text("/c"),
            ],
        };
        collect_paths(&value, &mut paths);
        assert_eq!(paths, ["/a", "/b"]);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
