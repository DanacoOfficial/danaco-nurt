// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `schedule.create`.

use nurt_contracts::bound::BoundPipeline;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::journal::{Expect, Mutation, MutationTx, TargetAction, TargetIndex};
use nurt_contracts::report::{Change, ChangeAction};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Instant, Value};

use super::{
    bool_arg, duration_arg, exactly_one, missing_dirs, notify_changed, now, running_link,
    schedule_record, store, target, text_arg,
};
use crate::action::{ActionRegistry, PIPELINE_KIND, args_hash, pipeline_hash, pipeline_text};
use crate::errors::invalid;
use crate::module::ModuleRequest;
use crate::settings::ScheduleSettings;
use crate::store::{Definition, FORMAT, OnMissed, Overlap, Status, encode_definition, next_id};
use crate::trigger::time::{self, value_duration};
use crate::trigger::{CronSpec, LOCAL, TriggerDef};

const COMMAND: &str = "schedule.create";
const TRIGGER_PARAMS: &[&str] = &["cron", "every", "at"];
const ACTION_PARAMS: &[&str] = &["action", "action_kind"];
const DIRECTORY_MODE: u32 = 0o700;

/// `schedule.create`: creates a schedule.
#[derive(Debug, Clone, Copy, Default)]
pub struct Create;

struct Action {
    kind: String,
    pipeline: Option<BoundPipeline>,
    args: Option<serde_json::Value>,
    text: String,
    hash: String,
}

impl StageImpl for Create {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo, Error> {
        let trigger = exactly_one(args, TRIGGER_PARAMS)?;
        exactly_one(args, ACTION_PARAMS)?;
        if trigger == "cron"
            && let Some(expression) = text_arg(args, "cron")?
        {
            CronSpec::parse(&expression)?;
        }
        if let Some(Value::Int(count)) = args.value("max_runs")
            && *count < 1
        {
            return Err(invalid("`max_runs` must be at least 1"));
        }
        Ok(StaticInfo::default())
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Create {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan, Error> {
        let store = store(ctx)?;
        let settings = ScheduleSettings::from_extensions(ctx.extensions());
        let now = now(ctx);
        let definitions = store.load_definitions()?.items;
        let state = store.read_state()?;
        let open = definitions
            .iter()
            .filter(|definition| {
                state.schedules.get(&definition.id).is_none_or(|entry| entry.status == Status::Active)
            })
            .count();
        if u32::try_from(open).unwrap_or(u32::MAX) >= settings.max_active {
            return Err(Error::new(
                ErrorCode::Limit,
                format!(
                    "{open} schedules exist, the limit `schedule.max_active` is {}; remove \
                     schedules with `schedule.remove`",
                    settings.max_active
                ),
            ));
        }
        let id = next_id(&definitions, &state);
        let definition = build(args, ctx, &settings, id, now)?;
        let path = store.definition_path(&definition.id);
        let running = running_link(ctx).is_some();
        let mut plan = EffectPlan::new();
        plan.changes
            .push(Change::new(path.display().to_string(), ChangeAction::Create));
        plan.preview
            .push(schedule_record(&definition, None, running, now));
        Ok(plan.with_state(definition))
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>, Error> {
        let definition: Definition = plan
            .take_state()
            .ok_or_else(|| crate::errors::failed("the plan of `schedule.create` has no definition"))?;
        let store = store(ctx)?;
        let path = store.definition_path(&definition.id);
        let dirs = missing_dirs(&path);
        let mut targets = Vec::with_capacity(dirs.len() + 1);
        for dir in &dirs {
            targets.push(target(dir, TargetAction::Create, Expect::Absent)?);
        }
        targets.push(target(&path, TargetAction::Create, Expect::Absent)?);
        tx.preserve(&targets)?;
        for index in 0..dirs.len() {
            let index = u32::try_from(index).unwrap_or(u32::MAX);
            tx.apply(TargetIndex(index), Mutation::CreateDir { mode: DIRECTORY_MODE })?;
        }
        let file_index = u32::try_from(dirs.len()).unwrap_or(u32::MAX);
        tx.apply(
            TargetIndex(file_index),
            Mutation::WriteBytes(encode_definition(&definition)?),
        )?;
        notify_changed(ctx);
        let link = running_link(ctx);
        if bool_arg(args, "run_now", false)?
            && let Some(link) = &link
        {
            link.request(ModuleRequest::new(
                "schedule.run",
                serde_json::json!({ "id": definition.id, "trigger": "run_now" }),
            ))?;
        }
        let record = schedule_record(&definition, None, link.is_some(), now(ctx));
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(record)));
        outcome.changes = plan.changes;
        Ok(outcome)
    }
}

fn build(
    args: &BoundArgs,
    ctx: &ExecCtx,
    settings: &ScheduleSettings,
    id: String,
    now: Instant,
) -> Result<Definition, Error> {
    let label = text_arg(args, "label")?.ok_or_else(|| {
        Error::new(ErrorCode::MissingParam, "`schedule.create` needs a value for `label`")
    })?;
    let trigger = trigger(args, ctx, settings, now)?;
    let action = action(args, ctx)?;
    let timeout = duration_arg(args, "timeout")?.unwrap_or(settings.default_timeout);
    if timeout > settings.max_timeout {
        return Err(invalid(format!(
            "the time limit of a run is at most {}; lower `timeout`",
            crate::trigger::format_duration(settings.max_timeout)
        )));
    }
    let expires = match args.value("expires") {
        Some(Value::Instant(at)) => Some(*at),
        Some(Value::Duration(after)) => Some(time::add(now, time::std_duration(*after))),
        _ => None,
    };
    if expires.is_some_and(|expires| expires <= now) {
        return Err(invalid("`expires` is a time that has already passed; give a later time"));
    }
    let max_runs = match args.value("max_runs") {
        Some(Value::Int(count)) if *count >= 1 => u64::try_from(*count).ok(),
        Some(Value::Int(_)) => return Err(invalid("`max_runs` must be at least 1")),
        _ => None,
    };
    let on_missed = enum_arg(args, "on_missed", OnMissed::parse, OnMissed::Skip)?;
    let overlap = enum_arg(args, "overlap", Overlap::parse, Overlap::Skip)?;
    let misfire = duration_arg(args, "misfire")?.unwrap_or(settings.misfire);
    let options = ctx.options();
    let scope = &ctx.config().scope;
    Ok(Definition {
        format: FORMAT.to_owned(),
        id,
        label,
        trigger,
        action_kind: action.kind,
        pipeline: action.pipeline,
        action_args: action.args,
        action: action.text,
        ast_hash: action.hash,
        on_missed,
        misfire: value_duration(misfire),
        overlap,
        timeout: value_duration(timeout),
        max_runs,
        expires,
        paused: !bool_arg(args, "enabled", true)?,
        created: now,
        changed: now,
        session: options.session.clone().or_else(|| ctx.config().session.id.clone()),
        base: options.base.clone().or_else(|| scope.base.clone()),
        project_roots: scope.project_roots.clone(),
    })
}

fn trigger(
    args: &BoundArgs,
    ctx: &ExecCtx,
    settings: &ScheduleSettings,
    now: Instant,
) -> Result<TriggerDef, Error> {
    let timezone = text_arg(args, "timezone")?.unwrap_or_else(|| LOCAL.to_owned());
    let host_zone = ctx.host().info().timezone.clone();
    match exactly_one(args, TRIGGER_PARAMS)?.as_str() {
        "cron" => {
            let expression = text_arg(args, "cron")?.unwrap_or_default();
            TriggerDef::cron(&expression, &timezone, &host_zone, now)
        }
        "every" => {
            let interval = duration_arg(args, "every")?.unwrap_or_default();
            TriggerDef::every(interval, &timezone, &host_zone, settings)
        }
        _ => match args.value("at") {
            Some(Value::Instant(at)) => TriggerDef::at(*at, &timezone, &host_zone, now),
            _ => Err(Error::new(ErrorCode::Type, "`at` expects an instant")),
        },
    }
}

fn action(args: &BoundArgs, ctx: &ExecCtx) -> Result<Action, Error> {
    if exactly_one(args, ACTION_PARAMS)? == "action" {
        let pipeline = args
            .pipeline("action")
            .cloned()
            .ok_or_else(|| Error::new(ErrorCode::Type, "`action` expects a pipeline"))?;
        return Ok(Action {
            kind: PIPELINE_KIND.to_owned(),
            text: pipeline_text(&pipeline),
            hash: pipeline_hash(&pipeline)?,
            pipeline: Some(pipeline),
            args: None,
        });
    }
    let kind = super::text_arg(args, "action_kind")?.unwrap_or_default();
    let action_args = args.value("action_args").map(Value::to_plain_json);
    let registry = ctx.extensions().get::<ActionRegistry>();
    let text = match registry.as_ref().and_then(|registry| registry.get(&kind)) {
        Some(local) => {
            let given = action_args.clone().unwrap_or(serde_json::Value::Null);
            local.validate(&given)?;
            local.describe(&given)
        }
        None => {
            let remote = remote_kinds(ctx);
            if !remote.contains(&kind) {
                return Err(unknown_kind(&kind, registry.as_deref(), &remote));
            }
            format!("{kind} {}", action_args.clone().unwrap_or(serde_json::Value::Null))
        }
    };
    Ok(Action {
        hash: args_hash(&kind, action_args.as_ref())?,
        kind,
        pipeline: None,
        args: action_args,
        text,
    })
}

fn remote_kinds(ctx: &ExecCtx) -> Vec<String> {
    running_link(ctx)
        .and_then(|link| {
            link.request(ModuleRequest::new("schedule.status", serde_json::json!({})))
                .ok()
        })
        .and_then(|status| {
            status.get("action_kinds").and_then(serde_json::Value::as_array).map(|kinds| {
                kinds
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
        })
        .unwrap_or_default()
}

fn unknown_kind(kind: &str, registry: Option<&ActionRegistry>, remote: &[String]) -> Error {
    let mut known: Vec<String> = vec![PIPELINE_KIND.to_owned()];
    if let Some(registry) = registry {
        known.extend(registry.kinds().map(str::to_owned));
    }
    known.extend(remote.iter().cloned());
    Error::new(
        ErrorCode::UnknownValue,
        format!(
            "no action kind `{kind}` is registered; use `action:` with a pipeline or one of: {}",
            known.join(", ")
        ),
    )
    .with_found(kind)
    .with_expected(serde_json::json!(known))
}

fn enum_arg<T>(
    args: &BoundArgs,
    name: &str,
    parse: fn(&str) -> Option<T>,
    default: T,
) -> Result<T, Error> {
    match text_arg(args, name)? {
        None => Ok(default),
        Some(text) => parse(&text).ok_or_else(|| {
            Error::new(ErrorCode::UnknownValue, format!("unknown value `{text}` of `{name}`"))
                .with_found(text)
        }),
    }
}
