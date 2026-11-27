// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Types of an execution plan.

use std::collections::BTreeSet;

use nurt_contracts::ast::Span;
use nurt_contracts::bound::{
    BoundCall, BoundExpr, BoundLimit, BoundParam, BoundRef, BoundStage, BoundStatement, Phase,
};
use nurt_contracts::kinds::{EffectKind, Reversibility};
use nurt_contracts::report::Correction;
use nurt_contracts::stage::{Demand, ExecStep, StageFrame};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const EFFECT_STEPS: &[ExecStep] = &[ExecStep::Plan, ExecStep::Apply];
const READ_STEPS: &[ExecStep] = &[ExecStep::Read];
const CHECK_STEPS: &[ExecStep] = &[ExecStep::Check];
const AFTER_STEPS: &[ExecStep] = &[ExecStep::After];
const VERIFY_STEPS: &[ExecStep] = &[ExecStep::Verify];

/// Options of planning that come from the request rather than the program.
///
/// ```
/// use nurt_plan::PlanOptions;
///
/// let options = PlanOptions::new().with_fields(["path", "size"]).with_limit(50);
/// assert_eq!(options.limit, Some(50));
/// assert_eq!(options.fields.as_ref().map(|fields| fields.len()), Some(2));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct PlanOptions {
    /// Fields of the result requested by the caller (`fields` of the
    /// request); `None` keeps every field.
    pub fields: Option<Vec<String>>,
    /// Page size of the result; applies to pipelines that only read.
    pub limit: Option<u64>,
    /// Corrections applied to the program text before binding; passed on to
    /// the plan and the check result.
    pub corrections: Vec<Correction>,
}

impl PlanOptions {
    /// Options without requested fields, limit or corrections.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests only the given fields of the result.
    #[must_use]
    pub fn with_fields<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.fields = Some(fields.into_iter().map(Into::into).collect());
        self
    }

    /// Sets the page size of the result.
    #[must_use]
    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Sets the corrections applied before binding.
    #[must_use]
    pub fn with_corrections(mut self, corrections: Vec<Correction>) -> Self {
        self.corrections = corrections;
        self
    }

    pub(crate) fn field_set(&self) -> Option<BTreeSet<String>> {
        self.fields
            .as_ref()
            .map(|fields| fields.iter().cloned().collect())
    }
}

/// Execution plan of a bound program.
///
/// The statements keep the order of the program. Every pipeline that is not
/// bound to a name is one operation; a program with several operations is a
/// script and its operations run one after another, each with its own
/// effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    /// Revision of the command catalog the program was bound with.
    pub catalog_rev: String,
    /// Revision of the correction tables applied before binding.
    pub corr_rev: String,
    /// Planned statements in program order.
    pub statements: Vec<PlanStatement>,
    /// Strongest effect of all stages, in the order of [`EffectKind`]:
    /// `read`, `local`, `external`, `process`.
    pub effect: EffectKind,
    /// Weakest reversibility of all stages: `read`, `reversible`,
    /// `reversible_if_preserved`, `irreversible`.
    pub reversibility: Reversibility,
    /// Corrections applied to the program text before binding.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<Correction>,
}

impl Plan {
    /// Operations of the plan in program order.
    pub fn operations(&self) -> impl Iterator<Item = &OperationPlan> {
        self.statements
            .iter()
            .filter_map(|statement| match statement {
                PlanStatement::Operation(operation) => Some(operation),
                PlanStatement::Let(_) | PlanStatement::Construct(_) => None,
            })
    }

    /// The control constructs at the top of the program, in order.
    pub fn constructs(&self) -> impl Iterator<Item = &ConstructPlan> {
        self.statements
            .iter()
            .filter_map(|statement| match statement {
                PlanStatement::Construct(construct) => Some(construct.as_ref()),
                _ => None,
            })
    }

    /// Named results of the plan in program order.
    pub fn lets(&self) -> impl Iterator<Item = &LetPlan> {
        self.statements
            .iter()
            .filter_map(|statement| match statement {
                PlanStatement::Let(binding) => Some(binding),
                PlanStatement::Operation(_) | PlanStatement::Construct(_) => None,
            })
    }

    /// Returns `true` when the program has more than one operation.
    #[must_use]
    pub fn is_script(&self) -> bool {
        self.operations().nth(1).is_some()
    }

    /// Returns `true` when a stage of the plan blocks the call by waiting.
    #[must_use]
    pub fn waits(&self) -> bool {
        self.stages().iter().any(|stage| stage.stage.waits)
    }

    /// Every planned stage: statements in order, the stages of each pipeline
    /// in order, the pipeline of a `verify` stage directly after that stage.
    #[must_use]
    pub fn stages(&self) -> Vec<&PlannedStage> {
        let mut stages = Vec::new();
        for statement in &self.statements {
            statement.collect_stages(&mut stages);
        }
        stages
    }
}

/// A planned statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PlanStatement {
    /// A named result, computed once as a snapshot.
    Let(LetPlan),
    /// An operation: a pipeline whose result is reported.
    Operation(OperationPlan),
    /// A control construct: a condition, a loop, `steps`, a function.
    Construct(Box<ConstructPlan>),
}

impl PlanStatement {
    /// The pipeline of a `let` or of an operation; a control construct has
    /// several, see [`ConstructPlan::pipelines`].
    #[must_use]
    pub fn pipeline(&self) -> Option<&PipelinePlan> {
        match self {
            Self::Let(binding) => Some(&binding.pipeline),
            Self::Operation(operation) => Some(&operation.pipeline),
            Self::Construct(_) => None,
        }
    }

    fn collect_stages<'a>(&'a self, stages: &mut Vec<&'a PlannedStage>) {
        match self {
            Self::Let(binding) => binding.pipeline.collect_stages(stages),
            Self::Operation(operation) => operation.pipeline.collect_stages(stages),
            Self::Construct(construct) => {
                for pipeline in construct.pipelines() {
                    pipeline.collect_stages(stages);
                }
            }
        }
    }
}

/// Plan of a named result.
///
/// The pipeline of a named result only reads, transforms and checks
/// records; it never has an effect stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LetPlan {
    /// Name of the result, without `@`.
    pub name: String,
    /// Plan of the pipeline.
    pub pipeline: PipelinePlan,
    /// Source range of the statement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Plan of one operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OperationPlan {
    /// Index of the operation in the program, from 0.
    pub index: u32,
    /// Plan of the pipeline.
    pub pipeline: PipelinePlan,
}

/// Plan of one pipeline.
///
/// Stages keep the order of the program. Each stage carries its execution
/// step: read (`2`) for the source and transformations, check (`3`) for
/// assertions before the effect, plan and apply (`4`, `5`) for the single
/// effect stage, after (`5a`) for the stages on the records of the effect
/// and verify (`6`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PipelinePlan {
    /// Named result that starts the pipeline; absent when the first stage is
    /// a command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<BoundRef>,
    /// Call of a defined function that produces the input of the stages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<BoundCall>,
    /// Planned stages in program order.
    pub stages: Vec<PlannedStage>,
    /// Strongest effect of the stages, including the pipeline of `verify`.
    pub effect: EffectKind,
    /// Weakest reversibility of the stages, including the pipeline of
    /// `verify`.
    pub reversibility: Reversibility,
    /// Source range of the pipeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

impl PipelinePlan {
    /// The stage that changes the world or runs a program, if any.
    #[must_use]
    pub fn effect_stage(&self) -> Option<&PlannedStage> {
        self.stages.iter().find(|stage| stage.is_effect())
    }

    /// Returns `true` when the pipeline has no effect stage.
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.effect_stage().is_none()
    }

    /// Stages whose first execution step is `step`.
    pub fn stages_in(&self, step: ExecStep) -> impl Iterator<Item = &PlannedStage> {
        self.stages.iter().filter(move |stage| stage.step == step)
    }

    /// Plan of the pipeline checked by the `verify` stage, if any.
    #[must_use]
    pub fn verify(&self) -> Option<&PipelinePlan> {
        self.stages.iter().find_map(|stage| stage.check.as_deref())
    }

    /// Every stage of the pipeline; the pipeline of a `verify` stage follows
    /// that stage.
    #[must_use]
    pub fn all_stages(&self) -> Vec<&PlannedStage> {
        let mut stages = Vec::new();
        self.collect_stages(&mut stages);
        stages
    }

    fn collect_stages<'a>(&'a self, stages: &mut Vec<&'a PlannedStage>) {
        for stage in &self.stages {
            stages.push(stage);
            if let Some(check) = stage.check.as_deref() {
                check.collect_stages(stages);
            }
        }
    }
}

/// One planned stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlannedStage {
    /// Index of the stage in its pipeline, from 0 for the head; a pipeline
    /// that starts with a named result has its first command at 1.
    pub index: u32,
    /// First execution step of the stage; the effect stage also runs in the
    /// following step ([`PlannedStage::steps`]).
    pub step: ExecStep,
    /// The bound stage.
    pub stage: BoundStage,
    /// Reversibility of the stage for its bound arguments.
    pub reversibility: Reversibility,
    /// What the rest of the pipeline needs from the output of the stage.
    pub demand: Demand,
    /// Plan of the pipeline checked by a `verify` stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<Box<PipelinePlan>>,
}

impl PlannedStage {
    /// Full command name.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.stage.name
    }

    /// Execution phase assigned by binding.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.stage.phase
    }

    /// Returns `true` for the effect stage of a pipeline.
    #[must_use]
    pub fn is_effect(&self) -> bool {
        self.step == ExecStep::Plan
    }

    /// Execution steps in which the stage runs: plan and apply for the
    /// effect stage, a single step for every other stage.
    #[must_use]
    pub fn steps(&self) -> &'static [ExecStep] {
        match self.step {
            ExecStep::Plan | ExecStep::Apply => EFFECT_STEPS,
            ExecStep::Check => CHECK_STEPS,
            ExecStep::After => AFTER_STEPS,
            ExecStep::Verify => VERIFY_STEPS,
            _ => READ_STEPS,
        }
    }

    /// Position of the stage passed to its execution context.
    #[must_use]
    pub fn frame(&self) -> StageFrame {
        StageFrame {
            index: self.index,
            command: self.stage.name.clone(),
            step: self.step,
            demand: self.demand.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests fail by panicking"
    )]

    use super::*;

    fn stage(name: &str, step: ExecStep) -> PlannedStage {
        PlannedStage {
            index: 0,
            step,
            stage: BoundStage {
                name: name.to_owned(),
                effect: EffectKind::Read,
                phase: Phase::Transform,
                waits: false,
                args: Vec::new(),
                span: None,
            },
            reversibility: Reversibility::Read,
            demand: Demand::default(),
            check: None,
        }
    }

    #[test]
    fn effect_stage_runs_in_plan_and_apply() {
        let planned = stage("remove", ExecStep::Plan);
        assert!(planned.is_effect());
        assert_eq!(planned.steps(), &[ExecStep::Plan, ExecStep::Apply]);
        assert_eq!(stage("filter", ExecStep::Read).steps(), &[ExecStep::Read]);
        assert_eq!(stage("assert", ExecStep::Check).steps(), &[ExecStep::Check]);
        assert_eq!(stage("count", ExecStep::After).steps(), &[ExecStep::After]);
        assert_eq!(
            stage("verify", ExecStep::Verify).steps(),
            &[ExecStep::Verify]
        );
    }

    #[test]
    fn frame_carries_index_command_step_and_demand() {
        let mut planned = stage("first", ExecStep::Read);
        planned.index = 3;
        planned.demand.limit = Some(10);
        let frame = planned.frame();
        assert_eq!(frame.index, 3);
        assert_eq!(frame.command, "first");
        assert_eq!(frame.step, ExecStep::Read);
        assert_eq!(frame.demand.limit, Some(10));
    }

    #[test]
    fn all_stages_lists_verify_pipeline_after_verify() {
        let check = PipelinePlan {
            head: None,
            call: None,
            stages: vec![stage("run", ExecStep::Plan)],
            effect: EffectKind::Process,
            reversibility: Reversibility::ReversibleIfPreserved,
            span: None,
        };
        let mut verify = stage("verify", ExecStep::Verify);
        verify.check = Some(Box::new(check));
        let pipeline = PipelinePlan {
            head: None,
            call: None,
            stages: vec![stage("replace", ExecStep::Plan), verify],
            effect: EffectKind::Process,
            reversibility: Reversibility::ReversibleIfPreserved,
            span: None,
        };
        let names: Vec<&str> = pipeline
            .all_stages()
            .iter()
            .map(|planned| planned.command())
            .collect();
        assert_eq!(names, ["replace", "verify", "run"]);
        assert_eq!(pipeline.verify().map(|check| check.stages.len()), Some(1));
        assert_eq!(
            pipeline.effect_stage().map(PlannedStage::command),
            Some("replace")
        );
        assert_eq!(pipeline.stages_in(ExecStep::Verify).count(), 1);
    }

    #[test]
    fn options_collect_requested_fields() {
        let options = PlanOptions::new().with_fields(["size", "path", "size"]);
        let fields = options.field_set().unwrap();
        assert_eq!(fields.len(), 2);
        assert!(PlanOptions::new().field_set().is_none());
        assert!(
            PlanOptions::new()
                .with_corrections(Vec::new())
                .corrections
                .is_empty()
        );
    }
}

/// A planned branch of a `when`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BranchPlan {
    /// Condition, of type `bool`.
    pub cond: BoundExpr,
    /// Planned statements of the branch.
    pub body: Vec<PlanStatement>,
}

/// What a control construct does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConstructShape {
    /// `when … otherwise when … otherwise …`.
    When {
        /// Branches in order.
        branches: Vec<BranchPlan>,
        /// Statements that run when no condition holds.
        otherwise: Option<Vec<PlanStatement>>,
    },
    /// `each … as name`.
    Each {
        /// Records to visit.
        source: BoundExpr,
        /// Name of the record of the iteration.
        name: String,
        /// Number of iterations that may run concurrently.
        parallel: Option<u32>,
        /// Whether a failure rolls back the completed iterations.
        atomic: bool,
        /// Statements of the body.
        body: Vec<PlanStatement>,
    },
    /// `repeat until: …`.
    Repeat {
        /// Condition that ends the loop.
        until: BoundExpr,
        /// Wait between iterations in milliseconds.
        every_ms: Option<u64>,
        /// Limit of the loop.
        limit: Option<BoundLimit>,
        /// Whether a failure rolls back the completed iterations.
        atomic: bool,
        /// Statements of the body.
        body: Vec<PlanStatement>,
    },
    /// `while …`.
    While {
        /// Condition that keeps the loop running.
        cond: BoundExpr,
        /// Limit of the loop.
        limit: Option<BoundLimit>,
        /// Whether a failure rolls back the completed iterations.
        atomic: bool,
        /// Statements of the body.
        body: Vec<PlanStatement>,
    },
    /// `steps { … }`.
    Steps {
        /// Planned steps.
        body: Vec<PlanStatement>,
    },
    /// `define name(…) { … }`.
    Define {
        /// Name of the function.
        name: String,
        /// Declared parameters.
        params: Vec<BoundParam>,
        /// Planned statements of the body.
        body: Vec<PlanStatement>,
    },
    /// `params { … }`.
    Params {
        /// Declared parameters.
        params: Vec<BoundParam>,
    },
    /// `break`.
    Break,
    /// `continue`.
    Continue,
}

/// A planned control construct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConstructPlan {
    /// Number among the operations of the program.
    pub index: u32,
    /// What the construct does, with its planned statements.
    pub shape: ConstructShape,
    /// The bound statement, recorded in the journal as the program of the
    /// construct.
    pub program: BoundStatement,
    /// Strongest effect of the statements inside.
    pub effect: EffectKind,
    /// Weakest reversibility of the statements inside.
    pub reversibility: Reversibility,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

impl ConstructPlan {
    /// Name of the construct as the journal records it: `when`, `each`,
    /// `repeat`, `while`, `steps`, `define`, `params`, `break`, `continue`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self.shape {
            ConstructShape::When { .. } => "when",
            ConstructShape::Each { .. } => "each",
            ConstructShape::Repeat { .. } => "repeat",
            ConstructShape::While { .. } => "while",
            ConstructShape::Steps { .. } => "steps",
            ConstructShape::Define { .. } => "define",
            ConstructShape::Params { .. } => "params",
            ConstructShape::Break => "break",
            ConstructShape::Continue => "continue",
        }
    }

    /// The statements of every block of the construct, not descending into
    /// nested constructs.
    #[must_use]
    pub fn blocks(&self) -> Vec<&[PlanStatement]> {
        match &self.shape {
            ConstructShape::When {
                branches,
                otherwise,
            } => branches
                .iter()
                .map(|branch| branch.body.as_slice())
                .chain(otherwise.iter().map(Vec::as_slice))
                .collect(),
            ConstructShape::Each { body, .. }
            | ConstructShape::Repeat { body, .. }
            | ConstructShape::While { body, .. }
            | ConstructShape::Steps { body }
            | ConstructShape::Define { body, .. } => vec![body.as_slice()],
            _ => Vec::new(),
        }
    }

    /// Every pipeline of the construct, nested constructs included.
    #[must_use]
    pub fn pipelines(&self) -> Vec<&PipelinePlan> {
        let mut found = Vec::new();
        for block in self.blocks() {
            for statement in block {
                match statement {
                    PlanStatement::Construct(inner) => found.extend(inner.pipelines()),
                    other => found.extend(other.pipeline()),
                }
            }
        }
        found
    }

    /// Whether the construct, or one inside, can change the world.
    #[must_use]
    pub fn changes(&self) -> bool {
        self.effect != EffectKind::Read
    }
}
