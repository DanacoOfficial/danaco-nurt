// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Construction of a plan from a bound document.
//!
//! Binding assigns every stage its phase. The planner checks the order of
//! the phases again, independently of binding, because the plan decides
//! which stages may change the world: a pipeline has no more than one effect
//! stage, only the stages of [`AFTER_EFFECT`] and a final `verify` may follow
//! it, and named results and the pipeline of `verify` never change files.

use nurt_catalog::Registry;
use nurt_contracts::ast::Span;
use nurt_contracts::bound::{
    BoundDocument, BoundExpr, BoundHead, BoundPipeline, BoundRef, BoundStage, BoundStatement, Phase,
};
use nurt_contracts::error::{At, Error, ErrorCode, Result};
use nurt_contracts::kinds::EffectKind;
use nurt_contracts::stage::{Demand, ExecStep};

use crate::effect::{EffectClass, stage_reversibility};
use crate::hints;
use crate::model::{
    LetPlan, OperationPlan, PipelinePlan, Plan, PlanOptions, PlanStatement, PlannedStage,
};
use crate::text;

/// Commands allowed after the effect stage, on the records of the effect.
pub(crate) const AFTER_EFFECT: &[&str] = &[
    "assert", "filter", "select", "first", "last", "skip", "count", "sort", "group",
];

/// Name of the verification command.
pub(crate) const VERIFY: &str = "verify";

/// Parameter of `verify` that holds the checked pipeline.
const VERIFY_CHECK: &str = "check";

/// Where a pipeline appears in the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// A pipeline whose result is reported.
    Operation,
    /// The pipeline of a named result.
    Let,
    /// The pipeline checked by `verify`.
    Verify,
}

/// A rule of the phase order that a pipeline breaks; indexes count the
/// stages of the pipeline without a named head.
#[derive(Debug)]
enum Violation {
    EffectInLet { stage: usize },
    ChangeInVerify { stage: usize },
    SecondEffect { first: usize, second: usize },
    NotAfterEffect { effect: usize, stage: usize },
    AfterVerify { verify: usize, stage: usize },
    Malformed { stage: usize, message: String },
}

/// Position of the effect stage and of `verify` in a valid pipeline.
#[derive(Debug, Default, Clone, Copy)]
struct Layout {
    effect: Option<usize>,
    verify: Option<usize>,
}

/// A pipeline split into its named head and its stages.
struct Stages<'a> {
    head: Option<&'a BoundRef>,
    list: Vec<&'a BoundStage>,
    span: Option<Span>,
}

impl<'a> Stages<'a> {
    fn of(pipeline: &'a BoundPipeline) -> Result<Self> {
        let (head, first) = match &pipeline.head {
            BoundHead::Stage(stage) => (None, Some(stage)),
            BoundHead::Ref(reference) => (Some(reference), None),
            _ => return Err(malformed("the pipeline head has an unsupported kind", None)),
        };
        Ok(Self {
            head,
            list: first.into_iter().chain(&pipeline.stages).collect(),
            span: pipeline.span,
        })
    }

    fn offset(&self) -> usize {
        usize::from(self.head.is_some())
    }

    fn text(&self, stages: &[&BoundStage]) -> Option<String> {
        text::stages_text(self.head, stages)
    }
}

fn at_of(span: Span) -> At {
    let length = (span.line == span.end_line).then(|| span.end_column.saturating_sub(span.column));
    At {
        line: Some(span.line),
        column: Some(span.column),
        length,
        stage: None,
    }
}

fn with_span(error: Error, span: Option<Span>) -> Error {
    match span {
        Some(span) => error.with_at(at_of(span)),
        None => error,
    }
}

fn malformed(message: impl Into<String>, span: Option<Span>) -> Error {
    with_span(
        Error::new(ErrorCode::AstSchema, message).with_stage(ExecStep::Parse),
        span,
    )
}

fn one_effect(message: String, span: Option<Span>, next_steps: Vec<String>) -> Error {
    let error = Error::new(ErrorCode::OneEffect, message).with_stage(ExecStep::Bind);
    next_steps
        .into_iter()
        .fold(with_span(error, span), Error::with_next_step)
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Source => "source",
        Phase::Transform => "transform",
        Phase::Check => "check",
        Phase::Mutate => "mutate",
        Phase::Run => "run",
        Phase::After => "after",
        Phase::Verify => "verify",
        _ => "unknown",
    }
}

fn effect_name(effect: EffectKind) -> &'static str {
    match effect {
        EffectKind::Read => "read",
        EffectKind::Local => "local",
        EffectKind::External => "external",
        EffectKind::Process => "process",
    }
}

/// Returns `true` for a stage that changes the world or runs a program.
fn has_effect(stage: &BoundStage) -> bool {
    stage.phase.is_effect() || stage.effect != EffectKind::Read
}

fn step_of(stage: &BoundStage) -> ExecStep {
    match stage.phase {
        Phase::Mutate | Phase::Run => ExecStep::Plan,
        Phase::Check => ExecStep::Check,
        Phase::After => ExecStep::After,
        Phase::Verify => ExecStep::Verify,
        _ => ExecStep::Read,
    }
}

fn record_effect(
    stage: &BoundStage,
    index: usize,
    role: Role,
    layout: &mut Layout,
) -> std::result::Result<(), Violation> {
    if !stage.phase.is_effect() {
        return Err(Violation::Malformed {
            stage: index,
            message: format!(
                "`{}` has the effect {} but the phase {}",
                stage.name,
                effect_name(stage.effect),
                phase_name(stage.phase)
            ),
        });
    }
    match role {
        Role::Let => return Err(Violation::EffectInLet { stage: index }),
        Role::Verify if stage.phase == Phase::Mutate || stage.effect != EffectKind::Process => {
            return Err(Violation::ChangeInVerify { stage: index });
        }
        _ => {}
    }
    if let Some(first) = layout.effect {
        return Err(Violation::SecondEffect {
            first,
            second: index,
        });
    }
    layout.effect = Some(index);
    Ok(())
}

fn check_after(
    stage: &BoundStage,
    index: usize,
    layout: Layout,
) -> std::result::Result<(), Violation> {
    let Some(effect) = layout.effect else {
        return Err(Violation::Malformed {
            stage: index,
            message: format!(
                "`{}` has the phase after but no effect stage precedes it",
                stage.name
            ),
        });
    };
    if AFTER_EFFECT.contains(&stage.name.as_str()) {
        Ok(())
    } else {
        Err(Violation::NotAfterEffect {
            effect,
            stage: index,
        })
    }
}

fn check_verify(
    stage: &BoundStage,
    index: usize,
    role: Role,
    layout: &mut Layout,
) -> std::result::Result<(), Violation> {
    let message = if stage.name != VERIFY {
        Some(format!("`{}` has the phase verify", stage.name))
    } else if role != Role::Operation {
        Some("`verify` can only end an operation".to_owned())
    } else if layout.effect.is_none() {
        Some("`verify` needs an effect stage before it".to_owned())
    } else {
        None
    };
    match message {
        Some(message) => Err(Violation::Malformed {
            stage: index,
            message,
        }),
        None => {
            layout.verify = Some(index);
            Ok(())
        }
    }
}

fn check_before(
    stage: &BoundStage,
    index: usize,
    is_head: bool,
    layout: Layout,
) -> std::result::Result<(), Violation> {
    if let Some(effect) = layout.effect {
        if AFTER_EFFECT.contains(&stage.name.as_str()) {
            return Err(Violation::Malformed {
                stage: index,
                message: format!(
                    "`{}` follows the effect stage and must have the phase after",
                    stage.name
                ),
            });
        }
        return Err(Violation::NotAfterEffect {
            effect,
            stage: index,
        });
    }
    let misplaced = match stage.phase {
        Phase::Source => !is_head,
        Phase::Transform | Phase::Check => is_head,
        _ => true,
    };
    if misplaced {
        return Err(Violation::Malformed {
            stage: index,
            message: format!(
                "`{}` cannot have the phase {} at position {}",
                stage.name,
                phase_name(stage.phase),
                index + 1
            ),
        });
    }
    Ok(())
}

/// Checks the order of phases in a pipeline.
fn validate(stages: &Stages<'_>, role: Role) -> std::result::Result<Layout, Violation> {
    let mut layout = Layout::default();
    for (index, stage) in stages.list.iter().enumerate() {
        if let Some(verify) = layout.verify {
            return Err(Violation::AfterVerify {
                verify,
                stage: index,
            });
        }
        let is_head = index == 0 && stages.head.is_none();
        if has_effect(stage) {
            record_effect(stage, index, role, &mut layout)?;
            continue;
        }
        match stage.phase {
            Phase::After => check_after(stage, index, layout)?,
            Phase::Verify => check_verify(stage, index, role, &mut layout)?,
            _ => check_before(stage, index, is_head, layout)?,
        }
    }
    Ok(layout)
}

/// Converts a violation into the error reported to the caller.
fn violation_error(stages: &Stages<'_>, violation: Violation) -> Error {
    let list = &stages.list;
    let name = |index: usize| list.get(index).map_or("", |stage| stage.name.as_str());
    let span_of = |index: usize| list.get(index).and_then(|stage| stage.span);
    match violation {
        Violation::EffectInLet { stage } => one_effect(
            format!(
                "`{}` changes the world inside `let`; a named result can only read records",
                name(stage)
            ),
            span_of(stage),
            Vec::new(),
        ),
        Violation::ChangeInVerify { stage } => one_effect(
            format!(
                "`{}` changes the world inside `verify`; the checked pipeline can only read records and run programs",
                name(stage)
            ),
            span_of(stage),
            Vec::new(),
        ),
        Violation::SecondEffect { first, second } => {
            let next_steps = [
                stages.text(&list[..second]),
                stages.text(&[&list[..first], &list[second..]].concat()),
            ];
            one_effect(
                format!(
                    "`{}` is a second effect stage after `{}`; run each effect as a separate call",
                    name(second),
                    name(first)
                ),
                span_of(second),
                next_steps.into_iter().flatten().collect(),
            )
        }
        Violation::NotAfterEffect { effect, stage } => one_effect(
            format!(
                "`{}` cannot follow the effect stage `{}`; only {} and `verify` can",
                name(stage),
                name(effect),
                AFTER_EFFECT.join(", ")
            ),
            span_of(stage),
            stages.text(&list[..stage]).into_iter().collect(),
        ),
        Violation::AfterVerify { verify, stage } => one_effect(
            format!(
                "`{}` follows `{}`; `verify` ends the pipeline",
                name(stage),
                name(verify)
            ),
            span_of(stage),
            stages.text(&list[..stage]).into_iter().collect(),
        ),
        Violation::Malformed { stage, message } => {
            malformed(message, span_of(stage).or(stages.span))
        }
    }
}

/// Builds plans of bound documents.
pub(crate) struct Planner<'a> {
    registry: &'a Registry,
    options: &'a PlanOptions,
}

impl<'a> Planner<'a> {
    pub(crate) fn new(registry: &'a Registry, options: &'a PlanOptions) -> Self {
        Self { registry, options }
    }

    pub(crate) fn plan(&self, document: &BoundDocument) -> Result<Plan> {
        let statements = &document.program.statements;
        let mut planned = Vec::with_capacity(statements.len());
        let mut operations: u32 = 0;
        for (position, statement) in statements.iter().enumerate() {
            match statement {
                BoundStatement::Let(binding) => {
                    let pipeline = self
                        .pipeline(&binding.value, Role::Let)
                        .map_err(|error| split_let(error, statements, position))?;
                    planned.push(PlanStatement::Let(LetPlan {
                        name: binding.name.clone(),
                        pipeline,
                        span: binding.span,
                    }));
                }
                BoundStatement::Pipeline(pipeline) => {
                    let pipeline = self.pipeline(pipeline, Role::Operation)?;
                    planned.push(PlanStatement::Operation(OperationPlan {
                        index: operations,
                        pipeline,
                    }));
                    operations = operations.saturating_add(1);
                }
                _ => return Err(malformed("the statement has an unsupported kind", None)),
            }
        }
        hints::annotate(&mut planned, self.registry, self.options);
        let class = planned.iter().fold(EffectClass::READ, |class, statement| {
            let pipeline = statement.pipeline();
            class.join(pipeline.effect, pipeline.reversibility)
        });
        Ok(Plan {
            catalog_rev: document.catalog_rev.clone(),
            corr_rev: document.corr_rev.clone(),
            statements: planned,
            effect: class.effect,
            reversibility: class.reversibility,
            corrections: self.options.corrections.clone(),
        })
    }

    fn pipeline(&self, pipeline: &BoundPipeline, role: Role) -> Result<PipelinePlan> {
        let stages = Stages::of(pipeline)?;
        validate(&stages, role).map_err(|violation| violation_error(&stages, violation))?;
        let mut planned = Vec::with_capacity(stages.list.len());
        for (index, stage) in stages.list.iter().enumerate() {
            planned.push(self.stage(stage, index + stages.offset())?);
        }
        let class = EffectClass::of_stages(&planned);
        Ok(PipelinePlan {
            head: stages.head.cloned(),
            stages: planned,
            effect: class.effect,
            reversibility: class.reversibility,
            span: stages.span,
        })
    }

    fn stage(&self, stage: &BoundStage, index: usize) -> Result<PlannedStage> {
        let spec = self
            .registry
            .require_command(&stage.name)
            .map_err(|error| with_span(error.with_stage(ExecStep::Bind), stage.span))?;
        let check = if stage.phase == Phase::Verify {
            Some(Box::new(self.verify_pipeline(stage)?))
        } else {
            None
        };
        Ok(PlannedStage {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            step: step_of(stage),
            stage: stage.clone(),
            reversibility: stage_reversibility(stage, spec),
            demand: Demand::default(),
            check,
        })
    }

    fn verify_pipeline(&self, stage: &BoundStage) -> Result<PipelinePlan> {
        let check = stage
            .args
            .iter()
            .find(|arg| arg.name == VERIFY_CHECK)
            .map(|arg| &arg.value);
        match check {
            Some(BoundExpr::Sub { p }) => self.pipeline(p, Role::Verify),
            _ => Err(with_span(
                Error::new(
                    ErrorCode::MissingParam,
                    "`verify` needs a pipeline for `check`",
                )
                .with_stage(ExecStep::Bind),
                stage.span,
            )),
        }
    }
}

/// Adds to an `E_ONE_EFFECT` error of a named result the calls that split
/// the program: the pipeline of the result on its own, then every following
/// statement.
fn split_let(error: Error, statements: &[BoundStatement], position: usize) -> Error {
    if error.code() != ErrorCode::OneEffect {
        return error;
    }
    let steps: Option<Vec<String>> = statements[position..]
        .iter()
        .enumerate()
        .map(|(offset, statement)| match statement {
            BoundStatement::Let(binding) if offset == 0 => text::pipeline(&binding.value),
            BoundStatement::Let(binding) => {
                text::pipeline(&binding.value).map(|text| format!("let {} = {text}", binding.name))
            }
            BoundStatement::Pipeline(pipeline) => text::pipeline(pipeline),
            _ => None,
        })
        .collect();
    steps
        .unwrap_or_default()
        .into_iter()
        .fold(error, Error::with_next_step)
}
