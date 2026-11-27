// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-exec::runner
// Responsibility:   Runner of pipeline arguments (`verify check:`, `write from:`).
// Dependencies:     nurt_catalog, nurt_contracts, nurt_types
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Runner of pipeline arguments (`verify check:`, `write from:`).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use nurt_catalog::Registry;
use nurt_contracts::bound::{BoundHead, BoundPipeline, BoundRef, BoundStage, Phase};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::report::Correction;
use nurt_contracts::stage::{
    Demand, ExecCtx, ExecStep, PipelineRunner, StageFrame, StageInput, StageOutput,
};
use nurt_contracts::value::Value;

use crate::effect::{ProgramSource, apply_effect};
use crate::eval::ExprBinding;
use crate::flow::{
    Selection, at_step, execute_split, guard_kinds, read_interface, resolve, step_of, tag_output,
};
use crate::grouped::plan_effect;
use crate::outcome::ResultData;

/// Runs the pipeline arguments of stages, such as the check pipeline of
/// `verify` and the content pipeline of `write from:`.
///
/// A pipeline argument starts with a source stage, with a named result, or
/// with a stage that reads the input given by the calling stage. Its stages
/// run one after another and each output is collected before the next stage
/// starts, within [`Limits::max_records`]; the output of the last stage is
/// returned. An effect stage in a pipeline argument (for example `run` in a
/// `verify` check) is planned and applied as an operation of its own in the
/// journal of the context; in a preview it is only planned and its preview
/// records are its output.
///
/// [`Limits::max_records`]: nurt_contracts::stage::Limits::max_records
pub struct SubpipelineRunner {
    registry: Arc<Registry>,
    binding: Arc<ExprBinding>,
    catalog_rev: String,
    corr_rev: String,
    corrections: Vec<Correction>,
    preserve_budget: Option<Duration>,
}

impl SubpipelineRunner {
    /// Runner resolving implementations in `registry` and evaluating
    /// arguments with `binding`.
    #[must_use]
    pub fn new(registry: Arc<Registry>, binding: Arc<ExprBinding>) -> Self {
        Self {
            registry,
            binding,
            catalog_rev: String::new(),
            corr_rev: String::new(),
            corrections: Vec::new(),
            preserve_budget: None,
        }
    }

    /// Sets the catalog and correction revisions and the corrections stored
    /// with operations of effect stages.
    #[must_use]
    pub fn with_program(
        mut self,
        catalog_rev: impl Into<String>,
        corr_rev: impl Into<String>,
        corrections: Vec<Correction>,
    ) -> Self {
        self.catalog_rev = catalog_rev.into();
        self.corr_rev = corr_rev.into();
        self.corrections = corrections;
        self
    }

    /// Sets the time budget for preserving the targets of launched programs.
    #[must_use]
    pub fn with_preserve_budget(mut self, budget: Duration) -> Self {
        self.preserve_budget = Some(budget);
        self
    }

    fn program(&self) -> ProgramSource<'_> {
        ProgramSource {
            registry: &self.registry,
            catalog_rev: &self.catalog_rev,
            corr_rev: &self.corr_rev,
            corrections: &self.corrections,
        }
    }

    fn stage(
        &self,
        stage: &BoundStage,
        index: usize,
        input: StageInput<'_>,
        ctx: &ExecCtx,
    ) -> Result<ResultData> {
        let step = step_of(stage.phase);
        let frame = StageFrame {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            command: stage.name.clone(),
            step,
            demand: Demand::default(),
        };
        let stage_ctx = ctx.for_stage(frame.clone());
        let args = self
            .binding
            .bind(stage, ctx.clock())
            .map_err(|error| at_step(error, step))?;
        let max = ctx.limits().max_records;
        let selection = resolve(&self.registry, &stage.name, input, None, max)
            .map_err(|error| at_step(error, step))?;
        if !stage.phase.is_effect() {
            return match selection {
                Selection::One(implementation, input) => {
                    let read = read_interface(&implementation, &stage.name)?;
                    let input = guard_kinds(input, &stage.name, &implementation.descriptor().input);
                    let output = read
                        .execute(&args, input, &stage_ctx)
                        .map_err(|error| at_step(error, step))?;
                    ResultData::collect(tag_output(output, step, &stage_ctx), max)
                }
                Selection::Split(split) => {
                    let output = execute_split(split, &stage.name, &args, &stage_ctx, max)
                        .map_err(|error| at_step(error, step))?;
                    ResultData::collect(tag_output(output, step, &stage_ctx), max)
                }
            };
        }
        let (effect, plan) = plan_effect(selection, &stage.name, &args, &stage_ctx)
            .map_err(|error| at_step(error, ExecStep::Plan))?;
        if ctx.options().preview == Some(true) {
            return Ok(ResultData::from_records(plan.preview));
        }
        let journal = ctx.journal().ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                format!("`{}` needs the journal to change the world", stage.name),
            )
            .with_stage(ExecStep::Apply)
        })?;
        let program = self.program();
        let document = program.document(None, std::iter::once(stage), stage.span);
        let budget = (stage.phase == Phase::Run)
            .then_some(self.preserve_budget)
            .flatten();
        let bulk = plan.bulk || plan.changes.len() > 1;
        let request = program.begin_request(&stage.name, document, ctx, bulk, budget);
        let apply_ctx = stage_ctx.for_stage(StageFrame {
            step: ExecStep::Apply,
            ..frame
        });
        apply_effect(&effect, &args, plan, journal, request, &apply_ctx)
            .map(|applied| applied.records)
    }

    /// A head stage starts the pipeline without input when it is a source,
    /// or an effect stage in its source form (`run` as the check of
    /// `verify`).
    fn starts_pipeline(&self, stage: &BoundStage) -> bool {
        stage.phase == Phase::Source
            || (stage.phase.is_effect()
                && self
                    .registry
                    .implementations(&stage.name)
                    .any(|descriptor| descriptor.source))
    }

    fn named(&self, reference: &BoundRef) -> Result<Value> {
        named_value(&self.binding, reference)
    }
}

/// Value of a reference to a named result, with its field path.
pub(crate) fn named_value(binding: &ExprBinding, reference: &BoundRef) -> Result<Value> {
    let value = binding.named(&reference.name).ok_or_else(|| {
        Error::new(
            ErrorCode::UnknownName,
            format!("unknown name `@{}`", reference.name),
        )
        .with_found(format!("@{}", reference.name))
        .with_stage(ExecStep::Read)
    })?;
    nurt_types::get_path(&value, &reference.fields)
}

impl PipelineRunner for SubpipelineRunner {
    fn run<'a>(
        &'a self,
        pipeline: &'a BoundPipeline,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let max = ctx.limits().max_records;
        let (first, mut data) = match &pipeline.head {
            BoundHead::Ref(reference) => (None, ResultData::from_value(self.named(reference)?)),
            BoundHead::Stage(stage) if self.starts_pipeline(stage) => {
                (Some(stage), self.stage(stage, 0, StageInput::none(), ctx)?)
            }
            BoundHead::Stage(stage) => {
                let input = ResultData::collect(input, max)?;
                (Some(stage), self.stage(stage, 0, input.into_input(), ctx)?)
            }
            _ => {
                return Err(Error::new(
                    ErrorCode::Internal,
                    "the pipeline argument starts with an unknown element",
                ));
            }
        };
        let offset = usize::from(first.is_some());
        for (position, stage) in pipeline.stages.iter().enumerate() {
            ctx.checkpoint()?;
            data = self.stage(stage, position + offset, data.into_input(), ctx)?;
        }
        Ok(data.into_output())
    }
}

impl fmt::Debug for SubpipelineRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubpipelineRunner")
            .field("catalog_rev", &self.catalog_rev)
            .field("corr_rev", &self.corr_rev)
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
