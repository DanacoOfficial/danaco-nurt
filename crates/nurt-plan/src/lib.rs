// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Execution planning of bound Danaco Nurt programs.
//!
//! The planner turns a bound program into an execution plan: bindings,
//! streaming pipeline stages, the optional changing stage and its effect, and
//! the checks that run before and after the change.
//!
//! | Step | Stages |
//! |---|---|
//! | read (`2`) | the source and the transformations |
//! | check (`3`) | assertions before the effect; a failure changes nothing |
//! | plan, apply (`4`, `5`) | the single stage that changes the world or runs a program |
//! | after (`5a`) | `assert`, `filter`, `select`, `first`, `last`, `skip`, `count`, `sort` and `group` on the records of the effect |
//! | verify (`6`) | the pipeline of `verify`; a failure rolls the effect back |
//!
//! Every stage carries a [`Demand`](nurt_contracts::stage::Demand): the
//! fields used later, whether file records must carry their entity tag and
//! how many records are used. [`check`] reports, without any effect, what
//! the plan would touch.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod address;
mod check;
mod effect;
mod hints;
mod model;
mod planner;
mod text;

use nurt_catalog::Registry;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::config::Config;
use nurt_contracts::env::Host;
use nurt_contracts::error::Result;

pub use check::{CHECK_FORMAT, CheckSummary, NetworkUse, ProgramUse, StageDetails, TargetSource};
pub use model::{
    BranchPlan, ConstructPlan, ConstructShape, LetPlan, OperationPlan, PipelinePlan, Plan,
    PlanOptions, PlanStatement, PlannedStage,
};

/// Plans a bound document with default options.
///
/// # Errors
///
/// As [`plan_with`].
///
/// # Examples
///
/// ```
/// use nurt_catalog::Registry;
/// use nurt_contracts::bound::{
///     BoundArg, BoundDocument, BoundExpr, BoundHead, BoundPipeline, BoundProgram, BoundStage,
///     BoundStatement, Phase,
/// };
/// use nurt_contracts::kinds::EffectKind;
///
/// fn stage(name: &str, effect: EffectKind, phase: Phase, args: Vec<BoundArg>) -> BoundStage {
///     BoundStage { name: name.to_owned(), effect, phase, waits: false, args, span: None }
/// }
///
/// let path = BoundArg::given("path", BoundExpr::Path { v: "/srv/app/build".to_owned() });
/// let find = stage("find", EffectKind::Read, Phase::Source, vec![path]);
/// let remove = stage("remove", EffectKind::Local, Phase::Mutate, Vec::new());
/// let pipeline = BoundPipeline { head: BoundHead::Stage(find), stages: vec![remove], span: None };
/// let program = BoundProgram { statements: vec![BoundStatement::Pipeline(pipeline)] };
/// let document = BoundDocument::new("cat-1", "corr-1", program);
///
/// let plan = nurt_plan::plan(&document, &Registry::load_embedded()?)?;
/// let operation = plan.operations().next().ok_or("no operation")?;
/// assert_eq!(operation.pipeline.effect_stage().map(|s| s.command()), Some("remove"));
/// assert!(operation.pipeline.stages[0].demand.etag);
/// assert_eq!(plan.effect, EffectKind::Local);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn plan(document: &BoundDocument, registry: &Registry) -> Result<Plan> {
    plan_with(document, registry, &PlanOptions::default())
}

/// Plans a bound document.
///
/// The order of phases is checked independently of binding.
///
/// # Errors
///
/// * `E_ONE_EFFECT` with `next_steps` that split the program, when a
///   pipeline has a second effect stage, a stage after the effect other than
///   the stages of step `5a` and a final `verify`, an effect inside `let`,
///   or a change inside the pipeline of `verify`;
/// * `E_AST_SCHEMA` when the phases contradict the position or the effect
///   of a stage, for example the phase `after` without an effect stage;
/// * `E_UNKNOWN_COMMAND` when a command is not in the registry;
/// * `E_MISSING_PARAM` when `verify` has no pipeline to check.
pub fn plan_with(
    document: &BoundDocument,
    registry: &Registry,
    options: &PlanOptions,
) -> Result<Plan> {
    planner::Planner::new(registry, options).plan(document)
}

/// Checks a plan without effect.
///
/// The summary lists the effect class and reversibility, the paths named by
/// the program, the network endpoints and external programs, whether a stage
/// waits, the command groups and the corrections applied before binding.
///
/// # Errors
///
/// * `E_UNKNOWN_COMMAND` when a command of the plan is not in the registry;
/// * the error of the static check of a stage implementation, which rejects
///   the program.
pub fn check(
    plan: &Plan,
    registry: &Registry,
    host: &dyn Host,
    config: &Config,
) -> Result<CheckSummary> {
    check::summarize(plan, registry, host, config)
}
