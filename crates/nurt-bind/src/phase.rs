// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Effect kinds, execution phases, waiting and the rule of one effect.
//!
//! A pipeline has at most one stage that changes the world (`mutate`) or
//! runs a program (`run`). Before it stand the source, transformations and
//! checks; after it only the stages that inspect the records of the effect
//! (phase `after`) and a final `verify`. The pipeline of a `let` has no
//! effect. A program that breaks the rule is rejected with `E_ONE_EFFECT`,
//! and `next_steps` holds the separate calls that complete the task.

use nurt_contracts::ast::{Head, Pipeline, Stage};
use nurt_contracts::bound::{BoundArg, BoundExpr, BoundStage, Phase};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::kinds::{Category, EffectKind};
use nurt_contracts::spec::{IoSpec, ParamSpec, StageSpec};
use nurt_contracts::sql_text::changes_database;

use crate::binder::print_pipeline;
use crate::error::BindError;
use crate::literal::duration_ms;
use crate::signature::Bound;

/// Commands admitted after the effect stage, on the records it produced.
pub(crate) const AFTER_STAGES: [&str; 9] = [
    "assert", "filter", "select", "first", "last", "skip", "count", "sort", "group",
];

/// Command that checks the effect and rolls it back on failure.
pub(crate) const VERIFY: &str = "verify";

/// Waiting threshold of a waiting parameter without a literal default: the
/// default `wait` of `run`, in milliseconds.
pub(crate) const DEFAULT_WAIT_THRESHOLD_MS: u64 = 3_000;

/// Value of a waiting parameter that waits until the time limit.
const FOREVER: &str = "forever";

/// Position in a pipeline relative to its effect stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Progress {
    Before,
    After,
    Verified,
}

/// Returns the effect of a bound stage: the effect of the first action of
/// the command that applies, or the command default.
///
/// An action applies when it names the kind of the input records, a value
/// of an enumeration parameter, or a parameter that is written with a value
/// other than `null` and `false`.
pub(crate) fn effect_of(spec: &StageSpec, args: &[BoundArg], input: &IoSpec) -> EffectKind {
    if spec.name == SQL {
        return sql_effect(spec, args);
    }
    spec.effects
        .iter()
        .find(|action| applies(&action.action, args, input))
        .map_or(spec.effect, |action| action.effect)
}

/// Name of the command whose effect depends on its statements.
const SQL: &str = "sql";

/// Effect of `sql`. An explicit `write: true` makes the call a change and
/// `write: false` a read; without `write`, the call is a change when its
/// query is not a literal or holds a statement that may change a database,
/// a connection, a transaction or a file.
fn sql_effect(spec: &StageSpec, args: &[BoundArg]) -> EffectKind {
    let changing = spec
        .effects
        .iter()
        .find(|action| action.action == "write")
        .map_or(spec.effect, |action| action.effect);
    let explicit = args
        .iter()
        .find(|arg| arg.name == "write" && !arg.default)
        .map(|arg| &arg.value);
    match explicit {
        Some(BoundExpr::Bool { v: false }) => EffectKind::Read,
        Some(BoundExpr::Null) | None => {
            match args
                .iter()
                .find(|arg| arg.name == "query")
                .map(|arg| &arg.value)
            {
                Some(BoundExpr::Text { v }) if !changes_database(v) => EffectKind::Read,
                _ => changing,
            }
        }
        Some(_) => changing,
    }
}

fn applies(action: &str, args: &[BoundArg], input: &IoSpec) -> bool {
    let input_kind = matches!(input, IoSpec::Record(kind) if kind == action);
    let enum_value = args
        .iter()
        .any(|arg| matches!(&arg.value, BoundExpr::Enum { v } if v == action));
    let parameter = args
        .iter()
        .any(|arg| arg.name == action && !arg.default && is_set(&arg.value));
    input_kind || enum_value || parameter
}

fn is_set(value: &BoundExpr) -> bool {
    !matches!(value, BoundExpr::Null | BoundExpr::Bool { v: false })
}

/// Returns `true` when the stage blocks the call by waiting: the command
/// always waits, or a waiting parameter has the value `forever` or a
/// duration above its default.
pub(crate) fn waits_of(spec: &StageSpec, args: &[BoundArg]) -> bool {
    spec.waits
        || spec.params.iter().filter(|param| param.waits).any(|param| {
            args.iter()
                .find(|arg| arg.name == param.name)
                .is_some_and(|arg| exceeds(&arg.value, wait_threshold(param)))
        })
}

fn wait_threshold(param: &ParamSpec) -> u64 {
    param
        .default
        .as_ref()
        .and_then(|default| duration_ms(default.as_str().trim()))
        .unwrap_or(DEFAULT_WAIT_THRESHOLD_MS)
}

fn exceeds(value: &BoundExpr, threshold: u64) -> bool {
    match value {
        BoundExpr::Enum { v } => v == FOREVER,
        BoundExpr::Dur { v } => *v > threshold,
        _ => false,
    }
}

/// Returns `true` when the command changes the world or runs a program for
/// its default or for one of its actions.
pub(crate) fn may_affect(spec: &StageSpec) -> bool {
    spec.effect != EffectKind::Read
        || spec
            .effects
            .iter()
            .any(|action| action.effect != EffectKind::Read)
}

/// Phase assignment over the stages of one surface pipeline.
pub(crate) struct Walk<'p> {
    pipeline: &'p Pipeline,
    let_name: Option<&'p str>,
    progress: Progress,
    first_effect: Option<usize>,
}

impl<'p> Walk<'p> {
    /// Starts the walk over a pipeline; `let_name` is the name of the `let`
    /// whose value the pipeline is.
    pub(crate) fn new(pipeline: &'p Pipeline, let_name: Option<&'p str>) -> Self {
        Self {
            pipeline,
            let_name,
            progress: Progress::Before,
            first_effect: None,
        }
    }

    /// Rejects a stage that cannot stand at its position before it is
    /// bound, so that the rule of one effect is reported before the types.
    pub(crate) fn admit(
        &self,
        spec: &StageSpec,
        stage: &Stage,
        index: usize,
    ) -> Result<(), BindError> {
        match self.progress {
            Progress::Before => Ok(()),
            Progress::Verified => Err(self.after_verify(stage, index)),
            Progress::After if may_affect(spec) => Err(self.second_effect(stage, index)),
            Progress::After
                if spec.name == VERIFY || AFTER_STAGES.contains(&spec.name.as_str()) =>
            {
                Ok(())
            }
            Progress::After => Err(self.not_after(stage, index)),
        }
    }

    /// Sets the phase of a bound stage at the surface position `index`.
    pub(crate) fn place(
        &mut self,
        bound: &mut Bound,
        index: usize,
        head: bool,
    ) -> Result<(), BindError> {
        let surface = self.surface_stage(index);
        let stage = &mut bound.stage;
        if stage.effect != EffectKind::Read {
            stage.phase = self.effect_phase(stage, surface, index)?;
            return Ok(());
        }
        let progress = self.progress;
        stage.phase = match progress {
            _ if head => Phase::Source,
            Progress::Before if bound.category == Category::Check => Phase::Check,
            Progress::Before => Phase::Transform,
            Progress::After if stage.name == VERIFY => {
                self.progress = Progress::Verified;
                Phase::Verify
            }
            Progress::After if AFTER_STAGES.contains(&stage.name.as_str()) => Phase::After,
            Progress::After => return Err(self.not_after_named(&stage.name, surface, index)),
            Progress::Verified => return Err(self.after_verify_named(surface, index)),
        };
        Ok(())
    }

    fn effect_phase(
        &mut self,
        stage: &BoundStage,
        surface: Option<&Stage>,
        index: usize,
    ) -> Result<Phase, BindError> {
        if let Some(name) = self.let_name {
            return Err(self.effect_in_let(name, &stage.name, surface));
        }
        if self.progress != Progress::Before {
            return Err(self.second_effect_named(&stage.name, surface, index));
        }
        self.progress = Progress::After;
        self.first_effect = Some(index);
        Ok(if stage.effect == EffectKind::Process {
            Phase::Run
        } else {
            Phase::Mutate
        })
    }

    fn surface_stage(&self, index: usize) -> Option<&'p Stage> {
        match index {
            0 => match &self.pipeline.head {
                Head::Stage(stage) => Some(stage),
                _ => None,
            },
            _ => self.pipeline.stages.get(index - 1),
        }
    }

    fn effect_name(&self) -> String {
        self.first_effect
            .and_then(|index| self.surface_stage(index))
            .map_or_else(
                || "the effect".to_owned(),
                |stage| format!("`{}`", stage.name),
            )
    }

    fn second_effect(&self, stage: &Stage, index: usize) -> BindError {
        self.second_effect_named(&stage.name, Some(stage), index)
    }

    fn second_effect_named(&self, name: &str, surface: Option<&Stage>, index: usize) -> BindError {
        let first = self.first_effect.unwrap_or(0);
        let total = self.pipeline.stages.len() + 1;
        let error = BindError::new(
            ErrorCode::OneEffect,
            format!(
                "`{name}` changes the world after {}; a pipeline has one changing or running stage, so run it as a separate call",
                self.effect_name()
            ),
        )
        .at(surface.and_then(|stage| stage.span))
        .found(name.to_owned())
        .help("pipelines");
        let separate = (0..first).chain(index..total);
        self.with_steps(
            error,
            [
                assemble(self.pipeline, 0..index),
                assemble(self.pipeline, separate),
            ],
        )
    }

    fn not_after(&self, stage: &Stage, index: usize) -> BindError {
        self.not_after_named(&stage.name, Some(stage), index)
    }

    fn not_after_named(&self, name: &str, surface: Option<&Stage>, index: usize) -> BindError {
        let error = BindError::new(
            ErrorCode::OneEffect,
            format!(
                "`{name}` cannot follow {}; after the effect only {} and a final `verify` are allowed",
                self.effect_name(),
                AFTER_STAGES.join(", ")
            ),
        )
        .at(surface.and_then(|stage| stage.span))
        .found(name.to_owned())
        .expected(&after_names())
        .help("pipelines");
        self.with_steps(error, [assemble(self.pipeline, 0..index)])
    }

    fn after_verify(&self, stage: &Stage, index: usize) -> BindError {
        self.after_verify_named(Some(stage), index)
    }

    fn after_verify_named(&self, surface: Option<&Stage>, index: usize) -> BindError {
        let error = BindError::new(
            ErrorCode::OneEffect,
            "`verify` is the last stage of a pipeline; remove the stages after it",
        )
        .at(surface.and_then(|stage| stage.span))
        .help("verify");
        self.with_steps(error, [assemble(self.pipeline, 0..index)])
    }

    fn effect_in_let(&self, let_name: &str, name: &str, surface: Option<&Stage>) -> BindError {
        let error = BindError::new(
            ErrorCode::OneEffect,
            format!(
                "the pipeline of `let {let_name}` cannot change the world and `{name}` changes it; run the pipeline as a separate statement"
            ),
        )
        .at(surface.and_then(|stage| stage.span))
        .found(name.to_owned())
        .help("let");
        self.with_steps(error, [Some(self.pipeline.clone())])
    }

    fn with_steps<const N: usize>(
        &self,
        error: BindError,
        steps: [Option<Pipeline>; N],
    ) -> BindError {
        steps
            .into_iter()
            .flatten()
            .filter_map(print_pipeline)
            .fold(error, |error, step| error.next_step(step))
    }
}

fn after_names() -> Vec<String> {
    AFTER_STAGES
        .iter()
        .chain(std::iter::once(&VERIFY))
        .map(|name| (*name).to_owned())
        .collect()
}

/// Builds a pipeline from the surface elements at the given positions;
/// position 0 is the head.
fn assemble(pipeline: &Pipeline, positions: impl IntoIterator<Item = usize>) -> Option<Pipeline> {
    let mut head: Option<Head> = None;
    let mut stages = Vec::new();
    for position in positions {
        let element = match position {
            0 => pipeline.head.clone(),
            _ => Head::Stage(pipeline.stages.get(position - 1)?.clone()),
        };
        if head.is_none() {
            head = Some(element);
            continue;
        }
        match element {
            Head::Stage(stage) => stages.push(stage),
            _ => return None,
        }
    }
    Some(Pipeline {
        head: head?,
        stages,
        span: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::kinds::{Implementation, Reversibility};
    use nurt_contracts::spec::{ActionEffect, DefaultValue, ParamType};

    fn spec(effect: EffectKind) -> StageSpec {
        StageSpec {
            name: "request".to_owned(),
            group: String::new(),
            category: Category::Source,
            reversibility: Reversibility::Read,
            effect,
            implementation: Implementation::Builtin,
            input: vec![IoSpec::None],
            output: vec![IoSpec::Any],
            summary: "Send a request.".to_owned(),
            reversibility_note: None,
            effects: Vec::new(),
            waits: false,
            os: Vec::new(),
            external: None,
            fallback: None,
            required_groups: Vec::new(),
            params: Vec::new(),
            forms: Vec::new(),
        }
    }

    fn action(name: &str, effect: EffectKind) -> ActionEffect {
        ActionEffect {
            action: name.to_owned(),
            effect,
            reversibility: Reversibility::Irreversible,
            note: None,
        }
    }

    fn wait_param(default: Option<&str>) -> ParamSpec {
        ParamSpec {
            name: "wait".to_owned(),
            ty: "duration | enum(forever)".parse::<ParamType>().unwrap(),
            position: None,
            required: false,
            repeated: false,
            per_record: false,
            default: default.map(DefaultValue::new),
            unbounded_default: false,
            default_from: None,
            default_note: None,
            values: vec!["forever".to_owned()],
            summary: "Time to wait.".to_owned(),
            exclusive_group: None,
            waits: true,
            secret: false,
        }
    }

    #[test]
    fn takes_the_effect_of_an_enumeration_action() {
        let mut request = spec(EffectKind::Read);
        request.effects.push(action("post", EffectKind::External));
        let get = [BoundArg::from_default(
            "method",
            BoundExpr::Enum {
                v: "get".to_owned(),
            },
        )];
        let post = [BoundArg::given(
            "method",
            BoundExpr::Enum {
                v: "post".to_owned(),
            },
        )];
        assert_eq!(effect_of(&request, &get, &IoSpec::None), EffectKind::Read);
        assert_eq!(
            effect_of(&request, &post, &IoSpec::None),
            EffectKind::External
        );
    }

    #[test]
    fn takes_the_effect_of_a_written_parameter_or_an_input_kind() {
        let mut command = spec(EffectKind::Read);
        command.effects.push(action("save", EffectKind::Local));
        command.effects.push(action("job", EffectKind::Local));
        let unset = [BoundArg::from_default("save", BoundExpr::Null)];
        let set = [BoundArg::given(
            "save",
            BoundExpr::Path { v: "/a".to_owned() },
        )];
        assert_eq!(effect_of(&command, &unset, &IoSpec::None), EffectKind::Read);
        assert_eq!(effect_of(&command, &set, &IoSpec::None), EffectKind::Local);
        let job = IoSpec::Record("job".to_owned());
        assert_eq!(effect_of(&command, &unset, &job), EffectKind::Local);
    }

    #[test]
    fn waits_above_the_default_or_forever() {
        let mut run = spec(EffectKind::Process);
        run.params.push(wait_param(Some("3s")));
        let wait = |value| [BoundArg::given("wait", value)];
        assert!(!waits_of(&run, &wait(BoundExpr::Dur { v: 3_000 })));
        assert!(waits_of(&run, &wait(BoundExpr::Dur { v: 3_001 })));
        assert!(waits_of(
            &run,
            &wait(BoundExpr::Enum {
                v: "forever".to_owned()
            })
        ));
        assert!(!waits_of(&run, &wait(BoundExpr::Null)));
    }

    #[test]
    fn waits_above_the_threshold_without_a_default() {
        let mut retry = spec(EffectKind::Process);
        retry.params.push(wait_param(None));
        let args = [BoundArg::given("wait", BoundExpr::Dur { v: 10_000 })];
        assert!(waits_of(&retry, &args));
        let mut sleep = spec(EffectKind::Read);
        sleep.waits = true;
        assert!(waits_of(&sleep, &[]));
    }

    fn sql_spec() -> StageSpec {
        let mut command = spec(EffectKind::Local);
        command.name = "sql".to_owned();
        command.effects.push(action("write", EffectKind::Local));
        command
    }

    fn query(text: &str) -> BoundArg {
        BoundArg::given("query", BoundExpr::Text { v: text.to_owned() })
    }

    #[test]
    fn sql_is_a_read_when_its_statements_only_read() {
        let command = sql_spec();
        for text in [
            "select 1",
            "select * from t; pragma user_version",
            "with a as (select 1) select * from a",
        ] {
            assert_eq!(
                effect_of(&command, &[query(text)], &IoSpec::None),
                EffectKind::Read,
                "{text}"
            );
        }
    }

    #[test]
    fn sql_is_a_change_when_a_statement_changes_something() {
        let command = sql_spec();
        for text in [
            "insert into t values (1)",
            "select 1; delete from t",
            "begin; select 1; commit",
            "attach 'a.db' as a",
            "vacuum",
            "pragma journal_mode = wal",
            "select load_extension('x')",
        ] {
            assert_eq!(
                effect_of(&command, &[query(text)], &IoSpec::None),
                EffectKind::Local,
                "{text}"
            );
        }
    }

    #[test]
    fn sql_with_a_query_that_is_not_a_literal_is_a_change() {
        let command = sql_spec();
        let computed = [BoundArg::given(
            "query",
            BoundExpr::Path {
                v: "/a.sql".to_owned(),
            },
        )];
        assert_eq!(
            effect_of(&command, &computed, &IoSpec::None),
            EffectKind::Local
        );
    }

    #[test]
    fn an_explicit_write_decides_the_effect_of_sql() {
        let command = sql_spec();
        let on = [
            query("select 1"),
            BoundArg::given("write", BoundExpr::Bool { v: true }),
        ];
        let off = [
            query("delete from t"),
            BoundArg::given("write", BoundExpr::Bool { v: false }),
        ];
        assert_eq!(effect_of(&command, &on, &IoSpec::None), EffectKind::Local);
        assert_eq!(effect_of(&command, &off, &IoSpec::None), EffectKind::Read);
    }
}
