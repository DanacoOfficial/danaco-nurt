// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Hints for stages: the fields the rest of the pipeline uses, whether file
//! records must carry their entity tag, and how many records are used.
//!
//! Demands are computed from the end of each pipeline towards its head.
//! The demand of a stage describes its output and is derived from the stage
//! that consumes it:
//!
//! * fields: a stage that passes its input through (catalog output `same`)
//!   needs the fields it reads plus the fields needed after it; `select` and
//!   stages that produce a single value need only the fields they read;
//!   effect stages and every other stage need all fields;
//! * entity tag: needed when a changing stage or `assert if_match:` follows
//!   before the effect, and in a named result used by a changing operation;
//! * limit: `first N` uses `N` records, `skip N` adds `N` to the limit after
//!   it, `select` keeps it, `assert count:` with an upper bound `H` uses
//!   `H + 1` records, `assert empty: true` uses one; the page size of the
//!   request limits the last stage of a pipeline that only reads. A stage
//!   whose static check reports the detail `limit` (an integer) takes at
//!   most that many input records; the hint replaces the rule of the
//!   command and is capped by the limit after the stage.

use std::collections::{BTreeSet, HashSet};
use std::ffi::OsString;
use std::path::PathBuf;

use nurt_catalog::Registry;
use nurt_contracts::bound::{BoundExpr, BoundRef, BoundStage, Phase};
use nurt_contracts::config::Config;
use nurt_contracts::env::{Host, HostInfo, Os, PathStyle};
use nurt_contracts::spec::{IoSpec, ParamType, ScalarType, StageSpec};
use nurt_contracts::stage::{CheckCtx, Demand, FieldDemand};
use nurt_contracts::value::Value;

use crate::check::{implementation_for, static_args};
use crate::model::{PipelinePlan, PlanOptions, PlanStatement, PlannedStage};

/// Detail of the static check of a stage that bounds its input records.
const LIMIT_DETAIL: &str = "limit";

/// Host without facts of its own: a limit hint never depends on the host,
/// so the static check during planning gets neutral values.
struct PlanningHost(HostInfo);

impl PlanningHost {
    fn new() -> Self {
        let (os, path_style) = if cfg!(windows) {
            (Os::Windows, PathStyle::Windows)
        } else if cfg!(target_os = "macos") {
            (Os::Macos, PathStyle::Posix)
        } else {
            (Os::Linux, PathStyle::Posix)
        };
        Self(HostInfo {
            os,
            os_version: String::new(),
            architecture: String::new(),
            hostname: String::new(),
            user: String::new(),
            home: PathBuf::new(),
            temp: PathBuf::new(),
            path_style,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        })
    }
}

impl Host for PlanningHost {
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

/// What the planner needs to ask a stage implementation for a hint.
struct Hinting<'a> {
    registry: &'a Registry,
    host: PlanningHost,
    config: Config,
}

impl Hinting<'_> {
    /// Limit reported by the static check of the implementation of `planned`.
    /// A rejected check gives no hint: the check of the plan reports the
    /// error.
    fn limit(&self, planned: &PlannedStage) -> Option<u64> {
        let command = planned.command();
        let spec = self.registry.command(command)?;
        let implementation = implementation_for(self.registry, command, planned.index == 0)?;
        let ctx = CheckCtx::new(spec, &self.host, &self.config);
        let info = implementation
            .check(&static_args(&planned.stage), &ctx)
            .ok()?;
        info.details.get(LIMIT_DETAIL).and_then(detail_limit)
    }
}

fn detail_limit(value: &Value) -> Option<u64> {
    match value {
        Value::Int(count) => u64::try_from(*count).ok(),
        Value::BigInt(digits) => digits.parse().ok(),
        _ => None,
    }
}

const SELECT: &str = "select";
const FIRST: &str = "first";
const SKIP: &str = "skip";
const ASSERT: &str = "assert";
const COUNT_PARAM: &str = "count";
const EMPTY_PARAM: &str = "empty";
const IF_MATCH_PARAM: &str = "if_match";

/// Annotates every stage of the planned statements with its demand.
pub(crate) fn annotate(
    statements: &mut [PlanStatement],
    registry: &Registry,
    options: &PlanOptions,
) {
    let tagged = lets_used_by_changes(statements);
    let requested = options.field_set();
    let hinting = Hinting {
        registry,
        host: PlanningHost::new(),
        config: Config::default(),
    };
    for statement in statements.iter_mut() {
        match statement {
            PlanStatement::Let(binding) => {
                let end = Demand {
                    fields: FieldDemand::All,
                    etag: tagged.contains(binding.name.as_str()),
                    limit: None,
                };
                annotate_pipeline(&mut binding.pipeline, &hinting, end);
            }
            PlanStatement::Operation(operation) => {
                let read_only = operation.pipeline.is_read_only();
                let end = Demand {
                    fields: requested
                        .clone()
                        .map_or(FieldDemand::All, FieldDemand::Only),
                    etag: false,
                    limit: if read_only { options.limit } else { None },
                };
                annotate_pipeline(&mut operation.pipeline, &hinting, end);
            }
        }
    }
}

fn annotate_pipeline(pipeline: &mut PipelinePlan, hinting: &Hinting<'_>, end: Demand) {
    let mut demand = end;
    for planned in pipeline.stages.iter_mut().rev() {
        if let Some(check) = planned.check.as_deref_mut() {
            annotate_pipeline(check, hinting, all_fields());
        }
        planned.demand = if planned.is_effect() {
            Demand {
                fields: demand.fields.clone(),
                etag: false,
                limit: None,
            }
        } else {
            demand.clone()
        };
        let hint = if planned.is_effect() {
            None
        } else {
            hinting.limit(planned)
        };
        demand = upstream(
            planned,
            hinting.registry.command(planned.command()),
            &planned.demand,
            hint,
        );
    }
}

/// Demand on the output of a pipeline whose records are used as a whole.
fn all_fields() -> Demand {
    Demand {
        fields: FieldDemand::All,
        etag: false,
        limit: None,
    }
}

/// Demand on the input of `consumer`, given the demand on its output.
fn upstream(
    consumer: &PlannedStage,
    spec: Option<&StageSpec>,
    after: &Demand,
    hint: Option<u64>,
) -> Demand {
    Demand {
        fields: input_fields(consumer, spec, &after.fields),
        etag: input_etag(consumer, after.etag),
        limit: input_limit(consumer, after.limit, hint),
    }
}

fn input_fields(
    consumer: &PlannedStage,
    spec: Option<&StageSpec>,
    after: &FieldDemand,
) -> FieldDemand {
    let Some(spec) = spec else {
        return FieldDemand::All;
    };
    if consumer.is_effect() || consumer.phase() == Phase::Verify {
        return FieldDemand::All;
    }
    let own = own_fields(&consumer.stage, spec);
    if consumer.command() == SELECT || spec.output.iter().all(|io| matches!(io, IoSpec::Value(_))) {
        return FieldDemand::Only(own);
    }
    if !spec.output.is_empty() && spec.output.iter().all(|io| *io == IoSpec::Same) {
        return match after {
            FieldDemand::All => FieldDemand::All,
            FieldDemand::Only(fields) => FieldDemand::Only(own.union(fields).cloned().collect()),
        };
    }
    FieldDemand::All
}

fn input_etag(consumer: &PlannedStage, after: bool) -> bool {
    match consumer.phase() {
        Phase::Mutate => true,
        Phase::Run | Phase::After | Phase::Verify => false,
        _ => {
            after
                || (consumer.command() == ASSERT
                    && given(&consumer.stage, IF_MATCH_PARAM).is_some())
        }
    }
}

fn input_limit(consumer: &PlannedStage, after: Option<u64>, hint: Option<u64>) -> Option<u64> {
    if consumer.is_effect() {
        return None;
    }
    if let Some(hint) = hint {
        return Some(after.map_or(hint, |after| after.min(hint)));
    }
    match consumer.command() {
        FIRST => match int_arg(&consumer.stage, COUNT_PARAM) {
            Some(count) => Some(after.map_or(count, |after| after.min(count))),
            None => after,
        },
        SKIP => after
            .zip(int_arg(&consumer.stage, COUNT_PARAM))
            .map(|(after, skipped)| after.saturating_add(skipped)),
        SELECT => after,
        ASSERT => assert_limit(&consumer.stage),
        _ => None,
    }
}

/// Records an assertion needs to decide: one more than the upper bound of
/// `count`, or one for `empty: true`; `None` for every other assertion.
fn assert_limit(stage: &BoundStage) -> Option<u64> {
    let mut given_args = stage
        .args
        .iter()
        .filter(|arg| !arg.default && arg.value != BoundExpr::Null);
    let arg = given_args.next()?;
    if given_args.next().is_some() {
        return None;
    }
    match (arg.name.as_str(), &arg.value) {
        (COUNT_PARAM, BoundExpr::Range { hi: Some(hi), .. }) => {
            u64::try_from(*hi).ok().map(|hi| hi.saturating_add(1))
        }
        (EMPTY_PARAM, BoundExpr::Bool { v: true }) => Some(1),
        _ => None,
    }
}

fn given<'a>(stage: &'a BoundStage, name: &str) -> Option<&'a BoundExpr> {
    stage
        .args
        .iter()
        .find(|arg| arg.name == name && !arg.default && arg.value != BoundExpr::Null)
        .map(|arg| &arg.value)
}

fn int_arg(stage: &BoundStage, name: &str) -> Option<u64> {
    let value = stage
        .args
        .iter()
        .find(|arg| arg.name == name)
        .map(|arg| &arg.value)?;
    match value {
        BoundExpr::Int { v } => u64::try_from(*v).ok(),
        _ => None,
    }
}

fn reads_fields(ty: &ParamType) -> bool {
    [
        ScalarType::Field,
        ScalarType::Expression,
        ScalarType::NamedExpression,
    ]
    .into_iter()
    .any(|scalar| ty.contains_scalar(scalar))
}

/// Fields of the input records read by the arguments of a stage.
fn own_fields(stage: &BoundStage, spec: &StageSpec) -> BTreeSet<String> {
    let mut fields = BTreeSet::new();
    for arg in &stage.args {
        if spec
            .param(&arg.name)
            .is_some_and(|param| reads_fields(&param.ty))
        {
            collect_fields(&arg.value, &mut fields);
        }
    }
    fields
}

fn collect_fields(value: &BoundExpr, fields: &mut BTreeSet<String>) {
    match value {
        BoundExpr::Field { name, of: None } => {
            fields.insert(name.clone());
        }
        BoundExpr::Field { of: Some(base), .. } => collect_fields(base, fields),
        BoundExpr::List { items } | BoundExpr::Call { args: items, .. } => {
            for item in items {
                collect_fields(item, fields);
            }
        }
        BoundExpr::Map { entries } => {
            for entry in entries {
                collect_fields(&entry.v, fields);
            }
        }
        BoundExpr::Un { x, .. } | BoundExpr::Spread { x } => collect_fields(x, fields),
        BoundExpr::Bin { l, r, .. } => {
            collect_fields(l, fields);
            collect_fields(r, fields);
        }
        _ => {}
    }
}

/// Names of the results used by an operation that changes files.
fn lets_used_by_changes(statements: &[PlanStatement]) -> HashSet<String> {
    let mut names = HashSet::new();
    for statement in statements {
        let PlanStatement::Operation(operation) = statement else {
            continue;
        };
        let pipeline = &operation.pipeline;
        let changes = pipeline
            .stages
            .iter()
            .any(|stage| stage.phase() == Phase::Mutate);
        if !changes {
            continue;
        }
        if let Some(head) = &pipeline.head {
            names.insert(head.name.as_str());
        }
        for stage in &pipeline.stages {
            for arg in &stage.stage.args {
                collect_refs(&arg.value, &mut names);
            }
        }
    }
    names.into_iter().map(str::to_owned).collect()
}

fn collect_refs<'a>(value: &'a BoundExpr, names: &mut HashSet<&'a str>) {
    match value {
        BoundExpr::Ref(BoundRef { name, .. }) => {
            names.insert(name.as_str());
        }
        BoundExpr::List { items } | BoundExpr::Call { args: items, .. } => {
            for item in items {
                collect_refs(item, names);
            }
        }
        BoundExpr::Map { entries } => {
            for entry in entries {
                collect_refs(&entry.v, names);
            }
        }
        BoundExpr::Field { of: Some(base), .. }
        | BoundExpr::Un { x: base, .. }
        | BoundExpr::Spread { x: base } => {
            collect_refs(base, names);
        }
        BoundExpr::Bin { l, r, .. } => {
            collect_refs(l, names);
            collect_refs(r, names);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests fail by panicking"
    )]

    use nurt_contracts::bound::BoundArg;
    use nurt_contracts::kinds::EffectKind;

    use super::*;

    fn stage(name: &str, args: Vec<BoundArg>) -> BoundStage {
        BoundStage {
            name: name.to_owned(),
            effect: EffectKind::Read,
            phase: Phase::Check,
            waits: false,
            args,
            span: None,
        }
    }

    fn range(lo: Option<i64>, hi: Option<i64>) -> BoundExpr {
        BoundExpr::Range { lo, hi }
    }

    #[test]
    fn assert_count_uses_one_record_more_than_upper_bound() {
        let exact = stage(
            ASSERT,
            vec![BoundArg::given(COUNT_PARAM, range(Some(3), Some(3)))],
        );
        assert_eq!(assert_limit(&exact), Some(4));
        let open = stage(
            ASSERT,
            vec![BoundArg::given(COUNT_PARAM, range(Some(3), None))],
        );
        assert_eq!(assert_limit(&open), None);
        let empty = stage(
            ASSERT,
            vec![BoundArg::given(EMPTY_PARAM, BoundExpr::Bool { v: true })],
        );
        assert_eq!(assert_limit(&empty), Some(1));
        let any = stage(
            ASSERT,
            vec![
                BoundArg::from_default(COUNT_PARAM, BoundExpr::Null),
                BoundArg::given("any", BoundExpr::Bool { v: true }),
            ],
        );
        assert_eq!(assert_limit(&any), None);
    }

    #[test]
    fn limit_detail_accepts_only_non_negative_integers() {
        assert_eq!(detail_limit(&Value::Int(7)), Some(7));
        assert_eq!(detail_limit(&Value::Int(0)), Some(0));
        assert_eq!(detail_limit(&Value::Int(-1)), None);
        assert_eq!(
            detail_limit(&Value::BigInt(u64::MAX.to_string())),
            Some(u64::MAX)
        );
        assert_eq!(detail_limit(&Value::BigInt("-9".to_owned())), None);
        assert_eq!(detail_limit(&Value::Text("7".to_owned())), None);
    }

    #[test]
    fn collects_fields_of_nested_expressions() {
        let value = BoundExpr::List {
            items: vec![
                BoundExpr::Call {
                    function: "lower".to_owned(),
                    args: vec![BoundExpr::Field {
                        name: "name".to_owned(),
                        of: Some(Box::new(BoundExpr::Field {
                            name: "parent".to_owned(),
                            of: None,
                        })),
                    }],
                },
                BoundExpr::Map {
                    entries: vec![nurt_contracts::bound::BoundMapEntry {
                        k: "big".to_owned(),
                        v: BoundExpr::Spread {
                            x: Box::new(BoundExpr::Field {
                                name: "size".to_owned(),
                                of: None,
                            }),
                        },
                    }],
                },
            ],
        };
        let mut fields = BTreeSet::new();
        collect_fields(&value, &mut fields);
        assert_eq!(fields.into_iter().collect::<Vec<_>>(), ["parent", "size"]);
    }

    #[test]
    fn collects_references_inside_expressions() {
        let value = BoundExpr::Bin {
            op: nurt_contracts::ast::BinOp::In,
            l: Box::new(BoundExpr::Field {
                name: "path".to_owned(),
                of: None,
            }),
            r: Box::new(BoundExpr::Ref(BoundRef {
                name: "keep".to_owned(),
                fields: vec!["path".to_owned()],
            })),
        };
        let mut names = HashSet::new();
        collect_refs(&value, &mut names);
        assert!(names.contains("keep"));
    }

    #[test]
    fn collects_references_inside_maps() {
        let value = BoundExpr::Map {
            entries: vec![nurt_contracts::bound::BoundMapEntry {
                k: "with".to_owned(),
                v: BoundExpr::Ref(BoundRef {
                    name: "keep".to_owned(),
                    fields: Vec::new(),
                }),
            }],
        };
        let mut names = HashSet::new();
        collect_refs(&value, &mut names);
        assert!(names.contains("keep"));
    }

    #[test]
    fn collects_nothing_from_literals() {
        let mut names = HashSet::new();
        collect_refs(&BoundExpr::Int { v: 1 }, &mut names);
        collect_refs(&BoundExpr::Text { v: "a".to_owned() }, &mut names);
        assert!(names.is_empty());
    }

    #[test]
    fn count_gives_a_limit_only_as_a_non_negative_literal() {
        let absent = stage("first", Vec::new());
        assert_eq!(int_arg(&absent, COUNT_PARAM), None);
        let negative = BoundArg::given(COUNT_PARAM, BoundExpr::Int { v: -1 });
        assert_eq!(int_arg(&stage("first", vec![negative]), COUNT_PARAM), None);
        let literal = BoundArg::given(COUNT_PARAM, BoundExpr::Int { v: 4 });
        assert_eq!(
            int_arg(&stage("first", vec![literal]), COUNT_PARAM),
            Some(4)
        );
    }
}
