// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stage of the `journal.pin` command: protects journal entries from expiry
//! by steps, age and size, or releases that protection.

use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{MutationTx, OpId, PinOutcome, PinRequest};
use nurt_contracts::record::EffectStatus;
use nurt_contracts::report::Irreversible;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, StageDescriptor,
    StageImpl, StageInput, StaticInfo,
};
use nurt_contracts::value::{Fields, Value};

use super::{Params, records};

const NAME: &str = "journal.pin";

/// Arguments of `journal.pin`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PinArgs {
    ids: Vec<OpId>,
    release: bool,
    /// Pin duration; `None` uses the configured maximum pin age.
    until: Option<Duration>,
}

/// Reads the arguments of `journal.pin`.
pub(super) fn parse(args: &BoundArgs) -> Result<PinArgs> {
    let params = Params::new(args);
    let ids = params.ops("id")?.unwrap_or_default();
    let release = params.flag("release")?;
    let until = match params.duration("until")? {
        None => None,
        Some(duration) => Some(
            duration
                .to_std()
                .filter(|duration| !duration.is_zero())
                .ok_or_else(|| params.invalid("`until` must be a positive duration"))?,
        ),
    };
    if ids.is_empty() {
        return Err(Error::new(
            ErrorCode::MissingParam,
            format!("`{NAME}` needs at least one entry number for `id`"),
        ));
    }
    Ok(PinArgs {
        ids,
        release,
        until,
    })
}

/// `effect` record of a pin or a release.
fn effect(parsed: &PinArgs, result: &PinOutcome) -> nurt_contracts::record::Effect {
    let action = if parsed.release { "released" } else { "pinned" };
    let mut after = Fields::new();
    after.insert("changed".to_owned(), records::op_list(&result.changed));
    after.insert(
        "not_available".to_owned(),
        records::op_list(&result.not_available),
    );
    after.insert("expired".to_owned(), records::op_list(&result.expired));
    let summary = [
        (action, result.changed.len()),
        ("not_available", result.not_available.len()),
        ("expired", result.expired.len()),
    ]
    .into_iter()
    .map(|(name, len)| (name.to_owned(), records::count(len)))
    .collect();
    nurt_contracts::record::Effect {
        targets: Some(records::count(result.changed.len())),
        summary: Some(summary),
        action: Some(if parsed.release { "release" } else { "pin" }.to_owned()),
        after: Some(Value::Map(after)),
        ..records::effect(NAME, EffectStatus::Ok)
    }
}

/// The `journal.pin` command.
pub(super) struct Pin;

impl StageImpl for Pin {
    fn descriptor(&self) -> StageDescriptor {
        super::source(NAME)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        parse(args)?;
        Ok(StaticInfo::default())
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Pin {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        parse(args)?;
        let mut plan = EffectPlan::new();
        plan.preview.push(records::planned(NAME));
        Ok(plan)
    }

    /// Pins or releases the entries; entries that expire at once because a
    /// release brought them over a limit carry `journal_entries_expired`.
    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        _plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        ctx.checkpoint()?;
        let parsed = parse(args)?;
        let result = super::store(ctx)?.pin(&PinRequest {
            ids: parsed.ids.clone(),
            release: parsed.release,
            until: parsed.until,
        })?;
        let irreversible = records::marker(Irreversible::JournalEntriesExpired, &result.expired)
            .into_iter()
            .collect();
        Ok(records::outcome(
            effect(&parsed, &result),
            Vec::new(),
            irreversible,
        ))
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::value::{self, Value};

    use super::super::testing::{Env, apply, call};
    use super::*;

    fn ids(list: &[i64]) -> Value {
        Value::List(list.iter().map(|id| Value::Int(*id)).collect())
    }

    #[test]
    fn pins_available_entries_and_reports_the_others() {
        let env = Env::new();
        let notes = env.seed("notes.txt", b"draft");
        env.write(&notes, b"final");
        let ctx = env.ctx(NAME);
        let done = apply(&env, &Pin, &call(NAME, &[("id", ids(&[1, 9]))]), &ctx).unwrap();
        let after = done.records[0].get("after").unwrap();
        assert_eq!(after.get("changed"), Some(&ids(&[1])));
        assert_eq!(after.get("not_available"), Some(&ids(&[9])));
        assert!(done.irreversible.is_empty());
        assert!(env.pinned(1));

        let release = call(NAME, &[("id", ids(&[1])), ("release", Value::Bool(true))]);
        let done = apply(&env, &Pin, &release, &ctx).unwrap();
        let effect = &done.records[0];
        assert_eq!(
            effect.get("action"),
            Some(&Value::Text("release".to_owned()))
        );
        assert!(!env.pinned(1));
    }

    #[test]
    fn needs_at_least_one_entry() {
        let error = parse(&call(NAME, &[])).unwrap_err();
        assert_eq!(error.code, ErrorCode::MissingParam);
        let zero = call(
            NAME,
            &[
                ("id", ids(&[1])),
                ("until", Value::Duration(value::Duration::ZERO)),
            ],
        );
        assert_eq!(parse(&zero).unwrap_err().code, ErrorCode::InvalidValue);
    }

    #[test]
    fn rejects_entry_numbers_below_one_and_values_that_are_not_numbers() {
        for id in [
            Value::Int(0),
            Value::Int(-5),
            Value::Int(i64::MIN),
            ids(&[1, 0]),
        ] {
            let error = parse(&call(NAME, &[("id", id)])).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidValue);
        }
        let mixed = Value::List(vec![Value::Int(1), Value::Text("2".to_owned())]);
        let error = parse(&call(NAME, &[("id", mixed)])).unwrap_err();
        assert_eq!(error.code, ErrorCode::Type);
        let single = parse(&call(NAME, &[("id", Value::Int(4))])).unwrap();
        assert_eq!(single.ids, [OpId(4)]);
        let largest = parse(&call(NAME, &[("id", Value::Int(i64::MAX))])).unwrap();
        assert_eq!(largest.ids, [OpId(u64::try_from(i64::MAX).unwrap())]);
    }

    #[test]
    fn rejects_negative_and_mistyped_pin_durations() {
        let negative = value::Duration::from_millis(-1000);
        let args = call(
            NAME,
            &[("id", ids(&[1])), ("until", Value::Duration(negative))],
        );
        assert_eq!(parse(&args).unwrap_err().code, ErrorCode::InvalidValue);
        let wrong = call(NAME, &[("id", ids(&[1])), ("until", Value::Int(5))]);
        assert_eq!(parse(&wrong).unwrap_err().code, ErrorCode::Type);
        let none = parse(&call(NAME, &[("id", ids(&[1]))])).unwrap();
        assert_eq!(none.until, None);
    }

    #[test]
    fn the_plan_only_announces_the_effect() {
        use super::super::testing::plan;

        let env = Env::new();
        let ctx = env.ctx(NAME);
        let planned = plan(&Pin, &call(NAME, &[("id", ids(&[1]))]), &ctx).unwrap();
        assert_eq!(planned.preview.len(), 1);
        assert!(planned.changes.is_empty());
        assert!(planned.irreversible.is_empty());
    }

    #[test]
    fn reads_the_pin_duration() {
        let hour = value::Duration::from_millis(3_600_000);
        let args = call(
            NAME,
            &[("id", ids(&[2, 3])), ("until", Value::Duration(hour))],
        );
        let parsed = parse(&args).unwrap();
        assert_eq!(parsed.ids, [OpId(2), OpId(3)]);
        assert_eq!(parsed.until, Some(Duration::from_secs(3600)));
        assert!(!parsed.release);
    }
}
