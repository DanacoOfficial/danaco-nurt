// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-journal::commands::limits
// Responsibility:   Stage of the `journal.limits` command: reports the effective expiry limits, or sets them and reports the entries that expire.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Stage of the `journal.limits` command: reports the effective expiry
//! limits, or sets them and reports the entries that expire.

use nurt_contracts::config::{AgeLimit, SettingParseError, SizeLimit, SizeSetting, StepLimit};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{LimitsOutcome, LimitsRequest, LimitsScope, MutationTx};
use nurt_contracts::record::{Effect, EffectStatus, FieldValue};
use nurt_contracts::report::Irreversible;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Fields, Record, Size, Value};

use super::{Params, records, status};

const NAME: &str = "journal.limits";

/// Scope of new limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    /// Until the session of the request ends.
    Session,
    /// In the user configuration file.
    User,
}

const SCOPES: [(&str, Scope); 2] = [("session", Scope::Session), ("user", Scope::User)];

/// Arguments of `journal.limits`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct LimitsArgs {
    steps: Option<StepLimit>,
    age: Option<AgeLimit>,
    size: Option<SizeLimit>,
    scope: Scope,
    /// Path on the volume; `None` selects every journal volume.
    volume: Option<String>,
}

impl LimitsArgs {
    /// The call sets at least one limit; otherwise it only reads.
    fn sets_limits(&self) -> bool {
        self.steps.is_some() || self.age.is_some() || self.size.is_some()
    }
}

/// Reads the arguments of `journal.limits`.
pub(super) fn parse(args: &BoundArgs) -> Result<LimitsArgs> {
    let params = Params::new(args);
    Ok(LimitsArgs {
        steps: steps(&params)?,
        age: age(&params)?,
        size: size(&params)?,
        scope: params.choice("scope", &SCOPES)?.unwrap_or(Scope::Session),
        volume: params.path("volume")?,
    })
}

fn setting<T>(params: &Params<'_>, parsed: std::result::Result<T, SettingParseError>) -> Result<T> {
    parsed.map_err(|error| params.invalid(error))
}

fn steps(params: &Params<'_>) -> Result<Option<StepLimit>> {
    match params.value("steps")? {
        None => Ok(None),
        Some(Value::Int(steps)) => setting(params, steps.to_string().parse()).map(Some),
        Some(Value::Enum(word) | Value::Text(word)) => setting(params, word.parse()).map(Some),
        Some(_) => Err(params.type_error("steps", "int or `unlimited`")),
    }
}

fn age(params: &Params<'_>) -> Result<Option<AgeLimit>> {
    match params.value("age")? {
        None => Ok(None),
        Some(Value::Duration(age)) => {
            let millis = u64::try_from(age.as_millis())
                .map_err(|_| params.invalid("`age` must be a positive duration"))?;
            setting(params, format!("{millis}ms").parse()).map(Some)
        }
        Some(Value::Enum(word) | Value::Text(word)) => setting(params, word.parse()).map(Some),
        Some(_) => Err(params.type_error("age", "duration or `unlimited`")),
    }
}

fn size(params: &Params<'_>) -> Result<Option<SizeLimit>> {
    match params.value("size")? {
        None => Ok(None),
        Some(Value::Size(size)) if size.bytes() < SizeLimit::MIN_BYTES => {
            Err(params.invalid(format!(
                "size budget {} is below the minimum {}",
                SizeSetting::from_bytes(size.bytes()),
                SizeSetting::from_bytes(SizeLimit::MIN_BYTES)
            )))
        }
        Some(Value::Size(size)) => Ok(Some(SizeLimit::Bytes(SizeSetting::from_bytes(
            size.bytes(),
        )))),
        Some(Value::Enum(word) | Value::Text(word)) => setting(params, word.parse()).map(Some),
        Some(_) => Err(params.type_error("size", "size, percent or `unlimited`")),
    }
}

fn request(parsed: &LimitsArgs, ctx: &ExecCtx, preview: bool) -> Result<LimitsRequest> {
    let scope = match parsed.scope {
        Scope::User => LimitsScope::User,
        Scope::Session => LimitsScope::Session(super::session(ctx).ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                "`scope: session` needs the session of the request; set the request option `session` or use `scope: user`",
            )
        })?),
    };
    Ok(LimitsRequest {
        steps: parsed.steps,
        age: parsed.age,
        size: parsed.size,
        scope,
        volume: super::resolve_opt(ctx, parsed.volume.as_deref())?,
        preview,
    })
}

/// `effect` record of new limits: the entries that expire, their size and
/// the effective limits per volume.
fn effect(result: &LimitsOutcome) -> Effect {
    let volumes = result
        .effective
        .iter()
        .map(|volume| {
            let mut fields = Fields::new();
            fields.insert(
                "volume".to_owned(),
                Value::Path(volume.volume.to_language_string()),
            );
            let limits = status::journal_limits(&volume.limits, None)
                .into_iter()
                .filter_map(FieldValue::to_field)
                .collect();
            fields.insert("limits".to_owned(), Value::List(limits));
            Value::Map(fields)
        })
        .collect();
    let mut after = Fields::new();
    after.insert("applied".to_owned(), Value::Bool(result.applied));
    after.insert("expiring".to_owned(), records::op_list(&result.expiring));
    after.insert(
        "expiring_bytes".to_owned(),
        Value::Size(Size::from_bytes(result.expiring_bytes)),
    );
    after.insert("volumes".to_owned(), Value::List(volumes));
    let summary = [(
        "entries_expiring".to_owned(),
        records::count(result.expiring.len()),
    )]
    .into_iter()
    .collect();
    let status = if result.applied {
        EffectStatus::Ok
    } else {
        EffectStatus::Planned
    };
    Effect {
        summary: Some(summary),
        after: Some(Value::Map(after)),
        ..records::effect(NAME, status)
    }
}

/// The `journal.limits` command.
pub(super) struct Limits;

impl StageImpl for Limits {
    fn descriptor(&self) -> StageDescriptor {
        super::source(NAME)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        Ok(super::static_info(
            parse(args)?.volume.into_iter().collect(),
        ))
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Limits {
    /// Without limits the plan shows the `journal_state` records; with limits
    /// it reports the entries that would expire, without setting anything.
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let parsed = parse(args)?;
        let store = super::store(ctx)?;
        let mut plan = EffectPlan::new();
        if parsed.sets_limits() {
            let result = store.limits(&request(&parsed, ctx, true)?)?;
            plan.irreversible.extend(records::marker(
                Irreversible::JournalEntriesExpired,
                &result.expiring,
            ));
            plan.preview.push(Record::from(effect(&result)));
        } else {
            let volume = super::resolve_opt(ctx, parsed.volume.as_deref())?;
            plan.preview = status::state_records(store, volume.as_ref())?;
        }
        Ok(plan)
    }

    /// Without limits the output is one `journal_state` record per volume;
    /// with limits it is the `effect` record, and entries that expired carry
    /// `journal_entries_expired`.
    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        _plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        ctx.checkpoint()?;
        let parsed = parse(args)?;
        let store = super::store(ctx)?;
        if !parsed.sets_limits() {
            let volume = super::resolve_opt(ctx, parsed.volume.as_deref())?;
            let states = status::state_records(store, volume.as_ref())?;
            return Ok(EffectOutcome::new(StageOutput::records(
                RecordStream::from_records(states),
            )));
        }
        let result = store.limits(&request(&parsed, ctx, ctx.options().preview())?)?;
        let irreversible = if result.applied {
            records::marker(Irreversible::JournalEntriesExpired, &result.expiring)
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        Ok(records::outcome(effect(&result), Vec::new(), irreversible))
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::value::{self, Value};

    use super::super::testing::{Env, apply, call, plan};
    use super::*;

    #[test]
    fn reports_the_limits_without_parameters() {
        let env = Env::new();
        env.three_operations();
        let ctx = env.ctx(NAME);
        let args = call(NAME, &[]);
        let planned = plan(&Limits, &args, &ctx).unwrap();
        assert_eq!(planned.preview.len(), 1);
        let done = apply(&env, &Limits, &args, &ctx).unwrap();
        assert_eq!(done.records.len(), 1);
        assert_eq!(
            done.records[0].kind,
            nurt_contracts::value::RecordKind::JournalState
        );
        assert!(done.irreversible.is_empty());
    }

    #[test]
    fn lowering_the_step_limit_expires_the_oldest_entry() {
        let env = Env::new();
        env.three_operations();
        let ctx = env.ctx(NAME);
        let args = call(NAME, &[("steps", Value::Int(2))]);

        let planned = plan(&Limits, &args, &ctx).unwrap();
        assert_eq!(planned.irreversible[0].count, Some(1));
        assert_eq!(env.available(), 3);

        let done = apply(&env, &Limits, &args, &ctx).unwrap();
        let after = done.records[0].get("after").unwrap();
        assert_eq!(after.get("applied"), Some(&Value::Bool(true)));
        assert_eq!(
            after.get("expiring"),
            Some(&Value::List(vec![Value::Int(1)]))
        );
        assert_eq!(
            done.irreversible[0].reason,
            Irreversible::JournalEntriesExpired
        );
        assert_eq!(env.available(), 2);
    }

    #[test]
    fn session_scope_needs_a_session() {
        let env = Env::new();
        let ctx = env.ctx_with(NAME, |options| options.session = None);
        let args = call(NAME, &[("steps", Value::Int(5))]);
        let error = apply(&env, &Limits, &args, &ctx).err().unwrap();
        assert_eq!(error.code, ErrorCode::InvalidValue);
    }

    #[test]
    fn reads_unlimited_and_text_forms_of_each_limit() {
        let word = || Value::Enum("unlimited".to_owned());
        let parsed = parse(&call(
            NAME,
            &[("steps", word()), ("age", word()), ("size", word())],
        ))
        .unwrap();
        assert_eq!(parsed.steps, Some(StepLimit::Unlimited));
        assert_eq!(parsed.age, Some(AgeLimit::Unlimited));
        assert_eq!(parsed.size, Some(SizeLimit::Unlimited));
        assert_eq!(parsed.scope, Scope::Session);
        assert!(parsed.sets_limits());

        let text = parse(&call(
            NAME,
            &[
                ("steps", Value::Text("100000".to_owned())),
                ("age", Value::Text("2h".to_owned())),
                ("volume", Value::Text("/mnt/zażółć".to_owned())),
            ],
        ))
        .unwrap();
        assert_eq!(text.steps, Some(StepLimit::Limited(100_000)));
        assert_eq!(text.age, Some("2h".parse().unwrap()));
        assert_eq!(text.volume.as_deref(), Some("/mnt/zażółć"));
        assert!(!parse(&call(NAME, &[])).unwrap().sets_limits());
    }

    #[test]
    fn size_budgets_are_accepted_from_the_minimum_upwards() {
        let at_minimum = Value::Size(Size::from_bytes(SizeLimit::MIN_BYTES));
        let parsed = parse(&call(NAME, &[("size", at_minimum)])).unwrap();
        assert_eq!(
            parsed.size,
            Some(SizeLimit::Bytes(SizeSetting::from_bytes(
                SizeLimit::MIN_BYTES
            )))
        );
        let below = Value::Size(Size::from_bytes(SizeLimit::MIN_BYTES - 1));
        let error = parse(&call(NAME, &[("size", below)])).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidValue);
    }

    #[test]
    fn rejects_mistyped_and_out_of_range_limits() {
        let invalid = [
            ("steps", Value::Int(100_001), ErrorCode::InvalidValue),
            ("steps", Value::Int(-1), ErrorCode::InvalidValue),
            ("steps", Value::Bool(true), ErrorCode::Type),
            (
                "age",
                Value::Duration(value::Duration::from_millis(-1)),
                ErrorCode::InvalidValue,
            ),
            (
                "age",
                Value::Duration(value::Duration::from_millis(9_999)),
                ErrorCode::InvalidValue,
            ),
            ("age", Value::Bool(true), ErrorCode::Type),
            ("size", Value::Bool(true), ErrorCode::Type),
            (
                "scope",
                Value::Enum("forever".to_owned()),
                ErrorCode::InvalidValue,
            ),
        ];
        for (name, given, code) in invalid {
            let error = parse(&call(NAME, &[(name, given.clone())])).unwrap_err();
            assert_eq!(error.code, code, "{name} {given:?}");
        }
    }

    #[test]
    fn a_report_without_limits_needs_no_session_and_changes_nothing() {
        let env = Env::new();
        env.three_operations();
        let ctx = env.ctx_with(NAME, |options| options.session = None);
        let args = call(NAME, &[]);
        let done = apply(&env, &Limits, &args, &ctx).unwrap();
        assert_eq!(done.records.len(), 1);
        assert!(done.irreversible.is_empty());
        assert_eq!(env.available(), 3);
    }

    #[test]
    fn a_preview_reports_the_entries_that_would_expire_and_changes_nothing() {
        let env = Env::new();
        env.three_operations();
        let ctx = env.ctx(NAME);
        let args = call(NAME, &[("steps", Value::Int(1))]);
        let planned = plan(&Limits, &args, &ctx).unwrap();
        assert_eq!(planned.irreversible[0].count, Some(2));
        assert_eq!(planned.preview.len(), 1);
        assert_eq!(env.available(), 3);
    }

    #[test]
    fn rejects_values_out_of_range() {
        let invalid = [
            ("steps", Value::Int(0)),
            ("age", Value::Duration(value::Duration::from_millis(1000))),
            ("size", Value::Size(Size::from_bytes(1024))),
            ("size", Value::Text("95%".to_owned())),
            ("steps", Value::Enum("forever".to_owned())),
        ];
        for (name, value) in invalid {
            let error = parse(&call(NAME, &[(name, value)])).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidValue, "{name}");
        }
        let parsed = parse(&call(
            NAME,
            &[
                ("size", Value::Text("50%free".to_owned())),
                ("scope", Value::Enum("user".to_owned())),
            ],
        ))
        .unwrap();
        assert_eq!(parsed.size, Some("50%free".parse().unwrap()));
        assert_eq!(parsed.scope, Scope::User);
        assert!(parsed.sets_limits());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
